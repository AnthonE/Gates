# SPDX-License-Identifier: MIT
"""Hero shots off the shipped models: a camp at dusk, and every held item
knolled on a bench. For the store page and the menu, not for the game.

    ci/blender.py hero --out DIR [--quick]

`--quick` renders at a quarter of the size and samples, to check framing.
"""
import math
import os
import random
import sys

import bmesh
import bpy
from mathutils import Matrix

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (args_after_dashes, bounds, fit_camera, import_glb, look_at, mesh_object,
                    new_scene, plain_material, render, setup_render, sky)

__all__ = ()


def place(models, name, at, yaw=0.0, scale=1.0):
    root, meshes = import_glb(models[name], name)
    lo, hi = bounds(meshes)
    root.scale = (scale, scale, scale)
    root.rotation_euler.z = math.radians(yaw)
    # The shipped models stand on y = 0 (glTF) = z = 0 here, centred in plan.
    root.location = (at[0], at[1], at[2] if len(at) > 2 else 0.0)
    bpy.context.view_layer.update()
    return root, meshes


def conifer(at, height, rng, bark, needles):
    """A stacked-cone pine, enough for a silhouette 20 m behind the camp."""
    bm = bmesh.new()
    r0 = height * 0.018
    bmesh.ops.create_cone(bm, cap_ends=True, segments=8, radius1=r0 * 1.4, radius2=r0 * 0.5,
                          depth=height * 0.45)
    bmesh.ops.translate(bm, verts=bm.verts, vec=(0, 0, height * 0.225))
    trunk = mesh_object("trunk", bm, bark)
    bm = bmesh.new()
    tiers = 12
    for i in range(tiers):
        t = i / (tiers - 1)
        z0 = height * (0.22 + 0.70 * t)
        rad = height * (0.13 - 0.105 * t) * rng.uniform(0.75, 1.2)
        depth = height * rng.uniform(0.10, 0.15)
        geom = bmesh.ops.create_cone(bm, cap_ends=True, segments=7, radius1=rad, radius2=rad * 0.1,
                                     depth=depth)["verts"]
        bmesh.ops.translate(bm, verts=geom, vec=(rng.uniform(-0.4, 0.4), rng.uniform(-0.4, 0.4),
                                                  z0 + depth / 2))
        bmesh.ops.rotate(bm, verts=geom, cent=(0, 0, z0 + depth / 2),
                         matrix=Matrix.Rotation(rng.uniform(-0.12, 0.12), 3, "X"))
    crown = mesh_object("crown", bm, needles)
    for ob in (trunk, crown):
        ob.location = at
        ob.rotation_euler.z = rng.uniform(0, 6.28)
        for p in ob.data.polygons:
            p.use_smooth = True
    return trunk, crown


def camp(models, out, quick):
    sc = new_scene()
    w, h = (960, 540) if quick else (2560, 1440)
    setup_render(sc, w, h, samples=48 if quick else 384, exposure=-0.55)
    sky(sc, elevation=4.5, rotation=35.0, strength=1.0, turbidity=7.0)

    dirt = plain_material("dirt", (0.10, 0.082, 0.05), rough=0.97, noise=0.85, noise_scale=0.3, bump=0.6)
    bm = bmesh.new()
    bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=120)
    mesh_object("ground", bm, dirt)

    rng = random.Random(20260731)
    bark = plain_material("bark", (0.09, 0.06, 0.04), rough=0.95, noise=0.5, noise_scale=3.0, bump=0.8)
    needles = plain_material("needles", (0.035, 0.07, 0.03), rough=0.95, noise=0.5, noise_scale=1.5, bump=0.6)
    for i in range(70):
        a = math.radians(rng.uniform(-40, 220))
        r = rng.uniform(15, 60)
        conifer((r * math.sin(a), r * math.cos(a), 0), rng.uniform(16, 30), rng, bark, needles)

    # The camp. Fire at the origin, the camera to the south-west.
    layout = [
        ("fire", (0, 0), 0), ("bag", (-2.3, 1.1), 20), ("box", (2.1, 1.5), -15),
        ("workbench", (3.4, -1.0), 100), ("furnace", (-3.5, -0.7), 70),
        ("hearth", (0.6, 4.6), 180), ("barricade", (-1.6, 5.8), 0),
        ("barbed_barricade", (3.2, 5.3), -10), ("planter_box", (-4.6, 2.6), 10),
        ("water_catcher", (4.9, 2.8), 0), ("generator", (1.7, -2.7), 200),
        ("battery", (2.5, -2.9), 190), ("solar", (-3.0, 7.2), 15),
        ("stump", (6.2, -3.2), 0), ("rock_a", (-9.5, 8.5), 0), ("rock_c", (7.5, 10.5), 40),
        ("rock_b", (-9.0, -2.0), 120), ("barrel", (5.2, 0.8), 0), ("crate", (-1.2, -3.4), 15),
        ("rock_d", (11.0, 4.0), 200), ("oil_barrel", (5.6, 1.5), 30), ("stump", (-7.5, 4.0), 70),
        ("cache", (-5.5, -1.8), 0), ("tire_stack", (7.0, 6.5), 0),
    ]
    for name, at, yaw in layout:
        if name in models:
            place(models, name, at, yaw)
    # The fire's own light, since its emissive map only glows at the camera.
    la = bpy.data.lights.new("fire", "POINT")
    la.energy = 1400
    la.color = (1.0, 0.55, 0.22)
    la.shadow_soft_size = 0.35
    fire = bpy.data.objects.new("fire_light", la)
    fire.location = (0, 0, 0.55)
    sc.collection.objects.link(fire)

    look_at(sc, (-6.0, -7.8, 1.55), (0.6, 0.9, 1.0), lens=30)
    return render(sc, os.path.join(out, "camp_dusk.png"))


