// Effect particle quads (crates/jc2-game/src/effects/particle_render.rs): the native pixel stage
// of the particle shaders (research/formats/particle-units-next.md section 4).
//
// Mesh layout: POSITION = the corner (already placed on the camera right / up axes), UV_0 = the
// atlas uv, COLOR = (particle colour * lit, alpha * distance fade * near fade).
//
//   colour = clamp01(4 * tex.rgb * vertex.rgb)
//   a      = clamp01(tex.a * vertex.a)
//   default blend (SRC_ALPHA, INV_SRC_ALPHA):  out = (colour, a^1.18)
//   flag bit7 blend (INV_DEST_COLOR, ONE):     out = colour * 1.35 * a^2
// The bit7 pixel shader (bundle offset 6734720) is INFERRED to be the one those leaves use.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}
#ifdef DISTANCE_FOG
#import bevy_pbr::{mesh_view_bindings::fog, pbr_functions::apply_fog}
#endif

struct ParticleParams {
    // x: 1 for the screen-blend (flag bit7) leaves
    mode: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ParticleParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var particle_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var particle_sampler: sampler;

struct ParticleVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) colour: vec4<f32>,
}

struct ParticleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
}

@vertex
fn vertex(v: ParticleVertex) -> ParticleOut {
    var out: ParticleOut;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    out.world = world.xyz;
    out.clip = position_world_to_clip(world.xyz);
    out.uv = v.uv;
    out.colour = v.colour;
    return out;
}

@fragment
fn fragment(in: ParticleOut) -> @location(0) vec4<f32> {
    let t = textureSample(particle_texture, particle_sampler, in.uv);
    let c = clamp(4.0 * t.rgb * in.colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let a = clamp(t.a * in.colour.a, 0.0, 1.0);
    if params.mode.x > 0.5 {
        return vec4<f32>(c * 1.35 * a * a, 0.0);
    }
    var out = vec4<f32>(c, pow(a, 1.18));
#ifdef DISTANCE_FOG
    out = apply_fog(fog, out, in.world, view.world_position.xyz, in.clip.xy);
#endif
    return out;
}
