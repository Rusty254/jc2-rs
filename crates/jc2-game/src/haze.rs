//! The native haze fog (JC2-155; research `research/formats/haze-fog-next.md`, JC2-116).
//!
//! The exe has one atmospheric fog layer, set every frame by `FUN_00a432d0` and uploaded to the
//! vertex constants `cb0[12]` / `cb0[13]` and the pixel constant `cb0[13]`:
//!
//! * the fog **colour** is `lerp(row 3, row 2, bc)` of the game-zone gradient state
//!   ([`LightState::haze_away`] / [`LightState::haze_toward_sun`], no gain, the same 1 s smoothing
//!   as the sun and ambient), with `bc = (1 + fwd_xz . toSun_xz / |toSun_xz|) / 2`
//!   ([`sun_blend`]): row 2 looking toward the sun, row 3 away. Hour enters through the rows.
//! * the five **numbers** `p0..p4` come from `landscape_effects.bin` section `19a8fc4a`
//!   ([`HazeSettings::params`]): night group lerped to the day group over the dawn/dusk windows;
//! * the fog **exponent** per vertex is `e = (d - p0)/(p1 - p0) + sat(d p2) (1 - min((y - p3)/p4,
//!   hc))`, `hc = sat((camY - p3)/p4)` ([`vs_constants`], [`fog_exponent`]), and the pixel
//!   keeps `T = min(2^-e, 1)` of the lit colour: `out = fog + (lit - fog) T`
//!   ([`transmittance`]; shader side `haze_fog.wgsl`).
//!
//! [`HazeFog`] is the resource that holds one frame of this; [`update_haze_fog`] fills it from the
//! clock, the zone light state, the camera and the sun. The custom WGSL materials (terrain,
//! General, foliage) read `c12`, `c13` and the colour from the shared `SunHorizon` storage buffer
//! (`sun_horizon.rs` writes them) and apply the exact term. Everything else that remains a
//! `StandardMaterial` keeps Bevy's [`DistanceFog`], set by [`HazeFog::distance_fog`]: an
//! **approximation** (exponential in distance with density `ln 2 / (p1 - p0)`, no height term, no
//! `p0` offset, no density ramp), labelled as such.
//!
//! **Not native / not done**
//!
//! * Rain and snow weights (`w_rain`, `w_snow`) come from [`AtmosphereState`] and stay 0 until the
//!   weather producer exists (JC2-156): only the clear path is live; the variant-1/2 path
//!   (parameter groups, tables `A`/`B` variants 1/2) is implemented and tested but unfed.
//! * The gradient rows are the base case of `FUN_00a5d390` (`sky_light.rs`): no altitude,
//!   underwater, rain or snow rows (JC2-132); the fog colour inherits that.
//! * Tables `E`/`F` are the **water** colour (`cb0[14]`) and are not used here.
//! * The sky dome and clear colour are still project policy (Gap 5), and the fog-gradient horizon
//!   band (`CRenderBlockFogGradient`) is not drawn (Gap 1), so the far terrain edge shows against
//!   the policy sky colour.
//! * The camera near/far in play (Gap 2): `d` is Bevy's `clip.w - clip.z` (eye depth - near).
//! * Degenerate numbers (`p1 == p0` or `p4 == 0`, never in retail) turn the term off here; the exe
//!   would divide by zero.

use crate::atmosphere::{AtmosphereState, GradientData};
use crate::sky::TimeOfDay;
use crate::sky_light::{SkyLighting, ZoneGradientData};
use crate::sun_horizon::HorizonShadow;
use bevy::asset::{AssetEventSystems, load_embedded_asset};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use jc2_formats::weather::{GradientTables, HazeSettings, RgbTable};

