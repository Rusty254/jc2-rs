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
//! **Which cameras share the chain (JC2-139 decision).** The world camera (`world.rs`) is the
//! one entity for the hero chase camera and the free fly camera (`--no-hero`, the capture
//! mode), so the free camera matches the game camera by construction: same `PostPass`, the
//! same no-HDR / `Tonemapping::None` / `Exposure::BLENDER` components. The model viewer
//! (`viewer.rs`), the mission-browser 3D model pane (`browser/model3d.rs`) and the foliage and
//! particle fixtures keep Bevy's default display (tone-mapped, sRGB encode) on purpose: they
//! are inspection tools whose project lights (a fixed 8,000 lux key, ambient 300) and the
//! stored GPU material/particle captures they are validated against (STATUS 8 October) would
//! all change under the native chain; none has a game zone, a clock or a scene to meter
//! (`A` would be 1), and `PostEffectPlugin` is only added to the world app. This is a
//! project choice, not a native fact (the exe has one render path).
//!
//! **JC2-138: the rest of the shipped composite** (`DepthOfField_EdgeFade`, PS 4863324, the
//! gameplay variant per `research/formats/post-composite-variant-next.md`; the `Scene` family
//! is the same code with a zero focus ramp). In `post_effects_render.rs` the scene is
//! blurred as the native "PostEffects" chain does (`Blur` PS 965692, seven equal-weight taps at
//! the offsets of `FUN_0092ec50`: pass 0 reads the full-size scene with a 1.75 texel spread and
//! writes a half-size target, pass 1 reads that with 2.625 texels), a HighPass pass (PS 6147124)
//! writes the quarter-size bloom texture `S0`/`S1` x `A`, and the composite then does: blend
//! `C = scene + sat(ramp * S25 + S4) * (blur - scene)` with the eye-distance ramp
//! `sat((S6 - eye) w) + sat((eye - S5) / S5)`, the overlay of the bloom texture (`S2` screen
//! branch, `S3` multiply branch), the saturation clamp, `* (A + S7 - 1) * tint`, the radial
//! EdgeFade vignette and `sqrt`. Eye distance comes from Bevy's depth prepass (non-native
//! stand-in for the native depth copy, as `sea.rs`); the nearest of the MSAA samples is used.
//!
//! Gaps (not implemented, not guessed): the auto-exposure meter geometry and `dt` source are
//! inferred (see `auto_exposure.rs`); heat haze S26 and the Underwater family are rendered
//! (JC2-193 / JC2-194, `water_normal.rs`, `post_effects.wgsl`) with the sea-surface depth as a
//! non-native analytic plane at y = 200 (the native depth copy holds the real surface);
//! underwater/height/rain/snow set weights; smoothing of the settings; the cutscene overrides
//! and film grain (brief Gaps 1, 2, 5..9).

use crate::Game;
use crate::auto_exposure;
use crate::auto_exposure::ClockModel;
use crate::sky::TimeOfDay;
use crate::sky_light::{SkyLighting, ZoneGradientData, update_sky_lighting};
use crate::water_normal::{advance_phase, load_water_bump};
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
    /// `cb1[0]` of the composite: x = heat-haze scale `0.004 * S26` (the `HeatHazeDOF` family
    /// when it exceeds `0.004 * 0.001` and `S5 > 0`; JC2-193), z = `S4` (blur blend),
    /// w = `S25` (depth-of-field blend gain); y unused.
    pub blend: Vec4,
    /// `cb1[5]`: the focus ramp `(S5, 1 / S5, S6, S6 > 0 ? 1 / (0.5 S6) : 0)`; all zero when
    /// `S5 <= 0` (the `Scene` family: the blend is then just `sat(S4)`).
    pub focus: Vec4,
    /// `(S2, S3, S0, S1)`: overlay strengths `cb1[3].xy` (screen / multiply branch) and the
    /// HighPass saturation mix and exponent `cb1[0].xy`.
    pub overlay: Vec4,
    /// x = camera near plane (m) of Bevy's reverse-Z depth (`eye = near / d`, set per camera by
    /// `update_post_effects`); y = 1 when the camera is below the sea (`200 > camera.y`, strict:
    /// the Underwater composite family, JC2-194); z = `cb0[6].y`, the minus forward Y of the
    /// camera; w = the water-clock phase (`water_normal.rs`).
    pub view: Vec4,
    /// xyz = camera world position (`cb0[5]`), w unused.
    pub cam_pos: Vec4,
    /// `[ndc.x, ndc.y, d, 1]` to the camera-relative world position (times its w): the camera
    /// rotation times the inverse projection. Camera-relative keeps f32 precision at 16 km.
    pub world_from_clip: Mat4,
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
            blend: Vec4::ZERO,
            focus: Vec4::ZERO,
            // overlay strengths 0 = identity on both branches; HighPass mix / exponent 1
            overlay: Vec4::new(0.0, 0.0, 1.0, 1.0),
            view: Vec4::new(DEFAULT_NEAR, 0.0, 0.0, 0.0),
            cam_pos: Vec4::ZERO,
            world_from_clip: Mat4::IDENTITY,
        }
    }
}

