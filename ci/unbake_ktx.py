#!/usr/bin/env python3
"""Decode a GLB's KTX2 textures back to PNG, undoing the packer's bend.

**This exists because the raw deliveries are gone.** `NOW.md` §0rk says to
re-pack the 23 models "from their raw deliveries"; those live in
`assets/models/To Examine/`, which `.gitignore` keeps out of the tree on
purpose (~600 MB of vendor zips), so on any fresh clone the only copy of a
prop's maps is the KTX2 already embedded in its `.glb`. Without this step
the repair is blocked on a directory nobody has.

**What was done to the data maps, and why it is exactly invertible.**
`ci/ktx_pack.py` asked `ktx create` for `R8G8B8A8_UNORM` and never said what
the input PNG was; the tool's documented default takes an untagged PNG as
sRGB and, for a non-`*_SRGB` format, runs the sRGB→linear curve over it. So
every normal and metal/rough map in the tree holds `srgb_to_linear(source)`
where it should hold `source`. The inverse is `linear_to_srgb`, applied to
the decoded texels, and two independent readings say it lands:

    normal X/Y   0.212 -> 0.498   (a tangent-space map centres on 0.500)
    roughness    0.411 -> 0.673   (MANIFEST.md §prop: "~0.67 was delivered")

Neither number was fitted — the first is what the curve does to 0.5 read
backwards, the second matches a figure written down three weeks before this
script existed.

**The check that is not arithmetic.** A tangent-space normal map is a field
of UNIT vectors; that is a property of what the file means, not of how it
was encoded, so it cannot be satisfied by getting the inverse curve subtly
wrong. Bent, 0.05 % of texels decode to unit length. `--verify` reports the
fraction before and after and refuses to write a file that does not clear
`UNIT_MIN`, which is `packed_maps.rs`'s own 90 %.

**What it cannot recover.** The KTX2 is UASTC, which is lossy (~47 dB at
1024), so this returns the delivery as the encoder saw it and not the
delivery. The re-pack that follows adds a second generation. That is the
price of having thrown the sources away, it is far below the 41° bend being
repaired, and `--verify`'s unit-length fraction is the evidence for it.
Colour maps are passed through untouched: they were packed as
`R8G8B8A8_SRGB`, and `ktx create` converts nothing for an `*_SRGB` format,
so the albedo never carried this defect (its patchwork is the generator's
per-island lighting, which `ci/flatten_charts.py` owns).

The output is a PNG-imaged GLB — the shape `ci/flatten_charts.py` and
`ci/ktx_pack.py` both expect as input. The full repair is three commands:

    ci/unbake_ktx.py   in.glb  a.glb --verify
    ci/flatten_charts.py  a.glb b.glb        # albedo only, scatter props
    ci/ktx_pack.py        b.glb out.glb      # refuses a bent map now

Needs the KTX-Software CLI (`ktx`), same as `ci/ktx_pack.py`: not in apt,
fetch the release and point `KTX_BIN`/`LD_LIBRARY_PATH` at it.

Usage:  ci/unbake_ktx.py <in.glb> <out.glb> [--verify] [--self-test]
"""
import io
import json
import os
import struct
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glbcharts as gc  # noqa: E402
from ktx_pack import KTX, LIB, read_glb, srgb_images, write_glb  # noqa: E402

# What fraction of a repaired normal map's texels must decode to a unit
# vector. `crates/client/tests/packed_maps.rs` gates the shipped file at the
# same 90 %, so a file that clears this here clears that there.
UNIT_MIN = 0.90

# How far a decoded normal's mean X/Y may sit from 0.5 after the repair.
# `packed_maps.rs` allows 0.45..0.55; this is the same band stated as a
# radius, checked before the file is written rather than after it ships.
NEUTRAL_TOL = 0.05


