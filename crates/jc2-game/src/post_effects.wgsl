// The final post pass (JC2-134 / JC2-137 / JC2-138; `post_effects.rs`): the shipped native
// composite, `DepthOfField_EdgeFade` PS 4863324 (research/formats/post-composite-variant-next.md;
// the `Scene` family is the same code with a zero focus ramp), run on the camera's 8-bit sRGB
// scene target.
//
//   eye  = near / depth                           (Bevy reverse-Z; native: 1 / (d cb1[2].z + cb1[2].w))
//   ramp = sat((S6 - eye) w) + sat((eye - S5) / S5)             (cb1[5])
//   m    = sat(ramp * S25 + S4)                                 (cb1[0].w, cb1[0].z)
//   C    = scene + m * (blur - scene)             (t1 = the two-pass Blur chain, post_blur.wgsl)
//   b    = bloom (t4, HighPass of the blur, quarter size)
//   C    = b < 0.5 ? 2 C (S3 (b - 0.5) + 0.5) : 1 - (2 - 2 C) (1 - (S2 (b - 0.5) + 0.5))
//   lum  = dot(C, (0.2125, 0.7154, 0.0721))
//   k    = clamp(lum * slope + offset, lo, hi)     (S16, S15, min(S14, S17), max(S14, S17))
//   C    = lum + k * (C - lum)
//   C   *= (A + S7 - 1) * tint                     (A = the auto-exposure of auto_exposure.wgsl, tint = S11..S13)
//   C   *= 1 - sat((length(uv - 0.5) - 0.35) * 2.2)             (EdgeFade vignette)
//   out  = sqrt(C)                                 (gamma 2.0, not the sRGB curve)
//
// The bound textures are sRGB views, so every colour is linear. The target is an sRGB view too, so
// the hardware encodes what the shader returns with the sRGB curve; to store `sqrt(C)` itself
// (what the native UNORM swap chain holds) the shader returns the sRGB *decode* of it, and the
// hardware encode gives `sqrt(C)` back (single encode, no double encoding).
// Heat haze (HeatHazeDOF) and the Underwater family are below (JC2-193 / JC2-194). Not implemented
// (listed as gaps in post_effects.rs): film grain. `A` (JC2-137) is the value metered in the previous frame by auto_exposure.wgsl
// (post_effects_render.rs runs that compute pass right before this one).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(3) var blur_texture: texture_2d<f32>;
// the scene depth: Bevy's depth prepass (multisampled when the camera uses MSAA)
#ifdef MSAA_DEPTH
@group(0) @binding(4) var depth_texture: texture_depth_multisampled_2d;
#else
@group(0) @binding(4) var depth_texture: texture_depth_2d;
#endif
@group(0) @binding(5) var bloom_texture: texture_2d<f32>;
// the generated water normal texture (`Q[3]`, `water_normal.rs`): RG distortion, A caustics;
// WRAP, linear, mip chosen by derivative (the native s5 / s6, JC2-103 section 5)
@group(0) @binding(6) var normal_texture: texture_2d<f32>;
@group(0) @binding(7) var normal_sampler: sampler;

struct PostPass {
    // rgb = tint (S11, S12, S13), w = S7 - 1
    tint: vec4<f32>,
    // x = offset (S15), y = slope (S16), z = lower clamp, w = upper clamp
    saturation: vec4<f32>,
    // x = S8, y = S9, z = S10, w = dt: read by the auto-exposure compute pass only
    meter: vec4<f32>,
    // cb1[0]: x = heat haze scale (unused), z = S4 blur blend, w = S25 depth-of-field gain
    blend: vec4<f32>,
    // cb1[5]: (S5, 1 / S5, S6, 1 / (0.5 S6)); all zero when S5 <= 0
    focus: vec4<f32>,
    // (S2, S3, S0, S1): overlay strengths and the HighPass mix / exponent
    overlay: vec4<f32>,
    // x = camera near plane (m), y = underwater family flag (camera.y < 200), z = cb0[6].y
    // (minus forward Y), w = water clock phase
    view: vec4<f32>,
    // xyz = camera world position (cb0[5])
    cam_pos: vec4<f32>,
    // [ndc.xy, d, 1] -> camera-relative world position * w
    world_from_clip: mat4x4<f32>,
}
@group(0) @binding(2) var<uniform> post: PostPass;

