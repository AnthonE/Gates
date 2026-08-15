# reference/DURABILITY.md — how the reference game wears an item out

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`DOORS.md` *who may pass*, `SAVES.md` *what survives a restart*,
`PROJECTILES.md` *what an arrow is*; this file answers **what it costs to
use a thing**, because we have no answer at all — `grep -rni condition
crates/sim-core/src/` returns building repair and nothing else, and the
operator asked for durability after a playtest (2026-08-15).

Dated 2026-08-15. §9 is the part that changes what we build.

## 0 · Provenance — and this one is stronger than its neighbours

Three tiers, ranked:

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so what it proves is the **shape**: which classes exist and
   which methods carry hooks. §1 reads the object model off it and does
   nothing more.
2. **`wiki.facepunch.com/rust/item/<slug>` — fetched whole on 2026-08-15,
   five pages** (`rock`, `torch`, `stonehatchet`, `hatchet`,
   `box.repair.bench`). The official wiki's own stat tables, read off the
   pages themselves. **This is the first `reference/*.md` whose numbers are
   not search summaries**, so the caveat every neighbour carries in its §0
   does not apply to §2 and §3 here.
3. **Community wikis and guides** for the prose rules — what happens at zero
   condition, the blueprint gate. Weakest tier, and §3 records one place
   where tier 3 **contradicts tier 2 and is probably wrong**.

⚠ **Two corrections to `SOURCES.md`'s measured map, both from this pass.**
Its table says `wiki.facepunch.com` "serves prose, but carries **no** yield
tables — the numbers are not there to take". That is false for
`/rust/item/<slug>`: those pages carry a **Gather Rates** table with a
per-resource **Condition Loss** column, plus Craft and Repair tables with
costs. It is where every number in §2 and §3 comes from, and `RIPLIST.md`
§4.1a took per-tool yields off `rusthelp.com` believing this host had none.
Second: `wiki.rustclash.com` still answers **403** (bot wall, unchanged
since 2026-08-09). Probed the same hour; `SOURCES.md` §0's standing
instruction is unchanged — this is a dated measurement, not a new standing
fact.

Nothing decompiled. Nothing here reaches `content/` without being re-priced
against our own economy (`CONTENT.md` §4's bands, `BALANCE.md` §6's ladder).

## 1 · The object model, read off the hook table

```
Item           [11]  CanStack(Item) · LoseCondition(Single) · UseItem(Int32)
                     MaxStackable() · SplitItem · MoveToContainer · Drop · …
RepairBench     [2]  RepairAnItem(Item,BasePlayer,BaseEntity,Single,Boolean)
ItemModRepair   [1]  OnItemRefill → ServerCommand(Item,String,BasePlayer)
```

Five structural facts fall out, and they are the valuable half of this file:

1. **Condition is instance state on `Item`, not on the definition.**
   `LoseCondition` is an instance method, so two hatchets in one inventory
   hold two different conditions. Whatever we build has to be per-stack
   state, not a per-item-id table.
2. **It is a `Single` — a float, decided by the caller.** Not an integer
   count of uses and not a constant the item owns. The *amount* comes from
   whatever is doing the wearing, which is exactly what §2 measures.
3. **The repair bench is an entity that takes `(Item, BasePlayer, …)`.**
   Repair is a **place you go**, gated on what you carry *and who you are* —
   the `BasePlayer` argument is what makes the blueprint check in §3
   possible at all. It is not an inventory verb.
4. **`ItemModRepair` is a separate module from the bench.** The repair-kit /
   refill path and the bench path are two systems, not one with a flag.
5. **Condition-bearing items do not stack.** `CanStack(Item)` exists, and
   every condition-bearing item measured here declares `Stack Size: 1`.
   **This is the load-bearing fact for us** — see §9.2.

## 2 · What wear costs, and it is keyed by (tool, resource)

Verbatim from the item pages (tier 2). "Bonus" is their extra-yield column;
kept for context because it moves with the same key.

