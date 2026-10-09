//! The native sun/moon colour and sky/ground ambient (JC2-122; research
//! `research/formats/night-light-next.md`, JC2-115).
//!
//! The exe does not pick these colours in code: every frame the gradient system
//! (`FUN_00a66c30` -> `FUN_00a5d390` -> setter `FUN_0089f9d0`) fills the sky object from
//! `textures\landscape\gamezone_gradients.bmp` (a set per game zone type, 24 hours across,
//! two intensity variants), and `FUN_00a58a00` hands them on as `cb0[3]` (sun/moon colour),
//! `cb0[10]` (sky ambient, up-facing) and `cb0[11]` (ground ambient, down-facing). Night is
//! dark because those tables are (the moon is the same single light after the flip, with the
//! same row 0 colour).
//!
//! * [`light_state`] is the steady-state value for `(set, intensity, hour)`: the base case of
//!   `FUN_00a5d390` (`wv * variant 0 + (1 - wv) * variant 1`, `wv = intensity / 3`), the sun
//!   colour through the final transform `((x ** (1 / 2.2)) * 2.2) ** 2.2` (max 5.667) and the
//!   ambients raw. **Native.**
//! * [`SkyLighting`] (resource) holds the zone set and intensity and the smoothed state. The
//!   state moves toward the target by `clamp(rate * dt)` per frame with `rate` = 1.0 from
//!   `landscape_effects.bin` (about a one-second time constant) and snaps on the first frame
//!   (the loader's dt of 10). The smoothing is applied to the table blend before the sun
//!   transform (brief section 4 order); the native `dt` is `FUN_004d3690(0)`, not identified,
//!   so **the real frame time (`Time<Real>`) is an inferred stand-in**.
//! * **The zone set** comes from the game zone byte under the camera (`FUN_00a66c30` via
//!   `FUN_00a5eb50`; `gamezone-map-next.md`, JC2-123): the last section of each terrain tile,
//!   looked up with the engine's float32 cell arithmetic ([`zone_cell`]) in the tiles the
//!   world streamer holds ([`update_zone_inputs`]); `set = gradient_for_zone[type]`,
//!   `intensity = byte >> 6`. **Non-native:** a tile that is not loaded (and a position outside
//!   the world) reads the reset byte 0xE0 (Sea, intensity 3), where the engine reads an
//!   unrelated ring slot; and with no world streamer at all (the viewers) the fields keep the
//!   stand-in `Default` set 7 / intensity 3.
//!   Not implemented: the height (set 8), underwater (set 9), rain (10) and snow (11) branches
//!   of the gradient blend, so the camera's altitude does not change the light, and the cloud
//!   rows 2/3.
//! * Without the files every value is the exe initialiser's 1.0 (`FUN_00a58640`), the state
//!   before the first gradient update.
//!
//! **Units are non-native.** Native colours are HDR multipliers of the albedo (sun up to
//! 5.667); Bevy wants absolute illuminance. [`LUX_PER_UNIT`] is the single scale: the sun is
//! `colour / max * LUX_PER_UNIT * max` lux, and the ambient uses the same scale divided by
//! pi ([`AMBIENT_PER_UNIT`]) because Bevy's diffuse BRDF carries the `1 / pi` that the native
//! `C * (sun * N.L + ambient)` does not, which keeps the native sun : ambient ratio.

use crate::Game;
use crate::sky::TimeOfDay;
use crate::world::Streamer;
use bevy::prelude::*;
use jc2_formats::terrain::{GAMEZONE_RESET_BYTE, GameZoneCell, zone_cell};
use jc2_formats::weather::{
    GameZoneGradients, LandscapeEffects, ROW_GROUND_AMBIENT, ROW_SKY_AMBIENT, ROW_SUN,
    ZONE_TYPE_DEFAULT, sun_colour_transform,
};

