// The native blur chain pass (JC2-138; `post_effects_render.rs`): the `Blur` pixel shader
// (PS 965692) with its vertex shader (VS 44952): seven equal-weight taps (1/7 each) at
// `uv + tap * (spread / width, spread / height)`. The tap table is the one `FUN_0092ec50`
// stores in the three blur quads (constants read from the exe at 0xe5becc..0xe5d5e8, float32);
// the spread is 1.75 texels of the full-size source for pass 0 (which writes the half-size
// target), 2.625 texels of the half-size target for pass 1 (3.5 for the underwater third pass,
// not used here). The step is computed on the CPU (`blur_step` in post_effects_render.rs).
// Source and target are 8-bit sRGB (native format 0x1d), the sampler is linear / clamp.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
struct BlurStep {
    // xy = spread / (width, height) of the pass
    step: vec4<f32>,
}
@group(0) @binding(2) var<uniform> blur: BlurStep;

const TAPS = array<vec2<f32>, 7>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(0.060393, 0.15534),
    vec2<f32>(-0.298919, -0.147507),
    vec2<f32>(0.480085, -0.139708),
    vec2<f32>(-0.346193, 0.569732),
    vec2<f32>(-0.161941, -0.817447),
    vec2<f32>(0.805884, 0.592074),
);

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 7; i++) {
        sum += textureSample(source, source_sampler, in.uv + TAPS[i] * blur.step.xy);
    }
    return sum * 0.142857;
}
