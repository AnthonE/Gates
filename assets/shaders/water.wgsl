// The sea's surface: a volume graded per pixel, the sky in the reflection,
// the sun on the waves. Binding and numbers are `render/water.rs`; the
// argument for each term is there too, next to the constant it reads.
//
// What one fragment does, in order:
//
//  1. **How much water is behind it.** With a depth prepass (the desktop) the
//     height of the surface over whatever the ray hits behind it — the
//     seabed, a boulder, a wading leg. Without one (WebGL2) the water column
//     the vertex read off the terrain. Either way it is a vertical depth.
//  2. **The body**: `S·(1 − T)` and `α = 1 − mean(T)` with `T = e^{−σ·path}`,
//     the same volume `water::depth_tint` states, over the REFRACTED path
//     (`water::refracted_path`) so the shallows stay clear from the sand.
//  3. **Foam**: the vertex's wash and whitecaps, plus a broken lip where the
//     water meets something standing in it (prepass only).
//  4. **Bevy's lighting for the body only** — sun, shadows, the hemisphere
//     fill — with the specular zeroed, because:
//  5. **The reflection is drawn here**: Schlick's Fresnel over the sky in the
//     reflected direction (an analytic clear sky plus the cloud deck's own
//     cubemap), and a GGX glint off each directional light.
//  6. **The blend is premultiplied** and Fresnel attenuates the background as
//     well as adding the sky: `α' = α + (1 − α)·F`.
//
// The vertex stage draws the swell, the breakers and the foam: the CPU only
// writes what each vertex is (`water::stream`, on a snap) and the phases
// (`water::animate`). `water::surface_local` is the same function in Rust and
// `tests/water.rs` holds it to the CPU's own `water::surface_at`; this one is
// its transcription, line for line. `tests/shaders.rs` composes and validates
// the file; what it draws, only a frame shows.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions,
    mesh_view_bindings::{view, lights, fog},
    mesh_view_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    lighting,
    shadows,
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc, position_world_to_clip},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif
#ifdef ATMOSPHERE
#import bevy_pbr::mesh_view_bindings::atmosphere_data
#endif

