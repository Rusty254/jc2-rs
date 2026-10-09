//! The game's terrain horizon / ambient-occlusion shadow of the Sun (JC2-022).
//!
//! The retail PC game has no cloud shadow (`cloudshadows.bin` is never read). The native
//! Sun attenuation is terrain self-shadowing from the static
//! `textures\landscape\ambientocclusionmap.dds` (research/formats/cloud-shadow-consumers-next.md,
//! `FUN_0091f870`, `FUN_00a58a00`, the `FastTer` pixel shader and 607 more of the 614 shaders
//! that read `t11`):
//!
//! ```text
//! t      = AO map (world.xz / 32768 + 0.5)       x = AO, y / w = horizon heights, z unused
//! shadow = max(saturate(cb0[0].z - dot(t.wy, cb0[0].xy)), cb0[10].w)
//! sun    = sun colour * N.L * shadow * (t.x / 2 + 1/2)
//! ambient= ambient * t.x
//! ```
//!
//! This module is the shared API for every material that wants it:
//!
//! * [`HorizonShadow`] (a resource, inserted once the terrain materials are set up): the map
//!   (`ao`: 1024² BC3, no mips, clamp, linear, one texel = 32 m) and the per-frame constants
//!   (`params`: a storage buffer holding [`SunHorizon`], rewritten by [`update_horizon`]
//!   whenever the Sun moves).
//! * `sun_horizon.wgsl` (import path `jc2::sun_horizon`): the shader side (`ao_uv`,
//!   `sun_factor`, `is_sun`). `terrain_splat.wgsl` is the reference consumer.
//!
//! Consuming it from a `MaterialExtension` (e.g. the General material): add
//! `#[texture(N)] ao: Handle<Image>` and `#[storage(N + 1, read_only)] sun_horizon:
//! Handle<ShaderBuffer>` filled from `HorizonShadow::ao` / `params`, sample the map with
//! any clamp/linear sampler at `ao_uv(world.xz)`, multiply `pbr_input.diffuse_occlusion` by
//! `t.x`, and after `apply_pbr_lighting` add `exposure * sun_light * cascade_shadow *
//! (sun_factor - 1)` for the directional light `is_sun` selects (see `sun_correction` in
//! `terrain_splat.wgsl`).

use crate::sky::Sun;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use jc2_formats::dds::{DdsFormat, DdsImage};

/// The map in the archives.
pub const AO_MAP: &str = "textures\\landscape\\ambientocclusionmap.dds";
/// `30.0` (rdata `0x00e24c30`): the gain of the horizon term and the penumbra scale.
pub const GAIN: f32 = 30.0;
/// `sin 45°` (`DAT_0108ae60._4_4_`): the noon elevation by day.
pub const K_DAY: f32 = 0.707_106_77;
/// The same after the night flip (`k²`).
pub const K_NIGHT: f32 = 0.5;
/// The shadow floor at full strength (`DAT_0108c9a8`).
pub const FLOOR: f32 = 0.15;

/// What the shader reads: `cb0[0]` and the floor, plus the Sun's direction to find it among
/// the directional lights (WGSL `SunHorizon`).
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SunHorizon {
    /// `cb0[0] = (a, b, c, 1)`.
    pub cb0: Vec4,
    /// xyz: unit vector towards the Sun, w: `cb0[10].w`.
    pub sun: Vec4,
}

impl SunHorizon {
    /// The constants for a Sun at `sun_dir` (unit vector towards the Sun, engine axes: x
    /// east, y up, z south, the same as Bevy's), as `FUN_00a58a00` / `FUN_0091f870` do.
    ///
    /// With `L = -sun_dir` the light direction: `floor = min(|L.y| 20, 1) 0.15`; when
    /// `L.y > 0` (the Sun is below the horizon) the vector is flipped and `k` squared;
    /// `L.y = min(L.y, 0.01)`, renormalised; `(a, b) = L.x >= 0 ? (0, 30) : (30, 0)` and
    /// `c = -L.y / k 30`.
    pub fn from_sun_direction(sun_dir: Vec3) -> Self {
        let sun_dir = sun_dir.normalize_or_zero();
        let mut l = -sun_dir;
        let floor = (l.y.abs() * 20.0).min(1.0) * FLOOR;
        let mut k = K_DAY;
        if l.y > 0.0 {
            l = -l;
            k = K_NIGHT;
        }
        l.y = l.y.min(0.01);
        l = l.normalize_or_zero();
        let (a, b) = if l.x >= 0.0 { (0.0, GAIN) } else { (GAIN, 0.0) };
        let c = -l.y / k * GAIN;
        Self {
            cb0: Vec4::new(a, b, c, 1.0),
            sun: sun_dir.extend(floor),
        }
    }

    /// The horizon shadow of a map texel `(x, y, z, w)` = `(AO, horizon, -, horizon)`.
    #[allow(dead_code)] // the CPU mirror of sun_horizon.wgsl (tests; other consumers)
    pub fn shadow(&self, texel: Vec4) -> f32 {
        let h = texel.w * self.cb0.x + texel.y * self.cb0.y;
        (self.cb0.z - h).clamp(0.0, 1.0).max(self.sun.w)
    }

