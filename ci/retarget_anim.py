#!/usr/bin/env python3
"""Move an animation library from one humanoid rig onto another.

**Why this exists.** The commissioned character (`assets/models/stumpy.glb`)
shipped with seven clips on its own 24-joint skeleton. The mannequin it
replaced carries **46** on a 53-joint Rigify skeleton -- death, hit reactions,
a sprint, a jump, a swim, a crouch, a sword swing, every one of them already
paid for and sitting in the tree unused. The bones map one-for-one down to the
fingers the new rig does not have, so this is arithmetic rather than art.

**It is a REST-POSE retarget, and that is the whole difficulty.** The two rigs
do not agree about what "no rotation" means: the mannequin's T-pose arm rest is
nothing like the new rig's, its spine is named in the opposite order
(`Spine02` is the *lowest* on the target and `spine.001` on the source), and
each skeleton is authored Z-up with a different stand-up rotation above it. So
a channel cannot be copied. What transfers is the *deviation from rest*,
expressed in world space:

    D(b)      = Gt_src(b) . G_src(b)^-1        the world delta from rest
    Gt_tgt(b) = D(b) . G_tgt(b)                the same delta, target's rest
    L_tgt(b)  = Gt_tgt(parent)^-1 . Gt_tgt(b)  back to a local channel

Globals are accumulated **from the scene root**, which is what makes the two
stand-up rotations cancel instead of needing a hand-derived correction: both
sides are then in their own rendered world space, both Y-up, and `D` is
directly comparable. Getting that wrong is the classic retarget failure -- it
compiles, it runs, and the character moves like a puppet with a broken neck.

**Rotation only, plus the hips.** Every other bone keeps its target rest
translation and scale: a source bone's translation is that skeleton's LIMB
LENGTH, and copying it would stretch the target onto the source's proportions,
which is the one thing a retarget must never do. The hips are the exception,
because that channel is the body moving through the world rather than a bone
length -- it is converted through the target's own unit scale (this target's
joints are in centimetres under a 0.01 root) and by the hip-height ratio, so a
jump lands at the right height on a body of a different build.

**Samples are re-baked at a fixed rate.** The source's 159 channels each carry
their own key times; rather than align them, every clip is evaluated at a
uniform rate and written as fresh keys. Costs a little size, removes a whole
class of off-by-one-key bugs, and the source is STEP-interpolated baked data
anyway.

Usage:
  ci/retarget_anim.py <source.gltf> <target.glb> <out.glb>
      [--fps 30]          resample rate
      [--suffix _alt]     what a name collision gets; the TARGET's own clip
                          wins the bare name, because it was authored for it
      [--only NAME]       repeatable; retarget just these
      [--skip NAME]       repeatable
      [--no-align]        anchor each bone on its OWN rest instead of on one
                          aligned to the source's. Wrong for these two rigs --
                          it is what put the arms 43 degrees low -- and kept
                          because a pair of rigs that genuinely share a rest
                          pose is better served without the correction.
"""
import json
import struct
import sys
from pathlib import Path

import numpy as np

# --- the bone map -----------------------------------------------------
#
# target bone -> source bone. Read off both hierarchies, and two entries are
# the ones a reader will want to check:
#
#   * The spine is REVERSED. The target chains Hips -> Spine02 -> Spine01 ->
#     Spine -> {shoulders, neck}, so `Spine02` is the lowest vertebra and maps
#     to the source's `spine.001`, not to `spine.003`. A retarget that trusts
#     the numbers rather than the hierarchy bends the character backwards from
#     the waist and looks almost right.
#   * The target has NO fingers and no `root` bone. The source's 30 finger
#     bones and its armature root are simply not addressed; the root's motion
#     is not lost, because it accumulates into every source global and comes
#     out in the hips.
BONES = {
    "Hips": "DEF-hips",
    "Spine02": "DEF-spine.001",
    "Spine01": "DEF-spine.002",
    "Spine": "DEF-spine.003",
    "neck": "DEF-neck",
    "Head": "DEF-head",
    "LeftShoulder": "DEF-shoulder.L",
    "LeftArm": "DEF-upper_arm.L",
    "LeftForeArm": "DEF-forearm.L",
    "LeftHand": "DEF-hand.L",
    "RightShoulder": "DEF-shoulder.R",
    "RightArm": "DEF-upper_arm.R",
    "RightForeArm": "DEF-forearm.R",
    "RightHand": "DEF-hand.R",
    "LeftUpLeg": "DEF-thigh.L",
    "LeftLeg": "DEF-shin.L",
    "LeftFoot": "DEF-foot.L",
    "LeftToeBase": "DEF-toe.L",
    "RightUpLeg": "DEF-thigh.R",
    "RightLeg": "DEF-shin.R",
    "RightFoot": "DEF-foot.R",
    "RightToeBase": "DEF-toe.R",
}
# The bone the body's world motion rides on.
ROOT_BONE = "Hips"

