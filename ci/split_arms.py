#!/usr/bin/env python3
"""Split a skinned character into an ARMS half and a BODY half, by bone weight.

**Why this exists, and the claim it corrects.** A first-person viewmodel needs
the arms and nothing else. The obvious way to get that is to hide the rest --
and on this character it does not work: the body is one mesh with one
material, so `Visibility` (which is per entity) has no limb to reach, and the
only lever that does reach one, collapsing a joint to zero scale, inherits down
the hierarchy, so hiding the torso hides the arms hanging off it. That was
reported here as "not reachable on this asset". It IS reachable; what was
missing was not a trick but **something to hide**.

So this makes one: the mesh becomes two nodes, `<name>_arms` and
`<name>_body`, both skinned to the same skeleton. Bevy spawns a named entity
per node, so either half is then one `Visibility` away. The third-person body
draws both and is unchanged.

**The vertex data is SHARED, not duplicated.** Both halves reference the same
POSITION / NORMAL / TEXCOORD / JOINTS / WEIGHTS accessors and differ only in
their index buffer, so the split costs one extra index array -- a few percent
of the file -- rather than a second copy of the mesh. It also means the two
halves cannot drift apart under a re-import.

**Triangles are assigned by MAJORITY, never per vertex.** A vertex is "arm" if
more than half its skin weight sits on an arm bone; a triangle goes to the arms
if at least two of its three vertices are. Per-vertex assignment tears the
boundary: a triangle spanning the shoulder belongs to neither half cleanly, so
both halves end up with a hole. Majority puts every boundary triangle in
exactly one half, so the seam is a clean edge -- findable from outside if you
look, invisible from a first-person camera, which is the only place the split
is used.

Usage:  ci/split_arms.py <in.glb> <out.glb> [--bones A,B,...] [--threshold 0.5]
"""
import json
import struct
import sys
from pathlib import Path

import numpy as np

# The chain a first-person view shows. Shoulders included: without them the
# arms start at the bicep and there is a hole where the deltoid was.
ARM_BONES = [
    "LeftShoulder", "LeftArm", "LeftForeArm", "LeftHand",
    "RightShoulder", "RightArm", "RightForeArm", "RightHand",
]

