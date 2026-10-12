// Grass: StandardMaterial lighting (with the terrain's normal on every vertex, so the
// blades shade like the ground under them) plus
//   * wind: the vertex shader sways a blade's tip by its weight (uv_b.x = height along
//     the blade × the type's sway factor, uv_b.y = the blade's phase byte), and
//   * a distance fade through alpha-to-coverage (the alpha fades out between
//     `range.x` and `range.y` metres from the camera).
// See crates/jc2-game/src/grass.rs.

#import jc2::haze_fog::haze_fog
#import jc2::sun_horizon::{ao_uv, horizon_shadow}
#import jc2::sun_correction::sun_correction
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_view_bindings::{view, globals},
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_diffuse_color},
}

struct GrassFade {
    // x: fade start, y: fade end
    range: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> grass: GrassFade;

// The shared per-frame constants (sun_horizon.rs); grass reads only the haze fog fields
// (haze.rs, haze_fog.wgsl), all zero = no fog. The native `grass` / `grass_lod*` pixel shaders end
// with `exp(-v4.w)`, `min(., 1)` and the mix toward `cb0[13].xyz` (shader bundle PS 6102356).
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> horizon: SunHorizon;

// t11: the terrain AmbientOcclusionMap (x = AO, y / w = horizon heights), bound by
// `sun_horizon::add_ao_binding`. The native grass pixel shaders (PS 6102356, `grass_lod0/1/2`)
// multiply the ambient by `t11.x` and the Sun by `cascade * horizon * (t11.x / 2 + 1/2)`.
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var horizon_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var horizon_sampler: sampler;

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

    // ambient * t11.x (native; JC2-186 does the same for the hemisphere models)
    let t11 = textureSample(horizon_tex, horizon_sampler, ao_uv(in.world_position.xz));
    pbr_input.diffuse_occlusion = pbr_input.diffuse_occlusion * t11.x;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // The hemisphere part of the native ambient (JC2-133): the grass vertex shaders (`grass`,
    // `grass_lod0/1`, bundle VS 194644 ff.) write lerp(ground, sky, sat(n.y / 2 + 1/2 - 5 sway))
    // into TEXCOORD1 and the pixel shader multiplies it by the AO. Bevy's uniform AmbientLight is
    // the mean of the two ambients; this adds `(sky - ground) / 2 * N.y` (sun_horizon.wgsl
    // `ambient_tilt`). N is the terrain normal every vertex carries; the native `n.y` is the
    // blade's own normal minus ten times the wind sway, not reproduced (gap in the research doc).
    // The grass pixel shader reads the AO map only through the factor here, which Bevy applies.
    let diffuse = calculate_diffuse_color(
        pbr_input.material.base_color.rgb, pbr_input.material.metallic,
        pbr_input.material.specular_transmission, pbr_input.material.diffuse_transmission);
    // the Sun: horizon shadow, `t11.x / 2 + 1/2` and the cascade floor 0.1 (sun_correction.wgsl);
    // the grass pixel shaders carry the cascade block (probe census: grass cascade=4 of 4)
    let sun_factor = horizon_shadow(t11, horizon.cb0, horizon.sun) * (t11.x * 0.5 + 0.5);
    out.color = vec4<f32>(
        out.color.rgb + sun_correction(pbr_input, sun_factor, horizon.sun, true)
            + view.exposure * diffuse * horizon.tilt.xyz * pbr_input.N.y * pbr_input.diffuse_occlusion,
        out.color.a);
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    out.color = vec4<f32>(
        haze_fog(out.color.rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour),
        out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
