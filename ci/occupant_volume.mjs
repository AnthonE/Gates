#!/usr/bin/env node
// Gate: the volume the server blocks is the mesh the client draws.
//
// `OCCUPANT_R_M` and `OCCUPANT_TOP_M` in `crates/sim-core/src/terrain.rs` say
// of themselves that they are "read off the client's authored geometry
// (`web/src/props.js` `ARCHETYPES`) rather than chosen". Until this file
// nothing checked that sentence for any occupant. The tables' own rule is
// directional and the doc states it: "the rule is the mesh's maximum
// horizontal extent, so the server never lets a body reach a triangle the
// client is rendering. Erring outward blocks a little air at a
// dodecahedron's flats; erring inward is the bug."
//
// That is a claim about a vertex buffer, and no `cargo test` can see one. The
// Rust side has a const-assert block holding `OCCUPANT_R_M` equal to
// `occupant_volume()`'s match — but both live in the same file, so they move
// together under one edit and prove only that the file agrees with itself.
// `tests/solid.rs` asserts orderings (a trunk is narrower than a barrel) and
// behaviour at the radius, never the radius. Every number could be wrong by a
// metre with the whole workspace green.
//
// The immediate cause was `NOW.md`'s "the trunk radius is pinned to a builder
// that no longer ships": `OCCUPANT_R_M[Tree]` is 0.26 off `pineGeometry`'s
// cone, while `pineParts()` — the ez-tree pine `props.js` also exports — has a
// trunk of 0.3069. Generalising it was cheaper than doing the one row, and it
// found a second: `CrateSlot` read 0.68 against a measured half-diagonal of
// 0.680074, inward by the doc's own definition of the bug.
//
// The rule asserted here sandwiches each row between two measurements of the
// same mesh, so neither direction can drift and neither bound is a number
// anyone chose:
//
//   lower — no vertex below the blocking top may lie outside the blocking
//           radius. This is the doc's rule, stated as geometry. Break it and
//           a body reaches a drawn triangle.
//   upper — the blocking radius may not exceed the mesh's widest horizontal
//           extent, and the blocking top may not exceed the mesh's own
//           bounding sphere lifted onto the ground (which is the derivation
//           the table's doc already gives for the nodes: "their tops are 1.5
//           and not 2.0"). Break either and the table is blocking air the
//           prop never occupies — which is also how a gate of the lower
//           bound alone would be satisfied forever, by widening.
//
// The radius needed a horizontal bound rather than a spherical one, and a
// mutation found out why: the shelter is 9.2 m tall, so its bounding sphere
// is 9.72 and its row could be widened a full metre with the gate green.
//
// Three rows are exempt and each exemption is itself asserted rather than
// skipped, because an exemption nothing checks is a hole:
//
//   Tree — you walk UNDER a canopy, so the cylinder is the trunk and the
//          general rule cannot apply. Section 4 does the trunk instead, and
//          proves its own measurement band rather than assuming it.
//   Bush — a deliberate zero (a bush you cannot push through is an invisible
//          wall). Asserted zero on both tables, with the mesh proved present,
//          so "passable by design" cannot decay into "archetype went missing".
//   8    — the client's felled-pine stump, not a sim occupant. The enum skips
//          the discriminant and the tables keep the hole; both halves checked.
//
// Arithmetic over vertex buffers: no GPU, no shard, no renderer tier.

import "./dom_shim.mjs";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ARCHETYPES, PINE_SHAPE, pineGeometry, pineParts } from "../web/src/props.js";

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

const terrainSrc = fs.readFileSync(path.join(ROOT, "crates/sim-core/src/terrain.rs"), "utf8");

// Cross-language numbers are read, never restated — `ci/haven_shelter.mjs`'s
// rule, and the only way this gate is not just a second copy of the table.
const rustConst = (name) => {
  const m = terrainSrc.match(new RegExp(`^pub const ${name}:\\s*\\w+\\s*=\\s*([-\\d.]+);`, "m"));
  if (!m) fail(`${name} not found in crates/sim-core/src/terrain.rs`);
  return parseFloat(m[1]);
};

// An entry is either a float literal or the name of a `pub const` above it.
const resolve = (tok) => (/^[-\d.]+$/.test(tok) ? parseFloat(tok) : rustConst(tok));

const rustTable = (name) => {
  const m = terrainSrc.match(new RegExp(`^pub const ${name}: \\[f32; (\\d+)\\] = \\[([^\\]]*)\\];`, "m"));
  if (!m) fail(`${name} not found in crates/sim-core/src/terrain.rs`);
  const declared = parseInt(m[1], 10);
  const rows = m[2]
    .split("\n")
    .map((l) => l.replace(/\/\/.*$/, "").trim())
    .filter((l) => l.length > 0)
    .map((l) => l.replace(/,$/, "").trim());
  check(
    rows.length === declared,
    `${name} declares [f32; ${declared}] but ${rows.length} entries parsed — ` +
      `this gate reads the table by text and a reformat that defeats the ` +
      `parse would leave every check below scoring nothing`,
  );
  return rows.map(resolve);
};

