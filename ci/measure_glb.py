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

**Three more, added 2026-09-05, because the six keepers above were all
inside every band and two of them were the wrong OBJECT** (operator: the
boulder "looks like mineable", the node is "square instead of spherical"):

  * **plan ratio** and **radius spread** (`glbcharts.plan_ratio`,
    `radius_spread`) -- the widest over the narrowest direction of the
    footprint, and how far the surface is from a sphere. Depth/width is
    blind to both: a ball is d/w 1.0 and so is a cube. Measured on the
    shipped set, the boulder in the operator's screenshot is a 1.17 ball
    and the stone node a 1.39 cube (1.0 is a circle, 1.39 is what 36 bins
    make of a square). The two rows take OPPOSITE bands, because the sim
    says a boulder is never a node and the look is the only thing telling a
    player which rock to swing at: a node must be ROUND in plan (which is
    also what its cylinder collision assumes -- a square node's faces sit
    0.26 m inside the blocked radius, an invisible skirt the max-radius
    gate cannot see), a formation must NOT be a ball.
  * **per-row luma** -- the stone node is the PALE rock and the formation
    the DARK one, split at the ground's granite identity. The shipped
    boulder measured 0.344 against the node's 0.303, i.e. the two classes
    were the wrong way round, and value was the second thing saying "node".
  * **chart contrast** (`glbcharts.chart_contrast`) -- how much the UV
    islands of the albedo disagree with each other. The generator bakes
    each island from a different view under a different light, and the
    seams follow polygon edges exactly: the "camouflage" patches. Decoded
    off the shipped KTX2 (2026-09-05): 0.139 on the boulder, 0.147, 0.089
    and 0.309 on the node -- and every one of them was inside every OTHER
    band, because the defect is in the differences. The ceiling is
    provisional (`DECISIONS.md` §open, scatter art v1); a natural stain
    reads as a few hundredths. `ci/flatten_charts.py` is the repair.

`--self-test` proves the bands on synthetic meshes (a sphere, a cube, a
slab, a patchy albedo) so a band that stops rejecting is red here rather
than discovered on the next purchase.

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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glbcharts as gc  # noqa: E402

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

# ── Shape and value, per ROW (2026-09-05) ─────────────────────────────────
#
# The two classes must differ in shape, value and attitude (`ART.md` rule 8).
# A node is round in plan -- a circle is 1.0, a square 1.39 in 36 bins -- and
# a formation is anything but a ball: not round in plan, OR spread enough
# that the surface is nowhere near a sphere. Measured on the shipped set the
# bands split exactly along the defect: rock_a 1.17/0.12 (reject: the ball
# in the screenshot), rock_b 1.44/0.20 and rock_c 1.37/0.10 (keep), the
# three nodes 1.39 / 1.39 / 1.75 (all reject: quarried blocks).
NODE_ROWS = ("StoneNode", "MetalNode", "SulfurNode")
FORMATION_ROWS = ("Rock",)
PLAN_ROUND_MAX = 1.20
PLAN_ANGULAR_MIN = 1.30
SPREAD_ANGULAR_MIN = 0.20
# The stone node is the pale rock and the formation is the dark one, split
# at granite. Metal and sulfur carry their identity in colour and glint, not
# value, so they take only the general band above.
PALE_ROWS = ("StoneNode",)
DARK_ROWS = ("Rock",)
# UV islands of the albedo disagreeing with each other. Shipped: 0.089 to
# 0.309, every one a patchwork on screen. Provisional -- see the docstring.
CHART_CONTRAST_MAX = 0.06


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
    """Linear luma, green-dominant share, and the albedo's chart contrast.

    `(None, None, None)` when the map cannot be decoded here -- no PIL, or a
    KTX2 already packed (`ci/ktx_pack.py` runs AFTER this step; a packed map
    is measured by `crates/client/tests/packed_maps.rs` instead).
    """
    try:
        from PIL import Image
    except ImportError:
        return None, None, None
    try:
        ti = g["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"]
        im = g["images"][g["textures"][ti]["source"]]
        bv = g["bufferViews"][im["bufferView"]]
        start = bv.get("byteOffset", 0)
        raw = blob[start:start + bv["byteLength"]]
        a = np.asarray(Image.open(io.BytesIO(raw)).convert("RGB"), dtype=np.float64) / 255.0
    except Exception:
        return None, None, None
    lin = gc.srgb_to_linear(a)
    lum = gc.luma(lin)
    luma = float(lum.mean())
    green = float(((a[..., 1] > a[..., 0] + 0.03) & (a[..., 1] > a[..., 2] + 0.03)).mean())
    pos, uv, idx = gc.primitive(g, blob)
    tri_chart, n = gc.charts(idx, len(pos))
    chart_map = gc.rasterize(uv, idx, tri_chart, a.shape[1], a.shape[0])
    contrast, _, _ = gc.chart_contrast(chart_map, lum, n)
    return luma, green, contrast


