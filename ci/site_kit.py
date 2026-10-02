#!/usr/bin/env python3
"""The Blender stage for the big authored sites: dress a kit's collision boxes
into a model, headless, and export one GLB the client draws in place of the
boxes.

    ci/site_kit.py gen --kit ci/kits/town.json --out assets/models/site/town.glb
    ci/site_kit.py --self-test

**The boxes are the law.** `sim-core` owns every part a body collides with
(`town.rs`, `monument.rs`); `examples/kit_dump.rs` writes them to
`ci/kits/<name>.json` and a drift test holds that file to the source. This
script only *dresses* them: a container wall becomes stacked, ribbed
containers with corner castings; a pylon becomes courses of dark ashlar with
gilt bands; the ring of gilt decor boxes becomes a broken torus; awnings sag;
a post is carried up to the roof it holds; tarp poles stand through their
tables; lamps hang on rods and string lights between them. Anything it adds
below head height stays over its part's footprint (±5 cm), so what you see is
what stops you.

**No textures.** Each mesh is named by its material role (`yard`, `cargo`,
`obsidian`, …) and the client binds the game's own photographed surfaces to
it (`render/town.rs`). UVs are in metres; vertex colour carries each piece's
tint times a baked ambient occlusion, so the look comes from the same CC0 maps
every other structure uses and the file stays geometry-only.

Blender runs as a Python module (`pip install bpy==4.5.*`, Python 3.11), as
`ci/rock_kit.py` does. `--self-test` needs no Blender and proves the
arithmetic (the axis map, the ring fit, the ribbing, the glyphs).
"""
import argparse
import hashlib
import json
import math
import os
import random
import sys

# ── bpy-free arithmetic (self-tested) ───────────────────────────────────────

ROLES = ("yard", "concrete", "sheet", "cargo", "timber", "steel", "obsidian",
         "gilt", "canvas", "lapis", "bulb")
ROOF, DECOR, STACK = 1, 2, 4
CONTAINER_L, CONTAINER_H = 6.06, 2.6
# Above this nobody's head reaches, so a drawn-only piece cannot be walked into.
HEAD_M = 2.0
# The furthest a post is carried up to its roof, or a lamp's rod reaches.
REACH_M = 3.0

GLYPHS = {
    "T": [31, 4, 4, 4, 4, 4, 4],
    "H": [17, 17, 17, 31, 17, 17, 17],
    "E": [31, 16, 16, 30, 16, 16, 31],
    "G": [15, 16, 16, 23, 17, 17, 15],
    "A": [14, 17, 17, 31, 17, 17, 17],
    "B": [30, 17, 17, 30, 17, 17, 30],
    "L": [16, 16, 16, 16, 16, 16, 31],
    "C": [14, 17, 16, 16, 16, 17, 14],
    "K": [17, 18, 20, 24, 20, 18, 17],
    "Z": [31, 1, 2, 4, 8, 16, 31],
    "I": [31, 4, 4, 4, 4, 4, 31],
    "U": [17, 17, 17, 17, 17, 17, 14],
    "R": [30, 17, 17, 30, 20, 18, 17],
    " ": [0] * 7,
}


def kit_to_blender(p):
    """Kit (x, y up, z) to Blender (x, -z, y): the glTF exporter's Y-up
    conversion maps it straight back, and the matrix is a rotation (det +1),
    so winding survives."""
    x, y, z = p
    return (x, -z, y)


def fit_circle(points):
    """Centre (x, y) and radius of the circle through `points` in the XY
    plane — the mean centre of a symmetric arc set and the mean distance."""
    n = len(points)
    cx = sum(p[0] for p in points) / n
    # The set is the upper arc and two lower stubs: the x-centre is the mean,
    # and the y-centre is where the two furthest-apart x extremes sit.
    left = min(points, key=lambda p: p[0])
    right = max(points, key=lambda p: p[0])
    cy = (left[1] + right[1]) / 2.0
    r = sum(math.hypot(p[0] - cx, p[1] - cy) for p in points) / n
    return cx, cy, r


def rib_profile(length, pitch=0.3, depth=0.035):
    """Offsets along a corrugated sheet: (position, outset) pairs, trapezoids
    of `pitch` with flats a third of it."""
    n = max(1, int(length / pitch))
    step = length / n
    out = []
    for i in range(n):
        a = i * step
        out += [(a, 0.0), (a + step / 3, depth), (a + 2 * step / 3, depth)]
    out.append((length, 0.0))
    return out


def glyph_cells(text):
    """(column, row) of every lit cell of `text` in the 5×7 font, columns
    counted across the whole string with one blank between letters."""
    cells = []
    for i, ch in enumerate(text):
        rows = GLYPHS.get(ch.upper(), GLYPHS[" "])
        for j, row in enumerate(rows):
            for k in range(5):
                if row & (1 << (4 - k)):
                    cells.append((i * 6 + k, j))
    return cells


