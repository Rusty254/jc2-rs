// Tree impostors (crates/jc2-game/src/impostors.rs): every quad is stored as its centre
// (POSITION), the tree's yaw (NORMAL.xy = cos, sin) and the corner's offset in the quad's
// own plane in metres (UV_1); the vertex shader builds the quad.
//
// An upright card turns about the vertical to face the camera (trunks and the broadleaf
// crowns, which are side views); a card whose NORMAL.z is 1 (palm and pine crowns, views
// from above) lies on the ground, turned by the tree's yaw.
//
// Distance fade (JC2-140, `veglod::card_alpha`): the native TreeImpostorTop / TreeImpostorTrunk
// vertex shaders compute `alpha = min(1 - sat(cheb * w - z), sat(euclid * y - x)) * 3 + 0.99`
// from the card centre and clip the whole card when it is below 0.996078; the pixel shader
// outputs `texture alpha * alpha - 0.996078`. The constants `(x, y, z, w)` are
// `VegetationFade::top` / `trunk` (camera FOV factor included). The card centre is this
// project's quad centre (patch position + the table's y offset), not a recovered native vertex.
//
// Fragment: StandardMaterial lighting with a normal that points up. The cut-out stays the
// provisional `Mask(0.4)` (the native blend / coverage state is unresolved, JC2-126 Gap 4).

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
    // the native `cb1[1]` of this card kind (see above); all zero = no distance fade
    fade: vec4<f32>,
}

const CARD_CLIP: f32 = 0.996078;

fn card_alpha(centre: vec3<f32>) -> f32 {
    if impostor.fade.w == 0.0 {
        return 3.99;
    }
    let d = centre - view.world_position;
    let cheb = max(abs(d.y) * 0.0625, max(abs(d.x), abs(d.z)));
    var euclid = length(d);
    if impostor.mode.x < 0.5 {
        euclid = length(vec3<f32>(d.x, d.y * 0.25, d.z));
    }
    let far = 1.0 - saturate(cheb * impostor.fade.w - impostor.fade.z);
    let near = saturate(euclid * impostor.fade.y - impostor.fade.x);
    return min(far, near) * 3.0 + 0.99;
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
    let alpha = card_alpha(centre);
    if alpha < CARD_CLIP {
        // clipped like the native `z := -w`: nothing of the card is left
        out.position.z = -out.position.w;
    }
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_UVS_B
    // the corner offset is not needed past this point: carry the card alpha instead
    out.uv_b = vec2<f32>(alpha, 0.0);
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
#ifdef VERTEX_UVS_B
    // native pixel shader: texture alpha * card alpha - 0.996078
    pbr_input.material.base_color.a = saturate(pbr_input.material.base_color.a * in.uv_b.x - CARD_CLIP);
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
