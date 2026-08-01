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
// ## Why three levels
//
// The clipmap shipped with two, and said why: nothing past ±192 m cast, so a
// third level would have redrawn an identical caster set at a third of the
// resolution and darkened not one new pixel. That bound was the far mesh being
// kept out of the map — and the level table was generated from
// (firstHalfWidth, scaleFactor, maxHalfWidth) precisely so that the day the
// horizon started casting, the levels would arrive from a constant.
//
// It does now (see `makeFarCasterDepthMaterial` below and terrain.js), so the
// constant moved: 240 m → 720 m, and the generator hands back 80 / 240 / 720.
// It stops at 720 because that is where the frame stops: fog is opaque by
// 1000 m (scene.js FOG_FAR), and 720 m of light-space half-width already
// reaches ~2000 m of ground along the sun's own bearing. A fourth level would
// be depth nobody can see through the fog.
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
//     `getShadow(...)` with one `gatesClipmapShadow()` — a single factor read
//     from ALL levels, hoisted out of three's unrolled directional loop and
//     multiplied in. The same patch selects that loop down to its first
//     iteration, because levels 1.. carry no energy and evaluating a
//     reflectance model for them costs a full GGX lobe to add zero
//     (`patchedLightsChunk`).
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
// Three deliberate deviations from the reference, all stated rather than
// silent:
//
//   1. Depth range, derived rather than flat. The reference's
//      `far = margin + 2·half` leaves `half` of coverage below the centre,
//      which is neither enough at level 0 (80 m, under the island's ~90 m
//      relief — TERRAIN.md §6) nor anywhere near enough at level 2, where the
//      box's own ground spans `half·cot(21°)` = 1913 m of light-space depth.
//      Both terms are written out at `_depthPerHalf`, where the geometry is.
//   2. No light-direction epsilon. The sun does not move in v0 (lighting v0
//      is a fixed azimuth/elevation), so the refresh-on-direction-change path
//      would be code no frame can reach. Day/night is M2's; the rule it must
//      obey — a direction change force-dirties every level — is written here
//      and in `DECISIONS.md` §open rather than half-built.
//   3. Per-level filter cost — see LEVEL_FILTER_TAPS, where it is argued.
//
// And one thing not built: the reference's §11 disposal path. Nothing tears a
// GameScene down in this client — there is one, for the life of the page — so
// a dispose() here would be untested code guarding a case that cannot happen.
// The day a second scene can be constructed, the level lights, their targets
// and their shadow maps are persistent GPU resources; treat a missing dispose
// then as a real leak.

import * as THREE from "three";

// --- the knobs (DECISIONS.md §open, "shadow clipmap v0") --------------------
// FIRST_HALF_M is lighting v0's SHADOW_RADIUS_M, unchanged: level 0 still
// covers exactly the box that shipped. What it holds changed — the ground
// casts now, so the map it fills is no longer nearly empty (see MAP_PX).
const FIRST_HALF_M = 80;
const SCALE_FACTOR = 3;
// The horizon casts now, so this is no longer sized to the near ring — it is
// sized to the frame. Fog closes at 1000 m and 720 m of light-space half-width
// already covers ~2000 m of ground along the sun's bearing (720/sin 21°); the
// island itself is 2048 m across. Ground past this box fades out through the
// cross-fade rather than cutting, which is what the guard band is for.
const MAX_HALF_M = 720;
// Per level, finest first; short entries repeat the last. One size now, where
// lighting v0 gave level 0 twice the rest.
//
// 2048 was sized against a caster set that did not include the ground: with
// the terrain culled out of the depth pass, level 0's map was nearly empty and
// its resolution was free. Now that the ground casts, level 0 is a map that
// gets FILLED, every frame, by the only level that redraws every frame — and
// 2048² is 4.2 M depth fragments a frame where 1024² is 1.05 M. That is not a
// main-thread number the loop rules would tell you to ignore: on a software
// rasterizer the fill lands on other threads, and at 2048 it starved a third
// browser tab of enough CPU to keep it from completing a WebTransport
// handshake in 60 s (gate box, shared, software GL — the ratio is the claim,
// not the milliseconds).
//
// And the resolution was never buying anything at that size. A level's map
// cannot resolve detail its CASTERS do not have, and the finest caster in this
// world is a 1 m terrain grid and a 0.32 m pine trunk. At 1024 px level 0's
// texel is 0.156 m: six across the trunk and six per metre of ground, still
// far finer than the geometry it is sampling. The coarse levels are unchanged
// at 1024 (0.469 m and 1.406 m).
const MAP_PX = [1024];
// Slack past the ground the level's own box reaches, along the sun ray, at
// both ends: headroom for anything standing on that ground (a pine, a tower)
// and for the height the island's relief adds.
const LIGHT_MARGIN_M = 100;
// The island's vertical relief, which is the other thing the depth has to
// carry (TERRAIN.md §6).
const RELIEF_M = 90;
const NEAR_M = 1;
const FAR_CAP_M = 8000;
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
// across a 0.078 m texel and a 0.469 m texel is not a coherent bias.
const NORMAL_BIAS_TEXELS = 1.2;
// How far the far mesh's DEPTH is sunk below its own surface when it casts
// (argued at makeFarCasterDepthMaterial). Metres.
const FAR_CASTER_SINK_M = 3;

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