def awning_uv(b, u, v):
    """Height of awning `b`'s canvas at fractions (u, v) across its x and z:
    0.5 m lower at the edge it overhangs (the lower |x| edge faces the
    street) and sagging between its z ends."""
    edge = u if abs(b[0]) < abs(b[3]) else 1 - u  # 0 at the street edge
    return b[4] - 0.5 * (1 - edge) - 0.12 * math.sin(math.pi * v)


def underside(p, x, z):
    """What part `p` offers from below at (x, z): an awning its own canvas,
    anything else its bottom face."""
    b = p["b"]
    if p["mat"] == "canvas" and p["flags"] & DECOR:
        return awning_uv(b, (x - b[0]) / (b[3] - b[0]), (z - b[2]) / (b[5] - b[2]))
    return b[1]


def over(b, x, z):
    return b[0] <= x <= b[3] and b[2] <= z <= b[5]


def above(parts, x, z, y):
    """The lowest underside at or above height `y` over (x, z), or None."""
    hits = [h for h in (underside(p, x, z) for p in parts if over(p["b"], x, z)) if h >= y - 1e-6]
    return min(hits) if hits else None


def below(parts, x, z, y):
    """The highest top at or below height `y` under (x, z), or None."""
    tops = [p["b"][4] for p in parts if over(p["b"], x, z) and p["b"][4] <= y + 1e-6]
    return max(tops) if tops else None


def reach(p, parts):
    """Part `p`'s box as drawn: a post whose top is out of reach and stops
    short of the roof over it is carried up to that roof (the watchtowers'
    legs collide to 8 m and their roofs sit at 9.6)."""
    b = p["b"]
    dx, dy, dz = b[3] - b[0], b[4] - b[1], b[5] - b[2]
    if p["mat"] not in ("timber", "steel") or dy <= max(dx, dz) * 2 or b[4] < HEAD_M:
        return b
    top = above(parts, (b[0] + b[3]) / 2, (b[2] + b[5]) / 2, b[4])
    if top is None or top - b[4] > REACH_M:
        return b
    return [b[0], b[1], b[2], b[3], top, b[5]]


def self_test():
    assert kit_to_blender((1, 2, 3)) == (1, -3, 2)
    pts = [(7.6 * math.cos(math.radians(a)), 18 + 7.6 * math.sin(math.radians(a)))
           for a in (0, 30, 60, 90, 120, 150, 180, 210, 330)]
    cx, cy, r = fit_circle(pts)
    assert abs(cx) < 0.5 and abs(cy - 18) < 1e-6 and abs(r - 7.6) < 0.2, (cx, cy, r)
    prof = rib_profile(6.0)
    assert prof[0] == (0.0, 0.0) and abs(prof[-1][0] - 6.0) < 1e-9
    assert all(prof[i][0] <= prof[i + 1][0] for i in range(len(prof) - 1))
    cells = glyph_cells("THE GATE")
    assert max(c for c, _ in cells) == 7 * 6 + 4 and len(cells) > 60
    kit = {"name": "t", "parts": [{"b": [0, 0, 0, 1, 1, 1], "mat": "cargo", "flags": 0, "door": 0}],
           "anchors": []}
    assert parse(json.dumps(kit))["parts"][0]["mat"] == "cargo"
    # An awning drops 0.5 m to its street edge and sags midway along z.
    aw = {"b": [4.2, 2.7, 0, 8.6, 2.9, 6], "mat": "canvas", "flags": ROOF | DECOR, "door": 0}
    assert abs(underside(aw, 4.2, 0) - 2.4) < 1e-9 and abs(underside(aw, 8.6, 6) - 2.9) < 1e-9
    assert abs(underside(aw, 8.6, 3) - 2.78) < 1e-9
    # A leg is carried up to the roof over it; a short post or a bare one is not.
    leg = {"b": [0, 0, 0, 0.4, 8, 0.4], "mat": "timber", "flags": 0, "door": 0}
    deck = {"b": [-1, 7, -1, 2, 7.3, 2], "mat": "timber", "flags": DECOR, "door": 0}
    roof = {"b": [-1, 9.6, -1, 2, 9.9, 2], "mat": "sheet", "flags": DECOR, "door": 0}
    assert reach(leg, [leg, deck, roof])[4] == 9.6
    assert reach(leg, [leg, deck])[4] == 8
    stub = {"b": [0, 0, 0, 0.2, 1.5, 0.2], "mat": "steel", "flags": 0, "door": 0}
    assert reach(stub, [stub, roof])[4] == 1.5
    assert above([deck, roof], 0.5, 0.5, 9.25) == 9.6 and below([deck, roof], 0.5, 0.5, 9.0) == 7.3
    print("site_kit self-test: OK")


def parse(text):
    kit = json.loads(text)
    for p in kit["parts"]:
        b = p["b"]
        assert b[0] < b[3] and b[1] < b[4] and b[2] < b[5], p
        assert p["mat"] in ROLES, p
    return kit


# ── the Blender half ────────────────────────────────────────────────────────


