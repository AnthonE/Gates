#!/usr/bin/env node
// Gate: the pine's SHAPE, as arithmetic — including the one number in the
// renderer that a sim constant is derived from.
//
// Why this is countable and not a screenshot. `ART.md` rule 6 is the bar:
// "Silhouette before surface. If a prop is not identifiable as a black shape
// against the sky, no material work will save it. Pines in the references are
// tall, thin, and ragged-edged; a smooth cone is wrong at any texture budget."
// Every word of that is a property of a vertex buffer. It does not need a GPU,
// a shard, a driver or a threshold that drifts with any of them — it needs the
// buffer, and this imports the SHIPPED builder and measures the real one.
//
// That import is the whole design of this file, and it is why `props.js`
// exists as a module that pulls in THREE and nothing else. `bump_basis.mjs`
// has to mirror GLSL in JS and then pin the mirror against the source text,
// because GLSL cannot be run here. Geometry can. A gate that built its own
// tree to score would pass forever while the tree changed underneath it —
// the "gate that matches nothing" failure, which the trap list calls the worst
// bug class in the repo.
//
// The load-bearing assertion is #1, and it is not about art at all.
// `world.rs` derives `SPAWN_CLEAR_M = 4.0` from a sentence about a JS
// constant: "the widest archetype the client draws is the tree cone at radius
// <= 1.7 m x 1.1 max scale ~= 1.9 m, plus the 0.4 m capsule". Nothing enforced
// it. A canopy widened for taste in `props.js` would have put fresh spawns
// back inside trees — the exact defect the beach-spawn row exists to have
// fixed — with every gate in `ci/gates.sh` green, because no gate could see
// across the language boundary. This one reads all four numbers from their own
// files and closes the arithmetic.

import "./dom_shim.mjs";

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  pineGeometry,
  PINE_BANDS,
  PINE_MAX_R,
  PINE_SHAPE,
  PINE_WHORL_BANDS,
  PINE_WIND,
  pineParts,
  PINE_VARIANTS,
} from "../web/src/props.js";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
let checks = 0;
const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};
const check = (cond, msg) => {
  checks++;
  if (!cond) fail(msg);
};

// Pull a `const NAME: ty = value;` out of a Rust source file. Same shape the
// knob registry uses, and for the same reason: a number restated here is a
// number that can drift.
function rustConst(file, name) {
  const src = fs.readFileSync(path.join(ROOT, file), "utf8");
  const m = src.match(new RegExp(`\\bconst\\s+${name}\\s*:\\s*[^=]+=\\s*([^;]+);`));
  if (!m) fail(`${file} has no \`const ${name}\` — this gate reads it by name`);
  const v = Number(m[1].trim().replace(/_/g, "").replace(/f(32|64)$/, ""));
  if (!Number.isFinite(v)) fail(`${file}: ${name} = ${m[1]} is not a number this gate can read`);
  return v;
}

const geo = pineGeometry();
const pos = geo.attributes.position.array;
const nrm = geo.attributes.normal.array;
const wnd = geo.attributes.aWind.array;
const col = geo.attributes.color.array;
const nVerts = geo.attributes.position.count;
const nTris = nVerts / 3;
check(Number.isInteger(nTris), `the pine has ${nVerts} vertices — not a whole number of triangles`);

let maxR = 0;
let maxY = -Infinity;
let minY = Infinity;
for (let i = 0; i < nVerts; i++) {
  const x = pos[i * 3];
  const y = pos[i * 3 + 1];
  const z = pos[i * 3 + 2];
  maxR = Math.max(maxR, Math.hypot(x, z));
  maxY = Math.max(maxY, y);
  minY = Math.min(minY, y);
}

// --- 1. the sim coupling ----------------------------------------------------
// The renderer must not draw a tree the sim's spawn clearance did not account
// for. Every number here comes from the file that owns it.
const simSpawnClear = rustConst("crates/sim-core/src/world.rs", "SPAWN_CLEAR_M");
const simCapsuleR = rustConst("crates/sim-core/src/collide.rs", "CAPSULE_RADIUS_M");
// The scatter's own visual scale range, read from its doc line rather than
// restated — `terrain.rs`: "Visual scale in [0.9, 1.1]".
const terrainRs = fs.readFileSync(path.join(ROOT, "crates/sim-core/src/terrain.rs"), "utf8");
const scaleDoc = terrainRs.match(/Visual scale in \[([\d.]+), ([\d.]+)\]/);
if (!scaleDoc) fail("terrain.rs no longer states its slot scale range — this gate reads it from the doc line");
const simMaxScale = Number(scaleDoc[2]);

