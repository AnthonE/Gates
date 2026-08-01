// Materials v0 — the surface read (DECISIONS.md §open, "materials v0").
//
// Lighting v0 gave the world shape; it had no *surface*. Everything was
// MeshLambertMaterial at a flat colour: no roughness anywhere, no detail
// normal, and the terrain was a per-vertex biome colour with a per-vertex
// tint jitter — so a hillside was one flat wash and a beach met a meadow on
// a smooth ramp.
//
// This is the splat material TERRAIN.md §4 has always specified ("one splat
// shader, four texture sets, blended in-shader from height, slope, and a
// noise channel recomputed in GLSL — no splatmap textures, no extra
// bandwidth"), built without textures at all: four authored PBR identities
// (sand · grass · forest litter · rock) blended by weights the worker
// derives from the SIM's own (height, moisture, slope), and broken up,
// mottled, roughened and bump-lit by ONE shared noise field evaluated three
// times per pixel.
//
// The graph order is `threejs-procedural-materials`' (MIT, Scott Sun —
// credited in CLAUDE.md; guidance only, no code from the pack ships here):
//
//   stable coordinates (world xz)
//     → structural fields (three octaves of one value-noise field)
//     → material identity weights (the splat attribute, broken up)
//     → causal modifiers (wetness at the waterline, snow on high rock,
//       darkening on cliff faces — each one drives colour AND roughness,
//       never a channel on its own)
//     → filtered microstructure (both bump octaves fade out by pixel
//       footprint, so no normal survives below a pixel)
//     → PBR channels (albedo, roughness, normal)
//
// The skill's failure list is the thing to keep passing: every channel here
// reads the same three noise samples, roughness is a per-identity property
// rather than a scalar afterthought, the high-frequency octave is footprint-
// faded, and the perturbed normal feeds a specular-AA term instead of a post
// pass hiding the shimmer.
//
// `uSurface` is the skill's required channel debug mode and the gate's
// handle: at 1 the field paints, at 0 it does not and the same frame renders
// with flat identities. ci/browser_smoke.mjs renders both and counts the
// pixels that moved — the only proof a material reaches the image.
//
// --- second pass: grain, the octave that only exists at arm's length --------
//
// What v0 shipped had no TEXTURE. Its finest octave is 1.7 m across, so from
// standing height the ground is smooth-shaded mottling and at arm's length it
// is nothing at all — a hillside reads, a footprint's worth of sand does not.
//
// So a fourth octave, at a wavelength the IDENTITY chooses (4 cm of sand,
// 12 cm of grass clump), with a per-identity shape (a soft stipple for sand,
// a ridged crystalline speckle for rock) and a per-identity contrast. One
// sample, not four: the scale, the shape and the contrast are each a dot
// product against the same splat weights, so they morph continuously across a
// biome boundary instead of cross-fading two grains. Like every octave above
// it, it is sampled on world XZ.
//
// `uGrain` is its own probe handle, separate from `uSurface`, because "the
// field paints" and "the surface has grain" fail differently and a gate that
// cannot separate them cannot name what broke. And unlike `uSurface` it has a
// COMPILED partner: `nograin` is the shipped program with the octave's
// instructions gone rather than weighted to zero, so `costProbe` can price the
// octave against its own run's floor. That pairing is the whole reason this
// landed on the second attempt — the first rejected grain on a ~9% frame delta
// that nothing had calibrated, and 9% turned out to be inside this box's noise.
//
// WHAT THIS PASS DELIBERATELY DOES NOT DO, and why it is written here rather
// than discovered again: at 4 cm the world-XZ projection is wrong on a slope.
// A world-XZ field on a face of upness u is stretched by 1/u along the slope,
// which at 1.7 m is a smear nobody reads as wrong and at 4 cm is a hillside
// combed downhill. The fix is to sample the grain — and only the grain —
// triplanar, and it was built and measured on the abandoned first attempt
// (`loop/m1-surface-grain`, unmerged) before being taken back out: on a 47°
// face it moved the slope-to-contour contrast ratio from 1.100 to 1.078, at
// ×1.00 overall contrast, once the ridge fold is applied per plane and the
// blend deviation restored by `1/|w|`; on level ground the weights are
// (0,1,0) exactly, so it changes nothing at all. Neither half survives in any
// tree — rebuilding it means rebuilding it, and that shape is the one that
// worked. It is `NOW.md`'s next surface item, not a line of half-built shader
// carried in the dark.

