# ashfall · CONTENT.md — the game as data (v0.1)

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
  door), per-material hp + upgrade cost (wood→stone→metal)
- **weapon**: kind (`melee|bow|firearm|throwable`), damage, headshot ×,
  rate, range falloff curve, ballistic (speed, drop) or hitscan, ammo id
- **armor**: slot, damage reduction %, movement penalty
- **consumable**: health/food/water deltas over seconds
- **deployable**: entity archetype (bag, hearth, cupboard, box, furnace,
  workbench), placement rules, hp
- **loot_table**: container archetype → weighted entries + count range
- **skin**: id, covers (item id), price (SCRY or MYRRH — one coin per
  row, bare tickers), season — the catalog is content too (dark until A3)

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
(animals are post-alpha; meat drops from… nothing yet — cut meat, keep
berries/mushrooms/corn) **(knob: cut list)**

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
   the band.
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
  jittered **(knob)**.

## 6 · What content deliberately cannot express

No schema field will ever exist for: stat-modifying skins, loot-odds
modifiers by identity or payment, XP/levels (progression is items and
knowledge only), or per-player drop rates. The absence is the never-table
(`DESIGN.md` §3.3) enforced at the schema layer — a field that can't be
written can't be sold.
