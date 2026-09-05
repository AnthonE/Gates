#!/usr/bin/env python3
"""Equalise the UV charts of a generated GLB's albedo, so the seams between
them stop reading as fractures.

**The defect, measured 2026-09-05 off the shipped KTX2** (decoded through
the same UASTC transcoder Bevy uses): the boulder pool's albedos are 116 to
149 UV islands each, and the islands disagree with each other by a
texel-weighted mean of **13.9 % / 14.7 % / 8.9 %** of the map's luma
(`glbcharts.chart_contrast`), the stone node's 384 islands by **30.9 %**.
The generator paints each island from the camera it projected it from, so
two islands that meet on the mesh were lit differently, and the seam is a
step in albedo that follows the polygon edge exactly. On a smooth-normalled
boulder it is the only hard edge in the frame -- the "camouflage" patches
the operator pointed at -- and on the node it reads as separate blocks.

**What this does.** Per chart and per channel, the mean linear value is
scaled to the map's global mean: a multiplicative gain per island,
`(global / chart) ** strength`, applied in linear light. Per CHANNEL and
not per luma, because the bake's islands disagree in white balance as well
as in level -- on the shipped boulder the per-chart R/B ratio varies by
12 %, warm islands beside cool ones, where the node's varies 1.3 %.
`--luma-only` keeps hue as delivered. The in-chart grain is untouched
either way: only the island's average moves, which is exactly the quantity
the bake got wrong. The gain is then DILATED into the generator's
gutter texels (the padding outside every island that a mip level samples
across), because a gutter left at the old level puts the seam back at
distance.

The gain is clamped to `GAIN_CLAMP`: a chart two times off the mean is a
painted crevice or a bake no rescale saves, and the residual it leaves is
reported rather than hidden.

**What it is not.** It is not the fix for the second half of the patchwork,
which is the packer having LINEARISED every normal map (`ci/ktx_pack.py`,
same day); a chart-flat albedo under a bent normal still shows the chart.
And a chart that is genuinely a different material -- lichen on the north
face -- is flattened with the rest: `strength` under 1.0 keeps a share of
the natural variation, at the price of the same share of the defect.

Runs on the RAW delivery (PNG/JPEG textures), before `ci/ktx_pack.py`, and
only touches the base colour map. `--self-test` proves the effect on a
synthetic patchwork and that a flat map passes through unchanged.

Usage:  ci/flatten_charts.py <in.glb> <out.glb> [--strength 1.0] [--gutter 16] [--luma-only]
        ci/flatten_charts.py --self-test
"""
import io
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glbcharts as gc  # noqa: E402

# How many texels outward the per-chart gain is carried into uncovered
# gutter. Meshy pads ~8-16; a mip 4 levels down samples 16 texels across.
GUTTER_DEFAULT = 16
# A chart more than this far from the mean is not a lighting error but a
# feature (a crevice painted near black) or a bake too broken to rescale:
# unclamped, the stone node's darkest chart wanted x14.85 and blew out.
GAIN_CLAMP = (0.5, 2.0)


def albedo_image_index(g):
    ti = g["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"]
    return g["textures"][ti]["source"]


def dilate_gain(gain, covered, passes):
    """Carry each covered texel's gain outward `passes` texels: an uncovered
    texel takes the mean gain of its covered 8-neighbours, then counts as
    covered for the next pass."""
    gain = gain.copy()
    cov = covered.copy()
    for _ in range(passes):
        acc = np.zeros_like(gain)
        cnt = np.zeros_like(gain)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                if dx == 0 and dy == 0:
                    continue
                sh = np.roll(np.roll(gain * cov, dy, axis=0), dx, axis=1)
                sc = np.roll(np.roll(cov.astype(gain.dtype), dy, axis=0), dx, axis=1)
                acc += sh
                cnt += sc
        grow = (~cov) & (cnt > 0)
        gain[grow] = acc[grow] / cnt[grow]
        cov = cov | grow
        if not grow.any():
            break
    return gain


def flatten(g, blob, strength=1.0, gutter=GUTTER_DEFAULT, luma_only=False):
    """Returns `(png_bytes, before, after, gain_map)` for the albedo; the
    gain map is `(h, w, 3)`, one gain per channel."""
    from PIL import Image
    ii = albedo_image_index(g)
    bv = g["bufferViews"][g["images"][ii]["bufferView"]]
    st = bv.get("byteOffset", 0)
    im = Image.open(io.BytesIO(blob[st:st + bv["byteLength"]])).convert("RGBA")
    a = np.asarray(im, dtype=np.float64) / 255.0
    lin = gc.srgb_to_linear(a[..., :3])
    lum = gc.luma(lin)
    h, w = lum.shape

    pos, uv, idx = gc.primitive(g, blob)
    tri_chart, n = gc.charts(idx, len(pos))
    chart_map = gc.rasterize(uv, idx, tri_chart, w, h)
    before, _, _ = gc.chart_contrast(chart_map, lum, n)
    covered = chart_map != gc.UNCOVERED
    channels = [lum] if luma_only else [lin[..., 0], lin[..., 1], lin[..., 2]]
    gains = []
    for value in channels:
        means, counts = gc.chart_means(chart_map, value, n)
        cc = counts > 0
        global_mean = float(np.sum(means[cc] * counts[cc]) / np.sum(counts[cc]))
        chart_gain = np.ones(n)
        chart_gain[cc] = (global_mean / np.maximum(means[cc], 1e-6)) ** strength
        chart_gain = np.clip(chart_gain, *GAIN_CLAMP)
        gmap = np.ones((h, w))
        gmap[covered] = chart_gain[chart_map[covered]]
        gains.append(dilate_gain(gmap, covered, gutter))
    gain = np.stack(gains * 3 if luma_only else gains, axis=-1)

    out_lin = np.clip(lin * gain, 0.0, 1.0)
    after, _, _ = gc.chart_contrast(chart_map, gc.luma(out_lin), n)
    rgb8 = (np.clip(gc.linear_to_srgb(out_lin), 0, 1) * 255 + 0.5).astype(np.uint8)
    rgba8 = np.concatenate([rgb8, (a[..., 3:4] * 255 + 0.5).astype(np.uint8)], axis=2)
    buf = io.BytesIO()
    Image.fromarray(rgba8, "RGBA").save(buf, format="PNG")
    return buf.getvalue(), before, after, gain


