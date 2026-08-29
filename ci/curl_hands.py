#!/usr/bin/env python3
"""Bend a rigged character's fingers out of their bind-pose splay, in the mesh.

**Why this exists.** `assets/models/stumpy.glb` arrived from the generator with
five separately modelled digits per hand -- 1,048 vertices in the right hand
alone -- and a 24-joint skeleton in which `RightHand` is a LEAF. The fingers
were modelled and never rigged, so the hand is frozen in the flat, spread pose
a rigger authors to bind in. That pose is correct as a BIND pose and wrong as a
rest pose: a relaxed hand curls, and this one is a rake in every frame of the
game, first person and third, holding something or not.

A joint would be the general fix and this is not it (`NOW.md` 0chr). This
moves the vertices instead, which is the whole of the fix for a hand nothing
can pose anyway, and it costs no bone, no weight and no runtime.

**Why it is safe to move them.** Every vertex this touches carries more than
half its skin weight on the hand bone, which is a leaf, so its skinned position
is `HandWorld * IBM_hand * v` and nothing else in the character depends on
where `v` sits. Moving it is exactly equivalent to the artist having modelled
the hand that way. That is NOT true of the prop importer's trick of baking a
correction into the vertices (`ci/import_char.py`'s header argues it at
length): a correction on the whole mesh moves the bind pose out from under
every inverse bind matrix. The difference is the scope -- one leaf bone's
exclusive vertices, versus all of them.

**Both halves get it for free.** `ci/split_arms.py` leaves `char1_arms` and
`char1_body` sharing one POSITION accessor and one NORMAL accessor, differing
only in their index array, so a vertex moved here is moved in the first-person
viewmodel and on every remote body in the world at the same time. There is no
version of this that fixes one and not the other, which is the point.

## What it derives rather than takes

Nothing about the hand is typed in. Per hand, from the geometry:

  * **The frame.** In the hand joint's own space the digits run up +Y (the
    joint origin is the wrist). Cutting across and taking connected components
    of the WELDED topology returns five islands -- the mesh is duplicated along
    every UV seam, so components have to be taken on position, not on index,
    or one seam reports a finger as three separate shells.
  * **Where each digit BEGINS, per digit.** One shared cut plane does not work
    and the failure is not subtle. A thumb starts near the wrist and the
    fingers start at the webbing, so a plane placed for the fingers catches
    1.5 units of thumb tip, the derived axis is a stub, and every thumb vertex
    below it sits past the end of a 1.5-unit digit. The first cut of this file
    extruded the thumb into a spike for exactly that reason. So the cut is
    SWEPT instead: a digit's base is the lowest plane at which its island is
    still topologically separate from the others, which is a definition of a
    knuckle rather than a guess at one. On this character the fingers separate
    around 0.50 of the hand and the thumb at 0.22.
  * **Which one is the thumb.** The island with the largest offset along the
    axis the other four do not spread on. On this character that is 9.9 units
    against a 5.2 spread, so it is not a close call.
  * **Which way is the palm.** The thumb opposes the fingers, so the sign of
    its offset along the palm-normal axis IS the direction the fingers close.
    Derived per hand, which is what makes the left hand a mirror of the right
    with no second constant and no `if left:` anywhere in this file.

The one thing that is a knob is HOW FAR, and it carries its documented default
below.

## The bend

Constant curvature about an axis through the knuckle, applied by ARCLENGTH
from that knuckle, so the angle is zero at the base and grows along the digit.
That is what keeps the knuckle from creasing: a single pivot rotation puts a
step discontinuity at the boundary between the vertices it moves and the ones
it does not, and the crease is visible from any angle. Here the digit's own
lowest vertices sit AT its base, so the angle there is zero by construction
and the palm it joins is untouched.

Normals are rotated by the same per-vertex rotation. Forgetting that is the
quiet version of this bug: the silhouette curls and the shading stays flat, and
it reads as a lighting fault rather than a geometry one.

Usage:
  ci/curl_hands.py <in.glb> <out.glb> [--degrees 85] [--thumb-degrees 35]
                   [--adduct 0.45] [--bones RightHand,LeftHand]
"""
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from split_arms import read_accessor, read_glb, write_glb  # noqa: E402

# The hands to bend. Both, because the shared vertex buffer means the world
# body shows the left one even though the viewmodel collapses it.
HAND_BONES = ["RightHand", "LeftHand"]