struct Water {
    // xyz `water::EXTINCT`, w `ALPHA_MAX`.
    optics: vec4<f32>,
    // xyz `SCATTER_ALBEDO`, w `WATER_IOR`.
    scatter: vec4<f32>,
    // xyz `FOAM_BODY`, w `FOAM_ALPHA`.
    foam: vec4<f32>,
    // xyz the clear sky's zenith, cd/m²; w the analytic sky's share (1 native, 0 browser).
    zenith: vec4<f32>,
    // xyz the clear sky's horizon, cd/m²; w `sky::AIR_FLOOR`.
    horizon: vec4<f32>,
    // xyz toward the sun; w cd/m² per deck texel unit.
    sun: vec4<f32>,
    // xyz the dusk glow toward a low sun, cd/m²; w spare.
    glow: vec4<f32>,
    // xyz the night floor, cd/m²; w 1 where the atmosphere lays the sky over
    // the deck per channel, 0 where through its transmittance's mean.
    night: vec4<f32>,
    // x the sun's share of daylight, y the noon sun's height, z `TWILIGHT_SKY`,
    // w `SKY_FALLOFF`.
    hour: vec4<f32>,
    // x `EDGE_M`, y `CONTACT_M`, z `CONTACT_FOAM`, w the breakers' phase.
    shore: vec4<f32>,
    // x F0, y `GLINT_A2_NEAR`, z `GLINT_A2_FAR`, w `GLINT_FAR_M`.
    glint: vec4<f32>,
    // xyz `water::LAKE_EXTINCT`, w one over the island's size (the open map).
    lake: vec4<f32>,
    // xyz `water::LAKE_SCATTER`, w one over `DEPTH_R_M`.
    lake_scatter: vec4<f32>,
    // A wave of `water::WAVES` a slot: xy its direction, z one over its length
    // (cycles a metre), w its amplitude.
    waves: array<vec4<f32>, 4>,
    // The swell's phases with the grid centre folded in, cycles.
    phase: vec4<f32>,
    // The weather's gain on each wave.
    swell: vec4<f32>,
    // x the weather's gain on the breakers, y one over `SURF_LEN_M`, z
    // `SURF_TRAIL_RAD`, w `SURF_FOAM_MAX / SURF_AMP_M`.
    surf: vec4<f32>,
    // x `CREST_FOAM_FROM`, y `CREST_FOAM_SPAN`, z `CREST_FOAM_MAX`, w one over
    // `PX_ANISO_MAX`.
    crest: vec4<f32>,
    // x `BREAKER_FROM_M`, y `BREAKER_SPAN_M`, z `WASH_FLOOR`, w `FOAM_SURGE_FLOOR`.
    breaker: vec4<f32>,
    // xy the grid centre the phases are folded around; z `PX_FOOTPRINT_K`,
    // w `DECK_FOOTPRINT_K`.
    field: vec4<f32>,
    // x `SHOAL_FULL_M`, y `HORIZON_NDOTV`, z `SURF_FRONT_M / SURF_LEN_M`, w
    // this sea state's `hidden_slope`.
    px: vec4<f32>,
    // x 1 with the eye under the sea, y `underwater::SHELL_UP`, z
    // `underwater::SHELL_DOWN`, w spare.
    under: vec4<f32>,
    // x how hard it rains on the sea, yzw the three drop layers' phases.
    rain: vec4<f32>,
    // xyz the drop layers' cycle counts, w `GLINT_RAIN_A2`.
    rain_cycle: vec4<f32>,
    // The wakes (`water::Wakes::pack`): per body, xy its box's centre off
    // `field.xy`, z the box's radius squared, w its live drops. Live first.
    wake_box: array<vec4<f32>, 4>,
    // Per body, 12 slots: xy the drop off its box's centre, z its age in
    // seconds, w its strength. Live first.
    wake: array<vec4<f32>, 48>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: Water;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var sky_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var sky_sampler: sampler;
// How much of the water is open sea, over the island (`water::OPEN_MAP_TEX`).
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var open_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var open_sampler: sampler;

// Deep by fiat: `water::DEEP_SENTINEL_M`. Saturates every curve here.
const DEEP_M: f32 = 64.0;

// The wakes' rings (`water::WAKE_*`; `tests/water.rs` holds the shared ones
// equal): where a ring starts, how fast it spreads, how long it lives, the
// ripple's wavelength and how far either side of the front it reaches.
const WAKE_FRONT_M: f32 = 0.15;
const WAKE_SPEED: f32 = 0.7;
const WAKE_LIFE_S: f32 = 2.6;
const WAKE_LEN_M: f32 = 0.3;
const WAKE_BAND_M: f32 = 0.6;
// The ripple's peak slope at full strength, and one over its envelope's
// half-width squared (0.22 m): a few ripples deep.
const WAKE_SLOPE: f32 = 0.12;
const WAKE_ENV: f32 = 20.66;
const TAU: f32 = 6.28318530718;

// ---------------------------------------------------------------------------
// The vertex stage: `water::surface_local`, transcribed.
// ---------------------------------------------------------------------------

// An angle given in cycles, reduced to [-π, π) before any trig: WGSL promises
// `sin` only near zero, and WebGL2 promises nothing.
fn reduced(cycles: f32) -> f32 {
    return TAU * (fract(cycles + 0.5) - 0.5);
}

// `water::shape`: the crest-sharpened sine, `2u² − ¾`, and its derivative.
fn wave_shape(theta: f32) -> vec2<f32> {
    let u = 0.5 + 0.5 * sin(theta);
    return vec2<f32>(2.0 * u * u - 0.75, 2.0 * u * cos(theta));
}

// One wave of the swell at `local`: its height and gradient, `(h, gx, gz)`.
// `w` is its `waves` slot, `share` the vertex's band weight times shoaling.
fn swell_term(w: vec4<f32>, phase: f32, gain: f32, share: f32, local: vec2<f32>) -> vec3<f32> {
    let s = wave_shape(reduced(dot(w.xy, local) * w.z + phase));
    let a = w.w * gain * share;
    return vec3<f32>(a * s.x, (a * s.y * TAU * w.z) * w.xy);
}

// `water::crest_foam`: whitecaps, judged against the clear sea's reach.
fn crest_foam(h: f32, reach: f32) -> f32 {
    if reach <= 0.0 {
        return 0.0;
    }
    let t = clamp((h / reach - water.crest.x) / water.crest.y, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t) * water.crest.z;
}

// How much a bore breaks in water this deep (`water::breaker_foam`'s).
fn breaking(depth: f32) -> f32 {
    return 1.0 - clamp((depth - water.breaker.x) / water.breaker.y, 0.0, 1.0);
}

// Locations 8–10 are `water::ATTRIBUTE_SWELL`, `ATTRIBUTE_SURF` and
// `ATTRIBUTE_REACH`, written on a snap. The position is the vertex's offset
// from the grid centre at sea level; the centre is the entity's translation.
// `uv_b` arrives as `[wash, water column]` and leaves as `[whitecaps, column]`;
// the wash, the bores' phase and their scale leave in `color` (`foam_px`).
@vertex
fn vertex(
    in: Vertex,
    @location(8) swell_w: vec4<f32>,
    @location(9) surf: vec4<f32>,
    @location(10) reach: f32,
) -> VertexOutput {
    let local = in.position.xz;

    // 1 · The swell, unrolled: constant indices only, which WebGL2's GLSL
    // and every GPU take without a second thought.
    let sw = swell_term(water.waves[0], water.phase.x, water.swell.x, swell_w.x, local)
        + swell_term(water.waves[1], water.phase.y, water.swell.y, swell_w.y, local)
        + swell_term(water.waves[2], water.phase.z, water.swell.z, swell_w.z, local)
        + swell_term(water.waves[3], water.phase.w, water.swell.w, swell_w.w, local);
    var h = sw.x;
    var gb = vec2<f32>(0.0);
    // Whitecaps on the swell alone, before the breaker is added.
    let caps = crest_foam(h, reach);

    // 2 · The breakers, phased on the distance to dry ground. Their height
    // and slope ride the vertex; their foam is phased per pixel (`foam_px`),
    // off the distance in breaker cycles, which interpolates exactly.
    let amp = surf.w * water.surf.x;
    var b = -1.0;
    if any(surf.yz != vec2<f32>(0.0)) {
        if amp > 0.0 {
            let s = wave_shape(reduced(surf.x * water.surf.y + water.shore.w / TAU));
            h = h + amp * s.x;
            gb = (amp * s.y * TAU * water.surf.y) * surf.yz;
        }
        b = water.surf.w * amp * breaking(in.uv_b.y);
    }

    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let p = vec3<f32>(in.position.x, in.position.y + h, in.position.z);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    // The breaker's normal only: the swell's is drawn per pixel
    // (`swell_px`), except on POTATO.
#ifdef WATER_VERTEX_NORMALS
    let g = sw.yz + gb;
#else
    let g = gb;
#endif
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        normalize(vec3<f32>(-g.x, 1.0, -g.y)),
        in.instance_index,
    );
    out.uv = in.uv;
    out.uv_b = vec2<f32>(caps, in.uv_b.y);
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        in.tangent,
        in.instance_index,
    );
    // `(open, wash, distance to dry ground in breaker cycles, bore scale)`:
    // the lake share stays in red; the rest is `foam_px`'s.
    out.color = vec4<f32>(in.color.r, in.uv_b.x, surf.x * water.surf.y, b);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index,
        world_from_local[3],
    );
