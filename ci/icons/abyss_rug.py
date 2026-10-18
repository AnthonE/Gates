#!/usr/bin/env python3
"""Make the Abyss Rug: its model (assets/models/deploy/abyss_rug.glb) and its
icon (assets/icons/abyss_rug.png).

    python3 ci/icons/abyss_rug.py            # from Meshy's painting (ships)
    python3 ci/icons/abyss_rug.py --drawn    # the rug drawn in code

**What ships is painted by Meshy** (`nano-banana-pro` image-to-image, with
abyss.trading's own icon as the reference, so the mark is theirs and not a
guess): the rug laid flat and shot straight down on magenta,
`abyss_rug_painting.png` beside this script. Meshy's image-to-3d was tried
first and rebuilt the flat rug as a 0.4 m box with the mark smeared over its
sides, so the geometry is ours: a 1.56 x 0.02 x 0.97 m slab (the painting's
own aspect; stretching it to the 1.1 m row would squash the mark) with 7 cm
alpha-cut tassels off each short end, 1.7 m overall. The painting is keyed,
squared up by its four fitted corners and becomes the albedo; the normal map
comes off its luminance and a pile grain, and the metal/rough map makes the
gold thread satin against matte wool.

`--drawn` is the rug drawn in code: a deep-teal woven field under a faint
caustic ripple, a gold Art Deco double border with stepped corners, and the
Abyss mark in the middle. **There the mark is one function, `abyss_mark`**;
the logo is pending Abyss's permission, and if a homage has to replace it,
it is either that function and `--drawn`, or a new painting off a new
reference. Palette (theirs): abyss #07161c, deep teal #0e3b43, gold #c7a05a,
pale gold #f3dda0, cream #f8f1e0.

Feet at y = 0, front toward +Z, one mesh, one primitive, one material: what
`tests/deploy_assets.rs` holds a deploy model to, inside the `DEPLOY` row in
`render/structures.rs`. Maps are packed to KTX2 by `ci/ktx_pack.py` (needs
the `ktx` CLI). Needs numpy, Pillow and scipy. Deterministic: same tools,
same bytes.
"""
import argparse
import json
import os
import struct
import subprocess
import sys
import tempfile
import io

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
GLB_OUT = os.path.join(ROOT, "assets/models/deploy/abyss_rug.glb")
ICON_OUT = os.path.join(ROOT, "assets/icons/abyss_rug.png")

# The rug, metres: the `DEPLOY` row (and `footprint::DEPLOY_FOOT`).
W_M, H_M, D_M = 1.7, 0.02, 1.1

# Meshy's painting (assets/models/MANIFEST.md has its prompt and task ids).
PAINTING = os.path.join(os.path.dirname(os.path.abspath(__file__)), "abyss_rug_painting.png")
# Where the woven body's corners are in it, px, TL TR BR BL: the teal
# binding's four edges, each fitted as a line and intersected (it came back
# about 0.4 degrees off square).
BODY_PX = ((126.1, 36.6), (1250.0, 38.4), (1250.1, 739.3), (121.1, 736.4))
FRINGE_PX = 52  # the tassels, past each short end
SPREAD_PX = 10  # how far they splay past the long sides
FRINGE_Y = 0.002  # the tassels lie on the floor, just off it
# The design's canvas, px: the rug's aspect, at ~0.8 mm a pixel.
CW, CH = 2048, round(2048 * D_M / W_M)
MAP_PX = 1024  # what ktx_pack.py packs each map to

ABYSS = (0x07, 0x16, 0x1C)
TEAL = (0x0E, 0x3B, 0x43)
GOLD = (0xC7, 0xA0, 0x5A)
PALE = (0xF3, 0xDD, 0xA0)
CREAM = (0xF8, 0xF1, 0xE0)


def c(rgb):
    return np.array(rgb, dtype=np.float64) / 255.0


def smooth_noise(shape, sigma, seed):
    rng = np.random.default_rng(seed)
    n = ndimage.gaussian_filter(rng.standard_normal(shape), sigma, mode="wrap")
    n -= n.min()
    return n / max(n.max(), 1e-9)


