// The cloud deck's shadows on the land (`render/cloud_shadow.rs`), for the
// ground's fragment (`ground_splat.wgsl`, which imports this only under
// `CLOUD_SHADOWS`: natively, where the bind group carries the field).
//
// The rig dims the one sun by the deck over its disk as the EYE sees it, so
// this is relative to the eye: the sun's direct term at a point once more,
// times how much more or less of the sun reaches the point than the eye.
// Nothing at the eye, so the ground under a prop stays in the prop's light;
// a far hill goes dark and light as the deck's shadows cross it. Bevy's own
// `directional_light` and shadow fetch, so the term is the sun's exactly —
// its colour through the atmosphere, its specular, its shadows (a slope the
// shadow map darkens is not darkened twice).

#import bevy_pbr::{
    mesh_view_bindings::{view, globals, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::PbrInput,
    pbr_functions::{calculate_diffuse_color, calculate_F0},
    lighting,
    lighting::LAYER_BASE,
    shadows::fetch_directional_shadow,
}

// Field for field `cloud_shadow::CloudShadowParams`.
struct CloudShadow {
    // x 1 where the shadows are drawn, 0 off; y the weather's floor under the
    // deck's cover (a heavy sky, fog); z the field threshold; w the edge.
    cover: vec4<f32>,
    // xy the field's offset at `sun.w`, noise units; zw how fast it moves,
    // noise units a second.
    drift: vec4<f32>,
    // xyz toward the sun; w the `globals.time` the drift was taken at.
    sun: vec4<f32>,
    // x the deck over the eye, m; y noise units a metre; z one over the
    // field's period, noise units; w the field's texels a side.
    deck: vec4<f32>,
    // x the horizon cutoff; y the fade above it; z the share of the sun a
    // full deck takes; w how far a texel's centre sits from its noise
    // coordinate, texels.
    fade: vec4<f32>,
}

// The deck's coverage field at the shadows' softness (`sky::SoftField`): one
// mip of the field the sky, the stars and the moon read, eight bits a texel
// packed four to a word, row by row — the very bytes the rig's `sun_cover`
// reads on the CPU. A buffer, not a texture: the stage's sampled textures
// are spent (`ground_splat.rs`).
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var<storage, read> cloud_field: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<uniform> cloud: CloudShadow;

// Texel `i` of the field, wrapping, `0..=1`.
fn field_texel(i: vec2<i32>, n: i32) -> f32 {
    let w = ((i % n) + n) % n;
    let k = u32(w.y * n + w.x);
    return f32((cloud_field[k >> 2u] >> ((k & 3u) * 8u)) & 0xffu) / 255.0;
}

// The field at `q`, noise units, bilinear by hand, as `SoftField::at` reads
// it: texel `i` of level 0 is noise coordinate `i · period / N`
// (`CloudField::sample`); a texel of level L is the box over 2^L of those,
// so its centre sits `fade.w` of its own texel on.
fn field_at(q: vec2<f32>) -> f32 {
    let n = i32(cloud.deck.w);
    let u = q * cloud.deck.z * cloud.deck.w - vec2(cloud.fade.w);
    let i = vec2<i32>(floor(u));
    let f = u - floor(u);
    let a = field_texel(i, n);
    let b = field_texel(i + vec2(1, 0), n);
    let c = field_texel(i + vec2(0, 1), n);
    let d = field_texel(i + vec2(1, 1), n);
    let top = a + (b - a) * f.x;
    let bot = c + (d - c) * f.x;
    return top + (bot - top) * f.y;
}

// How much of the sun reaches `p` (world, m), as the rig counts it at the
// eye (`rig::day_night`): `1 − 0.9 ×` the deck over the sun's disk, or the
// weather's floor, whichever is more. The deck rides the eye (`cloud.wgsl`'s
// `march`), so `p` reads it at its offset from the eye along the ray to the
// sun, and the eye reads what `SkyComposer::cover_at` reads.
fn sun_through(p: vec3<f32>, eye: vec3<f32>, l: vec3<f32>, drift: vec2<f32>, fade: f32) -> f32 {
    let up = max(cloud.deck.x - (p.y - eye.y), 0.0);
    let q = (p.xz - eye.xz + l.xz * (up / max(l.y, cloud.fade.x))) * cloud.deck.y + drift;
    let cover = clamp((field_at(q) - cloud.cover.z) / cloud.cover.w, 0.0, 1.0) * fade;
    return 1.0 - cloud.fade.z * max(cover, cloud.cover.y);
}

// The light to add to `apply_pbr_lighting`'s for the deck's shadows, before
// exposure: zero where the point and the eye see the sun alike.
fn cloud_shadow(in: PbrInput) -> vec3<f32> {
    if cloud.cover.x <= 0.0 {
        return vec3(0.0);
    }
    // Which directional light is the sun: the one pointing where the rig's
    // sun is. Bevy sorts the shadowless first, so the lightning's light is
    // ahead of the sun's whenever the sun casts.
    let count = lights.n_directional_lights;
    var sun = count;
    var best = 0.95;
    for (var i = 0u; i < count; i += 1u) {
        let c = dot(lights.directional_lights[i].direction_to_light, cloud.sun.xyz);
        if c > best {
            best = c;
            sun = i;
        }
    }
    if sun == count {
        return vec3(0.0);
    }
    // This frame's sun, off the light: the lanes are written rarely.
    let l = lights.directional_lights[sun].direction_to_light;
    let fade = clamp((l.y - cloud.fade.x) / cloud.fade.y, 0.0, 1.0);
    if fade <= 0.0 {
        return vec3(0.0);
    }
    let drift = cloud.drift.xy + cloud.drift.zw * (globals.time - cloud.sun.w);
    let eye = view.world_position;
    let here = sun_through(in.world_position.xyz, eye, l, drift, fade);
    let there = sun_through(eye, eye, l, drift, fade);
    // Under a step of the field's eight bits: nothing anyone sees, and the
    // second look at the sun below is not free.
    if abs(here - there) < 1.0 / 512.0 {
        return vec3(0.0);
    }

    // The sun on this point as `apply_pbr_lighting` lights it.
    let base = in.material.base_color.rgb;
    let metallic = in.material.metallic;
    let perceptual_roughness = in.material.perceptual_roughness;
    let n_dot_v = max(dot(in.N, in.V), 0.0001);
    var li: lighting::LightingInput;
    li.layers[LAYER_BASE].NdotV = n_dot_v;
    li.layers[LAYER_BASE].N = in.N;
    li.layers[LAYER_BASE].R = reflect(-in.V, in.N);
    li.layers[LAYER_BASE].perceptual_roughness = perceptual_roughness;
    li.layers[LAYER_BASE].roughness = lighting::perceptualRoughnessToRoughness(perceptual_roughness);
    li.P = in.world_position.xyz;
    li.V = in.V;
    li.diffuse_color = calculate_diffuse_color(
        base,
        metallic,
        in.material.specular_transmission,
        in.material.diffuse_transmission,
    );
    li.F0_ = calculate_F0(base, metallic, in.material.reflectance);
    li.F_ab = lighting::F_AB(perceptual_roughness, n_dot_v);
    var shadow = 1.0;
    if (in.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && (lights.directional_lights[sun].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
        let view_z = dot(vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z
        ), in.world_position);
        shadow = fetch_directional_shadow(sun, in.world_position, in.world_normal, view_z);
    }
    // The rig lit this point as it lit the eye; this is the difference.
    return lighting::directional_light(sun, &li, true) * shadow * (here / there - 1.0);
}
