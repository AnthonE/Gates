// The far treeline's card in the depth prepass (`render/far_trees.rs`): the
// same turn to the camera as `tree_card.wgsl`, so the card lands in the depth
// buffer where the main pass draws it.
//
// **Why the prepass at all.** With a depth prepass on the camera, the main
// pass writes no depth for opaque and masked meshes — the prepass already
// did. A card kept out of it wrote depth nowhere, so the sky pass fogged it
// at the distance of whatever terrain stood behind it (a see-through ghost of
// a tree) and cards could not even hide each other.
#import bevy_pbr::{
    mesh_functions,
    prepass_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> card: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var ring_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var ring_mask_sampler: sampler;

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(in.position, 1.0)).xyz;

    let chunk = vec2<i32>(floor(wp.xz / card.x));
    let side = i32(card.y);
    var hidden = false;
    if card.z < 0.5 && chunk.x >= 0 && chunk.y >= 0 && chunk.x < side && chunk.y < side {
        hidden = textureLoad(ring_mask, chunk, 0).a > 0.5;
    }

    let to_cam = view.world_position.xyz - wp;
    if card.z < 0.5 {
        let d = length(to_cam.xz);
        let keep = clamp((1800.0 * 1800.0) / max(d * d, 1.0), 0.3, 1.0);
        let h = fract(sin(dot(floor(wp.xz * 4.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
        if h > keep {
            hidden = true;
        }
    }
    var flat = vec2<f32>(to_cam.x, to_cam.z);
    let len = length(flat);
    if len > 1e-4 {
        flat = flat / len;
    } else {
        flat = vec2<f32>(0.0, 1.0);
    }
    let facing = vec3<f32>(flat.x, 0.0, flat.y);
    let right = vec3<f32>(flat.y, 0.0, -flat.x);
#ifdef VERTEX_UVS_B
    let side_m = in.uv_b.x;
    let s = in.uv_b.y;
#else
    let side_m = 0.0;
    let s = 0.0;
#endif
    let scale = length(world_from_local[0].xyz);
    wp = wp + right * side_m * scale;

    out.world_position = vec4<f32>(wp, 1.0);
    out.position = position_world_to_clip(wp);
    if hidden {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = in.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = in.uv_b;
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = normalize(facing * sqrt(max(1.0 - s * s, 0.0)) * 0.8 + right * s * 0.8 + vec3<f32>(0.0, 0.45, 0.0));
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(right, 1.0);
#endif
#endif
#ifdef VERTEX_COLORS
    out.color = in.color;
#endif
#ifdef MOTION_VECTOR_PREPASS
    // A card stands still; only the camera moves it.
    out.previous_world_position = out.world_position;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, world_from_local[3]);
#endif
    return out;
}
