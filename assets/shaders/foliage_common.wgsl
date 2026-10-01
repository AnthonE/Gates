// What every foliage surface does (`render/foliage.rs`): grass cards, bush
// leaves, and the near trees. Imported by `foliage.wgsl` (main pass) and
// `foliage_prepass.wgsl` (depth/normal prepass and shadows), so both passes
// move a vertex by the SAME function. A mismatch is not cosmetic: with a depth
// prepass the main pass tests against the prepass's depth, and a card the two
// place differently draws as holes.

#import bevy_pbr::mesh_view_bindings::view

struct FoliageParams {
    // x: bend amplitude, metres at the reference height
    // y: reference height, metres (a blade, a bush, a tree)
    // z: sway angular frequency, rad/s
    // w: flutter amplitude, metres at the reference height
    sway: vec4<f32>,
    // x: distance fade start, metres (0 = no fade)
    // y: distance fade end, metres
    // z: how hard cards seen edge-on are hidden (0 = never, 1 = leaves)
    // w: the mesh's root height in its own frame (local-frame meshes only)
    fade: vec4<f32>,
    // x: trample strength (0 = ignores trails)
    misc: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;
// 4×1 texels written every frame from the CPU: [0] = (wind dir x, wind dir z,
// strength, time), [1] = (trail window centre x, z, window edge m, 0).
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var foliage_state: texture_2d<f32>;
// Trails of flattened grass, world-anchored and wrapping (`foliage::Trample`).
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var trample_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var trample_sampler: sampler;

fn hash12(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

// Height above the root for a mesh drawn in its own frame (a tree, a bush).
fn local_height(local_y: f32, scale: f32) -> f32 {
    return max(local_y - foliage.fade.w, 0.0) * scale;
}

// A per-plant random number off where the plant stands.
fn origin_rand(origin: vec3<f32>) -> f32 {
    return hash12(floor(origin.xz * 8.0));
}

// The world-space offset for a vertex `h` metres above its plant's root.
// `root` is the vertex's own position before any offset; `anchor` is where
// the plant stands (its sway phase, so a whole tree swings as one); `rand` is
// per plant (or per card), so neighbours do not move in lockstep.
fn displace(root: vec3<f32>, anchor: vec3<f32>, h: f32, rand: f32) -> vec3<f32> {
    if h <= 0.0 {
        return vec3<f32>(0.0);
    }
    let s0 = textureLoad(foliage_state, vec2<i32>(0, 0), 0);
    let dir = s0.xy;
    let strength = s0.z;
    let t = s0.w;
    let perp = vec2<f32>(-dir.y, dir.x);
    let along = dot(anchor.xz, dir);

    // Gust bands travelling downwind, wavering across it.
    let gust = 0.55 + 0.45 * sin(along * 0.11 - t * 0.9 + sin(dot(anchor.xz, perp) * 0.05 + t * 0.13) * 1.5);
    let k = clamp(h / max(foliage.sway.y, 0.01), 0.0, 2.5);
    let w = foliage.sway.z;
    let osc = sin(t * w + rand * 6.2831 + along * 0.35);
    // A cantilever: the tip travels with the square of its height.
    let bend = foliage.sway.x * k * k * strength * (0.75 * gust + 0.35 * osc);
    // Flutter: fast and small, a different phase every few centimetres.
    let fphase = t * w * 3.7 + dot(root, vec3<f32>(2.1, 1.7, 2.9)) + rand * 6.2831;
    let flutter = foliage.sway.w * k * (0.3 + strength) * sin(fphase);

    var off = vec3<f32>(dir.x * bend + perp.x * flutter, flutter * 0.4, dir.y * bend + perp.y * flutter);
    // Bending a stem shortens it: drop the tip so it swings on an arc.
    off.y -= min(bend * bend / max(2.0 * h, 0.05), 0.5 * h);

    // Trails: flattened where somebody walked (grass only).
    if foliage.misc.x > 0.0 {
        let s1 = textureLoad(foliage_state, vec2<i32>(1, 0), 0);
        let rel = root.xz - s1.xy;
        let reach = s1.z * 0.5;
        let edge = clamp((reach - max(abs(rel.x), abs(rel.y))) * 0.5, 0.0, 1.0);
        let tr = textureSampleLevel(trample_map, trample_sampler, root.xz / max(s1.z, 1.0), 0.0).r
            * edge * foliage.misc.x;
        let a = rand * 6.2831;
        off = off * (1.0 - tr) + vec3<f32>(cos(a) * 0.7 * h, -0.75 * h, sin(a) * 0.7 * h) * tr;
    }

    // Distance fade: past `fade.x` each card sinks into the ground at its own
    // distance, so the grass thins out rather than ending on a line. Not in a
    // shadow cascade, whose "eye" is the sun: the near grass that casts
    // (`clutter::GRASS_SHADOW_TILES`) casts at full height.
    let ortho = view.clip_from_view[3].w == 1.0;
    if foliage.fade.y > 0.0 && !ortho {
        let d = distance(root, view.world_position);
        let span = max(foliage.fade.y - foliage.fade.x, 0.01);
        let start = foliage.fade.x + span * 0.5 * rand;
        let f = clamp((d - start) / (span * 0.5), 0.0, 1.0);
        off = off * (1.0 - f) - vec3<f32>(0.0, h * f, 0.0);
    }
    return off;
}

// How much of a card survives being seen edge-on: 1 face-on, 0 along its
// plane. Called at the TOP of the fragment stage (derivatives), and by both
// passes, so the prepass and the main pass cut the same pixels.
fn grazing_keep(world_position: vec3<f32>) -> f32 {
    let n = cross(dpdx(world_position), dpdy(world_position));
    let ortho = view.clip_from_view[3].w == 1.0;
    let v = view.world_position - world_position;
    let c = abs(dot(n, v)) / max(length(n) * length(v), 1e-8);
    let g = foliage.fade.z;
    let keep = smoothstep(0.06 * g, 0.26 * g, c);
    // Shadow cascades are orthographic: a card casts its shadow at any angle.
    if g < 0.01 || ortho {
        return 1.0;
    }
    return keep;
}