import * as THREE from "three";
import {
  clipmapFetches,
  installClipmapShadows,
  resolvedGlslChars,
} from "./shadows.js";

// --- the four identities (TERRAIN.md §4's four sets) ------------------------
// Colours are the retired vertex palette's, unchanged, so this slice changes
// the SURFACE and not the art direction: sand was C_BEACH, grass C_MEADOW,
// litter C_FOREST, and rock sits between C_HIGHLAND and C_CLIFF (whose
// midpoint is 0.505/0.495/0.49 — rock is rounded off it). They are
// working-space (linear) triples, which is what the vertex path fed too.
// `bump` is a unitless STRENGTH, not a height: the two bump octaves are
// 5.6× apart in wavelength, so one amplitude in metres cannot serve both
// (0.06 m over a 9.5 m wave is a slope of 0.006 — a surface nobody can see).
// The metres live per octave, below, and this scales them.
//
// `grain` is the second pass's per-identity trio, and the three numbers are
// what make one shared octave read as four different surfaces:
//   scale    cycles per metre of the identity's own grain — 25 /m is a 4 cm
//            sand clump, 8.3 /m a 12 cm tuft of grass.
//   contrast how far it swings albedo, ±, at full strength.
//   ridge    0 is the raw smooth field (a soft stipple), 1 is `1-|2g-1|`
//            (a ridged, crystalline speckle). Rock is nearly all ridge;
//            sand is nearly none.
export const IDENTITIES = [
  {
    name: "sand",
    color: [0.78, 0.71, 0.52],
    roughness: 0.92,
    bump: 0.35,
    grain: { scale: 25.0, contrast: 0.1, ridge: 0.15 },
  },
  {
    name: "grass",
    color: [0.35, 0.49, 0.23],
    roughness: 0.82,
    bump: 0.6,
    grain: { scale: 8.3, contrast: 0.2, ridge: 0.55 },
  },
  {
    name: "litter",
    color: [0.15, 0.33, 0.16],
    roughness: 0.88,
    bump: 0.9,
    grain: { scale: 11.0, contrast: 0.22, ridge: 0.45 },
  },
  {
    name: "rock",
    color: [0.5, 0.48, 0.46],
    roughness: 0.7,
    bump: 2.2,
    grain: { scale: 16.7, contrast: 0.14, ridge: 0.8 },
  },
];