class Soup:
    """One bmesh per role, built in kit space; tints ride a colour layer."""

    def __init__(self, bpy, bmesh):
        self.bpy, self.bmesh = bpy, bmesh
        self.bm = {r: bmesh.new() for r in ROLES}
        self.tint = {r: self.bm[r].loops.layers.float_color.new("tint") for r in ROLES}

    def face(self, role, pts, tint=(1, 1, 1)):
        bm = self.bm[role]
        vs = [bm.verts.new(kit_to_blender(p)) for p in pts]
        f = bm.faces.new(vs)
        lay = self.tint[role]
        for loop in f.loops:
            loop[lay] = (tint[0], tint[1], tint[2], 1.0)
        return f

    def box(self, role, b, tint=(1, 1, 1), bevel=0.0):
        x0, y0, z0, x1, y1, z1 = b
        if x1 - x0 <= 1e-4 or y1 - y0 <= 1e-4 or z1 - z0 <= 1e-4:
            return
        bm = self.bm[role]
        c = [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1),
             (x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]
        # Counter-clockwise seen from outside, in the kit's right-handed
        # frame: an engine culls back faces, so a box wound the other way
        # draws inside out (and its AO bake shoots into itself).
        quads = [(0, 1, 2, 3), (4, 7, 6, 5), (0, 4, 5, 1), (2, 6, 7, 3), (1, 5, 6, 2), (3, 7, 4, 0)]
        vs = [bm.verts.new(kit_to_blender(p)) for p in c]
        faces = []
        lay = self.tint[role]
        for q in quads:
            f = bm.faces.new([vs[i] for i in q])
            for loop in f.loops:
                loop[lay] = (tint[0], tint[1], tint[2], 1.0)
            faces.append(f)
        if bevel > 0.0:
            edges = list({e for f in faces for e in f.edges})
            m = min(x1 - x0, y1 - y0, z1 - z0)
            self.bmesh.ops.bevel(bm, geom=edges, offset=min(bevel, m * 0.3), segments=1,
                                 affect="EDGES", clamp_overlap=True)

    def prism(self, role, a, b, r, sides=6, tint=(1, 1, 1)):
        """A thin tube from point `a` to `b` (cables, pipes, lamp arms)."""
        ax, ay, az = a
        bx, by, bz = b
        d = (bx - ax, by - ay, bz - az)
        ln = math.sqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2)
        if ln < 1e-4:
            return
        d = (d[0] / ln, d[1] / ln, d[2] / ln)
        up = (0, 1, 0) if abs(d[1]) < 0.9 else (1, 0, 0)
        u = (d[1] * up[2] - d[2] * up[1], d[2] * up[0] - d[0] * up[2], d[0] * up[1] - d[1] * up[0])
        un = math.sqrt(sum(x * x for x in u))
        u = tuple(x / un for x in u)
        v = (d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0])
        ring = []
        for k in range(sides):
            t = 2 * math.pi * k / sides
            ring.append(tuple(r * (math.cos(t) * u[i] + math.sin(t) * v[i]) for i in range(3)))
        for k in range(sides):
            k2 = (k + 1) % sides
            p0 = tuple(a[i] + ring[k][i] for i in range(3))
            p1 = tuple(a[i] + ring[k2][i] for i in range(3))
            p2 = tuple(b[i] + ring[k2][i] for i in range(3))
            p3 = tuple(b[i] + ring[k][i] for i in range(3))
            self.face(role, [p0, p1, p2, p3], tint)

    def objects(self, name):
        bpy = self.bpy
        out = []
        for role, bm in self.bm.items():
            if not bm.faces:
                bm.free()
                continue
            self.bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
            mesh = bpy.data.meshes.new(role)
            bm.to_mesh(mesh)
            bm.free()
            obj = bpy.data.objects.new(role, mesh)
            bpy.context.collection.objects.link(obj)
            mat = bpy.data.materials.get(role) or bpy.data.materials.new(role)
            mesh.materials.append(mat)
            out.append(obj)
        return out


def corrugated_wall(s, role, b, axis, tint, rng):
    """A sheet face on both long sides of a thin wall box, ribbed along
    `axis` (0 = x, 2 = z), with posts at the bay joints."""
    x0, y0, z0, x1, y1, z1 = b
    s.box(role, b, tint)
    lo, hi = (x0, x1) if axis == 0 else (z0, z1)
    prof = rib_profile(hi - lo)
    for side in (0, 1):
        for i in range(len(prof) - 1):
            (a0, o0), (a1, o1) = prof[i], prof[i + 1]
            if axis == 0:
                zf = (z0 - o0, z0 - o1) if side == 0 else (z1 + o0, z1 + o1)
                pts = [(lo + a0, y0 + 0.05, zf[0]), (lo + a1, y0 + 0.05, zf[1]),
                       (lo + a1, y1 - 0.05, zf[1]), (lo + a0, y1 - 0.05, zf[0])]
                if side == 0:
                    pts.reverse()
            else:
                xf = (x0 - o0, x0 - o1) if side == 0 else (x1 + o0, x1 + o1)
                pts = [(xf[0], y0 + 0.05, lo + a0), (xf[1], y0 + 0.05, lo + a1),
                       (xf[1], y1 - 0.05, lo + a1), (xf[0], y1 - 0.05, lo + a0)]
                if side == 1:
                    pts.reverse()
            s.face(role, pts, tint)