/// Keeps the shared `jc2::haze_fog` shader module loaded.
#[derive(Resource)]
pub struct HazeFogShader(#[allow(dead_code)] pub Handle<Shader>);

/// Embed and load `haze_fog.wgsl` once; every plugin whose shader imports `jc2::haze_fog` calls
/// this.
pub fn ensure_shader(app: &mut App) {
    if !app.world().contains_resource::<HazeFogShader>() {
        bevy::asset::embedded_asset!(app, "haze_fog.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "haze_fog.wgsl");
        app.insert_resource(HazeFogShader(shader));
    }
}

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// `1 / x`, or 0 for a zero denominator (the exe would produce an infinity).
fn recip(x: f32) -> f32 {
    if x == 0.0 { 0.0 } else { 1.0 / x }
}

/// `FUN_0095b150`, layer 0: the vertex constants for the numbers `p` and the camera height.
/// `cb0[12] = (p0/(p1 - p0), p1, 1/(p1 - p0), camY)`,
/// `cb0[13] = (p3/p4, sat((camY - p3)/p4), 1/p4, p2)`.
pub fn vs_constants(p: [f32; 5], cam_y: f32) -> (Vec4, Vec4) {
    let [p0, p1, p2, p3, p4] = p;
    let k = recip(p1 - p0);
    let inv4 = recip(p4);
    (
        Vec4::new(k * p0, p1, k, cam_y),
        Vec4::new(inv4 * p3, sat((cam_y - p3) * inv4), inv4, p2),
    )
}

/// The base-2 fog exponent the vertex shader writes: `d` the clip `w - z`, `y` the world height.
#[allow(dead_code)] // CPU mirror of haze_fog.wgsl (tests, inspection)
pub fn fog_exponent(c12: Vec4, c13: Vec4, d: f32, y: f32) -> f32 {
    let h = y * c13.z - c13.x;
    let inner = sat(d * c13.w) * (1.0 - h.min(c13.y)) - c12.x;
    d * c12.z + inner
}

/// The pixel shader's surviving fraction of the lit colour: `min(2^-e, 1)` (no lower cap).
#[allow(dead_code)] // CPU mirror of haze_fog.wgsl (tests, inspection)
pub fn transmittance(e: f32) -> f32 {
    (-e).exp2().min(1.0)
}

/// `bc` of `FUN_00a432d0`: 1 looking along the sun's azimuth, 0 away, 0.5 sideways and looking
/// straight up or down. Native: the camera matrix row `+0x44` is minus the forward vector, the
/// argument is `L = -(direction to the sun)`, `dot = (r.x L.x + r.z L.z) / |L.xz|` (divisor 1 when
/// `|L.xz|^2 < 1e-6`) and `bc = (dot + 1) / 2`. Only the horizontal part of the forward vector
/// counts and it is not renormalised.
pub fn sun_blend(cam_forward: Vec3, to_sun: Vec3) -> f32 {
    let row = -cam_forward;
    let l = -to_sun;
    let n2 = l.x * l.x + l.z * l.z;
    let k = if n2 - 1e-6 >= 0.0 {
        1.0 / n2.sqrt()
    } else {
        1.0
    };
    (row.x * l.x * k + row.z * l.z * k + 1.0) * 0.5
}

/// The fog colour `cb0[13].rgb` in native scene units: `lerp(away, toward, bc)` of the zone
/// gradient rows, then toward `lerp(B, A, bc)` of variant 1 (rain) by `w_rain` and variant 2
/// (snow) by `w_snow` (each only when its weight is positive). The variant naming is inferred
/// (haze-fog-next.md section 8); `tables` are `gradients.bmp`.
pub fn fog_colour(
    toward_sun: Vec3,
    away: Vec3,
    tables: Option<&GradientTables>,
    hour: f32,
    bc: f32,
    w_rain: f32,
    w_snow: f32,
) -> Vec3 {
    let mut col = away.lerp(toward_sun, bc);
    if let Some(t) = tables {
        for (w, variant) in [(w_rain, 1), (w_snow, 2)] {
            if w > 0.0 {
                let a = Vec3::from(t.rgb_at(RgbTable::A, variant, hour));
                let b = Vec3::from(t.rgb_at(RgbTable::B, variant, hour));
                col = col.lerp(b.lerp(a, bc), w);
            }
        }
    }
    col
}

/// One frame of the haze: what the GPU constants and the fallback `DistanceFog` are built from.
/// [`Default`] is "no fog" (all zero), which the shaders treat as transmittance 1.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct HazeFog {
    /// `cb0[13].rgb`: the fog colour in native scene units (linear, unclamped).
    pub colour: Vec3,
    /// `p0..p4` (start, end, density, p3, p4).
    pub p: [f32; 5],
    /// Vertex `cb0[12]`.
    pub c12: Vec4,
    /// Vertex `cb0[13]`.
    pub c13: Vec4,
    /// The sun blend `bc` the colour used (kept for inspection).
    pub bc: f32,
}

impl Default for HazeFog {
    fn default() -> Self {
        Self {
            colour: Vec3::ZERO,
            p: [0.0; 5],
            c12: Vec4::ZERO,
            c13: Vec4::ZERO,
            bc: 0.0,
        }
    }
}

impl HazeFog {
    /// The fog of one frame. `rows` are the smoothed gradient rows 2 and 3 (`toward_sun`,
    /// `away`), `to_sun` the unit vector toward the unflipped sun and `cam_forward` the camera's
    /// forward vector (both Bevy = engine axes).
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        settings: &HazeSettings,
        toward_sun: Vec3,
        away: Vec3,
        tables: Option<&GradientTables>,
        hour: f32,
        cam_y: f32,
        cam_forward: Vec3,
        to_sun: Vec3,
        w_rain: f32,
        w_snow: f32,
    ) -> Self {
        let p = settings.params(hour, w_rain, w_snow);
        let (c12, c13) = vs_constants(p, cam_y);
        let bc = sun_blend(cam_forward, to_sun);
        Self {
            colour: fog_colour(toward_sun, away, tables, hour, bc, w_rain, w_snow),
            p,
            c12,
            c13,
            bc,
        }
    }

    /// The exponent at clip-space `d` and world height `y`.
    #[allow(dead_code)] // CPU mirror (tests)
    pub fn exponent(&self, d: f32, y: f32) -> f32 {
        fog_exponent(self.c12, self.c13, d, y)
    }

    /// The pixel colour's surviving fraction at `d` and `y`.
    #[allow(dead_code)] // CPU mirror (tests)
    pub fn transmittance(&self, d: f32, y: f32) -> f32 {
        transmittance(self.exponent(d, y))
    }

    /// `cb0[13]` as the pixel shader gets it: colour and `p2`.
    pub fn gpu_colour(&self) -> Vec4 {
        self.colour.extend(self.p[2])
    }

    /// Density of the **fallback** exponential fog: `T = 2^-(d / (p1 - p0))`, i.e. the slope of
    /// the exact term beyond the density ramp, `ln 2 / (p1 - p0)` per metre. Non-native shape.
    pub fn fallback_density(&self) -> f32 {
        let span = self.p[1] - self.p[0];
        if span > 0.0 {
            std::f32::consts::LN_2 / span
        } else {
            0.0
        }
    }

    /// The `DistanceFog` for meshes that stay `StandardMaterial` (**labelled approximation** of
    /// the native term: colour exact, shape exponential in distance only, no sun scatter).
    pub fn distance_fog(&self) -> DistanceFog {
        let c = self.colour;
        DistanceFog {
            color: Color::linear_rgb(c.x, c.y, c.z),
            directional_light_color: Color::NONE,
            directional_light_exponent: 8.0,
            falloff: FogFalloff::Exponential {
                density: self.fallback_density(),
            },
        }
    }
}

