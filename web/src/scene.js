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

// --- lighting v1: the midday register (DECISIONS.md §open, "lighting v1") ---
// One key, one fill, one bounded shadow map, one tone map — lighting v0's
// shape, re-metered end to end by a single owner, because sky, sun, fill,
// exposure, transfer, fog and shadow are one coupled set and moving any of
// them alone breaks the assumptions of the rest (`CLAUDE.md`, the
// coupled-lighting trap).
//
// The bar is `Rust Images/`, measured rather than remembered
// (`ci/reference_bar.mjs`): median p10 40 · p50 91 · p90 170 over the six
// outdoor-daylight reference frames. The capture that opened this item
// measured 41 · — · 70 — the shadows were already right and the whole top
// two stops of the image were missing.
//
// Three things were wrong and all three were arithmetic:
//
//   1. **A 21° sun delivers a third of its own light to flat ground.**
//      sin(0.36) = 0.35, so every horizontal surface in the world was lit at
//      35% of the key before albedo. The register is midday now.
//   2. **The transfer squared the shadows.** Khronos PBR Neutral subtracts
//      `x - 6.25x²` from every channel for `x < 0.08` (three r178,
//      `tonemapping_pars_fragment.glsl.js:179`), so for a roughly neutral
//      colour it resolves to `out ≈ 6.25c²` — a QUADRATIC toe over exactly
//      the range a shaded surface occupies. A face arriving at linear 0.02
//      left at 0.0025 and displayed as 8/255. Reinhard has no toe at all
//      (`c/(1+c)`, slope 1 at the origin) and still rolls the highlights off,
//      which is the property Neutral was chosen for. The lighting v0 row
//      rejected ACES for crushing the dark-albedo scatter; this is the same
//      finding, one tone map further down the list.
//   3. **The ambient was a rumour.** 1.15 of a hemisphere whose ground half
//      is 0.15 linear leaves a down-facing face at ~0.001 — under the toe,
//      under the 8-bit floor, and under any surface field the materials lane
//      can author. `DECISIONS.md` §open "prop albedo v1" reproduced the
//      visual judge's measured canopy underside, RGB (2,6,0), from these
//      constants alone. That is the arithmetic this row inherits and answers.
//
// Nothing here is a post stack; the only stages are still light ratios →
// tone map → sRGB output.

