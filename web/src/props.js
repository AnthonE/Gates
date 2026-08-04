// Prop geometry: everything the scatter pools are built OUT of, and nothing
// about how they are drawn, streamed or lit.
//
// Split out of `terrain.js` when the pine stopped being four primitives. Two
// reasons, and the second is the one that matters:
//
//   * `terrain.js` owns streaming, pools, chunk lifetime and the fell state
//     machine. The shape of a tree is not any of those.
//   * This module imports THREE and nothing else — no materials, no textures,
//     no DOM. That is what lets `ci/pine_shape.mjs` import the SHIPPED builder
//     and measure the actual buffer, instead of scoring a copy of it the way
//     `bump_basis` has to mirror GLSL. A geometry gate that built its own tree
//     would pass forever while the tree changed underneath it.
//
// Everything here is deterministic by construction: the hashes are over the
// vertex's own quantized position and a per-part seed, never over an RNG with
// state, so a chunk that streams out and back is bit-identical and every pine
// in the world is this pine.

import * as THREE from "three";
import { Tree } from "@dgreenheck/ez-tree";

// How the per-vertex wind weight rises with height: `aWind = (y/h)^WIND_CURVE`
// (materials.js does the displacing; this bakes the cantilever). Above 1 the
// trunk is stiffer than linear and the canopy carries the motion, which is the
// shape a conifer actually has.
export const WIND_CURVE = 1.7;

/** The per-vertex wind weight at a normalized height up the plant, 0..1. */
export function windWeight(t) {
  return Math.pow(Math.max(0, Math.min(t, 1)), WIND_CURVE);
}

/**
 * Merge positioned primitives into one non-indexed geometry carrying baked
 * vertex colours, each part ramped between two colours over a y band. This
 * is how an archetype gets more than one colour without costing a second
 * draw call: a pine's trunk and its whorls are one buffer (DECISIONS.md
 * §open, materials v0).
 */
