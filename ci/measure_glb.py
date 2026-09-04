#!/usr/bin/env python3
"""Measure a generated GLB against the sim row it has to fill, and say
whether to keep it.

**Generation is a sampling process, and this is the step that admits it.**
Measured over eleven rolls on 2026-09-02, the prompt controls some things and
not others, and the split is worth stating because guessing it wrong costs
credits either way:

  * **Naming a dimension the prompt had omitted WORKS.** Three boulders asked
    for "2.2 m wide and 2.0 m tall", with no depth given, reconstructed to
    depth/width 0.425, 1.000 and 0.195 -- a lottery. Three more asked for
    "2.2 wide, 2.2 deep and 2.0 tall, as deep as it is wide" came back 1.016,
    1.022 and 1.032. The generator was not ignoring the instruction; there
    was no instruction.
  * **Naming a colour to avoid WORKS.** "Lichen crusting the hollows" gave a
    boulder 47.6% green-dominant; dropping it for "mottled grey with
    rust-brown staining" gave 0.2-1.3%. "Bright yellow" sulfur drifted to
    28.3% green; "warm golden-yellow, mustard, never green" landed 0.4%.
  * **Asking for a RATIO more extreme than the object's natural one does
    not.** A canopy asked three ways to be "much wider than it is tall" came
    back with a WORSE aspect than the one it was meant to fix -- 1.329x
    against 1.291x.

So: say what the object IS, in full, including the axis nobody thinks to
mention -- then roll several and reject on a number. Six boulder rolls
produced three keepers, which is the honest hit rate to budget for.

So the loop is: `ci/meshy_gen.py` several times -> this -> keep the winners
-> `ci/import_meshy.py` -> `ci/ktx_pack.py`. Roughly 40 credits a roll makes
that cheap; a bad mesh discovered after it ships is not.

What it reports, and why each one is here rather than left to the eye:

  * **tris** against the row's ceiling. A site stands once on an island and a
    boulder stands a thousand times, so this is an instance-count question,
    not a quality one (`WANTED.md` §2, `RENDER.md` §6's 1.5 M frame ceiling).
  * **depth/width**, which is the failure a single reference image causes and
    nothing else catches: one three-quarter view of a rock reconstructs to a
    SLAB. Measured 0.425 and 0.195 on two of three rolls.
  * **aspect correction** -- `max(k)/min(k)` under `import_meshy.py
    --fit-axes`, i.e. how much the most-stretched axis is stretched relative
    to the least in order to fill the volume the sim blocks. The canopy
    shipped at 1.291x and reads as chunky rather than wrong; a 5.61x wafer
    does not.
  * **albedo linear luma** against `ART.md`'s `ALBEDO_LUMA_BAND` and against
    the ground identity the object stands beside. A node measured 0.081 on a
    "dark iron ore seams" prompt -- inside the band, and half of granite's
    0.292, i.e. a black lump in a grey world.
  * **green-dominant share**, because "lichen in the hollows" returned a
    boulder that was 47.6% green and the visual judge's standing complaint is
    green-dominant coverage. It also catches a yellow that has drifted: the
    first sulfur node measured 28.3%.

The target volume is READ OUT OF `sim-core` rather than typed here -- the
`occupant_volume` match arms and `props::archetype_lift` -- so this cannot
drift from what the game actually blocks. `ci/knob_registry.mjs` scrapes
source the same way and for the same reason.

Usage:
  ci/measure_glb.py --occupant Rock  a.glb b.glb c.glb
  ci/measure_glb.py --size W H D --tris 3000  a.glb
"""
import argparse
import io
import json
import os
import re
import struct
import sys

import numpy as np

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TERRAIN = os.path.join(ROOT, "crates/sim-core/src/terrain.rs")
PROPS = os.path.join(ROOT, "crates/client/src/render/props.rs")