/// Every frame: the haze from the clock, the zone light rows, the camera and the sun, and the
/// fallback `DistanceFog` on the camera. Without the gradient files there is no fog.
#[allow(clippy::too_many_arguments)]
pub fn update_haze_fog(
    time: Res<TimeOfDay>,
    lighting: Res<SkyLighting>,
    atmosphere: Res<AtmosphereState>,
    gradients: Res<GradientData>,
    zone: Res<ZoneGradientData>,
    mut haze: ResMut<HazeFog>,
    mut cams: Query<(&GlobalTransform, Option<&mut DistanceFog>), With<Camera3d>>,
) {
    let Some((tf, _)) = cams.iter().next() else {
        return;
    };
    let now = match zone.0.as_ref() {
        Some(z) => HazeFog::evaluate(
            &z.effects.haze,
            lighting.state.haze_toward_sun,
            lighting.state.haze_away,
            gradients.0.as_ref(),
            time.hours,
            tf.translation().y,
            tf.forward().as_vec3(),
            atmosphere.sun_dir,
            atmosphere.w1,
            atmosphere.w2,
        ),
        None => HazeFog::default(),
    };
    if *haze != now {
        *haze = now;
    }
    let want = now.distance_fog();
    for (_, fog) in &mut cams {
        if let Some(mut fog) = fog {
            fog.color = want.color;
            fog.directional_light_color = want.directional_light_color;
            fog.directional_light_exponent = want.directional_light_exponent;
            fog.falloff = want.falloff.clone();
        }
    }
}

/// The sky-card factor `F` (fraction of the fog colour mixed in) of the `cirrusclouds` / `clouds`
/// vertex shaders (JC2-163): radial distance from the camera scaled by `scale` (0.25 cirrus,
/// 0.7 clouds), the height scaled about the camera height, the exponent saturated, and for clouds
/// remapped `max(2 (e - 0.5), 0)`. CPU mirror of `haze_sky_card_factor` in `haze_fog.wgsl`.
#[allow(dead_code)] // CPU mirror of haze_fog.wgsl (tests, inspection)
pub fn sky_card_factor(
    c12: Vec4,
    c13: Vec4,
    len: f32,
    y: f32,
    cam_y: f32,
    scale: f32,
    cloud: bool,
) -> f32 {
    let len = len * scale;
    let y = (y - cam_y) * scale + cam_y;
    let h = y * c13.z - c13.x;
    let mut e = sat(len * c12.z + (sat(len * c13.w) * (1.0 - h.min(c13.y)) - c12.x));
    if cloud {
        e = ((e - 0.5) * 2.0).max(0.0);
    }
    1.0 - (-e).exp2()
}

/// The constants bound when there is no terrain horizon (model viewer, fixtures): all zero, i.e.
/// no fog.
#[derive(Resource)]
pub struct HazeFallback(pub Handle<ShaderBuffer>);

