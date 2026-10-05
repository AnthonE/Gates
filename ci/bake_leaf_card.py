#!/usr/bin/env python3
"""Bake the broadleaf leaf card from ambientCG's CC0 photographed leaf sets.

The sources are flat sheets of single leaves (`LeafSet014`, a pale doubly
serrated birch/hornbeam leaf, and `LeafSet001`, a darker one of the same
shape), so the card is composed the way `ci/bake_needle_card.py` composes the
pine's: a thin dark twig from the attach point (bottom centre, where
`bevy_procedural_tree` roots every leaf quad), side shoots off it, and the
leaves hung on the shoots in clusters of two to five, the way a birch carries
them on its short shoots. Clusters, not a comb: evenly spaced leaves along
straight twigs read as a fern frond (`tree::leaf_image` says how that was
learned).

Each leaf gets its own value (0.62-1.1) so the card carries the light and dark
of leaves turned to and from the sun; the back ones are drawn first and darker.

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
from bake_needle_card import axis, lin_to_srgb, place, srgb_to_lin  # noqa: E402

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
# One leaf's length as a share of the card. The broadleaf's card measures
# ~1.25 m of tree (`tree::leaf_image`), so 0.066-0.096 is an 8-12 cm leaf.
LEAF_LEN = (0.066, 0.096)
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

    # The structure: a main twig, shoots off it alternately, and a pair of
    # sub-shoots off each shoot. The bare foot of the main twig is the first
    # 12 %, where the card meets its limb.
    main_pts = curve(root, rng.uniform(-5, 5), W * 0.84, rng.uniform(-12, 12))
    twigs = [(main_pts, 0.0070, 0.0026)]
    shoots = []
    plan = [(0.14, 1, 0.40), (0.24, -1, 0.42), (0.36, 1, 0.38), (0.47, -1, 0.36),
            (0.59, 1, 0.30), (0.70, -1, 0.28), (0.81, 1, 0.20)]
    for t, side, ln in plan:
        base = at(main_pts, t + rng.uniform(-0.03, 0.03))
        ang = heading(main_pts, t) + side * rng.uniform(36, 62)
        pts = curve(base, ang, W * ln * rng.uniform(0.85, 1.1), -side * rng.uniform(5, 30))
        shoots.append(pts)
        twigs.append((pts, 0.0040, 0.0015))
        for u in (0.35, 0.65):
            if rng.uniform() < 0.25:
                continue
            sb = at(pts, u + rng.uniform(-0.05, 0.05))
            sside = rng.choice([-1, 1])
            sub = curve(sb, heading(pts, u) + sside * rng.uniform(30, 55),
                        W * ln * rng.uniform(0.30, 0.45), -sside * rng.uniform(0, 20), n=5)
            shoots.append(sub)
            twigs.append((sub, 0.0024, 0.0012))

    # Leaf clusters on short spurs along every shoot and a fuller one at each
    # tip — clumps with air between them, which is the read a broadleaf spray
    # has and a comb of evenly spaced leaves does not.
    clusters = []  # (point, outward heading, leaf count)
    for pts in shoots:
        for t in (0.30, 0.55, 0.78):
            if rng.uniform() < 0.25:
                continue
            p = at(pts, t + rng.uniform(-0.08, 0.08))
            clusters.append((p, heading(pts, t) + rng.choice([-1, 1]) * rng.uniform(20, 70),
                             int(rng.integers(2, 4))))
        clusters.append((pts[-1], heading(pts, 1.0), int(rng.integers(5, 8))))
    for t in (0.55, 0.75, 0.92):
        clusters.append((at(main_pts, t), heading(main_pts, t)
                         + rng.choice([-1, 1]) * rng.uniform(25, 60), int(rng.integers(2, 4))))
    clusters.append((main_pts[-1], heading(main_pts, 1.0), 6))

    jobs = []
    for p, h, n in clusters:
        # A fan about the cluster's heading.
        spread = 160 if n >= 4 else 100
        for k in range(n):
            f = (k + 0.5) / n - 0.5
            ang = h + f * spread + rng.uniform(-18, 18)
            jobs.append((rng.uniform(0, 1), p, ang))
    jobs.sort(key=lambda j: j[0])

    for pts, w0, w1 in twigs:
        axis(canvas, pts, W * w0, W * w1, twig)

    for depth, p, ang in jobs:
        spr, stem = sprites[int(rng.integers(0, len(sprites)))]
        # Back leaves darker, front leaves at full value, with a per-leaf
        # spread that stands for the leaf's own angle to the light.
        value = (0.62 + 0.30 * depth) * rng.uniform(0.85, 1.12)
        tint = np.array([rng.uniform(0.94, 1.06), 1.0, rng.uniform(0.88, 1.06), 1.0], np.float32)
        s = spr * np.array([value, value, value, 1.0], np.float32) * tint
        # A short petiole gap: the leaf hangs a few mm off its spur.
        off = W * 0.006
        q = (p[0] + math.sin(math.radians(ang)) * off, p[1] - math.cos(math.radians(ang)) * off)
        place(canvas, s, q, stem, ang, W * rng.uniform(*LEAF_LEN),
              mirror=bool(rng.integers(0, 2)))

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
