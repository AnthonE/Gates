/**
 * Ground clutter — the sub-metre population that answers `ART.md` rule 4.
 *
 * `ART.md` §1: "The ground is not a surface, it is a population… this is the
 * single largest structural difference between our frames and the references,
 * and no shader fixes it." Rule 4 makes it a number: no visible ground patch
 * larger than ~3 m² inside 15 m without scatter. Until this file the client
 * had no ground geometry at all below the 8 m scatter grid — turf was a
 * shaded plane, and two adjacent scatter cells at full occupancy still leave
 * ~60 m² of bare ground between their two props.
 *
 * WHERE THE DECISIONS ARE. Almost none of them are here. Placement, kind,
 * jitter, yaw, scale and the coverage guarantee all live in
 * `crates/sim-core/src/terrain.rs` (`clutter_cell`), are gated as arithmetic
 * by `crates/sim-core/tests/clutter.rs`, and arrive through
 * `terrain_fill_clutter`. This file draws what worldgen decided — the same
 * division `props.js` already has with the scatter grid. What IS decided here
 * is geometry and streaming, and nothing else.
 *
 * THREE CHOICES WORTH THE INK, each measured against this box rather than
 * taste:
 *
 *  1. SOLID TAPERED BLADES, NOT ALPHA CARDS. The usual instanced-grass build
 *     is a crossed card with an alpha-tested blade texture — 4 tris and a
 *     `discard`. This box renders through SwiftShader with no GPU path, so it
 *     is fill-bound long before it is vertex-bound, and `discard` is the one
 *     thing that defeats early-Z. Solid geometry at 6 tris a tuft costs two
 *     more triangles and no fill, needs no texture, and cannot read as a
 *     black cutout. (It also keeps materials parked, which is the operator's
 *     2026-08-04 call for this lane.)
 *  2. NO NEW SHADER AND NO NEW MATERIAL. Every kind takes an existing
 *     `surfaceMaterial` identity, so three's program cache is keyed the same
 *     way it already is and this adds zero programs — the prewarm gate counts
 *     program links after `inWorld`, and a lazily-linked grass program is
 *     exactly the 700 ms worst-frame `CLAUDE.md`'s trap list names. The wind
 *     injection, the prop field and the shadow path all come along for free.
 *  3. GRASS CASTS NOTHING. `castShadow = false` on every pool, so clutter is
 *     drawn ONCE rather than once per shadow level. That is a 3× triangle
 *     saving against a budget `DESIGN.md` §9 already has at ~70% spent, and
 *     it costs nothing that reads at this size — a 34 cm tuft's own shadow is
 *     sub-texel in every cascade. It still RECEIVES, which is the half that
 *     matters: grass standing in a pine's shadow has to go dark with it or
 *     the population reads as pasted on.
 *
 * NOT DONE HERE, deliberately, and named so the next pass does not have to
 * rediscover it: there is no distance fade, so the population ends at the
 * tile ring's edge rather than thinning into it. The recipe (thin
 * stochastically by instance hash, then scale survivors to zero) needs a
 * per-frame, player-relative term, and the cheap way to get one is a shader
 * patch — a new program, on the gate named in choice 2 above. Whether the
 * edge actually reads at 32-48 m is a question for the visual judge, not for
 * a guess made here.
 */

import * as THREE from "three";
import { bakedGeometry } from "./props.js";

// Geometry and numbers only — this file imports THREE and `props.js`, and
// nothing else, for the same reason `props.js` does: `ci/clutter_shape.mjs`
// runs it in bare node, and `materials.js` reaches for texture files node
// cannot load. The pools, the materials and the streaming live beside it in
// `clutterField.js`.

// ── The cross-language coupling ────────────────────────────────────────────
//
// These three mirror `crates/sim-core/src/terrain.rs` and are held equal to it
// by `ci/clutter_shape.mjs`, which reads the Rust source. Same arrangement as
// `PINE_MAX_R` against `SPAWN_CLEAR_M`: a comment saying "keep these in sync"
// is not a gate, and this seam has a fill buffer on the far side of it.

/** Tile edge in meters — `terrain::CLUTTER_TILE_M`. */
export const CLUTTER_TILE_M = 16.0;
/** The coverage stratum, one per cell — `terrain::CLUTTER_BASE_PER_TILE`. */
export const CLUTTER_BASE_PER_TILE = 625;
/**
 * The richness stratum's budget — `terrain::CLUTTER_RICH_PER_TILE`.
 *
 * A second element on cells whose ground is rich enough to have earned one:
 * grass and forest litter thicken, sand and rock do not. Same four kinds
 * through the same four pools, arriving in the same buffer, so like the
 * skirts it costs no material, no program and no draw call — only pool.
 */
