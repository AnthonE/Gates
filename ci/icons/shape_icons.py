#!/usr/bin/env python3
"""Draw the build wheel's shape glyphs as isometric line art.

    python3 ci/icons/shape_icons.py     # writes ci/icons/shape_*.svg
    python3 ci/bake_icons.py shape_...  # then bakes them into assets/icons

One family for every piece, the way the reference wheel draws its own:
the piece's outline in a constant stroke, hidden lines removed, and a fill
that fades in toward the bottom. Geometry is ours, in cell units (1 = one
3 m cell, 1 = one storey), taken from `render/structures.rs` and
`sim_core::circulation`. White on transparent, so the client tints it.

Needs `shapely`.
"""
import math
import pathlib

from shapely.geometry import LineString, MultiLineString, Polygon
from shapely.ops import linemerge, unary_union

OUT = pathlib.Path(__file__).resolve().parent
PX = 128
PAD = 6.0
STROKE = 6.5
# The largest a cell may be drawn, px per cell, so a wall face does not
# swell to fill the canvas a cube needs.
CAP = 84.0
# Fill opacity at the bottom of the glyph, fading to none at FADE_TOP of
# its height.
FILL = 0.5
FADE_TOP = 0.35

ISO = (45.0, 32.0)
# Triangles from higher up, so the top reads as a triangle and not a sliver.
TOP = (45.0, 52.0)
FRONT = (28.0, 24.0)


# ---- solids ---------------------------------------------------------------


def face(loops, normal, extra=()):
    return {"loops": loops, "n": normal, "extra": list(extra)}


