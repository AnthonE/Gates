# reference/WOUNDED.md — how the reference game lets a player fall down instead of die

Not our design, not our queue, not law. What the reference game does when a
player's health reaches zero — **it does not kill them**, most of the time: it
puts them on the ground for most of a minute, lets a friend pick them up, and
rolls a die at the end — read off the developer's own posts across eight years
of revising it, and **§9 what it means for us**. Written 2026-09-13 because the
operator asked for it in one sentence: *"when we die we need to like fall down
and stuff like u do in rust with a chance of recovery time."*

Nothing here ships. No asset, no name, no number copied into `content/`
without being re-priced against `CONTENT.md` §4's bands — though
`BALANCE.md` §6 says the default is to take theirs, and §9.4 does.

---

## 0 · Provenance — read this first

**Tier 1, fetched whole, on 2026-09-13.** Every `rust.facepunch.com` post
cited below was retrieved in full from this box today — Devblogs 53, 54, 57,
71 and 121, the July 2021 Wounding Update, the September 2021 update, the
August 2022 Hardcore post and the August 2023 *Wounded ☠️* post — and so were
the three `wiki.facepunch.com` item pages (syringe, bandage, large medkit).
That is `FORESTS.md` §0's result again and `DOORS.md` §0's caveat inverted
again: **reachability is a property of the container, not of the hosts —
probe.** Today's probe, so the next reader has a baseline rather than a
memory: `rust.facepunch.com` 200, `wiki.facepunch.com` 200,
`raw.githubusercontent.com` 200, `rust.fandom.com` 403 to a page fetch but
**its `api.php` answers** (the wikitext below came through it), `umod.org`
403, `github.com` 403, `corrosionhour.com` a challenge page with nothing
behind it.

**Tier 2 is the in-tree hook table**, `reference/rust-systems.txt`, MIT and
regenerable — and it was regenerated today from the live `Rust.opj` to check
whether the system had grown a verb: the live rip is 1,722 lines against the
tree's 1,716, and **the seven wounded-related lines are byte-identical**. §1
reads the object model off those lines and nothing else.

**Tier 3 is the community**: Rustafied's write-up of the 2021 update (dated,
by name), the fandom wiki's *Status Effects* and *Mechanics* wikitext, three
hosting-company guides, a games-press preview, and **one public plugin's
source** (`bmgjet/Downed`, GitHub, read raw) for the names of the verbs a
modder can call. Numbers from this tier are marked *(tier 3)* where they
stand alone; where a tier-1 sentence says the same thing, the tier-1 one is
cited.

**What no reachable source says, stated so nobody assumes otherwise:** the
crawl *speed*; the incapacitated state's own timer; the recovery-chance
formula between its two published endpoints; the health you stand up with;
whether an explosion, fire or drowning is *eligible* to wound; and **the
screen** — no written page reached here describes the camera, the colour or
the sound of being down. §4 separates what a source says from what a player
who has been there remembers, and marks the second kind. **The frames are the
source for that half, and the operator has the game.**

Nothing here is decompiled. A 2015 modding-forum thread that quotes the
game's own code was opened and used for **one** fact — that wounded is a
*player flag* — and for no number.

---

## 1 · The object model, read off the hook table

Seven lines in `reference/rust-systems.txt` are about this system. They are
short, and they settle the shape before any devblog is opened:

| hook | patched method | what the signature proves |
|---|---|---|
| `CanBeWounded` | `BasePlayer.EligibleForWounding(HitInfo)` | wounding is a **predicate on the hit**, not on the player. The same lethal damage wounds or kills depending on what dealt it and where it landed — a `HitInfo` carries the weapon, the body part and the damage-type mix, and nothing else is consulted |
| `OnPlayerWound` | `BasePlayer.BecomeWounded(HitInfo)` | **one entry point**, and it takes the hit — the hit decides *which* wounded state you enter (§2.1: a fall goes straight to the floor) |
| `OnPlayerRecover`, `OnPlayerRecovered` | `BasePlayer.RecoverFromWounded()` | **one exit for every cause** — timer roll, a friend's hands, a syringe, a medkit all end in the same method, hooked before and after. There is no `RevivedBy(...)`; who got you up is not part of the exit |
| `OnPlayerAssist` | `BasePlayer.RPC_Assist(RPCMessage)` | the bare-hands revive is an **RPC on the wounded player's entity**, sent by the helper — i.e. it is a verb aimed at a body, the same shape as looting one |
| `OnPlayerRevive` | `BasePlayer.OnMedicalToolApplied(BasePlayer, ItemDefinition, ItemModConsumable, MedicalTool, Boolean)` | the item revive is **the medical tool's ordinary apply-to-target path** — the same method a syringe uses on a standing teammate — with a flag on the end. There is no revive item; there is a healing item and a target that happens to be down |
| `OnHealingItemUse` | `MedicalTool.GiveEffectsTo(BasePlayer, IMedicalToolTarget)` | the target is an *interface*, so a syringe on yourself and a syringe on the body beside you are one code path and the wounded case is a branch inside it |

