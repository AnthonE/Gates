#!/usr/bin/env python3
"""Bake the fern card atlas from Poly Haven's `fern_02` frond sheet.

The source is five single fronds, stem at the bottom. A fern is a clump of
fronds arching out of one crown, so each cell is composed: six or seven fronds
bent outward and fanned from the cell's bottom centre, the most upright last so
it overlaps the ones lying out to the sides.

Same layout as `grass_card_albedo.png` — 2x2 cells of 512x256, roots on each
cell's bottom edge — so `clutter::card` draws it unchanged with only the
material swapped.

Source: Poly Haven `fern_02`, CC0. Fetched into the gitignored
`assets/textures/candidates/fern_02/` (auto-downloaded if missing). The alpha
is a separate 16-bit map.

Run:  python ci/bake_fern_atlas.py
Out:  assets/textures/fern_card_albedo.png   (1024x512 RGBA, alpha is the cutout)
"""
import math
import sys
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "assets/textures/candidates/fern_02"
URL = "https://dl.polyhaven.org/file/ph-assets/Models/{ext}/2k/fern_02/{name}"
SRC_RGB = "fern_02_diff_2k.jpg"
SRC_A = "fern_02_alpha_2k.png"
OUT = ROOT / "assets/textures/fern_card_albedo.png"

CELL_W, CELL_H = 512, 256
COLS = ROWS = 2
SS = 2
CUT = 0.5
# The five fronds' boxes in the 2k sheet (connected components of the alpha).
FRONDS = [(181, 229, 481, 1943), (642, 51, 932, 1563), (1462, 223, 1714, 1834),
          (1040, 55, 1349, 1024), (1049, 1046, 1287, 1977)]


def fetch(name: str) -> Path:
    p = SRC_DIR / name
    if not p.exists():
        SRC_DIR.mkdir(parents=True, exist_ok=True)
        ext = "png" if name.endswith(".png") else "jpg"
        print(f"fetching {name}")
        urllib.request.urlretrieve(URL.format(ext=ext, name=name), p)
    return p


