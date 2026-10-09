// The phase 1 final post pass (JC2-134; `post_effects.rs`): the composite tail of the native
// `Scene` pixel shader (research/formats/night-exposure-next.md section 1.3), run on the
// camera's 8-bit sRGB scene target.
//
//   C   = clamp(scene, 0, 1)                      (the native scene target is 8-bit, clamped)
//   lum = dot(C, (0.2125, 0.7154, 0.0721))
//   k   = clamp(lum * slope + offset, lo, hi)     (S16, S15, min(S14, S17), max(S14, S17))
//   C   = lum + k * (C - lum)
//   C  *= (A + S7 - 1) * tint                     (A = 1 in phase 1, tint = S11..S13)
//   out = sqrt(C)                                 (gamma 2.0, not the sRGB curve)
//
// The bound texture is an sRGB view, so `scene` is linear. The target is an sRGB view too, so
// the hardware encodes what the shader returns with the sRGB curve; to store `sqrt(C)` itself
// (what the native UNORM swap chain holds) the shader returns the sRGB *decode* of it, and the
// hardware encode gives `sqrt(C)` back (single encode, no double encoding).
// Not implemented (listed as gaps in post_effects.rs): blur mix, overlay (S2, S3), depth of
// field (S5, S25), heat haze, auto-exposure `A`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct PostPass {
    // rgb = tint (S11, S12, S13), w = A + S7 - 1
    tint: vec4<f32>,
    // x = offset (S15), y = slope (S16), z = lower clamp, w = upper clamp
    saturation: vec4<f32>,
}
@group(0) @binding(2) var<uniform> post: PostPass;

fn srgb_decode(v: vec3<f32>) -> vec3<f32> {
    let lo = v / 12.92;
    let hi = pow((v + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, v <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(screen_texture, texture_sampler, in.uv);
    var c = clamp(src.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let lum = dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
    let k = clamp(lum * post.saturation.y + post.saturation.x, post.saturation.z, post.saturation.w);
    c = vec3<f32>(lum) + k * (c - vec3<f32>(lum));
    c = c * (post.tint.w * post.tint.rgb);
    let v = clamp(sqrt(max(c, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(srgb_decode(v), src.a);
}
