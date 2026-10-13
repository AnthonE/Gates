#!/usr/bin/env python3
"""Rig and animate a generated four-legged animal in headless Blender.

    blender -b --factory-startup -P ci/rig_beast.py -- \\
        --in raw.glb --out wolf_a.glb --species wolf [--preview DIR]

Why this exists rather than a vendor's auto-rigger: Meshy's API rigs
humanoids only, and every animal auto-rigger measured (Tripo, RigNet,
SkinTokens, Anything World) returns legs too straight to walk without
crouching. Meshy's own web-app quadruped (`To Examine/…Spinefiend…`) has no
jaw, no neck and one spine joint, so it can neither bite nor howl. This
script builds the skeleton FROM THE MESH and authors the clips itself.

The steps, in order:

1. **Normalise.** Join, face the animal down Blender -Y (glTF +Z, the
   sim's yaw-0 nose direction), stand it on z = 0, scale it uniformly to the
   species' shoulder height, centre it between its leg columns.
2. **Find the landmarks.** The four leg columns from a low slice, each
   tracked up to where it joins the body; the back and belly line; the
   nose tip, the skull and the tail from slices along the body.
3. **Build the skeleton.** Every bone's roll puts its local X on world X,
   so all flexion is one rotation about one axis and a leg is planar
   arithmetic. Joint heights are anatomical fractions of the column they
   sit in (`SPECIES`), placed on the measured leg line.
4. **Weight it.** Bone heat on a watertight voxel proxy, transferred to the
   real mesh (heat weighting fails outright on the non-manifold bits a
   generator leaves), then the jaw is cut geometrically: heat cannot see a
   closed mouth.
5. **Animate.** Idle, Walk, Trot, Run, Bite, Howl, Lie. Feet are placed by
   trajectory and the legs solved by planar IK every frame, so a foot in
   stance stays where it landed at the clip's native speed (`CLIP_MPS` in
   the report, which `render/beasts.rs` plays the clip against).
6. **Glow.** The fel cracks are found in the albedo (bright, green-dominant)
   and written as an emissive map; nothing the generator calls emissive is
   trusted (`ci/import_meshy.py`'s rule).

The output is a GLB with one skinned mesh, one armature and the seven named
clips; `ci/ktx_pack.py` packs its maps afterwards.
"""
import json
import math
import os
import sys

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector

FPS = 30

# Per-species shape: the target size and where the joints sit, as fractions
# of the leg column's height (hind: the croup above the hind legs; front:
# the withers above the forelegs). `len_m`/`h_m` match `render/mobs.rs`'s
# WOLF_/PIG_ constants, so the hit cylinder and the drawn body agree.
SPECIES = {
    "wolf": {
        "h_m": 0.85,
        "head_frac": 0.17,
        "hind": {"hip": 0.80, "stifle": 0.54, "hock": 0.29, "paw": 0.045},
        "front": {"withers": 0.96, "shoulder": 0.74, "elbow": 0.52, "wrist": 0.21, "paw": 0.04},
        "gaits": {
            # name: (native m/s, cycle s, duty, lift as a fraction of leg height, offsets LH LF RH RF)
            "Walk": (1.3, 1.00, 0.62, 0.10, (0.0, 0.25, 0.5, 0.75)),
            "Trot": (2.7, 0.60, 0.42, 0.15, (0.0, 0.5, 0.5, 0.0)),
            "Run": (5.0, 0.44, 0.32, 0.22, (0.0, 0.62, 0.12, 0.52)),
        },
    },
    "stag": {
        # Withers, not the head: `render/mobs.rs`'s 1.3 m is to the top of
        # the skull, and the antlers stand above that, as a rack does.
        "h_m": 1.05,
        # The neck rises straight off the shoulders and the rack above it, so
        # the withers are read a third of the way back toward the hips.
        "withers_at": 0.33,
        "head_frac": 0.16,
        "hind": {"hip": 0.82, "stifle": 0.58, "hock": 0.36, "paw": 0.04},
        "front": {"withers": 0.97, "shoulder": 0.76, "elbow": 0.58, "wrist": 0.28, "paw": 0.035},
        "gaits": {
            "Walk": (1.3, 1.10, 0.62, 0.12, (0.0, 0.25, 0.5, 0.75)),
            "Trot": (2.8, 0.62, 0.42, 0.18, (0.0, 0.5, 0.5, 0.0)),
            "Run": (5.5, 0.46, 0.30, 0.28, (0.0, 0.08, 0.55, 0.62)),
        },
    },
    "pig": {
        "h_m": 0.78,
        "head_frac": 0.21,
        "hind": {"hip": 0.80, "stifle": 0.50, "hock": 0.24, "paw": 0.05},
        "front": {"withers": 0.95, "shoulder": 0.72, "elbow": 0.48, "wrist": 0.19, "paw": 0.045},
        "gaits": {
            "Walk": (1.1, 0.90, 0.64, 0.09, (0.0, 0.25, 0.5, 0.75)),
            "Trot": (2.3, 0.55, 0.45, 0.13, (0.0, 0.5, 0.5, 0.0)),
            "Run": (4.0, 0.42, 0.36, 0.18, (0.0, 0.58, 0.10, 0.48)),
        },
    },
}

LEGS = ("LH", "LF", "RH", "RF")


def args():
    a = sys.argv[sys.argv.index("--") + 1:]
    out = {"preview": None, "facing": "auto", "tris": 0}
    i = 0
    while i < len(a):
        k = a[i].lstrip("-")
        out[k] = a[i + 1]
        i += 2
    out["tris"] = int(out["tris"])
    return out


# ── 1 · import and normalise ────────────────────────────────────────────


def import_mesh(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=path)
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    bpy.ops.object.select_all(action="DESELECT")
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.parent_clear(type="CLEAR_KEEP_TRANSFORM")
    if len(meshes) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    for o in list(bpy.context.scene.objects):
        if o is not obj:
            bpy.data.objects.remove(o, do_unlink=True)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    obj.name = "Beast"
    return obj


def verts(obj):
    n = len(obj.data.vertices)
    a = np.empty(n * 3, dtype=np.float64)
    obj.data.vertices.foreach_get("co", a)
    return a.reshape(-1, 3)


def bake(obj, m):
    obj.data.transform(m)
    obj.data.update()