// --- what the near level's filter actually costs, read off three ------------
//
// The near level keeps three's own `getShadow`; the coarse levels take ONE
// comparison. That is a deliberate per-level choice, not a corner cut.
// Filtering the near kernel twice per fragment doubles the most expensive
// thing in the shader for every pixel on screen, and it buys softness on a
// level whose texels are 0.469 m and whose nearest content is 68 m away —
// where one texel is a couple of screen pixels and PCF has almost nothing
// left to smooth. It is measurable, too: at two soft levels this client
// slowed enough on the browser gate's software rasterizer to make the
// pre-existing timing assertions (join, held-walk bearing) go marginal.
// Cheaper filtering on coarse cascades is the ordinary answer and the
// reference asks for exactly this kind of per-level inspection (§9, §13).
//
// What changed here is only the NUMBER, and how it is obtained. This was a
// hand-typed `[36, 1]`, and the 36 was quoted from an older three: r1xx's
// PCF_SOFT was nine `texture2DShadowLerp` calls at four fetches each. The
// installed r178 compiles a different kernel — sixteen `texture2DCompare`
// calls over a 4×4 footprint, weighted 1/9 — so the constant overstated the
// near filter by 2.25×, in a line `ci/browser_smoke.mjs` printed as fact and
// `NOW.md` then pointed at as the obvious place to find headroom.
//
// A number that describes SOMEONE ELSE'S source rots the moment they change
// it, so it is read out of that source. This counts the fetches in the branch
// three will compile for the shadow type this client configures, and throws
// if that branch is gone — the same rule the `getShadow(…)` call site above
// is held to.
export const NEAR_FILTER_TYPE = THREE.PCFSoftShadowMap;
const NEAR_FILTER_BRANCH = "SHADOWMAP_TYPE_PCF_SOFT";

function nearFilterFetches() {
  const src = THREE.ShaderChunk.shadowmap_pars_fragment;
  const head = src.indexOf(NEAR_FILTER_BRANCH);
  if (head < 0) {
    throw new Error(
      `shadow clipmap: three ${THREE.REVISION}'s shadowmap chunk has no ` +
        `${NEAR_FILTER_BRANCH} branch — the near level's filter cost cannot be read`,
    );
  }
  let tail = src.length;
  for (const marker of ["#elif", "#else", "#endif"]) {
    const at = src.indexOf(marker, head + NEAR_FILTER_BRANCH.length);
    if (at >= 0 && at < tail) tail = at;
  }
  const n = src.slice(head, tail).split("texture2DCompare(").length - 1;
  if (n < 2) {
    throw new Error(
      `shadow clipmap: three ${THREE.REVISION}'s ${NEAR_FILTER_BRANCH} branch takes ` +
        `${n} depth fetches — a soft filter that is one tap means the kernel moved ` +
        `somewhere this cannot count it`,
    );
  }
  return n;
}