def gold_leaf(shape, seed):
    """Gold leaf: gold to pale gold over a soft mottle, lighter to the upper
    left, with fine crinkle."""
    h, w = shape
    yy, xx = np.mgrid[0:h, 0:w] / max(h, w)
    mottle = 0.65 * smooth_noise(shape, max(h, w) / 40, seed) + 0.35 * smooth_noise(shape, 2.0, seed + 1)
    t = np.clip(0.15 + 0.55 * mottle + 0.35 * (1 - (xx + yy) * 0.7), 0, 1)
    return c(GOLD) * (1 - t[..., None]) + c(PALE) * t[..., None]


# --- the mark (pending Abyss's permission: swap here, nowhere else) ----------

def abyss_mark(size):
    """The Abyss mark as an alpha mask, `size` px square: a triangle "A"
    whose lower left is cut away into four stripes rising parallel to the
    cut, the outer left leg left standing."""
    s = 4  # supersample
    n = size * s
    yy, xx = (np.mgrid[0:n, 0:n] + 0.5) / n

    def side(p, q, x, y):
        return (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0])

    apex, bl, br = (0.5, 0.06), (0.06, 0.94), (0.94, 0.94)
    outer = (side(apex, bl, xx, yy) <= 0) & (side(bl, br, xx, yy) <= 0) & (side(br, apex, xx, yy) <= 0)
    # The cut: a triangle under the left leg. Its left edge runs parallel to
    # the leg, `leg` in from it; its right edge rises from the base to meet it.
    leg = 0.06
    ox, oy = bl[0] - apex[0], bl[1] - apex[1]
    ol = np.hypot(ox, oy)
    sx, sy = oy / ol * leg, -ox / ol * leg  # inward, perpendicular to the leg
    on_leg = lambda y: (apex[0] + sx + (y - apex[1] - sy) * ox / oy, y)  # noqa: E731
    p = on_leg(0.31)
    lb = on_leg(0.94)
    rb = (0.60, 0.94)
    notch = (side(p, lb, xx, yy) <= 0) & (side(lb, rb, xx, yy) <= 0) & (side(rb, p, xx, yy) <= 0)
    # Stripes across the cut, parallel to its right edge: t runs 0 at that
    # edge to 1 at the cut's far corner; gap, stripe, ..., gap.
    ex, ey = rb[0] - p[0], rb[1] - p[1]
    nx, ny = -ey, ex
    ln = np.hypot(nx, ny)
    nx, ny = nx / ln, ny / ln
    dist = lambda x, y: (x - p[0]) * nx + (y - p[1]) * ny  # noqa: E731
    t = dist(xx, yy) / dist(*lb)
    gap = 0.045
    stripe = (1 - 5 * gap) / 4
    edges = np.cumsum([gap, stripe] * 4 + [gap])
    band = np.searchsorted(edges, t)
    # The stripes' tops are cut square to the leg, a gap in from it.
    cut = leg + 0.03
    tx, ty = apex[0] + oy / ol * cut, apex[1] - ox / ol * cut
    below_cut = (xx - tx) * oy - (yy - ty) * ox >= 0
    solid_in_notch = ((band % 2) == 1) & below_cut
    mask = outer & ~(notch & ~solid_in_notch)
    m = mask.reshape(size, s, size, s).mean(axis=(1, 3))
    return m


# --- the rug -----------------------------------------------------------------

def weave_height(h, w):
    """A plain weave: warp and weft threads crossing over and under."""
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    p = 5.0  # px a thread
    u, v = xx / p, yy / p
    over = ((np.floor(u) + np.floor(v)) % 2) == 0
    warp = np.sin(np.pi * (v % 1.0)) ** 0.6  # a thread's round, along v
    weft = np.sin(np.pi * (u % 1.0)) ** 0.6
    return np.where(over, warp, weft)


