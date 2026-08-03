// The three.js scene shell (DESIGN.md §9): first-person camera at the
// predicted capsule's eye, remote players as capsule+nose groups keyed by
// id, sky/fog/light/water. All per-frame math goes through preallocated
// vectors — no allocations, no closures in the RAF path (L8).

import * as THREE from "three";
import { materialFacts, propToggle, surfaceMaterial } from "./materials.js";
import {
  LEVEL_COUNT,
  ShadowClipmap,
  clipmapActiveLevels,
  setClipmapActiveLevels,
} from "./shadows.js";

const EYE_HEIGHT = 1.6; // cosmetic (DECISIONS.md §open, client cosmetics)
const YAW_TO_RAD = (Math.PI * 2) / 65536;

// --- lighting v0 (DECISIONS.md §open, "lighting v0") ------------------------
// One key, one fill, one bounded shadow map, one tone map. Nothing here is a
// post stack; the only stages are light ratios → tone map → sRGB output.
//
// The tone map is Khronos PBR Neutral, not ACES, and that was measured
// rather than chosen: ACES's toe put the dark-albedo scatter (the 0x2f6b33
// pine) at ~20/255 on its shaded side, which is a crushed image, not a dark
// one. Neutral is identity below ~0.8 linear and only rolls the highlights
// off, so what darkens this scene is the shadow map, not the transfer.
//
// --- the daylight register (DECISIONS.md §open, "the daylight register") ----
//
// v0's register was the retired art direction — "Rust with a darker edge" —
// and the operator retired that on 2026-08-01 ("just rip rust for now"). What
// it left behind was measured by the visual judge on pass 20260803-064506-01
// and it is not a matter of taste; it is a value hierarchy that is upside
// down against every one of the eighteen reference frames:
//
//   · **the sky was the DARKEST large surface in the frame** — ours 55–114
//     luma against lit sand at 160–200, where `generichighview2.jpg` and
//     `roads.jpeg` both put the sky at 186–191, above everything. A ground
//     brighter than its own sky is not a dark scene, it is an inverted one:
//     nothing in it can read as being lit BY the sky.
//   · **aerial perspective never engaged at all**, because the fog near plane
//     sat at 180 m against a near ring that is 160 m of visible ground. Not
//     "too little fog" — *no* fog, on every pixel the six vantages could see.
//     Measured near-to-far grass 68→73 luma and 0.677→0.558 saturation, flat,
//     where the reference runs 87→188 and 0.363→0.135 converging on the sky.
//   · **the dome could not hold a sun, a cloud or a horizon band**: 24×16
//     vertices, vertex-coloured, so the largest flat region in every frame
//     banded in ~7 px steps and the one shot aimed at the sky was 360 px of
//     pure linear gradient.
//
// `CLAUDE.md`'s trap list is explicit that these are ONE owner — "tonemap,
// sky, exposure and fog are one owner; split across parallel passes they
// break each other's assumptions faster than they improve" — so this slice
// takes all of them at once and gates the result on three counted asserts
// (`daylightProbe`, below) rather than on anybody's eye.
//
// **What this slice deliberately does NOT move: the sun's elevation.** The
// visual report asks for "sun elevation up to a near-midday register" and
// that is the one item here that cannot be taken without reddening a wall.
// The shadow gate's floors — 15% of the sweep darkened, 10% on every yaw —
// were measured against terrain self-shadowing under a 21° sun, where a
// hillside shades 2.66 m of ground per metre of relief. At 45° that is 1.0 m,
// which is most of the measured 24.0% gone, and the honest reading of a floor
// that a change would breach is that the change is not done, not that the
// floor is too high. It goes to `DECISIONS.md` §open as a proposal with the
// shadow work it needs, and the sky it hangs in is bright either way — a low
// sun under a bright sky is late afternoon, which is a register, where a
// bright ground under a dark sky is an error.
const SUN_AZIMUTH = 2.35;
const SUN_ELEVATION = 0.36;
// The key's colour, unchanged from v0 — and the pass tried moving it, measured
// it, and put it back, which is worth recording because the measurement says
// something the fill below depends on.
//
// v0 is a strongly orange sun over an ambient so dark that a shaded face
// rendered as near-black noise. Give that shaded face real light and the two
// illuminants stop being one dominant and one rounding error: every object is
// lit by a warm source on one side and whatever the fill is on the other, and
// the CHROMATIC distance between them is painted across every silhouette in
// the world. The prop gate reads that directly — its wall is the ratio of the
// boulder's chromaticity spread with the procedural field ON against OFF, and
// a bright NEUTRAL fill put 21% into that denominator without touching the
// numerator: x1.12 → x1.0986 against a x1.10 floor.
//
// The obvious fix is to walk the sun toward the fill, and it is the wrong one.
// Measured at `0xfff1de` (0.069 from the fill against v0's 0.132): the ratio
// fell FURTHER, to x1.06, because a warm key does not only cost spread, it
// AMPLIFIES the rock's own chromatic deviation — the numerator dropped 38%
// while the denominator barely moved. The light that shows a mineral is the
// coloured one. So the sun stays where it is and the fill comes to IT.
const SUN_COLOR = 0xffe1b8;
const SUN_INTENSITY = 3.0;
// The fill is sky-above / earth-below, and it is the whole ambient budget:
// every face the key cannot reach is lit by this and by nothing else.
//
// So it is also the **ambient floor**, which is the third defect above stated
// as arithmetic. The visual judge measured lit-vs-unlit faces of the same
// object at 89.5 vs 9.1 and 57.2 vs 6.1 — ratios of 0.10, i.e. a shaded face
// is a black silhouette carrying no albedo, no grain and no material identity
// at all. Half of every object's screen area was therefore unreadable, and
// that is the mechanism behind the report's OTHER finding, the one that reads
// as a materials defect: "not one carries a texture". A texture that renders
// at a tenth of its lit value is not a texture anybody can see. The judge's
// own bar is 0.30 and `daylightProbe` asserts it.
//
// Raised on both counts, and the ground half more than the sky half: the
// v0 pair is a clear-sky ratio, and the surface under this scene is sand and
// dry grass at high albedo, which bounces. `key > fill` still holds — the
// lighting gate asserts it, and a fill at or above the key is the flat
// hemisphere state lighting v0 exists to have ended.
// The pair is BRIGHT and very nearly ACHROMATIC, where v0's was dark and
// strongly split, and both halves of that were forced by a wall.
//
// A hemisphere light's colour is a function of the surface normal, so the gap
// between its two poles is a chromatic difference between the up-facing and
// down-facing facets of every object in the world — and the intensity scales
// that difference with it. v0's poles sit **0.217 apart** in linear
// chromaticity (a cold slate over a brown earth). Raising a pair that split
// is not "more ambient", it is more hue variation painted onto every object
// by the light, and the prop gate caught it immediately and correctly: the
// boulder's chromaticity spread with the procedural FIELD OFF rose far enough
// to take the field's own ratio from the x1.12 `DECISIONS.md` records to
// x1.07, i.e. the ambient started doing the hue work the material is gated on
// doing. This pair sits **0.051 apart**, so at 1.3 the facet split is 0.066
// against v0's 0.25 — a quarter of it — and the fill can be raised without
// spending a wall that belongs to somebody else's slice.
//
// Bright, because the number that has to move is the DOWN pole and v0's was
// nearly black. The previous pass wrote the arithmetic out and handed it
// here: a down-facing prop face receives only `FILL_GROUND x FILL_INTENSITY`,
// which delivered the pine canopy's underside at **RGB (2, 6, 1)** — the
// visual judge measured (2, 6, 0) on `03-canopy-up` — and that row says in
// its own words that no albedo can fix it, "because there is no light on it",
// and that the fix is this coupled edit. The down pole goes 0.132 → 0.75 of
// linear luminance, **5.7x**, and that is the whole of criterion 2's "half of
// every object is a black identity-free silhouette".
// **Nothing here moved, and that is this pass's largest finding rather than
// its omission.** The visual judge's third counted ask was an ambient floor —
// no unlit face below 0.30 of its lit one — and the previous pass wrote out
// the arithmetic and handed it to this one: a down-facing prop face receives
// only `FILL_GROUND x FILL_INTENSITY`, which delivers the pine canopy's
// underside at **RGB (2, 6, 1)** against the judge's measured (2, 6, 0), and
// that row says in its own words that no albedo can fix it "because there is
// no light on it".
//
// It cannot be raised yet, and the block is a WALL, not a difficulty. The
// prop gate's own assertion is the ratio of a prop's chromaticity spread with
// the material's procedural field ON against OFF, shipping at x1.12 (boulder)
// and x1.13 (pine) against a x1.10 floor — 0.02 of headroom. Every unit of
// ambient lands in that ratio's DENOMINATOR and none of it in the numerator,
// because light added to a face that was rendering as near-black noise is new
// chromaticity in the frame that the material did not put there. Six
// configurations were built and measured, and every one of them is red:
//
//   fill 1.9, poles 0.143 apart      → boulder x1.07, and the shadow gate at
//                                      14.57% against its own 15% floor
//   fill 1.5, poles 0.143 apart      → boulder x1.09
//   fill 1.3, poles 0.051 apart      → boulder x1.0995
//   …with EXPOSURE 0.8 -> 0.7        → boulder x1.0986
//   fill 1.3, poles ON the key's hue → boulder x1.067
//   key walked toward the fill       → boulder x1.06
//   **sky pole at v0, BOUNCE alone
//     raised 3.4x**                  → boulder RECOVERS, pine x1.07
//
// The last line is the one that settles it: the two poles fail through two
// different props. The sky pole lights the up- and side-facing majority of a
// boulder, the bounce pole lights a canopy's underside, and each breaks the
// prop whose faces it reaches. There is no split of this budget that raises
// the floor and leaves that ratio alone. And the two obvious escapes are
// worse rather than better, measured: putting the fill on the key's own
// chromaticity costs 38% of the NUMERATOR, because a coloured light is what
// makes a mineral's chromatic deviation visible in the first place.
//
// **So the floor waits for the numerator, which is somebody else's slice.**
// `NOW.md`'s "nothing that is not the ground has a surface" is the pass that
// raises a prop's authored chroma; with it, the same ambient buys a ratio
// that clears x1.10 and this becomes a two-line change. Raising it now would
// mean lowering a wall to fit, which ends a run. The six rows above are the
// measurement that pass inherits, so it starts from a bounded problem.
const FILL_SKY = 0xa9c3e2;
const FILL_GROUND = 0x6b5f4a;
const FILL_INTENSITY = 1.15;
// One tone map, owned by the renderer. No material sets its own.
//
// **Unchanged at v0's 0.8, and that is a measurement rather than an
// oversight.** This slice ran at 0.7 for three of its rounds, as the front-end
// compensation for a fill of 1.9 — the same stops arriving over a narrower
// range would otherwise have paid for a readable shadow with a chalky
// highlight. The fill ended at 1.3, so the compensation stopped being owed;
// and 0.7 cost something measurable on the way out, because the darkest
// pixels are where 8-bit chromaticity is most quantized: at 0.7 the boulder's
// delivered p05 fell to 22/255 and the prop gate's ship-vs-flat chroma ratio
// with it, to x1.0995 against its x1.10 floor. Exposure is a front-end scale
// on everything, so it is the cheapest thing in this file to leave alone.
const EXPOSURE = 0.8;
// Fog and the sky dome share a horizon colour, so the seam is exact by
// construction rather than by tuning two numbers against each other. That
// colour is now the HAZE: bright, desaturated, and above every ground value
// in the frame, so distance both lightens and washes out — which is what
// aerial perspective is, and what makes the sky the top of the value range
// instead of the bottom of it.
const FOG_COLOR = 0xc9d8e4;
// Inside the ring, by construction rather than by taste: the near ring is
// 5×5 chunks of 64 m, so the player can see 160 m of detailed ground and a
// near plane past that is a term no visible pixel evaluates. 20 m is close
// enough that the ramp starts inside the frame a standing player has.
const FOG_NEAR = 20;
// And the far plane is where the island stops being read rather than where it
// stops being drawn: 1400 m against the camera's 1500 m far and the island's
// 2048 m width. Deliberately not shorter. The far-shadow and horizon gates
// measure shadow contrast at 200–500 m, and fog is the one change in this file
// that can only ever REMOVE contrast at distance; 1400 m puts 14–34% of haze
// there against their ~29–48/255 mean deltas and a 6/255 bar.
const FOG_FAR = 1400;
const SKY_ZENITH = 0x4a7cc0;
// The horizon→zenith ramp, now **above** 1: the haze band is the first few
// degrees of sky holding the horizon colour before the blue takes over, and a
// curve under 1 does the exact opposite (v0's 0.62 lifted the zenith blue down
// to the ground line). At 1.6 the ramp is at 20% of the way to the zenith
// 8.5° up and half way at 34°.
const SKY_CURVE = 1.6;
const SKY_RADIUS = 10;
// The sun's own disc, and the reason the dome is a fragment program now.
// `roads.jpeg` and `generichighview2.jpg` both carry one; ours had none, while
// the water already rendered a specular track clipping to 252 in a frame whose
// 99th percentile is ~140 — a glitter with no source. Angular radii in radians
// (the real sun is 0.0047; this is the painted one, which every game enlarges
// so it survives a 75° FOV), then the halo's cosine power and the two gains in
// linear working space, over a sky that peaks near 0.6.
const SUN_DISC_RAD = [0.012, 0.03];
const SUN_DISC_GAIN = 9.0;
const SUN_GLOW_POW = 220.0;
const SUN_GLOW_GAIN = 0.9;
// One LSB of dither on the dome, in the linear space it is computed in. The
// sky is the largest flat region in any frame and an 8-bit ramp across it
// banded in ~7 px steps; the derivative of the sRGB transfer near the sky's
// own 0.5 linear is ~0.66, so 0.006 of linear is ~1.5/255 of output — enough
// to break a step, small enough that no other measurement can see it. Hashed
// on `gl_FragCoord` and nothing else, so it is a FIXED pattern: every probe in
// this file renders one frame twice and requires the pair to be identical.
const SKY_DITHER = 0.006;

// Shadow coverage is a clipmap now — concentric light-space levels, each
// snapped to its own texel grid, coarse ones cached (shadows.js owns the
// levels, their knobs and the shader patch). Lighting v0's single 80 m map is
// level 0 of it, unchanged, so nothing about the near frame moved.

