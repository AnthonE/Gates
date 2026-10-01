#!/usr/bin/env python3
"""The Blender stage for hand-modelled props: deployables and held items
built from parts in code, baked onto one material, headless.

    ci/prop_kit.py list
    ci/prop_kit.py gen furnace recycler        # -> the paths in RECIPES
    ci/prop_kit.py gen --all
    ci/prop_kit.py sheet furnace recycler      # a Cycles contact sheet PNG

**Why a third Blender tool.** `rock_kit.py` grows noise into a rock and
`site_kit.py` dresses collision boxes; neither can make a furnace or a
revolver. A recipe here is a short Python function that places boxes,
cylinders, lathes and sweeps, each tagged with a surface. The surfaces are
the depot's own photographs (`assets/textures/`, box-projected in metres)
plus CC0 sets fetched at bake time (`TEXSETS`), and a small procedural set
for what has no good photo (food, fur, plastic). The bake adds edge wear and
crease dirt (`weather`), so the props read as part of the same used world
as the walls.

**One mesh, one primitive, one material** — what `build_kit` and the
viewmodel load (`tests/deploy_assets.rs`, `tests/held_assets.rs`). The
parts are joined, unwrapped onto one atlas, and Cycles bakes albedo,
roughness, metallic, normal and AO off the multi-material mesh onto it.
The export is then `ci/ktx_pack.py`'d to KTX2/UASTC like every other model.

**Frames.** Recipes are written in Blender space: metres, Z up, the FRONT
of a deployable toward −Y (glTF +Z after the exporter's Y-up turn). A
deployable is fitted uniformly inside its `DEPLOY` row and stood on y = 0.
A held item is authored with its grip on the Z axis and is only lifted to
y = 0, never re-centred: `import_meshy.py` centring a bounding box is what
once put a hatchet's haft 121 mm off the fist.

Needs bpy (`pip install bpy==4.5.0`, Python 3.11, as `rock_kit.py`) and
the KTX CLI (`ci/ktx_pack.py`'s header).
"""
import argparse
import math
import os
import subprocess
import sys
import tempfile

import numpy as np

CI = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(CI)
sys.path.insert(0, CI)
import rock_kit as rk  # noqa: E402  (bake/export plumbing, need_bpy)

TEX = os.path.join(ROOT, "assets", "textures")
RAW = os.path.join(tempfile.gettempdir(), "prop_kit")
CACHE = os.path.expanduser("~/.cache/gates-prop-kit")

# Bake-time photo sets beyond the depot's own, all CC0: (host, id, the
# source's published width in mm, or our estimate where it publishes none).
# Fetched once into CACHE at 1K and baked INTO the models, so nothing here
# ships and no NOTICE is owed. Poly Haven: polyhaven.com/a/<id>; ambientCG:
# ambientcg.com/view?id=<id>.
TEXSETS = {
    "hessian": ("ph", "hessian_230", 268),
    "linen": ("ph", "hessian_380", 274),
    "jersey": ("ph", "cotton_jersey", 264),
    "hide": ("ph", "brown_leather", 400),
    "rust": ("ph", "rust_coarse_01", 2200),
    "brushed": ("acg", "Metal009", 600),
    "scratched": ("acg", "Metal038", 800),
    "paint": ("acg", "PaintedMetal004", 1000),
    "paper": ("acg", "Paper001", 400),
    "cord": ("acg", "Rope001", 250),
}