// Per-level filter cost, in depth fetches per fragment. Finest first; short
// entries repeat the last.
export const LEVEL_FILTER_TAPS = [nearFilterFetches(), 1];

// --- the shader variants the cost probe compiles ---------------------------
// A uniform cannot remove an instruction: `uClipLevels` weights every level's
// sample to zero and the sample is still taken, deliberately (a comparison
// sampler behind a per-pixel branch has undefined derivatives). So the only
// honest way to ask what a term COSTS is to compile the program without it
// and time both — which is what these are for.
//
//   ship   — exactly what the client renders. The default everywhere.
//   near1  — level 0 filtered like a coarse level. ship − near1 is the PCF.
//   off    — no shadow term at all, zero depth fetches.
//
// They exist for `GameScene.costProbe` and are never on a frame the player
// sees; `ship` is the only variant any material factory takes by default.
export const SHADOW_VARIANTS = ["ship", "near1", "off"];

/** Depth fetches per fragment, per variant. Counted, not timed. */
export function clipmapFetches(variant = "ship") {
  if (variant === "off") return 0;
  const near = variant === "near1" ? 1 : LEVEL_FILTER_TAPS[0];
  return near + (LEVEL_COUNT - 1) * levelTaps(1);
}

function levelTaps(i) {
  return LEVEL_FILTER_TAPS[Math.min(i, LEVEL_FILTER_TAPS.length - 1)];
}

