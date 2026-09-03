#!/usr/bin/env python3
"""Stand a held model up on the axis the hand grips it by.

`ci/import_meshy.py` normalises a generated mesh in three ways and stops one
short of the one the viewmodel actually rests on. It scales uniformly, it
strips the emissive, and it centres **the bounding box** on X/Z with the feet
on y = 0 -- and `crates/client/src/ui/hold.rs` then spends a single number
(`grip_frac` x `height_m`) UP THE MODEL'S +Y to say where the fist goes.

Those two agree only when the thing the hand holds is itself on +Y. On a
spear, a hammer and a pickaxe it is, so the gap was invisible for as long as
nobody held anything else. On `stone_hatchet.glb` it is not: the haft is
authored leaning 30 degrees, and the head hangs far enough off one side that
centring the BOX put the haft **121 mm** from the axis at the grip height.
`viewmodel::pose` did exactly what it says -- it slid the model until the
point `(0, grip_m, 0)` sat in the palm -- and that point is thin air three
palm-widths from the haft, so the axe hung beside the fist and pointed 30
degrees across the frame. `hunting_bow.glb` is the same defect at 165 mm.

So this is the missing step, and it is a rigid motion: nothing is stretched
and nothing is resampled.

  1. Fit the grip axis over a window of the model's height -- the part the
     hand closes on, named on the command line because which part that is is
     a judgement about the object, not arithmetic. The fit itself is
     arithmetic: the first principal axis of the vertices in the window.
  2. Rotate the whole mesh by the MINIMAL rotation taking that axis to +Y, so
     no spin about the haft is introduced that nobody asked for.
  3. Slide X/Z so the material AT THE GRIP HEIGHT is on the +Y axis, and drop
     the feet back to y = 0.

Step 3 uses the **median** of the vertices in a fist-high band around the grip
rather than the fitted line, and the difference is not a refinement -- it is
the bow. A bow's string is modelled, it runs parallel to the limbs, and a fit
over the whole mesh lands the axis in the air BETWEEN the two: `hunting_bow`
straightened by its own centroid put the riser 150 mm off the fist while
measuring a 0.3 mm "offset", which is a gate passing over a defect it cannot
see. A median is blind to the string (it is a few hundred vertices against
several thousand) and it answers the question the hand is asking, which is
where the material is, not where the average is.

Normals are rotated with the positions; the rotation is orthonormal, so they
stay unit and stay correct. UVs, indices, materials and textures are
untouched, which is why this is safe to run on a shipped asset.

**It is idempotent, and that is the check worth running.** A model that is
already on its axis measures a 0.00 degree tilt and a 0 mm offset, and the
transform is the identity. `crates/client/tests/held_assets.rs` re-measures
the RESULT against the fist rather than trusting this script.

The +Y extent changes when the tilt does -- a rotated box is a different box
-- so `height_m` in `ui::hold::HELD_MODELS` must be updated to whatever this
prints, and its own gate says so if you forget.

Usage:
  ci/stand_grip.py <in.glb> <out.glb> --shaft LO HI --grip FRAC
  ci/stand_grip.py <in.glb> --report [--shaft LO HI] [--grip FRAC]

`--shaft LO HI` are fractions of the model's height bounding the part the hand
closes on; `--grip FRAC` is `HELD_MODELS[..].grip_frac`, the height the fist
sits at, as a fraction of the model's height AFTER standing it up.
"""
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# One GLB reader and one writer in this directory, deliberately: a second copy
# of the chunk layout is the hand-kept mirror CLAUDE.md warns about. `positions`
# checks for float32 VEC3, which NORMAL is too.
from import_meshy import positions, read_glb, write_glb  # noqa: E402


def accessors(gltf, attr):
    return [p["attributes"][attr]
            for m in gltf["meshes"] for p in m["primitives"]
            if attr in p["attributes"]]


def all_positions(gltf, blob):
    return [positions(gltf, blob, i) for i in accessors(gltf, "POSITION")]


def fit_axis(verts, lo, hi):
    """Direction and centroid of the grip axis over `lo`..`hi` of the height."""
    y0, y1 = verts[:, 1].min(), verts[:, 1].max()
    a, b = y0 + (y1 - y0) * lo, y0 + (y1 - y0) * hi
    win = verts[(verts[:, 1] >= a) & (verts[:, 1] <= b)]
    if len(win) < 16:
        raise SystemExit(f"only {len(win)} vertices in {lo}..{hi} of the height")
    c = win.mean(0)
    _, sv, vt = np.linalg.svd(win - c, full_matrices=False)
    d = vt[0] * np.sign(vt[0][1] or 1.0)
    if sv[0] < 1.5 * sv[1]:
        raise SystemExit(
            f"the window {lo}..{hi} is not shaft-shaped (singular values "
            f"{sv.round(3)}) -- it holds no direction to stand up. Narrow it "
            f"to the part the hand actually closes on.")
    return d, c, len(win)