// Where the sun sits. Azimuth is the compass bearing of the sun itself
// (0 = +Z, increasing toward +X, matching the sim's yaw); elevation is its
// angle above the horizon.
const SUN_AZIMUTH = 2.35;
// **UNCHANGED at 0.36 rad (20.6°), and that is this slice's main finding
// rather than a thing it failed to do.**
//
// `NOW.md` item 1 opens by asking for the sun's register, midday. It was
// built, twice, and measured, and the world is not ready for it — for a
// reason that is arithmetic:
//
//   A normal perturbed by δ changes `N·L` on flat ground by `cot(elevation)·δ`
//   RELATIVE to the unperturbed value. So the ground's entire bump relief —
//   every octave of it, the thing that makes a heightfield read as a surface
//   — scales with cot(elevation), and nothing else about the rig can put it
//   back.
//
// Measured, with the shipped field byte-for-byte unchanged and only this
// constant moved (`browser_smoke` assertion 15, the surface probe):
//
//   | elevation | cot  | frame moved | mean Δluma | brightened, worst yaw |
//   |-----------|------|-------------|------------|-----------------------|
//   | 0.36 rad  | 2.66 | 12.81%      | ~19        | +0.5%  (floor 0.2%)   |
//   | 0.50 rad  | 1.83 |  2.03%      | 7.2-8.4    | +0.01%                |
//   | 0.785 rad | 1.00 |  0.47%      | 7.0-7.8    | +0.00%                |
//
// The last column is the one that settles it. Assertion 15's two-sidedness —
// every yaw must brighten pixels as well as darken them — is the wall that
// separates a FIELD from a wash, it already runs on 2.5× of margin at this
// spawn, and the pass before this one built a bump fix, measured it, and
// deliberately did not ship it rather than spend that margin (`DECISIONS.md`
// §open, "the quad-constant gradient"). Raising the sun spends it 20× over.
// A gate saying "the world you are about to light has no relief left" is a
// gate doing its job, and the answer is not to lower it.
//
// **What the register cost instead: nothing.** p10/p50/p90 are set by the
// transfer and the exposure, not by where the sun is — this slice lands them
// on the reference bar with the sun exactly where it was. What the elevation
// actually owns is shadow LENGTH and bump relief, and it is now blocked, with
// a number, on the ground's structure moving from bump into albedo. That is
// already `NOW.md` item 2's top want, and it now has a second reason and a
// measured exit condition: when assertion 15 holds its margins with the bump
// removed, this constant can rise.
const SUN_ELEVATION = 0.36;
// A midday sun is near-white. 0xffe1b8 was a low-sun colour attached to a
// low sun and it survived every earlier pass by looking deliberate.
const SUN_COLOR = 0xfff4e2;
const SUN_INTENSITY = 3.0;
// The fill is sky-above / earth-below and it is the whole ambient budget:
// there is no GI here, so this hemisphere IS every bounce in the world.
//
// **The two halves are separate knobs and this slice moves only one of them,
// which took two goes to see.** A hemisphere light lands
// `mix(ground, sky, 0.5 + 0.5·N·y)`, so:
//
//   · UP-facing ground in shadow is lit by the SKY half alone. That is what
//     `browser_smoke`'s shadow probe photographs, and its 15% / 10% floors
//     are calibrated against its depth.
//   · DOWN-facing prop faces are lit by the GROUND half alone. That is the
//     (2,6,0) canopy underside `DECISIONS.md` §open "prop albedo v1"
//     reproduced from these constants, and the thing this slice has to fix.
//
// The first cut raised INTENSITY to 1.5 and moved both. It bought the prop
// floor and cost the shadow probe half its darkened share (24.0% -> 11.5%,
// worst yaw 20.4% -> 4.6%) — because a lighter shadow moves fewer pixels past
// a fixed 6-level threshold, which is arithmetic and not a bug, but it is
// also two floors lowered for a fix that only needed the other half.
//
// So: the sky half is held at the value lighting v0 shipped (0.478/0.625/0.812
// linear against its 0.455/0.627/0.873 — a hue change, not a level one) and
// the earth half is raised 2.4x, which is where all of the down-facing lift
// comes from. Nothing about ground shadow depth moved, and no shadow floor
// had to.
const FILL_SKY = 0xbcd4ee;
const FILL_GROUND = 0xa89b7e;
const FILL_INTENSITY = 0.95;
// One tone map, owned by the renderer. No material sets its own.
//
// Metered against the measured bar rather than picked. The first cut ran at
// 1.0 and landed p10 45 · p50 138 · p90 148 against a reference of 40 · 91 ·
// 170: the midtones a stop and a half OVER the bar and the top of the image
// still missing, which is a narrower failure than the one this item opened
// with and the same shape. The scene's own linear p90/p50 was 1.25 where the
// reference's is 3.93 — no transfer can widen that, because the range was not
// there to compress. The sky is where it comes from (see SKY_GAIN); this pulls
// the midtones back down onto the bar once it is.
const EXPOSURE = 1.0;
// Fog and the sky dome share their horizon colour — the same constant, not
// two that agree — so the seam is exact by construction. Fog engages at 50 m
// because that is the distance range these frames actually show: at 180 m
// nothing inside a first-person framing was ever touched by it, and the
// visual judge measured water holding luminance 69→70 from 20 m to the
// horizon and then stepping 31 levels at the seam.
const FOG_NEAR = 50;
const FOG_FAR = 1000;
// The sky, authored as three bands rather than two. The horizon band is
// nearly flat for the first few degrees — that flat band IS the seam, and a
// single `y^0.62` ramp from the horizon colour put 16% of the whole gradient
// into the first 3° above eye level, which is the step the judge measured.
const SKY_HORIZON = 0xa9c6e0;
const SKY_HAZE = 0x8fb4d8;
const SKY_ZENITH = 0x4b7cb4;
// …and the reason the sky is not just another hex: **it is the only HDR
// surface in this scene, and it is where the image's top decile comes from.**
//
// A hex is a value in [0,1] linear, so a dome authored as one can be at most
// as radiant as a perfectly white diffuse surface in full sun — and it
// measured exactly that, sky 142 against ground 138, an image with no
// highlight anywhere in it. In the reference frames the sky is 1.6-2x the
// ground's median in DISPLAY, which after the transfer is nearly 4x in linear.
// So the dome's three colours are multiplied up out of the [0,1] box, which is
// what "the sky is brighter than anything it lights" means in a number.
//
// The same gain goes on the fog colour, and that is not a coincidence — it is
// the seam. Fully-fogged geometry has to arrive at the tone mapper carrying
// the SAME linear value as the sky above it, so if one is gained and the other
// is not, the horizon steps by the gain. (It also happens to be correct: haze
// at 800 m is as radiant as the sky, which is why distance washes out.)
const SKY_GAIN = 1.15;
// Where the haze band ends and the zenith ramp begins, as sin(elevation).
const SKY_HAZE_TOP = 0.22; // ~12.7° above the horizon
const SKY_CURVE = 1.15; // haze→zenith ramp; >1 holds the low sky pale
// The sun in the sky, which is a different object from the sun that lights
// the world and has to agree with it. Angular radius is 0.6°, roughly twice
// life size: the real 0.27° disc is 3 px across at this FOV and reads as a
// dead pixel, not a sun. The glow is the circumsolar aureole — the reason
// you can find the sun in an overcast photograph — and it is what makes the
// dome read as "lit by" rather than "painted with" a gradient.
const SUN_DISC_RAD = 0.0105;
const SUN_DISC_SOFT = 0.006; // limb softness, in radians of arc
const SUN_GLOW_POWER = 220; // cos^n falloff; n=220 is a ~10° aureole
const SUN_GLOW_GAIN = 0.55;
// The disc is HDR too, and by a wider margin than the sky: it is the source.
// Sized so the disc clears the dome around it by ~60 levels after the
// transfer, and small enough (1.2° of arc, ~13 px at this FOV) that it cannot
// blow out a measurable share of the frame.
const SUN_DISC_GAIN = 6.0;
const SUN_SKY_COLOR = 0xfff6e6;
// Hash dither, as a FRACTION of the value it perturbs (see the shader). The
// dome is a smooth ramp quantized to 8 bits at the very end, so it bands: the
// judge counted 131 distinct values over 360 rows with an 11 px longest flat
// run and named "no dither". One level of noise under the quantizer is the
// standard answer and costs one hash. 0.05 lands ~±1 display level across the
// whole dome; the gate measures the result as the share of adjacent sky
// pixels that differ at all, which is what banding actually is.
const SKY_DITHER = 0.05;
const SKY_RADIUS = 10;

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
 * The transfer: Khronos PBR Neutral with its black offset removed.
 *
 * Installed through three's own `CustomToneMapping` hook — one string
 * replacement in the shared chunk, done once at module load — so tone-map
 * ownership stays exactly where lighting v0 put it: the renderer maps, no
 * material sets its own, nothing re-encodes downstream.
 *
 * Why not one of the four three ships, all of which were built and measured
 * against this scene:
 *
 *   · **Neutral** subtracts `x - 6.25x²` from every channel for `x < 0.08`,
 *     which for a roughly neutral colour resolves to `out ≈ 6.25c²` — a
 *     quadratic toe over precisely the range a shaded surface occupies. It is
 *     why a prop face with in-band albedo displayed at 6/255. Its own author's
 *     intent is a filmic black; the effect here is a crushed one.
 *   · **ACES** was measured and rejected by lighting v0 for the same disease,
 *     worse.
 *   · **Reinhard** has no toe at all and fixed the shadows — and cost the
 *     midtones, because `c/(1+c)` compresses everywhere with slope
 *     `1/(1+c)²`. Measured: at the register below it took the shadow probe's
 *     darkened share to 14.24% against a 15% floor and mean Δluma to 20-26,
 *     with the light rig identical. A shadow that IS there and reads shallow
 *     is a different bug from a shadow that is crushed, and swapping one for
 *     the other is not progress.
 *   · **Linear** clips instead of rolling off, so the sun disc and every
 *     water specular hard-clip to white with a hue shift.
 *
 * What is left is what Neutral is once the offset is gone: EXACTLY identity
 * below 0.76 — no toe, no midtone compression, so the light rig's own
 * contrast reaches the framebuffer unmodified — and Neutral's hyperbolic
 * shoulder with its 0.15 desaturation above it, which is the part that was
 * always right. Both constants are Neutral's own, unchanged.
 */
