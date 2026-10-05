#!/usr/bin/env python3
"""Bake the broadleaf leaf card from ambientCG's CC0 photographed leaf sets.

The sources are flat sheets of single leaves (`LeafSet014`, a pale doubly
serrated birch/hornbeam leaf, and `LeafSet001`, a darker one of the same
shape), so the card is composed the way `ci/bake_needle_card.py` composes the
pine's: a thin dark twig from the attach point (bottom centre, where
`bevy_procedural_tree` roots every leaf quad), side shoots off it, a fan of
leaves at every shoot tip and single leaves and pairs along the shoots at
random. Not a comb: evenly spaced leaves along straight twigs read as a fern
frond (`tree::leaf_image` says how that was learned).

Leaves keep clear of each other (`MAX_OVERLAP`, `GAP_PX`) and are hung until
the card reaches `TARGET_COVERAGE`: a cluster of overlapping leaves fuses into
one blob a few mips down, and from 6 m that blob read as one big smeared leaf.
For the same reason they are drawn larger than a birch's (10-13 cm on the
~0.95 m median card, `examples/tree_sweep.rs`): at 8 m a pixel is ~14 mm, and
a 7 cm leaf is a smudge.

Each leaf gets its own value (0.47-1.17) so the card carries the light and
dark of leaves turned to and from the sun, and neighbouring leaves on one card
never share a tone.

The RGB is stored **mean-normalised**, exactly like the needle card: divided by
the covered texels' linear mean and scaled by `K` so the brightest 0.5 % fit in
a byte. The canopy's colour stays in `tree.rs`'s vertex bands; `tree::
LEAF_MAP_GAIN` is `1/K` and `tests/tree.rs` checks the two agree.

Sources: ambientCG LeafSet014 and LeafSet001 (CC0), fetched into the gitignored
`assets/textures/candidates/leafsets/` (auto-downloaded if missing).

Run:  python ci/bake_leaf_card.py
Out:  assets/textures/leaf_card_albedo.png   (512² RGBA, alpha is the cutout)
"""
import io
import math
import sys
import urllib.request
import zipfile
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_needle_card import axis, composite, lin_to_srgb, srgb_to_lin, transformed  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "assets/textures/candidates/leafsets"
URL = "https://ambientcg.com/get?file={name}_1K-PNG.zip"
SETS = ["LeafSet014", "LeafSet001"]
OUT = ROOT / "assets/textures/leaf_card_albedo.png"

OUT_SIZE = 512
SS = 2
W = OUT_SIZE * SS
CUT = 128
ALPHA_GAIN = 1.15
# One leaf's length as a share of the card: 10-13 cm on the ~0.95 m median
# card (see the header for why that is larger than a birch's).
LEAF_LEN = (0.105, 0.135)
# How much of a new leaf may land on one already hung (grown by `GAP_PX`
# supersampled texels, ~2 texels of the card) before it is turned or dropped.
MAX_OVERLAP = 0.12
GAP_PX = 4
# Coverage at the 0.5 cut the card is filled to: the old card's, which
# `tests/tree.rs` holds the canopy's density to.
TARGET_COVERAGE = 0.228
SEED = 0xB12C4


def fetch(name: str) -> tuple[Path, Path]:
    d = SRC_DIR / name
    rgb, a = d / f"{name}_1K-PNG_Color.png", d / f"{name}_1K-PNG_Opacity.png"
    if not rgb.exists() or not a.exists():
        d.mkdir(parents=True, exist_ok=True)
        print(f"fetching {name}")
        req = urllib.request.Request(URL.format(name=name), headers={"User-Agent": "gates-bake"})
        with urllib.request.urlopen(req) as r:
            zipfile.ZipFile(io.BytesIO(r.read())).extractall(d)
    return rgb, a


def leaves(name: str) -> list[tuple[np.ndarray, tuple[float, float]]]:
    """Every leaf on a sheet as (premultiplied linear RGBA, stem point)."""
    rgb_p, a_p = fetch(name)
    rgb = srgb_to_lin(np.asarray(Image.open(rgb_p).convert("RGB")).astype(np.float32) / 255)
    a = np.asarray(Image.open(a_p).convert("L")).astype(np.float32) / 255
    # Each sheet divided by its own mean leaf colour first, so the two sets
    # differ only by the value and tint `main` gives each leaf — divided
    # together, the darker set's hue went purple against the paler one's mean.
    rgb = rgb / rgb[a > 0.5].mean(axis=0)
    lab, n = ndimage.label(a > 0.5, structure=np.ones((3, 3)))
    out = []
    for i, sl in enumerate(ndimage.find_objects(lab), start=1):
        if sl is None:
            continue
        ys, xs = sl
        if (ys.stop - ys.start) < 80:
            continue
        y0, y1 = max(ys.start - 3, 0), min(ys.stop + 3, a.shape[0])
        x0, x1 = max(xs.start - 3, 0), min(xs.stop + 3, a.shape[1])
        keep = ndimage.binary_dilation(lab[y0:y1, x0:x1] == i, iterations=3)
        alpha = np.where(keep, a[y0:y1, x0:x1], 0.0)
        spr = np.concatenate([rgb[y0:y1, x0:x1] * alpha[..., None], alpha[..., None]], axis=-1)
        # The petiole is the lowest opaque run: the sheets lay every leaf
        # tip-up.
        rows = np.where((alpha > 0.5).any(axis=1))[0]
        yb = rows[-1]
        xb = float(np.where(alpha[yb] > 0.5)[0].mean())
        out.append((spr, (xb, float(yb))))
    return out