check(
  maxR <= PINE_MAX_R + 1e-6,
  `the pine reaches radius ${maxR.toFixed(3)} m against its own ceiling PINE_MAX_R = ${PINE_MAX_R} — ` +
    `world.rs derives SPAWN_CLEAR_M from that ceiling, so widening the canopy here puts fresh ` +
    `spawns back inside trees`,
);
const reach = PINE_MAX_R * simMaxScale + simCapsuleR;
check(
  reach <= simSpawnClear,
  `a pine at the ceiling (${PINE_MAX_R} m x ${simMaxScale} scale) plus the ${simCapsuleR} m capsule ` +
    `reaches ${reach.toFixed(2)} m, and world.rs clears only ${simSpawnClear} m — a fresh spawn ` +
    `can stand inside a tree`,
);

// --- 2. tall and thin -------------------------------------------------------
// ART.md rule 6, as a ratio. The four-primitive pine this replaced scored
// 5.2 / 3.4 = 1.53, which is a shrub.
const height = maxY - minY;
const slenderness = height / (2 * maxR);
// 2.2, not 2.0: at the 1.7 m ceiling a 6.6 m tree scores 1.94, so a floor of
// 2.0 was very nearly implied by the radius check above it and would have gone
// green on a tree that had quietly given the whole gain back. This one binds.
const SLENDERNESS_MIN = 2.2;
check(
  slenderness >= SLENDERNESS_MIN,
  `the pine is ${height.toFixed(2)} m tall and ${(2 * maxR).toFixed(2)} m wide — slenderness ` +
    `${slenderness.toFixed(2)} against ART.md rule 6's floor of ${SLENDERNESS_MIN}. "Tall, thin, ` +
    `ragged-edged; a smooth cone is wrong at any texture budget."`,
);

// --- 3. the silhouette is ragged, and in BOTH axes ---------------------------
// A cone's outline is one straight line per side and its skirt is a level
// disc. The pull breaks the first, the droop breaks the second, and a pass
// that quietly reverted either would still pass every check above this one.
//
// Counted as distinct values rather than as a variance, because what the eye
// reads on a horizon is the number of independent EVENTS in the outline.
const q = (v) => Math.round(v * 1000);
const radii = new Set();
const rimY = new Set();
for (let i = 0; i < nVerts; i++) {
  const r = Math.hypot(pos[i * 3], pos[i * 3 + 2]);
  if (r > 0.35) radii.add(q(r)); // skip the trunk's own near-axis rings
}
// The rim heights: the lowest vertex of each whorl's ring. A level disc
// contributes ONE value per whorl; a drooping fringe contributes many.
for (let i = 0; i < nVerts; i++) {
  const r = Math.hypot(pos[i * 3], pos[i * 3 + 2]);
  if (r > 0.35) rimY.add(q(pos[i * 3 + 1]));
}
const SILHOUETTE_MIN = 30;
check(
  radii.size >= SILHOUETTE_MIN,
  `the canopy outline has ${radii.size} distinct radii — under ${SILHOUETTE_MIN} it reads as the ` +
    `"clean polygon edge with no sub-branch break-up" the visual judge named`,
);
const RIM_MIN = 20;
check(
  rimY.size >= RIM_MIN,
  `the canopy has ${rimY.size} distinct vertex heights — a whorl whose rim is a level disc stacks ` +
    `into horizontal lines up the trunk, which is the same defect rotated 90 degrees`,
);