def arsenal(models, origs, out, quick):
    sc = new_scene()
    w, h = (960, 540) if quick else (2560, 1440)
    setup_render(sc, w, h, samples=32 if quick else 192, exposure=-0.3)
    sky(sc, elevation=60.0, rotation=0.0, strength=0.35)
    la = bpy.data.lights.new("key", "AREA")
    la.energy = 2500
    la.size = 6.0
    key = bpy.data.objects.new("key", la)
    key.location = (-2, -3, 6)
    key.rotation_euler = (math.radians(25), math.radians(-15), 0)
    sc.collection.objects.link(key)

    felt = plain_material("felt", (0.022, 0.024, 0.02), rough=1.0, noise=0.4, noise_scale=40.0, bump=0.3)
    bm = bmesh.new()
    bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=20)
    mesh_object("bench", bm, felt)

    # Lay every held item flat (its standing +Z down the bench's +Y; a long
    # one along X instead) and shelf-pack them by depth, deepest row first.
    held = sorted(n for n, p in origs.items() if "/held/" in p and "/skins/" not in p)
    items = []
    for name in held:
        root, meshes = import_glb(models[name], name)
        # The viewmodel draws mesh 0's first primitive only; a file that
        # carries more (bone_helmet ships a 2 m icosphere) is laid out by
        # its biggest mesh and the rest is hidden.
        meshes.sort(key=lambda o: -len(o.data.polygons))
        for extra in meshes[1:]:
            extra.hide_render = True
        meshes = meshes[:1]
        root.rotation_euler.x = math.radians(-90)
        bpy.context.view_layer.update()
        lo, hi = bounds(meshes)
        if hi.y - lo.y > 1.2:
            root.rotation_euler.z = math.radians(90)
            bpy.context.view_layer.update()
            lo, hi = bounds(meshes)
        items.append((root, meshes, hi.x - lo.x, hi.y - lo.y, lo))
    items.sort(key=lambda t: -t[3])
    gap = 0.12
    width = 8.6
    x = y = 0.0
    row_h = 0.0
    for root, meshes, iw, ih, lo in items:
        if x > 0 and x + iw > width:
            x = 0.0
            y -= row_h + gap
            row_h = 0.0
        root.location = (x - lo.x, y - lo.y - ih, -lo.z)
        x += iw + gap
        row_h = max(row_h, ih)
    bpy.context.view_layer.update()
    allm = [m for _, ms, *_ in items for m in ms]
    lo, hi = bounds(allm)
    fit_camera(sc, lo, hi, azimuth=0, pitch=89.5, lens=50, fill=0.94)
    print(f"arsenal: {len(items)} held items on a {hi.x-lo.x:.1f} x {hi.y-lo.y:.1f} m bench")
    return render(sc, os.path.join(out, "arsenal.png"))


def main():
    args = args_after_dashes()
    out = os.path.abspath(args[args.index("--out") + 1])
    quick = "--quick" in args
    only = [a.split("=", 1)[1] for a in args if a.startswith("--only=")]
    # Keyed by file stem: the decoded copy to import, and the original path
    # (which still says which category a model came from).
    models, origs = {}, {}
    for a in args:
        if "=" in a and not a.startswith("--"):
            orig, dec = a.split("=", 1)
            stem = os.path.basename(orig)[:-4]
            models[stem], origs[stem] = dec, orig
    if not only or "camp" in only:
        print(camp(models, out, quick))
    if not only or "arsenal" in only:
        print(arsenal(models, origs, out, quick))


main()