def leg_columns(V, h):
    """Four leg centroids from a low slice, split front/back by the widest
    gap along Y and left/right by X. Returns {leg: (x, y)} and the slice."""
    band = V[(V[:, 2] > 0.10 * h) & (V[:, 2] < 0.17 * h)]
    ys = np.sort(band[:, 1])
    gaps = np.diff(ys)
    # The widest gap in the middle 80% of the slice is the belly.
    lo, hi = int(len(ys) * 0.1), int(len(ys) * 0.9)
    g = lo + int(np.argmax(gaps[lo:hi]))
    split = 0.5 * (ys[g] + ys[g + 1])
    out = {}
    for front in (True, False):
        part = band[band[:, 1] < split] if front else band[band[:, 1] >= split]
        xm = np.median(part[:, 0])
        for left in (True, False):
            leg = part[part[:, 0] >= xm] if left else part[part[:, 0] < xm]
            out[("L" if left else "R") + ("F" if front else "H")] = (
                float(np.mean(leg[:, 0])), float(np.mean(leg[:, 1])))
    return out, split


def normalise(obj, sp, facing):
    V = verts(obj)
    ext = V.max(0) - V.min(0)
    # Long axis onto Y.
    if ext[0] > ext[1]:
        bake(obj, Matrix.Rotation(math.radians(90), 4, "Z"))
        V = verts(obj)
    V0 = V.min(0)
    bake(obj, Matrix.Translation((0, 0, -V0[2])))
    V = verts(obj)
    h0 = V[:, 2].max()
    cols, split = leg_columns(V, 0.6 * h0)
    # Which end is the head: more surface beyond the legs, and higher. A tail
    # is thin and low; a head and neck are neither.
    ylegs = [c[1] for c in cols.values()]
    ymin, ymax = min(ylegs), max(ylegs)
    span = ymax - ymin
    minus = V[V[:, 1] < ymin - 0.15 * span]
    plus = V[V[:, 1] > ymax + 0.15 * span]
    score_minus = len(minus) * (np.mean(minus[:, 2]) if len(minus) else 0)
    score_plus = len(plus) * (np.mean(plus[:, 2]) if len(plus) else 0)
    head_minus = score_minus >= score_plus
    if facing in ("+y", "-y"):
        head_minus = facing == "-y"
    if not head_minus:
        bake(obj, Matrix.Rotation(math.radians(180), 4, "Z"))
        V = verts(obj)
        cols, split = leg_columns(V, 0.6 * h0)
    # Centre: X on the leg columns' mean, Y midway between front and hind.
    cx = np.mean([c[0] for c in cols.values()])
    cy = 0.5 * (np.mean([cols["LF"][1], cols["RF"][1]]) + np.mean([cols["LH"][1], cols["RH"][1]]))
    bake(obj, Matrix.Translation((-cx, -cy, 0)))
    V = verts(obj)
    # Scale on the withers: the 90th percentile of height in a band around
    # the foreleg columns, BEHIND the head (spikes and horns are what a max
    # would find).
    fy = np.mean([cols["LF"][1], cols["RF"][1]]) - cy
    hy = np.mean([cols["LH"][1], cols["RH"][1]]) - cy
    fy = fy + sp.get("withers_at", 0.0) * (hy - fy)
    band = V[np.abs(V[:, 1] - fy) < 0.06 * ext.max()]
    withers = float(np.percentile(band[:, 2], 92))
    k = sp["h_m"] / withers
    bake(obj, Matrix.Scale(k, 4))
    V = verts(obj)
    cols, split = leg_columns(V, sp["h_m"])
    return {
        "scale": k,
        "flipped": not head_minus,
        "len_m": float(V[:, 1].max() - V[:, 1].min()),
        "top_m": float(V[:, 2].max()),
        "cols": cols,
        "split": split,
    }


# ── 2 · landmarks ───────────────────────────────────────────────────────


def track_leg(V, col, h, others):
    """The leg's centreline from the ground up: centroids of thin slices,
    each taken around the last one and away from the other legs, until the
    slice widens into the body. Returns ([(z, x, y)], merge_z)."""
    x, y = col
    line = []
    r0 = None
    merge = None
    for z in np.arange(0.03 * h, 0.9 * h, 0.02 * h):
        s = V[(V[:, 2] > z - 0.025 * h) & (V[:, 2] < z + 0.025 * h)]
        if len(s) == 0:
            continue
        d = np.hypot(s[:, 0] - x, s[:, 1] - y)
        near = s[d < max(0.06 * h, (r0 or 0.0) * 2.6)]
        # Drop points nearer another leg's column.
        if len(near) and others:
            own = np.hypot(near[:, 0] - x, near[:, 1] - y)
            ok = np.ones(len(near), bool)
            for (ox, oy) in others:
                ok &= own <= np.hypot(near[:, 0] - ox, near[:, 1] - oy)
            near = near[ok]
        if len(near) < 4:
            continue
        nx, ny = float(np.mean(near[:, 0])), float(np.mean(near[:, 1]))
        r = float(np.percentile(np.hypot(near[:, 0] - nx, near[:, 1] - ny), 90))
        # The paw is wider than the leg above it, so the reference width is
        # the narrowest the leg has been, not its first slice.
        if z > 0.08 * h:
            r0 = r if r0 is None else min(r0, r)
        if r0 is not None and r > 2.2 * r0 and z > 0.25 * h:
            merge = z
            break
        line.append((float(z), nx, ny))
        x, y = nx, ny
    return line, merge if merge is not None else line[-1][0]


def along(line, z):
    """The leg line's (x, y) at height z, clamped to its ends."""
    zs = [p[0] for p in line]
    if z <= zs[0]:
        return line[0][1], line[0][2]
    if z >= zs[-1]:
        return line[-1][1], line[-1][2]
    i = int(np.searchsorted(zs, z))
    t = (z - zs[i - 1]) / (zs[i] - zs[i - 1])
    return (line[i - 1][1] * (1 - t) + line[i][1] * t, line[i - 1][2] * (1 - t) + line[i][2] * t)


def profile(V, ys, h, xmax):
    """Back and belly height at each y, over the body's middle in X."""
    back, belly = [], []
    for y in ys:
        s = V[(np.abs(V[:, 1] - y) < 0.02) & (np.abs(V[:, 0]) < xmax) & (V[:, 2] > 0.30 * h)]
        if len(s) < 8:
            back.append(np.nan)
            belly.append(np.nan)
            continue
        back.append(float(np.percentile(s[:, 2], 90)))
        belly.append(float(np.percentile(s[:, 2], 4)))
    return np.array(back), np.array(belly)


