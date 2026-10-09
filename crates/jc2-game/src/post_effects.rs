//! The native final colour chain (JC2-134 phase 1, JC2-137 phase 2; research
//! `research/formats/night-exposure-next.md`, JC2-124; gamma proposal JC2-099).
//!
//! Native chain (brief section 4): lit colour is clamped to 0..1 and stored in an 8-bit sRGB
//! target; the post composite then applies the saturation clamp, `* (A + S7 - 1) * tint` and
//! writes `sqrt` (gamma 2.0) to a UNORM swap chain. There is no tone-mapping curve.
//!
//! * [`PostEffectState`] holds the 27 settings `S(n)` the exe's post manager holds, by hour:
//!   the night block `#0a10a074` and the day block `#0a48aa9b` of
//!   `settings\landscape_effects.bin` with the 04:30..05:30 / 18:30..19:30 linear blends and
//!   the per-zone overrides of the gradient set under the camera
//!   (`jc2_formats::weather::PostEffectBlocks::params`, `FUN_00a5e010`). **Native.** Applied
//!   instantly (the exe's smoothing factor `P+0xf0` is Gap 2 of the brief).
//! * [`PostPass`] is the per-camera uniform of the fullscreen pass (`post_effects.wgsl`).
//!   The game camera runs without HDR and with `Tonemapping::None`, so its main texture is
//!   Bevy's 8-bit sRGB target, which clamps and quantises the lit colour exactly like the
//!   native scene target (native step 1 comes for free). The pass then does the composite
//!   tail: saturation clamp, `* (A + S7 - 1) * tint`, `sqrt`. `A` is the auto-exposure of
//!   `auto_exposure.rs` / `auto_exposure.wgsl` (JC2-137), metered on the GPU from the same
//!   pre-post scene and read one frame late (render side: `post_effects_render.rs`). `x`, `y`
//!   and `key` of the meter are the block values `S8`, `S9`, `S10` (night x 0 / y 0.08, day
//!   x 0.3 / y 0.10, Arctic day y 0.04, by the same hour/zone blend as the other settings);
//!   `dt` is the real frame delta (inferred, brief Gap 3).
//! * **Encode (brief Gap 10, implementation decision):** the window surface is sRGB and
//!   Bevy's later blit decodes and re-encodes it, so a raw UNORM target is not available
//!   without replacing the upscaling node. The pass instead returns the sRGB *decode* of
//!   `sqrt(C)`; the sRGB target's hardware encode (and the identical decode/encode of the
//!   final blit) restore `sqrt(C)` exactly, so the surface holds the gamma 2.0 value the
//!   native swap chain holds, quantised once to 8 bits. [`encode_roundtrip`] and the tests
//!   pin this. The HUD is drawn after the post pass (`bevy_ui_render` runs after
//!   `Core3dSystems::PostProcess`), as native HUD is.
//!
//! Gaps (not implemented, not guessed): the auto-exposure meter geometry and `dt` source are
//! inferred (see `auto_exposure.rs`), blur mix, overlay S2/S3, depth of field S5/S25, heat haze S26, underwater/height/rain/snow sets, smoothing of the settings,
//! and the cutscene overrides (brief Gaps 1, 2, 5..9).

use crate::Game;
use crate::sky::TimeOfDay;
use crate::auto_exposure;
use crate::sky_light::{SkyLighting, ZoneGradientData, update_sky_lighting};
use bevy::asset::{AssetPath, embedded_path};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use jc2_formats::weather::{POST_SETTINGS, PostBlock, PostEffectBlocks, post};

/// The luminance weights of the native composite and luminance passes (`SampleLumInitial`,
/// the saturation term of `Scene`).
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub const LUMINANCE: Vec3 = auto_exposure::LUMINANCE;

/// The camera component the final pass and the meter read (`post_effects_render.rs`): matches
/// the WGSL `PostPass` struct of `post_effects.wgsl` and `auto_exposure.wgsl`.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct PostPass {
    /// rgb = tint (`S11..S13`), w = `S7 - 1` (the shader adds the auto-exposure `A`).
    pub tint: Vec4,
    /// x = saturation offset (`S15`), y = slope (`S16`), z / w = the clamp range
    /// (`min`/`max` of `S14` and `S17`).
    pub saturation: Vec4,
    /// The meter: x = weight of the darkest tap (`S8`), y = strength (`S9`), z = key (`S10`),
    /// w = frame delta in seconds (**inferred**, brief Gap 3; set by `update_post_effects`).
    pub meter: Vec4,
}

