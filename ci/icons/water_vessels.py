#!/usr/bin/env python3
"""Paint the water pass's vessel icons (assets/icons/salt_waterskin.png,
water_jug.png, full_water_jug.png).

The salt skin is `full_waterskin.png` salted — greyed, pale, a crust of
crystals — so it keeps that icon's credit. The jug is our own work: a clay
jug with a cork and a handle, drawn at 4x and reduced; fresh wears a drop
of water at the lip. `python3 ci/icons/water_vessels.py`.
"""
import math
import os
import random

from PIL import Image, ImageChops, ImageDraw, ImageFilter

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

# The jug: a clay body, a neck, a stopper, a handle; drawn at 4x and shrunk.
S = 512
def jug(water):
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    clay = (150, 92, 58)
    dark = (96, 56, 34)
    light = (196, 132, 86)
    # Handle (behind): a thick arc on the right.
    d.arc((300, 150, 440, 330), start=-80, end=95, fill=dark, width=34)
    # Body.
    d.ellipse((100, 170, 400, 470), fill=clay)
    # Neck and lip.
    d.rounded_rectangle((195, 90, 305, 210), radius=24, fill=clay)
    d.rounded_rectangle((180, 78, 320, 112), radius=14, fill=dark)
    # Stopper: cork.
    d.rounded_rectangle((210, 40, 290, 92), radius=12, fill=(176, 140, 92))
    # Shading: a soft dark on the right and light on the left.
    shade = Image.new("L", (S, S), 0)
    sd = ImageDraw.Draw(shade)
    sd.ellipse((210, 190, 420, 480), fill=110)
    shade = shade.filter(ImageFilter.GaussianBlur(40))
    body_mask = im.split()[3]
    darkl = Image.new("RGBA", (S, S), dark + (255,))
    im = Image.composite(darkl, im, ImageChops.multiply(shade, body_mask))
    hi = Image.new("L", (S, S), 0)
    hd = ImageDraw.Draw(hi)
    hd.ellipse((140, 220, 230, 330), fill=150)
    hi = hi.filter(ImageFilter.GaussianBlur(30))
    lightl = Image.new("RGBA", (S, S), light + (255,))
    im = Image.composite(lightl, im, ImageChops.multiply(hi, body_mask))
    d = ImageDraw.Draw(im)
    # A band round the shoulder.
    d.arc((118, 200, 382, 330), start=200, end=340, fill=dark, width=10)
    if water == "fresh":
        # A drip of clear water down the neck.
        d.ellipse((232, 210, 262, 250), fill=(120, 190, 230, 255))
        d.polygon([(247, 190), (232, 228), (262, 228)], fill=(120, 190, 230, 255))
    # Outline for legibility at 44 px.
    a = im.split()[3]
    edge = a.filter(ImageFilter.MaxFilter(9))
    outline = Image.new("RGBA", (S, S), (40, 24, 16, 255))
    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    base = Image.composite(outline, base, edge)
    base.alpha_composite(im)
    return base.resize((128, 128), Image.LANCZOS)

jug(None).save(f"{out}/water_jug.png")
jug("fresh").save(f"{out}/full_water_jug.png")
