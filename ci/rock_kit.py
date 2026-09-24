#!/usr/bin/env python3
"""The Blender stage of the prop pipeline: generate a rock kit piece, or edit
a generated delivery, headless, into the same GLB the rest of the pipeline
already knows how to measure, import and pack.

**Why a fourth tool beside `meshy_gen.py`.** A generator is a sampler
(`measure_glb.py`'s header: eleven rolls, six keepers), and that is the right
trade for a boulder or an ore node — one opaque hull, rolled and selected.
Three rock tiers `reference/ROCKS.md` §9 asks for are not that shape: a
cliff slab must abut its neighbours, a formation must fit a box list the sim
publishes with ledges a body can stand on, and both are scaled per instance.
You cannot prompt a sampler into a tiling seam or a collision volume. A
seeded script can be pointed at one, and re-run when the register changes
(`WORLD.md` §9.1: art for the wrong register is remade).

Three commands, one output contract:

    ci/rock_kit.py gen  --occupant Rock --kind boulder --seed 3 --out x.glb
    ci/rock_kit.py edit meshy.glb --out x.glb --decimate 2400 --rebake
    ci/rock_kit.py preview x.glb

`gen` builds a high-poly rock from seeded noise inside the volume the sim
blocks (read out of `sim-core` exactly as `measure_glb.py` reads it — one
law, never a typed size), decimates a copy to the row's triangle ceiling,
unwraps it, bakes normal / albedo / roughness / AO from the high-poly onto
it, and exports a GLB carrying albedo + normal + **ORM-packed** maps, which is
`assets/models/WANTED.md` §0's contract to the letter. `edit` does the back
half of that to a delivery from `meshy_gen.py`: decimate, voxel-remesh, cut
the buried bottom off, re-bake onto fresh UVs, and re-fit to the row. Both
write a `<out>.json` sidecar with the seed, every derived parameter, the
Blender version and the script's own hash, so a `MANIFEST.md` row is
transcribed rather than remembered — `meshy_gen.py`'s sidecar, for a
generator whose "prompt" is a number.

**Every output is measured by the triage before it is reported.** After the
export the script reopens the GLB with `measure_glb.py`'s own functions and
prints that script's verdict for the row, so a piece this tool makes faces
the same bands a purchase does. A reject is printed, not hidden, and is not
fatal: rejects are the expected output of a selection step.

**Blender runs here as a Python module** (`pip install bpy==4.5.0`, Python
3.11, ~370 MB), no GUI, no socket, Cycles on the CPU. `--self-test` proves the
bpy-free arithmetic (the target read, the fit, the ORM packing, the sidecar)
and runs on every box; a command that needs bpy on a box without it says
SKIP and exits 2 — the loud-skip rule, never a pass it did not earn.

Coordinates: Blender is Z-up and glTF is Y-up. Everything geometric below is
done in Blender space (height is z, plan radius is hypot(x, y)) and the
exporter's Y-up conversion turns it into the frame `import_meshy.py` and
`props.rs` expect. The sim's `(radius, top)` maps onto that as radius →
hypot(x, y) and `2 × (top − lift)` → the z extent, which is `measure_glb.py`'s
own target box.
"""
import argparse
import datetime as _dt
import hashlib
import json
import math
import os
import random
import sys

import numpy as np

CI = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, CI)
import measure_glb as mg  # noqa: E402  (the triage: targets, verdicts)

ROOT = os.path.dirname(CI)
KINDS = ("boulder", "formation", "slab", "small", "node")
# What the surface is made of. `granite` is the first cut's material and
# still the default for every non-node row, so a documented seed rebuilds the
# rock it was measured as; `formation` is the dark rock on the node looks'
# shared machinery (cracks, curvature, two scales of value) and is opt-in;
# the other three are the ore nodes' identities (`ART.md` rule 8: "the pale,
# round, half-buried lump with the glint").
LOOKS = ("granite", "stone", "metal", "sulfur", "formation")
BLENDER_PIN = "4.5"


# ── bpy-free arithmetic (self-tested, shared by gen and edit) ─────────────


def target_box(occupant, size):
    """Full extents (W, H, D) the piece has to fill, metres, and where from.

    `--occupant` reads `terrain::occupant_volume` and `props::archetype_lift`
    through `measure_glb.py` so this tool and the triage cannot disagree about
    what a row is. `--size` is for a tier the sim does not publish yet
    (`reference/ROCKS.md` §9.3's cliff slab): typed, and said to be typed.
    """
    if occupant:
        r, top = mg.sim_volume(occupant)
        lift = mg.sim_lift(occupant)
        return np.array([2 * r, 2 * (top - lift), 2 * r]), f"sim-core: {occupant} r={r} top={top} lift={lift}"
    if size:
        return np.array(size, dtype=float), "--size (typed; no sim row)"
    raise SystemExit("need --occupant or --size")


