# reference/LOOT.md — how the reference game gets loot **out of a broken thing and into a pair of hands**

**Owns nothing.** Research, not law — `DOORS.md`'s posture. Read it before
touching `sim-core/src/loot.rs`, `worldcont.rs`, `backpack.rs`, or the
question of what a smashed barrel leaves behind. `CONTENT.md` §4's bands
and `BALANCE.md` §6 still decide whether any number here may land.

Written because the operator put three sentences beside a frame
(2026-09-16): *"when u break a barrel it drops loot as 3d objects… we could
just make it generic now but eventually loot can fall down and out of reach
kinda thing. or roll a bit. and when u do loot a crate they despawn after
that. then u might wanna look at how rust does the inventory"*. Two of those
three turned out to name a defect in our own tree, and one of the two was a
day old.

## §0 · Provenance, per claim

Probed rather than assumed (`SOURCES.md` §0): `rust.facepunch.com` answered
from this box on 2026-09-16, as it did for `FORESTS.md`, `ROCKS.md` and
`ROADS.md`. `wiki.facepunch.com/rust/Loot` is a 404 (no such page),
`rust.fandom.com` answered **402 Payment Required**, and `umod.org` was
**403** as it was for `CRAFTING.md` — so the plugin evidence below is search
summaries of plugin descriptions, never the pages.

| tier | what | used for |
|---|---|---|
| **1 — in tree** | `reference/rust-systems.txt`, the Oxide hook table (MIT, facts only) | §2 entirely: the classes, their methods and the **argument lists**, which is the strongest thing in this doc |
| **1 — fetched whole** | `rust.facepunch.com/news/devblog-145` | §4's quotes, verbatim |
| **3 — search summary** | despawn tiers, loot sacks, barrels spilling on destruction, `containerMain/Belt/Wear` sizes | §1, §3, §5 — flagged at each use |
| **3 — inverse signal** | what the `LootBouncer` plugin family exists to fix | §4, and it is the load-bearing half |

⚠ **What the sources do NOT contain**, said here rather than guessed at
later: the *physics* of a dropped item (mass, drag, restitution, how far it
may roll, whether it sleeps), the despawn constants at primary tier, and
whether a destroyed barrel's items are spawned individually or as one sack
in the current build. §2's `Drop(Vector3, Vector3, Quaternion)` is the
closest thing to evidence about motion and it is a *signature*, not a
behaviour.

## §1 · Three shapes of loot in the world, not one

- **A junkpile** is a *cluster*: "the contents of junk piles are procedural,
  with anywhere from 1-4 barrels and if you're lucky, a crate" (tier 3). It
  is a spawned population member with a lifetime (§4).
- **A barrel** is destroyed rather than opened, and its contents "drop on the
  ground after the barrel has been demolished" (tier 3, and corroborated by
  an inverse signal: a plugin called *Auto Pickup Barrel* exists, which is
  only a feature if the loot lands on the ground rather than in a panel).
- **A crate** is opened with `E` and is a container. Emptied, it is gone —
  the operator's own observation, and consistent with tier-3 reports that
  bags "disappear if they are emptied out".

So the reference does **not** have one loot delivery mechanism. A thing you
*break* scatters; a thing you *open* is a container; and the container is
removed when there is nothing left in it.

## §2 · A dropped item is an entity, and the drop verb takes a velocity

The strongest section, because it is the in-tree hook table and therefore
facts rather than prose. Four classes carry the whole model:

| class · method | what it settles |
|---|---|
| `Item.Drop(Vector3, Vector3, Quaternion)` → `OnItemDropped` | a drop takes **position, velocity and rotation**. The operator's *"roll a bit"* is in the signature: the thing is handed momentum, not a resting place |
| `WorldItem.Pickup(BaseEntity/RPCMessage)` → `OnItemPickup` | an item lying in the world is picked up by an **RPC on the entity** — a verb aimed at a thing, not a container move |
| `WorldItem.RPC_OpenLoot` → `CanLootEntity` | and the same entity can be *looted* rather than taken, which is how a dropped backpack and a dropped rock are one class |
| `DroppedItem.OnDroppedOn(DroppedItem)` → `CanCombineDroppedItem` / `OnDroppedItemCombined` | **two dropped items merge when one lands on the other** — the stacking rule follows the item into the world |
| `DroppedItem.IdleDestroy()` → `OnItemDespawn` | despawn is a method named for *idleness*, so the clock is about being untouched rather than about age |
| `DroppedItemContainer` → `RPC_OpenLoot`, `PlayerOpenLoot` | the corpse/sack: one entity holding many stacks, opened as a panel. Our `backpack.rs` is this class |

