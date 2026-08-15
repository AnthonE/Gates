# Gates · CONTENT.md — the game as data (v0.1)

> Pillar 1 says content is data; this doc is where that becomes true.
> Everything a player can hold, craft, build, shoot, or loot is a row in
> `content/*.toml` — loaded and validated at boot, never hardcoded. A
> balance pass is a data commit; the sim code doesn't know what a
> "revolver" is, only what a `weapon` schema does. Numbers below are
> **opening values (all knobs)** — tuned through playtests by editing
> files, gated by `test_content`.

## 0 · The rules that make data safe

**Are these the reference game's numbers?** Some of them are, deliberately,
since 2026-08-08 — and `reference/BALANCE.md` is the file that answers this
properly. The short version:

- **Where a number has an equivalent in the reference and we have no reason
  of our own to differ, we take theirs and cite it at the row.** The
  operator's reason is product, not laziness: a player arriving from that
  game carries a table in their head, and every number of ours that
  contradicts it costs them a death to learn nothing about *our* game.
  Building blocks (250/500/1000), melee and tool damage, satchels-per-stone-
  wall, the boar's health.
- **Where we differ, the row says why.** The survival meters, gather yields,
  smelt rates, craft times, upkeep, decay and the armour ladder are all ours
  — mostly because they drive `§4`'s computed anchors, so moving one is a
  re-derivation of the economy rather than a lookup.
- **The bands still decide.** A reference value that does not fit our sim is
  refused by `test_content` exactly as an invented one would be, which is
  what stops "match the reference" from becoming "copy the spreadsheet". Two
  bands moved to admit the new numbers and both are spoken in
  `DECISIONS.md`.
- **The rails are unchanged and were never about arithmetic**: no traced
  art, no proper nouns, nothing decompiled (`ART.md` §7).

## 1 · Schemas (the shape, abridged — the .toml files are authoritative)

- **item**: id, name, stack, tier (0–2), rarity (`common|uncommon|rare|
  very_rare` → despawn multiplier), slot (`hand|head|body|none`)
- **gatherable**: node/tree archetype → per-tool yield table + hit count,
  weak-spot bonus, finish bonus. The node's payout is `hits × per-hit`
  and **both percentages move when it arrives, never how much**: the
  weak spot spends budget faster (skill buys speed), the finish share is
  withheld for whoever lands the last swing
- **recipe**: output, station (`none|workbench1|workbench2|workbench3|furnace`), inputs, seconds,
  and `blueprint` — locked until researched (see **research** below)
- **building_piece**: shape (foundation/wall/doorway/window/wall_frame/
  floor/stairs/roof/tri_foundation/tri_floor/tri_roof — the door is a
  deployable), per-material hp + upgrade cost (wood→stone→metal). One `cost` row
  serves three verbs: build it, upgrade into it, and **mend it** — a repair
  is charged that cost pro-rata against the hp being restored, scaled by
  `globals.repair_cost_pct` (100 = the damage's worth exactly, validated
  1..=100), rounded up and floored at one unit so no repair is ever free.
- **weapon**: kind (`melee|bow|firearm|throwable`), damage, headshot ×,
  rate, range falloff curve, ballistic (speed, drop) or hitscan, ammo id
- **armor**: slot, damage reduction %, movement penalty
- **consumable**: health/food/water deltas over seconds
- **mob**: one row per animal species — hp, speeds as a percentage of the
  player's own, the leash in metres, **two notice radii** (day and night,
  the only content number the *hour* selects — `night_spook_m`), the
  respawn in seconds, and the stacks a kill pays. Two species ship, prey
  and hunter, and they differ by content numbers alone: nothing in
  `mob.rs` branches on species. `content/mobs.toml`; the sim's side is
  `sim-core/src/mob.rs` and the design is `reference/ANIMALS.md` §9.
- **deployable**: entity archetype (bag, hearth, cupboard, box, furnace,
  workbench, door, lock, recycler, research), placement rules, hp
- **fuel / cook** (`cooking.toml`): what an oven burns — item, seconds per
  unit, byproduct + `byproduct_pct` (hundredths of a unit per unit burned,
  banked and paid whole, never rolled) — and one row per transformation:
  input → output, `count` (units paid, default 1), seconds, `station`
  (`fire|furnace|recycler`). All three are one thing in the sim
  (`oven.rs`); the station column is the only thing that separates them,
  and the recycler is the one that burns nothing.
  **Several rows may share a `(station, input)` and they fire together**
  off one slot timer — the bake holds such a set to a single `seconds` and
  refuses two rows paying the same output. That is what lets one component
  pay a material *and* a coin, and it is why arming `ALPHA.md`'s A2 faucet
  is an edit to this file rather than to `crates/`.