export function bakedGeometry(parts, wind) {
  let total = 0;
  const flat = parts.map((p) => {
    const g = p.geo.index ? p.geo.toNonIndexed() : p.geo;
    if (p.geo.index) p.geo.dispose();
    total += g.attributes.position.count;
    return { g, ...p };
  });
  const pos = new Float32Array(total * 3);
  const nrm = new Float32Array(total * 3);
  const col = new Float32Array(total * 3);
  // The wind cantilever, one weight per vertex (materials.js). Ramped over
  // the WHOLE archetype rather than per part, which is why it is a single
  // `{y0, y1}` here and not a band on each: the pine's whorls overlap in y,
  // and independently ramped parts would put a step in the middle of one
  // tree. Absent `wind` the array stays zero, and an archetype whose weights
  // are all zero is displaced by nothing — the rocks and the ore take the
  // same shader patch and never move.
  const wnd = new Float32Array(total);
  const lo = new THREE.Color();
  const hi = new THREE.Color();
  const c = new THREE.Color();
  let o = 0;
  for (const part of flat) {
    const n = part.g.attributes.position.count;
    const gp = part.g.attributes.position.array;
    pos.set(gp, o * 3);
    nrm.set(part.g.attributes.normal.array, o * 3);
    // setHex with SRGBColorSpace lands in the renderer's working space —
    // the same conversion the sky dome and every material colour gets.
    lo.setHex(part.lo, THREE.SRGBColorSpace);
    hi.setHex(part.hi === undefined ? part.lo : part.hi, THREE.SRGBColorSpace);
    const y0 = part.y0 === undefined ? 0 : part.y0;
    const y1 = part.y1 === undefined ? 1 : part.y1;
    for (let i = 0; i < n; i++) {
      const t = Math.max(0, Math.min((gp[i * 3 + 1] - y0) / (y1 - y0), 1));
      c.copy(lo).lerp(hi, t);
      col[(o + i) * 3] = c.r;
      col[(o + i) * 3 + 1] = c.g;
      col[(o + i) * 3 + 2] = c.b;
      if (wind) {
        wnd[o + i] = windWeight((gp[i * 3 + 1] - wind.y0) / (wind.y1 - wind.y0));
      }
    }
    o += n;
    part.g.dispose();
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geo.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
  geo.setAttribute("color", new THREE.BufferAttribute(col, 3));
  geo.setAttribute("aWind", new THREE.BufferAttribute(wnd, 1));
  return geo;
}

// How far a whorl ring may be pulled IN toward the trunk, as a share of its
// own radius. The visual judge's ranked gap 1(c): "the silhouette is a clean
// polygon edge with no needle cards, no alpha detail, no sub-branch break-up",
// and a blind reader named "flat-shaded cones stacked on a cylinder trunk with
// visible facet edges and no sub-branch silhouette" in 3 of the 3 frames that
// show a tree.
//
// It only ever pulls IN, never out, and that is load-bearing rather than
// tasteful — see PINE_MAX_R.
export const PINE_RAGGED = 0.34;

// …and how far a whorl's rim may hang BELOW its own base plane, in metres.
// The radial pull breaks the plan view; this breaks the elevation. Without it
// every whorl ends in a level disc and five level discs stacked up a trunk are
// five horizontal lines — the exact "clean polygon edge" the report named,
// rotated 90 degrees. The rim droops and the cap centre does not, so the
// underside becomes a shallow bowl rather than a plate, which is also the face
// `ART.md` §5 says every judge catches.
export const PINE_DROOP = 0.18;

/**
 * The radius no part of a pine may exceed, in metres — a CEILING, not the
 * radius it draws.
 *
 * This is the one number in this file that a sim constant depends on.
 * `world.rs` derives `SPAWN_CLEAR_M = 4.0` from "the widest archetype the
 * client draws is the tree cone at radius <= 1.7 m x 1.1 max scale ~= 1.9 m,
 * plus the 0.4 m capsule", so a canopy that grew past this would silently
 * invalidate a spoken sim number FROM THE RENDERER — a fresh spawn standing
 * inside a tree, which is the exact defect `DECISIONS.md`'s beach-spawn row
 * exists to have fixed.
 *
 * Until `ci/pine_shape.mjs` that coupling was a comment in a Rust file about a
 * constant in a JS file, which is the weakest kind of law this repo has. It is
 * a gate now, and the tree it guards draws at 1.38 — the ceiling is slack on
 * purpose, so `ART.md` rule 6's "tall, thin" has somewhere to go without
 * anyone having to re-derive a sim number to get it.
 */
export const PINE_MAX_R = 1.7;

// Nine, and odd on purpose: an even count puts a vertex diametrically opposite
// every other vertex, so the ragged pull reads as a squashed circle instead of
// a whorl. `browser_smoke` asserts DESIGN §9's 1.5 M tris over the main pass
// plus every shadow level; the arithmetic for this count is in
// `ci/pine_shape.mjs`, which prints the fleet cost rather than trusting it.
const PINE_SEGMENTS = 9;

/**
 * The whorl table — where a conifer's silhouette actually comes from.
 *
 * What this replaced: a trunk, one big cone and one small cone, 48 triangles,
 * and a profile that is two straight lines from base to tip. `ART.md` rule 6
 * says it in one sentence — "Pines in the references are tall, thin, and
 * ragged-edged; a smooth cone is wrong at any texture budget" — and no amount
 * of the material work that followed could reach it, because it is a shape
 * problem and material work is not a shape.
 *
 * Conifers put their branches in WHORLS: rings of them at intervals up the
 * trunk, longest low, shortest high, each drooping below its own attachment.
 * That is a stepped profile with five discontinuities, not one straight line —
 * and stacking five short capped cones is the cheapest honest way to get it.
 * Each ring is independently ragged (its own seed) so the whorls do not align
 * into vertical flutes, and each rim droops, so the steps are not level.
 *
 * Radii fall faster than heights, which is what makes it read as tall: the
 * bottom whorl is 1.38 m and the top is 0.55 m over 5.05 m of rise.
 */
const PINE_WHORLS = [
  { y: 1.55, r: 1.38, h: 2.15, seed: 0x51ed270b },
  { y: 2.4, r: 1.19, h: 2.0, seed: 0x2545f491 },
  // Distinct from the mixer's own constants on purpose: seeding a hash with
  // one of the multipliers inside it is how a whorl ends up correlated with
  // the one below it, and a correlated whorl is a vertical flute.
  { y: 3.2, r: 0.99, h: 1.9, seed: 0x1b873593 },
  { y: 4.0, r: 0.77, h: 1.75, seed: 0x7feb352d },
  { y: 4.8, r: 0.55, h: 1.8, seed: 0x846ca68b },
];

/** Overall height: the top whorl's apex. */
const PINE_H = 6.6;

// Where the trunk stops. NOT `PINE_H`: the top whorl tapers to a point and the
// trunk does not, so a trunk run to full height leaves a bare brown spike
// standing above the canopy on every tree in the forest — which is exactly
// what it did in the first render of this shape. At 5.7 m the trunk's 0.13 m
// tip sits inside a cone that is still 0.27 m wide there, so it is covered.
const PINE_TRUNK_H = 5.7;

/**
 * How far a canopy vertex's normal is blended from its own facet toward the
 * canopy volume, 0..1.
 *
 * The defect this exists for is the one that survives every silhouette fix:
 * nine segments to a whorl means nine ENORMOUS flat triangles, each catching
 * the sun as one uniform plate, and a stack of uniform plates is the whole
 * visual signature of an asset-pack tree. Raggedness does not touch it —
 * it is a shading problem, not an outline problem.
 *
 * A needle mass does not have facets; it scatters light as a soft volume. So
 * the canopy's normals are pulled toward "outward from the middle of the
 * canopy", which is what that volume's normals would be, and the nine plates
 * become one shell with a gradient across it. It is free — no extra vertex,
 * no extra triangle, no program — and it is the single cheapest thing in this
 * file per unit of "stops looking authored by a programmer".
 *
 * Caps are exempt. They are the canopy underside, they genuinely are flat
 * downward plates, and `ART.md` §5 wants them reading as such — blending them
 * into the volume would tip the upper whorls' undersides skyward and quietly
 * delete the face every judge catches.
 *
 * The volume direction is OUTWARD AND UP, not "away from the middle of the
 * canopy". The first version used the latter and it is worth recording why it
 * was wrong, because it looks more principled: away-from-centre points every
 * whorl below the canopy's midpoint DOWNWARD, so the bottom two thirds of the
 * tree stopped catching the sun and the whole thing rendered as a black
 * wedding cake. A needle does not lie on a sphere around the canopy — it fans
 * out from the trunk and tilts up toward the light, and that is the same
 * direction at every height.
 */
const PINE_NORMAL_BLEND = 0.7;
/** The upward tilt in that outward direction — see PINE_NORMAL_BLEND. */
const PINE_NORMAL_UP = 0.85;

// The pine's wind band, base of trunk to tip of crown. One ramp for all six
// parts — see `bakedGeometry`.
export const PINE_WIND = { y0: 0, y1: PINE_H };

/** Canopy geometry, for the shape gate to score the trunk against. */
export const PINE_SHAPE = { h: PINE_H, trunkH: PINE_TRUNK_H, whorls: PINE_WHORLS };

/**
 * The pine's authored colour bands, hoisted out of the geometry builder so the
 * albedo law can be asserted over them (`scatterFacts`) instead of over a hex
 * buried in a call argument.
 *
 * Three bands over six parts: the trunk, and `ART.md` §5's "two greens
 * minimum" — a dark lower green the bottom whorls wear and a lit upper green
 * the top three do. Each band ramps over the span of the whorls that wear it
 * rather than over each whorl's own height, so the gradient is continuous
 * across a whorl boundary instead of resetting five times up the tree.
 *
 * Trunk and skirt were rescaled by prop albedo v1 — both sat under
 * `ALBEDO_LUMA_BAND`'s floor, the trunk by 1.9x — and the rescale is a linear
 * multiply of both ends of each ramp, so every hue and every ramp shape here
 * is the one materials v0 authored.
 */
export const PINE_BANDS = [
  // The band stops at 1.9 m, not at the trunk's full height: everything above
  // that is inside the canopy and invisible, and ramping across the whole
  // 6.6 m would spend the visible bare trunk in the dark end of its own ramp.
  { part: "trunk", lo: 0x503b2b, hi: 0x70583f, y0: 0, y1: 1.9 },
  // The skirt's lo is one green level above the exact rescale: 0x204725 is
  // 0.0495 after 8-bit quantization and the band's floor is 0.05, so the
  // authored hex — not the arithmetic that produced it — is what has to clear
  // it. The gate scores the hex.
  { part: "skirt", lo: 0x204825, hi: 0x3c783d, y0: 1.55, y1: 4.4 },
  { part: "crown", lo: 0x2c5c2c, hi: 0x4d8845, y0: 3.2, y1: PINE_H },
];

/**
 * Which band each whorl wears: the low two dark, the top three lit.
 *
 * Exported because it is the fact `ART.md` §5's "two greens minimum" is
 * actually about. Counting distinct baked vertex COLOURS does not catch a
 * table collapsed onto one ramp — a single lo→hi ramp over 5 m of canopy still
 * produces twenty distinct colours, and the gate that only counted those
 * passed the mutation that deleted the second green.
 */
export const PINE_WHORL_BANDS = [1, 1, 2, 2, 2];

/**
 * Pull each ring of a cone in per vertex, and hang its rim below its own base
 * plane, so the outline is a ragged whorl instead of a circle and the bottom
 * edge is a fringe instead of a disc.
 *
 * Deterministic by construction: the hash is over the vertex's own quantized
 * angle and height plus the part's seed, so the two vertices a shared edge is
 * built from get the same pull and the seam cannot open — the same discipline
 * `instanceTint` carries for colour. The apex and the cap centre sit on the
 * axis and are skipped, which is what leaves the drooped cap a shallow bowl.
 */
export function raggedCone(geo, seed) {
  const pos = geo.attributes.position;
  let minY = Infinity;
  for (let i = 0; i < pos.count; i++) minY = Math.min(minY, pos.getY(i));
  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const z = pos.getZ(i);
    const r = Math.hypot(x, z);
    if (r < 1e-4) continue; // the apex and the cap centre stay put
    // A hash over the vertex's quantized angle and height, so the two vertices
    // a shared edge is built from get the same pull and the seam cannot open.
    const th = Math.atan2(z, x);
    const a = Math.round(((th < 0 ? th + Math.PI * 2 : th) / (Math.PI * 2)) * 4096) % 4096;
    const y = Math.round(pos.getY(i) * 512);
    let hh = Math.imul(a ^ (y * 0x9e3779b9) ^ seed, 0x85ebca6b);
    hh ^= hh >>> 13;
    hh = Math.imul(hh, 0xc2b2ae35);
    hh ^= hh >>> 16;
    const k = 1 - ((hh >>> 8) & 0x3ff) / 1023 * PINE_RAGGED;
    pos.setX(i, x * k);
    pos.setZ(i, z * k);
    // The rim, and only the rim: a different slice of the SAME hash, so the
    // side's base vertex and the cap's rim vertex at one angle droop together.
    // Reading a second hash here would tear the cap off the cone.
    if (Math.abs(pos.getY(i) - minY) < 1e-4) {
      pos.setY(i, pos.getY(i) - ((hh >>> 20) & 0x3ff) / 1023 * PINE_DROOP);
    }
  }
  pos.needsUpdate = true;
  geo.computeVertexNormals();
  return geo;
}

