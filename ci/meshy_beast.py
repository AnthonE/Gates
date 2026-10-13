#!/usr/bin/env python3
"""Generate a four-legged animal on Meshy, ready for `ci/rig_beast.py`.

    ci/meshy_beast.py wolf_a --prompt "A large grey timber wolf ..." --out DIR

Four stages, each resumable off the `<slug>.json` sidecar (a re-run polls the
task it already paid for instead of paying again):

1. `nano-banana-pro` text-to-image: the design, one three-quarter view.
2. `nano-banana-pro` image-to-image with `generate_multi_view`: the SAME
   animal from the front, the side and behind. One view of an animal
   reconstructs as a slab, the rock lesson in `ci/meshy_gen.py`'s header.
3. Each view cropped to its largest connected subject. The multi-view sheet
   is cut into images that carry slivers of the neighbouring view (measured
   on all six fel animals, 2026-10-13: a strip of horn, a hind leg), and a
   reconstruction builds whatever it is shown.
4. `meshy-7.1` multi-image-to-3d from the original plus the three views:
   PBR, lighting removed, remeshed to `--tris` triangles. `meshy-7` is
   deprecated (Meshy changelog, 2026-09-18) and `multi-image-to-3d` refuses
   Smart Topology.

The prompt should ask for a neutral standing pose with the legs straight and
apart, the mouth closed, a plain background, and **no fire, smoke or text**:
flames reconstruct as solid lumps, and a stated size draws a scale bar
(`assets/models/MANIFEST.md`, the boar). Costs 9 + 9 + 30 credits.

The key is read from `$MESHY_API_KEY` and never written anywhere.
"""
import argparse
import base64
import datetime
import json
import os
import sys
import time
import urllib.error
import urllib.request

import numpy as np
from PIL import Image

API = "https://api.meshy.ai"
VIEWS_PROMPT = (
    "The exact same creature as the reference image, unchanged in design, colours, "
    "markings, glow and proportions, in the same neutral standing pose with all four "
    "legs straight. Plain light grey background, soft even light. No fire, no smoke, no text."
)


def call(method, path, body=None):
    key = os.environ.get("MESHY_API_KEY", "").strip() or sys.exit("MESHY_API_KEY is not set")
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method)
    req.add_header("Authorization", "Bearer " + key)
    if data:
        req.add_header("Content-Type", "application/json")
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503) and attempt < 4:
                time.sleep(10 * (attempt + 1))
                continue
            sys.exit(f"{method} {path} -> {e.code}: {e.read().decode()[:400]}")
        except (urllib.error.URLError, TimeoutError):
            if attempt == 4:
                raise
            time.sleep(10)


