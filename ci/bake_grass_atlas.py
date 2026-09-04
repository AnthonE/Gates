#!/usr/bin/env python3
"""Bake the grass card atlas from the Poly Haven `grass_medium_01` scan.

**Why a bake script rather than a committed crop.** The source is a 2048²
photoscan atlas holding ~500 separate sprites — loose blades, seed heads,
debris, and five dense TUFTS along the bottom. Only the tufts are useful as a
billboard card: a card wants a clump that reads root-to-tip, not one blade.
Recording the extraction as code makes the choice auditable and re-runnable
against a re-fetched source, which a committed PNG alone would not be.

Source: Poly Haven `grass_medium_01`, CC0 (https://polyhaven.com/license).
CC0 owes no NOTICE; `assets/textures/MANIFEST.md` carries the provenance row
because that is what makes the licence rail auditable after the fact.

The source lives under `assets/textures/candidates/`, which is GITIGNORED
(1.5 GB, `CANDIDATES.md`). Re-fetch it with:

    python assets/textures/fetch_gates_texture_candidates.py --sets 9.7 --tier A

Run:  python ci/bake_grass_atlas.py
Out:  assets/textures/grass_card_albedo.png   (RGBA, alpha is the cutout)
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

Image.MAX_IMAGE_PIXELS = None
ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "assets/textures/candidates/9_7/97-PH-GRASS-MED01/07_grass_medium_01_diff_2k.png"
OUT = ROOT / "assets/textures/grass_card_albedo.png"

# The five dense tufts along the bottom of the source, as (x, y, w, h) found by
# connected-component analysis of the alpha. Four are baked; the fifth is a
# near-duplicate of the first and is left out so the atlas holds four distinct
# silhouettes rather than three and a repeat.
# Read off a coordinate-gridded render of the source's bottom strip, not from
# connected-component boxes: the component boxes were loose enough to swallow a
# fallen leaf, a black diagonal stick and a solid olive blob that sit BETWEEN
# the tufts, and tight enough at the bottom to slice the sprite. Each box below
# stops short of the source's 2048 edge, because the lower three sprites run
# into it and the clipped blade ends read as vertical stubs under the roots.
TUFTS = [(405, 1525, 530, 282),    # the big wide clump
         (1180, 1565, 372, 232),   # right-hand clump
         (487, 1826, 462, 196),    # bottom-middle
         (1187, 1826, 436, 196)]   # bottom-right

# 2x2 of 512x256, so the atlas is 1024x512 — both sides powers of two, which
# `render::mipmap::wants` requires before it will build a chain, and a chain is
# not optional here (`render::mipmap::Filter::Mask`: a box-filtered alpha mask
# goes bald at range).
#
# 2:1 cells because that is what the sprites measure — 1.86, 1.60, 2.36, 2.22 —
# and a square cell would leave the top half of every card empty, which on an
# alpha-tested quad is pure overdraw. The content is bottom-anchored so the
# roots sit on the card's baseline and the quad plants directly on the ground.
CELL_W, CELL_H = 512, 256
COLS = ROWS = 2
# Alpha at or above this is "covered" — the same test the runtime's
# `AlphaMode::Mask(0.5)` applies, so what is measured here is what is drawn.
CUT = 128


def clean(crop: Image.Image) -> Image.Image:
    """Drop everything not part of the tuft.

    The crops carry strays the bbox could not exclude — a fallen leaf, a
    diagonal stick, a scrap of the sprite next door. Keeping only the largest
    component would be wrong: a tuft is *many* disconnected blades, and that
    would keep one blade. So the rule is proximity to the main mass — a
    component whose centroid falls inside the largest component's box, grown
    by `PAD`, is part of this tuft; anything further out is not.
    """
    PAD = 0.06
    a = np.array(crop.getchannel("A"))
    lab, n = ndimage.label(a > CUT, structure=np.ones((3, 3)))
    if n == 0:
        return crop
    sizes = ndimage.sum(np.ones_like(lab), lab, range(1, n + 1))
    main = int(np.argmax(sizes)) + 1
    ys, xs = ndimage.find_objects(lab)[main - 1]
    h, w = ys.stop - ys.start, xs.stop - xs.start
    x0, x1 = xs.start - w * PAD, xs.stop + w * PAD
    y0, y1 = ys.start - h * PAD, ys.stop + h * PAD
    cents = ndimage.center_of_mass(np.ones_like(lab), lab, range(1, n + 1))
    keep = np.zeros(n + 1, dtype=bool)
    for i, (cy, cx) in enumerate(cents, start=1):
        keep[i] = x0 <= cx <= x1 and y0 <= cy <= y1
    out = np.array(crop)
    out[..., 3] = np.where(keep[lab], out[..., 3], 0)
    return Image.fromarray(out, "RGBA")


def trim(im: Image.Image) -> Image.Image:
    a = np.array(im.getchannel("A")) > CUT
    ys, xs = np.where(a)
    return im.crop((xs.min(), ys.min(), xs.max() + 1, ys.max() + 1))


def main() -> int:
    if not SRC.exists():
        print(f"missing source: {SRC}\nre-fetch it — see this file's docstring.",
              file=sys.stderr)
        return 1
    src = Image.open(SRC).convert("RGBA")
    atlas = Image.new("RGBA", (CELL_W * COLS, CELL_H * ROWS), (0, 0, 0, 0))
    for i, (x, y, w, h) in enumerate(TUFTS):
        tuft = trim(clean(src.crop((x, y, x + w, y + h))))
        # Fit inside the cell with a 2% margin so no blade touches the edge —
        # a card whose content runs to the border shows a hard cut where the
        # neighbouring cell's mip bleeds into it.
        m = int(CELL_H * 0.03)
        tuft.thumbnail((CELL_W - 2 * m, CELL_H - 2 * m), Image.LANCZOS)
        cx = (i % COLS) * CELL_W + (CELL_W - tuft.width) // 2
        # Bottom-anchored: roots on the cell's baseline.
        cy = (i // COLS) * CELL_H + CELL_H - m - tuft.height
        atlas.paste(tuft, (cx, cy), tuft)
        cov = (np.array(tuft.getchannel("A")) > CUT).mean()
        print(f"  cell {i}: {tuft.width}x{tuft.height} "
              f"aspect {tuft.width / tuft.height:.2f} coverage {cov * 100:.1f}%")
    # Bleed the edge colour into the transparent margin. A cutout's RGB under
    # alpha 0 is undefined in the source and bilinear filtering samples it
    # anyway, so leaving it black draws a dark fringe around every blade —
    # the classic alpha-halo, and it is worst exactly where the blades are
    # thinnest.
    atlas = dilate_rgb(atlas)
    atlas.save(OUT)
    print(f"wrote {OUT.relative_to(ROOT)} {atlas.size} "
          f"({OUT.stat().st_size:,} bytes)")
    return 0


def dilate_rgb(im: Image.Image, rounds: int = 6) -> Image.Image:
    """Push opaque colour outward under the transparent margin."""
    arr = np.array(im).astype(np.float32)
    rgb, a = arr[..., :3], arr[..., 3]
    known = a > 0
    for _ in range(rounds):
        if known.all():
            break
        k = np.ones((3, 3))
        cnt = ndimage.convolve(known.astype(np.float32), k, mode="nearest")
        acc = np.stack([ndimage.convolve(rgb[..., c] * known, k, mode="nearest")
                        for c in range(3)], axis=-1)
        fill = (cnt > 0) & ~known
        with np.errstate(invalid="ignore", divide="ignore"):
            avg = np.where(cnt[..., None] > 0, acc / cnt[..., None], 0)
        rgb[fill] = avg[fill]
        known = known | fill
    out = np.concatenate([rgb, a[..., None]], axis=-1).clip(0, 255).astype(np.uint8)
    return Image.fromarray(out, "RGBA")


if __name__ == "__main__":
    raise SystemExit(main())