DTYPE = {5126: "<f4", 5125: "<u4", 5123: "<u2", 5121: "u1", 5120: "i1", 5122: "<i2"}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def read_glb(path):
    raw = Path(path).read_bytes()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = json.loads(next(c for k, c in chunks if k == 0x4E4F534A))
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return js, bytearray(blob)


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    Path(path).write_bytes(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def read_accessor(gltf, blob, i):
    a = gltf["accessors"][i]
    n, comp = a["count"], NCOMP[a["type"]]
    dt = np.dtype(DTYPE[a["componentType"]])
    bv = gltf["bufferViews"][a["bufferView"]]
    start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride")
    if stride and stride != comp * dt.itemsize:
        out = np.empty((n, comp), dtype=dt)
        for k in range(n):
            off = start + k * stride
            out[k] = np.frombuffer(blob[off:off + comp * dt.itemsize], dtype=dt)
        return out
    end = start + n * comp * dt.itemsize
    return np.frombuffer(bytes(blob[start:end]), dtype=dt).reshape(n, comp)


def main():
    argv = sys.argv[1:]
    files = [a for a in argv if not a.startswith("--")]
    bones = list(ARM_BONES)
    threshold = 0.5
    for i, a in enumerate(argv):
        if a == "--bones":
            bones = [b.strip() for b in argv[i + 1].split(",")]
            files = [f for f in files if f != argv[i + 1]]
        if a == "--threshold":
            threshold = float(argv[i + 1])
            files = [f for f in files if f != argv[i + 1]]
    if len(files) != 2:
        raise SystemExit(__doc__)
    src, dst = files

    gltf, blob = read_glb(src)
    if len(gltf.get("meshes", [])) != 1 or len(gltf["meshes"][0]["primitives"]) != 1:
        raise SystemExit(f"{src}: expected exactly one mesh with one primitive")
    if not gltf.get("skins"):
        raise SystemExit(f"{src}: not skinned — there is nothing to split by")

    mesh = gltf["meshes"][0]
    prim = mesh["primitives"][0]
    attrs = prim["attributes"]
    for need in ("JOINTS_0", "WEIGHTS_0"):
        if need not in attrs:
            raise SystemExit(f"{src}: the primitive has no {need}")

    skin = gltf["skins"][0]
    names = [gltf["nodes"][j].get("name") for j in skin["joints"]]
    missing = [b for b in bones if b not in names]
    if missing:
        raise SystemExit(
            f"{src}: the skin has no bone(s) {missing} — a split that silently "
            f"matched nothing would produce an empty arms half. Bones are: {names}")
    arm_joint_ix = {i for i, n in enumerate(names) if n in bones}

    joints = read_accessor(gltf, blob, attrs["JOINTS_0"]).astype(np.int64)
    weights = read_accessor(gltf, blob, attrs["WEIGHTS_0"]).astype(np.float64)
    # Normalised weights are not guaranteed by the spec, only recommended.
    total = weights.sum(axis=1, keepdims=True)
    weights = np.divide(weights, total, out=np.zeros_like(weights), where=total > 0)

    is_arm_joint = np.isin(joints, list(arm_joint_ix))
    arm_weight = (weights * is_arm_joint).sum(axis=1)
    vert_is_arm = arm_weight > threshold

    idx = read_accessor(gltf, blob, prim["indices"]).reshape(-1).astype(np.int64)
    tris = idx.reshape(-1, 3)
    tri_arm_votes = vert_is_arm[tris].sum(axis=1)
    tri_is_arm = tri_arm_votes >= 2

    arms_idx = tris[tri_is_arm].reshape(-1)
    body_idx = tris[~tri_is_arm].reshape(-1)
    if len(arms_idx) == 0 or len(body_idx) == 0:
        raise SystemExit(
            f"{src}: the split put every triangle on one side "
            f"(arms {len(arms_idx)//3}, body {len(body_idx)//3}) — check --bones")

    # u32 throughout: the vertex indices are the ORIGINAL ones, so they can
    # exceed 65535 even when each half is small.
    def push_indices(arr):
        nonlocal blob
        data = np.ascontiguousarray(arr, dtype="<u4").tobytes()
        while len(blob) % 4:
            blob += b"\0"
        gltf["bufferViews"].append(
            {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data),
             "target": 34963})
        blob += data
        gltf["accessors"].append({
            "bufferView": len(gltf["bufferViews"]) - 1,
            "componentType": 5125,
            "count": int(len(arr)),
            "type": "SCALAR",
            "min": [int(arr.min())],
            "max": [int(arr.max())],
        })
        return len(gltf["accessors"]) - 1

    base = mesh.get("name") or "char"
    arms_prim = dict(prim)
    arms_prim["indices"] = push_indices(arms_idx)
    body_prim = dict(prim)
    body_prim["indices"] = push_indices(body_idx)

    mesh_arms = {"name": f"{base}_arms", "primitives": [arms_prim]}
    mesh_body = {"name": f"{base}_body", "primitives": [body_prim]}
    gltf["meshes"] = [mesh_arms, mesh_body]

    # The node that carried the single mesh becomes two siblings under the same
    # parent, each with the same skin. A skinned mesh node's own transform is
    # ignored by the spec, so copying it is harmless and keeps the two nodes
    # indistinguishable from the original in every way but their name.
    holder = None
    for i, n in enumerate(gltf["nodes"]):
        if n.get("mesh") == 0:
            holder = i
            break
    if holder is None:
        raise SystemExit(f"{src}: no node references the mesh")

    src_node = gltf["nodes"][holder]
    gltf["nodes"][holder] = dict(src_node, name=f"{base}_arms", mesh=0)
    body_node = dict(src_node, name=f"{base}_body", mesh=1)
    gltf["nodes"].append(body_node)
    body_ix = len(gltf["nodes"]) - 1

    parent = None
    for i, n in enumerate(gltf["nodes"]):
        if holder in n.get("children", []):
            parent = i
            break
    if parent is None:
        # A scene root; add the sibling to the same scene.
        for s in gltf["scenes"]:
            if holder in s["nodes"]:
                s["nodes"].append(body_ix)
    else:
        gltf["nodes"][parent]["children"].append(body_ix)

    gltf["buffers"][0]["byteLength"] = len(blob)
    write_glb(dst, gltf, blob)

    was, now = Path(src).stat().st_size, Path(dst).stat().st_size
    print(f"  {Path(dst).name:24s} {was/1e6:5.1f} -> {now/1e6:5.1f} MB")
    print(f"     {base}_arms  {len(arms_idx)//3:6d} tris  "
          f"({100*len(arms_idx)/len(idx):.1f}% of the mesh)")
    print(f"     {base}_body  {len(body_idx)//3:6d} tris")
    print(f"     vertices over the {threshold} arm-weight line: "
          f"{int(vert_is_arm.sum())} of {len(vert_is_arm)}")


if __name__ == "__main__":
    main()