Note what is absent: no `LootSpill`, no per-item physics hook, and
`LootContainer.SpawnLoot()` / `LootFill.DelayFill()` are the only fill
paths — so the *fill* is a container-side event even when the delivery ends
up on the ground.

## §3 · Despawn is keyed to rarity, and handling resets it

Tier 3 throughout, consistent across three independent guides: roughly
**5 minutes for common, 20 for uncommon, 60 for rare**; "picking up an item
and dropping it again will reset the despawn timer"; and the split that
matters for us — "items in bags, toolboxes and storage containers last as
long as the container survives, but items lying on floors or ground despawn
based on their tier rarity."

**We already ship this model** for the container case:
`backpack.rs`'s despawn is "one base constant × the rarity multiplier of the
rarest item inside", baked from `content/items.toml`'s rarity column. That
is not a coincidence to be proud of — it is the same reading of the same
game — but it does mean a loose-item despawn has a ladder to reuse rather
than a knob to invent.

## §4 · The unfinished container is *their* unsolved problem too

This is the section that changed our tree today, and it is primary.

**Devblog 145**, fetched whole:

> "If a junkpile still has unbroken barrels or unlooted crates, it's going
> to stick around until its despawn time of roughly 20 minutes."
>
> "This means it is eating up a population slot and reducing the overall
> number of fresh junkpiles to be found in the world."

And their fix, in the same post, is **content rather than code**:

> "I've added some Low Grade Fuel to the Oil Barrel as well as the existing
> crude. Hopefully people will finish the job now."

So: a container nobody finished occupies a slot in a capped population and
starves the world of fresh loot, and the shipped answer was *make the last
item worth taking*. Nine years later the code side is still patched by
third parties — the `LootBouncer` family "empties the containers when
players do not pick up all the items", and its fork states the purpose in
its own title: *"prevent spawn blocking and improve roadside loot
respawn"* (tier 3, inverse signal).

⚠ **Read as an inverse signal, this is the strongest finding in the doc**:
a whole plugin category exists because vanilla leaves partly-looted
containers in the world, and server owners consider that worth patching on
every wipe.

## §5 · The inventory: three containers, and wearing is a move

The operator's third sentence. What is sourceable:

- **Three separate containers on the player** — `containerMain` 24 slots,
  `containerBelt` 6, `containerWear` 7 (tier 3, consistent across several
  plugin-development threads) — snapshotted together but addressed apart.
  The *structure* is corroborated at tier 1 by the hook table's
  `SendUpdatedInventoryInternal(PlayerInventory/Type, ItemContainer, …)`:
  containers are typed and passed one at a time.
- **Wearing is a container move, not an equip verb** — `CanWearItem(Item,
  Int32)` beside `CanEquipItem`, one `MoveItem` under both, no `EquipItem`
  RPC anywhere. `reference/ARMOR.md` §9.2 already took this and it is why
  `CONT_WEAR` exists here.
- **What a container accepts is a predicate on the container** —
  `ItemContainer.CanAcceptItem(Item, Int32)`, specialized by the vending
  machine as `CanVendingAcceptItem`. `inventory::takes_deposits` is our
  answer (wire v64).
- **The move verb is one RPC** — `PlayerInventory.MoveItem(RPCMessage)` with
  `CanMoveItem` over it. One verb for arrange, loot, deposit and equip,
  exactly as `Command::Move` is here.

**The belt being its own container is the fact with a consequence for us**,
and §9.5 spends it.

## §9 · What it means for us

Written against the tree, `file:line` per claim, in the shape `NETWORK.md`
§9 set.

### §9.1 · Our three shapes are already theirs, with one swapped

