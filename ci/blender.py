#!/usr/bin/env python3
"""Run a ci/bl/ script inside headless Blender against the game's models.

    ci/blender.py sheet  [glb…|dir…] --out DIR     # turntable + scale sheets
    ci/blender.py hero   --out DIR                 # camp hero shots
    ci/blender.py gen    <generator> --out DIR     # procedural props (ci/bl/gen_*.py)
    ci/blender.py run    <script.py> [args…]       # any script, args after --

Blender cannot read the KTX2 textures the shipped GLBs carry, so every GLB
handed to a script is first decoded to a PNG-textured copy with
ci/unbake_ktx.py (needs the KTX-Software CLI, see ci/ktx_pack.py). The
copies are cached under $XDG_CACHE_HOME/gates-bl keyed by mtime.

Needs Blender 4.2+ (`BLENDER_BIN`, or `blender` on PATH). GPU is used when
Cycles sees one.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BL = os.path.join(HERE, "bl")
CACHE = os.environ.get("GATES_BL_CACHE") or os.path.join(tempfile.gettempdir(), "gates-bl")
PLAYER = os.path.join(ROOT, "assets", "models", "stumpy.glb")


def blender_bin():
    b = os.environ.get("BLENDER_BIN") or shutil.which("blender")
    if not b:
        raise SystemExit("blender not found: set BLENDER_BIN or put blender on PATH")
    return b


def decoded(glb):
    """Path of a PNG-textured copy of `glb`, decoding it on first use."""
    glb = os.path.abspath(glb)
    st = os.stat(glb)
    key = hashlib.sha1(f"{glb}:{st.st_mtime_ns}:{st.st_size}".encode()).hexdigest()[:16]
    out = os.path.join(CACHE, key, os.path.basename(glb))
    if os.path.exists(out):
        return out
    os.makedirs(os.path.dirname(out), exist_ok=True)
    r = subprocess.run([sys.executable, os.path.join(HERE, "unbake_ktx.py"), glb, out],
                       capture_output=True, text=True)
    if r.returncode != 0:
        # A GLB with no KTX2 in it (a fresh export) is fine as it is.
        if "ktx2" in (r.stdout + r.stderr).lower() or "KTX" in (r.stdout + r.stderr):
            sys.stderr.write(r.stdout + r.stderr)
            raise SystemExit(f"unbake failed on {glb}")
        shutil.copy(glb, out)
    return out


def expand(paths):
    for p in paths:
        if os.path.isdir(p):
            for dp, _, fs in os.walk(p):
                for f in sorted(fs):
                    if f.endswith(".glb"):
                        yield os.path.join(dp, f)
        elif p.endswith(".glb"):
            yield p


def run_script(script, args, env=None):
    cmd = [blender_bin(), "-b", "--python-exit-code", "1", "--python", script, "--", *args]
    e = dict(os.environ)
    e.setdefault("GATES_ROOT", ROOT)
    e["PYTHONPATH"] = BL + os.pathsep + e.get("PYTHONPATH", "")
    if env:
        e.update(env)
    r = subprocess.run(cmd, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    keep = [l for l in r.stdout.splitlines()
            if l.strip() and not l.startswith(("Blender ", "Read prefs", "Read blend"))
            and "| INFO" not in l and "glTF import" not in l]
    print("\n".join(keep[-60:]))
    if r.returncode:
        raise SystemExit(f"blender exited {r.returncode}")


def main():
    argv = sys.argv[1:]
    if not argv or argv[0] in ("-h", "--help"):
        raise SystemExit(__doc__)
    mode, rest = argv[0], argv[1:]
    if mode == "run":
        return run_script(rest[0], rest[1:])
    if mode == "gen":
        return run_script(os.path.join(BL, f"gen_{rest[0]}.py"), rest[1:])
    if mode in ("sheet", "hero"):
        glbs, flags = [], []
        for a in rest:
            (flags if a.startswith("--") or (flags and flags[-1] == "--out") else glbs).append(a)
        if not glbs:
            glbs = [os.path.join(ROOT, "assets", "models")]
        originals = [g for g in expand(glbs) if not g.endswith(("stumpy.glb", "mannequin.gltf"))]
        with ThreadPoolExecutor(4) as ex:
            files = list(ex.map(decoded, originals))
        env = {"GATES_BL_PLAYER": decoded(PLAYER)} if os.path.exists(PLAYER) else {}
        # The script needs the ORIGINAL path too, for its name and category.
        pairs = [f"{o}={d}" for o, d in zip(originals, files)]
        run_script(os.path.join(BL, f"{mode}.py"), [*flags, *pairs], env)
        if mode == "sheet":
            compose_sheets(flags[flags.index("--out") + 1])
        return None
    raise SystemExit(__doc__)


def compose_sheets(out):
    """Lay the four views of each model out as a labelled card, and the
    cards of a category as a contact sheet, with Pillow."""
    from PIL import Image, ImageDraw, ImageFont
    views = os.path.join(out, "_views")
    with open(os.path.join(views, "index.json")) as f:
        index = json.load(f)
    try:
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 15)
        bold = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 17)
    except OSError:
        font = bold = ImageFont.load_default()
    cards = {}
    for key, st in sorted(index.items()):
        cat, name = key.split("/", 1)
        tiles = [Image.open(os.path.join(views, cat, f"{name}_{i}.png")).convert("RGB") for i in range(4)]
        t = 320
        card = Image.new("RGB", (t * 2, t * 2 + 48), (24, 24, 26))
        for i, im in enumerate(tiles):
            card.paste(im.resize((t, t), Image.LANCZOS), ((i % 2) * t, (i // 2) * t))
        d = ImageDraw.Draw(card)
        w, h, dp = st["size"]
        d.text((10, t * 2 + 6), name, font=bold, fill=(235, 235, 235))
        d.text((10, t * 2 + 27),
               f"{w:.2f} x {h:.2f} x {dp:.2f} m   {st['tris']:,} tris   {st['materials']} mat   "
               f"{st['texture']}px   {st['bytes']/1e6:.1f} MB",
               font=font, fill=(170, 170, 175))
        os.makedirs(os.path.join(out, cat), exist_ok=True)
        card.save(os.path.join(out, cat, f"{name}.jpg"), quality=88)
        cards.setdefault(cat, []).append(card)
    for cat, cs in cards.items():
        cols = 4
        cw, ch = cs[0].size
        rows = (len(cs) + cols - 1) // cols
        sheet = Image.new("RGB", (cw * cols, ch * rows), (24, 24, 26))
        for i, c in enumerate(cs):
            sheet.paste(c, ((i % cols) * cw, (i // cols) * ch))
        sheet.save(os.path.join(out, f"{cat}.jpg"), quality=82)
        lineup = os.path.join(views, cat, "lineup.png")
        if os.path.exists(lineup):
            Image.open(lineup).convert("RGB").save(os.path.join(out, f"{cat}_lineup.jpg"), quality=90)
    print(f"sheets: {sum(len(c) for c in cards.values())} cards in {len(cards)} categories -> {out}")


if __name__ == "__main__":
    main()
