#!/usr/bin/env node
// Gate: the terrain bump's surface gradient is solved in WORLD XZ and never
// reads the triangle, so a smooth-shaded heightfield does not render its own
// triangulation.
//
// Why this is arithmetic and not a screenshot. The defect is a discontinuity
// in a formula, and a formula can be evaluated exactly — twice, on either side
// of a triangle edge — for nothing, on any box, with no GPU, no shard and no
// threshold that drifts with a driver. The image it produced is what named it
// (pass 20260802-050932-01, `04-ground-down.png`: scanning the sand at y=500
// gives a ~5-level ramp on a ~58 px period with a hard reset at each edge, and
// a blind reader called it "large low-poly facets readable across the
// surface") but the image is evidence, not the assertion.
//
// The mechanism. `normal_fragment_maps` perturbs the shading normal by the
// surface gradient of a procedural height gmH, reconstructed from screen
// derivatives. What shipped reconstructed it over `dFdx(vGmPos)` and
// `dFdy(vGmPos)` — vectors that span the TRIANGLE's flat plane — so the
// gradient came back tangent to whichever facet the fragment sat on. Adjacent
// facets of a smooth-shaded heightfield are tilted against each other while
// the shading normal is not, so the bump jumped at every edge by (facet tilt)
// × (bump slope). A sawtooth locked to the mesh.
//
// So this suite builds one quad of a heightfield — two triangles with
// genuinely different facet planes, sharing an edge along which the SMOOTH
// normal is continuous by construction — and evaluates each formulation at
// the same world point from each triangle's derivatives. The shared edge is
// the whole test: a correct reconstruction returns the same perturbed normal
// from either side, because the gradient of a continuous field is a property
// of the field and not of which triangle you asked from.
//
// THREE formulations, not two, and the middle one is the point. Rebuilding the
// basis on the geometric normal is the textbook correction, it is what this
// pass tried first, and it is not a fix — measured here at 0.325° against the
// retired 0.422°, because it still projects onto the same tilted facet.
// Anything that reads the triangle inherits the seam. That check is asserted
// so the shipped solve does not get "simplified" back into a cross product on
// the strength of the cross product being the familiar form.
//
// The JS below mirrors GLSL, so the last checks pin the mirror: the shipped
// shader source must contain the world-XZ solve and must contain neither
// retired form. A mirror nobody pinned is a second implementation that agrees
// with the first only until someone edits one of them.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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

// --- vec3, enough of it -----------------------------------------------------
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const scale = (a, k) => [a[0] * k, a[1] * k, a[2] * k];
const len = (a) => Math.sqrt(dot(a, a));
const norm = (a) => scale(a, 1 / len(a));
const sign = (x) => (x > 0 ? 1 : x < 0 ? -1 : 0);

// --- the surface under test -------------------------------------------------
// A heightfield with curvature, so adjacent quads have genuinely different
// facet planes and the smooth normal genuinely differs from every one of them.
// The cross term is the load-bearing one and the first cut of this suite left
// it out: a SEPARABLE height f(x) + g(z) makes every quad exactly planar
// (h(a) + h(d) == h(b) + h(c) identically), both triangles share one facet,
// and the seam this gate exists to measure is zero in both formulations. The
// suite caught that itself, through the coplanarity check below — which is
// why that check is there and not an afterthought.
// The cross term's amplitude and frequency are chosen so the quad's two
// triangles sit ~7° apart — ordinary heightfield curvature over a 1 m step,
// not a spike built to make the defect look large.
const terrain = (x, z) =>
  0.6 * Math.sin(x * 0.7) + 0.45 * Math.cos(z * 0.9) + 0.2 * x + 0.8 * Math.sin(x * 1.3) * Math.cos(z * 1.6);
const terrainNormal = (x, z) => {
  const h = 1e-5;
  const dx = (terrain(x + h, z) - terrain(x - h, z)) / (2 * h);
  const dz = (terrain(x, z + h) - terrain(x, z - h)) / (2 * h);
  return norm([-dx, 1, -dz]);
};
const P = (x, z) => [x, terrain(x, z), z];

