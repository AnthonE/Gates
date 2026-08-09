# reference/DOORS.md — how the reference game does doors and locks

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`SPAWN.md` answers *how the world gets placed*, `AUDIO.md` *what a player
hears*, `SAVES.md` *what survives a restart*; this file answers **who is
allowed through a door**, because we shipped a door and a lock in M1 and
`DECISIONS.md`'s "lock v0" row ends by naming shared access as the open
question and handing it to the operator.

Dated 2026-08-08. §9 is the part that changes what we build.

## 0 · Provenance — read this first

Three sources, and they are not equally strong. Ranked:

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. It is a
   *hook* table, so what it proves is the **shape**: which classes exist,
   which methods they hang hooks on, and which method names are shared. That
   is enough to read the object model off it, and §1 does exactly that and
   nothing more.
2. **The developer's own devblogs**, by number and date. Same posture as
   `AUDIO.md` §0: a sentence a developer published about their own work.
3. **Community wikis and guides** for numbers (hp, craft costs, raid costs).
   Weakest tier — player-maintained, undated, and the game has had a decade
   of balance passes over every number in §3. Read them as *ratios that held*,
   never as today's values.

**One honesty note about how this was gathered.** This container's egress
proxy blocked every page fetch attempted — `rust.facepunch.com`,
`wiki.facepunch.com`, both wikis, `umod.org`, every guide site. The devblog
and wiki facts below therefore come through **search-result summaries of
those pages, not the pages themselves**. That is a real weakening: a summary
can drop a qualifier. Facts from source 1 are read directly out of a file in
this repo and carry no such caveat, which is a second reason §1 leads.

Nothing here was decompiled. Nothing here ships: no asset, no name, no
number copied into `content/` without being re-priced against our own
economy.

## 1 · The object model, read off the hook table

Six classes matter, and the hook table gives their methods:

```
Door        [3]  RPC_OpenDoor · RPC_CloseDoor · RPC_KnockDoor
BaseLock    [1]  RPC_TakeLock
CodeLock    [7]  TryLock · TryUnlock · RPC_ChangeCode · UnlockWithCode
                 OnTryToOpen(BasePlayer) · OnTryToClose(BasePlayer)
KeyLock     [4]  Lock(BasePlayer) · RPC_Unlock
                 OnTryToOpen(BasePlayer) · OnTryToClose(BasePlayer)
DoorCloser  [1]  RPC_Take
DoorKnocker [1]  Knock(BasePlayer)
```

Five structural facts fall straight out of that, and they are the valuable
half of this document:

1. **The lock is a separate entity, not a bit on the door.** `BaseLock` is
   its own class with its own subclasses and its own RPC. A door *has* a
   lock or has none.
2. **A door with no lock is a door anyone may open.** There is no third
   state. The door's own class has exactly two access RPCs (open, close) and
   neither takes an owner.
3. **The lock is asked, and it is asked twice.** `CanUseLockedEntity` is one
   hook bound to *two* methods on *both* lock classes: `OnTryToOpen` and
   `OnTryToClose`. So closing is gated by the same predicate as opening — a
   locked door is not "openable by the owner", it is "operable by the
   authorized", and a stranger cannot shut you in either.
4. **The two lock kinds are polymorphic over one interface.** `CodeLock` and
   `KeyLock` bind the same three hook names (`CanLock`, `CanUnlock`,
   `CanUseLockedEntity`) to differently-named methods. The door does not know
   which kind it carries.
5. **A lock is removable and knocking is a first-class verb.**
   `RPC_TakeLock` on the base class means picking the lock back up is a
   normal action on any lock; `RPC_KnockDoor` puts knocking on the *door*,
   not on the lock, which is why it works on a door you have no access to.

Two more classes are worth naming because they are *deliberately not ours*:
`DoorCloser` is a deployable that auto-shuts a door (it has `RPC_Take`, so
it is picked up like any deployable) and `DoorKnocker` is a separate
knocking device. Both are peripherals hung off the same two verbs.

## 2 · The two locks

### 2.1 Key lock — the one they regret