- **research** (`research.toml`): `[coin]` names what research is paid in
  (one item for the whole table — `sim-core` never learns it is currency),
  and `[[research]] item + cost` is what may be learned. The recipe a row
  unlocks is the one that **outputs** that item, resolved at bake, so the
  file never names a recipe id and the two cannot drift. A recipe's
  `blueprint = true` is the other half; the two are checked against each
  other both ways — a gated recipe with no row is uncraftable forever, a
  row for an item nothing crafts is a coin sink that unlocks nothing, and
  `validate::structural` refuses both. **`requires` (2026-08-15) is the
  ladder**: item ids that must already be known, resolved at bake into a
  mask in `Player::known`'s own bit space. It is authored but not
  free-form — a blueprint-gated recipe whose *inputs* include another
  blueprint-gated item implies that edge, and a row that omits one is
  refused, so the tree can add to the recipes' dependencies and never
  contradict them. One fixpoint walk from the empty known-set refuses a
  cycle and a row stranded behind one as the same thing, because
  "unreachable" is what a player experiences and a cycle is one cause.
  ⚠ The BENCH tier (workbench 2/3, the tree UI) is a different system and
  is unbuilt — `NOW.md` §0tt, and the era is a spoken knob.
- **loot_table**: container archetype → weighted entries + count range
- **skin**: id, covers (item id), price (SCRY or MYRRH — one coin per
  row, bare tickers), season — the catalog is content too (dark until A3)

## 1.5 · The spawn kit

`content/balance.toml` `[[spawn_kit]]` — what a fresh character is holding
when they open their eyes. Entries are `{ item, count }`, granted **in
order** into inventory slots, so the first `HOTBAR_SLOTS` are the belt.

```toml
[[spawn_kit]]
item = "item.building_plan"
count = 1
```

Three rules, each with a refusal in `validate`:

- **Absent is legal and means naked.** The table is `#[serde(default)]`; a
  public shard expresses a beach spawn by deleting it, not by editing code.
- **One entry per item**, because `grant_kit` writes slots and never merges
  — two stacks of the same thing is a typo that halves the grant.
- **Count within the item's own `stack`**, and in practice comfortably
  under it: a grant at the ceiling is one balance edit from refusal.

It exists as **testing scaffolding** and an operator arms or empties it
(`DECISIONS.md` §open "spawn kit v0"). It is content rather than a server
flag because the content hash is already in the WAL header, so a replay
replays the kit it was played under — a `shard.toml` switch would diverge.

## 2 · The alpha item set (~45 items — this IS the shape of the game)

**Raw**: wood · stone · metal_ore · sulfur_ore · cloth · fat · charcoal ·
metal_frags (furnace) · sulfur (furnace) · gunpowder (charcoal+sulfur) ·
lowgrade (fat+cloth) · components: gears · rope · tarp (barrel-only —
the reason the coast road matters)

**T0 — naked era**: rock · torch · spear_wood · hatchet_stone ·
pickaxe_stone · bow · arrow_wood · bandage · sleeping_bag (cloth) ·
box_small · fire_pit

**T1 — workbench era**: workbench1 · hearth (the cupboard) · hatchet_metal
· pickaxe_metal · spear_metal · furnace · box_large · door_wood ·
building plan + hammer (free-craft) · armor_burlap set · arrow_metal ·
crossbow

**T2 — powder era**: revolver · pistol_ammo · satchel_charge (gunpowder ×
N + rope + tarp) · door_metal · armor_roadsign (component-gated) ·
medkit

**Food**: berries (the bush's side payout) · mushrooms (the tree's — the
forest floor pays through the tree that shades it, because a meadow/crop
node archetype is a sim occupant we don't have) · corn (a coast-road
barrel ration, standing in for the reference's riverside crops and
roadside food crates) · **raw_meat + cooked_meat** (the pig drops raw, a
campfire cooks it, and only the cooked half is edible) **(knob: cut list)**

The parenthesis used to read "animals are post-alpha; meat drops from…
nothing yet", and **both halves of that stopped being true on 2026-08-08**,
in two branches that landed hours apart and did not know about each other.
Animals arrived (`content/mobs.toml` — the pig pays `item.fat` and
`item.cloth`, and `item.fat` had existed since the first content set with
nothing in the world dropping it). Cooking arrived the same day
(`sim-core/oven.rs`), and §1's `cook` row says its table ships **empty**
because "cooking wants a raw food and the island pays none".

**Both are closed, and it cost four content rows and not one line of
code** (operator, 2026-08-08). `item.raw_meat` and `item.cooked_meat` in
`items.toml`, a `drops` row on the pig, a `[[cook]]` row on the fire, and
a `consumable` row on the cooked half. `cooking.toml`'s own header had
predicted the shape of it — "adding one is a one-row content edit and no
code" — which is the table shipping before its first row working as
intended, and both files keep that history rather than tidying it away.

**Raw meat is the one item in the set you cannot eat**, and that asymmetry
is the whole point: without it the fire is optional and the walk from a
kill to a meal is a detour rather than a loop. No sickness verb is invented
to punish eating it — the eat verb simply does not accept it, which the
schema already expresses (`consumables.toml` names what may be eaten and
nothing else). `crates/content/tests/content.rs` gates the three-file loop,
because each row validates perfectly on its own while any one of them
missing leaves a player holding an item with no use.

That's the whole alpha economy: two ores, one powder chain, one gun, one
raid tool. Everything else is reachable-by-schema without touching sim.

