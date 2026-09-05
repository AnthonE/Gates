#!/usr/bin/env python3
"""UV charts of a generated GLB, and the arithmetic over them that two tools
share: `ci/measure_glb.py` (reject a roll whose charts disagree) and
`ci/flatten_charts.py` (repair one that shipped).

**Why charts are the unit.** A generated mesh arrives UV-unwrapped into
many islands -- 116 to 149 on the three shipped boulders, 384 on the stone
node, measured 2026-09-05 -- and the generator bakes each island's texels
from whichever camera view it projected that island from. Two islands that
meet on the mesh were therefore painted under two different lights, and
the seam between them is a step in albedo that follows the polygon edge
exactly. On a smooth-normalled boulder that step is the only hard edge in
the frame, so it reads as a fracture or a mineral facet: the "camouflage"
patches the operator pointed at on 2026-09-05. Nothing in a bounding-box,
luma or hue measurement can see it, because every chart is individually
inside every band -- the defect is in the DIFFERENCES between them.

A chart is derived from the index buffer, not from the texture: a vertex is
split at every UV seam, so two triangles are in one chart exactly when they
share a vertex INDEX. Union-find over the triangle list gives the charts;
rasterising each triangle in UV space gives which texel belongs to which.
"""
import json
import struct

import numpy as np

# Per-texel chart id for a texel no triangle covers -- the generator's own
# dilation gutter, which `flatten_charts.py` has to treat as "whatever chart
# is nearest" rather than as a chart of its own.
UNCOVERED = -1


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = json.loads(next(c for k, c in chunks if k == 0x4E4F534A))
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return js, blob


_CT = {5120: "i1", 5121: "u1", 5122: "i2", 5123: "u2", 5125: "u4", 5126: "f4"}
_N = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def accessor(g, blob, i):
    """One accessor as an (count, n) numpy array. Tightly packed only --
    every generated file so far is, and an interleaved one should fail here
    loudly rather than be read at the wrong stride."""
    a = g["accessors"][i]
    bv = g["bufferViews"][a["bufferView"]]
    dt, n = np.dtype("<" + _CT[a["componentType"]]), _N[a["type"]]
    if bv.get("byteStride", dt.itemsize * n) != dt.itemsize * n:
        raise SystemExit("interleaved accessor is not handled")
    st = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    return np.frombuffer(blob, dtype=dt, count=a["count"] * n, offset=st).reshape(a["count"], n)


def primitive(g, blob, mesh=0, prim=0):
    """`(positions, uvs, triangles)` of one primitive, triangles as an
    (m, 3) index array."""
    p = g["meshes"][mesh]["primitives"][prim]
    pos = accessor(g, blob, p["attributes"]["POSITION"]).astype(np.float64)
    uv = accessor(g, blob, p["attributes"]["TEXCOORD_0"]).astype(np.float64)
    idx = accessor(g, blob, p["indices"]).astype(np.int64).reshape(-1, 3)
    return pos, uv, idx


def charts(idx, nverts):
    """Chart id per triangle, and how many charts there are.

    Union-find over shared vertex indices. Iterative, because a 2 k-vertex
    mesh is nothing but a 40 k one would recurse past Python's limit.
    """
    parent = np.arange(nverts)

    def find(x):
        root = x
        while parent[root] != root:
            root = parent[root]
        while parent[x] != root:
            parent[x], x = root, parent[x]
        return root

    for a, b, c in idx:
        for u, v in ((a, b), (b, c)):
            ru, rv = find(u), find(v)
            if ru != rv:
                parent[ru] = rv
    roots = np.array([find(v) for v in range(nverts)])
    _, labels = np.unique(roots, return_inverse=True)
    tri_chart = labels[idx[:, 0]]
    return tri_chart, int(labels.max()) + 1