Placed on a door; **keys** are items crafted *from that lock*, and only a key
whose number matches the lock's number opens it. The lock is assigned its
number at deploy time. Keys can be duplicated while the lock is in its
unlocked state, and a duplicate needs a working key already in the crafting
player's inventory. **Removing the lock invalidates every key**, including
after re-placing it.

Two devblog-dated corrections tell the whole story of the design:

- **Devblog 33 (Nov 2014)** — the code lock is concepted and modelled. So the
  key lock is the *older* of the two and the code lock arrived as the answer
  to it, not as a sidegrade.
- **Devblog 193 (Jan 2018)** — reconsidered outright. Two published reasons:
  players **locked themselves out of their own bases** by not making keys,
  and the developer's own verdict, roughly, that they had tried to make the
  system interesting and after years it simply did not work. The fixes
  shipped were (a) **the player who placed the lock can open and close it
  with no key at all**, and (b) the key space went **100 → 1,000,000**.

Fix (a) is the interesting one. It says: after four years, the *first* thing
they added to their key-based lock was an identity-based bypass for the
owner. The key stopped being the only credential.

### 2.2 Code lock — the one everyone uses

A 4-digit PIN, 10 000 combinations. The mechanics that matter:

- **Entering the correct code does not open the door. It authorizes you.**
  The player is added to a remembered list on that lock and thereafter opens
  it freely — the lock's own state, not a session token. This is the single
  most important mechanic in the document.
- **A guest code** (Devblog 149, Feb 2017) is a second PIN with reduced
  rights: a guest may open and close, and may **not** unlock, change the
  code, or take the lock off. The published use case is provisional, paid
  access — the example given is charging for access to a furnace room.
- **Changing the code** is available to any remembered player, as is setting
  the guest code.
- **Wrong codes hurt.** A failed entry gives an **electric shock**, escalating
  in steps of 5 damage per failed attempt starting at 5, with the escalation
  **resetting after 10 seconds**.
- **A lockout on top of that** (Dec 2021): eight failed attempts and the lock
  refuses further attempts for **15 minutes**.
- **The lock itself cannot be destroyed.** It comes off by being removed by
  someone authorized, or when the thing it is bolted to is destroyed. There
  is no shoot-the-lock.
- **It is not door-only.** Boxes, lockers, fridges and the tool cupboard all
  take a lock, through the same `CanUseLockedEntity` predicate.

Two things a code lock is **not**:

- It is **not** the building-privilege system. Cupboard authorization governs
  who may *build*; lock authorization governs who may *pass*. They are
  separate lists with separate verbs (`AddAuthorize` / `ClearList` on
  `BuildingPrivlidge`, versus `UnlockWithCode` on `CodeLock`). Community
  sources conflate these constantly and at least one search result asserted
  cupboard auth opens code-locked doors; the hook table says they are
  unrelated systems and that claim should be treated as wrong.
- It is **not** free. 100 metal fragments, which is a real early-game gate:
  a fresh base is a door with no lock for as long as it takes to smelt.

## 3 · Doors, as a tier ladder

Community-wiki numbers, source tier 3 — the ratios are the content, not the
literals:

| door | hp | notes |
|---|---|---|
| wooden | ~200 | has a **soft side** (the Z-frame face) that takes extra melee |
| sheet metal | ~250 | the standard; the airlock unit of account |
| garage door | ~600 | no soft side, spans a full wall frame |
| armored | ~800–1000 | tier-3 bench, HQM + gears, has a shootable viewing hatch |
| ladder hatch | ~200 | a floor door |

Three shape facts under the numbers:

1. **A door is always weaker than the wall around it.** That is the point of
   a door — it is the intended breach point, and the whole raid economy is
   priced off "how many explosives for the cheapest way in".
2. **Doors are not the only door-shaped thing.** Hatches (floors), garage
   doors (wall-frame span) and double doors are all the same two verbs over
   different sockets.
3. **The soft side exists so that melee is not useless**, and it is a
   *directional* property of a mesh — an orientation-dependent damage
   multiplier, not a health value.

## 4 · Knocking, and why it is a verb and not a sound