const R = rustTable("OCCUPANT_R_M");
const TOP = rustTable("OCCUPANT_TOP_M");

// Every comparison below is made at the resolution both sides actually live
// at. The table is `[f32; 11]` and the vertices come out of a `Float32Array`,
// so comparing two f32-derived quantities in f64 scores storage noise rather
// than geometry: the dodecahedron's unit vertices read back as 0.99999998,
// `Math.hypot` over a rock's f32 components lands 4.8e-8 above its authored
// 1.5, and the barrel's exactly-authored 0.45 measures one f32 step high.
//
// So the comparison carries ONE f32 ulp — `2**-23` relative, the format's own
// step — and nothing more. That is a statement about representation, not a
// tolerance anyone tuned, and the arithmetic says it cannot absorb a real
// defect: at the crate's 0.68 the allowance is 8.1e-8, while the one true
// defect this gate found (`CrateSlot` blocking 0.68 against a measured
// half-diagonal of 0.680074) is 7.4e-5 — roughly 900x larger. Anything a
// player could stand in is nine orders of magnitude above this line.
const f32 = Math.fround;
const ulp = (v) => Math.abs(f32(v)) * 2 ** -23;
const atLeast = (a, b) => f32(a) >= f32(b) - ulp(b); // a >= b, at f32 resolution
const atMost = (a, b) => f32(a) <= f32(b) + ulp(b); // a <= b, at f32 resolution

// A row may round OUTWARD off the measured extent — the doc permits it
// ("erring outward blocks a little air at a dodecahedron's flats") and
// `CrateSlot` uses it, writing 0.6801 for a measured 0.680074. This is how
// far outward, and it is not a tuned number: the tables are written to four
// decimal places, so a millimetre is ten times the finest row's own written
// resolution — while still four orders of magnitude inside the metre-wide
// silent widening the upper bound exists to catch.
const ROUND_OUT_M = 0.001;

// The enum, by name, so a variant added at the wrong discriminant is caught
// here and not by someone noticing a crate drawn as a tree stump
// (CLAUDE.md's positional-payload trap).
const enumBody = terrainSrc.match(/pub enum Occupant \{([\s\S]*?)\n\}/);
if (!enumBody) fail("pub enum Occupant not found in terrain.rs");
const OCC = new Map();
for (const m of enumBody[1].matchAll(/^\s*(\w+) = (\d+),/gm)) OCC.set(m[1], parseInt(m[2], 10));
check(OCC.size >= 10, `only ${OCC.size} Occupant variants parsed — the enum parse is broken`);

// `occupant_volume()` is the match `slot_blocks` actually calls; the tables
// are its published view. Rust const-asserts them equal, but reading both
// here is what proves THIS file parsed the same numbers the sim runs on.
const VOL = new Map();
const volBody = terrainSrc.match(/pub const fn occupant_volume\([\s\S]*?\n\}/);
if (!volBody) fail("occupant_volume not found in terrain.rs");
for (const m of volBody[0].matchAll(/Occupant::(\w+) => \(([^,]+),\s*([^)]+)\)/g)) {
  VOL.set(m[1], [resolve(m[2].trim()), resolve(m[3].trim())]);
}
check(VOL.size === OCC.size, `occupant_volume has ${VOL.size} arms for ${OCC.size} variants`);

// ------------------------------------------------------- 1. the three tables
// One row per archetype, in all three languages' worth of table. This is the
// alignment the enum's own comment says has to hold "by construction": the
// occupant value IS the index into `ARCHETYPES`.

check(
  R.length === TOP.length,
  `OCCUPANT_R_M is ${R.length} long and OCCUPANT_TOP_M is ${TOP.length} — ` +
    `the two tables are indexed by the same enum and cannot differ`,
);
check(
  ARCHETYPES.length === R.length,
  `ARCHETYPES has ${ARCHETYPES.length} entries against ${R.length} table rows — ` +
    `an occupant either draws nothing or blocks nothing`,
);

for (const [name, idx] of OCC) {
  check(idx < ARCHETYPES.length, `Occupant::${name} = ${idx} falls off the end of ARCHETYPES`);
  if (name === "None") continue;
  check(ARCHETYPES[idx] != null, `Occupant::${name} = ${idx} but ARCHETYPES[${idx}] is null`);
  const [vr, vt] = VOL.get(name);
  check(vr === R[idx], `occupant_volume(${name}).0 = ${vr} but OCCUPANT_R_M[${idx}] = ${R[idx]}`);
  check(vt === TOP[idx], `occupant_volume(${name}).1 = ${vt} but OCCUPANT_TOP_M[${idx}] = ${TOP[idx]}`);
}

