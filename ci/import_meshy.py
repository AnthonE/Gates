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

     ⚠ **X/Z is centred on the BOUNDING BOX, which is not the same thing as
     the axis a hand grips the object by, and on two shipped assets it was
     121 mm and 165 mm away from it.** `ui::hold` spends the grip as one
     number up the model's own +Y, so a hafted tool authored leaning, or a
     bow authored across a diagonal, gets slid into the fist by a point that
     is in mid-air. That is `ci/stand_grip.py`'s job -- run it after this on
     anything a hand closes around, and `crates/client/tests/held_assets.rs`
     fails if nobody did.

Scale is UNIFORM and fits INSIDE the declared box -- `k = min(W/w, H/h,
D/d)`. Non-uniform scaling to fill the box exactly is what squashes a
correctly-proportioned mesh, which is the defect the `DEPLOY` table was
fixed to stop. A mesh that does not fill its row is a row to re-measure,
not a mesh to stretch.

Vertices are baked rather than carried as a node transform, because Bevy
loads `GltfAssetLabel::Primitive` WITHOUT the node's transform -- a scale
left on the node is silently dropped at load.

`--fit-axes` is the second sizing mode and it exists for exactly two rows.
The uniform fit above is right when the target box is a RENDER row -- the
`DEPLOY` table has no sim consumer, so a mesh that does not fill it is "a
row to re-measure, not a mesh to stretch". The two authored SITE structures
are the other case: `terrain::SHELTER_BOXES` and `WAYSTATION_CANOPY_BOXES`
are the sim's own collision volume, `OCCUPANT_R_M`/`OCCUPANT_TOP_M` are
*defined* as their bounds, and `client/tests/greybox.rs` holds the drawn
bound to the published one within a millimetre. So under a uniform fit the
slack is not cosmetic: measured on the first pair it was **1.51 m of blocked
air above the shelter's roof** and **1.26 m of invisible skirt on each
horizontal axis of the canopy** -- a body stopped by nothing it can see,
which is the defect `SLACK_R_M` was closed to 1 mm to stop.

`--fit-axes` therefore scales X and Z by ONE factor and Y by another, so the
mesh fills the blocked volume exactly and the footprint stays square (a
plan-view stretch would shear a square pavilion into a rectangle). The cost
is an aspect correction, which is printed and gated rather than hidden:
`kxz / ky` is how much wider-per-height the drawn object is than the
generator made it. Above ~1.3 the fix is a better reference image, not a
bigger stretch.

**Normals are inverse-transpose, tangents are not**, and getting that
backwards is a lighting bug that no bounding-box check can see: a direction
that must stay perpendicular to a stretched surface scales by 1/k per axis,
while a tangent lies IN the surface and scales by k like a position.

`--center` is the third mode and it is about the ORIGIN rather than the
scale. Every asset before it stood on its base, because a deployable and a
building do. The scatter props do not: `props::archetype_lift` places a
boulder's origin 0.55 m above the slot's ground against a half-extent of
0.99, so 0.44 m of it is UNDER the surface -- `ART.md` rule 2, "nothing sits
ON the ground, everything sits IN it", which is also what hides the seam
where a hull meets a heightfield. Those meshes are centred on their origin
and `OCCUPANT_TOP_M` is documented as "lift + half-extent", so pinning their
feet to y = 0 would lift every rock on the island out of the ground and
break the gate in one move. With `--center` the pivot is the box's midpoint
in Y as well as in X/Z.

`--fit-radius` is the fourth mode, and it exists because fitting a BOX to a
prop whose sim volume is a CYLINDER is wrong by up to sqrt(2). The two
authored sites publish `OCCUPANT_R_M` as a half-DIAGONAL -- their footprint
IS the box, and `SHELTER_CORNER_R_M`'s doc says so. The scatter props do not:
their rows were measured off `blob_mesh`, which is round in plan, so
`OCCUPANT_R_M` is the radius of a roughly circular footprint. Fit such a mesh
by its bounding box and its CORNERS stick out past the cylinder the sim
blocks -- measured on the first ore node at **1.2737 m drawn against 0.9148 m
blocked**, i.e. 36 cm of visible rock a player walks straight through, which
is the worse of the two directions `client/tests/prop_assets.rs` separates.

So this mode solves for the radius the renderer actually measures (the
largest per-vertex `hypot(x, z)`, `render::tree::bounds`) rather than for the
box: one shared X/Z factor, so the plan shape is preserved and only its scale
moves, and Y independently as `--fit-axes` does.

Usage:  ci/import_meshy.py <in.glb> <out.glb> W H D
                           [--emissive] [--fit-axes] [--fit-radius] [--center]
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


def _rebound(gltf, acc_i, v):
    """Refresh an accessor's min/max only if it declared them."""
    a = gltf["accessors"][acc_i]
    if "min" in a:
        a["min"] = v.min(0).astype(float).tolist()
        a["max"] = v.max(0).astype(float).tolist()