Door knocking shipped **12 Nov 2014** — the published framing was that you
could now knock like a polite person instead of chirping the keypad. It is on
`Door`, as an RPC, not on the lock: **you knock on doors you cannot open.**
That is the entire reason it exists — it is the only channel a locked-out
player has to the person inside, and it converts a refusal into a social
event.

It is also, structurally, the cheapest possible feature: no state, no
persistence, no validation beyond reach. One broadcast.

## 5 · What a lock does *not* protect

Worth stating because our tree currently gets one of these wrong by
accident and one on purpose:

- **An unsecured door is anyone's.** Any player may *remove* a door that
  carries no lock. The lock is not only "who opens it", it is "whose door
  this is at all".
- **A lock protects the door, not the wall.** Every number in §3 is about
  going *through* the door with explosives, and the lock is irrelevant to
  that path.
- **A lock does not hide anything.** The lock's state is visible in the
  world — Devblog 43 (Jan 2015) shipped a fix for exactly the case where it
  *wasn't* ("code lock not displaying its status in the world"), which is a
  small fact that says something large: the reference treats *legibility of
  the access state* as a bug class.

## 6 · The failure modes the reference has published

Collected because they are the cheap lessons:

1. **A credential-only lock locks its owner out** (§2.1, Devblog 193). Any
   access scheme whose only key is a transferable object needs an
   identity-based bypass for the person who installed it, or it will be a
   support burden.
2. **A 100-value key space is not a key space** (§2.1). Same devblog. They
   shipped it and lived with it for years.
3. **A lock's state must be legible from outside** (§5, Devblog 43).
4. **Brute force is a real attack and rate-limiting is the answer, twice.**
   The shock (escalating damage) came first and was not enough; the hard
   8-attempt / 15-minute lockout came years later (Dec 2021). A 10 000-space
   PIN with unlimited attempts is a 10 000-press door.
5. **Guest access wants fewer rights, not a second lock** (§2.2, Devblog
   149). The guest code is the same lock with a rights bit.

## 7 · The verb inventory, complete

Every distinct thing a player can do to a door or a lock in the reference,
as a flat list — this is the checklist §9 scores us against:

| # | verb | who | notes |
|---|---|---|---|
| 1 | open / close the door | anyone the lock allows | one toggle, gated both ways |
| 2 | knock | anyone in reach | works *because* you are refused |
| 3 | place a lock on a door | anyone who can reach an unlocked door | costs the lock item |
| 4 | enter a code | anyone in reach | correct ⇒ remembered; wrong ⇒ shock |
| 5 | change the code | remembered players | clears nobody's memory but its own semantics |
| 6 | set / clear a guest code | remembered players | guests get verb 1 only |
| 7 | lock / unlock the lock | remembered players | unlocked ⇒ everyone gets verb 1 |
| 8 | take the lock off | remembered players | returns the item |
| 9 | remove the door | anyone, **if it has no lock** | this is why locks are claims |
| 10 | craft a key from a key lock | anyone with a key + the unlocked lock | key lock only |
| 11 | break the door | anyone with explosives | the lock is not involved |

## 8 · Sources

Tier 1 (in-tree, MIT): `reference/rust-systems.txt`, classes `Door`,
`BaseLock`, `CodeLock`, `KeyLock`, `DoorCloser`, `DoorKnocker`.

Tier 2 (developer devblogs, by number/date, **via search summary — see §0**):
Devblog 33 (Nov 2014, code lock concepted); the 12 Nov 2014 update (door
knocking); Devblog 43 (Jan 2015, lock status display fix); Devblog 149 (Feb
2017, guest code); Devblog 193 (Jan 2018, key lock reconsidered, owner
bypass, 100 → 1 000 000).

Tier 3 (community wikis and guides, **via search summary**): code lock craft
cost and shock schedule, the Dec 2021 8-attempt / 15-minute lockout, key
lock craft cost and key rules, the §3 door ladder.

## 9 · What it means for us

Owned by `sim-core/deploy.rs` and `sim-core/lock.rs`; `DECISIONS.md`'s "lock
v0" row asked the question this section answers.