/// **Policy, not native:** Bevy lux per unit of native sun colour. Chosen so the noon `Default`
/// sun (luminance 5.085 with the scene shader weights) gives 9,000 lux, the noon sun the
/// project used before the native tables (`sky.rs`, `SUN_LUX`): 9,000 / 5.085 = 1,770.
pub const LUX_PER_UNIT: f32 = 1_770.0;
/// **Policy, derived from [`LUX_PER_UNIT`]:** Bevy `AmbientLight` brightness per unit of native
/// ambient colour (`/ pi`, see the module comment).
pub const AMBIENT_PER_UNIT: f32 = LUX_PER_UNIT / std::f32::consts::PI;

/// The zone set used until the game zone map is found: gradient set 7, `Default`
/// (**non-native** stand-in).
pub const STAND_IN_ZONE_SET: usize = ZONE_TYPE_DEFAULT;
/// The zone intensity (bits 6..7 of the cell byte, 0..3) used until the map is found: the
/// maximum, so the variant weight is 1 (**non-native** stand-in).
pub const STAND_IN_ZONE_INTENSITY: f32 = 3.0;

/// What the exe's initialiser stores in the sky object before any gradient update.
const CONSTRUCTOR: LightState = LightState {
    sun: Vec3::ONE,
    sky_ambient: Vec3::ONE,
    ground_ambient: Vec3::ONE,
};

/// The tables and the file's constants, when the files could be read.
pub struct ZoneGradients {
    pub tables: GameZoneGradients,
    pub effects: LandscapeEffects,
}

#[derive(Resource, Default)]
pub struct ZoneGradientData(pub Option<ZoneGradients>);

/// The six rows the sky object receives, before the final sun transform (the smoothed state
/// of `Z+0x146fc` is of this kind).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rows {
    sun: Vec3,
    sky: Vec3,
    ground: Vec3,
}

impl Rows {
    fn lerp(self, to: Rows, f: f32) -> Rows {
        Rows {
            sun: self.sun.lerp(to.sun, f),
            sky: self.sky.lerp(to.sky, f),
            ground: self.ground.lerp(to.ground, f),
        }
    }
}

/// `cb0[3]`, `cb0[10]`, `cb0[11]` in native units (linear RGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightState {
    /// Sun colour by day, moon colour at night (the same light after the flip).
    pub sun: Vec3,
    /// `cb0[10]`: the ambient of an up-facing surface.
    pub sky_ambient: Vec3,
    /// `cb0[11]`: the ambient of a down-facing surface.
    pub ground_ambient: Vec3,
}

impl LightState {
    /// The hemisphere ambient of a surface with normal height `n_y`:
    /// `lerp(cb0[11], cb0[10], n_y / 2 + 1/2)` (native scene shaders).
    #[allow(dead_code)] // the CPU form of the shaders' tilt term (tests)
    pub fn hemisphere(&self, n_y: f32) -> Vec3 {
        self.ground_ambient
            .lerp(self.sky_ambient, (n_y * 0.5 + 0.5).clamp(0.0, 1.0))
    }

    /// The Bevy light values for this state (units: non-native, see the module comment).
    pub fn to_bevy(self) -> BevyLights {
        // colour / max with illuminance = max * scale keeps the HDR colour exactly
        let m = self.sun.max_element();
        let (sun_color, sun_illuminance) = if m > 0.0 {
            let c = self.sun / m;
            (Color::linear_rgb(c.x, c.y, c.z), m * LUX_PER_UNIT)
        } else {
            (Color::BLACK, 0.0)
        };
        // Bevy's AmbientLight is uniform: it carries the hemisphere at N.y = 0 (the mean of
        // the two ambients); the shaders add `tilt * N.y` for the rest (terrain, General).
        let mid = (self.sky_ambient + self.ground_ambient) * 0.5;
        let a = mid.max_element();
        let (ambient_color, ambient_brightness) = if a > 0.0 {
            let c = mid / a;
            (Color::linear_rgb(c.x, c.y, c.z), a * AMBIENT_PER_UNIT)
        } else {
            (Color::BLACK, 0.0)
        };
        BevyLights {
            sun_color,
            sun_illuminance,
            ambient_color,
            ambient_brightness,
            ambient_tilt: (self.sky_ambient - self.ground_ambient) * (0.5 * AMBIENT_PER_UNIT),
        }
    }
}