def fit_scale(verts, target, mode):
    """Per-axis scale factors that put `verts` (Blender space, z up) into
    `target` = (W, H, D).

    `radius`: one shared x/y factor solved from the largest per-vertex
    hypot(x, y) about the plan centre — `import_meshy.py --fit-radius`, the
    mode for a row the sim blocks as a cylinder — and z from the height.
    `axes`: each axis to its own extent — `--fit-axes`, for a box row.
    """
    lo, hi = verts.min(axis=0), verts.max(axis=0)
    cx, cy = (lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2
    height = max(hi[2] - lo[2], 1e-9)
    if mode == "radius":
        rmax = max(float(np.hypot(verts[:, 0] - cx, verts[:, 1] - cy).max()), 1e-9)
        k = (target[0] / 2) / rmax
        return np.array([k, k, target[1] / height])
    # Blender space is (x = W, y = D, z = H); the target is (W, H, D).
    w, d = max(hi[0] - lo[0], 1e-9), max(hi[1] - lo[1], 1e-9)
    return np.array([target[0] / w, target[2] / d, target[1] / height])


def origin_shift(verts, origin):
    """Translation that puts the piece's origin where the row expects it.

    `center`: the bounding box's midpoint on every axis — the scatter rows,
    whose `archetype_lift` is a half-height and which are buried by it
    (`import_meshy.py --center`). `base`: midpoint in plan, feet on z = 0 —
    `WANTED.md` §0.1's rule for everything that stands on its base.
    """
    lo, hi = verts.min(axis=0), verts.max(axis=0)
    mid = (lo + hi) / 2
    if origin == "center":
        return -mid
    return np.array([-mid[0], -mid[1], -lo[2]])


def pack_orm(ao, rough, metal=None):
    """The glTF ORM layout from grey maps: R = occlusion, G = roughness,
    B = metallic. A rock is never metal; the ore seams of a metal node are
    the one exception, and they arrive as their own baked mask."""
    ao = np.clip(np.asarray(ao, dtype=np.float64), 0, 1)
    rough = np.clip(np.asarray(rough, dtype=np.float64), 0, 1)
    orm = np.zeros(ao.shape + (3,), dtype=np.float64)
    orm[..., 0], orm[..., 1] = ao, rough
    if metal is not None:
        orm[..., 2] = np.clip(np.asarray(metal, dtype=np.float64), 0, 1)
    return orm


def kind_params(kind, radius, rng):
    """Noise strengths and scales for a kind, every one a multiple of the
    piece's own radius so a 0.5 m stone and a 6 m formation get the same
    shape language at their own size. Seeded, so the sidecar can name them."""
    r = radius
    p = {
        # Few, big, soft lobes own the silhouette (`props.rs::lumps`'s lesson:
        # three octaves of surface detail average out to a sphere, and the
        # first cut of this file was crumpled foil for the same reason).
        "lobes_scale": r * rng.uniform(1.1, 1.5),
        # A node's lobes are the smallest because its PLAN is the thing the
        # triage holds (`PLAN_ROUND_MAX`): a lobe is a bulge in plan too.
        "lobes_strength": r * {"boulder": 0.32, "formation": 0.26, "slab": 0.18, "small": 0.34, "node": 0.12}[kind],
        "crackle_scale": r * rng.uniform(0.55, 0.8),
        # A node's crackle is nearly off: its fractures are the CHIPS below,
        # because voronoi crackle at a boulder's strength is exactly the
        # stacked-blocks read the shipped node was rejected for.
        "crackle_strength": r * {"boulder": 0.075, "formation": 0.11, "slab": 0.09, "small": 0.06, "node": 0.02}[kind],
        "asym_scale": r * rng.uniform(1.8, 2.4),
        "asym_strength": r * {"boulder": 0.20, "formation": 0.16, "slab": 0.12, "small": 0.18, "node": 0.08}[kind],
        # Flat faces: near-coplanar high-poly faces within this angle are
        # dissolved into one, so a rock reads as fractured blocks rather
        # than as noise. Degrees; 0 turns it off — the node keeps its dense
        # high-poly, because its facets are cut, not dissolved, and the
        # curvature its material reads needs the vertices.
        "facet_deg": {"boulder": 7.0, "formation": 10.0, "slab": 9.0, "small": 6.0, "node": 0.0}[kind],
        # A boulder is wider than it is tall because that is how it came to
        # rest; a formation stands; a slab is bedding, stretched along x.
        "squash": {
            # Longer than wide by construction (the radial fit keeps the
            # plan's shape), so a boulder is never a ball in plan.
            "boulder": (1.0, rng.uniform(0.62, 0.82), rng.uniform(0.62, 0.80)),
            "formation": (rng.uniform(0.7, 1.0), rng.uniform(0.7, 1.0), rng.uniform(0.9, 1.25)),
            "slab": (1.0, rng.uniform(0.55, 0.8), rng.uniform(0.6, 0.9)),
            "small": (rng.uniform(0.8, 1.0), rng.uniform(0.8, 1.0), rng.uniform(0.6, 0.85)),
            "node": None,  # drawn below, after every other kind's draws
        }[kind],
        # Strata: how much the bedding direction is compressed before the
        # noise is sampled, so features run long along the bed. 1 = none.
        "strata": {"boulder": 1.0, "formation": 0.7, "slab": 0.3, "small": 1.0, "node": 1.0}[kind],
        "tilt_deg": {"boulder": 0.0, "formation": rng.uniform(-18, 18), "slab": rng.uniform(-25, 25), "small": 0.0, "node": 0.0}[kind],
        "voxel": r / (40.0 if kind == "node" else 28.0),
        "seed_offset": [rng.uniform(-40, 40) for _ in range(3)],
        "cut_bottom": {"boulder": 0.0, "formation": 0.12, "slab": 0.10, "small": 0.0, "node": None}[kind],
    }
    if kind == "node":
        # **Equal in plan, by construction** — the one thing `boulder` and
        # `small` forbid (their first squash term is never their second), and
        # the reason neither could make a node (`NOW.md` §0rk, measured
        # 1.82 and 1.33 against a ceiling of 1.20). Low, because the row is a
        # 1.83 m disc 1.25 m tall; cut well below the equator so the widest
        # ring is the one in the ground — a dome IN the turf, not a ball on
        # it (`ART.md` rule 2). Drawn after every other kind's draws so a seed
        # of any existing kind still builds the same rock it always did.
        p["squash"] = (1.0, 1.0, rng.uniform(0.72, 0.88))
        p["cut_bottom"] = rng.uniform(0.28, 0.36)
        # Chips: planes that shear the cap off a lump, each leaving a flat
        # fractured face with a crisp rim — what "a few shallow fractures and
        # small chipped facets" means as geometry. Normals lean upward
        # (25–85° elevation) because a chip on the flank flattens the PLAN,
        # which is the one thing a node is held to (15° measured 1.206 on the
        # first roll, over the 1.20 ceiling); the depth is a fraction
        # of the lump's own reach in that direction, so a chip is shallow on
        # a low dome and on a tall one alike.
        p["chips"] = [
            [round(math.cos(el) * math.cos(az), 6), round(math.cos(el) * math.sin(az), 6), round(math.sin(el), 6), round(d, 6)]
            for az, el, d in (
                (rng.uniform(0, 2 * math.pi), math.radians(rng.uniform(25, 85)), rng.uniform(0.80, 0.93))
                for _ in range(rng.randint(12, 18))
            )
        ]
    return p


def chip(verts, chips):
    """Shear each chip's cap flat: every vertex past the plane at `d` of the
    lump's reach along `n` is projected onto that plane. Pure numpy, so the
    self-test holds its two promises: a chip ends the reach along its own
    normal at exactly `d` of what it was, and it only ever moves a vertex
    TOWARD the centre (the plane sits on the near side of every vertex it
    moves), so no chip can push the lump out of the volume it is fitted to."""
    v = np.array(verts, dtype=np.float64)
    for c in chips:
        n = np.array(c[:3], dtype=np.float64)
        n /= np.linalg.norm(n)
        proj = v @ n
        plane = c[3] * proj.max()
        over = np.maximum(proj - plane, 0.0)
        v -= over[:, None] * n[None, :]
    return v


def row_luma(occupant, kind):
    """Linear albedo luma the piece is built to, from the triage's own bands:
    the stone node is the pale rock and the formation the dark one, split at
    granite (`ART.md` rule 8)."""
    if occupant in mg.PALE_ROWS:
        return 0.34
    if occupant in mg.DARK_ROWS or kind in ("formation", "slab"):
        return 0.21
    # The sulfur node's crust is a bright yellow over a third of it, so its
    # mean sits above the grey rock's; the triage gives metal and sulfur the
    # general band only, because their identity is colour and glint.
    if occupant == "SulfurNode":
        return 0.30
    return 0.26


def look_of(occupant, kind):
    """The surface a row wears unless `--look` says otherwise: each ore node
    its own identity, everything else the formation's granite."""
    return {"StoneNode": "stone", "MetalNode": "metal", "SulfurNode": "sulfur"}.get(occupant, "granite")


def sidecar(cmd, args, target, where, params, measured, verdict, extra=None):
    """Everything a `MANIFEST.md` row needs, in one file beside the GLB —
    `meshy_gen.py`'s shape, with a seed where the prompt would be."""
    script = open(os.path.abspath(__file__), "rb").read()
    side = {
        "tool": "ci/rock_kit.py",
        "script_sha256": hashlib.sha256(script).hexdigest()[:16],
        "command": cmd,
        "args": {k: v for k, v in vars(args).items() if k not in ("func",)},
        "target_m": [round(float(t), 4) for t in target],
        "target_from": where,
        "params": {k: (round(v, 5) if isinstance(v, float) else v) for k, v in params.items()},
        "measured": measured,
        "verdict": verdict or "KEEP",
        "date": _dt.date.today().isoformat(),
    }
    if extra:
        side.update(extra)
    return side


# ── bpy: the Blender half ──────────────────────────────────────────────────


def need_bpy():
    try:
        import bpy  # noqa: F401
    except ImportError:
        print(f"SKIP: bpy is not installed — pip install bpy=={BLENDER_PIN}.0 (Python 3.11); nothing was generated")
        sys.exit(2)
    import bpy
    v = bpy.app.version_string
    if not v.startswith(BLENDER_PIN):
        print(f"warning: bpy {v}, this script is written against {BLENDER_PIN} — a bake or an export may differ")
    return bpy


def fresh_scene(bpy, seed):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.cycles.seed = seed
    sc.cycles.use_animated_seed = False
    sc.cycles.use_denoising = False
    sc.unit_settings.system = "METRIC"
    sc.unit_settings.scale_length = 1.0
    return sc


def apply_modifiers(bpy, obj):
    with bpy.context.temp_override(object=obj, active_object=obj, selected_objects=[obj]):
        for m in list(obj.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)


def mesh_verts(obj):
    n = len(obj.data.vertices)
    out = np.empty(n * 3, dtype=np.float32)
    obj.data.vertices.foreach_get("co", out)
    return out.reshape(n, 3).astype(np.float64)


def set_verts(obj, verts):
    obj.data.vertices.foreach_set("co", np.asarray(verts, dtype=np.float32).ravel())
    obj.data.update()


def make_texture(bpy, name, kind, scale, seed):
    t = bpy.data.textures.new(name, type=kind)
    t.noise_scale = max(scale, 1e-3)
    if kind == "MUSGRAVE":
        t.musgrave_type = "RIDGED_MULTIFRACTAL"
        t.octaves = 2.0
        t.lacunarity = 2.1
        t.dimension_max = 0.9
        t.noise_intensity = 0.9
        t.noise_basis = "BLENDER_ORIGINAL"
    elif kind == "VORONOI":
        # F2 − F1: the cell edges, which is what a fractured block reads as.
        t.distance_metric = "DISTANCE_SQUARED"
        t.weight_1, t.weight_2, t.weight_3, t.weight_4 = -1.0, 1.0, 0.0, 0.0
        t.noise_intensity = 1.0
    elif kind == "CLOUDS":
        t.noise_depth = 1
        t.noise_basis = "IMPROVED_PERLIN"
    return t


def facet(bpy, obj, degrees, smooth_iters=1):
    """Smooth the remesh's stair-steps away, then dissolve near-coplanar
    faces so the high-poly carries flat facets with crisp edges — the read
    of a fractured block, which the normal bake then carries to the low-poly."""
    m = obj.modifiers.new("smooth", "SMOOTH")
    m.factor = 0.5
    m.iterations = smooth_iters
    apply_modifiers(bpy, obj)
    if degrees <= 0:
        return
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.dissolve_limited(angle_limit=math.radians(degrees), use_dissolve_boundaries=False)
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.quads_convert_to_tris(quad_method="BEAUTY", ngon_method="BEAUTY")
    bpy.ops.object.mode_set(mode="OBJECT")


def base_mesh(bpy, kind, rng, p):
    """The un-noised massing, radius ~1, in Blender space."""
    parts = []
    if kind in ("boulder", "small", "node"):
        # A node's chips are planes cut through this mesh, so their rims
        # follow its edges: one more subdivision halves the rim's facets.
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=5 if kind == "node" else 4, radius=1.0)
        parts.append(bpy.context.active_object)
    elif kind == "formation":
        # Two or three overlapping tilted blocks and a lobe; the voxel remesh
        # unions them into one mass — a formation is a pile, not a lump.
        n = rng.randint(2, 3)
        for i in range(n):
            bpy.ops.mesh.primitive_cube_add(size=1.0)
            o = bpy.context.active_object
            o.scale = (rng.uniform(0.7, 1.3), rng.uniform(0.6, 1.1), rng.uniform(0.9, 1.6))
            o.rotation_euler = (math.radians(rng.uniform(-12, 12)), math.radians(rng.uniform(-12, 12)), math.radians(rng.uniform(0, 180)))
            o.location = (rng.uniform(-0.5, 0.5) * i, rng.uniform(-0.5, 0.5) * i, rng.uniform(-0.2, 0.3))
            parts.append(o)
        bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=3, radius=0.7)
        o = bpy.context.active_object
        o.location = (rng.uniform(-0.6, 0.6), rng.uniform(-0.6, 0.6), rng.uniform(-0.3, 0.2))
        parts.append(o)
    elif kind == "slab":
        bpy.ops.mesh.primitive_cube_add(size=1.0)
        o = bpy.context.active_object
        o.scale = (1.6, 0.9, 1.2)
        parts.append(o)
        bpy.ops.mesh.primitive_cube_add(size=1.0)
        o = bpy.context.active_object
        o.scale = (1.1, 0.8, 0.9)
        o.location = (rng.uniform(-0.6, 0.6), rng.uniform(-0.3, 0.3), rng.uniform(-0.4, 0.2))
        o.rotation_euler = (0, 0, math.radians(rng.uniform(-20, 20)))
        parts.append(o)
    for o in parts:
        with bpy.context.temp_override(object=o, active_object=o, selected_objects=[o]):
            bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    obj = parts[0]
    if len(parts) > 1:
        with bpy.context.temp_override(object=obj, active_object=obj, selected_objects=parts, selected_editable_objects=parts):
            bpy.ops.object.join()
    obj.name = "rock_hi"
    return obj


