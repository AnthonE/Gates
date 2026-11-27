#!/usr/bin/env python3
"""Paint the strange gear's icons (assets/icons/{humming_blade,still_helm,
bright_draught}.png): the longsword's, the coffee can's and the tonic's,
recoloured through a cold verdigris ramp. They wear the forged pieces'
models until they have their own. `python3 ci/icons/strange_gear.py`.
"""
import os
from PIL import Image

OUT = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "icons")
STOPS = [(0.0, (8, 22, 30)), (0.45, (60, 140, 150)), (0.8, (170, 235, 230)), (1.0, (250, 255, 250))]
# (source stem, new stem, gain on the source's luminance)
PAIRS = [
    ("longsword", "humming_blade", 1.25),
    ("coffee_can_helmet", "still_helm", 1.3),
    ("charcoal_tonic", "bright_draught", 1.6),
]


def ramp(t):
    for (a, ca), (b, cb) in zip(STOPS, STOPS[1:]):
        if t <= b:
            u = (t - a) / (b - a)
            return tuple(int(ca[i] + (cb[i] - ca[i]) * u) for i in range(3))
    return STOPS[-1][1]


LUT = [ramp(i / 255) for i in range(256)]

for src, dst, gain in PAIRS:
    im = Image.open(os.path.join(OUT, src + ".png")).convert("RGBA")
    px = im.load()
    for y in range(im.height):
        for x in range(im.width):
            r, g, b, a = px[x, y]
            c = LUT[min(255, int((0.299 * r + 0.587 * g + 0.114 * b) * gain))]
            px[x, y] = (c[0], c[1], c[2], a)
    im.save(os.path.join(OUT, dst + ".png"), optimize=True)