function clipmapGlsl(variant) {
  if (variant === "off") {
    return /* glsl */ `
uniform float uClipLevels;
float gatesClipmapShadow() { return 1.0; }
`;
  }
  let body = "";
  for (let i = 0; i < LEVEL_COUNT; i++) {
    const sample =
      i === 0 && variant !== "near1"
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

// The directional section of that chunk, and the two edges of its unrolled
// loop. three replaces `UNROLLED_LOOP_INDEX` with the literal index when it
// unrolls, so a preprocessor test on it selects one iteration at compile time.
const DIR_SECTION_HEAD = "#if ( NUM_DIR_LIGHTS > 0 ) && defined( RE_Direct )";
const DIR_LOOP_HEAD = "for ( int i = 0; i < NUM_DIR_LIGHTS; i ++ ) {";
const DIR_LOOP_TAIL = "\n\t}\n\t#pragma unroll_loop_end";

/**
 * Replace `find` inside `src` exactly once, or throw naming `what`.
 *
 * Every shader patch in this file goes through here. A three upgrade that
 * renames a chunk must fail at boot, loudly, rather than quietly shipping a
 * scene lit once per level or a far mesh that casts through the near ring.
 */
function once(src, find, replacement, what) {
  const hits = src.split(find).length - 1;
  if (hits !== 1) {
    throw new Error(
      `shadow patch (${what}): three ${THREE.REVISION}'s shader source has ` +
        `${hits} of it, expected 1 — the patch no longer applies`,
    );
  }
  return src.split(find).join(replacement);
}

/**
 * three's per-light shadow term, replaced by the clipmap — and the rest of the
 * directional loop, collapsed onto the one light that carries the key.
 *
 * Two separate wins, both of which the level count multiplies, which is why
 * they are here rather than in a later pass:
 *
 *   1. `gatesClipmapShadow()` is evaluated once per FRAGMENT, not once per
 *      light. The stock call site is inside the directional loop and the loop
 *      is unrolled, so substituting the clipmap in place made every fragment
 *      read every level once per LEVEL — N² depth samples where N are needed.
 *      At two levels that was 74 fetches a fragment where 37 would do; the
 *      third would have made it 114.
 *   2. Levels 1.. are depth textures, not lights: they carry zero intensity
 *      by construction, so `directLight.color` is zero and `RE_Direct` adds
 *      exactly nothing — after running the whole GGX lobe, Fresnel and
 *      geometry term for every one of them, on every fragment. Selecting
 *      iteration 0 at compile time makes the reflectance model cost one light
 *      no matter how many levels the table generates.
 *
 * That second one changes what the gate's "only level 0 may carry intensity"
 * assertion is worth: the shader no longer DEPENDS on it. It stays, because a
 * level that was given intensity would now be silently ignored, and a light
 * configured to do something it cannot do is still a bug worth failing on.
 */
function patchedLightsChunk() {
  const stock = THREE.ShaderChunk.lights_fragment_begin;
  const head = stock.indexOf(DIR_SECTION_HEAD);
  const tail = stock.indexOf(DIR_LOOP_TAIL, head);
  if (head < 0 || tail < 0) {
    throw new Error(
      `shadow clipmap: three ${THREE.REVISION}'s lights_fragment_begin has no ` +
        `unrolled directional-light loop — the clipmap patch no longer applies`,
    );
  }
  const end = tail + DIR_LOOP_TAIL.length;
  let section = stock.slice(head, end);
  section = once(section, STOCK_DIRECTIONAL_GET_SHADOW, "gcClipFactor", "getShadow(…)");
  section = once(
    section,
    DIR_LOOP_HEAD,
    `${DIR_LOOP_HEAD}\n\t\t#if UNROLLED_LOOP_INDEX == 0`,
    DIR_LOOP_HEAD,
  );
  section = once(
    section,
    DIR_LOOP_TAIL,
    `\n\t\t#endif${DIR_LOOP_TAIL}`,
    "unroll_loop_end",
  );
  return (
    /* glsl */ `float gcClipFactor = gatesClipmapShadow();\n` +
    stock.slice(0, head) +
    section +
    stock.slice(end)
  );
}

// three resolves `#include <chunk>` on its way to the driver; this is the same
// expansion, run so a program's SIZE can be counted rather than guessed at
// from a template. Deliberately three's own rule — unknown chunk throws — so
// a renamed chunk fails here as loudly as it would there. Not a hot path: it
// runs once per compiled program, inside onBeforeCompile.
const INCLUDE_RE = /^[ \t]*#include +<([\w\d./]+)>/gm;

/** `src` with three's `#include`s expanded, in characters. */
export function resolvedGlslChars(src) {
  return resolveIncludes(src).length;
}

function resolveIncludes(src) {
  let out = src;
  for (let depth = 0; depth < 8 && out.includes("#include <"); depth++) {
    out = out.replace(INCLUDE_RE, (_line, name) => {
      const chunk = THREE.ShaderChunk[name];
      if (chunk === undefined) {
        throw new Error(
          `shader size: three ${THREE.REVISION} has no ShaderChunk '${name}'`,
        );
      }
      return chunk;
    });
  }
  return out;
}

let cachedChunk = null;
const cachedGlsl = new Map();

/**
 * Make one MeshStandardMaterial sample the clipmap instead of one map per
 * light. Composes with a material's own onBeforeCompile: call this from the
 * material factory and it wraps whatever is already there.
 *
 * Every patched material shares the `uClipLevels` uniform OBJECT, so the
 * probe's toggle moves the whole scene in one write and no material can
 * disagree with another about how many levels are live.
 *
 * `variant` is one of SHADOW_VARIANTS and is "ship" for everything the client
 * renders; the others exist so the cost probe can time a program compiled
 * WITHOUT a term instead of pretending a uniform removed it. It is also the
 * last patch any of these materials takes, so this is where the compiled
 * source is measured: `userData.programStats` is the size of exactly what
 * three hands the driver, which is the other half of `NOW.md` item 1's
 * suspect list (per-fragment cost, and program size).
 */
export function installClipmapShadows(material, variant = "ship") {
  if (!SHADOW_VARIANTS.includes(variant)) {
    throw new Error(`shadow clipmap: unknown variant ${variant}`);
  }
  if (cachedChunk === null) cachedChunk = patchedLightsChunk();
  if (!cachedGlsl.has(variant)) cachedGlsl.set(variant, clipmapGlsl(variant));
  const glsl = cachedGlsl.get(variant);
  const prior = material.onBeforeCompile;
  material.onBeforeCompile = (shader, renderer) => {
    if (prior) prior(shader, renderer);
    shader.uniforms.uClipLevels = activeLevels;
    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <shadowmap_pars_fragment>",
        `#include <shadowmap_pars_fragment>\n${glsl}`,
      )
      .replace("#include <lights_fragment_begin>", cachedChunk);
    material.userData.programStats = {
      shadowVariant: variant,
      depthFetches: clipmapFetches(variant),
      // Patched template size, and the size after three's `#include`s are
      // expanded — the second is the one that means something. A patched
      // template is a few kilobytes of `#include` lines; what the driver is
      // handed is that with every chunk pasted in, which is where a standard
      // material's real bulk lives and what `NOW.md` item 1 names as the
      // second suspect behind grain's failed merge.
      fragmentChars: shader.fragmentShader.length,
      vertexChars: shader.vertexShader.length,
      resolvedFragmentChars: resolveIncludes(shader.fragmentShader).length,
    };
  };
  return material;
}

// --- the far caster, and the seam it shares with the near ring --------------
// The far mesh is the 8 m LOD of the WHOLE island, and until this slice it did
// not cast. The reason given was sound about the part it was about: the near
// ring is the same ground at 1 m, and two disagreeing LODs of one hillside in
// one shadow map is self-shadow acne along the entire boundary. But that
// argument covers the OVERLAP and nothing else — outside the near ring there
// is no second LOD to disagree with, and outside the near ring is where the
// horizon lives. So a mountain at 400 m was lit flat, which is the whole of
// `NOW.md` item 1.
//
// The far mesh casts here, through a depth material that removes exactly the
// overlap and nothing more: a fragment whose world XZ lands inside the near
// ring's CURRENT footprint is discarded, so every XZ column of the world has
// exactly one caster — the 1 m mesh inside the ring, the 8 m mesh outside it.
// terrain.js owns the footprint (it owns the ring) and writes it here; a zero
// half-extent means "no hole", which is what boot looks like before a single
// near chunk has streamed and the far mesh is the only ground there is.
//
// ## The skirt
//
// A hole leaves a seam, and the seam is the one place the two LODs still meet.
// The 8 m surface stands above its own 1 m self by up to a couple of metres on
// rough ground, and at a 21° sun one metre of disagreement is 2.6 m of shadow
// — so an 8 m hillside just outside the hole would paint a dark band along the
// hole's edge, and that band would slide with the player. `TERRAIN.md` §4 has
// always wanted a skirt at this seam; this is it, in the only place a shadow
// can see one: the caster is sunk FAR_CASTER_SINK_M below its own surface.
//
// A sunk caster can cast LATE — its shadow starts sink/tan(21°) ≈ 8 m further
// along the sun ray — but it cannot invent a shadow that is not there. At the
// ranges this mesh is the caster for (≥ 128 m, and mostly 300 m+) 8 m is under
// a degree, and it is the error that hides rather than the one that crawls.
// The sink is constant rather than ramped at the seam on purpose: a ramp is
// another edge, and this one has nothing to buy — the far caster is never the
// close caster.
//
// ## Which level gets which caster
//
// A shadow travels along the light ray, so a caster and everything it darkens
// share the same light-space X and Y. A receiver reads the finest level that
// contains it. Put those together and a caster is only worth submitting to
// level i if some pixel that READS level i shares its light-space column —
// which makes the caster set per level, not global. Three levels, two LODs of
// one island and a scatter pool that only exists near the player is exactly
// the case where that matters, and it is measured: submitting everything to
// everything cost 1.48 M triangles a frame against DESIGN §9's 1.5 M budget,
// with the whole increase in work nothing could see.
//
//   * The near ring and its scatter stop at level 1. They span ±192 m, so
//     every pixel they can darken is inside level 1's box; level 2 is read
//     only by pixels past 204 m of light-space X, where the ring cannot
//     reach. (Its diagonal corners do poke to ~226 m — a sliver where a level
//     2 receiver loses the ring's trees. The far mesh covers that ground.)
//   * The far mesh starts at level 1. Level 0's box is ±80 m, entirely inside
//     the hole, so the far mesh would have filled a 2048² map only to discard
//     nearly all of it. What that costs is a whole map's fill per frame; what
//     it buys is the band from the hole edge out to 227 m along the sun's
//     bearing (level 0's frustum is deep, and light-space Y moves 0.35 m per
//     metre of ground at this sun) darkening pixels within 68 m of the player.
//     The near ring already casts over most of that band, and every pixel
//     past 68 m reads level 1, which has the far mesh. So it is dropped, and
//     that is the one thing here that trades an image for a budget.
//
// The hole stays on in EVERY level, including the one the near ring does not
// cast into. It leaves level 2 without a caster over the ring's footprint,
// which is correct rather than merely cheap: a level-2 receiver is more than
// 204 m out in light-space X and the footprint reaches 226 m, so all that goes
// missing is a sliver at the box's diagonal corners.

/**
 * The depth material the far mesh casts through. Returns a MeshDepthMaterial
 * carrying `userData.uniforms`; terrain.js writes `uNearHole` and nothing else
 * touches it.
 *
 * `getDepthMaterial` in three copies `side` onto whatever it hands back, from
 * the source material's `shadowSide` — which materials.js sets to FrontSide
 * for the ground, because a heightfield has no back faces to render and three's
 * default (cast from the BACK face, the closed-mesh answer to acne) culls it
 * out of the map entirely.
 */
export function makeFarCasterDepthMaterial() {
  const uniforms = {
    // xy = the near ring footprint's world centre, zw = its half extent in
    // world X and Z. zw == 0 is "no hole".
    uNearHole: { value: new THREE.Vector4(0, 0, 0, 0) },
    uCasterSink: { value: FAR_CASTER_SINK_M },
  };
  const material = new THREE.MeshDepthMaterial({
    depthPacking: THREE.RGBADepthPacking,
  });
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = once(
      shader.vertexShader,
      "#include <common>",
      /* glsl */ `#include <common>
uniform float uCasterSink;
varying vec3 vGatesFarPos;`,
      "far caster (vertex pars)",
    );
    shader.vertexShader = once(
      shader.vertexShader,
      "#include <begin_vertex>",
      /* glsl */ `#include <begin_vertex>
  // World XZ is read BEFORE the sink so the hole is a footprint and not a
  // function of how far the caster was pushed down.
  vGatesFarPos = ( modelMatrix * vec4( transformed, 1.0 ) ).xyz;
  transformed.y -= uCasterSink;`,
      "far caster (sink)",
    );
    shader.fragmentShader = once(
      shader.fragmentShader,
      "#include <common>",
      /* glsl */ `#include <common>
uniform vec4 uNearHole;
varying vec3 vGatesFarPos;`,
      "far caster (fragment pars)",
    );
    shader.fragmentShader = once(
      shader.fragmentShader,
      "#include <clipping_planes_fragment>",
      /* glsl */ `#include <clipping_planes_fragment>
  // The near ring owns this column of the world. Discarding rather than
  // clipping the geometry keeps the hole a uniform: it moves with the player
  // every time the ring re-centres, and no buffer is rebuilt for it.
  if ( all( lessThan( abs( vGatesFarPos.xz - uNearHole.xy ), uNearHole.zw ) ) ) discard;`,
      "far caster (hole)",
    );
  };
  material.userData.uniforms = uniforms;
  return material;
}