def rasterize(uv, idx, tri_chart, w, h):
    """(h, w) int array: which chart each texel belongs to, `UNCOVERED` where
    none does. glTF's UV origin is the top-left corner, so texel row = v * h
    with no flip."""
    out = np.full((h, w), UNCOVERED, dtype=np.int32)
    px = uv[:, 0] * w
    py = uv[:, 1] * h
    for t, (a, b, c) in enumerate(idx):
        xs, ys = px[[a, b, c]], py[[a, b, c]]
        x0, x1 = int(np.floor(xs.min())), int(np.ceil(xs.max()))
        y0, y1 = int(np.floor(ys.min())), int(np.ceil(ys.max()))
        x0, y0 = max(x0, 0), max(y0, 0)
        x1, y1 = min(x1, w - 1), min(y1, h - 1)
        if x1 < x0 or y1 < y0:
            continue
        gx, gy = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
        # Edge functions; a texel centre is inside when all three agree in
        # sign with the triangle's own winding (either winding is accepted).
        e0 = (xs[1] - xs[0]) * (gy - ys[0]) - (ys[1] - ys[0]) * (gx - xs[0])
        e1 = (xs[2] - xs[1]) * (gy - ys[1]) - (ys[2] - ys[1]) * (gx - xs[1])
        e2 = (xs[0] - xs[2]) * (gy - ys[2]) - (ys[0] - ys[2]) * (gx - xs[2])
        inside = ((e0 >= 0) & (e1 >= 0) & (e2 >= 0)) | ((e0 <= 0) & (e1 <= 0) & (e2 <= 0))
        sub = out[y0:y1 + 1, x0:x1 + 1]
        sub[inside] = tri_chart[t]
    return out


def srgb_to_linear(a):
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(a):
    return np.where(a <= 0.0031308, a * 12.92, 1.055 * np.power(np.maximum(a, 0.0), 1 / 2.4) - 0.055)


def luma(lin):
    return 0.2126 * lin[..., 0] + 0.7152 * lin[..., 1] + 0.0722 * lin[..., 2]


def chart_means(chart_map, value, n_charts):
    """Per-chart mean of `value` (an (h, w) array) over covered texels, and
    the texel count per chart. Charts with no texel read as NaN."""
    flat_c, flat_v = chart_map.ravel(), value.ravel()
    covered = flat_c != UNCOVERED
    counts = np.bincount(flat_c[covered], minlength=n_charts).astype(np.float64)
    sums = np.bincount(flat_c[covered], weights=flat_v[covered], minlength=n_charts)
    with np.errstate(invalid="ignore", divide="ignore"):
        means = sums / counts
    return means, counts


def chart_contrast(chart_map, lin_luma, n_charts):
    """The one number: texel-weighted mean of |chart mean / global mean - 1|
    over the covered texels. Zero when every chart agrees; a bake whose
    islands were lit from different views reads as a few hundredths and up.

    Returns `(contrast, means, counts)` so a caller can also report the
    worst chart.
    """
    means, counts = chart_means(chart_map, lin_luma, n_charts)
    covered = counts > 0
    g = float(np.sum(means[covered] * counts[covered]) / np.sum(counts[covered]))
    dev = np.abs(means[covered] / g - 1.0)
    contrast = float(np.sum(dev * counts[covered]) / np.sum(counts[covered]))
    return contrast, means, counts


# ── Shape, for the triage and for the gate ──────────────────────────────────
#
# Two numbers that tell a lump from a slab from a cube, which depth/width
# cannot: the first six boulder rolls were all "d/w ~1.0" and one of them was
# a ball, and the stone node that shipped is d/w 1.000 and a CUBE. Measured
# on the shipped set 2026-09-05 (vertex-only reading; the sampled one below
# moves them slightly): plan ratio 1.18 on the boulder in the operator's
# screenshot against 1.41 for the node -- a circle is 1.0, a square 1.41.
#
# Both are taken over SURFACE SAMPLES (every vertex plus points along every
# edge), not vertices alone, because a cube's eight vertices all sit at one
# radius and would read as a sphere. `EDGE_STEPS` samples per edge fills the
# angular bins of any closed mesh.
PLAN_BINS = 36
EDGE_STEPS = 8
# A mesh under this many triangles takes more steps per edge, so a
# twelve-triangle test cube still fills every bin. `edge_steps` is the rule
# and the Rust gate (`tests/prop_assets.rs`) carries the same one.
EDGE_SAMPLE_BUDGET = 2000


def edge_steps(tris):
    return max(EDGE_STEPS, int(np.ceil(EDGE_SAMPLE_BUDGET / max(tris, 1))))


