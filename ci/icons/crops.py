#!/usr/bin/env python3
"""Paint farming v2's second-wave icons into assets/icons/.

Our own work: a potato, a heap of cut potato eyes, a pumpkin, a heap of
pumpkin seeds and a pinch of berry seeds, each a few shaded blobs with the
depot's dark rim and soft drop shadow, drawn at 4x and reduced.
`python3 ci/icons/crops.py` (needs numpy and Pillow).
"""
import math
import os

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

S = 4
N = 128 * S
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "icons")
INK = np.array([20, 18, 16], dtype=np.float32)
# Light from the upper left, toward the viewer.
LIGHT = np.array([-0.45, -0.6, 0.66])
LIGHT = LIGHT / np.linalg.norm(LIGHT)
HALF = LIGHT + np.array([0.0, 0.0, 1.0])
HALF = HALF / np.linalg.norm(HALF)

YY, XX = np.mgrid[0:N, 0:N].astype(np.float32) + 0.5


def rng(seed):
    return np.random.default_rng(seed)


class Canvas:
    def __init__(self):
        self.rgb = np.zeros((N, N, 3), dtype=np.float32)
        self.a = np.zeros((N, N), dtype=np.float32)

    def shadow(self, cx, cy, rx, ry, alpha=110):
        im = Image.new("L", (N, N), 0)
        ImageDraw.Draw(im).ellipse(
            [(cx - rx) * S, (cy - ry) * S, (cx + rx) * S, (cy + ry) * S], fill=alpha
        )
        sh = np.asarray(im.filter(ImageFilter.GaussianBlur(4 * S)), dtype=np.float32) / 255.0
        self.over(np.zeros((N, N, 3), dtype=np.float32), sh)

    def over(self, rgb, a):
        a = a[..., None]
        out_a = a[..., 0] + self.a * (1.0 - a[..., 0])
        safe = np.maximum(out_a, 1e-6)[..., None]
        self.rgb = (rgb * a + self.rgb * self.a[..., None] * (1.0 - a)) / safe
        self.a = out_a

    def blob(
        self,
        cx,
        cy,
        rx,
        ry,
        base,
        angle=0.0,
        lumps=(),
        shine=0.25,
        rim=1.0,
        spots=None,
        flat=1.0,
    ):
        """A shaded ellipsoid seen from the front, centre (cx, cy) and radii
        (rx, ry) in 128-px units, turned `angle` radians, its outline pushed
        out by `lumps` [(k, amp, phase)] and inked `rim` px wide."""
        ca, sa = math.cos(angle), math.sin(angle)
        dx, dy = XX / S - cx, YY / S - cy
        u = (dx * ca + dy * sa) / rx
        v = (-dx * sa + dy * ca) / ry
        rho = np.sqrt(u * u + v * v)
        th = np.arctan2(v, u)
        edge = np.ones_like(rho)
        for k, amp, ph in lumps:
            edge = edge + amp * np.sin(k * th + ph)
        q = rho / edge
        # One 128-px unit in q, and a quarter of one (a 4x pixel) to feather.
        unit = 1.0 / min(rx, ry)
        feather = unit / S
        cover = np.clip((1.0 - q) / feather + 0.5, 0.0, 1.0)
        uu, vv = u / edge, v / edge
        nz = np.sqrt(np.clip(1.0 - uu * uu - vv * vv, 0.0, 1.0)) * flat
        # Back into image axes for the light.
        wx = uu * ca - vv * sa
        wy = uu * sa + vv * ca
        n = np.stack([wx, wy, nz], axis=-1)
        n = n / np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-6)
        diff = np.clip(n @ LIGHT, 0.0, 1.0)
        spec = np.clip(n @ HALF, 0.0, 1.0) ** 28
        col = np.array(base, dtype=np.float32)
        rgb = col * (0.38 + 0.78 * diff[..., None])
        rgb = rgb + 255.0 * shine * spec[..., None]
        if spots is not None:
            rgb = spots(rgb, uu, vv)
        ring = np.clip((q - (1.0 - rim * unit)) / feather + 0.5, 0.0, 1.0)
        rgb = rgb * (1.0 - ring[..., None]) + INK * ring[..., None]
        self.over(np.clip(rgb, 0, 255), cover)

    def image(self):
        out = np.dstack([self.rgb, self.a * 255.0]).clip(0, 255).astype(np.uint8)
        return Image.fromarray(out, "RGBA").resize((128, 128), Image.LANCZOS)


def potato_spots(seed):
    r = rng(seed)
    eyes = [(r.uniform(-0.6, 0.6), r.uniform(-0.5, 0.5)) for _ in range(5)]

    def f(rgb, u, v):
        for eu, ev in eyes:
            d = np.sqrt((u - eu) ** 2 + ((v - ev) * 1.6) ** 2)
            k = np.clip(1.0 - d / 0.09, 0.0, 1.0)[..., None]
            rgb = rgb * (1.0 - 0.45 * k)
        # Dry soil specks.
        speck = (np.sin(u * 61.0 + 1.3) * np.sin(v * 57.0 - 0.7) > 0.93)[..., None]
        return np.where(speck, rgb * 0.7, rgb)

    return f