def ktx_extract(ktx2_bytes, tmp, tag):
    """One KTX2 image's level 0, as an RGBA array in 0..1."""
    src = os.path.join(tmp, f"{tag}.ktx2")
    dst = os.path.join(tmp, f"{tag}.png")
    open(src, "wb").write(ktx2_bytes)
    env = dict(os.environ, LD_LIBRARY_PATH=LIB + ":" + os.environ.get("LD_LIBRARY_PATH", ""))
    subprocess.run([KTX, "extract", "--transcode", "rgba8", "--level", "0", src, dst],
                   check=True, capture_output=True, env=env)
    return np.asarray(Image.open(dst).convert("RGBA"), dtype=np.float64) / 255.0


def unbend(rgba):
    """Undo `srgb_to_linear` on the three data channels; alpha is not a
    colour and `ktx create` never touched it."""
    out = rgba.copy()
    out[..., :3] = np.clip(gc.linear_to_srgb(rgba[..., :3]), 0.0, 1.0)
    return out


def unit_fraction(rgba):
    """Share of texels whose RGB decodes to a unit-length tangent normal, and
    the mean X/Y. The tolerance is `packed_maps.rs`'s."""
    xyz = rgba[..., :3] * 2.0 - 1.0
    ln = np.linalg.norm(xyz, axis=-1)
    frac = float(((ln > 0.9) & (ln < 1.1)).mean())
    return frac, float(rgba[..., 0].mean()), float(rgba[..., 1].mean())


def to_png(rgba):
    b = (np.clip(rgba, 0.0, 1.0) * 255.0 + 0.5).astype(np.uint8)
    buf = io.BytesIO()
    Image.fromarray(b, "RGBA").save(buf, format="PNG")
    return buf.getvalue()


def normal_image_indices(gltf):
    """Image indices in a `normalTexture` slot — the ones whose repair has a
    physical check. Read off the material, never guessed from a name."""
    out = set()
    for m in gltf.get("materials", []):
        t = m.get("normalTexture")
        if t is not None:
            src = gltf["textures"][t["index"]].get("source")
            if src is not None:
                out.add(src)
    return out


def unbake(src_path, dst_path, verify=False):
    gltf, blob = read_glb(src_path)
    if not gltf.get("images"):
        raise SystemExit(f"{src_path}: no images")
    colour = srgb_images(gltf)
    normals = normal_image_indices(gltf)
    replacement, report = {}, []
    with tempfile.TemporaryDirectory() as tmp:
        for i, img in enumerate(gltf["images"]):
            if img.get("mimeType") != "image/ktx2":
                raise SystemExit(
                    f"{src_path}: image {i} is {img.get('mimeType')}, not image/ktx2 — "
                    "this script un-packs a packed model; nothing to do")
            bv_i = img["bufferView"]
            bv = gltf["bufferViews"][bv_i]
            st = bv.get("byteOffset", 0)
            rgba = ktx_extract(blob[st:st + bv["byteLength"]], tmp, f"img{i}")
            is_colour = i in colour
            fixed = rgba if is_colour else unbend(rgba)
            if i in normals:
                was, wx, wy = unit_fraction(rgba)
                now, nx, ny = unit_fraction(fixed)
                report.append((i, was, now, (wx, wy), (nx, ny)))
                if verify:
                    if now < UNIT_MIN:
                        raise SystemExit(
                            f"{src_path} image {i}: {now * 100:.1f}% of texels decode to a unit "
                            f"vector after the repair, under {UNIT_MIN * 100:.0f}% — the map is "
                            "not a tangent-space normal map and the inverse curve is not the "
                            "whole story. Refusing to write.")
                    off = max(abs(nx - 0.5), abs(ny - 0.5))
                    if off > NEUTRAL_TOL:
                        raise SystemExit(
                            f"{src_path} image {i}: repaired X/Y mean ({nx:.3f}, {ny:.3f}) is "
                            f"{off:.3f} from neutral, over {NEUTRAL_TOL}. Refusing to write.")
            replacement[bv_i] = to_png(fixed)
            img["mimeType"] = "image/png"

    out = bytearray()
    for i, bv in enumerate(gltf["bufferViews"]):
        data = replacement.get(i)
        if data is None:
            st = bv.get("byteOffset", 0)
            data = blob[st:st + bv["byteLength"]]
        out += b"\0" * (-len(out) % 4)
        bv["byteOffset"] = len(out)
        bv["byteLength"] = len(data)
        out += data
    gltf["buffers"][0]["byteLength"] = len(out)
    write_glb(dst_path, gltf, bytes(out))

    name = os.path.basename(dst_path)
    for i, was, now, (wx, wy), (nx, ny) in report:
        print(f"  {name:24s} normal img{i}: unit {was * 100:5.2f}% -> {now * 100:5.2f}%   "
              f"X/Y ({wx:.3f},{wy:.3f}) -> ({nx:.3f},{ny:.3f})")
    if not report:
        print(f"  {name:24s} no normal map — {len(gltf['images'])} maps decoded")
    return report


