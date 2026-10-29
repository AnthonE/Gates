#!/usr/bin/env python3
"""Draw the Patchwork Tunic: assets/skins/patchwork_tunic.png.

A tunic sewn from scraps: patches of denim, rust, mustard, moss, a plaid and
a ticking stripe, each its own weave, joined with dark running stitches, a
rope collar at the top edge and a frayed hem at the bottom. Drawn on the
tunic's canvas (`skinkit template tunic`), CC0. The top is the image's upper
half (collar at the top edge), the skirt the lower half (hem at the bottom);
left to right is one turn, so the patch columns wrap the left and right edges
without a seam.

    python3 assets/skins/draw_patchwork_tunic.py
"""
import random
from pathlib import Path

from PIL import Image

SIDE = 512
STITCH = (46, 36, 28)
COLLAR = (122, 96, 64)
HEM = (98, 80, 58)
# Faded on purpose: scraps that have been out in the weather.
CLOTHS = [
    ("denim", (74, 90, 112)),
    ("rust", (128, 74, 54)),
    ("mustard", (160, 132, 72)),
    ("moss", (92, 106, 70)),
    ("oat", (158, 146, 120)),
    ("plum", (96, 70, 84)),
    ("plaid", (124, 62, 50)),
    ("ticking", (164, 156, 138)),
]


def clamp(c):
    return max(0, min(255, int(c)))


def main() -> None:
    rng = random.Random(28)
    img = Image.new("RGB", (SIDE, SIDE))
    px = img.load()

    # The patch layout: rows of uneven heights, and in each row columns of
    # uneven widths that sum to the full turn, so x wraps cleanly.
    rows, y = [], 34
    while y < SIDE - 34:
        h = rng.randint(64, 104)
        rows.append((y, min(SIDE - 34, y + h)))
        y += h
    patches = []
    for y0, y1 in rows:
        x, start = 0, rng.randint(0, SIDE - 1)
        while x < SIDE:
            w = min(rng.randint(70, 150), SIDE - x)
            if SIDE - x - w < 40:
                w = SIDE - x
            name, base = rng.choice(CLOTHS)
            patches.append(((start + x) % SIDE, y0, w, y1, name, base, rng.randint(-10, 10)))
            x += w

    def cloth(name, base, shade, x, y, lx, ly):
        # each scrap a little sun-bleached along its top
        fade = max(0, 14 - ly // 3)
        r, g, b = (c + shade + fade for c in base)
        weave = 6 if (x // 2 + y // 2) % 2 else -6
        n = rng.randint(-4, 4)
        if name == "plaid":
            band = (lx // 18) % 3 == 0 or (ly // 18) % 3 == 0
            if band:
                r, g, b = r * 0.55, g * 0.62, b * 0.7
            if (lx % 18 < 3) or (ly % 18 < 3):
                r, g, b = r + 40, g + 34, b + 22
        elif name == "ticking":
            if (lx // 7) % 4 == 0:
                r, g, b = 70, 78, 96
        elif name == "denim":
            weave = 9 if (x + y) % 4 < 2 else -7
        return (clamp(r + weave + n), clamp(g + weave + n), clamp(b + weave + n))

    for x0, y0, w, y1, name, base, shade in patches:
        for y in range(y0, y1):
            for i in range(w):
                x = (x0 + i) % SIDE
                px[x, y] = cloth(name, base, shade, x, y, i, y - y0)
        # running stitches a few texels in from every edge
        for i in range(0, w, 6):
            for d in range(3):
                for yy in (y0 + 4, y1 - 5):
                    px[(x0 + i + d) % SIDE, yy] = STITCH
        for y in range(y0 + 4, y1 - 4, 6):
            for d in range(3):
                for xx in (x0 + 4, x0 + w - 5):
                    px[xx % SIDE, min(SIDE - 1, y + d)] = STITCH

    # Weather: darker toward the hem, scuffs and dirt specks everywhere.
    for y in range(SIDE):
        dark = 1.0 - 0.22 * (y / SIDE)
        for x in range(SIDE):
            r, g, b = px[x, y]
            k = dark * (0.9 if rng.random() < 0.04 else 1.0)
            px[x, y] = (clamp(r * k), clamp(g * k), clamp(b * k))
    # smudges: soft-edged, stretched sideways like wiped dirt, never round
    for _ in range(120):
        cx, cy = rng.randrange(SIDE), rng.randrange(34, SIDE - 34)
        rx, ry = rng.randint(6, 22), rng.randint(2, 6)
        for y in range(cy - ry, cy + ry + 1):
            for x in range(cx - rx, cx + rx + 1):
                d = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2
                if d <= 1 and 0 <= y < SIDE:
                    k = 1.0 - 0.14 * (1.0 - d)
                    r, g, b = px[x % SIDE, y]
                    px[x % SIDE, y] = (clamp(r * k), clamp(g * k * 0.99), clamp(b * k * 0.97))

    # The collar: a rope band at the top edge. The hem: a darker band, frayed.
    for y in range(0, 34):
        for x in range(SIDE):
            twist = ((x + y * 2) // 6) % 2
            c = tuple(clamp(v + (14 if twist else -14) + rng.randint(-5, 5)) for v in COLLAR)
            px[x, y] = c
    for y in range(SIDE - 34, SIDE):
        for x in range(SIDE):
            fray = y > SIDE - 10 and rng.random() < 0.35
            c = tuple(clamp(v + rng.randint(-8, 8) + (30 if fray else 0)) for v in HEM)
            px[x, y] = c

    out = Path(__file__).with_name("patchwork_tunic.png")
    img.save(out, optimize=True)
    print(f"{out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