// --- 3b. the trunk does not stand above the canopy ------------------------
// A bug this shipped with for exactly one render. The whorls taper to a point
// and the trunk does not, so a trunk run to full height leaves a bare brown
// spike over every tree in the forest — visible from any angle, and invisible
// to every check above this one, all of which it passed. Cheap to assert:
// where the trunk ends, the top whorl must still be wider than it is.
const top = PINE_SHAPE.whorls[PINE_SHAPE.whorls.length - 1];
const trunkTopR = 0.13; // CylinderGeometry(radiusTop, ...) in pineGeometry
check(
  PINE_SHAPE.trunkH < PINE_SHAPE.h,
  `the trunk is ${PINE_SHAPE.trunkH} m and the canopy tops out at ${PINE_SHAPE.h} m — the trunk is a spike above the tree`,
);
const coneRAtTrunkTop = top.r * ((top.y + top.h - PINE_SHAPE.trunkH) / top.h);
check(
  coneRAtTrunkTop > trunkTopR,
  `at the trunk's top (${PINE_SHAPE.trunkH} m) the crown is only ${coneRAtTrunkTop.toFixed(3)} m wide ` +
    `against the trunk's ${trunkTopR} m — the trunk pokes out through the canopy`,
);

// --- 4. the underside reads -------------------------------------------------
// ART.md §5 on the canopy: "the underside must still read (rule 3) — it is the
// face every judge has caught", and scene.js's fill and bounce poles were
// tuned against a capture of exactly that face (`03-canopy-up`). An open cone
// shows backfaces from below, which is to say nothing, so this counts the
// downward-facing area that actually exists.
let downArea = 0;
let totalArea = 0;
for (let t = 0; t < nTris; t++) {
  const i = t * 9;
  const ax = pos[i], ay = pos[i + 1], az = pos[i + 2];
  const bx = pos[i + 3], by = pos[i + 4], bz = pos[i + 5];
  const cx = pos[i + 6], cy = pos[i + 7], cz = pos[i + 8];
  const ux = bx - ax, uy = by - ay, uz = bz - az;
  const vx = cx - ax, vy = cy - ay, vz = cz - az;
  const nx = uy * vz - uz * vy;
  const ny = uz * vx - ux * vz;
  const nz = ux * vy - uy * vx;
  const area = Math.hypot(nx, ny, nz) * 0.5;
  totalArea += area;
  // The baked vertex normal, not the winding: that is what the shader lights.
  const ny0 = nrm[i + 1] + nrm[i + 4] + nrm[i + 7];
  if (ny0 < -0.3) downArea += area;
}
const downShare = downArea / totalArea;
const DOWN_SHARE_MIN = 0.15;
check(
  downShare >= DOWN_SHARE_MIN,
  `only ${(downShare * 100).toFixed(1)}% of the pine's area faces downward — under ` +
    `${DOWN_SHARE_MIN * 100}% the canopy has no underside for the fill and bounce to land on, ` +
    `and 03-canopy-up photographs a hollow shell`,
);

// --- 5. the wind cantilever is rooted ---------------------------------------
// materials.js displaces every vertex by `aWind`, so a base ring with a
// non-zero weight is a tree whose trunk slides along the ground — the single
// most obvious tell in animated foliage, and invisible to a still capture.
let baseMax = 0;
let tipMax = 0;
for (let i = 0; i < nVerts; i++) {
  const y = pos[i * 3 + 1];
  if (y - minY < 0.05) baseMax = Math.max(baseMax, wnd[i]);
  if (maxY - y < 0.05) tipMax = Math.max(tipMax, wnd[i]);
}
// Not `=== 0`: the base ring lands at y = 4.8e-8 rather than 0, because
// CylinderGeometry writes float32 and 6.6/2 does not round-trip. The weight
// that survives is 1.4e-14, and the assertion that means something is about
// MOTION — at any strength this file could plausibly carry (0.3 m today) a
// weight under 1e-6 is under a micrometre of sway. Rounding the attribute to
// hard zero would hide a real ramp bug behind a snap.
const ROOT_MAX_W = 1e-6;
check(
  baseMax < ROOT_MAX_W,
  `the pine's base ring carries wind weight ${baseMax} (> ${ROOT_MAX_W}) — the trunk will slide ` +
    `along the ground, which is the single most obvious tell in animated foliage`,
);
check(tipMax > 0.9, `the pine's tip carries wind weight ${tipMax} — the canopy barely moves`);
check(
  Math.abs(PINE_WIND.y1 - maxY) < 0.01,
  `the wind band tops out at ${PINE_WIND.y1} m and the tree is ${maxY.toFixed(2)} m — the ramp ` +
    `does not reach the tip it was authored for`,
);