#endif
    return out;
}

fn hash2(p: vec2<i32>) -> f32 {
    let q = bitcast<vec2<u32>>(p);
    var h = q.x * 374761393u + q.y * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

// Value noise in world metres over `1 / scale`, in [0, 1].
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = hash2(c);
    let b = hash2(c + vec2<i32>(1, 0));
    let d = hash2(c + vec2<i32>(0, 1));
    let e = hash2(c + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}

// The clear sky's air-mass weight toward elevation `y`: 0 at the zenith, 1
// at the horizon (`sky::backdrop_at`'s curve).
fn air_weight(y: f32) -> f32 {
    let e = clamp(y, 0.0, 1.0);
    let f = water.horizon.w;
    let m0 = 1.0 / (1.0 + f);
    return clamp((1.0 / (e + f) - m0) / (1.0 / f - m0), 0.0, 1.0);
}

#ifdef ATMOSPHERE
// The eye's radius in the atmosphere's frame: at the ground. The island's
// relief is noise against a 6 360 km planet.
fn ground_r() -> f32 {
    return atmosphere_data.atmosphere.bottom_radius + 1.0;
}
#endif

// The hour's light on the clear sky (`water::sky_light` says why it is here).
// Natively: the atmosphere's own transmittance toward the sun over its value
// at the noon sun the clear sky is calibrated at — darker and redder as the
// sun goes down, off the same LUT that reddens the sun on every surface. The
// hue is taken at a third: the sky is lit along many paths through the air,
// not only the direct one, and the full ratio turns a dusk zenith crimson
// where the atmosphere draws it near grey. The level falls as the ratio to
// `SKY_FALLOFF` (fitted at dusk) and is floored at `TWILIGHT_SKY` of the lux
// share, for the sky after sunset. Without an atmosphere, the lux share.
fn daylight_tint() -> vec3<f32> {
#ifdef ATMOSPHERE
    let r = ground_r();
    let mu = clamp(water.sun.y, -1.0, 1.0);
    let t = lighting::sample_transmittance_lut(r, max(mu, 0.0));
    let t_ref = lighting::sample_transmittance_lut(r, water.hour.y);
    let ratio = (t / max(t_ref, vec3<f32>(1e-4))) * smoothstep(-0.03, 0.02, mu);
    let lum = dot(ratio, vec3<f32>(0.2126, 0.7152, 0.0722));
    let hue = ratio / max(lum, 1e-5);
    let level = max(pow(max(lum, 0.0), water.hour.w), water.hour.z * water.hour.x);
    return level * mix(vec3<f32>(1.0), hue, 0.35 * smoothstep(0.0, 1e-3, lum));
#else
    return vec3<f32>(water.hour.x);
#endif
}

// Sky radiance toward `r` (unit, r.y >= 0), cd/m². The cloud deck is looked
// up toward `rd` instead: the same reflection off the wave's own normal,
// without the ripples. The deck has no mips, so a ripple-scattered ray reads
// its noise at full resolution and the clouds come back as glitter; the swell
// alone bends them the way a real sea smears them.
fn sky_radiance(r: vec3<f32>, rd: vec3<f32>, tint: vec3<f32>) -> vec3<f32> {
    var c = mix(water.zenith.rgb, water.horizon.rgb, air_weight(r.y));
    // A low sun warms the horizon on its own side (`sky::compose_range`).
    let sh = water.sun.xz;
    let rh = r.xz;
    let facing = max(dot(rh, sh) * inverseSqrt(max(dot(rh, rh) * dot(sh, sh), 1e-8)), 0.0);
    let low = max(1.0 - clamp(r.y, 0.0, 1.0) / 0.4, 0.0);
    c = c + water.glow.rgb * (facing * facing * facing) * (low * low);
    c = (c * tint + water.night.rgb) * water.zenith.w;
    // The deck, sampled the way the `Skybox` samples it (z flipped). Explicit
    // level: this runs under a branch.
    let raw = textureSampleLevel(sky_cube, sky_sampler, rd * vec3<f32>(1.0, 1.0, -1.0), 0.0).rgb;
    // Natively the atmosphere draws the deck as `inscatter + T·cube`, with T
    // the column toward that direction — and the deck is stored with T's hue
    // divided out (`sky::deck_hue`), so read raw it is blue (the first
    // capture's periwinkle speckle) and too bright at the horizon (the first
    // dusk's). Multiplying by the same LUT's T is the sky's own composite. A
    // browser's deck is already the finished sky.
#ifdef ATMOSPHERE
    // Through `T` as the sky pass lays it: per channel with dual-source
    // blending, its mean without (`water.night.w`) — the deck's lift was
    // made for whichever this device does (`sky::deck_hue`).
    let t = lighting::sample_transmittance_lut(ground_r(), max(rd.y, 0.0));
    let deck = raw * mix(vec3<f32>((t.r + t.g + t.b) / 3.0), t, water.night.w);
#else
    let deck = raw;
#endif
    return c + deck * water.sun.w;
}

// The sun (and any other directional light) off the surface: GGX, shadowed,
// through the atmosphere's transmittance where Bevy applies it.
fn sun_glint(N: vec3<f32>, V: vec3<f32>, wp: vec3<f32>, world_normal: vec3<f32>, flags: u32, n_var: f32, extra: f32) -> vec3<f32> {
    let dist = length(view.world_position.xyz - wp);
    // The ripples' share by distance, plus the slope this pixel cannot
    // resolve (`extra`: the swell's hidden variance, and the rain's).
    var a2 = mix(water.glint.y, water.glint.z, smoothstep(0.0, water.glint.w, dist)) + extra;
    // Specular anti-aliasing (Kaplanyan & Hoffman 2016): the normal's variance
    // across this pixel is slope the pixel cannot show, so it goes into the lobe.
    a2 = min(a2 + min(2.0 * n_var, 0.18), 1.0);
    let roughness = sqrt(a2);
    let perceptual = sqrt(roughness);
    let NdotV = max(dot(N, V), 1e-4);

    var input: lighting::LightingInput;
    input.layers[lighting::LAYER_BASE].N = N;
    input.layers[lighting::LAYER_BASE].R = reflect(-V, N);
    input.layers[lighting::LAYER_BASE].NdotV = NdotV;
    input.layers[lighting::LAYER_BASE].perceptual_roughness = perceptual;
    input.layers[lighting::LAYER_BASE].roughness = roughness;
    input.P = wp;
    input.V = V;
    input.diffuse_color = vec3<f32>(0.0);
    input.F0_ = vec3<f32>(water.glint.x);
    input.F_ab = lighting::F_AB(perceptual, NdotV);
#ifdef STANDARD_MATERIAL_CLEARCOAT
    input.clearcoat_strength = 0.0;
    input.layers[1].N = N;
    input.layers[1].R = reflect(-V, N);
    input.layers[1].NdotV = NdotV;
    input.layers[1].perceptual_roughness = 1.0;
    input.layers[1].roughness = 1.0;
#endif
#ifdef STANDARD_MATERIAL_ANISOTROPY
    input.anisotropy = 0.0;
    input.Ta = vec3<f32>(1.0, 0.0, 0.0);
    input.Ba = vec3<f32>(0.0, 0.0, 1.0);
#endif

    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), vec4<f32>(wp, 1.0));

    var total = vec3<f32>(0.0);
    let n = lights.n_directional_lights;
    for (var i: u32 = 0u; i < n; i = i + 1u) {
        var shadow = 1.0;
        if ((flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
                && (lights.directional_lights[i].flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            shadow = shadows::fetch_directional_shadow(i, vec4<f32>(wp, 1.0), world_normal, view_z);
        }
        total += lighting::directional_light(i, &input, false) * shadow;
    }
    return total * view.exposure;
}

// How much of the first directional light reaches `wp` (1 unshadowed).
fn sun_shadow(wp: vec3<f32>, world_normal: vec3<f32>, flags: u32) -> f32 {
    if lights.n_directional_lights == 0u
            || (flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) == 0u
            || (lights.directional_lights[0].flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), vec4<f32>(wp, 1.0));
    return shadows::fetch_directional_shadow(0u, vec4<f32>(wp, 1.0), world_normal, view_z);
}

