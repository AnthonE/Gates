#!/usr/bin/env python3
"""Paint the water catcher's icon (assets/icons/water_catcher.png).

Our own work: a dented blue barrel standing in a frame of four wooden poles,
under an olive tarp sagging into a funnel over its mouth, drawn at 4x and
reduced. `python3 ci/icons/water_catcher.py`.
"""
import os
from PIL import Image, ImageDraw, ImageFilter

S = 4
N = 128 * S
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "icons")

WOOD = (154, 106, 62, 255)
WOOD_DARK = (96, 64, 38, 255)
BLUE = (58, 106, 176, 255)
BLUE_HI = (104, 150, 214, 255)
BLUE_LO = (34, 66, 118, 255)
TARP = (104, 120, 60, 255)
TARP_HI = (140, 158, 86, 255)
TARP_LO = (66, 78, 38, 255)
INK = (20, 20, 24, 255)


def s(*v):
    return tuple(int(x * S) for x in v)


def paint():
    im = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).ellipse(s(22, 108, 106, 122), fill=(0, 0, 0, 110))
    im.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(4 * S)))
    d = ImageDraw.Draw(im)

    # The back poles, behind the barrel.
    for x in (34, 94):
        d.rectangle(s(x - 2, 30, x + 2, 104), fill=WOOD_DARK, outline=INK, width=S)

    # The barrel: a body, two hoops and a lit side.
    d.rounded_rectangle(s(40, 62, 88, 112), 6 * S, fill=BLUE, outline=INK, width=2 * S)
    d.rectangle(s(46, 66, 53, 108), fill=BLUE_HI)
    d.rectangle(s(78, 66, 84, 108), fill=BLUE_LO)
    for y in (74, 98):
        d.rectangle(s(40, y, 88, y + 4), fill=BLUE_LO, outline=INK, width=S)
    d.ellipse(s(42, 56, 86, 68), fill=BLUE_LO, outline=INK, width=2 * S)
    d.ellipse(s(52, 59, 76, 65), fill=(12, 16, 22, 255))

    # The hose from the funnel into the barrel's mouth.
    d.rectangle(s(61, 44, 67, 62), fill=(28, 28, 32, 255), outline=INK, width=S)

    # The tarp: a sagging funnel slung between the pole tops.
    funnel = [s(18, 30), s(110, 30), s(102, 36), s(70, 50), s(58, 50), s(26, 36)]
    d.polygon([p for xy in funnel for p in [xy]], fill=TARP, outline=INK)
    d.line(funnel + [funnel[0]], fill=INK, width=2 * S, joint="curve")
    d.polygon([s(24, 31), s(104, 31), s(98, 34), s(30, 34)], fill=TARP_HI)
    d.polygon([s(70, 50), s(102, 36), s(96, 38), s(68, 47)], fill=TARP_LO)

    # The front poles and the rope at their tops.
    for x in (20, 108):
        d.rectangle(s(x - 3, 26, x + 3, 112), fill=WOOD, outline=INK, width=S)
        d.ellipse(s(x - 5, 26, x + 5, 34), fill=(204, 170, 112, 255), outline=INK, width=S)
    return im.resize((128, 128), Image.LANCZOS)


paint().save(os.path.join(OUT, "water_catcher.png"), optimize=True)
print("wrote water_catcher")