// --- 6. two greens ----------------------------------------------------------
// ART.md §5: "Canopy: two greens minimum". Counted off the baked vertex
// colours, so a table edited down to one ramp fails here and not in a report.
const greens = new Set();
for (let i = 0; i < nVerts; i++) {
  if (Math.hypot(pos[i * 3], pos[i * 3 + 2]) > 0.35) {
    greens.add(`${q(col[i * 3])},${q(col[i * 3 + 1])},${q(col[i * 3 + 2])}`);
  }
}
check(greens.size >= 2, `the canopy bakes ${greens.size} distinct colour(s) — ART.md §5 wants two greens minimum`);
const canopyBands = PINE_BANDS.filter((b) => b.part !== "trunk").length;
check(canopyBands >= 2, `PINE_BANDS declares ${canopyBands} canopy ramp(s) — two greens is the floor`);
// …and that the whorls actually WEAR both of them. Neither check above sees a
// table collapsed onto one ramp: `PINE_BANDS` still declares two, and one
// lo→hi ramp over 5 m of canopy still bakes twenty distinct colours. This
// caught nothing until the mutation test proved the other two didn't either.
const wornBands = new Set(PINE_WHORL_BANDS).size;
check(
  wornBands >= 2,
  `the whorls wear ${wornBands} of the ${canopyBands} declared canopy ramps — a second green that ` +
    `nothing is assigned to is not a second green`,
);

// --- 7. determinism ---------------------------------------------------------
// Every pine in the world is this pine, and a chunk that streams out and back
// is bit-identical. An RNG with state anywhere in the builder breaks that, and
// nothing downstream would notice until a forest shimmered on re-entry.
const again = pineGeometry().attributes.position.array;
check(again.length === pos.length, `two builds of the pine gave ${again.length} and ${pos.length} floats`);
for (let i = 0; i < pos.length; i++) {
  if (again[i] !== pos[i]) {
    fail(`the pine is not deterministic: float ${i} is ${pos[i]} then ${again[i]} — something in the builder carries state`);
  }
}
checks++;

// --- 8. what the fleet costs ------------------------------------------------
// Reported, not asserted — the assertion belongs to `browser_smoke`, which can
// see the real frame. This is the arithmetic that says whether it is worth
// running: the near ring is 5x5 chunks of 64 m at 8 m scatter cells, forest
// biome draws trees at 260 per mille, and the pool casts into the main pass
// plus every level up to NEAR_CASTER_MAX_LEVEL.
const RING_CELLS = ((5 * 64) / 8) ** 2;
const TREE_PER_MILLE = 260;
const PASSES = 3; // main + shadow levels 0 and 1
const fleet = Math.round(RING_CELLS * (TREE_PER_MILLE / 1000)) * nTris * PASSES;

console.log(
  `pine shape: ${nTris} tris · ${height.toFixed(2)} m tall x ${(2 * maxR).toFixed(2)} m wide ` +
    `(slenderness ${slenderness.toFixed(2)} >= ${SLENDERNESS_MIN}) · ${radii.size} silhouette radii, ` +
    `${rimY.size} rim heights · ${(downShare * 100).toFixed(1)}% downward area · ` +
    `${greens.size} canopy colours · deterministic`,
);
console.log(
  `  sim coupling: canopy ${maxR.toFixed(3)} m <= ceiling ${PINE_MAX_R} m; ` +
    `${PINE_MAX_R} x ${simMaxScale} + ${simCapsuleR} = ${reach.toFixed(2)} m <= SPAWN_CLEAR_M ${simSpawnClear} m`,
);
console.log(
  `  fleet (reported, browser_smoke asserts): ~${Math.round(RING_CELLS * (TREE_PER_MILLE / 1000))} pines ` +
    `in a forest ring x ${nTris} tris x ${PASSES} passes = ~${(fleet / 1000).toFixed(0)}k tris`,
);