def displace(bpy, obj, p, seed):
    """Lobes, crackle, asymmetry — three seeded displacements in LOCAL
    coordinates, sampled at a seeded offset so every seed sees different
    noise, with the bedding axis compressed first so strata run long."""
    v = mesh_verts(obj)
    sq = np.array(p["squash"])
    v *= sq
    if p["tilt_deg"]:
        a = math.radians(p["tilt_deg"])
        c, s = math.cos(a), math.sin(a)
        y, z = v[:, 1].copy(), v[:, 2].copy()
        v[:, 1], v[:, 2] = c * y - s * z, s * y + c * z
    off = np.array(p["seed_offset"])
    strata = np.array([1.0, 1.0, p["strata"]])
    set_verts(obj, v * strata + off)
    for name, kind, scale, strength in (
        ("lobes", "MUSGRAVE", p["lobes_scale"], p["lobes_strength"]),
        ("crackle", "VORONOI", p["crackle_scale"], p["crackle_strength"]),
        ("asym", "CLOUDS", p["asym_scale"], p["asym_strength"]),
    ):
        m = obj.modifiers.new(name, "DISPLACE")
        m.texture = make_texture(bpy, f"{name}_{seed}", kind, scale, seed)
        m.texture_coords = "LOCAL"
        m.direction = "NORMAL"
        m.mid_level = 0.5
        m.strength = strength
    apply_modifiers(bpy, obj)
    v = mesh_verts(obj)
    set_verts(obj, (v - off) / strata)


def remesh(bpy, obj, voxel):
    m = obj.modifiers.new("remesh", "REMESH")
    m.mode = "VOXEL"
    m.voxel_size = voxel
    m.use_smooth_shade = True
    apply_modifiers(bpy, obj)


def cut_bottom(bpy, obj, fraction):
    """Slice the lowest `fraction` of the height off and cap it flat, so a
    base-origin piece sits on the ground instead of rocking on a point."""
    if fraction <= 0:
        return
    import bmesh
    v = mesh_verts(obj)
    z0 = v[:, 2].min() + fraction * (v[:, 2].max() - v[:, 2].min())
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    geom = bm.verts[:] + bm.edges[:] + bm.faces[:]
    bmesh.ops.bisect_plane(bm, geom=geom, plane_co=(0, 0, z0), plane_no=(0, 0, 1), clear_inner=True, clear_outer=False)
    bmesh.ops.holes_fill(bm, edges=bm.edges[:], sides=0)
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()


def decimate_to(bpy, obj, tris):
    for _ in range(3):
        current = sum(len(f.vertices) - 2 for f in obj.data.polygons)
        if current <= tris:
            break
        m = obj.modifiers.new("dec", "DECIMATE")
        m.decimate_type = "COLLAPSE"
        m.ratio = tris / current * 0.98
        m.use_collapse_triangulate = True
        apply_modifiers(bpy, obj)


def shade_smooth(bpy, obj):
    with bpy.context.temp_override(object=obj, active_object=obj, selected_objects=[obj], selected_editable_objects=[obj]):
        bpy.ops.object.shade_smooth()


def smart_uv(bpy, obj):
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.02, correct_aspect=True)
    bpy.ops.object.mode_set(mode="OBJECT")