| tool | resource | gather damage | bonus | **condition loss** |
|---|---|---|---|---|
| Rock | tree | 10 | 1 | 0.3 |
| Rock | ore | 5 | 1 | 0.3 |
| Rock | flesh | 10 | 1 | *none listed* |
| Stone hatchet | tree | 20 | 2 | 0.3 |
| Stone hatchet | flesh | 10 | 1 | 0.3 |
| Hatchet (metal) | tree | 30 | 3 | 0.3 |
| Hatchet (metal) | flesh | 12 | 1.2 | **1.0** |

Two facts, and the first is the design:

- **Condition loss is per (tool, resource), not per tool.** The metal
  hatchet pays 0.3 on a tree and **1.0 on flesh** — 3.3× — while the stone
  hatchet pays 0.3 on both. One tool, two rates, chosen by what it is
  swung at.
- **The wrong-job penalty is data, not a special case.** Community sources
  state the general rule (a pickaxe on a tree wears faster *and* gathers
  less); the metal hatchet's flesh row is that rule visible in the shipped
  table. There is no separate "is this the right tool" predicate to port —
  the table *is* the predicate.

## 3 · The repair bench

**Cost per repair — and tier 2 contradicts tier 3 here.** Both item pages
give craft and repair costs, and both reduce to the same ratio:

| item | craft | full repair | ratio |
|---|---|---|---|
| Stone hatchet | 200 wood + 100 stone | 40 wood + 20 stone | **0.20** |
| Hatchet (metal) | 100 wood + 75 frags | 20 wood + 15 frags | **0.20** |

Every guide consulted says repair costs **half** the craft cost. The wiki's
own tables say **a fifth**, on two items, on all four rows, exactly. Recorded
as **DISPUTED, primary preferred** (`SOURCES.md`: record both, never
average). Two readings that would explain the gap and neither is settled
here: the guides may be quoting the Legacy bench, or the wiki figure may be
the cost of a *full* repair from zero while the in-game price scales with
damage taken. **Do not take 0.20 into `content/` until one of them is
checked** — it is the number a whole tool economy hangs off.

**The permanent cost.** Both pages carry a `Condition Loss (%)` column in
the repair table and both read **20%**. Every repair removes a fifth of the
item's *maximum* condition, forever. So a tool has a finite number of lives,
not infinite repairs, and the ladder is the point: repair, repair, repair,
throw it away.

**The blueprint gate** (tier 3). Tier-1 items repair without the blueprint;
tier 2 and 3 require it, and the bench refuses with *"You don't have this
item's blueprint"*. That is §1's `BasePlayer` argument doing its job — and
it is a verb we already have the state for (`Player::known`, the blueprint
mask, `sim-core/research.rs`).

**At zero** (tier 3). A broken item **stays in the inventory** at 0 and
remains repairable. It does not vanish, and that is what makes the −20%
ladder legible: you keep holding the thing that is dying.

## 4 · What has no condition at all — and this is the starter-kit rule

| item | repairable | condition stat | craft |
|---|---|---|---|
| Rock | **False** | none listed | 10 stone |
| Torch | **False** | none listed | 30 wood + 1 cloth + 1 low-grade fuel |

**Both starting items sit outside the wear economy.** Whatever else breaks,
the two things a naked spawn holds cannot be repaired and are not shown to
carry condition — and the rock re-crafts for 10 stone, which is under one
swing of a stone node. A fresh spawn can always bootstrap. That is a
property worth copying deliberately rather than arriving at.

⚠ **One unresolved contradiction, stated rather than smoothed.** The rock's
page shows `Condition Loss 0.3` in its *gather* rows while listing
`Repairability: False` and no maximum. Two readings: (a) the rock does wear
out and is simply thrown away rather than repaired — coherent, given it
costs 10 stone; or (b) the column is inherited from the shared gather schema
and is inert for an item whose condition is disabled. Nothing available at
tier 2 or 3 separates them. Settling it needs the `ItemDefinition.condition`
flag or an in-game test, and §9.3 carries it as an open question rather than
a guess.

