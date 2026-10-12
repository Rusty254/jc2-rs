// The original General pixel shader (PS 6062008, `general-ps.asm` of the JC2-085 probe),
// reduced to what research proved, on top of Bevy's StandardMaterial lighting. See
// crates/jc2-game/src/general_material.rs for the proven / unproven list.
//
// Native, by instruction (the registers are the disassembly's):
//   r0 = t0(uv0)                         dp4(r0, cb2[2]) with alpha 1 when UseChannelTextures
//   r1 = r0 * v9                         v9 = vertex colour (VS 188280 packed-float decode)
//   r3 = t3(uv1)                         dp3(r3.rgb, cb2[1].xyz) when cb2[1].w > .5 (ChannelAO)
//   r1.rgb = r3 * r1.rgb                 C = diffuse * vertex colour * AO
//   r2 = t2(uv0)                         MPM: r2.x R, r2.y G, r2.z B
//   t11 = t11(v1.zw)                     shadow = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w)
//   r0.y = cascade * shadow              NO (t11.x / 2 + 1/2) factor here (terrain has one)
//   r4 = lerp(cb0[11], cb0[10], sat(N.y/2 + 1/2)) * t11.x + cb0[3] * sat(N.-L) * r0.y
//   r4 = r4 + B (2 - r4)                 mad r4, r2.zzzz, (2 - r4), r4
//   r4 = r4 + point lights               point_lights.wgsl (JC2-221)
//   r1.rgb = C * r4 + R * reflection     reflection needs t4 (cubemap) and cb0[3] + cb0[10]: NOT done
//   r1.rgb += cb0[1] * pow(max(dot(N, normalize(V + L)), .001), cb2[0].x) * r0.y * G * (1 - v6.w)
//   discard when (selected alpha * instance alpha - reference) < 0, if cb4[3].w
// Bevy's lighting supplies "C * (ambient t11.x + Sun N.L shadow)": the Sun's direct term is
// rescaled by the horizon shadow (as the terrain does), the ambient by `diffuse_occlusion`.
// The B remap is then `rgb + B (2 C - rgb)` = C (L + B (2 - L)) in Bevy's lit-output units.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    pbr_bindings,
    mesh_view_bindings::view,
}
#import jc2::haze_fog::haze_fog
#import jc2::point_lights::point_light_irradiance
#import jc2::sun_horizon::{ao_uv, horizon_shadow, is_sun, ambient_tilt}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_diffuse_color, calculate_F0_dielectric},
    pbr_types::PbrInput,
    mesh_view_bindings as view_bindings,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    lighting,
    lighting::LAYER_BASE,
    shadows,
}

// the per-frame horizon constants (sun_horizon.rs; sun_horizon.wgsl says how to use them)
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    // xyz: (sky - ground) / 2 ambient, see ambient_tilt in sun_horizon.wgsl
    tilt: vec4<f32>,
    // haze fog (haze.rs, haze_fog.wgsl): cb0[12], cb0[13], fog colour; all zero = no fog
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}

// the per-frame native light (general_material.rs `GeneralLight`): cb0[3] and cb0[10]
struct GeneralLight {
    sun: vec4<f32>,
    sky_ambient: vec4<f32>,
    // cb0[1].xyz = 4 x gradient row 1: the specular colour
    specular: vec4<f32>,
}

struct GeneralParams {
    // cb2[2]
    channel_texture_mask: vec4<f32>,
    // xyz cb2[1].xyz, w cb2[1].w
    channel_ao: vec4<f32>,
    // cb2[0].x
    specular_power: f32,
    flags: u32,
    alpha_reference: f32,
    reserved: f32,
    // instance record (Rust `GeneralInstance`): x receive cb1[2].w, y LOD fade v6.w, z alpha cb1[11].w,
    // w snow byte / 255
    instance: vec4<f32>,
    // instance tint (rgb, a), applied after the fog
    tint: vec4<f32>,
}

const SHADER_CHANNEL_TEXTURES: u32 = 1u;
const SHADER_HAS_MPM: u32 = 2u;
const SHADER_HAS_AO: u32 = 4u;
const SHADER_HAS_ENV: u32 = 16u;
const PI: f32 = 3.14159265358979;

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> general: GeneralParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mpm_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var mpm_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var ao_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var ao_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var horizon_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var horizon_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<storage, read> horizon: SunHorizon;
// t4 / s4: genericenvcube_dif.dds (clamp, linear, mip point, UNORM)
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var env_tex: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var env_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<storage, read> light: GeneralLight;

