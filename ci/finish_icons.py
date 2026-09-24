#!/usr/bin/env python3
"""Make the full-colour item icons the client ships.

    python3 ci/finish_icons.py              # every item
    python3 ci/finish_icons.py wood rock    # a subset, by stem

Item icons were white game-icons.net silhouettes tinted at the draw, so every
cell in the inventory was the same off-white. Rust's are colour pictures of
the item. This makes ours colour pictures too, from two sources:

- `ci/icons/renders/<stem>.png`: the item's own 3D model, photographed by
  `cargo run -p client --features render --bin iconbake -- ci/icons/renders`
  (512 px, transparent). Used wherever a model exists. It is the thing the
  player holds or places, lit and framed the way Rust frames its icons.
- `ci/icons/masks/<stem>.png`: the game-icons.net silhouette `bake_icons.py`
  rasterised, for items with no model yet. Painted here: the item's colours
  as a top-lit gradient, a bevel lit from the upper left (the renders' key
  light), and enclosed holes filled with a second colour.

Both get the same finish, a soft drop shadow, and land in
`assets/icons/<stem>.png` at 128 px. The UI glyphs (shape_*, verb_*, vital_*,
map_*, ui_*) are not items and stay white; the client tints those.

Needs numpy, Pillow and scipy; `pngquant` on PATH shrinks the output (see
`shrink`). Deterministic: same inputs and tools, same bytes.
"""
import pathlib
import shutil
import subprocess
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = pathlib.Path(__file__).resolve().parent.parent
RENDERS = ROOT / "ci/icons/renders"
MASKS = ROOT / "ci/icons/masks"
OUT = ROOT / "assets/icons"
PX = 128  # shipped edge
WORK = 512  # painting edge; four samples per shipped pixel

# Light for the painted bevel, in image space (x right, y DOWN, z toward the
# viewer): high and to the left, the same side as `iconbake`'s key light, so
# a painted icon and a rendered one sit in the same room.
LIGHT = np.array([-0.55, -0.75, 0.62])
LIGHT = LIGHT / np.linalg.norm(LIGHT)
HALF = LIGHT + np.array([0.0, 0.0, 1.0])
HALF = HALF / np.linalg.norm(HALF)


def hexc(h):
    """'#rrggbb' -> linear RGB."""
    h = h.lstrip("#")
    c = np.array([int(h[i : i + 2], 16) for i in (0, 2, 4)], dtype=np.float64) / 255
    return srgb_to_linear(c)