function installToneMap() {
  const stock = "vec3 CustomToneMapping( vec3 color ) { return color; }";
  const chunk = THREE.ShaderChunk.tonemapping_pars_fragment;
  // Read off the installed chunk and required to appear exactly once — the
  // same discipline the clipmap's `getShadow` patch uses. A three upgrade that
  // renames or reformats this function throws at boot rather than silently
  // leaving the scene on an identity transfer.
  const at = chunk.indexOf(stock);
  if (at < 0 || chunk.indexOf(stock, at + 1) >= 0) {
    throw new Error(
      "three's CustomToneMapping stub is not present exactly once in " +
        "tonemapping_pars_fragment — the transfer patch cannot be installed",
    );
  }
  THREE.ShaderChunk.tonemapping_pars_fragment = chunk.replace(
    stock,
    /* glsl */ `
vec3 CustomToneMapping( vec3 color ) {
  // Khronos PBR Neutral, minus the \`x - 6.25x*x\` black offset. Both
  // constants below are Neutral's own (three r178).
  const float StartCompression = 0.8 - 0.04;
  const float Desaturation = 0.15;
  color *= toneMappingExposure;
  float peak = max( color.r, max( color.g, color.b ) );
  if ( peak < StartCompression ) return color;
  float d = 1. - StartCompression;
  float newPeak = 1. - d * d / ( peak + d - StartCompression );
  color *= newPeak / peak;
  float g = 1. - 1. / ( Desaturation * ( peak - newPeak ) + 1. );
  return mix( color, newPeak * vec3( 1, 1, 1 ), g );
}`,
  );
}
installToneMap();