/// [`LightState`] in Bevy units.
#[derive(Clone, Copy, Debug)]
pub struct BevyLights {
    pub sun_color: Color,
    pub sun_illuminance: f32,
    pub ambient_color: Color,
    pub ambient_brightness: f32,
    /// `(sky - ground) / 2` in ambient-brightness units: the hemisphere ambient of a normal
    /// with height `n_y` is `ambient + ambient_tilt * n_y` (per channel, in Bevy units).
    pub ambient_tilt: Vec3,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from(a)
}

/// The six-row blend of `FUN_00a5d390`'s base case, rows 0, 4, 5.
fn target_rows(g: &ZoneGradients, set: usize, intensity: f32, hour: f32) -> Rows {
    let wv = g.effects.variant_weight(set, intensity);
    let r = g.tables.rows(set, wv, hour);
    Rows {
        sun: v3(r[ROW_SUN]),
        sky: v3(r[ROW_SKY_AMBIENT]),
        ground: v3(r[ROW_GROUND_AMBIENT]),
    }
}

fn state_of(g: &ZoneGradients, rows: Rows) -> LightState {
    LightState {
        sun: v3(sun_colour_transform(rows.sun.to_array(), g.effects.gain)),
        sky_ambient: rows.sky,
        ground_ambient: rows.ground,
    }
}

/// The steady-state sun/moon colour and ambients for gradient `set`, zone `intensity` (0..3)
/// and `hour` (0..24): the base case of the native gradient blend plus the final sun
/// transform, no smoothing. Native; the set and intensity inputs are stand-ins until the zone
/// map is found (module comment).
#[allow(dead_code)] // the steady-state function of the brief; the runtime goes through `SkyLighting::step`
pub fn light_state(g: &ZoneGradients, set: usize, intensity: f32, hour: f32) -> LightState {
    state_of(g, target_rows(g, set, intensity, hour))
}

/// The zone inputs and the smoothed light state (`Z+0x146fc`).
#[derive(Resource, Clone, Debug)]
pub struct SkyLighting {
    /// The gradient set under the camera. **Stand-in:** 7 (`Default`) until the game zone map
    /// is found (use `LandscapeEffects::set_for_zone_type` for a lookup).
    pub zone_set: usize,
    /// The zone intensity 0..3 (bits 6..7 of the cell byte). **Stand-in:** 3.
    pub zone_intensity: f32,
    smoothed: Option<Rows>,
    /// What the lights use.
    pub state: LightState,
}

impl Default for SkyLighting {
    fn default() -> Self {
        Self {
            zone_set: STAND_IN_ZONE_SET,
            zone_intensity: STAND_IN_ZONE_INTENSITY,
            smoothed: None,
            state: CONSTRUCTOR,
        }
    }
}

impl SkyLighting {
    /// Advance one frame: move the smoothed rows toward the target at `hour` by
    /// `clamp(rate * dt)` (snapping on the first frame) and refresh [`Self::state`]. Without
    /// tables the state is the exe initialiser's 1.0.
    pub fn step(&mut self, data: Option<&ZoneGradients>, hour: f32, dt: f32) {
        let Some(g) = data else {
            self.smoothed = None;
            self.state = CONSTRUCTOR;
            return;
        };
        let target = target_rows(g, self.zone_set, self.zone_intensity, hour);
        let now = match self.smoothed {
            // the loader's first update passes dt 10, so the factor clamps to 1
            None => target,
            Some(prev) => prev.lerp(target, (g.effects.smoothing_rate * dt).clamp(0.0, 1.0)),
        };
        self.smoothed = Some(now);
        self.state = state_of(g, now);
    }

    /// The Bevy light values for the current state.
    pub fn bevy(&self) -> BevyLights {
        self.state.to_bevy()
    }
}