// The caster/level policy argued above, as two numbers.
export const NEAR_CASTER_MAX_LEVEL = 1;
export const FAR_CASTER_MIN_LEVEL = 1;

/**
 * Submit `object` to the shadow pass only for clipmap levels in
 * [minLevel, maxLevel]; outside that range its draw is emptied.
 *
 * three has no per-light caster filter — `castShadow` is one flag for every
 * shadow map in the scene — so this rides `onBeforeShadow`/`onAfterShadow`,
 * which three calls around each object's depth draw and hands the shadow
 * camera. Collapsing the geometry's draw range to nothing leaves the call in
 * the list (a bound program and zero triangles) and takes every vertex and
 * fragment out of it, which is the half that costs. The range is restored on
 * the way out, so the main pass — which draws after the whole shadow pass —
 * always sees the full mesh.
 *
 * Both callbacks are per-object and allocation-free; a shadow camera that
 * carries no level tag (anything not ours) always draws.
 */
export function castInLevels(object, minLevel, maxLevel) {
  const geo = object.geometry;
  object.onBeforeShadow = (renderer, obj, camera, shadowCamera) => {
    const i = shadowCamera.userData.gatesClipLevel;
    if (i !== undefined && (i < minLevel || i > maxLevel)) geo.setDrawRange(0, 0);
  };
  object.onAfterShadow = () => {
    geo.setDrawRange(0, Infinity);
  };
  return object;
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
    // How much light-space DEPTH a level's own box costs, per metre of
    // half-width. A level bounds light-space X and Y; the ground inside those
    // bounds is a slab, and how long that slab is along the light ray is set
    // by how low the sun is. Level-space y and world ground trade at sin(el)
    // (720 m of Y is 2 km of ground at a 21° sun), and that ground carries
    // cos(el) of depth with it — so the box's own half-depth is
    // half·cos/sin = half·cot(el), and at this sun that is 2.66 m of depth per
    // metre of half-width.
    //
    // Lighting v0 carried a flat 260 m below the centre, which was right for
    // the 80 m map it shipped and quietly wrong for anything larger: level 2's
    // 720 m box needs 1913 m and had 260, so its ortho frustum could not
    // contain the ground it was drawn for and the far half of every level
    // clipped out along the sun's bearing. The far mesh casting made that
    // visible for the first time — nothing used to live out there.
    //
    // Relief is the second term and it is not scaled: the island is ~90 m tall
    // (TERRAIN.md §6) and height buys depth at 1/sin(el).
    this._depthPerHalf = Math.sqrt(1 - toSun.y * toSun.y) / toSun.y; // cot(el)
    this._depthRelief = RELIEF_M / toSun.y;
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
      // Half the depth this level's own box costs, plus relief and slack.
      const zReach =
        halfWidth * this._depthPerHalf + this._depthRelief + LIGHT_MARGIN_M;
      sh.camera.near = NEAR_M;
      sh.camera.far = Math.max(NEAR_M + 1, Math.min(FAR_CAP_M, 2 * zReach));
      sh.bias = 0;
      const texelM = (halfWidth * 2) / px;
      sh.normalBias = NORMAL_BIAS_TEXELS * texelM;
      // The clipmap owns every refresh: three must never render a level from
      // a transform the shader is not sampling (it skips updateMatrices on
      // the same test, which is what freezes the committed centre).
      sh.autoUpdate = false;
      sh.needsUpdate = false;
      // three hands the shadow camera to onBeforeShadow and nothing else that
      // identifies the level; tagging it here is what makes castInLevels()
      // above a lookup rather than a search.
      sh.camera.userData.gatesClipLevel = i;
      sh.camera.updateProjectionMatrix();
      scene.add(light);
      scene.add(light.target); // or its matrixWorld never updates
      return {
        light,
        halfWidth,
        texelM,
        // Where the light sits along the sun ray, relative to the committed
        // centre: half the frustum's depth, so the box is centred in it.
        zReach,
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
        .set(dx, dy, dz + L.zReach)
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
      // Depth fetches every shaded fragment pays for shadow, summed over the
      // levels — the per-fragment half of the budget, counted rather than
      // timed and therefore quotable anywhere. The near level's share is read
      // off three's own chunk (nearFilterFetches), so a three upgrade that
      // changes the kernel moves this number instead of rotting a comment.
      depthFetches: clipmapFetches("ship"),
      nearFilterType: NEAR_FILTER_TYPE,
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