// The swell at one pixel (`water::swell_px`, line for line): its gradient,
// the gradient the deck is reflected off, and the slope variance the pixel
// cannot resolve. Every wave retires on the pixel's footprint along it (and
// on its long side, past `PX_ANISO_MAX`), not on the mesh spacing
// (`band_weight`'s law), so the swell shapes the reflection far past the
// mesh's core.
struct SwellPx {
    g: vec2<f32>,
    gd: vec2<f32>,
    hidden: f32,
}

// `water::footprint_keep`, `c` the footprint in the wave's cycles.
fn footprint_keep(c: f32) -> f32 {
    return 1.0 - smoothstep(0.25, 0.5, c);
}

fn swell_px_term(acc: SwellPx, w: vec4<f32>, phase: f32, gain: f32, shoal: f32,
                 local: vec2<f32>, fx: vec2<f32>, fy: vec2<f32>, floor: f32) -> SwellPx {
    let c = max(max(abs(dot(fx, w.xy)), abs(dot(fy, w.xy))), floor) * w.z;
    let keep = footprint_keep(water.field.z * c);
    let keep_d = footprint_keep(water.field.w * c);
    let s = wave_shape(reduced(dot(w.xy, local) * w.z + phase));
    let ak = w.w * gain * shoal * TAU * w.z;
    var o = acc;
    o.g = acc.g + (ak * keep * s.y) * w.xy;
    o.gd = acc.gd + (ak * keep_d * s.y) * w.xy;
    o.hidden = acc.hidden + 0.625 * ak * ak * (1.0 - keep * keep);
    return o;
}

