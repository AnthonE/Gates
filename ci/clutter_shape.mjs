#!/usr/bin/env node
// Gate: the ground clutter the client DRAWS is the population worldgen placed,
// and it fits inside the frame budget it has to live in.
//
// `crates/sim-core/tests/clutter.rs` owns the placement side and measures
// ART.md rule 4 directly — the largest bare disc inside 15 m. Nothing there
// can see the client, so nothing there notices if the tile size the JS streams
// on stops matching the tile size the Rust fills, if a tuft is drawn as a
// horizontal card that lights black, or if the ring's triangle fleet quietly
// doubles. Those are this file's four jobs, and all four are arithmetic:
//
//   §1  the cross-language coupling — three constants read from BOTH sources
//   §2  the kind order — the JS row table against the Rust enum, by name
//   §3  the geometry contract — rooted wind, upward normals, tuft proportions
//   §4  the fleet budget — ASSERTED, not printed
//
// §4 is where this deliberately goes further than `ci/pine_shape.mjs`, whose
// own fleet section says in its comments that it "asserts nothing" and leaves
// the real number to `browser_smoke`. That was the right call for a forest of
// ~400 trees; it is the wrong one for a population of ~15,000 elements, which
// is the largest instance count in the client by an order of magnitude and the
// one most likely to be grown a triangle at a time by a later pass. A number
// that fails in twenty seconds beats a pixel that fails in nineteen minutes.

import "./dom_shim.mjs";

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CLUTTER_FILLS_PER_FRAME,
  CLUTTER_FLOATS,
  CLUTTER_PER_TILE,
  CLUTTER_POOL_CAP,
  CLUTTER_RING,
  CLUTTER_ROWS,
  CLUTTER_TILE_M,
} from "../web/src/clutter.js";

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

const terrainRs = fs.readFileSync(path.join(ROOT, "crates/sim-core/src/terrain.rs"), "utf8");
const bridgeRs = fs.readFileSync(path.join(ROOT, "crates/client-wasm/src/bridge.rs"), "utf8");

/** A Rust `const NAME: ty = value;` — the number, or a loud failure. */
function rustConst(src, name, where) {
  const m = src.match(new RegExp(`const\\s+${name}\\s*:\\s*[A-Za-z0-9_]+\\s*=\\s*([-0-9._]+)`));
  if (!m) fail(`${name} is not declared in ${where} — the coupling cannot be checked`);
  return Number(m[1]);
}

// ── §1 The cross-language coupling ────────────────────────────────────────
//
// The client streams on a tile size, sizes a cache on a per-tile cap and
// strides a buffer by a float count. All three are decided in Rust. A
// disagreement in any of them is not a visual defect — it is the client
// reading past the end of a fill or dropping most of a tile — and it would be
// invisible to every other gate here, exactly as `PINE_MAX_R` was.

check(
  CLUTTER_TILE_M === rustConst(terrainRs, "CLUTTER_TILE_M", "terrain.rs"),
  `CLUTTER_TILE_M: clutter.js has ${CLUTTER_TILE_M}, terrain.rs has ` +
    `${rustConst(terrainRs, "CLUTTER_TILE_M", "terrain.rs")}`,
);
check(
  CLUTTER_PER_TILE === rustConst(terrainRs, "CLUTTER_PER_TILE", "terrain.rs"),
  `CLUTTER_PER_TILE: clutter.js has ${CLUTTER_PER_TILE}, terrain.rs has ` +
    `${rustConst(terrainRs, "CLUTTER_PER_TILE", "terrain.rs")}`,
);
check(
  CLUTTER_FLOATS === rustConst(bridgeRs, "CLUTTER_FLOATS", "bridge.rs"),
  `CLUTTER_FLOATS: clutter.js strides by ${CLUTTER_FLOATS}, bridge.rs writes ` +
    `${rustConst(bridgeRs, "CLUTTER_FLOATS", "bridge.rs")}`,
);

// The grid has to divide exactly on this side too, or a tile's elements land
// outside the tile the client thinks it loaded.
const cellM = rustConst(terrainRs, "CLUTTER_CELL_M", "terrain.rs");
const perSide = rustConst(terrainRs, "CLUTTER_CELLS_PER_TILE", "terrain.rs");
check(
  Math.abs(perSide * cellM - CLUTTER_TILE_M) < 1e-6,
  `${perSide} cells of ${cellM} m is not a ${CLUTTER_TILE_M} m tile`,
);
check(
  perSide * perSide === CLUTTER_PER_TILE,
  `a ${perSide}×${perSide} tile holds ${perSide * perSide} cells, not ${CLUTTER_PER_TILE}`,
);

// The ring has to cover the distance ART.md rule 4 is actually about. The
// player stands anywhere inside their own tile, so the guaranteed reach is
// the ring's tiles MINUS the one they might be standing at the far edge of.
const RULE4_NEAR_M = 15.0;
const reach = CLUTTER_RING * CLUTTER_TILE_M;
check(
  reach >= RULE4_NEAR_M,
  `a ring of ${CLUTTER_RING} tiles guarantees only ${reach} m of population, ` +
    `under ART.md rule 4's ${RULE4_NEAR_M} m near field`,
);
check(
  CLUTTER_POOL_CAP === CLUTTER_PER_TILE * (2 * CLUTTER_RING + 1) ** 2,
  `the pool cap ${CLUTTER_POOL_CAP} is not the whole ring — a kind that filled ` +
    `the ring would be silently truncated`,
);
check(CLUTTER_FILLS_PER_FRAME >= 1, "the ring would never finish loading");

