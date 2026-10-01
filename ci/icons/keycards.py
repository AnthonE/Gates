#!/usr/bin/env python3
"""Paint the three keycard icons (assets/icons/{green,blue,red}_keycard.png).

Our own work: a tilted card with a gilt chip, a magnetic stripe and a lapis
glyph, drawn at 4x and reduced. `python3 ci/icons/keycards.py`.
"""
import os
from PIL import Image, ImageDraw, ImageFilter

S = 4
N = 128 * S
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "icons")
CARDS = {
    "green_keycard": (52, 150, 72),
    "blue_keycard": (48, 96, 200),
    "red_keycard": (186, 44, 38),
}


def card(rgb):
    im = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    body = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(body)
    x0, y0, x1, y1 = 14 * S, 34 * S, 114 * S, 96 * S
    shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (x0 + 4 * S, y0 + 5 * S, x1 + 4 * S, y1 + 5 * S), 9 * S, fill=(0, 0, 0, 120)
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(4 * S))
    d.rounded_rectangle((x0, y0, x1, y1), 9 * S, fill=rgb + (255,), outline=(20, 20, 24, 255), width=3 * S)
    # A lighter top band, the stripe and the chip.
    hi = tuple(min(255, c + 50) for c in rgb)
    d.rounded_rectangle((x0 + 3 * S, y0 + 3 * S, x1 - 3 * S, y0 + 16 * S), 7 * S, fill=hi + (255,))
    d.rectangle((x0 + 3 * S, y1 - 22 * S, x1 - 3 * S, y1 - 14 * S), fill=(24, 24, 28, 255))
    d.rounded_rectangle((x0 + 12 * S, y0 + 22 * S, x0 + 34 * S, y0 + 38 * S), 3 * S,
                        fill=(226, 180, 86, 255), outline=(120, 86, 30, 255), width=S)
    for k in range(1, 3):
        yy = y0 + 22 * S + k * 16 * S // 3
        d.line((x0 + 12 * S, yy, x0 + 34 * S, yy), fill=(150, 110, 40, 255), width=S)
    # The glyph: a stepped pyramid.
    gx, gy = x1 - 34 * S, y0 + 40 * S
    for i, w in enumerate((22, 15, 8)):
        d.rectangle((gx - w * S // 2 + 11 * S, gy - (i + 1) * 6 * S, gx + w * S // 2 + 11 * S, gy - i * 6 * S),
                    fill=(235, 240, 255, 230))
    body = body.rotate(12, resample=Image.BICUBIC, center=(N // 2, N // 2))
    shadow = shadow.rotate(12, resample=Image.BICUBIC, center=(N // 2, N // 2))
    im.alpha_composite(shadow)
    im.alpha_composite(body)
    return im.resize((128, 128), Image.LANCZOS)


for stem, rgb in CARDS.items():
    card(rgb).save(os.path.join(OUT, stem + ".png"), optimize=True)
    print("wrote", stem)