def shape(g, blob):
    """Plan ratio and radius spread of the first primitive."""
    pos, _, idx = gc.primitive(g, blob)
    return gc.plan_ratio(pos, idx), gc.radius_spread(pos, idx)


def verdict(occ, tris, tri_cap, dw, aspect, plan, spread, luma, green, contrast):
    """Every reason a roll is rejected, as a list; empty is KEEP.

    One function rather than a block in `main`, so `--self-test` can drive
    it against synthetic meshes with a known shape.
    """
    bad = []
    if tris > tri_cap:
        bad.append(f"tris>{tri_cap}")
    if aspect > ASPECT_MAX:
        bad.append(f"aspect>{ASPECT_MAX}")
    if not DEPTH_BAND[0] <= dw <= DEPTH_BAND[1]:
        bad.append(f"depth/width {dw:.2f} outside {DEPTH_BAND}")
    if occ in NODE_ROWS and plan > PLAN_ROUND_MAX:
        bad.append(f"plan {plan:.2f}>{PLAN_ROUND_MAX}: not round, reads as blocks and leaves a skirt")
    if occ in FORMATION_ROWS and plan < PLAN_ANGULAR_MIN and spread < SPREAD_ANGULAR_MIN:
        bad.append(f"plan {plan:.2f}<{PLAN_ANGULAR_MIN} and spread {spread:.2f}<{SPREAD_ANGULAR_MIN}: a ball, reads as a node")
    if luma is not None:
        if not ALBEDO_LUMA_BAND[0] <= luma <= ALBEDO_LUMA_BAND[1]:
            bad.append("luma out of band")
        if occ in PALE_ROWS and luma < GRANITE_LUMA:
            bad.append(f"luma {luma:.3f}<{GRANITE_LUMA}: the node is the pale rock")
        if occ in DARK_ROWS and luma > GRANITE_LUMA:
            bad.append(f"luma {luma:.3f}>{GRANITE_LUMA}: the formation is the dark rock")
        if green > GREEN_MAX:
            bad.append(f"green>{GREEN_MAX:.0%}")
    if contrast is not None and contrast > CHART_CONTRAST_MAX:
        bad.append(f"chart contrast {contrast:.3f}>{CHART_CONTRAST_MAX}: patchwork, flatten_charts.py")
    return bad