# How far a finger closes, degrees over its whole length, and it is a RELAXED
# hand rather than a fist. A hand at rest sits near 25 deg at the knuckle,
# 45 at the middle joint and 15 at the tip; spread over one constant-curvature
# bend that is 85. A fist is roughly double and is what a grip
# pose would want -- which is the argument, not this default, and it wants a
# second baked pose rather than a bigger number here.
CURL_DEG = 85.0
# The thumb closes less and it is the digit most likely to punch through the
# palm if it does not, because it starts pointing across the others.
THUMB_DEG = 35.0
# How much of the bind pose's SPREAD to take out, as a fraction. The splay is
# the half that reads at distance -- a hand is four pixels wide across a
# street, and the difference between a rake and a mitten survives that where
# any amount of curl does not. Not 1.0: the fingers are cylinders with no
# collision between them and closing the gap entirely interpenetrates them.
ADDUCT = 0.45
# The band of the hand's own length the base sweep searches, and its step.
# The top is above every knuckle on a human hand and the bottom is at the
# wrist; a digit that never separates inside this band is a segmentation
# failure and says so rather than being bent on a guessed axis.
SWEEP_HI, SWEEP_LO, SWEEP_STEP = 0.80, 0.12, 0.01
# The smallest island that counts as a digit. Below this it is a seam artefact
# or a fingernail, and taking it as a digit puts a bend axis through nothing.
MIN_ISLAND = 10


def rotate(v, axis, ang):
    """Rodrigues, vectorised over v with a per-row angle."""
    axis = axis / np.linalg.norm(axis)
    c = np.cos(ang)[:, None]
    s = np.sin(ang)[:, None]
    cross = np.cross(np.broadcast_to(axis, v.shape), v)
    dot = (v @ axis)[:, None]
    return v * c + cross * s + np.broadcast_to(axis, v.shape) * dot * (1.0 - c)


def welded_adjacency(local, hand_ix, tris):
    """Vertex adjacency over the hand, welded on POSITION.

    The mesh duplicates vertices along every UV seam, so index adjacency
    reports one finger as three or four separate shells and any segmentation
    built on it silently returns twenty islands instead of five.
    """
    hand = set(int(k) for k in hand_ix)
    weld = {int(k): tuple(np.round(local[k], 3)) for k in hand_ix}
    adj = {}
    for t in tris:
        a, b, c = int(t[0]), int(t[1]), int(t[2])
        if a in hand and b in hand and c in hand:
            for u, v in ((a, b), (b, c), (c, a)):
                adj.setdefault(weld[u], set()).add(weld[v])
                adj.setdefault(weld[v], set()).add(weld[u])
    return weld, adj


def islands_above(weld, adj, hand_ix, y, cut):
    """Connected components of the hand above `cut`, as sets of welded keys."""
    live = {weld[int(k)] for k in hand_ix if y[k] >= cut}
    seen, out = set(), []
    for start in live:
        if start in seen:
            continue
        stack, comp = [start], set()
        seen.add(start)
        while stack:
            u = stack.pop()
            comp.add(u)
            for w in adj.get(u, ()):
                if w in live and w not in seen:
                    seen.add(w)
                    stack.append(w)
        if len(comp) >= MIN_ISLAND:
            out.append(comp)
    return out


def digits(local, hand_ix, tris):
    """Each digit's own vertices, found by sweeping its base plane down.

    Returns a list of (vertex indices, base fraction), longest digit first.

    The sweep is the whole point — see the module docstring. Starting high,
    where every digit is separate, each one is followed downward until its
    island absorbs a second digit's seed; the plane before that merge is its
    knuckle. A thumb and an index finger therefore get different bases out of
    one pass, with nothing per-digit typed in.
    """
    y = local[:, 1]
    lo, hi = y[hand_ix].min(), y[hand_ix].max()
    weld, adj = welded_adjacency(local, hand_ix, tris)

    def at(frac):
        return islands_above(weld, adj, hand_ix, y, lo + (hi - lo) * frac)

    marks, best, frozen = [], {}, set()
    frac = SWEEP_HI
    while frac >= SWEEP_LO:
        for comp in at(frac):
            owners = [i for i, m in enumerate(marks) if m in comp]
            if not owners:
                # A digit too short to reach the top of the sweep, arriving as
                # its tip crosses the plane. The pinky and the thumb are both
                # this case on a human hand, so seeding once at the top and
                # only following downward finds three digits and calls it five.
                marks.append(max(comp))
                best[len(marks) - 1] = (frac, comp)
            elif len(owners) == 1:
                if owners[0] not in frozen:
                    best[owners[0]] = (frac, comp)
            else:
                # Two digits in one island: they have merged, and the plane
                # before this one is the knuckle of both.
                frozen.update(owners)
        frac -= SWEEP_STEP

    out = []
    for i in sorted(best, key=lambda k: -len(best[k][1])):
        frac_i, comp = best[i]
        ix = np.array([int(k) for k in hand_ix if weld[int(k)] in comp], dtype=np.int64)
        out.append((ix, frac_i))
    return out