def srgb_to_lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def lin_to_srgb(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def frond(rgb, a, box):
    """One frond as premultiplied linear RGBA, its neighbours masked out, and
    the column of its stem foot."""
    x0, y0, x1, y1 = box
    sub = a[y0:y1, x0:x1]
    lab, n = ndimage.label(sub > CUT, structure=np.ones((3, 3)))
    sizes = ndimage.sum(np.ones_like(lab), lab, range(1, n + 1))
    keep = ndimage.binary_dilation(lab == int(np.argmax(sizes)) + 1, iterations=3)
    alpha = np.where(keep, sub, 0.0)
    lin = srgb_to_lin(rgb[y0:y1, x0:x1])
    spr = np.concatenate([lin * alpha[..., None], alpha[..., None]], axis=-1)
    # The stem foot: the lowest covered row's mean column.
    rows = np.where((alpha > CUT).any(axis=1))[0]
    foot_y = rows.max()
    foot_x = float(np.mean(np.where(alpha[foot_y] > CUT)[0]))
    return spr, (foot_x, foot_y)


def bend(spr, foot, amount):
    """Arch a vertical frond sideways: the tip moves `amount` of the frond's
    height, quadratically from the foot."""
    h, w = spr.shape[:2]
    pad = int(abs(amount) * h) + 2
    out = np.zeros((h, w + 2 * pad, 4), np.float32)
    yy, xx = np.mgrid[0:h, 0:w + 2 * pad].astype(np.float32)
    t = np.clip((foot[1] - yy) / max(foot[1], 1), 0, 1)
    src_x = xx - pad - amount * h * t * t
    for c in range(4):
        out[..., c] = ndimage.map_coordinates(spr[..., c], [yy, src_x], order=1, cval=0.0)
    return out, (foot[0] + pad, foot[1])


def place(canvas, spr, foot, root, angle_deg, length_px):
    h, w = spr.shape[:2]
    scale = length_px / foot[1]
    nw, nh = max(1, int(w * scale)), max(1, int(h * scale))
    chans = [Image.fromarray(spr[..., c].astype(np.float32), "F").resize((nw, nh), Image.LANCZOS)
             for c in range(4)]
    fx, fy = foot[0] * scale, foot[1] * scale
    chans = [c.rotate(-angle_deg, resample=Image.BICUBIC, expand=True) for c in chans]
    rw, rh = chans[0].size
    t = math.radians(-angle_deg)
    dx, dy = fx - nw / 2, fy - nh / 2
    rx = dx * math.cos(t) + dy * math.sin(t)
    ry = -dx * math.sin(t) + dy * math.cos(t)
    ox, oy = int(round(root[0] - (rw / 2 + rx))), int(round(root[1] - (rh / 2 + ry)))
    layer = np.stack([np.asarray(c) for c in chans], axis=-1).clip(0, None)
    layer[..., 3] = layer[..., 3].clip(0, 1)
    H, W = canvas.shape[:2]
    x0, y0, x1, y1 = max(ox, 0), max(oy, 0), min(ox + rw, W), min(oy + rh, H)
    if x0 >= x1 or y0 >= y1:
        return
    src = layer[y0 - oy:y1 - oy, x0 - ox:x1 - ox]
    dst = canvas[y0:y1, x0:x1]
    dst[:] = src + dst * (1.0 - src[..., 3:4])


def cell(fronds, rng):
    W, H = CELL_W * SS, CELL_H * SS
    canvas = np.zeros((H, W, 4), np.float32)
    root = (W / 2 + rng.uniform(-0.03, 0.03) * W, H - 2)
    n = int(rng.integers(6, 8))
    # Angles off vertical, spread over both sides, outermost drawn first.
    angles = np.sort(rng.uniform(8, 74, n))[::-1]
    sides = np.where(np.arange(n) % 2 == 0, 1, -1) * (1 if rng.random() < 0.5 else -1)
    for ang, side in zip(angles, sides):
        spr, foot = fronds[int(rng.integers(0, len(fronds)))]
        if rng.random() < 0.5:
            spr = spr[:, ::-1]
            foot = (spr.shape[1] - 1 - foot[0], foot[1])
        # Fronds lying out to the side arch over more.
        bent, bfoot = bend(spr, foot, side * (0.06 + 0.16 * ang / 74))
        # Long enough to reach most of the cell's height when upright, and
        # its half-width when lying out.
        reach = H * 0.94 / max(math.cos(math.radians(ang)) + 0.35 * math.sin(math.radians(ang)), 0.6)
        length = min(reach, W * 0.47 / max(math.sin(math.radians(ang)), 0.2)) * rng.uniform(0.82, 1.0)
        place(canvas, bent, bfoot, root, side * ang, length)
    c = canvas.reshape(CELL_H, SS, CELL_W, SS, 4).mean(axis=(1, 3))
    return c


def main() -> int:
    rgb = np.asarray(Image.open(fetch(SRC_RGB)).convert("RGB")).astype(np.float32) / 255
    a16 = np.asarray(Image.open(fetch(SRC_A))).astype(np.float32)
    a = a16 / max(float(a16.max()), 1.0)
    fronds = [frond(rgb, a, b) for b in FRONDS]
    rng = np.random.default_rng(0xfe41)
    atlas = np.zeros((CELL_H * ROWS, CELL_W * COLS, 4), np.float32)
    for i in range(COLS * ROWS):
        cx, cy = (i % COLS) * CELL_W, (i // COLS) * CELL_H
        c = cell(fronds, rng)
        atlas[cy:cy + CELL_H, cx:cx + CELL_W] = c
        cov = (c[..., 3] > CUT).mean()
        print(f"  cell {i}: coverage {cov * 100:.1f}%")
    alpha = atlas[..., 3]
    col = np.where(alpha[..., None] > 1e-4, atlas[..., :3] / np.maximum(alpha[..., None], 1e-4), 0)
    # Bleed colour under the transparent texels.
    known = alpha > 0.02
    for _ in range(8):
        if known.all():
            break
        k = np.ones((3, 3))
        cnt = ndimage.convolve(known.astype(np.float32), k, mode="nearest")
        acc = np.stack([ndimage.convolve(col[..., i] * known, k, mode="nearest") for i in range(3)], -1)
        fill = (cnt > 0) & ~known
        col[fill] = (acc / np.maximum(cnt[..., None], 1))[fill]
        known = known | fill
    col[~known] = col[alpha > CUT].mean(axis=0)
    out = np.concatenate([lin_to_srgb(col), alpha[..., None]], axis=-1)
    Image.fromarray((out * 255 + 0.5).clip(0, 255).astype(np.uint8), "RGBA").save(OUT, optimize=True)
    print(f"wrote {OUT.relative_to(ROOT)} {atlas.shape[1]}x{atlas.shape[0]} "
          f"({OUT.stat().st_size:,} bytes) coverage {(alpha > CUT).mean():.3f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
