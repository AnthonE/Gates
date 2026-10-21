#!/usr/bin/env python3
"""Repaint a shipped held model with Meshy: a skin the way Rust makes one.

A Rust skin is the item's own mesh in a new finish, so the gun is held,
aimed and fired exactly as it always was. `ci/meshy_gen.py` makes a new
object from a picture; this keeps ours:

  1. Strip the shipped `.glb` to its geometry (positions, normals, UVs).
  2. `retexture` it on its ORIGINAL UVs (`enable_original_uv`), PBR maps on.
  3. Graft the delivered maps back onto the shipped geometry, untouched:
     same triangles, same height, same grip, so `tests/held_assets.rs`'s
     fit holds by construction. The delivery's UVs are checked against
     ours first; a delivery that moved them is refused.

`--glow S` gives the skin an emissive map burning at strength S
(`KHR_materials_emissive_strength`, which Bevy reads): the fel cracks burn
at 3.0. Without it the emissive is stripped, as `import_meshy.py` does.
⚠ **meshy-6's own emission map came back black** for a prompt that asked
for glowing cracks (2026-10-21, the first Verdigris rifle), though it
painted the cracks into the albedo. So `--veins HUE` makes the map from
the albedo instead, `ci/fel_veins.py`'s rule by hue: the cracks the
prompt painted burn and nothing else does. A map that comes out black
either way is refused rather than shipped as a glow.

Writes `<out>/<slug>_graft.glb` with PNG maps, and a `<slug>.json` sidecar
with the prompt, task id and credits for `assets/models/MANIFEST.md`. Then:

    ci/ktx_pack.py <out>/<slug>_graft.glb assets/models/held/skins/<slug>.glb --size 1024

The key is `$MESHY_API_KEY` (any value where a proxy injects it).

Usage:
  ci/meshy_retexture.py <base.glb> <slug> --style "..." --out DIR
                        [--glow S [--veins HUE]] [--resume TASK_ID]
"""
import argparse
import base64
import datetime
import io
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from meshy_gen import call, data_uri, fetch, wait  # noqa: E402

COMPONENTS = {5126: ("<f4", 4), 5125: ("<u4", 4), 5123: ("<u2", 2), 5121: ("<u1", 1)}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        sys.exit(f"{path}: not a GLB")
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = next(c for k, c in chunks if k == 0x4E4F534A)
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return json.loads(js), blob