def main() -> int:
    sprites = []
    for s in SETS:
        got = leaves(s)
        print(f"{s}: {len(got)} leaves")
        sprites += got
    rng = np.random.default_rng(SEED)

    # The twig colour, in the card's mean-1 space: the vertex green times
    # this is a dark olive-brown, darker than any leaf.
    twig = np.array([0.62, 0.30, 0.62], np.float32)

    canvas = np.zeros((W, W, 4), np.float32)
    root = (W / 2, W - 2)

    def curve(p0, ang_deg, length, bend, n=10):
        """Points along a gently bending shoot from `p0`, `ang_deg` off
        vertical (+ right), turning `bend` degrees over its length."""
        pts = [p0]
        x, y = p0
        seg = length / n
        for k in range(n):
            a = math.radians(ang_deg + bend * (k + 0.5) / n)
            x += math.sin(a) * seg
            y -= math.cos(a) * seg
            pts.append((x, y))
        return pts

    def at(pts, t):
        i = t * (len(pts) - 1)
        k = int(min(i, len(pts) - 2))
        f = i - k
        return (pts[k][0] + (pts[k + 1][0] - pts[k][0]) * f,
                pts[k][1] + (pts[k + 1][1] - pts[k][1]) * f)

    def heading(pts, t):
        a, b = at(pts, max(t - 0.05, 0.0)), at(pts, min(t + 0.05, 1.0))
        return math.degrees(math.atan2(b[0] - a[0], a[1] - b[1]))

    # The structure: a main twig, shoots off it alternately, and a sub-shoot
    # or two off each shoot. The bare foot of the main twig is the first 12 %,
    # where the card meets its limb.
    main_pts = curve(root, rng.uniform(-5, 5), W * 0.84, rng.uniform(-12, 12))
    twigs = [(main_pts, 0.0070, 0.0026)]
    shoots = []
    plan = [(0.14, 1, 0.42), (0.25, -1, 0.44), (0.37, 1, 0.40), (0.48, -1, 0.38),
            (0.60, 1, 0.32), (0.71, -1, 0.28), (0.82, 1, 0.20)]
    for t, side, ln in plan:
        base = at(main_pts, t + rng.uniform(-0.03, 0.03))
        ang = heading(main_pts, t) + side * rng.uniform(36, 62)
        pts = curve(base, ang, W * ln * rng.uniform(0.85, 1.1), -side * rng.uniform(5, 30))
        shoots.append(pts)
        twigs.append((pts, 0.0040, 0.0015))
        for u in (0.35, 0.65):
            if rng.uniform() < 0.35:
                continue
            sb = at(pts, u + rng.uniform(-0.05, 0.05))
            sside = rng.choice([-1, 1])
            sub = curve(sb, heading(pts, u) + sside * rng.uniform(30, 55),
                        W * ln * rng.uniform(0.30, 0.45), -sside * rng.uniform(0, 20), n=5)
            shoots.append(sub)
            twigs.append((sub, 0.0024, 0.0012))
    shoots.append(main_pts)

    for pts, w0, w1 in twigs:
        axis(canvas, pts, W * w0, W * w1, twig)

    # Leaf sites: a fan at every shoot tip, then single leaves and pairs along
    # the shoots at random, alternating sides. Tips go first so every shoot
    # ends in leaves however the card fills.
    def tip_sites():
        for pts in shoots:
            h = heading(pts, 1.0)
            n = int(rng.integers(2, 4))
            for k in range(n):
                f = (k + 0.5) / n - 0.5
                yield pts[-1], h + f * 110 + rng.uniform(-15, 15)

    def side_sites():
        while True:
            pts = shoots[int(rng.integers(0, len(shoots)))]
            t = rng.uniform(0.2, 0.95)
            side = rng.choice([-1, 1])
            h = heading(pts, t)
            yield at(pts, t), h + side * rng.uniform(30, 70)
            if rng.uniform() < 0.4:
                yield at(pts, t), h - side * rng.uniform(25, 60)

    # Each leaf must keep clear of the ones already hung: one whose opaque
    # footprint lands more than `MAX_OVERLAP` on another (grown by `GAP_PX`)
    # is turned a little and tried again, then dropped. Overlapping clusters
    # fuse into one blob a few mips down, and a blob lit as one card is what
    # read as a big smeared leaf from 6 m (2026-10-05).
    occupied = np.zeros((W, W), bool)
    jobs = []

    def try_leaf(p, ang):
        spr, stem = sprites[int(rng.integers(0, len(sprites)))]
        length = W * rng.uniform(*LEAF_LEN)
        mirror = bool(rng.integers(0, 2))
        for turn in (0.0, rng.uniform(15, 30), -rng.uniform(15, 30)):
            a = ang + turn
            # A short petiole gap: the leaf hangs a few mm off its spur.
            off = W * 0.006
            q = (p[0] + math.sin(math.radians(a)) * off, p[1] - math.cos(math.radians(a)) * off)
            layer, ox, oy = transformed(spr, q, stem, a, length, mirror)
            rh, rw = layer.shape[:2]
            x0, y0 = max(ox, 0), max(oy, 0)
            x1, y1 = min(ox + rw, W), min(oy + rh, W)
            if x0 >= x1 or y0 >= y1:
                continue
            solid = layer[y0 - oy:y1 - oy, x0 - ox:x1 - ox, 3] > 0.5
            area = solid.sum()
            if area < 20 or area < 0.97 * (layer[..., 3] > 0.5).sum():
                continue  # cut by the card edge: a straight edge reads
            hit = (solid & occupied[y0:y1, x0:x1]).sum()
            if hit > MAX_OVERLAP * area:
                continue
            grown = ndimage.binary_dilation(solid, iterations=GAP_PX)
            occupied[y0:y1, x0:x1] |= grown
            jobs.append((rng.uniform(0, 1), layer, ox, oy))
            return True
        return False

    def coverage():
        a = canvas[..., 3].copy()
        for _, layer, ox, oy in jobs:
            rh, rw = layer.shape[:2]
            x0, y0 = max(ox, 0), max(oy, 0)
            x1, y1 = min(ox + rw, W), min(oy + rh, W)
            src = layer[y0 - oy:y1 - oy, x0 - ox:x1 - ox, 3]
            a[y0:y1, x0:x1] = src + a[y0:y1, x0:x1] * (1 - src)
        red = a.reshape(OUT_SIZE, SS, OUT_SIZE, SS).mean(axis=(1, 3))
        return (np.clip(red * ALPHA_GAIN, 0, 1) > CUT / 255).mean()

    for p, ang in tip_sites():
        try_leaf(p, ang)
    tries = 0
    for p, ang in side_sites():
        tries += 1
        if try_leaf(p, ang) and len(jobs) % 8 == 0 and coverage() >= TARGET_COVERAGE:
            break
        if tries > 4000:
            break

    jobs.sort(key=lambda j: j[0])
    for depth, layer, ox, oy in jobs:
        # Back leaves darker, front leaves at full value, with a per-leaf
        # spread that stands for the leaf's own angle to the light. Wider than
        # a photo's own spread on purpose: neighbouring leaves on one card
        # must differ in value, or the card reads as one flat plate.
        value = (0.50 + 0.55 * depth) * rng.uniform(0.85, 1.12)
        tint = np.array([rng.uniform(0.94, 1.06), 1.0, rng.uniform(0.88, 1.06), 1.0], np.float32)
        composite(canvas, layer * np.array([value, value, value, 1.0], np.float32) * tint, ox, oy)

    # Reduce SSx: premultiplied box, so colour is weighted by coverage.
    c = canvas.reshape(OUT_SIZE, SS, OUT_SIZE, SS, 4).mean(axis=(1, 3))
    alpha = c[..., 3]
    col = np.where(alpha[..., None] > 1e-4, c[..., :3] / np.maximum(alpha[..., None], 1e-4), 0)
    alpha = np.clip(alpha * ALPHA_GAIN, 0, 1)

    covered = alpha > CUT / 255
    mean = col[covered].mean(axis=0)
    norm = col / mean
    k = 1.0 / np.percentile(norm[covered].max(axis=1), 99.5)
    col = norm * k

    known = alpha > 0.02
    rgb_out = col.copy()
    for _ in range(8):
        if known.all():
            break
        kern = np.ones((3, 3))
        cnt = ndimage.convolve(known.astype(np.float32), kern, mode="nearest")
        acc = np.stack([ndimage.convolve(rgb_out[..., i] * known, kern, mode="nearest")
                        for i in range(3)], axis=-1)
        fill = (cnt > 0) & ~known
        avg = acc / np.maximum(cnt[..., None], 1)
        rgb_out[fill] = avg[fill]
        known = known | fill
    rgb_out[~known] = k

    out = np.concatenate([lin_to_srgb(rgb_out), alpha[..., None]], axis=-1)
    Image.fromarray((out * 255 + 0.5).clip(0, 255).astype(np.uint8), "RGBA").save(OUT, optimize=True)
    print(f"wrote {OUT.relative_to(ROOT)} {OUT_SIZE}x{OUT_SIZE} "
          f"({OUT.stat().st_size:,} bytes) coverage {covered.mean():.3f} leaves {len(jobs)} "
          f"source mean lin rgb {mean.round(4).tolist()} K {k:.4f} gain {1 / k:.4f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
