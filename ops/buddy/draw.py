#!/usr/bin/env python3
"""Draw Doctor Wood's profile picture: ops/buddy/doctor-wood.png.

Original pixel art, CC0 (ours). 32x32 drawn here, scaled 8x with nearest
neighbour to 256x256: a square 8-bit RGB PNG, which is what the platform's
face rule takes (scry-forge meter/faces.py: 64-512 px, at most 256 KiB).

    python3 ops/buddy/draw.py            # rewrites doctor-wood.png beside it
"""
from pathlib import Path

from PIL import Image

N = 32
SCALE = 8

BG = (35, 80, 94)
HALO = (45, 100, 116)
OUTLINE = (43, 26, 16)
WOOD = (176, 118, 62)
GRAIN = (138, 90, 43)
WOOD_LIT = (204, 146, 84)
EYE = (244, 239, 230)
PUPIL = (27, 27, 27)
MIRROR = (217, 222, 227)
RIM = (140, 150, 160)
SHINE = (255, 255, 255)
BAND = (58, 58, 68)
COAT = (242, 242, 242)
COAT_SHADE = (196, 205, 212)
TUBE = (60, 110, 143)
METAL = (184, 194, 204)
LEAF = (95, 174, 78)
LEAF_DARK = (63, 127, 54)
CHEEK = (196, 106, 74)
MOUTH = (74, 42, 24)


def draw() -> Image.Image:
    px = [[BG] * N for _ in range(N)]

    def put(x: int, y: int, c) -> None:
        if 0 <= x < N and 0 <= y < N:
            px[y][x] = c

    # a soft halo behind the head
    for y in range(N):
        for x in range(N):
            if (x - 15.5) ** 2 + (y - 15) ** 2 <= 14.5**2:
                px[y][x] = HALO

    # the coat: a trapezoid that widens to the bottom edge, a V collar
    for y in range(23, N):
        half = 8 + (y - 23)
        for x in range(16 - half, 16 + half):
            put(x, y, COAT)
        put(16 - half, y, COAT_SHADE)
        put(15 + half, y, COAT_SHADE)
    for y in range(23, 28):
        d = y - 23
        put(15 - d, y, COAT_SHADE)
        put(16 + d, y, COAT_SHADE)
    # the neck, in wood, inside the collar
    for y in range(22, 25):
        for x in range(13, 19):
            put(x, y, GRAIN)
    for y in range(25, 27):
        for x in range(14 + (y - 25), 18 - (y - 25)):
            put(x, y, GRAIN)

    # the head: a block of wood with rounded corners
    x0, x1, y0, y1, r = 8, 23, 6, 22, 3
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            cx = min(max(x, x0 + r), x1 - r)
            cy = min(max(y, y0 + r), y1 - r)
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r + 1:
                put(x, y, WOOD)
    # its outline
    inside = {(x, y) for y in range(N) for x in range(N) if px[y][x] == WOOD}
    for x, y in inside:
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            if (x + dx, y + dy) not in inside and px[y + dy][x + dx] != GRAIN:
                put(x + dx, y + dy, OUTLINE)
    # grain: wavy vertical lines and a lit edge
    # (continuous, and kept off the middle of the face, where a broken
    # line reads as spots)
    for gx, phase in ((10, 0), (20, 3)):
        for y in range(y0 + 1, y1):
            step = 1 if (y + phase) % 8 >= 4 else 0
            for x in (gx + step, gx + (1 if (y - 1 + phase) % 8 >= 4 else 0)):
                if (x, y) in inside and px[y][x] == WOOD:
                    put(x, y, GRAIN)
    for y in range(y0 + 1, 11):
        x = 17 + (1 if y % 4 >= 2 else 0)
        if (x, y) in inside and px[y][x] == WOOD:
            put(x, y, GRAIN)
    for y in range(y0 + 2, y1 - 1):
        put(x0 + 1, y, WOOD_LIT)
    # a knot low on the right cheek
    for x, y in ((20, 19), (21, 19), (20, 20), (21, 20)):
        put(x, y, GRAIN)
    put(21, 20, OUTLINE)

    # the head mirror on its band
    for x in range(x0, x1 + 1):
        put(x, 9, BAND)
    mx, my = 13, 8
    for y in range(my - 3, my + 4):
        for x in range(mx - 3, mx + 4):
            d = (x - mx) ** 2 + (y - my) ** 2
            if d <= 9:
                put(x, y, RIM if d >= 6 else MIRROR)
    put(12, 7, SHINE)
    put(12, 8, SHINE)
    put(13, 7, SHINE)

    # brows, eyes, cheeks, smile
    for x in (11, 12, 13, 18, 19, 20):
        put(x, 12, OUTLINE)
    for ex in (11, 18):
        for y in (13, 14, 15):
            for x in (ex, ex + 1, ex + 2):
                put(x, y, EYE)
        put(ex + 1, 14, PUPIL)
        put(ex + 2, 14, PUPIL)
        put(ex + 1, 15, PUPIL)
        put(ex + 2, 15, PUPIL)
    put(10, 17, CHEEK)
    put(21, 17, CHEEK)
    put(12, 18, MOUTH)
    put(19, 18, MOUTH)
    for x in range(13, 19):
        put(x, 19, MOUTH)

    # the stethoscope: around the neck, two tubes down the coat, a bell
    for x in range(11, 21):
        put(x, 23, TUBE)
    for y in range(24, 29):
        put(11, y, TUBE)
    for y in range(24, 27):
        put(20, y, TUBE)
    for x, y in ((19, 27), (20, 27), (21, 27), (19, 28), (20, 28), (21, 28)):
        put(x, y, METAL)
    put(20, 28, OUTLINE)
    put(11, 29, METAL)
    put(12, 29, METAL)

    # a sprout: he is still growing
    for y in (3, 4, 5):
        put(16, y, GRAIN)
    for x, y in ((14, 2), (15, 2), (15, 3)):
        put(x, y, LEAF)
    put(14, 3, LEAF_DARK)
    for x, y in ((17, 1), (18, 1), (17, 2)):
        put(x, y, LEAF)
    put(18, 2, LEAF_DARK)

    img = Image.new("RGB", (N, N))
    img.putdata([c for row in px for c in row])
    return img.resize((N * SCALE, N * SCALE), Image.NEAREST)


if __name__ == "__main__":
    out = Path(__file__).with_name("doctor-wood.png")
    draw().save(out, optimize=True)
    print(f"{out} ({out.stat().st_size} bytes)")