# ── surfaces ────────────────────────────────────────────────────────────────
# `tex` = a photographed set in assets/textures/ at `tiles` per metre;
# `proc` = a procedural look. `tint` multiplies the albedo, `rough` is a
# value (procedural) or a multiplier on the map (photo), `metal` a constant,
# `sat` desaturates a photo. Colours are linear RGB.
SURF = {
    "planks": dict(tex="wood", tiles=1.0, tint=(1.0, 0.92, 0.82), rough=1.0, metal=0.0),
    "wood": dict(tex="wood", tiles=2.2, tint=(1.05, 0.86, 0.66), rough=1.0, metal=0.0),
    "darkwood": dict(tex="wood", tiles=2.2, tint=(0.62, 0.46, 0.33), rough=1.0, metal=0.0),
    "stone": dict(tex="stone", tiles=1.2, tint=(0.95, 0.9, 0.84), sat=0.3, rough=1.0, metal=0.0),
    "rock": dict(tex="rock", tiles=1.5, tint=(1.0, 1.0, 1.0), rough=1.0, metal=0.0),
    "concrete": dict(tex="concrete", tiles=0.8, tint=(0.8, 0.8, 0.8), rough=1.0, metal=0.0),
    "sheet": dict(tex="metal", tiles=0.8, tint=(0.95, 0.95, 0.95), rough=1.0, metal=0.85),
    "steel": dict(tex="brushed", sat=0.0, color=(0.50, 0.50, 0.51), rough=0.9, metal=1.0),
    "dark": dict(tex="scratched", sat=0.0, color=(0.20, 0.20, 0.21), rough=1.0, metal=1.0),
    "blued": dict(tex="scratched", sat=0.0, color=(0.09, 0.10, 0.12), rough=0.8, metal=1.0),
    "iron": dict(tex="rust", sat=0.25, color=(0.10, 0.08, 0.065), tiles=1.5, rough=1.0, metal=0.4),
    "rusty": dict(tex="rust", tint=(1.0, 1.0, 1.0), tiles=1.2, rough=1.0, metal=0.3),
    "brass": dict(tex="brushed", sat=0.0, color=(0.55, 0.40, 0.17), rough=0.8, metal=1.0),
    "soot": dict(proc="plain", color=(0.035, 0.03, 0.028), rough=0.95, metal=0.0, wear=0.0, grime=0.0),
    "char": dict(proc="plain", color=(0.06, 0.05, 0.045), rough=0.9, metal=0.0, wear=0.0, grime=0.0),
    "green": dict(tex="paint", sat=0.0, color=(0.075, 0.10, 0.06), rough=1.0, metal=None),
    "yellow": dict(tex="paint", sat=0.0, color=(0.55, 0.38, 0.05), rough=1.0, metal=None),
    "red": dict(tex="paint", sat=0.0, color=(0.42, 0.06, 0.04), rough=1.0, metal=None),
    "blue": dict(tex="paint", sat=0.0, color=(0.06, 0.14, 0.38), rough=1.0, metal=None),
    "white": dict(tex="paint", sat=0.0, color=(0.66, 0.65, 0.61), rough=1.0, metal=None),
    "rubber": dict(proc="plain", color=(0.04, 0.04, 0.04), rough=0.85, metal=0.0, wear=0.3),
    "burlap": dict(tex="hessian", tint=(1.0, 1.0, 1.0), rough=1.0, metal=0.0, wear=0.25),
    "canvas": dict(tex="linen", sat=0.3, color=(0.40, 0.37, 0.28), rough=1.0, metal=0.0, wear=0.25),
    "olive": dict(tex="linen", sat=0.0, color=(0.13, 0.15, 0.075), rough=1.0, metal=0.0, wear=0.25),
    "gauze": dict(tex="jersey", sat=0.0, color=(0.72, 0.70, 0.64), rough=1.0, metal=0.0, wear=0.0, grime=0.3),
    "rope": dict(tex="cord", sat=0.6, color=(0.42, 0.33, 0.20), rough=1.0, metal=0.0, wear=0.0),
    "leather": dict(tex="hide", sat=0.8, color=(0.14, 0.075, 0.04), rough=1.0, metal=0.0, wear=0.35),
    "paper": dict(tex="paper", sat=0.0, color=(0.70, 0.66, 0.54), rough=1.0, metal=0.0, wear=0.0),
    "blueprint": dict(tex="paper", sat=0.0, color=(0.10, 0.20, 0.42), rough=1.0, metal=0.0, wear=0.0),
    "glass": dict(proc="plain", color=(0.30, 0.38, 0.36), rough=0.08, metal=0.0, wear=0.0, grime=0.0),
    "plastic": dict(proc="plain", color=(0.70, 0.70, 0.68), rough=0.45, metal=0.0, wear=0.0, grime=0.0),
    "berry": dict(proc="plain", color=(0.25, 0.02, 0.10), rough=0.25, metal=0.0, wear=0.0, grime=0.0),
    "leaf": dict(proc="paint", color=(0.10, 0.22, 0.06), rough=0.6, metal=0.0, wear=0.0, grime=0.0),
    "cap": dict(proc="paint", color=(0.48, 0.30, 0.16), rough=0.55, metal=0.0, wear=0.0, grime=0.0),
    "stem": dict(proc="paint", color=(0.78, 0.72, 0.60), rough=0.7, metal=0.0, wear=0.0, grime=0.0),
    "corn": dict(proc="kernels", color=(0.80, 0.55, 0.08), rough=0.45, metal=0.0, wear=0.0, grime=0.0),
    "husk": dict(proc="paper", color=(0.30, 0.38, 0.12), rough=0.85, metal=0.0, wear=0.0, grime=0.0),
    "raw": dict(proc="meat", color=(0.55, 0.08, 0.08), rough=0.35, metal=0.0, wear=0.0, grime=0.0),
    "cooked": dict(proc="meat", color=(0.36, 0.16, 0.07), rough=0.5, metal=0.0, wear=0.0, grime=0.0),
    "burnt": dict(proc="meat", color=(0.06, 0.04, 0.03), rough=0.8, metal=0.0, wear=0.0, grime=0.0),
    "bone": dict(proc="paint", color=(0.80, 0.76, 0.64), rough=0.6, metal=0.0, wear=0.0, grime=0.0),
    "flame": dict(proc="plain", color=(0.35, 0.10, 0.02), rough=0.9, metal=0.0, wear=0.0, grime=0.0),
    "pighide": dict(proc="fur", color=(0.075, 0.052, 0.036), rough=0.85, metal=0.0, wear=0.0, grime=0.3),
    "wolffur": dict(proc="fur", color=(0.10, 0.085, 0.065), rough=0.9, metal=0.0, wear=0.0, grime=0.3),
    "skin": dict(proc="plain", color=(0.45, 0.22, 0.20), rough=0.5, metal=0.0, wear=0.0, grime=0.0),
    "hoof": dict(proc="plain", color=(0.03, 0.025, 0.02), rough=0.5, metal=0.0, wear=0.0, grime=0.0),
    "eye": dict(proc="plain", color=(0.01, 0.008, 0.006), rough=0.1, metal=0.0, wear=0.0, grime=0.0),
    "tan": dict(tex="linen", sat=0.5, color=(0.30, 0.15, 0.055), rough=1.0, metal=0.0, wear=0.25),
    "tape": dict(tex="jersey", sat=0.0, color=(0.26, 0.27, 0.27), rough=0.6, metal=0.0, wear=0.2),
    "kgreen": dict(proc="paint", color=(0.05, 0.40, 0.10), rough=0.4, metal=0.0, wear=0.0, grime=0.0),
    "kblue": dict(proc="paint", color=(0.04, 0.14, 0.55), rough=0.4, metal=0.0, wear=0.0, grime=0.0),
    "kred": dict(proc="paint", color=(0.55, 0.04, 0.04), rough=0.4, metal=0.0, wear=0.0, grime=0.0),
    "signgreen": dict(proc="plain", color=(0.02, 0.30, 0.10), rough=0.5, metal=0.0, wear=0.0, grime=0.0),
    "signwhite": dict(proc="plain", color=(0.85, 0.85, 0.82), rough=0.5, metal=0.0, wear=0.0, grime=0.0),
}


# ── fetched photo sets ──────────────────────────────────────────────────────