def align_to_y(d):
    """The minimal rotation taking unit `d` to +Y, as a 3x3 matrix."""
    y = np.array([0.0, 1.0, 0.0])
    v = np.cross(d, y)
    s, c = np.linalg.norm(v), float(np.dot(d, y))
    if s < 1e-12:
        return np.eye(3) if c > 0 else -np.eye(3)
    k = np.array([[0, -v[2], v[1]], [v[2], 0, -v[0]], [-v[1], v[0], 0]])
    return np.eye(3) + k + k @ k * ((1 - c) / s ** 2)


# Half the height a closed fist occupies on the thing it is holding, metres --
# `held_assets.rs::FIST_HALF_M`, restated because the band this slides on is
# the band that gate measures.
FIST_HALF_M = 0.045


def opt(name, n):
    """`n` floats after `--name`, or None. Consumed so they are not filenames."""
    if name not in sys.argv:
        return None
    i = sys.argv.index(name)
    return [float(v) for v in sys.argv[i + 1:i + 1 + n]]


def main():
    report = "--report" in sys.argv
    taken = {v for name, n in (("--shaft", 2), ("--grip", 1))
             if name in sys.argv
             for v in sys.argv[sys.argv.index(name) + 1:
                               sys.argv.index(name) + 1 + n]}
    args = [a for a in sys.argv[1:] if not a.startswith("--") and a not in taken]
    lo, hi = opt("--shaft", 2) or (0.02, 0.98)
    grip = (opt("--grip", 1) or [0.5])[0]
    if len(args) != (1 if report else 2):
        raise SystemExit(__doc__)

    gltf, blob = read_glb(args[0])
    views = all_positions(gltf, blob)
    verts = np.concatenate(views)
    d, centre, n = fit_axis(verts, lo, hi)
    tilt = float(np.degrees(np.arccos(np.clip(d[1], -1, 1))))
    before = np.hypot(centre[0], centre[2])
    print(f"  {args[0].rsplit('/', 1)[-1]}: {len(verts)} verts, {n} in the "
          f"{lo:.2f}..{hi:.2f} window")
    print(f"    grip axis {d.round(4)} -> tilt {tilt:.2f} deg off +Y, "
          f"axis offset {before * 1000:.1f} mm")
    print(f"    +Y extent {verts[:, 1].max() - verts[:, 1].min():.4f} m")
    if report:
        return

    r = align_to_y(d)
    for v in views:
        v[:] = v @ r.T
    for i in accessors(gltf, "NORMAL"):
        nrm = positions(gltf, blob, i)
        nrm[:] = nrm @ r.T

    # Feet down first, because the grip height is a fraction measured from
    # them and the rotation has just moved them.
    verts = np.concatenate(views)
    drop = verts[:, 1].min()
    for v in views:
        v[:, 1] -= drop

    verts = np.concatenate(views)
    gy = verts[:, 1].max() * grip
    band = verts[(np.abs(verts[:, 1] - gy) <= FIST_HALF_M)]
    if len(band) < 16:
        raise SystemExit(
            f"only {len(band)} vertices within {FIST_HALF_M} m of the grip "
            f"height {gy:.3f} -- there is nothing there to put in a fist")
    fist = np.median(band, axis=0)
    for v in views:
        v[:, 0] -= fist[0]
        v[:, 2] -= fist[2]
    for i in accessors(gltf, "POSITION"):
        v = positions(gltf, blob, i)
        gltf["accessors"][i]["min"] = v.min(0).astype(float).tolist()
        gltf["accessors"][i]["max"] = v.max(0).astype(float).tolist()

    verts = np.concatenate(all_positions(gltf, blob))
    d2, _, _ = fit_axis(verts, lo, hi)
    write_glb(args[1], gltf, blob)
    size = verts.max(0) - verts.min(0)
    gy = verts[:, 1].max() * grip
    band = verts[np.abs(verts[:, 1] - gy) <= FIST_HALF_M]
    m = np.median(band, axis=0)
    print(f"    -> tilt {np.degrees(np.arccos(np.clip(d2[1], -1, 1))):.2f} deg, "
          f"the fist at {grip:.2f} sits on material spanning "
          f"x [{band[:, 0].min():+.3f}, {band[:, 0].max():+.3f}] "
          f"z [{band[:, 2].min():+.3f}, {band[:, 2].max():+.3f}], "
          f"median {np.hypot(m[0], m[2]) * 1000:.1f} mm off")
    print(f"    -> {size[0]:.3f} x {size[1]:.3f} x {size[2]:.3f} m  "
          f"(height_m = {size[1]:.3f})")


if __name__ == "__main__":
    main()
