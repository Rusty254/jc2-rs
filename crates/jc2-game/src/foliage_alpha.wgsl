#define_import_path jc2::foliage_alpha

#import bevy_pbr::{mesh_functions, mesh_view_bindings::view}

struct FoliageParams {
    enabled: u32,
    // LIGHT_NORMAL_MAP 1, LIGHT_AOC 2, LIGHT_NATIVE 4 (foliage_alpha.rs)
    lighting: u32,
    // PS cb1[1].xy: specular amplitude and power
    spec_amplitude: f32,
    spec_power: f32,
    // x, y of the native `1 - sat(dist * y - x)`; z = 1 when the fade is active
    fade: vec4<f32>,
    // BillboardFoliage blocks: xyz = the mean of the card centres in model space (the native block
    // field +0x28, `cb2[5].xyz`), w = 1 (0 for every other foliage block)
    billboard: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;
// The main camera's world position (veg_fade::VegetationCamera), for orthographic shadow
// cascades whose `view.world_position` is the light's (JC2-153).
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<storage, read> veg_camera: vec4<f32>;

// The native-lighting switches of the material (foliage_alpha.rs `LIGHT_*`) and PS `cb1[1].xy`.
fn foliage_lighting_flags() -> u32 {
    return foliage.lighting;
}
fn foliage_specular() -> vec2<f32> {
    return vec2<f32>(foliage.spec_amplitude, foliage.spec_power);
}
fn foliage_is_billboard() -> bool {
    return foliage.billboard.w != 0.0;
}
fn foliage_billboard_centroid() -> vec3<f32> {
    return foliage.billboard.xyz;
}
// The native `cb1[1].xy` of the distance fade; (0, 0) = inactive (the fade term is then 1).
fn foliage_fade_constants() -> vec2<f32> {
    if foliage.fade.z == 0.0 {
        return vec2<f32>(0.0);
    }
    return foliage.fade.xy;
}

// The per-instance fade byte (model-instance header +0xb), carried in the entity's MeshTag as
// `255 - byte` (an entity without a tag is the native initial byte 255), so it is per instance
// in the main, prepass and shadow passes.
fn instance_byte(instance_index: u32) -> f32 {
    return f32(255u - min(mesh_functions::get_tag(instance_index), 255u));
}

// The VegetationFoliage / BillboardFoliage vertex shaders multiply
// `1 - sat(dist * cb1[1].y - cb1[1].x)` into the instance fade `cb2[16].x`, with `dist` the
// length of (model-origin x/z, transformed vertex y) minus the camera (JC2-126). The native
// evaluates it per vertex, this per fragment (non-native, same function of position).
// Directional shadow cascades (orthographic views) use the main camera position from
// `veg_camera`, so the shadow casters fade like the native shadow variants (which run the
// same vertex fade with the main camera constants); `view.world_position` would be the light's.
fn foliage_distance_fade(world_position: vec3<f32>, instance_index: u32) -> f32 {
    if foliage.fade.z == 0.0 {
        return 1.0;
    }
    var camera = view.world_position;
    if view.clip_from_view[3][3] == 1.0 {
        camera = veg_camera.xyz;
    }
    let origin = mesh_functions::get_world_from_local(instance_index)[3].xyz;
    let d = vec3<f32>(origin.x, world_position.y, origin.z) - camera;
    return 1.0 - saturate(length(d) * foliage.fade.y - foliage.fade.x);
}

// 009672a0 / 00949aa0: main ordinary shader doubles the texture alpha, then multiplies the
// instance/distance fade (`cb2[16].x`); the discard is strict `<` against the reference.
// Fade 0 clips the vertex natively: nothing of the mesh is left there.
// The reference is `round(byte/255*128)` as `(code*2-1)/510` (foliage.rs), per instance.
fn foliage_should_discard(texture_alpha: f32, fade: f32, instance_index: u32) -> bool {
    let fraction = instance_byte(instance_index) / 255.0;
    if foliage.billboard.w != 0.0 {
        // BillboardFoliage (PS billboardfoliage / prez / shadow): `4 * texture alpha * v4.z`
        // (v4.z = distance fade * instance byte / 255) against cb4[4].x. The billboard setups
        // (0x93deb0, 0x91bb30) call the comparison-7 setter with the constant 128 only, so the
        // reference is (2 * 128 - 1) / 510 = 0.5 for every instance (no per-instance reference).
        let scaled = 4.0 * texture_alpha * fraction * fade;
        return fade <= 0.0 || (foliage.enabled != 0u && scaled < 0.5);
    }
    let reference = (round(fraction * 128.0) * 2.0 - 1.0) / 510.0;
    let scaled_alpha = (texture_alpha + texture_alpha) * fraction * fade;
    return fade <= 0.0 || (foliage.enabled != 0u && scaled_alpha < reference);
}
