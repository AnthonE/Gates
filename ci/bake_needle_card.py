#!/usr/bin/env python3
"""Bake the conifer needle card from Poly Haven's `pine_tree_01` twig maps.

The source twig atlas holds two photographed pine sprigs (plus cones, bark and
loose needles). A canopy card is ~1.1 m of branch end, so the card is composed:
a woody axis from the attach point (bottom centre, where `bevy_procedural_tree`
roots every leaf quad) with whorls of sprigs off it and a leader on top, the
way a pine shoot grows.

The RGB is stored **mean-normalised**: each channel is divided by the covered
texels' linear mean and scaled by `K` so the brightest 0.5 % fit in a byte.
The canopy's colour stays in `tree.rs`'s vertex bands and the photo adds only
its variation (dry needles, stems, shading); `tree::NEEDLE_MAP_GAIN` is `1/K`
and `tests/tree.rs` checks the two agree.

Source: Poly Haven `pine_tree_01`, CC0. Fetched into the gitignored
`assets/textures/candidates/pine_tree_01/` (auto-downloaded if missing).

Run:  python ci/bake_needle_card.py
Out:  assets/textures/needle_card_albedo.png   (512² RGBA, alpha is the cutout)
"""
import math
import sys
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "assets/textures/candidates/pine_tree_01"
URL = "https://dl.polyhaven.org/file/ph-assets/Models/{ext}/2k/pine_tree_01/{name}"
SRC_RGB = "pine_tree_01_twig_diff_2k.jpg"
SRC_A = "pine_tree_01_twig_alpha_2k.png"
OUT = ROOT / "assets/textures/needle_card_albedo.png"

OUT_SIZE = 512
# Composed at 2x and reduced, so a needle's edge is anti-aliased into alpha.
SS = 2
W = OUT_SIZE * SS
CUT = 128
# The two sprigs, as (component bbox in the 2k source).
SPRIG_A = (73, 90, 450, 905)     # vertical, stem at the bottom
SPRIG_B = (1334, 701, 1918, 1061)  # horizontal, stem at the left
# Alpha gain at level 0: a 1-texel needle split across two texels lands at
# ~0.5 each and the 0.5 cut would break it into dashes.
ALPHA_GAIN = 1.35


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


def sprig(rgb: np.ndarray, a: np.ndarray, box) -> np.ndarray:
    """One sprig as premultiplied linear RGBA float, its neighbours masked out."""
    x0, y0, x1, y1 = box
    sub_a = a[y0:y1, x0:x1]
    lab, n = ndimage.label(sub_a > CUT / 255, structure=np.ones((3, 3)))
    sizes = ndimage.sum(np.ones_like(lab), lab, range(1, n + 1))
    keep = lab == (int(np.argmax(sizes)) + 1)
    # The soft edge sits just outside the >CUT component.
    keep = ndimage.binary_dilation(keep, iterations=3)
    alpha = np.where(keep, sub_a, 0.0)
    lin = srgb_to_lin(rgb[y0:y1, x0:x1])
    return np.concatenate([lin * alpha[..., None], alpha[..., None]], axis=-1)