/// Bevy's default perspective near plane, used when the camera has no perspective projection.
pub const DEFAULT_NEAR: f32 = 0.1;

/// Weight of the focus ramp: the native `cb1[5]` (`FUN_009256d0`: `S5 <= 0` gives all zero,
/// otherwise `(S5, 1 / S5, S6, S6 > 0 ? 1 / (0.5 S6) : 0)`).
fn focus_constants(s5: f32, s6: f32) -> Vec4 {
    if s5 <= 0.0 {
        return Vec4::ZERO;
    }
    Vec4::new(
        s5,
        1.0 / s5,
        s6,
        if s6 > 0.0 { 1.0 / (s6 * 0.5) } else { 0.0 },
    )
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
            // cb1[0] = (S26 * 0.004, 0, S4, S25) (FUN_009256d0, `* 0.004` constant read)
            blend: Vec4::new(
                s[post::HEAT_HAZE] * 0.004,
                0.0,
                s[post::BLUR],
                s[post::DOF_GAIN],
            ),
            focus: focus_constants(s[post::DOF_FAR], s[post::DOF_NEAR]),
            overlay: Vec4::new(
                s[post::OVERLAY_BRIGHT],
                s[post::OVERLAY_DARK],
                s[post::HIGHPASS_SATURATION],
                s[post::HIGHPASS_EXPONENT],
            ),
            view: Vec4::new(DEFAULT_NEAR, 0.0, 0.0, 0.0),
            cam_pos: Vec4::ZERO,
            world_from_clip: Mat4::IDENTITY,
        }
    }

    /// The blur blend of the composite for a pixel at eye distance `eye` (m):
    /// `sat(ramp * S25 + S4)` with `ramp = sat((S6 - eye) * w) + sat((eye - S5) / S5)`
    /// (`DepthOfField` PS 4861604 family; `focus` all zero gives `sat(S4)`, the `Scene` family).
    pub fn blur_blend(&self, eye: f32) -> f32 {
        let f = self.focus;
        let ramp = ((f.z - eye) * f.w).clamp(0.0, 1.0) + ((eye - f.x) * f.y).clamp(0.0, 1.0);
        (ramp * self.blend.w + self.blend.z).clamp(0.0, 1.0)
    }

    /// The HighPass pass (`highpass` PS 6147124): `pow(blur, S1)`, saturation mix `S0`, `* A`;
    /// the result is stored in an 8-bit target (clamped).
    #[allow(dead_code)] // CPU reference of the shader, used by the tests
    pub fn highpass(&self, blur: Vec3, a: f32) -> Vec3 {
        let c = blur.max(Vec3::ZERO).powf(self.overlay.w);
        let lum = c.dot(LUMINANCE);
        let c = Vec3::splat(lum) + (c - Vec3::splat(lum)) * self.overlay.z;
        (c * a).clamp(Vec3::ZERO, Vec3::ONE)
    }

    /// The overlay of the composite: `b < 0.5` multiply branch (strength `S3`), otherwise the
    /// screen branch (`S2`), per channel (`b` is the bloom texture value).
    pub fn overlay_blend(&self, c: Vec3, b: Vec3) -> Vec3 {
        let one = |c: f32, b: f32| {
            if b < 0.5 {
                2.0 * c * (self.overlay.y * (b - 0.5) + 0.5)
            } else {
                1.0 - (2.0 - 2.0 * c) * (1.0 - (self.overlay.x * (b - 0.5) + 0.5))
            }
        };
        Vec3::new(one(c.x, b.x), one(c.y, b.y), one(c.z, b.z))
    }

    /// The whole shipped composite (`DepthOfField_EdgeFade` PS 4863324, JC2-103: the gameplay
    /// variant) for one pixel on the CPU: `scene`/`blur` linear colours (t0, t1), `bloom` the
    /// HighPass texture (t4), `eye` the eye distance of the pixel, `a` the auto-exposure,
    /// `uv` the screen position (vignette). Returns the display value `sqrt(C)`.
    #[allow(dead_code)] // CPU reference of the shader, used by the tests
    pub fn composite(
        &self,
        scene: Vec3,
        blur: Vec3,
        bloom: Vec3,
        eye: f32,
        a: f32,
        uv: Vec2,
    ) -> Vec3 {
        let m = self.blur_blend(eye);
        let c = scene + (blur - scene) * m;
        let c = self.overlay_blend(c, bloom);
        let lum = c.dot(LUMINANCE);
        let k = (lum * self.saturation.y + self.saturation.x)
            .clamp(self.saturation.z, self.saturation.w);
        let c = Vec3::splat(lum) + (c - Vec3::splat(lum)) * k;
        let c = c * (a + self.tint.w) * self.tint.truncate() * vignette(uv);
        c.max(Vec3::ZERO).map(f32::sqrt)
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

/// `setting[0x12]`: the level the post predicate compares the camera height with (`FUN_00812c50`,
/// image dword 200.0). Not the physical sea level.
pub const UNDERWATER_LEVEL: f32 = 200.0;

/// The Underwater composite family is chosen when `setting[0x12] > camera.y` (`comiss`, `jbe`):
/// strictly below, so y = 200.0 exactly is above water (JC2-094 section 1).
pub fn is_underwater(camera_y: f32) -> bool {
    UNDERWATER_LEVEL > camera_y
}

/// The camera constants the Underwater composite needs: `cb0[5]` (camera position),
/// the camera-relative world reconstruction matrix and `cb0[6].y` (minus the forward Y; the
/// native row `+0x44` of the camera matrix is the negated forward, JC2-094 section 5).
pub fn underwater_view(position: Vec3, rotation: Quat, clip_from_view: Mat4) -> (Vec4, Mat4, f32) {
    let forward = rotation * Vec3::NEG_Z;
    (
        position.extend(1.0),
        Mat4::from_quat(rotation) * clip_from_view.inverse(),
        -forward.y,
    )
}

/// The caustic gain of the Underwater composite (`C *= gain`, PS 7485260 tail): `dist2` the
/// squared distance from the camera to the pixel's world position, `world_y` its height,
/// `look_y` = `cb0[6].y`, `a2 = 2 (a(0.08 uvc) + 0.5 a(0.3 uvc))` the caustic pattern.
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn caustic_gain(dist2: f32, world_y: f32, look_y: f32, a2: f32) -> f32 {
    let d = (1.0 / (0.07 * dist2)).min(0.4);
    let sg = (UNDERWATER_LEVEL - world_y).clamp(0.0, 1.0);
    let b = (2.0 * (look_y + 0.2).max(0.1)).min(1.0);
    1.0 + b * d * sg * a2 * a2 * a2
}

/// The EdgeFade vignette of every shipped composite (`*_EdgeFade` shaders, JC2-103): a circle in
/// uv space, `1 - sat((length(uv - 0.5) - 0.35) * 2.2)`.
pub fn vignette(uv: Vec2) -> f32 {
    1.0 - (((uv - Vec2::splat(0.5)).length() - 0.35) * 2.2).clamp(0.0, 1.0)
}

/// Eye distance (m) of a pixel from Bevy's reverse-Z infinite-far depth `d` (`d = near / z`).
/// The native pass derives the same view-space distance from its own reverse-Z buffer
/// (`1 / (d * cb1[2].z + cb1[2].w)`); `d = 0` (sky) gives infinity, the native far plane's
/// full ramp.
#[allow(dead_code)] // CPU reference of the shader, used by the tests
pub fn eye_from_depth(near: f32, d: f32) -> f32 {
    near / d.max(1e-9)
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
        // HighPass mix and exponent 1 (the shipped value)
        settings[post::HIGHPASS_SATURATION] = 1.0;
        settings[post::HIGHPASS_EXPONENT] = 1.0;
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
            .add_systems(Startup, (load_post_blocks, load_water_bump))
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
    (zone, data): (Res<ZoneGradientData>, Res<PostEffectData>),
    (mut clock, mut phase): (Local<ClockModel>, Local<f32>),
    real: Res<Time<Real>>,
    mut state: ResMut<PostEffectState>,
    mut cams: Query<(&mut PostPass, Option<&Projection>, Option<&GlobalTransform>)>,
) {
    // the intensity weight of the zone (inferred, see `PostEffectBlocks::params`); the file's
    // constants come with the sky-light tables
    let w = zone.0.as_ref().map_or(1.0, |g| {
        g.effects
            .variant_weight(lighting.zone_set, lighting.zone_intensity)
    });
    state.step(data.0.as_ref(), lighting.zone_set, w, time.hours);
    let mut pass = state.pass();
    // the meter's dt: the smoothed game clock `FUN_004d3690(0)` (JC2-135, native); the elapsed
    // ms are Bevy's real delta rounded to whole ms (inferred), time scale 1.0 (slow motion is
    // not ported), never paused (the tutorial pause is not ported). The GPU clamps 0.01..0.5.
    pass.meter.w = clock.tick((real.delta_secs() * 1000.0).round(), 1.0);
    // the water clock (`FUN_00a7f080`: the same smoothed dt, `* 0.45`, strict wrap at 10000)
    *phase = advance_phase(*phase, pass.meter.w);
    pass.view.w = *phase;
    for (mut p, projection, transform) in &mut cams {
        let mut want = pass;
        // Bevy's reverse-Z depth is `near / z`: the composite needs this camera's near plane
        if let Some(Projection::Perspective(persp)) = projection {
            want.view.x = persp.near;
        }
        if let (Some(projection), Some(transform)) = (projection, transform) {
            let (_, rotation, position) = transform.to_scale_rotation_translation();
            let (cam_pos, world_from_clip, look_y) =
                underwater_view(position, rotation, projection.get_clip_from_view());
            want.cam_pos = cam_pos;
            want.world_from_clip = world_from_clip;
            want.view.z = look_y;
            // the Underwater family: `setting[0x12] (200) > camera.y`, strict (JC2-094 section 1)
            want.view.y = f32::from(is_underwater(position.y));
        }
        if *p != want {
            *p = want;
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
        near(
            p.display(Vec3::new(0.25, 1.0, 4.0), 1.0),
            [0.5, 1.0, 1.0],
            1e-6,
        );
        near(
            p.display(Vec3::new(-1.0, 0.0, 0.04), 1.0),
            [0.0, 0.0, 0.2],
            1e-6,
        );
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
        assert!(
            (dawn.x - 0.15).abs() < 1e-6 && (dawn.y - 0.09).abs() < 1e-6,
            "{dawn:?}"
        );
        // the steady state the GPU reaches for a uniform 0.05 scene (brief 1.4)
        let t = |set, hour| {
            let v = m(set, hour);
            auto_exposure::exposure_target([0.05; 4], v.x, v.y, v.z)
        };
        assert!((t(7, 12.0) - 1.54).abs() < 1e-6, "day");
        assert!((t(7, 22.0) - 1.432).abs() < 1e-6, "night");
        assert!(
            (t(1, 12.0) - 1.216).abs() < 1e-6,
            "Arctic day: 1 + 0.04 * 5.4"
        );
    }

    /// The focus ramp and blur blend (`DepthOfField` PS): day block S5 120, S25 0.4, S4 0;
    /// night S5 150, S25 0.3; no blend inside S5, full gain at 2 S5, sky = full ramp.
    #[test]
    fn the_depth_of_field_blend_follows_the_blocks() {
        let Some(b) = retail() else { return };
        let day = PostPass::from_settings(&b.params(7, 1.0, 12.0));
        assert_eq!(day.focus, Vec4::new(120.0, 1.0 / 120.0, 0.0, 0.0));
        assert_eq!(day.blend.z, 0.0);
        assert!((day.blend.w - 0.4).abs() < 1e-6);
        assert_eq!(day.blur_blend(50.0), 0.0);
        assert_eq!(day.blur_blend(120.0), 0.0);
        assert!(
            (day.blur_blend(180.0) - 0.2).abs() < 1e-6,
            "half ramp x 0.4"
        );
        assert!((day.blur_blend(240.0) - 0.4).abs() < 1e-6);
        assert!((day.blur_blend(5000.0) - 0.4).abs() < 1e-6);
        assert!(
            (day.blur_blend(eye_from_depth(0.1, 0.0)) - 0.4).abs() < 1e-6,
            "sky"
        );
        let night = PostPass::from_settings(&b.params(7, 1.0, 0.0));
        assert_eq!(night.focus.x, 150.0);
        assert!((night.blur_blend(1e6) - 0.3).abs() < 1e-6);
        // Underwater set 9: S5 8, S25 0.65, blur 0.15 (blend = 0.15 + ramp * 0.65, clamped)
        let uw = PostPass::from_settings(&b.params(9, 1.0, 12.0));
        assert_eq!(uw.focus.x, 8.0);
        assert!((uw.blur_blend(4.0) - 0.15).abs() < 1e-6);
        assert!((uw.blur_blend(100.0) - 0.8).abs() < 1e-6);
        // S5 <= 0 (the Scene family): the blend is sat(S4) everywhere
        let mut s = PostEffectState::neutral().settings;
        s[post::BLUR] = 0.3;
        s[post::DOF_GAIN] = 0.9;
        let scene = PostPass::from_settings(&s);
        assert_eq!(scene.focus, Vec4::ZERO);
        assert_eq!(scene.blur_blend(0.5), 0.3);
        assert_eq!(scene.blur_blend(1e6), 0.3);
        // the near focus (S6 > 0): blend rises below S6 with weight 1 / (0.5 S6)
        s[post::DOF_FAR] = 100.0;
        s[post::DOF_NEAR] = 10.0;
        s[post::DOF_GAIN] = 0.5;
        let near = PostPass::from_settings(&s);
        assert_eq!(near.focus, Vec4::new(100.0, 0.01, 10.0, 0.2));
        assert!(
            (near.blur_blend(5.0) - (0.3 + 0.5 * 1.0)).abs() < 1e-6,
            "(10 - 5) * 0.2 = 1"
        );
    }

    /// The overlay (`S2` screen branch above 0.5, `S3` multiply branch below): ks = kp = 0 is the
    /// identity, `b = 0.5` is the identity, and the day block darkens dark bloom.
    #[test]
    fn the_overlay_branches() {
        let id = PostPass::default();
        let c = Vec3::new(0.2, 0.5, 0.9);
        near(
            id.overlay_blend(c, Vec3::new(0.1, 0.5, 0.9)),
            c.to_array(),
            1e-6,
        );
        let p = PostPass {
            overlay: Vec4::new(0.1, 0.7, 1.0, 1.0),
            ..default()
        };
        near(p.overlay_blend(c, Vec3::splat(0.5)), c.to_array(), 1e-6);
        // b = 0: multiply branch 2 C (0.7 * -0.5 + 0.5) = 0.3 C
        near(p.overlay_blend(c, Vec3::ZERO), (c * 0.3).to_array(), 1e-6);
        // b = 1: screen branch 1 - (2 - 2C) (1 - 0.55)
        let want = Vec3::ONE - (Vec3::splat(2.0) - c * 2.0) * 0.45;
        near(p.overlay_blend(c, Vec3::ONE), want.to_array(), 1e-6);
    }

    /// HighPass (`S0` mix, `S1` exponent, `* A`) with the shipped S0 = S1 = 1 is `blur * A`.
    #[test]
    fn highpass_is_the_blur_times_a_with_the_shipped_settings() {
        let Some(b) = retail() else { return };
        let day = PostPass::from_settings(&b.params(7, 1.0, 12.0));
        assert_eq!((day.overlay.z, day.overlay.w), (1.0, 1.0));
        near(
            day.highpass(Vec3::new(0.1, 0.2, 0.3), 1.5),
            [0.15, 0.3, 0.45],
            1e-6,
        );
        // the 8-bit target clamps
        near(day.highpass(Vec3::splat(0.8), 2.0), [1.0; 3], 1e-6);
        // exponent 2 and mix 0 (grey): pow, then luminance
        let p = PostPass {
            overlay: Vec4::new(0.0, 0.0, 0.0, 2.0),
            ..default()
        };
        let g = p.highpass(Vec3::new(0.5, 0.5, 0.5), 1.0);
        near(g, [0.25; 3], 1e-6);
    }

    /// The vignette (`1 - sat((length(uv - 0.5) - 0.35) * 2.2)`): centre 1, 1 to r = 0.35,
    /// 0.67 at the edge middles, 0.215 in the corners (JC2-103 section 6).
    #[test]
    fn the_vignette_matches_the_brief() {
        assert_eq!(vignette(Vec2::splat(0.5)), 1.0);
        assert!((vignette(Vec2::new(0.85, 0.5)) - 1.0).abs() < 1e-6);
        assert!((vignette(Vec2::new(1.0, 0.5)) - 0.67).abs() < 1e-6);
        assert!((vignette(Vec2::new(0.0, 0.0)) - 0.2146).abs() < 1e-3);
        assert_eq!(vignette(Vec2::new(0.5, 1.9)), 0.0);
    }

    /// The whole composite with neutral settings is the old pass; with the blocks it ties the
    /// pieces together (day, no ramp inside 120 m, A = 1, centre pixel).
    #[test]
    fn the_composite_reduces_to_the_old_pass_without_blend_and_overlay() {
        let p = PostPass::default();
        let uv = Vec2::splat(0.5);
        let scene = Vec3::new(0.25, 0.04, 0.0);
        near(
            p.composite(scene, Vec3::ONE, Vec3::splat(0.9), 1e6, 1.0, uv),
            p.display(scene, 1.0).to_array(),
            1e-6,
        );
        // the vignette multiplies the colour before the sqrt: corner factor 0.2146
        let corner = p.composite(
            Vec3::splat(0.5),
            Vec3::ZERO,
            Vec3::splat(0.5),
            1.0,
            1.0,
            Vec2::ZERO,
        );
        assert!((corner.x - (0.5 * vignette(Vec2::ZERO)).sqrt()).abs() < 1e-6);
    }

    /// The Underwater family selection is strict: y = 200.0 exactly is above water
    /// (JC2-094 section 1 fixtures: 250, 200.0, 200.00001526 above; 199.99998474 and 150 below).
    #[test]
    fn the_underwater_family_is_selected_below_200_strictly() {
        for y in [250.0, 200.0, f32::from_bits(200.0_f32.to_bits() + 1)] {
            assert!(!is_underwater(y), "{y}");
        }
        for y in [f32::from_bits(200.0_f32.to_bits() - 1), 150.0] {
            assert!(is_underwater(y), "{y}");
        }
    }

    /// The caustic gain of the research's illustrative pixel (eye 20 m, `Sg = 1`, `B = 1`,
    /// `A2 = 0.9`: gain 1.026) and its boundaries (JC2-094 section 7).
    #[test]
    fn the_caustic_gain_follows_the_shader_arithmetic() {
        let g = caustic_gain(400.0, 100.0, 0.3, 0.9);
        assert!((g - 1.026).abs() < 5e-4, "{g}");
        // A2 = 0 and the surface (world y 200, Sg = 0) leave the colour unchanged
        assert_eq!(caustic_gain(400.0, 100.0, 0.3, 0.0), 1.0);
        assert_eq!(caustic_gain(400.0, 200.0, 0.3, 0.9), 1.0);
        // D is capped at 0.4 within 5.976 m; B falls to 0.2 looking up (cb0[6].y <= -0.1)
        assert!((caustic_gain(0.0, 100.0, 1.0, 1.0) - 1.4).abs() < 1e-6);
        assert!((caustic_gain(0.0, 100.0, -1.0, 1.0) - 1.08).abs() < 1e-6);
    }

    /// The reconstruction matrix turns the pixel centre at depth `d` into the point `near / d`
    /// metres ahead, relative to the camera, and `cb0[6].y` is minus the forward Y.
    #[test]
    fn the_underwater_view_reconstructs_the_world_position() {
        let near = 0.1;
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, near);
        // looking down: forward = -Y
        let rot = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let (pos, m, look_y) = underwater_view(Vec3::new(10.0, 150.0, -3.0), rot, proj);
        assert_eq!(pos, Vec4::new(10.0, 150.0, -3.0, 1.0));
        assert!((look_y - 1.0).abs() < 1e-6);
        let p = m * Vec4::new(0.0, 0.0, 0.01, 1.0);
        let rel = p.truncate() / p.w;
        assert!(
            (rel - Vec3::new(0.0, -10.0, 0.0)).abs().max_element() < 1e-3,
            "{rel}"
        );
        // a level camera looking along -Z: look_y 0, the ray of an upper pixel rises
        let (_, m, look_y) = underwater_view(Vec3::ZERO, Quat::IDENTITY, proj);
        assert!(look_y.abs() < 1e-6);
        let p = m * Vec4::new(0.0, 0.5, 0.1, 1.0);
        let rel = p.truncate() / p.w;
        assert!(rel.y > 0.0 && (rel.z + 1.0).abs() < 1e-4, "{rel}");
    }

    /// The camera near plane comes from the projection into the pass view field.
    #[test]
    fn the_system_carries_the_camera_near_plane() {
        let mut app = App::new();
        app.insert_resource(TimeOfDay {
            hours: 12.0,
            speed: 0.0,
            paused: true,
        })
        .init_resource::<SkyLighting>()
        .init_resource::<ZoneGradientData>()
        .init_resource::<Time<Real>>()
        .init_resource::<PostEffectData>()
        .init_resource::<PostEffectState>()
        .add_systems(Update, update_post_effects);
        let cam = app
            .world_mut()
            .spawn((
                PostPass::default(),
                Projection::Perspective(PerspectiveProjection {
                    near: 0.25,
                    ..default()
                }),
            ))
            .id();
        let bare = app.world_mut().spawn(PostPass::default()).id();
        app.update();
        assert_eq!(app.world().get::<PostPass>(cam).unwrap().view.x, 0.25);
        assert_eq!(
            app.world().get::<PostPass>(bare).unwrap().view.x,
            DEFAULT_NEAR
        );
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
            // field order of PostPass: tint, saturation, meter, blend, focus, overlay, view (vec4s)
            assert!(src.find("tint: vec4<f32>") < src.find("saturation: vec4<f32>"));
            assert!(src.find("saturation: vec4<f32>") < src.find("meter: vec4<f32>"));
            assert!(src.find("meter: vec4<f32>") < src.find("blend: vec4<f32>"));
            assert!(src.find("blend: vec4<f32>") < src.find("focus: vec4<f32>"));
            assert!(src.find("focus: vec4<f32>") < src.find("overlay: vec4<f32>"));
            assert!(src.find("overlay: vec4<f32>") < src.find("view: vec4<f32>"));
            assert!(src.find("view: vec4<f32>") < src.find("cam_pos: vec4<f32>"));
            assert!(src.find("cam_pos: vec4<f32>") < src.find("world_from_clip: mat4x4<f32>"));
        }
        assert_eq!(std::mem::size_of::<PostPass>(), 192);
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
