#!/usr/bin/env python3
"""Draw the Field Medic Tunic: assets/skins/field_medic_tunic.png.

The first painted skin, and the proof of the path: drawn here on the tunic's
canvas (`skinkit template tunic`), CC0. The top is the image's upper half
(collar at the top edge), the skirt the lower half (hem at the bottom); left
to right is one turn starting at the chest, so the chest cross wraps the
left and right edges and the back cross sits in the middle.

    python3 assets/skins/draw_field_medic.py
"""
import random
from pathlib import Path

from PIL import Image

SIDE = 512
CLOTH = (178, 170, 148)
WEAVE = 7
RED = (168, 34, 30)
BELT = (74, 52, 34)
BUCKLE = (150, 128, 70)


def main() -> None:
    rng = random.Random(9)
    img = Image.new("RGB", (SIDE, SIDE))
    px = img.load()
    for y in range(SIDE):
        for x in range(SIDE):
            # A coarse weave: alternating threads, a little noise.
            w = WEAVE if (x // 2 + y // 2) % 2 else -WEAVE
            n = rng.randint(-5, 5)
            px[x, y] = tuple(max(0, min(255, c + w + n)) for c in CLOTH)

    def rect(x0, y0, x1, y1, col):
        for y in range(max(0, y0), min(SIDE, y1)):
            for x in range(x0, x1):
                px[x % SIDE, y] = col

    def cross(cx, cy):
        # Below the hood's hem (v about 0.36 of the image: a hood covers the
        # chest above it), a square cross on cloth that is wider around than
        # it is tall per texel.
        arm_w, arm_h = 24, 28
        long_u, long_v = 64, 80
        rect(cx - long_u // 2, cy - arm_h // 2, cx + long_u // 2, cy + arm_h // 2, RED)
        rect(cx - arm_w // 2, cy - long_v // 2, cx + arm_w // 2, cy + long_v // 2, RED)

    cy = int(0.36 * SIDE)
    cross(0, cy)            # the chest: wraps the left and right edges
    cross(SIDE // 2, cy)    # the back
    # The belt at the waist (the halves meet at v 0.5), a buckle at the chest.
    rect(0, SIDE // 2 - 34, SIDE, SIDE // 2 + 16, BELT)
    rect(-12, SIDE // 2 - 30, 12, SIDE // 2 + 12, BUCKLE)
    # A red band above the hem.
    rect(0, SIDE - 30, SIDE, SIDE - 14, RED)

    out = Path(__file__).with_name("field_medic_tunic.png")
    img.save(out, optimize=True)
    print(f"{out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
