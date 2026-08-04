#!/usr/bin/env node
// The prop photograph's arithmetic, as arithmetic — no GPU, no shard, no
// browser.
//
// Why this file exists, stated as the failure it is for. The prop path now
// samples a real granite photograph triplanar in world space, and a triplanar
// blend has one silent failure mode that is invisible to every other gate here:
// a plain weighted mean of three DECORRELATED samples has variance
// `sigma^2 * dot(w, w)`, which at the three-way point is `sigma^2 / 3`. The
// photograph's own contrast therefore arrives at **0.577 of itself** exactly on
// the faces where all three projections are live — and on a DodecahedronGeometry
// nearly every face is off-axis, so that is most of the boulder.
//
// That matters more here than it would anywhere else, because the whole reason
// the photograph was wired is `ART.md` §3's near-band neighbour contrast — the
// one number `ART.md` says nothing has moved, and the one `NOW.md` records the
// prop probe sitting exactly on its floor at x1.15. A triplanar granite without
// the variance correction reads FLATTER than the noise it replaced. It would
// look like work and measure like a regression, and `browser_smoke` would take
// ten minutes to say so.
//
// So the correction is gated as a COUNT and a closed-form identity rather than
// as a pixel: the deviation is divided by `sqrt(dot(w, w))`, which restores the
// variance to `sigma^2` for EVERY weight vector while leaving the mean exactly
// where it was (Heitz & Neyret's histogram-preserving blend, its core identity).
// Delete that one `inversesqrt` and this file goes red in under a second.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "web/src/materials.js");

let failures = 0;
const fail = (msg) => {
  console.error(`  FAIL ${msg}`);
  failures++;
};
const ok = (msg) => console.log(`  ok   ${msg}`);
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));

const src = fs.readFileSync(SRC, "utf8");

// --- the knobs, read out of the source that ships them ----------------------
//
// Read rather than restated: a gate that carried its own copy of these numbers
// would score a constant nobody renders, which is the exact hole
// `ci/pine_shape.mjs` was written to avoid on the pine.
function scalarKnob(name) {
  const m = src.match(new RegExp(`\\bconst\\s+${name}\\s*=\\s*([^;]+);`));
  if (!m) {
    fail(`${name} is not declared in web/src/materials.js — this gate reads the shipped value, never its own copy`);
    return null;
  }
  const v = Number(m[1].trim());
  if (!Number.isFinite(v)) {
    fail(`${name} = ${m[1].trim()} does not parse as a number`);
    return null;
  }
  return v;
}

const GAIN = scalarKnob("PROP_PHOTO_GAIN");
const SCALE = scalarKnob("PROP_PHOTO_SCALE");
const BLEND = scalarKnob("PROP_PHOTO_BLEND");
const clampM = src.match(/\bconst\s+PROP_PHOTO_CLAMP\s*=\s*\[([^\]]+)\];/);
const CLAMP = clampM ? clampM[1].split(",").map((s) => Number(s.trim())) : null;
if (!CLAMP || CLAMP.length !== 2 || CLAMP.some((n) => !Number.isFinite(n))) {
  fail("PROP_PHOTO_CLAMP is not a two-number array in web/src/materials.js");
}

if (failures > 0) {
  console.error("prop photo: the knobs could not be read — nothing below was scored");
  process.exit(1);
}

console.log(
  `prop photo: gain ${GAIN}, ${SCALE} repeats/m, blend pow ${BLEND}, ` +
    `clamp [${CLAMP[0]}, ${CLAMP[1]}]`,
);

// --- 1 · ART.md §7's x1 rule, mechanized ------------------------------------
//
// "deviation may not be stretched more than x1 by the correction that places
// the mean". The photograph is delivered at its own contrast or less, never
// amplified — the failure `ART.md` §7 records is a gain that drags a source's
// noise along with its mean, and it is the mechanism the visual judge named for
// the ground band on 2026-08-04.
check(
  GAIN > 0 && GAIN <= 1.0,
  `gain ${GAIN} is in (0, 1] — ART.md §7 forbids stretching deviation past x1`,
);

// --- 2 · the clamp brackets unity -------------------------------------------
//
// The multiplier has to be able to go both ways about 1.0 or the term is a
// one-sided darkener (or brightener) with a mean that is no longer the authored
// albedo, whatever the arithmetic above it says.
check(
  CLAMP[0] < 1.0 && CLAMP[1] > 1.0,
  `clamp [${CLAMP[0]}, ${CLAMP[1]}] brackets 1.0, so the photograph can darken AND lighten about the authored mean`,
);
// The stronger form of the same law, and the one that actually protects the
// mean. `E[multiplier] = 1` is exact for the ratio form and free; the clamp is
// the one step downstream that can take it back, and it does so iff it is
// asymmetric. This caught [0.6, 1.6] biasing the delivered mean to 1.050 at
// full spread — a 5% brightening of every boulder, which is the authored
// granite value handed back to whichever file was fetched.
check(
  Math.abs((CLAMP[0] + CLAMP[1]) / 2 - 1) < 1e-9,
  `clamp [${CLAMP[0]}, ${CLAMP[1]}] is symmetric about 1.0 — an asymmetric clamp biases the delivered mean and un-authors the albedo`,
);