export const CLUTTER_RICH_PER_TILE = 96;
/** Max grid elements one tile yields — `terrain::CLUTTER_PER_TILE`. */
export const CLUTTER_PER_TILE = CLUTTER_BASE_PER_TILE + CLUTTER_RICH_PER_TILE;
/**
 * Max PROP-SKIRT elements one tile yields — `terrain::SKIRT_PER_TILE`.
 *
 * The skirts are the same four kinds through the same four pools, arriving in
 * the same `terrain_fill_clutter` buffer behind the grid. Nothing downstream
 * distinguishes them, which is the whole reason they cost no draw call.
 */
export const SKIRT_PER_TILE = 256;
/** Everything one tile can yield — `terrain::CLUTTER_TILE_CAP`. */
export const CLUTTER_TILE_CAP = CLUTTER_PER_TILE + SKIRT_PER_TILE;
/** Floats per element in the bridge buffer — `bridge::CLUTTER_FLOATS`. */
export const CLUTTER_FLOATS = 6;

// ── Client-side knobs (DECISIONS.md §open, ground clutter v0) ──────────────

/**
 * Tiles loaded each side of the player's own, so the ring is
 * `(2 * CLUTTER_RING + 1)²` tiles. 2 puts the nearest edge at 32 m and the
 * far corner at ~45 m; `ART.md` rule 4 only asks for 15 m, and the margin is
 * spent on the horizon rather than on the rule.
 */
export const CLUTTER_RING = 2;
/**
 * Tile fills per frame. One, for the same reason the terrain ring builds one
 * chunk per frame: a fill is 625 cells × ~6 height taps through the wasm
 * boundary, and 25 of them in one frame is the stream-in spike `CLAUDE.md`
 * warns is the half everyone forgets. At 1/frame a full ring lands in 25
 * frames and a walk across a tile boundary refills at most 5.
 */
export const CLUTTER_FILLS_PER_FRAME = 1;

/** Instances one kind's pool can hold: the whole ring, if it were uniform. */
export const CLUTTER_POOL_CAP =
  CLUTTER_TILE_CAP * (2 * CLUTTER_RING + 1) * (2 * CLUTTER_RING + 1);

// ── Geometry ───────────────────────────────────────────────────────────────

/**
 * A blade: a tapered quad, 4 corners, 2 triangles, leaning outward along
 * `dir` as it rises. Written non-indexed because `bakedGeometry` wants it
 * that way and an indexed 4-vert quad would be expanded anyway.
 *
 * The normal is the load-bearing part. A blade's true face normal is
 * horizontal, and a horizontal normal under a key light 21° above the horizon
 * reads black from most of the compass — the vegetation pack's named failure
 * ("leaves reveal flat card normals under rotation"). So the shipped normal
 * is the pack's rounded form: the outward direction pushed toward world up,
 * which fans a tuft's three blades into something closer to a hemisphere and
 * lights like the ground it stands in rather than like a wall.
 */