Two more names come from public plugin source rather than the table, and
both are verbs the game exposes to a modder: **`ProlongWounding(seconds)`** —
the timer can be *extended* in place, which is the mechanism Devblog 121's
"interrupted revive attempts increase the time it takes to die" needs — and
**`StopWounded()`** beside `BecomeWounded(info)` and `IsIncapacitated()`,
from `bmgjet/Downed`. And from the 2015 forum thread, the one fact taken:
`player.HasPlayerFlag(BasePlayer.PlayerFlags.Wounded)` — **wounded is a flag
bit on the player entity**, which is how every other client knows to draw the
body on the ground. Not a component, not a separate entity: a bit.

---

## 2 · The state machine, as it ships

### 2.1 Two states, and which one you get

Since 1 July 2021 there are two wounded states, and the developer's own
sentence is the spec: *"Players now go to a crawling state when wounded in
most cases, instead of being fully incapacitated. They can move around slowly
and use doors."* Crawling is the default. **The older incapacitated state —
flat on the ground, able to look and speak and nothing else — still occurs**
in exactly three named situations: *"if you get looted while in crawling
wounded state, if you die from fall damage, or if you crawl too deep into
water."* A press preview of the same build says the crawl is *"on all fours"*
and that *"if someone decides to start picking your pockets while you're on
all fours, you'll get incapacitated anyway"* *(tier 3, pcgamesn)*.

So the machine is: **alive → (lethal, eligible hit) → crawling → [looted |
water] → incapacitated → die**, with **alive → (lethal fall) → incapacitated**
as the short road, and a recovery exit out of both wounded states.

### 2.2 What makes a hit eligible

Only what the developer has said: *"If you're shot in the head you'll die
instantly"* (Devblog 53, 2015) and it still holds — a 2021 guide restates
*"Fatal damage will still occur on headshots if you take enough damage
quickly"* *(tier 3)*. Falls are eligible but skip the crawl. The preview says
*"your health drops to zero by most means"* *(tier 3)*. **Nothing reached here
lists the ineligible damage types** beyond the headshot; §7 carries it.

### 2.3 What you can and cannot do down there

- **Crawling:** move slowly; open doors — and since the same update *"you'll
  now need to hold down the interact key to open the door while wounded"*
  *(tier 3 restating the changelist)*; look; talk. *"You can't pick up
  anything or fire weapons while wounded"* and *"you will immediately drop
  whatever item you had selected on the hotbar"* *(tier 3, two guides and the
  fandom wiki agree: the held item drops on entry, as if you had died)*.
- **Incapacitated:** Devblog 53's original: *"knocked to the ground, unable
  to do anything other than look around and shout."*
- **Either:** *"A wounded player's inventory can be freely accessed by all
  players, so they will take even your pants"* *(tier 3, fandom)*. Being
  looted is also the downgrade trigger (§2.1). There is **no give-up button**;
  guides send you to the console's `kill` command *(tier 3)*.
- **You do not bleed.** Devblog 54 (2 April 2015): *"You no longer constantly
  bleed out when wounded, so revival is actually an option."* The state is a
  timer, not a drain.

### 2.4 The timer

*"Players will be in the wounded state for 40 to 50 seconds (instead of 15 to
30) before succumbing to their wounds"* — Devblog 121, 4 August 2016, and the
2021 write-ups restate the same window for crawling: *"about 40 to 50
seconds"* (Rustafied). Guides round it to *"a 45 second timer"*. **The
incapacitated state's own length after a mid-timer downgrade is not
published** (§7).

### 2.5 The roll at the end

