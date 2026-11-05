# SPDX-License-Identifier: MIT
"""Render every model from four views and each category as a scale lineup
beside the 1.8 m player. ci/blender.py composes the frames into cards.

    blender -b --python ci/bl/sheet.py -- --out DIR orig.glb=decoded.glb …

Writes DIR/_views/<cat>/<name>_<i>.png, DIR/_views/<cat>/lineup.png and
DIR/_views/index.json (dims, tris, materials, texture sizes per model).
"""
import json
import os
import sys

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (PLAYER_H, args_after_dashes, bounds, fit_camera, ground, import_glb,
                    new_scene, render, setup_render, sky, sun)

__all__ = ()

# (azimuth, pitch): three-quarter front, three-quarter back, side, plan.
VIEWS = ((35, 18), (215, 18), (125, 8), (0, 89))
LINEUP_COLS = 12


def stats(meshes):
    tris = 0
    mats = set()
    tex = 0
    for o in meshes:
        me = o.data
        tris += sum(len(p.vertices) - 2 for p in me.polygons)
        for s in o.material_slots:
            if s.material:
                mats.add(s.material.name)
                if s.material.use_nodes:
                    for n in s.material.node_tree.nodes:
                        if n.type == "TEX_IMAGE" and n.image:
                            tex = max(tex, max(n.image.size))
    return tris, len(mats), tex


def stand(root, meshes):
    """Feet on z = 0, centred on the origin; return the standing bounds."""
    lo, hi = bounds(meshes)
    root.location = (-(lo.x + hi.x) / 2, -(lo.y + hi.y) / 2, -lo.z)
    bpy.context.view_layer.update()
    return bounds(meshes)


def clear(root, meshes):
    for o in list(root.children_recursive) + [root]:
        bpy.data.objects.remove(o, do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)


def main():
    args = args_after_dashes()
    out = os.path.abspath(args[args.index("--out") + 1])
    pairs = [a.split("=", 1) for a in args if "=" in a and not a.startswith("--")]
    player = os.environ.get("GATES_BL_PLAYER")
    views = os.path.join(out, "_views")

    sc = new_scene()
    setup_render(sc, 512, 512, samples=48, exposure=-0.4)
    sky(sc, strength=0.25)
    sun(sc, strength=3.0)
    ground(sc)

    index = {}
    by_cat = {}
    for orig, dec in pairs:
        cat = os.path.basename(os.path.dirname(orig))
        name = os.path.basename(orig)[:-4]
        root, meshes = import_glb(dec, name)
        if not meshes:
            clear(root, meshes)
            continue
        lo, hi = stand(root, meshes)
        tris, nmat, tex = stats(meshes)
        for i, (az, pt) in enumerate(VIEWS):
            fit_camera(sc, lo, hi, azimuth=az, pitch=pt, fill=0.78)
            render(sc, os.path.join(views, cat, f"{name}_{i}.png"))
        index[f"{cat}/{name}"] = {
            "size": [round(hi.x - lo.x, 3), round(hi.z - lo.z, 3), round(hi.y - lo.y, 3)],
            "tris": tris, "materials": nmat, "texture": tex,
            "bytes": os.path.getsize(orig),
        }
        by_cat.setdefault(cat, []).append((orig, dec, name))
        clear(root, meshes)
        print(f"{cat}/{name}: {tris} tris, {hi.x-lo.x:.2f}x{hi.z-lo.z:.2f}x{hi.y-lo.y:.2f} m")

    # Lineups: a category in rows, the player at the head of the first row.
    for cat, items in by_cat.items():
        roots = []
        allm = []
        if player and os.path.exists(player):
            r, m = import_glb(player, "player")
            lo, hi = stand(r, m)
            k = PLAYER_H / max(hi.z - lo.z, 1e-6)
            r.scale = (k, k, k)
            bpy.context.view_layer.update()
            roots.append((r, m))
        for orig, dec, name in items:
            r, m = import_glb(dec, name)
            stand(r, m)
            roots.append((r, m))
        cell = max(max(hi.x - lo.x, hi.y - lo.y) for lo, hi in (bounds(m) for _, m in roots)) * 1.25
        cell = max(cell, 0.3)
        for i, (r, m) in enumerate(roots):
            c, row = i % LINEUP_COLS, i // LINEUP_COLS
            r.location.x += c * cell
            r.location.y -= row * cell
            allm += m
        bpy.context.view_layer.update()
        lo, hi = bounds(allm)
        rows = (len(roots) + LINEUP_COLS - 1) // LINEUP_COLS
        sc.render.resolution_x = 2400
        sc.render.resolution_y = max(600, min(1400, 400 + 300 * rows))
        fit_camera(sc, lo, hi, azimuth=20, pitch=24, lens=45, fill=0.9)
        render(sc, os.path.join(views, cat, "lineup.png"))
        sc.render.resolution_x = sc.render.resolution_y = 512
        for r, m in roots:
            clear(r, m)

    with open(os.path.join(views, "index.json"), "w") as f:
        json.dump(index, f, indent=1, sort_keys=True)
    print(f"sheet: {len(index)} models, {len(by_cat)} categories -> {views}")


main()