function blade(dir, height, halfBase, halfTip, lean) {
  const [dx, dz] = dir;
  // Across the blade's width, perpendicular to its lean.
  const px = -dz;
  const pz = dx;
  const tipX = dx * lean;
  const tipZ = dz * lean;
  const corners = [
    [-px * halfBase, 0, -pz * halfBase],
    [px * halfBase, 0, pz * halfBase],
    [tipX + px * halfTip, height, tipZ + pz * halfTip],
    [tipX - px * halfTip, height, tipZ - pz * halfTip],
  ];
  // BOTH windings. A blade is a plane with no thickness, and the materials
  // here are `FrontSide` — the props' own setting, kept because changing it
  // would mean a second material configuration and the program-count gate is
  // the reason this file introduces none. One winding would make a tuft lose
  // blades as the camera walked around it. Two costs two triangles and is
  // correct from everywhere.
  const tris = [
    [0, 1, 2],
    [0, 2, 3],
    [2, 1, 0],
    [3, 2, 0],
  ];
  const pos = new Float32Array(tris.length * 9);
  const nrm = new Float32Array(tris.length * 9);
  // The rounded normal, and the SAME one on both windings — deliberately not
  // the geometric face normal, which for a blade is horizontal and would read
  // black from most of the compass under a key light 21° above the horizon
  // (the vegetation pack's "leaves reveal flat card normals under rotation").
  // Outward, pushed toward world up, so a tuft's three blades fan into
  // something closer to a hemisphere and light like the ground they stand in.
  // Giving the back winding the mirrored normal would just move the black
  // side rather than remove it.
  const nx = dx * 0.62;
  const nz = dz * 0.62;
  const nl = Math.hypot(nx, 1, nz);
  let o = 0;
  for (const t of tris) {
    for (const c of t) {
      pos[o] = corners[c][0];
      pos[o + 1] = corners[c][1];
      pos[o + 2] = corners[c][2];
      nrm[o] = nx / nl;
      nrm[o + 1] = 1 / nl;
      nrm[o + 2] = nz / nl;
      o += 3;
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  g.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
  return g;
}

/** One triangle with its own flat normal, appended to a position/normal pair. */
function pushTri(pos, nrm, a, b, c) {
  const ux = b[0] - a[0];
  const uy = b[1] - a[1];
  const uz = b[2] - a[2];
  const vx = c[0] - a[0];
  const vy = c[1] - a[1];
  const vz = c[2] - a[2];
  let nx = uy * vz - uz * vy;
  let ny = uz * vx - ux * vz;
  let nz = ux * vy - uy * vx;
  const l = Math.hypot(nx, ny, nz) || 1;
  nx /= l;
  ny /= l;
  nz /= l;
  for (const p of [a, b, c]) {
    pos.push(p[0], p[1], p[2]);
    nrm.push(nx, ny, nz);
  }
}

/** A cone of `sides` triangles plus its base fan — the pebble/shard body. */
function chip(sides, radius, height, tilt) {
  const pos = [];
  const nrm = [];
  const ring = [];
  for (let i = 0; i < sides; i++) {
    const a = (i / sides) * Math.PI * 2;
    ring.push([Math.cos(a) * radius, 0, Math.sin(a) * radius]);
  }
  const apex = [tilt * radius, height, 0];
  for (let i = 0; i < sides; i++) {
    pushTri(pos, nrm, ring[i], apex, ring[(i + 1) % sides]);
  }
  // The base, so a chip seen from downhill is a stone and not a hole. Its
  // normal points straight down, which is what a closed solid's underside is
  // for — never lit, never seen from above, and culled from there anyway.
  for (let i = 1; i < sides - 1; i++) {
    pushTri(pos, nrm, ring[0], ring[i], ring[i + 1]);
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute("normal", new THREE.BufferAttribute(new Float32Array(nrm), 3));
  return g;
}

const TUFT_H = 0.34;

/** Three blades at 120°, splayed — the grass channel's element. */
function tuftGeometry() {
  const parts = [];
  for (let i = 0; i < 3; i++) {
    const a = (i / 3) * Math.PI * 2;
    parts.push({
      geo: blade([Math.cos(a), Math.sin(a)], TUFT_H, 0.028, 0.006, 0.1),
      lo: 0x3c5a24,
      hi: 0x6e8f3c,
      y0: 0,
      y1: TUFT_H,
    });
  }
  return bakedGeometry(parts, { y0: 0, y1: TUFT_H });
}

/** A water-worn stone: four sides, low and blunt. Sand channel. */
function pebbleGeometry() {
  return bakedGeometry(
    [{ geo: chip(4, 0.075, 0.05, 0.25), lo: 0x8a8071, hi: 0xa89c8a, y0: 0, y1: 0.05 }],
    null,
  );
}

/** Angular scree off the cliff mask: three sides, taller, sharper. */
function shardGeometry() {
  return bakedGeometry(
    [{ geo: chip(3, 0.09, 0.115, 0.4), lo: 0x6d6a66, hi: 0x8e8b85, y0: 0, y1: 0.115 }],
    null,
  );
}

/**
 * A fallen stick: a long thin quad lying almost flat, one end propped a
 * centimetre higher than the other so it reads as resting on uneven ground
 * rather than decalled onto it.
 *
 * Built here rather than as a rotated `blade` — that was the first cut, and
 * laying a standing blade down carries its rounded normal down with it, so
 * the stick lit from underneath. Litter also gets NO wind weight: a twig that
 * swayed would be the one thing in the frame that is obviously wrong.
 */
function twigGeometry() {
  const L = 0.115;
  const W = 0.016;
  const lo = 0.008;
  const hi = 0.03;
  const c = [
    [-L, lo, -W],
    [L, hi, -W],
    [L, hi, W],
    [-L, lo, W],
  ];
  const pos = [];
  const nrm = [];
  pushTri(pos, nrm, c[0], c[2], c[1]);
  pushTri(pos, nrm, c[0], c[3], c[2]);
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute("normal", new THREE.BufferAttribute(new Float32Array(nrm), 3));
  return bakedGeometry([{ geo: g, lo: 0x4a3b28, hi: 0x6b5638, y0: lo, y1: hi }], null);
}

/**
 * Indexed by `terrain::Clutter` minus one — the sim's own enum order
 * (Pebble, Tuft, Twig, Shard), which is also the splat's channel order
 * (sand, grass, litter, rock). The kind a cell drew IS the ground identity
 * under it; see `terrain.rs`'s clutter note.
 */
export const CLUTTER_ROWS = [
  { name: "pebble", geo: pebbleGeometry, surface: "rock", tint: 0.1 },
  { name: "tuft", geo: tuftGeometry, surface: "foliage", tint: 0.16, wind: true },
  { name: "twig", geo: twigGeometry, surface: "wood", tint: 0.12 },
  { name: "shard", geo: shardGeometry, surface: "rock", tint: 0.12 },
];
