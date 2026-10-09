#import jc2::foliage_alpha::{foliage_should_discard, foliage_distance_fade}
#import jc2::haze_fog::haze_fog
#import bevy_pbr::{
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

// The shared per-frame constants (sun_horizon.rs); foliage reads only the haze fog fields
// (haze.rs, haze_fog.wgsl), all zero = no fog.
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> horizon: SunHorizon;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var input = pbr_input_from_standard_material(in, is_front);
    let fade = foliage_distance_fade(in.world_position.xyz, in.instance_index);
    if foliage_should_discard(input.material.base_color.a, fade) { discard; }
    // The common native test is the only alpha cutoff. The material remains in
    // the masked phase to run the same shader contract for depth and shadows.
    input.material.base_color.a = 1.0;
    var out: FragmentOutput;
    if (input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(input);
    } else {
        out.color = input.material.base_color;
    }
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    out.color = vec4<f32>(
        haze_fog(out.color.rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour),
        out.color.a);
    out.color = main_pass_post_lighting_processing(input, out.color);
    return out;
}
