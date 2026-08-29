#!/usr/bin/env python3
"""Normalise a generated **rigged character** GLB into a shippable asset.

`ci/import_meshy.py` is the prop importer and it bakes its corrections into
the VERTICES. That is exactly wrong here, and the reason is worth stating
before anything else in this file:

  **A skinned mesh is not drawn where its vertices say it is.** glTF says a
  skinned mesh node's own transform must be ignored, and Bevy obeys it -- the
  shader replaces the model matrix with the blended joint matrix
  `JointWorld_j * IBM_j`. So moving the vertices moves the mesh's bind pose
  out from under its own inverse bind matrices and the character deforms into
  a smear, while every measurement a tool prints stays reassuringly correct.

  The corrections therefore go on the **scene root node**, above both the
  mesh and the joints. At bind pose every `JointWorld_j * IBM_j` is the
  identity, so a rotation or a uniform scale on the root passes straight
  through to the drawn vertices and nothing else has to be touched. It is the
  one place a change is safe, and it is one line of JSON.

What arrives wrong, measured on the deliveries in `assets/models/`:

  1. **The up axis is Z on a merged-animation export and Y on the character
     export of the same model.** Same generator, same rig, two files, two
     answers -- the merged file's mesh runs -1.8..0 along Z and the character
     lies on its back. Corrected with a +90 deg X rotation on the root, which
     carries the skeleton and every clip with it.

  2. **Clip names are the generator's, not the game's.** `render/anim.rs`
     resolves clips BY NAME through `Gltf::named_animations` -- deliberately,
     because `GltfAssetLabel::Animation(i)` is positional and `CLAUDE.md`'s
     trap list is explicit about what positional payloads cost. So a delivery
     that calls its idle `Idle_11` is a delivery whose idle never plays, and
     the rename belongs here rather than in a match arm in the client.

  3. **Emission is on by default and is usually wrong** -- `import_meshy.py`
     measured a wooden spear glowing at 0.53. Stripped unless `--emissive`.

  4. **Everything is `doubleSided`.** For a closed character that is a
     doubled fragment cost for nothing. `--single-sided` turns it off; it is
     opt-in because a mesh with a few inverted normals loses those faces
     instead, and that is a thing to look at rather than assume.

Height is checked and only corrected on request: the deliveries here already
arrive at 1.800 m with their feet on y = 0, which is the sim's own body
(`render/anim.rs::ANIM_BODY_H_M`), and a scale that is already 1.0 should be
left alone rather than re-derived every import.

Textures are NOT touched here. `ci/ktx_pack.py` owns that and already does it
right (UASTC at 1024, mipmaps, sRGB per slot); run it after this.

Usage:
  ci/import_char.py <in.glb> <out.glb>
      [--rename OLD=NEW]...     rename a clip; repeatable
      [--height 1.8]            scale the root so the mesh stands this tall
      [--rot-x auto|<deg>]      root rotation; `auto` (default) stands up a
                                Z-up export and leaves a Y-up one alone
      [--roughness R]           override the material's roughness factor
      [--single-sided]          turn off doubleSided
      [--emissive]              keep the emissive map
"""
import json
import struct
import sys

import numpy as np


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
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    open(path, "wb").write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def mesh_extent(gltf):
    """The bind-pose box of every mesh, read off the POSITION accessors' own
    min/max. This is the space the skin's inverse bind matrices are written
    against, which is the space a root transform acts on -- so it is the box
    that predicts what gets drawn, and the node hierarchy is irrelevant to
    it."""
    lo = np.full(3, np.inf)
    hi = np.full(3, -np.inf)
    for m in gltf.get("meshes", []):
        for p in m["primitives"]:
            a = gltf["accessors"][p["attributes"]["POSITION"]]
            if "min" not in a or "max" not in a:
                raise SystemExit("a POSITION accessor carries no min/max")
            lo = np.minimum(lo, np.array(a["min"], dtype=float))
            hi = np.maximum(hi, np.array(a["max"], dtype=float))
    if not np.isfinite(lo).all():
        raise SystemExit("no meshes")
    return lo, hi


def quat_mul(a, b):
    """glTF quaternion order is (x, y, z, w)."""
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]


def roots(gltf):
    """The scene's root nodes -- where a correction is safe."""
    scene = gltf.get("scene", 0)
    return gltf["scenes"][scene]["nodes"]