// Field scales, in cycles per metre — one field, three octaves, shared by
// every channel below. Wavelengths ~48 m / ~9.5 m / ~1.7 m.
const SCALE_MACRO = 1 / 48;
const SCALE_MESO = 1 / 9.5;
const SCALE_MICRO = 1 / 1.7;
// How far the field may push the identity weights around before they are
// sharpened and renormalized. This is what turns a smooth biome ramp into a
// mottled boundary; the four offsets sum to zero so it redistributes rather
// than brightens.
const BLEND_BREAKUP = 0.34;
// Albedo mottling per octave (multiplicative, ±).
const MOTTLE = [0.16, 0.11, 0.07];
// Roughness variation from the micro octave (±).
const ROUGH_VAR = 0.18;
// Bump amplitude per octave, in metres, at identity strength 1. Chosen
// against each octave's own wavelength (9.5 m and 1.7 m) so both land in
// the 0.03–0.25 surface-slope band where a normal actually reads; the
// footprint fades below then retire each one as it stops resolving.
const AMP_MESO_M = 0.55;
const AMP_MICRO_M = 0.09;
const FADE_MESO = [2.0, 7.0];
const FADE_MICRO = [0.3, 1.1];
// Grain's own bump amplitude in metres and roughness swing. 0.008 m against
// the identities' 4–12 cm wavelengths is a surface slope of 0.07 (sand, bump
// 0.35) to 0.29 (rock, bump 2.2) — the same 0.03–0.25 band the two octaves
// above were chosen against, which is the band where a normal reads without
// turning into a relief map.
const AMP_GRAIN_M = 0.008;
const GRAIN_ROUGH = 0.1;
// Grain's footprint fade, in CYCLES PER PIXEL rather than metres, because its
// wavelength is a per-identity number and one metre threshold cannot serve
// four of them; every identity's grain dies on one curve, each at its own
// distance. Retiring at 0.09 under this `length(fwidth())` convention is ~11
// pixels per cycle — well inside Nyquist, so the octave is gone while it is
// still comfortably resolved rather than at the moment it stops being. That is
// deliberate on both counts: an octave that dies early cannot alias, and grain
// is the only octave whose cost scales with how far it reaches.
const FADE_GRAIN_CPP = [0.03, 0.09];
// The smallest grain scale any identity carries. Grain's own fade needs the
// WORLD footprint and the blended scale, both of which cost real work on
// every fragment in the frame; the horizontal footprint is a lower bound on
// the world one (xz is a subset of xyz), so `gmFw * this >= fade end` proves
// every identity's grain is already retired and the whole block can be
// skipped before any of it is computed. Conservative in the right direction:
// a steep face may enter the block and find the fade zero anyway.
const GRAIN_SCALE_MIN = Math.min(...IDENTITIES.map((i) => i.grain.scale));
// Specular-AA gain on the perturbed normal's variance (three already adds
// its own term from the *unperturbed* normal; this covers what we added).
const SPEC_AA = 0.5;
// Causal modifiers. Wetness: below the waterline the surface is dark and
// smooth, and it dries out over the first 1.6 m of beach — this is what
// retired the separate sea-floor palette entry (0.68 × sand lands on it).
const WET_RANGE = [-0.4, 1.6];
const WET_DARKEN = 0.68;
const WET_ROUGH = 0.28;
// Snow on high rock, replacing the palette's peak lerp; the band is the
// retired C_PEAK ramp's (52 m → 80 m) and the colour is C_PEAK itself.
const SNOW_RANGE = [52.0, 80.0];
const SNOW_COLOR = [0.72, 0.72, 0.75];
const SNOW_ROUGH = 0.55;
// Cliff faces darken. `upness` is the world normal's y: 0.643 is the sim's
// 50° cliff threshold, so this ramps in over the 39°–53° band the retired
// C_CLIFF blend covered.
const CLIFF_UPNESS = [0.6, 0.78];
const CLIFF_DARKEN = 0.86;

// --- the shared field, in GLSL ---------------------------------------------
// Hash-without-sine (Dave Hoskins' hash12): four hashes per noise sample,
// three samples per pixel. No textures, no trig, no dependent texture reads.
const FIELD_GLSL = /* glsl */ `
float gmHash(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
float gmNoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  float a = gmHash(i);
  float b = gmHash(i + vec2(1.0, 0.0));
  float c = gmHash(i + vec2(0.0, 1.0));
  float d = gmHash(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
`;

const TERRAIN_VERT_PARS = /* glsl */ `
attribute vec4 splat;
varying vec4 vGmSplat;
varying vec3 vGmPos;
varying vec3 vGmNorm;
`;

const TERRAIN_FRAG_PARS_HEAD = /* glsl */ `
varying vec4 vGmSplat;
varying vec3 vGmPos;
varying vec3 vGmNorm;
uniform float uSurface;
uniform vec3 uIdentColor[4];
uniform vec4 uIdentRough;
uniform vec4 uIdentBump;
uniform vec3 uScales;
uniform vec3 uMottle;
uniform vec4 uFade;
uniform vec2 uOct;
uniform vec3 uSnowColor;
uniform float uGrain;
uniform vec4 uGrainScale;
uniform vec4 uGrainContrast;
uniform vec4 uGrainRidge;
uniform vec2 uGrainFade;
uniform vec2 uGrainAmp;
${FIELD_GLSL}
`;

// Grain's own sample: the shared field, folded toward its ridge by the
// identity's own ridge amount, returned SIGNED (±0.5) so a soft stipple and a
// crystalline speckle sit on the same midpoint and neither biases the albedo
// it multiplies. Emitted only into the variants that carry grain — a helper
// nothing calls is still source the driver compiles, and program size is a
// budget this gate asserts.
const GRAIN_GLSL = /* glsl */ `
float gmGrainTap(vec2 uv, float s, float ridge) {
  float g = gmNoise(uv * s);
  return mix(g, 1.0 - abs(2.0 * g - 1.0), ridge) - 0.5;
}
`;