## 3 · Progression pacing targets (knobs — aspirations, mostly untested)

| milestone | target from fresh spawn |
|---|---|
| bow + 10 arrows | ~10 min solo |
| starter base (2×1 stone, door, bag, hearth, box) | ~45 min solo |
| revolver era | ~2 h solo / ~1 h duo |
| first satchel | ~3–4 h of group farm |
| full wipe arc | fits the cadence: nobody "finishes" week 1 |

This header used to claim the table was "tested as bands", and for four of
five rows nothing tested them — the computed `starter_minutes` anchor is
85.6 against the ~45 above, with no band asserting either, and the
farm-minute currency itself is ~20× slower than measured play (the
farmwalk measured 1001 wood/min effective against the declared 50 —
`DECISIONS.md` §open, "the economy has never been measured against the
sim"). Only the wood-wall cost and the upkeep ceiling in §4 anchor 3 are
banded. Whether this table becomes bands or stops claiming minutes is
that same open knob; until it is spoken these rows are aspirations, and
re-deriving them is the blocker in front of moving gather yields to the
reference's.

## 4 · The balance anchors (the three numbers that ARE the game)

1. **Raid ratio** — cost to open a base ÷ cost to build it, both in
   farm-minutes, computed from data (satchel chain vs piece hp): target
   **≈ 1.5× for a starter, rising with wall tier**. Below ~1.0 nobody
   builds; above ~3 nobody raids. `test_content` computes it and asserts
   the band. The divisor is `weapons.toml`'s `structure` column — every
   weapon's damage against a piece or a deployable, its own number and
   never `damage` scaled.

   **The melee face of the same anchor**, in swings (integer-exact out of
   the data; minutes would drag the sim's swing cadence into a content
   assert): the weakest door falls to the best melee weapon inside
   `bands.door_breach_swings` — the door is the intended breach point and
   must stay openable by hand — while every wall at every material sits
   above `bands.wall_breach_swings_min` and the ladder rises with tier.
   Two ordering laws carry no number: no weapon's `structure` may exceed
   its own `damage`, and the throwable raid tool's must strictly exceed
   every melee weapon's. `test_content` asserts all of it.
2. **TTK bands** (body hits, no armor): melee 3–5 · bow 3–4 · revolver
   4–6; headshot × 2. Armor may add at most +2 hits. Asserted from data.
3. **Farm rate** — a node pays the reference game's total over ~10 swings
   (2026-08-10: stone 1000, metal 600, sulfur 300, large tree 870 — it
   read "≈ 300 units" for every node before that); a full wood wall ≈ **4**
   min of wood at T1 tools. **That figure was 7 and moved the same day the
   build costs were taken** (`DECISIONS.md` §open "twig v0"): their wood
   wall is 200 wood where ours was 350, and 350 had never been compared to
   anything — the 2026-08-08 balance pass took the hp ladder out of
   `building.toml` and left the `cost` column alone. What made it visible
   was the node take: once a tree paid *their* 810 wood, a wall priced at
   *ours* cost 1.75× theirs in trees, the unit a player actually feels. So
   the pair now comes from one game. The band moved [5, 9] → [3, 5] and
   that is `BALANCE.md` §7's rule, not a loosening. Upkeep (DESIGN §2) prices decay in these
   same farm-minutes; the band keeps a solo's daily upkeep under ~15 min
   **(knob)**.

When a playtest says "raiding is too cheap," the fix is a `.toml` edit
that moves anchor 1 inside a re-spoken band — one commit, no code, WAL
hash notes it, replays unaffected retroactively.

## 5 · Loot tables (alpha)

- **barrel** (coast road): components-weighted, small metal_frags,
  lowgrade, a corn ration (§2's food line); rare: revolver blueprint-free
  drop **(knob: drop vs craft-only)**
- **crate** (haven periphery + headlands): T1/T2 mats, powder chain
  pieces, medkit
- Barrels respawn on the slot system (`TERRAIN.md` §2), 15–30 min
  jittered **(knob)** — the sim ships the spoken 20–45 min window
  (`DECISIONS.md` §open, "node/barrel respawn"), which is what
  `gather.rs` reads for nodes and barrels alike.
- Each table carries **`hits`**: swings to open the container
  **(knob: barrel 3, crate 5)**. Content, not code — re-pricing the walk
  between barrels is a balance pass. Zero is refused at validate: a
  container nothing can open never pays.
- The roll lands in a **ground container**, never in the smasher's
  inventory — the same store a death bag stands up in (`backpack.rs`), so
  the loot panel is one panel. Weighted pick is Lemire multiply-shift over
  the baked weight sum, so the weight-1 revolver keeps its odds instead of
  losing them to modulo bias.

## 6 · What content deliberately cannot express

No schema field will ever exist for: stat-modifying skins, loot-odds
modifiers by identity or payment, XP/levels (progression is items and
knowledge only), or per-player drop rates. The absence is `BUSINESS.md`
(`DESIGN.md` §3.3) enforced at the schema layer — a field that can't be
written can't be sold.
