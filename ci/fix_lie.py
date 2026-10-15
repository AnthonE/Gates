#!/usr/bin/env python3
"""Lift the hind feet of a fel beast's `Lie` clip out of the ground.

    python3 ci/fix_lie.py assets/models/mob/fel/wolf_a.glb [more.glb]

`ci/rig_beast.py`'s lie folds the hind legs under a trunk it has already
lowered, and on the wolves (and the tier-C pig) the planar IK cannot fold
them short enough: the hock and paw end up to 27 cm under the ground. This
re-poses each hind leg, frame by frame, wherever it goes under: the shin
turns about the knee to set the hock on the ground (on the side of the knee
it was already on), then the foot turns about the hock just far enough to
set the ball of the foot on it. The rotations are written back into the GLB
in place (same bytes, same layout; the KTX2 maps are untouched). Idempotent:
a leg already clear of the ground is left alone. Run it after any re-rig;
`client/tests/beasts.rs` holds every clip of every model to the ground.
"""
import json
import struct
import sys

import numpy as np

TOE_Y = 0.03
HOCK_Y = 0.05
LEGS = [("Shin.L", "Foot.LH", "Toe.LH"), ("Shin.R", "Foot.RH", "Toe.RH")]


def quat_mat(q):
    x, y, z, w = q
    return np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])


def mat_quat(m):
    t = np.trace(m)
    if t > 0:
        s = 2 * np.sqrt(1 + t)
        w, x, y, z = s / 4, (m[2, 1] - m[1, 2]) / s, (m[0, 2] - m[2, 0]) / s, (m[1, 0] - m[0, 1]) / s
    else:
        i = int(np.argmax(np.diag(m)))
        if i == 0:
            s = 2 * np.sqrt(1 + m[0, 0] - m[1, 1] - m[2, 2])
            w, x, y, z = (m[2, 1] - m[1, 2]) / s, s / 4, (m[0, 1] + m[1, 0]) / s, (m[0, 2] + m[2, 0]) / s
        elif i == 1:
            s = 2 * np.sqrt(1 + m[1, 1] - m[0, 0] - m[2, 2])
            w, x, y, z = (m[0, 2] - m[2, 0]) / s, (m[0, 1] + m[1, 0]) / s, s / 4, (m[1, 2] + m[2, 1]) / s
        else:
            s = 2 * np.sqrt(1 + m[2, 2] - m[0, 0] - m[1, 1])
            w, x, y, z = (m[1, 0] - m[0, 1]) / s, (m[0, 2] + m[2, 0]) / s, (m[1, 2] + m[2, 1]) / s, s / 4
    q = np.array([x, y, z, w])
    return q / np.linalg.norm(q)


def rot_x(a):
    c, s = np.cos(a), np.sin(a)
    return np.array([[1, 0, 0], [0, c, -s], [0, s, c]])


def angle_x(u, v):
    """The turn about +X that carries u's (y, z) direction onto v's."""
    return np.arctan2(u[1] * v[2] - u[2] * v[1], u[1] * v[1] + u[2] * v[2])


