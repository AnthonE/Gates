#!/usr/bin/env python3
"""Give the rig's hands finger bones: three per digit, bound to today's curl.

    git show 44acd3139b7d4d401617c234290d0c2dc91a81c6:assets/models/stumpy.glb > /tmp/orig.glb
    ci/rig_fingers.py /tmp/orig.glb assets/models/stumpy.glb assets/models/stumpy.glb

**Why.** `RightHand` and `LeftHand` were leaves, so a hand's shape was the
mesh's shape and nothing at runtime could close it round a handle
(`ci/curl_hands.py` bent the vertices into a relaxed grip because that was the
only lever). A bat held in that half-curl reads as a hand resting on it.

**Bound to the curl, not to the flat import.** Each digit gets three bones
placed exactly where `curl_hands.py`'s bend put that point of the digit, with
the hinge on the bend's own axis. So with every finger bone at its rest
rotation the skinned hand is the shipped hand to the vertex, nothing that
reads the mesh moves, and closing a fist is a positive turn about each bone's
local +X on top of it (`render::fingers`).

**The derivation is `curl_hands.py`'s, imported, not restated**
(`plan_digits`, `bend`): the digits, their bases, which one is the thumb and
which way the palm faces all come off the ORIGINAL import, which is why this
takes it as an input. It refuses unless bending that original reproduces the
shipped hands, so the bones cannot sit on a curl the file does not have.

Writes: 30 nodes (`RightHandIndex1..3` and so on), 30 skin joints with their
inverse bind matrices, and the digit vertices' JOINTS_0/WEIGHTS_0. Positions,
normals, clips and everything else are untouched.
"""
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from curl_hands import bend, digits, plan_digits, rotate  # noqa: E402
from split_arms import read_accessor, read_glb, write_glb  # noqa: E402

HANDS = ["RightHand", "LeftHand"]
# The shipped curl (`assets/models/MANIFEST.md`'s recipe). The bones are only
# right if these are the numbers the vertices were bent with, and the check
# below refuses the run if they are not.
OPTS = {"--degrees": 150.0, "--thumb-degrees": 90.0, "--adduct": 0.85}
# Where along each digit its three bones start, as a fraction of its length
# from the base: knuckle, middle joint, last joint. A finger's phalanges run
# roughly 45/30/25 of its length; the thumb's island starts lower on the
# hand, so its joints sit a little earlier.
FINGER_AT = (0.0, 0.45, 0.75)
THUMB_AT = (0.0, 0.40, 0.70)
# Half-width of the blend across each joint, as a fraction of the digit. Zero
# would crease every knuckle; much more and the segments stop bending as one.
BLEND = 0.06
FINGERS = ["Index", "Middle", "Ring", "Pinky"]


def frame_at(d, u):
    """The bent digit's frame at fraction `u` of its length, in bone space.

    Columns: +X the hinge (`curl_hands`' bend axis), +Y down the digit, +Z
    toward the palm. A positive turn about +X closes the digit.
    """
    L = d["length"]
    a = np.radians(d["deg"]) * u
    k = np.radians(d["deg"]) / L
    pos = d["base"] + (np.sin(a) / k) * d["dirv"] + ((1.0 - np.cos(a)) / k) * d["n"]
    t = rotate(d["dirv"][None, :], d["axis"], np.array([a]))[0]
    h = d["axis"].copy()
    if d["swing"] is not None:
        sw = np.array([d["swing"]])
        pos = d["base"] + rotate((pos - d["base"])[None, :], d["palm_n"], sw)[0]
        t = rotate(t[None, :], d["palm_n"], sw)[0]
        h = rotate(h[None, :], d["palm_n"], sw)[0]
    h /= np.linalg.norm(h)
    t -= h * (t @ h)
    t /= np.linalg.norm(t)
    m = np.eye(4)
    m[:3, 0], m[:3, 1], m[:3, 2], m[:3, 3] = h, t, np.cross(h, t), pos
    return m