// ── §2 The kind order ─────────────────────────────────────────────────────
//
// `CLUTTER_ROWS[k]` draws `Clutter` discriminant `k + 1`. This is the
// positional-payload shape CLAUDE.md's trap list names as where the reference
// ecosystem actually bled — the right value in the wrong position, invisible
// to a byte golden. Swap two rows here and grass grows on cliffs, with every
// other gate green. So it is checked by NAME against the enum.

const enumBlock = terrainRs.match(/pub enum Clutter \{([\s\S]*?)\n\}/);
if (!enumBlock) fail("the Clutter enum is not where terrain.rs said it was");
const variants = [...enumBlock[1].matchAll(/^\s{4}([A-Z][A-Za-z]*)\s*=\s*(\d+)/gm)].map((m) => [
  m[1],
  Number(m[2]),
]);
check(variants.length >= 2, `parsed only ${variants.length} Clutter variants`);
const live = variants.filter(([, v]) => v > 0);
check(
  live.length === CLUTTER_ROWS.length,
  `terrain.rs has ${live.length} live clutter kinds, clutter.js draws ${CLUTTER_ROWS.length}`,
);
for (const [name, value] of live) {
  const row = CLUTTER_ROWS[value - 1];
  check(
    row && row.name === name.toLowerCase(),
    `Clutter::${name} = ${value} is drawn by row ${value - 1}, which is ` +
      `"${row ? row.name : "missing"}" — the population would wear the wrong geometry`,
  );
}

// ── §3 The geometry contract ──────────────────────────────────────────────

const triOf = (g) => g.attributes.position.count / 3;
const attr = (g, n) => g.attributes[n];

const built = CLUTTER_ROWS.map((row) => ({ row, geo: row.geo() }));

for (const { row, geo } of built) {
  const pos = attr(geo, "position").array;
  const nrm = attr(geo, "normal").array;
  const wind = attr(geo, "aWind");
  check(!!wind, `${row.name} has no aWind attribute — the shared wind patch would read garbage`);
  check(!!attr(geo, "color"), `${row.name} has no vertex colours but its material declares them`);
  check(triOf(geo) >= 1, `${row.name} is empty`);

  // Nothing dips below its own origin: an element is placed AT the ground
  // height worldgen resolved, so geometry under y=0 is geometry buried in the
  // terrain — the "everything sits IN it" half of ART.md rule 2, from the
  // side this lane owns.
  let minY = Infinity;
  let maxY = -Infinity;
  let maxR = 0;
  for (let i = 0; i < pos.length; i += 3) {
    minY = Math.min(minY, pos[i + 1]);
    maxY = Math.max(maxY, pos[i + 1]);
    maxR = Math.max(maxR, Math.hypot(pos[i], pos[i + 2]));
  }
  check(minY >= -1e-6, `${row.name} reaches ${minY.toFixed(3)} m below its own base`);

  // No normal may be NEAR-HORIZONTAL. That is the black-cutout failure the
  // vegetation pack names: a normal perpendicular to up is grazed by a key
  // light 21° above the horizon and the surface reads black from most of the
  // compass. Straight DOWN is a different thing and allowed — it is what a
  // closed solid's underside is, it is never lit by design, and it is culled
  // from every angle this population is seen from. The first cut asserted
  // "points up" and caught the pebble's base cap, which is correct geometry.
  const HORIZON_GUARD = 0.15;
  let worst = Infinity;
  for (let i = 0; i < nrm.length; i += 3) {
    const l = Math.hypot(nrm[i], nrm[i + 1], nrm[i + 2]);
    if (Math.abs(l - 1) > 1e-3) fail(`${row.name} has a non-unit normal (length ${l.toFixed(4)})`);
    worst = Math.min(worst, Math.abs(nrm[i + 1] / l));
  }
  check(
    worst > HORIZON_GUARD,
    `${row.name} has a normal ${worst.toFixed(3)} off horizontal (guard ${HORIZON_GUARD}) — ` +
      `it would read black whenever the key light grazes it`,
  );
  // …and most of it must face UP. Counted by triangle rather than averaged,
  // because an average is a knob and this is a fact: a solid's underside is a
  // minority of its faces, and a mesh whose windings are inverted flips the
  // count outright. The first cut of `chip` had exactly that defect — every
  // side normal pointing down and inward — and it lit correctly by the guard
  // above while facing into the ground.
  let up = 0;
  let down = 0;
  for (let i = 0; i < nrm.length; i += 9) {
    if (nrm[i + 1] > 0) up++;
    else down++;
  }
  check(
    up > down,
    `${row.name} has ${down} downward faces against ${up} upward — its windings are inverted`,
  );

  // Wind is ROOTED or absent. The vegetation pack's stated failure is "leaf
  // wind moves card roots instead of remaining anchored"; a tuft whose base
  // slides is the tell, and litter must not sway at all.
  const w = wind.array;
  let wMax = 0;
  for (let i = 0; i < w.length; i++) wMax = Math.max(wMax, w[i]);
  if (row.wind) {
    check(wMax > 0.9, `${row.name} declares wind but its strongest weight is ${wMax.toFixed(3)}`);
    for (let i = 0; i < w.length; i++) {
      const y = pos[i * 3 + 1];
      if (y <= minY + 1e-4 && w[i] > 1e-4) {
        fail(`${row.name}'s base vertex ${i} has wind weight ${w[i].toFixed(3)}, not 0 — it slides`);
      }
    }
  } else {
    check(wMax <= 1e-6, `${row.name} does not declare wind but carries a weight of ${wMax}`);
  }

  built.find((b) => b.row === row).facts = { tris: triOf(geo), maxY, maxR };
}

