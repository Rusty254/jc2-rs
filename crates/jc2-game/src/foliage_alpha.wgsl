#define_import_path jc2::foliage_alpha

struct FoliageParams {
    enabled: u32,
    instance_fraction: f32,
    reference: f32,
    reserved: f32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;

// 009672a0 / 00949aa0: distance factor is explicitly1 until086 recovers
// its per-asset inputs and scheduler. Do not divide by fade or hide byte0.
fn foliage_should_discard(texture_alpha: f32) -> bool {
    let scaled_alpha = (texture_alpha + texture_alpha) * foliage.instance_fraction;
    return foliage.enabled != 0u && scaled_alpha < foliage.reference;
}