def quat(r):
    """Rotation matrix to a glTF [x, y, z, w] quaternion."""
    w = np.sqrt(max(0.0, 1.0 + r[0, 0] + r[1, 1] + r[2, 2])) / 2.0
    x = np.sqrt(max(0.0, 1.0 + r[0, 0] - r[1, 1] - r[2, 2])) / 2.0
    y = np.sqrt(max(0.0, 1.0 - r[0, 0] + r[1, 1] - r[2, 2])) / 2.0
    z = np.sqrt(max(0.0, 1.0 - r[0, 0] - r[1, 1] + r[2, 2])) / 2.0
    x = np.copysign(x, r[2, 1] - r[1, 2])
    y = np.copysign(y, r[0, 2] - r[2, 0])
    z = np.copysign(z, r[1, 0] - r[0, 1])
    q = np.array([x, y, z, w])
    return (q / np.linalg.norm(q)).tolist()


def blend(u, at):
    """Weights over (hand, bone 1, bone 2, bone 3) at fraction `u`."""
    w = np.zeros(4)
    # The base ramps in from the hand so the knuckle does not crease where
    # the digit leaves the palm.
    if u < 2 * BLEND:
        t = u / (2 * BLEND)
        w[0], w[1] = 1.0 - t, t
        return w
    # Boundary `b` sits at at[b], between bone b and bone b + 1.
    for b in (1, 2):
        lo, hi = at[b] - BLEND, at[b] + BLEND
        if u < lo:
            w[b] = 1.0
            return w
        if u <= hi:
            t = (u - lo) / (2 * BLEND)
            w[b], w[b + 1] = 1.0 - t, t
            return w
    w[3] = 1.0
    return w


