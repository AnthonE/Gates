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
import zlib

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
BONE = ("#f6efdc", "#b8a782")
PAINT = {
    "wood": dict(c=WOOD),
    "cloth": dict(mat="cloth", c=("#e2d3ae", "#a8936a")),
    "animal_fat": dict(mat="organic", c=("#f6e7c1", "#c9a468")),
    "charcoal": dict(mat="stone", c=("#767a84", "#27282d")),
    "metal_fragments": dict(c=STEEL, metal=True),
    "sulfur": dict(mat="cloth", c=("#e4d9b0", "#a8946a"), holes="#f2dc3c"),
    "gunpowder": dict(mat="powder", c=("#77777d", "#303036")),
    "low_grade_fuel": dict(c=("#e8563c", "#98281a"), metal=True),
    "gears": dict(c=STEEL, metal=True),
    "rope": dict(mat="cloth", c=("#dcbc80", "#94703c")),
    "tarp": dict(mat="cloth", c=("#8ea865", "#4b6232")),
    "junk": dict(c=("#c5cad1", "#6b727c"), metal=True),
    # Rust's components (2026-10-13): bare metal unless the part is painted.
    "metal_blade": dict(c=STEEL, metal=True),
    "empty_propane_tank": dict(c=("#d9dde2", "#6a737e"), metal=True),
    "sewing_kit": dict(mat="leather", c=("#d9a86c", "#8a5a2a")),
    "metal_spring": dict(c=STEEL, metal=True),
    "sheet_metal": dict(c=DARK_STEEL, metal=True),
    "metal_pipe": dict(c=DARK_STEEL, metal=True),
    "road_signs": dict(c=("#e0503e", "#8c2418"), metal=True),
    "semi_automatic_body": dict(c=DARK_STEEL, metal=True),
    "smg_body": dict(c=DARK_STEEL, metal=True),
    "rifle_body": dict(c=DARK_STEEL, metal=True),
    "tech_trash": dict(mat="plain", c=("#6cae64", "#2c5e2a"), metal=True),
    "electric_fuse": dict(c=("#e2c060", "#86641e"), metal=True),
    "blueprint": dict(mat="plain", c=("#7cb4f0", "#2d5c9c")),
    # Triple T's bat: honey-orange varnish, not the pale tool wood.
    "bat": dict(c=("#e8a25a", "#8f4512")),
    "torch": dict(c=WOOD, zone=dict(c=FLAME, box=(0.5, 0.0, 1.0, 0.45))),
    "wooden_spear": dict(c=WOOD),
    "stone_spear": dict(c=WOOD, zone=dict(c=STONE, box=(0.3, 0.3, 1.0, 1.0))),
    "stone_hatchet": dict(c=WOOD, zone=dict(c=STONE, thick=26, box=(0.0, 0.0, 1.0, 0.55))),
    "stone_pickaxe": dict(c=WOOD, zone=dict(c=STONE, box=(0.0, 0.0, 1.0, 0.40))),
    "hunting_bow": dict(c=WOOD),
    "wooden_arrow": dict(c=WOOD),
    "bandage": dict(c=("#f7f2e8", "#c8bca4")),
    "camp_fire": dict(c=FLAME),
    "waterskin": dict(mat="leather", c=("#b88a5c", "#6e4a2a")),
    "full_waterskin": dict(mat="leather", c=("#6fb4e0", "#2f6a96")),
    "wooden_barricade": dict(c=WOOD),
    "workbench_2": dict(c=DARK_STEEL, metal=True),
    "workbench_3": dict(c=("#f0d488", "#9a7428"), metal=True),
    "metal_hatchet": dict(
        c=WOOD, zone=dict(c=STEEL, thick=30, box=(0.0, 0.0, 1.0, 0.62)), metal=True
    ),
    "metal_pickaxe": dict(c=WOOD, zone=dict(c=STEEL, box=(0.0, 0.0, 1.0, 0.40)), metal=True),
    "metal_spear": dict(c=STEEL, metal=True),
    "furnace": dict(c=("#a39d96", "#57524d"), holes="#ff9a3a"),
    "code_lock": dict(c=DARK_STEEL, holes="#e8eef5", metal=True),
    "key_lock": dict(c=STEEL, metal=True),
    "recycler": dict(c=("#86d06a", "#3c8a34")),
    "research_table": dict(c=("#f1e3bd", "#c1a46c")),
    "wooden_door": dict(c=WOOD),
    "building_plan": dict(c=("#7cb4f0", "#2d5c9c")),
    "hammer": dict(
        c=WOOD, zone=dict(c=DARK_STEEL, thick=26, box=(0.0, 0.0, 1.0, 0.5)), metal=True
    ),
    "burlap_hood": dict(mat="cloth", c=("#d2b985", "#8b7044")),
    "burlap_tunic": dict(mat="cloth", c=("#d2b985", "#8b7044")),
    "metal_arrow": dict(c=STEEL, metal=True),
    "high_velocity_arrow": dict(c=("#d4e6f4", "#55799a"), metal=True),
    "bone_arrow": dict(c=WOOD, zone=dict(c=BONE, box=(0.44, 0.44, 1.0, 1.0))),
    "bone_fragments": dict(mat="bone", c=BONE),
    "leather": dict(mat="leather", c=("#b07a4a", "#5e3a1e")),
    "hide_poncho": dict(mat="leather", c=("#a8784c", "#5a3a20")),
    "bone_helmet": dict(mat="bone", c=BONE),
    "ash_mask": dict(mat="cloth", c=("#9a978e", "#45433c")),
    "ash_suit": dict(mat="cloth", c=("#9a978e", "#45433c"), holes="#2f4a3c"),
    # Clothing v1: Rust's clothes for every slot, and their teas.
    "burlap_trousers": dict(mat="cloth", c=("#d2b985", "#8b7044")),
    "burlap_shoes": dict(mat="cloth", c=("#d2b985", "#8b7044")),
    "burlap_gloves": dict(mat="cloth", c=("#d2b985", "#8b7044")),
    "hide_pants": dict(mat="leather", c=("#a8784c", "#5a3a20")),
    "hide_boots": dict(mat="leather", c=("#a8784c", "#5a3a20")),
    "hide_vest": dict(mat="leather", c=("#a8784c", "#5a3a20")),
    "hide_hood": dict(mat="leather", c=("#a8784c", "#5a3a20")),
    "leather_gloves": dict(mat="leather", c=("#8a5a34", "#3e2614")),
    "hoodie": dict(mat="cloth", c=("#8a96a4", "#3c4652")),
    "pants": dict(mat="cloth", c=("#6c84a4", "#2c3c54")),
    "jacket": dict(mat="cloth", c=("#7a8a5a", "#3a4626")),
    "snow_jacket": dict(mat="cloth", c=("#eef4fa", "#8ea4bc"), holes="#c84a3a"),
    "bandana": dict(mat="cloth", c=("#d4584a", "#842a20")),
    "improvised_balaclava": dict(mat="cloth", c=("#c6b286", "#6e5a34")),
    "beenie_hat": dict(mat="cloth", c=("#c45a3c", "#6e2a18")),
    "warming_tea": dict(c=("#f0a060", "#a04420"), holes="#7a2a16"),
    "strong_warming_tea": dict(c=("#e86a40", "#8a2a14"), holes="#5a1a0c"),
    "cooling_tea": dict(c=("#a8d8f0", "#3a7aa8"), holes="#1e4a6e"),
    "warding_tea": dict(c=("#8ac080", "#2e5a2c"), holes="#16301a"),
    "healing_tea": dict(mat="organic", c=("#e88aa0", "#9c3048")),
    "fire_arrow": dict(c=FLAME),
    "crossbow": dict(c=WOOD),
    "revolver": dict(c=DARK_STEEL, metal=True),
    # Weapon attachments (2026-10-23): our own silhouettes
    # (`ci/icons/masks/`), no game-icons source.
    "holosight": dict(c=DARK_STEEL, metal=True, holes="#5a1f1f"),
    "8x_zoom_scope": dict(c=("#3c4046", "#16181b"), metal=True),
    "silencer": dict(c=("#4a4e54", "#1d2024"), metal=True),
    "muzzle_brake": dict(c=DARK_STEEL, metal=True),
    "muzzle_boost": dict(c=("#c9a24a", "#6d531c"), metal=True),
    "weapon_lasersight": dict(c=("#3b3f45", "#17191c"), metal=True, holes="#ff4030"),
    "weapon_flashlight": dict(c=("#4d5258", "#1e2125"), metal=True),
    "extended_magazine": dict(c=("#5a5e64", "#24272b"), metal=True),
    "pistol_round": dict(c=("#f3d27a", "#a47a1e"), metal=True),
    "satchel_charge": dict(c=("#e85a44", "#9a2a1c")),
    "metal_door": dict(c=DARK_STEEL, metal=True),
    "roadsign_vest": dict(c=("#e9d56a", "#9c8622"), metal=True),
    "medkit": dict(c=("#ec5b4f", "#a72620"), holes="#fbf6ee"),
    "medical_syringe": dict(c=("#dfe9ef", "#6f8798"), metal=True),
    "charcoal_tonic": dict(c=("#3b3f38", "#121411"), holes="#cfe0d8"),
    "berries": dict(c=("#caa476", "#7c5a36"), holes="#c2385e"),
    "mushrooms": dict(c=("#ecd6ae", "#a37a4c")),
    "cactus_flesh": dict(mat="organic", c=("#a8cf6e", "#4d7a2e"), holes="#e4eeb8"),
    "corn": dict(c=("#f6d751", "#b1861a")),
    "corn_seed": dict(mat="organic", c=("#f2d27a", "#a9802a")),
    "hemp_seed": dict(mat="organic", c=("#b9a77a", "#6c5c34")),
    "planter_box": dict(mat="plain", c=("#86c45a", "#3f7a2a")),
    "auto_turret": dict(c=DARK_STEEL, metal=True),
    "vending_machine": dict(c=("#d8574a", "#7f2219"), metal=True, holes="#cfe6ee"),
    # Electricity's core kit (`sim_core::power`): copper for the wire, the
    # generator's hazard yellow, the panel's sun, a battery's green steel,
    # grey for the small parts and the lamp's warm white.
    "wire_tool": dict(c=("#e39a5b", "#8a4a1c"), metal=True),
    "small_generator": dict(c=("#e8c547", "#8f6d12"), metal=True),
    "large_solar_panel": dict(c=("#f6d55c", "#c47d16")),
    "small_battery": dict(c=("#8fc78a", "#2f6a34"), metal=True),
    "large_battery": dict(c=("#7fb37a", "#29572d"), metal=True),
    "switch": dict(c=DARK_STEEL, metal=True),
    "splitter": dict(c=STEEL, metal=True),
    "root_combiner": dict(c=STEEL, metal=True),
    "ceiling_light": dict(c=("#f4ead2", "#a8977a"), metal=True),
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


# What a painted surface is made of, when the colours do not say: the grain,
# the sheen and the texture a material carries. `mat=` on a PAINT row wins;
# otherwise metal rows are metal and the WOOD pair is wood (`material`).
MATS = {
    # amp: albedo variation; spec: (power, amount); kind: the texture.
    "plain": dict(amp=0.05, spec=(16.0, 0.14), kind="fbm"),
    "wood": dict(amp=0.16, spec=(14.0, 0.10), kind="grain"),
    "metal": dict(amp=0.045, spec=(34.0, 0.62), kind="brushed"),
    "cloth": dict(amp=0.10, spec=(8.0, 0.04), kind="weave"),
    "leather": dict(amp=0.10, spec=(18.0, 0.16), kind="fbm"),
    "stone": dict(amp=0.16, spec=(12.0, 0.10), kind="speckle"),
    "bone": dict(amp=0.08, spec=(16.0, 0.14), kind="fbm"),
    "organic": dict(amp=0.10, spec=(22.0, 0.22), kind="fbm"),
    "powder": dict(amp=0.20, spec=(8.0, 0.05), kind="speckle"),
}


def material(spec, pair=None):
    """The MATS row for a PAINT row (or for its zone, given the zone's pair)."""
    if "mat" in spec:
        return spec["mat"]
    pair = pair or spec["c"]
    if spec.get("metal") or pair in (STEEL, DARK_STEEL):
        return "metal"
    if pair == WOOD:
        return "wood"
    if pair == STONE:
        return "stone"
    if pair == BONE:
        return "bone"
    return "plain"


def principal_angle(mask):
    """The angle (radians, image space) of the shape's long axis."""
    ys, xs = np.nonzero(mask)
    if len(xs) < 2:
        return 0.0
    xs = xs - xs.mean()
    ys = ys - ys.mean()
    cov = np.cov(np.stack([xs, ys]))
    w, v = np.linalg.eigh(cov)
    ax = v[:, np.argmax(w)]
    return float(np.arctan2(ax[1], ax[0]))


def noise(rng, sigma, aniso=None):
    """Smooth unit noise at WORK: white noise blurred by `sigma` px, or along
    and across angle `aniso[0]` by `aniso[1:]` for a grain."""
    n = rng.standard_normal((WORK, WORK))
    if aniso is None:
        n = ndimage.gaussian_filter(n, sigma, mode="wrap")
    else:
        ang, along, across = aniso
        deg = np.degrees(ang)
        n = ndimage.rotate(n, deg, reshape=False, order=1, mode="wrap")
        n = ndimage.gaussian_filter(n, (across, along), mode="wrap")
        n = ndimage.rotate(n, -deg, reshape=False, order=1, mode="wrap")
    n -= n.mean()
    return n / max(n.std(), 1e-9)


def texture(kind, rng, angle):
    """A zero-mean, roughly unit field of the material's surface."""
    if kind == "grain":
        streak = noise(rng, 0, (angle, WORK * 0.10, 1.2))
        figure = noise(rng, WORK * 0.03)
        return np.tanh(streak * 0.9 + figure * 0.4)
    if kind == "brushed":
        return 0.6 * noise(rng, 0, (angle, WORK * 0.06, 1.2)) + 0.4 * noise(rng, WORK * 0.02)
    if kind == "weave":
        yy, xx = np.mgrid[0:WORK, 0:WORK].astype(np.float64)
        p = WORK / 40.0
        w = np.sin(xx * np.pi / p) * np.sin(yy * np.pi / p)
        return 0.55 * w + 0.45 * noise(rng, WORK * 0.012)
    if kind == "speckle":
        return 0.55 * noise(rng, 1.1) + 0.45 * noise(rng, WORK * 0.02)
    return 0.6 * noise(rng, WORK * 0.012) + 0.4 * noise(rng, WORK * 0.05)


def paint(path, spec, stem=""):
    """A silhouette as (premultiplied linear RGB, alpha) at WORK.

    Lit as a solid rather than a flat badge: the shape is a rounded slab (a
    quarter-round edge, a broad dome over its middle), lit by the renders'
    key light, a sky from above and a ground bounce from below, darkened in
    its creases, and its surface carries its material's grain or sheen.
    That is what keeps a painted item in the same family as the renders
    beside it in a cell.
    """
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

    # The height: a quarter-round over the first `edge_r` px inside every
    # edge, holes included, plus a broad dome so a wide shape is lit across
    # its middle and not only at its rim.
    d = ndimage.distance_transform_edt(inside)
    edge_r = WORK * 0.035
    e = np.clip(d / edge_r, 0.0, 1.0)
    rounded = np.sqrt(np.clip(1.0 - (1.0 - e) ** 2, 0.0, 1.0))
    dome = ndimage.gaussian_filter(inside.astype(np.float64), WORK * 0.06)
    h = rounded * edge_r * 0.9 + dome * edge_r * 1.4
    h = np.where(holes, edge_r * 0.25, h)
    h = ndimage.gaussian_filter(h, WORK / 400)

    rng = np.random.default_rng(zlib.crc32(stem.encode()))
    angle = principal_angle(inside)
    mats = {}

    def surface(mat):
        if mat not in mats:
            mats[mat] = texture(MATS[mat]["kind"], rng, angle)
        return mats[mat]

    # A little bump from the base material's texture, so grain catches light.
    base_mat = material(spec)
    h = h + surface(base_mat) * WORK * 0.0012
    gy, gx = np.gradient(h)
    nrm = np.stack([-gx, -gy, np.ones_like(h)], axis=-1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)

    # The colour: the item's pair as a gentle top-to-bottom ramp over its own
    # extent (the light now carries the form, so the ramp is half what it
    # was), pulled a touch toward grey — Rust's palette is muted.
    rows = np.where(solid.any(axis=1))[0]
    y0, y1 = (rows[0], rows[-1]) if len(rows) else (0, WORK - 1)
    yy = np.clip((np.arange(WORK, dtype=np.float64) - y0) / max(1, y1 - y0), 0, 1)
    t = np.repeat(yy[:, None], WORK, axis=1)[..., None] * 0.6 + 0.2

    def albedo(pair, mat):
        top, bot = (hexc(x) for x in pair)
        c = top * (1 - t) + bot * t
        lum = (c * np.array([0.2126, 0.7152, 0.0722])).sum(axis=-1, keepdims=True)
        c = c * 0.86 + lum * 0.14
        return c * (1.0 + MATS[mat]["amp"] * surface(mat))[..., None]

    col = albedo(spec["c"], base_mat)
    sp_pow, sp_amt = MATS[base_mat]["spec"]
    sp_pow = np.full(h.shape, sp_pow)
    sp_amt = np.full(h.shape, sp_amt)
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
        zm = ndimage.gaussian_filter(zm.astype(np.float64), WORK / 256)
        zmat = material(z, z["c"])
        col = col * (1 - zm[..., None]) + albedo(z["c"], zmat) * zm[..., None]
        zp, za = MATS[zmat]["spec"]
        sp_pow = sp_pow * (1 - zm) + zp * zm
        sp_amt = sp_amt * (1 - zm) + za * zm
    if spec.get("holes") is not None:
        hc = hexc(spec["holes"])
        hm = ndimage.gaussian_filter(holes.astype(np.float64), WORK / 400)[..., None]
        col = col * (1 - hm) + hc * hm
        alpha = np.maximum(alpha, np.clip(ndimage.gaussian_filter(holes * 1.0, 1.0), 0, 1))

    # The light: the key from the upper left, a sky from above and a warm
    # bounce from below, then the highlight. The crease shadow is the slab's
    # own coverage, blurred: narrow necks and inner corners get less sky.
    key = np.clip(nrm @ LIGHT, 0.0, 1.0)
    sky = 0.5 + 0.5 * (-nrm[..., 1])
    cavity = ndimage.gaussian_filter(solid.astype(np.float64), WORK * 0.02)
    cavity = np.clip(0.55 + 0.6 * cavity, 0.0, 1.0)
    light = (
        np.array([1.00, 0.97, 0.92]) * (0.95 * key)[..., None]
        + np.array([0.62, 0.68, 0.78]) * (0.42 * sky)[..., None]
        + np.array([0.40, 0.33, 0.27]) * (0.18 * (1 - sky))[..., None]
    ) * cavity[..., None]
    shine = np.clip(nrm @ HALF, 0.0, 1.0) ** sp_pow * sp_amt
    # The thickness at the rim, a hair darker: an object's edge turns away.
    rim = np.clip(d / (edge_r * 0.25), 0.0, 1.0)
    rim = np.where(holes, 1.0, rim)
    rgb = col * light * (0.72 + 0.28 * rim)[..., None] + shine[..., None]
    return rgb * alpha[..., None], alpha


# Rust's icons carry a solid near-black outline (~#161c22, 4-9 px at 512),
# which is what keeps a dark item legible on a dark slot. Ours is 5 px at
# WORK, about 1.25 at the shipped size.
OUTLINE_PX = 4
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
    args = sys.argv[1:]
    # `--out DIR` and `--renders DIR` point the run elsewhere, for a
    # contact sheet before anything lands in `assets/`.
    out_dir, renders = OUT, RENDERS
    for flag in ("--out", "--renders"):
        if flag in args:
            i = args.index(flag)
            if flag == "--out":
                out_dir = pathlib.Path(args[i + 1])
            else:
                renders = pathlib.Path(args[i + 1])
            del args[i : i + 2]
    # `--only-renders`: just the stems the renders directory has, which is
    # how the skins' pictures are made (`iconbake --skins`): no item's
    # painted silhouette belongs in `assets/icons/skins/`.
    only_renders = "--only-renders" in args
    if only_renders:
        args.remove("--only-renders")
    only = set(args)
    rendered = {p.stem for p in renders.glob("*.png")}
    stems = sorted(rendered if only_renders else set(PAINT) | rendered | set(RENDER_OF))
    todo = [s for s in stems if not only or s in only]
    missing = sorted(only - set(stems))
    if missing:
        sys.exit(f"no source for: {missing}")
    out_dir.mkdir(parents=True, exist_ok=True)
    for stem in todo:
        render = renders / f"{RENDER_OF.get(stem, stem)}.png"
        mask = MASKS / f"{stem}.png"
        if render.is_file():
            premul, alpha = from_render(render, FILL.get(stem, 0.90))
            how = "render"
        elif mask.is_file():
            premul, alpha = paint(mask, PAINT[stem], stem)
            how = "painted"
        else:
            sys.exit(f"{stem}: neither {render} nor {mask} exists")
        out = out_dir / f"{stem}.png"
        finish(premul, alpha).save(out, optimize=True)
        shrink(out)
        print(f"{stem:<20} {how}")


if __name__ == "__main__":
    main()