// -------------------------------------------------------- 2. the stump hole
// Index 8 is the one row that is structural rather than tuned: the client
// mints a stump when a tree comes down, the sim never places one. Both halves
// are asserted so the hole cannot quietly fill.

const STUMP = 8;
check(![...OCC.values()].includes(STUMP), `an Occupant variant took discriminant ${STUMP} — that index is the client's stump`);
check(ARCHETYPES[STUMP] != null, `ARCHETYPES[${STUMP}] is null — the stump the enum skips 8 to protect is gone`);
check(R[STUMP] === 0 && TOP[STUMP] === 0, `row ${STUMP} is ${R[STUMP]}/${TOP[STUMP]}, not the 0/0 hole`);

// -------------------------------------------------- 3. the sandwich, per row
// Measured against the geometry `terrain.js` actually instantiates: it calls
// `a.geo()` and offsets the mesh by `a.lift`, so world height is vertex y plus
// lift and the blocking cylinder is measured in the same frame.

const measure = (geo, lift) => {
  const p = geo.attributes.position;
  let maxR = 0; // widest horizontal extent, any height
  let maxTop = -Infinity; // highest vertex, in world height above slot ground
  let maxNorm = 0; // bounding-sphere radius about the geometry's own origin
  for (let i = 0; i < p.count; i++) {
    const x = p.getX(i);
    const y = p.getY(i);
    const z = p.getZ(i);
    const h = Math.sqrt(x * x + z * z);
    if (h > maxR) maxR = h;
    if (y + lift > maxTop) maxTop = y + lift;
    const n = Math.sqrt(h * h + y * y);
    if (n > maxNorm) maxNorm = n;
  }
  return { maxR, maxTop, maxNorm, verts: p.count };
};

// Widest horizontal extent strictly BELOW a given world height — the quantity
// the lower bound is about, since a body cannot reach what is over its head.
const extentBelow = (geo, lift, top) => {
  const p = geo.attributes.position;
  let r = 0;
  for (let i = 0; i < p.count; i++) {
    if (p.getY(i) + lift > top) continue;
    const h = Math.sqrt(p.getX(i) ** 2 + p.getZ(i) ** 2);
    if (h > r) r = h;
  }
  return r;
};

const TREE = OCC.get("Tree");
const BUSH = OCC.get("Bush");
const EXEMPT = new Map([
  [TREE, "canopy is walked under; section 4 pins the trunk"],
  [BUSH, "deliberately passable"],
  [STUMP, "not a sim occupant"],
]);

let sandwiched = 0;
for (const [name, idx] of OCC) {
  if (name === "None" || EXEMPT.has(idx)) continue;
  const a = ARCHETYPES[idx];
  const lift = a.lift ?? 0;
  const m = measure(a.geo(), lift);
  const below = extentBelow(a.geo(), lift, TOP[idx]);

  check(
    atLeast(R[idx], below),
    `${name}: blocks r=${R[idx]} but the mesh reaches ${below.toFixed(6)} m ` +
      `horizontally below its own top — a body walks into a drawn triangle. ` +
      `Erring inward is the bug (terrain.rs OCCUPANT_R_M doc).`,
  );
  check(
    atLeast(TOP[idx], m.maxTop),
    `${name}: blocks to ${TOP[idx]} m but the mesh reaches ${m.maxTop.toFixed(6)} m`,
  );
  check(
    atMost(R[idx], m.maxR + ROUND_OUT_M),
    `${name}: blocks r=${R[idx]} against a mesh only ${m.maxR.toFixed(6)} m ` +
      `wide at its widest — the table is blocking air the prop never ` +
      `occupies, which is also how a lower-bound-only gate stays green forever`,
  );
  check(
    atMost(TOP[idx], lift + m.maxNorm),
    `${name}: blocks to ${TOP[idx]} m against lift ${lift} + bounding sphere ` +
      `${m.maxNorm.toFixed(6)} = ${(lift + m.maxNorm).toFixed(6)}`,
  );
  sandwiched++;
  console.log(
    `  ${name.padEnd(13)} r ${below.toFixed(4)} <= ${String(R[idx]).padEnd(7)} <= ${(m.maxR + ROUND_OUT_M).toFixed(4)}` +
      `   top ${m.maxTop.toFixed(4)} <= ${String(TOP[idx]).padEnd(6)} <= ${(lift + m.maxNorm).toFixed(4)}`,
  );
}
check(sandwiched >= 6, `only ${sandwiched} occupants were sandwiched — the loop is scoring nothing`);