def caustics(h, w, seed):
    """A faint net of bright lines, the light through the sea's surface: the
    edges of a warped cellular noise, where the nearest two seeds tie."""
    from scipy.spatial import cKDTree

    rng = np.random.default_rng(seed)
    cell = w / 14.0
    n = int((w / cell) * (h / cell) * 1.4)
    pts = rng.uniform(0, 1, (n, 2)) * [w, h]
    # Tile the seeds once around, so the net does not thin at the rim.
    tiled = np.concatenate([pts + [dx * w, dy * h] for dx in (-1, 0, 1) for dy in (-1, 0, 1)])
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    warp = cell * 0.35
    xx = xx + warp * (smooth_noise((h, w), cell * 0.6, seed + 1) - 0.5) * 2
    yy = yy + warp * (smooth_noise((h, w), cell * 0.6, seed + 2) - 0.5) * 2
    d, _ = cKDTree(tiled).query(np.stack([xx.ravel(), yy.ravel()], -1), k=2)
    f = (d[:, 1] - d[:, 0]).reshape(h, w) / cell
    lines = np.clip(1 - f / 0.07, 0, 1) ** 1.5
    return ndimage.gaussian_filter(lines, 1.0)


def deco_frame(draw, x0, y0, x1, y1, step, width, fill):
    """A rectangle outline whose four corners step inward twice — the Art
    Deco frame of the Abyss card."""
    s1, s2 = step, step * 2
    pts = [
        (x0 + s2, y0), (x1 - s2, y0), (x1 - s2, y0 + s1), (x1 - s1, y0 + s1),
        (x1 - s1, y0 + s2), (x1, y0 + s2), (x1, y1 - s2), (x1 - s1, y1 - s2),
        (x1 - s1, y1 - s1), (x1 - s2, y1 - s1), (x1 - s2, y1), (x0 + s2, y1),
        (x0 + s2, y1 - s1), (x0 + s1, y1 - s1), (x0 + s1, y1 - s2), (x0, y1 - s2),
        (x0, y0 + s2), (x0 + s1, y0 + s2), (x0 + s1, y0 + s1), (x0 + s2, y0 + s1),
    ]
    draw.line(pts + [pts[0]], fill=fill, width=width, joint="curve")


def gold_mask(w, h):
    """Where the rug is gold: the double frame, the corner diamonds and a
    hairline rule inside, as an alpha mask (the mark is laid separately)."""
    s = 2
    im = Image.new("L", (w * s, h * s), 0)
    d = ImageDraw.Draw(im)
    u = h * s / 100.0  # one percent of the rug's depth
    deco_frame(d, 4.0 * u, 4.0 * u, w * s - 4.0 * u, h * s - 4.0 * u, 4.0 * u, round(1.6 * u), 255)
    deco_frame(d, 7.0 * u, 7.0 * u, w * s - 7.0 * u, h * s - 7.0 * u, 3.0 * u, round(0.7 * u), 255)
    # A diamond at each side's middle, between the lines.
    for cx, cy in [(w * s / 2, 5.5 * u), (w * s / 2, h * s - 5.5 * u),
                   (5.5 * u, h * s / 2), (w * s - 5.5 * u, h * s / 2)]:
        r = 1.6 * u
        d.polygon([(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)], fill=255)
    return np.asarray(im.resize((w, h), Image.LANCZOS), dtype=np.float64) / 255.0