| theirs | ours | same? |
|---|---|---|
| junkpile: a cluster with a lifetime | no equivalent — barrels are scatter cells with a respawn (`terrain::scatter`, `gather.rs`) | **no**, deliberately: our world is authored per cell, not populated per slot |
| barrel: destroyed, **spills items on the ground** | destroyed, and stands up **one container** at its feet (`world.rs:4352-4380` → `backpacks::stand_up`) | **no** — §9.3 |
| crate: opened, removed when empty | opened, **refills in place** (`worldcont.rs`) | **no**, deliberately: a crate stands at a terrain-authored cell and the destination gradient needs it to be there next visit |

The barrel row is the only one of the three that is a gap rather than a
decision, and it is the operator's first sentence.

### §9.2 · §4's defect was in our tree, worse than theirs — FIXED 2026-09-16

Their unfinished container occupies a population slot for ~20 minutes.
Ours was blocked **forever**: `worldcont::set_slot` armed the refill on the
transition to *empty*, so a crate with a leftover stack was never on the
clock at all — measured at one unit taken out of a stack of four and the
same three units still sitting there 960,003 ticks later. No malice
required; taking what you want and leaving the junk is how looting works.

Fixed by arming the clock on the **first** disturbance, which only became
safe when wire v64 made a deposit impossible (the narrow rule existed to
stop a player holding the refill off by putting an item back). Gates:
`a_crate_with_one_unit_taken_is_already_on_the_clock`,
`a_second_take_does_not_move_the_clock`.

**And Devblog 145's content-side lesson is checked rather than assumed**:
every entry in all three of our tables (`content/loot.toml`) is a resource
or component a player wants — there is no *crude-oil tail* nobody would
carry, so we do not currently rely on players finishing a job they have no
reason to finish. Worth re-checking whenever a table grows a junk row.

### §9.3 · Loose world items — **BUILT 2026-09-16** (ground items v0)

The operator's *"generic now, roll later"*, spoken the same day this doc
was written (*"yea lets cook it"*) and built the same day. What landed is
items 1, 3, 4, 5 and 6 below; **item 2 is the half that was deferred on
purpose** — the settle is a pure landing spot (`grounditem::rest_spot`,
whose height is `terrain::ground`, so loot lands downhill or over a lip)
rather than a tumble a player watches. `NOW.md` §0wc 2b carries the rest,
and the rule this section states about item 2 is the one a later slice
must keep.

What it is:

1. **A store** — `WorldItems`, capped in `limits.rs` with a stated overflow
   policy (wall 4), holding `(qx, qy, qz, ItemStack, despawn_at)`.
2. **A settle**, which is where the *"fall down and out of reach"* lives:
   a per-tick integrate for items still in flight, gravity plus the drop
   velocity `Item.Drop`'s signature says the reference hands out, resolved
   against `terrain::height` and `collide`. Determinism-safe — no trig, and
   the restricted operator set covers it (wall 1) — and cheap, because a
   settled item does no work. **This is the piece that must not be faked**:
   a client-side tumble over a server-side resting place is two truths about
   where an item is, and the item is a thing you can pick up.
3. **A wire lane** — a sync like `BagSync` (`protocol/src/event.rs`),
   AOI-filtered, plus a pickup action. One `PROTO_VER` turn (wall 6).
4. **A pickup verb.** `WorldItem.Pickup` is an RPC aimed at an entity;
   ours would be the nearest-in-reach shape `backpack.rs` and `gather::swing`
   already use, so nothing spoofable crosses the wire.
5. **The client**: a generic mesh per item (the operator's *"just make it
   generic"* — one sack, every item, which is also what stops this slice
   waiting on `assets/models/WANTED.md`), a prompt, and the pickup action.
6. **Despawn**, reusing §3's rarity ladder rather than minting a knob.

**What it is not**: a replacement for the death bag. The reference
consolidates a corpse into **one** `DroppedItemContainer` for cost, and
`backpack.rs`'s header already cites that decision as the reason ours is one
entity and not thirty. So the split to build is theirs: **a thing you break
scatters, a thing that dies leaves a container.**