# What each painted item is made of: its top and bottom colour (sRGB hex), and
# optionally a colour for the holes the silhouette encloses (a medkit's cross,
# a lock's dial), `metal` for a sharper highlight, and a `zone` painted in a
# second pair. A zone is the silhouette narrowed by `thick` — kept where a
# disc of that radius (px at WORK) fits, which finds an axe's head and drops
# its haft — and/or by `box`, a rectangle over the shape's own extent
# (x0, y0, x1, y1 in 0..1). Items with a render are not here: a render wins.
WOOD = ("#c79258", "#7a4a26")
STONE = ("#b8b2aa", "#625d57")
STEEL = ("#dde2e8", "#7c8591")
DARK_STEEL = ("#a7aeb8", "#4c535d")
FLAME = ("#ffd35c", "#e0501c")
PAINT = {
    "wood": dict(c=WOOD),
    "cloth": dict(c=("#e2d3ae", "#a8936a")),
    "animal_fat": dict(c=("#f6e7c1", "#c9a468")),
    "charcoal": dict(c=("#767a84", "#27282d"), metal=True),
    "metal_fragments": dict(c=STEEL, metal=True),
    "sulfur": dict(c=("#e4d9b0", "#a8946a"), holes="#f2dc3c"),
    "gunpowder": dict(c=("#77777d", "#303036")),
    "low_grade_fuel": dict(c=("#e8563c", "#98281a"), metal=True),
    "gears": dict(c=STEEL, metal=True),
    "rope": dict(c=("#dcbc80", "#94703c")),
    "tarp": dict(c=("#8ea865", "#4b6232")),
    "junk": dict(c=("#c5cad1", "#6b727c"), metal=True),
    "blueprint": dict(c=("#7cb4f0", "#2d5c9c")),
    "torch": dict(c=WOOD, zone=dict(c=FLAME, box=(0.5, 0.0, 1.0, 0.45))),
    "wooden_spear": dict(c=WOOD),
    "stone_hatchet": dict(c=WOOD, zone=dict(c=STONE, thick=26, box=(0.0, 0.0, 1.0, 0.55))),
    "stone_pickaxe": dict(c=WOOD, zone=dict(c=STONE, box=(0.0, 0.0, 1.0, 0.40))),
    "hunting_bow": dict(c=WOOD),
    "wooden_arrow": dict(c=WOOD),
    "bandage": dict(c=("#f7f2e8", "#c8bca4")),
    "fire_pit": dict(c=FLAME),
    "workbench_2": dict(c=DARK_STEEL, metal=True),
    "workbench_3": dict(c=("#f0d488", "#9a7428"), metal=True),
    "metal_hatchet": dict(
        c=WOOD, zone=dict(c=STEEL, thick=30, box=(0.0, 0.0, 1.0, 0.62)), metal=True
    ),
    "metal_pickaxe": dict(c=WOOD, zone=dict(c=STEEL, box=(0.0, 0.0, 1.0, 0.40)), metal=True),
    "metal_spear": dict(c=STEEL, metal=True),
    "furnace": dict(c=("#a39d96", "#57524d"), holes="#ff9a3a"),
    "code_lock": dict(c=DARK_STEEL, holes="#e8eef5", metal=True),
    "recycler": dict(c=("#86d06a", "#3c8a34")),
    "research_table": dict(c=("#f1e3bd", "#c1a46c")),
    "wooden_door": dict(c=WOOD),
    "building_plan": dict(c=("#7cb4f0", "#2d5c9c")),
    "hammer": dict(
        c=WOOD, zone=dict(c=DARK_STEEL, thick=26, box=(0.0, 0.0, 1.0, 0.5)), metal=True
    ),
    "burlap_hood": dict(c=("#d2b985", "#8b7044")),
    "burlap_tunic": dict(c=("#d2b985", "#8b7044")),
    "metal_arrow": dict(c=STEEL, metal=True),
    "crossbow": dict(c=WOOD),
    "revolver": dict(c=DARK_STEEL, metal=True),
    "pistol_round": dict(c=("#f3d27a", "#a47a1e"), metal=True),
    "satchel_charge": dict(c=("#e85a44", "#9a2a1c")),
    "metal_door": dict(c=DARK_STEEL, metal=True),
    "roadsign_vest": dict(c=("#e9d56a", "#9c8622"), metal=True),
    "medkit": dict(c=("#ec5b4f", "#a72620"), holes="#fbf6ee"),
    "berries": dict(c=("#caa476", "#7c5a36"), holes="#c2385e"),
    "mushrooms": dict(c=("#ecd6ae", "#a37a4c")),
    "corn": dict(c=("#f6d751", "#b1861a")),
    "raw_meat": dict(c=("#e5776d", "#9d2f33"), holes="#f3d8cc"),
    "cooked_meat": dict(c=("#c98449", "#6c3a1a")),
    "burnt_meat": dict(c=("#6a5448", "#241a16")),
    "metal_window_bars": dict(c=DARK_STEEL, metal=True),
    "garage_door": dict(c=("#b7c0ca", "#5f6874"), metal=True),
    "glass_window": dict(c=WOOD, holes="#a9dcef"),
    "wood_shutters": dict(c=WOOD),
}


# How much of the square a render's subject fills, where 0.90 is wrong. The
# small and large box share one model, so the small one is drawn smaller.
FILL = {
    "small_box": 0.70,
}
# Linear gain on every render (see `from_render`).
RENDER_GAIN = 1.4
# Items drawn from another item's render: the same model, one file.
RENDER_OF = {
    "small_box": "large_box",
}


