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
         "gilt", "canvas", "lapis", "bulb", "ashlar", "leaf", "paint", "ivy")
# A landmark kit's boxes also name the old masonry and natural rock; neither
# is a mesh role (`stone` dresses to `ashlar`, `rock` is the client's).
KIT_MATS = ROLES + ("stone", "rock")
# Roles whose pieces each take their own patch of the photograph, so a wall
# reads as many stones and a deck as many boards rather than one sheet.
PATCHED = ("ashlar", "timber")
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


def patch_offset(piece):
    """A piece's own patch of a tiling photograph, metres: a hash of its id,
    spread over 20 m so no two neighbours share one."""
    h = (piece * 2654435761) & 0xFFFFFFFF
    return ((h & 0xFFFF) / 65536.0 * 20.0, (h >> 16) / 65536.0 * 20.0)


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
        assert p["mat"] in KIT_MATS, p
    return kit


# ── the Blender half ────────────────────────────────────────────────────────


class Soup:
    """One bmesh per role, built in kit space; tints ride a colour layer."""

    def __init__(self, bpy, bmesh):
        self.bpy, self.bmesh = bpy, bmesh
        self.bm = {r: bmesh.new() for r in ROLES}
        self.tint = {r: self.bm[r].loops.layers.float_color.new("tint") for r in ROLES}
        self.pid = {r: self.bm[r].faces.layers.int.new("piece") for r in ROLES}
        self.piece = 0

    def new_piece(self):
        """Start a new piece: its faces share one patch of the photograph."""
        self.piece += 1
        return self.piece

    def face(self, role, pts, tint=(1, 1, 1)):
        bm = self.bm[role]
        vs = [bm.verts.new(kit_to_blender(p)) for p in pts]
        f = bm.faces.new(vs)
        f[self.pid[role]] = self.piece
        lay = self.tint[role]
        for loop in f.loops:
            loop[lay] = (tint[0], tint[1], tint[2], 1.0)
        return f

    def box(self, role, b, tint=(1, 1, 1), bevel=0.0):
        x0, y0, z0, x1, y1, z1 = b
        if x1 - x0 <= 1e-4 or y1 - y0 <= 1e-4 or z1 - z0 <= 1e-4:
            return
        self.new_piece()
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
            f[self.pid[role]] = self.piece
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
            # Log ends and stains are n-gons; the exporter's tangents want
            # triangles and quads.
            ngons = [f for f in bm.faces if len(f.verts) > 4]
            if ngons:
                self.bmesh.ops.triangulate(bm, faces=ngons)
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
    if kit["name"] in MARKS:
        MARKS[kit["name"]](s, kit, rng)
        return
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


# ── the landmarks ───────────────────────────────────────────────────────────
#
# `sim_core::landmark`'s built kinds (`mark_ruin`, `mark_stones`, `mark_mast`,
# `mark_tower`, `mark_yard`). The site law holds: below head height the
# dressing stays on its part's faces (±5 cm), and only what is out of reach
# leaves a box. A landmark's base is its centre's ground and the slope around
# it runs a few metres either way, so nothing is laid on y = 0 outside a part
# either: a part runs down to its footing, and a pebble beside it would float
# on the downhill side.

STONE = (0.80, 0.77, 0.72)
MORTAR = (0.42, 0.40, 0.37)
JOINT_M = 0.035
GALV = (0.58, 0.60, 0.61)
# A cast slab on the fine-grained yard surface, lifted from its gravel tint.
SLAB = (0.92, 0.91, 0.88)
MAST_RED = (0.62, 0.16, 0.12)
MAST_WHITE = (0.86, 0.85, 0.80)
CRANE = (0.78, 0.60, 0.16)


def tangents(axis, sign):
    """The two in-plane axes of the face `axis` = ±, ordered so t1 × t2 is
    the outward normal: a (u, v) quad listed counter-clockwise is front-facing."""
    return {(0, 1): (1, 2), (0, -1): (2, 1), (1, 1): (2, 0), (1, -1): (0, 2),
            (2, 1): (0, 1), (2, -1): (1, 0)}[(axis, sign)]


def course_lines(rng, y0=-1.5, y1=24.0, lo=0.6, hi=0.9):
    """One set of bed joints for a whole kit, so walls that meet share them.
    Below `y0` a footing is one deep course: only a steep slope shows it."""
    out, y = [y0], y0
    while y < y1:
        y += rng.uniform(lo, hi)
        out.append(y)
    return out


def courses(lines, y0, y1, least=0.3):
    """The courses of a part from `y0` to `y1` on the kit's bed joints; a
    sliver at either end joins its neighbour."""
    ys = [y0] + [y for y in lines if y0 + least < y < y1 - least] + [y1]
    return list(zip(ys, ys[1:]))


def joints(lo, hi, rng, short=0.8, long=1.9):
    """Head joints along one course: blocks of random length, the last one
    stretched or merged so none is a sliver."""
    xs, x = [lo], lo + rng.uniform(0.2, 1.0) * short
    while x < hi - short * 0.6:
        xs.append(x)
        x += rng.uniform(short, long)
    xs.append(hi)
    return list(zip(xs, xs[1:]))


def pillow(s, role, axis, sign, plane, r, box_edge, tint, chamfer=0.06, proud=0.0):
    """One dressed block's face on the plane `axis` = `plane` (outward
    `sign`): a face `proud` of the plane, chamfered down to joints sunk
    `JOINT_M` — except along a box edge, where it meets the next face flush.
    `r[a]` is the block's span on in-plane axis `a`, `box_edge[a]` whether
    its low and high sides lie on the box's edge."""
    t1, t2 = tangents(axis, sign)
    (u0, u1), (v0, v1) = r[t1], r[t2]
    c = min(chamfer, (u1 - u0) * 0.3, (v1 - v0) * 0.3)
    eu, ev = box_edge[t1], box_edge[t2]

    def at(u, v, d):
        p = [0.0, 0.0, 0.0]
        p[axis] = plane + sign * d
        p[t1], p[t2] = u, v
        return tuple(p)

    def rim(iu, iv):
        # A corner on a box edge is flush; any other sits in the joint.
        flush = (eu[iu] or ev[iv])
        return at((u0, u1)[iu], (v0, v1)[iv], 0.0 if flush else -JOINT_M)

    o = [rim(0, 0), rim(1, 0), rim(1, 1), rim(0, 1)]
    inner = [at(u0 + c, v0 + c, proud), at(u1 - c, v0 + c, proud),
             at(u1 - c, v1 - c, proud), at(u0 + c, v1 - c, proud)]
    s.new_piece()
    s.face(role, inner, tint)
    for i in range(4):
        j = (i + 1) % 4
        s.face(role, [o[i], o[j], inner[j], inner[i]], tint)


def stone_tint(rng, base=STONE):
    k = rng.uniform(0.74, 1.12)
    w = rng.uniform(-0.05, 0.05)
    return (base[0] * k * (1 + w), base[1] * k, base[2] * k * (1 - w))


def masonry(s, b, lines, rng, faces="xzt", slits=()):
    """A box of coursed rubble-faced blocks: a mortar core sunk behind every
    joint, and a pillow per block on each face named in `faces` (`x`, `z`:
    both of that axis's faces; `t`: the top). `slits` are (axis, sign, u, y)
    arrow slits: a block's width left as a dark slot."""
    x0, y0, z0, x1, y1, z1 = b
    k = JOINT_M + 0.005
    s.box("ashlar", (x0 + k, y0, z0 + k, x1 - k, y1 - k, z1 - k), MORTAR)
    cs = courses(lines, y0, y1)
    rows = [joints(*((x0, x1) if (x1 - x0) >= (z1 - z0) else (z0, z1)), rng) for _ in cs]
    along = 0 if (x1 - x0) >= (z1 - z0) else 2
    for axis in (0, 2):
        if "xz"[axis // 2] not in faces:
            continue
        other = 2 - axis
        lo, hi = b[other], b[other + 3]
        for sign in (-1, 1):
            plane = b[axis] if sign < 0 else b[axis + 3]
            for k_, (c0, c1) in enumerate(cs):
                # Through-stones on the long faces share a course's joints;
                # an end face is one block a course.
                if other == along:
                    spans = rows[k_]
                elif hi - lo <= 1.6:
                    spans = [(lo, hi)]
                else:
                    spans = joints(lo, hi, rng)
                for a0, a1 in spans:
                    slot = next((sl for sl in slits if sl[0] == axis and sl[1] == sign
                                 and a0 + 0.2 < sl[2] < a1 - 0.2 and c0 <= sl[3] < c1), None)
                    r = {other: (a0, a1), 1: (c0, c1)}
                    edge = {other: (abs(a0 - lo) < 1e-6, abs(a1 - hi) < 1e-6),
                            1: (k_ == 0, k_ == len(cs) - 1)}
                    if slot is None:
                        pillow(s, "ashlar", axis, sign, plane, r, edge, stone_tint(rng),
                               proud=rng.uniform(0.0, 0.025))
                    else:
                        slit(s, axis, sign, plane, r, edge, slot[2], rng)
    if "t" in faces:
        across = 2 - along
        lo, hi = b[along], b[along + 3]
        clo, chi = b[across], b[across + 3]
        cuts = [(clo, chi)] if chi - clo <= 1.6 else joints(clo, chi, rng)
        for a0, a1 in rows[-1]:
            for c0, c1 in cuts:
                r = {along: (a0, a1), across: (c0, c1), 1: (y1, y1)}
                edge = {along: (abs(a0 - lo) < 1e-6, abs(a1 - hi) < 1e-6),
                        across: (abs(c0 - clo) < 1e-6, abs(c1 - chi) < 1e-6)}
                pillow(s, "ashlar", 1, 1, y1, r, edge, stone_tint(rng), proud=rng.uniform(0.0, 0.02))


def slit(s, axis, sign, plane, r, edge, u, rng, w=0.16, depth=0.4):
    """A block with an arrow slit up its middle: two half-pillows and a dark
    slot between them, sunk into the wall (a slot a hand cannot enter, so
    the box stays the law)."""
    other = 2 - axis
    (a0, a1), (c0, c1) = r[other], r[1]
    left = {other: (a0, u - w / 2), 1: (c0, c1)}
    right = {other: (u + w / 2, a1), 1: (c0, c1)}
    pillow(s, "ashlar", axis, sign, plane, left, {other: (edge[other][0], False), 1: edge[1]},
           stone_tint(rng))
    pillow(s, "ashlar", axis, sign, plane, right, {other: (False, edge[other][1]), 1: edge[1]},
           stone_tint(rng))
    dark = (0.06, 0.06, 0.06)
    t1, t2 = tangents(axis, sign)

    def at(o, y, d):
        p = [0.0, 0.0, 0.0]
        p[axis] = plane - sign * d
        p[other], p[1] = o, y
        return tuple(p)

    lo, hi = u - w / 2, u + w / 2
    yb, yt = c0 + 0.04, c1 - 0.04
    # The slot's back and its four reveals, facing out of the wall.
    back = [at(lo, yb, depth), at(hi, yb, depth), at(hi, yt, depth), at(lo, yt, depth)]
    front = [at(lo, yb, -JOINT_M), at(hi, yb, -JOINT_M), at(hi, yt, -JOINT_M), at(lo, yt, -JOINT_M)]
    if (t1, t2) != (other, 1):
        back.reverse()
        front.reverse()
    s.new_piece()
    s.face("ashlar", back, dark)
    for i in range(4):
        j = (i + 1) % 4
        s.face("ashlar", [front[j], front[i], back[i], back[j]], (0.25, 0.24, 0.22))


def rough(s, role, b, tint, rng, amp=0.08, cell=0.45, taper=0.0, crown=0.0, out=0.03):
    """A rough stone filling box `b`: its faces gridded at `cell` and every
    vertex moved along its outward normal by noise in [-amp, out], harder at
    an edge (a worn arris) — plus an optional `taper` toward the top and a
    rounded `crown`. Shared vertices, so it stays closed."""
    from mathutils import Vector, noise
    x0, y0, z0, x1, y1, z1 = b
    n = [max(1, round((b[a + 3] - b[a]) / cell)) for a in range(3)]
    seed = Vector((rng.uniform(-500, 500), rng.uniform(-500, 500), rng.uniform(-500, 500)))
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    verts = {}

    def vert(i, j, k):
        key = (i, j, k)
        if key in verts:
            return verts[key]
        u, v, w = i / n[0], j / n[1], k / n[2]
        p = [x0 + (x1 - x0) * u, y0 + (y1 - y0) * v, z0 + (z1 - z0) * w]
        nrm = [(-1 if i == 0 else 1 if i == n[0] else 0),
               (-1 if j == 0 else 1 if j == n[1] else 0),
               (-1 if k == 0 else 1 if k == n[2] else 0)]
        edges = sum(1 for c in nrm if c)
        ln = math.sqrt(edges)
        q = Vector(p) * 0.9 + seed
        d = amp * (0.5 + 0.5 * noise.noise(q)) + 0.4 * amp * noise.noise(q * 3.1)
        d = -d * (1.0 + 0.6 * (edges - 1))
        d = min(out, d + out)
        for a in range(3):
            p[a] += nrm[a] / ln * d
        # Taper from the ground up: a standing stone narrows as it rises.
        g = max(y0, 0.0)
        t = max(0.0, (p[1] - g) / (y1 - g))
        sx = 1.0 - taper * t ** 1.4
        p[0] = cx + (p[0] - cx) * sx
        p[2] = cz + (p[2] - cz) * sx
        if crown and j == n[1]:
            rx = (p[0] - cx) / ((x1 - x0) / 2)
            rz = (p[2] - cz) / ((z1 - z0) / 2)
            p[1] -= crown * min(1.0, rx * rx + rz * rz)
        bm = s.bm[role]
        verts[key] = bm.verts.new(kit_to_blender(tuple(p)))
        return verts[key]

    bm = s.bm[role]
    lay = s.tint[role]
    s.new_piece()
    for axis in range(3):
        for sign in (-1, 1):
            t1, t2 = tangents(axis, sign)
            fixed = 0 if sign < 0 else n[axis]
            for a in range(n[t1]):
                for c in range(n[t2]):
                    quad = []
                    for (da, dc) in ((0, 0), (1, 0), (1, 1), (0, 1)):
                        idx = [0, 0, 0]
                        idx[axis] = fixed
                        idx[t1], idx[t2] = a + da, c + dc
                        quad.append(vert(*idx))
                    f = bm.faces.new(quad)
                    f[s.pid[role]] = s.piece
                    for loop in f.loops:
                        loop[lay] = (tint[0], tint[1], tint[2], 1.0)


def log(s, role, a, b, r, tint, sides=8):
    """A capped log from `a` to `b` (the prism's tube plus its two ends)."""
    s.new_piece()
    s.prism(role, a, b, r, sides, tint)
    ax, ay, az = a
    bx, by, bz = b
    d = (bx - ax, by - ay, bz - az)
    ln = math.sqrt(sum(x * x for x in d))
    d = tuple(x / ln for x in d)
    up = (0, 1, 0) if abs(d[1]) < 0.9 else (1, 0, 0)
    u = (d[1] * up[2] - d[2] * up[1], d[2] * up[0] - d[0] * up[2], d[0] * up[1] - d[1] * up[0])
    un = math.sqrt(sum(x * x for x in u))
    u = tuple(x / un for x in u)
    v = (d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0])
    ring = [tuple(r * (math.cos(2 * math.pi * k / sides) * u[i] + math.sin(2 * math.pi * k / sides) * v[i])
                  for i in range(3)) for k in range(sides)]
    end = (tint[0] * 1.15, tint[1] * 1.05, tint[2] * 0.9)
    s.face(role, [tuple(a[i] + p[i] for i in range(3)) for p in reversed(ring)], end)
    s.face(role, [tuple(b[i] + p[i] for i in range(3)) for p in ring], end)


def touches(parts, me, p):
    """Whether another part's box holds point `p` (a wall end's neighbour)."""
    return any(q is not me and q["mat"] not in ("rock",) and
               q["b"][0] - 1e-3 <= p[0] <= q["b"][3] + 1e-3 and
               q["b"][1] - 1e-3 <= p[1] <= q["b"][4] + 1e-3 and
               q["b"][2] - 1e-3 <= p[2] <= q["b"][5] + 1e-3 for q in parts)


def ragged_crown(s, p, parts, rng, lines):
    """A broken wall's top: what is left of the courses above the box, out
    of reach, stepping down toward a free end."""
    x0, y0, z0, x1, y1, z1 = p["b"]
    along = 0 if (x1 - x0) >= (z1 - z0) else 2
    lo, hi = p["b"][along], p["b"][along + 3]
    mid = [(x0 + x1) / 2, y1 - 0.5, (z0 + z1) / 2]
    free = []
    for end, step in ((lo, -0.3), (hi, 0.3)):
        q = list(mid)
        q[along] = end + step
        free.append(not touches(parts, p, q))
    level = rng.uniform(0.0, 2.0)
    for a0, a1 in joints(lo, hi, rng, 0.8, 1.7):
        level = min(2.4, max(0.0, level + rng.uniform(-0.9, 0.9)))
        t = (a0 - lo) / (hi - lo)
        cap = 2.4
        if free[0]:
            cap = min(cap, 3.0 * t)
        if free[1]:
            cap = min(cap, 3.0 * (1 - t))
        layers = int(min(level, cap))
        y = y1
        for k in range(layers):
            h = rng.uniform(0.5, 0.75)
            inset = 0.03 + 0.05 * k + rng.uniform(0, 0.05)
            bb = [x0 + inset, y, z0 + inset, x1 - inset, y + h, z1 - inset]
            bb[along], bb[along + 3] = a0 + 0.02, a1 - 0.02
            s.box("ashlar", tuple(bb), stone_tint(rng), bevel=0.06)
            y += h


def dress_ruin(s, kit, rng):
    parts = kit["parts"]
    lines = course_lines(rng)
    floor_p = next((p for p in parts if p["mat"] == "stone" and p["b"][4] <= 0.2
                    and min(p["b"][3] - p["b"][0], p["b"][5] - p["b"][2]) >= 4), None)
    walls = [p for p in parts if p["mat"] == "stone" and p["b"][4] >= 2.0 and p["b"][1] < 2.0]
    for p in parts:
        b = p["b"]
        x0, y0, z0, x1, y1, z1 = b
        h = y1 - max(y0, 0.0)
        wide = min(x1 - x0, z1 - z0)
        if p is floor_p or p["mat"] in ("yard", "timber"):
            continue
        if y1 <= 1.5:
            fallen(s, b, rng)
        elif y0 > 2.0 and h <= 1.0:
            parapet(s, b, rng, lines)
        elif wide >= 4.0:
            # The tower: masonry on its four faces, slits up two of them.
            cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
            slits = [(0, -1, cz, 4.0), (0, -1, cz, 9.0), (2, 1, cx, 6.5), (2, 1, cx, 11.0),
                     (0, 1, cz, 6.5), (2, -1, cx, 9.0)]
            ys = [c0 for c0, _ in courses(lines, y0, y1)]
            slits = [(a, sg, u, max(y for y in ys if y <= yy)) for a, sg, u, yy in slits]
            masonry(s, b, lines, rng, faces="xz", slits=slits)
        else:
            masonry(s, b, lines, rng)
            if y1 >= 2.0:
                ragged_crown(s, p, parts, rng, lines)

    rest = [p for p in parts if p is not floor_p and p["b"][1] < 2.0]
    keep = [(p["b"][0], p["b"][2], p["b"][3], p["b"][5]) for p in rest]
    # A cold fire in the court, near the lean-to's open side.
    lt = next((p["b"] for p in parts if p["mat"] == "timber"), None)
    fire = None
    if lt is not None:
        fire = (lt[3] + 1.6 if lt[0] < 0 else lt[0] - 1.6, (lt[2] + lt[5]) / 2)
        keep.append((fire[0] - 0.9, fire[1] - 0.9, fire[0] + 0.9, fire[1] + 0.9))
    if floor_p is not None:
        flagstones(s, floor_p["b"], rng, keep)
    for p in parts:
        if p["mat"] == "yard":
            rubble_heap(s, p["b"], rng)
        elif p["mat"] == "timber":
            lean_to(s, p["b"], rng)
    if floor_p is not None:
        fallen_floor(s, parts, walls, floor_p["b"], rng)
        if fire is not None:
            campfire(s, (fire[0], floor_p["b"][4], fire[1]), rng)
    for w in walls:
        ivy(s, w["b"], parts, rng)


RUBBLE = (0.74, 0.71, 0.66)
CHAR = (0.22, 0.19, 0.16)
IVY = [(0.56, 0.74, 0.34), (0.64, 0.8, 0.38), (0.46, 0.64, 0.27), (0.7, 0.82, 0.42)]