Order, if it is taken: the sim store and the settle first (gated as
arithmetic, invisible), then the wire, then the client mesh. The physics the
operator defers is item 2, and it is worth noticing that item 2 is *also*
the cheapest half — the expensive parts are the lane and the streamer.

### §9.4 · An emptied crate looked full — **BUILT 2026-09-17**

Their crate is *gone* when emptied, which is a distance signal: you can see
a picked-over site. Ours stood identical whether it held three stacks or
nothing, so a wasted walk was normal on a populated shard.

Their junkpile-lifetime model (§4) is still the alternative worth naming: a
container with a *lifetime* rather than a refill needs no "is it empty"
broadcast at all, because it is either there or not. Ours cannot take that
— the crate's position is a pure function of the seed (`worldcont.rs`, "It
does not place anything") and the client draws it from the same function.

⚠ **What this section got wrong is worth keeping, because the error was a
PRICE and it deferred the feature for a day.** It read the options as a
**state on the wire** (one bit per crate in AOI, the client drawing a lid
or an absence) or a **shorter window**, and the first turned out to cost
nothing: `gather::SlotLives` already answers *"is the slot in this cell
currently gone?"*, and a smashed barrel has ridden it since world structure
v1. It is on the wire (`EV_SLOT_HARVESTED`, the cell alone — the client
re-derives the occupant from shared worldgen, so nothing about this moved
`PROTO_VER`), mirrored on the client (`HarvestedSet`), handed to a late
joiner by the sync walk, saved, and asked by `occupy` before a slot may
block a body, hold a ray or carry ground — **on both sides**, which is the
quantize-both-sides law and the reason the client needed no prediction of
its own for this.

So the whole of it is one call at the sim's single `CONT_WORLD` write
(`World::set_cont_slot`): when the record goes empty, harvest its cell
**until the refill tick the record already rolled**. One number for both
facts, so a crate cannot come back before or after its loot. Everything
else follows from consumers that already existed — the mesh takes
`FellPart::Vanish` (`props::harvestable`), `resolve_open` stops offering
the verb, the drip closes the panel on the path a despawned bag uses, and
`respawn_due` announces it standing again.

Two things it cost, both stated rather than found later: a crate is
openable **one tick after** its deadline (the sweep runs after the tick's
commands, exactly as a barrel always has), and the sim needed a fourth
silent refusal on `OpenWorldCont` — `worldcont::open` re-derives the
occupant from the seed, so it would otherwise roll loot into a container
nobody can see on that one tick. `NOW.md` §0wc item 2 is closed; the
shorter window was not needed and is not taken.

### §9.5 · The belt is their own container, and ours is six slots of one

Our belt is `inv[0..HOTBAR_SLOTS]` of `CONT_SELF` — one array, one move verb
("'arrange the hotbar' and 'arrange the backpack' are one verb here, not
two", `inventory.rs`). That simplification has paid for itself: it is why
choosing what you hold needed no second verb, and it is load-bearing in
`hold::held_in_hand`.

But it means *"the first free slot"* is a belt slot, and the reference
cannot have that bug because a quick-move addresses `containerMain` and the
belt is a different container. So the deviation costs one line of walk
order in `ui::slots::quick_move`, not a redesign: prefer
`HOTBAR_SLOTS..INV_SLOTS`, fall back to the belt when the grid is full.
**Taken 2026-09-16** — §5's container split is the case for it, where
`NOW.md` §0p2 item 4d had it as an unsourced taste call.

### §9.6 · What not to take

- **A junkpile population.** Our loot sites are authored per cell by
  `terrain::scatter` on purpose (`TERRAIN.md` §7/§8, `MONUMENTS.md` §9), and
  a spawned-population model would be a second answer to where loot is.
- **A per-item despawn knob.** §3's ladder exists in `content/items.toml`
  and `backpack.rs` bakes it; a second constant for loose items would drift
  from it.
- **Their numbers.** Nothing in this doc is at a tier that may reach
  `content/` (`SOURCES.md`'s rail, and `MONUMENTS.md` §0's lesson about
  weak provenance). The 5/20/60 despawn tiers are a *shape*, not values.