// Build-grid render dimensions. Cell/level sizes are the sim's grid
// (DECISIONS.md §open, build grid v0). LIFT and WALL_T (and the doorway
// post width below) mirror sim-core collide.rs — collision truth since
// piece collision v0; SLAB and tier colors stay cosmetics.
const CELL = 3;
const LEVEL_H = 3;
const LIFT = 0.3; // collide.rs PIECE_LIFT_M
const SLAB = 0.3; // plane-piece thickness (cosmetic)
const WALL_T = 0.24; // collide.rs WALL_THICKNESS_M
const TIER_COLORS = [0x8a6a45, 0x84837c, 0x5f6a72]; // wood · stone · metal
// …and the response that makes the tier read at a distance, before any of
// them has a texture: wood is matte, stone is matte-but-tighter, metal is
// a conductor with a real specular lobe (materials v0). The reference
// frames' tier read is as much sheen as colour (`bases.webp`).
const TIER_SURFACES = ["wood", "stone", "metal"];
// Deployable stand-ins by archetype code (sim deploy.rs order: bag,
// hearth, box, fire, furnace, workbench, door): [w, h, d, color, surface].
// Cosmetics (DECISIONS.md §open, client cosmetics row).
const DEPLOY_STYLE = [
  [1.2, 0.25, 0.7, 0x7a9c4e, "cloth"], // bag
  [0.9, 0.9, 0.9, 0x8c3b2e, "stone"], // hearth
  [1.0, 0.7, 1.0, 0x7a5c3a, "wood"], // box
  [0.7, 0.4, 0.7, 0xd07030, "stone"], // fire
  [1.1, 1.5, 1.1, 0x4f4a45, "stone"], // furnace
  [1.6, 0.9, 0.9, 0xa1793f, "wood"], // workbench
  [0.12, 2.1, 0.9, 0x6b4a2b, "wood"], // door (thickness, height, width)
];
// The death backpack (backpack.rs): a low canvas bundle on the ground
// where a body fell, in the same cloth surface the sleeping bag wears, so
// it shares that program family and links nothing new after `inWorld`
// (the prewarm trap, CLAUDE.md). Cosmetics (DECISIONS.md §open, client
// cosmetics row).
const BAG_STYLE = [0.6, 0.35, 0.45, 0xa06a3c, "cloth"];

// A locked door reads as banded iron over the wood — the one bit of door
// state a passer-by can see, and the thing they'd have to break.
const DOOR_LOCKED_COLOR = 0x3c3f44;

// A conservative world radius for one placed piece or deployable, derived
// from the grid it sits on rather than picked: no piece spans more than a
// cell across or a level tall, so this sphere contains any of them.
const PIECE_RADIUS_M = Math.sqrt(CELL * CELL + LEVEL_H * LEVEL_H + CELL * CELL);

/**
 * Khronos PBR Neutral, in JS, exactly as three's `NeutralToneMapping` chunk
 * writes it — on a `THREE.Color` in the linear working space, in place.
 *
 * It exists for one colour: the fog's. Three applies fog AFTER the tone map
 * and after the output transform (`fog_fragment` is the last chunk in the
 * chain), so the fog colour is the one shaded quantity in the frame that the
 * renderer's transfer never touches. Lighting v0's claim that "fog and the
 * sky dome share a horizon colour, so the seam is exact by construction" was
 * therefore already off by the transfer — the dome is tone-mapped and exposed
 * and the fog is not, so one hex reached the image as two values (at exposure
 * 0.8 they landed within ~10% of each other, which is why nobody saw it).
 *
 * Raising the haze to a daylight register makes that gap visible: distant
 * ground would converge on a value ~28/255 ABOVE the sky directly over it,
 * which is the inverted hierarchy this whole slice exists to end, rebuilt at
 * the horizon line. So the fog is handed the colour the dome will actually
 * render — exposure applied, then this — and the seam is exact for real.
 */
function toneMapNeutral(c) {
  const startCompression = 0.8 - 0.04;
  const desaturation = 0.15;
  const x = Math.min(c.r, Math.min(c.g, c.b));
  const offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
  c.r -= offset;
  c.g -= offset;
  c.b -= offset;
  const peak = Math.max(c.r, Math.max(c.g, c.b));
  if (peak < startCompression) return c;
  const d = 1 - startCompression;
  const newPeak = 1 - (d * d) / (peak + d - startCompression);
  const scale = newPeak / peak;
  c.r *= scale;
  c.g *= scale;
  c.b *= scale;
  const g = 1 - 1 / (desaturation * (peak - newPeak) + 1);
  c.r += (newPeak - c.r) * g;
  c.g += (newPeak - c.g) * g;
  c.b += (newPeak - c.b) * g;
  return c;
}

/**
 * The bin at which a histogram's running count first reaches `want`.
 *
 * Used by `daylightProbe` for every quantile it reports. A histogram rather
 * than a sort because every quantity it asks about is already 8-bit, and
 * sorting two million values per yaw would be the probe's entire cost.
 * Returns the last bin when `want` is past the total, which is the same
 * answer a sort would give.
 */
function quantileBin(hist, want) {
  let run = 0;
  for (let i = 0; i < hist.length; i++) {
    run += hist[i];
    if (run >= want) return i;
  }
  return hist.length - 1;
}

/** Mark an object (and a group's children) as both caster and receiver. */
function shadowed(obj) {
  obj.castShadow = true;
  obj.receiveShadow = true;
  for (let i = 0; i < obj.children.length; i++) {
    obj.children[i].castShadow = true;
    obj.children[i].receiveShadow = true;
  }
  return obj;
}

/**
 * The sky, as a fragment program on a stock `MeshBasicMaterial`.
 *
 * v0 painted the ramp into 24×16 VERTEX colours, and the visual judge scored
 * exactly what that is: the largest flat region in every frame, banding in
 * ~7 px steps, with nowhere to put a sun. A vertex colour cannot hold a disc
 * a third of a degree across, and no amount of tessellation fixes a gradient
 * that is being reconstructed by the rasterizer at 8 bits.
 *
 * So the ramp, the haze band, the disc, its halo and the dither are all
 * evaluated per fragment from the view direction. Three things are kept from
 * v0 deliberately, because they are load-bearing and easy to lose:
 *
 *   1. It is still a **patched stock material**, not a raw `ShaderMaterial`.
 *      Three's own `tonemapping_fragment` and `colorspace_fragment` run
 *      after ours untouched, so the dome goes through the one tone map the
 *      renderer owns — the whole reason the sky is geometry and not a clear
 *      colour (a clear colour is the one surface the tone mapper never sees).
 *   2. The colour **at and below the horizon is exactly `FOG_COLOR`**, in the
 *      same linear working space the fog is converted into, so the seam where
 *      the fogged ground meets the sky is exact by construction. `t` is 0 for
 *      every direction at or under y = 0.
 *   3. The dither is hashed on `gl_FragCoord` alone. Every probe in this file
 *      renders one frame twice and requires the two to be bit-identical; a
 *      dither that moved with time or frame count would break all of them, and
 *      would be a temporal artefact in the bargain.
 */
function skyMaterial(toSun) {
  const mat = new THREE.MeshBasicMaterial({
    side: THREE.BackSide,
    fog: false,
    depthWrite: false,
    depthTest: false,
  });
  const uniforms = {
    uSkyHorizon: { value: new THREE.Color(FOG_COLOR) },
    uSkyZenith: { value: new THREE.Color(SKY_ZENITH) },
    uSkyCurve: { value: SKY_CURVE },
    uSunDir: { value: toSun.clone() },
    uSunDisc: { value: new THREE.Vector2(Math.cos(SUN_DISC_RAD[0]), Math.cos(SUN_DISC_RAD[1])) },
    uSunGains: { value: new THREE.Vector2(SUN_DISC_GAIN, SUN_GLOW_GAIN) },
    uSunGlowPow: { value: SUN_GLOW_POW },
    uSkyDither: { value: SKY_DITHER },
  };
  mat.userData.uniforms = uniforms;
  mat.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace("#include <common>", "#include <common>\nvarying vec3 vGmSkyDir;")
      .replace(
        "#include <begin_vertex>",
        "#include <begin_vertex>\nvGmSkyDir = position;",
      );
    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <common>",
        /* glsl */ `#include <common>
varying vec3 vGmSkyDir;
uniform vec3 uSkyHorizon;
uniform vec3 uSkyZenith;
uniform float uSkyCurve;
uniform vec3 uSunDir;
uniform vec2 uSunDisc;
uniform vec2 uSunGains;
uniform float uSunGlowPow;
uniform float uSkyDither;
float gmSkyHash(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}`,
      )
      // `color_fragment` is where a vertex colour would have multiplied in;
      // taking its place is what makes this the only writer of the dome's
      // albedo, with three's transfer still downstream of it.
      .replace(
        "#include <color_fragment>",
        /* glsl */ `
  vec3 gmDir = normalize(vGmSkyDir);
  // The ramp. Clamped at the horizon, so every direction at or below y = 0 is
  // exactly the fog colour and the seam needs no tuning.
  float gmUp = max(gmDir.y, 0.0);
  vec3 gmSky = mix(uSkyHorizon, uSkyZenith, pow(gmUp, uSkyCurve));
  // The disc, and the halo that says where its light came from.
  float gmCos = dot(gmDir, uSunDir);
  float gmDisc = smoothstep(uSunDisc.y, uSunDisc.x, gmCos);
  float gmGlow = pow(max(gmCos, 0.0), uSunGlowPow);
  gmSky += diffuse * (gmDisc * uSunGains.x + gmGlow * uSunGains.y);
  // One LSB, fixed per pixel.
  gmSky += (gmSkyHash(gl_FragCoord.xy) - 0.5) * uSkyDither;
  diffuseColor.rgb = max(gmSky, 0.0);`,
      );
  };
  // `diffuse` is the sun's own colour above: the disc and the halo are the
  // only things in the dome that carry it, and routing them through the
  // material's colour keeps three's uniform plumbing rather than adding a
  // ninth one that says the same thing.
  mat.color = new THREE.Color(SUN_COLOR);
  return mat;
}

export class GameScene {
  constructor(canvas) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    // Single tone-map ownership: the renderer maps, materials do not, and
    // nothing re-encodes sRGB downstream. The clear colour is the only
    // surface the tone mapper never sees, which is exactly why the sky is
    // geometry below — the clear is a fallback that the dome always covers.
    this._fogRender = toneMapNeutral(new THREE.Color(FOG_COLOR).multiplyScalar(EXPOSURE));
    this.renderer.setClearColor(this._fogRender);
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = EXPOSURE;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    // three resets renderer.info AFTER the shadow pass and before the main
    // one, so the default counters silently exclude every shadow draw — the
    // exact half of the budget this rig just added. Own the reset instead.
    this.renderer.info.autoReset = false;
    this.scene = new THREE.Scene();
    // The haze, on the transfer's own scale — see `toneMapNeutral`. The dome
    // paints `FOG_COLOR` and the renderer exposes and tone-maps it on the way
    // out; the fog is mixed in after both, so it is handed the answer instead
    // of the question, and the horizon seam is exact rather than off by a
    // transfer nobody had noticed was in the way.
    this.scene.fog = new THREE.Fog(this._fogRender, FOG_NEAR, FOG_FAR);
    this.camera = new THREE.PerspectiveCamera(
      75,
      window.innerWidth / window.innerHeight,
      0.1,
      1500,
    );

    // World-space unit vector pointing AT the sun. The sun does not move in
    // v0, so the light-space basis it defines is built once, inside the
    // clipmap, and only read in the RAF path. Built before the dome, which
    // now paints a disc where this points.
    const ce = Math.cos(SUN_ELEVATION);
    this._toSun = new THREE.Vector3(
      ce * Math.sin(SUN_AZIMUTH),
      Math.sin(SUN_ELEVATION),
      ce * Math.cos(SUN_AZIMUTH),
    ).normalize();

    // The sky is a dome, not a clear colour, for one reason: fog is shaded
    // and therefore tone-mapped, a clear colour is not. Painting the sky as
    // geometry puts both through the same tone map, so the horizon seam is
    // exact instead of tuned. Its horizon ring IS the fog colour.
    this.sky = new THREE.Mesh(
      new THREE.SphereGeometry(SKY_RADIUS, 24, 16),
      skyMaterial(this._toSun),
    );
    this.sky.renderOrder = -1;
    this.sky.frustumCulled = false;
    this.scene.add(this.sky);

    const fill = new THREE.HemisphereLight(FILL_SKY, FILL_GROUND, FILL_INTENSITY);
    this.scene.add(fill);
    this.fill = fill;
    // The key. It is level 0 of the clipmap — the only level carrying any
    // intensity — so "the sun" and "the near shadow map" are one object, and
    // the coarse levels exist purely as depth.
    this.clipmap = new ShadowClipmap(
      this.scene,
      SUN_COLOR,
      SUN_INTENSITY,
      this._toSun,
    );
    this.sun = this.clipmap.key;
    this._corner = new THREE.Vector3();

    // One translucent plane at sea level; nothing simulates (TERRAIN.md §4).
    // It neither casts nor receives: a transparent sheet in the shadow pass
    // buys artefacts, not depth.
    // Smooth, so the low sun leaves a specular track on it — the one thing
    // that separates water from a blue plane before it animates.
    const water = new THREE.Mesh(
      new THREE.PlaneGeometry(6144, 6144),
      surfaceMaterial("water", {
        color: 0x2b5d7d,
        transparent: true,
        opacity: 0.62,
      }),
    );
    water.rotation.x = -Math.PI / 2;
    water.position.set(1024, 0.0, 1024);
    this.scene.add(water);
    this.water = water;

    this.remotes = new Map(); // id -> { group, stamp }
    this._capsuleGeo = new THREE.CapsuleGeometry(0.4, 1.0, 3, 10);
    this._noseGeo = new THREE.ConeGeometry(0.12, 0.34, 8);
    this._noseGeo.rotateX(Math.PI / 2); // apex points +Z (the yaw forward)
    this._remoteMat = surfaceMaterial("cloth", { color: 0xc8a072 });
    this._remoteFrozenMat = surfaceMaterial("cloth", { color: 0x8a8a8a });

    // The weak-spot glint (DESIGN.md §2 "the Rust juice"): one unlit
    // octahedron parked on the marked node's flank; hidden when no mark.
    // Sizes are cosmetics (DECISIONS.md §open, client cosmetics row).
    this.weakMark = new THREE.Mesh(
      new THREE.OctahedronGeometry(0.18),
      new THREE.MeshBasicMaterial({ color: 0xffe066 }),
    );
    this.weakMark.visible = false;
    this.scene.add(this.weakMark);

