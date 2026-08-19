# reference/ARMOR.md — how the reference game dresses a player

Ripped facts, not design. `DURABILITY.md` answers *how an item wears out*,
`DOORS.md` *who is allowed through*, `PROJECTILES.md` *what a bow is*; this
file answers **what a player wears and what it does for them**, because
`content/armor.toml` has been sitting in this tree fully priced, validated,
slot-checked, hashed into the content hash and bounded by a balance anchor —
and *paying nobody*. `sim-core/src/combat.rs:37` says so in as many words:
"no armor reduction". The operator asked for equipment on 2026-08-16.

Dated 2026-08-16. §9 is the part that changes what we build.

## 0 · Provenance — read this first

Three tiers, and they are very far from equal here. Ranked:

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so what it proves is the **shape**: which classes exist and
   which methods they hang hooks on. §1 reads the object model off it and
   does nothing more. It is the only tier with no caveat, and on this
   subject it happens to answer the one question that actually decides our
   architecture — which is why §1 leads and why §9.2 is the strongest
   paragraph in the file.
2. **The developer's own devblogs**, by number. `AUDIO.md` §0's posture: a
   sentence a developer published about their own work.
3. **Community wikis and guides** for numbers. Weakest tier, and weaker on
   this subject than on most: armor has been reworked at least three times
   (Devblog 104, a correction after it, the 2025 Crafting Update's inserts,
   the 2026 "Built Different" set), so a guide's numbers are a snapshot of
   an unstated patch. Read them as *shapes that held*, never as values.

**The proxy caveat, in full, because it is `DOORS.md`'s and not
`DURABILITY.md`'s.** Every page fetch attempted from this container was
refused by the egress proxy: `wiki.facepunch.com`, `rust.fandom.com`,
`rustafied.com`, `corrosionhour.com`, `umod.org`, `rust-survival.com`. Tiers
2 and 3 below therefore arrive as **search-result summaries of those pages,
not the pages themselves**, which is a real weakening — a summary drops
qualifiers, and on a decade-old system the qualifier is usually *which
patch*. `DURABILITY.md`'s open box was a different day and a different
container; `SOURCES.md` §0's instruction stands, which is to probe rather
than to trust either measurement.

Nothing here was decompiled. Nothing here ships: no asset, no name, no
number copied into `content/` without being re-priced against our economy.

## 1 · The object model, read off the hook table

`PlayerInventory` carries, among its eleven:

```
CanEquipItem        CanEquipItem(Item,Int32)
CanWearItem         CanWearItem(Item,Int32)
CanMoveItem         MoveItem(BaseEntity/RPCMessage)
OnClothingItemChanged   OnClothingChanged(Item,Boolean)
OnInventoryNetworkUpdate
    SendUpdatedInventoryInternal(PlayerInventory/Type,ItemContainer,…)
```

and `Item` carries `MoveToContainer(ItemContainer,Int32,…)` and
`IOnLoseCondition → LoseCondition(Single)`.

**Four things this proves, and the first is the whole file.**

1. **Wearing is a container, and equipping is a move into it.** The wear
   check is `CanWearItem(Item, Int32)` — an item and a **slot index** — and
   it sits directly beside `CanEquipItem(Item, Int32)` on the same class,
   with `MoveItem(RPCMessage)` the one verb underneath both. There is no
   `EquipItem` RPC anywhere in the table. A player puts a helmet on by
   *moving a stack into a slot*, and the game answers by refusing the move.
2. **Wear and hand-equip are two containers, not two systems.** Two
   predicates of identical signature on one class is a shape, and the shape
   is one mover with a per-container predicate.
3. **Containers sync individually.** `SendUpdatedInventoryInternal` takes an
   `ItemContainer` and a mode — the wear container is a thing that is
   networked on its own, not a field of the player record.
4. **Condition lives on the item, not on the wearer.** `LoseCondition(Single)`
   is `Item`'s, which is `DURABILITY.md` §1's finding arriving from the
   other side: a worn helmet wearing out is the same mechanism as a hatchet
   wearing out, reached through the same method.

## 2 · The slot grid, and layering as a conflict rule

*(tier 3 — the current shape, undated)*

Head, chest and legs each carry **two layers**: a clothing layer worn
underneath and an armor layer over it. Hands and feet accept **one** item
each. Some items occupy **both** layers of an area — the metal facemask is
the named example — and a full-body suit occupies everything.

