// Shadow clipmap v0 (DECISIONS.md §open, "shadow clipmap v0").
//
// Lighting v0 shipped ONE directional shadow map bounded at an 80 m
// half-width in light space — the single case `threejs-shadow-systems` allows
// a single map, and honest only while nothing outside it matters. It does:
// a pine 100 m off the sun-perpendicular bearing casts nothing at all.
//
// This is the skill's cached clipmap, cut to what this world actually needs:
// concentric light-space square levels, each snapped to its OWN texel grid,
// each publishing a committed centre, near levels refreshed every frame and
// coarse ones cached under a per-frame budget, normal bias scaled by world
// texel width, and a containment cross-fade so a level boundary is a ramp
// rather than an edge.
//
// ## Why two levels and not six
//
// The reference builds levels out to 2000 m. Here the caster set stops long
// before that, by an earlier decision: the far mesh (the 8 m LOD of the whole
// island) does NOT cast — it is the same ground the 1 m near ring already
// casts from, and putting two disagreeing silhouettes of one hillside in one
// map buys self-shadow acne along the whole boundary (terrain.js). So the
// only casters that exist are the 5×5×64 m near ring, its scatter, placed
// pieces, deployables and players — all inside ±192 m of the player. A third
// level would render that identical set at a third of the resolution and
// darken not one new pixel. The level table is generated from
// (firstHalfWidth, scaleFactor, maxHalfWidth), so the day the horizon starts
// casting, the levels arrive from a constant.
//
// ## How it reaches the image, in WebGL
//
// The skill's node API is WebGPU/TSL; this client is WebGLRenderer, so the
// same structure is built out of what WebGL three actually gives you:
//
//   * N DirectionalLights, all `castShadow`, added finest-first. three sorts
//     shadow casters ahead of non-casters with a stable comparator, so
//     `directionalShadowMap[i]` is level i. Level 0's light carries the whole
//     key intensity; levels 1.. carry ZERO, so they light nothing and exist
//     only for their depth texture.
//   * A patch on every receiving material that replaces the stock per-light
//     `getShadow(...)` call with `gatesClipmapShadow()` — one factor computed
//     from ALL levels, applied to every directional light. The zero-intensity
//     levels multiply nothing; the key light gets the clipmap.
//   * `shadow.autoUpdate = false` on every level. three then skips both the
//     map render AND `shadow.updateMatrices()` for a level it does not
//     refresh — which is exactly the reference's "publish the centre from the
//     last completed map render": `shadow.matrix` (and therefore the shader's
//     containment box) cannot drift away from the map's contents, because the
//     same skip freezes both.
//
// Containment is computed in the level's own normalized shadow coordinates
// (`vDirectionalShadowCoord[i]` is already the light-space box mapped to
// [0,1]³), so the guard band and the cross-fade need no per-level uniforms
// and cannot disagree with the committed centre by construction.
//
// Two deliberate deviations from the reference, both stated rather than
// silent:
//
//   1. Depth range. The reference's `far = margin + 2·half` leaves only
//      `half` of coverage BELOW the level centre; at level 0 that is 80 m,
//      under the island's ~90 m relief (TERRAIN.md §6), so a valley floor
//      would fall past the far plane and go unshadowed. Lighting v0 carried
//      260 m below and this keeps it, as an explicit term.
//   2. No light-direction epsilon. The sun does not move in v0 (lighting v0
//      is a fixed azimuth/elevation), so the refresh-on-direction-change path
//      would be code no frame can reach. Day/night is M2's; the rule it must
//      obey — a direction change force-dirties every level — is written here
//      and in `DECISIONS.md` §open rather than half-built.

import * as THREE from "three";