def self_test():
    """The bands, proven on meshes whose shape is known by construction."""
    import tempfile
    cases = []

    def run(name, raw, occ, expect_bad):
        g, blob = gc.parse_glb(raw)
        size, tris, rmax = geometry(g, blob)
        plan, spread = shape(g, blob)
        luma, green, contrast = albedo(g, blob)
        bad = verdict(occ, tris, 3000, float(size[2] / size[0]), 1.0, plan, spread, luma, green, contrast)
        hit = [b for b in bad if any(k in b for k in expect_bad)] if expect_bad else []
        ok = (not bad) if not expect_bad else (len(hit) == len(expect_bad) and len(bad) == len(expect_bad))
        cases.append(ok)
        print(f"  {'ok ' if ok else 'BAD'} {name:34s} plan={plan:.3f} spread={spread:.3f} "
              f"luma={luma if luma is None else round(luma, 3)} chart={contrast if contrast is None else round(contrast, 3)} -> "
              f"{bad or 'KEEP'}")

    flat = [1.0] * 8
    patchy = [0.75, 1.3] * 4
    run("sphere as a node keeps", gc.synthetic_glb("sphere", gains=flat), "StoneNode", [])
    run("sphere as a formation is a ball", gc.synthetic_glb("sphere", gains=flat, base_luma=0.2), "Rock", ["a ball"])
    run("cube as a node is blocks", gc.synthetic_glb("cube", gains=[1.0] * 6), "StoneNode", ["not round"])
    run("cube as a formation keeps", gc.synthetic_glb("cube", gains=[1.0] * 6, base_luma=0.2), "Rock", [])
    run("slab is a wafer", gc.synthetic_glb("slab", gains=[1.0] * 6, base_luma=0.2), "Rock", ["depth/width"])
    run("patchy sphere as a node", gc.synthetic_glb("sphere", gains=patchy), "StoneNode", ["chart contrast"])
    run("pale formation", gc.synthetic_glb("cube", gains=[1.0] * 6, base_luma=0.34), "Rock", ["the formation is the dark rock"])
    run("dark node", gc.synthetic_glb("sphere", gains=flat, base_luma=0.2), "StoneNode", ["the node is the pale rock"])
    # The albedo path needs PIL; without it every luma-keyed case above reads
    # as KEEP and the run is worthless, which is the loud-SKIP rule.
    try:
        import PIL  # noqa: F401
    except ImportError:
        print("SKIP: PIL is not installed, the albedo cases did not run")
        sys.exit(2)
    if not all(cases):
        sys.exit("self-test FAILED")
    print(f"self-test: {len(cases)} cases ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("glb", nargs="*")
    ap.add_argument("--occupant", help="read the target from sim-core, e.g. Rock")
    ap.add_argument("--size", nargs=3, type=float, metavar=("W", "H", "D"))
    ap.add_argument("--tris", type=int, default=3000)
    ap.add_argument("--self-test", action="store_true", help="prove the bands on synthetic meshes")
    a = ap.parse_args()
    if a.self_test:
        self_test()
        return
    if not a.glb:
        ap.error("no GLB given")

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
    print(f"{'file':16s} {'tris':>7s} {'d/w':>6s} {'plan':>6s} {'sprd':>6s} {'aspect':>7s} "
          f"{'luma':>6s} {'green':>6s} {'chart':>6s} {'MB':>5s}  verdict")
    for path in a.glb:
        g, blob, nbytes = read_glb(path)
        size, tris, rmax = geometry(g, blob)
        # The importer's radial mode: one shared X/Z factor solving for the
        # blocked radius, Y for the blocked height.
        kxz = (target[0] / 2.0) / max(rmax, 1e-9)
        ky = target[1] / max(size[1], 1e-9)
        aspect = max(kxz, ky) / min(kxz, ky)
        dw = float(size[2] / size[0])
        plan, spread = shape(g, blob)
        luma, green, contrast = albedo(g, blob)
        bad = verdict(a.occupant, tris, a.tris, dw, aspect, plan, spread, luma, green, contrast)
        word = "KEEP" if not bad else "reject: " + ", ".join(bad)
        keep += 1 if not bad else 0
        ls = f"{luma:6.3f}" if luma is not None else "     ?"
        gs = f"{green * 100:5.1f}%" if green is not None else "     ?"
        cs = f"{contrast:6.3f}" if contrast is not None else "     ?"
        print(f"{os.path.basename(path)[:16]:16s} {tris:7d} {dw:6.3f} {plan:6.3f} {spread:6.3f} {aspect:6.2f}x "
              f"{ls} {gs} {cs} {nbytes / 1e6:5.1f}  {word}")
    # **Nonzero only when NOTHING is keepable**, which is what the docstring
    # promised and the first draft did not do: it exited 1 on any reject,
    # and rejects are the expected output of a triage step over a sampling
    # process. A batch script loops on this, so "some rolls were bad" must
    # not read as "the batch failed".
    print(f"{keep} of {len(a.glb)} keepable")
    sys.exit(0 if keep else 1)


if __name__ == "__main__":
    main()