def slice_centre(V, y, zmin=0.0, w=0.02, zmax=1e9):
    s = V[(np.abs(V[:, 1] - y) < w) & (V[:, 2] > zmin) & (V[:, 2] < zmax)]
    if len(s) < 6:
        return None
    return float(np.median(s[:, 0])), float(np.mean(s[:, 2])), float(s[:, 2].min()), float(s[:, 2].max())


def landmarks(obj, sp, norm):
    V = verts(obj)
    h = sp["h_m"]
    cols = norm["cols"]
    lines, merges = {}, {}
    for leg in LEGS:
        others = [cols[o] for o in LEGS if o != leg]
        lines[leg], merges[leg] = track_leg(V, cols[leg], h, others)
    yF = 0.5 * (cols["LF"][1] + cols["RF"][1])
    yH = 0.5 * (cols["LH"][1] + cols["RH"][1])
    half_w = 0.5 * abs(cols["LF"][0] - cols["RF"][0]) + 0.08
    ys = np.linspace(yF, yH, 9)
    back, belly = profile(V, ys, h, half_w)
    # The nose is the front-most point BELOW the rack: a stag's brow tines
    # can reach further forward than its muzzle.
    low = V[V[:, 2] < 1.25 * h]
    nose_i = np.argsort(low[:, 1])[:20]
    nose = low[nose_i].mean(0)
    tail_i = np.argsort(-V[:, 1])[:20]
    tail = V[tail_i].mean(0)
    return {
        "lines": lines,
        "merges": merges,
        "yF": float(yF),
        "yH": float(yH),
        "ys": ys.tolist(),
        "back": back.tolist(),
        "belly": belly.tolist(),
        "nose": nose.tolist(),
        "tail": tail.tolist(),
        "V": V,
    }


# ── 3 · skeleton ────────────────────────────────────────────────────────


def sag_angle(d):
    """A direction's angle in the sagittal plane, from straight down toward
    +Y (backward). A positive rotation about +X increases it."""
    return math.atan2(d[1], -d[2])


def build_armature(sp, lm):
    h = sp["h_m"]
    V = lm["V"]
    lines = lm["lines"]
    yF, yH = lm["yF"], lm["yH"]
    ys, back, belly = np.array(lm["ys"]), np.array(lm["back"]), np.array(lm["belly"])
    ok = ~np.isnan(back)
    spine_z = np.interp(np.linspace(yF, yH, 9), ys[ok], (belly[ok] + 0.62 * (back[ok] - belly[ok])))

    def spine_at(y):
        return float(np.interp(y, np.linspace(yF, yH, 9), spine_z))

    hind, front = sp["hind"], sp["front"]
    croup = float(np.interp(yH, ys[ok], back[ok]))
    withers = float(np.interp(yF, ys[ok], back[ok]))
    J = {}
    # Hind legs: hip inside the body, the rest on the measured leg line.
    for leg in ("LH", "RH"):
        L = lines[leg]
        x0 = L[0][1]
        def on(frac, dy=0.0):
            z = frac * croup
            x, y = along(L, z)
            return Vector((x, y + dy, z))
        hip = on(hind["hip"]); hip.y = along(L, lm["merges"][leg])[1] + 0.06 * croup
        hip.x = 0.75 * along(L, lm["merges"][leg])[0]
        # The hock is the leg line's backward bend: its rear-most point low
        # down. Found when the line reaches that high, else the fraction.
        low = [p for p in L if 0.15 * croup < p[0] < 0.45 * croup]
        hock_f = (max(low, key=lambda p: p[2])[0] / croup) if len(low) >= 3 else hind["hock"]
        J[leg] = {
            "hip": hip,
            "knee": on(max(hind["stifle"], hock_f + 0.2), -0.05 * croup),
            "hock": on(hock_f, 0.02 * croup),
            "paw": on(hind["paw"]),
        }
        J[leg]["toe"] = Vector((J[leg]["paw"].x, J[leg]["paw"].y - 0.11 * croup, 0.012))
    for leg in ("LF", "RF"):
        L = lines[leg]
        def on(frac, dy=0.0):
            z = frac * withers
            x, y = along(L, z)
            return Vector((x, y + dy, z))
        top = along(L, lm["merges"][leg])
        J[leg] = {
            "withers": Vector((0.45 * top[0], top[1] + 0.10 * withers, front["withers"] * withers)),
            "hip": Vector((0.85 * top[0], top[1] - 0.05 * withers, front["shoulder"] * withers)),
            "knee": on(front["elbow"], 0.05 * withers),
            "hock": on(front["wrist"]),
            "paw": on(front["paw"]),
        }
        J[leg]["toe"] = Vector((J[leg]["paw"].x, J[leg]["paw"].y - 0.09 * withers, 0.012))

    # One skeleton for both sides: the mesh is near-symmetric and a measured
    # left and right differ by tracking noise, which would walk lopsided.
    for a, b in (("LH", "RH"), ("LF", "RF")):
        for k in J[a]:
            pa, pb = J[a][k], J[b][k]
            x = 0.5 * (abs(pa.x) + abs(pb.x))
            y, z = 0.5 * (pa.y + pb.y), 0.5 * (pa.z + pb.z)
            J[a][k] = Vector((x if pa.x >= 0 else -x, y, z))
            J[b][k] = Vector((x if pb.x >= 0 else -x, y, z))

    # Spine: pelvis behind the hips, three bones to the withers.
    yP = yH + 0.10 * croup
    pelvis = Vector((0, yP, spine_at(min(yP, yH))))
    sp1 = Vector((0, yH + 0.33 * (yF - yH), spine_at(yH + 0.33 * (yF - yH))))
    sp2 = Vector((0, yH + 0.66 * (yF - yH), spine_at(yH + 0.66 * (yF - yH))))
    chest = Vector((0, yF, spine_at(yF)))
    # Neck to the skull: slice centres between the withers and the nose.
    nose = Vector(lm["nose"])
    head_len = sp["head_frac"] * (lm["V"][:, 1].max() - lm["V"][:, 1].min())
    occ_y = nose.y + head_len
    occ = slice_centre(V, occ_y, zmin=0.35 * h, zmax=1.3 * h)
    occiput = Vector((0, occ_y, occ[1] + 0.15 * (occ[3] - occ[1])))
    mid_y = 0.5 * (chest.y + occ_y)
    mid = slice_centre(V, mid_y, zmin=0.35 * h, zmax=1.3 * h) or (0, 0.5 * (chest.z + occiput.z), 0, 0)
    neck_mid = Vector((0, mid_y, mid[1]))
    snout = slice_centre(V, nose.y + 0.3 * head_len, zmin=0.2 * h, zmax=1.3 * h)
    # Jaw: hinged under the ear, ending at the chin under the nose.
    jaw_h = Vector((0, nose.y + 0.78 * head_len, occiput.z - 0.35 * (occ[3] - occ[2])))
    chin = Vector((0, nose.y + 0.12 * head_len, snout[2] + 0.30 * (snout[3] - snout[2])))
    mouth_z = snout[2] + 0.42 * (snout[3] - snout[2])
    nose_tip = Vector((0, nose.y, nose.z))
    # Tail: from the pelvis to the rear-most point, four bones.
    tip = Vector(lm["tail"])
    tail_pts = [pelvis.lerp(tip, t) for t in (0.0, 0.22, 0.46, 0.72, 1.0)]
    for i, p in enumerate(tail_pts[1:-1], 1):
        c = slice_centre(V, p.y, zmin=0.25 * h, w=0.03)
        if c:
            p.z = c[1]
            p.x = c[0]

    bpy.ops.object.armature_add(enter_editmode=True, location=(0, 0, 0))
    arm = bpy.context.object
    arm.name = "Rig"
    arm.data.name = "Rig"
    eb = arm.data.edit_bones
    for b in list(eb):
        eb.remove(b)

    def bone(name, head, tail, parent=None, connect=False, deform=True):
        b = eb.new(name)
        b.head, b.tail = head, tail
        d = (tail - head).normalized()
        b.align_roll(Vector((1, 0, 0)).cross(d))
        if parent:
            b.parent = eb[parent]
            b.use_connect = connect
        b.use_deform = deform
        return b

    bone("Hips", pelvis, sp1)
    bone("Spine", sp1, sp2, "Hips", True)
    bone("Chest", sp2, chest, "Spine", True)
    bone("Neck1", chest, neck_mid, "Chest", True)
    bone("Neck2", neck_mid, occiput, "Neck1", True)
    bone("Head", occiput, nose_tip, "Neck2", True)
    bone("Jaw", jaw_h, chin, "Head")
    for i in range(4):
        bone(f"Tail{i + 1}", tail_pts[i], tail_pts[i + 1], "Hips" if i == 0 else f"Tail{i}", i > 0)
    for leg in LEGS:
        j = J[leg]
        side = leg[0]
        if leg[1] == "H":
            bone(f"Thigh.{side}", j["hip"], j["knee"], "Hips")
            bone(f"Shin.{side}", j["knee"], j["hock"], f"Thigh.{side}", True)
            bone(f"Foot.{side}H", j["hock"], j["paw"], f"Shin.{side}", True)
            bone(f"Toe.{side}H", j["paw"], j["toe"], f"Foot.{side}H", True)
        else:
            bone(f"Scapula.{side}", j["withers"], j["hip"], "Chest")
            bone(f"Arm.{side}", j["hip"], j["knee"], f"Scapula.{side}", True)
            bone(f"Forearm.{side}", j["knee"], j["hock"], f"Arm.{side}", True)
            bone(f"Foot.{side}F", j["hock"], j["paw"], f"Forearm.{side}", True)
            bone(f"Toe.{side}F", j["paw"], j["toe"], f"Foot.{side}F", True)
    bpy.ops.object.mode_set(mode="OBJECT")
    belly_mid = float(np.nanmin(belly[ok][2:-2])) if ok.sum() > 4 else 0.45 * h
    # Where the skull ends and anything standing on it (a rack) begins.
    skull_top = max(occ[3], snout[3]) + 0.03
    return arm, {"mouth_z": mouth_z, "head_len": head_len, "croup": croup, "withers": withers,
                 "nose_y": nose.y, "occ_y": occ_y, "belly_m": belly_mid,
                 "skull_top": max(skull_top, 1.3 * h)}


