#!/usr/bin/env python3
"""Make the Pump Rug: a look for the Abyss Rug (skin 25, `content/skins.toml`),
its model (assets/models/held/skins/pump_rug.glb) and its picture
(assets/icons/skins/25.png).

    python3 ci/icons/pump_rug.py

pump.fun's pill on a rug, drawn in code (operator, 2026-10-25: *"we could make
a pump rug like we did abyss"*): a woven ink field, a pill-green double
border, the pill laid big across the middle (green and white, leaning up to
the right, with the Elo Pros chevrons on its green half), and a small pill in
each corner. The pill is `pill()`; swap it there and nowhere else.

The geometry, the glb writer, the normal map and the icon are the Abyss Rug's
own (`abyss_rug.py`): the same 1.7 x 0.02 x 1.1 m slab on the same `DEPLOY`
row, so the look lies exactly where the rug does. Maps are packed to KTX2 by
`ci/ktx_pack.py` (needs `ktx`). Needs numpy, Pillow and scipy.
"""
import os
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import abyss_rug as A  # noqa: E402

ROOT = A.ROOT
GLB_OUT = os.path.join(ROOT, "assets/models/held/skins/pump_rug.glb")
ICON_OUT = os.path.join(ROOT, "assets/icons/skins/25.png")

INK = (0x0A, 0x0C, 0x0A)
FIELD = (0x14, 0x1A, 0x16)
GREEN = (0x5F, 0xD3, 0x8D)   # the pill's green, by eye to pump.fun's
SHELL = (0xF2, 0xF4, 0xEE)
TILT = -35                   # degrees: the pill leans up to the right