// The procedural bump height, a continuous function of world XZ — as gmH is.
const gmH = (p) => 0.05 * Math.sin(p[0] * 2.3) * Math.cos(p[2] * 1.9);
// …and its exact world gradient, which is what the shipped solve claims to
// recover from screen derivatives alone.
const gmGradH = (p) => [
  0.05 * 2.3 * Math.cos(p[0] * 2.3) * Math.cos(p[2] * 1.9),
  -0.05 * 1.9 * Math.sin(p[0] * 2.3) * Math.sin(p[2] * 1.9),
];

// --- the two formulations ---------------------------------------------------
// Retired: the basis is built on the shading normal.
function perturbOnShadingNormal(n, dpdx, dpdy, dHdx, dHdy) {
  const R1 = cross(dpdy, n);
  const R2 = cross(n, dpdx);
  const det = dot(dpdx, R1);
  const grad = scale(
    [
      dHdx * R1[0] + dHdy * R2[0],
      dHdx * R1[1] + dHdy * R2[1],
      dHdx * R1[2] + dHdy * R2[2],
    ],
    sign(det),
  );
  return norm(sub(scale(n, Math.abs(det)), grad));
}

// Also retired, and kept because it is the correction everyone reaches for
// first: the same formula with the basis rebuilt on the GEOMETRIC normal.
// It is textbook-correct and it does not fix this, because it still projects
// onto the triangle's plane. The suite asserts that below rather than leaving
// it as a claim — it is the reason the shipped formulation is a solve and not
// a cross product.
function perturbOnGeometricNormal(n, dpdx, dpdy, dHdx, dHdy) {
  const ngRaw = cross(dpdx, dpdy);
  const area = Math.max(len(ngRaw), 1e-12);
  const ng = scale(ngRaw, 1 / area);
  const R1 = cross(dpdy, ng);
  const R2 = cross(ng, dpdx);
  const surf = scale(
    [
      dHdx * R1[0] + dHdy * R2[0],
      dHdx * R1[1] + dHdy * R2[1],
      dHdx * R1[2] + dHdy * R2[2],
    ],
    1 / area,
  );
  return norm(sub(n, surf));
}

// Shipped: solve for the gradient in world XZ, then add the two heightfields.
// Nothing in it reads the triangle, which is why it is exact.
function perturbInWorldXZ(n, dpdx, dpdy, dHdx, dHdy) {
  const dx = [dpdx[0], dpdx[2]];
  const dy = [dpdy[0], dpdy[2]];
  const det = dx[0] * dy[1] - dx[1] * dy[0];
  const invDet = Math.abs(det) > 1e-14 ? 1 / det : 0;
  const surf = [
    (dHdx * dy[1] - dHdy * dx[1]) * invDet,
    (dHdy * dx[0] - dHdx * dy[0]) * invDet,
  ];
  const ny = Math.max(n[1], 1e-3);
  return norm([n[0] - surf[0] * ny, ny, n[2] - surf[1] * ny]);
}

// --- the shared edge --------------------------------------------------------
// One quad, split the way terrainWorker.js splits it (a,c,b / b,c,d), sampled
// at a point ON the shared diagonal. Each triangle supplies its own screen
// derivatives — which is exactly what dFdx/dFdy hand a fragment — while the
// smooth normal and the height field are the same on both sides.
const S = 1.0; // quad size in metres, the near ring's own vertex step order
const x0 = 3.0;
const z0 = 5.0;
const a = P(x0, z0);
const b = P(x0 + S, z0);
const c = P(x0, z0 + S);
const d = P(x0 + S, z0 + S);

// Triangle 1 (a, c, b) and triangle 2 (b, c, d) meet along c–b.
const tri1 = { e1: sub(c, a), e2: sub(b, a) };
const tri2 = { e1: sub(c, b), e2: sub(d, b) };

