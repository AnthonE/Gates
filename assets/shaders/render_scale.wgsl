#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var frame: texture_2d<f32>;
@group(1) @binding(1) var frame_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // The native atmosphere leaves sky alpha untouched. Its RGB is already
    // the finished sky, so presentation must not blend that alpha again.
    return vec4(textureSample(frame, frame_sampler, in.uv).rgb, 1.0);
}
