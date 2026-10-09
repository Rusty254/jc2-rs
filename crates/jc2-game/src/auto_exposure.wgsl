// Auto-exposure luminance chain (JC2-137; CPU reference and status of every part in
// `auto_exposure.rs`; native pass `FUN_0093e660`, research/formats/night-exposure-next.md 1.2/1.3).
//
// Four dispatches in one compute pass, in order (wgpu inserts the storage barriers):
//   sample_initial : 64x64 map, mean of 4 bilinear taps of dot(rgb, LUM) of the pre-post scene
//   downscale_16   : 16x16 map from the 64x64 map (mean of 4 bilinear taps)
//   downscale_4    : 4x4 map from the 16x16 map
//   final_adapt    : SampleLumFinal (target T) + CalculateAdaptedLum (A), one thread
// Tap geometry is INFERRED (brief Gap 5): offsets (-0.5, -0.5), (-0.5, +1.5), (+1.5, -0.5),
// (+1.5, +1.5) source texels from the target pixel centre; the final stage samples the 4x4 map at
// (0.5 + offset / 4) * 0.5 + 0.25.
//
// The composite (post_effects.wgsl) multiplies by `ex.applied`, which is the value metered
// in the PREVIOUS frame (the native pass runs after the composite): one frame of latency, brief
// Gap 7. On the first frame `A = T` and that frame already uses it (non-native choice).

struct PostPass {
    tint: vec4<f32>,
    saturation: vec4<f32>,
    // x = S8 (weight of the darkest tap), y = S9 (strength), z = S10 (key), w = frame dt (s)
    meter: vec4<f32>,
}

struct Exposure {
    applied: f32,
    latest: f32,
    inited: f32,
    pad: f32,
    // 64x64 at 0, 16x16 at 4096, 4x4 at 4352
    lum: array<f32, 4368>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> post: PostPass;
@group(0) @binding(3) var<storage, read_write> ex: Exposure;

const LUMINANCE = vec3<f32>(0.2125, 0.7154, 0.0721);
const OFF_16 = 4096u;
const OFF_4 = 4352u;
const FLOOR = 0.01;
const RATE = 1.74878;

const TAPS = array<vec2<f32>, 4>(
    vec2<f32>(-0.5, -0.5),
    vec2<f32>(-0.5, 1.5),
    vec2<f32>(1.5, -0.5),
    vec2<f32>(1.5, 1.5),
);

// bilinear fetch of an n x n map at `off` (clamp to edge, texel centres at +0.5)
fn bilerp(off: u32, n: u32, uv: vec2<f32>) -> f32 {
    let p = uv * f32(n) - vec2<f32>(0.5);
    let f = floor(p);
    let t = p - f;
    let hi = i32(n) - 1;
    let x0 = u32(clamp(i32(f.x), 0, hi));
    let x1 = u32(clamp(i32(f.x) + 1, 0, hi));
    let y0 = u32(clamp(i32(f.y), 0, hi));
    let y1 = u32(clamp(i32(f.y) + 1, 0, hi));
    let top = mix(ex.lum[off + y0 * n + x0], ex.lum[off + y0 * n + x1], t.x);
    let bottom = mix(ex.lum[off + y1 * n + x0], ex.lum[off + y1 * n + x1], t.x);
    return mix(top, bottom, t.y);
}

@compute @workgroup_size(8, 8, 1)
fn sample_initial(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 64u || id.y >= 64u {
        return;
    }
    let size = vec2<f32>(textureDimensions(scene));
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / 64.0;
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let c = textureSampleLevel(scene, scene_sampler, uv + TAPS[i] / size, 0.0).rgb;
        sum += dot(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), LUMINANCE);
    }
    ex.lum[id.y * 64u + id.x] = sum * 0.25;
}

fn downscale(id: vec3<u32>, src_off: u32, src_n: u32, out_off: u32, out_n: u32) {
    if id.x >= out_n || id.y >= out_n {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / f32(out_n);
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        sum += bilerp(src_off, src_n, uv + TAPS[i] / f32(src_n));
    }
    ex.lum[out_off + id.y * out_n + id.x] = sum * 0.25;
}

@compute @workgroup_size(8, 8, 1)
fn downscale_16(@builtin(global_invocation_id) id: vec3<u32>) {
    downscale(id, 0u, 64u, OFF_16, 16u);
}

@compute @workgroup_size(8, 8, 1)
fn downscale_4(@builtin(global_invocation_id) id: vec3<u32>) {
    downscale(id, OFF_16, 16u, OFF_4, 4u);
}

@compute @workgroup_size(1, 1, 1)
fn final_adapt() {
    // SampleLumFinal: the four taps of the 4x4 map
    var taps: array<f32, 4>;
    for (var i = 0; i < 4; i++) {
        let uv = (vec2<f32>(0.5) + TAPS[i] / 4.0) * 0.5 + vec2<f32>(0.25);
        taps[i] = bilerp(OFF_4, 4u, uv);
    }
    let x = post.meter.x;
    let y = post.meter.y;
    let key = post.meter.z;
    let mean = (taps[0] + taps[1] + taps[2] + taps[3]) * 0.25;
    let r_avg = key / max(mean, FLOOR);
    var r_max = -3.0e38;
    for (var i = 0; i < 4; i++) {
        r_max = max(r_max, max(key / max(taps[i], FLOOR), 0.0));
    }
    let target_a = 1.0 + y * ((r_avg + x * (r_max - r_avg)) - 1.0);

    // CalculateAdaptedLum (the first frame is A = T)
    if ex.inited == 0.0 {
        let a = clamp(target_a, 0.0, 4.0);
        ex.latest = a;
        ex.applied = a;
        ex.inited = 1.0;
        return;
    }
    let prev = ex.latest;
    ex.applied = prev;
    let k = 1.0 - exp(-RATE * clamp(post.meter.w, 0.01, 0.5));
    var a = prev + (target_a - prev) * k;
    if a != a {
        a = target_a;
    }
    if a != a {
        a = 0.5;
    }
    ex.latest = clamp(a, 0.0, 4.0);
}
