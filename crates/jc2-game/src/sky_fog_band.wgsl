// The FogGradient horizon band (JC2-214; research `sky-gradient-next.md` sections 4, 10 and
// `haze-fog-next.md` section 6; `foggradientshader`).
//
// Vertex (native): `pos = v0 * cb1[4]` translated to the camera, clip z forced to 0 (far plane);
// COLOR0 = cb1[5]. Pixel: `rgb = cb1[5].rgb`, `a = texture alpha`. Blend SRC_ALPHA / INV_SRC_ALPHA.
// The ring mesh (16 segments, radius 2, y -10 .. +1, v = y) is INFERRED from a hand decode of the
// constructor; the scale and colour are `FUN_00a45110`.

#import bevy_pbr::mesh_view_bindings::view

struct BandParams {
    // cb1[4]: ring scale (30, 2 + 200 P, 30, 1)
    scale: vec4<f32>,
    // cb1[5]: (fog colour rgb, 1)
    colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> band: BandParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var ramp: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var ramp_sampler: sampler;

struct BandVertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct BandOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(v: BandVertex) -> BandOut {
    var out: BandOut;
    let world = view.world_position + v.position * band.scale.xyz;
    var c = view.clip_from_world * vec4<f32>(world, 1.0);
    c.z = 0.0;
    out.clip = c;
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: BandOut) -> @location(0) vec4<f32> {
    let a = textureSample(ramp, ramp_sampler, in.uv).a;
    return vec4<f32>(band.colour.rgb, a);
}