// --- the knobs (DECISIONS.md §open, "shadow clipmap v0") --------------------
// FIRST_HALF_M is lighting v0's SHADOW_RADIUS_M, unchanged: level 0 IS the
// map that shipped, so nothing about the near frame moves in this slice.
const FIRST_HALF_M = 80;
const SCALE_FACTOR = 3;
const MAX_HALF_M = 240; // holds the near ring's ±192 m of casters
// Per level, finest first; short entries repeat the last. Level 0 keeps
// lighting v0's 2048 px exactly. The coarse levels are HALF that, and it is
// not a cosmetic saving: a level's map is re-rendered whenever the player
// crosses one of its texels, so halving the resolution both quarters the
// fragments rasterized and halves how often that happens. At full 2048 the
// coarse level cost enough per frame to starve a third browser tab of CPU on
// the reference-class box the browser gate runs on. 0.469 m texels still put
// six across a pine's shadow at 150 m, which is what that level is for.
const MAP_PX = [2048, 1024];
// How far past the level box, along the sun ray, the light sits — the depth
// the map has for casters ABOVE the receivers.
const LIGHT_MARGIN_M = 100;
// …and how much depth it keeps BELOW the level centre (deviation 1 above).
const DEPTH_BELOW_M = 260;
const NEAR_M = 1;
const FAR_CAP_M = 3000;
// Sampled half-width is `halfWidth * (1 - GUARD_BAND)`: the outermost texels
// are rendered but never sampled, so a PCF tap near the edge cannot reach
// outside the map. BLEND_RATIO is the fraction of the sampled half-width the
// cross-fade to the next level occupies. Both are the reference's defaults.
const GUARD_BAND = 0.15;
const BLEND_RATIO = 0.15;
// Levels below this index refresh every frame and never touch the budget —
// they hold the moving casters (players, the piece being placed).
const DYNAMIC_LEVELS = 1;
// Cached levels per frame, and the age at which a cached level refreshes even
// though nothing it can see has moved (so a moving caster out there thaws).
const UPDATE_BUDGET = 1;
const MAX_CACHE_AGE = 64;
// Normal bias in TEXELS of the level's own grid — lighting v0's value, now
// scaled per level, which is the reference's whole point: one metre value
// across a 0.078 m texel and a 0.234 m texel is not a coherent bias.
const NORMAL_BIAS_TEXELS = 1.2;

/** Half-widths, finest first; the last is exactly MAX_HALF_M. */
function levelHalfWidths() {
  const out = [FIRST_HALF_M];
  while (out[out.length - 1] < MAX_HALF_M) {
    out.push(Math.min(out[out.length - 1] * SCALE_FACTOR, MAX_HALF_M));
  }
  return out;
}

export const LEVEL_HALF_WIDTHS = levelHalfWidths();
export const LEVEL_COUNT = LEVEL_HALF_WIDTHS.length;

// The shader's debug/probe handle, shared by every patched material: how many
// levels may contribute. At LEVEL_COUNT the clipmap is whole; at 1 only the
// near map answers (lighting v0's reach); at 0 nothing is shadowed. Weight a
// level cannot claim resolves to unshadowed, so lowering this only ever
// removes shadow — which is what makes the browser gate's two probes a
// difference measurement and not a re-render.
const activeLevels = { value: LEVEL_COUNT };

/** How many levels currently contribute (the gate reads this back). */
export function clipmapActiveLevels() {
  return activeLevels.value;
}

/** Set the contributing level count. Dev/probe only — never the RAF path. */
export function setClipmapActiveLevels(n) {
  activeLevels.value = n;
}

// --- the shader side -------------------------------------------------------
// Containment runs in normalized shadow coordinates, so `d` is 0 at the
// committed centre and 1 at the rendered box edge. The sampled edge is
// therefore `1 - GUARD_BAND` and the fade occupies the last BLEND_RATIO of
// it. Every level's depth-comparison sample is taken UNCONDITIONALLY and
// weighted afterwards — the reference's one hard GPU contract, because a
// comparison sampler behind a per-pixel branch has undefined derivatives.
const FADE_OUTER = 1 - GUARD_BAND;
const FADE_INNER = FADE_OUTER * (1 - BLEND_RATIO);

// Per-level filter cost, in depth taps. The near level keeps three's own
// `getShadow`, which under PCFSoftShadowMap is nine bilinear comparisons —
// thirty-six fetches — and is what every close silhouette in the frame has
// looked like since lighting v0. The coarse levels take ONE comparison.
//
// That is a deliberate per-level choice, not a corner cut. Filtering nine
// taps twice per fragment doubles the most expensive thing in the shader for
// every pixel on screen, and it buys softness on a level whose texels are
// 0.469 m and whose nearest content is 68 m away — where one texel is a
// couple of screen pixels and PCF has almost nothing left to smooth. It is
// measurable, too: at two soft levels this client slowed enough on the
// browser gate's software rasterizer to make the pre-existing timing
// assertions (join, held-walk bearing) go marginal. Cheaper filtering on
// coarse cascades is the ordinary answer and the reference asks for exactly
// this kind of per-level inspection (§9, §13).
export const LEVEL_FILTER_TAPS = [36, 1];

