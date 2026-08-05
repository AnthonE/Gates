#!/usr/bin/env node
// Gate: the haven pad's greybox is a building, and it is the building
// sim-core placed.
//
// The judge's ranked gap that asked for this (pass-20260805-002720-01, gap 3)
// named the mechanism precisely: as of scatter clumping the island has
// natural clearings by design — 2.2-4.1% of forest windows are empty against
// an independent-draw rate 35x lower — so the pad, which is a clearing, is
// now statistically indistinguishable from the scenery around it. A player
// who walks the road correctly to the destination has no way to know they
// arrived. Silhouette is what makes arrival legible, and the gap's own
// concrete item was "a batched, seeded mesh in a worldgen slot", gated as
// arithmetic against `ci/pine_shape.mjs`'s standard.
//
// So this scores three claims no `cargo test` can see, because they live on
// the far side of the Rust/JS line:
//
//   1. The mesh fits the slot sim-core reserved for it. `HAVEN_SHELTER_HALF_M`
//      is the number `crates/sim-core/src/terrain.rs` uses to prove the
//      structure clears every container and stands inside the exclusion zone.
//      If the mesh outgrows it, both proofs are false and nothing in the Rust
//      workspace would notice — the sim never sees a triangle.
//   2. It is a building you can walk into, not a solid block. Asserted by
//      flood-filling the floor plan at chest height from outside the
//      footprint: the interior must be reachable through the doorway, and —
//      the half that keeps this from being a gate that passes on an empty
//      lot — it must become UNREACHABLE when the doorway is filled in. A
//      reachability test on a structure with no walls passes trivially.
//   3. It stands above its own forest. A pine is `PINE_H` and scatter scales
//      it to at most 1.1x, so a silhouette that does not clear that is not a
//      silhouette, it is undergrowth.
//
// What it deliberately does NOT assert is that anything is COLLIDABLE. It is
// not: `crates/sim-core/src/collide.rs` knows only built pieces, and no
// scatter occupant — not a tree, not a boulder, not this — stops a player.
// A player walks through these walls today. That is the systems lane's path
// and it is left as a one-line request in `NOW.md` rather than reached into
// from here.

import "./dom_shim.mjs";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  ARCHETYPES,
  HAVEN_SHELTER_PARTS,
  HAVEN_SHELTER_PEAK,
  PINE_SHAPE,
  shelterGeometry,
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

// Cross-language numbers are read, never restated — the same rule
// `ci/haven_prize.mjs` follows, and held in an object so
// `ci/knob_registry.mjs` does not read these as knob declarations of their
// own.
const terrainSrc = fs.readFileSync(
  path.join(ROOT, "crates/sim-core/src/terrain.rs"),
  "utf8",
);
const rustConst = (name) => {
  const m = terrainSrc.match(
    new RegExp(`^pub const ${name}:\\s*\\w+\\s*=\\s*([-\\d.]+);`, "m"),
  );
  if (!m) fail(`${name} not found in crates/sim-core/src/terrain.rs`);
  return parseFloat(m[1]);
};
const k = {
  half: rustConst("HAVEN_SHELTER_HALF_M"),
  offset: rustConst("HAVEN_SHELTER_R_M"),
  padRadius: rustConst("HAVEN_RADIUS_M"),
  crateR: rustConst("HAVEN_CRATE_R_M"),
};

// The occupant the client table has to line up with, read off the enum
// rather than assumed. This is the alignment CLAUDE.md's positional-payload
// trap is about: the enum value IS the archetype index (`terrain.js:676`
// casts the slot's first float straight into `this.pools[]`), so a variant
// added at the wrong discriminant draws the shelter as something else with
// every gate in the workspace still green.
const occMatch = terrainSrc.match(/^\s*HavenShelter = (\d+),/m);
if (!occMatch) fail("Occupant::HavenShelter not found in terrain.rs");
const OCC = parseInt(occMatch[1], 10);

// ---------------------------------------------------------------- 1. wiring

