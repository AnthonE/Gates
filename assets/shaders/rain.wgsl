// Rain (weather v0): one mesh of streak quads the vertex stage keeps in a
// box around the eye. Every drop is a random home in the unit box plus the
// fall velocity times the clock, wrapped into the box around the camera — so
// the rain follows the player for free and never ends at a seam. Drops whose
// rank is over the rain's intensity collapse to nothing, which is how one
// mesh draws a drizzle and a downpour.
//
// Snow (snow v0): a drop whose own draw is under the snow share is a flake —
// a soft round billboard that drifts at a walking pace on its own wobble —
// so the sleet between rain and snow is a mix of the two.
#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
}

struct Rain {
    // xyz: fall velocity, m/s (wind included). w: streak length, m.
    fall: vec4<f32>,
    // xyz: the box's half extents, m. w: intensity 0..1.
    box_: vec4<f32>,
    // rgb: the streak's colour, already lit and exposed. w: the radius
    // around the eye kept clear (a roof overhead), m.
    look: vec4<f32>,
    // xyz: a snowflake's drift velocity, m/s (wind included). w: the share
    // of drops that fall as snow, 0..1.
    snow: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> rain: Rain;

struct VertexIn {
    // The drop's home in the unit box.
    @location(0) position: vec3<f32>,
    // The quad corner: x across (-1..1), y along (0 head .. 1 tail).
    @location(2) uv: vec2<f32>,
    // x: rank 0..1 (collapses above the intensity). y: the drop's own phase.
    // z: its snow draw (a flake while under the snow share).
    @location(5) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fade: f32,
    // 1 for a flake, 0 for a streak.
    @location(2) flake: f32,
}

const WIDTH: f32 = 0.012;
// A flake's radius, m, before its own size; how far it wanders off its
// line, m; how much brighter than a streak it is drawn — a flake is white
// and opaque where a drop is a glint of the sky behind it; and how many
// more flakes than drops one intensity is, since snow lingers in the air.
const FLAKE_R: f32 = 0.03;
const WOBBLE_M: f32 = 0.35;
const FLAKE_GAIN: f32 = 2.0;
const FLAKE_DENSITY: f32 = 1.5;

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let cam = view.world_position;
    let half = rain.box_.xyz;
    let size = half * 2.0;
    // Each drop runs on its own phase so a sheet of rain does not fall in
    // step, and its speed varies a little around the fall.
    let speed = 0.85 + 0.3 * in.color.y;
    let t = globals.time + in.color.y * 17.0;
    let flake = in.color.z < rain.snow.w;
    var p = in.position * size + select(rain.fall.xyz, rain.snow.xyz, flake) * speed * t;
    if flake {
        // A flake wanders off its line on two slow swings of its own.
        let ph = in.color.y * 40.0;
        p = p + vec3(sin(t * 1.3 + ph), 0.0, cos(t * 0.9 + ph * 1.7)) * WOBBLE_M;
    }
    let rel = p - cam;
    let wrapped = rel - size * floor(rel / size + 0.5);
    let head = cam + wrapped;

    let dir = normalize(rain.fall.xyz);
    let tail = head - dir * rain.fall.w;
    var along = mix(head, tail, in.uv.y);
    let to_cam = normalize(cam - along);
    var side = cross(dir, to_cam);
    let sl = length(side);
    side = select(vec3(WIDTH * 0.5, 0.0, 0.0), side / max(sl, 1e-4) * WIDTH * 0.5, sl > 1e-4);
    if flake {
        // A flake is a square facing the camera around its centre, the
        // quad's along-axis turned into the camera's up.
        let r = FLAKE_R * (0.6 + 0.8 * fract(in.color.y * 13.7));
        along = head + view.world_from_view[1].xyz * ((in.uv.y * 2.0 - 1.0) * r);
        side = view.world_from_view[0].xyz * r;
    }

    // Alive: under the intensity, outside the sheltered radius, and faded
    // toward the box's walls so the wrap is never seen. A flake also fades
    // out at arm's length, where it would be a grey disc over the view.
    let amount = select(rain.box_.w, min(rain.box_.w * FLAKE_DENSITY, 1.0), flake);
    let alive = select(0.0, 1.0, in.color.x < amount);
    let flat_d = length(wrapped.xz);
    let clear = rain.look.w;
    let open = select(1.0, smoothstep(clear, clear + 1.5, flat_d), clear > 0.0);
    let edge = max(max(abs(wrapped.x) / half.x, abs(wrapped.z) / half.z), abs(wrapped.y) / half.y);
    let near = select(1.0, smoothstep(0.4, 1.2, length(wrapped)), flake);
    let fade = alive * open * near * (1.0 - smoothstep(0.7, 1.0, edge));

    var world = along + side * in.uv.x;
    if fade <= 0.0 {
        world = head;
    }
    out.clip = position_world_to_clip(world);
    out.uv = in.uv;
    out.fade = fade;
    out.flake = select(0.0, 1.0, flake);
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    if in.flake > 0.5 {
        // A soft round flake: solid at its heart, gone at the quad's edge.
        let d = length(vec2(in.uv.x, in.uv.y * 2.0 - 1.0));
        let a = in.fade * (1.0 - smoothstep(0.3, 1.0, d)) * 0.85;
        return vec4(rain.look.rgb * FLAKE_GAIN, a);
    }
    let across = 1.0 - abs(in.uv.x);
    let a = in.fade * across * (1.0 - in.uv.y * 0.75) * 0.4;
    return vec4(rain.look.rgb, a);
}