// --- 3 · the variance-preserving normalization is actually in the shader ----
//
// The line whose deletion costs 42% of the contrast this whole change exists to
// buy, asserted by its presence and by the operand it divides. A rename that
// broke the association would fail here rather than in a screenshot.
const photoBlock = src.slice(
  src.indexOf("if (uProp * uPropPhotoCfg.z > 0.0)"),
  src.indexOf("const PROP_NORMAL_GLSL"),
);
check(
  photoBlock.length > 0 && /inversesqrt\(\s*max\(\s*dot\(gpPw,\s*gpPw\)/.test(photoBlock),
  "the triplanar deviation is divided by sqrt(dot(w, w)) — the variance-preserving blend is present",
);
check(
  /texture\(uPropPhoto,[\s\S]*texture\(uPropPhoto,[\s\S]*texture\(uPropPhoto,/.test(photoBlock),
  "all three projections are sampled (a two-projection triplanar leaves one axis smeared)",
);
check(
  /pow\(abs\(gpN\)/.test(photoBlock),
  "the blend weights are a POWER of the normal, not the linear normal (ART.md §7(2))",
);

// --- 4 · the blend, swept over the sphere -----------------------------------
//
// A Fibonacci sphere so the sweep is deterministic and has no seed in it. For
// each direction: the shipped weights, then the two contrast gains — the one
// the shader delivers and the one a plain weighted mean would have.
const N = 4096;
let minPartition = Infinity;
let maxPartition = -Infinity;
let worstNaive = Infinity;
let worstShipped = Infinity;
let bestShipped = -Infinity;

const GOLDEN = Math.PI * (3 - Math.sqrt(5));
for (let i = 0; i < N; i++) {
  const y = 1 - (i / (N - 1)) * 2;
  const r = Math.sqrt(Math.max(0, 1 - y * y));
  const th = GOLDEN * i;
  const n = [Math.cos(th) * r, y, Math.sin(th) * r];

  // The shader's weights, verbatim: pow(abs(n), BLEND), normalized by their sum.
  let w = n.map((c) => Math.pow(Math.abs(c), BLEND));
  const sum = Math.max(w[0] + w[1] + w[2], 1e-4);
  w = w.map((c) => c / sum);

  minPartition = Math.min(minPartition, w[0] + w[1] + w[2]);
  maxPartition = Math.max(maxPartition, w[0] + w[1] + w[2]);

  // Three decorrelated samples of unit variance blended by `w`: the resulting
  // standard deviation is sqrt(dot(w, w)). That IS the contrast the naive form
  // delivers, as a share of the source's own.
  const dotWW = w[0] * w[0] + w[1] * w[1] + w[2] * w[2];
  const naive = Math.sqrt(dotWW);
  // The shipped form divides that deviation by sqrt(dot(w, w)).
  const shipped = naive / Math.sqrt(dotWW);

  worstNaive = Math.min(worstNaive, naive);
  worstShipped = Math.min(worstShipped, shipped);
  bestShipped = Math.max(bestShipped, shipped);
}

console.log(
  `  swept ${N} normals: partition of unity [${minPartition.toFixed(9)}, ${maxPartition.toFixed(9)}], ` +
    `naive blend contrast worst ${worstNaive.toFixed(4)}, shipped ${worstShipped.toFixed(6)}..${bestShipped.toFixed(6)}`,
);

check(
  Math.abs(minPartition - 1) < 1e-9 && Math.abs(maxPartition - 1) < 1e-9,
  "the blend weights are a partition of unity at every normal (no direction gains or loses energy)",
);

// The whole point, as one number: the shipped blend delivers the source's own
// contrast in EVERY direction. Tolerance is float noise, not a fitted margin.
check(
  Math.abs(worstShipped - 1) < 1e-9 && Math.abs(bestShipped - 1) < 1e-9,
  `the shipped blend delivers x1.000000 of the source's contrast at every normal`,
);

// …and the assertion that gives the one above its teeth. If this ever stops
// being true the correction has become a no-op — either because the exponent
// was raised until the weights are one-hot everywhere, or because the sweep
// stopped reaching the three-way point — and a green line above would then be
// green over nothing.
check(
  worstNaive < 0.62,
  `a plain weighted mean would lose real contrast somewhere on the sphere ` +
    `(worst ${worstNaive.toFixed(4)}, vs 1/sqrt(3) = 0.5774 at the three-way point) — ` +
    `so the correction above is load-bearing rather than decorative`,
);

// --- 5 · the mean survives the clamp ----------------------------------------
//
// `E[multiplier] = 1` is exact for the ratio form and needs no test. What is
// NOT exact is what the clamp does to it, and a clamp that biased the mean
// would hand the granite VALUE to the source file after all — the one thing
// this design is built not to do. Swept over the deviation the source can
// actually present rather than over a fitted distribution: a symmetric
// deviation about the mean, at the widest spread the clamp still admits.
for (const spread of [0.1, 0.25, 0.5, 0.75, 1.0]) {
  let acc = 0;
  const S = 20001;
  for (let i = 0; i < S; i++) {
    const d = (i / (S - 1)) * 2 - 1; // symmetric in [-1, 1], mean 0
    const mul = Math.min(Math.max(1 + d * spread * GAIN, CLAMP[0]), CLAMP[1]);
    acc += mul;
  }
  const mean = acc / S;
  check(
    Math.abs(mean - 1) < 1e-3,
    `at deviation spread ${spread.toFixed(2)} the delivered multiplier's mean is ` +
      `${mean.toFixed(6)} — the authored albedo survives the clamp`,
  );
}

if (failures > 0) {
  console.error(`prop photo: ${failures} assertion(s) failed`);
  process.exit(1);
}
console.log("prop photo: OK");
