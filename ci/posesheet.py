#!/usr/bin/env python3
"""Draw the held item in the hand, for a list of candidate poses, on one sheet.

    ci/posesheet.py [out.png]

## What this is for

Picking `HeldModelDef`'s `lay`, `pose_yaw` and `scale` is a taste call, and
the only honest way to make one is to look. But looking costs `ci/scene.sh` --
a shard, a population, an Xvfb, six minutes -- **per candidate**, which is why
the hatchet's pose was converged on over three separate captures on
2026-09-01, two of which were wrong in a way that was obvious the moment the
frame arrived.

So this projects the geometry instead. It walks the same chain the client
does -- a model vertex through `viewmodel::pose`, into the item frame, then
`p_view = VIEWMODEL_HOLD + R(VIEWMODEL_TILT) · p_item`, which is the identity
`VIEWMODEL_GRIP_Q` was derived to satisfy -- and draws the result through the
renderer's own 75° vertical FOV at 16:9. The rig's right hand comes along, in
its own bone frame off `stumpy.glb`'s skin weights, so the axe is beside the
fist that holds it rather than floating in a void.

**It is not a gate and must not become one** (`CLAUDE.md`: there is no visual
gate here, a person looking is the visual gate, and the pixel gate that did
exist passed 36 checks on a beige smear). It scores nothing and asserts
nothing. It has no shading, no textures and no depth sort, so it can say
where a shape IS and never whether it looks good. What it buys is that six
poses cost one second instead of thirty-six minutes, and the one you then
photograph is a candidate somebody already chose.

Checked against the thing it models: the sheet's `lay 25 / yaw -120 / x0.85`
panel and the `ci/scene.sh` frame of that same row are the same picture.
"""
import json
import os
import struct
import sys

import numpy as np
from PIL import Image, ImageDraw

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


# ── the file ─────────────────────────────────────────────────────────────
def read_glb(path):
    raw = open(path, "rb").read()
    jlen = struct.unpack_from("<I", raw, 12)[0]
    gltf = json.loads(raw[20:20 + jlen])
    blen = struct.unpack_from("<I", raw, 20 + jlen)[0]
    return gltf, raw[20 + jlen + 8:20 + jlen + 8 + blen]


def accessor(gltf, blob, i):
    a = gltf["accessors"][i]
    ncomp = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}[a["type"]]
    dt = {5126: "<f4", 5123: "<u2", 5125: "<u4", 5121: "<u1"}[a["componentType"]]
    bv = gltf["bufferViews"][a["bufferView"]]
    off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    n = a["count"] * ncomp
    return np.frombuffer(blob, dtype=np.dtype(dt), count=n, offset=off).reshape(a["count"], ncomp)


# ── the constants, restated from the crate ───────────────────────────────
# A copy, deliberately: this is a bench, and a bench that imports the thing it
# is measuring cannot disagree with it out loud. If a number here drifts the
# sheet stops matching `ci/scene.sh`, which is the check.
HOLD = np.array([0.32, -0.30, -0.52])          # VIEWMODEL_HOLD
PALM = np.array([-0.040, 0.030, -0.015])       # VIEWMODEL_PALM
TILT_YXZ = (-0.50, 0.34, 0.14)                 # VIEWMODEL_TILT
GRIP_Q = (-0.163194, -0.429258, 0.687723, -0.562265)
GRIP_M = np.array([0.45838, -0.00628, -0.13165])
FOV_DEG, ASPECT = 75.0, 16 / 9                 # render::rig::FOV_DEG


def Rx(a):
    c, s = np.cos(a), np.sin(a)
    return np.array([[1, 0, 0], [0, c, -s], [0, s, c]])


def Ry(a):
    c, s = np.cos(a), np.sin(a)
    return np.array([[c, 0, s], [0, 1, 0], [-s, 0, c]])


def Rz(a):
    c, s = np.cos(a), np.sin(a)
    return np.array([[c, -s, 0], [s, c, 0], [0, 0, 1]])