def main():
    if len(sys.argv) != 4:
        raise SystemExit(__doc__)
    orig_path, keep_path, out_path = sys.argv[1:]
    og, ob = read_glb(orig_path)
    gltf, blob = read_glb(keep_path)

    prim = gltf["meshes"][0]["primitives"][0]["attributes"]
    for m in gltf["meshes"]:
        if m["primitives"][0]["attributes"] != prim:
            raise SystemExit("the meshes do not share their vertex attributes")
    oprim = og["meshes"][0]["primitives"][0]
    O = read_accessor(og, ob, oprim["attributes"]["POSITION"]).astype(np.float64)
    K = read_accessor(gltf, blob, prim["POSITION"]).astype(np.float64)
    if O.shape != K.shape:
        raise SystemExit(f"{orig_path} and {keep_path} are not the same mesh")
    tris = read_accessor(og, ob, oprim["indices"]).reshape(-1, 3).astype(np.int64)
    J = read_accessor(gltf, blob, prim["JOINTS_0"]).astype(np.int64).copy()
    W = read_accessor(gltf, blob, prim["WEIGHTS_0"]).astype(np.float64).copy()

    skin = gltf["skins"][0]
    names = [gltf["nodes"][j].get("name") for j in skin["joints"]]
    if any(n.startswith(h + "Index") for n in names for h in HANDS):
        raise SystemExit(f"{keep_path} already has finger bones; run on the unrigged file")
    ibm_acc = gltf["accessors"][skin["inverseBindMatrices"]]
    IBM = (read_accessor(gltf, blob, skin["inverseBindMatrices"]).astype(np.float64)
           .reshape(-1, 4, 4).transpose(0, 2, 1))
    oskin = og["skins"][0]
    oIBM = (read_accessor(og, ob, oskin["inverseBindMatrices"]).astype(np.float64)
            .reshape(-1, 4, 4).transpose(0, 2, 1))

    new_nodes, new_ibm = [], []
    for hand in HANDS:
        hj = names.index(hand)
        hand_node = skin["joints"][hj]
        if not np.allclose(IBM[hj], oIBM[[n.get("name") for n in
                                         (og["nodes"][j] for j in oskin["joints"])].index(hand)]):
            raise SystemExit(f"{hand}'s bind moved between the two files")
        M = IBM[hj]
        R, T = M[:3, :3], M[:3, 3]
        Rinv = np.linalg.inv(R)
        own_ix = np.nonzero(((J == hj) & (W > 0.5)).any(axis=1))[0]
        local = (O @ R.T) + T
        plan = plan_digits(local, digits(local, own_ix, tris), OPTS)
        if len(plan["digits"]) != 5:
            raise SystemExit(f"{hand}: {len(plan['digits'])} digits, not 5")

        # The bones are only right if this IS the curl the file carries.
        worst = 0.0
        for d in plan["digits"]:
            bent, _ = bend(local, d)
            worst = max(worst, float(np.abs((bent - T) @ Rinv.T - K[d["sel"]]).max()))
        if worst > 1e-4:
            raise SystemExit(
                f"{hand}: bending {orig_path} with {OPTS} lands {worst:.5f} off the "
                f"shipped hand — the curl changed, so re-derive it before rigging")

        thumb = next(d for d in plan["digits"] if d["thumb"])
        rest = sorted((d for d in plan["digits"] if not d["thumb"]),
                      key=lambda d: abs(d["spread"] - thumb["spread"]))
        order = [("Thumb", thumb)] + list(zip(FINGERS, rest))

        hand_children = gltf["nodes"][hand_node].setdefault("children", [])
        for label, d in order:
            at = THUMB_AT if d["thumb"] else FINGER_AT
            frames = [frame_at(d, u) for u in at]
            ids = []
            parent = np.eye(4)
            for b, F in enumerate(frames):
                localm = np.linalg.inv(parent) @ F
                node = {"name": f"{hand}{label}{b + 1}",
                        "translation": localm[:3, 3].tolist(),
                        "rotation": quat(localm[:3, :3])}
                ids.append(len(gltf["nodes"]) + len(new_nodes))
                new_nodes.append(node)
                new_ibm.append(np.linalg.inv(F) @ M)
                parent = F
            hand_children.append(ids[0])
            new_nodes[ids[0] - len(gltf["nodes"])]["children"] = [ids[1]]
            new_nodes[ids[1] - len(gltf["nodes"])]["children"] = [ids[2]]
            joints = [hj] + [len(skin["joints"]) + (i - len(gltf["nodes"])) for i in ids]

            # Re-weight the digit: the hand's share of each vertex is spread
            # over the hand and the three bones by how far up the digit it is.
            u_all = d["s"] / d["length"]
            for v, u in zip(d["sel"], u_all):
                hw = float(W[v][J[v] == hj].sum())
                acc = {}
                for j, w in zip(J[v], W[v]):
                    if j != hj and w > 0.0:
                        acc[int(j)] = acc.get(int(j), 0.0) + float(w)
                for j, f in zip(joints, blend(float(u), at)):
                    if f > 0.0:
                        acc[j] = acc.get(j, 0.0) + hw * float(f)
                top = sorted(acc.items(), key=lambda kv: -kv[1])[:4]
                total = sum(w for _, w in top)
                J[v] = 0
                W[v] = 0.0
                for slot, (j, w) in enumerate(top):
                    J[v][slot], W[v][slot] = j, w / total
        print(f"  {hand}: thumb + {', '.join(FINGERS)} — "
              f"{sum(len(d['sel']) for d in plan['digits'])} vertices re-weighted, "
              f"curl matches the shipped hand to {worst:.1e}")

    base_nodes = len(gltf["nodes"])
    gltf["nodes"].extend(new_nodes)
    skin["joints"].extend(range(base_nodes, base_nodes + len(new_nodes)))
    if max(skin["joints"]) > 255 or len(skin["joints"]) > 255:
        raise SystemExit("JOINTS_0 is u8 and the skin outgrew it")

    def overwrite(ix, data, dtype):
        acc = gltf["accessors"][ix]
        bv = gltf["bufferViews"][acc["bufferView"]]
        if bv.get("byteStride"):
            raise SystemExit("an interleaved attribute; this tool writes packed ones")
        start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
        raw = np.ascontiguousarray(data, dtype=dtype).tobytes()
        blob[start:start + len(raw)] = raw

    if gltf["accessors"][prim["JOINTS_0"]]["componentType"] != 5121:
        raise SystemExit("JOINTS_0 is not u8")
    overwrite(prim["JOINTS_0"], J, "<u1")
    overwrite(prim["WEIGHTS_0"], W, "<f4")

    # The inverse bind matrices grow, so they move to a new view at the end.
    all_ibm = np.concatenate([IBM, np.array(new_ibm)]).transpose(0, 2, 1)
    raw = np.ascontiguousarray(all_ibm, dtype="<f4").tobytes()
    blob.extend(b"\0" * (-len(blob) % 4))
    gltf["bufferViews"].append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(raw)})
    blob.extend(raw)
    ibm_acc["bufferView"] = len(gltf["bufferViews"]) - 1
    ibm_acc.pop("byteOffset", None)
    ibm_acc["count"] = len(skin["joints"])
    gltf["buffers"][0]["byteLength"] = len(blob)
    write_glb(out_path, gltf, blob)
    print(f"  {Path(out_path).name}: {len(new_nodes)} finger bones, "
          f"{len(skin['joints'])} joints")


if __name__ == "__main__":
    main()