check(
  ARCHETYPES.length > OCC,
  `Occupant::HavenShelter is ${OCC} but ARCHETYPES has ${ARCHETYPES.length} ` +
    `entries — the shelter would fall off the end of the pool table and ` +
    `draw nothing, silently`,
);
const arch = ARCHETYPES[OCC];
check(
  arch != null,
  `ARCHETYPES[${OCC}] is null — sim-core places a shelter the client has ` +
    `no archetype for`,
);
check(
  arch.geo === shelterGeometry && arch.composite === true,
  `ARCHETYPES[${OCC}] does not build from shelterGeometry() as a composite ` +
    `— terrain.js would wrap it in bakedGeometry a second time`,
);
check(
  arch.lift === 0,
  `the shelter has lift ${arch.lift}: its plinth is authored below y=0 ` +
    `already, and a lift would raise the whole building off its own base`,
);
check(
  HAVEN_SHELTER_PARTS.length >= 8,
  `${HAVEN_SHELTER_PARTS.length} parts is not a building`,
);

// -------------------------------------------------------------- 2. footprint

let maxHalf = 0;
let minY = Infinity;
let maxY = -Infinity;
for (const [name, cx, cy, cz, sx, sy, sz] of HAVEN_SHELTER_PARTS) {
  check(
    sx > 0 && sy > 0 && sz > 0,
    `part ${name} has a non-positive extent (${sx}, ${sy}, ${sz})`,
  );
  maxHalf = Math.max(
    maxHalf,
    Math.abs(cx) + sx * 0.5,
    Math.abs(cz) + sz * 0.5,
  );
  minY = Math.min(minY, cy - sy * 0.5);
  maxY = Math.max(maxY, cy + sy * 0.5);
}
check(
  maxHalf <= k.half + 1e-4,
  `the mesh reaches ${maxHalf.toFixed(2)} m from its origin against ` +
    `HAVEN_SHELTER_HALF_M = ${k.half} — sim-core proved the structure clears ` +
    `the containers and stays inside the pad using that number, and both ` +
    `proofs are now false`,
);
// The corner is what leaves a circle first, so the pad-containment claim is
// made on the diagonal and not on the side.
const corner = maxHalf * Math.SQRT2;
check(
  k.offset + corner < k.padRadius,
  `a corner reaches ${(k.offset + corner).toFixed(2)} m from the pad center ` +
    `against a ${k.padRadius} m exclusion zone — scatter fills the ground ` +
    `outside it, so a tree would grow through a wall`,
);
check(
  minY < -0.5,
  `the mesh bottoms out at ${minY.toFixed(2)} m: the pad is found flat, not ` +
    `carved flat (worst measured relief 3.76 m over 32 m), so a base that ` +
    `does not reach below its own floor will show daylight under a corner`,
);
check(
  Math.abs(maxY - HAVEN_SHELTER_PEAK) < 1e-6,
  `HAVEN_SHELTER_PEAK says ${HAVEN_SHELTER_PEAK} and the parts say ${maxY}`,
);

// ------------------------------------------------------------- 3. silhouette

