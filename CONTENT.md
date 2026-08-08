# Gates · CONTENT.md — the game as data (v0.1)

> Pillar 1 says content is data; this doc is where that becomes true.
> Everything a player can hold, craft, build, shoot, or loot is a row in
> `content/*.toml` — loaded and validated at boot, never hardcoded. A
> balance pass is a data commit; the sim code doesn't know what a
> "revolver" is, only what a `weapon` schema does. Numbers below are
> **opening values (all knobs)** — tuned through playtests by editing
> files, gated by `test_content`.

## 0 · The rules that make data safe

- **The content hash is part of determinism**: xxh3 over the canonical
  serialized content set, pinned into every WAL header. A replay loads
  the content it was played under; a balance patch is visible in the
  record. Changing content mid-wipe is allowed (hotfix) but stamps a WAL
  event.
- **Ids are permanent strings** (`item.stone_hatchet`); renames are new
  ids with a migration row. The WAL refers to ids forever.
- **`test_content` gates every commit**: schema validity, no orphan
  recipe inputs/outputs, every loot entry exists, every item has a
  despawn tier, and — the teeth — **computed balance bands** (§4): TTK,
  farm-hours, and raid-cost ratios are derived from the data and asserted
  inside declared ranges. A balance edit that breaks the band fails CI
  and forces the band (a `DECISIONS.md` knob) to be re-spoken, not
  silently drifted.

## 1 · Schemas (the shape, abridged — the .toml files are authoritative)

- **item**: id, name, stack, tier (0–2), rarity (`common|uncommon|rare|
  very_rare` → despawn multiplier), slot (`hand|head|body|none`)
- **gatherable**: node/tree archetype → per-tool yield table + hit count,
  weak-spot bonus
- **recipe**: output, station (`none|workbench1|furnace`), inputs, seconds
- **building_piece**: shape (foundation/wall/doorway/floor/stairs/roof/
  door), per-material hp + upgrade cost (wood→stone→metal). One `cost` row
  serves three verbs: build it, upgrade into it, and **mend it** — a repair
  is charged that cost pro-rata against the hp being restored, scaled by
  `globals.repair_cost_pct` (100 = the damage's worth exactly, validated
  1..=100), rounded up and floored at one unit so no repair is ever free.
- **weapon**: kind (`melee|bow|firearm|throwable`), damage, headshot ×,
  rate, range falloff curve, ballistic (speed, drop) or hitscan, ammo id
- **armor**: slot, damage reduction %, movement penalty
- **consumable**: health/food/water deltas over seconds
- **mob**: one animal species — hp, speeds as a percentage of the player's
  own, the leash and fright radii in metres, the respawn in seconds, and
  the stacks a kill pays. `content/mobs.toml`; the sim's side is
  `sim-core/src/mob.rs` and the design is `reference/ANIMALS.md` §9.
- **deployable**: entity archetype (bag, hearth, cupboard, box, furnace,
  workbench), placement rules, hp
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

**Food**: berries · mushrooms · corn (meadow scatter) · cooked_meat
(cut meat, keep berries/mushrooms/corn) **(knob: cut list)**

The parenthesis used to read "animals are post-alpha; meat drops from…
nothing yet", and half of that is now wrong: animals landed 2026-08-08 and
the pig drops `item.fat` and `item.cloth` (`content/mobs.toml`). **Meat is
still cut, for the other half of the reason** — cooking is a verb we do not
have, and one animal does not un-cut a verb. `item.fat` had existed since
the first content set with nothing in the world dropping it; it does now.

That's the whole alpha economy: two ores, one powder chain, one gun, one
raid tool. Everything else is reachable-by-schema without touching sim.

## 3 · Progression pacing targets (knobs, tested as bands)

| milestone | target from fresh spawn |
|---|---|
| bow + 10 arrows | ~10 min solo |
| starter base (2×1 stone, door, bag, hearth, box) | ~45 min solo |
| revolver era | ~2 h solo / ~1 h duo |
| first satchel | ~3–4 h of group farm |
| full wipe arc | fits the cadence: nobody "finishes" week 1 |

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
3. **Farm rate** — one node ≈ 300 units over ~10 swings; a full wood wall
   ≈ 7 min of wood at T1 tools. Upkeep (DESIGN §2) prices decay in these
   same farm-minutes; the band keeps a solo's daily upkeep under ~15 min
   **(knob)**.

When a playtest says "raiding is too cheap," the fix is a `.toml` edit
that moves anchor 1 inside a re-spoken band — one commit, no code, WAL
hash notes it, replays unaffected retroactively.

## 5 · Loot tables (alpha)

- **barrel** (coast road): components-weighted, small metal_frags,
  lowgrade; rare: revolver blueprint-free drop **(knob: drop vs craft-only)**
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
