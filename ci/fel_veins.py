#!/usr/bin/env python3
"""Light a generated model's green veins: an emissive map from its albedo.

`ci/rig_beast.py`'s `fel_glow` does this inside Blender for the fel beasts;
the Verdigris spires (`assets/models/prop/fel/`) are static and never touch
Blender, so this is the same arithmetic over the GLB's own albedo, with numpy
and Pillow. A texel glows by how green-dominant and how bright it is:

    m = clip((g - max(r, b) * 1.25) / 0.15) * clip((g - 0.28) / 0.15)

and the emissive map is the albedo times `m` — the veins the prompt asked
for, and nothing else. The client scales it by the hour (`render/boulders.rs`),
so the factor written here is 1.

`--lift L` also raises the albedo's mean linear luma to `L` where the
generator painted glass blacker than `ART.md`'s floor (0.05: no pure black) —
the veins keep their own value, everything else is scaled, so the obsidian
reads as black glass by its gloss and not by a texel at zero.

Run on a raw delivery, before `ci/import_meshy.py --emissive` (which keeps
the map this writes) and `ci/ktx_pack.py` (which packs it sRGB).

Usage:  ci/fel_veins.py <in.glb> <out.glb> [--lift 0.05]
"""
import io
import json
import struct
import sys

import numpy as np
from PIL import Image


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off = 12
    chunks = []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = next(c for k, c in chunks if k == 0x4E4F534A)
    bin_ = next((c for k, c in chunks if k == 0x004E4942), b"")
    return json.loads(js), bytearray(bin_)


def write_glb(path, gltf, blob):
    blob = bytes(blob) + b"\0" * (-len(blob) % 4)
    gltf["buffers"][0]["byteLength"] = len(blob)
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    body = (struct.pack("<II", len(js), 0x4E4F534A) + js
            + struct.pack("<II", len(blob), 0x004E4942) + blob)
    open(path, "wb").write(b"glTF" + struct.pack("<II", 2, 12 + len(body)) + body)


def image_bytes(gltf, blob, img_i):
    bv = gltf["bufferViews"][gltf["images"][img_i]["bufferView"]]
    start = bv.get("byteOffset", 0)
    return bytes(blob[start:start + bv["byteLength"]])


def vein_mask(rgb):
    """`rig_beast.fel_glow`'s mask, on linear-ish 0..1 albedo."""
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    return (np.clip((g - np.maximum(r, b) * 1.25) / 0.15, 0, 1)
            * np.clip((g - 0.28) / 0.15, 0, 1))


def to_linear(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_srgb(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1.0 / 2.4) - 0.055)


def put_image(gltf, blob, png):
    """Append a PNG to the buffer; its new image index."""
    blob.extend(b"\0" * (-len(blob) % 4))
    gltf["bufferViews"].append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(png)})
    blob.extend(png)
    gltf["images"].append({"bufferView": len(gltf["bufferViews"]) - 1, "mimeType": "image/png"})
    return len(gltf["images"]) - 1


