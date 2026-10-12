// The SkinnedGeneral pixel shader (PS 6753928) for the opaque double-sided canopy block, reduced to
// what research proved (research/formats/parachute-canopy-shading.md, "The lit colour"). Bevy's
// own lighting is replaced: every term is native and in native scene units (no view exposure on
// the native colours; the ambient tilt is stored in Bevy units, so it is scaled by the exposure,
// as `ambient_tilt` does).
//
//   hemi    = lerp(cb0[11], cb0[10], sat(0.5 N.y + 0.5))  = sky - tilt (1 - N.y)
//   direct  = R * (horizon * cascade * sat(N.L) * cb0[3] + t11.x * hemi + point lights)
//   spec    = G * R * max(horizon, 0.25) * pow(sat(N.H), 32 B + 4) * cb0[1]     (s = 0: cloth, lines)
//   rimcol  = ((hemi - cb0[1]) * 0.5 + cb0[1]) / max_component * (0.75 D + 0.125)
//   rim     = sat(horizon * sat(1.2 pow(sat(1 - N.V), e) - 0.2) * sat(N.y) * k * sat(2 R - 1) * rimcol)
//   colour  = D * direct + spec + rim, then the native haze fog
// D = diffuse (instance colour cb1[1] is white); N is the normal-mapped normal, flipped for back
// faces by `is_front` (the canopy is double sided). Inferred: cascade floor 0.1 + 0.9 c (as the
// General route), LOD scale = 1. Not done: underwater.
// Rico's character blocks (clothes, skin, hair, eyeball tile) take the same shader (board edb9fdda;
// the identity with the canopy's pixel shader is INFERRED, rico-model-detail.md section 2). They add:
//   * the alpha test: Bevy's `Mask` reference through `alpha_discard`, plus the explicit reference
//     `params.w` of the blended bit-18 hair pass (Bevy's `Blend` has no cutoff), native 3/510;
//   * the height ramp s = sat(cb1[2].y - 3.3333 y) with y the raw model-space vertex height
//     (vertex shader output o4.w = v0.y, parachute-canopy-shading.md): Dm = D + s (D D - D),
//     exponent 32 B + 4 + 64 s, specular scale 1 + 4 s. The loader stores y in UV1 (the
//     SkinnedGeneral vertex has no second UV set); without it s = 0.
#import jc2::haze_fog::haze_fog
#import jc2::point_lights::point_light_irradiance
#import jc2::sun_horizon::{ao_uv, horizon_shadow, is_sun}

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    mesh_view_bindings as view_bindings,
    mesh_view_bindings::view,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    shadows,
}

struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    // xyz: (sky - ground) / 2 ambient in Bevy units (multiply by the view exposure)
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}

// general_material.rs `GeneralLight`: cb0[3], cb0[10], cb0[1] (4 x gradient row 1)
struct GeneralLight {
    sun: vec4<f32>,
    sky_ambient: vec4<f32>,
    specular: vec4<f32>,
}

// x: rim scale (block +0x30), y: LOD scale (render state +0xcac), z: 1 when mpm is bound,
// w: explicit alpha reference of a blended pass (0 = none)
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mpm_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var mpm_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var horizon_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var horizon_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<storage, read> horizon: SunHorizon;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<storage, read> light: GeneralLight;

// The cascade shadow term of the Sun among the directional lights (1 when none / no shadows).
fn sun_cascade(world_position: vec4<f32>, world_normal: vec3<f32>, frag_xy: vec2<f32>, flags: u32) -> f32 {
    let n_lights = view_bindings::lights.n_directional_lights;
    for (var i: u32 = 0u; i < n_lights; i = i + 1u) {
        let l = &view_bindings::lights.directional_lights[i];
        if !is_sun((*l).direction_to_light, horizon.sun) {
            continue;
        }
        if (flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && ((*l).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = dot(vec4<f32>(
                view.view_from_world[0].z, view.view_from_world[1].z,
                view.view_from_world[2].z, view.view_from_world[3].z), world_position);
            return shadows::fetch_directional_shadow(i, world_position, world_normal, view_z, frag_xy);
        }
        return 1.0;
    }
    return 1.0;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let pbr_input = pbr_input_from_standard_material(in, is_front);
    let n = pbr_input.N;
    let v = pbr_input.V;
    // D: diffuse texture (instance colour white, no tint)
    // the native alpha test (Mask reference) and the explicit blended-pass reference
    let colour = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    if params.w > 0.0 && colour.a < params.w {
        discard;
    }
    let d_raw = colour.rgb;
    var d = d_raw;
    // the height ramp: s = sat(6 - 3.3333 y), y = raw model-space vertex height (UV1.x)
    var s = 0.0;
#ifdef VERTEX_UVS_B
    s = saturate(6.0 - 3.3333 * in.uv_b.x);
#endif
    d = d + s * (d * d - d);

    var mpm = vec3<f32>(1.0, 0.0, 0.5);
#ifdef VERTEX_UVS_A
    if params.z > 0.5 {
        mpm = textureSampleBias(mpm_tex, mpm_sampler, in.uv, view.mip_bias).rgb;
    }
#endif

    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    let h = horizon_shadow(t11, horizon.cb0, horizon.sun);
    let e0 = view.exposure;
    // lerp(cb0[11], cb0[10], sat(0.5 N.y + 0.5)) = sky - tilt (1 - N.y)
    let hemi = max(light.sky_ambient.rgb - horizon.tilt.xyz * e0 * (1.0 - n.y), vec3<f32>(0.0));

    let sun_dir = horizon.sun.xyz;
    var sun_term = vec3<f32>(0.0);
    var spec = vec3<f32>(0.0);
    if dot(sun_dir, sun_dir) > 0.5 {
        let cascade = sun_cascade(
            in.world_position, pbr_input.world_normal, pbr_input.frag_coord.xy, pbr_input.flags);
        sun_term = light.sun.rgb * (saturate(dot(n, sun_dir)) * h * (0.1 + 0.9 * cascade));
        let hv = normalize(v + sun_dir);
        spec = light.specular.rgb
            * (mpm.g * mpm.r * max(h, 0.25) * pow(saturate(dot(n, hv)), 32.0 * mpm.b + 4.0 + 64.0 * s)
                * (1.0 + 4.0 * s));
    }
    let points = point_light_irradiance(in.world_position.xyz, n);
    let direct = mpm.r * (sun_term + hemi * t11.x + points);

    // rim: constants of the manager constructor 0x00932250, t = camera distance ramp 4.2 .. 40
    let dist = length(view.world_position - in.world_position.xyz);
    let t = clamp((dist / params.y - 4.2) / 35.8, 0.0, 1.0);
    let e = 1.5 / mix(1.0, 3.0, t);
    let k = mix(3.1, 10.0, t) * params.x;
    let rim_base = (hemi - light.specular.rgb) * 0.5 + light.specular.rgb;
    let rim_colour = rim_base / max(max(rim_base.r, max(rim_base.g, rim_base.b)), 0.0001)
        * (0.75 * d_raw + vec3<f32>(0.125));
    let rim_factor = h
        * saturate(1.2 * pow(saturate(1.0 - dot(n, v)), e) - 0.2)
        * saturate(n.y) * k * saturate(2.0 * mpm.r - 1.0);
    let rim = saturate(rim_colour * rim_factor);

    var rgb = d * direct + spec + rim;
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    rgb = haze_fog(rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour);

    var out: FragmentOutput;
    out.color = vec4<f32>(rgb, colour.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