def flagstones(s, b, rng, keep_out):
    """Old flags over a floor box: irregular slabs a hair above its dark top
    (the joints), some lifted or missing to soil, weeds up the joints."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("ashlar", (x0, y0, z0, x1, y1, z1), (0.32, 0.31, 0.29))
    top = y1 + 0.004

    def blocked(ax0, az0, ax1, az1, m=0.02):
        return any(not (ax1 < k[0] - m or ax0 > k[2] + m or az1 < k[1] - m or az0 > k[3] + m) for k in keep_out)

    z = z0 + 0.02
    while z < z1 - 0.2:
        rh = rng.uniform(0.45, 0.8)
        z2 = min(z1 - 0.02, z + rh)
        x = x0 + 0.02 + rng.uniform(0, 0.3)
        while x < x1 - 0.2:
            w = rng.uniform(0.5, 1.1)
            x2 = min(x1 - 0.02, x + w)
            g = 0.018
            ax0, az0, ax1, az1 = x + g, z + g, x2 - g, z2 - g
            if not blocked(ax0, az0, ax1, az1):
                r = rng.random()
                if r < 0.12:
                    # Gone: soil and weeds where it was.
                    face_out(s, "yard", [(ax0, top, az0), (ax1, top, az0), (ax1, top, az1), (ax0, top, az1)],
                             DIRT, (ax0, top - 1, az0))
                    for _ in range(rng.randint(1, 3)):
                        tuft(s, rng.uniform(ax0, ax1), top, rng.uniform(az0, az1), rng.uniform(0.15, 0.4), rng)
                else:
                    k = rng.uniform(0.8, 1.05)
                    tint = (STONE[0] * k, STONE[1] * k * 0.99, STONE[2] * k * 0.97)
                    lift = 0.0 if r < 0.85 else rng.uniform(0.01, 0.035)
                    yt = top + 0.012 + lift
                    s.new_piece()
                    face_out(s, "ashlar", [(ax0, yt, az0), (ax1, yt, az0), (ax1, yt, az1), (ax0, yt, az1)], tint,
                             (ax0, yt - 1, az0))
                    if rng.random() < 0.18:
                        tuft(s, ax1 + g / 2, top, rng.uniform(az0, az1), rng.uniform(0.1, 0.25), rng)
            x = x2
        z = z2


def rubble_heap(s, b, rng):
    """A spoil heap filling its box: a low mound of grit and broken stone,
    the big pieces low in the middle, the small ones over and round them."""
    x0, y0, z0, x1, y1, z1 = b
    base = max(y0, 0.0)
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    hx, hz = (x1 - x0) / 2, (z1 - z0) / 2
    h = y1 - base
    rough(s, "yard", (x0 + 0.05, min(y0, base - 0.4), z0 + 0.05, x1 - 0.05, base + h * 0.55, z1 - 0.05),
          (0.62, 0.58, 0.52), rng, amp=0.12, cell=0.6, crown=h * 0.5)
    n = int(hx * hz * 14)
    for i in range(n):
        u, v = rng.uniform(-1, 1), rng.uniform(-1, 1)
        rr = max(abs(u), abs(v))
        px, pz = cx + u * (hx - 0.15), cz + v * (hz - 0.15)
        mound = base + h * 0.55 * (1 - rr ** 2)
        sz = rng.uniform(0.12, 0.42) * (1.0 - 0.6 * rr)
        py = min(y1 - sz * 0.35, mound + sz * 0.2)
        yaw = rng.uniform(0, math.pi)
        g = rng.uniform(0.82, 1.05)
        ybox(s, "ashlar", (px, py, pz), (math.cos(yaw), math.sin(yaw)), sz * rng.uniform(1.0, 1.7), sz * 0.7, sz,
             (RUBBLE[0] * g, RUBBLE[1] * g, RUBBLE[2] * g))
    for _ in range(int(hx * hz * 2)):
        tuft(s, cx + rng.uniform(-hx, hx) * 0.9, base + h * 0.3, cz + rng.uniform(-hz, hz) * 0.9,
             rng.uniform(0.2, 0.5), rng)


def fallen_floor(s, parts, walls, floor_b, rng):
    """The upper floor that was: joist stubs and empty sockets along the
    inside of the tallest wall at the old floor's height, and the beams that
    came down, their feet on the spoil heap under them."""
    heaps = [p["b"] for p in parts if p["mat"] == "yard" and p["b"][1] >= 0.0]
    tall = max(walls, key=lambda w: w["b"][4] - max(w["b"][1], 0.0) if min(w["b"][3] - w["b"][0], w["b"][5] - w["b"][2]) < 2 else -1)
    x0, y0, z0, x1, y1, z1 = tall["b"]
    along = 0 if (x1 - x0) >= (z1 - z0) else 2
    # The face toward the court.
    cc = ((floor_b[0] + floor_b[3]) / 2, (floor_b[2] + floor_b[5]) / 2)
    if along == 0:
        face, sg = (z0, -1) if cc[1] < z0 else (z1, 1)
    else:
        face, sg = (x0, -1) if cc[0] < x0 else (x1, 1)
    level = min(y1 - 1.2, 3.6)
    lo, hi = (x0, x1) if along == 0 else (z0, z1)
    u = lo + 0.8
    while u < hi - 0.5:
        stub = rng.random() < 0.55
        w = 0.22
        if stub:
            ln = rng.uniform(0.25, 0.9)
            if along == 0:
                obox(s, "timber", (u, level, face), (u, level - rng.uniform(0, 0.15), face + sg * ln), w, w, CHAR)
            else:
                obox(s, "timber", (face, level, u), (face + sg * ln, level - rng.uniform(0, 0.15), u), w, w, CHAR)
        else:
            d = 0.006
            if along == 0:
                q = [(u - w / 2, level - w / 2, face + sg * d), (u + w / 2, level - w / 2, face + sg * d),
                     (u + w / 2, level + w / 2, face + sg * d), (u - w / 2, level + w / 2, face + sg * d)]
                face_out(s, "ashlar", q, (0.12, 0.11, 0.1), (u, level, face - sg))
            else:
                q = [(face + sg * d, level - w / 2, u - w / 2), (face + sg * d, level - w / 2, u + w / 2),
                     (face + sg * d, level + w / 2, u + w / 2), (face + sg * d, level + w / 2, u - w / 2)]
                face_out(s, "ashlar", q, (0.12, 0.11, 0.1), (face - sg, level, u))
        u += rng.uniform(1.0, 1.4)
    # The beams down onto the heap nearest this wall.
    if not heaps:
        return
    hb = min(heaps, key=lambda q: abs(((q[2] + q[5]) / 2 if along == 0 else (q[0] + q[3]) / 2) - face))
    hx0, hy0, hz0, hx1, hy1, hz1 = hb
    for i in range(3):
        if along == 0:
            bx = rng.uniform(hx0 + 0.6, hx1 - 0.6)
            top = (bx + rng.uniform(-0.4, 0.4), level + rng.uniform(-0.2, 0.3), face + sg * 0.25)
            foot_z = hz1 - 0.5 if sg > 0 else hz0 + 0.5
            foot = (bx, hy1 - 0.15, foot_z)
        else:
            bz = rng.uniform(hz0 + 0.6, hz1 - 0.6)
            top = (face + sg * 0.25, level + rng.uniform(-0.2, 0.3), bz + rng.uniform(-0.4, 0.4))
            foot_x = hx1 - 0.5 if sg > 0 else hx0 + 0.5
            foot = (foot_x, hy1 - 0.15, bz)
        obox(s, "timber", foot, top, 0.22, 0.22, tuple(c * rng.uniform(0.8, 1.3) for c in CHAR))
    # Two more lying across the heap.
    for i in range(2):
        a = (rng.uniform(hx0 + 0.3, hx1 - 0.3), hy1 - 0.25, rng.uniform(hz0 + 0.3, hz1 - 0.3))
        yaw = rng.uniform(0, math.pi)
        ln = rng.uniform(1.4, 2.2)
        e = (a[0] + math.cos(yaw) * ln, a[1] - 0.1, a[2] + math.sin(yaw) * ln)
        e = (min(max(e[0], hx0 + 0.15), hx1 - 0.15), e[1], min(max(e[2], hz0 + 0.15), hz1 - 0.15))
        obox(s, "timber", a, e, 0.2, 0.2, CHAR)


def lean_to(s, b, rng):
    """A tarp over two poles against the wall, a bedroll under it, a crate
    for a table — all inside the box."""
    x0, y0, z0, x1, y1, z1 = b
    base = max(y0, 0.0)
    wall_x, open_x = (x0, x1) if abs(x0) > abs(x1) else (x1, x0)
    sg = 1 if open_x > wall_x else -1
    hi, lo = y1 - 0.05, base + 1.15
    pole_x = open_x - sg * 0.08
    for z in (z0 + 0.12, z1 - 0.12):
        cylinder(s, "timber", (pole_x, base, z), (pole_x, lo + 0.05, z), 0.045, 6, TIMBER)
    # The tarp, sagging between its edges.
    n, m = 6, 8
    pts = {}
    for i in range(n + 1):
        for k in range(m + 1):
            fx, fz = i / n, k / m
            x = wall_x + sg * 0.03 + (pole_x - wall_x - sg * 0.03) * fx
            y = hi + (lo - hi) * fx - 0.18 * math.sin(math.pi * fx) * math.sin(math.pi * fz)
            z = z0 + 0.05 + (z1 - z0 - 0.1) * fz
            pts[(i, k)] = (x, y, z)
    s.new_piece()
    tint = (0.42, 0.46, 0.34)
    for i in range(n):
        for k in range(m):
            q = [pts[(i, k)], pts[(i + 1, k)], pts[(i + 1, k + 1)], pts[(i, k + 1)]]
            face_out(s, "canvas", q, tint, (q[0][0], q[0][1] - 2.0, q[0][2]))
    # A rope tie and a batten along the wall.
    s.box("timber", (min(wall_x, wall_x + sg * 0.08), hi - 0.08, z0 + 0.02, max(wall_x, wall_x + sg * 0.08), hi + 0.02, z1 - 0.02),
          TIMBER)
    # Bedroll and a crate.
    bz = z0 + (z1 - z0) * 0.35
    bx = wall_x + sg * 0.55
    cylinder(s, "canvas", (bx, base + 0.14, bz - 0.9), (bx, base + 0.14, bz + 0.9), 0.14, 10, (0.5, 0.36, 0.26))
    cx = wall_x + sg * 0.45
    cz = z1 - 0.45
    s.box("timber", (cx - 0.3, base, cz - 0.25, cx + 0.3, base + 0.45, cz + 0.25), (0.62, 0.52, 0.4), bevel=0.02)
    cylinder(s, "steel", (cx, base + 0.45, cz), (cx, base + 0.6, cz), 0.09, 10, (0.25, 0.25, 0.25))


def campfire(s, at, rng):
    """A cold fire: a ring of stones, charred logs crossed in it, ash."""
    x, y, z = at
    pts = [(x + 0.55 * math.cos(a), y + 0.003, z + 0.55 * math.sin(a)) for a in [2 * math.pi * i / 10 for i in range(10)]]
    face_out(s, "yard", pts, (0.18, 0.17, 0.16), (x, y - 1, z))
    for i in range(9):
        a = 2 * math.pi * i / 9 + rng.uniform(-0.1, 0.1)
        sz = rng.uniform(0.14, 0.22)
        ybox(s, "ashlar", (x + 0.62 * math.cos(a), y + sz * 0.3, z + 0.62 * math.sin(a)), (math.cos(a + 1.57), math.sin(a + 1.57)),
             sz * 1.4, sz * 0.75, sz, (0.6, 0.58, 0.55))
    for i in range(4):
        a = math.pi * i / 4 + rng.uniform(-0.2, 0.2)
        r = 0.42
        obox(s, "timber", (x - r * math.cos(a), y + 0.05, z - r * math.sin(a)),
             (x + r * math.cos(a), y + 0.08 + 0.04 * i, z + r * math.sin(a)), 0.08, 0.08, CHAR)


def ivy(s, b, parts, rng):
    """Ivy up a wall's faces: strands climbing from the foot, wandering and
    branching, thinning as they rise, leaves flat to the stone within a few
    centimetres of it. More on the faces that look away from the court."""
    x0, y0, z0, x1, y1, z1 = b
    base = 0.0
    for axis, sign in ((0, -1), (0, 1), (2, -1), (2, 1)):
        plane = (x1 if sign > 0 else x0) if axis == 0 else (z1 if sign > 0 else z0)
        t = 2 if axis == 0 else 0
        lo, hi = b[t], b[t + 3]
        width = hi - lo
        if width < 0.8:
            continue
        # A face another part stands against has no air to grow in.
        probe = [0.0, 1.0, 0.0]
        probe[axis] = plane + sign * 0.3
        probe[t] = (lo + hi) / 2
        if touches(parts, None, probe) and not any(q["mat"] in ("yard", "timber") for q in parts
                                                   if q["b"][axis] <= probe[axis] <= q["b"][axis + 3]
                                                   and q["b"][t] <= probe[t] <= q["b"][t + 3]):
            continue
        outward = (sign > 0) == ((b[axis] + b[axis + 3]) / 2 > 0)
        # Ivy keeps to a few colonies per face rather than spreading evenly:
        # one or two big mats climbing from the foot, the odd strand hanging
        # from the top.
        colonies = max(1, int(width / 6.0 * (1.0 if outward else 0.4) * rng.uniform(0.6, 1.4)))
        starts = []
        for _ in range(colonies):
            cu = rng.uniform(lo + 0.6, hi - 0.6)
            spread = rng.uniform(0.6, 1.8)
            reach = (y1 - base) * rng.uniform(0.45, 0.98)
            for _ in range(rng.randint(5, 11)):
                starts.append((min(hi - 0.1, max(lo + 0.1, cu + rng.gauss(0, spread))), base, reach * rng.uniform(0.5, 1.0), 1))
            if outward and rng.random() < 0.6:
                for _ in range(rng.randint(2, 4)):
                    starts.append((min(hi - 0.1, max(lo + 0.1, cu + rng.gauss(0, spread))), y1 - 0.05,
                                   y1 - (y1 - base) * rng.uniform(0.2, 0.5), -1))
        for (u, y, reach, way) in starts:
            stack = [(u, y, reach)]
            leaves = 0
            while stack and leaves < 34:
                u, y, reach = stack.pop()
                while (y < reach if way > 0 else y > reach) and leaves < 34:
                    du = rng.uniform(-0.25, 0.25)
                    dy = rng.uniform(0.14, 0.28) * way
                    u = min(hi - 0.05, max(lo + 0.05, u + du))
                    y += dy
                    # A sprig or two off the stem here, a card each, leaning
                    # up and out the way the stem climbs.
                    for _ in range(rng.randint(1, 2)):
                        lu = u + rng.uniform(-0.15, 0.15)
                        ly = y + rng.uniform(-0.1, 0.1)
                        sz = rng.uniform(0.2, 0.34)
                        if not (lo + sz * 0.5 < lu < hi - sz * 0.5 and base + 0.05 < ly < y1 - sz * 0.5):
                            continue
                        a = rng.uniform(-1.0, 1.0) + (0.0 if way > 0 else math.pi)
                        off = rng.uniform(0.012, 0.045)
                        q = []
                        for (cu, cv) in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
                            pu = lu + sz * (cu * math.cos(a) + cv * math.sin(a))
                            pv = ly + sz * (-cu * math.sin(a) + cv * math.cos(a))
                            pp = [0.0, pv, 0.0]
                            pp[axis] = plane + sign * off
                            pp[t] = pu
                            q.append(tuple(pp))
                        s.new_piece()
                        s.face("ivy", q, rng.choice(IVY))
                        leaves += 1
                    if rng.random() < 0.1:
                        stack.append((u, y, y + (reach - y) * rng.uniform(0.3, 0.8)))


def fallen(s, b, rng):
    """Fallen blocks: one squared stone settled into its box's footing and a
    second, smaller, tipped against it — both inside the box."""
    x0, y0, z0, x1, y1, z1 = b
    along = 0 if (x1 - x0) >= (z1 - z0) else 2
    lo, hi = b[along], b[along + 3]
    cut = lo + (hi - lo) * rng.uniform(0.55, 0.68)
    big = list(b)
    big[along + 3] = cut
    rough(s, "ashlar", tuple(big), stone_tint(rng), rng, amp=0.06, cell=0.5, crown=0.06)
    small = list(b)
    small[along] = cut + 0.04
    small[4] = y1 - rng.uniform(0.15, 0.35)
    k = 0.12
    small[2 - along] += k
    small[5 - along] -= k
    rough(s, "ashlar", tuple(small), stone_tint(rng), rng, amp=0.08, cell=0.45, crown=0.1)


def parapet(s, b, rng, lines):
    """The tower's cap: corbels under the overhang, the cap's own blocks, and
    merlons along its rim, two of them fallen."""
    x0, y0, z0, x1, y1, z1 = b
    masonry(s, b, [y0, y1], rng, faces="xzt")
    # Corbels: the cap overhangs the tower by 0.4 m; a bracket every 0.8 m.
    for axis in (0, 2):
        other = 2 - axis
        for sign in (-1, 1):
            face = b[axis] if sign < 0 else b[axis + 3]
            a = b[other] + 0.6
            while a < b[other + 3] - 0.6:
                cb = [0.0] * 6
                cb[axis], cb[axis + 3] = (face, face + 0.38) if sign < 0 else (face - 0.38, face)
                cb[other], cb[other + 3] = a - 0.17, a + 0.17
                cb[1], cb[4] = y0 - 0.45, y0
                s.box("ashlar", tuple(cb), stone_tint(rng), bevel=0.04)
                a += 0.8
    # Merlons round the rim.
    depth = 0.55
    gone = set(rng.sample(range(24), 3))
    i = 0
    for axis in (0, 2):
        other = 2 - axis
        for sign in (-1, 1):
            face = b[axis] if sign < 0 else b[axis + 3]
            a = b[other] + 0.1
            while a + 0.9 <= b[other + 3] - 0.1 + 1e-6:
                if i not in gone:
                    mb = [0.0] * 6
                    mb[axis], mb[axis + 3] = (face, face + depth) if sign < 0 else (face - depth, face)
                    mb[other], mb[other + 3] = a, a + 0.9
                    mb[1], mb[4] = y1, y1 + rng.uniform(0.8, 1.0)
                    s.box("ashlar", tuple(mb), stone_tint(rng), bevel=0.05)
                i += 1
                a += 1.5


def dress_stones(s, kit, rng):
    parts = kit["parts"]
    uprights = []
    altar = None
    for p in parts:
        x0, y0, z0, x1, y1, z1 = p["b"]
        dx, dz = x1 - x0, z1 - z0
        # An upright carrying a lintel keeps a flat top to carry it on.
        carries = any(q is not p and q["b"][1] <= y1 + 1e-3 and q["b"][1] > y1 - 0.5
                      and q["b"][0] < x1 and q["b"][3] > x0 and q["b"][2] < z1 and q["b"][5] > z0
                      for q in parts)
        if y0 > 2.0:
            # The lintel: a slab across two stones, rough and sagging at its ends.
            rough(s, "ashlar", p["b"], stone_tint(rng, (0.70, 0.69, 0.66)), rng,
                  amp=0.1, cell=0.5, crown=0.08)
        elif y1 <= 0.4:
            fire_ring(s, p["b"], rng)
        elif y1 <= 1.5 and max(dx, dz) <= 1.0:
            cairn(s, p["b"], rng)
        elif y1 <= 1.2 and max(dx, dz) >= 3.0:
            # A stone's broken top, fallen and half sunk, its old crown at one end.
            rough(s, "ashlar", p["b"], stone_tint(rng, (0.68, 0.67, 0.64)), rng,
                  amp=0.1, cell=0.45, crown=0.3)
        elif y1 <= 1.5:
            # The altar: a slab on a plinth, both inside its box.
            rough(s, "ashlar", (x0 + 0.15, y0, z0 + 0.15, x1 - 0.15, y1 - 0.22, z1 - 0.15),
                  stone_tint(rng, (0.66, 0.64, 0.60)), rng, amp=0.05, cell=0.5)
            rough(s, "ashlar", (x0, y1 - 0.26, z0, x1, y1, z1),
                  stone_tint(rng, (0.74, 0.72, 0.68)), rng, amp=0.04, cell=0.45, crown=0.03)
            altar = p["b"]
        else:
            # A standing stone: tapering above the head, its crown worn round.
            rough(s, "ashlar", p["b"], stone_tint(rng, (0.70, 0.69, 0.66)), rng,
                  amp=0.08, cell=0.42, taper=0.06 if carries else 0.28,
                  crown=0.0 if carries else 0.35)
            uprights.append((p["b"], 0.06 if carries else 0.28))
    for b, _ in uprights:
        # Moss and grass crowding each stone's foot. (Lichen is the
        # weathering pass's: a patch drawn here stood off the tapered face.)
        x0, _, z0, x1, _, z1 = b
        for _ in range(rng.randint(6, 12)):
            side = rng.randrange(4)
            if side < 2:
                x, z = rng.uniform(x0, x1), (z0 - 0.06 if side == 0 else z1 + 0.06)
            else:
                x, z = (x0 - 0.06 if side == 2 else x1 + 0.06), rng.uniform(z0, z1)
            tuft(s, x, -0.05, z, rng.uniform(0.2, 0.45), rng)
    # Spirals cut into the faces that look at the altar, on every other stone.
    for i, (b, taper) in enumerate(uprights):
        if i % 2 == 0:
            spiral(s, b, taper, rng)
    if altar is not None:
        offerings(s, altar, rng)


def spiral(s, b, taper, rng):
    """A spiral pecked into the stone's face toward the ring's middle: a dark
    groove on the face where `rough`'s taper has drawn it in to, winding out
    three turns."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    # The face whose normal points most at the middle.
    axis, sign = (0, -1 if cx > 0 else 1) if abs(cx) >= abs(cz) else (2, -1 if cz > 0 else 1)
    plane = (x1 if sign > 0 else x0) if axis == 0 else (z1 if sign > 0 else z0)
    t = 2 if axis == 0 else 0
    cu = (b[t] + b[t + 3]) / 2
    w = (b[t + 3] - b[t]) / 2 - 0.15
    cy = min(y1 - 0.9, 2.0)
    # How far the taper has pulled this face in at the spiral's height, and
    # a little more for the stone's own roughness.
    g = max(y0, 0.0)
    half = ((x1 - x0) if axis == 0 else (z1 - z0)) / 2
    inset = half * taper * max(0.0, (cy - g) / (y1 - g)) ** 1.4 + 0.035
    plane -= sign * inset
    turns, n = 3.0, 60
    prev = None
    for i in range(n + 1):
        f = i / n
        a = f * turns * 2 * math.pi
        rr = w * f
        u, y = cu + rr * math.cos(a), cy + rr * math.sin(a)
        p = [0.0, y, 0.0]
        p[axis] = plane + sign * 0.006
        p[t] = u
        p = tuple(p)
        if prev is not None:
            s.prism("ashlar", prev, p, 0.018, 3, (0.32, 0.31, 0.29))
        prev = p


def cairn(s, b, rng):
    """Flat stones stacked to a point, each a little smaller and turned."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    y = max(y0, 0.0) - 0.05
    w = min(x1 - x0, z1 - z0) / 2
    while y < y1 - 0.08:
        h = rng.uniform(0.1, 0.18)
        a = rng.uniform(0, math.pi)
        ybox(s, "ashlar", (cx + rng.uniform(-0.05, 0.05), y + h / 2, cz + rng.uniform(-0.05, 0.05)),
             (math.cos(a), math.sin(a)), w * 1.6, h, w * 1.2, stone_tint(rng, (0.66, 0.65, 0.62)))
        y += h * 0.92
        w *= 0.82
    tuft(s, cx + w, max(y0, 0.0), cz - w, 0.25, rng)


def fire_ring(s, b, rng):
    """A ring of stones round a black hearth, and the half-burnt sticks."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    r = min(x1 - x0, z1 - z0) / 2
    pts = [(cx + (r - 0.1) * math.cos(a), 0.02, cz + (r - 0.1) * math.sin(a)) for a in [2 * math.pi * i / 10 for i in range(10)]]
    face_out(s, "yard", pts, (0.14, 0.13, 0.12), (cx, -1, cz))
    for i in range(11):
        a = 2 * math.pi * i / 11
        sz = rng.uniform(0.18, 0.24)
        ybox(s, "ashlar", (cx + (r - 0.14) * math.cos(a), sz * 0.45, cz + (r - 0.14) * math.sin(a)),
             (math.cos(a + 1.57), math.sin(a + 1.57)), sz * 1.4, sz * 0.9, sz, (0.55, 0.53, 0.5))
    for i in range(5):
        a = math.pi * i / 5 + rng.uniform(-0.2, 0.2)
        obox(s, "timber", (cx - 0.45 * math.cos(a), 0.05, cz - 0.45 * math.sin(a)),
             (cx + 0.45 * math.cos(a), 0.09 + 0.02 * i, cz + 0.45 * math.sin(a)), 0.07, 0.07, CHAR)


def offerings(s, b, rng):
    """What people leave on the altar: candle stubs with wax run down them
    (their flames are bulbs, lit at night), bowls, a bundle of dry flowers."""
    x0, y0, z0, x1, y1, z1 = b
    for _ in range(9):
        cx, cz = rng.uniform(x0 + 0.25, x1 - 0.25), rng.uniform(z0 + 0.25, z1 - 0.25)
        h = rng.uniform(0.06, 0.2)
        cylinder(s, "canvas", (cx, y1, cz), (cx, y1 + h, cz), rng.uniform(0.025, 0.04), 8, (0.9, 0.86, 0.72))
        cylinder(s, "canvas", (cx, y1, cz), (cx, y1 + 0.012, cz), 0.07, 8, (0.88, 0.84, 0.7))
        s.box("bulb", (cx - 0.008, y1 + h + 0.005, cz - 0.008, cx + 0.008, y1 + h + 0.04, cz + 0.008))
    for _ in range(2):
        cx, cz = rng.uniform(x0 + 0.4, x1 - 0.4), rng.uniform(z0 + 0.35, z1 - 0.35)
        cylinder(s, "canvas", (cx, y1, cz), (cx, y1 + 0.07, cz), 0.12, 10, (0.5, 0.36, 0.26))
    cx, cz = (x0 + x1) / 2 + 0.4, (z0 + z1) / 2
    tuft(s, cx, y1, cz, 0.3, rng, tint=(0.62, 0.5, 0.3))
    tuft(s, cx + 0.06, y1, cz + 0.04, 0.26, rng, tint=(0.72, 0.36, 0.3))


def lattice_face(s, a, b, y0, y1, bay, tint_at, r=0.035):
    """X-bracing between two legs' inner corners `a` and `b` (x, z), bays of
    `bay` metres from `y0` to `y1`, with a strut at every bay line."""
    n = max(1, round((y1 - y0) / bay))
    h = (y1 - y0) / n
    for i in range(n + 1):
        y = y0 + i * h
        s.prism("steel", (a[0], y, a[1]), (b[0], y, b[1]), r, 4, tint_at(y))
        if i < n:
            s.prism("steel", (a[0], y, a[1]), (b[0], y + h, b[1]), r, 4, tint_at(y + h / 2))
            s.prism("steel", (b[0], y, b[1]), (a[0], y + h, a[1]), r, 4, tint_at(y + h / 2))


