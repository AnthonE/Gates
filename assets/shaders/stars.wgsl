// Stars (weather v0): one mesh of tiny quads, each a star's direction. The
// vertex stage puts a star far out along it and sizes its quad in pixels, so
// it is a point at any resolution; the fragment stage hides it behind the
// cloud deck by sampling the deck's own noise field where the star's ray
// meets the deck (`sky.rs`'s `SkyComposer::cover_at`, on the GPU).
#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
}

struct Stars {
    // x: how much of the starfield shows. y: a quad's size, pixels.
    look: vec4<f32>,
    // x: the deck's altitude over its noise scale. y: the horizon cutoff
    // (a ray's y). z: the fade above the cutoff.
    deck: vec4<f32>,
    // xy: the deck's drift, noise units. z: the field threshold. w: the edge.
    cloud: vec4<f32>,
    // x: one over the field's period, noise units. y: half a texel, uv.
    field: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> stars: Stars;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var field_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var field_samp: sampler;

struct VertexIn {
    // The star's direction, unit, above the horizon.
    @location(0) position: vec3<f32>,
    // The quad corner, -1..1 both ways.
    @location(2) uv: vec2<f32>,
    // x: brightness. y: twinkle phase. z: warmth.
    @location(5) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) dir: vec3<f32>,
    @location(2) light: vec3<f32>,
}

// Past any land the ring draws; the projection is infinite reverse-Z, so the
// depth test still puts every hill in front.
const DIST: f32 = 20000.0;

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let d = in.position;
    var clip = position_world_to_clip(view.world_position + d * DIST);
    let px = stars.look.y * (0.6 + 0.4 * in.color.x);
    clip = vec4(clip.xy + in.uv * px / view.viewport.zw * clip.w, clip.z, clip.w);
    // Low stars sit in thicker air: dimmer, and twinkling harder.
    let tw = 1.0 + 0.3 * (1.2 - d.y) * sin(globals.time * (1.3 + 3.0 * in.color.y) + in.color.y * 40.0);
    let low = smoothstep(0.02, 0.22, d.y);
    let rgb = mix(vec3(0.78, 0.86, 1.0), vec3(1.0, 0.9, 0.74), in.color.z);
    out.clip = clip;
    out.uv = in.uv;
    out.dir = d;
    out.light = rgb * in.color.x * tw * low * stars.look.x;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let spot = max(1.0 - dot(in.uv, in.uv), 0.0);
    // Behind the deck: where this star's ray meets it, is there cloud?
    var clear = 1.0;
    let d = normalize(in.dir);
    if d.y > stars.deck.y {
        let q = d.xz * (stars.deck.x / d.y) + stars.cloud.xy;
        let f = textureSampleLevel(field_tex, field_samp, q * stars.field.x + stars.field.y, 0.0).r;
        let fade = clamp((d.y - stars.deck.y) / stars.deck.z, 0.0, 1.0);
        clear = 1.0 - clamp((f - stars.cloud.z) / stars.cloud.w, 0.0, 1.0) * fade;
    }
    // Alpha zero: `AlphaMode::Add` blends premultiplied (`One,
    // OneMinusSrcAlpha`), so any alpha here would erase the sky behind the
    // quad — black corners round every star, black specks in the cloud.
    return vec4(in.light * spot * clear, 0.0);
}