Verbatim, from the 2021 update: *"In both states you still have some chance
to recover after the wounded timer runs out. By default right now the base
value is a 20% chance while crawling and a 10% chance while incapacitated. But
you now get up to a 25% bonus on top of that (so max 20 + 25 = 45% total)
based on your food and water levels. Maximum food and water = full bonus."*
Failure is death. As convars, published in the same post:

| convar | default | what it is |
|---|---|---|
| `woundedrecoverchance` | 0.2 | base chance of getting up out of crawling |
| `incapacitatedrecoverchance` | 0.1 | base chance of getting up off your back |
| `woundedmaxfoodandwaterbonus` | 0.25 | the bonus at full food AND full water |
| `crawlingminhealth` / `crawlingmaxhealth` | 30 / 50 | the health you are given on entering the crawl — *"a bit more health than they used to in the old incapacitated state"*, which was **5 HP** *(tier 3, fandom)* |
| `woundingenabled` | true | the whole system, as a switch *(tier 3 for the name; the Hardcore post is the tier-1 evidence it is switchable)* |
| `rewounddelay` | 60 | §2.7 *(tier 3 for the name; Devblog 71 for the rule)* |

**The shape of the bonus between its endpoints is not published.** "Based on
your food and water levels" with "maximum food and water = full bonus" pins
two points; linear in the mean of the two fractions is the obvious reading
and it is *our* reading (§9.4).

**The medkit rule**, same post: *"A medkit in a belt slot gives a 100% chance
at recovery when the recovery timer runs out (you can still get killed while
wounded by taking damage though)"* — with one carve-out in the developer's
own words, *"Fall damage. So you can't dive out of a helicopter with your
trusty medkit for guaranteed survival."* Rustafied adds the consumption rule:
*"the Medkit will be used (unless you were going to recover anyway, in which
case you keep the Medkit)"* — the roll is made first and the medkit only pays
for a failure. The official wiki's item page carries it as a stat line:
*"Guarantees 100% recovery from the wounded state if placed in your toolbar."*

**Since 3 August 2023 the odds are on screen.** *"It will now tell you the
probability of recovery as well as how many seconds are left until
recovery/death"* — the state was opaque for eight years and then it was not.

### 2.6 Being picked up

Three ways, and the developer changed its mind about two of them twice:

1. **Hands.** *"Reviving a player now requires you to hold down E for 6
   seconds without moving"* (Devblog 121). Two rules travel with it: *"revive
   attempts that are interrupted will still increase the time it takes for
   the player to die from his wounds"* — §1's `ProlongWounding` — and
   *"players will never die from their wounds while someone is trying to
   revive them."*
2. **A syringe.** *"Bandages and syringes automatically revive players"*
   (Devblog 57, April 2015) → *"bandages and syringes can no longer be used to
   revive"* (Devblog 121, 2016) → *"other players can now once again jab you
   with a syringe to revive you immediately"* (Rustafied on the 2021 update).
   Syringe: +15 instant, +20 over time; bandage: +5, −50 bleeding; medkit:
   +10 instant, +100 over time, −100 bleeding (official wiki item pages).
3. **A medkit** — yours, in your belt, at the timer's end (§2.5); or a
   friend's, used on you, immediately *(tier 3)*.

### 2.7 Once a minute, at most

Devblog 71 (30 July 2015): *"being wounded again within 60 seconds of getting
up results in death."* The stated reason was raids — endless wound-and-revive
loops between a door and a defender. The rule regressed at least once: the
July 2021 changelist carries, under *Fixed*, *"If players recover from being
wounded, they can't get wounded again within one minute."*

### 2.8 What has been re-tuned since

*"Player crawling health reduced by 75%"* — the September 2021 update, two
months after the crawl shipped; the convars were renamed
`crawlingminimumhealth` / `crawlingmaximumhealth` in the same pass *(tier 3
for the rename)*. And the Hardcore gamemode (26 August 2022) lists *"crawling
when wounded"* under **Removed** — the crawl is a comfort they take away when
they want the game to bite.

---

## 3 · History — the order they built it, and what each step fixed

