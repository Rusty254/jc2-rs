// Sky sprites (crates/jc2-game/src/clouds.rs): clouds and cirrus as cards that always
// face the camera. Each card is stored as its centre in WORLD space (POSITION), a drift
// (NORMAL.xy: metres per second; NORMAL.z: the card's own opacity), the atlas uv (UV_0)
// and the corner's offset in metres (UV_1). The vertex shader wraps the field around the
// camera (a field of `range.x` metres repeats) so it never ends, and builds the card.
// The fragment fades the cards out near the camera and applies the tint.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_view_bindings::{view, globals},
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct SkySprite {
    // x: the repeat of the field (m), y: fade-in start, z: fade-in end (m from the camera),
    // w: far fade start (m); the far end is 1.25 x that
    range: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> sprite: SkySprite;

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var centre = v.position;
    let drift = vec2<f32>(v.normal.x, v.normal.y) * globals.time;
    let cam = view.world_position;
    // wrap the field's x, z around the camera
    let rep = sprite.range.x;
    let rel = centre.xz + drift - cam.xz;
    let wrapped = rel - rep * floor(rel / rep + 0.5);
    centre = vec3<f32>(cam.x + wrapped.x, centre.y, cam.z + wrapped.y);
    let to_cam = cam - centre;
    let dir = normalize(to_cam);
    var right = cross(vec3<f32>(0.0, 1.0, 0.0), dir);
    right = normalize(right + vec3<f32>(0.0001, 0.0, 0.0));
    let up = cross(dir, right);
    let world = centre + right * v.uv_b.x + up * v.uv_b.y;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = dir;
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_UVS_B
    // x: the card's opacity, y: its distance to the camera
    out.uv_b = vec2<f32>(v.normal.z, length(to_cam));
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
    var fade = 1.0;
#ifdef VERTEX_UVS_B
    let d = in.uv_b.y;
    fade = in.uv_b.x
        * smoothstep(sprite.range.y, sprite.range.z, d)
        * (1.0 - smoothstep(sprite.range.w, sprite.range.w * 1.25, d));
#endif
    pbr_input.material.base_color.a = pbr_input.material.base_color.a * fade;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