def dish(s, centre, facing, radius, depth=0.28, rings=5, segs=16):
    """A parabolic dish at `centre` opening along the unit (x, z) `facing`,
    its feed on a tripod in front of it."""
    fx, fz = facing
    ux, uz = -fz, fx  # horizontal, across the dish

    def at(rr, t, d):
        cx = centre[0] + ux * rr * math.cos(t) + fx * d
        cy = centre[1] + rr * math.sin(t)
        cz = centre[2] + uz * rr * math.cos(t) + fz * d
        return (cx, cy, cz)

    s.new_piece()
    for i in range(rings):
        r0, r1 = radius * i / rings, radius * (i + 1) / rings
        d0, d1 = depth * (i / rings) ** 2, depth * ((i + 1) / rings) ** 2
        for k in range(segs):
            t0, t1 = 2 * math.pi * k / segs, 2 * math.pi * (k + 1) / segs
            q = [at(r0, t0, d0), at(r1, t0, d1), at(r1, t1, d1), at(r0, t1, d0)]
            s.face("steel", q, MAST_WHITE)
    feed = at(0, 0, depth + radius * 0.7)
    for k in range(3):
        t = 2 * math.pi * k / 3 + 0.5
        s.prism("steel", at(radius * 0.9, t, depth), feed, 0.018, 4, GALV)
    s.box("steel", (feed[0] - 0.07, feed[1] - 0.07, feed[2] - 0.07,
                    feed[0] + 0.07, feed[1] + 0.07, feed[2] + 0.07), (0.3, 0.3, 0.3))