const terrainFragPars = (grain) =>
  grain === "on" ? TERRAIN_FRAG_PARS_HEAD + GRAIN_GLSL : TERRAIN_FRAG_PARS_HEAD;

// --- the cost variants (NOW.md item 1) --------------------------------------
// A uniform cannot remove an instruction. `uSurface` weights every field term
// to zero and every `gmNoise` call still runs; `uClipLevels` weights every
// shadow level to zero and every depth fetch is still taken (deliberately —
// a comparison sampler behind a per-pixel branch has undefined derivatives).
// So the two probes that prove those systems reach the image cannot say what
// either COSTS, and `NOW.md` item 1 is a question about cost: grain did not
// merge because the terrain program was already too expensive for the browser
// gate's third tab, and nothing in the tree could say which half.
//
// These compile the ground WITHOUT a term instead:
//
//   field: "off"   — the noise field gone, not zeroed. Its image is the same
//                    frame `uSurface = 0` produces (every field term is
//                    multiplied by uSurface, and 0 × finite is exactly 0), so
//                    the probe can CHECK that the variant removed only what
//                    the surface probe's toggle removes, and the time between
//                    them is then attributable to the field's instructions.
//   shadow: "near1"/"off" — shadows.js' variants, same argument.
//   grain: "off"   — the fourth octave gone: its helper, its block, its tap.
//                    Its image is the frame `uGrain = 0` produces (every grain
//                    term is multiplied by uGrain), so the same check the field
//                    gets applies to it, and the time between the two programs
//                    is then the octave's own. This is the variant `NOW.md`
//                    item 1 asked for by name: grain's first attempt rejected
//                    itself on a ~9% frame delta measured against nothing, and
//                    the control run in the same sweep is what turns a delta
//                    into a measurement or into "under the floor".
//
// Nothing the player sees is ever built from one: `makeTerrainMaterial()`
// with no argument is the shipped program, and the gate asserts its variant
// name is "ship".
// A sixth is not about cost at all. `noskip` compiles the micro octave
// UNCONDITIONALLY — the field exactly as materials v0 shipped it, before this
// slice made the sample conditional on its own footprint fade. The skip is
// bit-exact by construction (every use of that octave is multiplied by a fade
// that is exactly zero wherever the branch skips, and 0 × finite is 0), and an
// argument like that is worth precisely as much as the gate behind it. So the
// gate renders both and requires the same frame.
const TERRAIN_VARIANTS = ["ship", "nofield", "nograin", "near1", "noshadow", "noskip"];

const VARIANT_CONFIG = {
  ship: { field: "full", grain: "on", shadow: "ship" },
  // The field is one field: taking its instructions out takes the fourth
  // octave's with them, or `nofield` would be "the field minus three of its
  // four octaves" and its timing would be attributed to the wrong thing.
  nofield: { field: "off", grain: "off", shadow: "ship" },
  nograin: { field: "full", grain: "off", shadow: "ship" },
  near1: { field: "full", grain: "on", shadow: "near1" },
  noshadow: { field: "full", grain: "on", shadow: "off" },
  noskip: { field: "always", grain: "on", shadow: "ship" },
};

/** The three samples of the shared field, or the constants that replace it. */
function fieldGlsl(field) {
  if (field === "always") {
    // materials v0's own line, kept alive as the thing `ship` is checked
    // against. Not reachable from any material a player's frame is built from.
    return /* glsl */ `
        float gmMacro = gmNoise(gmXZ * uScales.x);
        float gmMeso  = gmNoise(gmXZ * uScales.y);
        float gmMicro = gmNoise(gmXZ * uScales.z);`;
  }
  if (field === "off") {
    // 0.5 is the field's own midpoint, so every `(gm* - 0.5)` term folds to
    // exactly zero and this variant lands on the `uSurface = 0` image.
    return /* glsl */ `
        float gmMacro = 0.5;
        float gmMeso  = 0.5;
        float gmMicro = 0.5;`;
  }
  return /* glsl */ `
        float gmMacro = gmNoise(gmXZ * uScales.x);
        float gmMeso  = gmNoise(gmXZ * uScales.y);
        // The micro octave is skipped where its own footprint fade has
        // already retired it. Every use of gmMicro below is multiplied by
        // gmFadeMicro, so at a fade of exactly zero its value cannot reach
        // the image and the skip is bit-exact rather than an approximation.
        // Not a derivative hazard either: every fwidth/dFdx in this shader is
        // taken outside this branch, which is why the footprint is computed
        // above the field rather than below it.
        float gmMicro = 0.0;
        if (gmFadeMicro > 0.0) gmMicro = gmNoise(gmXZ * uScales.z);`;
}