// The point on the shared edge both triangles are asked about.
const mid = scale([c[0] + b[0], c[1] + b[1], c[2] + b[2]], 0.5);
const nSmooth = terrainNormal(mid[0], mid[2]);

// Screen derivatives: a fragment's dFdx/dFdy are the triangle's edge vectors
// scaled by however many pixels it covers. The scale is deliberately DIFFERENT
// per triangle — a correct basis is invariant to it, and the retired one is not.
const deriv = (tri, k) => ({ dpdx: scale(tri.e1, k), dpdy: scale(tri.e2, k) });
const px1 = deriv(tri1, 0.02);
const px2 = deriv(tri2, 0.031);

// The height field's screen derivatives follow from the same footprints.
//
// Two ways of forming them, and the suite needs both. EXACT is the chain rule
// — ∇gmH dotted with the screen step — and it isolates the FORMULATION: given
// screen derivatives that are consistent with the footprint, does the formula
// return the world gradient or something that remembers the triangle? FINITE
// is what a GPU actually hands a fragment, a difference over one pixel, and it
// carries a truncation error that every formulation shares and that shrinks
// with the footprint. Asserting exact continuity against FINITE would be
// asserting that a finite difference is a derivative.
const hExact = (p, dp) => {
  const g = gmGradH(p);
  return g[0] * dp[0] + g[1] * dp[2];
};
const hFinite = (p, dp) => gmH([p[0] + dp[0], p[1] + dp[1], p[2] + dp[2]]) - gmH(p);
const derivsOf = (h, px) => ({ dHdx: h(mid, px.dpdx), dHdy: h(mid, px.dpdy) });
const H1 = derivsOf(hExact, px1);
const H2 = derivsOf(hExact, px2);
const F1 = derivsOf(hFinite, px1);
const F2 = derivsOf(hFinite, px2);

const angleBetween = (p, q) =>
  (Math.acos(Math.max(-1, Math.min(1, dot(p, q)))) * 180) / Math.PI;

const seamOf = (f, d1 = H1, d2 = H2) =>
  angleBetween(
    f(nSmooth, px1.dpdx, px1.dpdy, d1.dHdx, d1.dHdy),
    f(nSmooth, px2.dpdx, px2.dpdy, d2.dHdx, d2.dHdy),
  );

const oldSeam = seamOf(perturbOnShadingNormal);
const geoSeam = seamOf(perturbOnGeometricNormal);
const newSeam = seamOf(perturbInWorldXZ);
// The same three over a real one-pixel finite difference, which is what the
// hardware supplies.
const oldSeamFd = seamOf(perturbOnShadingNormal, F1, F2);
const newSeamFd = seamOf(perturbInWorldXZ, F1, F2);

// The facet planes really are different, or this quad proves nothing.
const facet1 = norm(cross(tri1.e1, tri1.e2));
const facet2 = norm(cross(tri2.e1, tri2.e2));
check(
  angleBetween(facet1, facet2) > 0.5,
  `the test quad's two triangles are coplanar to ${angleBetween(facet1, facet2).toFixed(3)}° — ` +
    `it cannot show a facet seam either way`,
);
check(
  angleBetween(facet1, nSmooth) > 0.5,
  `the smooth normal sits within ${angleBetween(facet1, nSmooth).toFixed(3)}° of the facet — ` +
    `the two bases would agree for a reason this suite is not testing`,
);