/**
 * The sky, as a shader on a dome rather than a vertex-coloured ramp.
 *
 * Three reasons it stopped being vertex colours on a 24×16 sphere:
 *
 *   · **The gradient was linear between rings 11° apart.** Reading the ramp
 *     per fragment is what lets the horizon band be flat and the zenith ramp
 *     be a curve — the shape the seam needs, which a lerp between two vertex
 *     colours cannot hold.
 *   · **A sun disc is a per-fragment question.** It is 1.2° across; the
 *     dome's vertices are 11° apart, so no vertex attribute can carry it.
 *     Criterion 8's "a sky that reads as sky" is, in this scene, mostly the
 *     presence of the thing everything else is lit by.
 *   · **Dither has to happen at the quantizer.** Adding noise to a vertex
 *     colour interpolates it away.
 *
 * The horizon colour and the fog colour are the SAME constant (`SKY_HORIZON`,
 * fed in as `uHorizon` and to `THREE.Fog`), so a fully-fogged surface and the
 * sky above it arrive at the tone mapper carrying identical linear values and
 * the seam is exact by construction rather than by tuning. Everything below
 * `y = 0` stays exactly that colour, so the seam holds under the horizon too
 * — which matters for water, whose far edge IS the seam.
 *
 * Tone mapping and the output transfer are `#include`d explicitly: a raw
 * ShaderMaterial gets neither for free, and a sky that skipped the tone map
 * while the world went through it would break single-transfer ownership in
 * the one place it is most visible.
 */