def procedural_rock_material(bpy, name, luma, rng, seed):
    """What the high-poly is baked FROM: greys around the row's luma broken
    by two noises, a rust-brown stain in the crevices, cavities darkened by
    the AO node — and no green anywhere (`measure_glb.py`'s green band)."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    nodes, links = nt.nodes, nt.links
    bsdf = nodes["Principled BSDF"]
    coord = nodes.new("ShaderNodeTexCoord")
    mapping = nodes.new("ShaderNodeMapping")
    mapping.inputs["Location"].default_value = (rng.uniform(0, 100), rng.uniform(0, 100), rng.uniform(0, 100))
    links.new(coord.outputs["Object"], mapping.inputs["Vector"])
    grain = nodes.new("ShaderNodeTexNoise")
    grain.inputs["Scale"].default_value = 18.0
    grain.inputs["Detail"].default_value = 6.0
    grain.inputs["Roughness"].default_value = 0.6
    links.new(mapping.outputs["Vector"], grain.inputs["Vector"])
    mottle = nodes.new("ShaderNodeTexNoise")
    mottle.inputs["Scale"].default_value = 2.2
    mottle.inputs["Detail"].default_value = 3.0
    links.new(mapping.outputs["Vector"], mottle.inputs["Vector"])
    # Grey band: luma × [0.72, 1.28], in linear, so the mean lands on the row.
    lo, hi = luma * 0.72, luma * 1.28
    ramp = nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (lo, lo * 0.98, lo * 0.95, 1)
    ramp.color_ramp.elements[1].color = (hi, hi * 0.99, hi * 0.97, 1)
    mix_g = nodes.new("ShaderNodeMix")
    mix_g.data_type = "FLOAT"
    mix_g.inputs["Factor"].default_value = 0.45
    links.new(mottle.outputs["Fac"], mix_g.inputs[2])
    links.new(grain.outputs["Fac"], mix_g.inputs[3])
    links.new(mix_g.outputs[0], ramp.inputs["Fac"])
    # Stain: rust-brown where a third noise is high, a tenth of the surface.
    stain_n = nodes.new("ShaderNodeTexNoise")
    stain_n.inputs["Scale"].default_value = 4.0
    stain_n.inputs["Detail"].default_value = 4.0
    links.new(mapping.outputs["Vector"], stain_n.inputs["Vector"])
    stain_ramp = nodes.new("ShaderNodeValToRGB")
    stain_ramp.color_ramp.elements[0].position = 0.64
    stain_ramp.color_ramp.elements[1].position = 0.74
    links.new(stain_n.outputs["Fac"], stain_ramp.inputs["Fac"])
    mix_s = nodes.new("ShaderNodeMix")
    mix_s.data_type = "RGBA"
    mix_s.inputs["Factor"].default_value = 1.0
    links.new(stain_ramp.outputs["Color"], mix_s.inputs["Factor"])
    links.new(ramp.outputs["Color"], mix_s.inputs[6])
    mix_s.inputs[7].default_value = (luma * 1.1, luma * 0.55, luma * 0.30, 1)
    # Cavities: the AO node darkens what the noise put in a crevice.
    ao = nodes.new("ShaderNodeAmbientOcclusion")
    ao.samples = 8
    ao.inputs["Distance"].default_value = 0.35
    links.new(mix_s.outputs[2], ao.inputs["Color"])
    mix_ao = nodes.new("ShaderNodeMix")
    mix_ao.data_type = "RGBA"
    mix_ao.inputs["Factor"].default_value = 0.35
    links.new(mix_s.outputs[2], mix_ao.inputs[6])
    links.new(ao.outputs["Color"], mix_ao.inputs[7])
    links.new(mix_ao.outputs[2], bsdf.inputs["Base Color"])
    # Roughness 0.72..0.96 by the grain.
    rr = nodes.new("ShaderNodeMapRange")
    rr.inputs["To Min"].default_value = 0.72
    rr.inputs["To Max"].default_value = 0.96
    links.new(grain.outputs["Fac"], rr.inputs["Value"])
    links.new(rr.outputs["Result"], bsdf.inputs["Roughness"])
    bsdf.inputs["Metallic"].default_value = 0.0
    return mat


def _link_or_set(nt, sock, x):
    if isinstance(x, (int, float)):
        sock.default_value = x
    elif isinstance(x, tuple):
        sock.default_value = x if len(x) == 4 else (*x, 1.0)
    else:
        nt.links.new(x, sock)


def _noise(nt, vec, scale, detail=2.0, rough=0.5, distortion=0.0):
    n = nt.nodes.new("ShaderNodeTexNoise")
    n.inputs["Scale"].default_value = scale
    n.inputs["Detail"].default_value = detail
    n.inputs["Roughness"].default_value = rough
    n.inputs["Distortion"].default_value = distortion
    nt.links.new(vec, n.inputs["Vector"])
    return n.outputs["Fac"]


def _voronoi(nt, vec, scale, feature="F1"):
    """(distance, per-cell random value) — the cell value is the R channel of
    Voronoi's Color output, one flat random number per cell. `DISTANCE_TO_EDGE`
    has no Color output, so it returns no cell value."""
    v = nt.nodes.new("ShaderNodeTexVoronoi")
    v.feature = feature
    v.inputs["Scale"].default_value = scale
    nt.links.new(vec, v.inputs["Vector"])
    if feature == "DISTANCE_TO_EDGE":
        return v.outputs["Distance"], None
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    nt.links.new(v.outputs["Color"], sep.inputs["Color"])
    return v.outputs["Distance"], sep.outputs["Red"]


def _band(nt, value, lo, hi):
    """0 below `lo`, 1 above `hi`, smoothstep between — a soft threshold."""
    m = nt.nodes.new("ShaderNodeMapRange")
    m.interpolation_type = "SMOOTHSTEP"
    m.clamp = True
    m.inputs["From Min"].default_value = lo
    m.inputs["From Max"].default_value = hi
    _link_or_set(nt, m.inputs["Value"], value)
    return m.outputs["Result"]


def _range(nt, value, lo, hi):
    """Linear remap of 0..1 onto lo..hi."""
    m = nt.nodes.new("ShaderNodeMapRange")
    m.clamp = True
    m.inputs["To Min"].default_value = lo
    m.inputs["To Max"].default_value = hi
    _link_or_set(nt, m.inputs["Value"], value)
    return m.outputs["Result"]


def _math(nt, op, a, b=None):
    m = nt.nodes.new("ShaderNodeMath")
    m.operation = op
    _link_or_set(nt, m.inputs[0], a)
    if b is not None:
        _link_or_set(nt, m.inputs[1], b)
    return m.outputs["Value"]


def _mix_rgb(nt, fac, a, b):
    m = nt.nodes.new("ShaderNodeMix")
    m.data_type = "RGBA"
    _link_or_set(nt, m.inputs["Factor"], fac)
    _link_or_set(nt, m.inputs[6], a)
    _link_or_set(nt, m.inputs[7], b)
    return m.outputs[2]


def _mul_rgb(nt, a, b):
    m = nt.nodes.new("ShaderNodeMix")
    m.data_type = "RGBA"
    m.blend_type = "MULTIPLY"
    m.inputs["Factor"].default_value = 1.0
    _link_or_set(nt, m.inputs[6], a)
    _link_or_set(nt, m.inputs[7], b)
    return m.outputs[2]


def _mix_f(nt, fac, a, b):
    m = nt.nodes.new("ShaderNodeMix")
    m.data_type = "FLOAT"
    _link_or_set(nt, m.inputs["Factor"], fac)
    _link_or_set(nt, m.inputs[2], a)
    _link_or_set(nt, m.inputs[3], b)
    return m.outputs[0]


def _grey_ramp(nt, fac, lo, hi):
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (*lo, 1)
    ramp.color_ramp.elements[1].color = (*hi, 1)
    nt.links.new(fac, ramp.inputs["Fac"])
    return ramp.outputs["Color"]


def _warp(nt, vec, scale, amount):
    """Domain warp: push the lookup point by a low-frequency noise, so a
    pattern sampled through it has no straight run and no repeat — the
    difference between a crack and a voronoi diagram (`ART.md` rule 7)."""
    n = nt.nodes.new("ShaderNodeTexNoise")
    n.inputs["Scale"].default_value = scale
    n.inputs["Detail"].default_value = 2.0
    nt.links.new(vec, n.inputs["Vector"])
    centred = nt.nodes.new("ShaderNodeVectorMath")
    centred.operation = "SUBTRACT"
    nt.links.new(n.outputs["Color"], centred.inputs[0])
    centred.inputs[1].default_value = (0.5, 0.5, 0.5)
    scaled = nt.nodes.new("ShaderNodeVectorMath")
    scaled.operation = "SCALE"
    nt.links.new(centred.outputs["Vector"], scaled.inputs[0])
    scaled.inputs["Scale"].default_value = amount
    add = nt.nodes.new("ShaderNodeVectorMath")
    add.operation = "ADD"
    nt.links.new(vec, add.inputs[0])
    nt.links.new(scaled.outputs["Vector"], add.inputs[1])
    return add.outputs["Vector"]


def procedural_node_material(bpy, name, luma, rng, look):
    """What an ORE NODE's high-poly is baked from, one identity per look —
    `ART.md` rule 8's "pale, round, half-buried lump with the glint", said
    three ways over one shared rock:

      * **the rock** — two scales of value (rule 1: a 0.3–1 m mottle
        stretched over the ramp's whole range, and a grain under 5 cm), a few
        CRACKS (isolines of a warped noise, kept only where a zone noise
        allows, so a face carries a fracture or two and never a net), and
        the two curvature terms a weathered stone has: convex rims
        worn paler, cavities collecting dark. Curvature is Cycles'
        `Pointiness` off the dense high-poly the node kind keeps.
      * **stone** — pale granite: a sparse salt-and-pepper of 1 cm dark and
        white crystals, faint quartz veins along a noise isoline, a little
        warm stain. The white crystals are glossy, which is the glint a
        granite node has, and nothing is metal.
      * **metal** — a brown-grey rock with two families of ore SEAMS that
        swell and pinch, and grains of ore scattered through the same zone,
        all METALLIC, dark and smooth (the only metal in the kit, baked into
        the ORM's blue channel through the socket this returns), in a
        rust-orange halo.
      * **sulfur** — a cooler grey rock under a crystalline yellow crust
        packed into the cavities and cracks plus a few broad patches, lemon
        to golden per crystal, glossier than the rock. Never green: every
        yellow here has R over G.
      * **formation** — not a node: the DARK rock of rule 8 for a boulder,
        with a wider value ramp, a sparse fleck and broad iron staining, so a
        kit boulder carries the detail the generated pair beside it does.

    Returns `(material, metal_mask_socket_or_None)`. Metallic is left
    UNLINKED on the BSDF on purpose: a DIFFUSE colour bake of a metallic
    surface reads black, and the albedo map has to carry the ore's colour.
    `only_local` AO, so the low-poly sitting inside the high-poly during a
    bake cannot darken it."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    bsdf.inputs["Metallic"].default_value = 0.0
    coord = nt.nodes.new("ShaderNodeTexCoord")
    mapping = nt.nodes.new("ShaderNodeMapping")
    mapping.inputs["Location"].default_value = (rng.uniform(0, 100), rng.uniform(0, 100), rng.uniform(0, 100))
    nt.links.new(coord.outputs["Object"], mapping.inputs["Vector"])
    vec = mapping.outputs["Vector"]
    L = luma

    # ── the shared rock ──
    grain = _noise(nt, vec, 18.0, 6.0, 0.6)
    mottle = _band(nt, _noise(nt, _warp(nt, vec, 0.7, 0.6), 1.6, 3.0), 0.40, 0.60)
    g = _mix_f(nt, 0.3, mottle, grain)
    geo = nt.nodes.new("ShaderNodeNewGeometry")
    # Bands centred on what this mesh actually reads: baked off a node's
    # high-poly, pointiness sits at p5 0.498 / p50 0.506 / p95 0.549, so a
    # textbook 0.44 cavity threshold never fires.
    edge_wear = _band(nt, geo.outputs["Pointiness"], 0.515, 0.56)
    cavity = _math(nt, "SUBTRACT", 1.0, _band(nt, geo.outputs["Pointiness"], 0.47, 0.502))
    ao = nt.nodes.new("ShaderNodeAmbientOcclusion")
    ao.samples = 8
    ao.only_local = True
    ao.inputs["Distance"].default_value = 0.25
    # A crack is an ISOLINE of a warped noise — a long open curve that
    # wanders and ends — and not a voronoi cell edge, which closes into a
    # polygon net: measured as the "cracked egg" read on the first metal
    # roll, where the zone mask could not thin a net into fractures.
    fissure = _math(nt, "ABSOLUTE", _math(nt, "SUBTRACT", _noise(nt, _warp(nt, vec, 1.2, 0.5), 1.4, 2.0), 0.5))
    crack_zone = _band(nt, _noise(nt, vec, 0.9, 2.0), 0.50, 0.60)
    crack = _math(nt, "MULTIPLY", _math(nt, "SUBTRACT", 1.0, _band(nt, fissure, 0.002, 0.007)), crack_zone)
    metal = None

    if look == "stone":
        col = _grey_ramp(nt, g, (L * 0.70, L * 0.69, L * 0.67), (L * 1.28, L * 1.27, L * 1.24))
        _, cell = _voronoi(nt, vec, 90.0)
        dark = _band(nt, cell, 0.925, 0.945)
        light = _math(nt, "SUBTRACT", 1.0, _band(nt, cell, 0.045, 0.065))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", dark, 0.75), col, (L * 0.42, L * 0.42, L * 0.43))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", light, 0.6), col, (L * 1.35, L * 1.35, L * 1.33))
        ridge = _math(nt, "ABSOLUTE", _math(nt, "SUBTRACT", _noise(nt, _warp(nt, vec, 0.8, 0.4), 0.9, 2.0), 0.5))
        vein = _math(nt, "SUBTRACT", 1.0, _band(nt, ridge, 0.005, 0.02))
        vein = _math(nt, "MULTIPLY", vein, _band(nt, _noise(nt, vec, 0.7), 0.42, 0.58))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", vein, 0.5), col, (L * 1.4, L * 1.4, L * 1.38))
        stain = _band(nt, _noise(nt, vec, 1.8, 4.0), 0.60, 0.74)
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", stain, 0.5), col, (L * 1.1, L * 0.82, L * 0.6))
        rough = _mix_f(nt, _math(nt, "MULTIPLY", light, 0.8), _range(nt, grain, 0.70, 0.90), 0.38)
        height = _mix_f(nt, 0.35, grain, mottle)
        bump_strength = 0.2
    elif look == "metal":
        col = _grey_ramp(nt, g, (L * 0.70, L * 0.66, L * 0.62), (L * 1.18, L * 1.12, L * 1.06))
        # Two vein families at two scales, each a thin band on a warped
        # isoline and each kept only inside a zone, so the ore reads as seams
        # running through the rock rather than as flakes lying on it.
        zone = _band(nt, _noise(nt, vec, 0.8, 2.0), 0.40, 0.56)
        # The main family is straighter (less warp) — ore fills a fracture,
        # and a fracture runs — and both swell and pinch: the distance to the
        # isoline is divided by a slow noise, so one threshold draws a seam
        # of changing width instead of a pen line (the first metal rolls).
        swell = _range(nt, _noise(nt, vec, 2.6, 2.0), 0.5, 1.35)
        r1 = _math(nt, "ABSOLUTE", _math(nt, "SUBTRACT", _noise(nt, _warp(nt, vec, 0.9, 0.25), 1.0, 2.0), 0.5))
        r1 = _math(nt, "DIVIDE", r1, swell)
        r2 = _math(nt, "ABSOLUTE", _math(nt, "SUBTRACT", _noise(nt, _warp(nt, vec, 1.7, 0.35), 2.3, 2.0), 0.5))
        r2 = _math(nt, "DIVIDE", r2, swell)
        vein = _math(nt, "MAXIMUM", _math(nt, "SUBTRACT", 1.0, _band(nt, r1, 0.008, 0.02)),
                     _math(nt, "SUBTRACT", 1.0, _band(nt, r2, 0.004, 0.010)))
        vein = _math(nt, "MULTIPLY", vein, zone)
        near = _math(nt, "MULTIPLY", _math(nt, "SUBTRACT", 1.0, _band(nt, r1, 0.02, 0.07)), zone)
        _, cell = _voronoi(nt, _warp(nt, vec, 3.0, 0.2), 16.0)
        # Grains of ore through the whole zone, densest beside a seam: at a
        # player's distance the seams are a few pixels wide and it is the
        # scatter of glints that says "metal".
        nugget = _math(nt, "MULTIPLY", _band(nt, cell, 0.74, 0.78), _math(nt, "MAXIMUM", near, _math(nt, "MULTIPLY", zone, 0.6)))
        metal = _math(nt, "MAXIMUM", vein, nugget)
        # Rust hugs the ore: a halo a few centimetres either side of a seam,
        # and a weak scatter of stain, both a brown-orange iron oxide.
        halo = _math(nt, "MULTIPLY", _math(nt, "SUBTRACT", 1.0, _band(nt, r1, 0.025, 0.09)), zone)
        patch = _band(nt, _noise(nt, vec, 2.4, 3.0), 0.62, 0.70)
        rust = _math(nt, "MAXIMUM", halo, _math(nt, "MULTIPLY", patch, 0.45))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", rust, 0.7), col, (L * 1.35, L * 0.64, L * 0.26))
        # DARK metal: a bright one mirrors the sky and reads as blue paint
        # (measured on the first two rolls); a dark one reads as a seam that
        # flashes where it catches the sun.
        col = _mix_rgb(nt, metal, col, (0.32, 0.31, 0.30))
        rough = _mix_f(nt, metal, _mix_f(nt, rust, _range(nt, grain, 0.78, 0.92), 0.88), 0.32)
        height = _mix_f(nt, 0.5, grain, metal)
        bump_strength = 0.3
    elif look == "sulfur":
        # The crust is the brightest thing in the kit, so the rock under it is
        # lifted and the yellow deepened toward gold: at 2.0× the rock's luma
        # every island that caught more crust than its neighbour read as a
        # patch (chart contrast 0.070 / 0.071 on two rolls); at ~1.6× the
        # yellow still carries and the islands agree.
        col = _grey_ramp(nt, g, (L * 0.72, L * 0.72, L * 0.74), (L * 1.08, L * 1.08, L * 1.10))
        # Its OWN occlusion node: the shared one below takes the finished
        # colour as its input, so reading this mask off it would close a loop
        # in the graph — which Cycles resolves by dropping links, silently.
        # Measured: the first sulfur roll baked 0.48 against a ramp of 0.24.
        ao_mask = nt.nodes.new("ShaderNodeAmbientOcclusion")
        ao_mask.samples = 8
        ao_mask.only_local = True
        ao_mask.inputs["Distance"].default_value = 0.25
        hollow = _math(nt, "MAXIMUM", cavity, _math(nt, "SUBTRACT", 1.0, _band(nt, ao_mask.outputs["AO"], 0.6, 0.9)))
        hollow = _math(nt, "MAXIMUM", hollow, crack)
        # Patches a hand across, not a metre: a metre-wide crust lands on one
        # UV island and not its neighbour, which the triage reads as a
        # patchwork (chart contrast 0.07 on the first roll at scale 1.4).
        patch = _band(nt, _noise(nt, _warp(nt, vec, 0.9, 0.5), 2.4, 3.0), 0.58, 0.64)
        crust = _math(nt, "MAXIMUM", _math(nt, "MULTIPLY", hollow, _band(nt, _noise(nt, vec, 4.0, 2.0), 0.35, 0.5)), patch)
        crystal_d, cell = _voronoi(nt, vec, 32.0)
        yellow = _mix_rgb(nt, cell, (0.76, 0.54, 0.035), (0.60, 0.36, 0.02))
        yellow = _mul_rgb(nt, yellow, _grey_of(nt, _range(nt, crystal_d, 1.1, 0.8)))
        col = _mix_rgb(nt, crust, col, yellow)
        rough = _mix_f(nt, crust, _range(nt, grain, 0.82, 0.94), _range(nt, cell, 0.40, 0.58))
        height = _mix_f(nt, 0.5, _mix_f(nt, 0.4, grain, crystal_d), crust)
        bump_strength = 0.35
    elif look == "formation":
        # The DARK rock of rule 8 on the shared machinery: a wider value
        # ramp than granite's, a sparse dark-and-pale fleck, and iron stain
        # in broad patches — so a kit boulder carries the same two scales of
        # detail as the generated pair it is pooled with.
        col = _grey_ramp(nt, g, (L * 0.66, L * 0.66, L * 0.68), (L * 1.30, L * 1.28, L * 1.26))
        _, cell = _voronoi(nt, vec, 70.0)
        dark = _band(nt, cell, 0.93, 0.95)
        light = _math(nt, "SUBTRACT", 1.0, _band(nt, cell, 0.03, 0.05))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", dark, 0.6), col, (L * 0.45, L * 0.45, L * 0.46))
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", light, 0.5), col, (L * 1.4, L * 1.4, L * 1.38))
        stain = _band(nt, _noise(nt, _warp(nt, vec, 1.0, 0.4), 2.2, 4.0), 0.62, 0.74)
        col = _mix_rgb(nt, _math(nt, "MULTIPLY", stain, 0.55), col, (L * 1.35, L * 0.75, L * 0.42))
        rough = _range(nt, grain, 0.74, 0.94)
        height = _mix_f(nt, 0.35, grain, mottle)
        bump_strength = 0.25
    else:
        raise SystemExit(f"no node material for look {look!r}")

    # Curvature last, so it weathers whatever the look put down: rims paler,
    # cavities and cracks darker — the two reads every stone photo has.
    col = _mix_rgb(nt, _math(nt, "MULTIPLY", edge_wear, 0.45), col, _mul_rgb(nt, col, (1.35, 1.35, 1.35)))
    col = _mix_rgb(nt, _math(nt, "MAXIMUM", _math(nt, "MULTIPLY", cavity, 0.45), _math(nt, "MULTIPLY", crack, 0.7)),
                   col, _mul_rgb(nt, col, (0.45, 0.44, 0.43)))
    nt.links.new(col, ao.inputs["Color"])
    col = _mix_rgb(nt, 0.3, col, ao.outputs["Color"])
    nt.links.new(col, bsdf.inputs["Base Color"])
    nt.links.new(rough, bsdf.inputs["Roughness"])
    height = _math(nt, "SUBTRACT", height, _math(nt, "MULTIPLY", crack, 0.6))
    bump = nt.nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = bump_strength
    bump.inputs["Distance"].default_value = 0.004
    nt.links.new(height, bump.inputs["Height"])
    nt.links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    return mat, metal