def prune(gltf, blob):
    """Drop the images and textures no material reads — the albedo `--lift`
    replaced and the generator's own emissive guess — and the bytes they held,
    so `ci/ktx_pack.py` does not encode maps nothing draws."""
    slots = []
    for m in gltf.get("materials", []):
        pbr = m.get("pbrMetallicRoughness", {})
        for owner, key in [(m, "normalTexture"), (m, "occlusionTexture"), (m, "emissiveTexture"),
                           (pbr, "baseColorTexture"), (pbr, "metallicRoughnessTexture")]:
            if key in owner:
                slots.append(owner[key])
    keep_tex = sorted({t["index"] for t in slots})
    tex_map = {old: new for new, old in enumerate(keep_tex)}
    textures = [gltf["textures"][i] for i in keep_tex]
    for t in slots:
        t["index"] = tex_map[t["index"]]
    keep_img = sorted({t["source"] for t in textures})
    img_map = {old: new for new, old in enumerate(keep_img)}
    images = [gltf["images"][i] for i in keep_img]
    for t in textures:
        t["source"] = img_map[t["source"]]
    used = {a["bufferView"] for a in gltf["accessors"] if "bufferView" in a}
    used |= {im["bufferView"] for im in images}
    bv_map, views, out = {}, [], bytearray()
    for i, bv in enumerate(gltf["bufferViews"]):
        if i not in used:
            continue
        st = bv.get("byteOffset", 0)
        data = blob[st:st + bv["byteLength"]]
        out.extend(b"\0" * (-len(out) % 4))
        nb = dict(bv)
        nb["byteOffset"] = len(out)
        nb["byteLength"] = len(data)
        bv_map[i] = len(views)
        views.append(nb)
        out.extend(data)
    for a in gltf["accessors"]:
        if "bufferView" in a:
            a["bufferView"] = bv_map[a["bufferView"]]
    for im in images:
        im["bufferView"] = bv_map[im["bufferView"]]
    gltf["textures"], gltf["images"], gltf["bufferViews"] = textures, images, views
    return out


def main():
    argv = sys.argv[1:]
    lift = None
    if "--lift" in argv:
        at = argv.index("--lift")
        lift = float(argv[at + 1])
        argv = argv[:at] + argv[at + 2:]
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 2:
        raise SystemExit(__doc__)
    src, dst = args
    gltf, blob = read_glb(src)
    if len(gltf.get("materials", [])) != 1:
        raise SystemExit(f"{src}: expected one material")
    mat = gltf["materials"][0]
    base = mat.get("pbrMetallicRoughness", {}).get("baseColorTexture")
    if base is None:
        raise SystemExit(f"{src}: no base colour texture to read veins from")
    tex = gltf["textures"][base["index"]]
    im = Image.open(io.BytesIO(image_bytes(gltf, blob, tex["source"]))).convert("RGB")
    rgb = np.asarray(im, dtype=np.float32) / 255.0
    m = vein_mask(rgb)
    glow = (rgb * m[..., None] * 255.0 + 0.5).clip(0, 255).astype(np.uint8)
    out = io.BytesIO()
    Image.fromarray(glow, "RGB").save(out, "PNG")
    png = out.getvalue()

    if lift is not None:
        lin = to_linear(rgb)
        luma = lin @ np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)
        # The gutter the generator pads its charts with is black and is not
        # surface; the veins keep their value.
        body = (luma > 1e-3) & (m < 0.05)
        mean = float(luma[body].mean()) if body.any() else lift
        gain = min(max(lift / max(mean, 1e-6), 1.0), 6.0)
        k = np.where(m[..., None] < 0.05, gain, 1.0)
        lifted = (to_srgb(lin * k) * 255.0 + 0.5).clip(0, 255).astype(np.uint8)
        buf = io.BytesIO()
        Image.fromarray(lifted, "RGB").save(buf, "PNG")
        tex["source"] = put_image(gltf, blob, buf.getvalue())
        print(f"  albedo mean luma {mean:.4f} lifted x{gain:.2f} toward {lift}")

    # A new image after everything already in the buffer, aligned to 4.
    new_tex = {"source": put_image(gltf, blob, png)}
    if "sampler" in tex:
        new_tex["sampler"] = tex["sampler"]
    gltf["textures"].append(new_tex)
    em = {"index": len(gltf["textures"]) - 1}
    if "texCoord" in base:
        em["texCoord"] = base["texCoord"]
    mat["emissiveTexture"] = em
    mat["emissiveFactor"] = [1.0, 1.0, 1.0]
    blob = prune(gltf, blob)
    write_glb(dst, gltf, blob)
    print(f"  {dst.rsplit('/', 1)[-1]}: veins glow on {m.mean() * 100:.2f}% of the albedo "
          f"(max {m.max():.2f}), {im.size[0]}x{im.size[1]}")


if __name__ == "__main__":
    main()
