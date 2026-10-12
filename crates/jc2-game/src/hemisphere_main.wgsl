// Plain StandardMaterial models with the native hemisphere ambient (JC2-177,
// research/formats/ambient-hemisphere-next.md): the Lambert / SkinnedGeneral / CarPaint /
// Window / VegetationBark / Facade / Merged / Decal / SplineRoad / Box pixel shaders use
// lerp(cb0[11], cb0[10], sat(N.y / 2 + 1/2)); Bevy's uniform AmbientLight carries the mean, this
// adds the `tilt * N.y` remainder (sun_horizon.wgsl `ambient_tilt`). Everything else is Bevy's
// own StandardMaterial lighting and fog, unchanged.
#import jc2::sun_horizon::{ambient_tilt, ao_uv, horizon_shadow, is_sun}
#import jc2::point_lights::point_light_irradiance
#import bevy_pbr::{
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, calculate_tbn_mikktspace, apply_pbr_lighting, main_pass_post_lighting_processing, calculate_diffuse_color, calculate_F0_dielectric},
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
    mesh_view_bindings as view_bindings,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    lighting,
    lighting::LAYER_BASE,
    shadows,
}

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

// The shared per-frame constants (sun_horizon.rs); only `tilt` is read here. The all-zero
// fallback buffer (no terrain: viewer, fixtures) leaves the ambient uniform.
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> horizon: SunHorizon;

// t11: the terrain AmbientOcclusionMap (x = AO), bound by hemisphere_material.rs `bind_ao`;
// ao_flags.x = 1 for the families whose native shader multiplies the ambient by it (JC2-186),
// .y = 1 where the native Sun term is scaled by the horizon shadow, .z = 1 where it is also scaled
// by `t11.x / 2 + 1/2` (research/formats/lighting-gaps-next.md G3, `sun_terms_probe.py`).
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var horizon_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var horizon_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var<uniform> ao_flags: vec4<f32>;

// The `_Dirt` bark blend (barkpl0_dirt PS, lighting-gaps-next.md G9): t2 = dirt diffuse (alpha = mask),
// t3 = dirt normal map, both at UV1. dirt_flags.x = enabled, .y = t3 present.
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var dirt_diffuse_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var dirt_diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var dirt_normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var dirt_normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var<uniform> dirt_flags: vec4<f32>;

// The Sun's direct term of the native pixel shaders: `cb0[3] * sat(N.-L) * shadow`, with
// `shadow = horizon * (floor + (1 - floor) * cascade)` and, for the families that have it, `* (t11.x / 2 + 1/2)`.
// Bevy sums all its lights inside apply_pbr_lighting with the cascade alone, so the Sun's own
// contribution is evaluated again and the difference added (the terrain and General routes do the
// same). The floor 0.1 is `cb0[9].w` = `[shadow object + 0x1f4]`, initialised to 0.1 (`FUN_0091a6e0`),
// INFERRED as the only value (general-terms-next.md section 4).
fn sun_correction(p: PbrInput, factor: f32) -> vec3<f32> {
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
        var floored = 1.0;
        if (p.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && ((*l).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = dot(vec4<f32>(
                view.view_from_world[0].z, view.view_from_world[1].z,
                view.view_from_world[2].z, view.view_from_world[3].z), p.world_position);
            cascade = shadows::fetch_directional_shadow(
                i, p.world_position, p.world_normal, view_z, p.frag_coord.xy);
            floored = 0.1 + 0.9 * cascade;
        }
        let unshadowed = lighting::directional_light(i, &li, true);
        return view.exposure * unshadowed * (factor * floored - cascade);
    }
    return vec3<f32>(0.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_B
    if dirt_flags.x > 0.5 {
        // diffuse = mix(t0, t2.rgb, t2.a)
        let dirt = textureSample(dirt_diffuse_tex, dirt_diffuse_sampler, in.uv_b);
        input.material.base_color = vec4<f32>(
            mix(input.material.base_color.rgb, dirt.rgb, dirt.a), input.material.base_color.a);
#ifdef VERTEX_TANGENTS
        if dirt_flags.y > 0.5 {
            // n = normalize(n0.xy / |n0| + 2 t3.xy - 1, 2 t3.z - 1) with n0 the bark normal map's
            // tangent-space vector, recovered from the already mapped N (no Y flip is set on this
            // material, so the TBN components are the texture's own); the native z is t3 alone.
            let t3 = textureSample(dirt_normal_tex, dirt_normal_sampler, in.uv_b).rgb;
            let tbn = calculate_tbn_mikktspace(input.world_normal, in.world_tangent);
            let n0 = vec3<f32>(dot(input.N, tbn[0]), dot(input.N, tbn[1]), dot(input.N, tbn[2]));
            let nt = normalize(vec3<f32>(n0.xy / length(n0) + 2.0 * t3.xy - 1.0, 2.0 * t3.z - 1.0));
            input.N = normalize(nt.x * tbn[0] + nt.y * tbn[1] + nt.z * tbn[2]);
        }
#endif
    }
#endif
    // The native alpha test (`FUN_00949aa0(5, ..)`: discard below the reference, research
    // rico-eyes.md section 1/4): this shader replaces `pbr.wgsl`, which is where Bevy calls
    // `alpha_discard`, so `AlphaMode::Mask` was inert on this route (hair and lash cards drew opaque).
    input.material.base_color = alpha_discard(input.material, input.material.base_color);
    // `ao_flags.w` > 0: the explicit reference of a BLENDED alpha-tested pass (the second hair pass,
    // reference 3/510), which Bevy's `Blend` mode has no cutoff for.
    if ao_flags.w > 0.0 && input.material.base_color.a < ao_flags.w {
        discard;
    }
    // ambient * t11.x (native; the sample is unconditional, the family flag blends it in)
    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    input.diffuse_occlusion = input.diffuse_occlusion * mix(1.0, t11.x, ao_flags.x);
    var out: FragmentOutput;
    if (input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(input);
        let diffuse = calculate_diffuse_color(
            input.material.base_color.rgb, input.material.metallic,
            input.material.specular_transmission, input.material.diffuse_transmission);
        // the native point-light loop (JC2-221): added to the irradiance, so multiplied by the
        // albedo like the sun and hemisphere terms (model doc 10.3; the characters' `t2.x`
        // scale and the car paint / window chains differ in detail: Gaps)
        // the Sun's horizon shadow (every family that samples t11 but the decals) and the
        // `t11.x / 2 + 1/2` factor (bark only), and the cascade floor
        let sun_factor = mix(1.0, horizon_shadow(t11, horizon.cb0, horizon.sun), ao_flags.y)
            * mix(1.0, t11.x * 0.5 + 0.5, ao_flags.z);
        out.color = vec4<f32>(
            out.color.rgb + sun_correction(input, sun_factor) + ambient_tilt(
                diffuse, input.N, horizon.tilt, input.diffuse_occlusion, view.exposure)
                + diffuse * point_light_irradiance(in.world_position.xyz, input.N),
            out.color.a);
    } else {
        out.color = input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(input, out.color);
    return out;
}