CARGO_TINTS = [(0.84, 0.57, 0.40), (0.47, 0.62, 0.59), (0.64, 0.65, 0.61),
               (0.72, 0.30, 0.24), (0.30, 0.42, 0.58), (0.80, 0.70, 0.36)]


def container(s, b, rng):
    """One shipping container filling box `b`: a shell, ribbed long faces,
    corner castings and door bars on one end."""
    x0, y0, z0, x1, y1, z1 = b
    tint = rng.choice(CARGO_TINTS)
    grime = 0.85 + 0.15 * rng.random()
    tint = tuple(c * grime for c in tint)
    s.box("cargo", (x0 + 0.02, y0, z0 + 0.02, x1 - 0.02, y1 - 0.02, z1 - 0.02), tint)
    along_x = (x1 - x0) >= (z1 - z0)
    length = (x1 - x0) if along_x else (z1 - z0)
    prof = rib_profile(length - 0.4, pitch=0.28, depth=0.03)
    for side in (0, 1):
        for i in range(len(prof) - 1):
            (a0, o0), (a1, o1) = prof[i], prof[i + 1]
            if along_x:
                zf = (z0 - o0, z0 - o1) if side == 0 else (z1 + o0, z1 + o1)
                pts = [(x0 + 0.2 + a0, y0 + 0.15, zf[0]), (x0 + 0.2 + a1, y0 + 0.15, zf[1]),
                       (x0 + 0.2 + a1, y1 - 0.15, zf[1]), (x0 + 0.2 + a0, y1 - 0.15, zf[0])]
                if side == 0:
                    pts.reverse()
            else:
                xf = (x0 - o0, x0 - o1) if side == 0 else (x1 + o0, x1 + o1)
                pts = [(xf[0], y0 + 0.15, z0 + 0.2 + a0), (xf[1], y0 + 0.15, z0 + 0.2 + a1),
                       (xf[1], y1 - 0.15, z0 + 0.2 + a1), (xf[0], y1 - 0.15, z0 + 0.2 + a0)]
                if side == 1:
                    pts.reverse()
            s.face("cargo", pts, tint)
    c = 0.17
    steel = (0.35, 0.37, 0.38)
    for cx in (x0, x1 - c):
        for cy in (y0, y1 - c):
            for cz in (z0, z1 - c):
                s.box("steel", (cx, cy, cz, cx + c, cy + c, cz + c), steel)
    # Door bars on the +end.
    for t in (0.2, 0.4, 0.6, 0.8):
        if along_x:
            zz = z0 + (z1 - z0) * t
            s.box("steel", (x1 - 0.02, y0 + 0.1, zz - 0.03, x1 + 0.03, y1 - 0.1, zz + 0.03), steel)
        else:
            xx = x0 + (x1 - x0) * t
            s.box("steel", (xx - 0.03, y0 + 0.1, z1 - 0.02, xx + 0.03, y1 - 0.1, z1 + 0.03), steel)


def container_stack(s, b, rng):
    """A wall box filled with containers two high, end to end along it."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    length = (x1 - x0) if along_x else (z1 - z0)
    base = max(y0, 0.0)
    # The footing below the floor is drawn as a plain concrete plinth.
    if y0 < 0.0:
        s.box("concrete", (x0, y0, z0, x1, 0.0, z1), (0.8, 0.8, 0.8))
    rows = max(1, int(round((y1 - base) / CONTAINER_H)))
    h = (y1 - base) / rows
    n = max(1, int(round(length / CONTAINER_L)))
    step = length / n
    for r in range(rows):
        for i in range(n):
            a = i * step
            if along_x:
                cb = (x0 + a, base + r * h, z0, x0 + a + step, base + (r + 1) * h, z1)
            else:
                cb = (x0, base + r * h, z0 + a, x1, base + (r + 1) * h, z0 + a + step)
            container(s, cb, rng)


def ashlar(s, b, rng, band_every=6.0):
    """Dark ancient stone: courses of chamfered blocks with gilt bands and a
    glowing lapis seam — the pylons, the dais, the ziggurat's terraces."""
    x0, y0, z0, x1, y1, z1 = b
    course = 1.5
    y = y0
    k = 0
    while y < y1 - 1e-3:
        top = min(y1, y + course)
        j = 0.03 * (rng.random() - 0.5)
        dark = 0.85 + 0.2 * rng.random()
        s.box("obsidian", (x0 - j, y, z0 - j, x1 + j, top, z1 + j), (dark, dark, dark), bevel=0.06)
        y = top
        k += 1
        if band_every and k * course % band_every < 1e-3 and top < y1 - 0.5:
            s.box("gilt", (x0 - 0.06, top - 0.12, z0 - 0.06, x1 + 0.06, top + 0.12, z1 + 0.06),
                  (0.95, 0.8, 0.5), bevel=0.02)
    tall = (y1 - y0) > 8.0
    if tall:
        # A lapis seam up each broad face.
        wide_x = (x1 - x0) >= (z1 - z0)
        for side in (0, 1):
            if wide_x:
                zz = z0 - 0.04 if side == 0 else z1 + 0.04
                cx = (x0 + x1) / 2
                s.box("lapis", (cx - 0.1, y0 + 1.0, min(zz, zz + 0.02), cx + 0.1, y1 - 1.0, max(zz, zz + 0.02)))
            else:
                xx = x0 - 0.04 if side == 0 else x1 + 0.04
                cz = (z0 + z1) / 2
                s.box("lapis", (min(xx, xx + 0.02), y0 + 1.0, cz - 0.1, max(xx, xx + 0.02), y1 - 1.0, cz + 0.1))