// The defect, reproduced: the retired basis returns two different normals for
// one point on one continuous surface.
// The floor is 0.25° because that is roughly where a normal seam stops being
// visible: at mid grey and mid incidence it moves the shading term by ~0.4%,
// about one 8-bit level, against the ~5 levels the frame actually showed.
check(
  oldSeam > 0.25,
  `the retired shading-normal basis is continuous across the edge to ${oldSeam.toFixed(4)}° — ` +
    `this suite no longer reproduces the defect it exists to keep fixed, so its pass means nothing`,
);
// And the correction that looks like the fix and is not — asserted, so nobody
// "simplifies" the solve back into a cross product on the strength of it
// being the textbook form. It still projects onto the facet.
check(
  geoSeam > oldSeam / 10,
  `rebuilding the basis on the geometric normal now cures the seam (${geoSeam.toFixed(4)}° against ` +
    `the retired ${oldSeam.toFixed(4)}°) — if that is really true the shipped solve is more machinery ` +
    `than this needs, so re-derive it rather than trusting this line`,
);
// The fix: the shipped formulation returns one normal, to floating point.
check(
  newSeam < 1e-6,
  `the world-XZ gradient differs by ${newSeam.toFixed(6)}° across a shared edge — the bump still ` +
    `depends on which triangle asked, which is the mesh rendering itself as shading`,
);
check(
  newSeam < oldSeam / 1000,
  `the shipped gradient's seam (${newSeam.toExponential(2)}°) is not decisively below the retired ` +
    `one's (${oldSeam.toFixed(4)}°)`,
);
// And the practical claim, on the finite difference a GPU really supplies:
// what survives is truncation, shared by every formulation and shrinking with
// the footprint — so it must be a small fraction of what the retired basis
// left behind, not merely smaller.
check(
  newSeamFd < oldSeamFd / 3,
  `over a real one-pixel finite difference the shipped seam is ${newSeamFd.toFixed(4)}° against the ` +
    `retired ${oldSeamFd.toFixed(4)}° — less than the 3× margin that separates a fixed sawtooth from ` +
    `a slightly quieter one`,
);

// Invariance to the pixel footprint alone, with the facet held fixed: a
// surface gradient is a property of the surface, not of how many pixels the
// triangle happens to cover.
const zoomA = perturbInWorldXZ(nSmooth, px1.dpdx, px1.dpdy, H1.dHdx, H1.dHdy);
const far = deriv(tri1, 0.002);
const Hfar = derivsOf(hExact, far);
const zoomB = perturbInWorldXZ(nSmooth, far.dpdx, far.dpdy, Hfar.dHdx, Hfar.dHdy);
check(
  angleBetween(zoomA, zoomB) < 0.35,
  `the shipped basis moved the normal ${angleBetween(zoomA, zoomB).toFixed(3)}° when only the pixel ` +
    `footprint changed — the bump is reading the camera and not the surface`,
);

