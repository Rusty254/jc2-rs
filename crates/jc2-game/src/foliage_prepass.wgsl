#import jc2::foliage_alpha::{foliage_should_discard, foliage_distance_fade}
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_bindings,
    pbr_types,
    pbr_functions,
    mesh_view_bindings::view,
}
#ifdef MOTION_VECTOR_PREPASS
#import bevy_pbr::pbr_prepass_functions::calculate_motion_vector
#endif

fn ordinary_alpha(in: VertexOutput) -> f32 {
    var alpha = pbr_bindings::material.base_color.a;
#ifdef VERTEX_UVS
#ifdef STANDARD_MATERIAL_BASE_COLOR_UV_B
    var uv = in.uv_b;
#else
    var uv = in.uv;
#endif
    uv = (pbr_bindings::material.uv_transform * vec3(uv, 1.0)).xy;
    if (pbr_bindings::material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        alpha = alpha * textureSampleBias(pbr_bindings::base_color_texture,
            pbr_bindings::base_color_sampler, uv, view.mip_bias).a;
    }
#endif
    return alpha;
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    let fade = foliage_distance_fade(in.world_position.xyz, in.instance_index);
    if foliage_should_discard(ordinary_alpha(in), fade, in.instance_index) { discard; }
    var out: FragmentOutput;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif
#ifdef NORMAL_PREPASS
    let double_sided = (pbr_bindings::material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    var normal = in.world_normal;
    if (pbr_bindings::material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_front);
    }
    out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
    return out;
}
#else
@fragment
fn fragment(in: VertexOutput) {
#ifdef VISIBILITY_RANGE_DITHER
    pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    let fade = foliage_distance_fade(in.world_position.xyz, in.instance_index);
    if foliage_should_discard(ordinary_alpha(in), fade, in.instance_index) { discard; }
}
#endif