def planks(s, b, rng):
    x0, y0, z0, x1, y1, z1 = b
    dx, dy, dz = x1 - x0, y1 - y0, z1 - z0
    tint = lambda: tuple([0.8 + 0.25 * rng.random()] * 3)
    if dy > max(dx, dz) * 2:
        s.box("timber", b, tint(), bevel=0.02)  # a post
        return
    along_x = dx >= dz
    w = 0.24
    span = dz if along_x else dx
    n = max(1, int(span / w))
    step = span / n
    for i in range(n):
        g = 0.008
        if along_x:
            s.box("timber", (x0, y0, z0 + i * step + g, x1, y1, z0 + (i + 1) * step - g), tint(), bevel=0.01)
        else:
            s.box("timber", (x0 + i * step + g, y0, z0, x0 + (i + 1) * step - g, y1, z1), tint(), bevel=0.01)


def awning(s, b, rng):
    """A sagging striped awning filling decor box `b`, sloping toward the
    side it overhangs (the lower |x| edge faces the street)."""
    x0, y0, z0, x1, y1, z1 = b
    stripes = [(0.78, 0.66, 0.48), (0.62, 0.22, 0.18)] if rng.random() < 0.5 else [(0.78, 0.66, 0.48), (0.2, 0.33, 0.52)]
    nx, nz = 6, 12
    def pt(i, k):
        u, v = i / nx, k / nz
        return (x0 + (x1 - x0) * u, awning_uv(b, u, v), z0 + (z1 - z0) * v)
    for i in range(nx):
        for k in range(nz):
            t = stripes[k % 2]
            q = [pt(i, k), pt(i + 1, k), pt(i + 1, k + 1), pt(i, k + 1)]
            s.face("canvas", q, t)
            s.face("canvas", list(reversed(q)), t)


def roof_sheet(s, b):
    x0, y0, z0, x1, y1, z1 = b
    s.box("sheet", b, (0.66, 0.68, 0.7))
    prof = rib_profile(z1 - z0, pitch=0.25, depth=0.03)
    for i in range(len(prof) - 1):
        (a0, o0), (a1, o1) = prof[i], prof[i + 1]
        s.face("sheet", [(x0, y1 + o0, z0 + a0), (x0, y1 + o1, z0 + a1),
                         (x1, y1 + o1, z0 + a1), (x1, y1 + o0, z0 + a0)][::-1], (0.66, 0.68, 0.7))


def ring(s, parts):
    """The broken gilt ring: fit the circle the decor boxes sit on and sweep
    a squared section through the arcs they cover."""
    pts = []
    for p in parts:
        b = p["b"]
        pts.append(((b[0] + b[3]) / 2, (b[1] + b[4]) / 2))
    if len(pts) < 3:
        return
    cx, cy, r = fit_circle(pts)
    angles = sorted((math.degrees(math.atan2(y - cy, x - cx)) % 360) for x, y in pts)
    # Arcs: consecutive decor boxes within 35° are one unbroken run.
    runs, run = [], [angles[0]]
    for a in angles[1:]:
        if a - run[-1] <= 35:
            run.append(a)
        else:
            runs.append(run)
            run = [a]
    runs.append(run)
    if len(runs) > 1 and (runs[0][0] + 360) - runs[-1][-1] <= 35:
        runs[0] = [a - 360 for a in runs[-1]] + runs[0]
        runs.pop()
    gold = (0.95, 0.78, 0.45)
    half = 0.6
    for run in runs:
        a0, a1 = run[0] - 12, run[-1] + 12
        n = max(2, int((a1 - a0) / 5))
        for i in range(n):
            t0 = math.radians(a0 + (a1 - a0) * i / n)
            t1 = math.radians(a0 + (a1 - a0) * (i + 1) / n)
            def at(t, rr, zz):
                return (cx + rr * math.cos(t), cy + rr * math.sin(t), zz)
            ri, ro = r - half, r + half
            quads = [
                [at(t0, ro, -half), at(t1, ro, -half), at(t1, ro, half), at(t0, ro, half)],
                [at(t0, ri, half), at(t1, ri, half), at(t1, ri, -half), at(t0, ri, -half)],
                [at(t0, ri, half), at(t0, ro, half), at(t1, ro, half), at(t1, ri, half)],
                [at(t1, ri, -half), at(t1, ro, -half), at(t0, ro, -half), at(t0, ri, -half)],
            ]
            for q in quads:
                s.face("gilt", q, gold)
            # A lapis line round the inner face.
            li = ri - 0.02
            s.face("lapis", [at(t0, li, 0.12), at(t1, li, 0.12), at(t1, li, -0.12), at(t0, li, -0.12)])
            # Glyph plates on the outer face every other segment.
            if i % 2 == 0:
                lo = ro + 0.03
                s.face("gilt", [at(t0 + 0.01, lo, -0.35), at(t1 - 0.01, lo, -0.35),
                                at(t1 - 0.01, lo, 0.35), at(t0 + 0.01, lo, 0.35)], (0.7, 0.55, 0.3))
        # Jagged broken ends.
        for a in (a0, a1):
            t = math.radians(a)
            for k in range(3):
                j = (k - 1) * 0.3
                p = (cx + (r + j) * math.cos(t), cy + (r + j) * math.sin(t), 0.0)
                s.box("gilt", (p[0] - 0.25, p[1] - 0.25, -0.5 + 0.2 * k, p[0] + 0.25, p[1] + 0.25, 0.0 + 0.2 * k), gold)