/// The zone byte at `(x, z)` from the resident tiles (`gamezone` by tile key), else 0xE0
/// (non-native, see the module comment). `FUN_00a5eb50`'s lookup without its 8 x 8 ring.
pub fn zone_byte<'a>(x: f32, z: f32, tile: impl Fn((u32, u32)) -> Option<&'a [u8]>) -> u8 {
    let (tx, tz, cx, cz) = zone_cell(x, z);
    if !(0..64).contains(&tx) || !(0..64).contains(&tz) {
        return GAMEZONE_RESET_BYTE;
    }
    tile((tx as u32, tz as u32))
        .and_then(|b| b.get(cz * 64 + cx))
        .copied()
        .unwrap_or(GAMEZONE_RESET_BYTE)
}

/// The gradient set and intensity of a zone byte: `set = gradient_for_zone[type]`,
/// `intensity = byte >> 6`.
pub fn zone_inputs(byte: u8, effects: Option<&LandscapeEffects>) -> (usize, f32) {
    let cell = GameZoneCell(byte);
    let ty = cell.zone_type() as usize;
    // without the file the identity map of the retail file stands (type N -> set N)
    let set = effects.map_or(ty, |e| e.set_for_zone_type(ty));
    (set, cell.intensity() as f32)
}

/// Feed the zone under the camera into [`SkyLighting`] (milestone 1 of the zone map: no height,
/// underwater, rain or snow inputs). Does nothing without a world streamer.
pub fn update_zone_inputs(
    streamer: Option<Res<Streamer>>,
    data: Res<ZoneGradientData>,
    cam: Query<&GlobalTransform, With<Camera3d>>,
    mut lighting: ResMut<SkyLighting>,
) {
    let (Some(st), Some(cam)) = (streamer, cam.iter().next()) else {
        return;
    };
    let p = cam.translation();
    let byte = zone_byte(p.x, p.z, |k| {
        st.loaded.get(&k).map(|t| t.gamezone.as_slice())
    });
    let (set, intensity) = zone_inputs(byte, data.0.as_ref().map(|g| &g.effects));
    if lighting.zone_set != set || lighting.zone_intensity != intensity {
        lighting.zone_set = set;
        lighting.zone_intensity = intensity;
    }
}

pub fn load_zone_gradients(game: Option<Res<Game>>, mut data: ResMut<ZoneGradientData>) {
    let Some(game) = game else { return };
    let loaded = GameZoneGradients::load(&game.fs)
        .and_then(|tables| LandscapeEffects::load(&game.fs).map(|effects| (tables, effects)));
    match loaded {
        Ok((tables, effects)) => data.0 = Some(ZoneGradients { tables, effects }),
        Err(e) => warn!("sky light: gamezone gradients not read ({e}); lights are the exe's 1.0"),
    }
}

/// Every frame, from the clock (the exe's gradient update runs every frame too).
pub fn update_sky_lighting(
    real: Res<Time<Real>>,
    time: Res<TimeOfDay>,
    data: Res<ZoneGradientData>,
    mut lighting: ResMut<SkyLighting>,
) {
    lighting.step(data.0.as_ref(), time.hours, real.delta_secs());
}

