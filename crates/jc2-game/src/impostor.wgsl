// Tree impostors (crates/jc2-game/src/impostors.rs): every quad is stored as its centre
// (POSITION), the tree's yaw (NORMAL.xy = cos, sin) and the corner's offset in the quad's
// own plane in metres (UV_1); the vertex shader builds the quad.
//
// An upright card turns about the vertical to face the camera (trunks and the broadleaf
// crowns, which are side views); a card whose NORMAL.z is 1 (palm and pine crowns, views
// from above) lies on the ground, turned by the tree's yaw.
//
// Fragment: StandardMaterial lighting with a normal that points up, alpha-to-coverage.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, visibility_range_dither},
    mesh_view_bindings::view,
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct ImpostorParams {
    // x: 0 trunk, 1 crown (the brightness of the material differs)
    mode: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> impostor: ImpostorParams;

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let centre = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0)).xyz;
    let to_cam = view.world_position - centre;
    let dist = max(length(to_cam), 0.001);
    let dir = to_cam / dist;
    let flat_len = max(length(vec2<f32>(to_cam.x, to_cam.z)), 0.001);
    let across = vec3<f32>(to_cam.z / flat_len, 0.0, -to_cam.x / flat_len);
    let corner = v.uv_b;
    var offset = across * corner.x + vec3<f32>(0.0, 1.0, 0.0) * corner.y;
    if v.normal.z > 0.5 {
        let lie = 1.0;
        let ax = vec3<f32>(v.normal.x, 0.0, v.normal.y);
        let az = vec3<f32>(-v.normal.y, 0.0, v.normal.x);
        let flat_offset = ax * corner.x - az * corner.y;
        offset = mix(offset, flat_offset, lie);
    }
    let world = centre + offset;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = v.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = v.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = v.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        v.instance_index, world_from_local[3]);
#endif
    return out;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