function levelTaps(i) {
  return LEVEL_FILTER_TAPS[Math.min(i, LEVEL_FILTER_TAPS.length - 1)];
}

function clipmapGlsl() {
  let body = "";
  for (let i = 0; i < LEVEL_COUNT; i++) {
    const sample =
      i === 0
        ? /* glsl */ `getShadow(
      directionalShadowMap[ ${i} ],
      directionalLightShadows[ ${i} ].shadowMapSize,
      directionalLightShadows[ ${i} ].shadowIntensity,
      directionalLightShadows[ ${i} ].shadowBias,
      directionalLightShadows[ ${i} ].shadowRadius,
      vDirectionalShadowCoord[ ${i} ] )`
        : /* glsl */ `gatesCoarseShadow(
      directionalShadowMap[ ${i} ],
      directionalLightShadows[ ${i} ].shadowIntensity,
      directionalLightShadows[ ${i} ].shadowBias,
      gcC )`;
    body += /* glsl */ `
  {
    vec3 gcC = vDirectionalShadowCoord[ ${i} ].xyz / vDirectionalShadowCoord[ ${i} ].w;
    float gcD = max(abs(gcC.x - 0.5), abs(gcC.y - 0.5)) * 2.0;
    float gcF = 1.0 - smoothstep(${FADE_INNER.toFixed(6)}, ${FADE_OUTER.toFixed(6)}, gcD);
    gcF *= step(${(i + 0.5).toFixed(1)}, uClipLevels);
    float gcS = ${sample};
    float gcW = gcF * gcRemaining;
    gcRemaining -= gcW;
    gcAcc += gcW * gcS;
  }`;
  }
  return /* glsl */ `
uniform float uClipLevels;
// One comparison, taken UNCONDITIONALLY and selected afterwards — stricter
// than three's own getShadow, which puts its taps behind the frustum test.
// Shadow maps are ClampToEdge and unmipped, so a sample outside [0,1] is
// defined; what must not happen is the sample living in divergent control
// flow (the reference's one hard GPU contract).
float gatesCoarseShadow( sampler2D depths, float intensity, float bias, vec3 coord ) {
  float raw = texture2DCompare( depths, coord.xy, coord.z + bias );
  bool inFrustum = coord.x >= 0.0 && coord.x <= 1.0
                && coord.y >= 0.0 && coord.y <= 1.0 && coord.z <= 1.0;
  return mix( 1.0, inFrustum ? raw : 1.0, intensity );
}
float gatesClipmapShadow() {
  float gcRemaining = 1.0;
  float gcAcc = 0.0;
${body}
  // Leftover weight is unshadowed: past the coarsest level the shadow fades
  // out instead of ending.
  return gcAcc + gcRemaining;
}
`;
}

// The exact call three's `lights_fragment_begin` makes per directional light.
// Read off the INSTALLED chunk rather than hardcoded, then required to appear
// exactly once — a three upgrade that renames a shadow uniform must fail the
// build loudly, not quietly hand every light its own map back.
const STOCK_DIRECTIONAL_GET_SHADOW =
  "getShadow( directionalShadowMap[ i ], directionalLightShadow.shadowMapSize, " +
  "directionalLightShadow.shadowIntensity, directionalLightShadow.shadowBias, " +
  "directionalLightShadow.shadowRadius, vDirectionalShadowCoord[ i ] )";

function patchedLightsChunk() {
  const stock = THREE.ShaderChunk.lights_fragment_begin;
  const hits = stock.split(STOCK_DIRECTIONAL_GET_SHADOW).length - 1;
  if (hits !== 1) {
    throw new Error(
      `shadow clipmap: three ${THREE.REVISION}'s lights_fragment_begin has ` +
        `${hits} directional getShadow calls, expected 1 — the clipmap patch ` +
        `no longer applies and every light would sample its own level`,
    );
  }
  return stock.split(STOCK_DIRECTIONAL_GET_SHADOW).join("gatesClipmapShadow()");
}

let cachedChunk = null;
let cachedGlsl = null;