def dress_mast(s, kit, rng):
    parts = kit["parts"]
    legs = [p for p in parts if p["mat"] == "steel" and p["b"][4] - p["b"][1] > 20
            and p["b"][3] - p["b"][0] < 1.0]
    band = lambda y: MAST_RED if y > 12 and int((y - 12) // 6) % 2 == 0 else (
        MAST_WHITE if y > 12 else GALV)
    top = max(p["b"][4] for p in legs)
    # Each leg an angle section in its box's outer corner, painted in bands.
    for p in legs:
        x0, y0, z0, x1, y1, z1 = p["b"]
        sx = 1 if x0 + x1 > 0 else -1
        sz = 1 if z0 + z1 > 0 else -1
        t = 0.06
        y = y0
        while y < y1 - 1e-3:
            ye = min(y1, (math.floor(y / 6.0) + 1) * 6.0)
            tint = band((y + ye) / 2)
            xo = (x1 - t, x1) if sx > 0 else (x0, x0 + t)
            zo = (z1 - t, z1) if sz > 0 else (z0, z0 + t)
            s.box("steel", (xo[0], y, z0, xo[1], ye, z1), tint)
            s.box("steel", (x0, y, zo[0], x1, ye, zo[1]), tint)
            y = ye
    # The four faces' bracing, from above head height to the top.
    inner = [((p["b"][0] + p["b"][3]) / 2, (p["b"][2] + p["b"][5]) / 2) for p in legs]
    xs = sorted({round(c[0], 3) for c in inner})
    zs = sorted({round(c[1], 3) for c in inner})
    if len(xs) == 2 and len(zs) == 2:
        corners = [(xs[0], zs[0]), (xs[1], zs[0]), (xs[1], zs[1]), (xs[0], zs[1])]
        for i in range(4):
            lattice_face(s, corners[i], corners[(i + 1) % 4], 2.4, top, 3.0, band)
    for p in parts:
        b = p["b"]
        x0, y0, z0, x1, y1, z1 = b
        dx, dy, dz = x1 - x0, y1 - y0, z1 - z0
        if p in legs:
            continue
        if p["mat"] == "concrete" and dy < 6 and y1 < 1.0 and max(dx, dz) > 6:
            slab(s, b, rng)
            keep = [(q["b"][0], q["b"][2], q["b"][3], q["b"][5]) for q in parts
                    if q is not p and q["b"][1] < 2.0 and q["b"][1] > b[1]]
            pad_life(s, b, rng, keep, joint=3.0)
        elif p["mat"] == "concrete" and y1 < 1.0:
            guy_anchor(s, b, top, rng)
        elif p["mat"] == "concrete":
            hut(s, b, rng)
        elif p["mat"] == "steel" and y0 > 2.0 and y0 < 3.6 and dy < 0.5:
            hut_roof(s, b)
        elif p["mat"] == "steel" and dx < 0.6 and dz < 0.6 and y0 >= top - 1:
            # The antenna: a tapered pole with two cross-arms.
            cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
            n = 6
            for i in range(n):
                ya, yb = y0 + dy * i / n, y0 + dy * (i + 1) / n
                r = 0.19 * (1 - 0.6 * i / n)
                s.prism("steel", (cx, ya, cz), (cx, yb, cz), r, 8, MAST_RED if i % 2 == 0 else MAST_WHITE)
            for y in (y0 + dy * 0.35, y0 + dy * 0.7):
                s.prism("steel", (cx - 1.2, y, cz), (cx + 1.2, y, cz), 0.03, 4, GALV)
                s.prism("steel", (cx, y, cz - 1.2), (cx, y, cz + 1.2), 0.03, 4, GALV)
            s.box("bulb", (cx - 0.12, y1, cz - 0.12, cx + 0.12, y1 + 0.25, cz + 0.12))
        elif p["mat"] == "steel" and min(dx, dz) <= 0.45 and dy >= 1.0:
            # A dish: its box is the dish's disc, thin along the way it faces.
            thin_x = dx < dz
            c = ((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2)
            facing = ((1 if c[0] > 0 else -1), 0) if thin_x else (0, (1 if c[2] > 0 else -1))
            back = (x0 if facing[0] > 0 else x1) if thin_x else (z0 if facing[1] > 0 else z1)
            base = (back, c[1], c[2]) if thin_x else (c[0], c[1], back)
            dish(s, base, facing, min(dy, dz if thin_x else dx) / 2)
        elif p["mat"] == "steel" and dy <= 0.35 and dx > 4.5 and dz > 4.5:
            platform(s, b, rail=True)
        elif p["mat"] == "steel" and dy <= 0.35 and dx > 3 and dz > 3:
            platform(s, b, rail=False)
        elif p["mat"] == "steel":
            # A girder between two legs: a channel, flanges top and bottom.
            s.box("steel", (x0, y0, z0, x1, y0 + 0.05, z1), GALV)
            s.box("steel", (x0, y1 - 0.05, z0, x1, y1, z1), GALV)
            wide_x = dx >= dz
            if wide_x:
                zc = (z0 + z1) / 2
                s.box("steel", (x0, y0, zc - 0.02, x1, y1, zc + 0.02), GALV)
            else:
                xc = (x0 + x1) / 2
                s.box("steel", (xc - 0.02, y0, z0, xc + 0.02, y1, z1), GALV)
        else:
            s.box(p["mat"], b, (1, 1, 1))


def slab(s, b, rng, joint=3.0, lines=()):
    """A cast slab: its box with a chamfered top edge and sawn joints every
    `joint` metres, flush to the top within a few millimetres."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("yard", b, SLAB, bevel=0.04)
    dark = (0.3, 0.3, 0.29)
    y = y1 + 0.004
    x = x0 + joint
    while x < x1 - 0.5:
        s.face("yard", [(x - 0.012, y, z0 + 0.05), (x - 0.012, y, z1 - 0.05),
                            (x + 0.012, y, z1 - 0.05), (x + 0.012, y, z0 + 0.05)], dark)
        x += joint
    z = z0 + joint
    while z < z1 - 0.5:
        s.face("yard", [(x0 + 0.05, y, z - 0.012), (x1 - 0.05, y, z - 0.012),
                            (x1 - 0.05, y, z + 0.012), (x0 + 0.05, y, z + 0.012)][::-1], dark)
        z += joint
    # Stains: the slab's oil and rust, flat on its top.
    for _ in range(int((x1 - x0) * (z1 - z0) / 30)):
        cx, cz = rng.uniform(x0 + 1, x1 - 1), rng.uniform(z0 + 1, z1 - 1)
        r = rng.uniform(0.3, 0.9)
        k = rng.uniform(0.45, 0.7)
        pts = [(cx + r * math.cos(t) * rng.uniform(0.7, 1.0), y + 0.002,
                cz + r * math.sin(t) * rng.uniform(0.7, 1.0))
               for t in [2 * math.pi * i / 9 for i in range(9)]]
        s.face("yard", pts[::-1], (SLAB[0] * k, SLAB[1] * k * 0.95, SLAB[2] * k * 0.9))
    for (a, b2, c) in lines:
        s.face("yard", [(a[0], y + 0.003, a[1]), (b2[0], y + 0.003, b2[1]),
                            (c[0], y + 0.003, c[1]), (a[0] + c[0] - b2[0], y + 0.003, a[1] + c[1] - b2[1])][::-1],
               (1.0, 0.8, 0.25))


def guy_anchor(s, b, top, rng):
    """A deadman block, its eye plate, and two guy wires up to the mast — the
    wires leave the block's footprint only once they are out of reach."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("concrete", b, (0.8, 0.79, 0.76), bevel=0.06)
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    s.box("steel", (cx - 0.08, y1, cz - 0.03, cx + 0.08, y1 + 0.18, cz + 0.03), (0.35, 0.3, 0.26))
    ln = math.hypot(cx, cz)
    ux, uz = cx / ln, cz / ln
    for h in (top * 0.55, top * 0.92):
        s.prism("steel", (cx, y1 + 0.15, cz), (ux * 1.75, h, uz * 1.75), 0.014, 4, (0.4, 0.4, 0.4))


def hut(s, b, rng):
    """The equipment hut: rendered block walls on a plinth, a steel door
    facing the mast, louvres, and a drip line under the roof."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("concrete", b, (0.88, 0.87, 0.84))
    s.box("concrete", (x0 - 0.04, y0, z0 - 0.04, x1 + 0.04, 0.35, z1 + 0.04), (0.6, 0.6, 0.58), bevel=0.02)
    door_x = x1 if abs(x1) < abs(x0) else x0
    out = 1 if door_x == x1 else -1
    cz = (z0 + z1) / 2
    fx = door_x + out * 0.035
    frame = (0.33, 0.35, 0.36)
    s.box("steel", (min(door_x, fx), 0.35, cz - 0.6, max(door_x, fx), 2.45, cz - 0.52), frame)
    s.box("steel", (min(door_x, fx), 0.35, cz + 0.52, max(door_x, fx), 2.45, cz + 0.6), frame)
    s.box("steel", (min(door_x, fx), 2.37, cz - 0.6, max(door_x, fx), 2.45, cz + 0.6), frame)
    fd = door_x + out * 0.02
    s.box("steel", (min(door_x, fd), 0.37, cz - 0.52, max(door_x, fd), 2.37, cz + 0.52), (0.42, 0.47, 0.44))
    # Louvres on the two long faces.
    for zf, sg in ((z0, -1), (z1, 1)):
        zl = zf + sg * 0.03
        s.box("steel", ((x0 + x1) / 2 - 0.5, 2.0, min(zf, zl), (x0 + x1) / 2 + 0.5, 2.6, max(zf, zl)), frame)
        for i in range(5):
            y = 2.05 + i * 0.11
            zl2 = zf + sg * 0.06
            s.box("steel", ((x0 + x1) / 2 - 0.46, y, min(zf, zl2), (x0 + x1) / 2 + 0.46, y + 0.04,
                            max(zf, zl2)), (0.25, 0.26, 0.27))


def hut_roof(s, b):
    x0, y0, z0, x1, y1, z1 = b
    s.box("steel", b, (0.4, 0.42, 0.43), bevel=0.02)
    # An air conditioner and the cable tray out to the mast, both up here.
    cx = (x0 + x1) / 2
    s.box("steel", (cx - 0.6, y1, z0 + 0.8, cx + 0.6, y1 + 0.8, z0 + 1.6), (0.7, 0.71, 0.7), bevel=0.03)
    near = x1 if abs(x1) < abs(x0) else x0
    leg = -1.7 if near < 0 else 1.7
    a, c = min(near, leg), max(near, leg)
    ty = y1 + 0.15
    s.box("steel", (a, ty, -0.15, c, ty + 0.06, 0.15), GALV)
    for k in (-0.15, 0.15):
        s.box("steel", (a, ty, k - 0.01, c, ty + 0.14, k + 0.01), GALV)


def platform(s, b, rail):
    """A grated service platform on its frame, railed round when `rail`."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("steel", (x0, y1 - 0.05, z0, x1, y1, z1), (0.38, 0.39, 0.4))
    for (a, c) in (((x0, z0), (x1, z0 + 0.12)), ((x0, z1 - 0.12), (x1, z1)),
                   ((x0, z0), (x0 + 0.12, z1)), ((x1 - 0.12, z0), (x1, z1))):
        s.box("steel", (a[0], y0, a[1], c[0], y1 - 0.05, c[1]), GALV)
    x = x0 + 0.3
    while x < x1:
        s.box("steel", (x - 0.012, y1, z0 + 0.12, x + 0.012, y1 + 0.006, z1 - 0.12), (0.25, 0.25, 0.26))
        x += 0.3
    if not rail:
        return
    yr = y1 + 1.1
    corners = [(x0 + 0.05, z0 + 0.05), (x1 - 0.05, z0 + 0.05), (x1 - 0.05, z1 - 0.05), (x0 + 0.05, z1 - 0.05)]
    for i in range(4):
        a, c = corners[i], corners[(i + 1) % 4]
        for y in (y1 + 0.55, yr):
            s.prism("steel", (a[0], y, a[1]), (c[0], y, c[1]), 0.025, 6, MAST_WHITE)
        n = max(1, round(math.hypot(c[0] - a[0], c[1] - a[1]) / 1.4))
        for k in range(n):
            t = k / n
            p = (a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t)
            s.prism("steel", (p[0], y1, p[1]), (p[0], yr, p[1]), 0.025, 6, MAST_WHITE)


TIMBER = (0.92, 0.88, 0.82)


def dress_tower(s, kit, rng):
    parts = kit["parts"]
    posts = [p for p in parts if p["b"][4] - p["b"][1] > 8 and p["b"][3] - p["b"][0] < 1.0]
    plank_tint = lambda: tuple(c * rng.uniform(0.8, 1.05) for c in TIMBER)
    for p in posts:
        s.box("timber", p["b"], plank_tint(), bevel=0.03)
    cs = [((p["b"][0] + p["b"][3]) / 2, (p["b"][2] + p["b"][5]) / 2) for p in posts]
    xs = sorted({round(c[0], 3) for c in cs})
    zs = sorted({round(c[1], 3) for c in cs})
    deck = next((p for p in parts if p["b"][1] > 5 and p["b"][4] - p["b"][1] < 0.5
                 and p["b"][3] - p["b"][0] > 4 and p["b"][4] < 11), None)
    if deck and len(xs) == 2 and len(zs) == 2:
        top = deck["b"][1]
        corners = [(xs[0], zs[0]), (xs[1], zs[0]), (xs[1], zs[1]), (xs[0], zs[1])]
        y0 = 3.3
        mid = (y0 + top) / 2
        for i in range(4):
            a, c = corners[i], corners[(i + 1) % 4]
            for (ya, yb) in ((y0, mid), (mid, top - 0.25)):
                s.new_piece()
                s.prism("timber", (a[0], ya, a[1]), (c[0], yb, c[1]), 0.09, 4, plank_tint())
                s.new_piece()
                s.prism("timber", (c[0], ya, c[1]), (a[0], yb, a[1]), 0.09, 4, plank_tint())
            s.box("timber", (min(a[0], c[0]) - 0.04, mid - 0.1, min(a[1], c[1]) - 0.04,
                             max(a[0], c[0]) + 0.04, mid + 0.1, max(a[1], c[1]) + 0.04), plank_tint(), bevel=0.02)
    for p in parts:
        if p in posts:
            continue
        b = p["b"]
        x0, y0, z0, x1, y1, z1 = b
        dx, dy, dz = x1 - x0, y1 - y0, z1 - z0
        if p is deck:
            # Joists under, boards across.
            for x in (x0 + 0.3, (x0 + x1) / 2, x1 - 0.3):
                s.box("timber", (x - 0.08, y0 - 0.22, z0, x + 0.08, y0, z1), plank_tint(), bevel=0.01)
            planks(s, b, rng)
        elif y0 > 5 and dy <= 1.2 and min(dx, dz) < 0.4:
            # A rail: boards on end, gaps a finger wide, a cap along the top.
            along_x = dx >= dz
            n = max(1, int((dx if along_x else dz) / 0.16))
            w = (dx if along_x else dz) / n
            for i in range(n):
                g = 0.008
                if along_x:
                    s.box("timber", (x0 + i * w + g, y0, z0, x0 + (i + 1) * w - g, y1 - 0.08, z1), plank_tint())
                else:
                    s.box("timber", (x0, y0, z0 + i * w + g, x1, y1 - 0.08, z0 + (i + 1) * w - g), plank_tint())
            s.box("timber", (x0 - 0.03, y1 - 0.08, z0 - 0.03, x1 + 0.03, y1, z1 + 0.03), plank_tint(), bevel=0.02)
        elif y0 > 10 and dx > 4:
            hip_roof(s, b, parts, rng)
        elif y0 > 10:
            continue  # the roof's cap: drawn by the hip roof's peak
        elif dy < 0.5 and y0 > 2:
            if dx > 4 and dz > 4:
                gable_roof(s, b, rng)
            else:
                s.box("timber", b, plank_tint(), bevel=0.02)  # a brace
        elif dy > 2 and dx > 4:
            cabin(s, b, rng)
        elif y1 <= 1.5:
            woodpile(s, b, rng)
        else:
            planks(s, b, rng)


def hip_roof(s, b, parts, rng):
    """A pyramid of boards over the deck, from the roof box's eaves up past
    its cap, with fascia and a finial."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    peak = max(p["b"][4] for p in parts) + 0.3
    apex = (cx, peak, cz)
    eave = [(x0, y0 + 0.1, z0), (x1, y0 + 0.1, z0), (x1, y0 + 0.1, z1), (x0, y0 + 0.1, z1)]
    # The underside, then each face as courses of overlapping boards.
    s.box("timber", (x0 + 0.1, y0, z0 + 0.1, x1 - 0.1, y0 + 0.1, z1 - 0.1), (0.55, 0.52, 0.48))
    rows = 7
    for i in range(4):
        a, c = eave[i], eave[(i + 1) % 4]
        for k in range(rows):
            t0, t1 = k / rows, (k + 1) / rows
            lift = 0.035
            p = lambda q, t: tuple(q[j] + (apex[j] - q[j]) * t for j in range(3))
            lo = [p(a, t0), p(c, t0)]
            lo = [(v[0], v[1] + lift, v[2]) for v in lo]
            q = [lo[0], p(a, t1), p(c, t1), lo[1]]
            s.new_piece()
            s.face("timber", q, tuple(cc * rng.uniform(0.62, 0.8) for cc in TIMBER))
        s.new_piece()
        s.prism("timber", a, c, 0.06, 4, TIMBER)
        s.prism("timber", a, apex, 0.07, 4, TIMBER)
    s.prism("timber", apex, (cx, peak + 0.6, cz), 0.06, 6, (0.4, 0.38, 0.35))


def gable_roof(s, b, rng):
    """A ridged board roof over the cabin, its ridge along the long axis and
    the gable ends boarded, all above the eaves the box sets."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    half = ((z1 - z0) if along_x else (x1 - x0)) / 2
    ridge = y0 + 0.12 + half * 0.55
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    shade = lambda: tuple(cc * rng.uniform(0.6, 0.78) for cc in TIMBER)
    rows = 6
    for side in (-1, 1):
        for k in range(rows):
            t0, t1 = k / rows, (k + 1) / rows
            if along_x:
                e0 = (z1 if side > 0 else z0)
                za, zb = e0 + (cz - e0) * t0, e0 + (cz - e0) * t1
                ya, yb = y0 + 0.1 + (ridge - y0 - 0.1) * t0, y0 + 0.1 + (ridge - y0 - 0.1) * t1
                q = [(x0, ya + 0.03, za), (x1, ya + 0.03, za), (x1, yb, zb), (x0, yb, zb)]
            else:
                e0 = (x1 if side > 0 else x0)
                xa, xb = e0 + (cx - e0) * t0, e0 + (cx - e0) * t1
                ya, yb = y0 + 0.1 + (ridge - y0 - 0.1) * t0, y0 + 0.1 + (ridge - y0 - 0.1) * t1
                q = [(xa, ya + 0.03, z1), (xa, ya + 0.03, z0), (xb, yb, z0), (xb, yb, z1)]
                if side < 0:
                    q = q[::-1]
            s.new_piece()
            s.face("timber", q, shade())
    s.box("timber", (x0, y0, z0, x1, y0 + 0.12, z1), (0.6, 0.57, 0.52))
    # Gable ends, set in from the overhang, and the ridge board.
    inset = 0.4
    for e in ((x0 + inset, x1 - inset) if along_x else (z0 + inset, z1 - inset)):
        if along_x:
            tri = [(e, y0 + 0.12, z0 + inset), (e, y0 + 0.12, z1 - inset), (e, ridge - 0.25, cz)]
            if e > cx:
                tri = tri[::-1]
        else:
            tri = [(x1 - inset, y0 + 0.12, e), (x0 + inset, y0 + 0.12, e), (cx, ridge - 0.25, e)]
            if e > cz:
                tri = tri[::-1]
        s.new_piece()
        s.face("timber", tri, TIMBER)
    if along_x:
        s.box("timber", (x0, ridge - 0.08, cz - 0.1, x1, ridge + 0.08, cz + 0.1), TIMBER, bevel=0.02)
        s.box("ashlar", (x1 - 1.6, ridge - 0.6, cz - 0.35, x1 - 1.0, ridge + 0.9, cz + 0.35), STONE, bevel=0.03)
    else:
        s.box("timber", (cx - 0.1, ridge - 0.08, z0, cx + 0.1, ridge + 0.08, z1), TIMBER, bevel=0.02)


def cabin(s, b, rng):
    """A log cabin filling its box: a stone sill down to the footing, logs
    flush to the faces and interleaved at the corners, a plank door toward
    the tower and a shuttered window."""
    x0, y0, z0, x1, y1, z1 = b
    masonry(s, (x0, y0, z0, x1, 0.3, z1), [y0, -1.0, -0.4, 0.3], rng, faces="xz")
    s.box("timber", (x0 + 0.2, 0.3, z0 + 0.2, x1 - 0.2, y1, z1 - 0.2), (0.25, 0.23, 0.2))
    r = 0.15
    y = 0.3 + r
    k = 0
    tint = lambda: (0.62 * rng.uniform(0.85, 1.1), 0.52 * rng.uniform(0.85, 1.1), 0.42 * rng.uniform(0.85, 1.1))
    while y + r <= y1 + 1e-3:
        if k % 2 == 0:
            for zc in (z0 + r, z1 - r):
                log(s, "timber", (x0, y, zc), (x1, y, zc), r, tint())
        else:
            for xc in (x0 + r, x1 - r):
                log(s, "timber", (xc, y, z0), (xc, y, z1), r, tint())
        y += r * 0.95
        k += 1
    door_x = x0 if abs(x0) < abs(x1) else x1
    out = -1 if door_x == x0 else 1
    cz = (z0 + z1) / 2
    fx = door_x + out * 0.04
    for i in range(5):
        za = cz - 0.5 + i * 0.2
        s.box("timber", (min(door_x, fx), 0.3, za + 0.006, max(door_x, fx), 2.2, za + 0.194), (0.5, 0.42, 0.33))
    fx2 = door_x + out * 0.05
    for yb in (0.6, 1.9):
        s.box("timber", (min(door_x, fx2), yb, cz - 0.48, max(door_x, fx2), yb + 0.14, cz + 0.48), (0.45, 0.38, 0.3))
    s.box("steel", (min(door_x, fx2), 1.1, cz + 0.3, max(door_x, fx2) + 0.01, 1.16, cz + 0.4), (0.2, 0.2, 0.2))
    # A shuttered window on the face toward +z.
    cx = (x0 + x1) / 2
    for sx in (-1, 1):
        s.box("timber", (cx + sx * 0.02 if sx > 0 else cx - 0.55, 1.2, z1, cx + 0.55 if sx > 0 else cx - 0.02, 2.1,
                         z1 + 0.04), (0.42, 0.36, 0.28))
    s.box("timber", (cx - 0.65, 1.12, z1, cx + 0.65, 1.2, z1 + 0.05), (0.5, 0.43, 0.34))


def woodpile(s, b, rng):
    """Split logs stacked along the pile's long axis, inside its box."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    r = 0.12
    w = (z1 - z0) if along_x else (x1 - x0)
    n = int(w / (2 * r))
    y = max(y0, 0.0) + r
    row = 0
    s.box("timber", (x0 + 0.1, y0, z0 + 0.1, x1 - 0.1, 0.1, z1 - 0.1), (0.3, 0.27, 0.23))
    while y + r <= y1 + 1e-3 and n - row > 0:
        m = n - (row % 2)
        off = (w - m * 2 * r) / 2
        for i in range(m):
            c = off + r + i * 2 * r
            t = (0.66 * rng.uniform(0.85, 1.1), 0.55 * rng.uniform(0.85, 1.1), 0.42 * rng.uniform(0.85, 1.1))
            j = rng.uniform(-0.06, 0.0)
            if along_x:
                log(s, "timber", (x0 - j, y, z0 + c), (x1 + j, y, z0 + c), r * rng.uniform(0.85, 1.0), t, 7)
            else:
                log(s, "timber", (x0 + c, y, z0 - j), (x0 + c, y, z1 + j), r * rng.uniform(0.85, 1.0), t, 7)
        y += 2 * r * 0.87
        row += 1


def dress_yard(s, kit, rng):
    parts = kit["parts"]
    slab_p = next((p for p in parts if p["mat"] == "concrete"), None)
    floor = slab_p["b"][4] if slab_p else 0.0
    cols = [p for p in parts if p["mat"] == "steel" and p["b"][4] - p["b"][1] > 6]
    for p in parts:
        b = p["b"]
        x0, y0, z0, x1, y1, z1 = b
        if p is slab_p:
            # Painted bay lines along the long axis, between the stacks.
            lines = []
            for z in (-4.6, 4.0):
                lines.append(((x0 + 1.5, z - 0.06), (x1 - 1.5, z - 0.06), (x1 - 1.5, z + 0.06)))
            slab(s, b, rng, joint=4.0, lines=lines)
            keep = [(q["b"][0], q["b"][2], q["b"][3], q["b"][5]) for q in parts
                    if q is not p and q["b"][1] < 2.0]
            pad_life(s, b, rng, keep, joint=4.0)
        elif p["mat"] == "cargo":
            if y0 < floor:
                b = (x0, floor, z0, x1, y1, z1)
            container(s, b, rng)
        elif p in cols:
            # An I-section column in the crane's yellow.
            ib = (x0, max(y0, floor - 0.2), z0, x1, y1, z1)
            ibeam(s, ib, vertical=True)
        elif p["mat"] == "steel":
            ibeam(s, b, vertical=False)
            # A trolley and its hook block hanging off the beam, out of reach.
            cz = (z0 + z1) / 2 + 2.0
            cx = (x0 + x1) / 2
            s.box("steel", (cx - 0.35, y0 - 0.5, cz - 0.5, cx + 0.35, y0, cz + 0.5), CRANE, bevel=0.03)
            for dz in (-0.12, 0.12):
                s.prism("steel", (cx, y0 - 0.5, cz + dz), (cx, 4.6, cz + dz), 0.015, 4, (0.25, 0.25, 0.25))
            s.box("steel", (cx - 0.2, 4.1, cz - 0.25, cx + 0.2, 4.6, cz + 0.25), CRANE, bevel=0.03)
            s.box("steel", (cx - 0.05, 3.75, cz - 0.05, cx + 0.05, 4.1, cz + 0.05), (0.2, 0.2, 0.2))
            # Knee braces between the beam and the columns.
            for c in cols:
                ccz = (c["b"][2] + c["b"][5]) / 2
                inward = -1 if ccz > 0 else 1
                s.prism("steel", (cx, y0 - 1.6, ccz), (cx, y0, ccz + inward * 1.6), 0.06, 4, CRANE)
        else:
            s.box(p["mat"], b, (1, 1, 1))


def ibeam(s, b, vertical):
    """An I-section filling `b`: two flanges and a web, in crane yellow."""
    x0, y0, z0, x1, y1, z1 = b
    t = 0.05
    if vertical:
        wide_x = (x1 - x0) >= (z1 - z0)
        if wide_x:
            s.box("steel", (x0, y0, z0, x1, y1, z0 + t), CRANE)
            s.box("steel", (x0, y0, z1 - t, x1, y1, z1), CRANE)
            xc = (x0 + x1) / 2
            s.box("steel", (xc - 0.02, y0, z0 + t, xc + 0.02, y1, z1 - t), CRANE)
        else:
            s.box("steel", (x0, y0, z0, x0 + t, y1, z1), CRANE)
            s.box("steel", (x1 - t, y0, z0, x1, y1, z1), CRANE)
            zc = (z0 + z1) / 2
            s.box("steel", (x0 + t, y0, zc - 0.02, x1 - t, y1, zc + 0.02), CRANE)
        # A base plate at the slab.
        s.box("steel", (x0 - 0.12, y0, z0 - 0.12, x1 + 0.12, y0 + 0.25, z1 + 0.12), (0.4, 0.4, 0.4))
    else:
        s.box("steel", (x0, y0, z0, x1, y0 + t, z1), CRANE)
        s.box("steel", (x0, y1 - t, z0, x1, y1, z1), CRANE)
        xc = (x0 + x1) / 2
        s.box("steel", (xc - 0.025, y0, z0, xc + 0.025, y1, z1), CRANE)


# ── the relay ───────────────────────────────────────────────────────────────
#
# A fenced comms compound by a road: chain link and barbed wire, a galvanised
# lattice tower with a caged ladder, panel antennas and dishes, the generator
# hut and its fuel tank, cabinets, drums, a cable drum, floodlights and the
# barriers at the gate. Every piece below head height stays on its part.

FENCE = (0.6, 0.62, 0.62)
CABINET = (0.5, 0.56, 0.5)
HAZARD = (0.88, 0.68, 0.12)
SOOT = (0.16, 0.15, 0.14)
DRUMS = [(0.2, 0.32, 0.55), (0.58, 0.2, 0.12), (0.32, 0.42, 0.26), (0.46, 0.3, 0.18)]


def face_out(s, role, pts, tint, centre):
    """A face wound so its normal points away from `centre` — for the convex
    pieces below, where working the winding out by hand is a bug a minute."""
    # Newell's normal: sound for any polygon, colinear first corners too.
    n = [0.0, 0.0, 0.0]
    for i in range(len(pts)):
        p, q = pts[i], pts[(i + 1) % len(pts)]
        n[0] += (p[1] - q[1]) * (p[2] + q[2])
        n[1] += (p[2] - q[2]) * (p[0] + q[0])
        n[2] += (p[0] - q[0]) * (p[1] + q[1])
    m = [sum(p[i] for p in pts) / len(pts) for i in range(3)]
    if sum(n[i] * (m[i] - centre[i]) for i in range(3)) < 0:
        pts = pts[::-1]
    return s.face(role, pts, tint)


def cylinder(s, role, a, b, r, sides, tint, caps=True):
    """A closed tube from `a` to `b` (tanks, drums, poles, flanges)."""
    d = [b[i] - a[i] for i in range(3)]
    ln = math.sqrt(sum(x * x for x in d))
    if ln < 1e-4:
        return
    d = [x / ln for x in d]
    up = (0, 1, 0) if abs(d[1]) < 0.9 else (1, 0, 0)
    u = (d[1] * up[2] - d[2] * up[1], d[2] * up[0] - d[0] * up[2], d[0] * up[1] - d[1] * up[0])
    un = math.sqrt(sum(x * x for x in u))
    u = [x / un for x in u]
    v = (d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0])
    ring = [[r * (math.cos(2 * math.pi * k / sides) * u[i] + math.sin(2 * math.pi * k / sides) * v[i])
             for i in range(3)] for k in range(sides)]
    ra = [tuple(a[i] + q[i] for i in range(3)) for q in ring]
    rb = [tuple(b[i] + q[i] for i in range(3)) for q in ring]
    c = tuple((a[i] + b[i]) / 2 for i in range(3))
    s.new_piece()
    for k in range(sides):
        k2 = (k + 1) % sides
        face_out(s, role, [ra[k], ra[k2], rb[k2], rb[k]], tint, c)
    if caps:
        face_out(s, role, ra, tint, c)
        face_out(s, role, rb, tint, c)


def ybox(s, role, c, u, w, h, d, tint):
    """A box `w` along the horizontal unit `u`, `d` across it and `h` tall,
    centred on `c` — the box turned about the vertical."""
    n = (-u[1], u[0])
    cs = []
    for (lx, lz) in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        cs.append((c[0] + lx * w / 2 * u[0] + lz * d / 2 * n[0], c[2] + lx * w / 2 * u[1] + lz * d / 2 * n[1]))
    y0, y1 = c[1] - h / 2, c[1] + h / 2
    s.new_piece()
    for i in range(4):
        a, b = cs[i], cs[(i + 1) % 4]
        face_out(s, role, [(a[0], y0, a[1]), (b[0], y0, b[1]), (b[0], y1, b[1]), (a[0], y1, a[1])], tint, c)
    face_out(s, role, [(p[0], y0, p[1]) for p in cs], tint, c)
    face_out(s, role, [(p[0], y1, p[1]) for p in cs], tint, c)


def chain_link(s, a, b, y0, y1, pitch=0.2, r=0.006):
    """Diamond mesh in the vertical plane from `a` to `b` (x, z), `y0`–`y1`."""
    L = math.hypot(b[0] - a[0], b[1] - a[1])
    ux, uz = (b[0] - a[0]) / L, (b[1] - a[1]) / L
    H = y1 - y0
    at = lambda t, y: (a[0] + ux * t, y, a[1] + uz * t)
    t = -H + (L % pitch) / 2
    while t < L:
        s0, e0 = max(t, 0.0), min(t + H, L)
        if e0 - s0 > 0.05:
            s.prism("steel", at(s0, y0 + s0 - t), at(e0, y0 + e0 - t), r, 3, FENCE)
            s.prism("steel", at(s0, y1 - (s0 - t)), at(e0, y1 - (e0 - t)), r, 3, FENCE)
        t += pitch


def fence_run(s, b, rng, signs=0):
    """A chain-link run filling thin box `b`: pipe posts, a top rail, the
    fabric, and barbed wire on outriggers leaning out, above head height."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    if along_x:
        zc = (z0 + z1) / 2
        a, e, out = (x0, zc), (x1, zc), (0.0, 1.0 if zc > 0 else -1.0)
    else:
        xc = (x0 + x1) / 2
        a, e, out = (xc, z0), (xc, z1), (1.0 if xc > 0 else -1.0, 0.0)
    L = math.hypot(e[0] - a[0], e[1] - a[1])
    ux, uz = (e[0] - a[0]) / L, (e[1] - a[1]) / L
    n = max(1, math.ceil(L / 2.6))
    posts = [(a[0] + ux * L * k / n, a[1] + uz * L * k / n) for k in range(n + 1)]
    for (px, pz) in posts:
        cylinder(s, "steel", (px, y0, pz), (px, y1, pz), 0.045, 8, FENCE)
        # The outrigger, and three strands' clips on it.
        tip = (px + out[0] * 0.38, y1 + 0.42, pz + out[1] * 0.38)
        s.prism("steel", (px, y1 - 0.05, pz), tip, 0.018, 4, FENCE)
    for k in range(n):
        p, q = posts[k], posts[k + 1]
        s.prism("steel", (p[0], y1 - 0.04, p[1]), (q[0], y1 - 0.04, q[1]), 0.022, 6, FENCE)
        s.prism("steel", (p[0], y0 + 0.12, p[1]), (q[0], y0 + 0.12, q[1]), 0.006, 3, FENCE)
        for f in (0.33, 0.66, 1.0):
            sag = 0.03 * rng.random()
            pa = (p[0] + out[0] * 0.38 * f, y1 + 0.42 * f - 0.03, p[1] + out[1] * 0.38 * f)
            qa = (q[0] + out[0] * 0.38 * f, y1 + 0.42 * f - 0.03, q[1] + out[1] * 0.38 * f)
            mid = ((pa[0] + qa[0]) / 2, (pa[1] + qa[1]) / 2 - sag, (pa[2] + qa[2]) / 2)
            s.prism("steel", pa, mid, 0.005, 3, (0.45, 0.43, 0.4))
            s.prism("steel", mid, qa, 0.005, 3, (0.45, 0.43, 0.4))
    chain_link(s, a, e, y0 + 0.1, y1 - 0.06)
    # Signs wired to the fabric's outer face.
    for k in range(signs):
        t = L * (k + 1) / (signs + 1) + rng.uniform(-0.4, 0.4)
        cx, cz = a[0] + ux * t + out[0] * 0.07, a[1] + uz * t + out[1] * 0.07
        hazard_sign(s, (cx, 1.35, cz), (ux, uz), out)


def hazard_sign(s, c, u, out):
    """A yellow plate with a black bolt on it, facing `out`."""
    ybox(s, "steel", c, u, 0.46, 0.34, 0.012, HAZARD)
    # The bolt: two slanted bars, a hair proud of the plate.
    f = 0.008
    o = (c[0] + out[0] * f, c[2] + out[1] * f)
    for (ta, ya, tb, yb) in ((0.05, 0.12, -0.04, -0.01), (0.04, 0.01, -0.05, -0.12)):
        p0 = (o[0] + u[0] * ta, c[1] + ya, o[1] + u[1] * ta)
        p1 = (o[0] + u[0] * tb, c[1] + yb, o[1] + u[1] * tb)
        s.prism("steel", p0, p1, 0.018, 4, (0.08, 0.08, 0.08))


def gate_leaf(s, hinge, u, out, w=2.0, y0=0.3, y1=1.95):
    """A chain-link gate leaf swung right back against the fence's outer
    face: a pipe frame and its fabric, 3 cm proud of the face."""
    hx, hz = hinge[0] + out[0] * 0.03, hinge[1] + out[1] * 0.03
    fx, fz = hx + u[0] * w, hz + u[1] * w
    for (p, q) in (((hx, y0, hz), (fx, y0, fz)), ((hx, y1, hz), (fx, y1, fz)),
                   ((hx, y0, hz), (hx, y1, hz)), ((fx, y0, fz), (fx, y1, fz))):
        s.prism("steel", p, q, 0.02, 6, FENCE)
    s.prism("steel", (hx, y0, hz), (fx, y1, fz), 0.015, 4, FENCE)
    chain_link(s, (hx, hz), (fx, fz), y0 + 0.04, y1 - 0.04, pitch=0.2)


def angle_leg(s, b, tint, centre, t=0.06):
    """A tower leg as an angle section in its box's corner away from the
    tower's `centre` (x, z)."""
    x0, y0, z0, x1, y1, z1 = b
    sx = 1 if (x0 + x1) / 2 > centre[0] else -1
    sz = 1 if (z0 + z1) / 2 > centre[1] else -1
    xo = (x1 - t, x1) if sx > 0 else (x0, x0 + t)
    zo = (z1 - t, z1) if sz > 0 else (z0, z0 + t)
    # Flanges narrower than the box: a 40 cm collision post drawn as a
    # 25 cm angle reads as lattice, not as a column.
    f = min(0.25, x1 - x0, z1 - z0)
    xa = (x1 - f, x1) if sx > 0 else (x0, x0 + f)
    za = (z1 - f, z1) if sz > 0 else (z0, z0 + f)
    s.box("steel", (xo[0], y0, za[0], xo[1], y1, za[1]), tint)
    s.box("steel", (xa[0], y0, zo[0], xa[1], y1, zo[1]), tint)
    # A base plate and its bolts on a pier, all inside the leg's footing.
    s.box("concrete", (x0 - 0.04, y0, z0 - 0.04, x1 + 0.04, y0 + 0.2, z1 + 0.04), (0.78, 0.77, 0.74), bevel=0.03)
    s.box("steel", (x0, y0 + 0.2, z0, x1, y0 + 0.23, z1), (0.4, 0.38, 0.35))


def caged_ladder(s, xa, xb, z, out, y0, y1):
    """Rails and rungs on the face plane `z` between `xa` and `xb`, and a
    safety cage bowed out along `out` (±1 in z) — begun above head height."""
    for x in (xa, xb):
        s.prism("steel", (x, y0, z), (x, y1, z), 0.025, 4, GALV)
    y = y0 + 0.15
    while y < y1:
        s.prism("steel", (xa, y, z), (xb, y, z), 0.013, 4, GALV)
        y += 0.3
    cx, r = (xa + xb) / 2, 0.38
    hoop = [(cx + r * math.cos(math.pi * k / 6), z + out * r * math.sin(math.pi * k / 6) + out * 0.05)
            for k in range(7)]
    y = y0 + 0.8
    while y < y1 - 0.2:
        for k in range(6):
            s.prism("steel", (hoop[k][0], y, hoop[k][1]), (hoop[k + 1][0], y, hoop[k + 1][1]), 0.015, 4, GALV)
        y += 1.0
    for k in (1, 3, 5):
        s.prism("steel", (hoop[k][0], y0 + 0.8, hoop[k][1]), (hoop[k][0], y1 - 0.2, hoop[k][1]), 0.012, 4, GALV)


def panel_antenna(s, base, facing):
    """A sector panel on its pipe, tilted down a touch, and the radio box
    behind it."""
    fx, fz = facing
    u = (-fz, fx)
    px, py, pz = base
    s.prism("steel", (px, py, pz), (px, py + 2.6, pz), 0.04, 6, GALV)
    c = (px + fx * 0.14, py + 1.6, pz + fz * 0.14)
    ybox(s, "steel", c, u, 0.32, 1.45, 0.12, (0.86, 0.86, 0.83))
    ybox(s, "steel", (px - fx * 0.12, py + 1.0, pz - fz * 0.12), u, 0.26, 0.4, 0.14, (0.4, 0.42, 0.43))
    for y in (py + 1.1, py + 2.1):
        s.prism("steel", (px, y, pz), (c[0], y, c[2]), 0.015, 4, GALV)


def drum_dish(s, centre, facing, r=0.35):
    """A microwave drum: a short white cylinder with a flat radome face."""
    fx, fz = facing
    a = (centre[0] - fx * 0.12, centre[1], centre[2] - fz * 0.12)
    b = (centre[0] + fx * 0.14, centre[1], centre[2] + fz * 0.14)
    cylinder(s, "steel", a, b, r, 14, (0.85, 0.85, 0.82))
    cylinder(s, "steel", (a[0] - fx * 0.1, a[1], a[2] - fz * 0.1), a, 0.1, 6, GALV)


def fuel_tank(s, b):
    """A horizontal tank on two saddles, its fill and vent on top."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    r = ((z1 - z0) if along_x else (x1 - x0)) / 2 - 0.01
    cy = y1 - r - 0.01
    if along_x:
        zc = (z0 + z1) / 2
        a, e = (x0 + 0.05, cy, zc), (x1 - 0.05, cy, zc)
        for x in (x0 + 0.6, x1 - 0.6):
            s.box("concrete", (x - 0.15, y0, z0 + 0.08, x + 0.15, cy - r * 0.55, z1 - 0.08), (0.7, 0.69, 0.66))
    else:
        xc = (x0 + x1) / 2
        a, e = (xc, cy, z0 + 0.05), (xc, cy, z1 - 0.05)
        for z in (z0 + 0.6, z1 - 0.6):
            s.box("concrete", (x0 + 0.08, y0, z - 0.15, x1 - 0.08, cy - r * 0.55, z + 0.15), (0.7, 0.69, 0.66))
    cylinder(s, "steel", a, e, r, 18, (0.72, 0.74, 0.66))
    # Weld bands, a manway, the vent pipe.
    for t in (0.25, 0.5, 0.75):
        p = tuple(a[i] + (e[i] - a[i]) * t for i in range(3))
        q = tuple(a[i] + (e[i] - a[i]) * (t + 0.008) for i in range(3))
        cylinder(s, "steel", p, q, r + 0.012, 18, (0.55, 0.56, 0.5), caps=False)
    m = tuple((a[i] + e[i]) / 2 for i in range(3))
    cylinder(s, "steel", (m[0], cy + r - 0.05, m[2]), (m[0], cy + r + 0.08, m[2]), 0.22, 10, (0.5, 0.5, 0.46))
    vx = a[0] + (e[0] - a[0]) * 0.8
    vz = a[2] + (e[2] - a[2]) * 0.8
    s.prism("steel", (vx, cy + r - 0.05, vz), (vx, y1 + 0.5, vz), 0.025, 6, GALV)


def cabinets(s, b):
    """Two outdoor equipment cabinets on a plinth, doors toward the gate."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("concrete", (x0, y0, z0, x1, y0 + 0.1, z1), (0.72, 0.71, 0.68), bevel=0.02)
    w = (x1 - x0) / 2
    for k in range(2):
        cx0, cx1 = x0 + k * w + 0.02, x0 + (k + 1) * w - 0.02
        s.box("steel", (cx0, y0 + 0.1, z0 + 0.04, cx1, y1 - 0.06, z1 - 0.02), CABINET, bevel=0.015)
        s.box("steel", (cx0 - 0.02, y1 - 0.06, z0, cx1 + 0.02, y1, z1), (0.44, 0.48, 0.44), bevel=0.01)
        # Door seam, handle and vents on the front (−z).
        zf = z0 + 0.04
        s.box("steel", (cx0 + 0.04, y0 + 0.16, zf - 0.006, cx1 - 0.04, y1 - 0.12, zf), (0.42, 0.47, 0.42))
        s.box("steel", (cx1 - 0.12, y0 + 0.85, zf - 0.03, cx1 - 0.08, y0 + 1.05, zf), (0.2, 0.2, 0.2))
        for i in range(4):
            y = y1 - 0.4 + i * 0.06
            s.box("steel", (cx0 + 0.15, y, zf - 0.012, cx1 - 0.2, y + 0.025, zf), (0.25, 0.27, 0.25))
        tri = [(cx0 + 0.22, y0 + 1.25, zf - 0.008), (cx0 + 0.34, y0 + 1.25, zf - 0.008),
               (cx0 + 0.28, y0 + 1.36, zf - 0.008)]
        face_out(s, "steel", tri, HAZARD, (cx0 + 0.28, y0 + 1.3, zf + 1))


def drum_pallet(s, b, rng):
    """Four oil drums on a timber pallet."""
    x0, y0, z0, x1, y1, z1 = b
    ph = 0.14
    for z in (z0 + 0.05, (z0 + z1) / 2 - 0.05, z1 - 0.15):
        s.box("timber", (x0, y0, z, x1, y0 + ph - 0.025, z + 0.1), TIMBER)
    s.new_piece()
    x = x0
    while x < x1 - 0.05:
        s.box("timber", (x, y0 + ph - 0.025, z0, min(x1, x + 0.1), y0 + ph, z1), TIMBER)
        x += 0.15
    r = min(x1 - x0, z1 - z0) / 4 - 0.02
    for (fx, fz) in ((0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)):
        cx, cz = x0 + (x1 - x0) * fx, z0 + (z1 - z0) * fz
        tint = rng.choice(DRUMS)
        a, e = (cx, y0 + ph, cz), (cx, y1, cz)
        cylinder(s, "steel", a, e, r, 12, tint)
        for t in (0.33, 0.66):
            y = y0 + ph + (y1 - y0 - ph) * t
            cylinder(s, "steel", (cx, y, cz), (cx, y + 0.025, cz), r + 0.012, 12, tint, caps=False)
        cylinder(s, "steel", (cx + r * 0.5, y1, cz), (cx + r * 0.5, y1 + 0.02, cz), 0.03, 6, (0.3, 0.3, 0.3))


def cable_drum(s, b, rng):
    """A timber cable drum standing on its flanges, cable wound on it."""
    x0, y0, z0, x1, y1, z1 = b
    along_z = (z1 - z0) <= (x1 - x0)
    r = (y1 - y0) / 2
    cy = y0 + r
    if along_z:
        c = (x0 + x1) / 2
        ends = [((c, cy, z0), (c, cy, z0 + 0.07)), ((c, cy, z1 - 0.07), (c, cy, z1))]
        core = ((c, cy, z0 + 0.07), (c, cy, z1 - 0.07))
    else:
        c = (z0 + z1) / 2
        ends = [((x0, cy, c), (x0 + 0.07, cy, c)), ((x1 - 0.07, cy, c), (x1, cy, c))]
        core = ((x0 + 0.07, cy, c), (x1 - 0.07, cy, c))
    for (a, e) in ends:
        cylinder(s, "timber", a, e, r - 0.01, 16, TIMBER)
    cylinder(s, "steel", core[0], core[1], r * 0.72, 16, (0.13, 0.13, 0.13), caps=False)
    cylinder(s, "steel", ends[0][0], ends[1][1], 0.06, 8, GALV)


def jersey(s, b):
    """A concrete barrier along the box's long axis, its footing below."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    hw = ((z1 - z0) if along_x else (x1 - x0)) / 2
    prof = [(-hw, 0.0), (hw, 0.0), (hw, 0.08), (hw * 0.62, 0.33), (hw * 0.25, y1),
            (-hw * 0.25, y1), (-hw * 0.62, 0.33), (-hw, 0.08)]
    tint = (0.78, 0.77, 0.73)
    if along_x:
        zc = (z0 + z1) / 2
        lo, hi = x0 + 0.03, x1 - 0.03
        pt = lambda t, p: (t, p[1], zc + p[0])
        c = ((x0 + x1) / 2, y1 / 2, zc)
    else:
        xc = (x0 + x1) / 2
        lo, hi = z0 + 0.03, z1 - 0.03
        pt = lambda t, p: (xc + p[0], p[1], t)
        c = (xc, y1 / 2, (z0 + z1) / 2)
    s.new_piece()
    for i in range(len(prof)):
        p, q = prof[i], prof[(i + 1) % len(prof)]
        face_out(s, "concrete", [pt(lo, p), pt(lo, q), pt(hi, q), pt(hi, p)], tint, c)
    face_out(s, "concrete", [pt(lo, p) for p in prof], tint, c)
    face_out(s, "concrete", [pt(hi, p) for p in prof], tint, c)
    if y0 < 0:
        f = 0.04
        if along_x:
            s.box("concrete", (x0 + f, y0, z0 + f, x1 - f, 0.0, z1 - f), (0.62, 0.61, 0.58))
        else:
            s.box("concrete", (x0 + f, y0, z0 + f, x1 - f, 0.0, z1 - f), (0.62, 0.61, 0.58))


def floodlight(s, b, centre=(0.0, 0.0)):
    """A tapered pole on its plate, and twin lamps at the top aimed in at
    the compound."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    hw = (x1 - x0) / 2
    s.box("steel", (x0, y0, z0, x1, y0 + 0.03, z1), (0.35, 0.35, 0.34))
    n = 4
    for i in range(n):
        ya, yb = y0 + (y1 - y0) * i / n, y0 + (y1 - y0) * (i + 1) / n
        r = hw * 0.75 * (1 - 0.35 * i / n)
        s.prism("steel", (cx, ya, cz), (cx, yb, cz), r, 8, GALV)
    dx, dz = centre[0] - cx, centre[1] - cz
    ln = math.hypot(dx, dz)
    fx, fz = dx / ln, dz / ln
    ux, uz = -fz, fx
    arm = y1 - 0.15
    s.prism("steel", (cx - ux * 0.6, arm, cz - uz * 0.6), (cx + ux * 0.6, arm, cz + uz * 0.6), 0.04, 6, GALV)
    for k in (-0.5, 0.5):
        lx, lz = cx + ux * k + fx * 0.15, cz + uz * k + fz * 0.15
        ybox(s, "steel", (lx, arm + 0.12, lz), (ux, uz), 0.42, 0.3, 0.34, (0.3, 0.31, 0.32))
        face_out(s, "bulb", [(lx - ux * 0.17 - fx * 0.13, arm - 0.035, lz - uz * 0.17 - fz * 0.13),
                             (lx + ux * 0.17 - fx * 0.13, arm - 0.035, lz + uz * 0.17 - fz * 0.13),
                             (lx + ux * 0.17 + fx * 0.13, arm - 0.035, lz + uz * 0.17 + fz * 0.13),
                             (lx - ux * 0.17 + fx * 0.13, arm - 0.035, lz - uz * 0.17 + fz * 0.13)],
                 (1, 1, 1), (lx, arm + 1.0, lz))


def relay_roof(s, b, hut_b, rng):
    """The generator hut's roof: its sheet, the air conditioner, the
    exhaust stack with soot round its foot, and a whip antenna."""
    x0, y0, z0, x1, y1, z1 = b
    s.box("steel", b, (0.4, 0.42, 0.43), bevel=0.02)
    hx0, hz0, hx1, hz1 = hut_b[0], hut_b[2], hut_b[3], hut_b[5]
    # The AC unit near the tower end, its fan grille on top.
    ax = hx1 - 1.1
    s.box("steel", (ax - 0.55, y1, hz0 + 0.5, ax + 0.55, y1 + 0.75, hz0 + 1.3), (0.72, 0.73, 0.71), bevel=0.03)
    cylinder(s, "steel", (ax, y1 + 0.75, hz0 + 0.9), (ax, y1 + 0.77, hz0 + 0.9), 0.3, 12, (0.2, 0.2, 0.2))
    # The exhaust: a stack with a rain cap, and the roof blackened round it.
    ex, ez = hx0 + 0.9, hz1 - 0.8
    cylinder(s, "steel", (ex, y1, ez), (ex, y1 + 1.6, ez), 0.09, 10, (0.3, 0.27, 0.24))
    cylinder(s, "steel", (ex, y1 + 1.66, ez), (ex, y1 + 1.7, ez), 0.18, 10, (0.25, 0.23, 0.21))
    pts = [(ex + 0.9 * math.cos(t) * rng.uniform(0.6, 1.0), y1 + 0.004, ez + 0.9 * math.sin(t) * rng.uniform(0.6, 1.0))
           for t in [2 * math.pi * i / 10 for i in range(10)]]
    face_out(s, "steel", pts, SOOT, (ex, y1 - 1, ez))
    # A whip antenna on the far corner.
    s.prism("steel", (hx0 + 0.3, y1, hz0 + 0.3), (hx0 + 0.3, y1 + 2.4, hz0 + 0.3), 0.012, 4, (0.15, 0.15, 0.15))
    # Rust streaks down the walls from the roof's drip edge.
    for _ in range(9):
        side = rng.randrange(4)
        h = rng.uniform(0.5, 1.6)
        w = rng.uniform(0.06, 0.16)
        top = hut_b[4] - 0.02
        col = (0.5, 0.36, 0.26)
        if side < 2:
            z = hz0 - 0.004 if side == 0 else hz1 + 0.004
            x = rng.uniform(hx0 + 0.3, hx1 - 0.3)
            q = [(x, top - h, z), (x + w, top - h * 0.8, z), (x + w, top, z), (x, top, z)]
            face_out(s, "concrete", q, col, (x, top, (hz0 + hz1) / 2))
        else:
            x = hx0 - 0.004 if side == 2 else hx1 + 0.004
            z = rng.uniform(hz0 + 0.3, hz1 - 0.3)
            q = [(x, top - h, z), (x, top - h * 0.8, z + w), (x, top, z + w), (x, top, z)]
            face_out(s, "concrete", q, col, ((hx0 + hx1) / 2, top, z))


WEED = (0.4, 0.5, 0.22)
DIRT = (0.4, 0.35, 0.26)
REBAR = (0.42, 0.26, 0.16)


def tuft(s, x, y, z, h, rng, tint=WEED):
    """A clump of blades: thin three-sided spikes leaning out of a crack."""
    s.new_piece()
    k = rng.uniform(0.85, 1.15)
    tint = (tint[0] * k, tint[1] * k, tint[2] * k)
    for _ in range(rng.randint(4, 7)):
        a = rng.uniform(0, 2 * math.pi)
        lean = rng.uniform(0.15, 0.5) * h
        tip = (x + math.cos(a) * lean, y + h * rng.uniform(0.55, 1.0), z + math.sin(a) * lean)
        w = rng.uniform(0.012, 0.025)
        base = [(x + math.cos(a + 2.1 * i) * w, y, z + math.sin(a + 2.1 * i) * w) for i in range(3)]
        c = tuple((sum(p[i] for p in base) + tip[i]) / 4 for i in range(3))
        for i in range(3):
            face_out(s, "leaf", [base[i], base[(i + 1) % 3], tip], tint, c)


def pad_life(s, b, rng, keep_out, joint=3.0, fences=()):
    """What years do to a slab, all under ankle height: cracks off the
    joints, soil drifted against the fence with weeds in it, weeds up the
    joints and in the corners, and broken concrete lying about. Nothing
    lands inside `keep_out` (x0, z0, x1, z1) boxes."""
    x0, y0, z0, x1, y1, z1 = b
    top = y1 + 0.004

    def free(x, z, m=0.12):
        if not (x0 + 0.3 < x < x1 - 0.3 and z0 + 0.3 < z < z1 - 0.3):
            return False
        return not any(k[0] - m < x < k[2] + m and k[1] - m < z < k[3] + m for k in keep_out)

    # Cracks: short random walks out of the sawn joints.
    for _ in range(int((x1 - x0) * (z1 - z0) / 14)):
        along_x = rng.random() < 0.5
        if along_x:
            x, z = rng.uniform(x0 + 1, x1 - 1), z0 + joint * rng.randint(1, max(1, int((z1 - z0) / joint) - 1))
        else:
            x, z = x0 + joint * rng.randint(1, max(1, int((x1 - x0) / joint) - 1)), rng.uniform(z0 + 1, z1 - 1)
        a = rng.uniform(0, 2 * math.pi)
        for _ in range(rng.randint(3, 7)):
            a += rng.uniform(-0.7, 0.7)
            ln = rng.uniform(0.15, 0.45)
            nx, nz = x + math.cos(a) * ln, z + math.sin(a) * ln
            if not (free(x, z, 0.0) and free(nx, nz, 0.0)):
                break
            w = rng.uniform(0.008, 0.018)
            px, pz = -math.sin(a) * w, math.cos(a) * w
            face_out(s, "yard", [(x - px, top + 0.001, z - pz), (nx - px, top + 0.001, nz - pz),
                                 (nx + px, top + 0.001, nz + pz), (x + px, top + 0.001, z + pz)],
                     (0.22, 0.22, 0.21), (x, top - 1, z))
            if rng.random() < 0.25:
                tuft(s, nx, top, nz, rng.uniform(0.08, 0.2), rng)
            x, z = nx, nz
    # Soil drifted against the fence's inside, weeds rooted in it.
    for fb in fences:
        fx0, _, fz0, fx1, _, fz1 = fb
        along_x = (fx1 - fx0) >= (fz1 - fz0)
        lo, hi = (fx0, fx1) if along_x else (fz0, fz1)
        inward = (-1 if (fz0 + fz1) > 0 else 1) if along_x else (-1 if (fx0 + fx1) > 0 else 1)
        edge = ((fz0 if inward < 0 else fz1) if along_x else (fx0 if inward < 0 else fx1))
        t = lo + rng.uniform(0.2, 1.5)
        while t < hi - 0.8:
            ln = rng.uniform(0.8, 2.6)
            te = min(hi - 0.3, t + ln)
            depth = rng.uniform(0.25, 0.7)
            pts = []
            n = 7
            for i in range(n + 1):
                u = t + (te - t) * i / n
                d = depth * math.sin(math.pi * i / n) ** 0.7 * rng.uniform(0.7, 1.0)
                pts.append((u, d))
            ring_ = [(te, 0.0), (t, 0.0)] + pts[1:-1]
            q = [((u, top + 0.002, edge + inward * d) if along_x else (edge + inward * d, top + 0.002, u))
                 for (u, d) in ring_]
            mid = ((te + t) / 2, edge + inward * depth * 0.3)
            c = (mid[0], top - 1, mid[1]) if along_x else (mid[1], top - 1, mid[0])
            k = rng.uniform(0.85, 1.1)
            face_out(s, "yard", q, (DIRT[0] * k, DIRT[1] * k, DIRT[2] * k), c)
            for _ in range(int((te - t) * 4.0)):
                u = rng.uniform(t + 0.1, te - 0.1)
                d = rng.uniform(0.05, depth * 0.8)
                px, pz = (u, edge + inward * d) if along_x else (edge + inward * d, u)
                if free(px, pz, 0.05):
                    tuft(s, px, top, pz, rng.uniform(0.15, 0.5) * (1.0 - 0.5 * d / depth), rng)
            t = te + rng.uniform(0.5, 3.5)
    # Weeds where the slab meets anything standing on it.
    for k in keep_out:
        for _ in range(rng.randint(2, 6)):
            side = rng.randrange(4)
            if side < 2:
                x = rng.uniform(k[0], k[2])
                z = (k[1] - 0.08) if side == 0 else (k[3] + 0.08)
            else:
                z = rng.uniform(k[1], k[3])
                x = (k[0] - 0.08) if side == 2 else (k[2] + 0.08)
            if free(x, z, 0.0):
                tuft(s, x, top, z, rng.uniform(0.12, 0.4), rng)
    # Broken concrete in drifts: a few heaps against the edges and the
    # things standing on the slab, big pieces in the middle of each, grit
    # round it, and now and then a rusted bar or a split board.
    heaps = 0
    while heaps < int((x1 - x0) * (z1 - z0) / 40):
        if rng.random() < 0.5 and keep_out:
            k = rng.choice(keep_out)
            x = rng.uniform(k[0], k[2]) + rng.choice((-1, 1)) * 0.0
            z = rng.choice((k[1] - 0.3, k[3] + 0.3))
        else:
            x, z = rng.uniform(x0 + 0.6, x1 - 0.6), rng.uniform(z0 + 0.6, z1 - 0.6)
            if min(x - x0, x1 - x) < min(z - z0, z1 - z):
                x = x0 + rng.uniform(0.4, 1.0) if x - x0 < x1 - x else x1 - rng.uniform(0.4, 1.0)
            else:
                z = z0 + rng.uniform(0.4, 1.0) if z - z0 < z1 - z else z1 - rng.uniform(0.4, 1.0)
        heaps += 1
        for i in range(rng.randint(4, 10)):
            r = 0.5 * rng.random() ** 0.7
            a = rng.uniform(0, 2 * math.pi)
            px, pz = x + math.cos(a) * r, z + math.sin(a) * r
            if not free(px, pz):
                continue
            sz = rng.uniform(0.1, 0.24) * (1.0 - r) if i < 2 else rng.uniform(0.03, 0.09)
            g = rng.uniform(0.55, 0.72)
            yaw = rng.uniform(0, math.pi)
            ybox(s, "concrete", (px, top + sz * 0.25, pz), (math.cos(yaw), math.sin(yaw)),
                 sz * rng.uniform(1.0, 1.7), sz * 0.5, sz, (g, g, g * 0.97))
        if rng.random() < 0.35:
            yaw = rng.uniform(0, math.pi)
            ln = rng.uniform(0.6, 1.4)
            ux, uz = math.cos(yaw) * ln / 2, math.sin(yaw) * ln / 2
            if free(x - ux, z - uz) and free(x + ux, z + uz):
                if rng.random() < 0.5:
                    s.prism("steel", (x - ux, top + 0.012, z - uz), (x + ux, top + 0.05, z + uz), 0.012, 4, REBAR)
                else:
                    ybox(s, "timber", (x, top + 0.015, z), (math.cos(yaw), math.sin(yaw)), ln, 0.025, 0.12, TIMBER)


def dress_relay(s, kit, rng):
    parts = kit["parts"]

    def dims(p):
        b = p["b"]
        return b[3] - b[0], b[4] - b[1], b[5] - b[2]

    legs = [p for p in parts if p["mat"] == "steel" and dims(p)[1] > 15 and dims(p)[0] < 0.6]
    top = max(p["b"][4] for p in legs)
    pad = next(p for p in parts if p["mat"] == "concrete" and p["b"][4] < 0.5 and dims(p)[0] > 10)
    hut_p = next(p for p in parts if p["mat"] == "concrete" and p["b"][4] > 2)
    floor = pad["b"][4]
    xs = sorted({round((p["b"][0] + p["b"][3]) / 2, 3) for p in legs})
    zs = sorted({round((p["b"][2] + p["b"][5]) / 2, 3) for p in legs})
    corners = [(xs[0], zs[0]), (xs[1], zs[0]), (xs[1], zs[1]), (xs[0], zs[1])]
    tower_c = ((xs[0] + xs[1]) / 2, (zs[0] + zs[1]) / 2)

    # The pad: joints, stains, and a hazard line round the tower's base.
    x0, _, z0, x1, _, z1 = (xs[0] - 0.6, 0, zs[0] - 0.6, xs[1] + 0.6, 0, zs[1] + 0.6)
    w = 0.08
    lines = [((x0, z0), (x1, z0), (x1, z0 + w)), ((x0, z1 - w), (x1, z1 - w), (x1, z1)),
             ((x0, z0), (x0 + w, z0), (x0 + w, z1)), ((x1 - w, z0), (x1, z0), (x1, z1))]
    slab(s, pad["b"], rng, joint=3.0, lines=lines)
    on_pad = [p for p in parts if p is not pad and p["b"][1] >= floor - 0.01 and p["mat"] != "concrete"
              or p is hut_p]
    fences = [p["b"] for p in on_pad if p["mat"] == "steel" and min(dims(p)[0], dims(p)[2]) <= 0.25
              and dims(p)[1] < 2.5]
    keep = [(p["b"][0], p["b"][2], p["b"][3], p["b"][5]) for p in on_pad
            if p["b"][1] < 2.0 and p["b"] not in fences]
    pad_life(s, pad["b"], rng, keep, joint=3.0, fences=fences)
    # A loose cable from the drum across the pad to the tower's foot, lying flat.
    pts = [(6.4, -0.6), (5.6, 0.3), (4.9, 0.6), (4.4, 1.3), (4.0, 1.6)]
    for i in range(len(pts) - 1):
        a, b = pts[i], pts[i + 1]
        s.prism("steel", (a[0], floor + 0.02, a[1]), (b[0], floor + 0.02, b[1]), 0.02, 4, (0.12, 0.12, 0.12))

    # The tower: galvanised legs, bracing, platforms, ladder, antennas.
    for p in legs:
        angle_leg(s, p["b"], GALV, tower_c)
    for i in range(4):
        lattice_face(s, corners[i], corners[(i + 1) % 4], 2.4, top, 2.5, lambda y: GALV)
    ladder_z = zs[0]
    caged_ladder(s, tower_c[0] - 0.25, tower_c[0] + 0.25, ladder_z + 0.04, -1, 2.4, top + 1.1)
    # Feeder cables down the west leg to the bridge, and the bridge to the hut.
    for k in (-0.05, 0.0, 0.05):
        s.prism("steel", (xs[0] - 0.26, 3.3, zs[0] + 0.6 + k), (xs[0] - 0.26, top, zs[0] + 0.6 + k), 0.014, 4,
                (0.1, 0.1, 0.1))
    hb = hut_p["b"]
    by = 3.25
    bz = zs[0] + 0.6
    s.box("steel", (hb[3], by, bz - 0.18, xs[0] - 0.2, by + 0.05, bz + 0.18), GALV)
    for k in (-0.18, 0.18):
        s.box("steel", (hb[3], by, bz + k - 0.01, xs[0] - 0.2, by + 0.12, bz + k + 0.01), GALV)
    for k in (-0.06, 0.0, 0.06):
        s.prism("steel", (hb[3], by + 0.08, bz + k), (xs[0] - 0.2, by + 0.08, bz + k), 0.014, 4, (0.1, 0.1, 0.1))

    for p in parts:
        b = p["b"]
        bx0, by0, bz0, bx1, by1, bz1 = b
        dx, dy, dz = dims(p)
        if p is pad or p in legs:
            continue
        if p["mat"] == "steel" and min(dx, dz) <= 0.25 and dy < 2.5 and max(dx, dz) > 4:
            near_gate = bz1 < -8 and max(abs(bx0), abs(bx1)) > 8
            fence_run(s, b, rng, signs=1 if (near_gate or bx0 > 8) else 0)
            # The gate leaves hang off the posts at the gap's two sides.
            if bz1 < -8:
                if abs(bx1) < 3:
                    gate_leaf(s, (bx1, bz0), (-1.0, 0.0), (0.0, -1.0), w=min(2.0, (bx1 - bx0) - 0.3))
                elif abs(bx0) < 3:
                    gate_leaf(s, (bx0, bz0), (1.0, 0.0), (0.0, -1.0), w=min(2.0, (bx1 - bx0) - 0.3))
        elif p["mat"] == "steel" and dy <= 0.35 and by0 > 5:
            platform(s, b, rail=True)
            if by0 >= top - 0.1:
                # Panel antennas on three corners, a microwave drum on the fourth face.
                cx, cz = (bx0 + bx1) / 2, (bz0 + bz1) / 2
                for (ox, oz) in ((1, 1), (1, -1), (-1, 1)):
                    base = (cx + ox * (dx / 2 - 0.2), by1, cz + oz * (dz / 2 - 0.2))
                    ln = math.hypot(ox, oz)
                    panel_antenna(s, base, (ox / ln, oz / ln))
                drum_dish(s, (xs[0] - 0.4, by0 - 2.4, cz + 0.4), (-1.0, 0.0))
                drum_dish(s, (cx - 0.5, by0 - 4.2, zs[1] + 0.4), (0.0, 1.0), r=0.3)
        elif p["mat"] == "steel" and dy <= 0.35:
            relay_roof(s, b, hb, rng)
        elif p["mat"] == "steel" and dx < 0.6 and dz < 0.6 and by0 >= top - 1:
            cx, cz = (bx0 + bx1) / 2, (bz0 + bz1) / 2
            n = 6
            for i in range(n):
                ya, yb = by0 + dy * i / n, by0 + dy * (i + 1) / n
                r = 0.15 * (1 - 0.6 * i / n)
                s.prism("steel", (cx, ya, cz), (cx, yb, cz), r, 8, MAST_RED if i % 2 == 0 else MAST_WHITE)
            s.box("bulb", (cx - 0.1, by1, cz - 0.1, cx + 0.1, by1 + 0.22, cz + 0.1))
        elif p["mat"] == "steel" and min(dx, dz) <= 0.45 and dy >= 1.0 and by0 > 5:
            thin_x = dx < dz
            c = ((bx0 + bx1) / 2, (by0 + by1) / 2, (bz0 + bz1) / 2)
            facing = ((1 if c[0] > tower_c[0] else -1), 0) if thin_x else (0, (1 if c[2] > tower_c[1] else -1))
            back = (bx0 if facing[0] > 0 else bx1) if thin_x else (bz0 if facing[1] > 0 else bz1)
            base = (back, c[1], c[2]) if thin_x else (c[0], c[1], back)
            dish(s, base, facing, min(dy, dz if thin_x else dx) / 2)
            # Its mount: a pipe across the face between the legs.
            if thin_x:
                s.prism("steel", (back - 0.05, c[1], zs[0]), (back - 0.05, c[1], zs[1]), 0.04, 6, GALV)
            else:
                s.prism("steel", (xs[0], c[1], back - 0.05), (xs[1], c[1], back - 0.05), 0.04, 6, GALV)
        elif p["mat"] == "steel" and dx <= 0.35 and dz <= 0.35 and dy > 4:
            floodlight(s, b, tower_c)
        elif p["mat"] == "steel" and max(dx, dz) > 3 and dy < 2:
            fuel_tank(s, b)
            # Its feed into the hut's back wall, along the pad.
            fx = (max(bx0, hb[0]) + min(bx1, hb[3])) / 2
            s.prism("steel", (fx, floor + 0.12, bz0 + 0.05), (fx, floor + 0.12, hb[5] - 0.01), 0.025, 6, GALV)
        elif p["mat"] == "steel" and dz < 1.0 and dy > 1.5:
            cabinets(s, b)
            # The bridge's post stands on the cabinets' top.
            s.prism("steel", ((bx0 + bx1) / 2, by1, bz), ((bx0 + bx1) / 2, by, bz), 0.035, 6, GALV)
        elif p["mat"] == "steel":
            drum_pallet(s, b, rng)
        elif p["mat"] == "concrete" and by1 > 2:
            hut(s, b, rng)
        elif p["mat"] == "concrete":
            jersey(s, b)
        elif p["mat"] == "cargo":
            container(s, (bx0, max(by0, floor), bz0, bx1, by1, bz1), rng)
        elif p["mat"] == "timber":
            cable_drum(s, b, rng)
        else:
            s.box(p["mat"], b, (1, 1, 1))


# ── the quarry ──────────────────────────────────────────────────────────────
#
# Fresh-cut benches stepped into the hill, drilled in lifts, scree at their
# feet; a gravel floor with a cart on its rails; a lattice derrick with a
# block hanging off its jib; a hopper fed by a conveyor off the bench; the
# cutters' shed. Below head height everything stays on its part.

QSTONE = (0.9, 0.87, 0.8)
QWORN = (0.66, 0.64, 0.6)
GRAVEL = (1.0, 0.93, 0.82)
CART = (0.5, 0.25, 0.15)


def obox(s, role, a, b, w, h, tint):
    """A box from point `a` to point `b`, `w` wide across and `h` deep up —
    a beam at any slope (conveyor channels, belts, braces)."""
    d = [b[i] - a[i] for i in range(3)]
    ln = math.sqrt(sum(x * x for x in d))
    d = [x / ln for x in d]
    side = (-d[2], 0.0, d[0])
    sn = math.hypot(side[0], side[2]) or 1.0
    side = (side[0] / sn, 0.0, side[2] / sn)
    up = (side[1] * d[2] - side[2] * d[1], side[2] * d[0] - side[0] * d[2], side[0] * d[1] - side[1] * d[0])
    if up[1] < 0:
        up = tuple(-x for x in up)
    c = tuple((a[i] + b[i]) / 2 for i in range(3))
    pts = {}
    for e, p in ((0, a), (1, b)):
        for sw in (-1, 1):
            for sh in (-1, 1):
                pts[(e, sw, sh)] = tuple(p[i] + side[i] * sw * w / 2 + up[i] * sh * h / 2 for i in range(3))
    s.new_piece()
    for (q) in ([(0, -1, -1), (0, 1, -1), (0, 1, 1), (0, -1, 1)], [(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)],
                [(0, -1, -1), (1, -1, -1), (1, 1, -1), (0, 1, -1)], [(0, -1, 1), (1, -1, 1), (1, 1, 1), (0, 1, 1)],
                [(0, -1, -1), (1, -1, -1), (1, -1, 1), (0, -1, 1)], [(0, 1, -1), (1, 1, -1), (1, 1, 1), (0, 1, 1)]):
        face_out(s, role, [pts[k] for k in q], tint, c)


def faces_toward(b, centre):
    """The vertical faces of box `b` that look at `centre` (x, z), as
    (axis, sign) with axis 0 = x, 2 = z."""
    out = []
    cx, cz = (b[0] + b[3]) / 2, (b[2] + b[5]) / 2
    for axis, sign in ((0, -1), (0, 1), (2, -1), (2, 1)):
        plane = b[axis + 3] if sign > 0 else b[axis]
        towards = (centre[0] - plane) if axis == 0 else (centre[1] - plane)
        if towards * sign > 0.5:
            out.append((axis, sign))
    return out


def open_span(b, axis, sign, others, floor):
    """Where face (axis, sign) of `b` is open to the air: the tangent span
    and the height it starts at, after the walls that stand against it."""
    plane = b[axis + 3] if sign > 0 else b[axis]
    t = 2 if axis == 0 else 0
    lo, hi = b[t], b[t + 3]
    v0 = floor
    eps = 0.1
    against = []
    for q in others:
        qb = q["b"]
        # It stands in front if it fills the first stretch of air off the face.
        if not (qb[axis] < plane + sign * eps < qb[axis + 3]):
            continue
        a, c = max(lo, qb[t]), min(hi, qb[t + 3])
        if c - a > 0.05:
            against.append((a, c, qb[4]))
    for (a, c, top) in sorted(against, key=lambda r: (r[1] - r[0]) / (hi - lo)):
        a, c = max(lo, a), min(hi, c)
        if c - a <= 0.05:
            continue
        if (c - a) >= 0.8 * (hi - lo):
            v0 = max(v0, top)
        elif a - lo > hi - c:
            hi = a
        else:
            lo = c
    return lo, hi, v0, plane


def cut_face(s, axis, sign, plane, lo, hi, v0, v1, rng):
    """A worked face on the plane: lifts of fresh stone with a lip at each
    lift line, drill-hole grooves down from each lift's top, and a few dark
    spalls where a blast took more than the drill meant."""
    t = 2 if axis == 0 else 0
    off = plane + sign * 0.035

    def at(u, y, d=0.0):
        p = [0.0, y, 0.0]
        p[axis] = off + sign * d
        p[t] = u
        return tuple(p)

    behind = at((lo + hi) / 2, (v0 + v1) / 2, -1.0)
    y = v0
    lifts = []
    while y < v1 - 0.3:
        y2 = min(v1, y + rng.uniform(1.2, 1.9))
        if v1 - y2 < 0.5:
            y2 = v1
        lifts.append((y, y2))
        y = y2
    for (ya, yb) in lifts:
        s.new_piece()
        k = rng.uniform(0.9, 1.04)
        tint = (QSTONE[0] * k, QSTONE[1] * k, QSTONE[2] * k * 0.98)
        face_out(s, "ashlar", [at(lo, ya), at(hi, ya), at(hi, yb), at(lo, yb)], tint, behind)
        # The drill lines: half-holes left by the split, from the lift's top.
        u = lo + rng.uniform(0.1, 0.3)
        while u < hi - 0.1:
            depth = (yb - ya) * rng.uniform(0.55, 1.0)
            w = 0.022
            face_out(s, "ashlar", [at(u - w, yb - depth, 0.002), at(u + w, yb - depth, 0.002),
                                   at(u + w, yb - 0.02, 0.002), at(u - w, yb - 0.02, 0.002)],
                     (tint[0] * 0.55, tint[1] * 0.55, tint[2] * 0.55), behind)
            u += rng.uniform(0.28, 0.5)
        # The lip where the next lift was split off above.
        if yb < v1:
            lip = [at(lo, yb - 0.05, 0.0), at(hi, yb - 0.05, 0.0), at(hi, yb + 0.03, 0.0), at(lo, yb + 0.03, 0.0)]
            face_out(s, "ashlar", [at(lo, yb + 0.03, 0.012), at(hi, yb + 0.03, 0.012),
                                   at(hi, yb - 0.05, 0.012), at(lo, yb - 0.05, 0.012)],
                     (tint[0] * 0.8, tint[1] * 0.8, tint[2] * 0.8), behind)
            face_out(s, "ashlar", [lip[3], lip[2], at(hi, yb + 0.03, 0.012), at(lo, yb + 0.03, 0.012)],
                     (tint[0] * 0.7, tint[1] * 0.7, tint[2] * 0.7), at((lo + hi) / 2, yb - 1.0, 0.0))
    # Spalls.
    for _ in range(int((hi - lo) * (v1 - v0) / 18)):
        cu, cy = rng.uniform(lo + 0.6, hi - 0.6), rng.uniform(v0 + 0.4, v1 - 0.4)
        r = rng.uniform(0.25, 0.7)
        pts = [at(cu + r * math.cos(a) * rng.uniform(0.5, 1.0), cy + r * 0.7 * math.sin(a) * rng.uniform(0.5, 1.0), 0.004)
               for a in [2 * math.pi * i / 7 for i in range(7)]]
        g = rng.uniform(0.62, 0.75)
        face_out(s, "ashlar", pts, (QSTONE[0] * g, QSTONE[1] * g, QSTONE[2] * g), behind)


def scree(s, axis, sign, plane, lo, hi, top, rng, floor_b, avoid=()):
    """Fans of spoil where a face meets the floor: big pieces against the
    face, smaller further out, on a darker bed of fines — kept off `avoid`
    (x0, z0, x1, z1) boxes."""
    t = 2 if axis == 0 else 0
    fx0, _, fz0, fx1, _, fz1 = floor_b

    def on_floor(p):
        if any(k[0] - 0.2 < p[0] < k[2] + 0.2 and k[1] - 0.2 < p[2] < k[3] + 0.2 for k in avoid):
            return False
        return fx0 + 0.1 < p[0] < fx1 - 0.1 and fz0 + 0.1 < p[2] < fz1 - 0.1

    u = lo + rng.uniform(0.5, 2.5)
    while u < hi - 0.8:
        w = rng.uniform(1.2, 3.0)
        reach = rng.uniform(0.8, 1.8)
        c = [0.0, top, 0.0]
        c[axis] = plane + sign * reach * 0.4
        c[t] = u
        bed = []
        for i in range(9):
            a = math.pi * i / 8
            p = [0.0, top + 0.004, 0.0]
            p[axis] = plane + sign * reach * math.sin(a) * rng.uniform(0.8, 1.0)
            p[t] = u + w / 2 * math.cos(a)
            bed.append(tuple(p))
        if all(on_floor(p) for p in bed):
            face_out(s, "yard", bed, (GRAVEL[0] * 0.78, GRAVEL[1] * 0.76, GRAVEL[2] * 0.74), (c[0], top - 1, c[2]))
            for _ in range(int(w * reach * 9)):
                f = rng.random() ** 1.5
                p = [0.0, 0.0, 0.0]
                p[axis] = plane + sign * (0.15 + reach * f * 0.95)
                p[t] = u + rng.uniform(-w / 2, w / 2) * (1.0 - 0.5 * f)
                sz = rng.uniform(0.06, 0.32) * (1.0 - 0.75 * f)
                p[1] = top + sz * 0.3
                if not on_floor(p):
                    continue
                yaw = rng.uniform(0, math.pi)
                g = rng.uniform(0.75, 1.0)
                ybox(s, "ashlar", tuple(p), (math.cos(yaw), math.sin(yaw)), sz * rng.uniform(1.0, 1.6),
                     sz * 0.7, sz, (QSTONE[0] * g, QSTONE[1] * g, QSTONE[2] * g))
        u += w + rng.uniform(1.0, 4.0)


def cut_wall(s, p, parts, floor_b, centre, rng, avoid=()):
    """One of the quarry's walls. Its body is the hill's rock, which the
    client draws (`render/boulders.rs`); this works the sides that face the
    floor — a skin of fresh stone with its lifts and drill lines, the scree
    at its foot, and weeds and loose stone along the cut's rim."""
    b = p["b"]
    floor = floor_b[4]
    others = [q for q in parts if q is not p and q["mat"] == "rock" and q["b"][4] >= 3.0]
    for axis, sign in faces_toward(b, centre):
        lo, hi, v0, plane = open_span(b, axis, sign, others, floor)
        if hi - lo < 1.0 or b[4] - 0.2 - v0 < 0.6:
            continue
        # A crisp skin of fresh rock behind the worked face, standing a
        # little proud of the weathered rock's rounded rim.
        fresh = list(b)
        fresh[axis] = plane - 1.4 if sign > 0 else plane
        fresh[axis + 3] = plane if sign > 0 else plane + 1.4
        t2 = 2 if axis == 0 else 0
        fresh[t2], fresh[t2 + 3] = max(b[t2], lo - 0.3), min(b[t2 + 3], hi + 0.3)
        k = rng.uniform(0.95, 1.05)
        rough(s, "ashlar", tuple(fresh), (QWORN[0] * 1.1 * k, QWORN[1] * 1.1 * k, QWORN[2] * 1.08 * k), rng,
              amp=0.03, cell=0.8, crown=0.06)
        # The rim: weeds and loose stone along the skin's top.
        t2 = 2 if axis == 0 else 0
        for _ in range(int((fresh[t2 + 3] - fresh[t2]) * 2.5)):
            pt = [0.0, 0.0, 0.0]
            pt[axis] = rng.uniform(fresh[axis] + 0.2, fresh[axis + 3] - 0.2)
            pt[t2] = rng.uniform(fresh[t2] + 0.3, fresh[t2 + 3] - 0.3)
            big = rng.random() < 0.2
            tuft(s, pt[0], b[4] - 0.08, pt[2], rng.uniform(0.5, 0.9) if big else rng.uniform(0.25, 0.5), rng)
        for _ in range(int((fresh[t2 + 3] - fresh[t2]) / 3)):
            pt = [0.0, 0.0, 0.0]
            pt[axis] = rng.uniform(fresh[axis] + 0.4, fresh[axis + 3] - 0.4)
            pt[t2] = rng.uniform(fresh[t2] + 0.6, fresh[t2 + 3] - 0.6)
            sz = rng.uniform(0.2, 0.45)
            yaw = rng.uniform(0, math.pi)
            ybox(s, "ashlar", (pt[0], b[4] - 0.06 + sz * 0.35, pt[2]), (math.cos(yaw), math.sin(yaw)), sz * 1.5,
                 sz * 0.7, sz, QWORN)
        cut_face(s, axis, sign, plane, lo + 0.08, hi - 0.08, v0 - 0.08 if v0 <= floor else v0, b[4] - 0.18, rng)
        if v0 <= floor + 0.01:
            scree(s, axis, sign, plane, lo, hi, floor, rng, floor_b, avoid)


def cut_block(s, b, rng, split=False):
    """A sawn block on the floor: crisp, pale, a row of half drill holes
    down one edge, and — when `split` — cracked in two with the steel
    wedges still standing in the line."""
    x0, y0, z0, x1, y1, z1 = b
    along_x = (x1 - x0) >= (z1 - z0)
    k = rng.uniform(0.92, 1.02)
    tint = (QSTONE[0] * k, QSTONE[1] * k, QSTONE[2] * k)
    if split:
        g = 0.05
        if along_x:
            m = (x0 + x1) / 2 + rng.uniform(-0.2, 0.2)
            halves = [(x0, y0, z0, m - g / 2, y1, z1), (m + g / 2, y0, z0, x1, y1 - 0.04, z1)]
        else:
            m = (z0 + z1) / 2 + rng.uniform(-0.2, 0.2)
            halves = [(x0, y0, z0, x1, y1, m - g / 2), (x0, y0, m + g / 2, x1, y1 - 0.04, z1)]
        for h in halves:
            rough(s, "ashlar", h, tint, rng, amp=0.02, cell=0.6, out=0.01)
        n = 5
        for i in range(n):
            f = (i + 0.5) / n
            if along_x:
                wx, wz = m, z0 + (z1 - z0) * f
                s.box("steel", (wx - 0.035, y1 - 0.08, wz - 0.03, wx + 0.035, y1 + 0.09, wz + 0.03), (0.3, 0.3, 0.32))
            else:
                wx, wz = x0 + (x1 - x0) * f, m
                s.box("steel", (wx - 0.03, y1 - 0.08, wz - 0.035, wx + 0.03, y1 + 0.09, wz + 0.035), (0.3, 0.3, 0.32))
    else:
        rough(s, "ashlar", b, tint, rng, amp=0.02, cell=0.6, out=0.01)
    # Half holes down the long faces' top edge.
    dark = (tint[0] * 0.5, tint[1] * 0.5, tint[2] * 0.5)
    for zf, sg in (((z0, -1), (z1, 1)) if along_x else ((x0, -1), (x1, 1))):
        u = (x0 if along_x else z0) + 0.15
        end = (x1 if along_x else z1) - 0.1
        while u < end:
            if along_x:
                q = [(u - 0.02, y1 - 0.45, zf + sg * 0.012), (u + 0.02, y1 - 0.45, zf + sg * 0.012),
                     (u + 0.02, y1 - 0.02, zf + sg * 0.012), (u - 0.02, y1 - 0.02, zf + sg * 0.012)]
                face_out(s, "ashlar", q, dark, (u, y1 - 0.2, zf - sg))
            else:
                q = [(zf + sg * 0.012, y1 - 0.45, u - 0.02), (zf + sg * 0.012, y1 - 0.45, u + 0.02),
                     (zf + sg * 0.012, y1 - 0.02, u + 0.02), (zf + sg * 0.012, y1 - 0.02, u - 0.02)]
                face_out(s, "ashlar", q, dark, (zf - sg, y1 - 0.2, u))
            u += 0.3


def derrick(s, mast_b, jib_b, winch_b, rng):
    """A lattice derrick in crane yellow: the mast, its cab, a triangular
    jib with the trolley and a block hanging in its sling, a counter-jib
    and its weight, the king post and its ties, and the winch's rope."""
    x0, y0, z0, x1, y1, z1 = mast_b
    floor = max(y0, 0.02)
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    s.box("concrete", (x0, y0, z0, x1, floor + 0.25, z1), (0.75, 0.74, 0.7), bevel=0.03)
    t = 0.07
    legs = [(x0, z0), (x1 - t, z0), (x1 - t, z1 - t), (x0, z1 - t)]
    for (lx, lz) in legs:
        s.box("steel", (lx, floor + 0.25, lz, lx + t, y1, lz + t), CRANE)
    inner = [(x0 + t / 2, z0 + t / 2), (x1 - t / 2, z0 + t / 2), (x1 - t / 2, z1 - t / 2), (x0 + t / 2, z1 - t / 2)]
    for i in range(4):
        lattice_face(s, inner[i], inner[(i + 1) % 4], floor + 0.25, y1, 1.0, lambda y: CRANE, r=0.022)
    # The slewing ring and the cab off the mast's top, on the side away from the jib.
    jx0, jy0, jz0, jx1, jy1, jz1 = jib_b
    s.box("steel", (x0 - 0.15, y1 - 0.6, z0 - 0.15, x1 + 0.15, y1 - 0.4, z1 + 0.15), (0.3, 0.3, 0.3))
    cab_z = z1 + 0.05
    s.box("steel", (cx - 0.55, y1 - 2.4, cab_z, cx + 0.55, y1 - 0.7, cab_z + 1.1), CRANE, bevel=0.03)
    face_out(s, "steel", [(cx - 0.45, y1 - 1.6, cab_z + 1.105), (cx + 0.45, y1 - 1.6, cab_z + 1.105),
                          (cx + 0.45, y1 - 0.9, cab_z + 1.105), (cx - 0.45, y1 - 0.9, cab_z + 1.105)],
             (0.08, 0.1, 0.12), (cx, y1 - 1.2, cab_z))
    # The jib: two bottom chords, a top chord, diagonals between.
    jz = (jz0 + jz1) / 2
    hw = (jz1 - jz0) / 2 - 0.03
    tip, root = (jx0, jx1) if abs(jx0 - cx) > abs(jx1 - cx) else (jx1, jx0)
    span = [tip, max(root, x1 + 2.6) if tip < cx else min(root, x0 - 2.6)]
    a, e = min(span), max(span)
    yb, yt = jy0 + 0.05, jy1 - 0.02
    for zz in (jz - hw, jz + hw):
        s.prism("steel", (a, yb, zz), (e, yb, zz), 0.035, 4, CRANE)
    s.prism("steel", (a, yt, jz), (e, yt, jz), 0.04, 4, CRANE)
    x = a
    flip = 1
    while x < e - 0.05:
        x2 = min(e, x + 0.6)
        for zz in (jz - hw, jz + hw):
            s.prism("steel", (x, yb, zz), (x2, yt, jz) if flip > 0 else (x, yt, jz), 0.018, 4, CRANE)
            s.prism("steel", (x2, yb, zz), (x2, yt, jz) if flip < 0 else (x, yt, jz), 0.018, 4, CRANE)
        s.prism("steel", (x, yb, jz - hw), (x, yb, jz + hw), 0.016, 4, CRANE)
        x = x2
        flip = -flip
    # The counterweight on the short end.
    back = e if tip < cx else a
    sgn = 1 if tip < cx else -1
    s.box("concrete", (min(back, back - sgn * 1.2), yb - 1.1, jz - 0.5, max(back, back - sgn * 1.2), yb - 0.05, jz + 0.5),
          (0.7, 0.69, 0.65), bevel=0.04)
    # King post and ties.
    kp = (cx, y1 + 2.6, cz)
    s.prism("steel", (cx, y1, cz), kp, 0.06, 6, CRANE)
    for end in (tip, back):
        s.prism("steel", kp, (end, yt, jz), 0.012, 4, (0.3, 0.3, 0.3))
    # Trolley, ropes, hook, and a block in its sling.
    hx = tip - sgn * -1.6 if tip < cx else tip - 1.6
    hx = tip + 1.8 if tip < cx else tip - 1.8
    s.box("steel", (hx - 0.3, yb - 0.3, jz - 0.25, hx + 0.3, yb, jz + 0.25), (0.3, 0.3, 0.3))
    hook_y = 5.4
    for dz in (-0.08, 0.08):
        s.prism("steel", (hx, yb - 0.3, jz + dz), (hx, hook_y + 0.4, jz + dz), 0.012, 4, (0.15, 0.15, 0.15))
    s.box("steel", (hx - 0.18, hook_y, jz - 0.12, hx + 0.18, hook_y + 0.4, jz + 0.12), CRANE, bevel=0.03)
    blk = (hx - 0.8, 3.2, jz - 0.45, hx + 0.8, 4.1, jz + 0.45)
    rough(s, "ashlar", blk, QSTONE, rng, amp=0.02, cell=0.5, out=0.01)
    for (sx, sz) in ((-0.6, -0.4), (0.6, -0.4), (-0.6, 0.4), (0.6, 0.4)):
        s.prism("steel", (hx, hook_y, jz), (hx + sx, blk[4], jz + sz), 0.01, 4, (0.2, 0.2, 0.2))
    # The winch: a skid, its drum, an engine box, and the rope rising out
    # of reach before it crosses to the mast.
    wx0, wy0, wz0, wx1, wy1, wz1 = winch_b
    wf = max(wy0, 0.02)
    for zz in (wz0 + 0.1, wz1 - 0.25):
        ibeam_h(s, (wx0, wf, zz, wx1, wf + 0.2, zz + 0.15))
    s.box("steel", (wx0, wf + 0.2, wz0, wx1, wf + 0.26, wz1), (0.35, 0.35, 0.33))
    dz_ = (wz0 + wz1) / 2
    s.box("steel", (wx1 - 1.0, wf + 0.26, wz0 + 0.15, wx1 - 0.1, wy1, wz1 - 0.15), CRANE, bevel=0.03)
    for i in range(4):
        y = wf + 0.5 + i * 0.18
        s.box("steel", (wx1 - 0.1, y, wz0 + 0.3, wx1 - 0.08, y + 0.08, wz1 - 0.3), (0.2, 0.2, 0.2))
    s.prism("steel", (wx1 - 0.3, wy1, wz0 + 0.3), (wx1 - 0.3, wy1 + 0.9, wz0 + 0.3), 0.04, 6, (0.25, 0.23, 0.2))
    drum_y = wf + 0.75
    da, de = (wx0 + 0.15, drum_y, dz_), (wx1 - 1.15, drum_y, dz_)
    cylinder(s, "steel", da, de, 0.42, 14, (0.3, 0.3, 0.32))
    cylinder(s, "steel", (da[0] + 0.08, drum_y, dz_), (de[0] - 0.08, drum_y, dz_), 0.36, 14, (0.12, 0.12, 0.12), caps=False)
    up = (wx0 + 0.4, 2.2, dz_)
    s.prism("steel", (wx0 + 0.4, drum_y + 0.36, dz_), up, 0.012, 4, (0.15, 0.15, 0.15))
    s.prism("steel", up, (x1, 2.4, cz), 0.012, 4, (0.15, 0.15, 0.15))


def ibeam_h(s, b):
    """A skid rail: a low I-section along the box's long axis."""
    x0, y0, z0, x1, y1, z1 = b
    t = 0.03
    s.box("steel", (x0, y0, z0, x1, y0 + t, z1), (0.3, 0.3, 0.3))
    s.box("steel", (x0, y1 - t, z0, x1, y1, z1), (0.3, 0.3, 0.3))
    zc = (z0 + z1) / 2
    s.box("steel", (x0, y0, zc - 0.012, x1, y1, zc + 0.012), (0.3, 0.3, 0.3))


def hopper(s, b, rng):
    """A stone hopper on four legs: the bin's sloped bottom to its outlet
    gate, stiffened walls above, rust running down them, and the spill of
    crushed stone underneath."""
    x0, y0, z0, x1, y1, z1 = b
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    leg_top = y0 + (y1 - y0) * 0.5
    slope_top = y0 + (y1 - y0) * 0.68
    paint = (0.42, 0.45, 0.4)
    t = 0.14
    for (lx, lz) in ((x0, z0), (x1 - t, z0), (x1 - t, z1 - t), (x0, z1 - t)):
        s.box("steel", (lx, y0, lz, lx + t, slope_top, lz + t), paint)
        s.box("steel", (lx - 0.04, y0, lz - 0.04, lx + t + 0.04, y0 + 0.03, lz + t + 0.04), (0.3, 0.3, 0.3))
    for (a, c) in (((x0 + t / 2, z0 + t / 2), (x1 - t / 2, z0 + t / 2)), ((x0 + t / 2, z1 - t / 2), (x1 - t / 2, z1 - t / 2)),
                   ((x0 + t / 2, z0 + t / 2), (x0 + t / 2, z1 - t / 2)), ((x1 - t / 2, z0 + t / 2), (x1 - t / 2, z1 - t / 2))):
        s.prism("steel", (a[0], y0 + 0.4, a[1]), (c[0], leg_top - 0.1, c[1]), 0.03, 4, paint)
        s.prism("steel", (c[0], y0 + 0.4, c[1]), (a[0], leg_top - 0.1, a[1]), 0.03, 4, paint)
    # The bin: the outlet square, the sloped bottom, the walls.
    o = 0.35
    out = [(cx - o, leg_top, cz - o), (cx + o, leg_top, cz - o), (cx + o, leg_top, cz + o), (cx - o, leg_top, cz + o)]
    rim = [(x0, slope_top, z0), (x1, slope_top, z0), (x1, slope_top, z1), (x0, slope_top, z1)]
    c = (cx, slope_top + 2.0, cz)
    for i in range(4):
        j = (i + 1) % 4
        q = [out[i], out[j], rim[j], rim[i]]
        # The sloped plates face out and down, away from a point above.
        face_out(s, "steel", q, paint, c)
    top = [(x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]
    cc = (cx, (slope_top + y1) / 2, cz)
    for i in range(4):
        j = (i + 1) % 4
        face_out(s, "steel", [rim[i], rim[j], top[j], top[i]], paint, cc)
        # The inside, seen from above.
        face_out(s, "steel", [rim[i], rim[j], top[j], top[i]], (0.3, 0.3, 0.28),
                 tuple(cc[k] + (top[i][k] + top[j][k]) / 2 - cc[k] + ((top[i][k] + top[j][k]) / 2 - cc[k]) for k in range(3)))
    # Stiffener ribs and a rim angle.
    for f in (0.25, 0.5, 0.75):
        for (p0, p1) in (((x0 + (x1 - x0) * f, z0 - 0.03), None), ((x0 + (x1 - x0) * f, z1 + 0.03), None),
                         ((x0 - 0.03, z0 + (z1 - z0) * f), None), ((x1 + 0.03, z0 + (z1 - z0) * f), None)):
            s.prism("steel", (p0[0], slope_top, p0[1]), (p0[0], y1, p0[1]), 0.025, 4, paint)
    s.box("steel", (x0 - 0.04, y1 - 0.08, z0 - 0.04, x1 + 0.04, y1, z1 + 0.04), (0.35, 0.35, 0.33))
    s.box("steel", (x0 + 0.04, y1 - 0.08, z0 + 0.04, x1 - 0.04, y1 - 0.02, z1 - 0.04), (0.25, 0.25, 0.24))
    # The outlet gate and a heap of crushed stone under it.
    s.box("steel", (cx - o - 0.05, leg_top - 0.35, cz - o - 0.05, cx + o + 0.05, leg_top, cz + o + 0.05), (0.3, 0.3, 0.3))
    for _ in range(40):
        r = 0.9 * rng.random() ** 0.8
        a = rng.uniform(0, 2 * math.pi)
        px, pz = cx + math.cos(a) * r, cz + math.sin(a) * r
        sz = rng.uniform(0.06, 0.16) * (1.0 - 0.6 * r)
        yaw = rng.uniform(0, math.pi)
        ybox(s, "ashlar", (px, y0 + sz * 0.35 + (0.9 - r) * 0.25, pz), (math.cos(yaw), math.sin(yaw)),
             sz * 1.3, sz * 0.7, sz, QSTONE)
    # Rust from the rim.
    for _ in range(8):
        side = rng.randrange(4)
        w = rng.uniform(0.08, 0.2)
        h = rng.uniform(0.4, 1.2)
        col = (0.48, 0.3, 0.2)
        if side < 2:
            z = (z0 - 0.004) if side == 0 else (z1 + 0.004)
            x = rng.uniform(x0 + 0.2, x1 - 0.4)
            face_out(s, "steel", [(x, y1 - 0.08 - h, z), (x + w, y1 - 0.08 - h * 0.8, z), (x + w, y1 - 0.08, z),
                                  (x, y1 - 0.08, z)], col, (x, y1 - 0.5, cz))
        else:
            x = (x0 - 0.004) if side == 2 else (x1 + 0.004)
            z = rng.uniform(z0 + 0.2, z1 - 0.4)
            face_out(s, "steel", [(x, y1 - 0.08 - h, z), (x, y1 - 0.08 - h * 0.8, z + w), (x, y1 - 0.08, z + w),
                                  (x, y1 - 0.08, z)], col, (cx, y1 - 0.5, z))


def conveyor(s, tail, head, trestles, rng):
    """A troughed belt from `tail` to `head`: two channel stringers, idlers
    every metre, the belt, a drum and drive at the head, and each trestle's
    post with its cross-head under the frame."""
    w = 0.8
    d = [head[i] - tail[i] for i in range(3)]
    ln = math.sqrt(sum(x * x for x in d))
    u = [x / ln for x in d]
    side = (-u[2], 0.0, u[0])
    sn = math.hypot(side[0], side[2])
    side = (side[0] / sn, 0.0, side[2] / sn)
    at = lambda t, sw, dy: tuple(tail[i] + u[i] * t + side[i] * sw + (dy if i == 1 else 0.0) for i in range(3))
    for sw in (-w / 2, w / 2):
        obox(s, "steel", at(0, sw, -0.12), at(ln, sw, -0.12), 0.06, 0.18, (0.38, 0.4, 0.38))
    obox(s, "steel", at(0, 0, 0.02), at(ln, 0, 0.02), w * 0.8, 0.02, (0.1, 0.1, 0.1))
    t = 0.5
    while t < ln - 0.3:
        s.prism("steel", at(t, -w / 2, -0.03), at(t, w / 2, -0.03), 0.045, 6, (0.3, 0.3, 0.3))
        t += 1.0
    cylinder(s, "steel", at(ln, -w / 2, -0.05), at(ln, w / 2, -0.05), 0.2, 12, (0.3, 0.3, 0.3))
    cylinder(s, "steel", at(0, -w / 2, -0.05), at(0, w / 2, -0.05), 0.16, 12, (0.3, 0.3, 0.3))
    m = at(ln - 0.2, w / 2 + 0.35, -0.05)
    s.box("steel", (m[0] - 0.25, m[1] - 0.2, m[2] - 0.2, m[0] + 0.25, m[1] + 0.2, m[2] + 0.2), (0.25, 0.35, 0.5), bevel=0.02)
    # A chute off the head into whatever is below it.
    hd = at(ln + 0.15, 0, -0.1)
    obox(s, "steel", hd, (hd[0] + u[0] * 0.4, hd[1] - 0.7, hd[2] + u[2] * 0.4), 0.7, 0.05, (0.35, 0.35, 0.33))
    for (px, pz, py0, py1) in trestles:
        tt = ((px - tail[0]) * u[0] + (pz - tail[2]) * u[2]) / math.hypot(u[0], u[2]) ** 2 * math.hypot(u[0], u[2])
        ty = tail[1] + u[1] * (tt / math.hypot(u[0], u[2])) - 0.22
        s.box("steel", (px - 0.12, py0, pz - 0.12, px + 0.12, ty, pz + 0.12), (0.38, 0.4, 0.38))
        s.box("steel", (px - 0.2, py0, pz - 0.2, px + 0.2, py0 + 0.04, pz + 0.2), (0.3, 0.3, 0.3))
        a = (px - side[0] * (w / 2 + 0.1), ty, pz - side[2] * (w / 2 + 0.1))
        e = (px + side[0] * (w / 2 + 0.1), ty, pz + side[2] * (w / 2 + 0.1))
        obox(s, "steel", a, e, 0.12, 0.12, (0.38, 0.4, 0.38))


def tip_cart(s, b, rng):
    """A tipping cart: four wheels on the rails, a frame, and a V-tub
    heaped with broken stone."""
    x0, y0, z0, x1, y1, z1 = b
    along_z = (z1 - z0) >= (x1 - x0)
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    r = 0.17
    for (a, c) in (((cx - 0.3, z0 + 0.35), (cx + 0.3, z0 + 0.35)), ((cx - 0.3, z1 - 0.35), (cx + 0.3, z1 - 0.35))):
        for wx in (a[0] - 0.02, c[0] + 0.02):
            cylinder(s, "steel", (wx - 0.04, y0 + 0.05 + r, a[1]), (wx + 0.04, y0 + 0.05 + r, a[1]), r, 10, (0.2, 0.2, 0.2))
        s.prism("steel", (a[0], y0 + 0.05 + r, a[1]), (c[0], y0 + 0.05 + r, c[1]), 0.03, 6, (0.25, 0.25, 0.25))
    fy = y0 + 0.05 + 2 * r
    s.box("steel", (cx - 0.35, fy - 0.08, z0 + 0.05, cx + 0.35, fy, z1 - 0.05), (0.25, 0.22, 0.2))
    hw = (x1 - x0) / 2 - 0.02
    prof = [(-0.15, fy + 0.02), (0.15, fy + 0.02), (hw, y1 - 0.25), (hw, y1 - 0.15), (-hw, y1 - 0.15), (-hw, y1 - 0.25)]
    pt = lambda zz, p: (cx + p[0], p[1], zz)
    c = (cx, (fy + y1) / 2, cz)
    s.new_piece()
    for i in range(len(prof)):
        p, q = prof[i], prof[(i + 1) % len(prof)]
        face_out(s, "steel", [pt(z0 + 0.03, p), pt(z0 + 0.03, q), pt(z1 - 0.03, q), pt(z1 - 0.03, p)], CART, c)
    face_out(s, "steel", [pt(z0 + 0.03, p) for p in prof], CART, c)
    face_out(s, "steel", [pt(z1 - 0.03, p) for p in prof], CART, c)
    for _ in range(14):
        px, pz = rng.uniform(cx - hw * 0.7, cx + hw * 0.7), rng.uniform(z0 + 0.2, z1 - 0.2)
        sz = rng.uniform(0.12, 0.26)
        yaw = rng.uniform(0, math.pi)
        ybox(s, "ashlar", (px, y1 - 0.15 + sz * 0.3, pz), (math.cos(yaw), math.sin(yaw)), sz * 1.3, sz * 0.7, sz, QSTONE)


def rails(s, x, z0, z1, top, gauge=0.6):
    """Narrow-gauge track along z at `x`: sleepers and two rails."""
    z = z0
    while z < z1:
        s.box("timber", (x - gauge / 2 - 0.25, top, z - 0.07, x + gauge / 2 + 0.25, top + 0.07, z + 0.07),
              (0.55, 0.48, 0.4))
        z += 0.65
    for sx in (-gauge / 2, gauge / 2):
        s.box("steel", (x + sx - 0.025, top + 0.07, z0, x + sx + 0.025, top + 0.13, z1), (0.3, 0.27, 0.25))


def quarry_shed(s, b, roof_b, toward, rng):
    """The cutters' shed: board and batten on a stone sill, a door and a
    window toward the floor, a tin roof falling to the back, a stove pipe,
    and tools hung flat on the wall."""
    x0, y0, z0, x1, y1, z1 = b
    masonry(s, (x0, y0, z0, x1, 0.4, z1), [y0, -1.0, -0.3, 0.4], random.Random(rng.random()), faces="xz")
    s.box("timber", (x0 + 0.05, 0.4, z0 + 0.05, x1 - 0.05, y1, z1 - 0.05), (0.42, 0.36, 0.3))
    tint = lambda: (0.62 * rng.uniform(0.8, 1.05), 0.55 * rng.uniform(0.8, 1.05), 0.46 * rng.uniform(0.8, 1.05))
    # Boards on every face, battens over their joints.
    for axis, sign in ((0, -1), (0, 1), (2, -1), (2, 1)):
        plane = (x1 if sign > 0 else x0) if axis == 0 else (z1 if sign > 0 else z0)
        lo, hi = (z0, z1) if axis == 0 else (x0, x1)
        u = lo
        while u < hi - 0.01:
            u2 = min(hi, u + 0.25)
            if axis == 0:
                s.box("timber", (min(plane, plane + sign * 0.02), 0.4, u + 0.004, max(plane, plane + sign * 0.02),
                                 y1, u2 - 0.004), tint())
                s.box("timber", (min(plane, plane + sign * 0.04), 0.4, u2 - 0.03, max(plane, plane + sign * 0.04),
                                 y1, u2 + 0.03), tint())
            else:
                s.box("timber", (u + 0.004, 0.4, min(plane, plane + sign * 0.02), u2 - 0.004, y1,
                                 max(plane, plane + sign * 0.02)), tint())
                s.box("timber", (u2 - 0.03, 0.4, min(plane, plane + sign * 0.04), u2 + 0.03, y1,
                                 max(plane, plane + sign * 0.04)), tint())
            u = u2
    # Door and window on the face toward the floor.
    zf = z1 if toward[1] > (z0 + z1) / 2 else z0
    sg = 1 if zf == z1 else -1
    dx = x0 + (x1 - x0) * 0.7
    s.box("timber", (dx - 0.5, 0.4, min(zf, zf + sg * 0.05), dx + 0.5, 2.3, max(zf, zf + sg * 0.05)), (0.36, 0.3, 0.24))
    for yb in (0.7, 1.9):
        s.box("timber", (dx - 0.48, yb, min(zf, zf + sg * 0.06), dx + 0.48, yb + 0.12, max(zf, zf + sg * 0.06)),
              (0.3, 0.25, 0.2))
    s.box("steel", (dx + 0.32, 1.2, min(zf, zf + sg * 0.065), dx + 0.4, 1.28, max(zf, zf + sg * 0.065)), (0.2, 0.2, 0.2))
    wx = x0 + (x1 - x0) * 0.3
    s.box("timber", (wx - 0.6, 1.2, min(zf, zf + sg * 0.05), wx + 0.6, 2.1, max(zf, zf + sg * 0.05)), (0.4, 0.34, 0.27))
    face_out(s, "steel", [(wx - 0.5, 1.28, zf + sg * 0.052), (wx + 0.5, 1.28, zf + sg * 0.052),
                          (wx + 0.5, 2.02, zf + sg * 0.052), (wx - 0.5, 2.02, zf + sg * 0.052)],
             (0.06, 0.07, 0.08), (wx, 1.6, zf - sg))
    s.box("timber", (wx - 0.004, 1.28, min(zf, zf + sg * 0.058), wx + 0.004, 2.02, max(zf, zf + sg * 0.058)), (0.4, 0.34, 0.27))
    # A pick and a shovel hung flat on the wall beside the door.
    for (hx, head) in ((dx - 0.9, "pick"), (dx - 1.25, "shovel")):
        zz = zf + sg * 0.065
        s.box("timber", (hx - 0.02, 0.7, min(zz, zz + sg * 0.03), hx + 0.02, 1.75, max(zz, zz + sg * 0.03)), TIMBER)
        if head == "pick":
            s.box("steel", (hx - 0.3, 1.68, min(zz, zz + sg * 0.03), hx + 0.3, 1.74, max(zz, zz + sg * 0.03)),
                  (0.3, 0.3, 0.3))
        else:
            s.box("steel", (hx - 0.12, 0.45, min(zz, zz + sg * 0.02), hx + 0.12, 0.75, max(zz, zz + sg * 0.02)),
                  (0.35, 0.35, 0.35))
    # The tin roof: corrugated, falling away from the floor, past its box above head height.
    rx0, ry0, rz0, rx1, ry1, rz1 = roof_b
    hi_z, lo_z = (rz1, rz0) if sg > 0 else (rz0, rz1)
    prof = rib_profile(rx1 - rx0, pitch=0.2, depth=0.035)
    y_hi, y_lo = ry0 + 0.9, ry0 + 0.15
    s.box("timber", (rx0 + 0.2, ry0, min(hi_z, lo_z) + 0.3, rx1 - 0.2, ry0 + 0.15, max(hi_z, lo_z) - 0.3), (0.45, 0.4, 0.34))
    c = ((rx0 + rx1) / 2, ry0 - 2, (rz0 + rz1) / 2)
    for i in range(len(prof) - 1):
        (a0, o0), (a1, o1) = prof[i], prof[i + 1]
        q = [(rx0 + a0, y_hi + o0, hi_z), (rx0 + a1, y_hi + o1, hi_z), (rx0 + a1, y_lo + o1, lo_z), (rx0 + a0, y_lo + o0, lo_z)]
        face_out(s, "sheet", q, (0.6, 0.55, 0.5), c)
        face_out(s, "sheet", q, (0.35, 0.32, 0.3), (c[0], ry1 + 5, c[2]))
    # The rafters' wall plates make up the gap under the high edge.
    s.box("timber", (x0, y1, min(zf, zf - sg * 0.2), x1, y_hi, max(zf, zf - sg * 0.2)), tint())
    for xx in (x0 + 0.1, x1 - 0.3):
        q = [(xx, y1, zf), (xx, y_hi, zf), (xx, y1 + 0.15, z0 if sg > 0 else z1)]
        face_out(s, "timber", q, tint(), (xx + 1 if xx < x1 - 1 else xx - 1, y1 + 0.3, (z0 + z1) / 2))
    # A stove pipe through the roof.
    px, pz = x0 + 1.0, (z0 + z1) / 2
    cylinder(s, "steel", (px, ry0, pz), (px, y_hi + 1.0, pz), 0.08, 10, (0.2, 0.2, 0.2))
    cylinder(s, "steel", (px, y_hi + 1.02, pz), (px, y_hi + 1.06, pz), 0.16, 10, (0.2, 0.2, 0.2))


def quarry_floor(s, b, rng, keep_out, rails_at, centre):
    """What lies on the floor: a loader's ruts from the shed to the faces,
    darker fines in the low spots, rails, weeds along the edges and round
    anything standing, spoil lying about."""
    x0, y0, z0, x1, y1, z1 = b
    # No slab: the floor is the levelled ground itself (`landmark::stamp`),
    # worn and gritted by the client; what lies on it is drawn here.
    top = y1

    def free(x, z, m=0.15):
        if not (x0 + 0.3 < x < x1 - 0.3 and z0 + 0.3 < z < z1 - 0.3):
            return False
        return not any(k[0] - m < x < k[2] + m and k[1] - m < z < k[3] + m for k in keep_out)

    # Ruts: two wheel tracks curving in from the south edge toward the faces.
    for off in (-0.9, 0.9):
        pts = []
        for i in range(25):
            t = i / 24
            x = 3.0 * math.sin(t * 2.2) + off - 2.0 + 1.5 * t
            z = z0 + 0.4 + (z1 - z0 - 2.8) * t
            pts.append((x, z))
        for i in range(len(pts) - 1):
            (ax, az), (bx, bz) = pts[i], pts[i + 1]
            if not (free(ax, az, 0.0) and free(bx, bz, 0.0)):
                continue
            ln = math.hypot(bx - ax, bz - az)
            nx, nz = -(bz - az) / ln * 0.22, (bx - ax) / ln * 0.22
            face_out(s, "yard", [(ax - nx, top + 0.003, az - nz), (bx - nx, top + 0.003, bz - nz),
                                 (bx + nx, top + 0.003, bz + nz), (ax + nx, top + 0.003, az + nz)],
                     (GRAVEL[0] * 0.72, GRAVEL[1] * 0.7, GRAVEL[2] * 0.68), (ax, top - 1, az))
    # Darker fines pooled in hollows.
    for _ in range(int((x1 - x0) * (z1 - z0) / 60)):
        cx, cz = rng.uniform(x0 + 2, x1 - 2), rng.uniform(z0 + 2, z1 - 2)
        if not free(cx, cz, 1.0):
            continue
        r = rng.uniform(0.6, 1.6)
        pts = [(cx + r * math.cos(a) * rng.uniform(0.6, 1.0), top + 0.002, cz + r * 0.7 * math.sin(a) * rng.uniform(0.6, 1.0))
               for a in [2 * math.pi * i / 9 for i in range(9)]]
        face_out(s, "yard", pts, (GRAVEL[0] * 0.62, GRAVEL[1] * 0.6, GRAVEL[2] * 0.58), (cx, top - 1, cz))
    if rails_at:
        rx, rz0, rz1 = rails_at
        rails(s, rx, rz0, rz1, top)
    # Weeds: thick along the edges, round anything standing.
    for _ in range(int(2 * (x1 - x0 + z1 - z0) * 1.6)):
        side = rng.randrange(4)
        inset = 0.35 + 1.2 * rng.random() ** 2
        if side < 2:
            x, z = rng.uniform(x0 + 0.4, x1 - 0.4), (z0 + inset if side == 0 else z1 - inset)
        else:
            x, z = (x0 + inset if side == 2 else x1 - inset), rng.uniform(z0 + 0.4, z1 - 0.4)
        if free(x, z, 0.05):
            tuft(s, x, top, z, rng.uniform(0.15, 0.5), rng)
    for k in keep_out:
        for _ in range(rng.randint(2, 5)):
            side = rng.randrange(4)
            if side < 2:
                x, z = rng.uniform(k[0], k[2]), (k[1] - 0.1 if side == 0 else k[3] + 0.1)
            else:
                x, z = (k[0] - 0.1 if side == 2 else k[2] + 0.1), rng.uniform(k[1], k[3])
            if free(x, z, 0.0):
                tuft(s, x, top, z, rng.uniform(0.12, 0.35), rng)
    # Spoil and offcuts.
    for _ in range(int((x1 - x0) * (z1 - z0) / 7)):
        x, z = rng.uniform(x0 + 0.5, x1 - 0.5), rng.uniform(z0 + 0.5, z1 - 0.5)
        if not free(x, z):
            continue
        sz = rng.uniform(0.05, 0.22) if rng.random() < 0.85 else rng.uniform(0.25, 0.45)
        yaw = rng.uniform(0, math.pi)
        g = rng.uniform(0.75, 1.0)
        ybox(s, "ashlar", (x, top + sz * 0.3, z), (math.cos(yaw), math.sin(yaw)), sz * rng.uniform(1.0, 1.8),
             sz * 0.65, sz, (QSTONE[0] * g, QSTONE[1] * g, QSTONE[2] * g))


LOADER = (0.84, 0.62, 0.14)


def cut_steps(s, steps, rng):
    """Steps cut up a face: each tread a crisp stone with a worn front edge
    and grit in its back corner, and a rope rail on iron pins up one side,
    the pins inside the treads."""
    steps = sorted(steps, key=lambda p: p["b"][4])
    for i, p in enumerate(steps):
        x0, y0, z0, x1, y1, z1 = p["b"]
        nxt = steps[i + 1]["b"][2] if i + 1 < len(steps) else z1
        # The visible tread runs from this step's front to the next riser.
        k = rng.uniform(0.92, 1.04)
        rough(s, "ashlar", (x0, max(y0, y1 - 0.9), z0, x1, y1, nxt + 0.02), (QSTONE[0] * k, QSTONE[1] * k, QSTONE[2] * k),
              rng, amp=0.02, cell=0.5, out=0.01, crown=0.02)
        if i == 0:
            rough(s, "ashlar", (x0, y0, z0, x1, y1 - 0.9 if y1 - 0.9 > y0 else y0 + 0.01, z1), QWORN, rng, amp=0.02, cell=0.8)
        # Wear: a darker hollow in the middle of the tread.
        cx = (x0 + x1) / 2
        face_out(s, "ashlar", [(cx - 0.35, y1 + 0.004, z0 + 0.08), (cx + 0.35, y1 + 0.004, z0 + 0.08),
                               (cx + 0.3, y1 + 0.004, nxt - 0.08), (cx - 0.3, y1 + 0.004, nxt - 0.08)],
                 (QSTONE[0] * 0.8, QSTONE[1] * 0.78, QSTONE[2] * 0.76), (cx, y1 - 1, (z0 + nxt) / 2))
        if rng.random() < 0.6:
            tuft(s, x1 - 0.12, y1, nxt - 0.1, rng.uniform(0.1, 0.25), rng)
    # Pins and a sagging rope along the open side.
    side = steps[0]["b"][3] - 0.1 if abs(steps[0]["b"][3]) < abs(steps[0]["b"][0]) else steps[0]["b"][0] + 0.1
    pins = []
    for p in steps[::2] + [steps[-1]]:
        x0, y0, z0, x1, y1, z1 = p["b"]
        pz = z0 + 0.15
        cylinder(s, "steel", (side, y1, pz), (side, y1 + 0.95, pz), 0.02, 6, (0.25, 0.22, 0.2))
        pins.append((side, y1 + 0.9, pz))
    for a, b in zip(pins, pins[1:]):
        mid = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2 - 0.08, (a[2] + b[2]) / 2)
        s.prism("timber", a, mid, 0.015, 4, (0.6, 0.5, 0.36))
        s.prism("timber", mid, b, 0.015, 4, (0.6, 0.5, 0.36))


def wheel_loader(s, b, centre, rng):
    """An articulated wheel loader parked with its bucket on the ground: the
    rear body with the engine and counterweight, the cab, the front frame
    and lift arms, four big tyres — faded yellow, mud to the axles."""
    x0, y0, z0, x1, y1, z1 = b
    along_z = (z1 - z0) >= (x1 - x0)
    # Work in a frame along the long axis: t from the back to the bucket.
    cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
    fwd = 1 if (centre[1] - cz if along_z else centre[0] - cx) > 0 else -1
    L = (z1 - z0) if along_z else (x1 - x0)
    W = (x1 - x0) if along_z else (z1 - z0)
    base = max(y0, 0.0)

    def P(t, w, y):
        """t along (0 = back, L = bucket lip), w across from the middle."""
        a = (cz - fwd * L / 2 + fwd * t) if along_z else (cx - fwd * L / 2 + fwd * t)
        return (cx + w, y, a) if along_z else (a, y, cz + w)

    def B(t0, t1, w0, w1, ya, yb, role, tint, bevel=0.0):
        p, q = P(t0, w0, ya), P(t1, w1, yb)
        s.box(role, (min(p[0], q[0]), min(p[1], q[1]), min(p[2], q[2]), max(p[0], q[0]), max(p[1], q[1]), max(p[2], q[2])),
              tint, bevel=bevel)

    k = rng.uniform(0.85, 1.0)
    paint = (LOADER[0] * k, LOADER[1] * k, LOADER[2] * k)
    mud = (0.38, 0.32, 0.25)
    r = 0.78
    tw = 0.5
    axles = (1.35, L - 2.15)
    for t in axles:
        for sw in (-1, 1):
            w = sw * (W / 2 - tw / 2 - 0.02)
            a, e = P(t, w - tw / 2, base + r), P(t, w + tw / 2, base + r)
            cylinder(s, "steel", a, e, r, 16, (0.12, 0.12, 0.12))
            # The hub, and the tread's blocks as a darker band.
            a2, e2 = P(t, w - tw / 2 - 0.01, base + r), P(t, w + tw / 2 + 0.01, base + r)
            cylinder(s, "steel", a2, e2, r * 0.45, 10, paint)
        B(t - 0.15, t + 0.15, -W / 2 + tw + 0.05, W / 2 - tw - 0.05, base + r - 0.15, base + r + 0.15, "steel", (0.2, 0.2, 0.2))
    # Rear body: engine hood stepping down to the counterweight at the back.
    hw = W / 2 - tw - 0.06
    B(0.0, 0.5, -hw - 0.1, hw + 0.1, base + 0.55, base + 1.6, "paint", paint, bevel=0.06)
    B(0.3, 2.6, -hw, hw, base + 0.75, base + 1.95, "paint", paint, bevel=0.05)
    for i in range(5):
        tt = 0.6 + i * 0.3
        B(tt, tt + 0.12, -hw - 0.005, hw + 0.005, base + 1.45, base + 1.75, "steel", (0.18, 0.18, 0.17))
    # Fenders over the tyres.
    for t in axles:
        for sw in (-1, 1):
            w = sw * (W / 2 - tw / 2 - 0.02)
            B(t - r - 0.05, t + r + 0.05, w - tw / 2 - 0.03, w + tw / 2 + 0.03, base + 2 * r + 0.02, base + 2 * r + 0.07, "steel",
              paint)
    # The cab: posts, roof, dark glass.
    c0, c1 = 2.2, 3.6
    cy0, cy1 = base + 1.95, min(y1, base + 3.05)
    for tt in (c0, c1 - 0.08):
        for sw in (-1, 1):
            B(tt, tt + 0.08, sw * (hw - 0.04) - 0.04, sw * (hw - 0.04) + 0.04, cy0, cy1, "paint", paint)
    B(c0 - 0.1, c1 + 0.1, -hw - 0.05, hw + 0.05, cy1 - 0.08, cy1, "paint", paint, bevel=0.02)
    glass = (0.07, 0.08, 0.09)
    B(c0 + 0.08, c1 - 0.08, -hw + 0.02, hw - 0.02, cy0 + 0.1, cy1 - 0.1, "steel", glass)
    # Exhaust and a beacon.
    e0 = P(0.9, hw - 0.25, base + 1.95)
    cylinder(s, "steel", e0, (e0[0], e0[1] + 0.7, e0[2]), 0.06, 8, (0.15, 0.15, 0.15))
    bc = P((c0 + c1) / 2, 0.0, cy1)
    s.box("bulb", (bc[0] - 0.08, cy1, bc[2] - 0.08, bc[0] + 0.08, cy1 + 0.14, bc[2] + 0.08))
    # Articulation and the front frame.
    B(3.6, 4.6, -0.35, 0.35, base + 0.6, base + 1.2, "paint", paint, bevel=0.03)
    # Lift arms from the front frame down to the bucket.
    lip = L - 0.05
    for sw in (-1, 1):
        a = P(3.9, sw * 0.55, base + 1.7)
        e = P(lip - 0.9, sw * 0.55, base + 0.75)
        obox(s, "steel", a, e, 0.16, 0.3, paint)
    # The bucket, mouth on the ground: a back, a floor, two sides.
    bw = W / 2 - 0.02
    t0 = lip - 1.2
    B(t0, t0 + 0.1, -bw, bw, base + 0.05, base + 1.15, "paint", paint)
    B(t0, lip, -bw, bw, base, base + 0.08, "steel", (0.3, 0.27, 0.22))
    for sw in (-1, 1):
        B(t0, lip, sw * bw - 0.04, sw * bw + 0.04, base + 0.05, base + 0.9, "paint", paint)
    for i in range(7):
        w = -bw + 0.2 + i * (2 * bw - 0.4) / 6
        B(lip - 0.02, lip + 0.04, w - 0.06, w + 0.06, base, base + 0.12, "steel", (0.3, 0.3, 0.3))
    # Spoil in the bucket, and mud up the body.
    for _ in range(18):
        p = P(rng.uniform(t0 + 0.2, lip - 0.15), rng.uniform(-bw + 0.15, bw - 0.15), base + 0.1)
        sz = rng.uniform(0.1, 0.25)
        yaw = rng.uniform(0, math.pi)
        ybox(s, "ashlar", (p[0], p[1] + sz * 0.3, p[2]), (math.cos(yaw), math.sin(yaw)), sz * 1.4, sz * 0.7, sz, QSTONE)
    for t in axles:
        for sw in (-1, 1):
            w = sw * (W / 2 - tw / 2 - 0.02)
            B(t - r * 0.9, t + r * 0.9, w - tw / 2 - 0.015, w + tw / 2 + 0.015, base, base + 0.35, "steel", mud)


def dress_quarry(s, kit, rng):
    parts = kit["parts"]

    def dims(p):
        b = p["b"]
        return b[3] - b[0], b[4] - b[1], b[5] - b[2]

    floor_p = next(p for p in parts if p["mat"] == "yard")
    fb = floor_p["b"]
    floor = fb[4]
    walls = [p for p in parts if p["mat"] == "rock"]
    bench = min((w for w in walls if w["b"][4] <= 3.5), key=lambda w: w["b"][2], default=None)
    stones = [p for p in parts if p["mat"] == "stone"]
    steps = [p for p in stones if bench is not None and abs(p["b"][5] - bench["b"][2]) < 0.01]
    blocks = [p for p in stones if p not in steps]
    steel = [p for p in parts if p["mat"] == "steel"]
    mast = next(p for p in steel if dims(p)[1] > 10)
    jib = next(p for p in steel if p["b"][1] > 8 and p is not mast)
    winch = next(p for p in steel if p["b"][1] < 0 and p is not mast)
    hop = next(p for p in steel if p not in (mast, jib) and p["b"][4] >= 4.5 and dims(p)[0] > 2)
    trestle = next(p for p in steel if dims(p)[0] < 0.6 and 2.5 < dims(p)[1] < 5)
    pole = next(p for p in steel if dims(p)[0] < 0.6 and dims(p)[1] > 5 and p is not mast)
    cart = next(p for p in steel if p["b"][4] < 1.5 and max(dims(p)[0], dims(p)[2]) > 1.5 and p is not winch)
    loader = next((p for p in steel if max(dims(p)[0], dims(p)[2]) > 5 and 2 < dims(p)[1] < 4), None)
    drums = [p for p in steel if p not in (mast, jib, winch, hop, trestle, pole, cart, loader)]
    shed = next(p for p in parts if p["mat"] == "timber" and dims(p)[1] > 2)
    roof = next(p for p in parts if p["mat"] == "timber" and p is not shed)
    # The middle of the worked floor, which the faces look at.
    centre = ((fb[0] + fb[3]) / 2, (max(fb[2], min(w["b"][2] for w in walls)) + fb[5]) / 2)

    stair_box = None
    if steps:
        stair_box = (min(p["b"][0] for p in steps), min(p["b"][2] for p in steps),
                     max(p["b"][3] for p in steps), max(p["b"][5] for p in steps))
    for w in walls:
        cut_wall(s, w, parts, fb, centre, rng, avoid=[stair_box] if stair_box else [])
    for i, p in enumerate(blocks):
        cut_block(s, p["b"], rng, split=(i == len(blocks) - 1))
    if steps:
        cut_steps(s, steps, rng)
    if loader is not None:
        wheel_loader(s, loader["b"], centre, rng)
    derrick(s, mast["b"], jib["b"], winch["b"], rng)
    hopper(s, hop["b"], rng)
    hb = hop["b"]
    tx = (trestle["b"][0] + trestle["b"][3]) / 2
    tz = (trestle["b"][2] + trestle["b"][5]) / 2
    if bench is not None:
        tail = (tx, bench["b"][4] + 0.45, bench["b"][2] + 2.2)
        head = (tx, hb[4] + 0.5, (hb[2] + hb[5]) / 2 + 0.6)
        conveyor(s, tail, head, [(tx, tz, floor, None)], rng)
        # The tail's frame on the bench top.
        s.box("steel", (tx - 0.5, bench["b"][4], tail[2] - 0.2, tx + 0.5, tail[1] - 0.2, tail[2] + 0.2), (0.38, 0.4, 0.38))
    tip_cart(s, cart["b"], rng)
    cx = (cart["b"][0] + cart["b"][3]) / 2
    floodlight(s, pole["b"], centre)
    for d in drums:
        drum_pallet(s, d["b"], rng)
    quarry_shed(s, shed["b"], roof["b"], centre, rng)
    keep = [(p["b"][0], p["b"][2], p["b"][3], p["b"][5]) for p in parts
            if p is not floor_p and p["b"][1] < 2.0 and p is not roof]
    quarry_floor(s, fb, rng, keep, (cx, fb[2] + 7.5, (bench["b"][2] if bench else fb[5]) - 0.1), centre)


MARKS = {"mark_ruin": dress_ruin, "mark_stones": dress_stones, "mark_mast": dress_mast,
         "mark_tower": dress_tower, "mark_yard": dress_yard, "mark_relay": dress_relay,
         "mark_quarry": dress_quarry}


# Rising damp: how a wall darkens and greens toward the ground it stands in,
# so it sits IN the turf rather than on it (`ART.md` §2 rule 2). The band's
# top wanders with the ground's wet, and a landmark's ground is y = 0 at its
# centre, so on a slope it reads a little high on one side — as damp does.
DAMP_TINT = (0.56, 0.6, 0.44)
DAMP_M = 1.4


def damp(co):
    """The rising-damp weight at a Blender-space point (z up)."""
    from mathutils import Vector, noise
    top = DAMP_M + 0.6 * noise.noise(Vector((co.x * 0.45, co.y * 0.45, 3.7)))
    t = min(1.0, max(0.0, (top - co.z) / top))
    return t ** 1.6


def bake_ao(bpy, objs, samples, rising=False):
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
    # Combine: Col = tint × (0.35 + 0.65 × ao), and the damp at the foot.
    for obj in objs:
        mesh = obj.data
        n = len(mesh.loops)
        tint = [0.0] * (n * 4)
        aov = [0.0] * (n * 4)
        mesh.attributes["tint"].data.foreach_get("color", tint)
        mesh.attributes["ao"].data.foreach_get("color", aov)
        if rising:
            wet = [damp(v.co) for v in mesh.vertices]
            for i, loop in enumerate(mesh.loops):
                w = wet[loop.vertex_index]
                for c in range(3):
                    tint[i * 4 + c] *= 1.0 + (DAMP_TINT[c] - 1.0) * w
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
        pid = bm.faces.layers.int.get("piece")
        vertical_ribs = obj.name in ("sheet", "cargo", "steel")
        patched = obj.name in PATCHED and pid is not None
        if obj.name == "ivy":
            # Each card spans its photograph once, corner to corner.
            for f in bm.faces:
                for loop, uvc in zip(f.loops, ((0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0))):
                    loop[uv].uv = uvc
            bm.to_mesh(obj.data)
            bm.free()
            continue
        for f in bm.faces:
            n = f.normal
            ax, ay, az = abs(n.x), abs(n.y), abs(n.z)
            du, dv = patch_offset(f[pid]) if patched else (0.0, 0.0)
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
                loop[uv].uv = (u + du, v + dv)
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
        bake_ao(bpy, objs, args.ao_samples, rising=kit["name"] in MARKS)
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


# The client's surfaces, for `look`: mesh role → (map set, tiles per metre,
# metallic, roughness). Mirrors `render/depot.rs` `Surface`.
LOOK_SURFACES = {
    "ashlar": ("ashlar", 0.55, 0.0, 0.8), "stone": ("stone", 1 / 3, 0.0, 1.0),
    "obsidian": ("stone", 1 / 3, 0.0, 0.42), "concrete": ("concrete", 1 / 3, 0.0, 1.0),
    "timber": ("wood", 1.0, 0.0, 0.88), "yard": ("gravel", 1.0, 0.0, 1.0),
    "steel": ("metal", 0.55, 0.0, 1.0), "sheet": ("metal", 0.55, 0.0, 1.0),
    "cargo": ("metal", 0.55, 0.0, 1.0), "gilt": ("metal", 0.55, 0.9, 0.38),
    "canvas": ("concrete", 1 / 3, 0.0, 1.0), "lapis": (None, 1.0, 0.0, 0.6),
    "bulb": (None, 1.0, 0.0, 0.6), "leaf": ("grass", 2.0, 0.0, 0.9),
    "paint": ("concrete", 1 / 3, 0.0, 0.7), "ivy": (None, 1.0, 0.0, 0.85),
}


def look(args):
    """Render a dressed GLB on turf under a sun, wearing the game's own
    photographed surfaces the way the client binds them — a preview for
    the eye, not a gate."""
    import bpy
    from mathutils import Vector
    tex = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "textures")
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    bpy.ops.import_scene.gltf(filepath=args.glb)
    obs = [o for o in sc.objects if o.type == "MESH"]
    for o in obs:
        role = o.data.name.split(".")[0]
        if role not in LOOK_SURFACES:
            role = o.name.split(".")[0]
        maps, tiles, metal, rough_ = LOOK_SURFACES.get(role, (None, 1.0, 0.0, 0.8))
        m = bpy.data.materials.new(role)
        m.use_nodes = True
        nt = m.node_tree
        bsdf = nt.nodes["Principled BSDF"]
        bsdf.inputs["Metallic"].default_value = metal
        bsdf.inputs["Roughness"].default_value = rough_
        vc = nt.nodes.new("ShaderNodeVertexColor")
        cols = o.data.color_attributes
        vc.layer_name = cols[0].name if len(cols) else ""
        if maps:
            uv = nt.nodes.new("ShaderNodeTexCoord")
            sc_ = nt.nodes.new("ShaderNodeMapping")
            sc_.inputs["Scale"].default_value = (tiles, tiles, 1)
            nt.links.new(uv.outputs["UV"], sc_.inputs["Vector"])
            alb = nt.nodes.new("ShaderNodeTexImage")
            alb.image = bpy.data.images.load(os.path.join(tex, f"{maps}_albedo.jpg"))
            nt.links.new(sc_.outputs["Vector"], alb.inputs["Vector"])
            mix = nt.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.blend_type = "MULTIPLY"
            mix.inputs["Factor"].default_value = 1.0
            src = alb.outputs["Color"]
            if role == "obsidian":
                src = None
            if src is not None:
                nt.links.new(src, mix.inputs[6])
            else:
                mix.inputs[6].default_value = (0.13, 0.13, 0.15, 1)
            nt.links.new(vc.outputs["Color"], mix.inputs[7])
            nt.links.new(mix.outputs[2], bsdf.inputs["Base Color"])
            nrm = nt.nodes.new("ShaderNodeTexImage")
            nrm.image = bpy.data.images.load(os.path.join(tex, f"{maps}_normal.jpg"))
            nrm.image.colorspace_settings.name = "Non-Color"
            nt.links.new(sc_.outputs["Vector"], nrm.inputs["Vector"])
            nm = nt.nodes.new("ShaderNodeNormalMap")
            nt.links.new(nrm.outputs["Color"], nm.inputs["Color"])
            nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
            if role == "gilt":
                bsdf.inputs["Base Color"].default_value = (1.0, 0.6, 0.15, 1)
        else:
            nt.links.new(vc.outputs["Color"], bsdf.inputs["Base Color"])
            bsdf.inputs["Emission Color"].default_value = (0.6, 1.4, 4.0, 1) if role == "lapis" else (6, 4.5, 2.2, 1)
            bsdf.inputs["Emission Strength"].default_value = 1.0
        o.data.materials.clear()
        o.data.materials.append(m)
    if args.kit:
        # The kit's rock parts, which the client draws and the model leaves
        # out: grey stand-ins, so a worked face is seen against its rock.
        rm = bpy.data.materials.new("rock")
        rm.use_nodes = True
        rm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.36, 0.35, 0.33, 1)
        rm.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 0.95
        for part in parse(open(args.kit).read())["parts"]:
            if part["mat"] != "rock":
                continue
            x0, y0, z0, x1, y1, z1 = part["b"]
            bpy.ops.mesh.primitive_cube_add(size=1.0)
            cube = bpy.context.active_object
            cube.location = kit_to_blender(((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2))
            cube.scale = (x1 - x0 - 0.3, z1 - z0 - 0.3, y1 - y0 - 0.15)
            cube.data.materials.append(rm)
    v = [o.matrix_world @ x.co for o in obs for x in o.data.vertices]
    lo = Vector((min(p.x for p in v), min(p.y for p in v), max(min(p.z for p in v), -0.5)))
    hi = Vector((max(p.x for p in v), max(p.y for p in v), max(p.z for p in v)))
    c = (lo + hi) / 2
    rad = (hi - lo).length / 2
    # Turf: the grass photograph on a plane at the kit's floor.
    bpy.ops.mesh.primitive_plane_add(size=400, location=(c.x, c.y, 0.0))
    g = sc.objects[-1] if sc.objects[-1].type == "MESH" else bpy.context.active_object
    gm = bpy.data.materials.new("turf")
    gm.use_nodes = True
    gt = gm.node_tree
    gi = gt.nodes.new("ShaderNodeTexImage")
    gi.image = bpy.data.images.load(os.path.join(tex, "grass_albedo.jpg"))
    gtc = gt.nodes.new("ShaderNodeTexCoord")
    gmp = gt.nodes.new("ShaderNodeMapping")
    gmp.inputs["Scale"].default_value = (0.4, 0.4, 0.4)
    gt.links.new(gtc.outputs["Object"], gmp.inputs["Vector"])
    gt.links.new(gmp.outputs["Vector"], gi.inputs["Vector"])
    gt.links.new(gi.outputs["Color"], gt.nodes["Principled BSDF"].inputs["Base Color"])
    gt.nodes["Principled BSDF"].inputs["Roughness"].default_value = 1.0
    bpy.context.active_object.data.materials.append(gm)
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    sc.collection.objects.link(cam)
    cam.data.lens = args.lens
    yaw = math.radians(args.yaw)
    elev = math.radians(args.elev)
    d = Vector((math.cos(yaw) * math.cos(elev), math.sin(yaw) * math.cos(elev), math.sin(elev)))
    dist = rad / math.tan(cam.data.angle / 2) * args.dist
    aim = Vector((c.x, c.y, lo.z + (hi.z - lo.z) * args.aim))
    cam.location = aim + d * dist
    if args.eye and args.target:
        # Kit coordinates (x, y up, z), as a player would stand.
        cam.location = Vector(kit_to_blender(args.eye))
        aim = Vector(kit_to_blender(args.target))
        d = (cam.location - aim).normalized()
    cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
    sc.camera = cam
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 4.0
    sun.data.angle = math.radians(2.0)
    sun.rotation_euler = (math.radians(55), 0, yaw + math.radians(70))
    sc.collection.objects.link(sun)
    w = bpy.data.worlds.new("w")
    w.use_nodes = True
    w.node_tree.nodes["Background"].inputs["Color"].default_value = (0.5, 0.62, 0.8, 1)
    w.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.9
    sc.world = w
    sc.render.engine = "CYCLES"
    sc.cycles.device = "CPU"
    sc.cycles.samples = args.samples
    try:
        sc.cycles.use_denoising = True
        sc.cycles.denoiser = "OPENIMAGEDENOISE"
    except Exception:
        pass
    sc.render.resolution_x, sc.render.resolution_y = args.size
    sc.view_settings.view_transform = "AgX"
    sc.render.filepath = os.path.abspath(args.out)
    bpy.ops.render.render(write_still=True)
    print(f"look -> {args.out}")
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
    lk = sub.add_parser("look", help="render a dressed GLB on turf in the game's surfaces")
    lk.add_argument("glb")
    lk.add_argument("--out", required=True)
    lk.add_argument("--yaw", type=float, default=-55.0, help="camera bearing, degrees")
    lk.add_argument("--elev", type=float, default=14.0, help="camera elevation, degrees")
    lk.add_argument("--dist", type=float, default=1.1, help="distance, × the framing radius")
    lk.add_argument("--lens", type=float, default=35.0)
    lk.add_argument("--aim", type=float, default=0.4, help="aim height, × the model's height")
    lk.add_argument("--eye", type=float, nargs=3, help="camera at this kit point (x y z)")
    lk.add_argument("--target", type=float, nargs=3, help="looking at this kit point")
    lk.add_argument("--kit", help="the kit JSON, to stand grey boxes in for its rock parts")
    lk.add_argument("--samples", type=int, default=24)
    lk.add_argument("--size", type=int, nargs=2, default=(960, 600))
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.cmd == "gen":
        return gen(args)
    if args.cmd == "look":
        return look(args)
    ap.error("a command (gen, look) or --self-test")


if __name__ == "__main__":
    sys.exit(main())