# ── 4 · weights ─────────────────────────────────────────────────────────


def bare_of(obj):
    total = np.zeros(len(obj.data.vertices))
    for v in obj.data.vertices:
        total[v.index] = sum(g.weight for g in v.groups)
    return np.where(total < 0.05)[0]


def heat(obj, arm, voxel, zcut):
    """Bone heat on a watertight voxel proxy, transferred to the real mesh.

    The proxy stops at `zcut`: a stag's rack failed the heat solve outright
    on two of three stags (21,126 and 23,559 vertices left bare even at a
    6 mm proxy), and a rack does not bend anyway. Cut off, its vertices take
    the nearest proxy face's weights, which is the crown of the skull, so
    the antlers ride the head."""
    for g in list(obj.vertex_groups):
        obj.vertex_groups.remove(g)
    proxy = obj.copy()
    proxy.data = obj.data.copy()
    proxy.name = "Proxy"
    bpy.context.scene.collection.objects.link(proxy)
    m = proxy.modifiers.new("vox", "REMESH")
    m.mode = "VOXEL"
    m.voxel_size = voxel
    bpy.context.view_layer.objects.active = proxy
    bpy.ops.object.select_all(action="DESELECT")
    proxy.select_set(True)
    bpy.ops.object.modifier_apply(modifier="vox")
    d = proxy.modifiers.new("dec", "DECIMATE")
    d.ratio = min(1.0, 60000 / max(1, len(proxy.data.polygons)))
    bpy.ops.object.modifier_apply(modifier="dec")
    bm = bmesh.new()
    bm.from_mesh(proxy.data)
    above = [v for v in bm.verts if v.co.z > zcut]
    if above:
        bmesh.ops.delete(bm, geom=above, context="VERTS")
        bm.to_mesh(proxy.data)
    bm.free()
    bpy.ops.object.select_all(action="DESELECT")
    proxy.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    for b in arm.data.bones:
        if b.use_deform:
            obj.vertex_groups.new(name=b.name)
    dt = obj.modifiers.new("dt", "DATA_TRANSFER")
    dt.object = proxy
    dt.use_vert_data = True
    dt.data_types_verts = {"VGROUP_WEIGHTS"}
    dt.vert_mapping = "POLYINTERP_NEAREST"
    dt.layers_vgroup_select_src = "ALL"
    dt.layers_vgroup_select_dst = "NAME"
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.ops.object.modifier_apply(modifier="dt")
    bpy.data.objects.remove(proxy, do_unlink=True)
    return bare_of(obj)


