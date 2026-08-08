#!/usr/bin/env python3
"""Rasterise the icons the client ships out of the game-icons.net archive.

Run when an item is added to `content/items.toml` or a mapping below changes:

    GAME_ICONS_DIR=/tmp/game-icons python3 ci/bake_icons.py

Needs `cairosvg`. Writes `assets/icons/*.png` and `assets/icons/CREDITS.md`,
both of which are committed — the depot ships `assets/` wholesale and no
build step runs this. `crates/client/tests/ui.rs` §G fails if the baked set
and `client::ui::icons::STEMS` ever disagree, or if an item in the content
has no picture.

The icons are CC BY 3.0. Nothing here is traced from the reference game.


White-on-transparent PNGs, so the client tints them with `ImageNode.color`
instead of shipping one file per state.
"""
import os
import pathlib
import re
import sys

import cairosvg

ROOT = pathlib.Path(__file__).resolve().parent.parent
# The archive game-icons.net publishes, unzipped. Fetch it with:
#   curl -L -o icons.zip \
#     https://game-icons.net/archives/svg/zip/ffffff/transparent/game-icons.net.svg.zip
#   unzip -q icons.zip -d <dir>
# White-on-transparent is the variant we want: the client tints at the draw.
SRC = pathlib.Path(
    os.environ.get("GAME_ICONS_DIR", "/tmp/game-icons")
) / "icons/ffffff/transparent/1x1"
OUT = ROOT / "assets/icons"
PX = 128

# The six building shapes the wheel draws. **No material icons**: the wheel
# is one ring since 2026-08-07 — the blueprint places the bottom rung and the
# hammer climbs the ladder — so `mat_wood`/`mat_stone`/`mat_metal` had no
# draw site and shipping them was dead weight in the depot. They come back
# with the hammer wheel, not before.
SHAPES = {
    "shape_foundation": "delapouite/flat-platform",
    "shape_wall": "delapouite/brick-wall",
    "shape_doorway": "delapouite/door",
    "shape_floor": "delapouite/floor-hatch",
    "shape_stairs": "delapouite/3d-stairs",
    "shape_roof": "delapouite/great-pyramid",
}

# Every item in `content/items.toml`, by its id minus the `item.` prefix.
ITEMS = {
    "wood": "delapouite/log",
    "stone": "delapouite/stone-pile",
    "metal_ore": "delapouite/gold-nuggets",
    "sulfur_ore": "lorc/crystal-cluster",
    "cloth": "delapouite/rolled-cloth",
    "fat": "lorc/meat",
    "charcoal": "delapouite/coal-pile",
    "metal_frags": "lorc/metal-bar",
    "sulfur": "delapouite/powder-bag",
    "gunpowder": "lorc/powder",
    "lowgrade": "delapouite/jerrycan",
    "gears": "lorc/gears",
    "rope": "delapouite/rope-coil",
    "tarp": "delapouite/camping-tent",
    "rock": "john-redman/rock",
    "torch": "delapouite/torch",
    "spear_wood": "lorc/spears",
    "hatchet_stone": "delapouite/hatchet",
    "pickaxe_stone": "delapouite/war-pick",
    "bow": "delapouite/bow-arrow",
    "arrow_wood": "delapouite/plain-arrow",
    "bandage": "lorc/bandage-roll",
    "sleeping_bag": "delapouite/sleeping-bag",
    "box_small": "delapouite/wooden-crate",
    "fire_pit": "lorc/campfire",
    "workbench1": "lorc/hammer-nails",
    "hearth": "delapouite/fireplace",
    "hatchet_metal": "delapouite/sharp-axe",
    "pickaxe_metal": "delapouite/mining-helmet",
    "spear_metal": "lorc/barbed-spear",
    "furnace": "delapouite/furnace",
    "box_large": "delapouite/cargo-crate",
    "door_wood": "delapouite/door",
    "building_plan": "delapouite/notebook",
    "hammer": "lorc/claw-hammer",
    "armor_burlap_head": "lorc/hood",
    "armor_burlap_body": "lorc/leather-vest",
    "arrow_metal": "delapouite/split-arrows",
    "crossbow": "carl-olsen/crossbow",
    "revolver": "delapouite/revolver",
    "pistol_ammo": "delapouite/heavy-bullets",
    "satchel_charge": "delapouite/dynamite",
    "door_metal": "delapouite/closed-doors",
    "armor_roadsign_body": "lorc/armor-vest",
    "medkit": "delapouite/first-aid-kit",
    "berries": "delapouite/berries-bowl",
    "mushrooms": "delapouite/mushrooms",
    "corn": "delapouite/corn",
}

# The wire carries an item's DISPLAY NAME, not its content id
# (`protocol::ItemCatalog` is names only), so the file a cell looks for is
# keyed off the name normalised the same way on both sides. 21 of the 48
# differ from their id — `fat` is "Animal Fat", `lowgrade` is "Low Grade
# Fuel" — so this is derived from `content/items.toml` rather than typed out,
# and a rename that broke it would break it loudly here instead of silently
# in a cell.
ITEMS_TOML = (ROOT / "content/items.toml").read_text()
PAIRS = re.findall(r'id = "item\.([a-z0-9_]+)"\s*\n(?:.*\n)*?name = "([^"]+)"', ITEMS_TOML)
BY_ID = {i: n for i, n in PAIRS}
if len(BY_ID) != len(ITEMS):
    sys.exit(f"content has {len(BY_ID)} items, the map has {len(ITEMS)}")


def norm(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")


unknown = sorted(set(ITEMS) - set(BY_ID))
if unknown:
    sys.exit(f"map names items the content does not have: {unknown}")

ALL = dict(SHAPES)
for item_id, icon in ITEMS.items():
    ALL[norm(BY_ID[item_id])] = icon

missing = [(k, v) for k, v in ALL.items() if not (SRC / f"{v}.svg").is_file()]
if missing:
    for k, v in missing:
        print(f"MISSING  {k} -> {v}", file=sys.stderr)
    sys.exit(f"{len(missing)} icon(s) not in the archive")

OUT.mkdir(parents=True, exist_ok=True)
for name, path in sorted(ALL.items()):
    cairosvg.svg2png(
        url=str(SRC / f"{path}.svg"),
        write_to=str(OUT / f"{name}.png"),
        output_width=PX,
        output_height=PX,
    )

# The attribution the CC BY 3.0 licence requires, per author actually used.
authors = sorted({v.split("/")[0] for v in ALL.values()})
credits = OUT / "CREDITS.md"
credits.write_text(
    "# Icon credits\n\n"
    "The PNGs in this directory are rasterised from **game-icons.net**,\n"
    "released under the **Creative Commons Attribution 3.0 Unported**\n"
    "licence (CC BY 3.0). The licence requires that the original authors be\n"
    "credited, which is what this file is for — it ships in the depot beside\n"
    "the icons, and `crates/client/tests/ui.rs` fails if it stops shipping.\n\n"
    "Icons made by:\n\n"
    + "".join(f"- {a}\n" for a in authors)
    + "\nAvailable on https://game-icons.net\n\n"
    "Licence: https://creativecommons.org/licenses/by/3.0/\n\n"
    "## What maps to what\n\n"
    "| file | source icon |\n|---|---|\n"
    + "".join(f"| `{k}.png` | `{v}` |\n" for k, v in sorted(ALL.items()))
)

print(f"baked {len(ALL)} icons at {PX}px into {OUT}")
print(f"authors: {', '.join(authors)}")
