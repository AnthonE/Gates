#!/usr/bin/env python3
"""The tonal bar, measured on both sides by one estimator.

The native client's captures are PNGs on disk, so this reads them directly —
and, critically, reads the reference set the same way in the same run. A bar
computed a different way than the frame it judges is not a bar (`ART.md` §7).

Rec.601 luma, whole frame, plus the sky band (top 25% of rows) and near band
(bottom 35%), which are the bands `ART.md` §3's table is stated in.

    GATES_REFERENCE_DIR=/path/to/reference-frames \\
        python3 ci/native_bar.py <dir-of-pngs>

**The reference frames are not in this repository** (`ART.md` §0, operator
2026-08-11): they are the reference game's screenshots, this repo is public and
MIT, and carrying them here would be redistributing them. They lived in
`Rust Images/` until 2026-08-11. So the directory is named by
`GATES_REFERENCE_DIR` and, when it is unset or incomplete, this tool **says so
loudly and exits nonzero** rather than scoring against nothing — a bar that
silently measures one side is the pass-it-didn't-earn class `CLAUDE.md` warns
about, and `ART.md` §3's table is already the recorded answer if you only want
the number.

Prints one row per frame plus the reference median, so a capture is read next
to the thing it is being compared to rather than against a remembered number.
"""

import sys
import os
import glob
import numpy as np
from PIL import Image

REFERENCE_DIR_ENV = "GATES_REFERENCE_DIR"

# The outdoor-daylight subset, inherited from the deleted ci/reference_bar.mjs.
# The interiors, menus and maps are excluded on purpose: a tonal bar taken over
# a fullscreen inventory screen would be a bar about UI, not about light.
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
    # ART.md §7: the measurement that separates DETAIL from NOISE is
    # direction, not amplitude. Resolve the near ground's high-frequency
    # residual along the local mean colour (a surface lighter here, darker
    # there — real detail, and what the contrast column counts) versus
    # orthogonal to it (the hue changed between neighbouring pixels). The
    # references run 0.077–0.193 chroma per unit luma, median 0.120; above
    # that band the frame is aliasing and no amount of blur is the fix.
    n3 = a[a.shape[0] - near.shape[0] :].astype(np.float32)
    dv = np.diff(n3, axis=1)                      # neighbour delta, per channel
    mean_col = n3[:, :-1, :] + n3[:, 1:, :]
    norm = np.linalg.norm(mean_col, axis=2, keepdims=True)
    unit = mean_col / np.maximum(norm, 1e-6)      # local mean colour direction
    along = np.sum(dv * unit, axis=2)             # lighter/darker: real detail
    ortho = np.linalg.norm(dv - along[..., None] * unit, axis=2)  # hue changed
    along_abs = float(np.mean(np.abs(along)))
    chroma_per_luma = float(np.mean(ortho)) / max(along_abs, 1e-6)

    return {
        "chroma": chroma_per_luma,
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
        f"  contrast {s['contrast']:5.2f}  chroma/luma {s['chroma']:5.3f}"
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

    ref_dir = os.environ.get(REFERENCE_DIR_ENV)
    if not ref_dir:
        print(
            f"native_bar: SKIP — {REFERENCE_DIR_ENV} is unset, so the reference\n"
            f"  side of the bar cannot be measured and only 'ours' is printed above.\n"
            f"  The reference frames are deliberately not in this repo (ART.md §0).\n"
            f"  Point {REFERENCE_DIR_ENV} at your own copy of the eighteen-frame set,\n"
            f"  or read ART.md §3's table, which is the same measurement recorded.",
            file=sys.stderr,
        )
        sys.exit(2)

    print(f"reference ({ref_dir}):")
    refs = []
    for f in REFERENCE_FRAMES:
        p = os.path.join(ref_dir, f)
        if not os.path.exists(p):
            print(
                f"native_bar: missing reference {f} under {ref_dir} — a missing\n"
                f"  frame is a moved bar, not a skip, so this is fatal.",
                file=sys.stderr,
            )
            sys.exit(1)
        s = stats(p)
        refs.append(s)
        row(f, s)
    row("MEDIAN", median(refs))


if __name__ == "__main__":
    main()
