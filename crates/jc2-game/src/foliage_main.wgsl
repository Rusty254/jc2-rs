// VegetationFoliage main pass. The native pixel shader (`vegetationfoliage`, PS 7510628,
// research/formats/foliage-lighting-next.md) is ported in `native_light` below; when the sky
// lighting or the terrain horizon constants are absent (viewer, fixtures) or the material has
// the native route off, Bevy's PBR draws as before.
#import jc2::foliage_alpha::{foliage_should_discard, foliage_distance_fade, foliage_lighting_flags, foliage_specular, foliage_is_billboard}
#import jc2::haze_fog::haze_fog
#import jc2::point_lights::point_light_irradiance
#import jc2::sun_horizon::{ao_uv, horizon_shadow, is_sun}
#import bevy_pbr::{
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
    mesh_view_bindings as view_bindings,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    shadows,
}

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

// The shared per-frame constants (sun_horizon.rs): the horizon shadow, the Sun's direction and the
// haze fog fields (haze.rs, haze_fog.wgsl), all zero = no terrain.
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> horizon: SunHorizon;

// t1: normal map (RGB - 0.5, A = translucency weight), t2: aoc (R scales the ambient, UV1),
// t11: the terrain AmbientOcclusionMap (x = AO, y / w = horizon heights).
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var aoc_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var aoc_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var horizon_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var horizon_sampler: sampler;