def catenary(a, b, sag, n=12):
    out = []
    for i in range(n + 1):
        t = i / n
        out.append((a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t - sag * 4 * t * (1 - t),
                    a[2] + (b[2] - a[2]) * t))
    return out


def lights(s, parts, anchors):
    lamps = [a["at"] for a in anchors if a["kind"] == "lamp"]
    steel = (0.3, 0.32, 0.33)
    for (x, y, z) in lamps:
        # A hooded head and its bulb, on a rod up to whatever is over it or
        # else down to whatever is under it (a street lamp's head already
        # sits on its post).
        s.box("steel", (x - 0.35, y - 0.12, z - 0.2, x + 0.35, y + 0.05, z + 0.2), steel, bevel=0.02)
        s.box("bulb", (x - 0.14, y - 0.24, z - 0.1, x + 0.14, y - 0.12, z + 0.1))
        up = above(parts, x, z, y + 0.05)
        down = below(parts, x, z, y - 0.12)
        if up is not None and up - (y + 0.05) <= REACH_M:
            s.prism("steel", (x, y + 0.05, z), (x, up + 0.02, z), 0.025, 6, steel)
        elif down is not None and (y - 0.12) - down <= REACH_M:
            s.prism("steel", (x, down, z), (x, y - 0.12, z), 0.03, 6, steel)
    # String lights along the market street: between consecutive lamps on
    # the street (|x| < 7), zig-zag across.
    street = sorted([l for l in lamps if abs(l[0]) < 7 and l[1] < 6], key=lambda l: l[2])
    for a, b in zip(street, street[1:]):
        a2 = (a[0], a[1] - 0.2, a[2])
        b2 = (-b[0], b[1] - 0.2, b[2]) if a[0] * b[0] > 0 else (b[0], b[1] - 0.2, b[2])
        pts = catenary(a2, b2, 1.2, 16)
        for p, q in zip(pts, pts[1:]):
            s.prism("steel", p, q, 0.012, 4, steel)
        for k, p in enumerate(pts[1:-1]):
            if k % 2 == 0:
                s.box("bulb", (p[0] - 0.05, p[1] - 0.12, p[2] - 0.05, p[0] + 0.05, p[1] - 0.02, p[2] + 0.05))


def poles(s, parts, anchors):
    """Tarp poles (`town.rs` `POLES`): up from the floor, through the table
    each stands in, to the canvas over it."""
    for a in anchors:
        if a["kind"] != "pole":
            continue
        x, y, z = a["at"]
        top = above(parts, x, z, HEAD_M)
        if top is not None:
            s.box("timber", (x - 0.07, y, z - 0.07, x + 0.07, top + 0.02, z + 0.07), (0.92, 0.92, 0.92), bevel=0.02)


def sign(s, text, beam, outward):
    """Block letters across a beam's outward face (`outward` = the axis and
    sign it faces: 'z+', 'z-', 'x+', 'x-')."""
    x0, y0, z0, x1, y1, z1 = beam
    cells = glyph_cells(text)
    cols = len(text) * 6 - 1
    span = (x1 - x0) if outward[0] == "z" else (z1 - z0)
    step = min(span * 0.9 / cols, (y1 - y0) * 0.75 / 7)
    top = (y0 + y1) / 2 + 3.5 * step
    left = -(cols * step) / 2
    gold = (0.95, 0.8, 0.45)
    for (c, r) in cells:
        u0 = left + c * step
        v1 = top - r * step
        # Reading left to right for someone standing outside, facing in:
        # facing -z their right is +x, facing +z it is -x, facing -x it is
        # -z and facing +x it is +z.
        cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
        if outward == "z+":
            s.box("gilt", (cx + u0, v1 - step, z1, cx + u0 + step, v1, z1 + 0.06), gold)
        elif outward == "z-":
            s.box("gilt", (cx - u0 - step, v1 - step, z0 - 0.06, cx - u0, v1, z0), gold)
        elif outward == "x+":
            s.box("gilt", (x1, v1 - step, cz - u0 - step, x1 + 0.06, v1, cz - u0), gold)
        else:
            s.box("gilt", (x0 - 0.06, v1 - step, cz + u0, x0, v1, cz + u0 + step), gold)


