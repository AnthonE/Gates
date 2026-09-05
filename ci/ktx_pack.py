#!/usr/bin/env python3
"""Re-encode a GLB's textures as KTX2/UASTC and rewrite it to reference them.

**The problem this solves is VRAM, not disk.** A JPEG in a `.glb` is
decompressed to full RGBA8 on the GPU: a 2048² map is 16.8 MB of video
memory whatever it weighs on disk, and three maps across twelve props is
~600 MB of texture alone. A GPU-compressed texture stays compressed in
VRAM, so it is the only lever that moves that number.

Measured on 2026-08-11 against one prop's three maps, which is why this
script encodes UASTC at 1024 and not one of the alternatives:

    | format      | disk/prop | VRAM/prop | albedo PSNR |
    |-------------|-----------|-----------|-------------|
    | 2K JPEG (was) | 7.9 MB  | ~50 MB    | source, NO mipmaps |
    | 2K ETC1S    | 1.4 MB    | 8.4 MB    | 28.0 dB — visibly lossy |
    | 2K UASTC    | 11.8 MB   | 16.8 MB   | ~lossless, disk GROWS |
    | 1K UASTC    | 3.2 MB    | 4.2 MB    | 46.9 dB — this one |

UASTC over ETC1S because 28 dB is a quality drop you can see, and the
operator's standing instruction is not to nerf the look. 1024 over 2048
because that is what makes UASTC cheaper than the JPEG it replaces rather
than 1.5x dearer. **The mipmaps are not a footnote**: the JPEGs shipped
with none at all, so every prop aliased at distance, and a mipmapped 1K
generally reads better in motion than a 2K that shimmers.

Colour space is per map and is not cosmetic — an albedo is sRGB and a
normal or metal/rough map is linear data. Encoding a normal map as sRGB
bends every vector in it.

⚠ **And this script bent every vector in every normal map it packed, from
2026-08-11 to 2026-09-05, the other way round.** The format was right —
`R8G8B8A8_UNORM` for the data maps — and `ktx create`'s documented default
did the rest: *"If the Input Transfer Function is not linear for formats
that are not one of the `*_SRGB` formats, an implicit conversion to linear
is done."* A PNG with no transfer-function chunk is taken as sRGB, so every
normal and metal/rough map was run through the sRGB→linear curve **as if it
were a photograph**. Measured 2026-09-05 by decoding the shipped KTX2
through the same UASTC transcoder Bevy uses: the normal maps' X and Y
average **0.212** where a tangent-space map centres on 0.5 (0.5 through
that curve is 0.214), only 0.05 % of texels decode to a unit vector, and the
mean decoded normal is bent ~41° in tangent space — on every prop, every
deployable, every held item and both sites. The tangent frame differs per
UV island, so a fixed bend in tangent space is a different bend in world
space on each island, which is the polygon-edged shading patchwork the
operator pointed at on the boulders. The roughness channel went through
the same curve (0.41 stored where the delivery said ~0.67), so every prop
is glossier than it was made. **Two fixes and both are here**: the
transfer function is now ASSIGNED explicitly per map, and every packed map
is decoded back through `ktx extract` and its channel means compared with
the source's — the linearisation reads as 0.21 against 0.50 and fails
loudly, and so does any future curve nobody meant to apply. The assets
already in the tree are re-packed from their raw deliveries with this
script (`NOW.md` §0rk); `crates/client/tests/packed_maps.rs` holds the
decoded means on what ships.

Needs the KTX-Software CLI (`ktx`), which a headless box has no reason to
carry. It is not in apt; fetch the release tarball from GitHub and point
`KTX_BIN`/`LD_LIBRARY_PATH` at it. That is the same class as the three
`-dev` packages `ci/gates.sh` names — install it, do not skip the step.

Usage:  ci/ktx_pack.py <in.glb> <out.glb> [--size 1024]
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

KTX = os.environ.get("KTX_BIN", os.path.expanduser("~/ktx/bin/ktx"))
LIB = os.path.expanduser("~/ktx/lib")

# glTF images that must be treated as colour rather than as data. Everything
# else — normal, metallic-roughness, occlusion — is linear.
SRGB_SLOTS = {"baseColorTexture", "emissiveTexture"}


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = next(c for k, c in chunks if k == 0x4E4F534A)
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return json.loads(js), blob


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = blob + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    open(path, "wb").write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def srgb_images(gltf):
    """Which image indices are colour. Read off the material SLOTS rather than
    guessed from a filename: the same PNG could in principle serve either, and
    the slot is the only place the meaning is written down."""
    out = set()
    for m in gltf.get("materials", []):
        pbr = m.get("pbrMetallicRoughness", {})
        for slot in SRGB_SLOTS:
            t = pbr.get(slot) or m.get(slot)
            if t is not None:
                tex = gltf["textures"][t["index"]]
                src = tex.get("source")
                if src is not None:
                    out.add(src)
    return out


# How far a packed map's decoded channel means may sit from the source's.
# UASTC at 1024 is ~47 dB, a mean error in the fourth decimal; the defect
# this exists to catch moved a mean from 0.50 to 0.21.
ROUNDTRIP_TOL = 0.01


def ktx(args, env):
    return subprocess.run([KTX] + args, check=True, capture_output=True, env=env)


def encode(png_bytes, size, is_srgb, tmp, label=""):
    src = os.path.join(tmp, "in.png")
    dst = os.path.join(tmp, "out.ktx2")
    back = os.path.join(tmp, "back.png")
    im = Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    if im.size != (size, size):
        im = im.resize((size, size), Image.LANCZOS)
    im.save(src)
    fmt = "R8G8B8A8_SRGB" if is_srgb else "R8G8B8A8_UNORM"
    tf = "srgb" if is_srgb else "linear"
    env = dict(os.environ, LD_LIBRARY_PATH=LIB + ":" + os.environ.get("LD_LIBRARY_PATH", ""))
    # `--assign-tf` tells the tool what the pixels ARE, so its implicit
    # conversion (see the header) has nothing to convert. Older releases
    # spell the same option `--assign-oetf`; try the current spelling and
    # fall back on a usage error, never on a silent default.
    create = ["create", "--format", fmt, "--encode", "uastc", "--zstd", "18",
              "--generate-mipmap"]
    try:
        ktx(create + ["--assign-tf", tf, src, dst], env)
    except subprocess.CalledProcessError as e:
        if b"assign-tf" not in e.stderr and b"unrecognized" not in e.stderr.lower():
            raise
        ktx(create + ["--assign-oetf", tf, src, dst], env)
    # Round trip: decode level 0 back and compare channel means with what
    # went in. This is the check that would have caught the linearisation
    # on 2026-08-11 — the packed normal map's X and Y read 0.21 against a
    # source of 0.50 — and it catches any transfer function applied by a
    # default nobody read, in either direction.
    ktx(["extract", "--transcode", "rgba8", "--level", "0", dst, back], env)
    want = np.asarray(im, dtype=np.float64).reshape(-1, 4).mean(0) / 255.0
    got = np.asarray(Image.open(back).convert("RGBA"), dtype=np.float64).reshape(-1, 4).mean(0) / 255.0
    if np.abs(want - got).max() > ROUNDTRIP_TOL:
        raise SystemExit(
            f"{label or 'map'} ({fmt}): packed channel means {np.round(got, 3).tolist()} "
            f"against source {np.round(want, 3).tolist()} — a transfer function was applied "
            f"that the source did not carry. Refusing to ship a bent map.")
    return open(dst, "rb").read()


def main():
    size = 1024
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if "--size" in sys.argv:
        at = sys.argv.index("--size") + 1
        size = int(sys.argv[at])
        # **`--size`'s VALUE is not a positional argument**, and until
        # 2026-08-17 this function counted it as one: passing the flag this
        # docstring documents left three "positionals" and printed the usage
        # text instead of packing anything. Silent in the sense that matters —
        # the flag's only documented value is also its default, so every
        # invocation that hit it either used the default or looked like a
        # typo. Filtered by position rather than by shape, because a size and
        # a path are both just strings.
        args = [a for i, a in enumerate(sys.argv[1:], start=1)
                if not a.startswith("--") and i != at]
    if len(args) != 2:
        raise SystemExit(__doc__)
    src_path, dst_path = args

    gltf, blob = read_glb(src_path)
    if not gltf.get("images"):
        raise SystemExit(f"{src_path}: no images to pack")
    colour = srgb_images(gltf)

    # Encode every image, keyed by the bufferView it currently occupies.
    replacement = {}
    with tempfile.TemporaryDirectory() as tmp:
        for i, img in enumerate(gltf["images"]):
            bv_i = img.get("bufferView")
            if bv_i is None:
                raise SystemExit(f"{src_path}: image {i} is a URI, not embedded")
            bv = gltf["bufferViews"][bv_i]
            st = bv.get("byteOffset", 0)
            replacement[bv_i] = encode(blob[st:st + bv["byteLength"]], size, i in colour, tmp,
                                       label=f"{os.path.basename(src_path)} image {i}")
            img["mimeType"] = "image/ktx2"

    # Rebuild the buffer. Indices are preserved so every accessor stays valid
    # — an accessor's own byteOffset is relative to its view, so only the
    # view's offset and length move.
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

    # **`textures[].source` stays, and `KHR_texture_basisu` is deliberately NOT
    # written.** The extension is the standard way to say this, and Bevy 0.18
    # does not implement it — its own table marks it ❌ (bevyengine#19104) and
    # its validator rejects any unknown entry in `extensionsRequired` outright,
    # so a spec-correct file fails to load at all. Measured: twelve models,
    # twelve `Unsupported extension` errors, white fallback everywhere.
    #
    # What Bevy DOES support is ktx2 as an image format, dispatched off the
    # mime type: `ImageFormat::from_mime_type("image/ktx2")` resolves under the
    # `ktx2` feature, and the glTF loader hands `Source::View`'s mime type
    # straight to it. So the file stays plain-glTF-shaped and only the mime
    # type is unusual. That is technically outside the core spec, which allows
    # `image/jpeg` and `image/png` — the tradeoff is that a third-party glTF
    # tool may refuse these, which is acceptable for an asset that exists to be
    # read by exactly one engine, and is why this comment is here rather than a
    # commit message nobody will find. Revisit when Bevy lands #19104.
    pass

    write_glb(dst_path, gltf, bytes(out))
    was, now = os.path.getsize(src_path), os.path.getsize(dst_path)
    print(f"  {os.path.basename(dst_path):24s} {was/1e6:5.1f} -> {now/1e6:5.1f} MB "
          f"({was/now:.1f}x)  {len(gltf['images'])} maps @ {size}px UASTC")


if __name__ == "__main__":
    main()