    /// The factor on the Sun's direct light: `shadow * (AO / 2 + 1/2)`.
    #[allow(dead_code)]
    pub fn sun_factor(&self, texel: Vec4) -> f32 {
        self.shadow(texel) * (texel.x * 0.5 + 0.5)
    }
}

/// The map and the per-frame constants, shared by every material that uses them.
#[derive(Resource, Clone, Debug)]
pub struct HorizonShadow {
    /// `ambientocclusionmap.dds` (or an open-ground 1 × 1 stand-in when it is missing).
    pub ao: Handle<Image>,
    /// A storage buffer holding one [`SunHorizon`], rewritten when the Sun moves.
    pub params: Handle<ShaderBuffer>,
    /// `JC2_NO_HORIZON=1` (a debug aid for before / after captures): an open-ground map
    /// and no Sun constants, i.e. the lighting without this term.
    pub enabled: bool,
}

impl HorizonShadow {
    /// Load the map once for the session.
    pub fn load(
        game: &crate::Game,
        images: &mut Assets<Image>,
        buffers: Option<&mut Assets<ShaderBuffer>>,
    ) -> Self {
        let enabled = std::env::var("JC2_NO_HORIZON").is_err();
        let image = game
            .fs
            .read_inflated(AO_MAP)
            .map_err(|e| e.to_string())
            .and_then(|d| DdsImage::parse(&d).map_err(|e| e.to_string()))
            .and_then(|d| ao_image(&d))
            .and_then(|i| {
                if enabled {
                    Ok(i)
                } else {
                    Err("JC2_NO_HORIZON is set".into())
                }
            });
        let image = image.unwrap_or_else(|e| {
            warn!("{AO_MAP}: {e}; terrain gets no horizon shadow or AO");
            open_ground()
        });
        Self {
            ao: images.add(image),
            params: buffers
                .map(|b| b.add(ShaderBuffer::from(SunHorizon::default())))
                .unwrap_or_default(),
            enabled,
        }
    }
}

/// The map as a GPU texture: BC3, linear, clamped, no mips.
pub fn ao_image(img: &DdsImage) -> Result<Image, String> {
    if img.format != DdsFormat::Dxt5 {
        return Err(format!("expected DXT5, got {:?}", img.format));
    }
    let bytes = (img.width.div_ceil(4) * img.height.div_ceil(4) * 16) as usize;
    if img.data.len() < bytes {
        return Err("texture data is short".into());
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bc3RgbaUnorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(img.data[..bytes].to_vec());
    image.sampler = clamp_linear();
    Ok(image)
}

/// AO 1, no horizon: the map of a world without shadows.
fn open_ground() -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255, 0, 0, 0],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = clamp_linear();
    image
}

fn clamp_linear() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    })
}