fn swell_px(local: vec2<f32>, fx: vec2<f32>, fy: vec2<f32>, fp: f32, shoal: f32) -> SwellPx {
    var o = SwellPx(vec2<f32>(0.0), vec2<f32>(0.0), 0.0);
    let floor = fp * water.crest.w;
    o = swell_px_term(o, water.waves[0], water.phase.x, water.swell.x, shoal, local, fx, fy, floor);
    o = swell_px_term(o, water.waves[1], water.phase.y, water.swell.y, shoal, local, fx, fy, floor);
    o = swell_px_term(o, water.waves[2], water.phase.z, water.swell.z, shoal, local, fx, fy, floor);
    o = swell_px_term(o, water.waves[3], water.phase.w, water.swell.w, shoal, local, fx, fy, floor);
    return o;
}

// `underwater::shade`: the shell's tint toward a direction's height, lighter
// above the eye and darker below.
fn shell_shade(y: f32) -> f32 {
    if y >= 0.0 {
        return 1.0 + (water.under.y - 1.0) * y;
    }
    return 1.0 + (1.0 - water.under.z) * y;
}

// One layer of raindrops on the sea at world `p` (never the grid's local
// offset, or every drop would jump on a snap): the slope its ring puts on the
// surface and the speck of its splash, `(gx, gz, speck)`. A cell holds one
// drop a cycle at a hashed spot and moment; `water::rain_cycle_of` is its
// timing in Rust, continuous across the layer's wrap and the counter's. Light
// rain is sparse, and a layer finer than the pixel retires on its footprint
// `fp` (`band_weight`'s law again).
fn rain_layer(p: vec2<f32>, density: f32, phase: f32, cycle: f32, salt: i32, fp: f32) -> vec3<f32> {
    let q = p * density + vec2<f32>(0.37, 0.71) * f32(salt);
    let cell = vec2<i32>(floor(q));
    let t = phase + hash2(cell + vec2<i32>(salt * 977, 0));
    let ph = fract(t);
    let cyc = (i32(cycle) + i32(floor(t))) & 4095;
    let key = cell + vec2<i32>(cyc * 7919, cyc * 6271 + salt * 131);
    let live = step(hash2(key + vec2<i32>(3, 0)), water.rain.x);
    if live == 0.0 {
        return vec3<f32>(0.0);
    }
    let dv = fract(q) - (0.35 + 0.3 * vec2<f32>(hash2(key), hash2(key + vec2<i32>(0, 1))));
    let r = length(dv);
    let front = 0.3 * ph;
    let x = (r - front) / 0.12;
    let env = max(1.0 - x * x, 0.0);
    let keep = 1.0 - smoothstep(0.05, 0.1, fp * density);
    let a = 0.25 * live * keep * (1.0 - ph) * (1.0 - ph) * env * env;
    let slope = a * cos(reduced((r - front) / 0.2)) * dv / max(r, 1e-3);
    let speck = live * keep * (1.0 - smoothstep(0.0, 0.12, ph)) * (1.0 - smoothstep(0.0, 0.06, r));
    return vec3<f32>(slope, speck);
}

// The wakes at `local` (`water::Wakes`): the slope their rings put on the
// surface, a young entry ring's splash, and the slope variance the pixel
// cannot resolve, `(gx, gz, splash, a2)`. Each drop's ring spreads from where
// it was laid at `WAKE_SPEED` and dies at `WAKE_LIFE_S`; laid along a moving
// body's track, their fronts draw the V. A ripple finer than the pixel retires
// on its footprint along the ring (`swell_px_term`'s law) into the glint.
// No derivatives here: the caller's `fx`, `fy` and `fp` are taken before any
// branch. Loop bounds come from the uniform, and nothing breaks out early.
fn wake_px(local: vec2<f32>, fx: vec2<f32>, fy: vec2<f32>, fp: f32) -> vec4<f32> {
    var g = vec2<f32>(0.0);
    var splash = 0.0;
    var a2 = 0.0;
    for (var b = 0; b < 4; b = b + 1) {
        let bx = water.wake_box[b];
        let lb = local - bx.xy;
        if bx.w > 0.0 && dot(lb, lb) < bx.z {
            let n = min(i32(bx.w), 12);
            for (var j = 0; j < n; j = j + 1) {
                let d = water.wake[b * 12 + j];
                let q = lb - d.xy;
                let rr = dot(q, q);
                let front = WAKE_FRONT_M + WAKE_SPEED * d.z;
                let lo = max(front - WAKE_BAND_M, 0.0);
                let hi = front + WAKE_BAND_M;
                if rr < hi * hi && rr > lo * lo {
                    let inv = inverseSqrt(max(rr, 1e-6));
                    let r = rr * inv;
                    let dir = q * inv;
                    let x = r - front;
                    let life = max(1.0 - d.z / WAKE_LIFE_S, 0.0);
                    let a = WAKE_SLOPE * d.w * life * life * exp(-x * x * WAKE_ENV)
                        * inverseSqrt(1.0 + 2.0 * r);
                    let c = max(max(abs(dot(fx, dir)), abs(dot(fy, dir))), fp * water.crest.w) / WAKE_LEN_M;
                    let keep = footprint_keep(water.field.z * c);
                    g += (a * keep * cos(TAU * x / WAKE_LEN_M)) * dir;
                    a2 += 0.5 * a * a * (1.0 - keep * keep);
                }
                // A body that went in hard (strength over 1): white where it
                // broke the surface, for the ring's first third of a second.
                if d.w > 1.0 && d.z < 0.35 && rr < 0.25 {
                    let s = (d.w - 1.0) * (1.0 - smoothstep(0.0, 0.35, d.z))
                        * (1.0 - smoothstep(0.1, 0.5, sqrt(rr)));
                    splash = max(splash, s);
                }
            }
        }
    }
    return vec4<f32>(g, splash, a2);
}