def srgb_to_linear(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(np.asarray(c, dtype=np.float64), 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def resize(chan, size):
    """One float channel, Lanczos."""
    im = Image.fromarray(chan.astype(np.float32), mode="F")
    return np.asarray(im.resize((size, size), Image.LANCZOS), dtype=np.float64)


def from_render(path, fill):
    """A render as (premultiplied linear RGB, alpha) at WORK, cropped to a
    square around what was drawn with the subject filling `fill` of it.

    `iconbake` frames by bounding box, which is loose for anything lying on a
    diagonal, so the tight frame is found here from the pixels themselves.
    The frame is resolved over a transparent black clear, so an edge pixel's
    colour already carries its coverage: it is premultiplied as it stands.
    """
    px = np.asarray(Image.open(path).convert("RGBA"), dtype=np.float64) / 255
    rgb, a = srgb_to_linear(px[..., :3]), px[..., 3]
    ys, xs = np.where(a > 2 / 255)
    if len(xs) == 0:
        sys.exit(f"{path}: the render is empty")
    side = max(xs.max() - xs.min() + 1, ys.max() - ys.min() + 1) / fill
    cx, cy = (xs.min() + xs.max() + 1) / 2, (ys.min() + ys.max() + 1) / 2
    x0, y0 = int(round(cx - side / 2)), int(round(cy - side / 2))
    n = int(round(side))
    pad = n
    rgb = np.pad(rgb, ((pad, pad), (pad, pad), (0, 0)))
    a = np.pad(a, pad)
    rgb = rgb[y0 + pad : y0 + pad + n, x0 + pad : x0 + pad + n]
    a = a[y0 + pad : y0 + pad + n, x0 + pad : x0 + pad + n]
    rgb = np.stack([resize(rgb[..., i], WORK) for i in range(3)], axis=-1)
    # Half a stop up: a render is lit like the world, and a cell is darker
    # than the world — the painted icons are keyed brighter for the same reason.
    return np.clip(rgb * RENDER_GAIN, 0.0, None), np.clip(resize(a, WORK), 0.0, 1.0)


def paint(path, spec):
    """A silhouette as (premultiplied linear RGB, alpha) at WORK."""
    m = np.asarray(Image.open(path).convert("RGBA"), dtype=np.float64)[..., 3] / 255
    alpha = np.clip(resize(m, WORK), 0.0, 1.0)
    inside = alpha > 0.5

    # Enclosed background — the cross in a medkit, a lock's dial. Found as
    # the background regions that do not reach the frame's edge.
    bg = ~inside
    labels, n = ndimage.label(bg)
    edge = set(np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]])))
    holes = bg & ~np.isin(labels, list(edge))
    if spec.get("holes") is None:
        holes[:] = False
    solid = inside | holes

    # A height field that rises over the first few pixels inside every edge,
    # holes included, so the painted edges bevel the way a lit object's do.
    d = ndimage.distance_transform_edt(inside)
    bevel = WORK * 0.028
    h = np.clip(d / bevel, 0.0, 1.0)
    h = h * h * (3 - 2 * h)
    h = np.where(holes, 0.35, h)
    h = ndimage.gaussian_filter(h, WORK / 340)
    gy, gx = np.gradient(h)
    k = bevel * 1.6
    nrm = np.stack([-gx * k, -gy * k, np.ones_like(h)], axis=-1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)
    diff = np.clip(nrm @ LIGHT, 0.0, 1.0)
    spec_pow, spec_amt = (28.0, 0.55) if spec.get("metal") else (14.0, 0.18)
    shine = np.clip(nrm @ HALF, 0.0, 1.0) ** spec_pow * spec_amt

    # The colour: a top-to-bottom gradient over the shape's own extent, so a
    # short item and a tall one get the whole ramp.
    rows = np.where(solid.any(axis=1))[0]
    y0, y1 = (rows[0], rows[-1]) if len(rows) else (0, WORK - 1)
    yy = np.clip((np.arange(WORK, dtype=np.float64) - y0) / max(1, y1 - y0), 0, 1)
    t = np.repeat(yy[:, None], WORK, axis=1)[..., None]
    top, bot = (hexc(x) for x in spec["c"])
    col = top * (1 - t) + bot * t
    if "zone" in spec:
        z = spec["zone"]
        zm = solid.copy()
        if "thick" in z:
            r = int(z["thick"])
            oy, ox = np.ogrid[-r : r + 1, -r : r + 1]
            head = ndimage.binary_opening(inside, structure=ox * ox + oy * oy <= r * r)
            # Grown back over the edge the opening rounded off, so the head
            # meets the haft at the silhouette's own line and not inside it.
            zm &= ndimage.binary_dilation(head, iterations=4)
        if "box" in z:
            bx0, by0, bx1, by1 = z["box"]
            cols = np.where(solid.any(axis=0))[0]
            x0, x1 = (cols[0], cols[-1]) if len(cols) else (0, WORK - 1)
            xs = (np.arange(WORK) - x0) / max(1, x1 - x0)
            ys = (np.arange(WORK) - y0) / max(1, y1 - y0)
            zm &= ((ys >= by0) & (ys <= by1))[:, None] & ((xs >= bx0) & (xs <= bx1))[None, :]
        zm = ndimage.gaussian_filter(zm.astype(np.float64), WORK / 256)[..., None]
        ztop, zbot = (hexc(x) for x in z["c"])
        col = col * (1 - zm) + (ztop * (1 - t) + zbot * t) * zm
    if spec.get("holes") is not None:
        hc = hexc(spec["holes"])
        hm = ndimage.gaussian_filter(holes.astype(np.float64), WORK / 400)[..., None]
        col = col * (1 - hm) + hc * hm
        alpha = np.maximum(alpha, np.clip(ndimage.gaussian_filter(holes * 1.0, 1.0), 0, 1))

    # The form: ambient plus the key, then the highlight on top. A darker
    # rim inside the edge reads as the object's own thickness at 34 px.
    shade = 0.52 + 0.62 * diff[..., None]
    rim = np.clip(d / (bevel * 0.35), 0.0, 1.0)
    rim = np.where(holes, 1.0, rim)
    shade *= (0.55 + 0.45 * rim)[..., None]
    rgb = col * shade + shine[..., None]
    return rgb * alpha[..., None], alpha