def wait(path, tid, label, limit_s):
    t0, last = time.time(), None
    while True:
        t = call("GET", f"{path}/{tid}")
        st, pr = t.get("status"), t.get("progress", 0)
        if (st, pr // 25) != last:
            print(f"  {label} {tid[:8]} {st} {pr}%", flush=True)
            last = (st, pr // 25)
        if st == "SUCCEEDED":
            return t
        if st in ("FAILED", "CANCELED"):
            sys.exit(f"{label} {tid}: {st} {t.get('task_error')}")
        if time.time() - t0 > limit_s:
            sys.exit(f"{label} {tid}: still {st} — re-run to resume")
        time.sleep(8)


def fetch(url, dest):
    with urllib.request.urlopen(url, timeout=600) as r:
        open(dest, "wb").write(r.read())
    return os.path.getsize(dest)


def data_uri(path):
    return "data:image/png;base64," + base64.b64encode(open(path, "rb").read()).decode()


def largest_subject(src, dst, thresh=24, down=4):
    """Black out everything but the largest connected bright region, then
    crop to it with a margin. Labelled on a 1/`down` grid by flood fill —
    numpy and PIL only, so a fresh box needs nothing new."""
    im = Image.open(src).convert("RGB")
    a = np.asarray(im)
    small = np.asarray(im.resize((max(1, im.width // down), max(1, im.height // down))))
    mask = small.max(axis=2) > thresh
    h, w = mask.shape
    label = np.zeros((h, w), np.int32)
    sizes = [0]
    for y0 in range(h):
        for x0 in range(w):
            if not mask[y0, x0] or label[y0, x0]:
                continue
            n = len(sizes)
            stack, count = [(y0, x0)], 0
            label[y0, x0] = n
            while stack:
                y, x = stack.pop()
                count += 1
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        yy, xx = y + dy, x + dx
                        if 0 <= yy < h and 0 <= xx < w and mask[yy, xx] and not label[yy, xx]:
                            label[yy, xx] = n
                            stack.append((yy, xx))
            sizes.append(count)
    keep_small = label == int(np.argmax(sizes))
    keep = np.asarray(Image.fromarray(keep_small.astype(np.uint8) * 255).resize(
        (im.width, im.height), Image.NEAREST)) > 0
    # Grow by a block so the edge the grid cut off comes back.
    grown = keep.copy()
    for d in range(1, down + 2):
        grown[d:] |= keep[:-d]
        grown[:-d] |= keep[d:]
        grown[:, d:] |= keep[:, :-d]
        grown[:, :-d] |= keep[:, d:]
    out = a * grown[..., None]
    xs = np.where(grown.any(axis=0))[0]
    m = 16
    x0, x1 = max(0, xs.min() - m), min(a.shape[1], xs.max() + m + 1)
    crop = Image.fromarray(out[:, x0:x1].astype(np.uint8))
    canvas = Image.new("RGB", (max(crop.width + 2 * m, int(crop.height * 0.6)), crop.height))
    canvas.paste(crop, ((canvas.width - crop.width) // 2, 0))
    canvas.save(dst)
    return int((mask & ~keep_small).sum()) * down * down


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("slug")
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--tris", type=int, default=15000)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    side_path = os.path.join(a.out, a.slug + ".json")
    side = json.load(open(side_path)) if os.path.exists(side_path) else {
        "slug": a.slug, "date": datetime.date.today().isoformat(), "vendor": "Meshy",
        "image_model": "nano-banana-pro", "image_prompt": a.prompt}

    def save():
        json.dump(side, open(side_path, "w"), indent=1)

    bal0 = call("GET", "/openapi/v1/balance")["balance"]
    # 1 · the design
    ref = os.path.join(a.out, a.slug + ".png")
    if "image_task" not in side:
        side["image_task"] = call("POST", "/openapi/v1/text-to-image", {
            "ai_model": "nano-banana-pro", "prompt": side["image_prompt"],
            "aspect_ratio": "4:3"})["result"]
        save()
    t = wait("/openapi/v1/text-to-image", side["image_task"], "image", 900)
    if not os.path.exists(ref):
        fetch((t.get("image_urls") or [t.get("image_url")])[0], ref)
    # 2 · the same animal from three more sides
    if "views_task" not in side:
        side["views_prompt"] = VIEWS_PROMPT
        side["views_task"] = call("POST", "/openapi/v1/image-to-image", {
            "ai_model": "nano-banana-pro", "prompt": VIEWS_PROMPT,
            "reference_image_urls": [data_uri(ref)], "generate_multi_view": True})["result"]
        save()
    v = wait("/openapi/v1/image-to-image", side["views_task"], "views", 900)
    urls = [u for u in (v.get("image_urls") or [v.get("image_url")]) if u][:3]
    clean = []
    for i, u in enumerate(urls):
        raw = os.path.join(a.out, f"{a.slug}_view{i}.png")
        if not os.path.exists(raw):
            fetch(u, raw)
        # 3 · one subject per view
        dst = os.path.join(a.out, f"{a.slug}_view{i}_clean.png")
        dropped = largest_subject(raw, dst)
        print(f"  view {i}: dropped {dropped} px of neighbouring views", flush=True)
        clean.append(dst)
    # 4 · the mesh
    if "mesh_task" not in side:
        body = {
            "image_urls": [data_uri(ref)] + [data_uri(p) for p in clean],
            "ai_model": "meshy-7.1", "should_texture": True, "enable_pbr": True,
            "texture_resolution": "2k", "texture_prompt": side["image_prompt"][:800],
            "remove_lighting": True, "should_remesh": True, "topology": "triangle",
            "target_polycount": a.tris, "origin_at": "bottom",
        }
        side["mesh_model"] = "meshy-7.1 multi-image-to-3d"
        side["mesh_params"] = {k: x for k, x in body.items() if k != "image_urls"}
        side["mesh_task"] = call("POST", "/openapi/v1/multi-image-to-3d", body)["result"]
        save()
    m = wait("/openapi/v1/multi-image-to-3d", side["mesh_task"], "mesh ", 2400)
    glb = os.path.join(a.out, a.slug + "_raw.glb")
    side["glb_bytes"] = fetch(m["model_urls"]["glb"], glb)
    side["credits"] = bal0 - call("GET", "/openapi/v1/balance")["balance"]
    save()
    print(f"\n{glb}  {side['glb_bytes'] / 1e6:.1f} MB  (next: ci/rig_beast.py, then ci/ktx_pack.py)")


if __name__ == "__main__":
    main()