function skyMaterial(toSun, horizon) {
  return new THREE.ShaderMaterial({
    uniforms: {
      // `horizon` is handed in rather than built here: it is the same value
      // the fog carries, computed once by the caller, so the seam cannot be
      // two constants that have to be kept equal by hand.
      uHorizon: { value: horizon },
      uHaze: { value: new THREE.Color(SKY_HAZE).multiplyScalar(SKY_GAIN) },
      uZenith: { value: new THREE.Color(SKY_ZENITH).multiplyScalar(SKY_GAIN) },
      uSunSky: { value: new THREE.Color(SUN_SKY_COLOR) },
      uToSun: { value: toSun },
    },
    side: THREE.BackSide,
    fog: false,
    depthWrite: false,
    depthTest: false,
    vertexShader: /* glsl */ `
      varying vec3 vDir;
      void main() {
        // The dome is never rotated and is parked on the camera, so its own
        // object space IS world direction. Normalizing here and again in the
        // fragment costs nothing and survives the interpolation.
        vDir = position;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */ `
      // No \`#include <tonemapping_pars_fragment>\` and no
      // \`<colorspace_pars_fragment>\`: three's ShaderMaterial prefix already
      // carries both (WebGLProgram's prefixFragment), so including them here
      // is a redefinition and the whole program fails to link. It did — the
      // dome silently stopped drawing and every vantage measured a flat,
      // un-tone-mapped 194 luma that read as a plausible overcast sky,
      // because what was actually being photographed was the clear colour.
      // The two APPLICATION chunks at the bottom of main() are not in the
      // prefix and do have to be included.
      uniform vec3 uHorizon;
      uniform vec3 uHaze;
      uniform vec3 uZenith;
      uniform vec3 uSunSky;
      uniform vec3 uToSun;
      varying vec3 vDir;

      // Hash without sine (Hoskins) — the same discipline the ground's field
      // is written to. A sin-based hash bands on some drivers, which is the
      // one thing a dither must never do.
      float skyHash(vec2 p) {
        vec3 p3 = fract(vec3(p.xyx) * 0.1031);
        p3 += dot(p3, p3.yzx + 33.33);
        return fract((p3.x + p3.y) * p3.z);
      }

      void main() {
        vec3 dir = normalize(vDir);
        float up = max(dir.y, 0.0);
        // Band 1: the haze. Flat at the horizon, so the fog seam has no step
        // across it, then lifting over the first ${SKY_HAZE_TOP} of sin(elevation).
        vec3 col = mix(uHorizon, uHaze, smoothstep(0.0, ${SKY_HAZE_TOP.toFixed(3)}, up));
        // Band 2: the zenith ramp, over what is left.
        float t = clamp((up - ${SKY_HAZE_TOP.toFixed(3)}) / ${(1.0 - SKY_HAZE_TOP).toFixed(3)}, 0.0, 1.0);
        col = mix(col, uZenith, pow(t, ${SKY_CURVE.toFixed(3)}));

        // The sun: a limb-softened disc inside a circumsolar aureole. Both
        // ride the SAME direction the key light uses, so they cannot drift
        // apart from the shadows they are supposed to explain.
        float c = dot(dir, uToSun);
        float glow = pow(max(c, 0.0), ${SUN_GLOW_POWER.toFixed(1)});
        float disc = smoothstep(
          cos(${(SUN_DISC_RAD + SUN_DISC_SOFT).toFixed(5)}),
          cos(${SUN_DISC_RAD.toFixed(5)}),
          c
        );
        // The aureole fades out below the horizon with the rest of the sky,
        // so a sun near setting does not glow through the ground.
        float above = smoothstep(-0.02, 0.06, dir.y);
        col += uSunSky *
          (glow * ${SUN_GLOW_GAIN.toFixed(3)} + disc * ${SUN_DISC_GAIN.toFixed(2)}) * above;

        // One level of noise under the 8-bit quantizer — RELATIVE, not
        // absolute. The chain from here to the framebuffer is toeless
        // Neutral (installToneMap) then
        // sRGB, and its slope varies 2.9x across this dome (measured: the
        // zenith at linear 0.52 is nearly three times as sensitive as the
        // horizon at 1.4). A constant linear dither is therefore a quarter of
        // a level at one end and four at the other — invisible where it is
        // needed and grain where it is not. Scaling with the value itself
        // tracks the transfer closely enough to land within a level of
        // uniform, at ~1 level peak.
        col *= 1.0 + (skyHash(gl_FragCoord.xy) - 0.5) * ${SKY_DITHER.toFixed(4)};

        gl_FragColor = vec4(max(col, 0.0), 1.0);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }
    `,
  });
}

export class GameScene {
  constructor(canvas) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    // Single tone-map ownership: the renderer maps, materials do not, and
    // nothing re-encodes sRGB downstream. The clear colour is the only
    // surface the tone mapper never sees, which is exactly why the sky is
    // geometry below — the clear is a fallback that the dome always covers.
    // Black, and not the sky's own colour. A clear colour that matches the
    // dome hides the dome failing to draw — which is exactly what happened
    // while this slice was being built: the sky measured a flat, untone-mapped
    // 194 luma across every vantage and read as a plausible overcast, because
    // what the probe was photographing was `setClearColor(SKY_HORIZON)`. The
    // clear is a fallback the dome always covers, so it should look like a
    // fallback.
    this.renderer.setClearColor(0x000000);
    // The toeless Neutral installed above. Single ownership, unchanged: the
    // renderer maps, no material sets its own, nothing re-encodes after.
    this.renderer.toneMapping = THREE.CustomToneMapping;
    this.renderer.toneMappingExposure = EXPOSURE;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    // three resets renderer.info AFTER the shadow pass and before the main
    // one, so the default counters silently exclude every shadow draw — the
    // exact half of the budget this rig just added. Own the reset instead.
    this.renderer.info.autoReset = false;
    this.scene = new THREE.Scene();
    // One value, two consumers. The fog's colour and the dome's horizon band
    // are computed once here and handed to both, gain included, so "the seam
    // is exact by construction" is a fact about the object graph rather than a
    // promise about two hexes.
    this._skyHorizon = new THREE.Color(SKY_HORIZON).multiplyScalar(SKY_GAIN);
    this.scene.fog = new THREE.Fog(this._skyHorizon, FOG_NEAR, FOG_FAR);
    this.camera = new THREE.PerspectiveCamera(
      75,
      window.innerWidth / window.innerHeight,
      0.1,
      1500,
    );

    // World-space unit vector pointing AT the sun. The sun does not move in
    // v1, so the light-space basis it defines is built once, inside the
    // clipmap, and only read in the RAF path. Built BEFORE the sky, because
    // the dome draws the same sun this vector lights the world with — one
    // vector, two consumers, so they cannot disagree.
    const ce = Math.cos(SUN_ELEVATION);
    this._toSun = new THREE.Vector3(
      ce * Math.sin(SUN_AZIMUTH),
      Math.sin(SUN_ELEVATION),
      ce * Math.cos(SUN_AZIMUTH),
    ).normalize();

    // The sky is a dome, not a clear colour, for one reason: fog is shaded
    // and therefore tone-mapped, a clear colour is not. Painting the sky as
    // geometry puts both through the same tone map, so the horizon seam is
    // exact instead of tuned. Its horizon band IS the fog colour.
    this.sky = new THREE.Mesh(
      new THREE.SphereGeometry(SKY_RADIUS, 32, 24),
      skyMaterial(this._toSun, this._skyHorizon),
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
   * Every caster in the scene that is NOT another player — the terrain ring,
   * the far mesh, the scatter pools, pieces and deployables. "The world."
   *
   * This exists so `shadowProbe` can take the mutation that calibrated its
   * floors instead of remembering it. On 2026-08-01 those floors were set by
   * hand-editing `castShadow` off the terrain and the scatter, running the
   * gate, and writing the resulting 6.12% into a comment — a calibration that
   * was true of one sun and one spawn and has no way to notice when it stops
   * being true. Taking it every run makes "the WORLD casts" a measurement.
   *
   * Flipping `Object3D.castShadow` is safe here in a way flipping it on the
   * LIGHT is not: it changes which objects the depth pass draws and nothing
   * about the lights state, so no program is rebuilt and the colour pass is
   * identical (the shadow-clipmap row's warning is about the light, and it
   * still holds).
   *
   * Which is why the `isMesh` test is not a tidiness filter and is load-
   * bearing: `castShadow` is an `Object3D` property, so a bare traverse
   * collects the clipmap's three DirectionalLights along with the geometry,
   * and switching THOSE off is precisely the mutation the row warns about. It
   * was written that way first and measured: the "world's shadow" leg came
   * back claiming 100% of one frame, because turning the key's own
   * `castShadow` off removes the shadow map entirely and re-versions the
   * lights state.
   *
   * Allocates: probes only, never the RAF path.
   */
  _worldCasters() {
    const remoteRoots = new Set();
    for (const r of this.remotes.values()) remoteRoots.add(r.group);
    const out = [];
    this.scene.traverse((o) => {
      if (o.castShadow !== true || o.isMesh !== true) return;
      for (let p = o; p; p = p.parent) if (remoteRoots.has(p)) return;
      out.push(o);
    });
    return out;
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
      // The air and the dome, so the gate can assert the seam is one colour
      // rather than two that happen to match today. `fogColor` is read off
      // the live `THREE.Fog` and `skyHorizon` off the live sky uniform: if
      // anyone ever splits the constant, these stop being equal and 16a
      // fails, which is the only way that regression is visible at all.
      fogNear: this.scene.fog.near,
      fogFar: this.scene.fog.far,
      // Linear RGB triples, not hexes: both carry SKY_GAIN and are therefore
      // above 1.0, where `getHex()` clamps to white and two different colours
      // would compare equal. The seam assertion has to see the actual values.
      fogColor: this.scene.fog.color.toArray(),
      skyHorizon: this.sky.material.uniforms.uHorizon.value.toArray(),
      skyZenith: this.sky.material.uniforms.uZenith.value.toArray(),
      skyGain: SKY_GAIN,
      // The sun the dome draws, and the sun the world is lit by, are the same
      // Vector3 object — asserted by identity, not by comparing two copies.
      skySunShared: this.sky.material.uniforms.uToSun.value === this._toSun,
      toSun: [this._toSun.x, this._toSun.y, this._toSun.z],
      calls: this.stats.calls,
      triangles: this.stats.triangles,
      peakCalls: this.stats.peakCalls,
      peakTriangles: this.stats.peakTriangles,
    };
  }

  /**
   * Dev-only: WHERE does this image sit?
   *
   * Every lighting number this client has ever shipped was a ratio against
   * the pass before it — "brighter than", "darker than", "×1.4 of". Six
   * consecutive passes of visual work improved ratios and the frames still
   * came back a stop and a half under `Rust Images/`, because a ratio cannot
   * see an offset. `shadowProbe` says the shadow map darkens pixels;
   * `surfaceProbe` says the field moves them; `propProbe` says a prop's field
   * is structured. Not one of them can say the whole image is too dark.
   *
   * So this one measures the ABSOLUTE tonal register and nothing else, in the
   * same statistic `ci/reference_bar.mjs` reads off the reference frames:
   * Rec.601 luma percentiles over the whole frame. Two numbers a light rig
   * owns and a material cannot fix — where the midtones sit (p50) and how far
   * the top reaches (p90).
   *
   * Per view it renders twice:
   *
   *   1. the frame as it ships;
   *   2. the same frame with the sky dome hidden and the clear colour set to
   *      a sentinel, which gives an EXACT dome mask — a pixel is sky iff the
   *      second render is the sentinel there. That is what makes the sky
   *      statistics below a measurement of the dome rather than of whatever
   *      happened to be near the top of the frame, and it costs one render
   *      instead of a heuristic.
   *
   * The sky matters separately because it is half of criterion 8 and it is
   * the one surface whose posterization is visible: a smooth ramp quantized
   * to 8 bits bands, and the count of DISTINCT luma levels in the dome says
   * whether the dither under the quantizer is doing its job. A banded ramp
   * scores tens; a dithered one scores hundreds.
   *
   * Allocates and renders 2N frames: never the RAF path. For the browser gate.
   */
  tonalProbe(views) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const shot = new Uint8Array(w * h * 4);
    const noSky = new Uint8Array(w * h * 4);
    const luma = (buf, p) => (buf[p] * 2 + buf[p + 1] * 5 + buf[p + 2]) >> 3;
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepClear = new THREE.Color();
    this.renderer.getClearColor(keepClear);
    // A colour nothing in this scene can produce: the sky is blue-dominant,
    // the world is warm, and no material anywhere is full-saturation magenta.
    // It is also never tone-mapped (a clear colour is not a fragment), so it
    // arrives at the framebuffer bit-exact and the mask is an equality test.
    const SENTINEL = [255, 0, 255];
    const samples = [];
    const total = new Int32Array(256);
    let totalN = 0;

    const pct = (hist, n) => {
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
      return {
        p05: at(0.05),
        p10: at(0.1),
        p50: at(0.5),
        p90: at(0.9),
        p95: at(0.95),
        mean: n > 0 ? sum / n : 0,
      };
    };

    for (const v of views) {
      const cp = Math.cos(v.pitch);
      this._dir.set(Math.sin(v.yaw) * cp, Math.sin(v.pitch), Math.cos(v.yaw) * cp);
      this._target.copy(cam.position).add(this._dir);
      cam.lookAt(this._target);

      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, shot);
      this.sky.visible = false;
      this.renderer.setClearColor(new THREE.Color(1, 0, 1));
      this.renderer.render(this.scene, cam);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, noSky);
      this.sky.visible = true;
      this.renderer.setClearColor(keepClear);

      const frame = new Int32Array(256);
      const dome = new Int32Array(256);
      // …and its complement, which is the only honest thing to compare the
      // dome against. Comparing the sky to the WHOLE frame's median is
      // degenerate on a vantage that is 73% sky: there the median IS sky, and
      // a perfectly bright dome scores 6 levels above itself.
      const world = new Int32Array(256);
      let worldN = 0;
      let domeN = 0;
      let peak = -1;
      let peakX = 0;
      let peakY = 0;
      // Banding, measured as what banding IS: adjacent pixels of a ramp that
      // carry the identical quantized value. Counting DISTINCT levels was
      // tried first and is the wrong instrument — the count is bounded above
      // by how many levels the gradient itself spans, so a view whose sky
      // band covers 16 levels scores 16 no matter how well it is dithered.
      // A run length is bounded by nothing.
      let pairs = 0;
      let broken = 0;
      let run = 0;
      let longestRun = 0;
      const isSky = (p) =>
        noSky[p] === SENTINEL[0] && noSky[p + 1] === SENTINEL[1] && noSky[p + 2] === SENTINEL[2];
      for (let y = 0; y < h; y++) {
        run = 0;
        for (let x = 0; x < w; x++) {
          const i = y * w + x;
          const p = i * 4;
          const l = luma(shot, p);
          frame[l]++;
          total[l]++;
          if (isSky(p)) {
            dome[l]++;
            domeN++;
            if (l > peak) {
              peak = l;
              peakX = x;
              peakY = y;
            }
            if (x + 1 < w && isSky(p + 4)) {
              pairs++;
              if (luma(shot, p + 4) !== l) {
                broken++;
                if (run > longestRun) longestRun = run;
                run = 0;
              } else {
                run++;
              }
            }
          } else {
            world[l]++;
            worldN++;
            if (run > longestRun) {
              longestRun = run;
              run = 0;
            }
          }
        }
        if (run > longestRun) longestRun = run;
      }
      totalN += w * h;
      let levels = 0;
      for (let l = 0; l < 256; l++) if (dome[l] > 0) levels++;
      const d = pct(dome, domeN);
      samples.push({
        label: v.label,
        yaw: v.yaw,
        pitch: v.pitch,
        ...pct(frame, w * h),
        skyPixels: domeN,
        skyFraction: domeN / (w * h),
        skyMean: d.mean,
        skyP05: d.p05,
        skyP95: d.p95,
        // Distinct luma levels present in the dome — reported, not walled,
        // for the reason in the loop above.
        skyLevels: levels,
        // The banding measure that IS walled: the share of horizontally
        // adjacent dome pixels whose quantized luma differs, and the longest
        // run of identical ones. A banded ramp breaks only at its band
        // boundaries (the judge measured an 11 px longest run); a dithered
        // one breaks at roughly half its pairs.
        skyBreak: pairs > 0 ? broken / pairs : 0,
        skyLongestRun: longestRun,
        // The frame with the sky taken out of it: what the dome has to be
        // brighter THAN.
        worldP50: pct(world, worldN).p50,
        worldPixels: worldN,
        // Brightest dome pixel and where it is — the sun, if there is one in
        // this view. In FRAMEBUFFER coordinates (y up from the bottom), which
        // is what `readPixels` hands back.
        skyPeak: peak,
        skyPeakXY: [peakX, peakY],
      });
    }

    cam.quaternion.copy(keepQ);
    this.renderer.render(this.scene, cam);
    return { width: w, height: h, samples, all: pct(total, totalN), pixels: totalN };
  }

  /**
   * Dev-only: is the sun in the sky the sun the world is lit by?
   *
   * `tonalProbe` finds the brightest dome pixel; this one says where it
   * SHOULD be. The camera is aimed straight down the key light's own
   * direction vector, so the disc must land at the principal point — and if
   * the dome ever draws a sun off a second copy of the bearing, or a pass
   * moves the key and forgets the sky, the peak walks off the centre and the
   * gate says so in pixels.
   *
   * Returns the frame's own peak, its offset from the centre, and the dome's
   * background level well away from the sun, so the assertion can be "the
   * disc is brighter than the sky it sits in" rather than "something is
   * bright somewhere".
   */
  sunProbe() {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const shot = new Uint8Array(w * h * 4);
    const noSky = new Uint8Array(w * h * 4);
    const cam = this.camera;
    const keepQ = cam.quaternion.clone();
    const keepClear = new THREE.Color();
    this.renderer.getClearColor(keepClear);
    this._target.copy(cam.position).add(this._toSun);
    cam.lookAt(this._target);
    this.renderer.render(this.scene, cam);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, shot);
    // The same sentinel dome mask `tonalProbe` uses, and for a reason the
    // first cut of this probe demonstrated: without it the "brightest pixel"
    // was 227 px off the aim point at 252 luma, because a specular highlight
    // on the world in the lower third of a 75°-FOV frame is brighter than the
    // sky. The question here is where the SKY's sun is; a world pixel is not
    // an answer to it, right or wrong.
    this.sky.visible = false;
    this.renderer.setClearColor(new THREE.Color(1, 0, 1));
    this.renderer.render(this.scene, cam);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, noSky);
    this.sky.visible = true;
    this.renderer.setClearColor(keepClear);

    let peak = -1;
    let peakX = 0;
    let peakY = 0;
    let sum = 0;
    let n = 0;
    let hot = 0;
    let domeN = 0;
    const cx = (w - 1) / 2;
    const cy = (h - 1) / 2;
    // "Well away from the sun": outside a quarter of the frame height from
    // the centre, which at FOV 75 is more than 18° of arc — four aureole
    // widths, so the background is background.
    const far = h * 0.25;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const p = (y * w + x) * 4;
        if (noSky[p] !== 255 || noSky[p + 1] !== 0 || noSky[p + 2] !== 255) continue;
        domeN++;
        const l = (shot[p] * 2 + shot[p + 1] * 5 + shot[p + 2]) >> 3;
        const dx = x - cx;
        const dy = y - cy;
        const r = Math.sqrt(dx * dx + dy * dy);
        if (r > far) {
          sum += l;
          n++;
        }
        if (l > peak) {
          peak = l;
          peakX = x;
          peakY = y;
        }
        // How big the disc reads, counted rather than assumed. A disc that
        // had grown into a hemisphere-wide wash would count most of the frame.
        if (l >= 250) hot++;
      }
    }
    cam.quaternion.copy(keepQ);
    this.renderer.render(this.scene, cam);
    return {
      width: w,
      height: h,
      peak,
      peakXY: [peakX, peakY],
      offsetPx: Math.hypot(peakX - cx, peakY - cy),
      background: n > 0 ? sum / n : 0,
      backgroundPixels: n,
      skyPixels: domeN,
      saturatedPixels: hot,
      saturatedFraction: hot / (w * h),
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
    // Leg 4 (lighting v1): the same frame with everything that is not another
    // player stopped from casting. The difference between this and the ship
    // leg is shadow the WORLD drew, by construction — which is the claim the
    // per-yaw floor was a proxy for, and the proxy tracked sun elevation while
    // the claim does not.
    const solo = new Uint8Array(w * h * 4);
    const worldCasters = this._worldCasters();
    const keepQ = this.camera.quaternion.clone();
    const pos = this.camera.position;
    const samples = [];
    let darkened = 0;
    let worldDarkened = 0;
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
      // …and the world stops casting. Same programs, same colour pass, one
      // depth pass with a shorter draw list.
      for (let k = 0; k < worldCasters.length; k++) worldCasters[k].castShadow = false;
      this._redrawShadows();
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, solo);
      for (let k = 0; k < worldCasters.length; k++) worldCasters[k].castShadow = true;
      this._redrawShadows();
      let n = 0;
      let sum = 0;
      let max = 0;
      let litSum = 0;
      let shadSum = 0;
      let world = 0;
      let worldSum = 0;
      for (let p = 0; p < lit.length; p += 4) {
        // Rec.601-ish integer luma; the absolute scale does not matter,
        // only the difference between two renders of the same pixel.
        const a = (lit[p] * 2 + lit[p + 1] * 5 + lit[p + 2]) >> 3;
        const b = (shad[p] * 2 + shad[p + 1] * 5 + shad[p + 2]) >> 3;
        const c = (solo[p] * 2 + solo[p + 1] * 5 + solo[p + 2]) >> 3;
        litSum += a;
        shadSum += b;
        const d = a - b;
        if (d > minDelta) {
          n++;
          sum += d;
          if (d > max) max = d;
        }
        // Darkened in the ship frame and NOT darkened when only players cast:
        // a pixel the world's own casters own.
        const dw = c - b;
        if (dw > minDelta) {
          world++;
          worldSum += dw;
        }
      }
      worldDarkened += world;
      samples.push({
        yaw: yaws[i],
        darkened: n,
        fraction: n / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
        // The world's own share, and what it is worth where it lands.
        worldDarkened: world,
        worldFraction: world / (w * h),
        worldMeanDelta: world > 0 ? worldSum / world : 0,
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
    return {
      width: w,
      height: h,
      pixels: w * h * yaws.length,
      darkened,
      worldDarkened,
      // How many casters the mutation actually took away. A leg that
      // suppressed nothing would measure zero world shadow and read as a
      // catastrophic failure; a leg that suppressed everything including the
      // avatars would read as a perfect one. The gate checks this count.
      worldCasters: worldCasters.length,
      samples,
    };
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