| date | post | what changed | what it was fixing |
|---|---|---|---|
| 2015-03-26 | Devblog 53, *Death Routine* | wounded instead of dead: on the ground, look and shout, loot-able, help-up-able, "a small chance they might recover", headshots kill outright | dying in one frame to a shot you never saw; group play with no way to save a friend |
| 2015-04-02 | Devblog 54 | no bleed-out while wounded | "revival is actually an option" — the timer had been a race against a drain |
| 2015-04-23 | Devblog 57 | bandages and syringes revive; the wounded sound | the revive had no verb |
| 2015-07-30 | Devblog 71 | wounded again within 60 s of getting up = death; the recover animation | raid revive loops |
| 2016-08-04 | Devblog 121, *Revive Reboot* | hold E 6 s, no moving; interruptions prolong; no death mid-revive; items no longer revive; 40–50 s from 15–30 | an instant item revive had made being down trivial |
| 2021-07-01 | *Wounding Update* | the crawl; 20 % / 10 % + 25 % food-and-water; medkit-in-belt; syringe revive back; hold to open doors; convars | the incapacitated minute was dead time for the person in it |
| 2021-09-02 | *September Update* | crawling health −75 % | crawlers escaping and tanking |
| 2022-08-26 | *Hardcore* | crawl removed in that mode | a mode that wanted the old bite |
| 2023-08-03 | *Wounded ☠️* | probability and seconds on screen | eight years of guessing |

Read as a sequence it says one thing three times: **every revision after the
first was a response to how players used the previous one** — as a revive
loop, as an escape, as free time. The number that never moved is the minute:
40–50 s since 2016, 60 s between downs since 2015.

---

## 4 · How it looks and sounds

**What a source says.** You are *"knocked to the ground"* (Devblog 53); the
crawl is *"on all fours"* *(tier 3)*; there is a *"recover from wounded
animation"* (Devblog 71); your held item leaves your hands *(tier 3, three
sources)*; there is a distinct wounded **sound** (Devblog 57 — the community
clipped it as *"new dying/wounded sound effect"*); the HUD carries *"a
sizeable on-screen text"* and, since 2023, *"two new gauges: Recovery Chance
and Time Remaining"* *(tier 3, BisectHosting)*; and the low-health overlay is
red — *"your screen is soaked in red"* *(tier 3)*, though that is the hurt
effect, not the wounded one. Devblog 53 also describes the *death* camera of
the era: *"for a few seconds you'll continue to see through your ragdoll
corpse eyes."*

**What no source here says, and is written down as memory of play, not as a
fact** — check it against the game before building to it: the first-person
camera drops to the ground with the body (roughly chest height in the crawl;
on your back when incapacitated, looking up and sideways); the view is
darkened at the edges and drained of colour, with a slow blur; you hear your
own breathing. If any of that is wrong the fix is a frame, not a search.
**The operator has the game; a death in it is the source.**

---

## 5 · The failure modes they published

- **The revive loop** (2015): a door, a defender and a friend with a bandage
  made a raid unwinnable. Fixed by a *cooldown on being saved* (§2.7), not by
  touching the revive.
- **The instant item revive** (2015–2016): removed in favour of six held
  seconds; then partly restored in 2021 with the crawl, because a downed
  player who can move is not the same problem as one who cannot. **The revive
  item is a knob they flipped twice**; whichever way we set it, it will be
  argued.
- **The tanking crawler** (2021): 30–50 HP on the ground was enough to crawl
  behind a wall and wait out a fight. Cut by three quarters within two months.
- **Regression of the minute rule** (fixed July 2021): a rule with no gate
  drifts, in their tree as in ours.
- **Opacity** (2015–2023): players could not see their odds or their clock for
  eight years. The 2023 post presents showing them as a feature, and it is
  one: a timer you can see is a decision you can make (crawl for the door, or
  not).
- **Hardcore removed the crawl** rather than tuning it — evidence that the
  crawl is a *mercy* in their own model, not part of the combat loop.

---

## 6 · The verb inventory, complete

For `MENUS.md`'s shape — every verb the state has, ours against theirs in
§9.6:

| verb | who | how |
|---|---|---|
| fall | the sim | lethal eligible hit; held item drops |
| crawl | the downed player | slow move; hold to open a door |
| look, speak | the downed player | both states |
| be looted | anyone | open inventory; downgrades a crawler |
| help up | a second player | hold E 6 s, no moving; prolongs the timer if broken off; suspends death while held |
| jab | a second player | syringe on the body, immediate |
| medkit | the downed player's own belt | 100 % at the timer's end, consumed only on a failed roll; not after a fall |
| give up | the downed player | console only |
| recover | the sim | the roll: 20 % / 10 % + up to 25 % |
| die | the sim | failed roll; any damage; water; a second down inside 60 s |

