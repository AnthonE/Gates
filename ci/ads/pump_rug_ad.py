#!/usr/bin/env python3
"""The Pump Rug's advert: the rug's own weave (`ci/icons/pump_rug.py`) laid on
a floor, with what it is, what it costs and where to get it.

    python3 ci/ads/pump_rug_ad.py        # writes posts/ads/pump-rug-*.png

Two cuts: 1600x900 (a post's card) and 1080x1350 (a phone's feed). Needs
numpy, Pillow and scipy (the rug's drawing needs them), and Inter if it is
installed; DejaVu otherwise.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "icons"))
import pump_rug as P  # noqa: E402

ROOT = P.ROOT
OUT = os.path.join(ROOT, "posts/ads")

INK = P.INK
GREEN = P.GREEN
SHELL = P.SHELL
DIM = (0x9A, 0xA8, 0x9E)

FONTS = {
    "black": ["/usr/share/fonts/opentype/inter/Inter-Black.otf",
              "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"],
    "bold": ["/usr/share/fonts/opentype/inter/Inter-Bold.otf",
             "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"],
    "reg": ["/usr/share/fonts/opentype/inter/Inter-Medium.otf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"],
}

HEAD = "PUMP RUG"
SUB = "pump.fun's pill, woven. Now in Gates."
STEPS = [
    ("1", "Pay THE OUTFITTER at THE GATE 100 junk."),
    ("2", "The pattern lands in your wallet in a minute or two."),
    ("3", "Twenty cloth weaves a Pump Rug anywhere."),
]
NOTE = "Lay it in your base. Sit near it to get comfortable."
FOOT = "GATES  ·  play free at elopros.com"


def font(kind, size):
    for path in FONTS[kind]:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def rug_image(width):
    """The rug's albedo as an RGB image `width` px wide."""
    albedo, _ = P.rug_design()
    im = Image.fromarray((albedo * 255).round().astype(np.uint8), "RGB")
    return im.resize((width, round(width * im.height / im.width)), Image.LANCZOS)


def perspective(src, quad):
    """`src` warped onto `quad` (tl, tr, br, bl) in its own bounding box:
    the image and its top-left corner."""
    xs, ys = [p[0] for p in quad], [p[1] for p in quad]
    x0, y0 = min(xs), min(ys)
    w, h = round(max(xs) - x0), round(max(ys) - y0)
    dst = [(x - x0, y - y0) for x, y in quad]
    sw, sh = src.size
    srcq = [(0, 0), (sw, 0), (sw, sh), (0, sh)]
    rows = []
    for (x, y), (u, v) in zip(dst, srcq):
        rows.append([x, y, 1, 0, 0, 0, -u * x, -u * y])
        rows.append([0, 0, 0, x, y, 1, -v * x, -v * y])
    rhs = [c for p in srcq for c in p]
    coeffs = np.linalg.solve(np.array(rows, float), np.array(rhs, float))
    rgba = src.convert("RGBA")
    return rgba.transform((w, h), Image.PERSPECTIVE, coeffs.tolist(), Image.BICUBIC), (x0, y0)


def backdrop(w, h, glow_at):
    """Ink with a green glow under the rug and a floor's falloff."""
    yy, xx = np.mgrid[0:h, 0:w]
    gx, gy = glow_at
    r = np.hypot((xx - gx) / (w * 0.55), (yy - gy) / (h * 0.45))
    glow = np.clip(1 - r, 0, 1) ** 2.2
    base = np.array(INK, float) * (1.05 - 0.25 * yy[..., None] / h)
    out = base + glow[..., None] * (np.array(GREEN, float) - base) * 0.22
    noise = np.random.default_rng(7).normal(0, 2.0, (h, w, 1))
    return Image.fromarray(np.clip(out + noise, 0, 255).astype(np.uint8), "RGB")


def lay_rug(canvas, quad):
    """The rug on the floor at `quad`, with a soft shadow under it."""
    rug, at = perspective(rug_image(1400), quad)
    shadow = Image.new("L", canvas.size, 0)
    ImageDraw.Draw(shadow).polygon([(x + 10, y + 22) for x, y in quad], fill=200)
    shadow = shadow.filter(ImageFilter.GaussianBlur(28))
    black = Image.new("RGB", canvas.size, (0, 0, 0))
    canvas.paste(Image.composite(black, canvas, shadow))
    canvas.paste(rug, (round(at[0]), round(at[1])), rug)


def pill_badge(size):
    """The rug's own pill, as a mark beside the headline."""
    return P.pill(size)


def text_block(d, x, y, scale):
    """Headline, line, steps, note; returns the y below them."""
    hf, sf = font("black", int(118 * scale)), font("bold", int(38 * scale))
    nf, bf = font("black", int(26 * scale)), font("reg", int(30 * scale))
    d.text((x, y), HEAD, font=hf, fill=SHELL)
    y += int(132 * scale)
    d.text((x, y), SUB, font=sf, fill=GREEN)
    y += int(78 * scale)
    r = int(22 * scale)
    for n, line in STEPS:
        d.ellipse([x, y, x + 2 * r, y + 2 * r], fill=GREEN)
        nw = d.textlength(n, font=nf)
        d.text((x + r - nw / 2, y + r), n, font=nf, fill=INK, anchor="lm")
        d.text((x + 2 * r + int(18 * scale), y + r), line, font=bf, fill=SHELL, anchor="lm")
        y += int(62 * scale)
    y += int(14 * scale)
    d.text((x, y), NOTE, font=font("reg", int(26 * scale)), fill=DIM)
    return y + int(40 * scale)


def footer(d, w, h, scale):
    f = font("bold", int(26 * scale))
    d.text((w // 2, h - int(48 * scale)), FOOT, font=f, fill=DIM, anchor="mm")
    d.line([(int(w * 0.08), h - int(92 * scale)), (int(w * 0.92), h - int(92 * scale))],
           fill=(0x25, 0x30, 0x2A), width=2)


def landscape():
    w, h = 1600, 900
    im = backdrop(w, h, (1130, 520))
    lay_rug(im, [(860, 250), (1500, 300), (1440, 760), (740, 680)])
    d = ImageDraw.Draw(im)
    badge = pill_badge(150)
    im.paste(badge, (96, 44), badge)
    text_block(d, 96, 214, 0.86)
    footer(d, w, h, 1.0)
    return im


def portrait():
    w, h = 1080, 1350
    im = backdrop(w, h, (540, 900))
    lay_rug(im, [(150, 700), (930, 700), (1030, 1150), (50, 1150)])
    d = ImageDraw.Draw(im)
    badge = pill_badge(170)
    im.paste(badge, (w - 250, 60), badge)
    text_block(d, 80, 80, 0.92)
    footer(d, w, h, 1.0)
    return im


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, make in (("pump-rug-1600x900.png", landscape), ("pump-rug-1080x1350.png", portrait)):
        path = os.path.join(OUT, name)
        make().save(path, optimize=True)
        print(f"  {os.path.relpath(path, ROOT)}")


if __name__ == "__main__":
    main()