The two layers are not cosmetic tiers. The under layer is described as
carrying comfort and cold, the over layer projectile and melee. So the grid
encodes a *kind* of protection by position, which is one way to get variety
without a stat per item.

**Layering is expressed as a conflict, not as an ordering.** An item that
occupies both layers does not stack with, or replace, the thing under it —
it makes the second slot unwearable. That is `CanWearItem(item, slot)`
returning false, and it is why the check takes a slot index rather than just
an item.

## 3 · Protection is a vector, and it is per covered area

*(tier 3)*

Every wearable declares a **separate resistance per damage type**. The types
named across sources: bullet, slash, blunt, stab, explosion, bite,
radiation, cold, heat, electric, falling. A hazmat suit and a metal
facemask are the same mechanism with different vectors, which is the whole
reason the vector exists — one scalar cannot make a suit a radiation answer
and not a bullet answer.

**Radiation is subtractive where damage is proportional**, and this is worth
its own line because it breaks the "everything is a percentage" reading: a
radiation zone is rated in RadPerSec (Minor/Low/Medium/High reported as
2/10/25/45), and a garment's radiation value is **subtracted** from that
rate. Damage-type protection is a reduction of the hit; radiation protection
is a reduction of the *rate*. Two different arithmetics under one word.

**Armor only protects what it covers.** A hit to an uncovered area takes
full damage — so the grid in §2 is not bookkeeping, it is the mechanic:
coverage *is* protection, and the reason a helmet matters is that heads get
hit, not that helmets have big numbers.

How several pieces over one area combine is the weakest claim in this file.
Guides describe the reduction as multiplicative on the covered zone rather
than a sum of percentages; no primary source was reachable to confirm it,
and §7 keeps it open.

## 4 · What it costs to wear

*(tier 3)*

The heavy plate set is the designed extreme and the numbers are consistent
across guides: a **40% movement penalty**, a locked/restricted view, no
proper aiming down sights, and a cold penalty on the jacket. It is described
everywhere as raid-defence gear you never roam in.

**The penalty does not stack** — any single heavy piece applies the whole
40%, and wearing three does not make it 120%. That is a deliberate
non-linearity and it is the interesting half: the cost is charged for
*entering the category*, not per item, so the player's decision is binary
and legible rather than a spreadsheet.

## 5 · Armor is hitpoints, and broken is not gone

*(tier 2 — Devblog 104 and the correction after it)*

Devblog 104's armor changes are described by the developer's own later post
as having had problems, and a subsequent pass made armor "closer to how it
should work". What that pass added, for armor worn over clothing:

> a condition value that works like hitpoints — when armor protects someone
> against damage, that damage is absorbed into the armor and reduces its
> condition by that damage value. When the armor reaches 0 it becomes broken
> and only provides **25% of its original protection**.

Two mechanisms in one paragraph and both are worth taking. The condition
cost is **the damage absorbed**, not a flat per-hit tick — so a piece that
saved you from a rocket is nearly spent and one that turned a rock is barely
touched, with no table needed to say so. And **broken is 25%, not zero**: a
binary break makes armor a cliff, and a cliff decides a fight before it
starts.

## 6 · What wears it: player damage only

*(tier 3, but consistent across sources including the plugin ecosystem that
exists to change it)*

Only damage **from players** decreases worn-item condition. NPC and
environmental damage does not, which is why third-party plugins advertising
"items lose durability on NPC hit" exist at all — the market for them is the
proof of the default.

## 7 · What could not be sourced, and what it blocks

- **The per-type protection values.** Every number in tier 3 is an undated
  snapshot across at least three reworks. Blocks nothing we are doing:
  `BALANCE.md` §6 wants their numbers, and these are not reliably theirs.
- **Whether stacking within a zone is additive, multiplicative, or
  highest-wins.** Guides say multiplicative; no primary source was
  reachable. Blocks §9.3's arithmetic — and note that with our two slots and
  one piece each, the question does not arise until a second layer does.
- **The absorbed-damage-to-condition ratio** beyond "that damage value" —
  whether it is 1:1 after reduction or before it. Blocks the exact wear rate
  and nothing structural.
- **The current slot count** after the 2026 "Built Different" set. Irrelevant
  to us: we would not take a five-area grid on a two-slot content table.

## 9 · What it means for us