def _grey_of(nt, value):
    c = nt.nodes.new("ShaderNodeCombineColor")
    for k in ("Red", "Green", "Blue"):
        nt.links.new(value, c.inputs[k])
    return c.outputs["Color"]


def bake_mask(bpy, sc, hi, lo, img, mat, socket, radius):
    """Bake one scalar of the high-poly's material through an Emission
    shader — how a mask with no bake type of its own (metallic) reaches a
    map. The material's surface link is restored afterwards."""
    nt = mat.node_tree
    out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL")
    was = out.inputs["Surface"].links[0].from_socket
    em = nt.nodes.new("ShaderNodeEmission")
    nt.links.new(_grey_of(nt, socket), em.inputs["Color"])
    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    try:
        bake(bpy, sc, hi, lo, img, "EMIT", 1, radius)
    finally:
        nt.links.new(was, out.inputs["Surface"])
        nt.nodes.remove(em)


def new_image(bpy, name, size, srgb):
    img = bpy.data.images.new(name, size, size, alpha=False, float_buffer=False)
    img.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
    return img


def bake_target_material(bpy, lo, img):
    """A throwaway material on the low-poly whose ACTIVE node is the image
    the bake writes into — Cycles' contract for a selected-to-active bake."""
    mat = bpy.data.materials.new("bake_target")
    mat.use_nodes = True
    node = mat.node_tree.nodes.new("ShaderNodeTexImage")
    node.image = img
    mat.node_tree.nodes.active = node
    lo.data.materials.clear()
    lo.data.materials.append(mat)
    return mat


def bake(bpy, sc, hi, lo, img, kind, samples, radius, pass_filter=None):
    bake_target_material(bpy, lo, img)
    bpy.ops.object.select_all(action="DESELECT")
    hi.select_set(True)
    lo.select_set(True)
    bpy.context.view_layer.objects.active = lo
    sc.cycles.samples = samples
    b = sc.render.bake
    b.use_selected_to_active = True
    b.cage_extrusion = 0.04 * radius
    b.max_ray_distance = 0.6 * radius
    b.margin = 8
    b.use_clear = True
    kw = dict(type=kind, use_selected_to_active=True, cage_extrusion=b.cage_extrusion,
              max_ray_distance=b.max_ray_distance, margin=8, use_clear=True)
    if kind == "NORMAL":
        kw.update(normal_space="TANGENT", normal_r="POS_X", normal_g="POS_Y", normal_b="POS_Z")
    if pass_filter:
        kw["pass_filter"] = pass_filter
    bpy.ops.object.bake(**kw)


def coverage_mask(lo, tex):
    """Which texels the low-poly's UV islands cover, rasterized off its own
    loop triangles with `glbcharts.rasterize` — the same rasterizer the
    triage's chart-contrast reading uses, so "covered" means the same thing
    in both places. Outside the islands a bake leaves black, and a mean over
    black is not a mean over the rock: measured on the first run, roughness
    read 0.52 for a map made of 0.72–0.96, which is 0.85 × 60 % coverage."""
    me = lo.data
    me.calc_loop_triangles()
    uvl = me.uv_layers.active.data
    tris = me.loop_triangles
    n = len(tris) * 3
    uv = np.empty((n, 2), dtype=np.float64)
    idx = np.arange(n, dtype=np.int64).reshape(-1, 3)
    for i, t in enumerate(tris):
        for j, loop in enumerate(t.loops):
            uv[i * 3 + j] = uvl[loop].uv
    # The exporter writes v' = 1 − v (measured: a Blender (0,0) corner
    # exports as (0,1)), and the rasterizer takes glTF's top-left origin, so
    # the mask is built in the frame the PNG rows are in.
    uv[:, 1] = 1.0 - uv[:, 1]
    chart = mg.gc.rasterize(uv, idx, np.zeros(len(tris), dtype=np.int64), tex, tex)
    return chart >= 0


