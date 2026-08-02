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
    `${angleBetween(zoomA, zoomB).toFixed(3)}° · ${checks} checks passed`,
);
