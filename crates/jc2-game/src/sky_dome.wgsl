// The native sky dome (JC2-214; research `sky-gradient-next.md` sections 2, 10; probe-pinned
// disassembly of `skygradientshader`).
//
// Vertex stage (native): `clip.xyw = local * (rotation . view-projection, camera translation)`,
// `clip.z = 0` (the far plane of the reverse-Z projection), `o1.xz = u + (A, B)`, `o1.yw = v`.
// Pixel stage (native): `vv = clamp(v, 0.01, 0.99)`, `rgb = tex(uA, vv) (1 - f) + tex(uB, vv) f`,
// alpha 0. The texture is `skygradient_dif.dds` read as sRGB (the sampler filters the decoded
// texels), wrap in u, clamp in v, bilinear, no mips (sampler fields of JC2-173).
//
// The mesh is the exe's 32-triangle cone in its own local space (+z towards the sun); `frame_*`
// are the rows of the dome rotation `FUN_004dc8a0`, built on the CPU (`native_sky.rs`).

#import bevy_pbr::mesh_view_bindings::view

struct DomeParams {
    frame_x: vec4<f32>,
    frame_y: vec4<f32>,
    frame_z: vec4<f32>,
    // x = A, y = B (u offsets of the two columns), z = f (blend)
    hour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> dome: DomeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var sky: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sky_sampler: sampler;

struct DomeVertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct DomeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(v: DomeVertex) -> DomeOut {
    var out: DomeOut;
    let world = view.world_position
        + v.position.x * dome.frame_x.xyz
        + v.position.y * dome.frame_y.xyz
        + v.position.z * dome.frame_z.xyz;
    var c = view.clip_from_world * vec4<f32>(world, 1.0);
    c.z = 0.0;
    out.clip = c;
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: DomeOut) -> @location(0) vec4<f32> {
    let vv = clamp(in.uv.y, 0.01, 0.99);
    let a = textureSample(sky, sky_sampler, vec2<f32>(in.uv.x + dome.hour.x, vv));
    let b = textureSample(sky, sky_sampler, vec2<f32>(in.uv.x + dome.hour.y, vv));
    let f = dome.hour.z;
    return vec4<f32>(a.rgb * (1.0 - f) + b.rgb * f, 0.0);
}
