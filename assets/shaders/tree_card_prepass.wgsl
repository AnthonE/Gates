// The far treeline's card in the depth prepass (`render/far_trees.rs`): the
// same turn to the camera as `tree_card.wgsl`, so the card lands in the depth
// buffer where the main pass draws it, and the same atlas read
// (`alpha_cut`), so it lands only where the main pass draws it.
//
// **Why the prepass at all.** With a depth prepass on the camera, the main
// pass writes no depth for opaque and masked meshes — the prepass already
// did. A card kept out of it wrote depth nowhere, so the sky pass fogged it
// at the distance of whatever terrain stood behind it (a see-through ghost of
// a tree) and cards could not even hide each other.
#import bevy_pbr::{
    mesh_functions,
    prepass_io,
    prepass_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
    pbr_bindings,
    pbr_functions,
    pbr_prepass_functions,
    pbr_types,
}
#import "shaders/alpha_cut.wgsl"::{cut_read, cut_uv}

// `far_trees::CardParams`. params: x = prop chunk edge (m), y = 1 on a ring
// card (its entity is the ring's own tree, so it ignores the mask). window:
// the mask's first chunk and its side; bits: one per window chunk, set where
// a prop ring draws the trees.
struct CardParams {
    params: vec4<f32>,
    window: vec4<i32>,
    bits: vec4<u32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> card: CardParams;

fn ring_holds(wp: vec3<f32>) -> bool {
    let local = vec2<i32>(floor(wp.xz / card.params.x)) - card.window.xy;
    let side = card.window.z;
    if local.x < 0 || local.y < 0 || local.x >= side || local.y >= side {
        return false;
    }
    let i = u32(local.y * side + local.x);
    var word = card.bits.x;
    if i >= 96u {
        word = card.bits.w;
    } else if i >= 64u {
        word = card.bits.z;
    } else if i >= 32u {
        word = card.bits.y;
    }
    return ((word >> (i % 32u)) & 1u) != 0u;
}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(in.position, 1.0)).xyz;

    let island = card.params.y < 0.5;
    var hidden = island && ring_holds(wp);

    let to_cam = view.world_position.xyz - wp;
    if island {
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

// The atlas UV, through the material's transform.
fn card_uv(in: VertexOutput) -> vec2<f32> {
#ifdef VERTEX_UVS
    return cut_uv(in.uv);
#else
    return vec2<f32>(0.0);
#endif
}

// The masked card's cut: the base colour exactly as `tree_card.wgsl` builds
// it, read through `alpha_cut` at the gradients the caller took first thing
// (`duv` of `uv`).
fn card_cut(in: VertexOutput, uv: vec2<f32>, duv: vec4<f32>) {
#ifdef MAY_DISCARD
    var c = pbr_bindings::material.base_color;
#ifdef VERTEX_COLORS
    c *= in.color;
#endif
#ifdef VERTEX_UVS
    if (pbr_bindings::material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        c *= cut_read(uv, duv);
    }
#endif
    if c.a < pbr_bindings::material.alpha_cutoff {
        discard;
    }
#endif
}

// `bevy_pbr`'s `pbr_prepass.wgsl` fragment (forward, non-bindless,
// non-meshlet, and no normal map: the card has none) with `card_cut` for its
// alpha test. The crossfade cuts between the gradients and the read: the read
// needs no live neighbour and nothing else here takes a derivative, so a
// pixel the crossfade removes skips it.
#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
    let uv = card_uv(in);
    let duv = vec4(dpdx(uv), dpdy(uv));
#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    card_cut(in, uv, duv);

    var out: prepass_io::FragmentOutput;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif
#ifdef NORMAL_PREPASS
    let flags = pbr_bindings::material.flags;
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
        let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_front);
        out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
    } else {
        out.normal = vec4(in.world_normal * 0.5 + vec3(0.5), 1.0);
    }
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
    return out;
}
#else
@fragment
fn fragment(in: VertexOutput) {
    let uv = card_uv(in);
    let duv = vec4(dpdx(uv), dpdy(uv));
#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    card_cut(in, uv, duv);
}
#endif
