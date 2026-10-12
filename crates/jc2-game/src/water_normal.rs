//! The generated water normal texture (`Q[3]`, native `water_normal_texture`; JC2-193 / JC2-194;
//! research `research/formats/water-consumers-next.md` section 2 and
//! `post-composite-families-next.md`).
//!
//! The native reflection pass composites `textures\water\water_bump.dds` (512 x 512 DXT5) four
//! times at scrolling uv into a 256 x 256 R8G8B8A8_UNORM target with 9 mips (`GenerateMips` after
//! the draw). The post composite reads it as `t5` / `t6`: RG distorts the screen lookup (heat
//! haze, underwater), alpha is the caustic pattern. This module owns the CPU side: the
//! bump-image handle (loaded once, GPU-resident only), the clock phase and the layer uv. The
//! render pass is `post_effects_render.rs` + `water_normal.wgsl`.
//!
//! Native vs inferred: the layer table, the alpha strength (1.0) and the phase clock
//! (`dt * 0.45` per frame, strict wrap above 10000, `FUN_00a7f080`) are native. The source
//! filter (linear, derivative-chosen mip, point between mips, as the other composite samplers),
//! the box `GenerateMips` filter and the reflection-pass reset of the phase (JC2-095: the
//! accumulator restarts when the pass has nothing queued above y = 200) are **not** recorded
//! and are not modelled.

use crate::Game;
use crate::forest_canopy::bc_image;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::TextureFormat;
use jc2_formats::dds::{DdsFormat, DdsImage};

/// The archive path of the bump image (the init at `FUN_00a8b220`, `Water\water_bump.dds`).
pub const BUMP_FILE: &str = "textures\\water\\water_bump.dds";

/// Side of the generated texture (`FUN_00a8b220`: 256 x 256) and its mip count (9 = full chain).
pub const NORMAL_SIZE: u32 = 256;
pub const NORMAL_MIPS: u32 = 9;

/// Phase units per simulation second (`FUN_00a7f080`: `phase = dt * 0.45 + phase`).
const PHASE_PER_SECOND: f32 = 0.45;
/// The accumulator resets to 0 when the unrounded sum exceeds this (strict).
pub const PHASE_WRAP: f32 = 10000.0;

/// The four composite layers: base uv transform `(flip_x, flip_y)` and the scroll per phase
/// unit (VS 336476).
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub const LAYERS: [(bool, bool, [f32; 2]); 4] = [
    (false, false, [0.06, 0.05]),
    (true, false, [-0.03, -0.07]),
    (false, true, [-0.04, 0.07]),
    (false, false, [-0.07, 0.03]),
];

/// The bump image once loaded (render world only; the CPU copy is dropped).
#[derive(Resource, Clone, ExtractResource)]
pub struct WaterBump(pub Handle<Image>);

/// The next phase from the current one and a frame's simulation delta (`FUN_00a7f080`:
/// float32 store, then the strict wrap test on the unrounded sum).
pub fn advance_phase(phase: f32, dt: f32) -> f32 {
    let sum = dt as f64 * PHASE_PER_SECOND as f64 + phase as f64;
    if sum > PHASE_WRAP as f64 { 0.0 } else { sum as f32 }
}

/// The uv layer `i` samples at quad position `(x, y)` (y = 0 at the top) for `phase`.
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn layer_uv(i: usize, x: f32, y: f32, phase: f32) -> [f32; 2] {
    let (fx, fy, scroll) = LAYERS[i];
    [
        if fx { 1.0 - x } else { x } + phase * scroll[0],
        if fy { 1.0 - y } else { y } + phase * scroll[1],
    ]
}

/// The bump image as a GPU BC3 chain with a repeat sampler.
pub fn bump_image(dds: &DdsImage) -> Result<Image, String> {
    if dds.format != DdsFormat::Dxt5 {
        return Err(format!("{:?}, expected DXT5", dds.format));
    }
    bc_image(
        dds,
        TextureFormat::Bc3RgbaUnorm,
        ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Nearest,
            ..default()
        }),
    )
}

pub fn load_water_bump(
    game: Option<Res<Game>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let Some(game) = game else { return };
    let loaded = game
        .fs
        .read_inflated(BUMP_FILE)
        .map_err(|e| e.to_string())
        .and_then(|d| DdsImage::parse(&d).map_err(|e| e.to_string()))
        .and_then(|d| bump_image(&d));
    match loaded {
        Ok(image) => commands.insert_resource(WaterBump(images.add(image))),
        Err(e) => warn!("{BUMP_FILE}: {e}; no water normal texture (neutral distortion)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;

    #[test]
    fn the_phase_follows_the_native_clock() {
        // one default frame (dt = 1/60 as float32): 0.0075000003
        let p = advance_phase(0.0, f32::from_bits(0x3c88_8889));
        assert_eq!(p, 0.007_500_000_3);
        // 600 default frames: 4.4999995 (research fixture)
        let mut p = 0.0;
        for _ in 0..600 {
            p = advance_phase(p, f32::from_bits(0x3c88_8889));
        }
        assert!((p - 4.5).abs() < 1e-4, "{p}");
        // the wrap is strict: 10000 itself is kept, above it the accumulator restarts
        assert_eq!(advance_phase(9999.5, 1.0 / 60.0), 9_999.508);
        assert_eq!(advance_phase(9999.999, 2.0 / 60.0), 0.0);
        assert_eq!(advance_phase(PHASE_WRAP - 0.0075, 1.0 / 60.0 * 1.0), PHASE_WRAP);
    }

    /// The layer table of the research: phase 0 offsets and the 600-frame fixture.
    #[test]
    fn the_layers_follow_the_vertex_shader() {
        assert_eq!(layer_uv(1, 0.25, 0.5, 0.0), [0.75, 0.5]);
        assert_eq!(layer_uv(2, 0.25, 0.5, 0.0), [0.25, 0.5]);
        // 600 default frames: layer scrolls (0.27, 0.225), (-0.135, -0.315), (-0.18, 0.315), (-0.315, 0.135)
        let want = [[0.27, 0.225], [-0.135, -0.315], [-0.18, 0.315], [-0.315, 0.135]];
        for (i, w) in want.iter().enumerate() {
            let (_, _, s) = LAYERS[i];
            assert!((4.5 * s[0] - w[0]).abs() < 1e-6 && (4.5 * s[1] - w[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn the_retail_bump_is_a_512_dxt5_chain() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = GameFs::open(dir) else { return };
        let bytes = fs.read_inflated(BUMP_FILE).expect("bump file in the archives");
        let dds = DdsImage::parse(&bytes).expect("bump parses");
        assert_eq!((dds.width, dds.height, dds.mips), (512, 512, 10));
        let image = bump_image(&dds).expect("bump image");
        assert_eq!(image.texture_descriptor.mip_level_count, 10);
        assert_eq!(image.texture_descriptor.format, TextureFormat::Bc3RgbaUnorm);
    }
}
