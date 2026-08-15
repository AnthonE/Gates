#!/usr/bin/env python3
"""Rasterise the icons the client ships out of the game-icons.net archive.

Run when an item is added to `content/items.toml` or a mapping below changes:

    python3 ci/bake_icons.py                 # fetches from github.com/game-icons/icons
    GAME_ICONS_DIR=/tmp/game-icons ...       # or reads a local unzipped archive

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
import urllib.request

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

# **The archive is now the fallback, not the requirement**, and that is worth
# a paragraph because the old shape cost real work. `game-icons.net` is behind
# some environments' egress policy (this repo has hit that block on every 3D
# asset host — `DECISIONS.md` 2026-08-07), so "fetch the zip by hand first"
# made adding an item impossible from those boxes; two icons were hand-drawn
# on 2026-08-08 for exactly that reason and then thrown away when someone
# asked the obvious question. The project publishes its sources on GitHub,
# which is reachable wherever `git push` is.
#
# The repo's SVGs are black-background + white path, where the archive ships a
# pre-recoloured white-on-transparent variant — so the full-canvas background
# rect is stripped on the way through. Verified equivalent rather than assumed:
# baking `delapouite/corn` from GitHub reproduces the committed
# `assets/icons/corn.png` to within two antialiased pixels.
GH = "https://raw.githubusercontent.com/game-icons/icons/master"
BG_RECT = re.compile(r'<path d="M0 0h512v512H0z"\s*/>')


def source_svg(path):
    """The icon's SVG text, from the local archive or from GitHub."""
    local = SRC / f"{path}.svg"
    if local.is_file():
        return local.read_text()
    with urllib.request.urlopen(f"{GH}/{path}.svg", timeout=60) as r:
        return BG_RECT.sub("", r.read().decode(), count=1)
OUT = ROOT / "assets/icons"
PX = 128

# The six building shapes the shape wheel draws. **Still no material icons**,
# and the hammer wheel landing is what settles that: the wheel is one ring
# since 2026-08-07 — the blueprint places the bottom rung and the hammer
# climbs the ladder — so `mat_wood`/`mat_stone`/`mat_metal` had no draw site.
# This comment used to say they came back with the hammer wheel; the hammer
# wheel is here and they did not, because `structure::next_material` picks the
# rung and never asks the player which. They come back with a verb that offers
# a choice, or not at all.
SHAPES = {
    "shape_foundation": "delapouite/flat-platform",
    "shape_wall": "delapouite/brick-wall",
    "shape_doorway": "delapouite/door",
    # Catalogue v1's two openings. The window is the obvious pick; the
    # wall frame is a GATE on purpose — a big opening something else will
    # fill — and not `broken-wall`, for the rule the hammer verbs already
    # follow: it shares `brick-wall`'s drawing family and a shape that
    # looks like another shape is the positional trap wearing a menu.
    "shape_window": "delapouite/window",
    "shape_wall_frame": "delapouite/gate",
    "shape_floor": "delapouite/floor-hatch",
    "shape_stairs": "delapouite/3d-stairs",
    "shape_roof": "delapouite/great-pyramid",
}