# Proxy resolutions tried in order. Heat's solve is sensitive to the proxy:
# measured on the horned wolf, 12 mm left 18,658 vertices unweighted, 9 mm
# 59 and 6 mm none, so the finer proxies are the retry, not the default
# (they cost the solve several times over on the meshes that never needed it).
VOXELS = (0.012, 0.009, 0.006)


def weight(obj, arm, info):
    bare = None
    for voxel in VOXELS:
        bare = heat(obj, arm, voxel, info["skull_top"])
        info["voxel_m"] = voxel
        if len(bare) <= 0.002 * len(obj.data.vertices):
            break

    # Bone heat gives up on some shapes outright ("failed to find solution",
    # measured on the horned wolf), and a vertex with no weight at all makes
    # the glTF exporter crash adding a neutral bone. Whatever it left bare is
    # weighted to its two nearest bones by distance to their segments.
    V = verts(obj)
    deform = [b for b in arm.data.bones if b.use_deform]
    A = np.array([b.head_local for b in deform])
    B = np.array([b.tail_local for b in deform])
    if len(bare):
        P = V[bare][:, None, :]
        AB = (B - A)[None]
        t = np.clip(((P - A[None]) * AB).sum(-1) / (AB * AB).sum(-1), 0, 1)
        d = np.linalg.norm(P - (A[None] + t[..., None] * AB), axis=-1) + 1e-4
        near = np.argsort(d, axis=1)[:, :2]
        for row, i in enumerate(bare):
            j0, j1 = near[row]
            w0, w1 = 1 / d[row, j0] ** 4, 1 / d[row, j1] ** 4
            obj.vertex_groups[deform[j0].name].add([int(i)], float(w0 / (w0 + w1)), "REPLACE")
            obj.vertex_groups[deform[j1].name].add([int(i)], float(w1 / (w0 + w1)), "REPLACE")
    info["bare_verts"] = int(len(bare))

    # The jaw, cut by geometry: under the mouth line, ahead of the hinge.
    V = verts(obj)
    jaw = obj.vertex_groups["Jaw"]
    names = {g.index: g.name for g in obj.vertex_groups}
    hinge_y = info["nose_y"] + 0.85 * info["head_len"]
    moved = 0
    for i, v in enumerate(V):
        if v[1] > hinge_y or v[2] > info["mouth_z"] + 0.02 or v[2] < info["mouth_z"] - 0.25:
            continue
        # Only what heat already gave the head or neck: a jaw cut must not
        # reach a forepaw raised under the chin.
        mine = {names[g.group]: g.weight for g in obj.data.vertices[i].groups}
        if sum(w for n, w in mine.items() if n in ("Head", "Neck2", "Jaw")) < 0.5:
            continue
        # Fade over the last 15% toward the hinge and 2 cm about the lip line.
        t_y = min(1.0, (hinge_y - v[1]) / (0.15 * info["head_len"]))
        t_z = min(1.0, max(0.0, (info["mouth_z"] + 0.02 - v[2]) / 0.03))
        w = t_y * t_z
        if w <= 0:
            continue
        for n, old in mine.items():
            if n != "Jaw":
                obj.vertex_groups[n].add([i], old * (1 - w), "REPLACE")
        jaw.add([i], mine.get("Jaw", 0.0) * (1 - w) + w, "REPLACE")
        moved += 1
    info["jaw_verts"] = moved
    # Four influences, normalised: what glTF and Bevy carry.
    bpy.ops.object.mode_set(mode="WEIGHT_PAINT")
    bpy.ops.object.vertex_group_limit_total(group_select_mode="ALL", limit=4)
    bpy.ops.object.vertex_group_normalize_all(group_select_mode="ALL", lock_active=False)
    bpy.ops.object.mode_set(mode="OBJECT")
    obj.parent = arm
    mod = obj.modifiers.new("Armature", "ARMATURE")
    mod.object = arm


# ── 5 · clips ───────────────────────────────────────────────────────────


class Pose2D:
    """Planar forward kinematics over the rig's bones, all rotating about X."""

    def __init__(self, arm):
        self.arm = arm
        self.rest = {}
        for b in arm.data.bones:
            h, t = b.head_local.copy(), b.tail_local.copy()
            self.rest[b.name] = (h, t, b.parent.name if b.parent else None)
        self.order = [b.name for b in arm.data.bones]  # parents first

    def angle_rest(self, name):
        h, t, _ = self.rest[name]
        return sag_angle(t - h)

    def length(self, name):
        h, t, _ = self.rest[name]
        return (t - h).length


def rot_yz(v, a):
    c, s = math.cos(a), math.sin(a)
    return Vector((v.x, v.y * c - v.z * s, v.y * s + v.z * c))


def solve2(H, Q, L1, L2, bend_back):
    """Two-segment planar IK from H to Q; returns the two absolute angles."""
    dv = Q - H
    D = max(abs(L1 - L2) + 1e-4, min(L1 + L2 - 1e-4, math.hypot(dv.y, dv.z)))
    beta = math.atan2(dv.y, -dv.z)
    g = math.acos(max(-1.0, min(1.0, (L1 * L1 + D * D - L2 * L2) / (2 * L1 * D))))
    a1 = beta + g if bend_back else beta - g
    K = H + Vector((0, L1 * math.sin(a1), -L1 * math.cos(a1)))
    k = Q - K
    a2 = math.atan2(k.y, -k.z)
    return a1, a2


def dir_of(a):
    return Vector((0, math.sin(a), -math.cos(a)))


