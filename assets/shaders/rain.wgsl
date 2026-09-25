// Rain (weather v0): one mesh of streak quads the vertex stage keeps in a
// box around the eye. Every drop is a random home in the unit box plus the
// fall velocity times the clock, wrapped into the box around the camera — so
// the rain follows the player for free and never ends at a seam. Drops whose
// rank is over the rain's intensity collapse to nothing, which is how one
// mesh draws a drizzle and a downpour.
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
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> rain: Rain;

struct VertexIn {
    // The drop's home in the unit box.
    @location(0) position: vec3<f32>,
    // The quad corner: x across (-1..1), y along (0 head .. 1 tail).
    @location(2) uv: vec2<f32>,
    // x: rank 0..1 (collapses above the intensity). y: the drop's own phase.
    @location(5) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fade: f32,
}

const WIDTH: f32 = 0.012;

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let cam = view.world_position;
    let half = rain.box_.xyz;
    let size = half * 2.0;
    // Each drop runs on its own phase so a sheet of rain does not fall in
    // step, and its speed varies a little around the fall.
    let speed = 0.85 + 0.3 * in.color.y;
    let p = in.position * size + rain.fall.xyz * speed * (globals.time + in.color.y * 17.0);
    let rel = p - cam;
    let wrapped = rel - size * floor(rel / size + 0.5);
    let head = cam + wrapped;

    let dir = normalize(rain.fall.xyz);
    let tail = head - dir * rain.fall.w;
    let along = mix(head, tail, in.uv.y);
    let to_cam = normalize(cam - along);
    var side = cross(dir, to_cam);
    let sl = length(side);
    side = select(vec3(WIDTH * 0.5, 0.0, 0.0), side / max(sl, 1e-4) * WIDTH * 0.5, sl > 1e-4);

    // Alive: under the intensity, outside the sheltered radius, and faded
    // toward the box's walls so the wrap is never seen.
    let alive = select(0.0, 1.0, in.color.x < rain.box_.w);
    let flat_d = length(wrapped.xz);
    let clear = rain.look.w;
    let open = select(1.0, smoothstep(clear, clear + 1.5, flat_d), clear > 0.0);
    let edge = max(max(abs(wrapped.x) / half.x, abs(wrapped.z) / half.z), abs(wrapped.y) / half.y);
    let fade = alive * open * (1.0 - smoothstep(0.7, 1.0, edge));

    var world = along + side * in.uv.x;
    if fade <= 0.0 {
        world = head;
    }
    out.clip = position_world_to_clip(world);
    out.uv = in.uv;
    out.fade = fade;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let across = 1.0 - abs(in.uv.x);
    let a = in.fade * across * (1.0 - in.uv.y * 0.75) * 0.4;
    return vec4(rain.look.rgb, a);
}