### 9.1 We have every number and none of the system

Measured on 2026-08-16, and it is worth stating plainly because it looks
built from the content side:

| exists | where |
|---|---|
| three armor rows, priced | `content/armor.toml` |
| `ArmorSlot {Head, Body}`, `Armor {reduction_pct, move_penalty_pct}` | `content/src/schema.rs:317` |
| `EquipSlot {Hand, Head, Body, None}` on every item | `content/src/schema.rs:26` |
| item-backed + slot-agreement + ≤90% validation | `content/src/validate.rs:507` |
| folded into the content hash | `content/src/canon.rs:170` |
| TTK anchor bounding each piece to `armor_extra_hits_max` | `content/src/balance.rs:117` |

| does not exist | evidence |
|---|---|
| ~~any sim reader of any of it~~ | **closed 2026-08-19** — see below |
| a wear container, a wear slot, an equip verb | ~~nothing~~ · a wear *slot* exists (`Player::worn`); the container and the verb do not |
| a wire field | nothing |
| a client surface | nothing |

`combat.rs:37` lists "no armor reduction" among the things melee v0
deliberately does not do. So this is `NOW.md`'s own recurring shape — *fully
built and gated and paid nobody* — and the balance anchor is the sharp end:
`balance.rs` is today asserting a TTK relationship for armor that cannot
affect a single hit.

⚠ **The first row and that last paragraph are HISTORY as of 2026-08-19, and
the distinction matters because only one half closed.** `bake_combat` bakes
an `ArmorDef` per item and `combat::hurt` reads it, so the shipped burlap
shirt turns a rock's five hits into six and the audit above is answered:
`grep -rn armor crates/sim-core/src` is no longer one comment. Two things
did **not** move and are the live half of this section. **Nothing can put
armor on** — §9.2's container move is unbuilt, so `Player::worn` is written
only by a save and by tests, and reachability waits on the wire bump §9.2
prices. **And the balance anchor is still asserting a relationship it cannot
see**, one level in rather than not at all: it is slot-blind, it is a
ceiling with no floor, and it cannot see a *set*, which is exactly the
sharp end this paragraph named — `DECISIONS.md` §open "armor reduction v0"
carries the +3-against-a-band-of-2 arithmetic and why re-speaking it is an
operator act. Read §9.3 next with one correction: **the ordering it
recommends was deliberately not taken** (one scalar shipped, types deferred),
on the ground that `reference/RIPLIST.md` §1h found their Projectile and
Melee cells equal on all three pieces we own.

### 9.2 Equip is a container move — do not add an equip verb

**The single most useful thing in this file.** The reference has no
`EquipItem` RPC: wearing is `MoveItem` into a wear container, refused by
`CanWearItem(item, slot)` (§1). We should take that exactly, and the reason
is not fidelity — it is that **we have already paid for that verb**.

`CLAUDE.md`'s trap list names the item-move verb as the most bug-prone thing
in the reference, and `ui/slots.rs` and `sim-core/inventory.rs` are the two
files in this tree written most carefully against it: a six-step refusal
ladder checked *before* anything is marshalled, `MoveArgs` with named fields
so a transposition cannot compile, per-kind slot widths via `slots_in`, and
role-gated event payloads. A new `ACT_EQUIP` would be a **second** path into
container mutation, with none of that, guarding the exact state whose
divergence presents as the player being disconnected.

As `CONT_WEAR`, the whole thing falls out:

- `slots_in(CONT_WEAR)` gives the grid its width, and the existing check
  that stops a drop on box slot 20 stops a helmet in the boots slot.
- `CanWearItem`'s job — *this item does not go in that slot* — is one new
  refusal reason beside `REFUSE_M_SLOT`, read off the item's existing
  `EquipSlot`, which content already carries and validates.
- The client's drag already addresses containers by kind. The panel we just
  built draws a wear grid by passing a different `kind`.

**It costs a wire widening, and that is wall 6.** `CONT_KIND_BITS = 2`
(`protocol/src/event.rs:279`) and `CONT_MAX = CONT_WORLD = 3` — all four
values of the field are spent, which `sim-core/inventory.rs` already says
out loud ("there is no forgeable kind left"). `CONT_WEAR = 4` needs three
bits, which is a packet layout change: `PROTO_VER` bump plus regenerated
goldens **in the same commit**. Cheap to plan, expensive to discover
halfway through.

