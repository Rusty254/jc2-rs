// BillboardFoliage vertex stage (board f012a36f, research/formats/billboard-foliage-next.md).
// Native VS `billboardfoliage` (shader bundle DX10_Shaders_F, 128 lines), the shadow variant
// `billboardfoliageshadow` and the depth variant run the same card construction; this file ports
// it for the main pass and for Bevy's depth / shadow prepass (the same entry point, selected by
// the `PREPASS_PIPELINE` shader def). Only blocks of type BillboardFoliage select this vertex
// stage (`FoliageExtension::specialize`).
//
// Mesh layout (rbm.rs `parse_billboard_foliage`): POSITION = centre + the corner offset in the
// XY plane (so the bounds have area), UV_1 = the corner offset (the native `v1.zw`), UV_0 = the
// texture coordinate. The centre is POSITION - (offset, 0).
//
// Outputs for the pixel stage (foliage_main.wgsl `billboard_light`), carried in `uv_b` of the
// main pass: x = native `o4.x` (fake sphere light times the corner term), y = native `o4.y`
// (pivot-distance ramp that scales the colour).
#import jc2::foliage_alpha::{foliage_billboard_centroid, foliage_fade_constants}
#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{Vertex, VertexOutput}
#else
#import bevy_pbr::forward_io::{Vertex, VertexOutput}
#endif

// The same shared per-frame constants as foliage_main.wgsl; here `sun.xyz` (the unit vector
// towards the Sun, 0 = none: the negated native `cb0[8]`) and `tilt.w` (the elapsed seconds,
// sun_horizon.rs: Bevy binds no `globals` in the depth / shadow prepass views).
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> horizon: SunHorizon;

// cb1[2] of the main / depth / shadow setups (0x93deb0, 0x91bb30): x = render-context time * 2
// (the context field +0xcd4; INFERRED to be the game clock in seconds, the rewrite uses Bevy's
// elapsed time, `horizon.tilt.w`), y = 0.03 (a literal in the setup code).
const SWAY_RATE: f32 = 2.0;
const SWAY_AMPLITUDE: f32 = 0.03;
// cb2[6].xyz and cb2[7].xy: the block constructor 0x970b20 stores (0, 5, 0) at +0x34 and (10, 10)
// at +0x40; the pass draw 0x954340 uploads them as written (no other writer found).
const PIVOT_OFFSET: vec3<f32> = vec3<f32>(0.0, 5.0, 0.0);
const CROWN_SCALE: vec2<f32> = vec2<f32>(10.0, 10.0);

// The vertex shader's sway: the angle (x + y - z + 2 t) / 2pi wrapped to [-pi, pi) and a seventh
// order Taylor sine / cosine (the same 0.159155, 6.28319, -3.14159 and series constants as the
// disassembly, so the error at the ends of the range is the original's). Returns (sin, cos).
fn sway(p: vec3<f32>) -> vec2<f32> {
    let t = (p.x + p.y - p.z + horizon.tilt.w * SWAY_RATE) * 0.159155;
    let x = fract(t) * 6.28319 - 3.14159;
    let x2 = x * x;
    let x3 = x * x2;
    let x4 = x3 * x;
    let x5 = x * x4;
    let x6 = x5 * x;
    let x7 = x * x6;
    let s = x - x3 * 0.166667 + x5 * 0.00833333 - x7 * 0.000198413;
    let c = 1.0 - x2 * 0.5 + x4 * 0.0416667 - x6 * 0.00138889;
    return vec2<f32>(s, c);
}

struct Card {
    world: vec3<f32>,
    // direction from the card to the viewpoint (the native `o2.xyz` = -view direction)
    facing: vec3<f32>,
    // native o4.x and o4.y
    light: f32,
    ramp: f32,
}

fn card(position: vec3<f32>, corner: vec2<f32>, instance_index: u32) -> Card {
    let world_from_local = mesh_functions::get_world_from_local(instance_index);
    let c = (world_from_local * vec4<f32>(position - vec3<f32>(corner, 0.0), 1.0)).xyz;
    // The viewpoint: the main camera and its up vector (render-context matrix row 1, +0xc78),
    // or, in a directional shadow cascade (orthographic), the far virtual camera of the native
    // shadow variant (centre - (cb0[4] + cb0[8] * 10000): along the light) and world up (the
    // shadow setup's cb1[0] source is unresolved, INFERRED).
    var cam_pos = view.world_position;
    var cam_up = view.world_from_view[1].xyz;
    var v = normalize(c - cam_pos);
    let ortho = view.clip_from_view[3][3] == 1.0;
    if ortho {
        v = view.world_from_view[2].xyz;
        cam_up = vec3<f32>(0.0, 1.0, 0.0);
    }
    let s = sway(c);
    let up = normalize(cam_up + SWAY_AMPLITUDE * vec3<f32>(s.x, 0.0, s.y));
    let right = cross(up, v);
    let top = cross(v, right);
    let offset = corner.x * right + corner.y * top;
    let world = c + offset;

    var out: Card;
    out.world = world;
    out.facing = -v;
    out.light = 1.0;
    out.ramp = 1.0;
#ifndef PREPASS_PIPELINE
    let origin = world_from_local[3].xyz;
    let to_light = horizon.sun.xyz;
    // the fake sphere light: the card's position in the crown (a sphere about 5 m above the model
    // origin, 10 m radius), as a normal that bulges towards the viewer
    let pivot = origin + PIVOT_OFFSET;
    let r = world - pivot;
    let vc = normalize(cam_pos - pivot);
    let right4 = normalize(cross(cam_up, vc));
    let rx = dot(right4, r) / CROWN_SCALE.x;
    let ry = dot(cam_up, r) / CROWN_SCALE.y;
    let n = normalize(cross(vc, right4) * ry + (rx * right4 + 0.25 * vc));
    let sphere = saturate((dot(n, to_light) + 1.0) * 0.5);
    // the corner term (corners towards the Sun are brighter), faded out with the distance fade
    let corner_dir = normalize(-offset);
    let corner_light = max(dot(corner_dir, -to_light) * 0.5 + 0.5, 0.0);
    let fade = foliage_fade_constants();
    let dist = length(vec3<f32>(origin.x, world.y, origin.z) - cam_pos);
    let near_fade = saturate((dist * fade.y - fade.x) * 2.0 + 1.0);
    out.light = mix(corner_light, 1.0, near_fade) * sphere;
    // the colour ramp: darker towards the centroid of the model's card centres
    let centroid = (world_from_local * vec4<f32>(foliage_billboard_centroid(), 1.0)).xyz;
    out.ramp = saturate(length(centroid - world) / CROWN_SCALE.x);
#endif
    return out;
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var corner = vec2<f32>(0.0);
#ifdef VERTEX_UVS_B
    corner = v.uv_b;
#endif
    let b = card(v.position, corner, v.instance_index);
    out.world_position = vec4<f32>(b.world, 1.0);
    out.position = position_world_to_clip(b.world);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vec2<f32>(b.light, b.ramp);
#endif
#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = b.facing;
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
#endif
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.previous_world_position = out.world_position;
#endif
#else
    out.world_normal = b.facing;
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
#endif
#endif
#ifdef VERTEX_COLORS
    out.color = v.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = v.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        v.instance_index, mesh_functions::get_world_from_local(v.instance_index)[3]);
#endif
    return out;
}