def glb_bytes(gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    return b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body


def accessor(gltf, blob, i):
    a = gltf["accessors"][i]
    bv = gltf["bufferViews"][a["bufferView"]]
    dt, size = COMPONENTS[a["componentType"]]
    n = WIDTH[a["type"]]
    st = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride", size * n)
    rows = np.ndarray((a["count"], n), dtype=dt, buffer=blob, offset=st,
                      strides=(stride, size))
    return np.array(rows)


# How the generator is shown the mesh. Its texturing leans on a front view,
# and a gun stood barrel-up (`ui::hold`'s guns) shows that view its muzzle:
# `gun` lays it back down, muzzle to +X, top up, side to the camera. A
# cyclic permutation of the axes, so a rotation; only the maps come back,
# so the shipped geometry never sees it.
POSES = {"upright": (0, 1, 2), "gun": (1, 2, 0)}


def geometry_only(gltf, blob, pose="upright"):
    """The shipped mesh with every image, texture and sampler taken out,
    its axes permuted to `pose` (`POSES`) for the generator's eyes."""
    prim = gltf["meshes"][0]["primitives"][0]
    out, views, accs = bytearray(), [], []
    keep = {k: prim["attributes"][k] for k in ("POSITION", "NORMAL", "TEXCOORD_0")}
    attrs = {}
    for name, i in list(keep.items()) + [("indices", prim["indices"])]:
        a = dict(gltf["accessors"][i])
        data = accessor(gltf, blob, i)
        if name == "indices":
            data = data.astype("<u4")
            a["componentType"] = 5125
        elif name in ("POSITION", "NORMAL"):
            data = np.ascontiguousarray(data[:, list(POSES[pose])])
            if "min" in a:
                a["min"] = [a["min"][k] for k in POSES[pose]]
                a["max"] = [a["max"][k] for k in POSES[pose]]
        raw = data.astype(data.dtype.newbyteorder("<")).tobytes()
        out += b"\0" * (-len(out) % 4)
        views.append({"buffer": 0, "byteOffset": len(out), "byteLength": len(raw)})
        out += raw
        a["bufferView"] = len(views) - 1
        a.pop("byteOffset", None)
        accs.append(a)
        attrs[name] = len(accs) - 1
    idx = attrs.pop("indices")
    g = {
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": attrs, "indices": idx, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.8, 0.8, 1.0]}}],
        "accessors": accs,
        "bufferViews": views,
        "buffers": [{"byteLength": len(out)}],
    }
    return glb_bytes(g, out)


def uv_match(base_uv, got_uv):
    """Fraction of the delivery's UVs that are one of ours, to 1/4096."""
    q = lambda uv: set(map(tuple, np.round(uv * 4096).astype(np.int64)))
    ours = q(base_uv)
    theirs = np.round(got_uv * 4096).astype(np.int64)
    hit = sum(1 for t in map(tuple, theirs) if t in ours)
    return hit / max(len(theirs), 1)


def veins(albedo, hue):
    """An emissive map out of the albedo's own coloured cracks: a texel burns
    by how close its hue is to `hue` (degrees), how saturated and how bright
    it is — `ci/fel_veins.py`'s rule, by hue rather than by green alone, so
    an ember skin's orange works the same way. The map is the albedo times
    that mask: the cracks the prompt painted, and nothing else."""
    rgb = np.asarray(albedo.convert("RGB"), dtype=np.float64) / 255.0
    mx, mn = rgb.max(-1), rgb.min(-1)
    sat = np.where(mx > 1e-6, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
    h = np.asarray(albedo.convert("HSV"), dtype=np.float64)[..., 0] * (360.0 / 255.0)
    dh = np.abs((h - hue + 180.0) % 360.0 - 180.0)
    ramp = lambda x, lo, hi: np.clip((x - lo) / (hi - lo), 0.0, 1.0)
    m = (1.0 - ramp(dh, 18.0, 40.0)) * ramp(sat, 0.45, 0.65) * ramp(mx, 0.30, 0.45)
    return rgb * m[..., None]


def png(img):
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("slug")
    ap.add_argument("--style", required=True, help="text_style_prompt")
    ap.add_argument("--out", required=True)
    ap.add_argument("--glow", type=float, help="keep the emission map at this strength")
    ap.add_argument("--veins", type=float, metavar="HUE",
                    help="with --glow: burn the albedo's cracks of this hue (degrees) "
                         "instead of the delivery's emission map")
    ap.add_argument("--model", default="meshy-6",
                    help="meshy-6 is the one that paints an emission map")
    ap.add_argument("--concept", metavar="REF",
                    help="first paint `--style` over REF (the item's own icon render, "
                         "side on) with nano-banana-pro, and texture from that picture")
    ap.add_argument("--pose", choices=sorted(POSES), default="upright",
                    help="how the generator is shown the mesh (`gun` for a barrel-up gun)")
    ap.add_argument("--resume", help="poll this retexture task instead of starting one")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)

    base, blob = read_glb(a.base)
    if len(base["meshes"]) != 1 or len(base["meshes"][0]["primitives"]) != 1:
        sys.exit(f"{a.base}: not a single-primitive model")
    prim = base["meshes"][0]["primitives"][0]

    bal0 = call("GET", "/openapi/v1/balance")["balance"]
    ep = "/openapi/v1/retexture"
    concept = None
    if a.concept and not a.resume:
        # The skin painted over the item's own silhouette first: a picture of
        # THIS gun in the new finish is a far stronger brief than words, and
        # it is how a skin artist works, over the model. White ground, as
        # every generator prompt here asks for.
        ref = Image.open(a.concept).convert("RGBA")
        flat = Image.new("RGBA", ref.size, (255, 255, 255, 255))
        flat.alpha_composite(ref)
        ref_png = os.path.join(a.out, a.slug + "_ref.png")
        flat.convert("RGB").save(ref_png)
        img_ep = "/openapi/v1/image-to-image"
        cid = call("POST", img_ep, {
            "ai_model": "nano-banana-pro",
            "prompt": "Exactly this object, the same shape, proportions and parts, seen the "
                      "same way, repainted as a premium cosmetic skin: " + a.style
                      + " Plain white background.",
            "reference_image_urls": [data_uri(ref_png)],
        })["result"]
        img = wait(img_ep, cid, "concept")
        concept = os.path.join(a.out, a.slug + "_concept.png")
        fetch((img.get("image_urls") or [img.get("image_url")])[0], concept)
    if a.resume:
        tid = a.resume
    else:
        geo = geometry_only(base, blob, a.pose)
        style = ({"image_style_url": data_uri(concept)} if concept
                 else {"text_style_prompt": a.style})
        tid = call("POST", ep, {
            "model_url": "data:application/octet-stream;base64," + base64.b64encode(geo).decode(),
            **style,
            "ai_model": a.model,
            "enable_original_uv": True,
            "enable_pbr": True,
            "texture_resolution": "2k",
            "target_formats": ["glb"],
        })["result"]
    t = wait(ep, tid, "retexture")

    raw = os.path.join(a.out, a.slug + "_raw.glb")
    fetch(t["model_urls"]["glb"], raw)
    got, got_blob = read_glb(raw)
    gp = got["meshes"][0]["primitives"][0]
    match = uv_match(accessor(base, blob, prim["attributes"]["TEXCOORD_0"]),
                     accessor(got, got_blob, gp["attributes"]["TEXCOORD_0"]))
    print(f"  UVs: {match:.1%} of the delivery's are ours")
    if match < 0.98:
        sys.exit(f"{raw}: the delivery's UVs are not the shipped model's ({match:.1%}) "
                 f"— its maps would land in the wrong places")

    tex = t["texture_urls"][0]
    maps = {}
    for k in ("base_color", "metallic", "roughness", "normal", "emission"):
        if tex.get(k):
            dest = os.path.join(a.out, f"{a.slug}_{k}.png")
            fetch(tex[k], dest)
            maps[k] = Image.open(dest)
    if "base_color" not in maps:
        sys.exit(f"retexture {tid}: no base colour map")
    size = maps["base_color"].size
    grey = lambda k, d: (np.asarray(maps[k].convert("L").resize(size), dtype=np.uint8)
                         if k in maps else np.full(size[::-1], d, np.uint8))
    # glTF's metallicRoughness: roughness in G, metal in B.
    mr = np.stack([np.full(size[::-1], 255, np.uint8), grey("roughness", 200),
                   grey("metallic", 0)], -1)
    images = {
        "baseColorTexture": png(maps["base_color"].convert("RGBA")),
        "metallicRoughnessTexture": png(Image.fromarray(mr, "RGB")),
    }
    if "normal" in maps:
        images["normalTexture"] = png(maps["normal"].convert("RGB"))
    glow = None
    if a.glow:
        if a.veins is not None:
            em = veins(maps["base_color"], a.veins)
        elif "emission" in maps:
            em = np.asarray(maps["emission"].convert("RGB"), dtype=np.float64) / 255.0
        else:
            sys.exit(f"retexture {tid}: --glow and no emission map came back (try --veins)")
        lit = float((em.max(-1) > 0.2).mean())
        print(f"  emission: {lit:.1%} of texels lit")
        if lit < 0.002:
            sys.exit(f"retexture {tid}: the emission map is black — try --veins HUE")
        images["emissiveTexture"] = png(Image.fromarray((em * 255).round().astype(np.uint8)))
        glow = a.glow

    # The shipped file, its images swapped for the delivery's: geometry and
    # every accessor stay byte for byte.
    out = bytearray()
    views, remap = [], {}
    old_images = {im["bufferView"] for im in base.get("images", [])}
    for i, bv in enumerate(base["bufferViews"]):
        if i in old_images:
            continue
        st = bv.get("byteOffset", 0)
        data = blob[st:st + bv["byteLength"]]
        out += b"\0" * (-len(out) % 4)
        nb = dict(bv, byteOffset=len(out))
        out += data
        remap[i] = len(views)
        views.append(nb)
    for acc in base["accessors"]:
        acc["bufferView"] = remap[acc["bufferView"]]
    base["images"], base["textures"] = [], []
    mat = {"name": a.slug, "doubleSided": base["materials"][0].get("doubleSided", True),
           "pbrMetallicRoughness": {}}
    for slot, data in images.items():
        out += b"\0" * (-len(out) % 4)
        views.append({"buffer": 0, "byteOffset": len(out), "byteLength": len(data)})
        out += data
        base["images"].append({"bufferView": len(views) - 1, "mimeType": "image/png"})
        base["textures"].append({"sampler": 0, "source": len(base["images"]) - 1})
        ref = {"index": len(base["textures"]) - 1}
        if slot in ("baseColorTexture", "metallicRoughnessTexture"):
            mat["pbrMetallicRoughness"][slot] = ref
        else:
            mat[slot] = ref
    mat["emissiveFactor"] = [1.0, 1.0, 1.0] if glow else [0.0, 0.0, 0.0]
    if glow:
        mat["extensions"] = {"KHR_materials_emissive_strength": {"emissiveStrength": glow}}
        base["extensionsUsed"] = sorted(set(base.get("extensionsUsed", []))
                                        | {"KHR_materials_emissive_strength"})
    base["materials"] = [mat]
    base["samplers"] = base.get("samplers") or [{"magFilter": 9729, "minFilter": 9987}]
    base["bufferViews"] = views
    base["buffers"] = [{"byteLength": len(out)}]
    graft = os.path.join(a.out, a.slug + "_graft.glb")
    open(graft, "wb").write(glb_bytes(base, out))

    bal1 = call("GET", "/openapi/v1/balance")["balance"]
    side = {
        "slug": a.slug,
        "date": datetime.date.today().isoformat(),
        "vendor": "Meshy",
        "base": os.path.relpath(a.base),
        "style_prompt": a.style,
        "concept_ref": a.concept and os.path.relpath(a.concept),
        "retexture_task": tid,
        "ai_model": a.model,
        "glow": glow,
        "pose": a.pose,
        "uv_match": round(match, 4),
        "credits": bal0 - bal1,
    }
    json.dump(side, open(os.path.join(a.out, a.slug + ".json"), "w"), indent=1)
    print(f"\n{graft}  {bal0 - bal1} credits (retexture {tid})")


if __name__ == "__main__":
    main()