def place(canvas, spr, root_xy, stem_xy, angle_deg, length_px, mirror):
    """Paste `spr` (premultiplied) so its stem point lands on `root_xy`,
    pointing `angle_deg` off vertical (+ = right), `length_px` tall."""
    h, w = spr.shape[:2]
    img = spr
    sx, sy = stem_xy
    if mirror:
        img = img[:, ::-1]
        sx = w - 1 - sx
    scale = length_px / h
    pil = [Image.fromarray(img[..., c].astype(np.float32), "F") for c in range(4)]
    nw, nh = max(1, int(w * scale)), max(1, int(h * scale))
    pil = [p.resize((nw, nh), Image.LANCZOS) for p in pil]
    sx, sy = sx * scale, sy * scale
    # Rotate about the stem point: PIL rotates CCW about the centre, so turn
    # `angle_deg` right by -angle.
    pil = [p.rotate(-angle_deg, resample=Image.BICUBIC, expand=True) for p in pil]
    rw, rh = pil[0].size
    t = math.radians(-angle_deg)
    cx, cy = nw / 2, nh / 2
    dx, dy = sx - cx, sy - cy
    # Image y points down; a CCW rotation by t in image space.
    rx = dx * math.cos(t) + dy * math.sin(t)
    ry = -dx * math.sin(t) + dy * math.cos(t)
    px, py = rw / 2 + rx, rh / 2 + ry
    ox, oy = int(round(root_xy[0] - px)), int(round(root_xy[1] - py))
    layer = np.stack([np.asarray(p) for p in pil], axis=-1).clip(0, None)
    layer[..., 3] = layer[..., 3].clip(0, 1)
    # Composite "over", premultiplied.
    H, Wc = canvas.shape[:2]
    x0, y0 = max(ox, 0), max(oy, 0)
    x1, y1 = min(ox + rw, Wc), min(oy + rh, H)
    if x0 >= x1 or y0 >= y1:
        return
    src = layer[y0 - oy:y1 - oy, x0 - ox:x1 - ox]
    dst = canvas[y0:y1, x0:x1]
    dst[:] = src + dst * (1.0 - src[..., 3:4])


def axis(canvas, pts, w0, w1, colour):
    """A tapered woody axis through `pts`, premultiplied, drawn under the sprigs."""
    H, Wc = canvas.shape[:2]
    yy, xx = np.mgrid[0:H, 0:Wc].astype(np.float32)
    n = len(pts) - 1
    for i in range(n):
        (ax, ay), (bx, by) = pts[i], pts[i + 1]
        ra = w0 + (w1 - w0) * i / n
        rb = w0 + (w1 - w0) * (i + 1) / n
        x0, x1 = int(min(ax, bx) - ra - 2), int(max(ax, bx) + ra + 2)
        y0, y1 = int(min(ay, by) - ra - 2), int(max(ay, by) + ra + 2)
        x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, Wc), min(y1, H)
        px, py = xx[y0:y1, x0:x1], yy[y0:y1, x0:x1]
        vx, vy = bx - ax, by - ay
        L2 = vx * vx + vy * vy
        t = np.clip(((px - ax) * vx + (py - ay) * vy) / L2, 0, 1)
        d = np.hypot(px - (ax + t * vx), py - (ay + t * vy))
        r = ra + (rb - ra) * t
        cov = np.clip(r - d + 0.5, 0, 1)
        # Bark is darker along its edges.
        shade = 0.55 + 0.45 * np.clip(1 - d / np.maximum(r, 1e-3), 0, 1)
        src = np.concatenate([colour[None, None, :] * shade[..., None] * cov[..., None],
                              cov[..., None]], axis=-1)
        dst = canvas[y0:y1, x0:x1]
        dst[:] = src + dst * (1.0 - src[..., 3:4])