def terraces(s, rng):
    """The ziggurat's trim: a gilt cornice under each terrace's lip and a row
    of lapis glyph tiles along each face, broken where the stairs climb."""
    gold = (0.95, 0.78, 0.45)
    for k, hw in enumerate((32.0, 26.0, 20.0, 14.0, 8.0)):
        top = 5.0 * (k + 1)
        o = hw + 0.05
        for side in range(4):
            # Each face as segments along its length, skipping the stairs.
            segs = [(-hw, -3.2), (3.2, hw)] if side == 0 else [(-hw, hw)]
            for a0, a1 in segs:
                if side in (0, 1):
                    zz = -o if side == 0 else o
                    s.box("gilt", (a0, top - 0.45, min(zz, zz * 1.0 + 0.0) - 0.1, a1, top - 0.2, zz + 0.1), gold, bevel=0.02)
                else:
                    xx = -o if side == 2 else o
                    s.box("gilt", (xx - 0.1, top - 0.45, a0, xx + 0.1, top - 0.2, a1), gold, bevel=0.02)
                # Glyph tiles every 3 m at mid-height.
                n = int((a1 - a0) / 3.0)
                for i in range(n):
                    c = a0 + (i + 0.5) * (a1 - a0) / n
                    y = top - 2.6
                    lit = rng.random() < 0.35
                    role = "lapis" if lit else "gilt"
                    col = (1, 1, 1) if lit else (0.62, 0.48, 0.26)
                    if side in (0, 1):
                        zz = -o - 0.04 if side == 0 else o + 0.04
                        s.box(role, (c - 0.35, y - 0.35, min(zz, zz + 0.03), c + 0.35, y + 0.35, max(zz, zz + 0.03)), col)
                    else:
                        xx = -o - 0.04 if side == 2 else o + 0.04
                        s.box(role, (min(xx, xx + 0.03), y - 0.35, c - 0.35, max(xx, xx + 0.03), y + 0.35, c + 0.35), col)


def dress(s, kit, rng, label):
    parts = kit["parts"]
    # The town's broken ring is fitted from its gilt decor; elsewhere gilt
    # decor (the ziggurat's crown) is shards, drawn as boxes.
    ring_parts = [p for p in parts if p["mat"] == "gilt" and p["flags"] & DECOR
                  and kit["name"] == "town"]
    for p in parts:
        b = reach(p, parts)
        mat, flags = p["mat"], p["flags"]
        if p.get("door", 0):
            continue  # doors are drawn by the client, which animates them
        if p in ring_parts:
            continue
        if mat == "cargo":
            if flags & STACK:
                container_stack(s, b, rng)
            else:
                container(s, b, rng)
        elif mat == "obsidian":
            ashlar(s, b, rng, band_every=6.0 if b[4] - b[1] > 8 else 0.0)
        elif mat == "timber":
            planks(s, b, rng)
        elif mat == "canvas":
            if flags & DECOR:
                awning(s, b, rng)
            else:
                s.box("canvas", b, (0.78, 0.66, 0.48))
        elif mat == "sheet":
            if flags & (ROOF | DECOR):
                roof_sheet(s, b)
            else:
                axis = 0 if (b[3] - b[0]) >= (b[5] - b[2]) else 2
                corrugated_wall(s, "sheet", b, axis, (0.62, 0.78, 0.80), rng)
        elif mat == "steel":
            s.box("steel", b, (0.37, 0.42, 0.43), bevel=0.02)
        elif mat == "gilt":
            s.box("gilt", b, (0.95, 0.78, 0.45), bevel=0.05)
        elif mat == "yard":
            s.box("yard", b, (0.56, 0.59, 0.62))
        else:
            s.box(mat, b, (1, 1, 1), bevel=0.03 if mat == "concrete" else 0.0)
    ring(s, ring_parts)
    # The sign on every gate beam: steel beams whose top is above 6 m and
    # span a gate (one axis ≥ 10 m, above head height).
    for p in parts:
        b = p["b"]
        if p["mat"] != "steel" or b[1] < 6.0:
            continue
        dx, dz = b[3] - b[0], b[5] - b[2]
        if dx >= 10 and dz < 2:
            sign(s, label, b, "z+" if b[5] > 0 else "z-")
        elif dz >= 10 and dx < 2:
            sign(s, label, b, "x+" if b[3] > 0 else "x-")
    lights(s, parts, kit["anchors"])
    poles(s, parts, kit["anchors"])
    if kit["name"] == "ziggurat":
        terraces(s, rng)