def quat(x, y, z, w):
    n = np.sqrt(x * x + y * y + z * z + w * w)
    x, y, z, w = x / n, y / n, z / n, w / n
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


TILT = Ry(TILT_YXZ[0]) @ Rx(TILT_YXZ[1]) @ Rz(TILT_YXZ[2])
GRIP = quat(*GRIP_Q)


def held_model(rel):
    gltf, blob = read_glb(os.path.join(ROOT, "assets", rel))
    p = gltf["meshes"][0]["primitives"][0]
    return accessor(gltf, blob, p["attributes"]["POSITION"]).astype(np.float64)


def right_hand():
    """The rig's right hand, in the RightHand bone's own frame (centimetres)."""
    gltf, blob = read_glb(os.path.join(ROOT, "assets/models/stumpy.glb"))
    sk = gltf["skins"][0]
    names = [gltf["nodes"][n].get("name") for n in sk["joints"]]
    h = names.index("RightHand")
    ib = accessor(gltf, blob, sk["inverseBindMatrices"]).astype(np.float64)[h].reshape(4, 4).T
    out = []
    for m in gltf["meshes"]:
        p = m["primitives"][0]
        pos = accessor(gltf, blob, p["attributes"]["POSITION"]).astype(np.float64)
        jnt = accessor(gltf, blob, p["attributes"]["JOINTS_0"]).astype(np.int64)
        wgt = accessor(gltf, blob, p["attributes"]["WEIGHTS_0"]).astype(np.float64)
        sel = np.where(jnt == h, wgt, 0.0).sum(1) > 0.5
        v = pos[sel]
        out.append((np.concatenate([v, np.ones((len(v), 1))], 1) @ ib.T)[:, :3])
    return np.concatenate(out)


def hand_view(hand):
    # The bone's units are centimetres (the rig root carries scale 0.01) and
    # `HeldItem`'s own translation GRIP_M is in those units too, so both reach
    # metres by the same 0.01; GRIP transposed undoes the grip rotation.
    return HOLD + (0.01 * ((hand - GRIP_M) @ GRIP)) @ TILT.T


def item_view(model, height_m, grip_frac, lay, yaw, scale):
    """`viewmodel::pose` for one row, then the item frame into view space."""
    rot = Ry(yaw) @ Rx(-lay)
    t = PALM - rot @ (np.array([0, 1, 0.0]) * (height_m * scale * grip_frac))
    return HOLD + ((model * scale) @ rot.T + t) @ TILT.T


def haft_angles(lay, yaw):
    """Where the model's +Y points in VIEW SPACE: lean and elevation.

    ⚠ **The first of these is not the angle you see** and is returned only
    because the other two are worth having. See [`clock`].
    """
    v = TILT @ (Ry(yaw) @ Rx(-lay)) @ np.array([0, 1, 0.0])
    return (np.degrees(np.arctan2(v[1], v[0])),                    # NOT the screen angle
            np.degrees(np.arctan2(v[2], v[1])),                    # + = toward the eye
            np.degrees(np.arctan2(v[1], np.hypot(v[0], v[2]))))    # elevation


def oclock(screen_deg):
    """The screen angle as a clock hour, which is how the operator says it."""
    return int(round((90.0 - screen_deg) / 30.0)) % 12 or 12