fn make_haze_fallback(mut commands: Commands, buffers: Option<ResMut<Assets<ShaderBuffer>>>) {
    if let Some(mut buffers) = buffers {
        commands.insert_resource(HazeFallback(
            buffers.add(ShaderBuffer::from(crate::sun_horizon::SunHorizon::default())),
        ));
    }
}

/// A custom material that binds the shared `SunHorizon` buffer (binding 101) for the haze fog
/// fields; see [`add_haze_binding`].
pub trait HazeBound: Asset {
    fn haze_buffer(&self) -> &Handle<ShaderBuffer>;
    fn set_haze_buffer(&mut self, buffer: Handle<ShaderBuffer>);
}

#[derive(Resource)]
struct HazeFallbackRegistered;

/// Register the all-zero fallback buffer (once) and the system that points every `M` at the
/// shared `SunHorizon` buffer.
pub fn add_haze_binding<M: HazeBound>(app: &mut App) {
    if !app.world().contains_resource::<HazeFallbackRegistered>() {
        app.insert_resource(HazeFallbackRegistered);
        app.add_systems(Startup, make_haze_fallback);
    }
    app.add_systems(PostUpdate, bind_haze::<M>.before(AssetEventSystems));
}

/// Point every `M` at the shared `SunHorizon` buffer (whose fog fields
/// `sun_horizon::update_horizon` writes every frame): the terrain's when there is one and it is
/// enabled, otherwise the all-zero fallback. Writes only materials that differ.
fn bind_haze<M: HazeBound>(
    horizon: Option<Res<HorizonShadow>>,
    fallback: Option<Res<HazeFallback>>,
    mut materials: ResMut<Assets<M>>,
) {
    let Some(fallback) = fallback else { return };
    let buffer = match &horizon {
        Some(h) if h.enabled => h.params.clone(),
        _ => fallback.0.clone(),
    };
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.haze_buffer().id() != buffer.id())
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.set_haze_buffer(buffer.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky_light::{ZoneGradients, light_state};
    use jc2_formats::GameFs;
    use jc2_formats::weather::{
        GAMEZONE_GRADIENTS_FILE, GameZoneGradients, HAZE_DAY, HAZE_NIGHT, LANDSCAPE_EFFECTS_FILE,
        LandscapeEffects,
    };

    /// The retail section `19a8fc4a` (probe `PARAMS_EXPECT` / `WIN_EXPECT`), for tests that need
    /// no game files.
    fn retail_settings() -> HazeSettings {
        HazeSettings {
            windows: [5.5, 18.5, 20.0, 4.0],
            groups: [
                [200.0, 12_000.0, 0.001, -1806.0, 3006.0],
                [300.0, 9_000.0, 0.007, -1800.0, 4000.0],
                [-50.0, 4_869.0, 0.005, -200.0, 800.0],
                [0.0, 1_500.0, 0.007, -200.0, 800.0],
                [0.0, 3_000.0, 0.01, -1806.0, 6000.0],
                [0.0, 3_000.0, 0.01, -1800.0, 6000.0],
            ],
        }
    }

    fn game() -> Option<GameFs> {
        GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()
    }

    fn retail_zone() -> Option<ZoneGradients> {
        let fs = game()?;
        (fs.contains(GAMEZONE_GRADIENTS_FILE) && fs.contains(LANDSCAPE_EFFECTS_FILE)).then(|| {
            ZoneGradients {
                tables: GameZoneGradients::load(&fs).expect("gamezone_gradients.bmp parses"),
                effects: LandscapeEffects::load(&fs).expect("landscape_effects.bin parses"),
            }
        })
    }

    fn close(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() <= eps * b.abs().max(1.0), "{a} vs {b}");
    }

    #[test]
    fn the_retail_file_gives_the_probe_windows_and_groups() {
        let Some(z) = retail_zone() else { return };
        assert_eq!(z.effects.haze, retail_settings());
    }

    #[test]
    fn the_constructor_state_is_the_exes_before_the_file_is_read() {
        let d = HazeSettings::default();
        assert_eq!(d.windows, [8.0, 17.0, 19.0, 5.0]);
        assert!(
            d.groups
                .iter()
                .all(|g| *g == [0.0, 32_000.0, 0.0, 0.0, 100.0])
        );
    }

    /// Probe `EXPECT_PARAMS`: the day factor and p0..p4 over the windows (clear weather).
    #[test]
    fn day_factor_and_parameters_follow_the_windows() {
        let s = retail_settings();
        let rows: [(f32, f32, [f32; 5]); 11] = [
            (0.0, 0.0, [300.0, 9000.0, 0.007, -1800.0, 4000.0]),
            (4.0, 0.0, [300.0, 9000.0, 0.007, -1800.0, 4000.0]),
            (
                5.0,
                0.6667,
                [233.333_33, 11_000.0, 0.003, -1804.0, 3337.3333],
            ),
            (5.5, 1.0, [200.0, 12000.0, 0.001, -1806.0, 3006.0]),
            (6.0, 1.0, [200.0, 12000.0, 0.001, -1806.0, 3006.0]),
            (12.0, 1.0, [200.0, 12000.0, 0.001, -1806.0, 3006.0]),
            (18.0, 1.0, [200.0, 12000.0, 0.001, -1806.0, 3006.0]),
            (18.5, 1.0, [200.0, 12000.0, 0.001, -1806.0, 3006.0]),
            (19.25, 0.5, [250.0, 10500.0, 0.004, -1803.0, 3503.0]),
            (20.0, 0.0, [300.0, 9000.0, 0.007, -1800.0, 4000.0]),
            (23.0, 0.0, [300.0, 9000.0, 0.007, -1800.0, 4000.0]),
        ];
        for (h, t, p) in rows {
            close(s.day_factor(h), t, 1e-4);
            let got = s.params(h, 0.0, 0.0);
            for i in 0..5 {
                close(got[i], p[i], 1e-5);
            }
        }
        // the window edges: 4 / 5.5 / 18.5 / 20 are the ends of the ramps
        assert_eq!(s.day_factor(4.0), 0.0);
        assert_eq!(s.day_factor(5.5), 1.0);
        assert_eq!(s.day_factor(18.5), 1.0);
        assert_eq!(s.day_factor(20.0), 0.0);
        close(s.day_factor(4.75), 0.5, 1e-6);
        close(s.day_factor(19.0), 2.0 / 3.0, 1e-6);
        // the groups: day and night are the file's
        assert_eq!(s.groups[HAZE_DAY][1], 12_000.0);
        assert_eq!(s.groups[HAZE_NIGHT][1], 9_000.0);
    }

    #[test]
    fn rain_and_snow_blend_their_groups_in_when_weighted() {
        let s = retail_settings();
        // noon, full rain: the rain day group; half snow: halfway to the snow day group
        let rain = s.params(12.0, 1.0, 0.0);
        assert_eq!(rain, [-50.0, 4_869.0, 0.005, -200.0, 800.0]);
        let snow_half = s.params(12.0, 0.0, 0.5);
        close(snow_half[1], (12_000.0 + 3_000.0) / 2.0, 1e-6);
        // midnight rain uses the rain night group
        assert_eq!(
            s.params(0.0, 1.0, 0.0),
            [0.0, 1_500.0, 0.007, -200.0, 800.0]
        );
        // rain then snow both applied in order
        let both = s.params(12.0, 0.5, 0.5);
        let a = s.params(12.0, 0.5, 0.0);
        let b = [0.0, 3_000.0, 0.01, -1806.0, 6_000.0];
        for i in 0..5 {
            close(both[i], a[i] + (b[i] - a[i]) * 0.5, 1e-6);
        }
    }

    /// Probe `vs_constants`: noon and midnight at camera height 202.
    #[test]
    fn vertex_constants_match_the_probe() {
        let s = retail_settings();
        let (c12, c13) = vs_constants(s.params(12.0, 0.0, 0.0), 202.0);
        close(c12.x, 0.016_949, 1e-4);
        assert_eq!((c12.y, c12.w), (12_000.0, 202.0));
        close(c12.z, 8.4746e-5, 1e-4);
        close(c13.x, -0.600_798, 1e-5);
        close(c13.y, 0.667_997, 1e-5);
        close(c13.z, 0.000_333, 1e-3);
        close(c13.w, 0.001, 1e-6);
        let (c12, c13) = vs_constants(s.params(0.0, 0.0, 0.0), 202.0);
        close(c12.x, 0.034_483, 1e-5);
        assert_eq!(c12.y, 9_000.0);
        close(c12.z, 0.000_115, 1e-2);
        close(c13.x, -0.45, 1e-6);
        close(c13.y, 0.5005, 1e-6);
        close(c13.z, 0.000_25, 1e-6);
        close(c13.w, 0.007, 1e-6);
    }

    /// Probe `EXPECT_TR`: camera and ground at y = 202, clear weather.
    #[test]
    fn exponent_and_transmittance_match_the_probe_table() {
        let s = retail_settings();
        let rows: [(f32, f32, f32, f32); 8] = [
            (12.0, 100.0, 0.0247, 0.983),
            (12.0, 1_000.0, 0.3998, 0.758),
            (12.0, 5_000.0, 0.7388, 0.5992),
            (12.0, 20_000.0, 2.01, 0.2483),
            (0.0, 100.0, 0.3267, 0.7974),
            (0.0, 1_000.0, 0.58, 0.669),
            (0.0, 5_000.0, 1.0397, 0.4864),
            (0.0, 20_000.0, 2.7639, 0.1472),
        ];
        for (h, d, e, t) in rows {
            let (c12, c13) = vs_constants(s.params(h, 0.0, 0.0), 202.0);
            let got = fog_exponent(c12, c13, d, 202.0);
            assert!((got - e).abs() < 1e-4, "h{h} d{d}: e {got} vs {e}");
            assert!((transmittance(got) - t).abs() < 1e-4, "h{h} d{d}: T");
        }
    }

    /// The shared WGSL module (all fog functions, the sky-card one included) parses and validates.
    #[test]
    fn the_fog_module_validates() {
        let src: String = include_str!("haze_fog.wgsl")
            .lines()
            .filter(|l| !l.starts_with("#define_import_path"))
            .collect::<Vec<_>>()
            .join("\n");
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(src.contains("fn haze_sky_card_factor"));
    }

    /// JC2-163: the cloud and cirrus factor (VS 104892 / 106016 of the shader bundle).
    #[test]
    fn sky_card_factors_follow_the_cloud_and_cirrus_shaders() {
        let (c12, c13) = vs_constants(retail_settings().params(12.0, 0.0, 0.0), 202.0);
        // no fog constants: no fog
        let z = Vec4::ZERO;
        assert_eq!(
            sky_card_factor(z, z, 9_000.0, 3_000.0, 202.0, 0.7, true),
            0.0
        );
        assert_eq!(
            sky_card_factor(z, z, 9_000.0, 3_000.0, 202.0, 0.25, false),
            0.0
        );
        // cirrus: len 20000 -> 5000 (density ramp 1); y 5202 -> 1452, above the camera's own
        // height factor, so the height term is 1 - hc
        let e = (5_000.0f32 * c12.z + (1.0 - c13.y) - c12.x).clamp(0.0, 1.0);
        let f = sky_card_factor(c12, c13, 20_000.0, 5_202.0, 202.0, 0.25, false);
        assert!((f - (1.0 - (-e).exp2())).abs() < 1e-5, "{f} vs e {e}");
        // clouds: near cards sit in the dead zone (e < 0.5 gives 0), far ones do not
        assert_eq!(
            sky_card_factor(c12, c13, 1_000.0, 3_000.0, 202.0, 0.7, true),
            0.0
        );
        let far = sky_card_factor(c12, c13, 40_000.0, 4_000.0, 202.0, 0.7, true);
        assert!(far > 0.0 && far < 1.0, "{far}");
        assert!(far >= sky_card_factor(c12, c13, 20_000.0, 4_000.0, 202.0, 0.7, true));
    }

    #[test]
    fn transmittance_is_capped_at_one_only() {
        // negative exponents (before p0) clamp to 1; large ones tend to 0 without a cap
        assert_eq!(transmittance(-3.0), 1.0);
        assert_eq!(transmittance(0.0), 1.0);
        assert_eq!(transmittance(1.0), 0.5);
        assert!(transmittance(20.0) > 0.0 && transmittance(20.0) < 1e-5);
        // all-zero constants mean no fog at any distance
        assert_eq!(fog_exponent(Vec4::ZERO, Vec4::ZERO, 12_345.0, 700.0), 0.0);
        assert_eq!(HazeFog::default().transmittance(40_000.0, 200.0), 1.0);
    }

    #[test]
    fn ground_below_the_camera_is_hazier_and_the_camera_height_caps_it() {
        // noon, camera at y = 202: a vertex lower than the camera has h < hc, so (1 - h) is
        // larger; a vertex at or above the camera's height sees (1 - hc) (min(h, hc) = hc)
        let s = retail_settings();
        let (c12, c13) = vs_constants(s.params(12.0, 0.0, 0.0), 202.0);
        let at = |y: f32| fog_exponent(c12, c13, 5_000.0, y);
        let cam = at(202.0);
        assert_eq!(
            cam,
            at(700.0),
            "above the camera the height term is capped by hc"
        );
        assert_eq!(cam, at(1_300.0));
        assert!(at(0.0) > cam && at(-1_000.0) > at(0.0));
        // the base of the layer (y = p3 = -1806): h = 0, the full density term
        close(at(-1_806.0), 5_000.0 * c12.z - c12.x + 1.0, 1e-5);
        // a camera high above the layer (y = 1300 > p3 + p4 = 1200): hc = 1, the term vanishes
        // for everything at or above the camera and only the p0..p1 ramp is left
        let (c12, c13) = vs_constants(s.params(12.0, 0.0, 0.0), 1_300.0);
        assert_eq!(c13.y, 1.0);
        close(
            fog_exponent(c12, c13, 5_000.0, 1_300.0),
            5_000.0 * c12.z - c12.x,
            1e-5,
        );
    }

    #[test]
    fn the_sun_blend_is_one_toward_zero_away_half_sideways_and_up() {
        let sun = Vec3::new(1.0, 0.5, 0.0);
        close(sun_blend(Vec3::X, sun), 1.0, 1e-6);
        close(sun_blend(-Vec3::X, sun), 0.0, 1e-6);
        close(sun_blend(Vec3::Z, sun), 0.5, 1e-6);
        close(sun_blend(Vec3::Y, sun), 0.5, 1e-6);
        // only the horizontal part counts and it is not renormalised: a forward vector tilted
        // 60 degrees from the sun's azimuth with a vertical part gives (1 + cos * horizontal) / 2
        let fwd = Vec3::new(0.5, 0.5, 0.0).normalize();
        close(sun_blend(fwd, sun), (1.0 + 0.5_f32.sqrt()) / 2.0, 1e-5);
        // the sun straight overhead: |L.xz| is ~0, the divisor is 1, and bc = (row.xz . L.xz + 1)/2
        close(sun_blend(Vec3::X, Vec3::Y), 0.5, 1e-6);
    }

    /// The probe's `EXPECT_FOG` table: Default (set 7) and Arctic (set 1) at intensity 3
    /// (variant weight 1), rows 2 and 3 at 00/06/12/18, blended by `bc`.
    #[test]
    fn fog_colours_match_the_probe_fixtures() {
        let Some(z) = retail_zone() else { return };
        let fixtures: &[(usize, f32, f32, [f32; 3])] = &[
            (7, 0.0, 0.5, [0.0278, 0.0718, 0.1275]),
            (7, 6.0, 0.0, [0.142, 0.1573, 0.1975]),
            (7, 6.0, 0.5, [0.3062, 0.1687, 0.1584]),
            (7, 6.0, 1.0, [0.4704, 0.1801, 0.1193]),
            (7, 12.0, 0.0, [0.0, 0.3204, 0.9406]),
            (7, 12.0, 1.0, [0.0, 0.3204, 0.9406]),
            (7, 18.0, 0.0, [0.187, 0.187, 0.1542]),
            (7, 18.0, 0.5, [0.323, 0.1393, 0.0856]),
            (7, 18.0, 1.0, [0.4591, 0.0915, 0.017]),
            (1, 0.0, 0.5, [0.0677, 0.0846, 0.1036]),
            (1, 6.0, 0.0, [0.1768, 0.1801, 0.1835]),
            (1, 6.0, 0.5, [0.2986, 0.2259, 0.2092]),
            (1, 6.0, 1.0, [0.4205, 0.2716, 0.2349]),
            (1, 12.0, 0.0, [0.3787, 0.4936, 0.5732]),
            (1, 12.0, 0.5, [0.4077, 0.5239, 0.6026]),
            (1, 12.0, 1.0, [0.4368, 0.5542, 0.632]),
            (1, 18.0, 0.0, [0.1801, 0.1801, 0.1768]),
            (1, 18.0, 0.5, [0.2719, 0.2036, 0.1768]),
            (1, 18.0, 1.0, [0.3636, 0.2271, 0.1768]),
        ];
        for &(set, hour, bc, want) in fixtures {
            let l = light_state(&z, set, 3.0, hour);
            let got = fog_colour(l.haze_toward_sun, l.haze_away, None, hour, bc, 0.0, 0.0);
            for c in 0..3 {
                assert!(
                    (got[c] - want[c]).abs() < 1.5e-4,
                    "set {set} h{hour} bc{bc}: {got:?} vs {want:?}"
                );
            }
        }
        // sets 2..7 share rows 2 and 3 (Desert .. Default); Arctic differs
        let default = light_state(&z, 7, 3.0, 12.0);
        for set in 2..8 {
            let l = light_state(&z, set, 3.0, 12.0);
            assert_eq!(l.haze_toward_sun, default.haze_toward_sun, "set {set}");
            assert_eq!(l.haze_away, default.haze_away, "set {set}");
        }
        assert_ne!(
            light_state(&z, 1, 3.0, 12.0).haze_toward_sun,
            default.haze_toward_sun
        );
    }

    /// Rain/snow variant blending of the colour: weight 0 is the clear colour, weight 1 the
    /// `lerp(B, A, bc)` of that variant's `gradients.bmp` tables.
    #[test]
    fn the_rain_and_snow_colour_blend_uses_the_ab_tables_of_variants_one_and_two() {
        let Some(fs) = game() else { return };
        if !fs.contains(jc2_formats::weather::GRADIENTS_FILE) {
            return;
        }
        let t = GradientTables::load(&fs).expect("gradients.bmp parses");
        let (toward, away) = (Vec3::new(0.4, 0.5, 0.6), Vec3::new(0.1, 0.2, 0.3));
        let clear = fog_colour(toward, away, Some(&t), 12.0, 0.5, 0.0, 0.0);
        assert!((clear - away.lerp(toward, 0.5)).length() < 1e-6);
        // noon: variant 1 A = B = (0.080, 0.082, 0.096); variant 2 (0.307, 0.410, 0.470)
        let rain = fog_colour(toward, away, Some(&t), 12.0, 0.5, 1.0, 0.0);
        assert!((rain - Vec3::new(0.080, 0.082, 0.096)).abs().max_element() < 2e-3);
        let snow = fog_colour(toward, away, Some(&t), 12.0, 0.5, 0.0, 1.0);
        assert!((snow - Vec3::new(0.307, 0.410, 0.470)).abs().max_element() < 2e-3);
        // no tables: the clear colour only (the variants cannot be evaluated)
        assert_eq!(fog_colour(toward, away, None, 12.0, 0.5, 1.0, 0.0), clear);
    }

    #[test]
    fn the_fallback_fog_has_the_colour_and_the_slope_of_the_exact_term() {
        let s = retail_settings();
        let h = HazeFog::evaluate(
            &s,
            Vec3::new(0.4, 0.5, 0.6),
            Vec3::new(0.1, 0.2, 0.3),
            None,
            12.0,
            202.0,
            Vec3::X,
            Vec3::new(1.0, 0.5, 0.0).normalize(),
            0.0,
            0.0,
        );
        assert_eq!(h.bc, 1.0);
        assert_eq!(h.colour, Vec3::new(0.4, 0.5, 0.6));
        close(
            h.fallback_density(),
            std::f32::consts::LN_2 / 11_800.0,
            1e-6,
        );
        let fog = h.distance_fog();
        let c = fog.color.to_linear();
        assert_eq!((c.red, c.green, c.blue), (0.4, 0.5, 0.6));
        assert_eq!(fog.directional_light_color.alpha(), 0.0);
        match fog.falloff {
            FogFalloff::Exponential { density } => close(density, h.fallback_density(), 1e-7),
            other => panic!("{other:?}"),
        }
        // beyond the density ramp, 11.8 km further halves the surviving fraction in both
        let (d0, d1) = (3_000.0, 3_000.0 + 11_800.0);
        let exact = h.transmittance(d1, 202.0) / h.transmittance(d0, 202.0);
        close(exact, 0.5, 1e-4);
        // the disabled default has no density
        assert_eq!(HazeFog::default().fallback_density(), 0.0);
        // a degenerate span turns the term off instead of dividing by zero
        let (c12, c13) = vs_constants([100.0, 100.0, 0.001, 0.0, 0.0], 0.0);
        assert!(fog_exponent(c12, c13, 500.0, 10.0).is_finite());
    }

    /// The system: no gradient files means no fog; with files it follows the hour, the camera
    /// height and heading, and writes the fallback `DistanceFog`.
    #[test]
    fn the_system_follows_the_clock_the_camera_and_the_light_state() {
        use crate::sky_light::SkyLighting;
        let mut app = App::new();
        app.insert_resource(TimeOfDay {
            hours: 12.0,
            speed: 0.0,
            paused: true,
        })
        .init_resource::<SkyLighting>()
        .init_resource::<AtmosphereState>()
        .init_resource::<GradientData>()
        .init_resource::<ZoneGradientData>()
        .init_resource::<HazeFog>()
        .add_systems(Update, update_haze_fog);
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Transform::from_xyz(0.0, 202.0, 0.0).looking_to(Vec3::X, Vec3::Y),
                GlobalTransform::from(
                    Transform::from_xyz(0.0, 202.0, 0.0).looking_to(Vec3::X, Vec3::Y),
                ),
                DistanceFog::default(),
            ))
            .id();
        app.update();
        assert_eq!(*app.world().resource::<HazeFog>(), HazeFog::default());
        // synthetic rows through the real light-state plumbing
        let mut zone = crate::sky_light::synthetic();
        zone.effects.haze = retail_settings();
        app.insert_resource(ZoneGradientData(Some(zone)));
        app.update();
        let h = *app.world().resource::<HazeFog>();
        close(h.p[1], 12_000.0, 1e-6);
        close(h.c12.w, 202.0, 1e-6);
        let fog = app.world().get::<DistanceFog>(cam).unwrap();
        match fog.falloff {
            FogFalloff::Exponential { density } => close(density, h.fallback_density(), 1e-7),
            _ => panic!("the fallback is exponential"),
        }
        // night: the night group
        app.world_mut().resource_mut::<TimeOfDay>().hours = 0.0;
        app.update();
        close(app.world().resource::<HazeFog>().p[1], 9_000.0, 1e-6);
    }
}