// the adapted auto-exposure of this frame (the first four floats of the buffer
// auto_exposure.wgsl writes: applied, latest, inited, pad)
struct Exposure {
    applied: f32,
}
@group(1) @binding(0) var<storage, read> exposure: Exposure;

fn srgb_decode(v: vec3<f32>) -> vec3<f32> {
    let lo = v / 12.92;
    let hi = pow((v + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, v <= vec3<f32>(0.04045));
}

// nearest (largest reverse-Z) of the depth samples of a pixel
fn scene_depth(px: vec2<i32>) -> f32 {
#ifdef MSAA_DEPTH
    var d = 0.0;
    let n = i32(textureNumSamples(depth_texture));
    for (var i = 0; i < n; i++) {
        d = max(d, textureLoad(depth_texture, px, i));
    }
    return d;
#else
    return textureLoad(depth_texture, px, 0);
#endif
}

// everything between the blur blend and the saturation clamp / exposure of the composite:
// the overlay of the bloom `b`, the saturation clamp and `* (A + S7 - 1) * tint`
fn grade(blended: vec3<f32>, bloom: vec3<f32>) -> vec3<f32> {
    var c = blended;
    let multiply = 2.0 * c * (post.overlay.y * (bloom - vec3<f32>(0.5)) + vec3<f32>(0.5));
    let screen = vec3<f32>(1.0)
        - (vec3<f32>(2.0) - 2.0 * c) * (vec3<f32>(1.0) - (post.overlay.x * (bloom - vec3<f32>(0.5)) + vec3<f32>(0.5)));
    c = select(screen, multiply, bloom < vec3<f32>(0.5));

    let lum = dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
    let k = clamp(lum * post.saturation.y + post.saturation.x, post.saturation.z, post.saturation.w);
    c = vec3<f32>(lum) + k * (c - vec3<f32>(lum));
    return c * ((exposure.applied + post.tint.w) * post.tint.rgb);
}

// the EdgeFade vignette, the single `sqrt` and the sRGB-target trick described above
fn finish(c: vec3<f32>, uv: vec2<f32>, alpha: f32) -> vec4<f32> {
    let lit = c * (1.0 - saturate((length(uv - vec2<f32>(0.5)) - 0.35) * 2.2));
    let v = clamp(sqrt(max(lit, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(srgb_decode(v), alpha);
}

// `Underwater_EdgeFade` PS 7485260 (JC2-194; research/formats/underwater-render-next.md
// sections 6 and 7): the screen lookup is displaced by `0.01 (t5.rg 2 - 1)` (t5 at 0.1 uv), the
// depth, colour, blur and bloom are read at the displaced uv, and the world position rebuilt
// from it gives the caustic gain on the lit colour (before the vignette).
//
// Depth: the native copy contains the water surface seen from below; Bevy's prepass does not
// (the sea is alpha blended). Non-native stand-in: the flat sea plane y = 200 is intersected
// analytically and the nearer of it and the prepass depth is used (the sea's vertex waves
// are not ported either). The sky, d = 0, is limited to the native far plane (40000 m).
fn underwater(in: FullscreenVertexOutput) -> vec4<f32> {
    let n = textureSample(normal_texture, normal_sampler, 0.1 * in.uv);
    let uv = in.uv + 0.01 * (n.rg * 2.0 - vec2<f32>(1.0));
    let dims = vec2<f32>(textureDimensions(depth_texture));
    let px = vec2<i32>(clamp(uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);

    // the near-plane point of this ray, relative to the camera
    let qh = post.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let q = qh.xyz / qh.w;
    var d = max(scene_depth(px), post.view.x / 40000.0);
    let rise = 200.0 - post.cam_pos.y;
    if q.y > 0.0 && rise > 0.0 {
        // the plane hit is at eye distance near * rise / q.y: depth near / eye
        d = max(d, q.y / rise);
    }
    let eye = post.view.x / d;

    let scene = clamp(textureSample(screen_texture, texture_sampler, uv).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let blurred = textureSample(blur_texture, texture_sampler, uv).rgb;
    let bloom = textureSample(bloom_texture, texture_sampler, uv).rgb;
    let ramp = saturate((post.focus.z - eye) * post.focus.w)
        + saturate((eye - post.focus.x) * post.focus.y);
    let m = saturate(ramp * post.blend.w + post.blend.z);
    var c = grade(scene + m * (blurred - scene), bloom);

    let ph = post.world_from_clip * vec4<f32>(ndc, d, 1.0);
    let rel = ph.xyz / ph.w;
    let world_y = post.cam_pos.y + rel.y;
    let dist2 = max(dot(rel, rel), 1e-12);
    let glow = min(1.0 / (0.07 * dist2), 0.4);
    let sg = saturate(200.0 - world_y);
    let look = min(2.0 * max(post.view.z + 0.2, 0.1), 1.0);
    let world_x = post.cam_pos.x + rel.x;
    let world_z = post.cam_pos.z + rel.z;
    let uvc = vec2<f32>(world_x - 0.25 * world_y, world_z - 0.25 * world_y);
    let a_fine = textureSample(normal_texture, normal_sampler, 0.08 * uvc).a;
    let a_coarse = textureSample(normal_texture, normal_sampler, 0.3 * uvc).a;
    let a2 = 2.0 * (a_fine + 0.5 * a_coarse);
    c = c * (1.0 + look * glow * sg * a2 * a2 * a2);
    return finish(c, in.uv, 1.0);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    if post.view.y > 0.5 {
        return underwater(in);
    }
    let src = textureSample(screen_texture, texture_sampler, in.uv);
    let scene = clamp(src.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let bloom = textureSample(bloom_texture, texture_sampler, in.uv).rgb;

    // `HeatHazeDOF_EdgeFade` PS 6142692 (JC2-193): the blurred scene is read at
    // `uv + (t5(0.4 uv).rg 2 - 1) * 0.004 S26`; the family needs S26 > 0.001 and S5 > 0
    var blur_uv = in.uv;
    if post.blend.x > 0.004 * 0.001 && post.focus.x > 0.0 {
        let n = textureSample(normal_texture, normal_sampler, 0.4 * in.uv).rg;
        blur_uv = (n * 2.0 - vec2<f32>(1.0)) * post.blend.x + in.uv;
    }
    let blurred = textureSample(blur_texture, texture_sampler, blur_uv).rgb;

    let d = scene_depth(vec2<i32>(in.position.xy));
    let eye = post.view.x / max(d, 1e-9);
    let ramp = saturate((post.focus.z - eye) * post.focus.w)
        + saturate((eye - post.focus.x) * post.focus.y);
    let m = saturate(ramp * post.blend.w + post.blend.z);
    let c = grade(scene + m * (blurred - scene), bloom);
    return finish(c, in.uv, src.a);
}

// The HighPass pass (PS 6147124, `FUN_00953060`): reads the blurred half-size texture through
// `screen_texture`, writes the quarter-size bloom texture (8-bit sRGB, so the result is clamped):
// `pow(blur, S1)`, saturation mix `S0`, `* A`. (Native `log`/`exp` are base 2.)
@fragment
fn highpass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(screen_texture, texture_sampler, in.uv).rgb;
    var c = exp2(log2(max(src, vec3<f32>(1e-30))) * post.overlay.w);
    let lum = dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
    c = vec3<f32>(lum) + post.overlay.z * (c - vec3<f32>(lum));
    return vec4<f32>(c * exposure.applied, 1.0);
}
