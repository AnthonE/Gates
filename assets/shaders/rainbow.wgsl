// Rainbows (`rainbow.rs`): sunlight turned back by the rain, in rings round
// the point opposite the sun. One fullscreen triangle at the far plane in
// the TRANSPARENT pass, so it lands on sky pixels only, after the deck, the
// stars and the moon (the shower is in front of all of them) and before the
// sky's fog, which hides a bow as it hides the land behind the rain. It adds
// light and covers nothing, but for the little Alexander's band takes away.
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::utils::coords_to_viewport_uv
#import "shaders/deck.wgsl"::deck_fog

// Field for field `rainbow::RainbowParams`. Angles are radians from the
// antisolar point.
struct Rainbow {
    // xyz toward the antisolar point, unit; w how strongly the bow shows
    // (0: none, and the shader does nothing).
    anti: vec4<f32>,
    // xyz a ring's light at its peak, cd/m², warmed by a low sun; w how much
    // Alexander's band darkens what is behind it.
    light: vec4<f32>,
    // The rings' radii: x the primary's violet, y its red; z the
    // secondary's violet, w its red (the order reversed: red inside).
    radii: vec4<f32>,
    // x a ring's spread toward its dark side, y toward its bright side; z
    // the secondary's spread over the primary's; w its brightness.
    spread: vec4<f32>,
    // x the supernumerary's brightness, y how far inside its ring it sits,
    // z its spread; w the glow inside the primary.
    extra: vec4<f32>,
    // x, y the angles between which any ring reaches; z how far in from
    // the violet the glow takes to arrive; w the dark band's soft edge.
    span: vec4<f32>,
    // `sky::SkyFog::lanes` where the bow fogs itself (WebGL2); zero where
    // the sky's fog layer draws over it.
    air: vec4<f32>,
    // Violet to red, each ring's share of the sun's white; w unused.
    bands: array<vec4<f32>, 7>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> bow: Rainbow;

const RINGS: u32 = 7u;

struct VertexIn {
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

// One ring's light `x` toward its bright side: a caustic's edge on the dark
// side, blurred by the sun's disk, and a slower fall into the bright one.
fn lobe(x: f32, dark: f32, bright: f32) -> f32 {
    let u = x / select(dark, bright, x > 0.0);
    return exp(-u * u);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let k = bow.anti.w;
    if k <= 0.0 {
        return vec4(0.0);
    }
    let ndc = coords_to_viewport_uv(in.clip.xy, view.viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    let v = view.view_from_clip * vec4(ndc, 1.0, 1.0);
    let d = normalize((view.world_from_view * vec4(v.xyz / v.w, 0.0)).xyz);
    let theta = acos(clamp(dot(d, bow.anti.xyz), -1.0, 1.0));
    let r = bow.radii;

    var rgb = vec3(0.0);
    if theta > bow.span.x && theta < bow.span.y {
        for (var i = 0u; i < RINGS; i += 1u) {
            let t = (f32(i) + 0.5) / f32(RINGS);
            // The primary: bright on the inside.
            let x1 = mix(r.x, r.y, t) - theta;
            let s = (x1 - bow.extra.y) / bow.extra.z;
            let p1 = lobe(x1, bow.spread.x, bow.spread.y) + bow.extra.x * exp(-s * s);
            // The secondary: reflected twice, so bright on the outside.
            let x2 = theta - mix(r.z, r.w, t);
            let p2 = bow.spread.w * lobe(x2, bow.spread.x * bow.spread.z, bow.spread.y * bow.spread.z);
            rgb += (p1 + p2) * bow.bands[i].rgb;
        }
    }
    // Inside the primary every drop sends a little back: the rings summed,
    // which is the sun's white.
    rgb += vec3(bow.extra.w * smoothstep(0.0, 1.0, (r.x - theta) / bow.span.z));
    // Between the bows no drop sends anything.
    let e = bow.span.w;
    let dark = clamp((theta - r.y) / e, 0.0, 1.0) * clamp((r.w - theta) / e, 0.0, 1.0);

    let a = k * (1.0 - deck_fog(d.y, bow.air));
    return vec4(rgb * bow.light.rgb * view.exposure * a, bow.light.w * dark * a);
}