/**
 * Blend a canopy part's normals toward the canopy volume — see
 * `PINE_NORMAL_BLEND`. The volume normal is the vertex's own outward direction
 * from the trunk axis, tilted up, so the whole canopy shades as one mass
 * instead of five stacks of plates.
 */
function volumeNormals(geo) {
  const pos = geo.attributes.position;
  const nrm = geo.attributes.normal;
  for (let i = 0; i < pos.count; i++) {
    // The caps stay exactly as computed: they are the underside.
    if (nrm.getY(i) < -0.5) continue;
    const dx = pos.getX(i);
    const dy = PINE_NORMAL_UP * Math.hypot(pos.getX(i), pos.getZ(i));
    const dz = pos.getZ(i);
    const d = Math.hypot(dx, dy, dz);
    if (d < 1e-4) continue;
    const b = PINE_NORMAL_BLEND;
    const nx = nrm.getX(i) * (1 - b) + (dx / d) * b;
    const ny = nrm.getY(i) * (1 - b) + (dy / d) * b;
    const nz = nrm.getZ(i) * (1 - b) + (dz / d) * b;
    const n = Math.hypot(nx, ny, nz) || 1;
    nrm.setXYZ(i, nx / n, ny / n, nz / n);
  }
  nrm.needsUpdate = true;
  return geo;
}

