// Foliage, main pass (`render/foliage.rs`): `StandardMaterial` with a vertex
// stage that moves the plant (wind, trails, distance fade) and a fragment
// stage that is `bevy_pbr`'s own plus one cut — cards seen edge-on fade out.
// The prepass twin is `foliage_prepass.wgsl`; both call `foliage_common`.
#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif
#import "shaders/foliage_common.wgsl"::{displace, grazing_keep, local_height, origin_rand}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let wp0 = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0)).xyz;
    let origin = world_from_local[3].xyz;
#ifdef VERTEX_UVS_B
    // Baked (a grass tile): UV1 carries the height above the card's root and
    // the card's own random number.
    let h = in.uv_b.x;
    let rand = in.uv_b.y;
    let anchor = wp0;
#else
    // A plant in its own frame: height off the local y, one phase per plant.
    let h = local_height(in.position.y, length(world_from_local[1].xyz));
    let rand = origin_rand(origin);
    let anchor = origin;
#endif
    let wp = wp0 + displace(wp0, anchor, h, rand);

    out.world_position = vec4<f32>(wp, 1.0);
    out.position = position_world_to_clip(wp);
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
#endif
#ifdef VERTEX_UVS_A
    out.uv = in.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = in.uv_b;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        in.tangent,
        in.instance_index
    );
#endif
#ifdef VERTEX_COLORS
    out.color = in.color;
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

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var in = vertex_output;
    // First, while every invocation is still live: it takes derivatives.
    let keep = grazing_keep(in.world_position.xyz);

#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // An opaque material ignores alpha in `alpha_discard`, so the bark that
    // shares this shader is never cut.
    pbr_input.material.base_color.a *= keep;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