# Which child defines a bone's DIRECTION, where the hierarchy gives more than
# one candidate. Everything else is derived: a mapped bone with exactly one
# mapped child points at it, and a mapped bone with none is a leaf that
# inherits its parent's correction.
DIRECTION = {"Hips": "Spine02", "Spine": "neck"}


# --- quaternions, (x, y, z, w) like glTF ------------------------------
def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return np.array([
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ])


def qinv(q):
    """Conjugate. Every quaternion here is a unit rotation, so this is the
    inverse — and normalising defensively costs nothing against a file whose
    keys drifted."""
    q = q / np.linalg.norm(q)
    return np.array([-q[0], -q[1], -q[2], q[3]])


def qrot(q, v):
    t = 2.0 * np.cross(q[:3], v)
    return v + q[3] * t + np.cross(q[:3], t)


def qbetween(u, v):
    """The minimal rotation taking unit vector `u` onto unit vector `v`.

    Minimal — i.e. no twist about the resulting axis — because a single pair of
    directions cannot determine roll, and inventing one is how a retargeted
    forearm ends up corkscrewed."""
    d = float(np.dot(u, v))
    if d > 1.0 - 1e-9:
        return np.array([0.0, 0.0, 0.0, 1.0])
    if d < -1.0 + 1e-9:
        axis = np.cross(u, np.array([1.0, 0.0, 0.0]))
        if np.linalg.norm(axis) < 1e-6:
            axis = np.cross(u, np.array([0.0, 1.0, 0.0]))
        axis /= np.linalg.norm(axis)
        return np.array([axis[0], axis[1], axis[2], 0.0])
    w = np.cross(u, v)
    s = np.sqrt((1.0 + d) * 2.0)
    q = np.array([w[0] / s, w[1] / s, w[2] / s, s * 0.5])
    return q / np.linalg.norm(q)


def qslerp(a, b, t):
    d = float(np.dot(a, b))
    if d < 0.0:
        b, d = -b, -d
    if d > 0.9995:
        out = a + t * (b - a)
        return out / np.linalg.norm(out)
    th0 = np.arccos(d)
    th = th0 * t
    s0 = np.sin(th0 - th) / np.sin(th0)
    s1 = np.sin(th) / np.sin(th0)
    return s0 * a + s1 * b


