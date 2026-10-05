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
// ⚠ Nothing in CI compiles this file. Boot the game.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, lights},
    mesh_view_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    lighting,
    shadows,
    view_transformations::{position_ndc_to_world, frag_coord_to_ndc},
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
    // xyz the night floor, cd/m²; w spare.
    night: vec4<f32>,
    // x the sun's share of daylight, y the noon sun's height, z `TWILIGHT_SKY`,
    // w `SKY_FALLOFF`.
    hour: vec4<f32>,
    // x `EDGE_M`, y `CONTACT_M`, z `CONTACT_FOAM`, w the breakers' phase.
    shore: vec4<f32>,
    // x F0, y `GLINT_A2_NEAR`, z `GLINT_A2_FAR`, w `GLINT_FAR_M`.
    glint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: Water;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var sky_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var sky_sampler: sampler;

// Deep by fiat: `water::DEEP_SENTINEL_M`. Saturates every curve here.
const DEEP_M: f32 = 64.0;

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
    let deck = raw * lighting::sample_transmittance_lut(ground_r(), max(rd.y, 0.0));
#else
    let deck = raw;
#endif
    return c + deck * water.sun.w;
}

// The sun (and any other directional light) off the surface: GGX, shadowed,
// through the atmosphere's transmittance where Bevy applies it.
fn sun_glint(N: vec3<f32>, V: vec3<f32>, wp: vec3<f32>, world_normal: vec3<f32>, flags: u32, n_var: f32) -> vec3<f32> {
    let dist = length(view.world_position.xyz - wp);
    var a2 = mix(water.glint.y, water.glint.z, smoothstep(0.0, water.glint.w, dist));
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

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let N = pbr_input.N;
    let V = pbr_input.V;
    let wp = in.world_position.xyz;

    // Every derivative before any branch: naga does not hold the fragment
    // stage to uniform control flow, so a `dpdx` under an `if` is a smear,
    // not an error.
    let dndx = dpdx(N);
    let dndy = dpdy(N);
    let n_var = 0.5 * (dot(dndx, dndx) + dot(dndy, dndy));

    // 1 · The water behind this pixel, vertical metres.
#ifdef VERTEX_UVS_B
    var foam = clamp(in.uv_b.x, 0.0, 1.0);
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

    // 2 · The body, over the refracted path.
    let ior = water.scatter.w;
    let cos_i = clamp(abs(V.y), 0.0, 1.0);
    let sin2_t = (1.0 - cos_i * cos_i) / (ior * ior);
    let path = dz / sqrt(max(1.0 - sin2_t, 1e-4));
    let T = exp(-water.optics.xyz * path);
    let alpha_max = water.optics.w;
    let a_body = clamp((1.0 - (T.x + T.y + T.z) / 3.0) * alpha_max, 0.0, alpha_max);
    var body = vec4<f32>(water.scatter.xyz * (1.0 - T), a_body);

    // 3 · Foam. The vertex's wash and bores are bands along the depth
    // contours, and a band of one even white reads as a drawn streak: break
    // it into patches, in world space so a bore runs THROUGH them. The fine
    // octave settles to its mean before it is finer than a pixel.
    let dist = length(view.world_position.xyz - wp);
    let settle = smoothstep(25.0, 90.0, dist);
    let fine = mix(vnoise(wp.xz * 0.9 + vec2<f32>(5.0, 11.0)), 0.5, settle);
    let clump = vnoise(wp.xz * 0.3) * 0.6 + fine * 0.4;
    foam = clamp(foam * (0.25 + 1.3 * clump), 0.0, 1.0);
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
    var color = apply_pbr_lighting(pbr_input);

    // 5 · The reflection, from above only.
    if is_front {
        // Where the ripple normal turns faster than the pixels can follow,
        // the mirror reads it as speckle — at a grazing view one ripple swings
        // the reflected ray between the horizon and the blue above it. A real
        // pixel there averages many facets, so reflect off their mean: the
        // wave's own normal, by how fast N is changing across the pixel.
        let calm = 0.8 * smoothstep(0.002, 0.02, n_var);
        let Nr = normalize(mix(N, pbr_input.world_normal, calm));
        let NdotV = clamp(dot(Nr, V), 0.0, 1.0);
        let F0 = water.glint.x;
        let smooth_share = edge * (1.0 - foam);
        let F = (F0 + (1.0 - F0) * pow(1.0 - NdotV, 5.0)) * smooth_share;
        var R = reflect(-V, Nr);
        // A ray reflected downward would meet the next wave: read the horizon.
        R = normalize(vec3<f32>(R.x, max(R.y, 0.0) + 1e-3, R.z));
        var Rd = reflect(-V, pbr_input.world_normal);
        Rd = normalize(vec3<f32>(Rd.x, max(Rd.y, 0.0) + 1e-3, Rd.z));
        let sky = sky_radiance(R, Rd, daylight_tint()) * view.exposure;
        let glint = sun_glint(N, V, wp, pbr_input.world_normal, pbr_input.flags, n_var) * smooth_share;
        // 6 · Premultiplied: the sky replaces what Fresnel takes from below.
        color = vec4<f32>(
            color.rgb * (1.0 - F) + sky * F + glint,
            color.a + (1.0 - color.a) * F
        );
    }

    // Fog and the rest of Bevy's post-lighting work act on the straight
    // colour; the premultiply is ours (`Premultiplied` passes it through).
    let a = max(color.a, 1e-4);
    let post = main_pass_post_lighting_processing(pbr_input, vec4<f32>(color.rgb / a, color.a));
    var out: FragmentOutput;
    out.color = vec4<f32>(post.rgb * a, post.a);
    return out;
}