# Rust's icons carry a solid near-black outline (~#161c22, 4-9 px at 512),
# which is what keeps a dark item legible on a dark slot. Ours is 5 px at
# WORK, about 1.25 at the shipped size.
OUTLINE_PX = 5
INK = "#161c22"


def finish(premul, alpha):
    """WORK premultiplied linear -> the shipped RGBA8, with its outline and
    its shadow."""
    oy, ox = np.ogrid[-OUTLINE_PX : OUTLINE_PX + 1, -OUTLINE_PX : OUTLINE_PX + 1]
    ring = ndimage.grey_dilation(alpha, footprint=ox * ox + oy * oy <= OUTLINE_PX**2)
    ring = np.clip(ndimage.gaussian_filter(ring, 0.8), 0.0, 1.0)
    under = ring * (1 - alpha)
    premul = premul + hexc(INK) * under[..., None]
    alpha = alpha + under
    small = np.stack([resize(premul[..., i], PX) for i in range(3)], axis=-1)
    a = np.clip(resize(alpha, PX), 0.0, 1.0)
    small = np.clip(small, 0.0, None)
    # The drop shadow: down and to the right of the key light, soft, and
    # dark enough to keep a pale item off a pale cell.
    sh = ndimage.shift(ndimage.gaussian_filter(a, 1.6), (1.6, 1.0), order=1)
    sh = np.clip(sh, 0.0, 1.0) * 0.58
    out_a = a + sh * (1 - a)
    rgb = np.where(out_a[..., None] > 1e-6, small / np.maximum(out_a, 1e-6)[..., None], 0.0)
    rgb8 = np.round(linear_to_srgb(rgb) * 255).astype(np.uint8)
    a8 = np.round(np.clip(out_a, 0, 1) * 255).astype(np.uint8)
    return Image.fromarray(np.dstack([rgb8, a8]), mode="RGBA")


def shrink(path):
    """Palette the PNG with `pngquant` when it is installed.

    A colour icon is ~20 KB as truecolour and ~8 KB paletted, with no
    difference a player can see at 34 px (compared side by side at 2x on
    2026-09-24) — and the browser build downloads every one. Optional, so
    the script runs anywhere; `apt-get install pngquant` for the small ones.
    Exit 99 is pngquant declining a quality it cannot meet, which keeps the
    truecolour file.
    """
    if shutil.which("pngquant") is None:
        return
    subprocess.run(
        ["pngquant", "--quality=80-98", "--speed", "1", "--strip", "--force",
         "--output", str(path), str(path)],
        check=False,
    )


def main():
    only = set(sys.argv[1:])
    stems = sorted(set(PAINT) | {p.stem for p in RENDERS.glob("*.png")} | set(RENDER_OF))
    todo = [s for s in stems if not only or s in only]
    missing = sorted(only - set(stems))
    if missing:
        sys.exit(f"no source for: {missing}")
    OUT.mkdir(parents=True, exist_ok=True)
    for stem in todo:
        render = RENDERS / f"{RENDER_OF.get(stem, stem)}.png"
        mask = MASKS / f"{stem}.png"
        if render.is_file():
            premul, alpha = from_render(render, FILL.get(stem, 0.90))
            how = "render"
        elif mask.is_file():
            premul, alpha = paint(mask, PAINT[stem])
            how = "painted"
        else:
            sys.exit(f"{stem}: neither {render} nor {mask} exists")
        out = OUT / f"{stem}.png"
        finish(premul, alpha).save(out, optimize=True)
        shrink(out)
        print(f"{stem:<20} {how}")


if __name__ == "__main__":
    main()