## 5 · What could not be sourced, and what it blocks

- **The maximum condition of any item.** Not on the Facepunch item pages,
  not on `rusthelp.com`'s, and `wiki.rustclash.com` 403s. So `0.3` is a rate
  with **no denominator**: hits-per-life = `max ÷ loss` is unknown, and it
  is the only number that decides whether durability is felt once an hour or
  once a week. Everything in §9.4 is staged around not having it.
- Whether a **weapon** loses condition per shot, and at what rate.
- Whether **armour** loses condition when hit.
- Whether the bench's price scales with damage taken (§3's open reading).

## 9 · What it means for us

### 9.1 The schema already has the right shape

`content/gatherables.toml` keys `yield_per_hit` by tool, per node. §2's
condition loss keys **identically**, so it is a sibling table and not a new
concept:

```toml
[gatherable.yield_per_hit]      # exists today
"item.hatchet_stone" = 81

[gatherable.condition_loss]     # what §2 adds
"item.hatchet_stone" = 0.3
```

The wrong-job penalty then costs nothing to express — it is a row on the
node that punishes it, exactly as §2's metal-hatchet-on-flesh is. Wall 7
holds: no code learns which tool suits which node.

### 9.2 The cost is one struct, and it is wall 6

`ItemStack` is `{ item: u16, count: u16 }` (`sim-core/src/gather.rs:350`)
and has no room. Condition touches the most-used type in the tree: the wire
(**`PROTO_VER` bump + regenerated goldens in the same commit**), `persist.rs`,
`worldsave.rs`, `state_hash`, every container, every golden, and the
`boxed_array` sizes.

Two shapes, and the choice is not obvious:

- **(a) A third field on `ItemStack`.** Every stack in the game pays the
  bytes, including the 900 wood that will never wear.
- **(b) A side table** keyed by (container, slot), holding condition only
  for the items that have it — which §1.5 says is exactly the items that
  stack to 1, so the table is small and `ItemStack` and its goldens never
  move.

**Recommend (a), and the reason is our own trap list.** The item-move verb
is named there as *the most bug-prone thing in the reference*, failing as a
disconnect, with the bug always in validation ordering against the mutation
rather than in arithmetic. (b) doubles the state every move must keep
consistent — move the stack, move its condition entry, keep them atomic
across a splice — which is that trap with a second structure bolted to it.
(a) costs bytes; (b) costs a class of bug we have already been warned about
in writing. Bytes are cheaper.

### 9.3 What must be spoken before any of it lands

Three, none of them inventable (`CLAUDE.md`: knobs are spoken, never
invented — these belong in `DECISIONS.md` §open):

1. **Maximum condition**, per tool or as one constant. §5 says there is no
   source, so this is ours to choose and must be chosen out loud.
2. **Does the rock wear?** §4's contradiction. Our answer decides whether a
   naked spawn can ever be *stuck*, which is a gameplay question and not a
   fidelity one.
3. **Where you repair.** We have no repair bench — `build::repair` mends
   structure pieces and knows nothing about items. A bench is a deployable
   (`content/deployables.toml`), a verb, and a container UI, and §1.3 says
   it must be a place rather than an inventory action.

### 9.4 Staging — the first slice does not need the bench

Durability is playable and complete without repair, and §4 is the proof: the
reference's own rock is an unrepairable tool you re-craft. So v0 is
**condition on tools, loss on gather, no bench** — a tool wears out and is
re-made, which needs §9.3 item 1 and nothing else. The −20% ladder, the
blueprint gate and the bench are v1, and they are where §5's missing
denominator starts to matter.

What v0 buys that is worth having on its own: it makes the stone hatchet a
*cost* rather than a permanent upgrade, which is the first thing in this
economy that would make a player return to a stone node they had already
outgrown.