def main() -> int:
    rgb = np.asarray(Image.open(fetch(SRC_RGB)).convert("RGB")).astype(np.float32) / 255
    a = np.asarray(Image.open(fetch(SRC_A)).convert("L")).astype(np.float32) / 255
    A = sprig(rgb, a, SPRIG_A)
    # B points right with its stem at the left; turned to point up.
    B = sprig(rgb, a, SPRIG_B)
    B = np.rot90(B, k=1).copy()
    # Stem points (where each sprig's woody end is), in each sprite's frame.
    stem_a = (205, A.shape[0] - 4)
    # B's stem was at (35, 240) of its 584x360 crop; rot90 CCW maps (x, y) to
    # (y, w - 1 - x).
    stem_b = (240, B.shape[0] - 1 - 35)

    # The woody axis's colour: the median of sprig A's stem pixels.
    stem_px = srgb_to_lin(rgb[820:900, 270:290]).reshape(-1, 3)
    bark = np.median(stem_px, axis=0)

    canvas = np.zeros((W, W, 4), np.float32)
    root = (W / 2, W - 2)
    rng = np.random.default_rng(0x9e3779b9)
    # The axis: from the attach point to the leader's node, bowing a little.
    top = (W / 2 + W * 0.02, W * 0.40)
    pts = [(root[0] + (top[0] - root[0]) * t + math.sin(t * math.pi) * W * 0.015,
            root[1] + (top[1] - root[1]) * t) for t in np.linspace(0, 1, 12)]

    def at(t):
        i = t * (len(pts) - 1)
        k = int(min(i, len(pts) - 2))
        f = i - k
        return (pts[k][0] + (pts[k + 1][0] - pts[k][0]) * f,
                pts[k][1] + (pts[k + 1][1] - pts[k][1]) * f)

    # Whorls, oldest (lowest, longest, widest) first so younger ones overlap.
    # (position along the axis, angle off vertical, length as a share of W)
    whorls = [
        (0.22, 70, 0.46),
        (0.30, -64, 0.44),
        (0.48, 52, 0.42),
        (0.55, -48, 0.42),
        (0.74, 34, 0.40),
        (0.80, -30, 0.40),
        (0.88, 12, 0.36),
    ]
    for i, (t, ang, length) in enumerate(whorls):
        use_b = i % 2 == 1
        spr, stem = (B, stem_b) if use_b else (A, stem_a)
        jitter = rng.uniform(-6, 6)
        place(canvas, spr, at(t), stem, ang + jitter, W * length * rng.uniform(0.92, 1.05),
              mirror=bool(rng.integers(0, 2)))
    axis(canvas, pts, W * 0.007, W * 0.004, bark)
    # The leader, last: straight up off the axis top.
    place(canvas, A, at(1.0), stem_a, rng.uniform(-4, 4), W * 0.46, mirror=False)

    # Reduce SSx: premultiplied box, so colour is weighted by coverage.
    c = canvas.reshape(OUT_SIZE, SS, OUT_SIZE, SS, 4).mean(axis=(1, 3))
    alpha = c[..., 3]
    col = np.where(alpha[..., None] > 1e-4, c[..., :3] / np.maximum(alpha[..., None], 1e-4), 0)
    alpha = np.clip(alpha * ALPHA_GAIN, 0, 1)

    # Mean-normalise the colour over what the cut keeps.
    covered = alpha > CUT / 255
    mean = col[covered].mean(axis=0)
    norm = col / mean
    k = 1.0 / np.percentile(norm[covered].max(axis=1), 99.5)
    col = norm * k

    # Bleed colour under the transparent texels so filtering never pulls in
    # black at a needle's edge.
    known = alpha > 0.02
    rgb_out = col.copy()
    for _ in range(8):
        if known.all():
            break
        kern = np.ones((3, 3))
        cnt = ndimage.convolve(known.astype(np.float32), kern, mode="nearest")
        acc = np.stack([ndimage.convolve(rgb_out[..., i] * known, kern, mode="nearest")
                        for i in range(3)], axis=-1)
        fill = (cnt > 0) & ~known
        avg = acc / np.maximum(cnt[..., None], 1)
        rgb_out[fill] = avg[fill]
        known = known | fill
    rgb_out[~known] = k  # mean colour, normalised

    out = np.concatenate([lin_to_srgb(rgb_out), alpha[..., None]], axis=-1)
    Image.fromarray((out * 255 + 0.5).clip(0, 255).astype(np.uint8), "RGBA").save(OUT, optimize=True)
    print(f"wrote {OUT.relative_to(ROOT)} {OUT_SIZE}x{OUT_SIZE} "
          f"({OUT.stat().st_size:,} bytes) coverage {covered.mean():.3f} "
          f"source mean lin rgb {mean.round(4).tolist()} K {k:.4f} gain {1 / k:.4f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