def rug_design():
    """The rug's albedo (RGB, 0..1) and its height field, CH x CW."""
    h, w = CH, CW
    yy, xx = np.mgrid[0:h, 0:w]
    # The field: deep teal falling to the abyss at the rim, a slow mottle of
    # dye, the weave's own shade, and the caustic light.
    r = np.hypot((xx - w / 2) / (w / 2), (yy - h / 2) / (h / 2))
    deep = np.clip((r - 0.35) / 0.9, 0, 1) ** 1.3
    dye = smooth_noise((h, w), 60, 7)
    field = c(TEAL) * (1 - deep[..., None]) + c(ABYSS) * deep[..., None]
    field = field * (0.88 + 0.22 * dye[..., None])
    weave = weave_height(h, w)
    field = field * (0.82 + 0.25 * weave[..., None])
    light = caustics(h, w, 11) * (1 - 0.7 * deep)
    field = field + (c(CREAM) - field) * (0.13 * light)[..., None]
    # The dark band at the edge, where the binding is.
    u = h / 100.0
    edge = (np.minimum.reduce([xx, yy, w - 1 - xx, h - 1 - yy]) < 2.2 * u)
    field[edge] = c(ABYSS) * (0.85 + 0.2 * weave[edge, None])

    leaf = gold_leaf((h, w), 3) * (0.9 + 0.12 * weave[..., None])
    gm = gold_mask(w, h)[..., None]
    out = field * (1 - gm) + leaf * gm
    height = weave * 0.6 + gm[..., 0] * 0.4

    # The mark, centred, 46 % of the depth.
    ms = int(h * 0.46)
    mk = abyss_mark(ms)
    y0, x0 = (h - ms) // 2 - int(h * 0.01), (w - ms) // 2
    a = mk[..., None]
    # A soft dark halo under it so it lifts off the field.
    halo = np.zeros((h, w))
    halo[y0:y0 + ms, x0:x0 + ms] = mk
    halo = ndimage.gaussian_filter(halo, h * 0.012)[..., None]
    out = out * (1 - 0.45 * halo)
    region = out[y0:y0 + ms, x0:x0 + ms]
    mleaf = gold_leaf((ms, ms), 5) * (0.92 + 0.1 * weave[y0:y0 + ms, x0:x0 + ms, None])
    out[y0:y0 + ms, x0:x0 + ms] = region * (1 - a) + mleaf * a
    height[y0:y0 + ms, x0:x0 + ms] += mk * 0.4
    return np.clip(out, 0, 1), height