struct SunTerms {
    // exposure * Sun diffuse * (cascade floor * receive-scaled horizon shadow - cascade): added
    // to Bevy's lit colour (which holds the unfloored cascade and no horizon shadow)
    correction: vec3<f32>,
    // the native specular: cb0[1] * pow(N.H, power) * shadow * MPM.g * (1 - v6.w), in native scene units (no exposure)
    specular: vec3<f32>,
}

// The Sun among the directional lights: its shadow correction and the native Blinn-Phong
// highlight. Same fetch of the cascade shadow as apply_pbr_lighting. `horizon` is the raw
// horizon term h; the receive strength w gives h' = 1 + w (h - 1) (horizon only, never the
// cascade); the cascade term keeps the floor 0.1 + 0.9 c (INFERRED, general-terms-next.md 4).
fn sun_terms(p: PbrInput, h: f32, power: f32, gloss: f32) -> SunTerms {
    var t: SunTerms;
    t.correction = vec3<f32>(0.0);
    t.specular = vec3<f32>(0.0);
    let n_lights = view_bindings::lights.n_directional_lights;
    for (var i: u32 = 0u; i < n_lights; i = i + 1u) {
        let l = &view_bindings::lights.directional_lights[i];
        if !is_sun((*l).direction_to_light, horizon.sun) {
            continue;
        }
        let NdotV = max(dot(p.N, p.V), 0.0001);
        let roughness = lighting::perceptualRoughnessToRoughness(p.material.perceptual_roughness);
        let base = p.material.base_color.rgb;
        var li: lighting::LightingInput;
        li.layers[LAYER_BASE].NdotV = NdotV;
        li.layers[LAYER_BASE].N = p.N;
        li.layers[LAYER_BASE].R = reflect(-p.V, p.N);
        li.layers[LAYER_BASE].perceptual_roughness = p.material.perceptual_roughness;
        li.layers[LAYER_BASE].roughness = roughness;
        li.P = p.world_position.xyz;
        li.V = p.V;
        li.diffuse_color = calculate_diffuse_color(
            base, p.material.metallic, p.material.specular_transmission,
            p.material.diffuse_transmission);
        li.metallic = p.material.metallic;
        li.F0_dielectric = calculate_F0_dielectric(p.material.reflectance);
        li.F0_metallic = base;
        li.F_ab = lighting::F_AB(p.material.perceptual_roughness, NdotV);
        var cascade = 1.0;
        if (p.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && ((*l).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = dot(vec4<f32>(
                view.view_from_world[0].z, view.view_from_world[1].z,
                view.view_from_world[2].z, view.view_from_world[3].z), p.world_position);
            cascade = shadows::fetch_directional_shadow(
                i, p.world_position, p.world_normal, view_z, p.frag_coord.xy);
        }
        let unshadowed = lighting::directional_light(i, &li, true);
        let shadow = (1.0 + general.instance.x * (h - 1.0)) * (0.1 + 0.9 * cascade);
        t.correction = view.exposure * unshadowed * (shadow - cascade);
        // H = normalize(V + L), no N.L; (1 - v6.w) is the LOD fade
        let hv = normalize(p.V + (*l).direction_to_light);
        let s = pow(max(dot(p.N, hv), 0.001), power);
        t.specular = light.specular.rgb * (shadow * s * gloss * (1.0 - general.instance.y));
        return t;
    }
    return t;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var base = pbr_input.material.base_color;
    var uv0 = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv0 = in.uv;
#endif

    // UseChannelTextures: the diffuse becomes dot(t0, ChannelTextureMask), alpha 1
    if (general.flags & SHADER_CHANNEL_TEXTURES) != 0u {
        let raw = textureSampleBias(
            pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv0, view.mip_bias);
        var vertex_colour = vec4<f32>(1.0);
#ifdef VERTEX_COLORS
        vertex_colour = in.color;
#endif
        base = vec4<f32>(
            vec3<f32>(dot(raw, general.channel_texture_mask)) * vertex_colour.rgb,
            1.0);
    } else {
        // the diffuse alpha is the texture's: the vertex colour alpha is not read at all
        // (VS `mov o9.w, cb1[11].w`), Bevy's base colour carries it, so take the texture alone (the loader's base colour alpha is 1)
        let raw = textureSampleBias(
            pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv0, view.mip_bias);
        base = vec4<f32>(base.rgb, raw.a);
    }
    // instance alpha (VS cb1[11].w -> v9.w): alpha test and output alpha use diffuse.a * it
    base = vec4<f32>(base.rgb, base.a * general.instance.z);

    var ao_r = 1.0;
    // t3 at UV1, optionally one channel (UseChannelAO), multiplies the albedo
#ifdef VERTEX_UVS_B
    if (general.flags & SHADER_HAS_AO) != 0u {
        var t3 = textureSampleBias(ao_tex, ao_sampler, in.uv_b, view.mip_bias).rgb;
        if general.channel_ao.w > 0.5 {
            t3 = vec3<f32>(dot(t3, general.channel_ao.xyz));
        }
        ao_r = t3.r;
        base = vec4<f32>(base.rgb * t3, base.a);
    }
#endif

    // t2: MPM (R environment reflection, G specular, B lighting remap)
    var mpm = vec3<f32>(0.0);
    if (general.flags & SHADER_HAS_MPM) != 0u {
        mpm = textureSampleBias(mpm_tex, mpm_sampler, uv0, view.mip_bias).rgb;
    }

    // alpha test (render flag 0x100): strictly below the reference is discarded
    pbr_input.material.base_color = alpha_discard(pbr_input.material, base);
    base = pbr_input.material.base_color;

    // t11: AO (x) and the horizon heights (y, w); the ambient term is multiplied by x
    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    pbr_input.diffuse_occlusion = pbr_input.diffuse_occlusion * t11.x;
    let h = horizon_shadow(t11, horizon.cb0, horizon.sun);
    let sun = sun_terms(pbr_input, h, general.specular_power, mpm.g);

    var out: FragmentOutput;
    let lit = apply_pbr_lighting(pbr_input);
    // the hemisphere part of the native ambient (the uniform AmbientLight is its mean)
    let diffuse = calculate_diffuse_color(
        base.rgb, pbr_input.material.metallic, pbr_input.material.specular_transmission,
        pbr_input.material.diffuse_transmission);
    var rgb = lit.rgb + sun.correction + ambient_tilt(
        diffuse, pbr_input.N, horizon.tilt, pbr_input.diffuse_occlusion, view.exposure);
    // MPM B: C (L + B (2 - L)) = C L + B (2 C - C L), over the lit diffuse C L
    rgb = rgb + mpm.b * (2.0 * base.rgb - rgb);
    // the native point-light loop (JC2-221, point_lights.wgsl): L joins the irradiance after the
    // B remap and before the albedo product (`E += L; colour = albedo * E`, model doc 10.3), so
    // it is multiplied by the diffuse albedo; unshadowed and without the AO texel. The lit output is
    // already in native units (sun lux = native * pi / e0 and ambient = native / e0 are scaled by
    // the view exposure e0 inside Bevy), so the native term is added without the exposure.
    rgb = rgb + diffuse * point_light_irradiance(in.world_position.xyz, pbr_input.N);
    // MPM R: sat(R (1 - sat(N.V)^8)) * cube(reflect(-V, N)) * (cb0[3] + cb0[10]) (native units,
    // no albedo), added to C L before the specular. rgb is already on the native scene scale
    // here (Bevy's lighting applied e0 to lux / ambient that were divided by it), so the native
    // colours are added as is, with no view.exposure (JC2-236: that factor made it ~1000x dark)
    if (general.flags & SHADER_HAS_ENV) != 0u {
        let n_dot_v = saturate(dot(pbr_input.N, pbr_input.V));
        let n2 = n_dot_v * n_dot_v;
        let n4 = n2 * n2;
        let weight = saturate(mpm.r * (1.0 - n4 * n4));
        let cube = textureSampleLevel(
            env_tex, env_sampler, reflect(-pbr_input.V, pbr_input.N), 0.0).rgb;
        rgb = rgb + weight * cube * (light.sun.rgb + light.sky_ambient.rgb);
    }
    rgb = rgb + sun.specular;
    // snow (render flag 0x40 gate in the Rust loader; instance byte +0xc = f, 0 = off):
    // lerp(rgb, cb1[1].rgb * L, s). APPROXIMATION: L is taken as rgb / albedo (Bevy's lit
    // colour includes the albedo); no producer writes f yet (Gap 1), so s = 0 today.
    let f = general.instance.w;
    if f > 0.0 {
        var snow_rgb = vec3<f32>(0.508, 0.54, 0.645);
        var snow_w = 1.0 - 2.0 * f;
        var sign = 1.0;
        if f >= 0.5 {
            snow_rgb = vec3<f32>(0.2932, 0.2011, 0.1247);
            snow_w = 1.0 - 2.0 * (f - 0.5);
            sign = -1.0;
        }
        let fade = general.instance.y;
        let x = saturate(sign * fade * fade + ao_r);
        let s = saturate((pbr_input.N.y + x - 1.7 - snow_w) * 5.0);
        let l = rgb / max(base.rgb, vec3<f32>(0.001));
        rgb = mix(rgb, snow_rgb * l, s);
    }
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    rgb = haze_fog(rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour);
    // instance tint after the fog: fogged * (1 - a) + tint (already native units, no exposure)
    rgb = mix(rgb, general.tint.rgb, general.tint.a);
    out.color = vec4<f32>(rgb, lit.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