/**
 * The grain octave, or the constants that replace it.
 *
 * Grain's scale, shape and contrast are each a dot product against the same
 * splat weights, so a biome boundary morphs one grain into another rather than
 * cross-fading two of them.
 *
 * The whole block sits behind one multiply and one compare, because everything
 * in it — a second fwidth, the blended scale, the fade, the tap — is work the
 * far field would throw away. Its footprint is the WORLD one, not gmXZ's: a
 * pixel on a steep face barely moves in XZ while covering metres of surface,
 * and filtering on the horizontal footprint alone would keep a 4 cm octave
 * live at any distance on exactly the faces it reads worst on. The cheap
 * horizontal footprint is still what guards the block, because it is a lower
 * bound on the world one — conservative in the right direction.
 *
 * Nothing inside takes a derivative, and the fade it is cut on is already zero
 * at the cut, so `gmH` stays continuous across the boundary and the `dFdx(gmH)`
 * below sees no step.
 */
function grainGlsl(grain) {
  if (grain === "off") {
    // Both terms are the additive identity of what they feed (albedo swing and
    // a signed bump/roughness offset), so this variant lands on the `uGrain=0`
    // image exactly and the gate can check that it did.
    return /* glsl */ `
        float gmGrain = 0.0;
        float gmGrainSwing = 0.0;`;
  }
  return /* glsl */ `
        float gmGrain = 0.0;
        float gmGrainSwing = 0.0;
        if (gmFw * ${GRAIN_SCALE_MIN.toFixed(4)} < uGrainFade.y) {
          float gmFw3 = max(length(fwidth(vGmPos)), 1e-5);
          float gmGScale = dot(uGrainScale, gmW);
          float gmFadeGrain = 1.0 - smoothstep(uGrainFade.x, uGrainFade.y, gmFw3 * gmGScale);
          if (gmFadeGrain > 0.0) {
            gmGrain = gmGrainTap(gmXZ, gmGScale, dot(uGrainRidge, gmW))
                    * 2.0 * gmFadeGrain * uSurface * uGrain;
            gmGrainSwing = gmGrain * dot(uGrainContrast, gmW);
          }
        }`;
}

/**
 * The terrain material. One instance serves every chunk and the far mesh,
 * so the uniforms below (including the probe's `uSurface`) are the scene's
 * single handle on the ground's surface.
 *
 * @param {string} [variantName] one of TERRAIN_VARIANTS; "ship" is what the
 *   client renders and the only one built outside `costProbe`.
 */
