// A browser's sky, per pixel (`deck.rs`): `sky::compose_range`'s browser
// branch on the GPU — the clear sky, the moon's halo, the cloud deck and
// the fog band — so a cloud's edge is sharp at any resolution instead of
// a 128² cube face magnified six times. One fullscreen triangle at the far
// plane; land and sea are in front of it by depth.
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::utils::coords_to_viewport_uv
#import "shaders/deck.wgsl"::{deck_point, deck_fade, deck_detail}

// Field for field `deck::DeckParams`. Colours are linear, in deck units
// (1.0 is one `sky::CLOUD_NITS`), already graded by the hour and weather.
struct Deck {
    // xyz the clear zenith; w `sky::AIR_FLOOR`.
    zenith: vec4<f32>,
    // xyz the clear horizon; w cd/m² per deck unit (the `Skybox`'s).
    horizon: vec4<f32>,
    // xyz the night floor; w how far into the night.
    night: vec4<f32>,
    // xyz the dusk glow at its strongest; w unused.
    glow: vec4<f32>,
    // xy toward the sun, horizontal, unit; zw the light march's step.
    sun: vec4<f32>,
    // xyz toward the moon; w the halo's cosine.
    moon: vec4<f32>,
    // xyz the halo at its centre; w the moonlit scatter's cosine.
    halo: vec4<f32>,
    // xyz the moonlit scatter at the moon; w unused.
    scatter: vec4<f32>,
    // xyz a lit top; w the field threshold.
    top: vec4<f32>,
    // xyz a grey base; w the edge's width.
    base: vec4<f32>,
    // xyz the fog's colour; w the fog band's strength.
    fog: vec4<f32>,
    // x altitude over noise scale; y the horizon cutoff; z the fade above
    // it; w the fine detail's amplitude.
    shape: vec4<f32>,
    // xy the drift, noise units; z one over the field's period; w half a
    // texel, uv.
    drift: vec4<f32>,
    // x `CORE_SHADE`; y `RIM_GLOW`; z `SIDE_VIEW_Y`; w unused.
    shade: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> deck: Deck;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var field_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var field_samp: sampler;

struct VertexIn {
    // Clip-space x and y of the fullscreen triangle.
    @location(0) position: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Depth 0 is infinitely far under reverse-Z: drawn only where nothing is.
    out.clip = vec4(in.position.xy, 0.0, 1.0);
    return out;
}

// The pixel's world ray, as `skybox.wgsl` builds it (off the near plane,
// since the far one is at infinity).
fn ray(position: vec2<f32>) -> vec3<f32> {
    let ndc = coords_to_viewport_uv(position, view.viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    let v = view.view_from_clip * vec4(ndc, 1.0, 1.0);
    return normalize((view.world_from_view * vec4(v.xyz / v.w, 0.0)).xyz);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let d = ray(in.clip.xy);

    // Where the ray meets the deck, and everything that needs a derivative,
    // before any branch.
    let q = deck_point(d, deck.shape.x, deck.shape.y) + deck.drift.xy;
    let qs = q + deck.sun.zw;
    let uv = q * deck.drift.z + deck.drift.w;
    let gx = dpdx(uv);
    let gy = dpdy(uv);
    let foot = max(length(dpdx(q)), length(dpdy(q)));
    let f0 = textureSampleGrad(field_tex, field_samp, uv, gx, gy).r;
    let fs0 = textureSampleGrad(field_tex, field_samp, qs * deck.drift.z + deck.drift.w, gx, gy).r;
    let f = f0 + deck.shape.w * deck_detail(q, foot);
    let f_sun = fs0 + deck.shape.w * deck_detail(qs, foot);

    // The clear sky: zenith to horizon along the air-mass curve.
    let af = deck.zenith.w;
    let m = 1.0 / (clamp(d.y, 0.0, 1.0) + af);
    let w_air = clamp((m - 1.0 / (1.0 + af)) / (1.0 / af - 1.0 / (1.0 + af)), 0.0, 1.0);
    var sky = mix(deck.zenith.rgb, deck.horizon.rgb, w_air) + deck.night.rgb;
    if d.y > -0.1 {
        let h = d.xz / max(length(d.xz), 1e-6);
        let facing = max(dot(h, deck.sun.xy), 0.0);
        let low = max(1.0 - max(d.y, 0.0) / 0.4, 0.0);
        sky += deck.glow.rgb * facing * facing * facing * low * low;
    }
    // The moon's halo; its disk is `moon.wgsl`'s.
    let cm = dot(d, deck.moon.xyz);
    if d.y > 0.0 && cm > deck.moon.w {
        let k = (cm - deck.moon.w) / (1.0 - deck.moon.w);
        sky += deck.halo.rgb * k * k;
    }

    // The cloud over it.
    let thresh = deck.top.w;
    let edge = deck.base.w;
    let c0 = clamp((f - thresh) / edge, 0.0, 1.0);
    let cov = c0 * deck_fade(d.y, deck.shape.y, deck.shape.z);
    // Lit top vs grey base: thinner toward the sun is a lit face.
    let lit = clamp((f - f_sun) * 6.0 + 0.5, 0.0, 1.0);
    let thick = clamp((f - thresh) / (3.0 * edge), 0.0, 1.0);
    let side = clamp((deck.shade.z - d.y) / (deck.shade.z - deck.shape.y), 0.0, 1.0);
    let under = deck.base.rgb * (1.0 - deck.shade.x * thick)
        + (deck.top.rgb - deck.base.rgb) * deck.shade.y * (1.0 - c0);
    let w = max(lit * (1.0 - 0.5 * thick), side * side * (3.0 - 2.0 * side));
    var cloud = mix(under, deck.top.rgb, w);
    let s = max((cm - deck.halo.w) / (1.0 - deck.halo.w), 0.0);
    cloud += deck.scatter.rgb * s * s * (1.0 - 0.6 * thick);
    var rgb = mix(sky, cloud, cov);

    // Fog lies between the eye and everything: a band at the horizon.
    let band = deck.fog.w * clamp(1.0 - (d.y + 0.02) / 0.25, 0.0, 1.0);
    rgb = mix(rgb, deck.fog.rgb, band);

    return vec4(rgb * deck.horizon.w * view.exposure, 1.0);
}