def fill_outside(arr, mask, fill):
    """Replace every uncovered texel with `fill`, so a mip level near a chart
    edge averages the rock with more rock rather than with black — a black
    normal-map background is the vector (-1, -1, -1)."""
    out = np.array(arr, dtype=np.float64)
    out[~mask] = fill
    return out


def image_array(img):
    """A baked image as top-down rows, the PNG's order. Blender stores pixel
    0 at the BOTTOM-left (measured: index 0 lands on the last row of a saved
    PNG), so the buffer is flipped here; the first cut of this file did not,
    and every normal and ORM map it wrote was upside down against the albedo
    Blender saved itself — a defect no whole-image statistic can see."""
    w, h = img.size
    a = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(a)
    return a.reshape(h, w, 4)[::-1, :, :3].astype(np.float64)


def save_png(path, rgb01):
    from PIL import Image
    Image.fromarray(np.clip(rgb01 * 255 + 0.5, 0, 255).astype(np.uint8), "RGB").save(path, optimize=True)


def final_material(bpy, lo, albedo_png, normal_png, orm_png):
    """The material the exporter turns into baseColor + normal + ORM: the
    two data maps Non-Color, roughness from G and metallic from B through a
    Separate Color node, occlusion from R through the exporter's own
    `glTF Material Output` group so it packs into the same texture."""
    mat = bpy.data.materials.new("rock")
    mat.use_nodes = True
    nt = mat.node_tree
    nodes, links = nt.nodes, nt.links
    bsdf = nodes["Principled BSDF"]
    alb = nodes.new("ShaderNodeTexImage")
    alb.image = bpy.data.images.load(albedo_png)
    alb.image.colorspace_settings.name = "sRGB"
    links.new(alb.outputs["Color"], bsdf.inputs["Base Color"])
    nrm = nodes.new("ShaderNodeTexImage")
    nrm.image = bpy.data.images.load(normal_png)
    nrm.image.colorspace_settings.name = "Non-Color"
    nmap = nodes.new("ShaderNodeNormalMap")
    nmap.space = "TANGENT"
    links.new(nrm.outputs["Color"], nmap.inputs["Color"])
    links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    orm = nodes.new("ShaderNodeTexImage")
    orm.image = bpy.data.images.load(orm_png)
    orm.image.colorspace_settings.name = "Non-Color"
    sep = nodes.new("ShaderNodeSeparateColor")
    links.new(orm.outputs["Color"], sep.inputs["Color"])
    links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
    links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
    ng = bpy.data.node_groups.get("glTF Material Output") or bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
    if "Occlusion" not in [s.name for s in ng.interface.items_tree]:
        ng.interface.new_socket("Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
    grp = nodes.new("ShaderNodeGroup")
    grp.node_tree = ng
    links.new(sep.outputs["Red"], grp.inputs["Occlusion"])
    # A closed hull has an inside nobody sees; the exporter writes
    # `doubleSided: true` unless culling is on, and Bevy would draw both.
    mat.use_backface_culling = True
    lo.data.materials.clear()
    lo.data.materials.append(mat)
    return mat


def export_glb(bpy, lo, path):
    bpy.ops.object.select_all(action="DESELECT")
    lo.select_set(True)
    bpy.context.view_layer.objects.active = lo
    bpy.ops.export_scene.gltf(
        filepath=path, export_format="GLB", use_selection=True, export_apply=True,
        export_texcoords=True, export_normals=True, export_tangents=True,
        export_materials="EXPORT", export_image_format="AUTO", export_yup=True,
        export_animations=False, export_skins=False, export_morph=False,
        export_lights=False, export_cameras=False, export_extras=False,
    )


def measure(path, occupant, tri_cap):
    """The triage's own reading of what was exported, and its verdict."""
    g, blob, nbytes = mg.read_glb(path)
    size, tris, rmax = mg.geometry(g, blob)
    plan, spread = mg.shape(g, blob)
    luma, green, contrast = mg.albedo(g, blob)
    mat = g.get("materials", [{}])[0]
    pbr = mat.get("pbrMetallicRoughness", {})
    slots = {
        "baseColor": "baseColorTexture" in pbr,
        "metallicRoughness": "metallicRoughnessTexture" in pbr,
        "normal": "normalTexture" in mat,
        "occlusion": "occlusionTexture" in mat,
    }
    measured = {
        "extent_m": [round(float(s), 4) for s in size],
        "radius_m": round(rmax, 4),
        "tris": int(tris),
        "plan_ratio": round(float(plan), 3),
        "spread": round(float(spread), 3),
        "luma": None if luma is None else round(luma, 3),
        "green": None if green is None else round(green, 4),
        "chart_contrast": None if contrast is None else round(contrast, 3),
        "bytes": nbytes,
        "texture_slots": slots,
    }
    dw = float(size[2] / size[0])
    bad = mg.verdict(occupant, tris, tri_cap, dw, 1.0, plan, spread, luma, green, contrast)
    return measured, bad


def report(path, measured, bad):
    m = measured
    print(f"{os.path.basename(path)}: {m['extent_m'][0]:.3f} x {m['extent_m'][1]:.3f} x {m['extent_m'][2]:.3f} m, "
          f"r {m['radius_m']:.4f}, {m['tris']} tris, plan {m['plan_ratio']}, spread {m['spread']}, "
          f"luma {m['luma']}, green {m['green']}, chart {m['chart_contrast']}, {m['bytes'] / 1e6:.1f} MB")
    print("  slots:", ", ".join(k for k, v in m["texture_slots"].items() if v) or "NONE")
    print("  verdict:", "KEEP" if not bad else "reject: " + ", ".join(bad))


def fit_and_origin(obj, target, mode, origin, followers=()):
    """Fit `obj` exactly and apply the same scale and shift to `followers`.

    Called twice in `gen`: once on the high-poly so the noise is sized, and
    again on the LOW-poly after decimation with the high-poly following —
    because a collapse can move a vertex a few millimetres outward, and
    `tests/prop_assets.rs` allows 0.1 mm past the blocked radius. The
    exported mesh is the one that has to be exact; the bake source only has
    to stay aligned with it, and a 0.2 % correction is inside the cage.
    """
    v = mesh_verts(obj)
    k = fit_scale(v, target, mode)
    v = v * k
    shift = origin_shift(v, origin)
    set_verts(obj, v + shift)
    for f in followers:
        set_verts(f, mesh_verts(f) * k + shift)
    return k


def calibrate_luma(png, luma, mask):
    """Scale the baked albedo so the mean linear luma OF THE ROCK is the
    row's number, then fill the uncovered texels with that mean colour.

    The procedural material aims at `luma` and lands under it — the AO term
    and the stain both take value away — and `--luma` should mean what it
    says, because the triage's pale/dark split reads this mean. One gain in
    linear space over the covered texels, bounded so a broken bake cannot be
    laundered into the band. The fill is what makes the triage's
    whole-image reading and the surface's own mean the same number.
    """
    from PIL import Image
    a = np.asarray(Image.open(png).convert("RGB"), dtype=np.float64) / 255.0
    lin = mg.gc.srgb_to_linear(a)
    mean = float(mg.gc.luma(lin)[mask].mean())
    gain = float(np.clip(luma / max(mean, 1e-6), 0.5, 2.0))
    lin = np.clip(lin * gain, 0, 1)
    lin = fill_outside(lin, mask, lin[mask].mean(axis=0))
    out = mg.gc.linear_to_srgb(lin)
    Image.fromarray(np.clip(out * 255 + 0.5, 0, 255).astype(np.uint8), "RGB").save(png, optimize=True)
    print(f"albedo mean luma over {mask.mean() * 100:.0f}% covered texels {mean:.3f} → {luma:.3f} (gain {gain:.3f})")
    return gain


def bake_all(bpy, sc, hi, lo, radius, tex, seed, workdir, stem, luma=0.26, metal=None, stats=None):
    """Normal, albedo, roughness and AO from `hi` onto `lo`'s UVs; the
    albedo and normal PNGs and a packed ORM PNG come back as paths.
    `metal` is `(material, mask socket)` for a surface with metal in it,
    baked into the ORM's blue channel; `None` is a rock, and blue stays 0.
    `stats`, when given, receives the measured metal share for the sidecar."""
    paths = {}
    mask = coverage_mask(lo, tex)
    normal = new_image(bpy, "bake_normal", tex, srgb=False)
    bake(bpy, sc, hi, lo, normal, "NORMAL", 1, radius)
    paths["normal"] = os.path.join(workdir, f"{stem}_normal.png")
    save_png(paths["normal"], fill_outside(image_array(normal), mask, (0.5, 0.5, 1.0)))
    alb = new_image(bpy, "bake_albedo", tex, srgb=True)
    bake(bpy, sc, hi, lo, alb, "DIFFUSE", 16, radius, pass_filter={"COLOR"})
    paths["albedo"] = os.path.join(workdir, f"{stem}_albedo.png")
    alb.filepath_raw = paths["albedo"]
    alb.file_format = "PNG"
    alb.save()
    calibrate_luma(paths["albedo"], luma, mask)
    rough = new_image(bpy, "bake_rough", tex, srgb=False)
    bake(bpy, sc, hi, lo, rough, "ROUGHNESS", 1, radius)
    ao = new_image(bpy, "bake_ao", tex, srgb=False)
    bake(bpy, sc, hi, lo, ao, "AO", 24, radius)
    r_arr, a_arr = image_array(rough)[..., 0], image_array(ao)[..., 0]
    r_arr = fill_outside(r_arr, mask, r_arr[mask].mean())
    a_arr = fill_outside(a_arr, mask, a_arr[mask].mean())
    print(f"roughness over the rock {r_arr[mask].mean():.3f}, occlusion {a_arr[mask].mean():.3f}, coverage {mask.mean() * 100:.0f}%")
    m_arr = None
    if metal is not None:
        img = new_image(bpy, "bake_metal", tex, srgb=False)
        bake_mask(bpy, sc, hi, lo, img, metal[0], metal[1], radius)
        m_arr = fill_outside(image_array(img)[..., 0], mask, 0.0)
        share = float((m_arr[mask] > 0.5).mean())
        print(f"metallic over the rock {m_arr[mask].mean():.3f} (share over 0.5: {share * 100:.1f}%)")
        if stats is not None:
            stats["metal_share"] = round(share, 4)
    orm = pack_orm(a_arr, r_arr, m_arr)
    paths["orm"] = os.path.join(workdir, f"{stem}_orm.png")
    save_png(paths["orm"], orm)
    return paths


def preview_render(bpy, sc, obj, lift, radius, out_stem, views=3):
    """Three Cycles views at a size an agent can read, with a 1.8 m post
    beside the piece for scale and the piece seated as the game seats it."""
    import mathutils
    obj.location = (0, 0, lift)
    bpy.ops.mesh.primitive_plane_add(size=max(12.0, radius * 10))
    ground = bpy.context.active_object
    gm = bpy.data.materials.new("ground")
    gm.use_nodes = True
    gm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.11, 0.13, 0.06, 1)
    gm.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 0.95
    ground.data.materials.append(gm)
    bpy.ops.mesh.primitive_cylinder_add(radius=0.2, depth=1.8, location=(radius + 0.9, 0, 0.9))
    post = bpy.context.active_object
    pm = bpy.data.materials.new("post")
    pm.use_nodes = True
    pm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.6, 0.35, 0.2, 1)
    post.data.materials.append(pm)
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.5
    sun.data.angle = math.radians(2.0)
    sun.rotation_euler = (math.radians(50), 0, math.radians(35))
    sc.collection.objects.link(sun)
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (0.45, 0.55, 0.75, 1)
    bg.inputs["Strength"].default_value = 0.6
    sc.world = world
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    cam.data.lens = 35
    sc.collection.objects.link(cam)
    sc.camera = cam
    sc.render.resolution_x, sc.render.resolution_y = 720, 450
    sc.render.resolution_percentage = 100
    sc.cycles.samples = 48
    sc.render.image_settings.file_format = "PNG"
    centre = mathutils.Vector((0, 0, lift + radius * 0.3))
    dist = max(radius * 3.2, 3.5)
    outs = []
    for i in range(views):
        az = math.radians(30 + 110 * i)
        el = math.radians(18 if i < 2 else 48)
        pos = centre + mathutils.Vector((math.cos(az) * math.cos(el), math.sin(az) * math.cos(el), math.sin(el))) * dist
        cam.location = pos
        cam.rotation_euler = (centre - pos).to_track_quat("-Z", "Y").to_euler()
        sc.render.filepath = f"{out_stem}_view{i + 1}.png"
        bpy.ops.render.render(write_still=True)
        outs.append(sc.render.filepath)
    return outs