def bake_ao(bpy, objs, samples):
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = samples
    for obj in objs:
        mesh = obj.data
        ao = mesh.color_attributes.new("ao", "FLOAT_COLOR", "CORNER")
        mesh.color_attributes.active_color = ao
        bpy.ops.object.select_all(action="DESELECT")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.bake(type="AO", target="VERTEX_COLORS")
    # Combine: Col = tint × (0.35 + 0.65 × ao).
    for obj in objs:
        mesh = obj.data
        n = len(mesh.loops)
        tint = [0.0] * (n * 4)
        aov = [0.0] * (n * 4)
        mesh.attributes["tint"].data.foreach_get("color", tint)
        mesh.attributes["ao"].data.foreach_get("color", aov)
        col = mesh.color_attributes.new("Col", "FLOAT_COLOR", "CORNER")
        out = [0.0] * (n * 4)
        # Per-corner AO smears into triangles across a face tens of metres
        # long; the dressed stone's faces are, so the stone goes without.
        flat = obj.name.startswith("obsidian")
        for i in range(n):
            a = 1.0 if flat else 0.35 + 0.65 * aov[i * 4]
            out[i * 4 + 0] = tint[i * 4 + 0] * a
            out[i * 4 + 1] = tint[i * 4 + 1] * a
            out[i * 4 + 2] = tint[i * 4 + 2] * a
            out[i * 4 + 3] = 1.0
        col.data.foreach_set("color", out)
        mesh.color_attributes.remove(mesh.color_attributes["ao"])
        mesh.color_attributes.remove(mesh.color_attributes["tint"])
        mesh.color_attributes.active_color = mesh.color_attributes["Col"]
        mesh.color_attributes.render_color_index = 0


def uv_metres(bpy, objs):
    """Box-projected UVs in metres, per face by its dominant axis. Sheet-like
    roles run their v up vertical faces, so the photographed ribs stand."""
    import bmesh
    for obj in objs:
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        uv = bm.loops.layers.uv.new("UVMap")
        vertical_ribs = obj.name in ("sheet", "cargo", "steel")
        for f in bm.faces:
            n = f.normal
            ax, ay, az = abs(n.x), abs(n.y), abs(n.z)
            for loop in f.loops:
                co = loop.vert.co  # Blender space: z is up
                if az >= ax and az >= ay:
                    u, v = co.x, co.y
                elif ax >= ay:
                    u, v = co.y, co.z
                else:
                    u, v = co.x, co.z
                if vertical_ribs and not (az >= ax and az >= ay):
                    u, v = v, u
                loop[uv].uv = (u, v)
        bm.to_mesh(obj.data)
        bm.free()


def gen(args):
    try:
        import bpy  # noqa: F401
        import bmesh
    except ImportError:
        print("SKIP: bpy is not installed — pip install 'bpy==4.5.*' (Python 3.11)")
        return 2
    import bpy
    kit = parse(open(args.kit).read())
    rng = random.Random(args.seed)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    s = Soup(bpy, bmesh)
    dress(s, kit, rng, args.label)
    objs = s.objects(kit["name"])
    uv_metres(bpy, objs)
    for obj in objs:
        for poly in obj.data.polygons:
            poly.use_smooth = False
    if args.ao_samples > 0:
        bake_ao(bpy, objs, args.ao_samples)
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=args.out, export_format="GLB", use_selection=True, export_apply=True,
        export_texcoords=True, export_normals=True, export_tangents=True,
        export_materials="EXPORT", export_vertex_color="ACTIVE", export_yup=True,
        export_animations=False, export_skins=False, export_morph=False,
        export_lights=False, export_cameras=False, export_extras=False,
    )
    tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in objs)
    side = {
        "kit": os.path.relpath(args.kit), "seed": args.seed, "label": args.label,
        "blender": bpy.app.version_string, "python": sys.version.split()[0],
        "script_sha256": hashlib.sha256(open(__file__, "rb").read()).hexdigest(),
        "kit_sha256": hashlib.sha256(open(args.kit, "rb").read()).hexdigest(),
        "triangles": tris, "meshes": sorted(o.name for o in objs),
        "bytes": os.path.getsize(args.out),
    }
    with open(args.out + ".json", "w") as f:
        json.dump(side, f, indent=2)
        f.write("\n")
    print(json.dumps(side, indent=2))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--self-test", action="store_true", help="prove the bpy-free arithmetic")
    sub = ap.add_subparsers(dest="cmd")
    g = sub.add_parser("gen", help="dress a kit into a GLB")
    g.add_argument("--kit", required=True)
    g.add_argument("--out", required=True)
    g.add_argument("--seed", type=int, default=1)
    g.add_argument("--label", default="THE GATE")
    g.add_argument("--ao-samples", type=int, default=24)
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.cmd == "gen":
        return gen(args)
    ap.error("a command (gen) or --self-test")


if __name__ == "__main__":
    sys.exit(main())