---

## 7 · What could not be sourced, and what it blocks

- **Crawl speed.** Every source says "slowly". Ours is a knob (§9.4).
- **The incapacitated timer after a downgrade** — whether being looted resets
  the 40–50 s, continues it, or starts a shorter one. Ours is a knob.
- **The bonus curve's interior** (§2.5). Ours is linear in the mean.
- **Health on recovery.** Not published; Rustafied's "a little higher" is
  about the crawl, not the stand-up.
- **Which damage types are ineligible** beyond the headshot — explosion, fire,
  drowning, cold, hunger. Ours: a starving player dies, a shot one falls;
  §9.4 says why.
- **The screen** (§4). Blocks nothing in the sim; blocks a *confident* client
  slice, which is why §9.5 lands the arithmetic and leaves the look to the
  person who boots the game.

---

## 8 · Sources

Tier 1 — the developer, fetched whole 2026-09-13:
- Devblog 53, 26 March 2015 — `rust.facepunch.com/news/devblog-53` (*Death Routine*)
- Devblog 54, 2 April 2015 — `/news/devblog-54`
- Devblog 57, 23 April 2015 — `/news/devblog-57`
- Devblog 71, 30 July 2015 — `/news/devblog-71`
- Devblog 121, 4 August 2016 — `/news/devblog-121` (*Revive Reboot*)
- *Wounding Update & Voice Props DLC*, 1 July 2021 — `/news/wounding-and-voice-props`; changelist `/changelist/3907`
- *September Update*, 2 September 2021 — `/news/september-update-2021`
- *Hardcore Gamemode*, 26 August 2022 — `/news/hardcore`
- *Wounded ☠️*, 3 August 2023 — `/news/wounded`; changelist `/changelist/3954`
- `wiki.facepunch.com/rust/item/syringe.medical`, `/item/bandage`, `/item/largemedkit`

Tier 2 — `reference/rust-systems.txt` (MIT, Oxide.Rust's `Rust.opj`), re-ripped
2026-09-13 and unchanged on these lines.

Tier 3 — Rustafied, *Crawling, Voice Props, and DLSS*, 24 June 2021, and
*Update time!*, 2 September 2021; PCGamesN, *Rust due to get new wounded and
recovery mechanics next update*, June 2021; `rust.fandom.com` *Status
Effects* and *Mechanics* (wikitext via `api.php`); `gtxgaming.co.uk`,
`rust420.com` and BisectHosting guides (undated); `guided.news`, 21 June
2021; `bmgjet/Downed` (GitHub, plugin source); one `oxidemod.org` thread,
2015, for the flag name only.

---

## 9 · What it means for us

Written against the tree on 2026-09-13, the day the slice landed, so every
claim below has a file behind it and the file is the claim.

### 9.1 What is built: a body falls down instead of dying

**`crates/sim-core/src/wound.rs`** is the arithmetic and nothing else — the
window, the odds, the crawl, as pure integer functions over
`rng::cell_hash`, so the die is a function of `(seed, id, tick)` and a
replay rolls it the same way (wall 5). **`world.rs`** decides:

- `World::down_or_die` sits between the funnel and the corpse. Every one of
  the eight kill sites that used to call `die` on the funnel's `died` now
  calls it, and it asks the reference's question (`wound::wounds` — a
  predicate on the *hit*, §1): a swing, a bite or a body shot lays the body
  down; a headshot, a blast, the clock and the sea make the corpse they
  always made. Three more things send any blow to `die`: a body already
  down, a sleeper, and a body inside its minute.
- The fall writes `Player::wounded`, `wound_until = tick + span`, `hp =
  WOUNDED_HP`, and the four facts the death screen is made of
  (`death_by/cause/item/range_cm`) — early, because a failed roll is that
  death and `die` needs them then. It pushes `EV_WOUNDED` (the clock in
  ticks and the odds per mille) and an `EV_HEALTH` so the bar ends at the
  crawl's hp and not at the zero the emit site announced.
- `World::wound_tick` runs in the live branch *and* the sleeping branch: at
  `wound_until` it re-reads the meters, rolls, and either stands the body up
  (`EV_RECOVERED`, `rewound_until = tick + REWOUND_TICKS`) or calls `die`
  with the downing blow's four facts, so the kill feed credits the hand that
  put the body down a minute earlier — the reference's rule.
