#!/usr/bin/env python3
"""Normalise a generated GLB into a shippable game asset.

Generated meshes arrive correct in shape and wrong in three ways that are
cheap to fix and expensive to leave:

  1. **Scale is not to be trusted.** Meshy's `auto_size` uses a vision
     estimate of real-world size. It is excellent on a standard object (a
     55-gallon drum came back 0.880 x 0.585 m against a real 0.88 x 0.58)
     and unreliable on anything whose size is ours to decide: measured
     across eleven starting-tier assets it was off 0.34x on a hand rock,
     0.55x on a campfire, 2.5x on a bedroll and 9x on a spear.
     `DECISIONS.md` 2026-08-11 carries the numbers. So scale comes from
     OUR table, never from the file.

  2. **The emissive map is usually wrong and occasionally right.** Meshy
     ships one on nearly every asset with `emissiveFactor = [1,1,1]`.
     Measured: the campfire's genuinely carries ember glow (max 0.24), and
     the wooden spear's peaks at **0.53** -- a stick that glows in the
     dark. So emission is opt-in per asset, never inherited.

  3. **Origin is right and is left alone.** `origin_at: "bottom"` held on
     all twelve assets (min.y == 0.000), so this only re-centres X/Z.

Scale is UNIFORM and fits INSIDE the declared box -- `k = min(W/w, H/h,
D/d)`. Non-uniform scaling to fill the box exactly is what squashes a
correctly-proportioned mesh, which is the defect the `DEPLOY` table was
fixed to stop. A mesh that does not fill its row is a row to re-measure,
not a mesh to stretch.

Vertices are baked rather than carried as a node transform, because Bevy
loads `GltfAssetLabel::Primitive` WITHOUT the node's transform -- a scale
left on the node is silently dropped at load.

Usage:  ci/import_meshy.py <in.glb> <out.glb> W H D [--emissive]
"""
import json
import struct
import sys

import numpy as np


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off = 12
    chunks = []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = next(c for k, c in chunks if k == 0x4E4F534A)
    bin_ = next((c for k, c in chunks if k == 0x004E4942), b"")
    return json.loads(js), bytearray(bin_)


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    open(path, "wb").write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def positions(gltf, blob, acc_i):
    """A writable float32 view of one POSITION accessor."""
    a = gltf["accessors"][acc_i]
    if a["componentType"] != 5126 or a["type"] != "VEC3":
        raise SystemExit("POSITION is not float32 VEC3")
    bv = gltf["bufferViews"][a["bufferView"]]
    start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or 12
    if stride != 12:
        raise SystemExit("interleaved POSITION is not handled")
    n = a["count"]
    return np.frombuffer(memoryview(blob)[start:start + n * 12], dtype="<f4").reshape(n, 3)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    keep_emissive = "--emissive" in sys.argv
    if len(args) != 5:
        raise SystemExit(__doc__)
    src, dst = args[0], args[1]
    target = np.array([float(v) for v in args[2:5]])

    gltf, blob = read_glb(src)
    accs = [p["attributes"]["POSITION"]
            for m in gltf["meshes"] for p in m["primitives"]]

    lo = np.full(3, np.inf)
    hi = np.full(3, -np.inf)
    for i in accs:
        v = positions(gltf, blob, i)
        lo = np.minimum(lo, v.min(0))
        hi = np.maximum(hi, v.max(0))
    size = hi - lo

    k = float(np.min(target / np.maximum(size, 1e-9)))
    # centre X/Z on the origin, keep the feet on y = 0
    pivot = np.array([(lo[0] + hi[0]) / 2, lo[1], (lo[2] + hi[2]) / 2])

    for i in accs:
        v = positions(gltf, blob, i)
        v[:] = (v - pivot) * k
        a = gltf["accessors"][i]
        a["min"] = v.min(0).astype(float).tolist()
        a["max"] = v.max(0).astype(float).tolist()

    for m in gltf.get("materials", []):
        if keep_emissive:
            continue
        m.pop("emissiveTexture", None)
        m["emissiveFactor"] = [0.0, 0.0, 0.0]

    write_glb(dst, gltf, blob)
    out = size * k
    print(f"  {dst.rsplit('/', 1)[-1]:22s} {size[0]:.2f}x{size[1]:.2f}x{size[2]:.2f}"
          f" -> {out[0]:.2f}x{out[1]:.2f}x{out[2]:.2f} m  (x{k:.3f})"
          f"  emissive={'kept' if keep_emissive else 'off'}")


if __name__ == "__main__":
    main()