export function makeTerrainMaterial(variantName = "ship") {
  if (!TERRAIN_VARIANTS.includes(variantName)) {
    throw new Error(`unknown terrain material variant: ${variantName}`);
  }
  const variant = VARIANT_CONFIG[variantName];
  const material = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    roughness: 1.0,
    metalness: 0.0,
  });
  const uniforms = {
    uSurface: { value: 1 },
    uIdentColor: { value: IDENTITIES.map((i) => new THREE.Vector3(...i.color)) },
    uIdentRough: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.roughness)) },
    uIdentBump: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.bump)) },
    uScales: { value: new THREE.Vector3(SCALE_MACRO, SCALE_MESO, SCALE_MICRO) },
    uMottle: { value: new THREE.Vector3(...MOTTLE) },
    uFade: { value: new THREE.Vector4(FADE_MESO[0], FADE_MESO[1], FADE_MICRO[0], FADE_MICRO[1]) },
    uOct: { value: new THREE.Vector2(AMP_MESO_M, AMP_MICRO_M) },
    uSnowColor: { value: new THREE.Vector3(...SNOW_COLOR) },
    // The second pass's handle, and the four vectors the grain octave reads
    // its identity off. It ships at 1: a probe input, not a quality setting,
    // and the gate reads the live value back to prove it.
    uGrain: { value: 1 },
    uGrainScale: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.scale)) },
    uGrainContrast: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.contrast)) },
    uGrainRidge: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.ridge)) },
    uGrainFade: { value: new THREE.Vector2(...FADE_GRAIN_CPP) },
    uGrainAmp: { value: new THREE.Vector2(AMP_GRAIN_M, GRAIN_ROUGH) },
  };

  material.onBeforeCompile = (shader) => {
    // The baseline, captured BEFORE the first replace: three's own standard
    // material as three handed it over. Measured rather than inferred, and
    // this is the second time round — the first cut of this slice took the
    // `noshadow` variant's size for "stock" and called the remainder ours,
    // which understated what this repo adds to the ground by 2.9x. A variant
    // is the shipped program minus ONE term; it is not the unpatched one.
    material.userData.cost.stockFragmentChars = resolvedGlslChars(shader.fragmentShader);
    Object.assign(shader.uniforms, uniforms);

    shader.vertexShader = shader.vertexShader
      .replace("#include <common>", `#include <common>\n${TERRAIN_VERT_PARS}`)
      .replace(
        "#include <beginnormal_vertex>",
        `#include <beginnormal_vertex>
        vGmNorm = normalize(mat3(modelMatrix) * objectNormal);`,
      )
      .replace(
        "#include <begin_vertex>",
        `#include <begin_vertex>
        vGmSplat = splat;
        vGmPos = (modelMatrix * vec4(transformed, 1.0)).xyz;`,
      );

    // Everything the material decides happens here, in the graph's order,
    // and leaves two locals (gmRough, gmH) for the roughness and normal
    // stages below — three's chunks run in one main(), so they carry.
    shader.fragmentShader = shader.fragmentShader
      .replace("#include <common>", `#include <common>\n${terrainFragPars(variant.grain)}`)
      .replace(
        "#include <color_fragment>",
        /* glsl */ `
        vec2 gmXZ = vGmPos.xz;

        // Filtered microstructure, hoisted: both octaves fade by pixel
        // footprint, so detail that can no longer be resolved is gone rather
        // than aliasing — and an octave that is gone need not be SAMPLED.
        // The fades are computed here, above the field, for that second
        // reason (see fieldGlsl).
        float gmFw = max(length(fwidth(gmXZ)), 1e-5);
        float gmFadeMeso = 1.0 - smoothstep(uFade.x, uFade.y, gmFw);
        float gmFadeMicro = 1.0 - smoothstep(uFade.z, uFade.w, gmFw);
${fieldGlsl(variant.field)}

        // Identity weights: the sim's own (height, moisture, slope) call,
        // pushed around by the field so the boundary is mottled, then
        // squared (a splat blend, not a dissolve) and renormalized.
        vec4 gmW = max(vGmSplat, 0.0);
        vec4 gmWob = vec4(gmMacro, gmMeso, 1.0 - gmMacro, 1.0 - gmMeso) - 0.5;
        gmW = max(gmW + gmWob * (${BLEND_BREAKUP.toFixed(4)} * uSurface), 0.0);
        gmW *= gmW;
        gmW /= max(dot(gmW, vec4(1.0)), 1e-4);

        vec3 gmAlbedo = uIdentColor[0] * gmW.x + uIdentColor[1] * gmW.y
                      + uIdentColor[2] * gmW.z + uIdentColor[3] * gmW.w;
        float gmRough = dot(uIdentRough, gmW);
        float gmBump = dot(uIdentBump, gmW);

        // Causal modifiers — each drives colour and roughness together.
        float gmWet = 1.0 - smoothstep(${WET_RANGE[0].toFixed(2)}, ${WET_RANGE[1].toFixed(2)}, vGmPos.y);
        float gmSnow = smoothstep(${SNOW_RANGE[0].toFixed(1)}, ${SNOW_RANGE[1].toFixed(1)}, vGmPos.y) * gmW.w;
        float gmCliff = (1.0 - smoothstep(${CLIFF_UPNESS[0].toFixed(3)}, ${CLIFF_UPNESS[1].toFixed(3)}, clamp(vGmNorm.y, 0.0, 1.0))) * gmW.w;
        gmAlbedo *= mix(1.0, ${CLIFF_DARKEN.toFixed(3)}, gmCliff);
        gmAlbedo = mix(gmAlbedo, uSnowColor, gmSnow);
        gmRough = mix(gmRough, ${SNOW_ROUGH.toFixed(3)}, gmSnow);
        gmAlbedo *= mix(1.0, ${WET_DARKEN.toFixed(3)}, gmWet);
        gmRough = mix(gmRough, ${WET_ROUGH.toFixed(3)}, gmWet);
${grainGlsl(variant.grain)}

        gmAlbedo *= 1.0 + uSurface * (
            (gmMacro - 0.5) * uMottle.x
          + (gmMeso - 0.5) * uMottle.y * gmFadeMeso
          + (gmMicro - 0.5) * uMottle.z * gmFadeMicro)
          + gmGrainSwing;
        gmRough = clamp(
          gmRough + uSurface * (gmMicro - 0.5) * ${ROUGH_VAR.toFixed(3)} * gmFadeMicro
                  + gmGrain * uGrainAmp.y,
          0.04, 1.0);

        float gmH = gmBump * (uSurface * (
            (gmMeso - 0.5) * 2.0 * uOct.x * gmFadeMeso
          + (gmMicro - 0.5) * 2.0 * uOct.y * gmFadeMicro)
          + gmGrain * uGrainAmp.x);

        diffuseColor.rgb *= max(gmAlbedo, 0.0);
        `,
      )
      .replace(
        "#include <roughnessmap_fragment>",
        `#include <roughnessmap_fragment>\n        roughnessFactor = gmRough;`,
      )
      .replace(
        "#include <normal_fragment_maps>",
        /* glsl */ `
        #include <normal_fragment_maps>
        // Surface-gradient bump: perturb the shading normal by the screen
        // derivatives of gmH against those of world position, so the bump is
        // in metres and needs no tangent frame or UVs (there are none).
        {
          vec3 gmDpdx = dFdx(vGmPos);
          vec3 gmDpdy = dFdy(vGmPos);
          vec3 gmR1 = cross(gmDpdy, normal);
          vec3 gmR2 = cross(normal, gmDpdx);
          float gmDet = dot(gmDpdx, gmR1);
          vec3 gmGrad = sign(gmDet) * (dFdx(gmH) * gmR1 + dFdy(gmH) * gmR2);
          normal = normalize(abs(gmDet) * normal - gmGrad);
        }
        // Specular AA on what we just perturbed (procedural-materials
        // reference): variance of the shading normal widens the lobe instead
        // of letting it sparkle.
        {
          vec3 gmNx = dFdx(normal);
          vec3 gmNy = dFdy(normal);
          float gmVar = max(dot(gmNx, gmNx), dot(gmNy, gmNy));
          roughnessFactor = clamp(
            sqrt(roughnessFactor * roughnessFactor + gmVar * ${SPEC_AA.toFixed(3)}),
            0.0, 1.0);
        }
        `,
      );
  };
  // The ground is the biggest shadow RECEIVER in the frame, so it takes the
  // clipmap patch like everything else (shadow clipmap v0). Installed after
  // the splat patch above; it composes with it rather than replacing it.
  installClipmapShadows(material, variant.shadow);
  // …and it is the biggest CASTER, which it was not until this line existed.
  //
  // three derives the shadow pass's material from this one and, for a
  // FrontSide material, flips the side: `shadowSide[FrontSide] = BackSide`.
  // That default is the closed-mesh answer to acne — render the far side of a
  // solid so the near side never self-shadows — and it is exactly wrong for a
  // heightfield, which has one side. Every terrain triangle faces the sky,
  // the sun is in the sky, so every one of them was culled out of the depth
  // pass: hills cast NOTHING, near or far, and the only shadows in the frame
  // came from the closed geometry (scatter, pieces, players). Naming the side
  // explicitly is the fix; the acne the default was avoiding is what the
  // per-level normal bias above is for.
  material.shadowSide = THREE.FrontSide;
  // One program for every chunk: without this three re-runs onBeforeCompile
  // per material-instance key and can compile the same shader repeatedly.
  // The variant is part of the key or the probe's programs would collide with
  // the shipped one and it would time the same shader four times.
  material.customProgramCacheKey = () => `gates-terrain-splat-v2-grain-clipmap-${variantName}`;
  material.userData.uniforms = uniforms;
  // What this program costs, counted. `programStats` (the compiled source's
  // size) is filled by installClipmapShadows on first compile — it is the
  // last patch, so it measures what three actually hands the driver.
  material.userData.cost = {
    variant: variantName,
    field: variant.field,
    grain: variant.grain,
    shadow: variant.shadow,
    // Depth fetches and noise samples per shaded fragment: the two things
    // `NOW.md` item 1 is trying to buy headroom from, both counted. The count
    // is SITES in the program, not evaluations — two of the four are behind
    // their own footprint fade, so a near fragment pays four and a far one
    // pays two. It is an upper bound, which is what a budget wants.
    depthFetches: clipmapFetches(variant.shadow),
    noiseSamples:
      variant.field === "off" ? 0 : 3 + (variant.grain === "on" ? 1 : 0),
    microSkipped: variant.field === "full",
    grainOn: variant.grain === "on",
  };
  return material;
}

