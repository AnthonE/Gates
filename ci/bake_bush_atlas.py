#!/usr/bin/env python3
"""Bake the bush card atlas from the Poly Haven `shrub_01` leaf sheet.

**Why this one is composed rather than cropped.** `ci/bake_grass_atlas.py` had
it easy: its source contains whole photographed TUFTS, so a card is a crop.
`shrub_01` is a sheet of individual leaves — there is no bush anywhere in it —
so a bush card has to be assembled, and the assembly is the thing worth
recording. Each cell places a dozen leaves at hashed angles, scales and offsets
inside a rough disc, back-to-front so the near ones overlap the far ones.

The alternative was a smooth 320-triangle blob, which is what ships today and
what `props.rs` already calls out: *"the bush wants a ragged outline
(`ART.md` rule 6) and 320 smooth [triangles do not give one]"*. A leaf cluster
is a measured ragged outline.

Source: Poly Haven `shrub_01`, CC0 (https://polyhaven.com/license). The RGB is
the `diff` sheet and the cutout is the separate `alpha` map — the diff PNG's
own alpha channel is fully opaque and is NOT the mask, which is the trap in
this source.

The source lives under `assets/textures/candidates/`, which is GITIGNORED.
Re-fetch it with:

    python assets/textures/fetch_gates_texture_candidates.py --sets 9.10 --tier B

Run:  python ci/bake_bush_atlas.py
Out:  assets/textures/bush_card_albedo.png   (RGBA, alpha is the cutout)
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

Image.MAX_IMAGE_PIXELS = None
ROOT = Path(__file__).resolve().parent.parent
SRC_RGB = ROOT / "assets/textures/candidates/9_10/910-PH-SHRUB01/02_shrub_01_diff_2k.png"
SRC_A = ROOT / "assets/textures/candidates/9_10/910-PH-SHRUB01/03_shrub_01_alpha_2k.png"
OUT = ROOT / "assets/textures/bush_card_albedo.png"

# Square cells: a bush is about as wide as it is tall, unlike a grass tuft.
CELL = 512
COLS = ROWS = 2
CUT = 128
# Leaves per cell.
#
# **The first cut used 14 and it was wrong by an order of magnitude.** The
# sprites are 400-800 px leaves and it drew them at 0.30-0.52 scale into a
# 512 cell, so each leaf covered a third of the card and the result read as
# "four enormous leaves", not as a bush. A bush 1.4 m across carries leaves
# around 6 cm, i.e. ~4% of its width — so the scale is ~0.08 of the sprite and
# it takes a hundred of them to fill the silhouette. Looking at the baked sheet
# is what caught it; no statistic here would have.
LEAVES = 110

# Sprite scale, as a fraction of the source leaf's own size. See `LEAVES`.
LEAF_SCALE = (0.055, 0.125)

# How many of the sheet's sprites to use, largest first. The tail of the
# component list is flower heads, buds and seed discs rather than foliage —
# a few would be fine on a shrub, but at this scale they read as litter stuck
# to the card, and the top rows are unambiguously leaves.
LEAF_KINDS = 7

# Bbox aspects treated as "round", and therefore not a leaf. See `sprites`.
ROUND_LO, ROUND_HI = 0.85, 1.18


def sprites(rgb: Image.Image, alpha: np.ndarray) -> list[Image.Image]:
    """Every leaf in the sheet, cut out and trimmed, largest first.

    The bottom strip is a bark/soil band spanning the full width; it is
    excluded by aspect, because a leaf is never 2048 wide.
    """
    lab, n = ndimage.label(alpha > CUT, structure=np.ones((3, 3)))
    out = []
    for i, sl in enumerate(ndimage.find_objects(lab), start=1):
        if sl is None:
            continue
        ys, xs = sl
        h, w = ys.stop - ys.start, xs.stop - xs.start
        if w < 120 or h < 120 or w > rgb.width * 0.6:
            continue
        # **Round sprites are not leaves.** The sheet carries seed heads and
        # flower discs among the foliage, and at bush scale they read as
        # litter stuck to the card rather than as botany. A leaf is elongated;
        # a seed head is as wide as it is tall, so the bbox aspect separates
        # them without needing to recognise either.
        aspect = w / h
        if ROUND_LO <= aspect <= ROUND_HI:
            continue
        # Mask to THIS component: two leaves whose boxes overlap must not
        # bring each other's pixels along.
        m = (lab[sl] == i).astype(np.uint8) * 255
        a = np.minimum(alpha[sl], m)
        cut = rgb.crop((xs.start, ys.start, xs.stop, ys.stop)).convert("RGBA")
        cut.putalpha(Image.fromarray(a))
        out.append((int((a > CUT).sum()), cut))
    out.sort(key=lambda t: -t[0])
    return [c for _, c in out]


def cell(leaves: list[Image.Image], seed: int) -> Image.Image:
    """One bush: leaves scattered in a disc, back to front."""
    rng = np.random.default_rng(seed)
    img = Image.new("RGBA", (CELL, CELL), (0, 0, 0, 0))
    order = rng.permutation(len(leaves))
    for k in range(LEAVES):
        leaf = leaves[order[k % len(order)]]
        # Radius biased outward (sqrt of uniform is area-uniform), so the
        # silhouette gets its ragged edge and the centre does not go solid.
        r = CELL * 0.46 * float(np.sqrt(rng.uniform(0.02, 1.0)))
        th = float(rng.uniform(0, 2 * np.pi))
        # Squashed vertically: a bush is wider than tall and sits on the
        # ground, so the cluster's centre is below the cell's middle.
        cx = CELL * 0.5 + r * float(np.cos(th))
        # **Centred, and squashed only a little.** The cluster used to sit at
        # 0.58 of the cell and be squashed to 0.78, which put the leaf mass low
        # and left the top quarter of every card empty. A card quad samples the
        # WHOLE cell, so empty cell is empty quad: the blob's smooth crown
        # showed above the leaves, which is the bald-top look this atlas exists
        # to remove. Centred content also means the quad can be square, and a
        # square quad over a square cell is the only ratio that does not
        # stretch the leaves — see `props::BUSH_CARD_HALF`.
        cy = CELL * 0.5 + r * float(np.sin(th)) * 0.88
        scale = float(rng.uniform(*LEAF_SCALE))
        w = max(8, int(leaf.width * scale))
        h = max(8, int(leaf.height * scale))
        s = leaf.resize((w, h), Image.LANCZOS).rotate(
            float(rng.uniform(0, 360)), expand=True, resample=Image.BICUBIC
        )
        img.alpha_composite(s, (int(cx - s.width / 2), int(cy - s.height / 2)))
    return img


def dilate_rgb(im: Image.Image, rounds: int = 6) -> Image.Image:
    """Push opaque colour outward under the transparent margin, or bilinear
    filtering samples undefined RGB and draws a dark fringe on every leaf."""
    arr = np.array(im).astype(np.float32)
    rgb, a = arr[..., :3], arr[..., 3]
    known = a > 0
    k = np.ones((3, 3))
    for _ in range(rounds):
        if known.all():
            break
        cnt = ndimage.convolve(known.astype(np.float32), k, mode="nearest")
        acc = np.stack(
            [ndimage.convolve(rgb[..., c] * known, k, mode="nearest") for c in range(3)],
            axis=-1,
        )
        fill = (cnt > 0) & ~known
        with np.errstate(invalid="ignore", divide="ignore"):
            avg = np.where(cnt[..., None] > 0, acc / cnt[..., None], 0)
        rgb[fill] = avg[fill]
        known = known | fill
    out = np.concatenate([rgb, a[..., None]], axis=-1).clip(0, 255).astype(np.uint8)
    return Image.fromarray(out, "RGBA")


def main() -> int:
    for p in (SRC_RGB, SRC_A):
        if not p.exists():
            print(f"missing source: {p}\nre-fetch it — see this file's docstring.",
                  file=sys.stderr)
            return 1
    rgb = Image.open(SRC_RGB).convert("RGB")
    a16 = Image.open(SRC_A)
    alpha = np.array(a16)
    if alpha.dtype != np.uint8:
        alpha = (alpha / 257.0).astype(np.uint8)
    leaves = sprites(rgb, alpha)[:LEAF_KINDS]
    print(f"  {len(leaves)} leaf sprites")
    if len(leaves) < 4:
        print("too few leaves to compose a bush", file=sys.stderr)
        return 1

    atlas = Image.new("RGBA", (CELL * COLS, CELL * ROWS), (0, 0, 0, 0))
    for i in range(COLS * ROWS):
        # Fixed seeds: the atlas has to be reproducible, or re-running this
        # script produces a different game.
        c = cell(leaves, 0x9E37 + i * 977)
        cov = (np.array(c.getchannel("A")) > CUT).mean()
        print(f"  cell {i}: coverage {cov * 100:.1f}%")
        atlas.alpha_composite(c, ((i % COLS) * CELL, (i // COLS) * CELL))
    atlas = dilate_rgb(atlas)
    atlas.save(OUT)
    print(f"wrote {OUT.relative_to(ROOT)} {atlas.size} ({OUT.stat().st_size:,} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
