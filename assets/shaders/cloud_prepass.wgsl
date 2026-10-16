// The cloud deck's prepass (`deck.rs` `CloudMaterial`): what the `Skybox`'s
// own prepass wrote before the deck replaced it on this target.
//
// **Motion vectors, and they are the reason this file exists.** TAA
// reprojects every pixel by its motion vector, and a pixel the prepass
// never touched reads zero — a sky that does not move when the camera
// turns. Smooth sky survives that; a cloud's edge smears across the screen.
// A direction at infinity moves only with the view's rotation, so its
// vector is its clip position now against its clip position last frame.
//
// Depth is 0, the clear value, and the normal is zero: sky, to every pass
// that reads the prepass.
#import bevy_pbr::{
    prepass_bindings,
    mesh_view_bindings::view,
}
#import bevy_pbr::utils::coords_to_viewport_uv
#ifdef PREPASS_FRAGMENT
#import bevy_pbr::prepass_io::FragmentOutput
#endif

struct VertexIn {
    @location(0) position: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4(in.position.xy, 0.0, 1.0);
    return out;
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: VertexOut) -> FragmentOutput {
    var out: FragmentOutput;
#ifdef NORMAL_PREPASS
    out.normal = vec4(0.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    let ndc = coords_to_viewport_uv(in.clip.xy, view.viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    let v = view.view_from_clip * vec4(ndc, 1.0, 1.0);
    let d = vec4((view.world_from_view * vec4(v.xyz / v.w, 0.0)).xyz, 0.0);
    let now = view.unjittered_clip_from_world * d;
    let then = prepass_bindings::previous_view_uniforms.clip_from_world * d;
    out.motion_vector = (now.xy / now.w - then.xy / then.w) * vec2(0.5, -0.5);
#endif
#ifdef DEFERRED_PREPASS
    out.deferred = vec4(0u);
    out.deferred_lighting_pass_id = 0u;
#endif
    return out;
}
#else
// A depth-only prepass still runs a fragment stage for a masked material;
// depth 0 is all it writes.
@fragment
fn fragment(in: VertexOut) {}
#endif