/**
 * Make one MeshStandardMaterial sample the clipmap instead of one map per
 * light. Composes with a material's own onBeforeCompile: call this from the
 * material factory and it wraps whatever is already there.
 *
 * Every patched material shares the `uClipLevels` uniform OBJECT, so the
 * probe's toggle moves the whole scene in one write and no material can
 * disagree with another about how many levels are live.
 */
export function installClipmapShadows(material) {
  if (cachedChunk === null) {
    cachedChunk = patchedLightsChunk();
    cachedGlsl = clipmapGlsl();
  }
  const prior = material.onBeforeCompile;
  material.onBeforeCompile = (shader, renderer) => {
    if (prior) prior(shader, renderer);
    shader.uniforms.uClipLevels = activeLevels;
    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <shadowmap_pars_fragment>",
        `#include <shadowmap_pars_fragment>\n${cachedGlsl}`,
      )
      .replace("#include <lights_fragment_begin>", cachedChunk);
  };
  return material;
}

/**
 * The concentric levels themselves: the lights, their committed centres, and
 * the update policy that decides which of them redraws this frame.
 */
export class ShadowClipmap {
  /**
   * @param {THREE.Scene} scene
   * @param {number} color  key-light colour (level 0 carries it)
   * @param {number} intensity  key-light intensity
   * @param {THREE.Vector3} toSun  unit vector pointing AT the sun
   */
  constructor(scene, color, intensity, toSun) {
    // Matrix4.lookAt puts +Z along (eye − target): eye at the sun, target at
    // the origin, so +Z_light points at the sun and the matrix IS the shadow
    // cameras' shared basis. Pure rotation, so its transpose is its inverse.
    this._lightToWorld = new THREE.Matrix4().lookAt(
      toSun,
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(0, 1, 0),
    );
    this._worldToLight = this._lightToWorld.clone().transpose();
    this._probe = new THREE.Vector3();
    this._place = new THREE.Vector3();
    this._first = true;

    this.levels = LEVEL_HALF_WIDTHS.map((halfWidth, i) => {
      const light = new THREE.DirectionalLight(color, i === 0 ? intensity : 0);
      light.castShadow = true;
      const sh = light.shadow;
      const px = MAP_PX[Math.min(i, MAP_PX.length - 1)];
      sh.mapSize.set(px, px);
      sh.camera.left = -halfWidth;
      sh.camera.right = halfWidth;
      sh.camera.top = halfWidth;
      sh.camera.bottom = -halfWidth;
      sh.camera.near = NEAR_M;
      sh.camera.far = Math.max(
        NEAR_M + 1,
        Math.min(FAR_CAP_M, halfWidth + LIGHT_MARGIN_M + DEPTH_BELOW_M),
      );
      sh.bias = 0;
      const texelM = (halfWidth * 2) / px;
      sh.normalBias = NORMAL_BIAS_TEXELS * texelM;
      // The clipmap owns every refresh: three must never render a level from
      // a transform the shader is not sampling (it skips updateMatrices on
      // the same test, which is what freezes the committed centre).
      sh.autoUpdate = false;
      sh.needsUpdate = false;
      sh.camera.updateProjectionMatrix();
      scene.add(light);
      scene.add(light.target); // or its matrixWorld never updates
      return {
        light,
        halfWidth,
        texelM,
        // Committed light-space centre. Parked far away and marked invalid
        // until the level's first render, so containment can never select a
        // level whose map holds nothing.
        cx: 1e9,
        cy: 1e9,
        cz: 1e9,
        valid: false,
        forceDirty: false,
        // Staggered, so the coarse levels do not all expire on one frame.
        age: Math.floor((-i * MAX_CACHE_AGE) / LEVEL_COUNT),
        renders: 0,
        dynamic: i < DYNAMIC_LEVELS,
      };
    });
    this.key = this.levels[0].light;
    this.budgetLast = 0;
  }