/// Recompute the constants from the Sun's direction (its light points along `-back`), and
/// rewrite the buffer only when they changed.
pub fn update_horizon(
    horizon: Option<Res<HorizonShadow>>,
    sun: Query<&Transform, With<Sun>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut last: Local<Option<SunHorizon>>,
) {
    let (Some(horizon), Ok(sun)) = (horizon, sun.single()) else {
        return;
    };
    if !horizon.enabled {
        return;
    }
    let now = SunHorizon::from_sun_direction(sun.back().as_vec3());
    if *last == Some(now) {
        return;
    }
    if let Some(mut b) = buffers.get_mut(&horizon.params) {
        b.set_data(now);
        *last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::weather::sun_direction;

    fn at(hour: f32) -> SunHorizon {
        SunHorizon::from_sun_direction(Vec3::from(sun_direction(hour)))
    }

    fn near(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() < eps, "{a} vs {b}");
    }

    /// The probe's `cb0[0]` / `cb0[10].w` table (`cloud_shadow_consumers_probe.py`, §3).
    #[test]
    fn cb0_matches_the_probe_table() {
        // (hour, a, b, c, floor)
        let rows = [
            (5.5, 30.0, 0.0, 0.220, 0.0156),
            (5.0 + 42.0 / 60.0, 30.0, 0.0, 1.592, 0.1126),
            (6.0, 30.0, 0.0, 3.659, 0.15),
            (8.0, 30.0, 0.0, 16.83, 0.15),
            (12.0, 0.0, 30.0, 30.00, 0.15),
            (16.0, 0.0, 30.0, 16.83, 0.15),
            (18.0 + 18.0 / 60.0, 0.0, 30.0, 1.592, 0.1126),
            // night flip: the moon's side, k = 0.5
            (0.0, 0.0, 30.0, 32.08, 0.15),
        ];
        for (hour, a, b, c, floor) in rows {
            let s = at(hour);
            if hour == 0.0 {
                // the Sun is due north-south at midnight: L.x is rounding noise, so the
                // side is a coin flip (as in the original), only the pair is fixed
                assert_eq!(s.cb0.x + s.cb0.y, a + b, "hour {hour}");
            } else {
                assert_eq!((s.cb0.x, s.cb0.y), (a, b), "hour {hour}");
            }
            near(s.cb0.z, c, 0.01 * c.max(1.0));
            assert_eq!(s.cb0.w, 1.0);
            near(s.sun.w, floor, 5e-4);
        }
    }

    /// East/west: the morning sun is on +x (weather.md's sign), so `L = -sun` has x < 0 and
    /// `cb0.xy = (30, 0)` picks `t11.w`; in the afternoon `(0, 30)` picks `t11.y`.
    #[test]
    fn morning_reads_w_and_afternoon_reads_y() {
        let am = at(8.0);
        assert_eq!((am.cb0.x, am.cb0.y), (30.0, 0.0));
        let pm = at(16.0);
        assert_eq!((pm.cb0.x, pm.cb0.y), (0.0, 30.0));
        // a horizon only on the w side shadows the morning but not the afternoon
        let t = Vec4::new(1.0, 0.0, 0.0, 0.9);
        assert!(am.shadow(t) <= FLOOR + 1e-6);
        assert_eq!(pm.shadow(t), 1.0);
        // a horizon only on the y side does the opposite
        let t = Vec4::new(1.0, 0.9, 0.0, 0.0);
        assert_eq!(am.shadow(t), 1.0);
        assert!(pm.shadow(t) <= FLOOR + 1e-6);
    }

    /// The probe's sun-factor table for sample texels (x, y, w).
    #[test]
    fn sun_factor_matches_the_probe_table() {
        let (noon, pm, am) = (at(12.0), at(16.0), at(8.0));
        let open = Vec4::new(1.0, 0.0, 0.0, 0.0);
        let full = Vec4::new(1.0, 1.0, 0.0, 1.0);
        let mixed = Vec4::new(0.0, 0.2, 0.0, 0.9);
        for s in [noon, pm, am] {
            near(s.sun_factor(open), 1.0, 1e-6);
            near(s.sun_factor(full), FLOOR, 1e-6);
        }
        near(noon.sun_factor(mixed), 0.5, 1e-4);
        near(pm.sun_factor(mixed), 0.5, 1e-4);
        near(am.sun_factor(mixed), 0.075, 1e-4);
    }

    /// 5:30 to 6:00: the floor ramps with `|L.y| 20`, capped at 1 (0.15).
    #[test]
    fn the_floor_ramps_at_dawn() {
        let mut prev = -1.0;
        for minute in 0..=30 {
            let f = at(5.5 + minute as f32 / 60.0).sun.w;
            assert!((prev - 1e-6..=FLOOR + 1e-6).contains(&f), "{minute}: {f}");
            prev = f;
        }
        near(at(5.5).sun.w, 0.0156, 5e-4);
        near(at(6.0).sun.w, FLOOR, 1e-6);
        // |L.y| = 0.025 is the ramp's midpoint
        near(
            SunHorizon::from_sun_direction(Vec3::new(0.9997, 0.025, 0.0))
                .sun
                .w,
            0.5 * FLOOR,
            2e-3,
        );
    }

    /// The night flip: below the horizon the vector flips and `k` squares.
    #[test]
    fn night_flips_the_light() {
        let s = SunHorizon::from_sun_direction(Vec3::new(0.0, -0.5, 0.866_025_4));
        near(s.cb0.z, 0.5 / K_NIGHT * GAIN, 1e-3);
        near(s.sun.w, FLOOR, 1e-6);
        // just above the horizon the day constant applies
        let s = SunHorizon::from_sun_direction(Vec3::new(0.0, 0.5, 0.866_025_4));
        near(s.cb0.z, 0.5 / K_DAY * GAIN, 1e-3);
    }

    /// Only the hour matters: no cover or time input exists.
    #[test]
    fn the_constants_depend_on_the_direction_only() {
        assert_eq!(at(10.0), at(10.0));
        assert_ne!(at(10.0), at(10.5));
    }

    #[test]
    fn ao_map_texture_is_linear_clamped_without_mips() {
        // 8x8 BC3: 4 blocks of 16 bytes
        let img = DdsImage {
            width: 8,
            height: 8,
            mips: 1,
            format: DdsFormat::Dxt5,
            data: vec![0; 64],
        };
        let t = ao_image(&img).unwrap();
        assert_eq!(t.texture_descriptor.mip_level_count, 1);
        assert_eq!(t.texture_descriptor.format, TextureFormat::Bc3RgbaUnorm);
        assert!(matches!(
            t.sampler,
            ImageSampler::Descriptor(ref d) if d.address_mode_u == ImageAddressMode::ClampToEdge
                && d.mag_filter == ImageFilterMode::Linear
        ));
        let short = DdsImage {
            data: vec![0; 10],
            ..img
        };
        assert!(ao_image(&short).is_err());
    }
}
