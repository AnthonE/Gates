// Foliage in the depth/normal prepass and the shadow pass
// (`render/foliage.rs`). The same `displace` as `foliage.wgsl`, so a card
// lands in the depth buffer where the main pass draws it, and the same
// edge-on cut, so the prepass writes no depth where the main pass discards.
//
// The fragment stage is `bevy_pbr`'s `pbr_prepass.wgsl` (forward, non-bindless,
// non-meshlet: the only way this material is ever drawn) with the alpha test
// widened by the edge-on factor.
#import bevy_pbr::{
    mesh_functions,
    prepass_io,
    prepass_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
    pbr_bindings,
    pbr_types,
    pbr_functions,
}
#ifdef MOTION_VECTOR_PREPASS
#import bevy_pbr::pbr_prepass_functions
#endif
#import "shaders/foliage_common.wgsl"::{displace, grazing_keep, local_height, origin_rand}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let wp0 = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0)).xyz;
    let origin = world_from_local[3].xyz;
#ifdef VERTEX_UVS_B
    let h = in.uv_b.x;
    let rand = in.uv_b.y;
    let anchor = wp0;
#else
    let h = local_height(in.position.y, length(world_from_local[1].xyz));
    let rand = origin_rand(origin);
    let anchor = origin;
#endif
    let wp = wp0 + displace(wp0, anchor, h, rand);

    out.world_position = vec4<f32>(wp, 1.0);
    out.position = position_world_to_clip(wp);
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
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        in.tangent,
        in.instance_index
    );
#endif
#endif
#ifdef VERTEX_COLORS
    out.color = in.color;
#endif
#ifdef MOTION_VECTOR_PREPASS
    // Close enough for motion blur and TAA: this game runs neither.
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

// `pbr_prepass_functions::prepass_alpha_discard`, non-bindless, with the
// edge-on factor multiplied in before the test.
fn foliage_alpha_discard(in: VertexOutput, keep: f32) {
#ifdef MAY_DISCARD
    var output_color: vec4<f32> = pbr_bindings::material.base_color;
    let flags = pbr_bindings::material.flags;
#ifdef VERTEX_UVS
#ifdef STANDARD_MATERIAL_BASE_COLOR_UV_B
    var uv = in.uv_b;
#else
    var uv = in.uv;
#endif
    let uv_transform = pbr_bindings::material.uv_transform;
    uv = (uv_transform * vec3(uv, 1.0)).xy;
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        output_color = output_color * textureSampleBias(
            pbr_bindings::base_color_texture,
            pbr_bindings::base_color_sampler,
            uv,
            view.mip_bias
        );
    }
#endif
    output_color.a *= keep;
    let alpha_mode = flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_MASK {
        if output_color.a < pbr_bindings::material.alpha_cutoff {
            discard;
        }
    } else if (alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND ||
            alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_ADD ||
            alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_ALPHA_TO_COVERAGE) {
        if output_color.a < 0.05 {
            discard;
        }
    }
#endif
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: VertexOutput) -> prepass_io::FragmentOutput {
    // Every face is the front, for `foliage.wgsl`'s reason: a foliage normal
    // is its plant's volume, the same from either side.
    let is_front = true;
    let keep = grazing_keep(in.world_position.xyz);
    let flags = pbr_bindings::material.flags;

#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif

    foliage_alpha_discard(in, keep);

    var out: prepass_io::FragmentOutput;

#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif

#ifdef NORMAL_PREPASS
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
        var normal = pbr_functions::prepare_world_normal(
            in.world_normal,
            double_sided,
            is_front,
        );
#ifdef VERTEX_UVS
#ifdef VERTEX_TANGENTS
#ifdef STANDARD_MATERIAL_NORMAL_MAP
#ifdef STANDARD_MATERIAL_NORMAL_MAP_UV_B
        let uv = (pbr_bindings::material.uv_transform * vec3(in.uv_b, 1.0)).xy;
#else
        let uv = (pbr_bindings::material.uv_transform * vec3(in.uv, 1.0)).xy;
#endif
        let Nt = textureSampleBias(
            pbr_bindings::normal_map_texture,
            pbr_bindings::normal_map_sampler,
            uv,
            view.mip_bias,
        ).rgb;
        let TBN = pbr_functions::calculate_tbn_mikktspace(normal, in.world_tangent);
        normal = pbr_functions::apply_normal_mapping(
            flags,
            TBN,
            double_sided,
            is_front,
            Nt,
        );
#endif
#endif
#endif
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
    let keep = grazing_keep(in.world_position.xyz);
    foliage_alpha_discard(in, keep);
}
#endif