// general_material.rs `GeneralLight`: cb0[3] (sun / moon colour), cb0[10], cb0[1] (4 x gradient
// row 1, the specular colour) and cb0[12].xyz = cb0[10] + cb0[11], all in native colour units.
struct NativeLight {
    sun: vec4<f32>,
    sky_ambient: vec4<f32>,
    specular: vec4<f32>,
    ambient_sum: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var<storage, read> light: NativeLight;

const LIGHT_NORMAL_MAP: u32 = 1u;
const LIGHT_AOC: u32 = 2u;
const LIGHT_NATIVE: u32 = 4u;

// PS cb1[0].xy of an ordinary instance. The writer (`FUN_009672a0`) stores
// `cb1[0] = (1 - f, 0, 0, w)` with `f = instance byte +0xc / 127` for a byte in 1..127 and 0
// otherwise; the instance header initialiser (`FUN_009236d0`) sets that byte to 255, so f = 0.
// No producer in the rewrite writes the byte (as for the General snow byte), so f = 0.
const CB1_0_X: f32 = 1.0;
const CB1_0_Y: f32 = 0.0;

// The Sun among the directional lights: its direction and `0.1 + 0.9 cascade`. Bevy's cascades stand
// in for the native ones (NON-native, general-terms-next.md Gap 2); the floor is cb0[9].w = 0.1
// (INFERRED, the only writer's default).
struct SunSample {
    to_light: vec3<f32>,
    shadow: f32,
}
fn sun_sample(p: PbrInput) -> SunSample {
    var s: SunSample;
    s.to_light = vec3<f32>(0.0, 1.0, 0.0);
    s.shadow = 1.0;
    let n_lights = view_bindings::lights.n_directional_lights;
    for (var i: u32 = 0u; i < n_lights; i = i + 1u) {
        let l = &view_bindings::lights.directional_lights[i];
        if !is_sun((*l).direction_to_light, horizon.sun) {
            continue;
        }
        s.to_light = (*l).direction_to_light;
        var cascade = 1.0;
        if (p.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && ((*l).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = dot(vec4<f32>(
                view.view_from_world[0].z, view.view_from_world[1].z,
                view.view_from_world[2].z, view.view_from_world[3].z), p.world_position);
            cascade = shadows::fetch_directional_shadow(
                i, p.world_position, p.world_normal, view_z, p.frag_coord.xy);
        }
        s.shadow = 0.1 + 0.9 * cascade;
        return s;
    }
    return s;
}

// The pixel shader's lighting (PS 7510628, lines 275 to 337 of its disassembly), in native units:
//   N      = sign * normalize(lerp(T (t1.x - .5) + B (t1.y - .5) + Ng (t1.z - .5), Ng, cb1[0].y))
//   shadow = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w) * cascade
//   S      = shadow * (t11.x / 2 + 1/2)
//   w      = (1 - cb1[0].y) * cb1[0].x * (clamp(10 (N.L)(-N.V) - 2, -1, 1) / 2 + 1/2)
//   sun    = mix(sat(N.L) S cb0[3], (1 - t1.a) S cb0[3] (.5, .6, .15), w)
//   light  = cb0[12] t2.r t11.x + sun + point lights
//   colour = diffuse light, mixed toward light (.508, .54, .645) by sat((3 t2.r)^1.5 ((1 - t1.a) - cb1[0].x))
//   colour += cb0[1] cb1[1].x pow(max(N.H, .001), cb1[1].y) max(shadow, .3) (1 - cb1[0].y)
// The vertex shader's point-light sum (v10) is the shared point-light loop here.
fn native_light(in: VertexOutput, is_front: bool, p: PbrInput, flags: u32, diffuse: vec3<f32>) -> vec3<f32> {
    var uv0 = vec2<f32>(0.0);
    var uv1 = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv0 = in.uv;
    uv1 = in.uv;
#endif
#ifdef VERTEX_UVS_B
    uv1 = in.uv_b;
#endif
    var t1 = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    if (flags & LIGHT_NORMAL_MAP) != 0u {
        t1 = textureSampleBias(normal_tex, normal_sampler, uv0, view.mip_bias);
    }
    var t2r = 1.0;
    if (flags & LIGHT_AOC) != 0u {
        t2r = textureSampleBias(aoc_tex, aoc_sampler, uv1, view.mip_bias).r;
    }

    let n_geo = normalize(in.world_normal);
    var n = n_geo;
#ifdef VERTEX_TANGENTS
    let tangent = normalize(in.world_tangent.xyz);
    let bitangent = in.world_tangent.w * cross(n_geo, tangent);
    let ts = t1.rgb - vec3<f32>(0.5);
    // cb1[0].y = 0: the normal map in full; with y = 1 it would be the geometric normal
    let mapped = normalize(ts.x * tangent + ts.y * bitangent + ts.z * n_geo);
    n = normalize(mix(mapped, n_geo, CB1_0_Y));
#endif
    if !is_front {
        n = -n;
    }

    let v = normalize(view.world_position.xyz - in.world_position.xyz);
    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    let sun = sun_sample(p);
    let sun_col = light.sun.rgb;
    let shadow = horizon_shadow(t11, horizon.cb0, horizon.sun) * sun.shadow;
    let s = shadow * (t11.x * 0.5 + 0.5);
    let n_l = dot(n, sun.to_light);
    let back = clamp(10.0 * (n_l * dot(n, -v)) - 2.0, -1.0, 1.0) * 0.5 + 0.5;
    let weight = (1.0 - CB1_0_Y) * CB1_0_X * back;
    let see_through = 1.0 - t1.a;
    let front_sun = saturate(n_l) * s * sun_col;
    let back_sun = see_through * s * sun_col * vec3<f32>(0.5, 0.6, 0.15);
    let lit = light.ambient_sum.rgb * (t2r * t11.x)
        + mix(front_sun, back_sun, weight)
        + point_light_irradiance(in.world_position.xyz, n);
    var rgb = diffuse * lit;
    let k = saturate(pow(t2r * 3.0, 1.5) * (see_through - CB1_0_X));
    rgb = mix(rgb, lit * vec3<f32>(0.508, 0.54, 0.645), k);
    let spec = foliage_specular();
    let h = normalize(v + sun.to_light);
    rgb = rgb + light.specular.rgb * spec.x * pow(max(dot(h, n), 0.001), spec.y)
        * max(shadow, 0.3) * (1.0 - CB1_0_Y);
    return rgb;
}

// The BillboardFoliage pixel shader (`billboardfoliage`, 45 lines, research/formats/
// billboard-foliage-next.md). No normal, no N.L, no shadow map, no point lights:
//   rgb   = tex * v4.y
//   light = cb0[3] * (sat(v2.w + 1) * horizon * v4.x * (t11.x / 2 + 1/2)) + 0.5 * cb0[12] * t11.x
// v4.x and v4.y come from the vertex stage (foliage_billboard.wgsl: fake sphere light and the
// pivot-distance ramp) in `uv_b`; sat(v2.w + 1) is 1 for every vertex output (v2.w is already
// saturated and non-negative). The fog is the shared haze fog below.
fn billboard_light(in: VertexOutput, diffuse: vec3<f32>) -> vec3<f32> {
    var v4 = vec2<f32>(1.0, 1.0);
#ifdef VERTEX_UVS_B
    v4 = in.uv_b;
#endif
    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    let s = horizon_shadow(t11, horizon.cb0, horizon.sun) * v4.x * (t11.x * 0.5 + 0.5);
    let lit = light.sun.rgb * s + light.ambient_sum.rgb * (0.5 * t11.x);
    return diffuse * v4.y * lit;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var input = pbr_input_from_standard_material(in, is_front);
    let fade = foliage_distance_fade(in.world_position.xyz, in.instance_index);
    if foliage_should_discard(input.material.base_color.a, fade, in.instance_index) { discard; }
    // The common native test is the only alpha cutoff. The material remains in
    // the masked phase to run the same shader contract for depth and shadows.
    input.material.base_color.a = 1.0;
    var out: FragmentOutput;
    let flags = foliage_lighting_flags();
    // the native route needs the sky light colours and the terrain horizon constants (the Sun's
    // direction); without either (viewer, fixtures) Bevy's PBR draws
    let native = (flags & LIGHT_NATIVE) != 0u
        && dot(light.sun.rgb, vec3<f32>(1.0)) > 0.0
        && dot(horizon.sun.xyz, horizon.sun.xyz) > 0.5;
    if (input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u {
        out.color = input.material.base_color;
    } else if native && foliage_is_billboard() {
        out.color = vec4<f32>(billboard_light(in, input.material.base_color.rgb), 1.0);
    } else if native {
        out.color = vec4<f32>(
            native_light(in, is_front, input, flags, input.material.base_color.rgb), 1.0);
    } else {
        out.color = apply_pbr_lighting(input);
    }
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    out.color = vec4<f32>(
        haze_fog(out.color.rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour),
        out.color.a);
    out.color = main_pass_post_lighting_processing(input, out.color);
    return out;
}