/** Build one of each cost variant. `costProbe` owns the lifetime. */
export function makeTerrainCostVariants() {
  return TERRAIN_VARIANTS.map((name) => makeTerrainMaterial(name));
}

// --- authored identities for everything that is not the ground -------------
// Per-surface roughness/metalness bundles, so a stone wall, a wooden door and
// a metal sheet answer the same key light differently. Boxes and cones until
// there are models; the RESPONSE is what makes the tier read (bases.webp).
export const SURFACES = {
  wood: { roughness: 0.95, metalness: 0.0 },
  stone: { roughness: 0.88, metalness: 0.0 },
  metal: { roughness: 0.42, metalness: 0.8 },
  foliage: { roughness: 0.86, metalness: 0.0 },
  rock: { roughness: 0.85, metalness: 0.0 },
  ore: { roughness: 0.55, metalness: 0.45 },
  cloth: { roughness: 0.78, metalness: 0.0 },
  water: { roughness: 0.14, metalness: 0.0 },
};

/**
 * A MeshStandardMaterial with one of the authored responses above, wired to
 * the shadow clipmap.
 *
 * Every standard material in the scene goes through here, which is the point:
 * the clipmap replaces three's one-map-per-light shadow term, so a material
 * that skipped this patch would be lit by N lights and shadowed by none of
 * them. `installClipmapShadows` hands each of them the same onBeforeCompile
 * source, so they still share one compiled program.
 */