def fix(path):
    raw = bytearray(open(path, "rb").read())
    jlen = struct.unpack_from("<I", raw, 12)[0]
    j = json.loads(raw[20:20 + jlen])
    bin0 = 20 + jlen + 8
    nodes = j["nodes"]
    by_name = {n.get("name"): i for i, n in enumerate(nodes)}
    parent = {c: i for i, n in enumerate(nodes) for c in n.get("children", [])}

    def view(acc):
        a = j["accessors"][acc]
        bv = j["bufferViews"][a["bufferView"]]
        assert a["componentType"] == 5126 and "byteStride" not in bv
        comp = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[a["type"]]
        off = bin0 + bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        return off, comp, a["count"]

    def read(acc):
        off, comp, n = view(acc)
        return np.array(struct.unpack_from("<%df" % (comp * n), raw, off)).reshape(n, comp)

    anim = next(a for a in j["animations"] if a["name"] == "Lie")
    chans = {}
    for c in anim["channels"]:
        chans[(c["target"]["node"], c["target"]["path"])] = anim["samplers"][c["sampler"]]["output"]
    frames = read(anim["samplers"][0]["input"]).shape[0]
    vals = {k: read(acc) for k, acc in chans.items()}

    def pose(k):
        t = {i: np.array(n.get("translation", [0, 0, 0]), float) for i, n in enumerate(nodes)}
        r = {i: np.array(n.get("rotation", [0, 0, 0, 1]), float) for i, n in enumerate(nodes)}
        for (i, p), v in vals.items():
            if p == "translation":
                t[i] = v[k]
            elif p == "rotation":
                r[i] = v[k]
        return t, r

    def world(i, t, r):
        m = np.eye(4)
        m[:3, :3] = quat_mat(r[i])
        m[:3, 3] = t[i]
        while i in parent:
            i = parent[i]
            p = np.eye(4)
            p[:3, :3] = quat_mat(r[i])
            p[:3, 3] = t[i]
            m = p @ m
        return m

    worst = 0.0
    for k in range(frames):
        for shin_n, foot_n, toe_n in LEGS:
            shin, foot, toe = by_name[shin_n], by_name[foot_n], by_name[toe_n]
            t, r = pose(k)
            ws, wf, wt = world(shin, t, r), world(foot, t, r), world(toe, t, r)
            K, A, B = ws[:3, 3], wf[:3, 3], wt[:3, 3]
            if B[1] >= TOE_Y - 1e-4 and A[1] >= HOCK_Y - 1e-4:
                continue
            worst = min(worst, B[1], A[1])
            # The hock down onto the ground on the side of the knee it was
            # already on (behind it: the sphinx a dog lies in), the shin
            # turning about the knee to put it there…
            d1 = 0.0
            if A[1] < HOCK_Y:
                l1 = np.linalg.norm((A - K)[1:])
                dy = K[1] - HOCK_Y
                dz = np.sqrt(max(l1 * l1 - dy * dy, 0.0)) * (1.0 if A[2] >= K[2] else -1.0)
                d1 = angle_x(A - K, np.array([0.0, -dy, dz]))
            A2 = K + rot_x(d1) @ (A - K)
            B2 = A2 + rot_x(d1) @ (B - A)
            # …then the foot turning about the hock just far enough to put
            # the ball of the foot on the ground, the nearer of the two ways.
            d2 = 0.0
            if B2[1] < TOE_Y:
                v = (B2 - A2)[1:]
                l2 = np.linalg.norm(v)
                y = np.clip(TOE_Y - A2[1], -l2, l2)
                z = np.sqrt(max(l2 * l2 - y * y, 0.0))
                d2 = min(
                    (angle_x(np.array([0.0, *v]), np.array([0.0, y, zz])) for zz in (z, -z)),
                    key=abs,
                )
            d2 += d1
            # Shin: turn its world rotation about X at the knee, then put it
            # back in its parent's frame. Foot: likewise, under the new shin.
            ps = world(parent[shin], t, r)[:3, :3]
            ws2 = rot_x(d1) @ ws[:3, :3]
            wf2 = rot_x(d2) @ wf[:3, :3]
            for bone, local in ((shin, ps.T @ ws2), (foot, ws2.T @ wf2)):
                q = mat_quat(local)
                if np.dot(q, vals[(bone, "rotation")][k]) < 0:
                    q = -q
                vals[(bone, "rotation")][k] = q
    for (i, p), acc in chans.items():
        if p == "rotation" and nodes[i].get("name") in {l[0] for l in LEGS} | {l[1] for l in LEGS}:
            off, comp, n = view(acc)
            struct.pack_into("<%df" % (comp * n), raw, off, *vals[(i, p)].reshape(-1))
    open(path, "wb").write(raw)
    print(f"{path}: re-posed, lowest hind joint was {worst:+.3f} m" if worst < 0 else f"{path}: clear")


for p in sys.argv[1:]:
    fix(p)