def rewrite(g, blob, image_index, png):
    """The GLB with one image's bytes replaced; every other view is carried
    across at a new offset, the way `ci/ktx_pack.py` rebuilds its buffer."""
    target = g["images"][image_index]["bufferView"]
    out = bytearray()
    for i, bv in enumerate(g["bufferViews"]):
        st = bv.get("byteOffset", 0)
        data = png if i == target else blob[st:st + bv["byteLength"]]
        out += b"\0" * (-len(out) % 4)
        bv["byteOffset"] = len(out)
        bv["byteLength"] = len(data)
        out += data
    g["buffers"][0]["byteLength"] = len(out)
    g["images"][image_index]["mimeType"] = "image/png"
    return gc.glb_bytes(g, bytes(out))


def self_test():
    try:
        import PIL  # noqa: F401
    except ImportError:
        print("SKIP: PIL is not installed, nothing ran")
        sys.exit(2)
    patchy = gc.synthetic_glb("sphere", gains=[0.75, 1.3] * 4)
    g, blob = gc.parse_glb(patchy)
    png, before, after, gain = flatten(g, blob)
    assert before > 0.2, f"fixture is not patchy: {before}"
    assert after < 0.02, f"flatten left contrast at {after}"
    # The global level is preserved: equalising to the mean is not a
    # brightness change.
    g2, blob2 = gc.parse_glb(rewrite(g, blob, albedo_image_index(g), png))
    from PIL import Image
    ii = albedo_image_index(g2)
    bv = g2["bufferViews"][g2["images"][ii]["bufferView"]]
    st = bv.get("byteOffset", 0)
    a = np.asarray(Image.open(io.BytesIO(blob2[st:st + bv["byteLength"]])).convert("RGB"), dtype=np.float64) / 255
    bv0 = g["bufferViews"][g["images"][ii]["bufferView"]]
    st0 = bv0.get("byteOffset", 0)
    a0 = np.asarray(Image.open(io.BytesIO(blob[st0:st0 + bv0["byteLength"]])).convert("RGB"), dtype=np.float64) / 255
    m0, m1 = gc.luma(gc.srgb_to_linear(a0)).mean(), gc.luma(gc.srgb_to_linear(a)).mean()
    assert abs(m1 / m0 - 1) < 0.02, f"global luma moved {m0:.4f} -> {m1:.4f}"
    # The gutter carries the gain: a texel just outside a chart whose gain is
    # not 1 must not be left at 1, or the seam comes back at distance.
    pos, uv, idx = gc.primitive(g, blob)
    tri_chart, n = gc.charts(idx, len(pos))
    cm = gc.rasterize(uv, idx, tri_chart, a.shape[1], a.shape[0])
    covered = cm != gc.UNCOVERED
    ring = np.zeros_like(covered)
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            ring |= np.roll(np.roll(covered, dy, 0), dx, 1)
    ring &= ~covered
    assert (np.abs(gain[ring][:, 1] - 1.0) > 0.05).mean() > 0.9, "gutter texels were not dilated"
    # A flat map is left alone: no gain past a texel's own rounding.
    flat = gc.synthetic_glb("sphere", gains=[1.0] * 8)
    gf, bf = gc.parse_glb(flat)
    _, b2, a2, gain2 = flatten(gf, bf)
    assert b2 < 0.01 and abs(gain2 - 1).max() < 0.02, f"flat map was changed: {b2} {a2} {abs(gain2 - 1).max()}"
    # Half strength halves the correction, in log terms.
    _, _, a_half, _ = flatten(g, blob, strength=0.5)
    assert 0.3 * before < a_half < 0.7 * before, f"strength 0.5 gave {a_half} against {before}"
    print(f"self-test ok: patchy {before:.3f} -> {after:.3f}, half strength -> {a_half:.3f}, flat map untouched")


def main():
    argv = sys.argv[1:]
    if "--self-test" in argv:
        self_test()
        return
    strength, gutter, luma_only = 1.0, GUTTER_DEFAULT, False
    args = []
    it = iter(argv)
    for tok in it:
        if tok == "--strength":
            strength = float(next(it))
        elif tok == "--gutter":
            gutter = int(next(it))
        elif tok == "--luma-only":
            luma_only = True
        else:
            args.append(tok)
    if len(args) != 2:
        raise SystemExit(__doc__)
    src, dst = args
    g, blob = gc.read_glb(src)
    png, before, after, _ = flatten(g, blob, strength, gutter, luma_only)
    open(dst, "wb").write(rewrite(g, blob, albedo_image_index(g), png))
    print(f"  {os.path.basename(dst):24s} chart contrast {before:.3f} -> {after:.3f}"
          f"  (strength {strength}, gutter {gutter})")


if __name__ == "__main__":
    main()