    // Placed building pieces, keyed by grid address. Shared geometries +
    // one material per tier; meshes are added on placement events (never
    // the RAF path) and swept only by a piece-set reset.
    this.pieces = new Map(); // "cx,cz,level,loc" -> Object3D
    this.deploys = new Map(); // "cx,cz,level,loc" -> Object3D
    this.bags = new Map(); // backpack id -> Object3D
    this._bagMat = null; // shared: every bag is the same bundle
    this._deployMats = new Map(); // arch -> material (shared per kind)
    this._planeGeo = new THREE.BoxGeometry(CELL - 0.04, SLAB, CELL - 0.04);
    this._wallGeo = new THREE.BoxGeometry(WALL_T, LEVEL_H, CELL - 0.04);
    this._postGeo = new THREE.BoxGeometry(WALL_T, LEVEL_H, 0.9);
    this._lintelGeo = new THREE.BoxGeometry(WALL_T, 0.9, CELL - 0.04 - 1.8);
    this._stairsGeo = new THREE.BoxGeometry(CELL - 0.04, SLAB, 4.15);
    this._tierMats = TIER_COLORS.map((c, i) =>
      surfaceMaterial(TIER_SURFACES[i], { color: c }),
    );
    // The placement ghost: one wireframe box, rescaled to the aimed
    // piece's shape each frame build mode is on.
    this.ghost = new THREE.Mesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshBasicMaterial({ color: 0x9fd08f, wireframe: true }),
    );
    this.ghost.visible = false;
    this.scene.add(this.ghost);

    this._dir = new THREE.Vector3();
    this._target = new THREE.Vector3();
    // Last frame's draw counts (DESIGN §9's budget), refreshed in render(),
    // plus the running peak. The peak is what the clipmap made necessary: a
    // cached coarse level draws on SOME frames, so the frame the gate happens
    // to read is not the expensive one. The budget is what the GPU was ever
    // asked to draw, not what it was asked on a lucky frame.
    this.stats = { calls: 0, triangles: 0, peakCalls: 0, peakTriangles: 0 };

    window.addEventListener("resize", () => {
      this.camera.aspect = window.innerWidth / window.innerHeight;
      this.camera.updateProjectionMatrix();
      this.renderer.setSize(window.innerWidth, window.innerHeight);
    });
    this.renderer.setSize(window.innerWidth, window.innerHeight);
  }

  /** Feet position + look angles → camera at the eye. */
  setCamera(x, y, z, yawRad, pitchRad) {
    const c = this.camera;
    c.position.set(x, y + EYE_HEIGHT, z);
    const cp = Math.cos(pitchRad);
    this._dir.set(Math.sin(yawRad) * cp, Math.sin(pitchRad), Math.cos(yawRad) * cp);
    this._target.copy(c.position).add(this._dir);
    c.lookAt(this._target);
    this.sky.position.copy(c.position);
    this.clipmap.update(x, y, z);
  }

  /** Park the weak-spot glint at a world position, or hide it. */
  setWeakMark(x, y, z) {
    this.weakMark.position.set(x, y, z);
    this.weakMark.visible = true;
  }

  hideWeakMark() {
    this.weakMark.visible = false;
  }

  /**
   * Upsert one placed piece. `groundY` is the shared-worldgen terrain
   * height at the cell center — both tabs derive the same y, no piece
   * height rides the wire. Shape codes are sim-core build.rs's.
   */
  setPiece(cx, cz, level, loc, shape, material, groundY) {
    const key = `${cx},${cz},${level},${loc}`;
    const old = this.pieces.get(key);
    if (old) this.scene.remove(old);
    const mat = this._tierMats[material] || this._tierMats[0];
    const baseY = groundY + LIFT + level * LEVEL_H;
    const cxm = cx * CELL + CELL / 2;
    const czm = cz * CELL + CELL / 2;
    let obj;
    if (shape === 1 || shape === 2) {
      // Wall / doorway on the west (x = cx·3) or north (z = cz·3) edge;
      // the doorway keeps its opening — the intended breach point reads.
      if (shape === 1) {
        obj = new THREE.Mesh(this._wallGeo, mat);
      } else {
        obj = new THREE.Group();
        const a = new THREE.Mesh(this._postGeo, mat);
        a.position.z = -(CELL - 0.9) / 2 + 0.0;
        const b = new THREE.Mesh(this._postGeo, mat);
        b.position.z = (CELL - 0.9) / 2 - 0.0;
        const l = new THREE.Mesh(this._lintelGeo, mat);
        l.position.y = LEVEL_H / 2 - 0.45;
        obj.add(a, b, l);
      }
      if (loc === 2) {
        obj.position.set(cx * CELL, baseY + LEVEL_H / 2, czm);
      } else {
        obj.rotation.y = Math.PI / 2;
        obj.position.set(cxm, baseY + LEVEL_H / 2, cz * CELL);
      }
    } else if (shape === 4) {
      // Stairs: a ramp through the level. The grid stores no facing, so
      // the ramp always rises toward +Z (cosmetic, v0).
      obj = new THREE.Mesh(this._stairsGeo, mat);
      obj.rotation.x = -Math.PI / 4;
      obj.position.set(cxm, baseY + LEVEL_H / 2, czm);
    } else {
      // Foundation / floor / roof: a slab whose top is the level plane.
      obj = new THREE.Mesh(this._planeGeo, mat);
      obj.position.set(cxm, baseY - SLAB / 2, czm);
    }
    shadowed(obj);
    this.scene.add(obj);
    this._invalidateShadows(obj);
    this.pieces.set(key, obj);
  }

  /**
   * A caster appeared or vanished here: force every cached clipmap level
   * whose box reaches it to redraw. Without this a base going up 150 m away
   * casts nothing until the coarse level's age expires — the reference's
   * "important streamed geometry remains unshadowed". Placement events only;
   * the RAF path never calls it.
   */
  _invalidateShadows(obj) {
    this.clipmap.invalidate(
      obj.position.x,
      obj.position.y,
      obj.position.z,
      PIECE_RADIUS_M,
    );
  }

  clearPieces() {
    for (const obj of this.pieces.values()) {
      this._invalidateShadows(obj);
      this.scene.remove(obj);
    }
    this.pieces.clear();
  }

  removePiece(cx, cz, level, loc) {
    const key = `${cx},${cz},${level},${loc}`;
    const obj = this.pieces.get(key);
    if (obj) {
      this._invalidateShadows(obj);
      this.scene.remove(obj);
      this.pieces.delete(key);
    }
  }

  /**
   * Upsert one deployable: a colored box per archetype, standing on the
   * level plane (body deploys) or filling a doorway edge (doors).
   */
  /**
   * Park a deployable at a grid address. `open` and `locked` only mean
   * anything for a door: closed it fills its doorway edge, open it swings
   * a quarter turn onto its hinge — the same read the sim's collision
   * has, so a player never walks through a leaf that still looks shut —
   * and locked it wears the iron.
   */
  setDeploy(cx, cz, level, loc, arch, groundY, open, locked) {
    const key = `${cx},${cz},${level},${loc}`;
    const old = this.deploys.get(key);
    if (old) this.scene.remove(old);
    const [w, h, d, color, surface] = DEPLOY_STYLE[arch] || DEPLOY_STYLE[2];
    // Two materials for the door archetype, one for everything else;
    // both cached, because this runs on every door swing. The locked leaf
    // takes the metal response with the iron colour — the band is what a
    // passer-by sees, the sheen is what tells them it is not wood.
    const ironclad = arch === 6 && locked;
    const matKey = ironclad ? "door-locked" : arch;
    let mat = this._deployMats.get(matKey);
    if (!mat) {
      mat = ironclad
        ? surfaceMaterial("metal", { color: DOOR_LOCKED_COLOR })
        : surfaceMaterial(surface, { color });
      this._deployMats.set(matKey, mat);
    }
    const obj = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), mat);
    const baseY = groundY + LIFT + level * LEVEL_H;
    if (loc === 2 || loc === 3) {
      // A door in a doorway edge, oriented like the wall there. Open, it
      // swings off the hinge end of its leaf and lies across the cell.
      if (loc === 2) {
        if (open) {
          obj.rotation.y = Math.PI / 2;
          obj.position.set(cx * CELL + d / 2, baseY + h / 2, cz * CELL + CELL / 2 - d / 2);
        } else {
          obj.position.set(cx * CELL, baseY + h / 2, cz * CELL + CELL / 2);
        }
      } else if (open) {
        obj.position.set(cx * CELL + CELL / 2 - d / 2, baseY + h / 2, cz * CELL + d / 2);
      } else {
        obj.rotation.y = Math.PI / 2;
        obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL);
      }
    } else {
      obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL + CELL / 2);
    }
    shadowed(obj);
    this.scene.add(obj);
    this._invalidateShadows(obj);
    this.deploys.set(key, obj);
  }

  /**
   * Reconcile the standing death backpacks against the client's whole set
   * (`client_bag_ids_ptr` / `client_bags_ptr`). The client hands the set,
   * not a delta, so this adds what is new and removes what is gone —
   * ≤ MAX_BACKPACKS entries and only on an `APPLIED_BAGS` message, which
   * is a death or a loot, not a frame.
   *
   * `ids` and `pos` are wasm-memory views; `n` is the live count, because
   * the views are sized for the cap.
   */
  setBags(ids, pos, n) {
    const [w, h, d, color, surface] = BAG_STYLE;
    if (!this._bagMat) this._bagMat = surfaceMaterial(surface, { color });
    const live = new Set();
    for (let i = 0; i < n; i++) {
      const id = ids[i];
      live.add(id);
      if (this.bags.has(id)) continue; // a bag never moves
      const obj = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), this._bagMat);
      // The sim drops it at the body's feet, so lift by half its height
      // to stand it on the ground rather than half-sunk in it.
      obj.position.set(pos[i * 3], pos[i * 3 + 1] + h / 2, pos[i * 3 + 2]);
      shadowed(obj);
      this.scene.add(obj);
      this._invalidateShadows(obj);
      this.bags.set(id, obj);
    }
    for (const [id, obj] of this.bags) {
      if (live.has(id)) continue;
      this._invalidateShadows(obj);
      this.scene.remove(obj);
      obj.geometry.dispose();
      this.bags.delete(id);
    }
  }

  removeDeploy(cx, cz, level, loc) {
    const key = `${cx},${cz},${level},${loc}`;
    const obj = this.deploys.get(key);
    if (obj) {
      this._invalidateShadows(obj);
      this.scene.remove(obj);
      this.deploys.delete(key);
    }
  }

  clearDeploys() {
    for (const obj of this.deploys.values()) {
      this._invalidateShadows(obj);
      this.scene.remove(obj);
    }
    this.deploys.clear();
  }

  /** Park the placement ghost over the aimed address. */
  setGhost(shape, cx, cz, level, loc, groundY) {
    const g = this.ghost;
    const baseY = groundY + LIFT + level * LEVEL_H;
    const cxm = cx * CELL + CELL / 2;
    const czm = cz * CELL + CELL / 2;
    if (shape === 1 || shape === 2) {
      g.scale.set(WALL_T, LEVEL_H, CELL);
      g.rotation.y = loc === 3 ? Math.PI / 2 : 0;
      if (loc === 2) g.position.set(cx * CELL, baseY + LEVEL_H / 2, czm);
      else g.position.set(cxm, baseY + LEVEL_H / 2, cz * CELL);
    } else if (shape === 4) {
      g.scale.set(CELL, LEVEL_H, CELL);
      g.rotation.y = 0;
      g.position.set(cxm, baseY + LEVEL_H / 2, czm);
    } else {
      g.scale.set(CELL, SLAB, CELL);
      g.rotation.y = 0;
      g.position.set(cxm, baseY - SLAB / 2, czm);
    }
    g.visible = true;
  }

  hideGhost() {
    this.ghost.visible = false;
  }

  /** Upsert one interpolated remote; `stamp` drives mark-and-sweep. */
  upsertRemote(id, x, y, z, yawWire, live, stamp) {
    let r = this.remotes.get(id);
    if (!r) {
      const group = new THREE.Group();
      const body = new THREE.Mesh(this._capsuleGeo, this._remoteMat);
      body.position.y = 0.9;
      const nose = new THREE.Mesh(this._noseGeo, this._remoteMat);
      nose.position.set(0, 1.45, 0.42);
      group.add(body);
      group.add(nose);
      shadowed(group);
      this.scene.add(group);
      r = { group, body, nose, stamp: 0 };
      this.remotes.set(id, r);
    }
    r.group.position.set(x, y, z);
    r.group.rotation.y = yawWire * YAW_TO_RAD;
    const mat = live ? this._remoteMat : this._remoteFrozenMat;
    if (r.body.material !== mat) {
      r.body.material = mat;
      r.nose.material = mat;
    }
    r.stamp = stamp;
  }

  /** Remove remotes not seen this frame (entity left the interest set). */
  sweepRemotes(stamp) {
    for (const [id, r] of this.remotes) {
      if (r.stamp !== stamp) {
        this.scene.remove(r.group);
        this.remotes.delete(id);
      }
    }
  }

  render() {
    // Reset before, not after: with autoReset off these counts then cover
    // BOTH passes — the budget in DESIGN §9 is what the GPU was asked to
    // draw, not what is in view once. Copied into plain numbers so the
    // debug snapshot can be read without holding a live renderer object.
    this.renderer.info.reset();
    this.renderer.render(this.scene, this.camera);
    const r = this.renderer.info.render;
    this.stats.calls = r.calls;
    this.stats.triangles = r.triangles;
    if (r.calls > this.stats.peakCalls) this.stats.peakCalls = r.calls;
    if (r.triangles > this.stats.peakTriangles) this.stats.peakTriangles = r.triangles;
  }

  /**
   * The ground's material is built by Terrain (it owns the worker that feeds
   * it); the scene borrows its uniforms so the surface probe has one handle
   * on the whole splat system. Called once at boot.
   */
  attachTerrainMaterial(material) {
    this._terrainMat = material;
    this._terrainUniforms = material.userData.uniforms || null;
  }

  /**
   * Compile every program this session can wear before play (CLAUDE.md trap:
   * median fps hides shader-compile stalls — a program that links mid-play is
   * a 100 ms-class hitch the sim never sees). Two halves, because programs
   * come from two places:
   *
   * COLOR programs — `renderer.compile()` builds them for everything in the
   * scene graph, so materials whose first draw is late (the terrain, attached
   * before any chunk mesh exists; remotes, first drawn when a player enters
   * the AOI; pieces, first placement) each ride a hidden dummy mesh into the
   * call. Door and deployable materials share these programs (same feature
   * set), so the samples here cover them.
   *
   * DEPTH programs — the shadow pass builds those, and no shadow pass exists
   * before `inWorld` (the clipmap only updates from `setCamera`). So the
   * dummies STAY, casting, and the caller parks them at the player for the
   * first in-world frames (`prewarmAt`), then removes them (`prewarmDone`)
   * once the depth program has linked. `browser_smoke` asserts the sum of
   * both halves: zero program links after its snapshot.
   */
  prewarm(extras = []) {
    const mats = [this._remoteMat, this._remoteFrozenMat, ...this._tierMats];
    if (this._terrainMat) mats.push(this._terrainMat);
    const geo = new THREE.PlaneGeometry(0.02, 0.02);
    const group = new THREE.Group();
    for (let i = 0; i < mats.length; i++) {
      // Each material twice: straight, and mirrored (scale.x = -1). A
      // negative-determinant transform flips which side the shadow pass
      // renders, and `flipSided` is a PROGRAM define — the depth variant a
      // mirrored (or shadowSide-set) caster wears is a separate link.
      for (const sx of [1, -1]) {
        const m = new THREE.Mesh(geo, mats[i]);
        m.castShadow = true;
        m.frustumCulled = false;
        m.position.y = i * 0.06 + (sx < 0 ? 0.03 : 0);
        m.scale.x = sx;
        group.add(m);
      }
    }
    // Callers hand in dummies for program families a plain plane cannot
    // reach — instanced pools, custom depth materials (terrain.prewarmObjects).
    for (const o of extras) group.add(o);
    group.position.set(0, -40, 0);
    this.scene.add(group);
    this._prewarmGroup = group;
    this._prewarmGeo = geo;
    this.renderer.compile(this.scene, this.camera);
    return this.renderer.info.programs.length;
  }

  /** Park the prewarm dummies where the first shadow pass will see them. */
  prewarmAt(x, y, z) {
    if (this._prewarmGroup) this._prewarmGroup.position.set(x, y, z);
  }

  /** Depth programs linked — the dummies have no further job. */
  prewarmDone() {
    if (!this._prewarmGroup) return;
    this.scene.remove(this._prewarmGroup);
    this._prewarmGroup.traverse((o) => {
      if (o.geometry) o.geometry.dispose();
      if (o.instanceMatrix) o.dispose(); // InstancedMesh owns GPU buffers
    });
    this._prewarmGroup = null;
    this._prewarmGeo = null;
  }

  /**
   * The far mesh's depth uniforms, borrowed the same way and for the same
   * reason: `horizonProbe` needs one handle on the horizon's caster, and
   * Terrain owns it because Terrain owns the mesh. Called once at boot.
   */
  attachFarCaster(uniforms) {
    this._farHole = uniforms.uNearHole;
  }

  /**
   * The ground's cost variants and the swapper that wears them, borrowed the
   * same way (Terrain owns both). Dev-only: main.js does not call this on a
   * public shard, and `costProbe` returns null without it.
   */
  attachTerrainCost(hooks) {
    this._terrainCost = hooks;
  }

  /** Force every clipmap level to redraw on the next render. Probes only. */
  _redrawShadows() {
    for (const L of this.clipmap.levels) L.light.shadow.needsUpdate = true;
  }

  /**
   * Dev-only: does the HORIZON cast?
   *
   * The near ring casts and always did. What this slice adds is the far mesh —
   * the 8 m LOD of the whole island — casting everywhere the ring is not, and
   * no pixel count on its own can separate the two: a frame full of shadow is
   * a frame full of shadow whether a pine at 30 m or a ridge at 500 m drew it.
   *
   * So this toggles the caster rather than the camera. The far mesh casts
   * through a depth material that discards the near ring's footprint; opening
   * that hole to swallow the world discards ALL of it, which is exactly the
   * state that shipped before this slice — the ring casting, the horizon
   * receiving and never casting. Two renders of one frame, and the difference
   * is the horizon, by construction rather than by argument: the hole means
   * every pixel counted here was darkened by geometry more than the ring's own
   * footprint away.
   *
   * It is a uniform toggle and nothing else. Same programs, same geometry, the
   * same levels forced to redraw for both — which is what makes the difference
   * attributable (flipping `castShadow` instead would move the lights-state
   * version, recompile every program, and change the draw counts too).
   *
   * Allocates and renders 3N frames: never the RAF path. For the browser gate.
   */
  horizonProbe(yaws, pitchRad, minDelta, heightM) {
    if (!this._farHole) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const casting = new Uint8Array(w * h * 4);
    const suppressed = new Uint8Array(w * h * 4);
    // The zero point, measured on every yaw: two renders of the SAME state,
    // both with their levels re-drawn, must differ nowhere. Anything else and
    // the counts below are partly the rasterizer talking.
    const control = new Uint8Array(w * h * 4);
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();
    const hole = this._farHole.value;
    const keepHole = hole.clone();
    cam.position.y += heightM;
    this.sky.position.copy(cam.position);
    const samples = [];
    let darkened = 0;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(cam.position).add(this._dir);
      cam.lookAt(this._target);

      this._redrawShadows();
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, casting);
      this._redrawShadows();
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
      // The hole eats the world: every far-mesh fragment is discarded and the
      // horizon casts nothing at all.
      hole.set(keepHole.x, keepHole.y, 1e7, 1e7);
      this._redrawShadows();
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, suppressed);
      hole.copy(keepHole);

      let n = 0;
      let sum = 0;
      let max = 0;
      let lifted = 0;
      let noise = 0;
      for (let p = 0; p < casting.length; p += 4) {
        const a = (suppressed[p] * 2 + suppressed[p + 1] * 5 + suppressed[p + 2]) >> 3;
        const b = (casting[p] * 2 + casting[p + 1] * 5 + casting[p + 2]) >> 3;
        const c = (control[p] * 2 + control[p + 1] * 5 + control[p + 2]) >> 3;
        const cd = b - c;
        if (cd > minDelta || cd < -minDelta) noise++;
        const d = a - b;
        if (d > minDelta) {
          n++;
          sum += d;
          if (d > max) max = d;
        } else if (d < -minDelta) {
          // A caster can only remove light. Brighter with it than without it
          // would mean the hole is inverted — the far mesh casting INSIDE the
          // ring and nowhere else, which is the acne case wearing a disguise.
          lifted++;
        }
      }
      samples.push({
        yaw: yaws[i],
        darkened: n,
        lifted,
        noise,
        fraction: n / (w * h),
        liftedFraction: lifted / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
      });
      darkened += n;
    }
    hole.copy(keepHole);
    this._redrawShadows();
    cam.quaternion.copy(keepQ);
    cam.position.copy(keepPos);
    this.sky.position.copy(keepPos);
    this.renderer.render(this.scene, cam);
    return {
      width: w,
      height: h,
      pixels: w * h * yaws.length,
      darkened,
      heightM,
      // What the hole was while the frames were scored — the gate checks this
      // is the live ring footprint and not a probe leftover.
      holeHalf: [keepHole.z, keepHole.w],
      samples,
    };
  }

  /**
   * Dev-only: does the surface have GRAIN, and does the grain go away?
   *
   * `surfaceProbe` counts pixels that moved, which is the right question for
   * "is the field painting at all" and the wrong one for grain. A uniform
   * wash moves every pixel it touches; so does a tint; so does an exposure
   * slip. None of them is texture. What separates grain from all three is
   * that grain is HIGH FREQUENCY — it changes between one pixel and the next
   * — so this measures neighbour-to-neighbour contrast and not just delta.
   *
   * Three renders per view, not two:
   *   - the toggled uniform at 1,
   *   - the same state again, which is the CONTROL. Two renders of one state
   *     must differ nowhere, or every ceiling below is partly the rasterizer
   *     talking — and half the claims this probe exists for are ceilings
   *     ("grain is gone at 140 m"), which a noisy zero point would pass for
   *     free.
   *   - the toggled uniform at 0.
   *
   * Metrics per view: how many pixels moved and which way (the surfaceProbe
   * measure, kept because a contrast ratio on an empty mask means nothing),
   * and the mean contrast of both frames over the SAME pixel set — the set
   * the toggle moved. Same set for both states, so the ratio is honest: it
   * asks whether what the toggle added was detail or a wash.
   *
   * Since materials v3 it carries a SECOND, parallel track with its own mask,
   * because luma cannot see a luminance-neutral octave at all. Every octave
   * this material had before the tint multiplied albedo by a scalar, and a
   * scalar multiply leaves chromaticity exactly where it found it — so the
   * luma track above is blind to hue, and the tint octave is deliberately
   * blind to value (`materials.js`, `TINT_LUMA_NEUTRAL`). Neither track can
   * score the other's octave, which is the point of having two.
   *
   * The chroma track masks on |Δchromaticity| > `minChroma` instead of
   * |Δluma| > `minDelta`, and over THAT mask reports:
   *   chromaOn/chromaOff  the RMS spread of the chromaticity cloud per state
   *   chromaShift         how far the cloud's centre moved
   *   chromaUp/chromaDown the signed split on the red-chromaticity axis
   *   chromaMoved         the mask's own size
   * plus `lumaOn`/`lumaOff`, the whole frame's mean luma in each state, over
   * every scored pixel rather than either mask. Together they separate three
   * things one number blurs: a texture (spread up, centre still), a cast
   * (centre moves), and an exposure slip (mean luma moves).
   *
   * Views are (eye, target) pairs in world space, resolved by the caller
   * because this class holds the camera and not the terrain. It moves the
   * camera, which surfaceProbe does not, so the sky dome rides along and
   * everything is restored at the end.
   *
   * Allocates and renders 3N frames; never call it from the RAF path.
   */
  contrastProbe(views, uniformName, minDelta, minChroma = 0) {
    const u = this._terrainUniforms;
    if (!u || !u[uniformName]) return null;
    const knob = u[uniformName];
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const on = new Uint8Array(w * h * 4);
    const control = new Uint8Array(w * h * 4);
    const off = new Uint8Array(w * h * 4);
    const luma = (buf, p) => (buf[p] * 2 + buf[p + 1] * 5 + buf[p + 2]) >> 3;
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();
    const keepVal = knob.value;
    const samples = [];
    for (const view of views) {
      cam.position.set(view.eye[0], view.eye[1], view.eye[2]);
      this.sky.position.copy(cam.position);
      this._target.set(view.at[0], view.at[1], view.at[2]);
      cam.lookAt(this._target);

      knob.value = 1;
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, on);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
      knob.value = 0;
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, off);

      let up = 0;
      let down = 0;
      let sum = 0;
      let noise = 0;
      let mask = 0;
      let cOn = 0;
      let cOff = 0;
      // The chroma track: its OWN mask, and moments over it. `luma` above
      // cannot see the difference between a surface that gained a texture and
      // one that gained a brightness pattern — multiplying an RGB triple by a
      // scalar leaves its chromaticity exactly where it was — and it equally
      // cannot see an octave that moved hue at constant brightness, which is
      // what the tint is. Chromaticity is `(r, g) / (r + g + b)`, and the +1
      // is for the black pixels a sky or a shadow contributes.
      let cMask = 0;
      let cUp = 0;
      let cDown = 0;
      let xOn = 0;
      let yOn = 0;
      let xxOn = 0;
      let yyOn = 0;
      let xOff = 0;
      let yOff = 0;
      let xxOff = 0;
      let yyOff = 0;
      // Whole-frame luma, both states, summed over every scored pixel and not
      // just the mask — see `lumaOn`/`lumaOff` below.
      let lOn = 0;
      let lOff = 0;
      // Stop one row and one column short: the contrast measure reads the
      // right and lower neighbour of every pixel it scores.
      for (let y = 0; y < h - 1; y++) {
        for (let x = 0; x < w - 1; x++) {
          const p = (y * w + x) * 4;
          const a = luma(on, p);
          const b = luma(off, p);
          const c = luma(control, p);
          lOn += a;
          lOff += b;
          const cd = a - c;
          if (cd > minDelta || cd < -minDelta) noise++;
          // The chroma track, scored over every pixel and masked on its own
          // threshold — it cannot ride the luma mask, because the octave it
          // exists for moves no luma and would find that mask empty.
          if (minChroma > 0) {
            const sA = on[p] + on[p + 1] + on[p + 2] + 1;
            const sB = off[p] + off[p + 1] + off[p + 2] + 1;
            const sC = control[p] + control[p + 1] + control[p + 2] + 1;
            const xA = on[p] / sA;
            const yA = on[p + 1] / sA;
            const xB = off[p] / sB;
            const yB = off[p + 1] / sB;
            if (Math.hypot(xA - control[p] / sC, yA - control[p + 1] / sC) > minChroma) noise++;
            const dx = xA - xB;
            if (Math.hypot(dx, yA - yB) > minChroma) {
              cMask++;
              if (dx > 0) cUp++;
              else if (dx < 0) cDown++;
              xOn += xA;
              yOn += yA;
              xxOn += xA * xA;
              yyOn += yA * yA;
              xOff += xB;
              yOff += yB;
              xxOff += xB * xB;
              yyOff += yB * yB;
            }
          }
          const d = a - b;
          if (d > minDelta) up++;
          else if (d < -minDelta) down++;
          else continue;
          sum += d < 0 ? -d : d;
          mask++;
          const right = p + 4;
          const below = p + w * 4;
          const aR = luma(on, right) - a;
          const aB = luma(on, below) - a;
          const bR = luma(off, right) - b;
          const bB = luma(off, below) - b;
          cOn += (aR < 0 ? -aR : aR) + (aB < 0 ? -aB : aB);
          cOff += (bR < 0 ? -bR : bR) + (bB < 0 ? -bB : bB);
        }
      }
      const scored = (w - 1) * (h - 1);
      // Spread, not range: the RMS deviation of the chromaticity cloud, which
      // is what a field that moves hue raises and a field that moves only
      // brightness does not.
      const spread = (sx, sy, sxx, syy, n) => {
        if (n <= 1) return 0;
        const mx = sx / n;
        const my = sy / n;
        return Math.sqrt(Math.max(0, sxx / n - mx * mx) + Math.max(0, syy / n - my * my));
      };
      samples.push({
        label: view.label || "",
        scored,
        up,
        down,
        moved: mask,
        noise,
        movedFraction: mask / scored,
        upFraction: up / scored,
        downFraction: down / scored,
        meanDelta: mask > 0 ? sum / mask : 0,
        // Mean neighbour contrast over the moved set, in luma per pixel step.
        contrastOn: mask > 0 ? cOn / (2 * mask) : 0,
        contrastOff: mask > 0 ? cOff / (2 * mask) : 0,
        contrastRatio: cOff > 0 ? cOn / cOff : 0,
        // …and the chroma track, over its own mask. `chromaShift` is how far
        // the cloud's CENTRE moved, which separates "the surface gained colour
        // variation" from "the frame was tinted": a cast moves the centre and
        // leaves the spread, a texture does the opposite.
        chromaMoved: cMask,
        chromaMovedFraction: cMask / scored,
        chromaUpFraction: cUp / scored,
        chromaDownFraction: cDown / scored,
        chromaOn: spread(xOn, yOn, xxOn, yyOn, cMask),
        chromaOff: spread(xOff, yOff, xxOff, yyOff, cMask),
        chromaRatio:
          spread(xOff, yOff, xxOff, yyOff, cMask) > 0
            ? spread(xOn, yOn, xxOn, yyOn, cMask) / spread(xOff, yOff, xxOff, yyOff, cMask)
            : 0,
        chromaShift:
          cMask > 0 ? Math.hypot(xOn / cMask - xOff / cMask, yOn / cMask - yOff / cMask) : 0,
        // The frame's own mean luma, both states, over the whole scored area
        // rather than the mask — the claim this octave makes is that it
        // changes the ground's VARIANCE and not its average, and an average
        // taken only over the pixels that moved cannot test that.
        lumaOn: lOn / scored,
        lumaOff: lOff / scored,
      });
    }
    knob.value = keepVal;
    cam.quaternion.copy(keepQ);
    cam.position.copy(keepPos);
    this.sky.position.copy(keepPos);
    this.renderer.render(this.scene, cam);
    return { width: w, height: h, uniform: uniformName, samples };
  }

  /**
   * Dev-only: is the grain laid ON the surface, or stamped through it?
   *
   * Every probe above scores whether a term REACHES the image. This one scores
   * a term's PROJECTION, which no amount of "did it move pixels" can see: a
   * grain combed downhill and a grain lying on the slope move the same pixels
   * by the same amount, and `contrastProbe` scores both at the same ratio. The
   * defect is anisotropy — a world-XZ field on a face of upness `u` is
   * stretched by `1/u` along the slope and untouched across it — so the
   * measurement has to be anisotropy too.
   *
   * Three things make that measurable rather than arguable:
   *
   *   1. **Two compiled programs, one camera, one run.** `flatgrain` is the
   *      shipped program with materials v1's world-XZ tap in place of the
   *      triplanar one. Both are rendered from the same eye at the same
   *      instant, so everything that is not the projection — the terrain, the
   *      light, the fog, the rasterizer, this box's weather — is common mode
   *      and cancels. Nothing here is a threshold on an absolute number.
   *   2. **The grain's OWN difference image.** `d = luma(uGrain 1) −
   *      luma(uGrain 0)` is what the octave contributed and nothing else. The
   *      frame's own gradients — a shadow edge, a biome boundary, the albedo
   *      ramp — are in both terms and subtract out, so scoring `d` rather than
   *      the lit frame is what stops a hill's silhouette being counted as
   *      grain.
   *   3. **Both axes, over the mask only.** `gradX` is the mean step of `d`
   *      between horizontal neighbours and `gradY` between vertical ones, and
   *      a pair is only scored when BOTH its pixels are pixels the octave
   *      moved — otherwise the mask's own boundary would be scored as a step
   *      the size of the whole octave.
   *   4. **The octave's ALBEDO channel alone.** `uGrainAmp` is held at zero for
   *      the duration, so grain drives its colour swing and neither its bump
   *      nor its roughness. This one is not a nicety. A bump reaches the image
   *      through `−∇h·L`, so its screen anisotropy is set by where the SUN is
   *      relative to the face — at 21° elevation that term dominated the
   *      difference image and buried the projection under it (measured: both
   *      programs scored 0.37–0.41 where an isotropic field must score 1.00,
   *      and their ORDER was the reverse of the geometry's). Albedo has no
   *      such preferred direction: it is the field, multiplied by a shading
   *      term that is smooth across a pixel. `uGrain` is restored at the end
   *      and 15b is what proves the full octave — bump included — reaches the
   *      frame at all.
   *
   * The camera is the caller's, and so is the comparison. What the browser
   * gate builds out of these numbers is `(gradX + gradY) / (2·amp)` — the
   * octave's detail, direction-averaged and normalised by its own amplitude,
   * an inverse characteristic length in pixels — at a perpendicular view of a
   * TILTED patch and of a LEVEL one, per program. The ratio between those two
   * is how much that program's grain coarsens when the ground tilts under it,
   * which is the defect stated as a number.
   *
   * The screen-axis split is reported and is NOT the measure, which is worth
   * writing down because it looks like it should be. Measured here: the level
   * control scores `gradX/gradY` 1.11 in both programs (they agree to 0.001,
   * as level ground requires) and the 46.6° face scores 0.39–0.42 in both. A
   * 2.5x screen-axis bias that both programs share belongs to the view — a
   * 107° horizontal frustum over curved terrain — and it is four times the
   * 1.456x the projection is worth. Direction-averaging drops it; asserting on
   * it would have been asserting on the frustum.
   *
   * `amp` is reported for its own reason: the cheap way to win any isotropy
   * contest is to blend the octave into mush — which is exactly what a
   * triplanar blend does if the deviation is not restored by `1/|w|` — and a
   * gate that read only a shape measure would score that a fix.
   *
   * `vsFirst` compares a later program's lit frame against the first's over
   * the whole view, which is the identity half: on ground that is level the
   * two projections are the same program and this must be 0.
   *
   * Renders 3 frames per (program, view) and allocates four framebuffer-sized
   * byte buffers plus two per-pixel scratch arrays: never the RAF path.
   */
  projectionProbe(views, minDelta) {
    const hooks = this._terrainCost;
    if (!hooks || !hooks.projection) return null;
    const shipped = this._terrainMat;
    if (!shipped) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const on = new Uint8Array(w * h * 4);
    const control = new Uint8Array(w * h * 4);
    const off = new Uint8Array(w * h * 4);
    // One reference frame PER VIEW, not one for the probe. A single buffer
    // reused across the view loop leaves every later program compared against
    // the LAST view of the first one — a different camera — and the comparison
    // then reports the difference between two landscapes as the difference
    // between two projections. That bug shipped once and was caught in judging;
    // the array is the fix and this note is why it is an array.
    const first = views.map(() => new Uint8Array(w * h * 4));
    const diff = new Int16Array(w * h);
    const mask = new Uint8Array(w * h);
    const luma = (b, p) => (b[p] * 2 + b[p + 1] * 5 + b[p + 2]) >> 3;
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();

    const programs = [
      { program: "triplanar", material: shipped },
      { program: "xz", material: hooks.projection() },
    ];
    const samples = [];
    for (let pi = 0; pi < programs.length; pi++) {
      const { program, material } = programs[pi];
      hooks.use(material);
      const u = material.userData.uniforms;
      const keepGrain = u.uGrain.value;
      // Albedo only, for the duration (see the note above): the bump's screen
      // anisotropy belongs to the sun, not to the projection.
      const keepAmp = u.uGrainAmp.value.clone();
      u.uGrainAmp.value.set(0, 0);
      for (let vi = 0; vi < views.length; vi++) {
        const view = views[vi];
        cam.position.set(view.eye[0], view.eye[1], view.eye[2]);
        this.sky.position.copy(cam.position);
        this._target.set(view.at[0], view.at[1], view.at[2]);
        cam.lookAt(this._target);

        u.uGrain.value = 1;
        this.renderer.render(this.scene, cam);
        gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, on);
        this.renderer.render(this.scene, cam);
        gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
        u.uGrain.value = 0;
        this.renderer.render(this.scene, cam);
        gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, off);
        u.uGrain.value = 1;

        // Pass one: the octave's own contribution, and which pixels carry it.
        let moved = 0;
        let noise = 0;
        let amp = 0;
        for (let i = 0; i < w * h; i++) {
          const p = i * 4;
          const a = luma(on, p);
          const cd = a - luma(control, p);
          if (cd > minDelta || cd < -minDelta) noise++;
          const d = a - luma(off, p);
          diff[i] = d;
          const m = d < 0 ? -d : d;
          if (m > minDelta) {
            mask[i] = 1;
            moved++;
            amp += m;
          } else {
            mask[i] = 0;
          }
        }
        // Pass two: how fast that contribution changes along each screen axis,
        // over pairs that are both inside the mask.
        let gx = 0;
        let gy = 0;
        let nx = 0;
        let ny = 0;
        for (let y = 0; y < h - 1; y++) {
          for (let x = 0; x < w - 1; x++) {
            const i = y * w + x;
            if (!mask[i]) continue;
            if (mask[i + 1]) {
              const s = diff[i + 1] - diff[i];
              gx += s < 0 ? -s : s;
              nx++;
            }
            if (mask[i + w]) {
              const s = diff[i + w] - diff[i];
              gy += s < 0 ? -s : s;
              ny++;
            }
          }
        }
        // The confinement half: the first program's frame at THIS view is kept
        // and every later program's frame at the same view is compared against
        // it, pixel for pixel.
        //
        // `changed` counts any difference at all, including a single luma step,
        // so it runs ahead of the magnitude: on near-level ground the two
        // projections differ on 14% of the frame and by at most 1/255, which
        // the count cannot tell from a real disagreement. `meanAbsMasked` is
        // the one that can — the mean difference over the grain's own pixels,
        // which the caller divides by `amp` to read the projection's effect in
        // units of the octave it is a projection OF. Both are reported; only
        // the second is worth asserting on.
        let vsFirst = null;
        if (pi === 0) {
          first[vi].set(on);
        } else {
          const ref = first[vi];
          let changed = 0;
          let max = 0;
          let sum = 0;
          let n = 0;
          for (let i = 0; i < w * h; i++) {
            const p = i * 4;
            const d = luma(on, p) - luma(ref, p);
            if (d !== 0) changed++;
            const m = d < 0 ? -d : d;
            if (m > max) max = m;
            if (mask[i]) {
              sum += m;
              n++;
            }
          }
          vsFirst = {
            changed,
            changedFraction: changed / (w * h),
            maxDelta: max,
            meanAbsMasked: n > 0 ? sum / n : 0,
          };
        }
        const gradX = nx > 0 ? gx / nx : 0;
        const gradY = ny > 0 ? gy / ny : 0;
        samples.push({
          program,
          label: view.label || "",
          scored: w * h,
          moved,
          movedFraction: moved / (w * h),
          noise,
          amp: moved > 0 ? amp / moved : 0,
          gradX,
          gradY,
          pairsX: nx,
          pairsY: ny,
          // Above 1 the octave changes faster across the screen than down it,
          // which on a view aimed down a fall line is the comb.
          anisotropy: gradY > 0 ? gradX / gradY : 0,
          vsFirst,
        });
      }
      u.uGrain.value = keepGrain;
      u.uGrainAmp.value.copy(keepAmp);
    }
    hooks.use(shipped);
    cam.quaternion.copy(keepQ);
    cam.position.copy(keepPos);
    this.sky.position.copy(keepPos);
    this.renderer.render(this.scene, cam);
    return { width: w, height: h, minDelta, samples };
  }

  /**
   * Dev-only: does the procedural surface actually reach the frame?
   *
   * Same shape as shadowProbe and for the same reason. Every structural
   * fact about a material can be right — standard material, splat weights
   * on the geometry, four authored identities, a shader that compiled —
   * while the image is a flat wash: a field scaled into a single lattice
   * cell, a break-up amplitude of zero, uniforms never bound, a bump term
   * cancelled by its own footprint fade. So this renders the live scene
   * twice per yaw with `uSurface` at 1 and at 0 and counts the pixels that
   * moved, separately by direction.
   *
   * What the toggle holds fixed is the vertex splat weights, the four
   * authored identities and the causal modifiers (wetness, snow, cliff);
   * what it removes is every contribution of the noise field — the weight
   * break-up, the albedo mottling, the roughness variation and the bump.
   * So the delta is the field, and nothing else.
   *
   * The direction split is the part that is hard to fake: microstructure
   * lightens some pixels and darkens others, and any uniform change (an
   * exposure slip, a global tint) can only move them one way.
   *
   * Allocates and renders 2N frames; never call it from the RAF path.
   */
  surfaceProbe(yaws, pitchRad, minDelta) {
    const u = this._terrainUniforms;
    if (!u) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const full = new Uint8Array(w * h * 4);
    const flat = new Uint8Array(w * h * 4);
    const keepQ = this.camera.quaternion.clone();
    const pos = this.camera.position;
    const samples = [];
    let changed = 0;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      this.camera.lookAt(this._target);
      u.uSurface.value = 1;
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, full);
      u.uSurface.value = 0;
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flat);
      let up = 0;
      let down = 0;
      let sum = 0;
      let max = 0;
      for (let p = 0; p < full.length; p += 4) {
        const a = (full[p] * 2 + full[p + 1] * 5 + full[p + 2]) >> 3;
        const b = (flat[p] * 2 + flat[p + 1] * 5 + flat[p + 2]) >> 3;
        const d = a - b;
        const m = d < 0 ? -d : d;
        if (m > minDelta) {
          if (d > 0) up++;
          else down++;
          sum += m;
          if (m > max) max = m;
        }
      }
      const n = up + down;
      samples.push({
        yaw: yaws[i],
        up,
        down,
        changed: n,
        fraction: n / (w * h),
        upFraction: up / (w * h),
        downFraction: down / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
      });
      changed += n;
    }
    u.uSurface.value = 1;
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return { width: w, height: h, pixels: w * h * yaws.length, changed, samples };
  }

  /**
   * Dev-only: is the daylight register the right way up, and is there air?
   *
   * The three questions here are the three the visual judge left as counted
   * asserts, and each is a difference of frames rather than a threshold on
   * one — the same shape every other probe in this file wears, for the same
   * reason: a single number off a single render is a claim about this box's
   * rasterizer, and a difference between two renders of one scene is a claim
   * about the scene.
   *
   *   **a · the sky is above the ground.** Rendered again with the dome
   *      hidden over a magenta clear, which the tone mapper never touches, so
   *      "sky" is the set of pixels that come back EXACTLY (255, 0, 255) —
   *      a mask by construction, not by a colour distance nobody calibrated.
   *      The frame's own sky luma is then compared against its own ground.
   *      This is the inverted-hierarchy defect stated as a number: ours was
   *      55–114 sky against 160–200 sand, the reference is 186–191 over
   *      everything.
   *
   *   **b · distance lightens and washes out.** Rendered again with the fog
   *      pushed past every geometry in the frame, and the per-pixel delta
   *      between the two IS the amount of air in front of that pixel — a
   *      depth channel this client has no other way to read, and one that is
   *      monotone in distance by the transfer's own construction. So the
   *      changed pixels are split into terciles OF THAT DELTA (near, mid,
   *      far) and each tercile's mean luma and mean saturation are reported.
   *      Aerial perspective is exactly the claim that luma rises and
   *      saturation falls across those three. The sky cannot contaminate it:
   *      the dome is `fog: false`, so its delta is exactly zero and it never
   *      enters a tercile.
   *
   *   **c · the ambient floor.** Rendered again with the key at zero
   *      intensity, which leaves the hemisphere fill and nothing else — so
   *      per ground pixel, `luma(fill only) / luma(full)` is the share of
   *      that pixel's light that survives losing the sun, which is what an
   *      unlit face of a lit object gets. The judge's bar is 0.30 and its
   *      measurements were taken in this same 8-bit output space, so this is
   *      measured there too rather than being converted into a linear ratio
   *      the bar was never written against.
   *
   * Four renders and four readbacks per yaw. Nothing here recompiles a
   * program: the fog is moved by its own near/far uniforms rather than by
   * `scene.fog = null` (which is a `USE_FOG` define, i.e. a relink of every
   * material in the scene), and the key is moved by intensity.
   *
   * Allocates and renders 4N frames; never call it from the RAF path.
   */
  daylightProbe(yaws, pitchRad, minDelta, heightM = 0) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const px = w * h;
    const full = new Uint8Array(px * 4);
    const nofog = new Uint8Array(px * 4);
    const ambient = new Uint8Array(px * 4);
    const nosky = new Uint8Array(px * 4);
    const keepQ = this.camera.quaternion.clone();
    const keepPos = this.camera.position.clone();
    // The lift, on the same argument `horizonProbe` and `farShadowProbe` make
    // for theirs: from eye height on a beach, the ground past 100 m is a band
    // a few pixels tall under the horizon, so a question about DISTANCE asked
    // from there is answered by almost no pixels. The caller sweeps twice —
    // once at the eye, where the register and the ambient floor live, and once
    // lifted, where the air is.
    this.camera.position.y += heightM;
    this.sky.position.copy(this.camera.position);
    const pos = this.camera.position;
    const fog = this.scene.fog;
    const keepNear = fog ? fog.near : 0;
    const keepFar = fog ? fog.far : 0;
    const keepKey = this.sun.intensity;
    // Past the camera's own far plane, so no drawn fragment can reach the
    // ramp. A window of 1 m keeps the transfer well conditioned.
    const OFF_NEAR = 1e6;
    const samples = [];
    // Reused across yaws: a 256-bin histogram of the fog FACTOR, and one of
    // the ambient ratio. Histograms rather than sorted arrays because both
    // quantities are 8-bit — a sort of a million values per yaw would be the
    // probe's whole cost.
    const fHist = new Int32Array(256);
    const rHist = new Int32Array(256);
    // The value a fully fogged pixel takes, in the 8-bit output space this
    // probe reads. Three mixes the fog colour in after the transfer, so it
    // reaches the frame as the sRGB encoding of the colour handed to
    // `THREE.Fog` — no exposure and no tone map, which is exactly what
    // `toneMapNeutral` above pre-applies so that this ends up equal to what
    // the dome renders at the horizon.
    const fogHex = fog ? fog.color.getHex(THREE.SRGBColorSpace) : 0;
    const fogOut =
      ((((fogHex >> 16) & 255) * 2 + ((fogHex >> 8) & 255) * 5 + (fogHex & 255)) >> 3) || 1;
    // How far a pixel's own no-fog value must sit from the haze before the
    // factor can be recovered from it at all. `f = (a - b) / (fogOut - b)` is
    // a division by that gap, so a pixel already the colour of the haze
    // carries no usable depth and is dropped rather than estimated.
    const FOG_MIN_GAP = 12;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      this.camera.lookAt(this._target);

      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, full);

      if (fog) {
        fog.near = OFF_NEAR;
        fog.far = OFF_NEAR + 1;
      }
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, nofog);
      if (fog) {
        fog.near = keepNear;
        fog.far = keepFar;
      }

      this.sun.intensity = 0;
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, ambient);
      this.sun.intensity = keepKey;

      this.sky.visible = false;
      this.renderer.setClearColor(0xff00ff, 1);
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, nosky);
      this.sky.visible = true;
      this.renderer.setClearColor(this._fogRender);

      fHist.fill(0);
      rHist.fill(0);
      let skyN = 0;
      let skySum = 0;
      let groundN = 0;
      let groundSum = 0;
      let ratioN = 0;
      let noFogSum = 0;
      let maxDelta = 0;
      const groundHist = new Int32Array(256);
      let changed = 0;
      let up = 0;
      let fogN = 0;
      for (let p = 0; p < full.length; p += 4) {
        const a = (full[p] * 2 + full[p + 1] * 5 + full[p + 2]) >> 3;
        if (nosky[p] === 255 && nosky[p + 1] === 0 && nosky[p + 2] === 255) {
          skyN++;
          skySum += a;
          continue;
        }
        groundN++;
        groundSum += a;
        groundHist[a]++;
        const b = (nofog[p] * 2 + nofog[p + 1] * 5 + nofog[p + 2]) >> 3;
        noFogSum += b;
        const d = a - b;
        if (d > maxDelta) maxDelta = d;
        if (d > minDelta || d < -minDelta) {
          changed++;
          if (d > 0) up++;
        }
        // The depth channel. NOT the raw delta: a delta is `f x (haze - the
        // pixel's own colour)`, so binning on it sorts by how dark a pixel
        // was as much as by how far away it is, and the first cut of this
        // probe read a "far" band that was really the frame's dark half. The
        // FACTOR divides that colour term back out.
        //
        // Only pixels the haze actually moved carry one. At 8 bits a pixel
        // inside ~30 m rounds to zero lift no matter what the transfer says,
        // and there are enough of those to own a whole tercile — which is how
        // the first cut ended up comparing "unfogged ground" against "barely
        // fogged ground" and reading the terrain's own albedo as the result.
        const gap = fogOut - b;
        if (d > 0 && (gap > FOG_MIN_GAP || gap < -FOG_MIN_GAP)) {
          const f = d / gap;
          fHist[Math.max(0, Math.min(255, Math.round(f * 255)))]++;
          fogN++;
        }
        const c = (ambient[p] * 2 + ambient[p + 1] * 5 + ambient[p + 2]) >> 3;
        // A pixel with no light at all in EITHER frame carries no ratio; it
        // is the one case where 0/0 would otherwise read as a perfect floor.
        if (a > 0) {
          rHist[Math.min(255, Math.round((c / a) * 255))]++;
          ratioN++;
        }
      }

      // The tercile edges of the fog FACTOR, over every ground pixel that
      // carries one — near, mid and far thirds of the visible ground, by the
      // amount of air in front of each.
      const t1 = quantileBin(fHist, fogN / 3);
      const t2 = quantileBin(fHist, (2 * fogN) / 3);
      const band = [
        { luma: 0, sat: 0, f: 0, lift: 0, drop: 0, n: 0 },
        { luma: 0, sat: 0, f: 0, lift: 0, drop: 0, n: 0 },
        { luma: 0, sat: 0, f: 0, lift: 0, drop: 0, n: 0 },
      ];
      const hsvSat = (buf, p) => {
        const r = buf[p];
        const g = buf[p + 1];
        const bl = buf[p + 2];
        const mx = r > g ? (r > bl ? r : bl) : g > bl ? g : bl;
        const mn = r < g ? (r < bl ? r : bl) : g < bl ? g : bl;
        return mx > 0 ? (mx - mn) / mx : 0;
      };
      for (let p = 0; p < full.length; p += 4) {
        if (nosky[p] === 255 && nosky[p + 1] === 0 && nosky[p + 2] === 255) continue;
        const a = (full[p] * 2 + full[p + 1] * 5 + full[p + 2]) >> 3;
        const b = (nofog[p] * 2 + nofog[p + 1] * 5 + nofog[p + 2]) >> 3;
        const gap = fogOut - b;
        if (!(a - b > 0) || (gap <= FOG_MIN_GAP && gap >= -FOG_MIN_GAP)) continue;
        const fb = Math.max(0, Math.min(255, Math.round(((a - b) / gap) * 255)));
        const k = fb <= t1 ? 0 : fb <= t2 ? 1 : 2;
        band[k].luma += a;
        band[k].sat += hsvSat(full, p);
        // Each band against ITSELF without the air, which is the half that
        // cannot be faked by the terrain: a band's raw luma is mostly what
        // its ground is made of, but its luma LIFT is only the haze.
        band[k].lift += a - b;
        band[k].drop += hsvSat(nofog, p) - hsvSat(full, p);
        band[k].f += fb / 255;
        band[k].n++;
      }

      samples.push({
        yaw: yaws[i],
        skyFraction: skyN / px,
        skyLuma: skyN > 0 ? skySum / skyN : 0,
        groundLuma: groundN > 0 ? groundSum / groundN : 0,
        groundMedian: quantileBin(groundHist, groundN / 2),
        groundP90: quantileBin(groundHist, groundN * 0.9),
        fogFraction: changed / px,
        fogUpShare: changed > 0 ? up / changed : 0,
        // What the air did to the whole ground, not only to the pixels it
        // moved past the bar — the diagnostic that separates "the haze is
        // thin here" from "there is no haze".
        fogMeanLift: groundN > 0 ? (groundSum - noFogSum) / groundN : 0,
        fogMaxDelta: maxDelta,
        bands: band.map((x) => ({
          n: x.n,
          luma: x.n > 0 ? x.luma / x.n : 0,
          sat: x.n > 0 ? x.sat / x.n : 0,
          f: x.n > 0 ? x.f / x.n : 0,
          lift: x.n > 0 ? x.lift / x.n : 0,
          drop: x.n > 0 ? x.drop / x.n : 0,
        })),
        ambientP05: quantileBin(rHist, ratioN * 0.05) / 255,
        ambientP50: quantileBin(rHist, ratioN * 0.5) / 255,
      });
    }
    this.camera.position.copy(keepPos);
    this.sky.position.copy(keepPos);
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return { width: w, height: h, pixels: px, minDelta, heightM, samples };
  }

  /**
   * Dev-only: is a screen DERIVATIVE reaching the image as noise?
   *
   * Every other probe in this file asks whether a term reaches the frame.
   * This one asks whether something reaches it that should not, and it can
   * name the class exactly, because the artefact has a fingerprint no
   * material can forge: **it is constant inside a 2x2 quad.**
   *
   * `dFdx`/`dFdy`/`fwidth` are differences taken across the rasterizer's 2x2
   * quad, so any quantity derived from one is the SAME for all four fragments
   * of that quad and unrelated to the next quad's. Albedo is not — a noise
   * field evaluated per fragment varies between the two pixels of a quad
   * exactly as much as it varies across the quad boundary. So comparing
   * neighbour steps WITHIN a quad against neighbour steps ACROSS a quad
   * boundary separates the two sources with no threshold on brightness,
   * contrast or detail:
   *
   *   ratio ~ 1     whatever high frequency is there came from the field
   *   ratio >> 1    a derivative is in the image, and it is noise
   *
   * That ratio is why this is a gate and not a screenshot: it is scale-free
   * and it cannot be satisfied by flattening the material. A flat wash scores
   * 1, and so does a correctly filtered one — the two are separated by the
   * contrast probes (15/15b) and the chroma one (15d), which fail on a wash.
   * Passing all of them at once requires detail that is actually resolved.
   *
   * Measured on the visual judge's own frames (pass 20260802-163821-01,
   * `05-held-level.png` x 1000-1270 y 560-700): mean |dLuma| of 1.9 within
   * quads against 21.4 across them, a ratio of 11.3, which is the "literal
   * checkerboard" the report and a blind reader both named.
   *
   * Four states per view, each a use of an existing uniform, so nothing is
   * compiled or branched for the probe's benefit:
   *   `ship`    what a player sees.
   *   `nobump`  the two bump amplitudes and grain's zeroed (`uOct`,
   *             `uGrainAmp.x`) — gmH is then identically zero, so the bump
   *             solve and the specular-AA pass that follows it contribute
   *             nothing, while every albedo term is untouched. This is the
   *             bisection: if the artefact is gone here it came through the
   *             gradient solve, and if it survives it came from the wobble.
   *             It is also the instrument's FLOOR — the same scene with no
   *             derivative in it at all, which is what "ratio 1" looks like
   *             on this frame rather than in theory.
   *   `nograinbump` only `uGrainAmp.x` zeroed, so the two structural octaves
   *             keep their bump. Names the octave when this goes red.
   *   `flat`    `uSurface = 0` — the whole field gone, which is also where
   *             the ground mask below comes from.
   *
   * The mask is `|luma(ship) - luma(flat)| > minDelta`: the pixels the field
   * actually paints. Both endpoints of a step must be in it, so the sky, the
   * scatter and the far ground the fades have already retired cannot dilute
   * the ratio in either direction.
   *
   * A control render per view (the same state twice) must differ nowhere, for
   * the same reason `contrastProbe` takes one: a rasterizer with any noise of
   * its own would show up here as quad-locked energy that no shader wrote.
   *
   * Allocates and renders 5N frames; never call it from the RAF path.
   */
  aliasProbe(views, minDelta) {
    const u = this._terrainUniforms;
    if (!u) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const ship = new Uint8Array(w * h * 4);
    const control = new Uint8Array(w * h * 4);
    const nobump = new Uint8Array(w * h * 4);
    const flat = new Uint8Array(w * h * 4);
    const nograin = new Uint8Array(w * h * 4);
    const luma = (buf, p) => (buf[p] * 2 + buf[p + 1] * 5 + buf[p + 2]) >> 3;
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();
    const keepOct = u.uOct.value.clone();
    const keepGrainAmp = u.uGrainAmp.value.clone();
    const mask = new Uint8Array(w * h);
    const samples = [];

    // The quad statistic over one frame, restricted to `mask`. Steps are
    // bucketed by the parity of the lower coordinate: a step from an even x to
    // its odd neighbour stays inside a quad, a step from an odd x to the next
    // even one crosses into the next one. `readPixels` row 0 is GL's bottom
    // row and the quad grid is aligned to it, so the same parity argument
    // holds vertically without a flip.
    const quadStat = (buf) => {
      let inSum = 0;
      let inN = 0;
      let outSum = 0;
      let outN = 0;
      for (let y = 0; y < h; y++) {
        const row = y * w;
        for (let x = 0; x < w; x++) {
          const i = row + x;
          if (!mask[i]) continue;
          const l = luma(buf, i * 4);
          if (x + 1 < w && mask[i + 1]) {
            const d = Math.abs(luma(buf, (i + 1) * 4) - l);
            if (x & 1) { outSum += d; outN++; } else { inSum += d; inN++; }
          }
          if (y + 1 < h && mask[i + w]) {
            const d = Math.abs(luma(buf, (i + w) * 4) - l);
            if (y & 1) { outSum += d; outN++; } else { inSum += d; inN++; }
          }
        }
      }
      const within = inN > 0 ? inSum / inN : 0;
      const across = outN > 0 ? outSum / outN : 0;
      return {
        within,
        across,
        steps: inN + outN,
        // Guarded so an empty or perfectly flat mask reports 1 (no quad
        // structure) rather than an infinity that would read as a failure.
        ratio: within > 1e-6 ? across / within : 1,
      };
    };

    for (const view of views) {
      cam.position.set(view.eye[0], view.eye[1], view.eye[2]);
      this.sky.position.copy(cam.position);
      this._target.set(view.at[0], view.at[1], view.at[2]);
      cam.lookAt(this._target);

      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, ship);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
      u.uOct.value.set(0, 0);
      u.uGrainAmp.value.setX(0);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, nobump);
      u.uOct.value.copy(keepOct);
      u.uGrainAmp.value.copy(keepGrainAmp);
      u.uGrainAmp.value.setX(0);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, nograin);
      u.uGrainAmp.value.copy(keepGrainAmp);
      u.uSurface.value = 0;
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flat);
      u.uSurface.value = 1;

      let masked = 0;
      let noise = 0;
      for (let i = 0; i < w * h; i++) {
        const p = i * 4;
        const a = luma(ship, p);
        if (a !== luma(control, p)) noise++;
        const m = Math.abs(a - luma(flat, p)) > minDelta ? 1 : 0;
        mask[i] = m;
        masked += m;
      }
      samples.push({
        label: view.label,
        masked,
        maskFraction: masked / (w * h),
        noise,
        ship: quadStat(ship),
        nobump: quadStat(nobump),
        nograinbump: quadStat(nograin),
        flat: quadStat(flat),
      });
    }

    u.uOct.value.copy(keepOct);
    u.uGrainAmp.value.copy(keepGrainAmp);
    u.uSurface.value = 1;
    cam.position.copy(keepPos);
    this.sky.position.copy(cam.position);
    cam.quaternion.copy(keepQ);
    this.renderer.render(this.scene, cam);
    return { width: w, height: h, samples };
  }

  /**
   * Dev-only: do the PROPS have a surface, or only a silhouette?
   *
   * Every probe above this one asks its question of the ground. The visual
   * judge's ranked gap 1 is about everything else — "rock, wood and canopy are
   * each one flat colour per facet", a 4,386-pixel boulder facet at luma sd
   * 0.96 — so this one holds the frame fixed, toggles the prop field off at
   * `propToggle`, and measures what the difference is made of.
   *
   * Three measures per view, because the first two have both been shown
   * elsewhere in this file not to bite on their own:
   *
   *   · **moved** — how much of the frame the field reaches, and which way. A
   *     field that darkens everything is a wash, not a surface (assertion 15's
   *     lesson, and the mutation that taught it collapsed the noise scales and
   *     sailed past a fraction floor).
   *   · **contrast** — mean neighbour-to-neighbour luma step INSIDE the moved
   *     set, in both states. Texture is by definition what changes between one
   *     pixel and the next; a tint moves the pixels without moving the detail
   *     between them (assertion 15b's argument, applied to props).
   *   · **chroma** — the spread of chromaticity over the moved set, in both
   *     states. Every prop term before this one multiplied albedo by a scalar,
   *     and `k*(r,g,b)` has the chromaticity of `(r,g,b)`, so a luma-only
   *     measure is structurally blind to the deviation that gives a granite
   *     boulder its two minerals (assertion 15d's argument, same).
   *
   * Plus a control render — the same state twice — which must differ nowhere,
   * so a measurement can never be renderer noise wearing a verdict.
   */
  propProbe(views, minDelta) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const ship = new Uint8Array(w * h * 4);
    const control = new Uint8Array(w * h * 4);
    const flat = new Uint8Array(w * h * 4);
    const mask = new Uint8Array(w * h);
    const diff = new Float32Array(w * h);
    const luma = (buf, p) => (buf[p] * 2 + buf[p + 1] * 5 + buf[p + 2]) >> 3;
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();
    const samples = [];

    // Mean |ΔL| between neighbouring pixels, both of which are in the mask.
    const contrast = (buf) => {
      let sum = 0;
      let n = 0;
      for (let y = 0; y < h; y++) {
        const row = y * w;
        for (let x = 0; x < w; x++) {
          const i = row + x;
          if (!mask[i]) continue;
          const l = luma(buf, i * 4);
          if (x + 1 < w && mask[i + 1]) {
            sum += Math.abs(luma(buf, (i + 1) * 4) - l);
            n++;
          }
          if (y + 1 < h && mask[i + w]) {
            sum += Math.abs(luma(buf, (i + w) * 4) - l);
            n++;
          }
        }
      }
      return n > 0 ? sum / n : 0;
    };

    // The same neighbour statistic taken on the field's OWN difference image
    // (ship − flat), normalised by that image's magnitude — so it says what the
    // field is made of and nothing about what it was laid on.
    //
    // `contrast` above compares two states, and its denominator carries the
    // mesh's facet edges, its vertex-colour ramp and the shadow map, none of
    // which the toggle removes. That makes the ratio a statement about the
    // object as much as about the material: measured, the same field scores
    // x1.62 on a pale faceted boulder and x1.26 on a dark canopy whose baseline
    // already has structure in it. This one has no baseline. A constant offset
    // — a wash, a tint, an exposure slip — has zero neighbour variation in the
    // difference image and scores exactly 0, by construction and not by
    // calibration.
    const diffStructure = () => {
      let mag = 0;
      let magN = 0;
      let step = 0;
      let stepN = 0;
      for (let y = 0; y < h; y++) {
        const row = y * w;
        for (let x = 0; x < w; x++) {
          const i = row + x;
          if (!mask[i]) continue;
          mag += Math.abs(diff[i]);
          magN++;
          if (x + 1 < w && mask[i + 1]) {
            step += Math.abs(diff[i + 1] - diff[i]);
            stepN++;
          }
          if (y + 1 < h && mask[i + w]) {
            step += Math.abs(diff[i + w] - diff[i]);
            stepN++;
          }
        }
      }
      if (magN === 0 || stepN === 0) return { mag: 0, step: 0, ratio: 0 };
      const m = mag / magN;
      const s = step / stepN;
      return { mag: m, step: s, ratio: m > 1e-6 ? s / m : 0 };
    };

    // Chromaticity spread over the mask: r/(r+g+b), b/(r+g+b), which is
    // brightness-free by construction, so a term that only moved value scores
    // exactly the same in both states.
    const chroma = (buf) => {
      let sr = 0;
      let sb = 0;
      let n = 0;
      for (let i = 0; i < w * h; i++) {
        if (!mask[i]) continue;
        const p = i * 4;
        const s = buf[p] + buf[p + 1] + buf[p + 2];
        if (s < 12) continue; // near-black carries no usable chromaticity
        sr += buf[p] / s;
        sb += buf[p + 2] / s;
        n++;
      }
      if (n === 0) return 0;
      const mr = sr / n;
      const mb = sb / n;
      let acc = 0;
      for (let i = 0; i < w * h; i++) {
        if (!mask[i]) continue;
        const p = i * 4;
        const s = buf[p] + buf[p + 1] + buf[p + 2];
        if (s < 12) continue;
        const dr = buf[p] / s - mr;
        const db = buf[p + 2] / s - mb;
        acc += dr * dr + db * db;
      }
      return Math.sqrt(acc / n);
    };

    // The delivered VALUE the class occupies, as a histogram over the same
    // mask — and the one measure above that is not a ratio.
    //
    // Every other number this probe returns is scale-free, and that is a hole
    // rather than a virtue. `contrastRatio`, `diffStructure` and `chromaRatio`
    // all divide the field by itself or by what it was laid on, so a field
    // that swings +-1 luma on a base of 10 scores EXACTLY what the same field
    // swinging +-20 on a base of 120 scores. That is how prop surfaces v0
    // shipped green — structure 0.050 and 0.041, well clear of every floor —
    // while the visual judge measuring the merged frames found "a solid",
    // best-fit-plane residual 1.23/255 over 7,800 px, and named the amplitude
    // rather than the absence as the bug.
    //
    // So: percentiles rather than a mean, because the question is a RANGE. A
    // class a player can name is one whose lit face and whose shaded face are
    // both recognisably that material, which is the visual rubric's criterion
    // 2 and its exact sentence for our failure — "the same rock asset reads
    // L=78 warm beige in 01/03 and L=10 near-black in 05, so stone has no
    // recognisable value range to identify it by".
    const band = (buf) => {
      const hist = new Int32Array(256);
      let n = 0;
      for (let i = 0; i < w * h; i++) {
        if (!mask[i]) continue;
        hist[luma(buf, i * 4)]++;
        n++;
      }
      if (n === 0) return { p05: 0, p50: 0, p95: 0, mean: 0 };
      const at = (q) => {
        let acc = 0;
        const want = q * n;
        for (let l = 0; l < 256; l++) {
          acc += hist[l];
          if (acc >= want) return l;
        }
        return 255;
      };
      let sum = 0;
      for (let l = 0; l < 256; l++) sum += l * hist[l];
      return { p05: at(0.05), p50: at(0.5), p95: at(0.95), mean: sum / n };
    };

    for (const view of views) {
      cam.position.set(view.eye[0], view.eye[1], view.eye[2]);
      this.sky.position.copy(cam.position);
      this._target.set(view.at[0], view.at[1], view.at[2]);
      cam.lookAt(this._target);

      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, ship);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
      propToggle.value = 0;
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flat);
      propToggle.value = 1;

      let masked = 0;
      let noise = 0;
      let up = 0;
      let down = 0;
      for (let i = 0; i < w * h; i++) {
        const p = i * 4;
        const a = luma(ship, p);
        if (a !== luma(control, p)) noise++;
        const d = a - luma(flat, p);
        const m = Math.abs(d) > minDelta ? 1 : 0;
        mask[i] = m;
        diff[i] = d;
        masked += m;
        if (m) {
          if (d > 0) up++;
          else down++;
        }
      }
      const px = w * h;
      const cShip = contrast(ship);
      const cFlat = contrast(flat);
      const kShip = chroma(ship);
      const kFlat = chroma(flat);
      const ds = diffStructure();
      const bShip = band(ship);
      samples.push({
        label: view.label,
        surface: view.surface || null,
        // Carried through from the caller so a failure can say whether the
        // probe photographed the wrong thing or the right thing badly.
        // `distance` is SPAWN-to-instance — how far the probe had to search —
        // and `viewDistance` is eye-to-target, which is the framing every
        // measure below is actually taken at. The merge-gate judge caught the
        // summary line reporting the first while reading as the second.
        distance: view.distance,
        viewDistance: Math.hypot(
          view.eye[0] - view.at[0],
          view.eye[1] - view.at[1],
          view.eye[2] - view.at[2],
        ),
        instances: view.instances,
        eye: view.eye,
        masked,
        maskFraction: masked / px,
        noise,
        upFraction: up / px,
        downFraction: down / px,
        contrastShip: cShip,
        contrastFlat: cFlat,
        contrastRatio: cFlat > 1e-6 ? cShip / cFlat : 0,
        diffMean: ds.mag,
        diffStep: ds.step,
        diffStructure: ds.ratio,
        chromaShip: kShip,
        chromaFlat: kFlat,
        chromaRatio: kFlat > 1e-9 ? kShip / kFlat : 0,
        // Delivered value, in 8-bit luma, over the same mask. `diffMean` is
        // the field's own amplitude in those same units — the pair answers
        // "is this surface visible", which no ratio above can.
        lumaP05: bShip.p05,
        lumaP50: bShip.p50,
        lumaP95: bShip.p95,
        lumaMean: bShip.mean,
      });
    }

    propToggle.value = 1;
    cam.position.copy(keepPos);
    this.sky.position.copy(cam.position);
    cam.quaternion.copy(keepQ);
    this.renderer.render(this.scene, cam);
    return { width: w, height: h, samples };
  }

  /**
   * Dev-only: where does the ground's fragment budget actually go?
   *
   * `NOW.md` item 1 is a cost question. Grain measured well and did not merge
   * because the terrain program was already too expensive for the browser
   * gate's third tab, and nothing in the tree could say which half was
   * expensive — per-fragment shading, or program size. Both probes that exist
   * prove a system REACHES the image and neither can say what it costs,
   * because both work by weighting a term to zero with a uniform and a
   * uniform cannot remove an instruction.
   *
   * So this compiles the ground six ways (materials.js TERRAIN_VARIANTS) and
   * measures each one:
   *
   *   1. **Compile.** The first render through a variant pays its compile and
   *      link. Timed with `gl.finish()` on both sides, and reported net of a
   *      warm frame at the same scale — on a software rasterizer this is real
   *      CPU on the tab's own thread, which is the join-window suspect. The
   *      `ship` variant is the LIVE material and compiled at boot, so its
   *      figure is near zero by construction; it is the others that say what
   *      compiling one of these programs costs.
   *   2. **Fill.** The same frame rendered at three viewport scales. The
   *      camera's projection is untouched, so every scale draws the same
   *      image at a different pixel count and coverage is held fixed; a
   *      least-squares fit of ms against megapixels then splits the frame
   *      into a per-fragment slope and a fixed cost (vertex work, state, JS).
   *      Level 0's `needsUpdate` is already consumed when the probe runs and
   *      the probe never sets it, so no shadow map is redrawn inside the
   *      timing loop and the slope is the MAIN pass alone.
   *   3. **Attribution.** A time difference between two programs is worth
   *      nothing unless the image difference is known, so each variant's
   *      frame is read back and compared against two references from the
   *      shipped program: `uSurface = 1` (what ships) and `uSurface = 0`
   *      (what the surface probe already calls "the field, removed"). The
   *      `nofield` variant should land on the second exactly — every field
   *      term is multiplied by `uSurface` and 0 × finite is 0 — and the gate
   *      asserts it, which is what makes its timing the field's cost rather
   *      than some other edit's.
   *
   * Every number here is TIMED and this box is a shared 4-core VM running a
   * software rasterizer, so the milliseconds are not a claim about reference
   * hardware. The ratios between variants measured in one sweep on one box
   * are the useful part, and the counted facts (`depthFetches`,
   * `fragmentChars`) are the part that is quotable anywhere.
   *
   * Renders ~18 frames per variant and allocates four framebuffer-sized
   * readback buffers: never the RAF path.
   */
  costProbe(yaw, pitchRad, scales, frames, reps) {
    const hooks = this._terrainCost;
    const u = this._terrainUniforms;
    if (!hooks || !u) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const size = this.renderer.getSize(new THREE.Vector2());
    const shipped = this._terrainMat;
    const ref = new Uint8Array(w * h * 4);
    const flat = new Uint8Array(w * h * 4);
    const flatGrain = new Uint8Array(w * h * 4);
    const shot = new Uint8Array(w * h * 4);
    // The GPU sync. `gl.finish()` alone measured 0.2 ms for a 1280x720 frame
    // on a software rasterizer that needs two orders of magnitude more than
    // that — through Chrome's command buffer the call returns once the
    // commands are consumed, not once the pixels exist, so what it timed was
    // three's JS. A one-pixel `readPixels` cannot return before the frame it
    // reads has actually been rasterized, so that is the barrier both ends of
    // every measurement below use. (The gate's own fit caught this: a frame
    // whose cost did not rise with its pixel count.)
    const sync1 = new Uint8Array(4);
    const sync = () => {
      gl.finish();
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, sync1);
    };

    // Aim, like every other probe: a fill measurement is a measurement of how
    // much ground is on screen, so the bearing has to be pinned or the number
    // is about where the last assertion happened to leave the camera.
    const keepQ = this.camera.quaternion.clone();
    const cp = Math.cos(pitchRad);
    this._dir.set(Math.sin(yaw) * cp, Math.sin(pitchRad), Math.cos(yaw) * cp);
    this._target.copy(this.camera.position).add(this._dir);
    this.camera.lookAt(this._target);

    // The three reference frames, all from the SHIPPED program, so a variant's
    // image is compared against the thing it is a variant of. One per uniform
    // handle a variant has a compiled partner for: `uSurface = 0` is what
    // `nofield` must land on, `uGrain = 0` is what `nograin` must land on.
    this.renderer.render(this.scene, this.camera);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, ref);
    u.uSurface.value = 0;
    this.renderer.render(this.scene, this.camera);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flat);
    u.uSurface.value = 1;
    u.uGrain.value = 0;
    this.renderer.render(this.scene, this.camera);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flatGrain);
    u.uGrain.value = 1;

    const luma = (b, p) => (b[p] * 2 + b[p + 1] * 5 + b[p + 2]) >> 3;
    const compare = (a, b) => {
      let up = 0;
      let down = 0;
      let max = 0;
      for (let p = 0; p < a.length; p += 4) {
        const d = luma(a, p) - luma(b, p);
        if (d > 0) up++;
        else if (d < 0) down++;
        const m = d < 0 ? -d : d;
        if (m > max) max = m;
      }
      return { up, down, changed: up + down, maxDelta: max };
    };

    // The runs, and the CONTROL that calibrates them. The shipped material is
    // measured twice — once first, once last — and the difference between two
    // sweeps of the same program is this box's timing resolution, measured
    // rather than assumed. It is placed at the far end deliberately: adjacent
    // repetitions share their weather, and the number worth having is the one
    // that spans the whole probe.
    //
    // Without it the timed half is unreadable. It was: the first run of this
    // measured the shipped ground at 179 ms/Mpx and the same ground with the
    // whole noise field compiled OUT at 373 — a program doing strictly less
    // work, "measured" twice as slow, because a 700 ms frame on four shared
    // cores drifts further than the thing being measured is worth. A probe
    // that cannot say that about itself will hand back exactly that number
    // with a straight face.
    const runs = hooks.variants().map((m) => ({ label: m.userData.cost.variant, material: m }));
    runs.push({ label: "control", material: shipped });

    const out = [];
    for (const { label, material } of runs) {
      hooks.use(material);
      // Compile + link, paid on the first render through this program.
      sync();
      const c0 = performance.now();
      this.renderer.render(this.scene, this.camera);
      sync();
      const firstMs = performance.now() - c0;
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, shot);

      // The probe's self-test. This is the same batch the sweep times, run
      // once with NO barrier after it — so it measures JS submission and
      // nothing else. `fullFrameMs / submitMs` is then how much of a timed
      // frame is the GPU, and it is the number that catches the bug this
      // probe shipped with for three runs: `gl.finish()` through Chrome's
      // command buffer returned once the commands were consumed, not once
      // the pixels existed, and every "frame time" was 0.2 ms of JS. The
      // ratio was ~1 then and is ~100 now. It is a check on the instrument,
      // not on the hardware — which is why it is a ratio and not a duration.
      sync();
      const u0 = performance.now();
      for (let f = 0; f < frames; f++) this.renderer.render(this.scene, this.camera);
      const submitMs = (performance.now() - u0) / frames;
      sync();

      const points = [];
      for (const s of scales) {
        const vw = Math.max(1, Math.round(size.x * s));
        const vh = Math.max(1, Math.round(size.y * s));
        this.renderer.setScissorTest(true);
        this.renderer.setViewport(0, 0, vw, vh);
        this.renderer.setScissor(0, 0, vw, vh);
        // Min over repetitions, not mean: on a box that shares four cores
        // with nineteen other services, contention only ever adds time, so
        // the smallest observation is the least contaminated one.
        let best = Infinity;
        let calls = 0;
        for (let r = 0; r < reps; r++) {
          sync();
          this.renderer.info.reset();
          const t0 = performance.now();
          for (let f = 0; f < frames; f++) this.renderer.render(this.scene, this.camera);
          sync();
          const ms = (performance.now() - t0) / frames;
          calls = this.renderer.info.render.calls / frames;
          if (ms < best) best = ms;
        }
        points.push({
          scale: s,
          megapixels: (vw * vh * this.renderer.getPixelRatio() ** 2) / 1e6,
          msPerFrame: best,
          // What the timed frames actually submitted. A fill measurement over
          // an empty scene would fit a clean line through nothing.
          calls,
        });
      }
      this.renderer.setScissorTest(false);
      this.renderer.setViewport(0, 0, size.x, size.y);
      this.renderer.setScissor(0, 0, size.x, size.y);

      // Least squares on (megapixels, ms): slope is fill, intercept is
      // everything a smaller viewport does not make cheaper.
      let sx = 0;
      let sy = 0;
      let sxx = 0;
      let sxy = 0;
      for (const p of points) {
        sx += p.megapixels;
        sy += p.msPerFrame;
        sxx += p.megapixels * p.megapixels;
        sxy += p.megapixels * p.msPerFrame;
      }
      const n = points.length;
      const den = n * sxx - sx * sx;
      const msPerMpx = den === 0 ? 0 : (n * sxy - sx * sy) / den;
      const fixedMs = den === 0 ? 0 : (sy * sxx - sx * sxy) / den;
      const full = points.find((p) => p.scale === 1) || points[0];

      out.push({
        ...material.userData.cost,
        ...(material.userData.programStats || {}),
        variant: label,
        firstFrameMs: firstMs,
        submitMs,
        compileMs: firstMs - full.msPerFrame,
        msPerMpx,
        fixedMs,
        fullFrameMs: full.msPerFrame,
        points,
        vsShipped: compare(shot, ref),
        vsFlat: compare(shot, flat),
        vsFlatGrain: compare(shot, flatGrain),
      });
    }

    hooks.use(shipped);
    u.uSurface.value = 1;
    u.uGrain.value = 1;
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return {
      width: w,
      height: h,
      pixels: w * h,
      pixelRatio: this.renderer.getPixelRatio(),
      yaw,
      pitch: pitchRad,
      frames,
      reps,
      variants: out,
    };
  }

  /** The material system's structural facts, for the browser gate. */
  materials() {
    const m = this._terrainMat;
    return {
      ...materialFacts(),
      terrain: {
        type: m ? m.type : null,
        // The splat shader is a patch on a stock standard material; the
        // uniforms it hands back are the proof the patch is installed.
        patched: !!this._terrainUniforms,
        surface: this._terrainUniforms ? this._terrainUniforms.uSurface.value : null,
        // The second pass's handle, read off the LIVE uniform: it is a probe
        // input and it must ship armed, so a probe that forgets to put it back
        // — or a merge that lands it at 0 — is a gate failure and not an
        // invisible loss of the surface.
        grain: this._terrainUniforms ? this._terrainUniforms.uGrain.value : null,
        // The fourth pass's handle, read off the live uniform for the same
        // reason: it ships armed or the ground goes back to four flat hues.
        tint: this._terrainUniforms ? this._terrainUniforms.uTint.value : null,
        roughness: m ? m.roughness : null,
        // What the shipped ground program costs per fragment and how big its
        // source is — counted, so quotable anywhere (the fill times next to
        // them in costProbe are not). `programStats` is null until the first
        // compile, which has long happened by the time the gate reads this.
        cost: m ? m.userData.cost : null,
        programStats: m ? m.userData.programStats || null : null,
      },
      tiers: this._tierMats.map((t) => [t.type, t.roughness, t.metalness]),
      water: [this.water.material.roughness, this.water.material.metalness],
      remote: [this._remoteMat.roughness, this._remoteMat.metalness],
    };
  }

  /** The structural facts about the rig, for the browser gate to assert. */
  lighting() {
    const near = this.clipmap.levels[0];
    return {
      shadowMap: this.renderer.shadowMap.enabled,
      shadowType: this.renderer.shadowMap.type,
      sunCasts: this.sun.castShadow,
      mapSize: near.light.shadow.mapSize.x,
      // The near level's numbers keep the names lighting v0 published: it IS
      // the map that shipped, so the assertions written against it still mean
      // the same thing. What the clipmap adds is reported under `clipmap`.
      radiusM: near.halfWidth,
      texelM: near.texelM,
      normalBias: near.light.shadow.normalBias,
      clipmap: this.clipmap.facts(),
      // Where the sun is, so a probe can aim relative to it instead of
      // hardcoding a bearing that a lighting change would silently rot.
      sunAzimuth: SUN_AZIMUTH,
      sunElevation: SUN_ELEVATION,
      toneMapping: this.renderer.toneMapping,
      exposure: this.renderer.toneMappingExposure,
      fillIntensity: this.fill.intensity,
      sunIntensity: this.sun.intensity,
      // The daylight register's structural half. `near`/`far` are read off
      // the live fog object rather than off the constants, because
      // `daylightProbe` moves them and a probe that forgot to put them back
      // must be a gate failure and not an invisible loss of the air.
      air: this.scene.fog
        ? {
            near: this.scene.fog.near,
            far: this.scene.fog.far,
            color: this.scene.fog.color.getHex(),
            // The ring the player can actually see in detail (5×5 chunks of
            // 64 m): the near plane has to be inside it or the term exists
            // only in the source.
            ringM: 160,
          }
        : null,
      sky: {
        // The dome is a patched stock material now, not a vertex ramp: a
        // gradient reconstructed across 24×16 vertices is what banded, and
        // it is also what could not hold a disc. Both facts are asserted.
        patched: !!this.sky.material.userData.uniforms,
        vertexColors: this.sky.material.vertexColors === true,
        // The horizon ring is the fog colour by construction (`air.color`);
        // this is the other end of the ramp.
        zenith: SKY_ZENITH,
        curve: SKY_CURVE,
        dither: SKY_DITHER,
        discRad: SUN_DISC_RAD,
        discGain: SUN_DISC_GAIN,
        glowGain: SUN_GLOW_GAIN,
      },
      calls: this.stats.calls,
      triangles: this.stats.triangles,
      peakCalls: this.stats.peakCalls,
      peakTriangles: this.stats.peakTriangles,
    };
  }

  /**
   * Dev-only: does the shadow map actually darken the frame?
   *
   * A flag says the renderer was ASKED for shadows. This measures whether
   * any pixel got one. Per sample yaw it renders the live scene three times
   * and reads the drawing buffer back twice:
   *
   *   1. every level forced to redraw, all levels contributing — the frame
   *      as it ships, and the draw count INCLUDING every shadow pass;
   *   2. the identical frame with `needsUpdate` already consumed, so three
   *      skips the shadow passes and redraws nothing but the main one. The
   *      pixels are bit-identical (the maps did not change); the difference
   *      in draw calls IS the shadow passes, exactly;
   *   3. `uClipLevels = 0`, which gives every level zero containment weight
   *      and resolves the whole factor to unshadowed. Same programs, same
   *      maps, same geometry — the ONLY thing removed is the shadow term.
   *
   * Splitting it that way is why this measures more than the old two-render
   * flip did: that one toggled `castShadow`, which changed the lights-state
   * version, recompiled every program and moved the draw count and the
   * pixels together. Here each number comes from a frame that differs in one
   * respect. It restores the camera, the level count and the frame before
   * returning, so a probe leaves nothing behind.
   *
   * Allocates and renders 3N frames: never call it from the RAF path. It
   * exists for ci/browser_smoke.mjs.
   */
  shadowProbe(yaws, pitchRad, minDelta) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const lit = new Uint8Array(w * h * 4);
    const shad = new Uint8Array(w * h * 4);
    const keepQ = this.camera.quaternion.clone();
    const pos = this.camera.position;
    const samples = [];
    let darkened = 0;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      this.camera.lookAt(this._target);
      setClipmapActiveLevels(LEVEL_COUNT);
      for (const L of this.clipmap.levels) L.light.shadow.needsUpdate = true;
      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);
      const callsShadowed = this.renderer.info.render.calls;
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, shad);
      // three cleared needsUpdate on every level it just drew, and every
      // level owns autoUpdate=false, so this frame skips the shadow passes.
      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);
      const callsUnshadowed = this.renderer.info.render.calls;
      setClipmapActiveLevels(0);
      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, lit);
      setClipmapActiveLevels(LEVEL_COUNT);
      let n = 0;
      let sum = 0;
      let max = 0;
      let litSum = 0;
      let shadSum = 0;
      for (let p = 0; p < lit.length; p += 4) {
        // Rec.601-ish integer luma; the absolute scale does not matter,
        // only the difference between two renders of the same pixel.
        const a = (lit[p] * 2 + lit[p + 1] * 5 + lit[p + 2]) >> 3;
        const b = (shad[p] * 2 + shad[p + 1] * 5 + shad[p + 2]) >> 3;
        litSum += a;
        shadSum += b;
        const d = a - b;
        if (d > minDelta) {
          n++;
          sum += d;
          if (d > max) max = d;
        }
      }
      samples.push({
        yaw: yaws[i],
        darkened: n,
        fraction: n / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
        // Whole-frame means, so a probe that reads back nothing at all is
        // distinguishable from a rig that casts nothing.
        litMean: litSum / (w * h),
        shadowedMean: shadSum / (w * h),
        // The same frame drawn with and without the shadow pass. The
        // difference IS the shadow pass, which is how the draw budget below
        // is shown to be counting it.
        callsShadowed,
        callsUnshadowed,
      });
      darkened += n;
    }
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return { width: w, height: h, pixels: w * h * yaws.length, darkened, samples };
  }

  /**
   * Dev-only: does anything past the NEAR level's box cast a shadow?
   *
   * The whole claim of this slice. Per sample yaw it renders the live scene
   * twice — the whole clipmap, then `uClipLevels = 1`, which is exactly
   * lighting v0's reach (level 0 alone, everything else resolving to
   * unshadowed) — and counts pixels the coarse levels took DOWN.
   *
   * The part that makes it a claim about distance rather than about darkness
   * is the camera. The probe lifts it `heightM` above the player, pitches it
   * down, pushes the near plane out to `nearM` and narrows the FOV, so no
   * fragment nearer than that is drawn at all, and then it measures — it does
   * not assume — how far the frame is from the near level's box: it
   * unprojects all eight frustum corners, transforms them into light space,
   * and returns the smallest |x − centre| among them.
   *
   * The lift is why it is worth doing at all. From eye height a near plane at
   * 130 m leaves the distant ground as a few-pixel strip under the horizon —
   * the shadows are there and they are strong, but almost nothing is sampled.
   * From 80 m up the same band is most of the frame. The clipmap's centre
   * stays on the PLAYER either way, so lifting the viewpoint cannot change
   * which level covers what; it only changes how much of that we get to see.
   * Light-space X is the horizontal axis perpendicular to the sun's bearing,
   * and a linear function on a convex hull takes its extremes at the
   * vertices, so when every corner sits on the same side of the centre that
   * minimum bounds the WHOLE frustum. If it exceeds the near level's
   * half-width, every pixel in the frame is outside the near box, and every
   * pixel that moved was shadowed by a coarser level.
   *
   * That axis is chosen, not incidental. Light-space Y mixes the sun-bearing
   * ground distance with height (at a 21° sun, ×0.35 and ×0.94), so the near
   * box already reaches ~227 m along the sun's bearing and a claim there
   * would be weak. X is the direction where the 80 m bound is tight.
   *
   * Allocates and renders 2N frames; never call it from the RAF path.
   */
  farShadowProbe(yaws, pitchRad, minDelta, nearM, fovDeg, heightM) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const full = new Uint8Array(w * h * 4);
    const nearOnly = new Uint8Array(w * h * 4);
    // The probe's own zero point. Two renders of the SAME near-only state
    // must differ nowhere, or the counts below are partly the rasterizer
    // talking (dither, AA, an undrawn frame) and the floor is guarding
    // nothing. Measured rather than argued, on every yaw.
    const control = new Uint8Array(w * h * 4);
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepPos = cam.position.clone();
    const keepNear = cam.near;
    const keepFov = cam.fov;
    cam.position.y += heightM;
    this.sky.position.copy(cam.position); // or the dome is left below us
    const pos = cam.position;
    cam.near = nearM;
    cam.fov = fovDeg;
    cam.updateProjectionMatrix();
    const samples = [];
    let darkened = 0;
    let minLightXm = Infinity;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      cam.lookAt(this._target);
      cam.updateMatrixWorld(true);

      // How far this frustum is from the near level's committed centre,
      // along the axis where the near box is 80 m and not 227.
      const centerX = this.clipmap.levels[0].cx;
      let lo = Infinity;
      let sawPos = false;
      let sawNeg = false;
      for (let c = 0; c < 8; c++) {
        this._corner
          .set((c & 1 ? 1 : -1), (c & 2 ? 1 : -1), (c & 4 ? 1 : -1))
          .applyMatrix4(cam.projectionMatrixInverse)
          .applyMatrix4(cam.matrixWorld);
        const dx = this.clipmap.toLight(this._corner).x - centerX;
        if (dx > 0) sawPos = true;
        else sawNeg = true;
        if (Math.abs(dx) < lo) lo = Math.abs(dx);
      }
      // The frustum straddles the centre plane: no bound holds, and the
      // sample must not be able to claim one.
      const reach = sawPos && sawNeg ? 0 : lo;
      if (reach < minLightXm) minLightXm = reach;

      setClipmapActiveLevels(LEVEL_COUNT);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, full);
      setClipmapActiveLevels(1);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, nearOnly);
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, control);
      setClipmapActiveLevels(LEVEL_COUNT);

      let n = 0;
      let sum = 0;
      let max = 0;
      let lifted = 0;
      let noise = 0;
      for (let p = 0; p < full.length; p += 4) {
        const a = (nearOnly[p] * 2 + nearOnly[p + 1] * 5 + nearOnly[p + 2]) >> 3;
        const b = (full[p] * 2 + full[p + 1] * 5 + full[p + 2]) >> 3;
        const c = (control[p] * 2 + control[p + 1] * 5 + control[p + 2]) >> 3;
        const cd = a - c;
        if (cd > minDelta || cd < -minDelta) noise++;
        const d = a - b;
        if (d > minDelta) {
          n++;
          sum += d;
          if (d > max) max = d;
        } else if (d < -minDelta) {
          // A coarse level can only ever REMOVE light out here. Anything
          // brighter would mean the extra levels are lighting the scene
          // rather than shadowing it — the exact bug a zero-intensity level
          // is there to avoid — so it is counted and asserted on.
          lifted++;
        }
      }
      samples.push({
        yaw: yaws[i],
        darkened: n,
        lifted,
        noise,
        fraction: n / (w * h),
        liftedFraction: lifted / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
        reachM: reach,
      });
      darkened += n;
    }
    cam.near = keepNear;
    cam.fov = keepFov;
    cam.updateProjectionMatrix();
    cam.quaternion.copy(keepQ);
    cam.position.copy(keepPos);
    this.sky.position.copy(keepPos);
    this.renderer.render(this.scene, cam);
    return {
      width: w,
      height: h,
      pixels: w * h * yaws.length,
      darkened,
      minLightXm,
      heightM,
      nearLevelHalfWidthM: this.clipmap.levels[0].halfWidth,
      activeLevels: clipmapActiveLevels(),
      samples,
    };
  }
}