impl Default for PostPass {
    /// The neutral pass: clamp, no tint, no desaturation, `sqrt` encode only (`A` stays 1).
    fn default() -> Self {
        Self {
            // w = S7 - 1 = 0
            tint: Vec4::new(1.0, 1.0, 1.0, 0.0),
            saturation: Vec4::new(1.0, 0.0, 1.0, 1.0),
            // y = 0: the target is 1 for any scene (A stays 1)
            meter: Vec4::ZERO,
        }
    }
}

impl PostPass {
    /// The uniform for the settings `s` (brief 1.3 / 2.2: tint `S11..S13`, `cb1[2].x = S7 - 1`,
    /// `cb1[4] = (S15, S16, min(S14, S17), max(S14, S17))`, meter `(S8, S9, S10)`); `dt` 0.
    pub fn from_settings(s: &PostBlock) -> Self {
        Self {
            tint: Vec4::new(
                s[post::TINT_R],
                s[post::TINT_G],
                s[post::TINT_B],
                s[post::EXPOSURE_ONE] - 1.0,
            ),
            saturation: Vec4::new(
                s[post::SATURATION_OFFSET],
                s[post::SATURATION_SLOPE],
                s[post::SATURATION_END].min(s[post::SATURATION_OTHER_END]),
                s[post::SATURATION_END].max(s[post::SATURATION_OTHER_END]),
            ),
            meter: Vec4::new(s[post::AUTO_X], s[post::AUTO_Y], s[post::AUTO_KEY], 0.0),
        }
    }

    /// The pass on the CPU (the same arithmetic as `post_effects.wgsl`, before the encode
    /// trick): `scene` is the linear colour read from the 8-bit target, `a` the auto-exposure;
    /// the result is the display value `sqrt(C)` the native swap chain would hold (0..1).
    #[allow(dead_code)] // CPU reference of the shader, used by the tests
    pub fn display(&self, scene: Vec3, a: f32) -> Vec3 {
        let c = scene.clamp(Vec3::ZERO, Vec3::ONE);
        let lum = c.dot(LUMINANCE);
        let k = (lum * self.saturation.y + self.saturation.x)
            .clamp(self.saturation.z, self.saturation.w);
        let c = Vec3::splat(lum) + (c - Vec3::splat(lum)) * k;
        let c = c * (a + self.tint.w) * self.tint.truncate();
        c.max(Vec3::ZERO)
            .map(f32::sqrt)
            .clamp(Vec3::ZERO, Vec3::ONE)
    }
}

/// The final pass shader (embedded, `post_effects.wgsl`).
pub fn post_shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("post_effects.wgsl")).with_source("embedded"),
    )
}

/// The sRGB transfer functions (IEC 61966-2-1) the encode argument uses.
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn srgb_decode(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn srgb_encode(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// What the stored (sRGB-encoded) surface value is for a pass result `v = sqrt(C)`: the shader
/// returns `srgb_decode(v)` and the hardware encodes it, so this is `v` again.
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn encode_roundtrip(v: f32) -> f32 {
    srgb_encode(srgb_decode(v))
}

/// The blocks of `landscape_effects.bin`, when the file could be read.
#[derive(Resource, Default)]
pub struct PostEffectData(pub Option<PostEffectBlocks>);

/// The current post-effect settings `S(n)` (the post manager's floats). The auto-exposure `A`
/// is metered and adapted on the GPU (`auto_exposure.wgsl`) and is not readable on the CPU.
#[derive(Resource, Clone, Debug)]
pub struct PostEffectState {
    pub settings: PostBlock,
}

impl PostEffectState {
    /// No data: `S7 = 1`, tint 1, saturation clamp `[1, 1]` (identity), everything else 0.
    pub fn neutral() -> Self {
        let mut settings = [0.0; POST_SETTINGS];
        settings[post::EXPOSURE_ONE] = 1.0;
        settings[post::TINT_R] = 1.0;
        settings[post::TINT_G] = 1.0;
        settings[post::TINT_B] = 1.0;
        settings[post::SATURATION_END] = 1.0;
        settings[post::SATURATION_OTHER_END] = 1.0;
        settings[post::SATURATION_OFFSET] = 1.0;
        Self { settings }
    }

    /// Select the blocks for gradient `set`, intensity weight `w` and `hour`.
    pub fn step(&mut self, data: Option<&PostEffectBlocks>, set: usize, w: f32, hour: f32) {
        self.settings = match data {
            Some(b) => b.params(set, w, hour),
            None => Self::neutral().settings,
        };
    }

    pub fn pass(&self) -> PostPass {
        PostPass::from_settings(&self.settings)
    }
}

impl Default for PostEffectState {
    fn default() -> Self {
        Self::neutral()
    }
}

pub struct PostEffectPlugin;

impl Plugin for PostEffectPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "post_effects.wgsl");
        app.add_plugins(crate::post_effects_render::PostRenderPlugin)
            .init_resource::<PostEffectData>()
            .init_resource::<PostEffectState>()
            .add_systems(Startup, load_post_blocks)
            .add_systems(Update, update_post_effects.after(update_sky_lighting));
    }
}

