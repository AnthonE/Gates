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
    "lock_code": "delapouite/dial-padlock",
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

# THE SECOND SOURCE, AND WHY IT IS NOT THE FIRST
#
# game-icons.net's zip is unreachable from this box (the egress proxy
# refuses the domain), but the project's own GitHub repo is not:
#
#   git clone --depth 1 https://github.com/game-icons/icons /tmp/gi
#
# Its SVGs are the same drawings in their source form — a black background
# rect followed by a white icon path — so the archive's
# `ffffff/transparent` variant is that file with the rect removed:
#
#   svg.replace('<path d="M0 0h512v512H0z"/>', "", 1)
#
# **Measured, not assumed**: rasterised that way at PX, 35 of the 54
# committed PNGs come back BYTE-IDENTICAL and 19 differ. The 19 are
# upstream art drift — the drawings have been revised since the archive
# snapshot these were baked from — which is exactly why this is a source
# for a NEW icon and **not** a re-bake of the set. Running the whole map
# through GitHub would silently redraw a third of the client's icons in a
# commit about something else.
#
# So: the zip above stays the source of record, and the clone is what you
# reach for when one row is added and the zip is out of reach. Say which
# you used in the commit.

# The wire carries an item's DISPLAY NAME, not its content id
# (`protocol::ItemCatalog` is names only), so the file a cell looks for is
# keyed off the name normalised the same way on both sides. 21 of the 48
# differ from their id — `fat` is "Animal Fat", `lowgrade` is "Low Grade
# Fuel" — so this is derived from `content/items.toml` rather than typed out,
# and a rename that broke it would break it loudly here instead of silently
# in a cell.
# The icons that are OURS, and the reason they exist.
#
# `raw_meat` and `cooked_meat` are not in the archive's table because they
# could not be rasterised from it: game-icons.net is behind this session's
# egress policy, the same block `DECISIONS.md` 2026-08-07 records for every
# 3D asset host, and the food loop needed two pictures the day it landed.
# `burnt_meat` followed by the same route for the same reason — the burnt
# state's item could not land without a picture (NOW.md §0v records the
# park), and the archive is still unreachable. They are authored SVGs in
# `ci/icons/`, committed so they regenerate, drawn in the archive's own
# 512-unit box at the archive's own weight.
#
# They are kept in a SEPARATE map rather than dropped into `ITEMS` because
# the CC BY notice below is generated from that map: sweeping our own art
# into a credit table would claim a licence over files the licence does not
# cover, which is the opposite of what a notice licence asks for.
OURS = {
    "raw_meat": "raw_meat",
    "cooked_meat": "cooked_meat",
    "burnt_meat": "burnt_meat",
}
OURS_SRC = ROOT / "ci/icons"

ITEMS_TOML = (ROOT / "content/items.toml").read_text()
PAIRS = re.findall(r'id = "item\.([a-z0-9_]+)"\s*\n(?:.*\n)*?name = "([^"]+)"', ITEMS_TOML)
BY_ID = {i: n for i, n in PAIRS}
if len(BY_ID) != len(ITEMS) + len(OURS):
    sys.exit(
        f"content has {len(BY_ID)} items, the maps have "
        f"{len(ITEMS)} + {len(OURS)} ours"
    )


def norm(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")


unknown = sorted((set(ITEMS) | set(OURS)) - set(BY_ID))
if unknown:
    sys.exit(f"map names items the content does not have: {unknown}")

ALL = dict(SHAPES)
for item_id, icon in ITEMS.items():
    ALL[norm(BY_ID[item_id])] = icon
MINE = {norm(BY_ID[item_id]): f for item_id, f in OURS.items()}

# Bake a subset by stem when asked. The archive is a hand-fetched zip that
# this box cannot reach, so re-baking our own two must not require it —
# `python3 ci/bake_icons.py raw_meat cooked_meat` does exactly that.
only = set(sys.argv[1:])
todo = {k: v for k, v in ALL.items() if not only or k in only}
todo_mine = {k: v for k, v in MINE.items() if not only or k in only}

missing = [(k, v) for k, v in todo.items() if not (SRC / f"{v}.svg").is_file()]
if missing:
    for k, v in missing:
        print(f"MISSING  {k} -> {v}", file=sys.stderr)
    sys.exit(f"{len(missing)} icon(s) not in the archive")

OUT.mkdir(parents=True, exist_ok=True)
for name, path in sorted(todo.items()):
    cairosvg.svg2png(
        url=str(SRC / f"{path}.svg"),
        write_to=str(OUT / f"{name}.png"),
        output_width=PX,
        output_height=PX,
    )
for name, path in sorted(todo_mine.items()):
    cairosvg.svg2png(
        url=str(OURS_SRC / f"{path}.svg"),
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
    + "\n## Not from game-icons.net\n\n"
    "These are **ours**, authored in `ci/icons/` and rasterised by the same\n"
    "script. No attribution is owed for them and the CC BY notice above does\n"
    "not cover them — they are listed here so the line between what the\n"
    "licence covers and what it does not is written down rather than\n"
    "inferred from a table.\n\n"
    "They exist because game-icons.net is unreachable from the environment\n"
    "the food loop landed in (`DECISIONS.md` 2026-08-07 records the same\n"
    "block for every 3D asset host), and two pictures were needed that day.\n"
    "Replacing them with archive icons later is a mapping move in\n"
    "`ci/bake_icons.py` and nothing else.\n\n"
    "| file | source |\n|---|---|\n"
    + "".join(f"| `{k}.png` | `ci/icons/{v}.svg` (ours) |\n" for k, v in sorted(MINE.items()))
)

print(f"baked {len(todo)} archive + {len(todo_mine)} own icons at {PX}px into {OUT}")
print(f"authors: {', '.join(authors)}")