/**
 * A conifer: a bare tapered trunk carrying five ragged, drooping whorls.
 *
 * 102 triangles against the four primitives' 48 — see `PINE_WHORLS` for why
 * that is the shape and `ci/pine_shape.mjs` for what the fleet costs.
 */
export function pineGeometry() {
  // Full height, not just the bare part: the trunk is also the spire the top
  // whorl closes around, and an open-ended cylinder costs the same 12
  // triangles either way.
  const trunk = new THREE.CylinderGeometry(0.13, 0.26, PINE_TRUNK_H, 6, 1, true);
  trunk.translate(0, PINE_TRUNK_H * 0.5, 0);
  const parts = [{ ...PINE_BANDS[0], geo: trunk }];
  for (let i = 0; i < PINE_WHORLS.length; i++) {
    const w = PINE_WHORLS[i];
    const cone = new THREE.ConeGeometry(w.r, w.h, PINE_SEGMENTS);
    // Capped, deliberately. An open cone shows its backfaces from underneath,
    // which is to say nothing at all — and the canopy underside is the face
    // `ART.md` §5 says every judge catches, the one the fill and bounce poles
    // in `scene.js` were tuned against (`03-canopy-up`).
    cone.translate(0, w.y + w.h * 0.5, 0);
    parts.push({
      ...PINE_BANDS[PINE_WHORL_BANDS[i]],
      geo: volumeNormals(raggedCone(cone, w.seed)),
    });
  }
  return bakedGeometry(parts, PINE_WIND);
}