def _get(url):
    import time
    import urllib.request
    for attempt in range(5):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "gates-prop-kit"})
            return urllib.request.urlopen(req, timeout=120).read()
        except Exception:  # resets and truncated bodies (IncompleteRead) alike
            if attempt == 4:
                raise
            time.sleep(2 ** attempt)


def texset(name):
    """{albedo, rough, normal[, metal]} paths for a photo set: the depot's
    own (`assets/textures/<name>_*.jpg`) or a TEXSETS row, fetched on first
    use. Normals are the OpenGL (+Y) convention Blender bakes in."""
    if name not in TEXSETS:
        return {k: os.path.join(TEX, f"{name}_{k}.jpg") for k in ("albedo", "rough", "normal")}
    host, aid, _ = TEXSETS[name]
    d = os.path.join(CACHE, aid)
    paths = {k: os.path.join(d, f"{k}.jpg") for k in ("albedo", "rough", "normal", "metal")}
    if not os.path.exists(paths["normal"]):
        os.makedirs(d, exist_ok=True)
        if host == "ph":
            import json
            files = json.loads(_get(f"https://api.polyhaven.com/files/{aid}"))
            pick = {"albedo": ("Diffuse", "diff", "col_01"), "rough": ("Rough", "rough"),
                    "normal": ("nor_gl",), "metal": ("Metal", "metal")}
            for k, keys in pick.items():
                key = next((x for x in keys if x in files), None)
                if key is not None:
                    open(paths[k], "wb").write(_get(files[key]["1k"]["jpg"]["url"]))
        else:
            import io
            import zipfile
            z = zipfile.ZipFile(io.BytesIO(_get(f"https://ambientcg.com/get?file={aid}_1K-JPG.zip")))
            pick = {"albedo": "_Color.jpg", "rough": "_Roughness.jpg", "normal": "_NormalGL.jpg",
                    "metal": "_Metalness.jpg"}
            for k, suffix in pick.items():
                n = next((x for x in z.namelist() if x.endswith(suffix)), None)
                if n is not None:
                    open(paths[k], "wb").write(z.read(n))
    return {k: p for k, p in paths.items() if os.path.exists(p)}


def mean_luma(path, sat):
    """Linear mean of an albedo after the surface's desaturation — what a
    `color` divides by so a photo keeps its texture and takes our value.

    Blender's Hue/Saturation node works in HSV, so saturation 0 leaves the
    pixel's MAX channel, not its luma: red paint desaturates to ~0.8, not
    ~0.2. Measured the node's way, or every tinted photo comes out pale."""
    from PIL import Image
    a = np.asarray(Image.open(path).convert("RGB").resize((128, 128)), dtype=np.float64) / 255
    lin = np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)
    v = lin.max(axis=-1, keepdims=True)
    col = v + (lin - v) * sat
    return col.reshape(-1, 3).mean(0)


# ── the parts kit ───────────────────────────────────────────────────────────