- A downed body's frame is stepped through `wound::crawl_frame` (no sprint,
  no jump, no swing, axes ÷ `CRAWL_DIV`), and the client's `Predictor`
  steps and replays the *same* frames through the *same* function once
  `EventMsg::Wounded` has arrived (`Predictor::crawl`), which is the
  quantize-both-sides law applied to a gait.
- `live_slot_of` refuses a downed body every verb that spends a hand;
  `awake_slot_of` keeps it the door (`Command::Use`), which is the
  reference's short list (§2.3) minus the hold.
- **`EV_DEATH` moved into `World::die`, and so did the death count.** Until
  this slice every kill site pushed the broadcast itself and
  `combat::debit` counted the death — right while a lethal debit and a
  corpse were one event. They are not any more, and a feed that announced
  every lethal blow would credit a kill to a raider whose victim got up.
  `die` says it once, before the bag drop, because the client's own-bag
  latch is armed by `Death` and spent by the next `BagDropped`.

**The wire** (`PROTO_VER` 62 → 63): `EntityState::wounded`, one
unconditional bit after `dead`, for `dead`'s reasons (a state a body
entering AOI has to be able to learn; flips too rarely for a change flag);
`SUB_WOUNDED` and `SUB_RECOVERED`, own-fact, unicast. The server sends a
downed body's hand as empty and its torch as out (§2.3's dropped item). **The
saves** (`SAVE_FORMAT` 5 → 6, `WORLD_SAVE_FORMAT` 12 → 13) carry the bit and
its two clocks, because `state_hash` folds them and the combat storm's
save/load round trip went red the hour they were left out — logging off or
a restart is not a way to dodge the roll.

**The client**: the camera eases from `EYE_HEIGHT` to `CRAWL_EYE_M` and the
horizon rolls `WOUND_ROLL_RAD` with the same fraction; a radial vignette and
the line the reference added in 2023 (`WOUNDED  41 s  33% to get up`),
counted down at the tick rate from the event; a toast either side; the fall
plays `Cue::Death` (the reference shipped one wounded sound, Devblog 57); a
remote body plays `Death01` and holds it. `input::gather` strips the three
bits the sim strips, so the viewmodel does not swing an arm the sim ignores.

**Gates**: `sim-core/tests/wounded.rs` (the fall, the finish, both ends of
the roll, the minute, the crawl's ratio, the refused verbs, the sleeper, the
hash, the knobs), two role checks in `event_roles.rs`, the `damage_routes`
row that classifies the fall's hp write as a gain after a loss, the persist
field ledger, the byte golden of the save head, and the replay golden
regenerated in the same commit with its reason written beside it.

### 9.2 What was taken from theirs, and what was not

Taken as `BALANCE.md` §6 says to — theirs, no case needed: **40–50 s**
(Devblog 121, restated 2021), **20 %** base (`woundedrecoverchance`), **+25 %
at full food and water** (`woundedmaxfoodandwaterbonus`), **60 s** between
downs (Devblog 71, `rewounddelay`), the door as the one verb kept, the held
item dropped, the headshot as the outright death, the sleeper as
ineligible, the kill credited to the downing blow.

Ours, each with its reason written at the constant:

- **`WOUNDED_HP = 10`** — theirs shipped the crawl at 30–50 and cut it by
  three quarters two months later (§2.8); ours is the midpoint of the cut
  band. The consequence is the one they wanted: nearly any second blow
  finishes a crawl.
- **A blast kills.** Not published (§7); a body inside a satchel's radius
  that stood up 45 s later would make the most expensive damage in the game
  the weakest way to kill a defender.
- **A melee blow to the head wounds.** Their `BaseMelee → eligible` (§1);
  the headshot rule is a projectile's.
- **The bonus is linear in the mean of the two meters** between the two
  published endpoints (§2.5).
- **The crawl is a third of the walk** (`CRAWL_DIV = 3`, ~1.0 m/s); theirs
  is "considerably slower than walking" and no number.
- **The odds are re-read at the roll**, so the number `EV_WOUNDED` carried
  at the fall can drift a point over the minute as the meters drain. The
  reference re-evaluates too (a guide says every five seconds, tier 3);
  ours does not resend. Say so on the screen or resend it — `NOW.md` §0wnd.

### 9.3 One state, not two

