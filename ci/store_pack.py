#!/usr/bin/env python3
"""Build the asset store packs: our models, re-packed for any engine.

The game's GLBs carry KTX2/UASTC textures, which Bevy reads and Blender,
Godot, Unity and three.js do not (and the files name `image/ktx2` without
declaring `KHR_texture_basisu`). A pack's copy decodes every texture back to
PNG, so the file opens anywhere. `assets/store/packs.toml` says what goes in
each pack; this writes one folder per pack:

    <out>/<id>/elo-asset.json     the upload form: name, sha256 and size per file
    <out>/<id>/<file>.glb         PNG textures, the clips renamed where asked
    <out>/<id>/preview.jpg        assets/store/previews/<id>.jpg

The Elo Pros box lists a folder with `meter/asset_house.py publish <folder>`.

The data maps (normal, metal-rough) are the one judgement call: a model
packed before `ktx_pack.py` learned `--assign-tf linear` holds
`srgb_to_linear(source)` there, one packed after holds the source. Each model's
normal map decides which, by `unbake_ktx.py`'s own physical check (the share
of texels that decode to a unit vector), and every data map in that model gets
the same answer.

Needs the KTX-Software CLI (`ktx`, `KTX_BIN`), as `ktx_pack.py` does.

Usage:  ci/store_pack.py <out dir> [pack id …]
"""
import hashlib
import json
import os
import shutil
import sys
import tempfile
import tomllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ktx_pack import read_glb, srgb_images, write_glb  # noqa: E402
from unbake_ktx import (UNIT_MIN, ktx_extract, normal_image_indices,  # noqa: E402
                        to_png, unbend, unit_fraction)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACKS = os.path.join(ROOT, "assets", "store", "packs.toml")
PREVIEWS = os.path.join(ROOT, "assets", "store", "previews")
COPYRIGHT = "Elo Pros (elopros.com), from Gates. CC-BY-4.0"


def bent(gltf, decoded, src):
    """Whether this model's data maps carry the old packer's sRGB bend, read
    off its normal map. False when it has none: today's packer does not bend."""
    normals = sorted(normal_image_indices(gltf))
    if not normals:
        return False
    raw, _, _ = unit_fraction(decoded[normals[0]])
    fixed, _, _ = unit_fraction(unbend(decoded[normals[0]]))
    if max(raw, fixed) < UNIT_MIN:
        raise SystemExit(f"{src}: its normal map decodes to unit vectors {raw:.0%} as stored "
                         f"and {fixed:.0%} un-bent; neither is a normal map. Refusing.")
    return fixed > raw


def convert(src, dst, rename):
    gltf, blob = read_glb(src)
    colour = srgb_images(gltf)
    decoded = {}
    with tempfile.TemporaryDirectory() as tmp:
        for i, img in enumerate(gltf.get("images", [])):
            bv = gltf["bufferViews"][img["bufferView"]]
            st = bv.get("byteOffset", 0)
            data = blob[st:st + bv["byteLength"]]
            if img.get("mimeType") == "image/ktx2":
                decoded[i] = ktx_extract(data, tmp, f"img{i}")
    is_bent = bent(gltf, decoded, src)
    replacement = {}
    for i, rgba in decoded.items():
        img = gltf["images"][i]
        fixed = rgba if (i in colour or not is_bent) else unbend(rgba)
        replacement[img["bufferView"]] = to_png(fixed)
        img["mimeType"] = "image/png"
        img.pop("name", None)
    for ext in ("extensionsUsed", "extensionsRequired"):
        if ext in gltf:
            gltf[ext] = [e for e in gltf[ext] if e != "KHR_texture_basisu"]
            if not gltf[ext]:
                del gltf[ext]
    for a in gltf.get("animations", []):
        if a.get("name") in rename:
            a["name"] = rename[a["name"]]
    gltf.setdefault("asset", {})["copyright"] = COPYRIGHT
    gltf["asset"]["generator"] = "Gates ci/store_pack.py"

    out = bytearray()
    for i, bv in enumerate(gltf["bufferViews"]):
        data = replacement.get(i)
        if data is None:
            st = bv.get("byteOffset", 0)
            data = blob[st:st + bv["byteLength"]]
        out += b"\0" * (-len(out) % 4)
        bv["byteOffset"] = len(out)
        bv["byteLength"] = len(data)
        out += data
    gltf["buffers"][0]["byteLength"] = len(out)
    write_glb(dst, gltf, bytes(out))
    return check(dst, is_bent)