def clock(model, height_m, grip_frac, lay, yaw, scale, frame=(1280, 720)):
    """The angle the item draws at ON SCREEN, degrees, 90 = straight up.

    Butt centroid to head centroid, projected, **in pixels**. This is the
    number to solve a rotation on, and the two obvious alternatives are both
    wrong -- each was shipped once here before the frame disagreed with it:

      · **The haft's direction vector** (`haft_angles`' first return) is in
        view space, where x and y are aspect-corrected and pixels are not.
        A 16:9 frame stretches x by 1.78, so the two differ by ~7 degrees:
        the vector read 99 on a pose the capture showed at 106.
      · **The principal axis of the projected silhouette** is the direction
        of the point CLOUD, which is the head's axis and not the haft's on
        any model whose head is a large mass hung off one side -- the stone
        hatchet's is 61% of its own length. It read 97 where the picture
        read 106, and solving a 60 degree swing on it laid the axe flat.

    Checked against the thing it models: it puts the hatchet's head at
    (830, 348) where the `ci/scene.sh` capture of that pose has it at about
    (835, 375), and read 11.5 o'clock on the pose the operator called 11.
    """
    iv = item_view(model, height_m, grip_frac, lay, yaw, scale)
    f = 1.0 / np.tan(np.radians(FOV_DEG) / 2)
    w, h = frame
    x = (f / ASPECT) * iv[:, 0] / (-iv[:, 2]) * (w / 2)
    y = f * iv[:, 1] / (-iv[:, 2]) * (h / 2)
    p = np.stack([x, y], 1)
    d = p[model[:, 1] > height_m * 0.80].mean(0) - p[model[:, 1] < height_m * 0.06].mean(0)
    return np.degrees(np.arctan2(d[1], d[0]))


def panel(pts_hand, pts_item, label, w, h):
    im = Image.new("RGB", (w, h), (28, 30, 34))
    d = ImageDraw.Draw(im)
    f = 1.0 / np.tan(np.radians(FOV_DEG) / 2)
    for pv, col in ((pts_hand, (150, 110, 80)), (pts_item, (225, 220, 205))):
        pv = pv[pv[:, 2] < -0.1]
        x = (f / ASPECT) * pv[:, 0] / (-pv[:, 2])
        y = f * pv[:, 1] / (-pv[:, 2])
        for px, py in zip((x * 0.5 + 0.5) * w, (0.5 - y * 0.5) * h):
            if 0 <= px < w and 0 <= py < h:
                d.point((px, py), fill=col)
    d.line([(w // 2 - 6, h // 2), (w // 2 + 6, h // 2)], fill=(90, 95, 100))
    d.line([(w // 2, h // 2 - 6), (w // 2, h // 2 + 6)], fill=(90, 95, 100))
    d.text((6, 6), label, fill=(235, 235, 235))
    return im


# The row under review, and the poses to compare. Edit these two.
ROW = dict(rel="models/held/stone_hatchet.glb", height_m=0.5616, grip_frac=0.25)
ROW["grip_frac"] = 0.15
CANDIDATES = [
    ("shipped", 0.960, -0.663, 0.85),
    ("before the 1 o'clock swing", 0.881, 0.244, 0.85),
    ("flat upright", 0.0, np.pi / 2, 1.00),
    ("too far back", 0.436, -2.094, 0.85),
]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "posesheet.png"
    w, h = 640, 360
    model, hand = held_model(ROW["rel"]), right_hand()
    hv = hand_view(hand)
    cols = 2
    rows = (len(CANDIDATES) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * w + (cols + 1) * 8, rows * h + (rows + 1) * 8), (16, 17, 19))
    for i, (tag, lay, yaw, scale) in enumerate(CANDIDATES):
        _, lean, elev = haft_angles(lay, yaw)
        sc = clock(model, ROW["height_m"], ROW["grip_frac"], lay, yaw, scale)
        lab = (f"{tag}\n  lay {np.degrees(lay):.0f}  yaw {np.degrees(yaw):.0f}  x{scale:.2f}\n"
               f"  screen {sc:.0f} ({oclock(sc)} o'clock)  "
               f"lean {lean:+.0f}  elev {elev:.0f}")
        iv = item_view(model, ROW["height_m"], ROW["grip_frac"], lay, yaw, scale)
        sheet.paste(panel(hv, iv, lab, w, h), (8 + (i % cols) * (w + 8), 8 + (i // cols) * (h + 8)))
        print(f"  {tag:14s} lay {np.degrees(lay):5.1f}  yaw {np.degrees(yaw):7.1f}  "
              f"scale {scale:.2f}   screen {sc:6.1f} ({oclock(sc):2d} o'clock)"
              f"  lean {lean:+6.1f}  elev {elev:5.1f}")
    sheet.save(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