/**
 * What a felled pine leaves behind.
 *
 * `TERRAIN.md` §2 has said "Trees fell to stumps the same way" since before
 * there was a client to draw one. The stump is the world's memory of the last
 * ten swings: it stands for the whole 20–45 minute respawn window and vanishes
 * when the tree comes back, so a cleared ridge still reads as cleared an hour
 * later.
 *
 * Six sides and both caps is 24 triangles, and it only exists where a tree
 * came down, so the pool is bounded by the felled trees inside the near ring
 * rather than by the forest. Its ramp is the pine trunk's own, which puts the
 * pale end of the bark ramp on the cut face — a stump is lightest where the
 * axe left it.
 */
export function stumpGeometry() {
  return bakedGeometry([
    {
      part: "stump",
      geo: new THREE.CylinderGeometry(0.26, 0.32, 0.34, 6),
      lo: PINE_BANDS[0].lo,
      hi: PINE_BANDS[0].hi,
      y0: -0.17,
      y1: 0.17,
    },
  ]);
}

// Scatter archetypes, indexed by sim-core Occupant (1..7). `surface` names
// an authored PBR response in materials.js; `tint` is the amplitude of the
// deterministic per-instance colour variation (0 = every instance alike).
export const ARCHETYPES = [
  null,
  { geo: pineGeometry, composite: true, wind: PINE_WIND, surface: "foliage", lo: 0xffffff, lift: 0, tint: 0.17 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "rock", lo: 0x8f9399, lift: 0.5, tint: 0.11 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xa1785c, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xbfae4a, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.IcosahedronGeometry(0.7), surface: "foliage", lo: 0x2c5f2e, hi: 0x4b8a3f, y0: -0.7, y1: 0.7, wind: { y0: -0.7, y1: 0.7 }, lift: 0.45, tint: 0.19 },
  { geo: () => new THREE.DodecahedronGeometry(1.5), surface: "rock", lo: 0x75726d, lift: 0.55, tint: 0.12 },
  { geo: () => new THREE.CylinderGeometry(0.45, 0.45, 0.95, 10), surface: "metal", lo: 0x5e6b78, lift: 0.5, tint: 0.08 },
  // 8: the stump. The only archetype sim-core does not scatter — it is not a
  // slot occupant but the CONSEQUENCE of one, minted when a tree comes down
  // and retired when the slot respawns. It rides every other path a scatter
  // entry does (chunk lists, swap-remove on stream-out, the tint), so the
  // index has to be a real archetype rather than a special case bolted to the
  // side; `_addScatter` only ever reads occupants 1..7 out of the slot buffer,
  // so 8 cannot collide with one.
  { geo: stumpGeometry, composite: true, surface: "wood", lo: PINE_BANDS[0].lo, hi: PINE_BANDS[0].hi, lift: 0.17, tint: 0.11 },
  // 9: the haven pad's container (sim-core `Occupant::CrateSlot`). The enum
  // skips 8 rather than this table shifting to make room — the stump index is
  // read from five places in terrain.js and the alignment has to hold without
  // anyone remembering it does. Squarer and taller than the road barrel at
  // index 7 so the destination reads different from the route at silhouette
  // distance, which is the only range at which either is legible.
  { geo: () => new THREE.BoxGeometry(1.1, 0.8, 0.8), surface: "wood", lo: 0x6b5334, hi: 0x8a6d47, y0: -0.4, y1: 0.4, lift: 0.4, tint: 0.09 },
];