# --- glTF io ----------------------------------------------------------
def read_any(path):
    """A `.glb` or a `.gltf` with a sibling `.bin`."""
    raw = Path(path).read_bytes()
    if raw[:4] == b"glTF":
        off, chunks = 12, []
        while off < len(raw):
            ln, kind = struct.unpack("<II", raw[off:off + 8])
            chunks.append((kind, raw[off + 8:off + 8 + ln]))
            off += 8 + ln
        js = json.loads(next(c for k, c in chunks if k == 0x4E4F534A))
        blob = next((c for k, c in chunks if k == 0x004E4942), b"")
        return js, bytearray(blob)
    js = json.loads(raw)
    buf = js["buffers"][0]
    uri = buf.get("uri")
    if uri is None:
        raise SystemExit(f"{path}: .gltf with no buffer uri")
    if uri.startswith("data:"):
        import base64
        blob = base64.b64decode(uri.split(",", 1)[1])
    else:
        blob = (Path(path).parent / uri).read_bytes()
    return js, bytearray(blob)


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    Path(path).write_bytes(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


DTYPE = {5126: "<f4", 5125: "<u4", 5123: "<u2", 5121: "u1", 5120: "i1", 5122: "<i2"}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


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


class Rig:
    """A scene graph, its rest pose, and its world-space rest rotations."""

    def __init__(self, gltf, blob):
        self.gltf, self.blob = gltf, blob
        self.nodes = gltf["nodes"]
        self.parent = {}
        for i, n in enumerate(self.nodes):
            for c in n.get("children", []):
                self.parent[c] = i
        self.roots = gltf["scenes"][gltf.get("scene", 0)]["nodes"]
        self.by_name = {}
        for i, n in enumerate(self.nodes):
            if "name" in n:
                self.by_name.setdefault(n["name"], i)
        self.rest_r = {}
        self.rest_t = {}
        self.rest_s = {}
        for i, n in enumerate(self.nodes):
            if "matrix" in n:
                raise SystemExit(f"node {i} carries a matrix, not TRS — unhandled")
            self.rest_r[i] = np.array(n.get("rotation", [0.0, 0.0, 0.0, 1.0]), dtype=float)
            self.rest_t[i] = np.array(n.get("translation", [0.0, 0.0, 0.0]), dtype=float)
            self.rest_s[i] = np.array(n.get("scale", [1.0, 1.0, 1.0]), dtype=float)
        # Rest globals, accumulated from the SCENE ROOT — which is what makes
        # two differently-stood-up skeletons comparable without a correction
        # term anybody has to derive by hand.
        self.grest_r = {}
        self.grest_p = {}
        self.gscale = {}
        for r in self.roots:
            self._walk(r, np.array([0.0, 0.0, 0.0, 1.0]), np.zeros(3), 1.0)

    def _walk(self, i, pq, pp, ps):
        q = qmul(pq, self.rest_r[i])
        p = pp + qrot(pq, self.rest_t[i] * ps)
        s = ps * float(self.rest_s[i][0])
        self.grest_r[i], self.grest_p[i], self.gscale[i] = q, p, s
        for c in self.nodes[i].get("children", []):
            self._walk(c, q, p, s)

    def order(self):
        """Nodes, parents before children."""
        out = []
        stack = list(reversed(self.roots))
        while stack:
            i = stack.pop()
            out.append(i)
            stack.extend(reversed(self.nodes[i].get("children", [])))
        return out


class Sampled:
    """One node's animated channels for one clip, evaluable at any time."""

    def __init__(self):
        self.rot = None
        self.tr = None

    @staticmethod
    def _eval(times, values, interp, t, slerp):
        if t <= times[0]:
            return values[0]
        if t >= times[-1]:
            return values[-1]
        k = int(np.searchsorted(times, t) - 1)
        if interp == "STEP":
            return values[k]
        span = times[k + 1] - times[k]
        u = 0.0 if span <= 0 else (t - times[k]) / span
        if slerp:
            return qslerp(values[k], values[k + 1], u)
        return values[k] * (1 - u) + values[k + 1] * u

    def rotation(self, t, rest):
        if self.rot is None:
            return rest
        times, values, interp = self.rot
        # CUBICSPLINE stores in/value/out triples; the middle one is the value,
        # and sampling it as a plain key is a small quality loss rather than a
        # wrong pose. Nothing in this library uses it.
        if interp == "CUBICSPLINE":
            values = values[1::3]
            interp = "LINEAR"
        return self._eval(times, values, interp, t, True)

    def translation(self, t, rest):
        if self.tr is None:
            return rest
        times, values, interp = self.tr
        if interp == "CUBICSPLINE":
            values = values[1::3]
            interp = "LINEAR"
        return self._eval(times, values, interp, t, False)


def channels_of(gltf, blob, anim):
    """Per-node rotation/translation channels for one animation."""
    out = {}
    for ch in anim["channels"]:
        path = ch["target"]["path"]
        node = ch["target"].get("node")
        if node is None or path not in ("rotation", "translation"):
            continue
        s = anim["samplers"][ch["sampler"]]
        times = read_accessor(gltf, blob, s["input"]).astype(float).reshape(-1)
        vals = read_accessor(gltf, blob, s["output"]).astype(float)
        interp = s.get("interpolation", "LINEAR")
        entry = out.setdefault(node, Sampled())
        if path == "rotation":
            entry.rot = (times, vals, interp)
        else:
            entry.tr = (times, vals, interp)
    return out


def duration(gltf, anim):
    d = 0.0
    for s in anim["samplers"]:
        a = gltf["accessors"][s["input"]]
        if a.get("max"):
            d = max(d, float(a["max"][0]))
    return d


def main():
    argv = sys.argv[1:]
    files = []
    fps = 30.0
    suffix = "_alt"
    only, skip = [], []
    no_align = False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--fps":
            fps = float(argv[i + 1]); i += 2
        elif a == "--suffix":
            suffix = argv[i + 1]; i += 2
        elif a == "--only":
            only.append(argv[i + 1]); i += 2
        elif a == "--skip":
            skip.append(argv[i + 1]); i += 2
        elif a == "--no-align":
            no_align = True; i += 1
        elif a.startswith("--"):
            raise SystemExit(f"unknown flag {a}\n\n{__doc__}")
        else:
            files.append(a); i += 1
    if len(files) != 3:
        raise SystemExit(__doc__)
    src_path, tgt_path, out_path = files

    sg, sb = read_any(src_path)
    tg, tb = read_any(tgt_path)
    src, tgt = Rig(sg, sb), Rig(tg, tb)

    # Resolve the map once, loudly. A name that is not in the file is the one
    # failure that would otherwise show up as a limb that never moves.
    pairs = []
    for t_name, s_name in BONES.items():
        if t_name not in tgt.by_name:
            raise SystemExit(f"target has no bone {t_name!r}")
        if s_name not in src.by_name:
            raise SystemExit(f"source has no bone {s_name!r}")
        pairs.append((tgt.by_name[t_name], src.by_name[s_name]))
    tmap = dict(pairs)

    # --- rest alignment ------------------------------------------------
    #
    # **The two rigs do not rest in the same POSE, and transferring a delta
    # silently assumes they do.** Measured between these two: the source rests
    # in a true T — its upper arm points [1, 0, 0], dead horizontal — and the
    # target rests in an A, its arm 43 degrees below that. Spine 2-7 degrees,
    # legs 3-17, arms **36-43**. So every arm motion arrived 43 degrees low and
    # a hand-to-chest pose put the hand inside the torso. Reported from the
    # bench as "the right arm is inside the model", and it is not proportions:
    # it is this.
    #
    # The fix anchors each bone on a VIRTUAL rest whose bone direction matches
    # the source's, rather than on its own: `A(b)` is the minimal rotation
    # taking the target's rest direction onto the source's, and the anchor
    # becomes `A(b) . G_tgt(b)`. At the source's rest the target then strikes
    # the source's pose rather than its own, which is what "the same pose"
    # has to mean for a delta to be transferable at all.
    #
    # Per-bone and independent, which the global formulation allows for free —
    # each bone's world orientation is built from its own anchor and only then
    # localised against the parent, so corrections do not have to compose by
    # hand. A leaf (hand, head, toe) has no direction of its own and inherits
    # its parent's, which keeps a hand from twisting off the forearm.
    align = {}
    if not no_align:
        for t_name in BONES:
            ti = tgt.by_name[t_name]
            kids = [c for c in tgt.nodes[ti].get("children", [])
                    if tgt.nodes[c].get("name") in BONES]
            if len(kids) == 1:
                child = tgt.nodes[kids[0]]["name"]
            elif t_name in DIRECTION:
                child = DIRECTION[t_name]
            else:
                continue  # leaf; filled in below
            si, sc = src.by_name[BONES[t_name]], src.by_name[BONES[child]]
            tc = tgt.by_name[child]
            ds = src.grest_p[sc] - src.grest_p[si]
            dt = tgt.grest_p[tc] - tgt.grest_p[ti]
            ns, nt = np.linalg.norm(ds), np.linalg.norm(dt)
            if ns < 1e-9 or nt < 1e-9:
                continue
            align[ti] = qbetween(dt / nt, ds / ns)
        # Leaves inherit the nearest mapped ancestor's correction.
        for t_name in BONES:
            ti = tgt.by_name[t_name]
            if ti in align:
                continue
            p = tgt.parent.get(ti)
            while p is not None and p not in align:
                p = tgt.parent.get(p)
            align[ti] = align[p] if p is not None else np.array([0.0, 0.0, 0.0, 1.0])
        worst = max(align.items(),
                    key=lambda kv: abs(2.0 * np.arccos(min(1.0, abs(kv[1][3])))))
        print(f"  rest alignment: worst "
              f"{np.degrees(2.0 * np.arccos(min(1.0, abs(worst[1][3])))):.1f}deg "
              f"on {tgt.nodes[worst[0]].get('name')}")
    anchor = {i: (qmul(align[i], tgt.grest_r[i]) if i in align else tgt.grest_r[i])
              for i in tmap}

    hips_t = tgt.by_name[ROOT_BONE]
    hips_s = src.by_name[BONES[ROOT_BONE]]
    # Hip height decides how far a jump carries on a body of another build.
    hip_ratio = 1.0
    if src.grest_p[hips_s][1] > 1e-6:
        hip_ratio = float(tgt.grest_p[hips_t][1] / src.grest_p[hips_s][1])
    # World metres -> the target's own local units for that channel. This rig
    # keeps its joints in centimetres under a 0.01 root, so this is 100.
    hips_parent = tgt.parent.get(hips_t)
    unit = 1.0 / (tgt.gscale[hips_parent] if hips_parent is not None else 1.0)
    hips_parent_q = tgt.grest_r[hips_parent] if hips_parent is not None else np.array([0.0, 0, 0, 1.0])

    have = {a.get("name") for a in tg.get("animations", [])}
    torder = tgt.order()
    new_blob = bytearray(tb)
    added = 0

    def push(arr, comp_type, kind):
        """Append an accessor over fresh bytes and return its index."""
        nonlocal new_blob
        data = np.ascontiguousarray(arr, dtype="<f4").tobytes()
        while len(new_blob) % 4:
            new_blob += b"\0"
        bv = {"buffer": 0, "byteOffset": len(new_blob), "byteLength": len(data)}
        new_blob += data
        tg.setdefault("bufferViews", []).append(bv)
        acc = {
            "bufferView": len(tg["bufferViews"]) - 1,
            "componentType": comp_type,
            "count": int(len(arr)),
            "type": kind,
        }
        if kind == "SCALAR":
            acc["min"] = [float(np.min(arr))]
            acc["max"] = [float(np.max(arr))]
        tg.setdefault("accessors", []).append(acc)
        return len(tg["accessors"]) - 1

    for anim in sg.get("animations", []):
        name = anim.get("name") or f"clip{added}"
        if only and name not in only:
            continue
        if name in skip:
            continue
        dur = duration(sg, anim)
        if dur <= 0.0:
            print(f"  skip {name}: zero length")
            continue
        chans = channels_of(sg, sb, anim)
        n = max(2, int(round(dur * fps)) + 1)
        times = np.linspace(0.0, dur, n)

        rot_out = {t: np.empty((n, 4), dtype=float) for t in tmap}
        hips_out = np.empty((n, 3), dtype=float)

        for k, t in enumerate(times):
            # Source globals, over the WHOLE source hierarchy — unmapped
            # ancestors included, because the armature root is where a
            # root-motion clip puts the body's travel.
            gs_r, gs_p = {}, {}
            for i_node in src.order():
                p = src.parent.get(i_node)
                pq = gs_r[p] if p is not None else np.array([0.0, 0, 0, 1.0])
                pp = gs_p[p] if p is not None else np.zeros(3)
                ps = src.gscale[p] if p is not None else 1.0
                c = chans.get(i_node)
                lr = c.rotation(t, src.rest_r[i_node]) if c else src.rest_r[i_node]
                lt = c.translation(t, src.rest_t[i_node]) if c else src.rest_t[i_node]
                gs_r[i_node] = qmul(pq, lr)
                gs_p[i_node] = pp + qrot(pq, lt * ps)

            # Target globals: the world delta from rest, re-anchored on the
            # target's own rest, then localised.
            gt_r = {}
            for i_node in torder:
                p = tgt.parent.get(i_node)
                pq = gt_r[p] if p is not None else np.array([0.0, 0, 0, 1.0])
                if i_node in tmap:
                    s_node = tmap[i_node]
                    d = qmul(gs_r[s_node], qinv(src.grest_r[s_node]))
                    gt_r[i_node] = qmul(d, anchor[i_node])
                    rot_out[i_node][k] = qmul(qinv(pq), gt_r[i_node])
                else:
                    gt_r[i_node] = qmul(pq, tgt.rest_r[i_node])

            # The body through the world. Everything else keeps the target's
            # own bone lengths, on purpose.
            dp = (gs_p[hips_s] - src.grest_p[hips_s]) * hip_ratio
            local = qrot(qinv(hips_parent_q), dp) * unit
            hips_out[k] = tgt.rest_t[hips_t] + local

        t_acc = push(times, 5126, "SCALAR")
        samplers, channels = [], []
        for node, arr in rot_out.items():
            arr = arr / np.linalg.norm(arr, axis=1, keepdims=True)
            samplers.append({"input": t_acc, "output": push(arr, 5126, "VEC4"),
                             "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1,
                             "target": {"node": node, "path": "rotation"}})
        samplers.append({"input": t_acc, "output": push(hips_out, 5126, "VEC3"),
                         "interpolation": "LINEAR"})
        channels.append({"sampler": len(samplers) - 1,
                         "target": {"node": hips_t, "path": "translation"}})

        out_name = name
        if out_name in have:
            # The target's own clip keeps the bare name: it was authored for
            # this character and a retarget of somebody else's is not an
            # improvement on it.
            out_name = name + suffix
        have.add(out_name)
        tg.setdefault("animations", []).append(
            {"name": out_name, "samplers": samplers, "channels": channels})
        added += 1
        print(f"  {out_name:24s} {dur:5.2f}s  {n:4d} frames")

    tg["buffers"][0]["byteLength"] = len(new_blob)
    write_glb(out_path, tg, new_blob)
    was = Path(tgt_path).stat().st_size
    now = Path(out_path).stat().st_size
    print(f"\n  {Path(out_path).name}: +{added} clips, "
          f"{was/1e6:.1f} -> {now/1e6:.1f} MB, {len(tg['animations'])} total")


if __name__ == "__main__":
    main()
