// Grass: StandardMaterial lighting (with the terrain's normal on every vertex, so the
// blades shade like the ground under them) plus
//   * wind: the vertex shader sways a blade's tip by its weight (uv_b.x = height along
//     the blade × the type's sway factor, uv_b.y = the blade's phase byte), and
//   * a distance fade through alpha-to-coverage (the alpha fades out between
//     `range.x` and `range.y` metres from the camera).
// See crates/jc2-game/src/grass.rs.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_view_bindings::{view, globals},
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct GrassFade {
    // x: fade start, y: fade end
    range: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> grass: GrassFade;

// how far a full-weight tip travels (metres per unit of sway weight)
const SWAY_REACH: f32 = 7.0;

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
#ifdef VERTEX_UVS_B
    // two out-of-phase waves, running across the field
    let t = globals.time;
    let phase = v.uv_b.y * 0.11;
    let w = v.uv_b.x * SWAY_REACH;
    world.x += w * (sin(t * 1.9 + phase + world.z * 0.08) + 0.5 * sin(t * 3.1 + world.x * 0.15));
    world.z += w * 0.6 * sin(t * 1.4 + phase * 1.3 + world.x * 0.07);
#endif
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
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
    return out;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let dist = distance(in.world_position.xyz, view.world_position);
    let fade = 1.0 - smoothstep(grass.range.x, grass.range.y, dist);
    pbr_input.material.base_color.a = pbr_input.material.base_color.a * fade;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
