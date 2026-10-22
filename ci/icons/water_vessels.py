#!/usr/bin/env python3
"""Paint the water pass's salt waterskin icon (assets/icons/salt_waterskin.png).

It is `full_waterskin.png` salted — greyed, pale, a crust of crystals — so
it keeps that icon's credit. `python3 ci/icons/water_vessels.py`.
"""
import os
import random

from PIL import Image, ImageDraw

src = out = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "icons")

# Salt waterskin: the full skin, its belly crusted pale with dried salt.
skin = Image.open(f"{src}/full_waterskin.png").convert("RGBA")
r, g, b, a = skin.split()
grey = Image.merge("RGB", (r, g, b)).convert("L")
pale = Image.merge("RGB", [grey.point(lambda v: min(255, int(v * 0.55 + 120)))] * 3)
tint = Image.new("RGB", skin.size, (236, 232, 222))
salt = Image.blend(pale, tint, 0.25)
mix = Image.blend(Image.merge("RGB", (r, g, b)), salt, 0.3)
# Speckle of crystals.
d = ImageDraw.Draw(mix)
random.seed(7)
px = a.load()
for _ in range(260):
    x, y = random.randrange(128), random.randrange(128)
    if px[x, y] > 200:
        v = random.randrange(225, 255)
        d.point((x, y), fill=(v, v, v - 6))
Image.merge("RGBA", (*mix.split(), a)).save(f"{out}/salt_waterskin.png")