export function surfaceMaterial(surface, opts = {}) {
  const s = SURFACES[surface];
  if (!s) throw new Error(`unknown surface identity: ${surface}`);
  return installClipmapShadows(
    new THREE.MeshStandardMaterial({
      roughness: s.roughness,
      metalness: s.metalness,
      ...opts,
    }),
  );
}

/** The material system's structural facts, for the browser gate to assert. */
export function materialFacts() {
  return {
    identities: IDENTITIES.map((i) => i.name),
    identityRoughness: IDENTITIES.map((i) => i.roughness),
    identityBump: IDENTITIES.map((i) => i.bump),
    surfaces: Object.fromEntries(
      Object.entries(SURFACES).map(([k, v]) => [k, [v.roughness, v.metalness]]),
    ),
    breakup: BLEND_BREAKUP,
    specAA: SPEC_AA,
    fadeMicroM: FADE_MICRO,
    // The second pass. Scales in cycles/m, so the gate can check they are
    // genuinely finer than the micro octave (0.588 /m) as well as distinct.
    grainScale: IDENTITIES.map((i) => i.grain.scale),
    grainContrast: IDENTITIES.map((i) => i.grain.contrast),
    grainRidge: IDENTITIES.map((i) => i.grain.ridge),
    grainFadeCpp: FADE_GRAIN_CPP,
    grainAmpM: AMP_GRAIN_M,
    grainRough: GRAIN_ROUGH,
    microScale: SCALE_MICRO,
  };
}