def apply_to_roots(gltf, rot_x_deg, scale_k):
    """Compose a rotation and a uniform scale onto every scene root.

    A root carrying a `matrix` rather than TRS is refused rather than
    silently mishandled: decomposing one is a different job and no delivery
    seen here has needed it."""
    half = np.deg2rad(rot_x_deg) / 2.0
    rx = [float(np.sin(half)), 0.0, 0.0, float(np.cos(half))]
    for i in roots(gltf):
        n = gltf["nodes"][i]
        if "matrix" in n:
            raise SystemExit(f"node {i} carries a matrix, not TRS -- unhandled")
        if rot_x_deg:
            n["rotation"] = quat_mul(rx, n.get("rotation", [0.0, 0.0, 0.0, 1.0]))
            # The translation is in the pre-rotation frame, so it has to turn
            # with it or a root that was offset lands somewhere else.
            t = n.get("translation")
            if t:
                c, s = float(np.cos(np.deg2rad(rot_x_deg))), float(np.sin(np.deg2rad(rot_x_deg)))
                n["translation"] = [t[0], c * t[1] - s * t[2], s * t[1] + c * t[2]]
        if scale_k != 1.0:
            s = n.get("scale", [1.0, 1.0, 1.0])
            n["scale"] = [v * scale_k for v in s]
            t = n.get("translation")
            if t:
                n["translation"] = [v * scale_k for v in t]


def main():
    argv = sys.argv[1:]
    files = [a for a in argv if not a.startswith("--")]
    # Strip the values that belong to flags out of the positional list.
    valued = {"--rename", "--height", "--rot-x", "--roughness"}
    skip = set()
    for i, a in enumerate(argv):
        if a in valued and i + 1 < len(argv):
            skip.add(argv[i + 1])
    files = [f for f in files if f not in skip]
    if len(files) != 2:
        raise SystemExit(__doc__)
    src, dst = files

    renames = {}
    for i, a in enumerate(argv):
        if a == "--rename" and i + 1 < len(argv):
            old, _, new = argv[i + 1].partition("=")
            if not old or not new:
                raise SystemExit("--rename wants OLD=NEW")
            renames[old] = new
    height = None
    if "--height" in argv:
        height = float(argv[argv.index("--height") + 1])
    rot_x = "auto"
    if "--rot-x" in argv:
        rot_x = argv[argv.index("--rot-x") + 1]
    roughness = None
    if "--roughness" in argv:
        roughness = float(argv[argv.index("--roughness") + 1])
    single_sided = "--single-sided" in argv
    keep_emissive = "--emissive" in argv

    gltf, blob = read_glb(src)
    lo, hi = mesh_extent(gltf)
    size = hi - lo

    # --- up axis -------------------------------------------------------
    if rot_x == "auto":
        # A Z-up export is deeper than it is tall by a wide margin. The test
        # is deliberately coarse: a character is 3-4x taller than it is deep,
        # so 1.5x either way is nowhere near a real body's proportions and no
        # delivery can land in the gap by accident.
        deg = 90.0 if size[2] > size[1] * 1.5 else 0.0
    else:
        deg = float(rot_x)

    # --- height --------------------------------------------------------
    # After the rotation, "tall" is whichever axis ends up on Y.
    tall = size[1] if deg == 0.0 else size[2]
    k = 1.0
    if height is not None:
        k = height / tall
    apply_to_roots(gltf, deg, k)

    # --- clips ---------------------------------------------------------
    have = [a.get("name") for a in gltf.get("animations", [])]
    missing = [o for o in renames if o not in have]
    if missing:
        raise SystemExit(
            f"--rename names {missing} but the file has {have} -- a rename that "
            "silently does nothing is how a clip stops playing with every gate green")
    for a in gltf.get("animations", []):
        if a.get("name") in renames:
            a["name"] = renames[a["name"]]

    # --- material ------------------------------------------------------
    for m in gltf.get("materials", []):
        if not keep_emissive:
            m.pop("emissiveTexture", None)
            m["emissiveFactor"] = [0.0, 0.0, 0.0]
        if single_sided:
            m["doubleSided"] = False
        if roughness is not None:
            m.setdefault("pbrMetallicRoughness", {})["roughnessFactor"] = roughness

    write_glb(dst, gltf, blob)

    out = size * k
    if deg:
        out = np.array([out[0], out[2], out[1]])
    print(f"  {dst.rsplit('/', 1)[-1]:24s} "
          f"{size[0]:.2f}x{size[1]:.2f}x{size[2]:.2f} -> "
          f"{out[0]:.2f}x{out[1]:.2f}x{out[2]:.2f} m"
          f"  rot-x={deg:.0f} scale=x{k:.4f}")
    for a in gltf.get("animations", []):
        print(f"     clip  {a.get('name')}")


if __name__ == "__main__":
    main()