# ── commands ───────────────────────────────────────────────────────────────


def cmd_gen(args):
    bpy = need_bpy()
    target, where = target_box(args.occupant, args.size)
    origin = args.origin or ("center" if args.occupant else "base")
    fit_mode = args.fit or ("radius" if args.occupant else "axes")
    radius = float(target[0]) / 2
    rng = random.Random(args.seed)
    p = kind_params(args.kind, 1.0, rng)  # massing is built at radius ~1, then fitted
    luma = args.luma or row_luma(args.occupant, args.kind)
    look = args.look or look_of(args.occupant, args.kind)
    out = os.path.abspath(args.out)
    workdir = os.path.dirname(out) or "."
    os.makedirs(workdir, exist_ok=True)
    stem = os.path.splitext(os.path.basename(out))[0]
    print(f"target {target[0]:.3f} x {target[1]:.3f} x {target[2]:.3f} m ({where}); kind {args.kind}, look {look}, seed {args.seed}, luma {luma}")

    sc = fresh_scene(bpy, args.seed)
    hi = base_mesh(bpy, args.kind, rng, p)
    displace(bpy, hi, p, args.seed)
    if p.get("chips"):
        set_verts(hi, chip(mesh_verts(hi), p["chips"]))
    remesh(bpy, hi, p["voxel"])
    facet(bpy, hi, p["facet_deg"])
    cut_bottom(bpy, hi, p["cut_bottom"])
    k = fit_and_origin(hi, target, fit_mode, origin)
    shade_smooth(bpy, hi)
    hi_tris = sum(len(f.vertices) - 2 for f in hi.data.polygons)
    metal = None
    if look == "granite":
        hi.data.materials.append(procedural_rock_material(bpy, "rock_hi", luma, rng, args.seed))
    else:
        mat, mask = procedural_node_material(bpy, "rock_hi", luma, rng, look)
        hi.data.materials.append(mat)
        metal = (mat, mask) if mask is not None else None

    lo = hi.copy()
    lo.data = hi.data.copy()
    lo.name = "rock_lo"
    sc.collection.objects.link(lo)
    decimate_to(bpy, lo, args.tris)
    k2 = fit_and_origin(lo, target, fit_mode, origin, followers=(hi,))
    shade_smooth(bpy, lo)
    smart_uv(bpy, lo)
    lo_tris = sum(len(f.vertices) - 2 for f in lo.data.polygons)
    print(f"high-poly {hi_tris} tris (voxel {p['voxel'] * radius:.3f} m), low-poly {lo_tris} tris, "
          f"fit {np.round(k, 4).tolist()}, post-decimate correction {np.round(k2, 4).tolist()}")

    stats = {}
    paths = bake_all(bpy, sc, hi, lo, radius, args.tex, args.seed, workdir, stem, luma=luma, metal=metal, stats=stats)
    final_material(bpy, lo, paths["albedo"], paths["normal"], paths["orm"])
    hi.hide_render = True
    export_glb(bpy, lo, out)
    measured, bad = measure(out, args.occupant, args.tris)
    report(out, measured, bad)
    extra = {"seed": args.seed, "kind": args.kind, "look": look, "luma": luma, "origin": origin, "fit": fit_mode,
             "blender": bpy.app.version_string, "python": sys.version.split()[0],
             "high_poly_tris": hi_tris, "textures": {k: os.path.basename(v) for k, v in paths.items()}, **stats}
    if args.preview:
        extra["previews"] = [os.path.basename(x) for x in preview_render(bpy, sc, lo, (mg.sim_lift(args.occupant) if args.occupant else 0.0), radius, os.path.join(workdir, stem))]
    json.dump(sidecar("gen", args, target, where, {kk: (vv * radius if kk.endswith(("_scale", "_strength", "voxel")) else vv) for kk, vv in p.items()},
                      measured, bad, extra), open(out + ".json", "w"), indent=1)
    print(f"wrote {out} and {out}.json")


def cmd_edit(args):
    bpy = need_bpy()
    sc = fresh_scene(bpy, args.seed)
    src = os.path.abspath(args.glb)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in sc.objects if o.type == "MESH"]
    if not meshes:
        raise SystemExit(f"{src}: no mesh")
    hi = meshes[0]
    with bpy.context.temp_override(object=hi, active_object=hi, selected_objects=meshes, selected_editable_objects=meshes):
        if len(meshes) > 1:
            bpy.ops.object.join()
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    hi.name = "rock_hi"
    # Blender imports glTF Y-up into its own Z-up, so the fit arithmetic
    # below is the same as gen's.
    target = where = None
    if args.occupant or args.size:
        target, where = target_box(args.occupant, args.size)
        fit_mode = args.fit or ("radius" if args.occupant else "axes")
        origin = args.origin or ("center" if args.occupant else "base")
        fit_and_origin(hi, target, fit_mode, origin)
    v = mesh_verts(hi)
    radius = float(np.hypot(v[:, 0] - v[:, 0].mean(), v[:, 1] - v[:, 1].mean()).max())
    if args.cut_below:
        cut_bottom(bpy, hi, args.cut_below)
    rebake = args.rebake or bool(args.remesh)
    lo = hi
    if rebake:
        lo = hi.copy()
        lo.data = hi.data.copy()
        lo.name = "rock_lo"
        sc.collection.objects.link(lo)
    if args.remesh:
        remesh(bpy, lo, args.remesh)
    if args.decimate:
        decimate_to(bpy, lo, args.decimate)
    if target is not None:
        fit_and_origin(lo, target, fit_mode, origin, followers=(hi,) if lo is not hi else ())
    shade_smooth(bpy, lo)
    out = os.path.abspath(args.out)
    workdir = os.path.dirname(out) or "."
    os.makedirs(workdir, exist_ok=True)
    stem = os.path.splitext(os.path.basename(out))[0]
    extra = {"source": os.path.basename(src), "blender": bpy.app.version_string, "rebaked": rebake}
    params = {"decimate": args.decimate, "remesh": args.remesh, "cut_below": args.cut_below}
    if rebake:
        smart_uv(bpy, lo)
        paths = bake_all(bpy, sc, hi, lo, radius, args.tex, args.seed, workdir, stem,
                         luma=args.luma or row_luma(args.occupant, "boulder"))
        final_material(bpy, lo, paths["albedo"], paths["normal"], paths["orm"])
        hi.hide_render = True
        extra["textures"] = {k: os.path.basename(v) for k, v in paths.items()}
    export_glb(bpy, lo, out)
    measured, bad = measure(out, args.occupant, args.tris)
    report(out, measured, bad)
    if args.preview:
        lift = mg.sim_lift(args.occupant) if args.occupant else 0.0
        extra["previews"] = [os.path.basename(x) for x in preview_render(bpy, sc, lo, lift, radius, os.path.join(workdir, stem))]
    json.dump(sidecar("edit", args, target if target is not None else [0, 0, 0], where or "unchanged", params, measured, bad, extra),
              open(out + ".json", "w"), indent=1)
    print(f"wrote {out} and {out}.json")