// A pixel's foam (`water::foam_px`, line for line): the whitecaps, the shore
// wash surging with the bores (or with the swell's `s0` where none runs) and
// the bore itself, phased per pixel off `cs`, the distance to dry ground in
// breaker cycles. The front arrives over `SURF_FRONT_M` or the pixel, and a
// pixel wider than a quarter of a breaker settles to the trail's mean.
fn foam_px(caps: f32, wash0: f32, cs: f32, b: f32, s0: f32, cw: f32) -> f32 {
    let x = fract(cs + water.shore.w / TAU - 0.25);
    var trail = exp(-TAU * x / water.surf.z);
    trail = mix(trail, 1.0, smoothstep(1.0 - max(cw, water.px.z), 1.0, x));
    let mean = water.surf.z / TAU * (1.0 - exp(-TAU / water.surf.z));
    trail = mix(trail, mean, smoothstep(0.25, 0.6, cw));
    let m = clamp(b + 1.0, 0.0, 1.0);
    let surge = mix(
        water.breaker.w + (1.0 - water.breaker.w) * (0.5 + 0.5 * s0),
        water.breaker.z + (1.0 - water.breaker.z) * trail,
        m,
    );
    return clamp(caps + wash0 * surge + max(b, 0.0) * trail, 0.0, 1.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let wp = in.world_position.xyz;
    // Every derivative before any branch: naga does not hold the fragment
    // stage to uniform control flow, so a `dpdx` under an `if` is a smear,
    // not an error.
    let fx = dpdx(wp.xz);
    let fy = dpdy(wp.xz);
    // How many breaker cycles this pixel spans (`foam_px`), and how many
    // metres it spans (the rain's footprint).
    let cw = fwidth(in.color.b);
    let fp = max(length(fx), length(fy));
    let local = wp.xz - water.field.xy;
    let V0 = normalize(view.world_position.xyz - wp);
    // How much of this water is open sea rather than lake: the vertex's own
    // sample near the eye, and the island's open map out past `DEPTH_R_M`,
    // where no vertex is sampled and the skirt is too coarse to find a lake.
    let far = smoothstep(0.7, 1.0, max(abs(local.x), abs(local.y)) * water.lake_scatter.w);
    let open_v = clamp(in.color.r, 0.0, 1.0);
    let open_m = textureSampleLevel(open_map, open_sampler, wp.xz * water.lake.w, 0.0).r;
    let open = mix(open_v, min(open_v, open_m), far);
    var vin = in;
    // `color` carries the foam's inputs past red (the lake share): Bevy must
    // not multiply them into the base colour it builds.
    vin.color = vec4<f32>(1.0);
#ifdef WATER_VERTEX_NORMALS
    // POTATO: the vertex's swell normal, and the swell's hidden slope by
    // distance alone (`GLINT_A2_FAR` is the ripples' share).
    let n_deck_up = in.world_normal;
    let hid = water.px.w * smoothstep(0.0, water.glint.w, length(view.world_position.xyz - wp));
#else
    // The swell per pixel, over the breaker's normal from the vertex. The
    // shoaling is the vertex column's (the prepass depth would halo a leg).
    let shoal = smoothstep(0.0, water.px.x, in.uv_b.y) * open;
    let sw = swell_px(local, fx, fy, fp, shoal);
    let nb = normalize(in.world_normal);
    let gb = -nb.xz / max(nb.y, 0.2);
    var ns = normalize(vec3<f32>(-(sw.g.x + gb.x), 1.0, -(sw.g.y + gb.y)));
    // A face turned past the eye at the horizon is bent back to graze, but
    // never past the flat sea's own grazing angle: far out the flat sea shows
    // the eye less than the floor, and bending it would dim the whole far sea.
    let graze = min(water.px.y, max(dot(nb, V0), 0.0));
    ns = normalize(ns + V0 * select(0.0, max(0.0, graze - dot(ns, V0)), is_front));
    vin.world_normal = ns;
    let n_deck_up = normalize(vec3<f32>(-(sw.gd.x + gb.x), 1.0, -(sw.gd.y + gb.y)));
    let hid = sw.hidden;
#endif
    var pbr_input = pbr_input_from_standard_material(vin, is_front);
    var N = pbr_input.N;
    let V = pbr_input.V;

    let dndx = dpdx(N);
    let dndy = dpdy(N);
    let n_var = 0.5 * (dot(dndx, dndx) + dot(dndy, dndy));
    // The sea is a height field: from above it, a back face under the eye is
    // the far slope of a crest, behind its near slope, and from below a front
    // face over the eye is a farther wave, behind the window. The sea writes
    // no depth, so either would show through (dark teeth under every crest).
    // `under` is the flat sea's (`WeatherNow::underwater`), so a crest the eye
    // is inside, over or under it, keeps its faces.
    let eye_y = view.world_position.y;
    if select(!is_front && wp.y < eye_y, is_front && wp.y > eye_y, water.under.x > 0.5) {
        discard;
    }

    // 1 · The water behind this pixel, vertical metres.
#ifdef VERTEX_UVS_B
    // With no wash and no bore (the open sea), `foam_px` is the caps alone.
    var foam = clamp(in.uv_b.x, 0.0, 1.0);
    if in.color.g > 0.0 || in.color.a > 0.0 {
        // The swell's own surge, which the wash breathes with where no bore runs.
        let s0 = sin(reduced(dot(water.waves[0].xy, local) * water.waves[0].z + water.phase.x));
        foam = foam_px(in.uv_b.x, in.color.g, in.color.b, in.color.a, s0, cw);
    }
    var dz = max(in.uv_b.y, 0.0);
#else
    var foam = 0.0;
    var dz = DEEP_M;
#endif
    var contact = false;
#ifdef DEPTH_PREPASS
    if is_front {
        let d = prepass_depth(in.position, 0u);
        if d > 0.0 {
            let behind = position_ndc_to_world(frag_coord_to_ndc(vec4<f32>(in.position.xy, d, 1.0)));
            dz = max(wp.y - behind.y, 0.0);
        } else {
            // Nothing drawn behind (reverse-Z far plane): open sea.
            dz = DEEP_M;
        }
        contact = true;
    }
#endif

    // 2 · The body, over the refracted path. A lake is murkier than the sea:
    // its optics blend in by the lake share (`water::LAKE_EXTINCT`).
#ifdef VERTEX_COLORS
    let lake = 1.0 - open;
#else
    let lake = 0.0;
#endif
    let sigma = mix(water.optics.xyz, water.lake.xyz, lake);
    let albedo = mix(water.scatter.xyz, water.lake_scatter.xyz, lake);
    let ior = water.scatter.w;
    let cos_i = clamp(abs(V.y), 0.0, 1.0);
    let sin2_t = (1.0 - cos_i * cos_i) / (ior * ior);
    let path = dz / sqrt(max(1.0 - sin2_t, 1e-4));
    let T = exp(-sigma * path);
    let alpha_max = water.optics.w;
    let a_body = clamp((1.0 - (T.x + T.y + T.z) / 3.0) * alpha_max, 0.0, alpha_max);
    var body = vec4<f32>(albedo * (1.0 - T), a_body);

    // 3 · Foam. The vertex's wash and bores are bands along the depth
    // contours, and a band of one even white reads as a drawn streak: break
    // it into patches, in world space so a bore runs THROUGH them. The fine
    // octave settles to its mean before it is finer than a pixel.
    let dist = length(view.world_position.xyz - wp);
    let settle = smoothstep(25.0, 90.0, dist);
    if foam > 0.0 {
        let fine = mix(vnoise(wp.xz * 0.9 + vec2<f32>(5.0, 11.0)), 0.5, settle);
        let clump = vnoise(wp.xz * 0.3) * 0.6 + fine * 0.4;
        foam = clamp(foam * (0.25 + 1.3 * clump), 0.0, 1.0);
    }
    // Rain on the water near the eye: three layers of spreading rings and
    // their splashes, none where a drop is finer than the pixel.
    if water.rain.x > 0.0 && fp * 2.6 < 0.1 {
        var r = rain_layer(wp.xz, 2.6, water.rain.y, water.rain_cycle.x, 1, fp);
        if fp * 4.5 < 0.1 {
            r += rain_layer(wp.xz, 4.5, water.rain.z, water.rain_cycle.y, 2, fp);
        }
        if fp * 6.4 < 0.1 {
            r += rain_layer(wp.xz, 6.4, water.rain.w, water.rain_cycle.z, 3, fp);
        }
        N = normalize(N - vec3<f32>(r.x, 0.0, r.y));
        pbr_input.N = N;
        foam = max(foam, 0.3 * r.z);
    }
    // The wakes: rings off swimmers and waders, and a splash where a body
    // went in. Seen from below too, where `N` faces down, so the slope's sign
    // follows the face.
    var wake_a2 = 0.0;
    if water.wake_box[0].w > 0.0 {
        let wk = wake_px(local, fx, fy, fp);
        N = normalize(N - select(-1.0, 1.0, is_front) * vec3<f32>(wk.x, 0.0, wk.y));
        pbr_input.N = N;
        foam = max(foam, min(wk.z, 1.0));
        wake_a2 = wk.w;
    }
    // The lip: a noisy band a few decimetres up whatever stands in the water,
    // lapping with the breakers' phase.
    if contact {
        let band = smoothstep(0.0, 0.06, dz) * (1.0 - smoothstep(0.06, water.shore.y, dz));
        if band > 0.0 {
            let lip_fine = mix(vnoise(wp.xz * 4.1 + vec2<f32>(17.0, -9.0)), 0.5, settle);
            let nz = vnoise(wp.xz * 1.6) * 0.65 + lip_fine * 0.35;
            let lap = 0.5 + 0.5 * sin(water.shore.w + nz * 6.2831853);
            let lip = smoothstep(0.40, 0.70, nz * (0.6 + 0.4 * lap));
            foam = max(foam, band * lip * water.shore.z);
        }
    }
    // The contact line itself is a polygon intersection: everything fades in
    // from it, or it is drawn as a line.
    let edge = smoothstep(0.0, water.shore.x, dz);
    foam = foam * smoothstep(0.0, 0.04, dz);
    let fa = water.foam.w;
    body = vec4<f32>(
        body.rgb + (water.foam.rgb * fa - body.rgb) * foam,
        body.a + (fa - body.a) * foam
    );

    // 4 · Bevy lights the body. No specular from it: the sky and the sun are
    // drawn below, and its environment specular is the hemisphere fill.
    pbr_input.material.base_color = body;
    pbr_input.material.reflectance = vec3<f32>(0.0);
    // SSAO at this pixel is the seabed's, not the water's.
    pbr_input.diffuse_occlusion = vec3<f32>(1.0);
    // From below, only foam reads the lit body (Snell's window, 5b).
    var color = body;
    if is_front || water.under.x < 0.5 || foam > 0.0 {
        color = apply_pbr_lighting(pbr_input);
    }

    // Where the ripple normal turns faster than the pixels can follow, the
    // mirror reads it as speckle — at a grazing view one ripple swings the
    // reflected ray between the horizon and the blue above it. A real pixel
    // there averages many facets, so reflect off their mean: the wave's own
    // normal, by how fast N is changing across the pixel.
    let calm = 0.8 * smoothstep(0.002, 0.02, n_var);

    // 5 · The reflection, from above.
    if is_front {
        let Nr = normalize(mix(N, pbr_input.world_normal, calm));
        let NdotV = clamp(dot(Nr, V), 0.0, 1.0);
        let F0 = water.glint.x;
        let smooth_share = edge * (1.0 - foam);
        let F = (F0 + (1.0 - F0) * pow(1.0 - NdotV, 5.0)) * smooth_share;
        var R = reflect(-V, Nr);
        // A ray reflected downward would meet the next wave: read the horizon.
        R = normalize(vec3<f32>(R.x, max(R.y, 0.0) + 1e-3, R.z));
        var Rd = reflect(-V, n_deck_up);
        Rd = normalize(vec3<f32>(Rd.x, max(Rd.y, 0.0) + 1e-3, Rd.z));
        let sky = sky_radiance(R, Rd, daylight_tint()) * view.exposure;
        // The rain dulls the glitter it lands on.
        let wet = water.rain_cycle.w * water.rain.x;
        let glint = sun_glint(N, V, wp, pbr_input.world_normal, pbr_input.flags, n_var, hid + wet + wake_a2) * smooth_share;
        // 6 · Premultiplied: the sky replaces what Fresnel takes from below.
        color = vec4<f32>(
            color.rgb * (1.0 - F) + sky * F + glint,
            color.a + (1.0 - color.a) * F
        );
    } else if water.under.x > 0.5 {
        // 5b · From below, Snell's window: inside the critical angle (48.6°
        // off straight up) the sky, refracted and rippling; outside it the
        // surface is a mirror of the water itself (total internal
        // reflection), in the underwater shell's own shade. Bevy flips only
        // the ripple's tangent-space normal on a back face, so the wave's
        // normal is turned down here by hand. Opaque: from below, the
        // surface is the end of the view either way. The haze is still
        // Bevy's, below (one owner).
        let wn = -pbr_input.world_normal;
        let Nr = normalize(mix(N, wn, calm));
        let I = -V;
        let t = refract(I, Nr, water.scatter.w);
        let ct = clamp(dot(t, -Nr), 0.0, 1.0);
        let F = water.glint.x + (1.0 - water.glint.x) * pow(1.0 - ct, 5.0);
        let rim = smoothstep(0.0, 0.25, ct);
        // Past the critical angle `refract` gives nothing and `rim` is 0: the
        // sky is looked up only where some of it shows, and dimmed where the
        // sun is shadowed (a foundation or a pier over the water).
        var window = vec3<f32>(0.0);
        if rim > 0.0 {
            let td = refract(I, -n_deck_up, water.scatter.w);
            let tdu = select(vec3<f32>(0.0, 1.0, 0.0), td, dot(td, td) > 0.5);
            let up = normalize(vec3<f32>(t.x, max(t.y, 0.0) + 1e-3, t.z));
            let upd = normalize(vec3<f32>(tdu.x, max(tdu.y, 0.0) + 1e-3, tdu.z));
            let lit = 0.2 + 0.8 * sun_shadow(wp, pbr_input.world_normal, pbr_input.flags);
            window = sky_radiance(up, upd, daylight_tint()) * view.exposure * (1.0 - F) * lit;
        }
        let mirror = fog.base_color.rgb * shell_shade(reflect(I, Nr).y);
        // Foam is lit from above and seen through: Bevy's body, foam and all.
        let body_lit = color.rgb / max(color.a, 1e-4);
        color = vec4<f32>(mix(mix(mirror, window + F * mirror, rim), body_lit, foam), 1.0);
    }

    // Fog and the rest of Bevy's post-lighting work act on the straight
    // colour; the premultiply is ours (`Premultiplied` passes it through).
    let a = max(color.a, 1e-4);
    let post = main_pass_post_lighting_processing(pbr_input, vec4<f32>(color.rgb / a, color.a));
    var out: FragmentOutput;
    out.color = vec4<f32>(post.rgb * a, post.a);
    return out;
}