/** Client-only archetype index for a felled pine's stump. */
export const ARCH_STUMP = 8;
/** The archetype a fell animation applies to (sim-core `Occupant::Tree`). */
export const ARCH_TREE = 1;

/**
 * Every authored colour band an archetype bakes, as `{part, lo, hi}`.
 *
 * The pine is the only archetype built from more than one band, and its bands
 * live in `PINE_BANDS` because the geometry builder and the albedo gate both
 * need them. Everything else carries its ramp on the archetype row itself.
 */
export function albedoParts(k) {
  const a = ARCHETYPES[k];
  if (!a) return [];
  return a.geo === pineGeometry ? PINE_BANDS : [{ part: a.surface, lo: a.lo, hi: a.hi }];
}

// --- the real pine ----------------------------------------------------------
//
// Everything above this line builds a pine out of cones, and three passes of
// that proved the thing the reference says in one glance: a conifer's canopy is
// made of ALPHA CARDS, and an opaque hull with a polygon edge cannot get there
// from any amount of geometry (`DECISIONS.md` §open, "the prop bench").
//
// So the tree is generated by `ez-tree` (MIT, © Daniel Greenheck, npm
// `@dgreenheck/ez-tree`) — a dependency, not a reference. The cone builder
// stays: it is the LOD1 mesh's starting point and `ci/pine_shape.mjs` still
// scores it, but nothing draws it in the near ring any more.
//
// Four things had to be settled to make a 59,616-triangle 55-metre library
// tree into something a 416-tree ring can afford, and all four are measured
// rather than argued (`ci/pine_shape.mjs` prints every one):
//
//   * HEIGHT is a parameter. `branch.length[0] = 50` is what makes it 55 m;
//     the geometry is scaled to `PINE_H` on the way out, so nothing downstream
//     — the wind band, the fell pivot, `PINE_MAX_R` — has to know.
//   * WIDTH is the sim's business. The scaled tree is measured against
//     `PINE_MAX_R` and the gate fails if a generated canopy exceeds it, which
//     is the same wall the cones lived under and now guards a generator whose
//     output nobody authored by hand.
//   * DETAIL is a dial, and the dial has a floor. Cutting first-level branches
//     does not make a smaller tree, it makes a BALD one — measured at 1,136
//     and 568 triangles, both of which read as dead sticks. Fullness lives in
//     branch count and branch count is most of the cost, so the near tree is
//     dialled to ~5 k and everything past the near ring is a billboard, not a
//     thinner mesh.
//   * VARIATION cannot come from the instance. Yaw, scale and tint are all an
//     instance has, and 400 copies of one silhouette read as 400 copies. Three
//     seeds, assigned by cell hash, is three geometries and three extra draw
//     calls against DESIGN §9's 300.

/**
 * Distinct generated pines. Each is a geometry pair and two draw calls.
 *
 * ONE for v0. A generated conifer is not rotationally symmetric the way a cone
 * is, so yaw alone buys variation the old pool could not have — but 400 copies
 * of one silhouette will read as 400 copies eventually, and the second variant
 * is four draw calls against DESIGN §9's 300 rather than a design problem.
 */
export const PINE_VARIANTS = 1;

/**
 * The dial-down, against `Pine Medium` as shipped. Measured in the bench:
 * 59,616 tris at the preset, 7,104 at "hero" (46/18/double), 2,976 at "near"
 * (30/12/double). This sits between them — the last setting where the canopy
 * still closes.
 */
const PINE_EZ = {
  children0: 40,
  sections0: 8,
  segments0: 6,
  sections1: 7,
  segments1: 4,
  // Branch length and leaf size are what set the canopy's WIDTH, and width is
  // the sim's business: the preset lands at 2.22 m against `PINE_MAX_R`'s
  // 1.7 m ceiling. Swept in the bench — 23.87/1.9 -> 2.22 m, 19/1.7 -> 1.81,
  // 17/1.6 -> 1.64, 15/1.5 -> 1.47. The last has margin for seed variance,
  // which a generator needs and a hand-authored cone did not.
  length1: 13.2,
  // 26, not the preset's 30 and not the 15 the first wiring shipped: at 15 the
  // bottom half of every tree was bare trunk, which is the one thing the
  // reference forest never is. Leaf count is the knob that actually spends the
  // budget — every unit is ~160 triangles.
  leafCount: 26,
  leafSize: 1.45,
  // …and the first branch whorl starts lower. The preset hangs it at 27% of
  // trunk height, which on a 6.6 m tree is 1.8 m — above eye level, so a
  // player walking a forest sees a colonnade of poles.
  branchStart: 0.16,
  // The preset's trunk is 1.05 radius on a 55 m tree, which normalizes to a
  // 0.13 m sapling. The reference's trunk fills a frame at melee range.
  trunkRadius: 2.3,
};

