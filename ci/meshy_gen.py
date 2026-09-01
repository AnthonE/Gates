#!/usr/bin/env python3
"""Drive the settled Meshy pipeline from the command line.

`DECISIONS.md` 2026-08-11 settles the recipe and `assets/models/MANIFEST.md`
records what it produced; what neither had was a way to RUN it from this
box. The twelve shipped assets were made through the Meshy MCP server, which
is a tool binding rather than a script -- so the pipeline was reproducible in
prose and not in a command, and the audit trail (prompt, task id, date) had
to be copied by hand out of a chat.

This is that command. It does the two stages the recipe names and nothing
else:

    nano-banana-pro text-to-image  ->  image-to-3d (smart-topology / meshy-t2)

and writes a `<slug>.json` sidecar next to the `.glb` holding the prompts,
both task ids, the credit costs and the date -- which is exactly the set
`MANIFEST.md` asks for per asset, so the row can be transcribed rather than
remembered.

**It deliberately does not size, texture-pack, or import.** That is
`ci/import_meshy.py` then `ci/ktx_pack.py`, in that order, and the ordering
trap is real: a retexture DISCARDS `auto_size` and `origin_at`, so scale is
imposed after texturing and never before.

The key is read from `$MESHY_API_KEY` and is never written to disk -- not
into the sidecar, not into a log line.

Usage:
  ci/meshy_gen.py <slug> --image-prompt "..." --mesh-prompt "..." --out DIR
  ci/meshy_gen.py --resume <image_task> <mesh_task> --out DIR   (poll only)
"""
import argparse
import datetime
import json
import os
import sys
import time
import urllib.error
import urllib.request

API = "https://api.meshy.ai"
POLL_S = 10
# A generation that has not finished in twenty minutes has not failed, it has
# hung: the API's own estimate is 2-3 minutes and the slowest observed here
# is under six. Bail loudly rather than block a session forever -- the task
# id is printed, so `--resume` picks it back up.
TIMEOUT_S = 1200


def key():
    k = os.environ.get("MESHY_API_KEY", "").strip()
    if not k:
        sys.exit("MESHY_API_KEY is not set")
    return k


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method)
    req.add_header("Authorization", "Bearer " + key())
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        sys.exit(f"{method} {path} -> {e.code}: {e.read().decode()[:400]}")


def wait(path, task_id, label):
    """Poll one task to SUCCEEDED, or exit saying which task and why."""
    t0 = time.time()
    last = None
    while True:
        t = call("GET", f"{path}/{task_id}")
        st, pr = t.get("status"), t.get("progress", 0)
        if (st, pr) != last:
            print(f"  {label} {task_id[:8]} {st} {pr}%", flush=True)
            last = (st, pr)
        if st == "SUCCEEDED":
            return t
        if st in ("FAILED", "CANCELED"):
            sys.exit(f"{label} {task_id}: {st} — {t.get('task_error')}")
        if time.time() - t0 > TIMEOUT_S:
            sys.exit(f"{label} {task_id}: still {st} after {TIMEOUT_S}s — "
                     f"re-run with --resume {task_id}")
        time.sleep(POLL_S)


def fetch(url, dest):
    req = urllib.request.Request(url)
    with urllib.request.urlopen(req, timeout=300) as r:
        open(dest, "wb").write(r.read())
    return os.path.getsize(dest)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("slug")
    ap.add_argument("--image-prompt", required=True)
    ap.add_argument("--mesh-prompt", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--aspect", default="1:1")
    ap.add_argument("--resume-image", help="skip stage 1, use this task id")
    ap.add_argument("--resume-mesh", help="skip stage 2, use this task id")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)

    bal0 = call("GET", "/openapi/v1/balance")["balance"]
    print(f"balance {bal0}")

    # ── Stage 1 · the reference image ────────────────────────────────────
    # `nano-banana-pro` rather than `nano-banana`: the recipe names it, and
    # the image is what the mesh stage's quality rests on.
    if a.resume_image:
        img_id = a.resume_image
    else:
        img_id = call("POST", "/openapi/v1/text-to-image", {
            "ai_model": "nano-banana-pro",
            "prompt": a.image_prompt,
            "aspect_ratio": a.aspect,
        })["result"]
    img = wait("/openapi/v1/text-to-image", img_id, "image")
    img_url = (img.get("image_urls") or [img.get("image_url")])[0]
    fetch(img_url, os.path.join(a.out, a.slug + ".png"))

    # ── Stage 2 · the mesh ───────────────────────────────────────────────
    # smart-topology / meshy-t2 beat meshy-7 on every axis measured
    # (DECISIONS.md 2026-08-11): more triangles, higher albedo contrast, a
    # third of the file size, 15 credits against 55, and it PRESERVES
    # auto_size/origin_at where the meshy-7 retexture destroyed them.
    # `remove_lighting` is not passed: smart-topology refuses it outright
    # (400), and it measured as a no-op on the other path anyway.
    if a.resume_mesh:
        mesh_id = a.resume_mesh
    else:
        mesh_id = call("POST", "/openapi/v1/image-to-3d", {
            "image_url": img_url,
            "model_type": "smart-topology",
            "ai_model": "meshy-t2",
            "should_texture": True,
            "enable_pbr": True,
            "texture_resolution": "2k",
            "texture_prompt": a.mesh_prompt,
            "auto_size": True,
            "origin_at": "bottom",
        })["result"]
    mesh = wait("/openapi/v1/image-to-3d", mesh_id, "mesh ")

    glb = os.path.join(a.out, a.slug + "_raw.glb")
    size = fetch(mesh["model_urls"]["glb"], glb)
    bal1 = call("GET", "/openapi/v1/balance")["balance"]

    side = {
        "slug": a.slug,
        "date": datetime.date.today().isoformat(),
        "vendor": "Meshy",
        "image_prompt": a.image_prompt,
        "mesh_prompt": a.mesh_prompt,
        "image_task": img_id,
        "mesh_task": mesh_id,
        "image_model": "nano-banana-pro",
        "mesh_model": "meshy-t2 / smart-topology",
        "credits": bal0 - bal1,
        "glb_bytes": size,
    }
    json.dump(side, open(os.path.join(a.out, a.slug + ".json"), "w"), indent=1)
    print(f"\n{glb}  {size/1e6:.1f} MB   {bal0 - bal1} credits "
          f"(image {img_id}, mesh {mesh_id})")


if __name__ == "__main__":
    main()
