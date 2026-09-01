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

⚠ **`--multiview` exists because ONE VIEW OF A ROCK IS A SLAB.** Measured
2026-09-02 on the first generated boulder: a single three-quarter reference
reconstructed to 2.081 x 2.000 x **0.884** m against a target footprint that
is square, i.e. the depth axis needed a 2.5x stretch that would have smeared
every fracture into a vertical streak. A building survives one view because
a three-quarter of a box describes the box; an irregular natural object does
not. With the flag the image stage asks `nano-banana-pro` for several
viewpoints of the same object and the mesh stage reconstructs from all of
them -- which **forces the meshy-7 path**, because `multi-image-to-3d`
refuses smart-topology outright.

The key is read from `$MESHY_API_KEY` and is never written to disk -- not
into the sidecar, not into a log line.

⚠ **`credits` in the sidecar is a balance delta, so it is only true for a
run that had the account to itself.** Two of these running at once each
charge themselves part of the other's spend: measured 2026-09-01, two
concurrent assets reported 48 and 39 against a true combined 72. Run them
serially if the per-asset figure matters, or record the total and say so.

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
    # A site stands once on an island; a boulder stands a thousand times, so
    # the mesh stage needs a ceiling rather than whatever the generator felt
    # like. `WANTED.md` carries the per-row budget and `RENDER.md` §6 the
    # frame's; this is how the row reaches the API.
    ap.add_argument("--tris", type=int, help="target_polycount for the mesh stage")
    # See the --multiview note in the header: one view of a rock is a slab.
    ap.add_argument("--multiview", action="store_true",
                    help="ask for several viewpoints and reconstruct from all of them")
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
            # The two are mutually exclusive: the API refuses an
            # `aspect_ratio` alongside `generate_multi_view` (400), because a
            # multi-view sheet has a shape of its own.
            **({"generate_multi_view": True} if a.multiview
               else {"aspect_ratio": a.aspect}),
        })["result"]
    img = wait("/openapi/v1/text-to-image", img_id, "image")
    urls = [u for u in (img.get("image_urls") or [img.get("image_url")]) if u]
    if not urls:
        sys.exit(f"image {img_id} succeeded with no image url: {img}")
    for i, u in enumerate(urls):
        fetch(u, os.path.join(a.out, a.slug + (f"_{i}" if i else "") + ".png"))
    print(f"  image gave {len(urls)} view(s)")

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
        common = {
            "should_texture": True,
            "enable_pbr": True,
            "texture_resolution": "2k",
            "texture_prompt": a.mesh_prompt,
            "auto_size": True,
            "origin_at": "bottom",
            **({"target_polycount": a.tris} if a.tris else {}),
        }
        if a.multiview and len(urls) > 1:
            # `multi-image-to-3d` REFUSES smart-topology: the docs are explicit
            # that meshy-t1/t2 and ultra_mode are single-image-only. So this
            # path costs a meshy-7 mesh instead of a meshy-t2 one, and it buys
            # depth with it. That is the right trade for a rock and the wrong
            # one for anything whose silhouette one view already describes.
            mesh_id = call("POST", "/openapi/v1/multi-image-to-3d", {
                "image_urls": urls[:4],
                "ai_model": "meshy-7",
                **common,
            })["result"]
            mesh_ep = "/openapi/v1/multi-image-to-3d"
        else:
            mesh_id = call("POST", "/openapi/v1/image-to-3d", {
                "image_url": urls[0],
                "model_type": "smart-topology",
                "ai_model": "meshy-t2",
                **common,
            })["result"]
            mesh_ep = "/openapi/v1/image-to-3d"
    if a.resume_mesh:
        mesh_ep = "/openapi/v1/image-to-3d"
    mesh = wait(mesh_ep, mesh_id, "mesh ")

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
        "mesh_model": mesh_ep.rsplit("/", 1)[-1],
        "views": len(urls),
        "target_polycount": a.tris,
        "credits": bal0 - bal1,
        "glb_bytes": size,
    }
    json.dump(side, open(os.path.join(a.out, a.slug + ".json"), "w"), indent=1)
    print(f"\n{glb}  {size/1e6:.1f} MB   {bal0 - bal1} credits "
          f"(image {img_id}, mesh {mesh_id})")


if __name__ == "__main__":
    main()