def main():
    argv = sys.argv[1:]
    opts = {"--degrees": CURL_DEG, "--thumb-degrees": THUMB_DEG,
            "--adduct": ADDUCT}
    bones = list(HAND_BONES)
    files = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in opts:
            opts[a] = float(argv[i + 1])
            i += 2
        elif a == "--bones":
            bones = [b.strip() for b in argv[i + 1].split(",")]
            i += 2
        elif a.startswith("--"):
            raise SystemExit(f"unknown flag {a}\n\n{__doc__}")
        else:
            files.append(a)
            i += 1
    if len(files) != 2:
        raise SystemExit(__doc__)
    src, dst = files

    gltf, blob = read_glb(src)
    if not gltf.get("skins"):
        raise SystemExit(f"{src}: not skinned — there is no hand bone to find")
    prims = [m["primitives"][0] for m in gltf["meshes"]]
    pos_ix = {p["attributes"]["POSITION"] for p in prims}
    nrm_ix = {p["attributes"]["NORMAL"] for p in prims}
    if len(pos_ix) != 1 or len(nrm_ix) != 1:
        raise SystemExit(
            f"{src}: the meshes do not share one POSITION/NORMAL accessor "
            f"(positions {sorted(pos_ix)}, normals {sorted(nrm_ix)}). This tool "
            f"edits the shared buffer once; separate buffers would leave one "
            f"half curled and the other splayed.")
    pos_ix, nrm_ix = pos_ix.pop(), nrm_ix.pop()

    P = read_accessor(gltf, blob, pos_ix).astype(np.float64).copy()
    N = read_accessor(gltf, blob, nrm_ix).astype(np.float64).copy()
    J = read_accessor(gltf, blob, prims[0]["attributes"]["JOINTS_0"]).astype(np.int64)
    W = read_accessor(gltf, blob, prims[0]["attributes"]["WEIGHTS_0"]).astype(np.float64)
    tris = read_accessor(gltf, blob, prims[0]["indices"]).reshape(-1, 3).astype(np.int64)

    skin = gltf["skins"][0]
    names = [gltf["nodes"][j].get("name") for j in skin["joints"]]
    missing = [b for b in bones if b not in names]
    if missing:
        raise SystemExit(f"{src}: the skin has no bone(s) {missing}. Bones: {names}")
    IBM = (read_accessor(gltf, blob, skin["inverseBindMatrices"])
           .astype(np.float64).reshape(-1, 4, 4).transpose(0, 2, 1))

    moved_total = 0
    for bone in bones:
        ji = names.index(bone)
        M = IBM[ji]
        R, T = M[:3, :3], M[:3, 3]
        Rinv = np.linalg.inv(R)
        # A rotation times a uniform scale, and asserted rather than assumed:
        # the bend happens in bone space and comes back through `Rinv`, which
        # only preserves a normal's DIRECTION when the linear part is a
        # similarity. Under shear or a non-uniform scale the positions would
        # still be right and every normal would be quietly wrong.
        gram = R @ R.T
        k2 = np.trace(gram) / 3.0
        skew = np.abs(gram - k2 * np.eye(3)).max() / k2
        if skew > 1e-4:
            raise SystemExit(
                f"{src}: {bone}'s inverse bind matrix is not a rotation times "
                f"a uniform scale (off by {skew:.2e}). Bending through it "
                f"would shear the normals.")
        own = ((J == ji) & (W > 0.5)).any(axis=1)
        own_ix = np.nonzero(own)[0]
        if len(own_ix) == 0:
            raise SystemExit(f"{src}: no vertex is dominated by {bone}")
        local = (P @ R.T) + T

        found = digits(local, own_ix, tris)
        if len(found) != 5:
            raise SystemExit(
                f"{src}: {bone} segments into {len(found)} digit(s), not 5 "
                f"(sizes {[len(x[0]) for x in found]}). Either the sweep band "
                f"{SWEEP_LO}..{SWEEP_HI} is wrong for this hand or this is not "
                f"a five-digit hand — bending it blind would fold whatever it "
                f"did find.")

        cent = np.array([local[ix].mean(axis=0) for ix, _ in found])
        # The axis the four fingers spread on is the one with the LARGER spread
        # once the outlier is out; the thumb is the outlier on the other. Try
        # both candidate axes and take the reading that separates one digit
        # furthest from the rest — on this character it is 9.9 against 5.2.
        pick = None
        for spread_ax, palm_ax in ((2, 0), (0, 2)):
            order = np.argsort(cent[:, palm_ax])
            for cand in (order[0], order[-1]):
                rest = [k for k in range(5) if k != cand]
                gap = abs(cent[cand, palm_ax] - cent[rest, palm_ax].mean())
                if pick is None or gap > pick[0]:
                    pick = (gap, int(cand), spread_ax, palm_ax)
        _, thumb, spread_ax, palm_ax = pick
        rest = [k for k in range(5) if k != thumb]
        palm_sign = np.sign(cent[thumb, palm_ax] - cent[rest, palm_ax].mean())
        palm_n = np.zeros(3)
        palm_n[palm_ax] = palm_sign
        # The finger the others close toward: the middle of the four on the
        # spread axis, which is also the longest, so an adduction leaves it
        # where it is and swings the other three onto it.
        mid = np.median(cent[rest, spread_ax])

        moved = np.zeros(len(P), dtype=bool)
        bases = []
        for k, (sel, base_frac) in enumerate(found):
            v = local[sel]
            band = 0.2 * np.ptp(v[:, 1])
            base = v[v[:, 1] <= v[:, 1].min() + band].mean(axis=0)
            tipc = v[v[:, 1] >= v[:, 1].max() - band].mean(axis=0)
            dirv = tipc - base
            dirv /= np.linalg.norm(dirv)
            r = local[sel] - base
            s = r @ dirv
            live = s > 0.0
            if not live.any():
                continue
            sel, r, s = sel[live], r[live], s[live]
            # The digit's own reach, not the axis sample's: `s` runs over
            # exactly the vertices about to move, so no vertex can end up past
            # the end of the bend and be flung down the tangent. That was this
            # file's first bug and it turned the thumb into a spike.
            length = s.max()
            deg = opts["--thumb-degrees"] if k == thumb else opts["--degrees"]
            ang = np.radians(deg) * (s / length)

            n = palm_n - dirv * (palm_n @ dirv)
            n /= np.linalg.norm(n)
            axis = np.cross(dirv, n)
            axis /= np.linalg.norm(axis)
            kcurv = np.radians(deg) / length
            perp = r - np.outer(s, dirv)
            arc = (np.outer(np.sin(ang) / kcurv, dirv)
                   + np.outer((1.0 - np.cos(ang)) / kcurv, n))
            new_local = base + arc + rotate(perp, axis, ang)

            swing = None
            if k != thumb and opts["--adduct"] > 0.0:
                # Close the spread: swing the digit about the palm normal,
                # through its own base, toward the middle finger.
                off = base[spread_ax] - mid
                swing = np.full(len(sel), -np.arctan2(off, length) * opts["--adduct"])
                new_local = base + rotate(new_local - base, palm_n, swing)

            P[sel] = (new_local - T) @ Rinv.T
            nl = rotate(N[sel] @ R.T, axis, ang)
            if swing is not None:
                nl = rotate(nl, palm_n, swing)
            # Normalised in MESH space, which is where they are stored and the
            # only space the length means anything in. Doing it in bone space
            # and then mapping back divides every normal by the armature's
            # scale — 100 on this rig — and glTF requires unit normals, so the
            # whole hand shades as if unlit with no error anywhere.
            nm = nl @ Rinv.T
            nm /= np.linalg.norm(nm, axis=1, keepdims=True)
            N[sel] = nm
            moved[sel] = True
            moved_total += int(len(sel))
            bases.append(f"{base_frac:.2f}")

        print(f"  {bone}: {len(own_ix)} verts, 5 digits, thumb is island "
              f"{thumb}, palm is {'+' if palm_sign > 0 else '-'}"
              f"{'xyz'[palm_ax]}, bases at {' '.join(bases)} of the hand, "
              f"{int(moved.sum())} vertices bent")

    if moved_total == 0:
        raise SystemExit(f"{src}: nothing moved — refusing to write a copy")

    def overwrite(ix, data):
        bv = gltf["bufferViews"][gltf["accessors"][ix]["bufferView"]]
        start = bv.get("byteOffset", 0) + gltf["accessors"][ix].get("byteOffset", 0)
        raw = np.ascontiguousarray(data, dtype="<f4").tobytes()
        if len(raw) != bv["byteLength"]:
            raise SystemExit("the rewrite changed the buffer's length")
        blob[start:start + len(raw)] = raw

    overwrite(pos_ix, P)
    overwrite(nrm_ix, N)
    # POSITION carries min/max by spec and a stale one is a wrong bounding box
    # everywhere downstream — including the frustum cull that already cost this
    # viewmodel a capture.
    gltf["accessors"][pos_ix]["min"] = [float(x) for x in P.min(axis=0)]
    gltf["accessors"][pos_ix]["max"] = [float(x) for x in P.max(axis=0)]
    gltf["buffers"][0]["byteLength"] = len(blob)
    write_glb(dst, gltf, blob)

    was, now = Path(src).stat().st_size, Path(dst).stat().st_size
    print(f"  {Path(dst).name:24s} {was/1e6:5.1f} -> {now/1e6:5.1f} MB "
          f"({moved_total} vertices moved, no accessor added)")


if __name__ == "__main__":
    main()
