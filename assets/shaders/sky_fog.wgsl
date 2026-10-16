// The weather's fog over the sky (`deck.rs` `SkyFogMaterial`), wherever the
// atmosphere draws. One fullscreen triangle at the far plane in the
// TRANSPARENT pass, so it lands on sky pixels only and after
// `AtmosphereNode::RenderSky`: fog lies between the eye and the whole sky,
// the atmosphere's own light included. Folded into the deck instead, it was
// composited THROUGH the air column like a cloud is, and a fogged sky came
// out tan with a red band where the horizon should have been white.
//
// The colour and the amount are the ground's: `DistanceFog` mixes geometry
// toward its colour in exposed units, and so does this, by
// `sky::SkyFog::band`, which at the horizon is the far land's own fog.
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::utils::coords_to_viewport_uv
#import "shaders/deck.wgsl"::deck_haze

struct SkyFog {
    // xyz the ground fog's colour (`DistanceFog::color`); w unused.
    color: vec4<f32>,
    // `sky::SkyFog::lanes`.
    air: vec4<f32>,
    // xyz `DistanceFog::directional_light_color` times its alpha; w its
    // exponent.
    glow: vec4<f32>,
    // `sky::SkyFog::dust_lanes`.
    dust: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> fog: SkyFog;

struct VertexIn {
    @location(0) position: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4(in.position.xy, 0.0, 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let ndc = coords_to_viewport_uv(in.clip.xy, view.viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    let v = view.view_from_clip * vec4(ndc, 1.0, 1.0);
    let d = normalize((view.world_from_view * vec4(v.xyz / v.w, 0.0)).xyz);
    let band = deck_haze(d.y, fog.air, fog.dust);
    // The glow toward each directional light, as Bevy's `apply_fog` lays it
    // on the ground's fog (no shadow: nothing shades the sky), so the two
    // still meet toward the sun and toward a lightning bolt.
    var scattering = vec3(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i += 1u) {
        let l = lights.directional_lights[i];
        scattering += pow(max(dot(d, l.direction_to_light), 0.0), fog.glow.w) * l.color.rgb * view.exposure;
    }
    let c = fog.color.rgb + scattering * fog.glow.rgb;
    return vec4(c * band, band);
}
