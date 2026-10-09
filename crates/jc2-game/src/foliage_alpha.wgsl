#define_import_path jc2::foliage_alpha

#import bevy_pbr::{mesh_functions, mesh_view_bindings::view}

struct FoliageParams {
    enabled: u32,
    instance_fraction: f32,
    reference: f32,
    reserved: f32,
    // x, y of the native `1 - sat(dist * y - x)`; z = 1 when the fade is active
    fade: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;

// The VegetationFoliage / BillboardFoliage vertex shaders multiply
// `1 - sat(dist * cb1[1].y - cb1[1].x)` into the instance fade `cb2[16].x`, with `dist` the
// length of (model-origin x/z, transformed vertex y) minus the camera (JC2-126). The native
// evaluates it per vertex, this per fragment (non-native, same function of position).
// Directional shadow cascades (orthographic views) have no camera position here, so they
// skip the fade (Gap): `view.world_position` would be the light's.
fn foliage_distance_fade(world_position: vec3<f32>, instance_index: u32) -> f32 {
    if foliage.fade.z == 0.0 || view.clip_from_view[3][3] == 1.0 {
        return 1.0;
    }
    let origin = mesh_functions::get_world_from_local(instance_index)[3].xyz;
    let d = vec3<f32>(origin.x, world_position.y, origin.z) - view.world_position;
    return 1.0 - saturate(length(d) * foliage.fade.y - foliage.fade.x);
}

// 009672a0 / 00949aa0: main ordinary shader doubles the texture alpha, then multiplies the
// instance/distance fade (`cb2[16].x`); the discard is strict `<` against the reference.
// Fade 0 clips the vertex natively: nothing of the mesh is left there.
fn foliage_should_discard(texture_alpha: f32, fade: f32) -> bool {
    let scaled_alpha = (texture_alpha + texture_alpha) * foliage.instance_fraction * fade;
    return fade <= 0.0 || (foliage.enabled != 0u && scaled_alpha < foliage.reference);
}