### 9.3 One scalar where the reference has a vector — and it hardens with every row

`Armor.reduction_pct` is a single number. The reference's is a vector keyed
by damage type (§3), and that is load-bearing rather than decorative: it is
the difference between a hazmat suit and a helmet, and it is why one number
is never right for both a knife and a rocket. Radiation is worse than a
missing column — it is *subtractive on a rate* (§3), so a percentage field
is the wrong shape for it twice over.

This is `PROJECTILES.md` §9.3's argument arriving on a different table, with
the same warning attached: **it gets harder every armor row we add first**,
because a schema change costs one rewrite per existing row and every row is
a number somebody has to re-source.

**And it cannot land yet, for a mechanism reason and not an effort one** —
which matters, because `BALANCE.md` §6.2 names effort as a cost wearing
principle's clothes and would refuse the effort argument outright.
`content/weapons.toml` has **no damage-type column at all**: `Weapon` carries
`kind` (melee/bow/firearm/throwable), `damage`, `structure`, `headshot_mult`.
A protection vector has nothing to key against until weapons carry a type,
so this is two schema moves in order, and the armor half is second. Write
the ordering into `DECISIONS.md` §open rather than discovering it.

### 9.4 Armor condition is free — `ItemStack` already carries it

Item durability v0 landed 2026-08-15 and `ItemStack` is `{item, count,
cond}` (`sim-core/src/gather.rs:461`). So §5's model costs **no new state**:
an arm in the damage path that debits `cond` on the worn stack by the damage
it absorbed, and a `cond == 0` branch that returns a quarter of the
reduction instead of all of it.

Take the 25% floor. It is the half a first implementation always gets wrong
— binary break is the obvious code and it is the cliff §5 describes — and it
costs one multiply.

Take §6's rule too (**player damage only wears armor**): free, and it
protects the thing the destination gradient depends on, since a player who
farms in armor should not be paying a repair bill for the trees.

### 9.5 What must be spoken before any of it lands

`DECISIONS.md` §open, and none of these may be invented in code:

1. **The slot grid.** Two areas (our `ArmorSlot {Head, Body}`) or the
   reference's five areas × two layers? Recommend ours, unchanged: the
   layering in §2 exists to carry a distinction (comfort vs projectile) that
   our sim has no temperature to express.
2. **Whether damage types land now** — §9.3's ordering, and the answer
   decides whether armor v0 is one scalar or waits.
3. **The broken floor** — 25%, theirs, or a spoken number of ours.
4. **The movement penalty's non-stacking rule** (§4). `move_penalty_pct` is
   already a content column and already zero on two of three rows, so this
   is a rule about *combining*, which is exactly the kind of thing a content
   file cannot state and a knob must.

### 9.6 Staging — the first slice is small and it is felt

`CONT_WEAR` + the two slots content already declares + `reduction_pct`
applied on the covered area in `combat.rs`, with the damage-type vector
deferred **by name** (§9.3) rather than silently. That is one wire version,
one refusal reason, one arm in the damage path, and one grid in the panel —
and it turns `balance.rs`'s anchor 2 from an assertion about nothing into an
assertion about the game.

Condition-on-armor (§9.4) rides after it, not with it: it needs the
reduction path to exist before it has anything to debit.

## Sources

Tier 1 is `reference/rust-systems.txt` in this tree. Tiers 2–3 reached as
search summaries only (§0):

- <https://wiki.facepunch.com/rust/Clothing_Slots> — the slot grid and layering
- <https://rust.facepunch.com/news/devblog-104>, <https://rust.facepunch.com/news/devblog-135> — the armor rework and the condition-as-hitpoints model
- <https://rust.fandom.com/wiki/Damage_Types>, <https://rust.fandom.com/wiki/Armor> — the damage-type list
- <https://www.rustafied.com/rust-damage-armor-and-you-guide>, <https://rust-survival.com/23_Combat_Math> — combat math and stacking
- <https://rustly.com/guides/rust-armor-guide/>, <https://xgamingserver.com/blog/rust-armor-clothing-guide/> — protection values, heavy plate penalties
- <https://rustly.com/guides/rust-radiation-guide/> — RadPerSec zones and subtractive rad protection
- <https://www.corrosionhour.com/the-rust-armor-weapon-inserts-guide-what-you-need-to-know/> — the 2025 insert system
