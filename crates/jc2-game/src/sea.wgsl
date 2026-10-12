// The ocean pixel stage (JC2-164; research `research/formats/water-inputs-next.md` and
// `water-consumers-next.md`, JC2-102 / JC2-095), as far as it is decoded and has its inputs here.
//
// Native LOD0 pixel shader, per fragment:
//
//   thickness = eye_u - surface_eye                     (scene depth below the water, along view depth)
//   a         = max(0, 1 - 2^(-thickness * cb0[14].w))  (water colour mix; cb0[14].rgb = table E/F)
//   colour    = lerp(refracted_scene, cb0[14].rgb, a) + cb1[0].rgb * pow(sat(N.H), 256)
//   o1.w      = sat(sat(4 (t_u - 0.5)) + 0.0005 surface_eye)       (SRC1_ALPHA blend factor)
//   target    = (colour, fog) * o1.w + target * (1 - o1.w)         (dual-source blend)
//
// Implemented here with the scene colour `refracted_scene = target` (no (0.8, 0.97, 0.98) tint, no
// caustics, no distortion) so the dual-source blend collapses to premultiplied alpha blending:
// with S = water * a + spec, T the haze transmittance and F the fog colour,
//
//   rgb   = o1w * (T * S + (1 - T) * F)
//   alpha = o1w * (1 - T * (1 - a))
//   out   = rgb + target * (1 - alpha)                   (AlphaMode::Premultiplied: One, OneMinusSrcAlpha)
//
// which equals the native expression exactly for that scene colour (checked in sea.rs).
//
// NOT done (listed as Gaps in the hand-in): the 4-layer composite normal map and foam, the planar
// reflection, the refraction tint / distortion / caustics, vertex displacement, the sky colour at
// the horizon. The specular uses a FLAT normal (0, 1, 0): non-native stand-in for the normal map.
// The scene depth is Bevy's depth prepass (non-native stand-in for the native depth copy).

#import jc2::haze_fog::{haze_exponent, haze_transmittance}
#import bevy_pbr::{
    mesh_view_bindings::view,
    forward_io::VertexOutput,
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct SeaParams {
    // rgb: cb0[14].rgb (water colour, linear, native scene units); w: optical density
    water: vec4<f32>,
    // cb1[0]: sun-specular colour rgb and exponent w
    spec: vec4<f32>,
    // xyz: unit vector towards the light (the flipped one: the moon at night); w unused
    to_light: vec4<f32>,
    // x: pixel-shader tier (0 lod0, 1 lod1, 2 lod2, 3 below); y: 1 = debug tint by tier
    flags: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sea: SeaParams;

struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<storage, read> horizon: SunHorizon;

// 1e9 m: "no geometry behind the water" (open sea). Native depth 0 is the far plane.
const OPEN_WATER_EYE: f32 = 1.0e9;

@fragment
fn fragment(in: VertexOutput, @builtin(sample_index) sample_index: u32) -> @location(0) vec4<f32> {
    let world = in.world_position.xyz;
    let surface_eye = -(view.view_from_world * vec4<f32>(world, 1.0)).z;

    // scene depth behind the water (depth prepass; reverse-Z, 0 = far)
    var eye_u = OPEN_WATER_EYE;
#ifdef DEPTH_PREPASS
    let d = prepass_depth(in.position, sample_index);
    if d > 0.0 {
        let uv = (in.position.xy - view.viewport.xy) / view.viewport.zw;
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let p = view.view_from_clip * vec4<f32>(ndc, d, 1.0);
        eye_u = -p.z / p.w;
    }
#endif

    let thickness = eye_u - surface_eye;
    let a = max(0.0, 1.0 - exp2(-thickness * sea.water.w));
    // o1.w
    let shore = saturate(saturate(4.0 * (thickness - 0.5)) + 0.0005 * surface_eye);

    // sun highlight: flat normal; light vector (x, max(y, 0.3), z) as the vertex stage's cb1[0]
    let v = normalize(view.world_position - world);
    let l = normalize(vec3<f32>(sea.to_light.x, max(sea.to_light.y, 0.3), sea.to_light.z));
    let h = normalize(l + v);
    let spec = sea.spec.xyz * pow(saturate(h.y), sea.spec.w);

    var s = sea.water.rgb * a + spec;
    if sea.flags.y > 0.5 {
        var tint = array<vec3<f32>, 4>(vec3<f32>(1.0, 0.1, 0.1), vec3<f32>(0.1, 1.0, 0.1), vec3<f32>(0.2, 0.2, 1.0), vec3<f32>(1.0, 1.0, 0.1));
        s = tint[u32(sea.flags.x)] * a + spec;
    }

    // native haze (haze_fog.wgsl): T = min(2^-e, 1) of the lit colour survives, the rest is fog
    let clip = view.clip_from_view * (view.view_from_world * vec4<f32>(world, 1.0));
    let t = haze_transmittance(haze_exponent(clip, world.y, horizon.fog_a, horizon.fog_b));
    let fog = horizon.fog_colour.rgb;

    let rgb = shore * (t * s + (1.0 - t) * fog);
    let alpha = shore * (1.0 - t * (1.0 - a));
    return vec4<f32>(rgb, alpha);
}