// --- 9. the tree the client actually draws ----------------------------------
// Everything above scores the cone builder, which is now the billboard LOD's
// starting point and nothing else. What the near ring draws is generated by
// ez-tree, and a generator needs MORE checking than a hand-authored shape, not
// less: nobody placed those vertices, so nobody can testify to where they went.
const parts = pineParts();
const triOf = (g) => (g.index ? g.index.count : g.attributes.position.count) / 3;
check(parts.length === 2, `the pine draws ${parts.length} part(s) — bark and needles are separate meshes`);
const bark = parts.find((p) => p.part === "bark");
const needle = parts.find((p) => p.part === "needle");
check(!!bark && !!needle, "the pine is missing its bark or its needle part");

// THE assertion this whole slice exists for. Three passes of geometry could not
// break a polygon silhouette; a cutout can. If this ever reads 0 the canopy is
// an opaque hull again and the tree is back to being programmer art.
check(
  needle.maps.alphaTest > 0,
  `the needle part has alphaTest ${needle.maps.alphaTest} — the canopy is an opaque hull and its ` +
    `silhouette is a polygon edge again`,
);
check(!!needle.maps.map, "the needle part has no texture — there is no alpha to test against");
check(!!bark.maps.map && !!bark.maps.normalMap, "the bark part is missing its colour or normal map");

// The sim coupling again, on the generated geometry — the number that matters
// most, because a preset tweak moves it and no human reviews 4,700 vertices.
let genR = 0;
let genTop = -Infinity;
let genBase = Infinity;
let genRootW = 0;
for (const part of parts) {
  const p = part.geo.attributes.position;
  const w = part.geo.attributes.aWind;
  check(!!w, `the ${part.part} part carries no aWind attribute — it will not move in wind`);
  for (let i = 0; i < p.count; i++) {
    genR = Math.max(genR, Math.hypot(p.getX(i), p.getZ(i)));
    genTop = Math.max(genTop, p.getY(i));
    genBase = Math.min(genBase, p.getY(i));
    if (p.getY(i) < 0.05) genRootW = Math.max(genRootW, w.getX(i));
  }
}
check(
  genR <= PINE_MAX_R,
  `the generated pine reaches ${genR.toFixed(3)} m against PINE_MAX_R ${PINE_MAX_R} — a preset ` +
    `tweak just widened the canopy past what world.rs clears a spawn for`,
);
check(
  Math.abs(genTop - PINE_SHAPE.h) < 0.05,
  `the generated pine is ${genTop.toFixed(2)} m against the ${PINE_SHAPE.h} m the wind band and the ` +
    `fell sink are authored for`,
);
check(genBase > -0.05, `the generated pine starts at y ${genBase.toFixed(3)} — it floats or it is buried`);
check(genRootW < ROOT_MAX_W, `the generated pine's base carries wind weight ${genRootW} — the trunk will slide`);

// Budget. Reported against the ring arithmetic, and ASSERTED against a ceiling,
// because this is the number a generator can run away with in one parameter.
const genTris = parts.reduce((a, p) => a + triOf(p.geo), 0);
const GEN_TRI_MAX = 8000;
check(
  genTris <= GEN_TRI_MAX,
  `the generated pine is ${genTris} triangles against a ${GEN_TRI_MAX} ceiling — at ~20 trees inside ` +
    `40 m over 3 passes that is ${Math.round((genTris * 20 * 3) / 1000)} k of DESIGN §9's 1.5 M`,
);

// Determinism, on the generator this time. ez-tree seeds its own RNG; if the
// seed were ever left unset the forest would differ per client and per reload.
const regen = pineParts();
for (const part of ["bark", "needle"]) {
  const a = parts.find((p) => p.part === part).geo.attributes.position.array;
  const b = regen.find((p) => p.part === part).geo.attributes.position.array;
  check(a.length === b.length, `two generations of the ${part} part differ in size`);
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) fail(`the generated pine is not deterministic: ${part} float ${i} moved`);
  }
}
checks++;

console.log(
  `generated pine: ${genTris} tris (${triOf(bark.geo)} bark + ${triOf(needle.geo)} needle) · ` +
    `${genTop.toFixed(2)} m tall, ${genR.toFixed(2)} m radius (ceiling ${PINE_MAX_R}) · ` +
    `alphaTest ${needle.maps.alphaTest} · ${PINE_VARIANTS} variant(s) · deterministic`,
);

console.log(`pine shape: ${checks} checks passed`);