class Animator:
    def __init__(self, arm, sp):
        self.arm = arm
        self.sp = sp
        self.P = Pose2D(arm)
        for pb in arm.pose.bones:
            pb.rotation_mode = "QUATERNION"

    def leg_bones(self, leg):
        s = leg[0]
        if leg[1] == "H":
            return [f"Thigh.{s}", f"Shin.{s}", f"Foot.{s}H", f"Toe.{s}H"], "Hips"
        return [f"Arm.{s}", f"Forearm.{s}", f"Foot.{s}F", f"Toe.{s}F"], f"Scapula.{s}"

    def frame(self, f, body):
        """Key one frame. `body` holds the frame's intent:
        bob (m), lunge (m, forward), pitch (rad, + nose up) for the trunk;
        spine (rad), neck (rad), head (rad), jaw (rad); tail (wag rad, lift rad);
        paws {leg: (dy, dz)} offsets from rest in the ground frame;
        toe {leg: rad}; wrist {leg: rad}."""
        P = self.P
        q = {}
        delta = {}  # world sagittal delta per bone
        pos = {}  # posed head per bone
        # Trunk: the Hips bone carries the bob, the lunge and the pitch.
        hips_h, _, _ = P.rest["Hips"]
        pitch = -body.get("pitch", 0.0)  # + nose up is a negative turn about X
        delta["Hips"] = pitch
        trans = Vector((0, -body.get("lunge", 0.0), body.get("bob", 0.0)))
        pos["Hips"] = hips_h + trans
        q["Hips"] = (pitch, trans)
        local = {
            "Spine": -body.get("spine", 0.0) * 0.5,
            "Chest": -body.get("spine", 0.0) * 0.5,
            "Neck1": -body.get("neck", 0.0) * 0.5,
            "Neck2": -body.get("neck", 0.0) * 0.5,
            "Head": -body.get("head", 0.0),
            "Jaw": body.get("jaw", 0.0),
        }
        for name in P.order:
            h, t, parent = P.rest[name]
            if name == "Hips" or name.startswith(("Thigh", "Shin", "Foot", "Toe", "Arm", "Forearm")):
                continue  # the root above; the legs are solved below
            ph, _, _ = P.rest[parent]
            pos[name] = pos[parent] + rot_yz(h - ph, delta[parent])
            d = local.get(name, 0.0)
            delta[name] = delta[parent] + d
            q[name] = (d, None)
        # Tail: a wag about each bone's own Z, a lift about X, shared out.
        wag, lift = body.get("tail", (0.0, 0.0))
        for i in range(4):
            q[f"Tail{i + 1}"] = (-lift / 4, None, wag / 4)
        # Legs.
        for leg in LEGS:
            names, root = self.leg_bones(leg)
            H = pos[root] + rot_yz(P.rest[names[0]][0] - P.rest[root][0], delta[root])
            L = [P.length(n) for n in names]
            ar = [P.angle_rest(n) for n in names]
            paw_rest = P.rest[names[2]][1]
            dy, dz = body.get("paws", {}).get(leg, (0.0, 0.0))
            Ptarget = Vector((paw_rest.x, paw_rest.y + dy, paw_rest.z + dz))
            if leg[1] == "H":
                # Pantograph: the cannon stays parallel-offset to the thigh.
                c = ar[2] - ar[0]
                a1 = ar[0]
                for _ in range(12):
                    Q = Ptarget - L[2] * dir_of(a1 + c)
                    a1, a2 = solve2(H, Q, L[0], L[1], bend_back=False)
                a3 = a1 + c
            else:
                w = body.get("wrist", {}).get(leg, 0.0)
                a2 = ar[1]
                for _ in range(12):
                    Q = Ptarget - L[2] * dir_of(a2 + (ar[2] - ar[1]) + w)
                    a1, a2 = solve2(H, Q, L[0], L[1], bend_back=True)
                a3 = a2 + (ar[2] - ar[1]) + w
            a4 = ar[3] + body.get("toe", {}).get(leg, 0.0)
            absolute = [a1, a2, a3, a4]
            parent_d = delta[root]
            prev = parent_d
            for n, a, r in zip(names, absolute, ar):
                d_world = a - r
                q[n] = (d_world - prev, None)
                delta[n] = d_world
                prev = d_world
        # Write keys.
        for pb in self.arm.pose.bones:
            v = q.get(pb.name)
            if v is None:
                pb.rotation_quaternion = Quaternion()
                pb.location = Vector()
            else:
                rot = Quaternion(Vector((1, 0, 0)), v[0])
                if len(v) > 2:
                    rot = rot @ Quaternion(Vector((0, 0, 1)), v[2])
                pb.rotation_quaternion = rot
                if v[1] is not None:
                    # Armature-space translation into the bone's rest frame.
                    R = pb.bone.matrix_local.to_3x3().inverted()
                    pb.location = R @ v[1]
                else:
                    pb.location = Vector()
            pb.keyframe_insert("rotation_quaternion", frame=f)
            pb.keyframe_insert("location", frame=f)

    def clip(self, name, seconds, fn, loop=True):
        act = bpy.data.actions.new(name)
        act.use_fake_user = True
        self.arm.animation_data_create()
        self.arm.animation_data.action = act
        n = max(2, int(round(seconds * FPS)))
        last = n if loop else n
        for i in range(last + 1):
            t = i / n
            self.frame(i + 1, fn(t))
        # Linear between samples; the last key of a loop equals the first.
        for fc in act.fcurves:
            for k in fc.keyframe_points:
                k.interpolation = "LINEAR"
        tr = self.arm.animation_data.nla_tracks.new()
        tr.name = name
        tr.strips.new(name, 1, act)
        tr.mute = True
        self.arm.animation_data.action = None
        return act


