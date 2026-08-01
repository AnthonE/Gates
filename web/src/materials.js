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

import * as THREE from "three";

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
export const IDENTITIES = [
  { name: "sand", color: [0.78, 0.71, 0.52], roughness: 0.92, bump: 0.35 },
  { name: "grass", color: [0.35, 0.49, 0.23], roughness: 0.82, bump: 0.6 },
  { name: "litter", color: [0.15, 0.33, 0.16], roughness: 0.88, bump: 0.9 },
  { name: "rock", color: [0.5, 0.48, 0.46], roughness: 0.7, bump: 2.2 },
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

const TERRAIN_FRAG_PARS = /* glsl */ `
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
${FIELD_GLSL}
`;

/**
 * The terrain material. One instance serves every chunk and the far mesh,
 * so the uniforms below (including the probe's `uSurface`) are the scene's
 * single handle on the ground's surface.
 */
export function makeTerrainMaterial() {
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
  };

  material.onBeforeCompile = (shader) => {
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
      .replace("#include <common>", `#include <common>\n${TERRAIN_FRAG_PARS}`)
      .replace(
        "#include <color_fragment>",
        /* glsl */ `
        vec2 gmXZ = vGmPos.xz;
        float gmMacro = gmNoise(gmXZ * uScales.x);
        float gmMeso  = gmNoise(gmXZ * uScales.y);
        float gmMicro = gmNoise(gmXZ * uScales.z);

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

        // Filtered microstructure. Both octaves fade by pixel footprint, so
        // detail that can no longer be resolved is gone rather than aliasing.
        float gmFw = max(length(fwidth(gmXZ)), 1e-5);
        float gmFadeMeso = 1.0 - smoothstep(uFade.x, uFade.y, gmFw);
        float gmFadeMicro = 1.0 - smoothstep(uFade.z, uFade.w, gmFw);

        gmAlbedo *= 1.0 + uSurface * (
            (gmMacro - 0.5) * uMottle.x
          + (gmMeso - 0.5) * uMottle.y * gmFadeMeso
          + (gmMicro - 0.5) * uMottle.z * gmFadeMicro);
        gmRough = clamp(
          gmRough + uSurface * (gmMicro - 0.5) * ${ROUGH_VAR.toFixed(3)} * gmFadeMicro,
          0.04, 1.0);

        float gmH = gmBump * uSurface * (
            (gmMeso - 0.5) * 2.0 * uOct.x * gmFadeMeso
          + (gmMicro - 0.5) * 2.0 * uOct.y * gmFadeMicro);

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
  // One program for every chunk: without this three re-runs onBeforeCompile
  // per material-instance key and can compile the same shader repeatedly.
  material.customProgramCacheKey = () => "gates-terrain-splat-v0";
  material.userData.uniforms = uniforms;
  return material;
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

/** A MeshStandardMaterial with one of the authored responses above. */
export function surfaceMaterial(surface, opts = {}) {
  const s = SURFACES[surface];
  if (!s) throw new Error(`unknown surface identity: ${surface}`);
  return new THREE.MeshStandardMaterial({
    roughness: s.roughness,
    metalness: s.metalness,
    ...opts,
  });
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
  };
}