def check(path, was_bent):
    """What a builder would ask of the file: no KTX2 left, and its counts."""
    gltf, _ = read_glb(path)
    left = [i for i, im in enumerate(gltf.get("images", [])) if im.get("mimeType") != "image/png"]
    if left:
        raise SystemExit(f"{path}: images {left} are still not PNG")
    tris = sum(gltf["accessors"][p["indices"]]["count"] // 3
               for m in gltf.get("meshes", []) for p in m["primitives"] if "indices" in p)
    return {"tris": tris, "joints": max((len(s["joints"]) for s in gltf.get("skins", [])), default=0),
            "clips": len(gltf.get("animations", [])), "maps": len(gltf.get("images", [])),
            "bent": was_bent}


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(pack, top, out):
    folder = os.path.join(out, pack["id"])
    shutil.rmtree(folder, ignore_errors=True)
    os.makedirs(folder)
    files = []
    for f in pack["files"]:
        dst = os.path.join(folder, f["name"])
        stats = convert(os.path.join(ROOT, f["src"]), dst, pack.get("rename_clips", {}))
        size = os.path.getsize(dst)
        files.append({"name": f["name"], "sha256": sha(dst), "size": size})
        print(f"  {f['name']:34s} {size / 2**20:5.1f} MB  {stats['tris']:6d} tris  "
              f"{stats['joints']:3d} joints  {stats['clips']:3d} clips  {stats['maps']} maps"
              f"{'  (un-bent)' if stats['bent'] else ''}", flush=True)
    m = {"elo_asset": 1, "id": pack["id"], "name": pack["name"], "about": pack["about"].strip(),
         "kind": "model", "version": pack.get("version", top["version"]),
         "license": top["license"], "engines": top["engines"], "price_cents": 0,
         "tags": pack["tags"], "files": files,
         "credit": f"{pack['name']} from Gates, by Elo Pros (elopros.com), {top['license']}",
         "made_with": {"parts": ["art"], "note": pack["made_with"]}}
    if pack.get("source"):
        m["source"] = pack["source"]
    pic = os.path.join(PREVIEWS, pack["id"] + ".jpg")
    if os.path.exists(pic):
        shutil.copyfile(pic, os.path.join(folder, "preview.jpg"))
        m["preview"] = "preview.jpg"
    else:
        print(f"  (no preview at {os.path.relpath(pic, ROOT)})")
    with open(os.path.join(folder, "elo-asset.json"), "w") as fh:
        json.dump(m, fh, indent=1)
        fh.write("\n")
    return folder


def main(argv):
    if not argv:
        print(__doc__.strip().splitlines()[-1])
        return 2
    out, want = argv[0], set(argv[1:])
    with open(PACKS, "rb") as fh:
        top = tomllib.load(fh)
    packs = [p for p in top["pack"] if not want or p["id"] in want]
    unknown = want - {p["id"] for p in top["pack"]}
    if unknown:
        print(f"no pack named {', '.join(sorted(unknown))}")
        return 2
    for p in packs:
        print(f"{p['id']}:", flush=True)
        build(p, top, out)
    folders = " ".join(os.path.join(out, p["id"]) for p in packs)
    print(f"\n{len(packs)} pack(s) in {out}. List them from the Elo Pros box, in this order "
          f"(the store shows them in the order they were first published):\n"
          f"  meter/.venv/bin/python meter/asset_house.py publish {folders}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