def surface_samples(pos, idx):
    """Vertices plus `edge_steps(tris)` points along each triangle edge."""
    a, b, c = pos[idx[:, 0]], pos[idx[:, 1]], pos[idx[:, 2]]
    steps = edge_steps(len(idx))
    t = (np.arange(1, steps) / steps)[:, None, None]
    edges = np.concatenate([
        a + (b - a) * t, b + (c - b) * t, c + (a - c) * t,
    ], axis=1).reshape(-1, 3)
    return np.concatenate([pos, edges])


def plan_ratio(pos, idx):
    """Widest over narrowest direction of the footprint, about the X/Z
    bounding-box centre: the largest radius in each of `PLAN_BINS` angular
    bins, max over min. 1.0 is a circle, 1.414 a square, higher a slab."""
    p = surface_samples(pos, idx)
    cx = (p[:, 0].min() + p[:, 0].max()) / 2
    cz = (p[:, 2].min() + p[:, 2].max()) / 2
    x, z = p[:, 0] - cx, p[:, 2] - cz
    ang = np.arctan2(z, x)
    b = ((ang + np.pi) / (2 * np.pi) * PLAN_BINS).astype(int) % PLAN_BINS
    r = np.hypot(x, z)
    per = np.full(PLAN_BINS, -1.0)
    np.maximum.at(per, b, r)
    if (per < 0).any():
        raise SystemExit("plan_ratio: an angular bin has no surface sample -- not a closed mesh?")
    return float(per.max() / max(per.min(), 1e-9))


def radius_spread(pos, idx):
    """Standard deviation over mean of the surface samples' distance from
    the bounding-box centre. 0 is a sphere; a cube reads ~0.13, a slab more."""
    p = surface_samples(pos, idx)
    c = (p.min(0) + p.max(0)) / 2
    d = np.linalg.norm(p - c, axis=1)
    return float(d.std() / max(d.mean(), 1e-9))


# ── A synthetic fixture, so both tools can prove their own bands ────────────

