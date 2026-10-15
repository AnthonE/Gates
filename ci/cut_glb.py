#!/usr/bin/env python3
"""Cut the triangles inside a box out of an imported GLB.

    ci/cut_glb.py <in.glb> <out.glb> --box X0 Y0 Z0 X1 Y1 Z1 [--spare X Z R]

A generator adds what the prompt said to leave off, and some of it has to go
because the code draws it instead: the gunship came back with stubs of rotor
blade on its hub, a cross that would stand still under the spinning rotor
`render/heli.rs` draws. This drops every triangle whose three corners lie
inside the box (metres, in the imported frame, so run it after
`ci/import_meshy.py` and before `ci/ktx_pack.py`). `--spare X Z R` keeps any
triangle whose centroid is within R of the vertical axis at (X, Z): the hub
inside the stubs.

The kept triangles move to the front of the index buffer and the accessor's
count shrinks; vertices are left where they are (an unreferenced vertex
costs bytes, not a draw).
"""
import argparse
import sys

import numpy as np

from import_meshy import positions, read_glb, write_glb

DTYPES = {5121: "<u1", 5123: "<u2", 5125: "<u4"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("--box", nargs=6, type=float, required=True)
    ap.add_argument("--spare", nargs=3, type=float)
    a = ap.parse_args()
    lo, hi = np.array(a.box[:3]), np.array(a.box[3:])

    gltf, blob = read_glb(a.src)
    total = cut = 0
    for m in gltf["meshes"]:
        for p in m["primitives"]:
            if "indices" not in p:
                sys.exit("a primitive without indices is not handled")
            v = positions(gltf, blob, p["attributes"]["POSITION"])
            acc = gltf["accessors"][p["indices"]]
            bv = gltf["bufferViews"][acc["bufferView"]]
            start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
            dt = np.dtype(DTYPES[acc["componentType"]])
            idx = np.frombuffer(memoryview(blob)[start:start + acc["count"] * dt.itemsize],
                                dtype=dt).reshape(-1, 3)
            corners = v[idx]
            inside = np.all((corners >= lo) & (corners <= hi), axis=(1, 2))
            if a.spare:
                c = corners.mean(1)
                x, z, r = a.spare
                inside &= np.hypot(c[:, 0] - x, c[:, 2] - z) > r
            keep = idx[~inside].copy()
            idx[:len(keep)] = keep
            acc["count"] = int(keep.size)
            total += len(idx)
            cut += int(inside.sum())
    write_glb(a.dst, gltf, blob)
    print(f"  {a.dst.rsplit('/', 1)[-1]}: cut {cut} of {total} triangles")


if __name__ == "__main__":
    main()