class Kit:
    """Parts accumulate as separate objects (one surface each) and are
    joined at the end. Every helper takes `at` (a translation), `rot` (XYZ
    Euler, degrees) and `bevel` (metres; 0 for none). Returns the object."""

    def __init__(self, bpy):
        import bmesh
        self.bpy, self.bmesh = bpy, bmesh
        self.parts = []

    # -- plumbing
    def _emit(self, bm, surf, at, rot, bevel, segs=1, angle=30.0, scale=None, smooth=True):
        bmesh, bpy = self.bmesh, self.bpy
        from mathutils import Euler, Matrix, Vector
        if bevel > 0:
            edges = [e for e in bm.edges if len(e.link_faces) == 2
                     and e.calc_face_angle(0.0) > math.radians(angle)]
            if edges:
                bmesh.ops.bevel(bm, geom=edges, offset=bevel, segments=segs, profile=0.5,
                                affect="EDGES", clamp_overlap=True)
        m = Matrix.Translation(Vector(at)) @ Euler([math.radians(a) for a in rot], "XYZ").to_matrix().to_4x4()
        if scale is not None:
            m = m @ Matrix.Diagonal(Vector((*scale, 1.0)))
        bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
        me = bpy.data.meshes.new(surf)
        bm.to_mesh(me)
        bm.free()
        if smooth:
            for p in me.polygons:
                p.use_smooth = True
            me.set_sharp_from_angle(angle=math.radians(40))
        ob = bpy.data.objects.new(surf, me)
        bpy.context.scene.collection.objects.link(ob)
        ob.data.materials.append(material(bpy, surf))
        self.parts.append(ob)
        return ob

    # -- primitives
    def box(self, size, at=(0, 0, 0), surf="wood", rot=(0, 0, 0), bevel=0.006, segs=1):
        bm = self.bmesh.new()
        self.bmesh.ops.create_cube(bm, size=1.0)
        self.bmesh.ops.scale(bm, vec=size, verts=bm.verts)
        return self._emit(bm, surf, at, rot, bevel, segs)

    def cyl(self, r, h, at=(0, 0, 0), surf="steel", axis="Z", rot=(0, 0, 0), n=16, r2=None,
            bevel=0.004, segs=1, cap=True):
        bm = self.bmesh.new()
        self.bmesh.ops.create_cone(bm, cap_ends=cap, cap_tris=False, segments=n,
                                   radius1=r, radius2=r if r2 is None else r2, depth=h)
        self._axis(bm, axis)
        return self._emit(bm, surf, at, rot, bevel, segs)

    def ball(self, r, at=(0, 0, 0), surf="steel", scale=(1, 1, 1), rot=(0, 0, 0), n=12):
        bm = self.bmesh.new()
        self.bmesh.ops.create_uvsphere(bm, u_segments=n, v_segments=max(4, n // 2 + 1), radius=r)
        return self._emit(bm, surf, at, rot, 0.0, scale=scale)

    def torus(self, R, r, at=(0, 0, 0), surf="steel", axis="Z", rot=(0, 0, 0), n=20, m=8, arc=360.0):
        bm = self.bmesh.new()
        full = arc >= 359.9
        rings = n if full else n + 1
        grid = []
        for i in range(rings):
            a = math.radians(arc) * i / n
            ring = []
            for j in range(m):
                b = 2 * math.pi * j / m
                rr = R + r * math.cos(b)
                ring.append(bm.verts.new((rr * math.cos(a), rr * math.sin(a), r * math.sin(b))))
            grid.append(ring)
        for i in range(n):
            i2 = (i + 1) % rings if full else i + 1
            for j in range(m):
                j2 = (j + 1) % m
                bm.faces.new((grid[i][j], grid[i2][j], grid[i2][j2], grid[i][j2]))
        if not full:
            bm.faces.new(list(reversed(grid[0])))
            bm.faces.new(grid[-1])
        self._axis(bm, axis)
        return self._emit(bm, surf, at, rot, 0.0)

    def lathe(self, profile, at=(0, 0, 0), surf="steel", axis="Z", rot=(0, 0, 0), n=16,
              scale=None, bevel=0.0):
        """Revolve [(radius, z), ...] about Z. A radius of 0 closes the end."""
        bm = self.bmesh.new()
        rings = []
        for rad, z in profile:
            if rad <= 1e-6:
                rings.append([bm.verts.new((0, 0, z))])
                continue
            rings.append([bm.verts.new((rad * math.cos(2 * math.pi * j / n),
                                        rad * math.sin(2 * math.pi * j / n), z)) for j in range(n)])
        for a, b in zip(rings, rings[1:]):
            if len(a) == 1 and len(b) == 1:
                continue
            for j in range(n):
                j2 = (j + 1) % n
                if len(a) == 1:
                    bm.faces.new((a[0], b[j], b[j2]))
                elif len(b) == 1:
                    bm.faces.new((a[j], b[0], a[j2]))
                else:
                    bm.faces.new((a[j], b[j], b[j2], a[j2]))
        if len(rings[0]) > 1:
            bm.faces.new(list(reversed(rings[0])))
        if len(rings[-1]) > 1:
            bm.faces.new(rings[-1])
        self.bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        self._axis(bm, axis)
        return self._emit(bm, surf, at, rot, bevel, scale=scale)

    def slab(self, outline, thick, at=(0, 0, 0), surf="steel", plane="XZ", rot=(0, 0, 0),
             bevel=0.002, segs=1):
        """Extrude a 2D outline [(u, v), ...] by `thick`, centred. `plane`
        names the axes u and v lie on; the extrusion is the third axis."""
        bm = self.bmesh.new()
        vs = [bm.verts.new((u, v, -thick / 2)) for u, v in outline]
        f = bm.faces.new(vs)
        r = self.bmesh.ops.extrude_face_region(bm, geom=[f])
        top = [g for g in r["geom"] if isinstance(g, self.bmesh.types.BMVert)]
        self.bmesh.ops.translate(bm, vec=(0, 0, thick), verts=top)
        self.bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        from mathutils import Matrix
        if plane == "XZ":
            self.bmesh.ops.rotate(bm, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, "X"), verts=bm.verts)
        elif plane == "YZ":
            self.bmesh.ops.rotate(bm, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, "X"), verts=bm.verts)
            self.bmesh.ops.rotate(bm, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, "Z"), verts=bm.verts)
        return self._emit(bm, surf, at, rot, bevel, segs)

    def tube(self, path, r, surf="rope", n=8, closed=False, r_end=None):
        """Sweep a circle of radius r (tapering to r_end) along a polyline."""
        from mathutils import Vector
        bm = self.bmesh.new()
        pts = [Vector(p) for p in path]
        rings = []
        up = Vector((0, 0, 1))
        for i, p in enumerate(pts):
            a = pts[max(i - 1, 0)]
            b = pts[min(i + 1, len(pts) - 1)]
            t = (b - a).normalized()
            ref = up if abs(t.dot(up)) < 0.9 else Vector((1, 0, 0))
            u = t.cross(ref).normalized()
            v = t.cross(u).normalized()
            rr = r if r_end is None else r + (r_end - r) * i / (len(pts) - 1)
            rings.append([bm.verts.new(p + rr * (math.cos(2 * math.pi * j / n) * u
                                                 + math.sin(2 * math.pi * j / n) * v)) for j in range(n)])
        segs = len(rings) if closed else len(rings) - 1
        for i in range(segs):
            a, b = rings[i], rings[(i + 1) % len(rings)]
            for j in range(n):
                bm.faces.new((a[j], b[j], b[(j + 1) % n], a[(j + 1) % n]))
        if not closed:
            bm.faces.new(list(reversed(rings[0])))
            bm.faces.new(rings[-1])
        self.bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        return self._emit(bm, surf, (0, 0, 0), (0, 0, 0), 0.0)

    def turn_all(self, rot, at=(0, 0, 0)):
        """Pose every part placed so far as one body: XYZ Euler degrees,
        then a translation (Blender space)."""
        from mathutils import Euler, Matrix, Vector
        m = Matrix.Translation(Vector(at)) @ Euler([math.radians(a) for a in rot], "XYZ").to_matrix().to_4x4()
        # A part placed by setting `.location` has a stale matrix_world until
        # the depsgraph runs; read it fresh or that offset is lost.
        self.bpy.context.view_layer.update()
        for p in self.parts:
            p.matrix_world = m @ p.matrix_world

    def _axis(self, bm, axis):
        from mathutils import Matrix
        if axis == "X":
            self.bmesh.ops.rotate(bm, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, "Y"), verts=bm.verts)
        elif axis == "Y":
            self.bmesh.ops.rotate(bm, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, "X"), verts=bm.verts)

    # -- finishing
    def join(self):
        bpy = self.bpy
        bpy.ops.object.select_all(action="DESELECT")
        for p in self.parts:
            p.select_set(True)
        bpy.context.view_layer.objects.active = self.parts[0]
        bpy.ops.object.join()
        ob = bpy.context.view_layer.objects.active
        ob.name = "prop"
        # Join leaves the FIRST part's object transform on the result; the
        # game loads the primitive and drops node transforms, so bake it in.
        from mathutils import Matrix
        ob.data.transform(ob.matrix_world)
        ob.matrix_world = Matrix.Identity(4)
        # Tangents (the normal bake's and the exporter's) need tris/quads;
        # bevels and caps leave n-gons.
        bm = self.bmesh.new()
        bm.from_mesh(ob.data)
        self.bmesh.ops.triangulate(bm, faces=bm.faces, quad_method="BEAUTY", ngon_method="BEAUTY")
        bm.to_mesh(ob.data)
        bm.free()
        return ob