# The four verbs on the hammer's wheel, keyed as `ui::hammer::verb_icon`
# names them.
#
# **Chosen at the size they are drawn, not off the archive's preview page.**
# A wedge glyph is a 38 px node fed by a 128 px bake and tinted flat — red on
# the cream band, near-white on the chosen wedge — so the only question that
# matters is what survives that, and it was answered by rendering the
# candidates under both tints before picking (`delapouite/monkey-wrench` and
# `skoll/open-palm` both read fine at 512 and go thin and characterless at
# 38). The second criterion is that the four differ from EACH OTHER in
# silhouette, because the ring is scanned at a flick: chevrons, a circle, a
# tower, a hand.
#
# `delapouite/broken-wall` lost demolish on a rule rather than a look — it is
# the same drawing family as `shape_wall`'s `delapouite/brick-wall`, and a
# verb that looks like a shape is the positional-payload trap wearing a menu.
VERBS = {
    "verb_upgrade": "delapouite/upgrade",
    "verb_repair": "lorc/auto-repair",
    "verb_demolish": "lorc/demolish",
    "verb_pick_up": "lorc/grab",
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
    "recycler": "lorc/recycle",
    # The blueprint you unroll, not the bench you unroll it on: a table at
    # 44 px is a rectangle, and the thing the verb produces reads.
    "research_table": "lorc/scroll-unfurled",
    # The coin that pays the ferryman (DESIGN.md §3.1). Two coins rather
    # than one because a single disc at 44 px reads as a full stop.
    "obol": "delapouite/two-coins",
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
    # The food loop's pair. A flat marbled cut and a drumstick are the two
    # most different meat silhouettes in the set, which is what a hotbar
    # holding both at 44 px needs — and a drumstick is the universal
    # game-shorthand for cooked, so the pair reads raw/cooked without a
    # label. (`delapouite/hot-meal`, a steaming cloche, says *cooked* more
    # loudly and *meat* not at all; it lost on that.)
    "raw_meat": "delapouite/steak",
    "cooked_meat": "lorc/chicken-leg",
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
# The icons that are OURS — one, now, and for a reason that outlived the
# one it was written for.
#
# `raw_meat`, `cooked_meat` and `burnt_meat` were all hand-drawn on
# 2026-08-08 because game-icons.net was behind this session's egress policy
# and the food loop needed pictures the day it landed. **That reason is
# gone**: the project publishes its sources on GitHub, which is reachable,
# and `source_svg` above fetches from there — so the first two are archive
# icons again (`delapouite/steak`, `lorc/chicken-leg`) and their SVGs are
# deleted.
#
# `burnt_meat` stays ours, and this is the honest reason rather than the
# inherited one: **the archive has no burnt-meat icon.** Probed
# `burning-meat`, `carbonized-material`, `ash-cloud` and four more; the
# nearest hits are `lorc/burning-embers` and `lorc/fire-silhouette`, neither
# of which reads as food. A drawing of ours beats a picture of the wrong
# thing.
#
# It is kept in a SEPARATE map rather than dropped into `ITEMS` because the
# CC BY notice below is generated from that map: sweeping our own art into a
# credit table would claim a licence over a file the licence does not cover,
# which is the opposite of what a notice licence asks for.
OURS = {
    "burnt_meat": "burnt_meat",
}
OURS_SRC = ROOT / "ci/icons"

ITEMS_TOML = (ROOT / "content/items.toml").read_text()
PAIRS = re.findall(r'id = "item\.([a-z0-9_]+)"\s*\n(?:.*\n)*?name = "([^"]+)"', ITEMS_TOML)
BY_ID = {i: n for i, n in PAIRS}
if len(BY_ID) != len(ITEMS) + len(OURS):
    sys.exit(
        f"content has {len(BY_ID)} items, the maps have "
        f"{len(ITEMS)} archive + {len(OURS)} ours"
    )


def norm(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")


unknown = sorted((set(ITEMS) | set(OURS)) - set(BY_ID))
if unknown:
    sys.exit(f"map names items the content does not have: {unknown}")

ALL = dict(SHAPES)
ALL.update(VERBS)
for item_id, icon in ITEMS.items():
    ALL[norm(BY_ID[item_id])] = icon
MINE = {norm(BY_ID[item_id]): f for item_id, f in OURS.items()}
# Bake a subset by stem when asked: `python3 ci/bake_icons.py raw_meat`.
#
# **Prefer that to a full run when you are adding an item.** The committed
# PNGs were baked from the downloaded archive, whose SVGs are the same
# artwork with slightly different path data; re-rendering them through the
# GitHub source changes ~20 files by a pixel or two along hard edges with no
# visual difference at all (measured 2026-08-08). That churn is noise in a
# diff about one item, and re-baking the set is a deliberate act rather than
# a side effect of adding a picture.
only = set(sys.argv[1:])
todo = {k: v for k, v in ALL.items() if not only or k in only}
todo_mine = {k: v for k, v in MINE.items() if not only or k in only}

OUT.mkdir(parents=True, exist_ok=True)
failed = []
for name, path in sorted(todo.items()):
    try:
        svg = source_svg(path)
    except Exception as e:  # network or a name the project renamed
        failed.append((name, path, e))
        continue
    cairosvg.svg2png(
        bytestring=svg.encode(),
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
if failed:
    for name, path, e in failed:
        print(f"MISSING  {name} -> {path}: {e}", file=sys.stderr)
    sys.exit(f"{len(failed)} icon(s) could not be sourced")

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
    "Ours, authored in `ci/icons/` and rasterised by the same script. **No\n"
    "attribution is owed for these and the CC BY notice above does not cover\n"
    "them** — they are listed so the line between what the licence covers and\n"
    "what it does not is written down rather than inferred from a table.\n\n"
    "Each is here because the archive has no icon for the thing, not because\n"
    "the archive was unreachable — that was true for one day and is not any\n"
    "more (`ci/bake_icons.py` fetches from GitHub).\n\n"
    "| file | source |\n|---|---|\n"
    + "".join(f"| `{k}.png` | `ci/icons/{v}.svg` (ours) |\n" for k, v in sorted(MINE.items()))
)

print(f"baked {len(todo)} archive + {len(todo_mine)} own icons at {PX}px into {OUT}")
print(f"authors: {', '.join(authors)}")