const pineMax = PINE_SHAPE.h * 1.1;
check(
  maxY > pineMax + 1.5,
  `the shelter peaks at ${maxY.toFixed(2)} m against a ${pineMax.toFixed(2)} m ` +
    `pine at its largest scale — it does not clear its own forest, so ` +
    `nothing about arriving is legible from the road`,
);
// A tower, not a block. Measured on the BOUNDING footprint at each height,
// not on the material in it: a thin-walled room encloses 6.5 m of silhouette
// with 10 m² of wall, and summing the wall area would score this building as
// nearly as solid at the top as at the bottom while the outline says
// otherwise. The silhouette is what the criterion is about, so the
// silhouette is what gets measured.
// Width of the outline, min to max — NOT the reach from the origin. The
// tower stands at z = -1.4 so it is 2.5 m from center and 2.2 m wide, and
// scoring it by its reach called a slender tower a block.
const spanAt = (y) => {
  let x0 = Infinity;
  let x1 = -Infinity;
  let z0 = Infinity;
  let z1 = -Infinity;
  for (const [, cx, cy, cz, sx, sy, sz] of HAVEN_SHELTER_PARTS) {
    if (y < cy - sy * 0.5 || y > cy + sy * 0.5) continue;
    x0 = Math.min(x0, cx - sx * 0.5);
    x1 = Math.max(x1, cx + sx * 0.5);
    z0 = Math.min(z0, cz - sz * 0.5);
    z1 = Math.max(z1, cz + sz * 0.5);
  }
  return x1 < x0 ? 0 : Math.max(x1 - x0, z1 - z0);
};
const lowSpan = spanAt(2.0);
const highSpan = spanAt(maxY - 0.5);
check(lowSpan > 0 && highSpan > 0, "the mesh is empty at one of its own bands");
check(
  highSpan < lowSpan * 0.6,
  `the outline 0.5 m under the peak is ${highSpan.toFixed(1)} m wide against ` +
    `${lowSpan.toFixed(1)} m at the walls — that is a block, not a tower, ` +
    `and a block has no silhouette to recognise`,
);
// And the tower has to be tall enough to be the thing you see, not a lump on
// a roof: at least a third of the building's height above the wall line.
const wallTop = HAVEN_SHELTER_PARTS.find((p) => p[0] === "roof");
const roofY = wallTop[2] + wallTop[5] * 0.5;
check(
  maxY - roofY >= (maxY - minY) / 3,
  `only ${(maxY - roofY).toFixed(2)} m of the shelter stands above its roof ` +
    `line out of ${(maxY - minY).toFixed(2)} m overall — the tower is a ` +
    `detail rather than the silhouette`,
);

// ------------------------------------------------- 4. it is a building

// The floor plan at chest height, on a 0.1 m grid over a square comfortably
// larger than the footprint, flood-filled inward from the boundary. `open`
// lets the doorway be closed off so the same routine can prove the walls
// enclose anything at all.
const PROBE_Y = 1.2;
const STEP = 0.1;
const SPAN = Math.ceil((maxHalf + 1.0) / STEP);
const solidAt = (x, z, open) => {
  for (const [name, cx, cy, cz, sx, sy, sz] of HAVEN_SHELTER_PARTS) {
    if (!open && name.startsWith("jamb")) continue;
    if (PROBE_Y < cy - sy * 0.5 || PROBE_Y > cy + sy * 0.5) continue;
    if (Math.abs(x - cx) <= sx * 0.5 && Math.abs(z - cz) <= sz * 0.5) return true;
  }
  return false;
};
// `open = false` fills the doorway instead: the jambs are dropped and the
// whole front wall is treated as solid, which is the control.
const blockedFront = (x, z) => {
  const front = HAVEN_SHELTER_PARTS.find((p) => p[0] === "jamb-left");
  return Math.abs(z - front[3]) <= front[6] * 0.5 && Math.abs(x) <= maxHalf;
};

const reachInterior = (open) => {
  const seen = new Set();
  const key = (i, j) => (i + SPAN) * (2 * SPAN + 1) + (j + SPAN);
  const stack = [];
  for (let i = -SPAN; i <= SPAN; i++) {
    for (const j of [-SPAN, SPAN]) {
      stack.push([i, j]);
      stack.push([j, i]);
    }
  }
  while (stack.length) {
    const [i, j] = stack.pop();
    if (i < -SPAN || i > SPAN || j < -SPAN || j > SPAN) continue;
    if (seen.has(key(i, j))) continue;
    const x = i * STEP;
    const z = j * STEP;
    if (solidAt(x, z, open)) continue;
    if (!open && blockedFront(x, z)) continue;
    seen.add(key(i, j));
    stack.push([i + 1, j], [i - 1, j], [i, j + 1], [i, j - 1]);
  }
  // The interior sample: the middle of the room, behind the doorway and
  // clear of the tower, which stands at z = -1.4.
  return seen.has(key(0, Math.round(1.5 / STEP)));
};

check(
  reachInterior(true),
  `the inside of the shelter cannot be reached from outside at ${PROBE_Y} m ` +
    `— it is a monolith, and the whole point of a greybox is that a player ` +
    `can walk into it`,
);
check(
  !reachInterior(false),
  `the interior is still reachable with the doorway filled in — the walls ` +
    `enclose nothing, so the reachability check above passes on an empty ` +
    `lot and proves nothing`,
);

