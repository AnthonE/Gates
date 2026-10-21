#!/usr/bin/env python3
"""Turn a geometry-only Meshy rock (`meshy_gen.py --no-texture`) into a rock
piece `render/boulders.rs` draws a formation's block with: welded, every
hole capped, on its base at the origin and centred in X/Z, positions and
normals only. Then the far piece, the same rock remeshed low by Meshy.

    ci/rock_piece.py RAW.glb OUT.glb [--far OUT_far.glb --far-tris 300]

**Why the caps.** A remeshed delivery is not closed: the first six came back
with 3 to 18 open loops each, one of them a hole across half the top of a
block. A hole in a rock is sky through it, the defect the procedural rock
was fixed for (operator, 2026-10-20: "see through corners"). A loop is capped
with a fan about its centroid, after the fins (flaps of triangles hanging
off a crack's edge) and any loose debris are peeled off.

The key is never read here: `meshy_gen.py`'s rule (unset, the session's
network secret adds it).
"""
import argparse
import base64
import json
import os
import struct
import sys
import time
import urllib.request
from collections import defaultdict

import numpy as np

API = "https://api.meshy.ai"


def read_glb(path):
    """The first primitive's positions and triangles."""
    raw = open(path, "rb").read()
    off, js, blob = 12, None, b""
    while off + 8 <= len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        body = raw[off + 8:off + 8 + ln]
        if kind == 0x4E4F534A:
            js = json.loads(body)
        elif kind == 0x004E4942:
            blob = body
        off += 8 + ln

    def acc(i):
        a = js["accessors"][i]
        bv = js["bufferViews"][a["bufferView"]]
        n = {"SCALAR": 1, "VEC3": 3}[a["type"]]
        dt = {5126: np.float32, 5125: np.uint32, 5123: np.uint16}[a["componentType"]]
        st = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = bv.get("byteStride", n * np.dtype(dt).itemsize)
        rows = np.ndarray((a["count"], n), dt, blob, st, (stride, np.dtype(dt).itemsize))
        return np.array(rows)

    prim = js["meshes"][0]["primitives"][0]
    pos = acc(prim["attributes"]["POSITION"]).astype(np.float64)
    if "indices" in prim:
        tri = acc(prim["indices"]).reshape(-1, 3).astype(np.int64)
    else:
        tri = np.arange(len(pos)).reshape(-1, 3)
    return pos, tri


def weld(pos, tri):
    """One vertex per position (to a millionth of the rock), no degenerate
    triangle."""
    q = np.round(pos / np.ptp(pos, 0).max() * 1e6).astype(np.int64)
    uq, first, inv = np.unique(q, axis=0, return_index=True, return_inverse=True)
    t = inv.reshape(-1)[tri]
    t = t[(t[:, 0] != t[:, 1]) & (t[:, 1] != t[:, 2]) & (t[:, 2] != t[:, 0])]
    return pos[first], t


def edge_counts(tri):
    n = defaultdict(int)
    for a, b, c in tri:
        for u, v in ((a, b), (b, c), (c, a)):
            n[(min(u, v), max(u, v))] += 1
    return n


def tri_edges(t):
    a, b, c = t
    return [(min(a, b), max(a, b)), (min(b, c), max(b, c)), (min(c, a), max(c, a))]


def peel(tri):
    """Drop the fins (flaps hanging off a crack, an open edge beside one three
    triangles share) and anything not joined to the rock's main body."""
    while True:
        n = edge_counts(tri)
        uses = np.bincount(tri.reshape(-1))
        # Beside a shared crease, or a tip no other triangle touches.
        fin = [any(n[e] == 1 for e in tri_edges(t))
               and (any(n[e] >= 3 for e in tri_edges(t)) or uses[t].min() == 1)
               for t in tri]
        if not any(fin):
            break
        tri = tri[~np.array(fin)]
    by_edge = defaultdict(list)
    for i, t in enumerate(tri):
        for e in tri_edges(t):
            by_edge[e].append(i)
    comp, k = -np.ones(len(tri), int), 0
    for s in range(len(tri)):
        if comp[s] >= 0:
            continue
        stack, comp[s] = [s], k
        while stack:
            i = stack.pop()
            for e in tri_edges(tri[i]):
                for j in by_edge[e]:
                    if comp[j] < 0:
                        comp[j] = k
                        stack.append(j)
        k += 1
    return tri[comp == np.bincount(comp).argmax()]