def normal_map(height, strength):
    gy, gx = np.gradient(height * strength)
    n = np.stack([-gx, -gy, np.ones_like(height)], axis=-1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    return n


def png_bytes(arr_u8, mode):
    b = io.BytesIO()
    Image.fromarray(arr_u8, mode).save(b, "PNG", optimize=True)
    return b.getvalue()


# --- the painting ------------------------------------------------------------

def painted_layout():
    """The painted rug in metres: body half-extents, the canvas's half-depth
    (body plus the tassels' splay), and the canvas size in px."""
    (tl, tr, br, bl) = [np.array(p) for p in BODY_PX]
    length = (np.linalg.norm(tr - tl) + np.linalg.norm(br - bl)) / 2
    depth = (np.linalg.norm(bl - tl) + np.linalg.norm(br - tr)) / 2
    m = W_M / (length + 2 * FRINGE_PX)  # metres a painted pixel
    bx, bz = length * m / 2, depth * m / 2
    fz = bz + SPREAD_PX * m
    return bx, bz, fz, (2048, round(2048 * 2 * fz / W_M))


def homography(dst, src):
    """PIL's PERSPECTIVE coefficients taking output points `dst` to input
    points `src` (four each)."""
    a, b = [], []
    for (x, y), (u, v) in zip(dst, src):
        a.append([x, y, 1, 0, 0, 0, -u * x, -u * y]); b.append(u)
        a.append([0, 0, 0, x, y, 1, -v * x, -v * y]); b.append(v)
    return tuple(np.linalg.solve(np.array(a, float), np.array(b, float)))


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def painted_design():
    """The painting squared up onto the rug's canvas: albedo RGBA (alpha is
    the tassels' cutout), height, and metal/rough, all 0..1 at the canvas
    size, plus the layout."""
    bx, bz, fz, (w, h) = painted_layout()
    px = lambda x, z: ((x + W_M / 2) / W_M * w, (z + fz) / (2 * fz) * h)  # noqa: E731
    body = [px(-bx, -bz), px(bx, -bz), px(bx, bz), px(-bx, bz)]
    src = Image.open(PAINTING).convert("RGB")
    sq = src.transform((w, h), Image.PERSPECTIVE, homography(body, BODY_PX), Image.BICUBIC)
    rgb = np.asarray(sq, dtype=np.float64) / 255.0
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    yy, xx = np.mgrid[0:h, 0:w]
    in_body = ((xx >= body[0][0] - 1) & (xx <= body[1][0] + 1)
               & (yy >= body[0][1] - 1) & (yy <= body[2][1] + 1))
    # The key: magenta is red and blue over green. Then take the spill back
    # out of the tassels' edges, and give the cut-away texels the tassels'
    # own cream so a mip level does not blend magenta back in.
    mag = np.minimum(r, b) - g
    alpha = np.where(in_body, 1.0, 1 - smoothstep(0.15, 0.45, mag))
    rgb = np.clip(rgb - np.clip(mag, 0, None)[..., None] * np.array([1.0, 0.0, 1.0]), 0, 1)
    cream = rgb[(~in_body) & (alpha > 0.95)].mean(0)
    rgb[alpha < 0.05] = cream
    # Off the long sides no face samples the canvas, but the filter and the
    # mips reach there from the body's edge and from the slab's sides, which
    # sample that edge: carry the binding out, opaque, or both pick up the
    # key's fringe and the edge erodes at a distance.
    x0, x1 = int(np.ceil(body[0][0])), int(np.floor(body[1][0])) + 1
    y0, y1 = int(np.ceil(body[0][1])), int(np.floor(body[2][1]))
    rgb[:y0 + 2, x0:x1] = rgb[y0 + 2, x0:x1]
    rgb[y1 - 1:, x0:x1] = rgb[y1 - 2, x0:x1]
    alpha[:, x0:x1] = 1.0
    # Gold thread: saturated, red well over blue. Teal has blue over red and
    # the cream tassels too little saturation to pass.
    mx, mn = rgb.max(-1), rgb.min(-1)
    sat = (mx - mn) / np.maximum(mx, 1e-6)
    gold = smoothstep(0.30, 0.45, sat) * smoothstep(0.12, 0.22, r - b) * smoothstep(0.25, 0.4, mx)
    gold = np.where(in_body, gold, 0.0)
    # Height: the painted pattern's relief (the wave lines and the thread
    # stand a little proud of the field) and a cut-pile grain under it.
    lum = rgb @ np.array([0.2126, 0.7152, 0.0722])
    relief = lum - ndimage.gaussian_filter(lum, 6)
    height = 2.5 * relief + 0.3 * gold + 0.12 * smooth_noise((h, w), 0.8, 21)
    rough = np.where(in_body, 0.93 - 0.45 * gold, 0.88)
    metal = 0.45 * gold
    return np.dstack([rgb, alpha]), height, np.dstack([rough, metal]), (bx, bz, fz)


def painted_mesh(bx, bz, fz):
    """The body slab (top carries the painting's body, its sides and
    underside sample the edge of the binding) and a tassel strip lying on
    the floor off each short end."""
    hx = W_M / 2
    U = lambda x: (x + hx) / W_M  # noqa: E731
    V = lambda z: (z + fz) / (2 * fz)  # noqa: E731
    e = 4 / 2048  # a few texels in, clear of the key
    u0, u1, v0, v1 = U(-bx) + e, U(bx) - e, V(-bz) + e, V(bz) - e
    # Each side is the top's outer band of binding folded down over the
    # edge: its top row is the edge, its foot the band `H_M` in from it.
    du, dv = H_M / W_M, H_M / (2 * fz)
    faces = [
        ((0, 1, 0), [(-bx, H_M, bz), (bx, H_M, bz), (bx, H_M, -bz), (-bx, H_M, -bz)],
         [(u0, v1), (u1, v1), (u1, v0), (u0, v0)]),
        ((0, -1, 0), [(-bx, 0, -bz), (bx, 0, -bz), (bx, 0, bz), (-bx, 0, bz)], [(u0, 0.5)] * 4),
        ((0, 0, 1), [(-bx, 0, bz), (bx, 0, bz), (bx, H_M, bz), (-bx, H_M, bz)],
         [(u0, v1 - dv), (u1, v1 - dv), (u1, v1), (u0, v1)]),
        ((0, 0, -1), [(bx, 0, -bz), (-bx, 0, -bz), (-bx, H_M, -bz), (bx, H_M, -bz)],
         [(u1, v0 + dv), (u0, v0 + dv), (u0, v0), (u1, v0)]),
        ((1, 0, 0), [(bx, 0, bz), (bx, 0, -bz), (bx, H_M, -bz), (bx, H_M, bz)],
         [(u1 - du, v1), (u1 - du, v0), (u1, v0), (u1, v1)]),
        ((-1, 0, 0), [(-bx, 0, -bz), (-bx, 0, bz), (-bx, H_M, bz), (-bx, H_M, -bz)],
         [(u0 + du, v0), (u0 + du, v1), (u0, v1), (u0, v0)]),
    ]
    for x0, x1 in ((-hx, -bx), (bx, hx)):
        faces.append(((0, 1, 0), [(x0, FRINGE_Y, fz), (x1, FRINGE_Y, fz), (x1, FRINGE_Y, -fz),
                                  (x0, FRINGE_Y, -fz)],
                      [(U(x0), 1), (U(x1), 1), (U(x1), 0), (U(x0), 0)]))
    pos, nrm, uv, idx = [], [], [], []
    for n, quad, uvs in faces:
        b = len(pos)
        pos += quad
        nrm += [n] * 4
        uv += uvs
        idx += [b, b + 1, b + 2, b, b + 2, b + 3]
    return (np.array(pos, np.float32), np.array(nrm, np.float32),
            np.array(uv, np.float32), np.array(idx, np.uint16))


# --- the model ---------------------------------------------------------------

def slab():
    """A box with its feet at y = 0, centred on x/z. The top carries the
    design (u along +x, v along +z, so the mark stands upright to a viewer
    at its front); the sides and the underside sample the binding."""
    hx, hz = W_M / 2, D_M / 2
    rim = (0.004, 0.5)
    faces = [
        # (normal, four corners counter-clockwise seen from outside, uvs)
        ((0, 1, 0), [(-hx, H_M, hz), (hx, H_M, hz), (hx, H_M, -hz), (-hx, H_M, -hz)],
         [(0, 1), (1, 1), (1, 0), (0, 0)]),
        ((0, -1, 0), [(-hx, 0, -hz), (hx, 0, -hz), (hx, 0, hz), (-hx, 0, hz)], [rim] * 4),
        ((0, 0, 1), [(-hx, 0, hz), (hx, 0, hz), (hx, H_M, hz), (-hx, H_M, hz)], [rim] * 4),
        ((0, 0, -1), [(hx, 0, -hz), (-hx, 0, -hz), (-hx, H_M, -hz), (hx, H_M, -hz)], [rim] * 4),
        ((1, 0, 0), [(hx, 0, hz), (hx, 0, -hz), (hx, H_M, -hz), (hx, H_M, hz)], [rim] * 4),
        ((-1, 0, 0), [(-hx, 0, -hz), (-hx, 0, hz), (-hx, H_M, hz), (-hx, H_M, -hz)], [rim] * 4),
    ]
    pos, nrm, uv, idx = [], [], [], []
    for n, quad, uvs in faces:
        b = len(pos)
        pos += quad
        nrm += [n] * 4
        uv += uvs
        idx += [b, b + 1, b + 2, b, b + 2, b + 3]
    return (np.array(pos, np.float32), np.array(nrm, np.float32),
            np.array(uv, np.float32), np.array(idx, np.uint16))


def write_glb(path, mesh, albedo_png, normal_png, mr_png=None):
    """One mesh, one primitive, one material. With a metal/rough map the
    albedo's alpha is a cutout (the tassels)."""
    pos, nrm, uv, idx = mesh
    blob = bytearray()
    views = []

    def view(data, target=None):
        blob.extend(b"\0" * (-len(blob) % 4))
        v = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target:
            v["target"] = target
        blob.extend(data)
        views.append(v)
        return len(views) - 1

    acc = [
        {"bufferView": view(pos.tobytes(), 34962), "componentType": 5126, "count": len(pos),
         "type": "VEC3", "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
        {"bufferView": view(nrm.tobytes(), 34962), "componentType": 5126, "count": len(nrm), "type": "VEC3"},
        {"bufferView": view(uv.tobytes(), 34962), "componentType": 5126, "count": len(uv), "type": "VEC2"},
        {"bufferView": view(idx.tobytes(), 34963), "componentType": 5123, "count": len(idx), "type": "SCALAR"},
    ]
    img_a = view(albedo_png)
    img_n = view(normal_png)
    pbr = {"baseColorTexture": {"index": 0}, "metallicFactor": 0.0, "roughnessFactor": 0.9}
    mat = {"name": "abyss_rug", "pbrMetallicRoughness": pbr, "normalTexture": {"index": 1},
           "emissiveFactor": [0.0, 0.0, 0.0]}
    textures = [{"sampler": 0, "source": 0}, {"sampler": 0, "source": 1}]
    images = [{"bufferView": img_a, "mimeType": "image/png", "name": "albedo"},
              {"bufferView": img_n, "mimeType": "image/png", "name": "normal"}]
    if mr_png is not None:
        images.append({"bufferView": view(mr_png), "mimeType": "image/png", "name": "metal_rough"})
        textures.append({"sampler": 0, "source": 2})
        pbr.update(metallicFactor=1.0, roughnessFactor=1.0, metallicRoughnessTexture={"index": 2})
        mat.update(alphaMode="MASK", alphaCutoff=0.5)
    gltf = {
        "asset": {"version": "2.0", "generator": "ci/icons/abyss_rug.py"},
        "scene": 0,
        "scenes": [{"name": "Scene", "nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "abyss_rug"}],
        "meshes": [{"name": "abyss_rug", "primitives": [
            {"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
        "materials": [mat],
        "textures": textures,
        "images": images,
        "samplers": [{"magFilter": 9729, "minFilter": 9987}],
        "accessors": acc,
        "bufferViews": views,
        "buffers": [{"byteLength": len(blob)}],
    }
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob.extend(b"\0" * (-len(blob) % 4))
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + bytes(blob))
    with open(path, "wb") as f:
        f.write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


# --- the icon ----------------------------------------------------------------

def icon(albedo, body_u=(0.0, 1.0)):
    """The rug seen from high in front, as the renders frame an item, on a
    transparent ground with the finisher's outline and drop shadow. `albedo`
    is RGB or RGBA; `body_u` is the span of it that has a thickness (the
    tassels have none)."""
    work = 512
    if albedo.shape[-1] == 3:
        albedo = np.dstack([albedo, np.ones(albedo.shape[:2])])
    src = Image.fromarray((albedo * 255).round().astype(np.uint8), "RGBA")
    sw, sh = src.size
    # Where the rug's four corners land: far edge narrower and higher.
    far_y, near_y = 0.20 * work, 0.80 * work
    far_half, near_half = 0.40 * work, 0.47 * work
    cx = work / 2
    quad = [(cx - far_half, far_y), (cx + far_half, far_y),
            (cx + near_half, near_y), (cx - near_half, near_y)]
    # PIL's perspective maps OUTPUT to INPUT: solve the homography.
    srcpts = [(0, 0), (sw, 0), (sw, sh), (0, sh)]
    a, b = [], []
    for (x, y), (u, v) in zip(quad, srcpts):
        a.append([x, y, 1, 0, 0, 0, -u * x, -u * y]); b.append(u)
        a.append([0, 0, 0, x, y, 1, -v * x, -v * y]); b.append(v)
    coeff = np.linalg.solve(np.array(a, float), np.array(b, float))
    top = src.transform((work, work), Image.PERSPECTIVE, tuple(coeff), Image.BICUBIC)
    rgba = np.asarray(top, dtype=np.float64) / 255.0
    # The rug's near edge, its thickness: a dark strip under the front.
    t = 0.022 * work
    (x3, y3), (x2, _) = quad[3], quad[2]
    nl, nr = x3 + body_u[0] * (x2 - x3), x3 + body_u[1] * (x2 - x3)
    edge = Image.new("L", (work, work), 0)
    ImageDraw.Draw(edge).polygon([(nl, y3), (nr, y3), (nr, y3 + t), (nl, y3 + t)], fill=255)
    e = np.asarray(edge, dtype=np.float64)[..., None] / 255.0
    rgb = rgba[..., :3] * rgba[..., 3:] + c(ABYSS) * 0.8 * e * (1 - rgba[..., 3:])
    alpha = np.clip(rgba[..., 3] + e[..., 0], 0, 1)
    # Outline in the finisher's ink, then the drop shadow, at 128.
    ink = c((0x16, 0x1C, 0x22))
    oy, ox = np.ogrid[-4:5, -4:5]
    ring = ndimage.grey_dilation(alpha, footprint=ox * ox + oy * oy <= 16)
    ring = np.clip(ndimage.gaussian_filter(ring, 0.8), 0, 1)
    under = ring * (1 - alpha)
    premul = rgb * alpha[..., None] + ink * under[..., None]
    alpha = alpha + under
    px = 128

    def down(ch):
        return np.asarray(Image.fromarray(ch.astype(np.float32), "F").resize((px, px), Image.LANCZOS),
                          dtype=np.float64)

    small = np.clip(np.stack([down(premul[..., i]) for i in range(3)], -1), 0, None)
    a = np.clip(down(alpha), 0, 1)
    shadow = ndimage.shift(ndimage.gaussian_filter(a, 1.6), (1.6, 1.0), order=1)
    shadow = np.clip(shadow, 0, 1) * 0.58
    out_a = a + shadow * (1 - a)
    col = np.where(out_a[..., None] > 1e-6, small / np.maximum(out_a, 1e-6)[..., None], 0.0)
    rgb8 = np.round(np.clip(col, 0, 1) * 255).astype(np.uint8)
    a8 = np.round(out_a * 255).astype(np.uint8)
    return Image.fromarray(np.dstack([rgb8, a8]), "RGBA")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--drawn", action="store_true", help="the rug drawn in code, not the painting")
    drawn = ap.parse_args().drawn
    mr = None
    if drawn:
        albedo, height = rug_design()
        mesh, body_u = slab(), (0.0, 1.0)
    else:
        albedo, height, mr, (bx, bz, fz) = painted_design()
        mesh, body_u = painted_mesh(bx, bz, fz), (0.5 - bx / W_M, 0.5 + bx / W_M)
    icon(albedo, body_u).save(ICON_OUT, optimize=True)
    print(f"  {os.path.relpath(ICON_OUT, ROOT)}")

    sq = (MAP_PX, MAP_PX)
    a8 = (albedo * 255).round().astype(np.uint8)
    a8 = np.asarray(Image.fromarray(a8, "RGBA" if a8.shape[-1] == 4 else "RGB")
                    .resize(sq, Image.LANCZOS).convert("RGBA"))
    # The height field at the map's own size, then its normals, so the weave
    # is a few texels a thread and the map stays near flat on average.
    h = np.asarray(Image.fromarray(height.astype(np.float32), "F").resize(sq, Image.LANCZOS),
                   dtype=np.float64)
    n = normal_map(h, 1.2)
    n8 = np.round((n + 1) * 127.5).clip(0, 255).astype(np.uint8)
    n8 = np.dstack([n8, np.full(sq, 255, np.uint8)])
    mr_png = None
    if mr is not None:
        # glTF's metal/rough: green is roughness, blue is metalness.
        m8 = np.round(mr * 255).clip(0, 255).astype(np.uint8)
        full = np.full(m8.shape[:2], 255, np.uint8)
        m8 = np.dstack([full, m8[..., 0], m8[..., 1], full])
        mr_png = png_bytes(np.asarray(Image.fromarray(m8, "RGBA").resize(sq, Image.LANCZOS)), "RGBA")
    with tempfile.TemporaryDirectory() as tmp:
        raw = os.path.join(tmp, "abyss_rug.png.glb")
        write_glb(raw, mesh, png_bytes(a8, "RGBA"), png_bytes(n8, "RGBA"), mr_png)
        subprocess.run([sys.executable, os.path.join(ROOT, "ci/ktx_pack.py"), raw, GLB_OUT,
                        "--size", str(MAP_PX)], check=True)
    print(f"  {os.path.relpath(GLB_OUT, ROOT)}")


if __name__ == "__main__":
    main()