// The doorway a player has to fit through, measured rather than declared.
const jambL = HAVEN_SHELTER_PARTS.find((p) => p[0] === "jamb-left");
const jambR = HAVEN_SHELTER_PARTS.find((p) => p[0] === "jamb-right");
const lintel = HAVEN_SHELTER_PARTS.find((p) => p[0] === "lintel");
check(
  jambL && jambR && lintel,
  "the front wall is not a doorway: jamb-left, jamb-right and lintel must " +
    "all be present, and the flood fill above names them",
);
const clearW = jambR[1] - jambR[4] * 0.5 - (jambL[1] + jambL[4] * 0.5);
const floorY = HAVEN_SHELTER_PARTS.find((p) => p[0] === "plinth");
const floorTop = floorY[2] + floorY[5] * 0.5;
const clearH = lintel[2] - lintel[5] * 0.5 - floorTop;
// A player is a 0.4 m capsule (`world.rs` derives SPAWN_CLEAR_M from it), so
// 1.2 m of clear width is three body widths and cannot be a squeeze bug.
check(
  clearW >= 1.2,
  `the doorway is ${clearW.toFixed(2)} m wide — a player is a 0.4 m capsule ` +
    `and a door narrower than 1.2 m is a place they get stuck`,
);
check(
  clearH >= 2.0,
  `the doorway is ${clearH.toFixed(2)} m of headroom — under 2.0 m it reads ` +
    `as a hatch rather than an entrance`,
);
check(
  jambL[3] > 0 && jambR[3] > 0,
  `the doorway is on the -Z face: sim-core's yaw faces the shelter IN at the ` +
    `pad center and LUT index 0 is +Z, so a +Z doorway is the one a player ` +
    `walking off the road sees. On -Z they arrive at the back wall.`,
);

// ------------------------------------------------ 5. budget and determinism

const g = shelterGeometry();
const tris = g.attributes.position.count / 3;
check(
  Number.isInteger(tris) && tris > 0,
  `shelterGeometry() produced ${tris} triangles`,
);
check(
  tris <= 400,
  `${tris} triangles for one greybox — DESIGN §9's fleet budget is 1.5 M and ` +
    `this is a single instance, but a greybox that needs a budget conversation ` +
    `is no longer a greybox`,
);
check(
  g.attributes.color != null && g.attributes.aWind != null,
  "the baked buffer is missing an attribute every other archetype carries",
);
// A greybox with no value gradient goes flat against the sky.
const distinct = new Set();
const col = g.attributes.color.array;
for (let i = 0; i < col.length; i += 3) {
  distinct.add(
    [col[i], col[i + 1], col[i + 2]].map((v) => Math.round(v * 1000)).join(","),
  );
}
check(
  distinct.size >= 2,
  `the shelter bakes ${distinct.size} distinct vertex colour(s) — one flat ` +
    `value has no gradient to read the form by`,
);
// Wind must be zero everywhere: the cantilever is rooted at a trunk base and
// a swaying building is worse than a still one.
const wind = g.attributes.aWind.array;
let maxWind = 0;
for (const w of wind) maxWind = Math.max(maxWind, Math.abs(w));
check(maxWind < 1e-6, `the shelter carries wind weight ${maxWind} — it sways`);

const g2 = shelterGeometry();
const a = g.attributes.position.array;
const b = g2.attributes.position.array;
check(a.length === b.length, "shelterGeometry() is not deterministic in size");
let same = true;
for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) same = false;
check(same, "shelterGeometry() is not deterministic float-for-float");

console.log(
  `haven shelter: ${HAVEN_SHELTER_PARTS.length} parts, ${tris} tris, ` +
    `${maxY.toFixed(2)} m peak over a ${pineMax.toFixed(2)} m pine, ` +
    `footprint half ${maxHalf.toFixed(2)} m of ${k.half} allowed`,
);
console.log(
  `haven shelter: doorway ${clearW.toFixed(2)} m x ${clearH.toFixed(2)} m, ` +
    `corner ${(k.offset + corner).toFixed(2)} m from the pad center of ` +
    `${k.padRadius} m, containers on a ${k.crateR} m ring`,
);
console.log(`haven shelter: ${checks} checks passed`);
