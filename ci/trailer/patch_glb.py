"""Point a Bevy-style GLB's KTX2 textures at KHR_texture_basisu so three.js's KTX2Loader picks them up.

    python3 patch_glb.py IN.glb OUT.glb
"""
import json
import struct
import sys

src, dst = sys.argv[1], sys.argv[2]
f = open(src, "rb").read()
jl = struct.unpack("<I", f[12:16])[0]
j = json.loads(f[20:20 + jl])
rest = f[20 + jl:]
ktx = {i for i, im in enumerate(j.get("images", [])) if im.get("mimeType") == "image/ktx2" or im.get("uri", "").endswith(".ktx2")}
for t in j.get("textures", []):
    if "source" in t and t["source"] in ktx:
        t["extensions"] = {**t.get("extensions", {}), "KHR_texture_basisu": {"source": t.pop("source")}}
if ktx:
    j["extensionsUsed"] = sorted(set(j.get("extensionsUsed", []) + ["KHR_texture_basisu"]))
b = json.dumps(j, separators=(",", ":")).encode()
b += b" " * (-len(b) % 4)
out = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(b) + len(rest)) + struct.pack("<I4s", len(b), b"JSON") + b + rest
open(dst, "wb").write(out)