// The bush's zero is a design call, so it is asserted as one: zero on both
// tables, over a mesh that is demonstrably there.
const bushGeo = ARCHETYPES[BUSH].geo();
check(R[BUSH] === 0 && TOP[BUSH] === 0, `Bush is ${R[BUSH]}/${TOP[BUSH]} — a bush you cannot push through is a wall you cannot see over`);
check(measure(bushGeo, 0).maxR > 0.1, `Bush's mesh has no width — its zero row would be a missing prop, not a design call`);

// ------------------------------------------------------------- 4. the trunk
// `OCCUPANT_R_M[Tree]` is the TRUNK, not the canopy. Both pine builders taper
// upward from a base ring, so the widest trunk point is the base and a short
// band off the ground measures it — but the band is PROVED rather than
// assumed: whichever builder is being measured must be trunk-only throughout
// it, which is what `canopyY0` below establishes.
const TRUNK_BAND_M = 0.5;

const trunkOf = (geo, label) => {
  const p = geo.attributes.position;
  let r = 0;
  for (let i = 0; i < p.count; i++) {
    if (p.getY(i) > TRUNK_BAND_M) continue;
    const h = Math.sqrt(p.getX(i) ** 2 + p.getZ(i) ** 2);
    if (h > r) r = h;
  }
  check(r > 0, `${label}: no vertices below ${TRUNK_BAND_M} m — the trunk band measured nothing`);
  // Lowest height at which the mesh is wider than the base ring. If that is
  // inside the band, the band is measuring canopy and the number is not a
  // trunk radius.
  let canopyY0 = Infinity;
  for (let i = 0; i < p.count; i++) {
    const h = Math.sqrt(p.getX(i) ** 2 + p.getZ(i) ** 2);
    if (h > r + 1e-4 && p.getY(i) < canopyY0) canopyY0 = p.getY(i);
  }
  check(
    canopyY0 > TRUNK_BAND_M,
    `${label}: the mesh widens past the base ring at ${canopyY0.toFixed(4)} m, ` +
      `inside the ${TRUNK_BAND_M} m trunk band — this measurement is canopy, not trunk`,
  );
  return { r, canopyY0 };
};

const cone = trunkOf(pineGeometry(), "cone pine");
const gen = trunkOf(pineParts().find((p) => p.part === "bark").geo, "generated pine");

// Which tree the client instantiates is `ARCHETYPES[1]`'s own choice —
// `terrain.js` branches on `a.parts ? a.parts() : a.geo()`. The DRAWN one is
// what the table has to equal, so wiring the other in without moving the
// table goes red here rather than shipping a trunk you walk through.
const drawnParts = ARCHETYPES[TREE].parts != null;
const drawn = drawnParts ? gen : cone;
const drawnName = drawnParts ? "generated (pineParts)" : "cone (pineGeometry)";
check(
  Math.abs(drawn.r - R[TREE]) < 1e-4,
  `ARCHETYPES[${TREE}] draws the ${drawnName} pine, whose trunk measures ` +
    `${drawn.r.toFixed(4)} m, but OCCUPANT_R_M[${TREE}] is ${R[TREE]}. The ` +
    `table is read off the drawn mesh; move it in this commit.`,
);
check(
  TOP[TREE] === PINE_SHAPE.trunkH,
  `OCCUPANT_TOP_M[${TREE}] is ${TOP[TREE]} against PINE_SHAPE.trunkH ${PINE_SHAPE.trunkH}`,
);
check(
  TOP[TREE] < PINE_SHAPE.h,
  `OCCUPANT_TOP_M[${TREE}] ${TOP[TREE]} reaches the crown at ${PINE_SHAPE.h} — ` +
    `the tree is supposed to stop at the trunk so a body walks under the canopy`,
);

// The builder that is NOT drawn, pinned as a measurement so an ez-tree preset
// or `PINE_EZ` change cannot move it unnoticed. It is 18% wider than the cone
// today, which is the whole reason the pin above is written against the drawn
// builder rather than a fixed one.
const PINE_EZ_TRUNK_R_M = 0.3069; // measured off the shipped builder, 2026-08-05
check(
  Math.abs(gen.r - PINE_EZ_TRUNK_R_M) < 5e-4,
  `the generated pine's trunk measures ${gen.r.toFixed(4)} m against a ` +
    `recorded ${PINE_EZ_TRUNK_R_M}. If that is intended, update this pin — and ` +
    `if ARCHETYPES[${TREE}] gains a \`parts\` key, OCCUPANT_R_M[${TREE}] moves ` +
    `to this number in the same commit.`,
);
console.log(
  `  Tree          trunk cone ${cone.r.toFixed(4)} (canopy from ${cone.canopyY0.toFixed(2)} m), ` +
    `generated ${gen.r.toFixed(4)} (canopy from ${gen.canopyY0.toFixed(2)} m); ` +
    `drawn = ${drawnName}, table ${R[TREE]}/${TOP[TREE]}`,
);

console.log(`ok: occupant volume — ${checks} checks, ${sandwiched} rows sandwiched`);
