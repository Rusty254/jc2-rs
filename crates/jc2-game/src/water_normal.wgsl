// The generated water normal texture `water_normal_texture` (`Q[3]`, JC2-193 / JC2-194;
// research/formats/water-consumers-next.md section 2, "Water bump texture" pass of the reflection
// post `FUN_00a7f080`).
//
// `compose` is WaterBumpComposite VS 336476 / PS 7535708 on one full-target quad: four samples of
// `textures\water\water_bump.dds` (512 x 512 DXT5, WRAP) at the layer uv
//
//   layer 0 = (x, y)     + phase * ( 0.06,  0.05)        layer 1 = (1 - x, y) + phase * (-0.03, -0.07)
//   layer 2 = (x, 1 - y) + phase * (-0.04,  0.07)        layer 3 = (x, y)     + phase * (-0.07,  0.03)
//
// (y = 0 at the top of the target), `rgb = mean`, `a = cb1[0].w * (mean alpha - 0.5) + 0.5` with the
// literal `cb1[0].w = 1.0`, i.e. the mean alpha. No blending. `phase` is the reflection-pass clock
// (`dt * 0.45` per frame, wrapped above 10000), carried in `PostPass.view.w`.
//
// `downsample` is the `GenerateMips` that follows the draw (`FUN_0094bbf0`): each level is the
// bilinear (2 x 2 box) reduction of the one above (the exact filter of D3D10 `GenerateMips` is
// not recorded: inferred).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct PostPass {
    tint: vec4<f32>,
    saturation: vec4<f32>,
    meter: vec4<f32>,
    blend: vec4<f32>,
    focus: vec4<f32>,
    overlay: vec4<f32>,
    // x = near plane, y = underwater flag, z = -forward.y, w = the water clock phase
    view: vec4<f32>,
    cam_pos: vec4<f32>,
    world_from_clip: mat4x4<f32>,
}
@group(0) @binding(2) var<uniform> post: PostPass;

@fragment
fn compose(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = post.view.w;
    let x = in.uv.x;
    let y = in.uv.y;
    let s0 = textureSample(source, source_sampler, vec2<f32>(x, y) + p * vec2<f32>(0.06, 0.05));
    let s1 = textureSample(source, source_sampler, vec2<f32>(1.0 - x, y) + p * vec2<f32>(-0.03, -0.07));
    let s2 = textureSample(source, source_sampler, vec2<f32>(x, 1.0 - y) + p * vec2<f32>(-0.04, 0.07));
    let s3 = textureSample(source, source_sampler, vec2<f32>(x, y) + p * vec2<f32>(-0.07, 0.03));
    let mean = (s0 + s1 + s2 + s3) * 0.25;
    return vec4<f32>(mean.rgb, 1.0 * (mean.a - 0.5) + 0.5);
}

@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}