def vecs(gltf, blob, acc_i, want):
    """A writable float32 view of one VEC`want` accessor, or None if absent."""
    a = gltf["accessors"][acc_i]
    if a["componentType"] != 5126 or a["type"] != f"VEC{want}":
        raise SystemExit(f"attribute is not float32 VEC{want}")
    bv = gltf["bufferViews"][a["bufferView"]]
    start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or 4 * want
    if stride != 4 * want:
        raise SystemExit("interleaved attributes are not handled")
    n = a["count"]
    return np.frombuffer(
        memoryview(blob)[start:start + n * 4 * want], dtype="<f4"
    ).reshape(n, want)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    keep_emissive = "--emissive" in sys.argv
    fit_axes = "--fit-axes" in sys.argv
    fit_radius = "--fit-radius" in sys.argv
    center = "--center" in sys.argv
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

    ratio = target / np.maximum(size, 1e-9)
    if fit_radius:
        # The pivot has to be known before the radius means anything, because
        # `hypot(x, z)` is measured about the origin the mesh will be drawn
        # at -- so this repeats the centring the transform below applies.
        cx, cz = (lo[0] + hi[0]) / 2, (lo[2] + hi[2]) / 2
        rmax = 0.0
        for i in set(accs):
            v = positions(gltf, blob, i)
            rmax = max(rmax, float(np.hypot(v[:, 0] - cx, v[:, 2] - cz).max()))
        kxz = (target[0] / 2.0) / max(rmax, 1e-9)
        k = np.array([kxz, float(ratio[1]), kxz])
    elif fit_axes:
        # Each axis to its own target, so the drawn BOUNDING BOX equals the
        # blocked volume rather than fitting inside it. (The renderer's radius
        # is a per-vertex max, which is a hair under the box's corner unless a
        # vertex sits on it -- `client/tests/site_assets.rs` measures that gap
        # and allows `SITE_SHORT_M` of it.) Sharing one factor between X
        # and Z was the first draft and it was the wrong instinct: both site
        # targets are square, so a shared factor leaves the shorter source
        # axis short of the box (7.5 cm on the shelter's z, which is a skirt),
        # where independent factors land both on 7.000 and make the FOOTPRINT
        # square rather than merely scaled squarely.
        k = ratio.astype(float)
    else:
        k = np.full(3, float(np.min(ratio)))
    # centre X/Z on the origin; Y is the mode's business (see --center).
    pivot = np.array([
        (lo[0] + hi[0]) / 2,
        (lo[1] + hi[1]) / 2 if center else lo[1],
        (lo[2] + hi[2]) / 2,
    ])
    # Inverse-transpose for normals; for a diagonal scale that is 1/k per
    # axis. Identical to `k` under a uniform fit, which is why this file
    # never needed it until `--fit-axes`.
    kn = 1.0 / k

    # Two primitives may share one accessor; scaling it twice would square
    # the factor and pass every bounding-box check on the second read.
    done = set()
    for m in gltf["meshes"]:
        for p in m["primitives"]:
            at = p["attributes"]
            if at["POSITION"] in done:
                continue
            done.add(at["POSITION"])
            v = positions(gltf, blob, at["POSITION"])
            v[:] = (v - pivot) * k
            a = gltf["accessors"][at["POSITION"]]
            a["min"] = v.min(0).astype(float).tolist()
            a["max"] = v.max(0).astype(float).tolist()
            # **Only under a non-uniform scale.** A uniform one leaves every
            # normal where it was, so touching them would rewrite the whole
            # buffer to re-derive the same directions -- and not bit-for-bit,
            # because a multiply-then-renormalise is not the identity in
            # float. Nothing gates a normal here, so that drift would be
            # silent; the guard makes the uniform path byte-identical to what
            # this script did before `--fit-axes` existed.
            if "NORMAL" in at and fit_axes:
                n = vecs(gltf, blob, at["NORMAL"], 3)
                n *= kn
                n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)
                _rebound(gltf, at["NORMAL"], n)
            if "TANGENT" in at and fit_axes:
                # VEC4: xyz is a direction IN the surface, w is handedness and
                # a scale must not touch it.
                t = vecs(gltf, blob, at["TANGENT"], 4)
                t[:, :3] *= k
                t[:, :3] /= np.maximum(
                    np.linalg.norm(t[:, :3], axis=1, keepdims=True), 1e-12
                )
                _rebound(gltf, at["TANGENT"], t)

    for m in gltf.get("materials", []):
        if keep_emissive:
            continue
        m.pop("emissiveTexture", None)
        m["emissiveFactor"] = [0.0, 0.0, 0.0]

    write_glb(dst, gltf, blob)
    out = size * k
    # The aspect correction: how much the most-stretched axis was stretched
    # relative to the least. 1.000 is a uniform fit by luck; above ~1.3 the
    # answer is a better reference image, not a bigger stretch.
    how = (f"x{k[0]:.3f} y{k[1]:.3f} z{k[2]:.3f} aspect {k.max() / k.min():.3f}x"
           if (fit_axes or fit_radius) else f"x{k[0]:.3f}")
    if fit_radius:
        how += " (radial)"
    how += "  origin=" + ("center" if center else "base")
    print(f"  {dst.rsplit('/', 1)[-1]:22s} {size[0]:.2f}x{size[1]:.2f}x{size[2]:.2f}"
          f" -> {out[0]:.2f}x{out[1]:.2f}x{out[2]:.2f} m  ({how})"
          f"  emissive={'kept' if keep_emissive else 'off'}")


if __name__ == "__main__":
    main()