/** Bake a height-ramped `aWind` onto a generated part, rooted at the base. */
function windAttribute(geo, y0, y1) {
  const pos = geo.attributes.position;
  const w = new Float32Array(pos.count);
  for (let i = 0; i < pos.count; i++) {
    w[i] = windWeight((pos.getY(i) - y0) / (y1 - y0));
  }
  geo.setAttribute("aWind", new THREE.BufferAttribute(w, 1));
  return geo;
}

/**
 * Generate one pine: `{ bark, leaf, textures }`, both geometries non-indexed,
 * scaled to `PINE_H`, carrying `aWind`.
 *
 * The textures come off the meshes ez-tree already built rather than through
 * its texture API — same objects, one less import, and it keeps this file's
 * dependency surface to the `Tree` class alone.
 */
export function pineTree(variant) {
  const t = new Tree();
  t.loadPreset("Pine Medium");
  const o = t.options;
  // Fixed per variant, never random: every client generates the same forest,
  // and a chunk that streams out and back is bit-identical (the same law the
  // cone builder's hashes carry).
  o.seed = 1013904223 + variant * 2654435761;
  o.branch.children[0] = PINE_EZ.children0;
  o.branch.sections[0] = PINE_EZ.sections0;
  o.branch.segments[0] = PINE_EZ.segments0;
  o.branch.sections[1] = PINE_EZ.sections1;
  o.branch.length[1] = PINE_EZ.length1;
  o.branch.start[1] = PINE_EZ.branchStart;
  o.branch.segments[1] = PINE_EZ.segments1;
  o.branch.radius[0] = PINE_EZ.trunkRadius;
  o.leaves.count = PINE_EZ.leafCount;
  o.leaves.size = PINE_EZ.leafSize;
  o.leaves.billboard = "double";
  t.generate();

  const bark = t.branchesMesh.geometry;
  const leaf = t.leavesMesh.geometry;
  const box = new THREE.Box3().setFromObject(t);
  const k = PINE_H / (box.max.y - box.min.y);
  for (const g of [bark, leaf]) {
    g.scale(k, k, k);
    g.translate(0, -box.min.y * k, 0);
    windAttribute(g, 0, PINE_H);
  }
  return {
    bark,
    leaf,
    barkMaps: {
      map: t.branchesMesh.material.map,
      normalMap: t.branchesMesh.material.normalMap,
      roughnessMap: t.branchesMesh.material.roughnessMap,
      aoMap: t.branchesMesh.material.aoMap,
    },
    leafMaps: {
      map: t.leavesMesh.material.map,
      // Passed through, NOT defaulted. A `|| 0.3` here reads as safe and is
      // the opposite: it silently substitutes a cutoff whenever the library
      // stops providing one, so the canopy could go back to an opaque hull
      // with the gate green. Mutation-tested — the fallback swallowed exactly
      // that mutation until it was removed.
      alphaTest: t.leavesMesh.material.alphaTest,
    },
  };
}

/**
 * The pine as the scatter draws it: one entry per InstancedMesh the archetype
 * needs, sharing one instance-matrix stream (`terrain.js` writes the same
 * matrix to every part of a group).
 *
 * Two parts, because they cannot be one: bark is opaque and leaves are
 * alpha-tested, and an object gets ONE `customDepthMaterial` — so a merged
 * geometry with two material groups would have to alpha-test the trunk against
 * the leaf texture in the shadow pass. Two meshes, two depth materials, two
 * draw calls a variant.
 */
export function pineParts() {
  const t = pineTree(0);
  return [
    { part: "bark", geo: t.bark, surface: "bark", maps: t.barkMaps, tint: 0.1 },
    { part: "needle", geo: t.leaf, surface: "needle", maps: t.leafMaps, tint: 0.17, alpha: true },
  ];
}
