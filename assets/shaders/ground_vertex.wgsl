// Terrain-only vertex stage. Bevy still owns every transform and PBR varying;
// this adds road interpolants without stealing UVs or splat weights.
#import bevy_pbr::{
    mesh_functions,
    forward_io::Vertex,
    view_transformations::position_world_to_clip,
}

struct GroundVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(4) world_tangent: vec4<f32>,
    @location(5) color: vec4<f32>,
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    @location(6) @interpolate(flat) instance_index: u32,
#endif
#ifdef VISIBILITY_RANGE_DITHER
    @location(7) @interpolate(flat) visibility_range_dither: i32,
#endif
    @location(8) road: vec2<f32>,
    @location(9) markings: vec4<f32>,
}

@vertex
fn vertex(in: Vertex, @location(8) road: vec2<f32>, @location(9) markings: vec4<f32>) -> GroundVertexOutput {
    var out: GroundVertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4(in.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local, in.tangent, in.instance_index);
    out.uv = in.uv;
    out.uv_b = in.uv_b;
    out.color = in.color;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, world_from_local[3]);
#endif
    out.road = road;
    out.markings = markings;
    return out;
}