fn load_post_blocks(game: Option<Res<Game>>, mut data: ResMut<PostEffectData>) {
    let Some(game) = game else { return };
    match PostEffectBlocks::load(&game.fs) {
        Ok(b) => data.0 = Some(b),
        Err(e) => warn!("post effects: landscape_effects.bin not read ({e}); neutral final pass"),
    }
}

/// Every frame from the clock and the zone set (the exe's `FUN_00a66c30` -> `FUN_00a5e010`).
pub fn update_post_effects(
    time: Res<TimeOfDay>,
    lighting: Res<SkyLighting>,
    zone: Res<ZoneGradientData>,
    data: Res<PostEffectData>,
    real: Res<Time<Real>>,
    mut state: ResMut<PostEffectState>,
    mut cams: Query<&mut PostPass>,
) {
    // the intensity weight of the zone (inferred, see `PostEffectBlocks::params`); the file's
    // constants come with the sky-light tables
    let w = zone.0.as_ref().map_or(1.0, |g| {
        g.effects
            .variant_weight(lighting.zone_set, lighting.zone_intensity)
    });
    state.step(data.0.as_ref(), lighting.zone_set, w, time.hours);
    let mut pass = state.pass();
    // the meter's dt: the real frame delta (inferred, brief Gap 3); the GPU clamps it 0.01..0.5
    pass.meter.w = real.delta_secs();
    for mut p in &mut cams {
        if *p != pass {
            *p = pass;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::weather::LANDSCAPE_EFFECTS_FILE;

    fn retail() -> Option<PostEffectBlocks> {
        let fs = GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()?;
        fs.contains(LANDSCAPE_EFFECTS_FILE)
            .then(|| PostEffectBlocks::load(&fs).expect("post_effects parses"))
    }

    fn near(a: Vec3, b: [f32; 3], eps: f32) {
        assert!(
            (a - Vec3::from(b)).abs().max_element() < eps,
            "{a:?} vs {b:?}"
        );
    }

    #[test]
    fn the_neutral_pass_is_clamp_and_sqrt() {
        let p = PostPass::default();
        near(p.display(Vec3::new(0.25, 1.0, 4.0), 1.0), [0.5, 1.0, 1.0], 1e-6);
        near(p.display(Vec3::new(-1.0, 0.0, 0.04), 1.0), [0.0, 0.0, 0.2], 1e-6);
        assert_eq!(PostEffectState::neutral().pass(), p);
    }

    /// The probe's composite model: night block, A = 1, grey 0.1 -> sqrt(0.1 * (1, 1, 1.6)).
    #[test]
    fn night_block_tints_grey_like_the_probe() {
        let Some(b) = retail() else { return };
        let night = PostPass::from_settings(&b.params(7, 1.0, 0.0));
        near(
            night.display(Vec3::splat(0.1), 1.0),
            [0.316_227_77, 0.316_227_77, 0.4],
            1e-6,
        );
        // uniform ambient: the tint is the only change, exposure offset S7 - 1 = 0
        assert_eq!(night.tint, Vec4::new(1.0, 1.0, 1.6, 0.0));
        // saturation: night k = clamp(10 lum + 0.6, 0, 0.95)
        assert_eq!(night.saturation, Vec4::new(0.6, 10.0, 0.0, 0.95));
    }

    #[test]
    fn night_saturation_is_a_luminance_dependent_desaturation() {
        let Some(b) = retail() else { return };
        let night = PostPass::from_settings(&b.params(7, 1.0, 0.0));
        // a dark red: lum = 0.2125 * 0.04 = 0.0085 -> k = 0.6 + 0.085 = 0.685
        let c = Vec3::new(0.04, 0.0, 0.0);
        let lum: f32 = 0.2125 * 0.04;
        let k: f32 = 10.0 * lum + 0.6;
        let want = [
            ((lum + k * (0.04 - lum)) * 1.0).sqrt(),
            (lum + k * (0.0 - lum)).sqrt(),
            ((lum + k * (0.0 - lum)) * 1.6).sqrt(),
        ];
        near(night.display(c, 1.0), want, 1e-6);
        // a black pixel stays black; a bright one hits the 0.95 ceiling
        near(night.display(Vec3::ZERO, 1.0), [0.0; 3], 1e-7);
        let bright = Vec3::new(1.0, 0.2, 0.2);
        let lum = bright.dot(LUMINANCE);
        let want_g = (lum + 0.95 * (0.2 - lum)).sqrt();
        assert!((night.display(bright, 1.0).y - want_g).abs() < 1e-6);
        // the day block has no desaturation: the clamp range is [1, 1]
        let noon = PostPass::from_settings(&b.params(7, 1.0, 12.0));
        assert_eq!(noon.saturation.z, 1.0);
        assert_eq!(noon.saturation.w, 1.0);
        near(
            noon.display(Vec3::new(0.25, 0.04, 0.0), 1.0),
            [0.5, 0.2, 0.0],
            1e-6,
        );
    }

    #[test]
    fn the_exposure_offset_and_a_scale_before_the_encode() {
        let mut s = PostEffectState::neutral().settings;
        s[post::EXPOSURE_ONE] = 1.0;
        // A = 2: the colour is doubled before the sqrt
        let p = PostPass::from_settings(&s);
        near(p.display(Vec3::splat(0.125), 2.0), [0.5, 0.5, 0.5], 1e-6);
        // S7 = 0.5: the offset S7 - 1 = -0.5 is added to A (A + S7 - 1 = 0.5 for A = 1)
        s[post::EXPOSURE_ONE] = 0.5;
        let p = PostPass::from_settings(&s);
        assert_eq!(p.tint.w, -0.5);
        near(p.display(Vec3::splat(0.5), 1.0), [0.5, 0.5, 0.5], 1e-6);
    }

    /// The retail blocks through the state: the start base (Arctic, set 1).
    #[test]
    fn the_state_follows_the_hour_and_the_zone_set() {
        let Some(b) = retail() else { return };
        let mut st = PostEffectState::default();
        st.step(Some(&b), 1, 1.0, 12.0);
        assert_eq!(st.pass().tint, Vec4::new(1.0, 1.0, 1.25, 0.0));
        st.step(Some(&b), 1, 1.0, 22.0);
        assert_eq!(
            st.pass().tint,
            Vec4::new(1.0, 1.0, 1.0, 0.0),
            "Arctic night"
        );
        st.step(Some(&b), 7, 1.0, 22.0);
        assert_eq!(
            st.pass().tint,
            Vec4::new(1.0, 1.0, 1.6, 0.0),
            "Default night"
        );
        st.step(Some(&b), 7, 1.0, 5.0);
        let p = st.pass();
        assert!((p.tint.z - 1.3).abs() < 1e-6, "dawn blend: {:?}", p.tint);
        assert!((p.saturation.w - 0.975).abs() < 1e-6);
        // without data the state is the neutral pass again
        st.step(None, 7, 1.0, 22.0);
        assert_eq!(st.pass(), PostPass::default());
    }

    /// The meter parameters follow the blocks: night x 0 / y 0.08, day x 0.3 / y 0.10 (key 0.32),
    /// Arctic (set 1) day y 0.04, and the 04:30..05:30 blend (JC2-137).
    #[test]
    fn the_meter_parameters_follow_the_blocks() {
        let Some(b) = retail() else { return };
        let m = |set: usize, hour: f32| PostPass::from_settings(&b.params(set, 1.0, hour)).meter;
        assert_eq!(m(7, 0.0), Vec4::new(0.0, 0.08, 0.32, 0.0), "night");
        assert_eq!(m(7, 12.0), Vec4::new(0.3, 0.10, 0.32, 0.0), "day");
        assert_eq!(m(1, 12.0).y, 0.04, "Arctic day");
        let dawn = m(7, 5.0);
        assert!((dawn.x - 0.15).abs() < 1e-6 && (dawn.y - 0.09).abs() < 1e-6, "{dawn:?}");
        // the steady state the GPU reaches for a uniform 0.05 scene (brief 1.4)
        let t = |set, hour| {
            let v = m(set, hour);
            auto_exposure::exposure_target([0.05; 4], v.x, v.y, v.z)
        };
        assert!((t(7, 12.0) - 1.54).abs() < 1e-6, "day");
        assert!((t(7, 22.0) - 1.432).abs() < 1e-6, "night");
        assert!((t(1, 12.0) - 1.216).abs() < 1e-6, "Arctic day: 1 + 0.04 * 5.4");
    }

    /// Single encode: the pass returns `srgb_decode(sqrt(C))`, the sRGB target stores
    /// `srgb_encode` of it, which is `sqrt(C)` again (not the sRGB curve of C).
    #[test]
    fn the_sqrt_survives_the_srgb_target() {
        for c in [0.0_f32, 0.01, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let v = c.sqrt();
            assert!((encode_roundtrip(v) - v).abs() < 1e-5, "{c}");
            // and it differs from what Bevy's plain sRGB output would have stored
            if (0.02..0.9).contains(&c) {
                assert!((srgb_encode(c) - v).abs() > 0.004, "{c}");
            }
        }
        // the encode fixtures of JC2-099: linear 0, 0.25, 0.5, 1 -> 0, 0.5, 0.7071, 1
        for (c, want) in [
            (0.0, 0.0),
            (0.25, 0.5),
            (0.5, std::f32::consts::FRAC_1_SQRT_2),
            (1.0, 1.0),
        ] {
            let stored = encode_roundtrip(f32::sqrt(c));
            assert!((stored - want).abs() < 1e-5, "{c}: {stored}");
        }
        // gamma 2.0 against sRGB at 0.05: -0.0237 (underwater brief 4: up to 0.037 apart)
        assert!((0.05_f32.sqrt() - srgb_encode(0.05) + 0.0237).abs() < 1e-3);
    }

    #[test]
    fn the_wgsl_matches_the_uniform_layout_and_parses() {
        let src = include_str!("post_effects.wgsl");
        let meter = include_str!("auto_exposure.wgsl");
        for src in [src, meter] {
            assert!(src.contains("tint: vec4<f32>") && src.contains("saturation: vec4<f32>"));
            // field order of PostPass: tint, saturation, meter (three vec4)
            assert!(src.find("tint: vec4<f32>") < src.find("saturation: vec4<f32>"));
            assert!(src.find("saturation: vec4<f32>") < src.find("meter: vec4<f32>"));
        }
        assert_eq!(std::mem::size_of::<PostPass>(), 48);
    }

    /// The system moves the settings onto the camera component each frame.
    #[test]
    fn the_system_updates_the_camera_pass_from_the_clock() {
        let Some(blocks) = retail() else { return };
        let mut app = App::new();
        app.insert_resource(TimeOfDay {
            hours: 22.0,
            speed: 0.0,
            paused: true,
        })
        .init_resource::<SkyLighting>()
        .init_resource::<ZoneGradientData>()
        .init_resource::<Time<Real>>()
        .insert_resource(PostEffectData(Some(blocks)))
        .init_resource::<PostEffectState>()
        .add_systems(Update, update_post_effects);
        let cam = app.world_mut().spawn(PostPass::default()).id();
        app.update();
        // stand-in set 7 (Default), night: tint B 1.6
        assert_eq!(app.world().get::<PostPass>(cam).unwrap().tint.z, 1.6);
        app.world_mut().resource_mut::<TimeOfDay>().hours = 12.0;
        app.update();
        assert_eq!(app.world().get::<PostPass>(cam).unwrap().tint.z, 1.0);
        app.world_mut().resource_mut::<SkyLighting>().zone_set = 1;
        app.update();
        assert_eq!(app.world().get::<PostPass>(cam).unwrap().tint.z, 1.25);
    }
}
