#!/usr/bin/env python3
"""Read the index `ci/blender.py sheet` wrote and say which models are over
budget: triangles against the per-row ceilings the tests and WANTED.md
name, textures over the 1024 the packer emits, more than one material
(the loaders read material 0), and files over 4 MB.

Also reads each GLB's own JSON for what the index cannot see: the loaders
draw `mesh 0, primitive 0`, so a file with more meshes, primitives or a
skin carries geometry the game never shows (held/bone_helmet ships a 2 m
icosphere and an armature).

    ci/bl_audit.py DIR/_views/index.json [--models assets/models] [--md]
"""
import json
import os
import struct
import sys

# Triangle ceilings. The `prop_models` rows are what `tests/prop_assets.rs`
# enforces, so those are hard. Everything else is flagged against the
# larger of a floor per category and `OUTLIER` times the category's median:
# a generated held item lands near 3,000 tris whatever WANTED.md asked for,
# and the useful signal is the gun at twice that, not the whole drawer.
CEILING = {
    "held": 1000, "skins": 1000, "deploy": 1500, "crops": 3000,
    "prop": 3000, "rock": 3000, "fel": 12000, "mob": 12000, "site": 12000,
}
OUTLIER = 1.75
PROP_ROW = {"barrel": 800, "crate": 600, "cache": 600,
            "node_stone": 1500, "node_metal": 1500, "node_sulfur": 1500, "stump": 2000}
SINGLE = {"held", "skins", "deploy", "prop", "rock", "crops"}
TEX_MAX = 1024
BYTES_MAX = 4_000_000


def ceiling(cat, name, median=0):
    if cat == "prop" and name in PROP_ROW:
        return PROP_ROW[name]
    return max(CEILING.get(cat, 3000), int(OUTLIER * median))


def medians(index):
    by = {}
    for key, st in index.items():
        by.setdefault(key.split("/", 1)[0], []).append(st["tris"])
    return {cat: sorted(v)[len(v) // 2] for cat, v in by.items()}


def glb_structure(path):
    """(meshes, primitives, skins, nodes) straight off the GLB's JSON."""
    with open(path, "rb") as f:
        raw = f.read()
    n = struct.unpack("<I", raw[12:16])[0]
    j = json.loads(raw[20:20 + n])
    meshes = j.get("meshes", [])
    prims = sum(len(m.get("primitives", [])) for m in meshes)
    return len(meshes), prims, len(j.get("skins", [])), len(j.get("nodes", []))


def audit(index, models_dir=None):
    rows = []
    med = medians(index)
    for key, st in sorted(index.items()):
        cat, name = key.split("/", 1)
        cap = ceiling(cat, name, med.get(cat, 0))
        flags = []
        if models_dir:
            path = find_glb(models_dir, cat, name)
            if path:
                nm, npr, nsk, nn = glb_structure(path)
                # Held items, deployables, props and crops load one primitive
                # (`tests/*_assets.rs`); sites and mobs load more on purpose.
                if cat in SINGLE and (nm > 1 or npr > 1):
                    flags.append(f"{nm} meshes/{npr} primitives, game draws 1")
                if cat in SINGLE and nsk:
                    flags.append(f"carries a rig ({nn} nodes)")
        if st["tris"] > cap:
            flags.append(f"tris {st['tris']:,} > {cap:,}")
        if st["texture"] > TEX_MAX:
            flags.append(f"texture {st['texture']}px")
        # Sites load every material; the single-primitive loaders read one.
        if st["materials"] > 1 and cat in SINGLE:
            flags.append(f"{st['materials']} materials")
        if st["bytes"] > BYTES_MAX:
            flags.append(f"{st['bytes']/1e6:.1f} MB")
        rows.append((key, st, cap, flags))
    return rows


def find_glb(models_dir, cat, name):
    for dp, _, fs in os.walk(models_dir):
        if os.path.basename(dp) == cat and f"{name}.glb" in fs:
            return os.path.join(dp, f"{name}.glb")
    return None


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    with open(sys.argv[1]) as f:
        index = json.load(f)
    models_dir = sys.argv[sys.argv.index("--models") + 1] if "--models" in sys.argv else None
    rows = audit(index, models_dir)
    bad = [r for r in rows if r[3]]
    total_tris = sum(st["tris"] for _, st, _, _ in rows)
    total_mb = sum(st["bytes"] for _, st, _, _ in rows) / 1e6
    if "--md" in sys.argv:
        print(f"{len(rows)} models, {total_tris:,} tris, {total_mb:.0f} MB; {len(bad)} over budget\n")
        print("| model | tris | ceiling | size (m) | tex | flags |")
        print("|---|---:|---:|---|---:|---|")
        for key, st, cap, flags in sorted(bad, key=lambda r: -r[1]["tris"] / r[2]):
            w, h, d = st["size"]
            print(f"| {key} | {st['tris']:,} | {cap:,} | {w:.2f}×{h:.2f}×{d:.2f} | {st['texture']} | {'; '.join(flags)} |")
    else:
        for key, st, cap, flags in sorted(bad, key=lambda r: -r[1]["tris"] / r[2]):
            print(f"{key:40s} {st['tris']:7,} / {cap:6,}  {'; '.join(flags)}")
        print(f"{len(rows)} models, {total_tris:,} tris, {total_mb:.0f} MB; {len(bad)} over budget")


if __name__ == "__main__":
    main()