def cmd_preview(args):
    bpy = need_bpy()
    sc = fresh_scene(bpy, 0)
    src = os.path.abspath(args.glb)
    bpy.ops.import_scene.gltf(filepath=src)
    meshes = [o for o in sc.objects if o.type == "MESH"]
    obj = meshes[0]
    v = mesh_verts(obj)
    radius = float(np.hypot(v[:, 0], v[:, 1]).max())
    lift = mg.sim_lift(args.occupant) if args.occupant else 0.0
    stem = os.path.splitext(args.out or src)[0]
    for pth in preview_render(bpy, sc, obj, lift, radius, stem):
        print("wrote", pth)


def self_test():
    """The bpy-free arithmetic, proven on shapes known by construction."""
    ok = []

    def check(name, cond):
        ok.append(bool(cond))
        print(f"  {'ok ' if cond else 'BAD'} {name}")

    # 1 · one law: the target box is measure_glb's own arithmetic.
    r, top = mg.sim_volume("Rock")
    lift = mg.sim_lift("Rock")
    t, where = target_box("Rock", None)
    check("Rock target is [2r, 2(top-lift), 2r] off sim-core", np.allclose(t, [2 * r, 2 * (top - lift), 2 * r]) and where.startswith("sim-core"))
    # 2 · the radial fit puts the largest plan radius on r and the height on H.
    rng = np.random.default_rng(7)
    v = rng.normal(size=(4000, 3)) * [1.0, 0.7, 0.5] + [3.0, -2.0, 5.0]
    k = fit_scale(v, t, "radius")
    w = v * k
    w = w + origin_shift(w, "center")
    rmax = np.hypot(w[:, 0] - (w[:, 0].min() + w[:, 0].max()) / 2, w[:, 1] - (w[:, 1].min() + w[:, 1].max()) / 2).max()
    check("radial fit: plan radius == r within 1e-6", abs(rmax - r) < 1e-6)
    check("radial fit: height == 2(top-lift) within 1e-6", abs((w[:, 2].max() - w[:, 2].min()) - t[1]) < 1e-6)
    check("center origin: bbox midpoint at 0", np.allclose((w.min(axis=0) + w.max(axis=0)) / 2, 0, atol=1e-6))
    ka = fit_scale(v, np.array([3.0, 2.0, 1.5]), "axes")
    wa = v * ka
    wa = wa + origin_shift(wa, "base")
    check("axes fit: every extent on target (W→x, D→y, H→z)", np.allclose(wa.max(axis=0) - wa.min(axis=0), [3.0, 1.5, 2.0], atol=1e-6))
    check("base origin: feet on z=0, plan centred", abs(wa[:, 2].min()) < 1e-6 and np.allclose((wa[:, :2].min(axis=0) + wa[:, :2].max(axis=0)) / 2, 0, atol=1e-6))
    # 3 · the ORM layout.
    orm = pack_orm(np.full((4, 4), 0.25), np.full((4, 4), 0.8))
    check("ORM packs R=ao G=rough B=0", np.allclose(orm[..., 0], 0.25) and np.allclose(orm[..., 1], 0.8) and np.all(orm[..., 2] == 0))
    seam = np.zeros((4, 4))
    seam[1, 2] = 1.0
    orm = pack_orm(np.full((4, 4), 0.25), np.full((4, 4), 0.8), seam)
    check("ORM packs a metal mask into B and nothing else", np.array_equal(orm[..., 2], seam) and np.allclose(orm[..., 1], 0.8))
    # 4 · kind params scale with the radius and are seeded.
    a, b = kind_params("boulder", 1.0, random.Random(3)), kind_params("boulder", 2.0, random.Random(3))
    check("params scale with radius", abs(b["lobes_strength"] / a["lobes_strength"] - 2.0) < 1e-9 and abs(b["voxel"] / a["voxel"] - 2.0) < 1e-9)
    check("params are seeded", kind_params("slab", 1.0, random.Random(5)) == kind_params("slab", 1.0, random.Random(5)))
    check("every kind has params", all(kind_params(kd, 1.0, random.Random(1)) for kd in KINDS))
    # A node is round in plan BY CONSTRUCTION — the squash is equal on the
    # two plan axes on every seed — and a boulder never is, which is why the
    # boulder could not make one (`NOW.md` §0rk).
    ball = rng.normal(size=(3000, 3))
    ball /= np.linalg.norm(ball, axis=1, keepdims=True)
    one = [[0.3, 0.2, 0.93, 0.85]]
    n1 = np.array(one[0][:3]) / np.linalg.norm(one[0][:3])
    cut = chip(ball, one)
    check("a chip ends the reach along its normal at d", abs((cut @ n1).max() - 0.85 * (ball @ n1).max()) < 1e-9)
    many = kind_params("node", 1.0, random.Random(4))["chips"]
    cut = chip(ball, many)
    check("chips only move a vertex toward the centre",
          np.all(np.linalg.norm(cut, axis=1) <= np.linalg.norm(ball, axis=1) + 1e-12))
    nodes = [kind_params("node", 1.0, random.Random(s)) for s in range(1, 40)]
    check("node squash is equal in plan and low on every seed",
          all(n["squash"][0] == n["squash"][1] and n["squash"][2] < 1.0 and n["cut_bottom"] > 0 for n in nodes))
    check("boulder squash is never equal in plan",
          all(kind_params("boulder", 1.0, random.Random(s))["squash"][1] < 1.0 for s in range(1, 40)))
    # Adding a kind must not move another kind's draws: a seed names a rock,
    # and the rows in `MANIFEST.md` §rock_kit were measured off those names.
    # Pinned values, not a recomputation — a recomputed expectation would
    # share any change it was meant to catch.
    b1 = kind_params("boulder", 1.0, random.Random(1))
    check("existing kinds' seeds are unmoved by the node kind",
          abs(b1["lobes_scale"] - 1.1537457) < 1e-6 and abs(b1["squash"][1] - 0.6710138) < 1e-6)
    check("each ore node wears its own look, everything else granite",
          [look_of(o, "node") for o in ("StoneNode", "MetalNode", "SulfurNode", "Rock", None)]
          == ["stone", "metal", "sulfur", "granite", "granite"])
    # 5 · row luma follows the triage's bands.
    check("formation dark, node pale, split at granite",
          row_luma("Rock", "boulder") < mg.GRANITE_LUMA < row_luma("StoneNode", "boulder"))
    # 6 · the sidecar carries what a MANIFEST row needs.
    ns = argparse.Namespace(seed=1, kind="boulder", occupant="Rock", size=None, out="x.glb")
    side = sidecar("gen", ns, t, where, a, {"tris": 1}, [], {"seed": 1})
    check("sidecar has tool, hash, target, params, measured, verdict, date",
          all(kk in side for kk in ("tool", "script_sha256", "target_m", "target_from", "params", "measured", "verdict", "date")))
    if not all(ok):
        sys.exit("self-test FAILED")
    print(f"self-test: {len(ok)} cases ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--self-test", action="store_true", help="prove the bpy-free arithmetic; needs no Blender")
    sub = ap.add_subparsers(dest="cmd")

    def common(sp):
        sp.add_argument("--occupant", help="read the target from sim-core, e.g. Rock")
        sp.add_argument("--size", nargs=3, type=float, metavar=("W", "H", "D"), help="typed target for a tier the sim does not publish yet")
        sp.add_argument("--origin", choices=("center", "base"), help="default: center for an occupant row, base otherwise")
        sp.add_argument("--fit", choices=("radius", "axes"), help="default: radius for an occupant row, axes otherwise")
        sp.add_argument("--tris", type=int, default=2400, help="low-poly triangle ceiling")
        sp.add_argument("--tex", type=int, default=1024, help="bake resolution")
        sp.add_argument("--seed", type=int, default=1)
        sp.add_argument("--preview", action="store_true", help="render three views beside the GLB")
        sp.add_argument("--out", required=True)

    g = sub.add_parser("gen", help="generate a seeded kit piece")
    common(g)
    g.add_argument("--kind", choices=KINDS, default="boulder")
    g.add_argument("--look", choices=LOOKS, help="the surface; default from the row (each ore node its own, else granite)")
    g.add_argument("--luma", type=float, help="linear albedo luma; default from the row's band")
    g.set_defaults(func=cmd_gen)

    e = sub.add_parser("edit", help="edit a delivery: decimate, remesh, cut, rebake, refit")
    e.add_argument("glb")
    common(e)
    e.add_argument("--decimate", type=int, help="collapse to this many triangles")
    e.add_argument("--remesh", type=float, help="voxel remesh at this size in metres (implies --rebake)")
    e.add_argument("--cut-below", type=float, help="slice off the lowest fraction of the height and cap it")
    e.add_argument("--rebake", action="store_true", help="fresh UVs and a normal/albedo/ORM bake from the original")
    e.add_argument("--luma", type=float, help="linear albedo luma for a rebake; default from the row's band")
    e.set_defaults(func=cmd_edit)

    p = sub.add_parser("preview", help="render three views of a GLB")
    p.add_argument("glb")
    p.add_argument("--occupant")
    p.add_argument("--out")
    p.set_defaults(func=cmd_preview)

    a = ap.parse_args()
    if a.self_test:
        self_test()
        return
    if not a.cmd:
        ap.error("a command (gen, edit, preview) or --self-test")
    a.func(a)


if __name__ == "__main__":
    main()
