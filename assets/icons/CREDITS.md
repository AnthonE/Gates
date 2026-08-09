# Icon credits

The PNGs in this directory are rasterised from **game-icons.net**,
released under the **Creative Commons Attribution 3.0 Unported**
licence (CC BY 3.0). The licence requires that the original authors be
credited, which is what this file is for — it ships in the depot beside
the icons, and `crates/client/tests/ui.rs` fails if it stops shipping.

Icons made by:

- carl-olsen
- delapouite
- john-redman
- lorc

Available on https://game-icons.net

Licence: https://creativecommons.org/licenses/by/3.0/

## What maps to what

| file | source icon |
|---|---|
| `animal_fat.png` | `lorc/meat` |
| `bandage.png` | `lorc/bandage-roll` |
| `berries.png` | `delapouite/berries-bowl` |
| `building_plan.png` | `delapouite/notebook` |
| `burlap_hood.png` | `lorc/hood` |
| `burlap_tunic.png` | `lorc/leather-vest` |
| `charcoal.png` | `delapouite/coal-pile` |
| `cloth.png` | `delapouite/rolled-cloth` |
| `code_lock.png` | `delapouite/dial-padlock` |
| `corn.png` | `delapouite/corn` |
| `crossbow.png` | `carl-olsen/crossbow` |
| `fire_pit.png` | `lorc/campfire` |
| `furnace.png` | `delapouite/furnace` |
| `gears.png` | `lorc/gears` |
| `gunpowder.png` | `lorc/powder` |
| `hammer.png` | `lorc/claw-hammer` |
| `hearth.png` | `delapouite/fireplace` |
| `hunting_bow.png` | `delapouite/bow-arrow` |
| `large_box.png` | `delapouite/cargo-crate` |
| `low_grade_fuel.png` | `delapouite/jerrycan` |
| `medkit.png` | `delapouite/first-aid-kit` |
| `metal_arrow.png` | `delapouite/split-arrows` |
| `metal_door.png` | `delapouite/closed-doors` |
| `metal_fragments.png` | `lorc/metal-bar` |
| `metal_hatchet.png` | `delapouite/sharp-axe` |
| `metal_ore.png` | `delapouite/gold-nuggets` |
| `metal_pickaxe.png` | `delapouite/mining-helmet` |
| `metal_spear.png` | `lorc/barbed-spear` |
| `mushrooms.png` | `delapouite/mushrooms` |
| `pistol_round.png` | `delapouite/heavy-bullets` |
| `revolver.png` | `delapouite/revolver` |
| `roadsign_vest.png` | `lorc/armor-vest` |
| `rock.png` | `john-redman/rock` |
| `rope.png` | `delapouite/rope-coil` |
| `satchel_charge.png` | `delapouite/dynamite` |
| `shape_doorway.png` | `delapouite/door` |
| `shape_floor.png` | `delapouite/floor-hatch` |
| `shape_foundation.png` | `delapouite/flat-platform` |
| `shape_roof.png` | `delapouite/great-pyramid` |
| `shape_stairs.png` | `delapouite/3d-stairs` |
| `shape_wall.png` | `delapouite/brick-wall` |
| `sleeping_bag.png` | `delapouite/sleeping-bag` |
| `small_box.png` | `delapouite/wooden-crate` |
| `stone.png` | `delapouite/stone-pile` |
| `stone_hatchet.png` | `delapouite/hatchet` |
| `stone_pickaxe.png` | `delapouite/war-pick` |
| `sulfur.png` | `delapouite/powder-bag` |
| `sulfur_ore.png` | `lorc/crystal-cluster` |
| `tarp.png` | `delapouite/camping-tent` |
| `torch.png` | `delapouite/torch` |
| `wood.png` | `delapouite/log` |
| `wooden_arrow.png` | `delapouite/plain-arrow` |
| `wooden_door.png` | `delapouite/door` |
| `wooden_spear.png` | `lorc/spears` |
| `workbench.png` | `lorc/hammer-nails` |

## Not from game-icons.net

These are **ours**, authored in `ci/icons/` and rasterised by the same
script. No attribution is owed for them and the CC BY notice above does
not cover them — they are listed here so the line between what the
licence covers and what it does not is written down rather than
inferred from a table.

They exist because game-icons.net is unreachable from the environment
the food loop landed in (`DECISIONS.md` 2026-08-07 records the same
block for every 3D asset host), and two pictures were needed that day.
Replacing them with archive icons later is a mapping move in
`ci/bake_icons.py` and nothing else.

| file | source |
|---|---|
| `cooked_meat.png` | `ci/icons/cooked_meat.svg` (ours) |
| `raw_meat.png` | `ci/icons/raw_meat.svg` (ours) |