def smooth(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


def gait_fn(sp, name, legh):
    v, T, duty, lift, offs = sp["gaits"][name]
    S = v * duty * T  # metres a paw travels backward in stance
    run = name == "Run"
    trot = name == "Trot"

    def fn(t):
        paws, toes, wrists = {}, {}, {}
        for leg, o in zip(LEGS, offs):
            p = (t + o) % 1.0
            if p < duty:
                s = p / duty
                dy = -S / 2 + S * s  # forward (-y) at touchdown, back at lift-off
                dz = 0.0
                toes[leg] = 0.0
                wrists[leg] = 0.0
            else:
                s = (p - duty) / (1 - duty)
                e = smooth(s)
                dy = S / 2 - S * e
                dz = lift * legh * math.sin(math.pi * s)
                toes[leg] = 0.5 * math.sin(math.pi * s)
                wrists[leg] = 1.1 * math.sin(math.pi * min(1.0, s * 1.3)) if leg[1] == "F" else 0.0
            paws[leg] = (dy, dz)
        bob_amp = 0.012 if not run else 0.03
        b = {
            "paws": paws,
            "toe": toes,
            "wrist": wrists,
            "bob": -bob_amp * (0.5 + 0.5 * math.cos(4 * math.pi * t)) if not run else bob_amp * math.sin(2 * math.pi * t),
            "pitch": 0.06 * math.sin(2 * math.pi * t + 0.6) if run else 0.0,
            "spine": 0.16 * math.sin(2 * math.pi * t) if run else 0.0,
            "neck": (0.05 * math.sin(4 * math.pi * t) if not run else 0.10 * math.sin(2 * math.pi * t + 1.2)) - (0.12 if run else 0.0),
            "head": 0.03 * math.sin(4 * math.pi * t),
            "tail": (0.25 * math.sin(2 * math.pi * t) * (0.4 if trot or run else 1.0), 0.15 if run else 0.0),
        }
        return b

    return fn


def idle_fn(t):
    br = math.sin(2 * math.pi * t)
    return {
        "bob": 0.004 * br,
        "spine": 0.01 * br,
        "neck": 0.04 * math.sin(2 * math.pi * t + 0.8),
        "head": 0.05 * math.sin(4 * math.pi * t),
        "tail": (0.18 * math.sin(2 * math.pi * t), 0.0),
    }


def bite_fn(species, legh):
    def fn(t):
        # Wind back, lunge with the jaw wide, snap shut at the peak, recover.
        wind = smooth(t / 0.25) * (1 - smooth((t - 0.25) / 0.15))
        lunge = smooth((t - 0.25) / 0.2) * (1 - smooth((t - 0.6) / 0.4))
        jaw = smooth((t - 0.15) / 0.2) * (1 - smooth((t - 0.45) / 0.08))
        if species == "stag":
            # A butt: the rack lowered and driven forward, no bite in it.
            return {"lunge": 0.25 * legh * lunge - 0.05 * legh * wind, "pitch": -0.08 * lunge,
                    "neck": -0.50 * lunge, "head": -0.35 * lunge, "jaw": 0.05 * jaw,
                    "bob": -0.03 * wind, "tail": (0.0, 0.3 * lunge)}
        if species == "pig":
            # A gore: head down, then tossed up through the strike.
            toss = smooth((t - 0.40) / 0.15) * (1 - smooth((t - 0.6) / 0.35))
            return {"lunge": 0.22 * legh * lunge - 0.05 * legh * wind, "pitch": -0.10 * lunge,
                    "neck": -0.35 * lunge + 0.55 * toss, "head": -0.25 * lunge + 0.35 * toss,
                    "jaw": 0.2 * jaw, "bob": -0.03 * wind, "tail": (0.0, 0.2 * lunge)}
        return {"lunge": 0.30 * legh * lunge - 0.06 * legh * wind, "pitch": -0.12 * lunge,
                "neck": -0.25 * lunge + 0.10 * wind, "head": -0.15 * lunge, "jaw": 0.55 * jaw,
                "bob": -0.04 * wind, "tail": (0.0, 0.25 * lunge)}
    return fn


def howl_fn(species):
    def fn(t):
        up = smooth(t / 0.16) * (1 - smooth((t - 0.80) / 0.20))
        jaw = smooth((t - 0.10) / 0.12) * (1 - smooth((t - 0.78) / 0.12))
        if species == "stag":
            # A bellow: the neck stretched forward and up, the mouth wide.
            return {"neck": 0.40 * up, "head": -0.20 * up, "jaw": 0.40 * jaw, "pitch": 0.04 * up,
                    "tail": (0.0, 0.2 * up)}
        if species == "pig":
            # A squeal: the head jerks up and shakes.
            return {"neck": 0.45 * up, "head": 0.25 * up + 0.08 * up * math.sin(40 * t),
                    "jaw": 0.30 * jaw, "pitch": 0.05 * up, "tail": (0.3 * math.sin(20 * t) * up, 0.0)}
        return {"neck": 0.95 * up, "head": 0.40 * up, "jaw": 0.32 * jaw, "pitch": 0.10 * up,
                "bob": -0.03 * up, "tail": (0.0, -0.15 * up)}
    return fn


def lie_fn(legh, drop):
    # Down onto the belly, the sphinx a dog sleeps in: the trunk comes down
    # until the belly meets the ground, the forelegs fold and reach forward,
    # the hind legs tuck under, the head drops toward the paws.
    def fn(t):
        u = smooth(t)
        # The hind end settles first, the way a dog lies down.
        uh = smooth(t * 1.4)
        paws = {}
        for leg in LEGS:
            if leg[1] == "F":
                paws[leg] = (-0.55 * legh * u, 0.0)
            else:
                paws[leg] = (-0.30 * legh * uh, 0.0)
        return {"bob": -drop * u, "pitch": 0.10 * (uh - u), "paws": paws,
                "neck": -0.30 * u, "head": 0.10 * u,
                "toe": {k: 0.5 * u for k in LEGS}, "wrist": {"LF": 0.3 * u, "RF": 0.3 * u},
                "tail": (0.7 * u, 0.30 * u)}
    return fn


# ── 6 · glow ────────────────────────────────────────────────────────────


def fel_glow(obj):
    """Emissive map from the albedo's bright green, wired into the material."""
    mat = obj.active_material
    nt = mat.node_tree
    bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
    link = bsdf.inputs["Base Color"].links
    if not link:
        return 0.0
    img = link[0].from_node.image
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    px = px.reshape(-1, 4)
    r, g, b = px[:, 0], px[:, 1], px[:, 2]
    m = np.clip((g - np.maximum(r, b) * 1.25) / 0.15, 0, 1) * np.clip((g - 0.28) / 0.15, 0, 1)
    out = np.zeros_like(px)
    out[:, 0] = r * m
    out[:, 1] = g * m
    out[:, 2] = b * m
    out[:, 3] = 1.0
    em = bpy.data.images.new("fel_glow", w, h, alpha=False)
    em.pixels.foreach_set(out.ravel())
    em.file_format = "PNG"
    em.pack()
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = em
    uv = link[0].from_node.inputs["Vector"].links
    if uv:
        nt.links.new(uv[0].from_socket, tex.inputs["Vector"])
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Emission Color"])
    bsdf.inputs["Emission Strength"].default_value = 1.0
    return float(m.mean())


# ── preview ─────────────────────────────────────────────────────────────


def preview(arm, out_dir, slug, shots, scale=1.0):
    scn = bpy.context.scene
    scn.render.engine = "CYCLES"
    scn.cycles.samples = 12
    scn.cycles.use_denoising = False
    scn.render.resolution_x, scn.render.resolution_y = 720, 480
    scn.render.film_transparent = False
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.6, 0.62, 0.6, 1)
    world.node_tree.nodes["Background"].inputs[1].default_value = 1.0
    scn.world = world
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.0
    sun.rotation_euler = (math.radians(50), 0, math.radians(30))
    scn.collection.objects.link(sun)
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scn.collection.objects.link(cam)
    scn.camera = cam
    cam.location = Vector((3.2, -1.6, 1.1)) * scale
    cam.data.lens = 50
    d = Vector((0, 0, 0.45 * scale)) - cam.location
    cam.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()
    for act_name, frac in shots:
        act = bpy.data.actions[act_name]
        arm.animation_data.action = act
        f0, f1 = act.frame_range
        scn.frame_set(int(f0 + frac * (f1 - f0)))
        scn.render.filepath = os.path.join(out_dir, f"{slug}_{act_name}_{int(frac * 100):02d}.png")
        bpy.ops.render.render(write_still=True)
    arm.animation_data.action = None