def _png_bytes(rgb8):
    """PNG-encode an (h, w, 3) uint8 array. PIL if present, else a minimal
    zlib writer -- the fixture must not depend on a decoder being installed
    to be BUILT, only to be measured."""
    try:
        from PIL import Image
        import io
        buf = io.BytesIO()
        Image.fromarray(rgb8, "RGB").save(buf, format="PNG")
        return buf.getvalue()
    except ImportError:
        import zlib
        h, w, _ = rgb8.shape
        raw = b"".join(b"\0" + rgb8[y].tobytes() for y in range(h))

        def chunk(tag, data):
            c = tag + data
            return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def synthetic_glb(shape="sphere", islands=8, gains=None, size=256, radius=1.0, height=None,
                  base_luma=0.35, textured=True):
    """A closed test mesh, UV-unwrapped into `islands` charts laid out on a
    grid, with an albedo whose chart `i` is the base grey times `gains[i]`.
    Returns the GLB as bytes.

    `shape`: "sphere" (plan ratio 1, spread ~0), "cube" (square in plan) or
    "slab" (a wafer 5x wider and taller than it is deep -- the shape one
    reference image reconstructs a rock as). The charts are longitude
    sectors on the sphere and faces on the boxes.
    """
    if shape == "sphere":
        lat, lon = 12, 48
        sectors = islands
        per = lon // sectors
        pos, uv, idx = [], [], []
        ncols = int(np.ceil(np.sqrt(sectors)))
        tile = 1.0 / ncols
        for s in range(sectors):
            base = len(pos)
            u0, v0 = (s % ncols) * tile, (s // ncols) * tile
            for i in range(lat + 1):
                th = np.pi * i / lat
                for j in range(per + 1):
                    ph = 2 * np.pi * (s * per + j) / lon
                    pos.append([radius * np.sin(th) * np.cos(ph), radius * np.cos(th), radius * np.sin(th) * np.sin(ph)])
                    uv.append([u0 + tile * 0.05 + tile * 0.9 * j / per, v0 + tile * 0.05 + tile * 0.9 * i / lat])
            for i in range(lat):
                for j in range(per):
                    a = base + i * (per + 1) + j
                    b, c, d = a + 1, a + per + 1, a + per + 2
                    idx += [[a, c, b], [b, c, d]]
    else:
        hx = hz = radius
        hy = radius if height is None else height
        if shape == "slab":
            hz = radius / 5.0
        faces = [
            ([-1, -1, 1], [1, -1, 1], [1, 1, 1], [-1, 1, 1]),
            ([1, -1, -1], [-1, -1, -1], [-1, 1, -1], [1, 1, -1]),
            ([1, -1, 1], [1, -1, -1], [1, 1, -1], [1, 1, 1]),
            ([-1, -1, -1], [-1, -1, 1], [-1, 1, 1], [-1, 1, -1]),
            ([-1, 1, 1], [1, 1, 1], [1, 1, -1], [-1, 1, -1]),
            ([-1, -1, -1], [1, -1, -1], [1, -1, 1], [-1, -1, 1]),
        ]
        pos, uv, idx = [], [], []
        ncols = 3
        tile = 1.0 / ncols
        for f, quad in enumerate(faces):
            base = len(pos)
            u0, v0 = (f % ncols) * tile, (f // ncols) * tile
            for k, (sx, sy, sz) in enumerate(quad):
                pos.append([sx * hx, sy * hy, sz * hz])
                uu, vv = [(0, 0), (1, 0), (1, 1), (0, 1)][k]
                uv.append([u0 + tile * (0.05 + 0.9 * uu), v0 + tile * (0.05 + 0.9 * vv)])
            idx += [[base, base + 1, base + 2], [base, base + 2, base + 3]]
        islands = 6
    pos = np.array(pos, dtype="<f4")
    uv = np.array(uv, dtype="<f4")
    idx = np.array(idx, dtype="<u4")

    # The albedo: base grey with a little noise, then each chart's tile
    # scaled by its gain, in linear light, encoded sRGB.
    rng = np.random.default_rng(7)
    lin = np.full((size, size, 3), base_luma) * (1.0 + 0.08 * rng.standard_normal((size, size, 1)))
    gains = list(gains or [1.0] * islands)
    ncols = int(np.ceil(np.sqrt(islands))) if shape == "sphere" else 3
    tile_px = size // ncols
    for i, g in enumerate(gains):
        x0, y0 = (i % ncols) * tile_px, (i // ncols) * tile_px
        lin[y0:y0 + tile_px, x0:x0 + tile_px] *= g
    rgb8 = (np.clip(linear_to_srgb(np.clip(lin, 0, 1)), 0, 1) * 255 + 0.5).astype(np.uint8)
    png = _png_bytes(rgb8)

    blob = bytearray()
    views = []

    def view(data):
        while len(blob) % 4:
            blob.append(0)
        views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)})
        blob.extend(data)
        return len(views) - 1

    vp, vu, vi = view(pos.tobytes()), view(uv.tobytes()), view(idx.tobytes())
    g = {
        "asset": {"version": "2.0", "generator": "glbcharts.synthetic_glb"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0, "name": "Mesh_0"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1},
                                    "indices": 2, "material": 0}]}],
        "accessors": [
            {"bufferView": vp, "componentType": 5126, "count": len(pos), "type": "VEC3",
             "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
            {"bufferView": vu, "componentType": 5126, "count": len(uv), "type": "VEC2"},
            {"bufferView": vi, "componentType": 5125, "count": idx.size, "type": "SCALAR"},
        ],
        "materials": [{"name": "Material_0", "pbrMetallicRoughness": {}}],
        "bufferViews": views, "buffers": [{"byteLength": 0}],
    }
    if textured:
        vimg = view(png)
        g["images"] = [{"bufferView": vimg, "mimeType": "image/png", "name": "Image_0"}]
        g["textures"] = [{"source": 0}]
        g["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = {"index": 0}
    g["buffers"][0]["byteLength"] = len(blob)
    return glb_bytes(g, bytes(blob))


def glb_bytes(gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = blob + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    return b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body


def parse_glb(raw):
    """`read_glb` for bytes already in memory."""
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = json.loads(next(c for k, c in chunks if k == 0x4E4F534A))
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return js, blob