def cap(pos, tri):
    """Fan every open loop shut about its centroid, wound against the
    triangle on its first edge."""
    n = edge_counts(tri)
    adj = defaultdict(list)
    for (u, v), c in n.items():
        if c == 1:
            adj[u].append(v)
            adj[v].append(u)
    winds = {}
    for a, b, c in tri:
        for u, v in ((a, b), (b, c), (c, a)):
            winds[(u, v)] = True
    used, pos, new, loops = set(), list(pos), [], 0
    for u0 in list(adj):
        for v0 in adj[u0]:
            if (min(u0, v0), max(u0, v0)) in used:
                continue
            used.add((min(u0, v0), max(u0, v0)))
            path, cur = [u0], v0
            while cur != u0:
                path.append(cur)
                nxt = [w for w in adj[cur] if (min(cur, w), max(cur, w)) not in used]
                if not nxt:
                    path = None
                    break
                used.add((min(cur, nxt[0]), max(cur, nxt[0])))
                cur = nxt[0]
            if not path or len(path) < 3:
                continue
            if (path[0], path[1]) in winds:
                path.reverse()
            c = len(pos)
            pos.append(np.mean([pos[i] for i in path], 0))
            new += [(path[k], path[(k + 1) % len(path)], c) for k in range(len(path))]
            loops += 1
    return np.array(pos), (np.vstack([tri, new]) if new else tri), loops


def normals(pos, tri):
    n = np.zeros_like(pos)
    f = np.cross(pos[tri[:, 1]] - pos[tri[:, 0]], pos[tri[:, 2]] - pos[tri[:, 0]])
    for k in range(3):
        np.add.at(n, tri[:, k], f)
    return n / np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)


def write_glb(path, pos, tri):
    pos = pos.astype(np.float32)
    nrm = normals(pos.astype(np.float64), tri).astype(np.float32)
    idx = tri.astype(np.uint32).reshape(-1)
    blob = pos.tobytes() + nrm.tobytes() + idx.tobytes()
    n, m = len(pos), len(idx)
    js = {
        "asset": {"version": "2.0", "generator": "ci/rock_piece.py"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2}]}],
        "buffers": [{"byteLength": len(blob)}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 12 * n, "target": 34962},
            {"buffer": 0, "byteOffset": 12 * n, "byteLength": 12 * n, "target": 34962},
            {"buffer": 0, "byteOffset": 24 * n, "byteLength": 4 * m, "target": 34963},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": n, "type": "VEC3",
             "min": pos.min(0).tolist(), "max": pos.max(0).tolist()},
            {"bufferView": 1, "componentType": 5126, "count": n, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5125, "count": m, "type": "SCALAR"},
        ],
    }
    j = json.dumps(js, separators=(",", ":")).encode()
    j += b" " * (-len(j) % 4)
    blob += b"\0" * (-len(blob) % 4)
    body = struct.pack("<II", len(j), 0x4E4F534A) + j + struct.pack("<II", len(blob), 0x004E4942) + blob
    open(path, "wb").write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def piece(src, out):
    pos, tri = weld(*read_glb(src))
    pos, tri, loops = cap(pos, peel(tri))
    left = sum(1 for c in edge_counts(tri).values() if c == 1)
    lo, hi = pos.min(0), pos.max(0)
    pos = pos - np.array([(lo[0] + hi[0]) / 2, lo[1], (lo[2] + hi[2]) / 2])
    v = pos[tri]
    vol = np.einsum("ij,ij->i", v[:, 0], np.cross(v[:, 1], v[:, 2])).sum() / 6
    if vol < 0:
        tri = tri[:, ::-1]
    write_glb(out, pos, tri)
    size = hi - lo
    print(f"{out}: {len(tri)} tris, {loops} holes capped, {left} open edges left, "
          f"w:d:h {size[0]:.2f}:{size[2]:.2f}:{size[1]:.2f}{'' if vol >= 0 else ', rewound'}")


def call(method, path, body=None):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode() if body else None,
                                 method=method)
    if os.environ.get("MESHY_API_KEY"):
        req.add_header("Authorization", "Bearer " + os.environ["MESHY_API_KEY"].strip())
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())


def remesh(src, out, tris):
    uri = "data:application/octet-stream;base64," + base64.b64encode(open(src, "rb").read()).decode()
    tid = call("POST", "/openapi/v1/remesh", {"model_url": uri, "target_formats": ["glb"],
                                              "topology": "triangle", "target_polycount": tris})["result"]
    t0 = time.time()
    while True:
        t = call("GET", f"/openapi/v1/remesh/{tid}")
        if t["status"] == "SUCCEEDED":
            break
        if t["status"] in ("FAILED", "CANCELED") or time.time() - t0 > 900:
            sys.exit(f"remesh {tid}: {t['status']} {t.get('task_error')}")
        time.sleep(5)
    raw = out + ".raw.glb"
    with urllib.request.urlopen(t["model_urls"]["glb"], timeout=300) as r:
        open(raw, "wb").write(r.read())
    print(f"remesh {tid}: {t.get('consumed_credits')} credits")
    return raw


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("raw")
    ap.add_argument("out")
    ap.add_argument("--far", help="also write the far piece here")
    ap.add_argument("--far-tris", type=int, default=300)
    a = ap.parse_args()
    piece(a.raw, a.out)
    if a.far:
        raw = remesh(a.out, a.far, a.far_tris)
        piece(raw, a.far)
        os.remove(raw)


if __name__ == "__main__":
    main()
