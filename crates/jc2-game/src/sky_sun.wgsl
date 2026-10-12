// The sun disc and glow billboards of pass SUN (JC2-214; research `sun-glow-next.md` section 2;
// `Open` shader pair, probe-pinned).
//
// Two camera-relative quads in the sun orbit frame: world = camera + x h row0 + dist row1 + z h row2
// (disc: h 80, dist 1500; glow: h 800, dist 500), clip z forced to 0 (far plane, behind
// everything). `Open` pixel stage: out = tex * tint * vertex colour (all four channels); blend
// SRC_ALPHA / ONE, so dst += rgb a. The packed white vertex colour unpacks to
// (0.984375, 0.999756, 0.999996, 0.99999994) (section 2.2).
//
// ASSUMED (not decoded): the in-plane orientation of rows 0 and 2 (the textures are radial) and
// that the plane is exactly perpendicular to the sun direction (rows from `FUN_00a53460`).

#import bevy_pbr::mesh_view_bindings::view

struct SunParams {
    frame_x: vec4<f32>,
    frame_y: vec4<f32>,
    frame_z: vec4<f32>,
    // x = half size h, y = distance
    geom: vec4<f32>,
    // `block + 0x1c`: the tint
    tint: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sun: SunParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var sun_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sun_sampler: sampler;

const VERTEX_COLOUR: vec4<f32> = vec4<f32>(0.984375, 0.999756, 0.999996, 0.99999994);

struct SunVertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct SunOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(v: SunVertex) -> SunOut {
    var out: SunOut;
    let h = sun.geom.x;
    let world = view.world_position
        + v.position.x * h * sun.frame_x.xyz
        + sun.geom.y * sun.frame_y.xyz
        + v.position.z * h * sun.frame_z.xyz;
    var c = view.clip_from_world * vec4<f32>(world, 1.0);
    c.z = 0.0;
    out.clip = c;
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: SunOut) -> @location(0) vec4<f32> {
    let t = textureSample(sun_tex, sun_sampler, in.uv);
    return t * sun.tint * VERTEX_COLOUR;
}