// --- the SECOND defect in these lines: a quad-constant gradient -------------
//
// Everything above is about continuity across a triangle edge, and the shipped
// solve is exact there. It is still reconstructed from `dFdx`/`dFdy`, and those
// are differences taken across the rasterizer's 2x2 quad — so the perturbed
// normal is CONSTANT over each quad and can only change at a quad boundary. No
// formulation fixes that; it is the technique.
//
// What it costs depends entirely on how fast the height field turns over. Over
// the 2 px between quad centres a wave of wavelength L advances `2h/L` of a
// cycle — `4*PI*cpp` radians of phase — so the reconstructed gradient lands
// that far along its own cycle each quad, and successive values differ by
// `2*sin(phase/2)` of the gradient's own amplitude. Below a few degrees that
// reads as relief. Approaching 60 degrees the gradient changes by more than
// the whole of itself between neighbours, and the ground renders a 2x2 mosaic
// — which is what the visual judge's pass 20260802-163821-01 called "a literal
// checkerboard alternating dark teal and khaki", measured on its own
// `05-held-level.png` at 1.9 luma/px of neighbour contrast within quads
// against 21.4 across them.
//
// So this builds the reconstruction — sample gmH at real quad corners, take
// the one-pixel difference each quad actually gets, and compare neighbours —
// and asserts the shipped band is inside the bound while the ALBEDO band, the
// one the bump was faded on before `FADE_BUMP_CPP`, is not. Same discipline as
// the retired formulations above: a suite that cannot still show the defect
// has not proved it would catch it.
const FOUR_PI = 4 * Math.PI;
// The jump between adjacent quads' reconstructed gradients, as a fraction of
// the true gradient's own amplitude, for a unit sinusoid sampled at `cpp`
// cycles per pixel. Built, not assumed: quads start at even pixels, each takes
// the fine difference across its own two columns, and the worst neighbouring
// pair over a full cycle is what is reported.
function quadJump(cpp) {
  const lambda = 1;
  const h = cpp * lambda; // pixel footprint in the wave's own units
  const wave = (x) => Math.sin((2 * Math.PI * x) / lambda);
  const amp = (2 * Math.PI) / lambda; // peak |d wave / dx|
  // The quad grid is not aligned to the wave, so the answer is the worst any
  // alignment gives — swept, not assumed, because a coarse grid at one lucky
  // offset can miss the peak by a lot and would understate the defect.
  const quads = Math.max(8, Math.ceil(lambda / (2 * h)) + 2);
  let worst = 0;
  for (let o = 0; o < 2048; o++) {
    const off = (o / 2048) * 2 * h;
    let prev = null;
    for (let k = 0; k < quads; k++) {
      const x0 = off + 2 * k * h;
      const g = (wave(x0 + h) - wave(x0)) / h;
      if (prev !== null) worst = Math.max(worst, Math.abs(g - prev));
      prev = g;
    }
  }
  return worst / amp;
}
// The closed form `materials.js` states, so the prose is pinned to the
// arithmetic rather than sitting beside it. Two factors, and both are the
// derivation: `2·sin(2π·cpp)` is how far apart two samples of a wave a phase
// of `4π·cpp` apart can be, and `sinc(cpp)` is the attenuation the one-pixel
// finite difference applies to the gradient it recovers.
const closedForm = (cpp) =>
  2 * Math.sin(2 * Math.PI * cpp) * (Math.sin(Math.PI * cpp) / (Math.PI * cpp));
for (const cpp of [0.005, 0.0208, 0.05, 0.09]) {
  const built = quadJump(cpp);
  const closed = closedForm(cpp);
  check(
    Math.abs(built - closed) <= 1e-4 * closed,
    `at ${cpp} cycles/pixel the built quad-to-quad gradient jump is ${built.toFixed(6)} of amplitude ` +
      `against the ${closed.toFixed(6)} that 2·sin(2π·cpp)·sinc(cpp) predicts — the derivation in ` +
      `materials.js no longer describes what the reconstruction does`,
  );
}
// What the two ends of the ladder cost, so the number that names the defect is
// derived here rather than asserted from a screenshot. The albedo band's end
// is read off the material — it is what gmH is faded on TODAY — and the bound
// a bump needs is stated as a phase per quad, in degrees, which is the unit
// the fix will carry.
const matSrc = fs.readFileSync(path.join(ROOT, "web/src/materials.js"), "utf8");
const albedoEndM = matSrc.match(/\bconst\s+FADE_OCTAVE_CPP\s*=\s*([^;]+);/);
if (!albedoEndM) fail("web/src/materials.js has no const FADE_OCTAVE_CPP — this gate reads the shipped law from it");
const albedoEnd = JSON.parse(albedoEndM[1].replace(/\s+/g, ""))[1];
// Hold the gradient's phase step under 15° per quad and it jumps ~26% of its
// own amplitude between neighbours — relief. This is the bound `NOW.md` item 1
// carries as the fix; it is asserted here as arithmetic so the number cannot
// drift while the prose stays.
const BUMP_PHASE_BOUND_DEG = 15;
const bumpEnd = ((BUMP_PHASE_BOUND_DEG * Math.PI) / 180) / FOUR_PI;
const boundedJump = quadJump(bumpEnd);
const albedoJump = quadJump(albedoEnd);
check(
  bumpEnd < albedoEnd,
  `a ${BUMP_PHASE_BOUND_DEG}° phase bound puts the bump's retirement at ${bumpEnd.toFixed(4)} cycles/pixel ` +
    `and the albedo's is ${albedoEnd} — if the bound were the looser of the two there would be nothing to fix`,
);
check(
  boundedJump <= 0.3,
  `at the ${BUMP_PHASE_BOUND_DEG}° bound the reconstructed gradient still jumps ` +
    `${(boundedJump * 100).toFixed(1)}% of its own amplitude between adjacent quads — over 30% and the quad ` +
    `grid is what the player sees, so the bound is not a bound`,
);
// The defect, reproduced as arithmetic: gmH is faded on the ALBEDO band today,
// and at that band's end the gradient changes by more than the whole of itself
// between neighbouring quads. That is the 2x2 mosaic the visual judge named on
// pass 20260802-163821-01 and `scene.aliasProbe` measures at x3.12/x6.15.
check(
  albedoJump >= 1.0,
  `at the albedo band's end the quad-to-quad gradient jump is only ${(albedoJump * 100).toFixed(1)}% of ` +
    `amplitude — this suite no longer reproduces the defect that makes the bump need a band of its own, so ` +
    `the bound above is unmotivated and its pass means nothing`,
);
check(
  albedoJump > boundedJump * 3,
  `the two bands' quad jumps are ${boundedJump.toFixed(3)} and ${albedoJump.toFixed(3)} — not decisively ` +
    `apart, so splitting the laws would buy nothing`,
);