  /**
   * Retarget every level on the player, snapped to its own texel grid, and
   * decide which maps redraw this frame.
   *
   * A directional shadow map that simply tracks the camera crawls: the
   * projected texel grid slides under the geometry and every silhouette edge
   * shimmers. Quantizing the centre in LIGHT space by the world width of one
   * texel nails the grid to the world, so the box moves in whole-texel steps.
   * Z is quantized far more coarsely on purpose — it moves depth coverage,
   * not the projected grid.
   *
   * Scalar math on two preallocated vectors; no allocation, no closure (L8).
   */
  update(x, y, z) {
    const p = this._probe.set(x, y, z).applyMatrix4(this._worldToLight);
    // First frame: every level gets to draw, or nothing is shadowed at all
    // until the budget trickles the coarse maps in.
    let budget = this._first ? this.levels.length : UPDATE_BUDGET;
    this._first = false;
    this.budgetLast = budget;
    for (let i = 0; i < this.levels.length; i++) {
      const L = this.levels[i];
      L.age++;
      const t = L.texelM;
      const dx = Math.round(p.x / t) * t;
      const dy = Math.round(p.y / t) * t;
      const zq = L.halfWidth * 0.5;
      const dz = Math.round(p.z / zq) * zq;
      const moved = dx !== L.cx || dy !== L.cy || dz !== L.cz;
      const dirty =
        L.dynamic || !L.valid || L.forceDirty || moved || L.age >= MAX_CACHE_AGE;
      if (!dirty) {
        L.light.shadow.needsUpdate = false;
        continue;
      }
      // Dynamic levels never touch the budget; an explicit invalidation
      // bypasses it (the reference keeps that exception on purpose — a
      // streamed-in hillside that waits its turn is a visible hole).
      if (!L.dynamic && !L.forceDirty && L.valid) {
        if (budget <= 0) {
          L.light.shadow.needsUpdate = false;
          continue;
        }
        budget--;
      }
      // Commit the camera and the map together: the transform below is the
      // one three renders from AND the one the shader samples, because
      // `shadow.matrix` is only rebuilt on the frames the map is.
      L.cx = dx;
      L.cy = dy;
      L.cz = dz;
      L.forceDirty = false;
      L.valid = true;
      L.age = 0;
      L.renders++;
      this._place
        .set(dx, dy, dz + L.halfWidth + LIGHT_MARGIN_M)
        .applyMatrix4(this._lightToWorld);
      L.light.position.copy(this._place);
      this._place.set(dx, dy, dz).applyMatrix4(this._lightToWorld);
      L.light.target.position.copy(this._place);
      L.light.shadow.needsUpdate = true;
    }
  }

  /**
   * Force a refresh of every level whose box can see a world-space sphere.
   * This is what keeps a cached level honest when the WORLD changes rather
   * than the camera: a near chunk streaming in or out, a base going up.
   *
   * Conservative square test in light-space XY, per the reference: it ignores
   * Z and the exact projected distance, so it may refresh a level it did not
   * have to. Cheap and never wrong in the direction that shows.
   */
  invalidate(x, y, z, radius) {
    const p = this._probe.set(x, y, z).applyMatrix4(this._worldToLight);
    for (let i = 0; i < this.levels.length; i++) {
      const L = this.levels[i];
      if (L.dynamic) continue; // redraws next frame anyway
      const reach = L.halfWidth + radius;
      if (Math.abs(p.x - L.cx) < reach && Math.abs(p.y - L.cy) < reach) {
        L.forceDirty = true;
      }
    }
  }

  /** World position → light space, into `out`. For the far-shadow probe. */
  toLight(out) {
    return out.applyMatrix4(this._worldToLight);
  }

  /** The reference's diagnostics, cut to what a gate can assert. */
  facts() {
    return {
      levelCount: LEVEL_COUNT,
      activeLevels: activeLevels.value,
      guardBand: GUARD_BAND,
      blendRatio: BLEND_RATIO,
      dynamicLevels: DYNAMIC_LEVELS,
      updateBudget: UPDATE_BUDGET,
      maxCacheAge: MAX_CACHE_AGE,
      normalBiasTexels: NORMAL_BIAS_TEXELS,
      levels: this.levels.map((L, i) => ({
        halfWidthM: L.halfWidth,
        sampledHalfWidthM: L.halfWidth * FADE_OUTER,
        mapPx: L.light.shadow.mapSize.x,
        texelM: L.texelM,
        normalBias: L.light.shadow.normalBias,
        farM: L.light.shadow.camera.far,
        center: [L.cx, L.cy, L.cz],
        valid: L.valid,
        forceDirty: L.forceDirty,
        age: L.age,
        renders: L.renders,
        dynamic: L.dynamic,
        filterTaps: levelTaps(i),
        casts: L.light.castShadow,
        intensity: L.light.intensity,
      })),
    };
  }
}
