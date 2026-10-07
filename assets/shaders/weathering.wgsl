// Weathering on the monuments (`render/weathering.rs`): `bevy_pbr`'s own
// fragment with the base colour stained before lighting — a world-space
// mottle, rain or rust streaks down the walls, lichen on the sides, moss on
// the tops.
//
// ⚠ Nothing in CI compiles this file. Boot the game.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

struct Weather {
    // x amplitude, y fine frequency /m, z broad frequency /m.
    mottle: vec4<f32>,
    // x strength, y frequency across the face /m.
    streak: vec4<f32>,
    // rgb what a streak multiplies by.
    streak_tint: vec4<f32>,
    // x moss on upward faces, y lichen on the sides.
    moss: vec4<f32>,
    // rgb moss albedo, linear.
    moss_tint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> weather: Weather;

// An integer hash: world coordinates run to thousands of metres, where a
// `fract(sin(·))` hash has lost its low bits.
fn hash3(c: vec3<i32>) -> f32 {
    // WGSL will not mix `^` with `*` unparenthesised.
    var h = (bitcast<u32>(c.x) * 0x8da6b343u) ^ (bitcast<u32>(c.y) * 0xd8163841u) ^ (bitcast<u32>(c.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<i32>(1, 0, 0)), u.x);
    let b = mix(hash3(i + vec3<i32>(0, 1, 0)), hash3(i + vec3<i32>(1, 1, 0)), u.x);
    let c = mix(hash3(i + vec3<i32>(0, 0, 1)), hash3(i + vec3<i32>(1, 0, 1)), u.x);
    let d = mix(hash3(i + vec3<i32>(0, 1, 1)), hash3(i + vec3<i32>(1, 1, 1)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// How much of a noise at `freq` cycles/m a pixel `px` metres across can
// carry: whole at six pixels a cycle, gone at two. Past that a term is
// speckle that crawls as the camera moves — the ziggurat at 95 m read as
// sparkling black sand before this.
fn band(px: f32, freq: f32) -> f32 {
    return clamp(1.5 - px * freq * 3.0, 0.0, 1.0);
}

// Value noise at `freq`, faded to its mean where the pixel cannot carry it.
fn vnoise_aa(p: vec3<f32>, freq: f32, px: f32) -> f32 {
    return mix(0.5, vnoise(p * freq), band(px, freq));
}

fn fbm(p: vec3<f32>, freq: f32, px: f32) -> f32 {
    return (vnoise_aa(p, freq, px) * 0.5
        + vnoise_aa(p + vec3<f32>(5.1), freq * 2.03, px) * 0.25
        + vnoise_aa(p + vec3<f32>(9.7), freq * 4.07, px) * 0.125) / 0.875;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    let p = in.world_position.xyz;
    // The pixel's footprint, metres — before any branch, as derivatives need.
    let px = max(length(dpdx(p)), length(dpdy(p)));
    // The face's own normal, turned toward the eye on a two-sided face: the
    // masks ask which way the stone faces, not which way its map bumps.
    var ng = normalize(in.world_normal);
    if !is_front {
        ng = -ng;
    }
    var c = pbr_input.material.base_color.rgb;

    // 1 · Mottle: no wall is one value.
    let fine = fbm(p, weather.mottle.y, px) - 0.5;
    let broad = vnoise_aa(p + vec3<f32>(31.0), weather.mottle.z, px) - 0.5;
    c *= max(1.0 + weather.mottle.x * (1.6 * fine + 1.2 * broad), 0.0);

    // 2 · Streaks, on the sides only: noise long down the wall and narrow
    // across it, its across-coordinate the face's own horizontal.
    let side = 1.0 - smoothstep(0.35, 0.7, abs(ng.y));
    let across = p.x * -ng.z + p.z * ng.x;
    let f = weather.streak.y;
    let s1 = mix(0.5, vnoise(vec3<f32>(across * f, p.y * 0.16, across * 0.11)), band(px, f));
    let s2 = mix(0.5, vnoise(vec3<f32>(across * f * 2.7 + 3.0, p.y * 0.35, 1.7)), band(px, f * 2.7));
    // A faded streak field sits at its mean, under the threshold: a far wall
    // keeps its value and loses the stripes rather than greying evenly.
    let streak = smoothstep(0.38, 0.78, s1 * 0.7 + s2 * 0.3) * side * weather.streak.x;
    c = mix(c, c * weather.streak_tint.rgb, streak);

    // 3 · Lichen on the sides: crusts a hand to a forearm across, pale
    // yellow-green over the stone's own value rather than paint over it.
    let crust = smoothstep(0.56, 0.72, fbm(p + vec3<f32>(71.0), 1.6, px));
    let lichen = crust * side * weather.moss.y;
    c = mix(c, c * vec3<f32>(0.86, 0.95, 0.62) + vec3<f32>(0.012, 0.016, 0.004), lichen);

    // 4 · Moss on what faces up, in patches. Flat tops only: a block's 45°
    // chamfer is a strip a pixel wide at range, and moss on every one of them
    // sparkled across the black stone like sand.
    let up = smoothstep(0.82, 0.96, ng.y);
    let patches = smoothstep(0.42, 0.66, fbm(p + vec3<f32>(13.0), 0.6, px));
    let moss = clamp(up * patches * weather.moss.x, 0.0, 1.0);
    let grain = 0.6 + 0.8 * vnoise_aa(p, 9.0, px);
    c = mix(c, weather.moss_tint.rgb * grain, moss);

    pbr_input.material.base_color = vec4<f32>(c, pbr_input.material.base_color.a);
    pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 1.0, moss * 0.7);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