// --- the mirror is pinned to the shader -------------------------------------
const src = fs.readFileSync(path.join(ROOT, "web/src/materials.js"), "utf8");
// Comments stripped first, and not as a tidiness measure: the block above is
// commented with the names of the formulations it retired, so a pin that read
// prose would report the retired basis as still shipping. It did, on the first
// run of this gate.
const block = src
  .slice(
    src.indexOf("#include <normal_fragment_maps>"),
    src.indexOf("Specular AA on what we just perturbed"),
  )
  .replace(/^\s*\/\/.*$/gm, "");
check(block.length > 200, "could not find the bump block in web/src/materials.js — this gate is not reading the shader");
check(
  /dFdx\(\s*vGmPos\.xz\s*\)/.test(block) && /gmDx\.x\s*\*\s*gmDy\.y/.test(block),
  "the shipped bump block does not solve for the gradient in world XZ — the mirror above is testing " +
    "a formula the shader does not use",
);
check(
  !/cross\(/.test(block),
  "the shipped bump block builds a basis with a cross product — every such basis spans the " +
    "TRIANGLE's plane and carries the facet seam this gate measures, whichever normal it is built on",
);
check(
  !/dFdx\(\s*vGmPos\s*\)/.test(block),
  "the shipped bump block still takes dFdx of the full world position — that is the retired " +
    "formulation, and this suite's own reproduction above shows what it does",
);
check(
  /min\(\s*gmSlope\s*,/.test(block),
  "the shipped bump block does not cap its surface gradient — a screen derivative over a screen " +
    "footprint is unbounded (CLAUDE.md wall 4)",
);

console.log(
  `bump basis: exact-derivative seam — retired ${oldSeam.toFixed(3)}°, geometric-normal ` +
    `${geoSeam.toFixed(3)}° (not a fix), shipped ${newSeam.toExponential(2)}° · one-pixel finite ` +
    `difference ${oldSeamFd.toFixed(3)}° → ${newSeamFd.toFixed(3)}° · footprint invariance ` +
    `${angleBetween(zoomA, zoomB).toFixed(3)}° · quad-constant gradient: jump ` +
    `${(boundedJump * 100).toFixed(1)}% of amplitude at a ${BUMP_PHASE_BOUND_DEG}°/quad bound ` +
    `(${bumpEnd.toFixed(4)} cpp) against ${(albedoJump * 100).toFixed(1)}% at the albedo band's ` +
    `${albedoEnd}, which is what gmH is faded on today · ${checks} checks passed`,
);