/// A synthetic set where row 0 / 4 / 5 are bright by day (hours 6..18) and dim at night.
#[cfg(test)]
pub(crate) fn synthetic() -> ZoneGradients {
    let day = |h: usize| (6..18).contains(&h);
    ZoneGradients {
        tables: GameZoneGradients::from_fn(|_, row, _, h| match row {
            ROW_SUN => {
                if day(h) {
                    [0.9, 0.8, 0.6]
                } else {
                    [0.1, 0.12, 0.2]
                }
            }
            ROW_SKY_AMBIENT => {
                if day(h) {
                    [0.01, 0.02, 0.05]
                } else {
                    [0.05, 0.1, 0.2]
                }
            }
            ROW_GROUND_AMBIENT => {
                if day(h) {
                    [0.8, 0.7, 0.5]
                } else {
                    [0.0, 0.0, 0.03]
                }
            }
            _ => [0.0; 3],
        }),
        effects: LandscapeEffects {
            image: String::new(),
            zone_set: [0, 1, 2, 3, 4, 5, 6, 7],
            gain: 2.2,
            smoothing_rate: 1.0,
            max_gradient: [3.0; 10],
            set_altitude: 8,
            set_underwater: 9,
            set_rain: 10,
            set_snow: 11,
            altitude: [30.0, 300.0, 1000.0],
            underwater_depth: 120.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;

    fn retail() -> Option<ZoneGradients> {
        let fs = GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()?;
        (fs.contains(jc2_formats::weather::GAMEZONE_GRADIENTS_FILE)
            && fs.contains(jc2_formats::weather::LANDSCAPE_EFFECTS_FILE))
        .then(|| ZoneGradients {
            tables: GameZoneGradients::load(&fs).expect("gamezone_gradients.bmp parses"),
            effects: LandscapeEffects::load(&fs).expect("landscape_effects.bin parses"),
        })
    }

    fn near(a: Vec3, b: [f32; 3], eps: f32) {
        assert!(
            (a - Vec3::from(b)).abs().max_element() < eps,
            "{a:?} vs {b:?}"
        );
    }

    #[test]
    fn default_inputs_are_the_documented_stand_ins() {
        let l = SkyLighting::default();
        assert_eq!((l.zone_set, l.zone_intensity), (7, 3.0));
        assert_eq!(l.state, CONSTRUCTOR);
    }

    #[test]
    fn without_tables_the_state_is_the_exe_initialiser() {
        let mut l = SkyLighting::default();
        l.step(None, 3.0, 0.016);
        assert_eq!(l.state, CONSTRUCTOR);
        let b = l.bevy();
        assert_eq!(b.sun_illuminance, LUX_PER_UNIT);
        assert_eq!(b.ambient_tilt, Vec3::ZERO);
    }

    /// The probe's Default rows through `light_state` (set 7, intensity 3).
    #[test]
    fn default_zone_noon_and_midnight_match_the_probe() {
        let Some(g) = retail() else { return };
        let noon = light_state(&g, 7, 3.0, 12.0);
        near(noon.sun, [5.667, 5.051, 3.697], 6e-4);
        near(noon.sky_ambient, [0.000, 0.017, 0.041], 6e-4);
        near(noon.ground_ambient, [0.730, 0.554, 0.263], 6e-4);
        let night = light_state(&g, 7, 3.0, 0.0);
        near(night.sun, [0.330, 0.723, 1.223], 6e-4);
        near(night.sky_ambient, [0.008, 0.145, 0.316], 6e-4);
        near(night.ground_ambient, [0.006, 0.006, 0.032], 6e-4);
        // Arctic
        let arctic = light_state(&g, 1, 3.0, 12.0);
        near(arctic.sun, [1.922, 1.763, 1.376], 6e-4);
        near(
            light_state(&g, 1, 3.0, 0.0).sun,
            [0.532, 0.616, 0.707],
            6e-4,
        );
        // the sun transform tops out at 2.2 ** 2.2 = 5.667 in the brightest channel
        let peak = (0..24)
            .map(|h| light_state(&g, 7, 3.0, h as f32).sun.max_element())
            .fold(0.0, f32::max);
        assert!((peak - 5.667).abs() < 2e-3, "{peak}");
        assert!(peak <= 2.2_f32.powf(2.2) + 1e-4);
    }

    #[test]
    fn light_and_ambient_follow_the_hour() {
        for g in [Some(synthetic()), retail()].into_iter().flatten() {
            let noon = light_state(&g, 7, 3.0, 12.0);
            let night = light_state(&g, 7, 3.0, 0.0);
            let lum = |c: Vec3| c.dot(Vec3::new(0.2125, 0.7154, 0.0721));
            assert!(lum(night.sun) < lum(noon.sun) * 0.5, "night light dimmer");
            // the ground ambient collapses at night (< 1 % of noon in the retail table)
            assert!(lum(night.ground_ambient) < lum(noon.ground_ambient) * 0.1);
            // the up-facing sky glow does not
            assert!(lum(night.sky_ambient) > lum(noon.sky_ambient));
            // the hour interpolates between the table entries
            // (the sun transform is non-linear, so compare the raw rows)
            let ra = target_rows(&g, 7, 3.0, 5.0);
            let rb = target_rows(&g, 7, 3.0, 6.0);
            let rm = target_rows(&g, 7, 3.0, 5.5);
            assert!((rm.sun - (ra.sun + rb.sun) * 0.5).abs().max_element() < 1e-5);
            assert!(
                (rm.ground - (ra.ground + rb.ground) * 0.5)
                    .abs()
                    .max_element()
                    < 1e-5
            );
        }
    }

    #[test]
    fn intensity_blends_the_variants_by_a_third() {
        // sets 2, 3 differ per variant in the retail picture; synthetic: make variants differ
        let mut g = synthetic();
        g.tables = GameZoneGradients::from_fn(|_, row, v, _| {
            if row == ROW_SUN {
                [if v == 0 { 0.9 } else { 0.3 }; 3]
            } else {
                [0.0; 3]
            }
        });
        let lo = target_rows(&g, 7, 0.0, 12.0).sun;
        let mid = target_rows(&g, 7, 1.5, 12.0).sun;
        let hi = target_rows(&g, 7, 3.0, 12.0).sun;
        assert!((lo.x - 0.3).abs() < 1e-6 && (hi.x - 0.9).abs() < 1e-6);
        assert!((mid.x - 0.6).abs() < 1e-6, "{mid:?}");
    }

    #[test]
    fn the_first_frame_snaps_and_later_frames_smooth_at_the_files_rate() {
        let g = synthetic();
        let mut l = SkyLighting::default();
        l.step(Some(&g), 12.0, 0.016);
        let noon = light_state(&g, 7, 3.0, 12.0);
        assert_eq!(l.state, noon, "first update snaps (loader dt 10)");
        // the hour jumps to midnight: half a second at rate 1.0 covers half the distance
        l.step(Some(&g), 0.0, 0.5);
        let night_rows = target_rows(&g, 7, 3.0, 0.0);
        let noon_rows = target_rows(&g, 7, 3.0, 12.0);
        let half = noon_rows.lerp(night_rows, 0.5);
        assert!(
            (l.smoothed.unwrap().ground - half.ground)
                .abs()
                .max_element()
                < 1e-6
        );
        assert_eq!(l.state, state_of(&g, half));
        // a frame of 2 s clamps the factor to 1: it reaches the target
        l.step(Some(&g), 0.0, 2.0);
        assert_eq!(l.state, light_state(&g, 7, 3.0, 0.0));
        // a steady hour stays put
        let s = l.state;
        l.step(Some(&g), 0.0, 0.016);
        assert_eq!(l.state, s);
        // losing the tables returns to the initialiser values and the next load snaps again
        l.step(None, 0.0, 0.016);
        assert_eq!(l.state, CONSTRUCTOR);
        l.step(Some(&g), 12.0, 0.016);
        assert_eq!(l.state, noon);
    }

    #[test]
    fn the_zone_byte_comes_from_the_resident_tile_or_the_reset_byte() {
        use std::collections::HashMap;
        // tile (29, 18) holds 0xC8 everywhere except cell (x 32, z 57)
        let mut tile = vec![0xC8u8; 4096];
        tile[57 * 64 + 32] = 0xD0;
        let tiles: HashMap<(u32, u32), Vec<u8>> = HashMap::from([((29, 18), tile)]);
        let get = |k: (u32, u32)| tiles.get(&k).map(|v| v.as_slice());
        assert_eq!(zone_byte(-1280.0, -6712.0, get), 0xD0);
        assert_eq!(zone_byte(-1280.0 + 8.0, -6712.0, get), 0xC8);
        // not loaded and outside the world: 0xE0 (non-native)
        assert_eq!(zone_byte(0.0, 0.0, get), 0xE0);
        assert_eq!(zone_byte(20_000.0, 0.0, get), 0xE0);
        assert_eq!(zone_byte(-20_000.0, 0.0, get), 0xE0);
    }

    #[test]
    fn a_zone_byte_gives_the_light_set_and_intensity() {
        // the start base is Arctic at full intensity (probe fixture 0xC8)
        assert_eq!(zone_inputs(0xC8, None), (1, 3.0));
        assert_eq!(zone_inputs(0xE0, None), (4, 3.0), "the reset byte is Sea");
        assert_eq!(zone_inputs(0xF8, None), (7, 3.0), "Default");
        assert_eq!(zone_inputs(0x3A, None), (7, 0.0), "airport: intensity 0");
        let g = synthetic();
        assert_eq!(zone_inputs(0xD0, Some(&g.effects)), (2, 3.0));
    }

    /// The start base's night differs from Default's: the zone set matters.
    #[test]
    fn the_arctic_start_base_is_lit_by_set_one() {
        let Some(g) = retail() else { return };
        let (set, intensity) = zone_inputs(0xC8, Some(&g.effects));
        let arctic = light_state(&g, set, intensity, 12.0);
        near(arctic.sun, [1.922, 1.763, 1.376], 6e-4);
        let default = light_state(&g, 7, 3.0, 12.0);
        assert!(default.sun.x > arctic.sun.x * 2.0);
    }

    #[test]
    fn the_zone_set_is_a_field() {
        let g = synthetic();
        let mut l = SkyLighting {
            zone_set: 1,
            ..Default::default()
        };
        l.step(Some(&g), 12.0, 1.0);
        assert_eq!(l.state, light_state(&g, 1, 3.0, 12.0));
        assert_eq!(g.effects.set_for_zone_type(1), 1);
    }

    #[test]
    fn bevy_units_keep_the_hdr_colour_and_the_native_hemisphere() {
        let s = LightState {
            sun: Vec3::new(5.667, 5.051, 3.697),
            sky_ambient: Vec3::new(0.0, 0.017, 0.041),
            ground_ambient: Vec3::new(0.73, 0.554, 0.263),
        };
        let b = s.to_bevy();
        let c = b.sun_color.to_linear();
        let back = Vec3::new(c.red, c.green, c.blue) * (b.sun_illuminance / LUX_PER_UNIT);
        assert!((back - s.sun).abs().max_element() < 1e-4, "{back:?}");
        assert!((b.sun_illuminance - 5.667 * LUX_PER_UNIT).abs() < 1e-2);
        // uniform ambient + tilt * N.y reproduces lerp(cb0[11], cb0[10], N.y / 2 + 1/2)
        let a = b.ambient_color.to_linear();
        let uniform = Vec3::new(a.red, a.green, a.blue) * b.ambient_brightness;
        for n_y in [-1.0_f32, -0.3, 0.0, 0.6, 1.0] {
            let got = uniform + b.ambient_tilt * n_y;
            let want = s.hemisphere(n_y) * AMBIENT_PER_UNIT;
            assert!(
                (got - want).abs().max_element() < 1e-2,
                "{n_y}: {got:?} vs {want:?}"
            );
        }
        // black stays black
        let z = LightState {
            sun: Vec3::ZERO,
            sky_ambient: Vec3::ZERO,
            ground_ambient: Vec3::ZERO,
        }
        .to_bevy();
        assert_eq!((z.sun_illuminance, z.ambient_brightness), (0.0, 0.0));
    }

    /// Noon `Default` must stay comparable to the project's former noon sun (9,000 lux).
    #[test]
    fn the_scale_puts_noon_default_near_the_former_sun() {
        let Some(g) = retail() else { return };
        let b = light_state(&g, 7, 3.0, 12.0).to_bevy();
        let c = b.sun_color.to_linear();
        let lux = Vec3::new(c.red, c.green, c.blue) * b.sun_illuminance;
        let lum = lux.dot(Vec3::new(0.2125, 0.7154, 0.0721));
        assert!((lum - 9_000.0).abs() < 100.0, "{lum}");
    }
}