def self_test():
    """The curve is its own check: bend a synthetic unit-normal field the way
    the packer did, un-bend it, and require the unit fraction back.

    8-bit quantisation is the only loss, so this runs on the numbers rather
    than on a file and needs no `ktx`."""
    try:
        import PIL  # noqa: F401
    except ImportError:
        print("SKIP: PIL is not installed, the unbake cases did not run")
        raise SystemExit(2)
    # A tangent-space field, not an arbitrary one: X and Y are small
    # perturbations and Z is what makes the vector unit. A field of uniformly
    # random directions is NOT this test's fixture and quietly passes the bend
    # — measured, 33 % of it still reads unit length, and 15 % at scale 0.22 —
    # because a component already near 1.0 is barely moved by the sRGB curve.
    # The defect lives where real normal maps live, around (0, 0, 1).
    #
    # The scale is MEASURED rather than picked: the repaired `rock_a.glb`
    # normal map has an X/Y standard deviation of 0.0204 in byte space, i.e.
    # 0.041 in the -1..1 the vector is decoded into. A fixture looser than its
    # subject is a fixture that tests something else.
    rng = np.random.default_rng(7)
    xy = np.clip(rng.normal(scale=0.041, size=(256, 256, 2)), -0.9, 0.9)
    z = np.sqrt(np.maximum(1.0 - (xy ** 2).sum(-1, keepdims=True), 1e-6))
    v = np.concatenate([xy, z], axis=-1)
    v /= np.linalg.norm(v, axis=-1, keepdims=True)
    src = np.concatenate([v * 0.5 + 0.5, np.ones((256, 256, 1))], axis=-1)
    src = (src * 255 + 0.5).astype(np.uint8) / 255.0

    bent = src.copy()
    bent[..., :3] = gc.srgb_to_linear(src[..., :3])
    bent = (bent * 255 + 0.5).astype(np.uint8) / 255.0
    f_bent, bx, _ = unit_fraction(bent)
    assert f_bent < 0.05, f"the synthetic bend did not break unit length ({f_bent:.3f})"
    assert abs(bx - 0.212) < 0.02, f"bent X mean {bx:.3f}, expected ~0.212"

    back = unbend(bent)
    f_back, rx, ry = unit_fraction(back)
    assert f_back > UNIT_MIN, f"repaired unit fraction {f_back:.3f} under {UNIT_MIN}"
    assert abs(rx - 0.5) < NEUTRAL_TOL and abs(ry - 0.5) < NEUTRAL_TOL, \
        f"repaired X/Y ({rx:.3f},{ry:.3f}) not neutral"
    worst = float(np.abs(back[..., :3] - src[..., :3]).max())

    # The roughness reading from MANIFEST.md §prop, as a case rather than a
    # sentence: what ships stores 0.411 and the delivery said ~0.67.
    rough = float(gc.linear_to_srgb(np.array([0.411]))[0])
    assert abs(rough - 0.673) < 0.005, f"roughness recovers to {rough:.3f}, expected 0.673"

    print(f"unbake self-test: bend {f_bent * 100:.2f}% unit -> repair {f_back * 100:.2f}% unit, "
          f"worst texel error {worst:.4f}, roughness 0.411 -> {rough:.3f}  OK")


def main():
    argv = sys.argv[1:]
    if "--self-test" in argv:
        self_test()
        return
    verify = "--verify" in argv
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 2:
        raise SystemExit(__doc__)
    unbake(args[0], args[1], verify)


if __name__ == "__main__":
    main()