1. **Our lock is a bit; theirs is an entity, and the difference is the
   mechanic.** `DeployRec::locked` plus `DeployRec::owner` is a *one-player*
   lock with no cost, no removal and no way to share. Every interesting
   thing in §2.2 — the remembered list, the guest tier, the code itself —
   needs somewhere to live that a bool has not got. The fix is the pattern
   this codebase already uses twice: a **dense side-store keyed by the
   deployable's grid address**, exactly as `HearthRec` and `BoxRec` are. That
   is the same relationship the reference has (a lock parented to a door)
   arrived at from our own architecture rather than copied.
2. **"A door places locked to its placer" is the one rule to retire.** It is
   §5's first bullet inverted: we made the door free and the security free,
   and the reference makes the door free and charges for the security. Theirs
   is better and it is *cheaper for us*, because "unlocked" stops being a
   weird state a player chooses and becomes the default state of a door
   nobody has paid for yet.
3. **Authorization is a list, and ours must be bounded** (wall 4). The
   reference's remembered list is unbounded; ours needs a cap and a stated
   overflow policy. A refusal is the right policy — silently dropping the
   oldest authorized player would make a door that forgets its owner.
4. **Close is gated too** (§1 fact 3). Our `use_door` toggles, so this is
   free — but it must stay one predicate, asked once, exactly as
   `door_in_reach` is shared between the use and lock verbs today.
5. **Knock is nearly free and is the highest ratio of feel to work in this
   whole document** (§4). One broadcast event on the refusal path we already
   have, one sound in `crate::sound`. It also fixes a real gap: today a
   stranger's press produces a *sender-only* refusal, so being locked out is
   silent to everyone including the person inside.
6. **The shock is affordable; the 15-minute lockout is the one that
   matters** (§6.4). Escalating damage is a subtraction against `player_hp`
   we already do; the lockout is a tick number on the lock record. Ours
   should ship both at once rather than repeat their decade-long two-step,
   and the counter belongs **on the lock** rather than per-player-per-lock:
   the door is what is under attack, and a per-player table is unbounded.
7. **Keys are blocked on something real, and it is not effort.** §2.1's key
   is an item carrying a lock's number — per-item **instance data**. Our
   `ItemStack` is `(row, count)` and has nowhere to put a number that varies
   between two stacks of the same item. Adding instance data touches the
   inventory, the wire, the save and every container. Given §2.1 is also the
   system the reference itself gave up on, **the code lock is the whole
   answer and the key lock is deliberately not built** — that is a
   conclusion, not a deferral.
8. **Boxes want the same lock** (§2.2). `inventory.rs`'s `CONT_BOX` comment
   already reasons "open to anyone, exactly like an unlocked door" and points
   at the door's owner bit as the thing it declined to copy. Once the lock is
   a side-store keyed by an address, a box has an address too, and the
   predicate is the same function. It is a follow-on slice with no new
   concepts, which is the best kind.
9. **What we should NOT copy**: the key lock (§7 verb 10, and item 7 above);
   `DoorCloser` and `DoorKnocker` as deployables (peripherals on verbs we do
   not have yet); the soft side (§3, a directional damage multiplier that
   wants a hit normal our `combat.rs` does not carry); and door tiers beyond
   the two we have — the ladder in §3 is a *shape* we already satisfy with
   wood and metal, and a third rung is a content row, not a mechanic.
10. **Verb-by-verb, where we stand.** §7's eleven, scored:
    1 open/close ✅ · 2 knock ✅ (lock v1) · 3 place a lock ✅ (lock v1) ·
    4 enter a code ✅ (lock v1) · 5 change the code ✅ (lock v1) ·
    6 guest code ✅ (lock v1) · 7 lock/unlock ✅ · 8 take the lock ✅
    (lock v1) · 9 remove an unsecured door ❌ (no deployable-pickup verb
    exists at all — a separate slice, named in `NOW.md`) · 10 craft a key ❌
    (item 7, deliberately) · 11 break the door ✅ (`charge.rs`, `combat.rs`).