def box(x0, x1, y0, y1, z0, z1):
    return [
        face([[(x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]], (0, 1, 0)),
        face([[(x0, y0, z0), (x0, y0, z1), (x1, y0, z1), (x1, y0, z0)]], (0, -1, 0)),
        face([[(x1, y0, z0), (x1, y0, z1), (x1, y1, z1), (x1, y1, z0)]], (1, 0, 0)),
        face([[(x0, y0, z0), (x0, y1, z0), (x0, y1, z1), (x0, y0, z1)]], (-1, 0, 0)),
        face([[(x0, y0, z1), (x0, y1, z1), (x1, y1, z1), (x1, y0, z1)]], (0, 0, 1)),
        face([[(x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0)]], (0, 0, -1)),
    ]


def signed_area(loop):
    return 0.5 * sum(
        a[0] * b[1] - b[0] * a[1] for a, b in zip(loop, loop[1:] + loop[:1])
    )


def extrude(loops, to3, w0, w1, axis):
    """Extrude 2D `loops` (outer first, then holes) from w0 to w1.

    `to3(a, b, w)` lifts a profile point; `axis` is the unit vector w runs
    along. Side normals come from the edge direction and the loop's winding,
    so a hole's walls face into the hole.
    """
    out = [
        face([[to3(a, b, w1) for a, b in lp] for lp in loops], axis),
        face(
            [[to3(a, b, w0) for a, b in reversed(lp)] for lp in loops],
            tuple(-c for c in axis),
        ),
    ]
    for k, lp in enumerate(loops):
        # A wall faces away from the material: out of the outer loop, into a
        # hole. That is the right of the edge for a ccw outer loop or a cw
        # hole, the left otherwise.
        right = (k == 0) == (signed_area(lp) > 0)
        for (a0, b0), (a1, b1) in zip(lp, lp[1:] + lp[:1]):
            da, db = a1 - a0, b1 - b0
            na, nb = (db, -da) if right else (-db, da)
            ln = math.hypot(na, nb) or 1.0
            p0, p1 = to3(0, 0, 0), to3(na / ln, nb / ln, 0)
            n = tuple(q - p for p, q in zip(p0, p1))
            out.append(
                face(
                    [[to3(a0, b0, w0), to3(a1, b1, w0), to3(a1, b1, w1), to3(a0, b0, w1)]],
                    n,
                )
            )
    return out


def wall(loops, t=0.16):
    """A wall in the x-y plane, `t` thick toward the camera."""
    return extrude(loops, lambda a, b, w: (a, b, w), 0.0, t, (0, 0, 1))


def slab(loops, y0, y1, extra=()):
    """A plate in the x-z plane, profile points as (x, z)."""
    faces = extrude(loops, lambda a, b, w: (a, w, b), y0, y1, (0, 1, 0))
    faces[0]["extra"] = [[(a, y1, b) for a, b in line] for line in extra]
    return faces


def flight(along, x0, x1, foot, head, y_foot, y_head, steps):
    """A flight solid to the ground: `steps` treads from `foot` to `head`
    along `along` ("x" or "z"), across [x0, x1] on the other axis."""
    da = (head - foot) / steps
    dy = (y_head - y_foot) / steps
    prof = [(foot, 0.0), (head, 0.0)]
    for i in reversed(range(steps)):
        y = y_foot + dy * (i + 1)
        prof += [(foot + da * (i + 1), y), (foot + da * i, y)]
    prof = ccw(dedupe(prof))
    if along == "z":
        return extrude([prof], lambda a, b, w: (w, b, a), x0, x1, (1, 0, 0))
    return extrude([prof], lambda a, b, w: (a, b, w), x0, x1, (0, 0, 1))


def dedupe(pts):
    out = []
    for p in pts:
        if not out or abs(out[-1][0] - p[0]) > 1e-9 or abs(out[-1][1] - p[1]) > 1e-9:
            out.append(p)
    if len(out) > 1 and out[0] == out[-1]:
        out.pop()
    return out


def wedge(cx, cz, a0, a1, r, y, sub=3):
    """A pie slice of a fan, from the ground up to `y`."""
    pts = [(cx, cz)]
    for k in range(sub + 1):
        a = a0 + (a1 - a0) * k / sub
        pts.append((cx + r * math.cos(a), cz + r * math.sin(a)))
    if signed_area(pts) < 0:
        pts.reverse()
    return slab([pts], 0.0, y)


def tri_wedge(p0, p1, p2, y):
    pts = [p0, p1, p2]
    if signed_area(pts) < 0:
        pts.reverse()
    return slab([pts], 0.0, y)


def ccw(pts):
    return pts if signed_area(pts) > 0 else list(reversed(pts))


def cw(pts):
    return pts if signed_area(pts) < 0 else list(reversed(pts))


# ---- the pieces -------------------------------------------------------------

SQ = ccw([(0, 0), (1, 0), (1, 1), (0, 1)])
TRI = ccw([(0, 0), (1, 0), (0, 1)])


def inset_tri(d):
    # The right triangle's incentre-scaled copy: every rail `d` wide.
    k = 1.0 - d * (2.0 + math.sqrt(2.0))
    c = d
    return [(c, c), (c + k, c), (c, c + k)]


def courses(n, tri=False):
    """Roofing courses on a plate's top, running across x."""
    lines = []
    for i in range(1, n + 1):
        z = i / (n + 1)
        lines.append([(0.0, z), (1.0 - z if tri else 1.0, z)])
    return lines


PIECES = {
    "shape_foundation": (ISO, box(0, 1, 0, 0.42, 0, 1)),
    "shape_tri_foundation": (TOP, slab([TRI], 0, 0.42)),
    "shape_wall": (FRONT, wall([SQ])),
    "shape_half_wall": (FRONT, wall([ccw([(0, 0), (1, 0), (1, 0.5), (0, 0.5)])])),
    "shape_low_wall": (FRONT, wall([ccw([(0, 0), (1, 0), (1, 1 / 3), (0, 1 / 3)])])),
    "shape_doorway": (
        FRONT,
        wall([ccw([(0, 0), (0.3, 0), (0.3, 0.73), (0.7, 0.73), (0.7, 0), (1, 0), (1, 1), (0, 1)])]),
    ),
    "shape_window": (
        FRONT,
        wall([SQ, cw([(0.3, 0.33), (0.7, 0.33), (0.7, 0.73), (0.3, 0.73)])]),
    ),
    "shape_wall_frame": (
        FRONT,
        wall([ccw([(0, 0), (0.12, 0), (0.12, 0.86), (0.88, 0.86), (0.88, 0), (1, 0), (1, 1), (0, 1)])]),
    ),
    "shape_floor": (ISO, slab([SQ], 0, 0.1)),
    "shape_floor_frame": (
        ISO,
        slab([SQ, cw([(0.2, 0.2), (0.8, 0.2), (0.8, 0.8), (0.2, 0.8)])], 0, 0.1),
    ),
    "shape_tri_floor": (TOP, slab([TRI], 0, 0.1)),
    "shape_tri_floor_frame": (TOP, slab([TRI, cw(inset_tri(0.14))], 0, 0.1)),
    "shape_roof": (ISO, slab([SQ], 0, 0.1, extra=courses(3))),
    "shape_tri_roof": (TOP, slab([TRI], 0, 0.1, extra=courses(2, tri=True))),
    "shape_stairs": (ISO, flight("z", 0.25, 0.75, 1, 0, 0, 1, 5)),
    "shape_foundation_steps": (ISO, flight("z", 0, 1, 1, 0, 0, 0.5, 3)),
    "shape_ramp": (
        ISO,
        extrude([ccw([(1, 0), (0, 0), (0, 0.5)])], lambda a, b, w: (w, b, a), 0, 1, (1, 0, 0)),
    ),
    # `circulation::patches`, turned half a turn so every flight climbs away
    # from the eye and shows its risers.
    "shape_stairs_l": (
        ISO,
        flight("z", 2 / 3, 1, 1, 1 / 3, 0, 0.5, 3)
        + box(2 / 3, 1, 0, 0.5, 0, 1 / 3)
        + flight("x", 0, 1 / 3, 2 / 3, 0, 0.5, 1.0, 3),
    ),
    "shape_stairs_u": (
        ISO,
        flight("z", 0, 1 / 3, 1, 1 / 3, 0, 0.5, 3)
        + box(0, 1, 0, 0.5, 0, 1 / 3)
        + flight("z", 2 / 3, 1, 1 / 3, 1, 0.5, 1.0, 3),
    ),
    "shape_stairs_spiral": (
        TOP,
        [f for i in range(6) for f in wedge(
            0.5, 0.5,
            math.pi * (1.0 - i / 6), math.pi * (1.0 - (i + 1) / 6),
            0.5, 0.5 * (i + 1) / 6,
        )],
    ),
    "shape_stairs_tri_spiral": (
        TOP,
        # The fan turns about the right-angle corner, put nearest the eye so
        # every riser faces it.
        [f for i in range(4) for f in tri_wedge(
            (1, 1),
            (i / 4, 1 - i / 4),
            ((i + 1) / 4, 1 - (i + 1) / 4),
            0.5 * (i + 1) / 4,
        )],
    ),
}


# ---- projection and hidden lines -------------------------------------------


def camera(yaw, pitch):
    a, b = math.radians(yaw), math.radians(pitch)
    e = (math.sin(a) * math.cos(b), math.sin(b), math.cos(a) * math.cos(b))
    r = (e[2], 0.0, -e[0])
    lr = math.hypot(r[0], r[2])
    r = (r[0] / lr, 0.0, r[2] / lr)
    u = (
        e[1] * r[2] - e[2] * r[1],
        e[2] * r[0] - e[0] * r[2],
        e[0] * r[1] - e[1] * r[0],
    )
    return e, r, u


def dot(p, q):
    return p[0] * q[0] + p[1] * q[1] + p[2] * q[2]


def render(view, faces):
    e, r, u = camera(*view)

    def proj(p):
        return (dot(p, r), -dot(p, u))

    vis = []
    for f in faces:
        if dot(f["n"], e) <= 1e-6:
            continue
        loops = [[proj(p) for p in lp] for lp in f["loops"]]
        poly = Polygon(loops[0], loops[1:])
        if not poly.is_valid:
            poly = poly.buffer(0)
        if poly.area < 1e-9:
            continue
        p0 = f["loops"][0][0]
        n = f["n"]
        d = dot(n, p0)

        def depth(x, y, n=n, d=d):
            # The point on the face's plane under screen (x, y): larger is
            # nearer the eye.
            return (d - x * dot(n, r) + y * dot(n, u)) / dot(n, e)

        vis.append({
            "poly": poly,
            "loops": loops,
            "extra": [[proj(p) for p in line] for line in f["extra"]],
            "depth": depth,
        })

    # Order near to far by pairwise depth wherever two faces overlap on
    # screen — a centroid sort gets interlocking steps wrong.
    nearer = {i: set() for i in range(len(vis))}
    for i in range(len(vis)):
        for j in range(i + 1, len(vis)):
            inter = vis[i]["poly"].intersection(vis[j]["poly"])
            if inter.area < 1e-7:
                continue
            pt = inter.representative_point()
            di = vis[i]["depth"](pt.x, pt.y)
            dj = vis[j]["depth"](pt.x, pt.y)
            if abs(di - dj) < 1e-7:
                continue
            if di > dj:
                nearer[j].add(i)
            else:
                nearer[i].add(j)
    order, done = [], set()
    while len(order) < len(vis):
        ready = [i for i in range(len(vis)) if i not in done and nearer[i] <= done]
        if not ready:  # a cycle: take the nearest centroid and carry on
            ready = [max(
                (i for i in range(len(vis)) if i not in done),
                key=lambda i: vis[i]["depth"](*vis[i]["poly"].centroid.coords[0]),
            )]
        for i in ready:
            order.append(i)
            done.add(i)

    fills, lines = [], []
    cover = Polygon()
    for i in order:
        v = vis[i]
        fills.append(v["poly"].difference(cover))
        edges = [LineString(lp + lp[:1]) for lp in v["loops"]]
        edges += [LineString(line) for line in v["extra"]]
        lines.append(MultiLineString(edges).difference(cover))
        cover = unary_union([cover, v["poly"]])
    return cover, fills, lines


def fit(cover):
    x0, y0, x1, y1 = cover.bounds
    w, h = x1 - x0, y1 - y0
    room = PX - 2 * PAD
    s = min(room / w, room / h, CAP)
    ox = PX / 2 - s * (x0 + x1) / 2
    oy = PX / 2 - s * (y0 + y1) / 2
    return lambda x, y: (ox + s * x, oy + s * y), (oy + s * y0, oy + s * y1)


def polys_of(g):
    if g.is_empty:
        return []
    if g.geom_type == "Polygon":
        return [g]
    return [p for p in getattr(g, "geoms", []) if p.geom_type == "Polygon"]


def lines_of(g):
    if g.is_empty:
        return []
    if g.geom_type == "LineString":
        return [g]
    if g.geom_type == "MultiLineString":
        merged = linemerge(g)
        return [merged] if merged.geom_type == "LineString" else list(merged.geoms)
    return [p for p in getattr(g, "geoms", []) if p.geom_type == "LineString"]


def svg(view, faces):
    cover, fills, lines = render(view, faces)
    tr, (top, bot) = fit(cover)

    def ring(coords):
        pts = [tr(x, y) for x, y in coords]
        return "M" + "L".join(f"{x:.2f} {y:.2f}" for x, y in pts) + "Z"

    def path(coords):
        pts = [tr(x, y) for x, y in coords]
        return "M" + "L".join(f"{x:.2f} {y:.2f}" for x, y in pts)

    fill_d = "".join(
        ring(p.exterior.coords) + "".join(ring(h.coords) for h in p.interiors)
        for g in fills
        for p in polys_of(g)
        if p.area > 1e-9
    )
    line_d = "".join(path(ls.coords) for g in lines for ls in lines_of(g) if ls.length > 1e-6)
    fade = top + (bot - top) * FADE_TOP
    return (
        "<!-- Gates building glyph; drawn by ci/icons/shape_icons.py from our "
        "geometry, not third-party art. -->\n"
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {PX} {PX}">'
        f'<defs><linearGradient id="f" gradientUnits="userSpaceOnUse" x1="0" '
        f'y1="{fade:.2f}" x2="0" y2="{bot:.2f}">'
        f'<stop offset="0" stop-color="#fff" stop-opacity="0"/>'
        f'<stop offset="1" stop-color="#fff" stop-opacity="{FILL}"/>'
        f"</linearGradient></defs>"
        f'<path fill="url(#f)" fill-rule="evenodd" d="{fill_d}"/>'
        f'<path fill="none" stroke="#fff" stroke-width="{STROKE}" '
        f'stroke-linejoin="round" stroke-linecap="round" d="{line_d}"/>'
        "</svg>\n"
    )


if __name__ == "__main__":
    for name, (view, faces) in PIECES.items():
        (OUT / f"{name}.svg").write_text(svg(view, faces))
    print(f"drew {len(PIECES)} shape glyphs into {OUT}")