// The tuft is the whole point of the exercise, so it gets the shape assertion
// ART.md §1 actually asks for: "thousands of individual lit blades standing
// 20-40 cm", which is taller than wide and inside that band.
const tuft = built.find((b) => b.row.name === "tuft");
check(!!tuft, "there is no tuft row — the grass channel would draw nothing");
check(
  tuft.facts.maxY >= 0.2 && tuft.facts.maxY <= 0.4,
  `a tuft stands ${tuft.facts.maxY.toFixed(2)} m, outside ART.md §1's 0.20-0.40 m band`,
);
check(
  tuft.facts.maxY > tuft.facts.maxR * 2,
  `a tuft is ${tuft.facts.maxY.toFixed(2)} m tall and ${tuft.facts.maxR.toFixed(2)} m wide — ` +
    `blades stand, they do not sprawl`,
);
// Three blades at 120°, not three copies of one: the outward lean has to
// actually point three different ways or a tuft is a flat card with extra
// triangles.
const tp = attr(tuft.geo, "position").array;
const bearings = new Set();
for (let i = 0; i < tp.length; i += 3) {
  if (tp[i + 1] < tuft.facts.maxY * 0.9) continue;
  bearings.add(Math.round(Math.atan2(tp[i + 2], tp[i]) * 2) / 2);
}
check(
  bearings.size >= 3,
  `a tuft's tips point in ${bearings.size} directions — it is not splayed`,
);

// Determinism: two builds of the same row are bit-identical. Nothing here
// seeds an RNG today, and this is what notices the day something does.
for (const { row, geo } of built) {
  const again = row.geo();
  const a = attr(geo, "position").array;
  const b = attr(again, "position").array;
  check(a.length === b.length, `two builds of ${row.name} differ in size`);
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) fail(`${row.name} is not deterministic: float ${i} moved`);
  }
}

// ── §4 The fleet budget, ASSERTED ─────────────────────────────────────────
//
// The worst case is a uniform biome: one kind fills the whole ring. Clutter
// casts no shadow (clutter.js, choice 3), so it is drawn once per frame and
// not once per cascade — that is the whole reason the number below fits.
//
// The ceiling is a fifth of `DESIGN.md` §9's 1.5 M frame budget. Not a
// measurement of what is left over — a declared share, so that the day the
// terrain or the forest grows, this population is not what silently pushed
// the frame over. `browser_smoke` measures the real peak; this bounds the
// contribution before it gets there.
const FRAME_TRI_BUDGET = 1_500_000;
const CLUTTER_TRI_SHARE = 0.2;
const worstKind = built.reduce((a, b) => (b.facts.tris > a.facts.tris ? b : a));
const fleet = worstKind.facts.tris * CLUTTER_POOL_CAP;
check(
  fleet <= FRAME_TRI_BUDGET * CLUTTER_TRI_SHARE,
  `a full ring of ${worstKind.row.name} is ${worstKind.facts.tris} tris × ${CLUTTER_POOL_CAP} = ` +
    `${(fleet / 1000).toFixed(0)} k triangles, over the ${(
      (FRAME_TRI_BUDGET * CLUTTER_TRI_SHARE) /
      1000
    ).toFixed(0)} k share (${CLUTTER_TRI_SHARE * 100}% of DESIGN §9's ${
      FRAME_TRI_BUDGET / 1000
    } k) this population is allowed`,
);
check(
  built.length <= 8,
  `${built.length} clutter kinds is ${built.length} draw calls a frame against a 300 budget`,
);

console.log(
  `clutter: ${built.map((b) => `${b.row.name} ${b.facts.tris}t`).join(" · ")} · ring ` +
    `${2 * CLUTTER_RING + 1}² tiles of ${CLUTTER_TILE_M} m (${reach} m guaranteed, rule 4 wants ` +
    `${RULE4_NEAR_M}) · cap ${CLUTTER_POOL_CAP}/kind · worst fleet ${(fleet / 1000).toFixed(
      0,
    )} k tris = ${((100 * fleet) / FRAME_TRI_BUDGET).toFixed(1)}% of the frame budget`,
);
console.log(`clutter shape: ${checks} checks passed`);