# ── materials ───────────────────────────────────────────────────────────────

def _img(bpy, path, srgb):
    im = bpy.data.images.load(path, check_existing=True)
    im.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
    return im


def material(bpy, key):
    m = bpy.data.materials.get(key)
    if m is not None:
        return m
    s = SURF[key]
    m = bpy.data.materials.new(key)
    m.use_nodes = True
    nt = m.node_tree
    N, L = nt.nodes, nt.links
    bsdf = N["Principled BSDF"]
    co = N.new("ShaderNodeTexCoord")
    metal = N.new("ShaderNodeValue")
    metal.name = "METAL"
    metal.outputs[0].default_value = s["metal"] or 0.0
    L.new(metal.outputs[0], bsdf.inputs["Metallic"])

    def tex(path, srgb, vec):
        t = N.new("ShaderNodeTexImage")
        t.image = _img(bpy, path, srgb)
        t.projection = "BOX"
        t.projection_blend = 0.3
        L.new(vec, t.inputs["Vector"])
        return t

    def math_(op, a, b):
        n = N.new("ShaderNodeMath")
        n.operation = op
        for i, x in enumerate((a, b)):
            if isinstance(x, (int, float)):
                n.inputs[i].default_value = x
            else:
                L.new(x, n.inputs[i])
        return n.outputs[0]

    def mix(fac, a, b, blend="MIX"):
        n = N.new("ShaderNodeMix")
        n.data_type = "RGBA"
        n.blend_type = blend
        for sock, x in ((n.inputs[0], fac), (n.inputs[6], a), (n.inputs[7], b)):
            if isinstance(x, (int, float)):
                sock.default_value = x
            elif isinstance(x, tuple):
                sock.default_value = (*x, 1.0)
            else:
                L.new(x, sock)
        return n.outputs[2]

    def noise(scale, detail=4.0, vec=None, sx=(1, 1, 1)):
        n = N.new("ShaderNodeTexNoise")
        n.inputs["Scale"].default_value = scale
        n.inputs["Detail"].default_value = detail
        mp = N.new("ShaderNodeMapping")
        mp.inputs["Scale"].default_value = sx
        L.new(vec or co.outputs["Object"], mp.inputs["Vector"])
        L.new(mp.outputs[0], n.inputs["Vector"])
        return n.outputs["Fac"]

    def bump(height, strength):
        b = N.new("ShaderNodeBump")
        b.inputs["Strength"].default_value = strength
        b.inputs["Distance"].default_value = 0.002
        L.new(height, b.inputs["Height"])
        L.new(b.outputs["Normal"], bsdf.inputs["Normal"])

    def ramp(fac, lo, hi):
        return mix(fac, lo, hi)

    if "tex" in s:
        maps = texset(s["tex"])
        tiles = s.get("tiles") or 1000.0 / TEXSETS[s["tex"]][2]
        mp = N.new("ShaderNodeMapping")
        mp.inputs["Scale"].default_value = (tiles,) * 3
        L.new(co.outputs["Object"], mp.inputs["Vector"])
        v = mp.outputs[0]
        alb = tex(maps["albedo"], True, v)
        hs = N.new("ShaderNodeHueSaturation")
        hs.inputs["Saturation"].default_value = s.get("sat", 1.0)
        L.new(alb.outputs["Color"], hs.inputs["Color"])
        if "color" in s:
            # Keep the photo's variation, take our value: divide by its mean.
            mean = mean_luma(maps["albedo"], s.get("sat", 1.0))
            tint = tuple(min(4.0, c / max(m_, 1e-3)) for c, m_ in zip(s["color"], mean))
        else:
            tint = tuple(s["tint"])
        L.new(mix(1.0, hs.outputs["Color"], tint, "MULTIPLY"), bsdf.inputs["Base Color"])
        rough = tex(maps["rough"], False, v)
        L.new(math_("MINIMUM", math_("MULTIPLY", rough.outputs["Color"], s["rough"]), 1.0),
              bsdf.inputs["Roughness"])
        nrm = tex(maps["normal"], False, v)
        nm = N.new("ShaderNodeNormalMap")
        nm.inputs["Strength"].default_value = s.get("bumpiness", 1.0)
        L.new(nrm.outputs["Color"], nm.inputs["Color"])
        L.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
        if s["metal"] is None and "metal" in maps:
            L.new(tex(maps["metal"], False, v).outputs["Color"], bsdf.inputs["Metallic"])
        weather(N, L, bsdf, s, mix, math_, noise)
        return m

    c = tuple(s["color"])
    lo = tuple(x * 0.7 for x in c)
    hi = tuple(min(1.0, x * 1.25) for x in c)
    p = s["proc"]
    if p == "plain":
        n = noise(60.0, 3.0)
        L.new(ramp(n, lo if s["rough"] > 0.5 else c, hi if s["rough"] > 0.5 else c), bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = s["rough"]
        bump(n, 0.08)
    elif p == "paint":
        # Flat colour with chips: bare dark steel shows through where a
        # coarse noise crosses a threshold.
        n = noise(35.0, 6.0)
        chip = N.new("ShaderNodeMapRange")
        chip.inputs["From Min"].default_value = 0.66
        chip.inputs["From Max"].default_value = 0.70
        L.new(noise(14.0, 8.0), chip.inputs["Value"])
        L.new(mix(chip.outputs[0], ramp(n, lo, hi), (0.12, 0.11, 0.10)), bsdf.inputs["Base Color"])
        r = math_("ADD", math_("MULTIPLY", chip.outputs[0], -0.15), s["rough"])
        L.new(math_("ADD", r, math_("MULTIPLY", n, 0.15)), bsdf.inputs["Roughness"])
        _relabel_metal(N, L, metal, math_("ADD", math_("MULTIPLY", chip.outputs[0], 0.8), s["metal"]))
        bump(math_("ADD", n, math_("MULTIPLY", chip.outputs[0], -0.6)), 0.25)
    elif p == "fur":
        # Short coarse hair: noise stretched along the body (Blender Y) for
        # streaks, a second scale for patchiness.
        streak = noise(70.0, 3.0, sx=(1, 0.12, 1))
        patch = noise(5.0, 4.0)
        col = mix(math_("ADD", math_("MULTIPLY", streak, 0.6), math_("MULTIPLY", patch, 0.4)), lo, hi)
        L.new(col, bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = s["rough"]
        bump(streak, 0.4)
    elif p == "kernels":
        vor = N.new("ShaderNodeTexVoronoi")
        vor.inputs["Scale"].default_value = 120.0
        L.new(co.outputs["Object"], vor.inputs["Vector"])
        L.new(ramp(vor.outputs["Distance"], hi, lo), bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = s["rough"]
        bump(vor.outputs["Distance"], 0.6)
    elif p == "paper":
        n = noise(25.0, 8.0)
        L.new(ramp(n, lo, hi), bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = s["rough"]
        bump(n, 0.05)
    elif p == "meat":
        fat = N.new("ShaderNodeTexWave")
        fat.wave_type = "BANDS"
        fat.inputs["Scale"].default_value = 9.0
        fat.inputs["Distortion"].default_value = 6.0
        fat.inputs["Detail"].default_value = 4.0
        L.new(co.outputs["Object"], fat.inputs["Vector"])
        streak = N.new("ShaderNodeMapRange")
        streak.inputs["From Min"].default_value = 0.85
        streak.inputs["From Max"].default_value = 0.95
        L.new(fat.outputs["Fac"], streak.inputs["Value"])
        n = noise(18.0, 6.0)
        fat_col = tuple(min(1.0, x * 2.2 + 0.25) for x in c) if c[0] > 0.2 else tuple(x * 1.6 for x in c)
        col = mix(streak.outputs[0], ramp(n, lo, hi), fat_col)
        L.new(col, bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = s["rough"]
        bump(n, 0.3)
    else:
        raise SystemExit(f"unknown proc {p}")
    weather(N, L, bsdf, s, mix, math_, noise)
    return m


def family(s):
    if s.get("proc") == "paint" or s.get("tex") == "paint":
        return "paint"
    if (s["metal"] if s["metal"] is not None else 1.0) >= 0.5:
        return "metal"
    return "other"


WEAR = {"paint": 1.0, "metal": 0.7, "other": 0.45}


def weather(N, L, bsdf, s, mix, math_, noise):
    """Use, baked in: edges scuffed (paint worn to bare steel, metal
    polished, everything else lightened) and dirt settled in the creases.

    The edge mask is a Bevel normal against the true normal — it is 1 only
    where the surface turns a corner — broken up by noise so wear reads as
    handling, not as an outline. The crease mask is local AO. Both are
    Cycles shader nodes, so they bake with everything else; `wear`/`grime`
    per surface set the amounts and 0 turns either off."""
    fam = family(s)
    wear = s.get("wear", WEAR[fam])
    grime = s.get("grime", 0.5)
    if wear <= 0 and grime <= 0:
        return

    def src(sock):
        if sock.links:
            return sock.links[0].from_socket
        v = sock.default_value
        if hasattr(v, "__len__"):
            n = N.new("ShaderNodeRGB")
            n.outputs[0].default_value = tuple(v)
        else:
            n = N.new("ShaderNodeValue")
            n.outputs[0].default_value = v
        return n.outputs[0]

    C = src(bsdf.inputs["Base Color"])
    R = src(bsdf.inputs["Roughness"])
    M = src(bsdf.inputs["Metallic"])
    if wear > 0:
        bev = N.new("ShaderNodeBevel")
        bev.samples = 4
        bev.inputs["Radius"].default_value = 0.012
        geo = N.new("ShaderNodeNewGeometry")
        dot = N.new("ShaderNodeVectorMath")
        dot.operation = "DOT_PRODUCT"
        L.new(bev.outputs["Normal"], dot.inputs[0])
        L.new(geo.outputs["Normal"], dot.inputs[1])
        rim = N.new("ShaderNodeMapRange")
        rim.inputs["From Min"].default_value = 0.995
        rim.inputs["From Max"].default_value = 0.85
        L.new(dot.outputs["Value"], rim.inputs["Value"])
        brk = N.new("ShaderNodeMapRange")
        brk.inputs["From Min"].default_value = 0.38
        brk.inputs["From Max"].default_value = 0.62
        L.new(noise(22.0, 8.0), brk.inputs["Value"])
        edge = math_("MULTIPLY", math_("MULTIPLY", rim.outputs[0], brk.outputs[0]), wear)
        if fam == "paint":
            C = mix(edge, C, (0.40, 0.40, 0.41))
            M = math_("MINIMUM", math_("ADD", M, edge), 1.0)
            R = math_("ADD", math_("MULTIPLY", R, math_("SUBTRACT", 1.0, edge)), math_("MULTIPLY", edge, 0.32))
        elif fam == "metal":
            C = mix(edge, C, mix(1.0, C, (1.7, 1.7, 1.7), "MULTIPLY"))
            R = math_("MAXIMUM", math_("SUBTRACT", R, math_("MULTIPLY", edge, 0.18)), 0.12)
        else:
            C = mix(edge, C, mix(1.0, C, (1.3, 1.3, 1.3), "MULTIPLY"))
    if grime > 0:
        ao = N.new("ShaderNodeAmbientOcclusion")
        ao.samples = 4
        ao.only_local = True
        ao.inputs["Distance"].default_value = 0.06
        dirt = math_("MINIMUM", math_("MULTIPLY", math_("SUBTRACT", 1.0, ao.outputs["AO"]), grime * 1.2), 0.55)
        C = mix(dirt, C, (0.045, 0.038, 0.03))
        R = math_("MINIMUM", math_("ADD", R, math_("MULTIPLY", dirt, 0.25)), 1.0)
        M = math_("MULTIPLY", M, math_("SUBTRACT", 1.0, math_("MULTIPLY", dirt, 0.6)))
    L.new(C, bsdf.inputs["Base Color"])
    L.new(R, bsdf.inputs["Roughness"])
    L.new(M, bsdf.inputs["Metallic"])
    if wear > 0:
        # Round the shading normal over every edge too, so the baked normal
        # map catches a highlight on corners the low-poly mesh keeps sharp.
        nb = N.new("ShaderNodeBevel")
        nb.samples = 4
        nb.inputs["Radius"].default_value = 0.005
        if bsdf.inputs["Normal"].links:
            L.new(bsdf.inputs["Normal"].links[0].from_socket, nb.inputs["Normal"])
        L.new(nb.outputs["Normal"], bsdf.inputs["Normal"])


def _relabel_metal(N, L, metal_value, socket):
    """Point the bsdf's Metallic and the METAL bake tap at a computed socket.
    The tap is a Math node named METAL so the emission bake finds one name."""
    bsdf = N["Principled BSDF"]
    L.new(socket, bsdf.inputs["Metallic"])
    n = socket.node
    N.remove(metal_value)
    n.name = "METAL"


# ── bake & export ───────────────────────────────────────────────────────────

def fresh(bpy):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.cycles.use_denoising = False
    return sc


def unwrap(bpy, ob):
    me = ob.data
    while me.uv_layers:
        me.uv_layers.remove(me.uv_layers[0])
    uv = me.uv_layers.new(name="UVMap")
    me.uv_layers.active = uv
    uv.active_render = True
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(60), island_margin=0.008,
                             area_weight=0.0, correct_aspect=True, scale_to_bounds=False)
    bpy.ops.uv.pack_islands(rotate=True, margin=0.006)
    bpy.ops.object.mode_set(mode="OBJECT")


def _targets(bpy, ob, img):
    for m in ob.data.materials:
        nt = m.node_tree
        n = nt.nodes.get("BAKE") or nt.nodes.new("ShaderNodeTexImage")
        n.name = "BAKE"
        n.image = img
        nt.nodes.active = n


def bake_pass(bpy, sc, ob, img, kind, samples, **kw):
    _targets(bpy, ob, img)
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    sc.cycles.samples = samples
    bpy.ops.object.bake(type=kind, margin=16, use_clear=True, use_selected_to_active=False, **kw)


def bake_input(bpy, sc, ob, img, name):
    """Bake what feeds each material's Principled `name` input (a link or
    its constant) through an emission shader, one EMIT bake.

    **Base colour goes this way too, not through a DIFFUSE/COLOR bake**:
    that pass is the diffuse albedo, base colour × (1 − metallic), so every
    metal part baked black and drew black in the game. Metallic has no bake
    type at all (`rock_kit.bake_mask`'s trick)."""
    saved = []
    for m in ob.data.materials:
        nt = m.node_tree
        out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL")
        bsdf = nt.nodes["Principled BSDF"]
        sock = bsdf.inputs[name]
        was = out.inputs["Surface"].links[0].from_socket
        em = nt.nodes.new("ShaderNodeEmission")
        made = None
        if sock.links:
            nt.links.new(sock.links[0].from_socket, em.inputs["Color"])
        else:
            v = sock.default_value
            rgb = tuple(v)[:3] if hasattr(v, "__len__") else (v, v, v)
            made = nt.nodes.new("ShaderNodeRGB")
            made.outputs[0].default_value = (*rgb, 1.0)
            nt.links.new(made.outputs[0], em.inputs["Color"])
        nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
        saved.append((nt, out, was, em, made))
    try:
        bake_pass(bpy, sc, ob, img, "EMIT", 8)
    finally:
        for nt, out, was, em, made in saved:
            nt.links.new(was, out.inputs["Surface"])
            nt.nodes.remove(em)
            if made is not None:
                nt.nodes.remove(made)


def bake_maps(bpy, sc, ob, size, work, stem):
    imgs = {k: rk.new_image(bpy, f"{stem}_{k}", size, k == "albedo")
            for k in ("albedo", "rough", "metal", "normal", "ao")}
    bake_input(bpy, sc, ob, imgs["albedo"], "Base Color")
    bake_pass(bpy, sc, ob, imgs["rough"], "ROUGHNESS", 8)
    bake_input(bpy, sc, ob, imgs["metal"], "Metallic")
    bake_pass(bpy, sc, ob, imgs["normal"], "NORMAL", 8, normal_space="TANGENT",
              normal_r="POS_X", normal_g="POS_Y", normal_b="POS_Z")
    sc.world = sc.world or bpy.data.worlds.new("w")
    bake_pass(bpy, sc, ob, imgs["ao"], "AO", 48)
    a = {k: rk.image_array(v) for k, v in imgs.items()}
    # The albedo bake saved through Blender's own colour management; the
    # array read is the raw buffer, which for an sRGB image is display values.
    paths = {}
    paths["albedo"] = os.path.join(work, f"{stem}_albedo.png")
    rk.save_png(paths["albedo"], a["albedo"])
    paths["normal"] = os.path.join(work, f"{stem}_normal.png")
    rk.save_png(paths["normal"], a["normal"])
    ao = 0.35 + 0.65 * a["ao"][..., 0]
    orm = np.stack([ao, a["rough"][..., 0], a["metal"][..., 0]], axis=-1)
    paths["orm"] = os.path.join(work, f"{stem}_orm.png")
    rk.save_png(paths["orm"], orm)
    return paths


def mesh_np(ob):
    me = ob.data
    v = np.empty(len(me.vertices) * 3, dtype=np.float64)
    me.vertices.foreach_get("co", v)
    return v.reshape(-1, 3)


def fit(ob, recipe):
    """Deployables: uniform scale inside the DEPLOY row, centred in plan,
    stood on 0. `centre`: the same fit, centred on all three axes (the
    death bag's cuboid convention). Held: lifted onto 0 only (the grip axis
    is authored)."""
    v = mesh_np(ob)
    if recipe["kind"] in ("deploy", "centre"):
        w, h, d = recipe["size"]
        lo, hi = v.min(0), v.max(0)
        ext = hi - lo
        s = min(w / ext[0], d / ext[1], h / ext[2]) * 0.9995
        c = (lo + hi) / 2
        foot = lo[2] if recipe["kind"] == "deploy" else c[2]
        v = (v - np.array([c[0], c[1], foot])) * s
    elif recipe["kind"] == "held":
        v = v - np.array([0.0, 0.0, v[:, 2].min()])
    ob.data.vertices.foreach_set("co", v.astype(np.float32).ravel())
    ob.data.update()


def build(name, out_dir=None):
    bpy = rk.need_bpy()
    r = RECIPES[name]
    sc = fresh(bpy)
    k = Kit(bpy)
    r["fn"](k)
    ob = k.join()
    tris = sum(len(p.vertices) - 2 for p in ob.data.polygons)
    unwrap(bpy, ob)
    os.makedirs(RAW, exist_ok=True)
    size = r.get("tex", 1024)
    maps = bake_maps(bpy, sc, ob, size, RAW, name)
    rk.final_material(bpy, ob, maps["albedo"], maps["normal"], maps["orm"])
    ob.data.materials[0].name = name
    for p in ob.data.polygons:
        p.material_index = 0
    fit(ob, r)
    raw = os.path.join(RAW, f"{name}.raw.glb")
    rk.export_glb(bpy, ob, raw)
    out = os.path.join(ROOT, "assets", r["out"]) if out_dir is None else os.path.join(out_dir, f"{name}.glb")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    subprocess.run([sys.executable, os.path.join(CI, "ktx_pack.py"), raw, out, "--size", str(size)],
                   check=True)
    v = mesh_np(ob)
    ext = v.max(0) - v.min(0)
    print(f"  {name:16s} {tris:6d} tris  {ext[0]:.3f} x {ext[2]:.3f} x {ext[1]:.3f} m (w x h x d)  -> {r['out']}")
    return raw


# ── preview ─────────────────────────────────────────────────────────────────

def sheet(names, out_png, px=384):
    bpy = rk.need_bpy()
    from PIL import Image
    tiles = []
    for n in names:
        raw = os.path.join(RAW, f"{n}.raw.glb")
        sc = fresh(bpy)
        bpy.ops.import_scene.gltf(filepath=raw)
        obs = [o for o in sc.objects if o.type == "MESH"]
        v = np.concatenate([np.array([o.matrix_world @ x.co for x in o.data.vertices]) for o in obs])
        lo, hi = v.min(0), v.max(0)
        c = (lo + hi) / 2
        rad = float(np.linalg.norm(hi - lo)) / 2
        cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
        sc.collection.objects.link(cam)
        cam.data.lens = 50
        dist = rad / math.tan(cam.data.angle / 2) * 1.05
        from mathutils import Vector
        d = Vector((0.75, -1.0, 0.55)).normalized()
        cam.location = Vector(c) + d * dist
        cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
        sc.camera = cam
        sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
        sun.data.energy = 3.0
        sun.rotation_euler = (math.radians(40), math.radians(10), math.radians(30))
        sc.collection.objects.link(sun)
        w = bpy.data.worlds.new("w")
        w.use_nodes = True
        w.node_tree.nodes["Background"].inputs["Color"].default_value = (0.45, 0.5, 0.58, 1)
        w.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.8
        sc.world = w
        sc.render.resolution_x = sc.render.resolution_y = px
        sc.cycles.samples = 24
        sc.view_settings.view_transform = "AgX" if "AgX" in [i.identifier for i in sc.view_settings.bl_rna.properties["view_transform"].enum_items] else "Filmic"
        path = os.path.join(RAW, f"{n}.png")
        sc.render.filepath = path
        bpy.ops.render.render(write_still=True)
        tiles.append(Image.open(path).convert("RGB"))
    cols = min(4, len(tiles))
    rows = (len(tiles) + cols - 1) // cols
    out = Image.new("RGB", (cols * px, rows * px), (30, 30, 30))
    for i, t in enumerate(tiles):
        out.paste(t, ((i % cols) * px, (i // cols) * px))
    out.save(out_png)
    print(f"  sheet -> {out_png}")


from prop_recipes import RECIPES  # noqa: E402  (the objects themselves)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    sp.add_parser("list")
    g = sp.add_parser("gen")
    g.add_argument("names", nargs="*")
    g.add_argument("--all", action="store_true")
    s = sp.add_parser("sheet")
    s.add_argument("names", nargs="+")
    s.add_argument("--out", default=os.path.join(RAW, "sheet.png"))
    a = ap.parse_args()
    if a.cmd == "list":
        for n, r in RECIPES.items():
            print(f"  {n:16s} {r['kind']:6s} {r['out']}")
    elif a.cmd == "gen":
        names = list(RECIPES) if a.all else a.names
        for n in names:
            build(n)
    elif a.cmd == "sheet":
        sheet(a.names, a.out)


if __name__ == "__main__":
    main()
