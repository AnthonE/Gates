// The moon (`moon.rs`): one quad far out along the moon's direction, its
// disk cut per pixel so the rim is sharp at any resolution, its seas laid
// on it, and hidden behind the cloud deck by the deck's own field — the
// same arithmetic the sky draws that cloud with (`deck.wgsl`).
#import bevy_pbr::{
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#import "shaders/deck.wgsl"::{deck_point, deck_fade, deck_detail, deck_value}

// Field for field `moon::MoonParams`.
struct Moon {
    // xyz toward the moon, unit; w the quad's half-size, as a tangent.
    dir: vec4<f32>,
    // xyz the disk's radiance at full night, cd/m²; w how much shows.
    light: vec4<f32>,
    // x the deck's altitude over its noise scale; y the horizon cutoff;
    // z the fade above it; w the fine detail's amplitude (0: none).
    deck: vec4<f32>,
    // xy the deck's drift, noise units; z the field threshold; w the edge.
    cloud: vec4<f32>,
    // x one over the field's period; y half a texel, uv; z 1 where the disk
    // covers what is behind it, 0 where it adds; w the quad over the disk.
    field: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> moon: Moon;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var field_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var field_samp: sampler;

struct VertexIn {
    // The quad's corner, -1..1 both ways.
    @location(0) position: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // On the disk: 1 at the rim, x right and y up as the eye sees it.
    @location(0) p: vec2<f32>,
    @location(1) dir: vec3<f32>,
}

// Past any land; the projection is infinite reverse-Z (`stars.wgsl`).
const DIST: f32 = 20000.0;

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let d = moon.dir.xyz;
    // Up is the sky's up, so the seas keep their places as the moon rides.
    let c = cross(d, vec3(0.0, 1.0, 0.0));
    let right = select(vec3(1.0, 0.0, 0.0), normalize(c), dot(c, c) > 1e-6);
    let up = cross(right, d);
    let corner = in.position.xy;
    let dir = d + (right * corner.x + up * corner.y) * moon.dir.w;
    out.clip = position_world_to_clip(view.world_position + dir * DIST);
    out.p = corner * moon.field.w;
    out.dir = dir;
    return out;
}

// A sea: 1 inside an ellipse at `c` with radii `r`, its shore soft and
// ragged, so neighbouring seas run into each other.
fn mare(p: vec2<f32>, c: vec2<f32>, r: vec2<f32>, rag: f32) -> f32 {
    let e = length((p - c) / r) + rag;
    return 1.0 - smoothstep(0.35, 1.15, e);
}

// A bright young crater: a soft spot.
fn crater(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    let e = length(p - c) / r;
    return max(1.0 - e * e, 0.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.p;
    let r = length(p);
    // The rim, antialiased over one pixel.
    let px = max(fwidth(r), 1e-4);
    let a = clamp((1.0 - r) / px + 0.5, 0.0, 1.0);

    // The near side's seas, roughly where a person would look for them.
    let rag = (deck_value(p * 3.0 + vec2(3.1, 7.7)) - 0.5) * 0.5
        + (deck_value(p * 7.0 + vec2(-4.2, 1.3)) - 0.5) * 0.25;
    var m = mare(p, vec2(-0.50, 0.05), vec2(0.36, 0.60), rag);  // Procellarum
    m = max(m, mare(p, vec2(-0.24, 0.38), vec2(0.32, 0.28), rag)); // Imbrium
    m = max(m, 0.7 * mare(p, vec2(0.02, 0.70), vec2(0.48, 0.11), rag)); // Frigoris
    m = max(m, mare(p, vec2(0.18, 0.36), vec2(0.19, 0.19), rag));  // Serenitatis
    m = max(m, 0.8 * mare(p, vec2(0.02, 0.16), vec2(0.14, 0.11), rag)); // Vaporum
    m = max(m, mare(p, vec2(0.34, 0.10), vec2(0.25, 0.21), rag));  // Tranquillitatis
    m = max(m, mare(p, vec2(0.68, 0.28), vec2(0.12, 0.14), rag));  // Crisium
    m = max(m, 0.85 * mare(p, vec2(0.54, -0.15), vec2(0.15, 0.21), rag)); // Fecunditatis
    m = max(m, 0.85 * mare(p, vec2(0.32, -0.30), vec2(0.12, 0.12), rag)); // Nectaris
    m = max(m, 0.8 * mare(p, vec2(-0.22, -0.02), vec2(0.19, 0.15), rag)); // Insularum
    m = max(m, 0.85 * mare(p, vec2(-0.17, -0.38), vec2(0.21, 0.16), rag)); // Nubium
    m = max(m, mare(p, vec2(-0.48, -0.40), vec2(0.13, 0.13), rag)); // Humorum
    var albedo = 1.0 - 0.34 * m;
    albedo += 0.3 * crater(p, vec2(-0.13, -0.70), 0.05);  // Tycho
    albedo += 0.2 * crater(p, vec2(-0.33, 0.20), 0.04);   // Copernicus
    albedo += 0.2 * crater(p, vec2(-0.68, 0.33), 0.03);   // Aristarchus
    // Highland grain, while the disk is big enough on screen to hold it.
    let g1 = clamp(1.0 / (14.0 * px) - 2.0, 0.0, 1.0);
    let g2 = clamp(1.0 / (31.0 * px) - 2.0, 0.0, 1.0);
    albedo *= 1.0 + 0.10 * g1 * (deck_value(p * 14.0) - 0.5) + 0.08 * g2 * (deck_value(p * 31.0 + 5.0) - 0.5);
    // A full moon is nearly flat; only the very limb darkens.
    let z = sqrt(max(1.0 - r * r, 0.0));
    albedo *= 0.84 + 0.16 * sqrt(z);

    // Behind the deck: where this pixel's ray meets it, is there cloud?
    let d = normalize(in.dir);
    let q = deck_point(d, moon.deck.x, moon.deck.y) + moon.cloud.xy;
    let uv = q * moon.field.x + moon.field.y;
    let foot = max(length(dpdx(q)), length(dpdy(q)));
    let f0 = textureSampleGrad(field_tex, field_samp, uv, dpdx(uv), dpdy(uv)).r;
    let f = f0 + moon.deck.w * deck_detail(q, foot);
    let cov = clamp((f - moon.cloud.z) / moon.cloud.w, 0.0, 1.0) * deck_fade(d.y, moon.deck.y, moon.deck.z);

    let k = a * (1.0 - cov) * moon.light.w;
    return vec4(moon.light.rgb * albedo * k * view.exposure, k * moon.field.z);
}