def pill(length):
    """The pill lying flat, `length` px long, RGBA: an ink outline, the white
    half trailing, the green half leading with two chevrons, a seam."""
    h = int(length * 0.40)
    ol = h * 0.085
    r = h / 2
    im = Image.new("RGBA", (length, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([0, 0, length - 1, h - 1], radius=r, fill=INK)
    inner = [ol, ol, length - 1 - ol, h - 1 - ol]
    mid = length / 2
    d.rounded_rectangle(inner, radius=r - ol, fill=SHELL)
    green = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ImageDraw.Draw(green).rounded_rectangle(inner, radius=r - ol, fill=GREEN)
    cut = Image.new("L", im.size, 0)
    ImageDraw.Draw(cut).rectangle([mid, 0, length, h], fill=255)
    im.paste(green, (0, 0), Image.composite(green.getchannel("A"), cut, cut))
    d.rectangle([mid - ol / 2, ol, mid + ol / 2, h - 1 - ol], fill=INK)
    # the chevrons: the Elo Pros mark's proportions (arm reach 18, drop 15,
    # stroke 9 on its 64 grid), pointing the way the pill leans
    half, depth, thick = 18, 15, 9
    k = (h - 2 * ol) * 0.62 / (2 * half)
    cy = h / 2
    step = (depth + thick) * k
    first = mid + (mid - ol - r - 2 * step) / 2 + step * 0.9
    for ax in (first + step, first + 2 * step):
        d.polygon([(ax - depth * k, cy - half * k), (ax, cy), (ax - depth * k, cy + half * k),
                   (ax - (depth + thick) * k, cy + half * k), (ax - thick * k, cy),
                   (ax - (depth + thick) * k, cy - half * k)], fill=INK)
    return im.rotate(-TILT, resample=Image.BICUBIC, expand=True)


def lay(out, height, sprite, cx, cy, weave, lift):
    """Lay an RGBA sprite on the albedo (0..1) centred at (cx, cy), with a soft
    dark halo under it, and raise the height field where it lies."""
    h, w = height.shape
    a = np.asarray(sprite, dtype=np.float64) / 255.0
    sh, sw = a.shape[:2]
    x0, y0 = int(cx - sw / 2), int(cy - sh / 2)
    ys, xs = slice(max(y0, 0), min(y0 + sh, h)), slice(max(x0, 0), min(x0 + sw, w))
    sy, sx = slice(ys.start - y0, ys.stop - y0), slice(xs.start - x0, xs.stop - x0)
    alpha = a[sy, sx, 3:4]
    halo = np.zeros((h, w))
    halo[ys, xs] = alpha[..., 0]
    out *= (1 - 0.45 * ndimage.gaussian_filter(halo, h * 0.012))[..., None]
    rgb = a[sy, sx, :3] * (0.9 + 0.12 * weave[ys, xs, None])
    out[ys, xs] = out[ys, xs] * (1 - alpha) + rgb * alpha
    height[ys, xs] += alpha[..., 0] * lift


def rug_design():
    """The rug's albedo (RGB, 0..1) and its height field, CH x CW."""
    h, w = A.CH, A.CW
    yy, xx = np.mgrid[0:h, 0:w]
    r = np.hypot((xx - w / 2) / (w / 2), (yy - h / 2) / (h / 2))
    deep = np.clip((r - 0.45) / 0.9, 0, 1) ** 1.3
    dye = A.smooth_noise((h, w), 60, 9)
    out = A.c(FIELD) * (1 - deep[..., None]) + A.c(INK) * deep[..., None]
    out = out * (0.9 + 0.2 * dye[..., None])
    weave = A.weave_height(h, w)
    out = out * (0.82 + 0.25 * weave[..., None])
    height = weave * 0.6
    # the binding, then a green double border inside it
    u = h / 100.0
    edge = np.minimum.reduce([xx, yy, w - 1 - xx, h - 1 - yy])
    out[edge < 2.2 * u] = A.c(INK) * (0.85 + 0.2 * weave[edge < 2.2 * u, None])
    for at, width in ((4.0, 1.6), (7.0, 0.7)):
        band = (edge >= at * u) & (edge < (at + width) * u)
        out[band] = A.c(GREEN) * (0.85 + 0.2 * weave[band, None])
        height[band] += 0.4
    # the big pill across the middle, and a small one in each corner
    lay(out, height, pill(int(w * 0.62)), w / 2, h / 2, weave, 0.4)
    small = pill(int(h * 0.22))
    for fx, fy in ((0.12, 0.2), (0.88, 0.2), (0.12, 0.8), (0.88, 0.8)):
        lay(out, height, small, w * fx, h * fy, weave, 0.3)
    return np.clip(out, 0, 1), height


def main():
    albedo, height = rug_design()
    os.makedirs(os.path.dirname(ICON_OUT), exist_ok=True)
    A.icon(albedo).save(ICON_OUT, optimize=True)
    print(f"  {os.path.relpath(ICON_OUT, ROOT)}")
    sq = (A.MAP_PX, A.MAP_PX)
    a8 = np.asarray(Image.fromarray((albedo * 255).round().astype(np.uint8), "RGB")
                    .resize(sq, Image.LANCZOS).convert("RGBA"))
    hm = np.asarray(Image.fromarray(height.astype(np.float32), "F").resize(sq, Image.LANCZOS),
                    dtype=np.float64)
    n = A.normal_map(hm, 1.2)
    n8 = np.round((n + 1) * 127.5).clip(0, 255).astype(np.uint8)
    n8 = np.dstack([n8, np.full(sq, 255, np.uint8)])
    with tempfile.TemporaryDirectory() as tmp:
        raw = os.path.join(tmp, "pump_rug.png.glb")
        A.write_glb(raw, A.slab(), A.png_bytes(a8, "RGBA"), A.png_bytes(n8, "RGBA"))
        subprocess.run([sys.executable, os.path.join(ROOT, "ci/ktx_pack.py"), raw, GLB_OUT,
                        "--size", str(A.MAP_PX)], check=True)
    print(f"  {os.path.relpath(GLB_OUT, ROOT)}")


if __name__ == "__main__":
    main()