The reference's second, incapacitated state (§2.1) is reached by three
triggers — being looted while crawling, a fall, deep water — and this game
has none of them: a live body cannot be looted (`Command::Loot` empties
bags), there is no fall damage (`DEATH_BY_*` has no fall), and wading is a
speed multiplier rather than a depth. So the downgrade has nothing to fire
it, and building the state first would be building a room with no door. It
is additive when a trigger exists: a second flag, `StartWoundedTick(10,
25)`'s shape on the clock, `incapacitatedrecoverchance 0.1` on the roll.

### 9.4 The knobs

One row, `DECISIONS.md` §open "wounded v0", every constant in it pinned by
`ci/knob_registry.mjs`: `WOUND_MIN_TICKS`, `WOUND_MAX_TICKS`,
`RECOVER_BASE_PM`, `RECOVER_BONUS_MAX_PM`, `WOUNDED_HP`, `REWOUND_TICKS`,
`CRAWL_DIV` in the sim; `CRAWL_EYE_M`, `WOUND_ROLL_RAD`, `WOUND_DROP_S`,
`WOUND_RIM_ALPHA`, `WOUND_CLEAR_PCT` on the client; `EV_WOUNDED`,
`EV_RECOVERED`, `SUB_WOUNDED`, `SUB_RECOVERED`, `CHANCE_PM_BITS` on the wire.
The window is held to the tick rate by a test, so a changed `TICK_HZ`
cannot shorten the minute in silence.

### 9.5 The look is the one thing here nobody has seen

§4 is honest about it: no page describes the reference's wounded camera,
and the five client knobs are a first cut. What to check when booting the
game, in order: that the drop reads as a fall and not as a crouch (the
ease, then the height); that the roll does not make a door hard to keep in
frame while crawling to it; that the vignette darkens the edges without
hiding the person standing over you; that the line is legible over sky and
over turf; and that a remote body's fallen pose sliding at a metre a second
reads as a crawl and not as lag. There is no pixel gate and there will not
be one (`CLAUDE.md`); `NOW.md` §LOOK is where this joins the list.

### 9.6 Staging — what the minute still cannot do

In the order they earn their keep, each a slice of its own, all in
`NOW.md` §0wnd:

1. **Hands.** `RPC_Assist`'s shape: a verb aimed at a downed body, held six
   seconds without moving, that prolongs the clock when broken off and
   suspends the roll while held (§2.6). A new action on the wire, a hold
   counter on the target, a `Verb` in the pick — and the feature-gated
   match trap `CLAUDE.md` lists twice.
2. **A syringe.** The medical tool's ordinary apply-to-target path with the
   target down (§1) — ours is `Command::Consume` with a target, and
   `content/consumables.toml` already prices a bandage and a medkit.
3. **The medkit in the belt**, consumed only on a failed roll (§2.5).
   The sim is built (2026-09-20): `SurvivalContent::belt_recovery` opts items
   in, and `wound_tick` spends the first qualifying belt stack after a failed
   natural roll. Hands and a natural recovery keep it; a finishing blow
   still kills. `content/consumables.toml` arms the medkit's `belt_recovery`
   flag, validated and included in the content hash. Fall damage has
   no wounded entry path here, so its exemption needs no new predicate.
4. **A voice of its own for the fall** (`synth.rs` row, bank entry,
   `assets/sound/WANTED.md`), and **a drag clip** so the crawl stops sliding.
5. **Refusals while down are silent.** `live_slot_of` refuses without an
   event, so a downed player pressing craft hears nothing. A `REFUSE_*`
   reason on each refused verb's own event is the honest fix.
6. **Give up.** Theirs has none (console only). A `Respawn` from a crawl
   would be a design call, not a default.

### 9.7 What the slice cost, so the next one budgets for it

Every ledger in the tree that could see the change did, and in the same
hour: `damage_routes.rs` (an hp write outside the funnel), `event_roles.rs`
(two codes past `EV_MAX`), `persist.rs`'s field ledger (three fields on
`Player`), `event.rs`'s wire-domain tables (a new module, a new width), the
baseline-index bit count, the world-size note in `.cargo/config.toml`, the
replay golden, and the combat storm's save/load round trip. None of them
was a false alarm. The one that would have shipped a bug silently was the
last: a world save that forgot the crawl restored a world that hashed
differently, and only a storm that saves mid-fight could see it.