# ART.md's band, and the ground identities a prop is seen against
# (assets/textures/MANIFEST.md). Advisory rather than fatal: a sulfur node is
# allowed to be brighter than granite.
ALBEDO_LUMA_BAND = (0.05, 0.55)
GRANITE_LUMA = 0.292
# Past this, filling the sim's volume means a stretch you can see on a face
# with any regular feature in it. The canopy shipped at 1.291.
ASPECT_MAX = 1.50
# A rock that is half green is not a rock.
GREEN_MAX = 0.10
# Depth over width, and it needs its own bound because the RADIAL fit is
# blind to it by construction: scaling X and Z by one factor solved from the
# largest radius leaves a wafer a wafer, safely inside the blocked cylinder
# and hopeless to look at. The first six boulder rolls measured 0.195, 0.425,
# 1.000, 1.016, 1.022 and 1.032, and the two under this floor are the two a
# single reference image reconstructed as slabs. 1.7 is the same bound the
# other way up, for a plank.
DEPTH_BAND = (0.60, 1.70)


def sim_volume(occ):
    """(radius, top) from `terrain::occupant_volume`'s own match arms."""
    src = open(TERRAIN).read()
    m = re.search(rf"Occupant::{occ} => \(([\d.]+), ([\d.]+)\)", src)
    if not m:
        raise SystemExit(f"no occupant_volume row for {occ}")
    return float(m.group(1)), float(m.group(2))


def sim_lift(occ):
    """`props::archetype_lift`, read off its match arms.

    The arms are grouped (`A | B | C => 0.5`), so the pattern has to allow a
    list -- and a lift that is missed reads as 0.0, which would silently make
    every centred prop look like a base-authored one. Absent is an error.
    """
    src = open(PROPS).read()
    body = src[src.index("pub fn archetype_lift"):]
    body = body[: body.index("\n}\n")]
    for line in body.splitlines():
        line = line.split("//")[0].strip()
        m = re.match(r"Occupant::(.+?)\s*=>\s*([\d.]+),", line)
        if m and occ in [p.strip().replace("Occupant::", "") for p in m.group(1).split("|")]:
            return float(m.group(2))
    raise SystemExit(f"no archetype_lift arm for {occ}")


def read_glb(path):
    raw = open(path, "rb").read()
    if raw[:4] != b"glTF":
        raise SystemExit(f"{path}: not a GLB")
    off, chunks = 12, []
    while off < len(raw):
        ln, kind = struct.unpack("<II", raw[off:off + 8])
        chunks.append((kind, raw[off + 8:off + 8 + ln]))
        off += 8 + ln
    js = json.loads(next(c for k, c in chunks if k == 0x4E4F534A))
    blob = next((c for k, c in chunks if k == 0x004E4942), b"")
    return js, blob, len(raw)


def geometry(g, blob):
    """Extent, triangles, and the RADIUS the renderer measures.

    The radius is the largest per-vertex `hypot(x, z)` about the centred
    origin — `render::tree::bounds`' number — and not half the bounding box.
    The two differ by up to sqrt(2) on a square-ish footprint, and the
    difference is not academic: fitting the first ore node by its box drew it
    out to 1.2737 m against a blocked 0.9148, i.e. 36 cm of rock a player
    walks through. `ci/import_meshy.py --fit-radius` is the mode that solves
    for this one, so the selector has to report the same number the importer
    will hit.
    """
    lo, hi, tris = np.full(3, np.inf), np.full(3, -np.inf), 0
    verts = []
    for m in g["meshes"]:
        for p in m["primitives"]:
            i = p["attributes"]["POSITION"]
            a = g["accessors"][i]
            lo = np.minimum(lo, a["min"])
            hi = np.maximum(hi, a["max"])
            tris += g["accessors"][p["indices"]]["count"] // 3
            bv = g["bufferViews"][a["bufferView"]]
            st = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
            n = a["count"]
            verts.append(np.frombuffer(blob[st:st + n * 12], dtype="<f4").reshape(n, 3))
    v = np.concatenate(verts)
    cx, cz = (lo[0] + hi[0]) / 2, (lo[2] + hi[2]) / 2
    rmax = float(np.hypot(v[:, 0] - cx, v[:, 2] - cz).max())
    return hi - lo, tris, rmax


