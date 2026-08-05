#!/usr/bin/env python3
"""The tonal bar, measured on both sides by one estimator.

`ci/reference_bar.mjs` does this for the browser client by decoding in a page.
The native client's captures are PNGs on disk, so this reads them directly —
and, critically, reads `Rust Images/` the same way in the same run. A bar
computed a different way than the frame it judges is not a bar (`ART.md` §7).

Rec.601 luma, whole frame, plus the sky band (top 25% of rows) and near band
(bottom 35%), which are the bands `ART.md` §3's table is stated in.

    python3 ci/native_bar.py <dir-of-pngs>

Prints one row per frame plus the reference median, so a capture is read next
to the thing it is being compared to rather than against a remembered number.
"""

import sys
import os
import glob
import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# The outdoor-daylight subset, identical to ci/reference_bar.mjs's list. The
# interiors, menus and maps are excluded on purpose: a tonal bar taken over a
# fullscreen inventory screen would be a bar about UI, not about light.
REFERENCE_FRAMES = [
    "generichighview2.jpg",
    "gameplayfoundbase.jpeg",
    "choppingtree.jpg",
    "spawnedrock.jpg",
    "generic2.jpeg",
    "roads.jpeg",
]


def stats(path):
    im = Image.open(path).convert("RGB")
    a = np.asarray(im).astype(np.float32)
    luma = 0.299 * a[:, :, 0] + 0.587 * a[:, :, 1] + 0.114 * a[:, :, 2]
    h = luma.shape[0]
    sky = luma[: max(1, h // 4)]
    near = luma[h - max(1, int(h * 0.35)) :]
    # Near-band neighbour contrast: |L(x+1) - L(x)| averaged over the band.
    # ART.md §3's last row, the one nothing has ever moved.
    d = np.abs(np.diff(near, axis=1))
    # Saturation, HSV S, over the near band.
    n = a[a.shape[0] - near.shape[0] :]
    mx = n.max(axis=2)
    mn = n.min(axis=2)
    sat = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
    return {
        "p10": float(np.percentile(luma, 10)),
        "p50": float(np.percentile(luma, 50)),
        "p90": float(np.percentile(luma, 90)),
        "sky": float(sky.mean()),
        "near": float(near.mean()),
        "sat": float(sat.mean()) * 100.0,
        "contrast": float(d.mean()),
    }


def row(label, s):
    print(
        f"  {label:<22} p10 {s['p10']:6.1f}  p50 {s['p50']:6.1f}  p90 {s['p90']:6.1f}"
        f"  sky {s['sky']:6.1f}  near {s['near']:6.1f}  sat {s['sat']:5.1f}%"
        f"  contrast {s['contrast']:5.2f}"
    )


def median(rows):
    return {k: float(np.median([r[k] for r in rows])) for k in rows[0]}


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    shots = sorted(glob.glob(os.path.join(sys.argv[1], "*.png")))
    if not shots:
        print(f"native_bar: no PNGs in {sys.argv[1]}", file=sys.stderr)
        sys.exit(1)

    print("ours:")
    ours = []
    for p in shots:
        s = stats(p)
        ours.append(s)
        row(os.path.basename(p), s)
    row("MEDIAN", median(ours))

    print("reference (Rust Images/):")
    refs = []
    for f in REFERENCE_FRAMES:
        p = os.path.join(ROOT, "Rust Images", f)
        if not os.path.exists(p):
            print(f"native_bar: missing reference {f}", file=sys.stderr)
            sys.exit(1)
        s = stats(p)
        refs.append(s)
        row(f, s)
    row("MEDIAN", median(refs))


if __name__ == "__main__":
    main()