def paint_potato():
    c = Canvas()
    c.shadow(64, 108, 54, 10)
    skin = (186, 142, 88)
    c.blob(86, 54, 30, 23, (170, 128, 78), angle=-0.5, lumps=[(3, 0.05, 0.4), (5, 0.03, 1.1)], spots=potato_spots(2), shine=0.08)
    c.blob(56, 74, 48, 31, skin, angle=-0.35, lumps=[(2, 0.04, 0.2), (3, 0.05, 1.7), (5, 0.025, 0.4)], spots=potato_spots(1), shine=0.1)
    return c.image()


def paint_potato_seed():
    c = Canvas()
    c.shadow(64, 106, 46, 9)
    r = rng(7)
    skin = (170, 128, 78)
    flesh = (236, 222, 170)
    pieces = [(40, 78), (68, 84), (90, 74), (54, 58), (80, 56), (30, 96), (64, 98), (94, 96)]
    for i, (x, y) in enumerate(pieces):
        a = r.uniform(-1.2, 1.2)
        rx, ry = r.uniform(13, 17), r.uniform(9, 12)
        c.blob(x, y, rx, ry, skin, angle=a, lumps=[(3, 0.06, r.uniform(0, 6))], shine=0.06)
        # The cut face: pale flesh on the upper side of the chunk.
        c.blob(x - 2, y - 2, rx * 0.72, ry * 0.62, flesh, angle=a, lumps=[(3, 0.05, r.uniform(0, 6))], shine=0.12, rim=0.5, flat=0.4)
        if i % 3 == 0:
            # A sprouting eye.
            c.blob(x + rx * 0.4, y - ry * 0.7, 3.2, 3.2, (128, 96, 150), shine=0.2, rim=0.6)
    return c.image()


def paint_pumpkin():
    c = Canvas()
    c.shadow(64, 108, 48, 10)
    orange = (226, 118, 28)
    # Lobes back to front: the outer ones first, the middle last.
    for x, rx, k in [(30, 18, 0.82), (98, 18, 0.82), (44, 22, 0.92), (84, 22, 0.92), (64, 24, 1.0)]:
        col = tuple(v * k for v in orange)
        c.blob(x, 72, rx, 34, col, lumps=[(2, 0.03, 0.3)], shine=0.22)
    # The stem, curling off the top.
    im = c.image().resize((N, N), Image.LANCZOS)
    d = ImageDraw.Draw(im)
    stem = [(60, 42), (64, 30), (70, 22), (78, 20)]
    d.line([(x * S, y * S) for x, y in stem], fill=(20, 18, 16, 255), width=11 * S, joint="curve")
    d.line([(x * S, y * S) for x, y in stem], fill=(112, 104, 52, 255), width=7 * S, joint="curve")
    d.line([(x * S - 2 * S, y * S) for x, y in stem[:2]], fill=(150, 142, 82, 255), width=2 * S)
    return im.resize((128, 128), Image.LANCZOS)


def seed_heap(seed, base, n, r0, r1, pointed=False, rows=((64, 80, 34),)):
    c = Canvas()
    c.shadow(64, 104, 46, 9)
    r = rng(seed)
    spots = []
    for cx, cy, spread in rows:
        for _ in range(n):
            a = r.uniform(0, 2 * math.pi)
            d = spread * math.sqrt(r.uniform(0, 1))
            spots.append((cx + d * math.cos(a), cy + d * math.sin(a) * 0.55))
    spots.sort(key=lambda p: p[1])
    for x, y in spots:
        rx = r.uniform(r0, r1)
        ry = rx * r.uniform(0.55, 0.7)
        k = r.uniform(0.88, 1.08)
        col = tuple(min(255, v * k) for v in base)
        lumps = [(1, 0.18, r.uniform(0, 6))] if pointed else [(3, 0.04, r.uniform(0, 6))]
        c.blob(x, y, rx, ry, col, angle=r.uniform(-1.4, 1.4), lumps=lumps, shine=0.3, rim=0.9)
    return c.image()


def paint_pumpkin_seed():
    return seed_heap(11, (236, 224, 182), 24, 11, 14, pointed=True, rows=((64, 78, 42),))


def paint_berry_seed():
    return seed_heap(13, (132, 58, 44), 46, 6.5, 8, rows=((64, 80, 42),))


for name, paint in [
    ("potato", paint_potato),
    ("potato_seed", paint_potato_seed),
    ("pumpkin", paint_pumpkin),
    ("pumpkin_seed", paint_pumpkin_seed),
    ("berry_seed", paint_berry_seed),
]:
    paint().save(os.path.join(OUT, f"{name}.png"), optimize=True)
    print(f"wrote {name}")