def albedo(g, blob):
    """Linear luma and green-dominant share of the base colour map."""
    try:
        from PIL import Image
    except ImportError:
        return None, None
    try:
        ti = g["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"]
        im = g["images"][g["textures"][ti]["source"]]
        bv = g["bufferViews"][im["bufferView"]]
        start = bv.get("byteOffset", 0)
        raw = blob[start:start + bv["byteLength"]]
        a = np.asarray(Image.open(io.BytesIO(raw)).convert("RGB"), dtype=np.float64) / 255.0
    except Exception:
        return None, None
    lin = np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)
    luma = float((0.2126 * lin[..., 0] + 0.7152 * lin[..., 1] + 0.0722 * lin[..., 2]).mean())
    green = float(((a[..., 1] > a[..., 0] + 0.03) & (a[..., 1] > a[..., 2] + 0.03)).mean())
    return luma, green


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("glb", nargs="+")
    ap.add_argument("--occupant", help="read the target from sim-core, e.g. Rock")
    ap.add_argument("--size", nargs=3, type=float, metavar=("W", "H", "D"))
    ap.add_argument("--tris", type=int, default=3000)
    a = ap.parse_args()

    if a.occupant:
        r, top = sim_volume(a.occupant)
        half = top - sim_lift(a.occupant)
        target = np.array([2 * r, 2 * half, 2 * r])
        where = f"sim-core: {a.occupant} r={r:.4f} top={top:.4f} lift={sim_lift(a.occupant)}"
    elif a.size:
        target = np.array(a.size)
        where = "--size"
    else:
        raise SystemExit("need --occupant or --size")

    print(f"target {target[0]:.3f} x {target[1]:.3f} x {target[2]:.3f} m   ({where})")
    keep = 0
    print(f"{'file':16s} {'tris':>7s} {'d/w':>6s} {'aspect':>7s} {'luma':>6s} "
          f"{'green':>6s} {'MB':>5s}  verdict")
    for path in a.glb:
        g, blob, nbytes = read_glb(path)
        size, tris, rmax = geometry(g, blob)
        # The importer's radial mode: one shared X/Z factor solving for the
        # blocked radius, Y for the blocked height.
        kxz = (target[0] / 2.0) / max(rmax, 1e-9)
        ky = target[1] / max(size[1], 1e-9)
        aspect = max(kxz, ky) / min(kxz, ky)
        dw = float(size[2] / size[0])
        luma, green = albedo(g, blob)
        bad = []
        if tris > a.tris:
            bad.append(f"tris>{a.tris}")
        if aspect > ASPECT_MAX:
            bad.append(f"aspect>{ASPECT_MAX}")
        if not DEPTH_BAND[0] <= dw <= DEPTH_BAND[1]:
            bad.append(f"depth/width {dw:.2f} outside {DEPTH_BAND}")
        if luma is not None:
            if not ALBEDO_LUMA_BAND[0] <= luma <= ALBEDO_LUMA_BAND[1]:
                bad.append("luma out of band")
            if green > GREEN_MAX:
                bad.append(f"green>{GREEN_MAX:.0%}")
        verdict = "KEEP" if not bad else "reject: " + ", ".join(bad)
        keep += 1 if not bad else 0
        ls = f"{luma:6.3f}" if luma is not None else "     ?"
        gs = f"{green * 100:5.1f}%" if green is not None else "     ?"
        print(f"{os.path.basename(path)[:16]:16s} {tris:7d} {dw:6.3f} {aspect:6.2f}x "
              f"{ls} {gs} {nbytes / 1e6:5.1f}  {verdict}")
    # **Nonzero only when NOTHING is keepable**, which is what the docstring
    # promised and the first draft did not do: it exited 1 on any reject,
    # and rejects are the expected output of a triage step over a sampling
    # process. A batch script loops on this, so "some rolls were bad" must
    # not read as "the batch failed".
    print(f"{keep} of {len(a.glb)} keepable")
    sys.exit(0 if keep else 1)


if __name__ == "__main__":
    main()
