#define_import_path jc2::sun_correction

// The Sun's direct term of the native pixel shaders for the custom-WGSL routes that keep Bevy's
// lighting (grass, tree impostors; the same maths as `sun_correction` in hemisphere_main.wgsl and
// terrain_splat.wgsl):
//
//   sun = cb0[3] * N.L * horizon * cascade * (t11.x / 2 + 1/2)
//   cascade term = sat(avg * (1 - f) + f), f = cb0[9].w = 0.1 (the grass PS, lines 1 to 24)
//
// Bevy sums all its lights inside apply_pbr_lighting with the bare cascade, so the Sun's own
// contribution is evaluated again and the difference added. `factor` is the horizon / AO-half
// product; `cascaded` is false for a native pixel shader without the cascade block (the tree
// impostors: the census has `cascade=0`), in which case Bevy's cascade is cancelled instead of
// floored. The floor 0.1 is `cb0[9].w`, INFERRED as the only value (general-terms-next.md
// section 4, confirmed written every frame by `0x007c6ba0`, shadow-cascades-next.md).
//
// `sun` is `SunHorizon::sun` (xyz: direction to the Sun, 0 = no terrain / no Sun constants).

#import jc2::sun_horizon::is_sun
#import bevy_pbr::{
    mesh_view_bindings::view,
    pbr_functions::{calculate_diffuse_color, calculate_F0_dielectric},
    pbr_types::PbrInput,
    mesh_view_bindings as view_bindings,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    lighting,
    lighting::LAYER_BASE,
    shadows,
}

fn sun_correction(p: PbrInput, factor: f32, sun: vec4<f32>, cascaded: bool) -> vec3<f32> {
    let n_lights = view_bindings::lights.n_directional_lights;
    for (var i: u32 = 0u; i < n_lights; i = i + 1u) {
        let l = &view_bindings::lights.directional_lights[i];
        if !is_sun((*l).direction_to_light, sun) {
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
            if cascaded {
                floored = 0.1 + 0.9 * cascade;
            }
        }
        let unshadowed = lighting::directional_light(i, &li, true);
        return view.exposure * unshadowed * (factor * floored - cascade);
    }
    return vec3<f32>(0.0);
}