def carcass(obj, arm, an, path):
    """The animal killed, as a static mesh in `render/mobs.rs`'s carcass frame:
    lying on its right side, the back along x = 0, the legs out toward -x,
    the belly-side on the ground. Posed limp first so it does not read as a
    statue tipped over."""
    limp = {"neck": -0.45, "head": -0.2, "jaw": 0.15, "tail": (0.0, 0.35),
            "paws": {"LF": (-0.08, 0.06), "RF": (-0.12, 0.04), "LH": (0.10, 0.05), "RH": (0.06, 0.03)},
            "toe": {k: 0.4 for k in LEGS}, "wrist": {"LF": 0.5, "RF": 0.4}}
    an.frame(1, limp)
    bpy.context.view_layer.update()
    dead = obj.copy()
    dead.data = obj.data.copy()
    dead.name = "Carcass"
    bpy.context.scene.collection.objects.link(dead)
    bpy.ops.object.select_all(action="DESELECT")
    dead.select_set(True)
    bpy.context.view_layer.objects.active = dead
    for mod in list(dead.modifiers):
        bpy.ops.object.modifier_apply(modifier=mod.name)
    # Lying still on the ground, it needs a fraction of the living animal's
    # triangles (and `ci/ktx_pack.py --size 512` packs it).
    dec = dead.modifiers.new("dec", "DECIMATE")
    dec.ratio = 0.4
    bpy.ops.object.modifier_apply(modifier="dec")
    dead.parent = None
    dead.matrix_world = Matrix.Identity(4)
    dead.vertex_groups.clear()
    bake(dead, Matrix.Rotation(math.radians(90), 4, "Y"))
    V = verts(dead)
    bake(dead, Matrix.Translation((-V[:, 0].max(), 0, -V[:, 2].min())))
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
                              export_animations=False, export_skins=False, export_yup=True,
                              export_materials="EXPORT", export_image_format="AUTO")
    bpy.data.objects.remove(dead, do_unlink=True)
    # Back to the rest pose for the main export.
    for pb in arm.pose.bones:
        pb.rotation_quaternion = Quaternion()
        pb.location = Vector()
    bpy.context.view_layer.update()


def main():
    a = args()
    sp = SPECIES[a["species"]]
    obj = import_mesh(a["in"])
    if a["tris"]:
        d = obj.modifiers.new("dec", "DECIMATE")
        d.ratio = min(1.0, a["tris"] / max(1, len(obj.data.polygons)))
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier="dec")
    norm = normalise(obj, sp, a["facing"])
    lm = landmarks(obj, sp, norm)
    arm, info = build_armature(sp, lm)
    weight(obj, arm, info)
    glow = fel_glow(obj)
    # The clips are keyed at FPS; the exporter times them by the scene's rate,
    # and a factory scene runs at 24 (every clip came out 1.25x long).
    bpy.context.scene.render.fps = FPS
    an = Animator(arm, sp)
    legh = 0.5 * (info["croup"] + info["withers"])
    an.clip("Idle", 3.0, idle_fn)
    clip_mps = {}
    for g in ("Walk", "Trot", "Run"):
        an.clip(g, sp["gaits"][g][1], gait_fn(sp, g, legh))
        clip_mps[g] = sp["gaits"][g][0]
    an.clip("Bite", 0.45, bite_fn(a["species"], legh), loop=False)
    an.clip("Howl", 2.2, howl_fn(a["species"]), loop=False)
    an.clip("Lie", 0.8, lie_fn(legh, max(0.0, info["belly_m"] - 0.04)), loop=False)
    for pb in arm.pose.bones:
        pb.rotation_quaternion = Quaternion()
        pb.location = Vector()
    report = {
        "species": a["species"],
        "scale": norm["scale"],
        "flipped": norm["flipped"],
        "len_m": norm["len_m"],
        "top_m": norm["top_m"],
        "croup_m": info["croup"],
        "withers_m": info["withers"],
        "glow_frac": glow,
        "tris": len(obj.data.polygons),
        "bones": len(arm.data.bones),
        "clip_mps": clip_mps,
        "merges": lm["merges"],
        "bare_verts": info.get("bare_verts", 0),
        "voxel_m": info.get("voxel_m"),
        "jaw_verts": info.get("jaw_verts", 0),
        "lines": {k: [[round(c, 3) for c in p] for p in v[::3]] for k, v in lm["lines"].items()},
    }
    if a.get("carcass"):
        # `frame` keys the pose it writes; key into a scratch action so the
        # clips stay as authored.
        scratch = bpy.data.actions.new("_limp")
        arm.animation_data.action = scratch
        carcass(obj, arm, an, a["carcass"])
        arm.animation_data.action = None
        bpy.data.actions.remove(scratch)
    if a["preview"]:
        os.makedirs(a["preview"], exist_ok=True)
        slug = os.path.splitext(os.path.basename(a["out"]))[0]
        # Framed for a wolf; anything taller (a stag and its rack) backs off.
        preview(arm, a["preview"], slug, [("Idle", 0.0), ("Walk", 0.25), ("Run", 0.3),
                                          ("Bite", 0.5), ("Howl", 0.5), ("Lie", 1.0)],
                scale=max(1.0, norm["top_m"] / 0.97))
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=a["out"],
        export_format="GLB",
        use_selection=False,
        export_animations=True,
        export_animation_mode="NLA_TRACKS",
        export_force_sampling=True,
        export_skins=True,
        export_all_influences=False,
        export_def_bones=True,
        export_yup=True,
        export_apply=False,
        export_materials="EXPORT",
        export_image_format="AUTO",
        export_reset_pose_bones=True,
        export_optimize_animation_size=False,
        export_anim_slide_to_zero=True,
    )
    report["glb_bytes"] = os.path.getsize(a["out"])
    with open(os.path.splitext(a["out"])[0] + ".rig.json", "w") as f:
        json.dump(report, f, indent=1)
    print("RIG_REPORT", json.dumps(report))


main()
