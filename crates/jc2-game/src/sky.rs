//! Day and night: a clock, a sun and a moon that follow it, and the ambient light, fog and
//! sky colour that go with them (`research/formats/weather.md`).
//!
//! * [`TimeOfDay`] is the hour (0..24) and the rate it runs at: 1 game minute per real
//!   second by default (`--day-speed` scales that, `--time 18:30` starts elsewhere).
//!   Keys: `[` and `]` step half an hour back and forward, `T` stops and starts the clock.
//! * The sun's direction is the game's own orbit ([`jc2_formats::weather::sun_direction`],
//!   engine axes, which are Bevy's — x east, z south, `coords.rs`): rises in the east,
//!   stands 45° high in the south at noon, 32° below the horizon at
//!   midnight, up from about 5:40 to 18:20. The moon is the same orbit twelve hours on.
//! * **The sky is native since JC2-214** (`native_sky.rs`): the gradient dome, the fog band, the
//!   stars and the sun disc and glow draw as the exe's passes do. There is no sky colour policy
//!   any more: the scene is cleared to the native linear (0.498, 0.498, 1.0)
//!   ([`SCENE_CLEAR`], JC2-203) and the dome adds its colour. The sun and ambient light are native tables.
//! * **Fog is native since JC2-155** (`haze.rs`): colour, distances, density and height term
//!   come from the haze section of `landscape_effects.bin` and the zone gradient rows 2/3;
//!   [`crate::haze::update_haze_fog`] runs in this plugin's chain.
//! * An unlit disc marks the moon far away until `clouds.rs` replaces it with the textured sprite
//!   (the moon pass is JC2-130's); the sun disc and glow are `native_sky.rs`'s.
//! * **One** directional light (the [`Sun`] entity), as in the exe (`FUN_00a58a00`): when the
//!   sun is below the horizon its direction is negated, so the same light now comes from the
//!   moon's side (`light-tables-next.md` section 5). The flip has no colour attenuation
//!   term, only the 0.15 shadow floor (`sun_horizon.rs`). Its **colour is native**: the sun/moon
//!   colour, and the sky/ground ambient, come from `gamezone_gradients.bmp` by hour and zone
//!   (`sky_light.rs`, JC2-122; the zone set is a labelled stand-in there). Only the scale from
//!   native colour units to Bevy lux (`sky_light::lux_per_unit`) is the exposure identity `pi / e0`. There is no
//!   separate Moon or sky-fill light (the exe has none).
//! * Still **non-native** policy here: the moon disc, and the sun colour /
//!   moon strength the cloud tint reads ([`SkyState::sun_colour`], [`SkyState::moon_lux`];
//!   `clouds.rs`).
//! * **Units (JC2-139).** The world camera shows `sqrt(clamp01(scene) * A * tint)`; the sky
//!   passes write native scene units (1.0 = display white). The fog colour (JC2-155) is a native scene
//!   colour, rows 2/3 of the zone gradient, and is used as is. Tables `E`/`F` of
//!   `gradients.bmp` are the water colour (`cb0[14]`), not the haze, and stay unused here.

use crate::Game;
use crate::atmosphere::{
    AtmosphereState, GradientData, load_gradients, update_atmosphere,
};
use crate::coords::to_bevy;
use crate::sky_light::{
    SkyLighting, ZoneGradientData, load_zone_gradients, update_sky_lighting, update_zone_inputs,
};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder};
use bevy::prelude::*;
use jc2_formats::weather::{Atmosphere, elevation_degrees, moon_direction, sun_direction};

/// **Non-native, cloud tint only (not the light):** the sun's colour when
/// high / when on the horizon.
const SUN_HIGH: Vec3 = Vec3::new(1.0, 0.97, 0.92);
const SUN_LOW: Vec3 = Vec3::new(1.0, 0.52, 0.25);
/// **Non-native, cloud tint only:** the moon's relative strength ([`SkyState::moon_lux`]).
const MOON_LUX: f32 = 2400.0;
/// Distance of the moon disc from the camera (inside the 30 km far plane).
const DISC_DISTANCE: f32 = 25_000.0;

#[derive(Resource, Clone, Debug)]
pub struct SkyConfig {
    /// Hour of the day to start at.
    pub start_hour: f32,
    /// Game minutes per real second.
    pub speed: f32,
    /// Cascaded shadow maps from the sun.
    pub sun_shadows: bool,
}

impl Default for SkyConfig {
    fn default() -> Self {
        Self {
            start_hour: 10.0,
            speed: 1.0,
            sun_shadows: false,
        }
    }
}

/// The clock.
#[derive(Resource, Clone, Debug)]
pub struct TimeOfDay {
    /// 0..24.
    pub hours: f32,
    /// Game minutes per real second.
    pub speed: f32,
    pub paused: bool,
}

impl TimeOfDay {
    pub fn advance(&mut self, real_seconds: f32) {
        if !self.paused {
            self.hours = (self.hours + real_seconds * self.speed / 60.0).rem_euclid(24.0);
        }
    }

    pub fn scrub(&mut self, hours: f32) {
        self.hours = (self.hours + hours).rem_euclid(24.0);
    }

    /// `HH:MM`.
    pub fn clock(&self) -> String {
        let total = (self.hours * 60.0).floor() as u32 % (24 * 60);
        format!("{:02}:{:02}", total / 60, total % 60)
    }
}

/// The game's sky table, when the files could be read.
#[derive(Resource, Default)]
pub struct SkyData(pub Option<Atmosphere>);

#[derive(Component)]
pub struct Sun;
#[derive(Component)]
pub(crate) struct MoonDisc;
#[derive(Component)]
struct ClockText;

/// Everything the lights and the sky look like at one hour.
#[derive(Clone, Debug)]
pub struct SkyState {
    /// Unit vectors towards the sun and moon, Bevy axes.
    pub sun_dir: Vec3,
    pub moon_dir: Vec3,
    pub sun_elevation: f32,
    /// **Non-native** project colour of the low or high sun: read by the cloud tint, not by the
    /// light (`sky_light.rs` sets that).
    pub sun_colour: Vec3,
    /// The single directional light: towards the sun by day, the negated vector (the moon's
    /// side) once the sun is below the horizon (native flip). Its colour and illuminance are
    /// `sky_light.rs`'s.
    pub light_dir: Vec3,
    /// **Non-native** moon strength (0..[`MOON_LUX`]) for the cloud tint only.
    pub moon_lux: f32,
    /// 0 at night, 1 by day (**non-native** smoothstep of the sun's elevation, used by the moon
    /// disc and the cloud tint only).
    pub daylight: f32,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl SkyState {
    pub fn at(hour: f32) -> Self {
        let sun = sun_direction(hour);
        let moon = moon_direction(hour);
        let e = elevation_degrees(sun);
        let moon_e = elevation_degrees(moon);
        let sky_w = smoothstep(-14.0, 8.0, e);

        let sun_dir = to_bevy(sun);
        // native: L = -sun; if L.y > 0 (sun below the horizon) L = -L
        let light_dir = if sun_dir.y < 0.0 { -sun_dir } else { sun_dir };
        let sun_colour = SUN_LOW.lerp(SUN_HIGH, smoothstep(0.0, 22.0, e));

        Self {
            sun_dir,
            moon_dir: to_bevy(moon),
            sun_elevation: e,
            sun_colour,
            light_dir,
            moon_lux: MOON_LUX * (1.0 - smoothstep(-6.0, 6.0, e)) * smoothstep(-2.0, 12.0, moon_e),
            daylight: sky_w,
        }
    }
}

pub struct SkyPlugin {
    pub config: SkyConfig,
}

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .insert_resource(TimeOfDay {
                hours: self.config.start_hour.rem_euclid(24.0),
                speed: self.config.speed,
                paused: false,
            })
            .init_resource::<SkyData>()
            .init_resource::<GradientData>()
            .init_resource::<AtmosphereState>()
            .init_resource::<ZoneGradientData>()
            .init_resource::<SkyLighting>()
            .init_resource::<crate::haze::HazeFog>()
            .init_resource::<ClearColor>()
            .add_systems(
                Startup,
                (
                    load_sky_data,
                    load_gradients,
                    load_zone_gradients,
                    spawn_sky,
                )
                    .chain(),
            )
            .init_resource::<crate::sun_shadow::FrameStats>()
            .add_systems(
                Update,
                (
                    clock_keys,
                    tick_clock,
                    update_atmosphere,
                    update_zone_inputs,
                    update_sky_lighting,
                    crate::haze::update_haze_fog,
                    apply_sky,
                    follow_camera,
                    clock_text,
                )
                    .chain(),
            );
        if self.config.sun_shadows && !crate::sun_shadow::disabled_by_env() {
            crate::sun_shadow::insert_shadow_map(app);
            app.add_systems(Update, crate::sun_shadow::update_cascades.after(follow_camera));
        }
        if std::env::var_os("JC2_FRAME_STATS").is_some() {
            app.add_systems(Update, crate::sun_shadow::frame_stats);
        }
    }
}

fn load_sky_data(game: Option<Res<Game>>, mut data: ResMut<SkyData>) {
    let Some(game) = game else { return };
    match Atmosphere::load(&game.fs) {
        Ok(a) => data.0 = Some(a),
        Err(e) => warn!("sky: weather files not read ({e}); using built-in colours"),
    }
}

fn spawn_sky(
    mut commands: Commands,
    cfg: Res<SkyConfig>,
    time: Res<TimeOfDay>,
    lighting: Res<SkyLighting>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let st = SkyState::at(time.hours);
    let lights = lighting.bevy();
    let mut sun = commands.spawn((
        Sun,
        Name::new("sun"),
        DirectionalLight {
            illuminance: lights.sun_illuminance,
            color: lights.sun_color,
            shadow_maps_enabled: cfg.sun_shadows,
            ..default()
        },
        Transform::default().looking_to(-st.light_dir, Vec3::Y),
    ));
    if cfg.sun_shadows && crate::sun_shadow::disabled_by_env() {
        // before-capture / comparison: Bevy's own non-native split
        sun.insert(
            CascadeShadowConfigBuilder {
                num_cascades: 4,
                minimum_distance: 0.5,
                maximum_distance: 800.0,
                first_cascade_far_bound: 25.0,
                ..default()
            }
            .build(),
        );
    } else if cfg.sun_shadows {
        // native cascade set-up (`sun_shadow.rs`); the bounds are refreshed every frame
        let c = crate::sun_shadow::native_cascades(0.0, 10.0);
        sun.insert(CascadeShadowConfig {
            bounds: c.view_bounds().to_vec(),
            overlap_proportion: 1.0 - crate::sun_shadow::EDGE_FADE_START,
            minimum_distance: 0.1,
        });
    }
    commands.spawn((
        ClockText,
        crate::hud::DevText,
        Text::new(time.clock()),
        TextFont::from_font_size(14.0),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(10.0),
            top: Val::Px(8.0),
            ..default()
        },
    ));

    // the discs (only with a renderer)
    if let (Some(mut meshes), Some(mut materials)) = (meshes, materials) {
        let mut disc = |radius: f32, base: Color, alpha: bool| {
            (
                Mesh3d(meshes.add(Sphere::new(radius).mesh().ico(2).unwrap())),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: base,
                    unlit: true,
                    fog_enabled: false,
                    alpha_mode: if alpha {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    },
                    ..default()
                })),
                bevy::light::NotShadowCaster,
            )
        };
        commands.spawn((
            MoonDisc,
            Name::new("moon disc"),
            disc(
                380.0,
                crate::native_units::native_colour(Vec3::new(0.86, 0.89, 0.96), 1.0),
                true,
            ),
            Transform::from_translation(st.moon_dir * DISC_DISTANCE),
        ));
    }
}

fn clock_keys(keys: Res<ButtonInput<KeyCode>>, mut time: ResMut<TimeOfDay>) {
    if keys.just_pressed(KeyCode::BracketLeft) {
        time.scrub(-0.5);
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        time.scrub(0.5);
    }
    if keys.just_pressed(KeyCode::KeyT) {
        time.paused = !time.paused;
    }
}

fn tick_clock(real: Res<Time<Real>>, mut time: ResMut<TimeOfDay>) {
    // real seconds, so a paused or slowed virtual clock does not stop the sky
    time.advance(real.delta_secs());
}

/// The scene clear colour, linear: bytes (127, 127, 255, 255) / 255 (`FUN_00928740` truncates
/// `x * 255`; `FUN_0096a2d0` multiplies by 0.003921569).
pub const SCENE_CLEAR: Color = Color::linear_rgba(127.0 / 255.0, 127.0 / 255.0, 1.0, 1.0);

fn apply_sky(
    time: Res<TimeOfDay>,
    lighting: Res<SkyLighting>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut cams: Query<&mut AmbientLight, With<Camera3d>>,
) {
    let st = SkyState::at(time.hours);
    // native colour of the one light and the ambient (sky_light.rs); units are the exposure identity
    let lights = lighting.bevy();
    for (mut l, mut tf) in &mut sun {
        l.illuminance = lights.sun_illuminance;
        l.color = lights.sun_color;
        *tf = Transform::default().looking_to(-st.light_dir, Vec3::Y);
    }
    // NATIVE (JC2-203, `sky-gradient-next.md` section 12.4, `FUN_00a52f30`): the CLEAR pass
    // clears the scene target to the float4 (0.5, 0.5, 1.0, 1.0), stored as bytes
    // (127, 127, 255, 255) and given as a linear value to the sRGB view. The additive dome then
    // sums onto it (section 12.5).
    clear.0 = SCENE_CLEAR;
    // the fog is `haze.rs`'s (native colour and numbers, JC2-155)
    for mut amb in &mut cams {
        amb.color = lights.ambient_color;
        amb.brightness = lights.ambient_brightness;
    }
}

/// The moon disc rides with the camera at a fixed distance in the moon's direction.
#[allow(clippy::type_complexity)]
fn follow_camera(
    time: Res<TimeOfDay>,
    cam: Query<&GlobalTransform, With<Camera3d>>,
    mut moon: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        (With<MoonDisc>, Without<Camera3d>),
    >,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let Some(cam) = cam.iter().next() else { return };
    let st = SkyState::at(time.hours);
    let origin = cam.translation();
    for (mut tf, mut vis, mat) in &mut moon {
        tf.translation = origin + st.moon_dir * DISC_DISTANCE;
        *vis = if st.moon_dir.y > -0.02 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(mut m) = materials.as_mut().and_then(|a| a.get_mut(&mat.0)) {
            // faint in daylight, full at night
            m.base_color = crate::native_units::native_colour(
                Vec3::new(0.86, 0.89, 0.96),
                0.12 + 0.88 * (1.0 - st.daylight),
            );
        }
    }
}

fn clock_text(time: Res<TimeOfDay>, mut text: Query<&mut Text, With<ClockText>>) {
    if !time.is_changed() {
        return;
    }
    for mut t in &mut text {
        t.0 = format!(
            "{}{}   [ ] 30 min, T {}",
            time.clock(),
            if time.paused { " (paused)" } else { "" },
            if time.paused { "resumes" } else { "pauses" },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::pbr::DistanceFog;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// The sky plugin on a bare app with one camera, stepped in 0.1 s frames. The zone
    /// gradients are a synthetic table (bright by day, dim at night); the retail tables are
    /// checked in `sky_light.rs`.
    fn app(start: f32, speed: f32) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )))
            .add_plugins(SkyPlugin {
                config: SkyConfig {
                    start_hour: start,
                    speed,
                    sun_shadows: false,
                },
            });
        app.insert_resource(ZoneGradientData(Some(crate::sky_light::synthetic())));
        app.world_mut().spawn((
            Camera3d::default(),
            AmbientLight::default(),
            DistanceFog::default(),
        ));
        app.update();
        app
    }

    struct Look {
        /// Height of the single light's direction (towards the light).
        sun_up: f32,
        sun_lux: f32,
        ambient: f32,
        clear: Color,
    }

    fn look(app: &mut App) -> Look {
        let w = app.world_mut();
        let (sun_tf, sun_l) = w
            .query_filtered::<(&Transform, &DirectionalLight), With<Sun>>()
            .single(w)
            .unwrap();
        let sun_up = -sun_tf.forward().y;
        let sun_lux = sun_l.illuminance;
        let ambient = w.query::<&AmbientLight>().single(w).unwrap().brightness;
        Look {
            sun_up,
            sun_lux,
            ambient,
            clear: w.resource::<ClearColor>().0,
        }
    }

    /// Set the clock and run frames until the 1 s light smoothing has settled (150 x 0.1 s).
    fn at(app: &mut App, hour: f32) -> Look {
        app.world_mut().resource_mut::<TimeOfDay>().hours = hour;
        app.world_mut().resource_mut::<TimeOfDay>().paused = true;
        for _ in 0..150 {
            app.update();
        }
        look(app)
    }

    #[test]
    fn one_light_is_the_sun_by_day_and_the_moon_side_at_night() {
        let mut a = app(10.0, 1.0);
        let noon = at(&mut a, 12.0);
        assert!(noon.sun_up > 0.6, "sun height {}", noon.sun_up);
        assert!(noon.sun_lux > 8_000.0, "{}", noon.sun_lux);
        // the single sun/moon light: no separate Moon light and no sky fill (the exe has none)
        let w = a.world_mut();
        assert_eq!(w.query::<&DirectionalLight>().iter(w).count(), 1);
        // midnight: the sun is 32 degrees below the horizon, so the light is the negated
        // sun vector (above the horizon on the moon's side), dim but not black
        let midnight = at(&mut a, 0.0);
        let sun_below = SkyState::at(0.0).sun_dir;
        assert!(sun_below.y < -0.4);
        assert!(midnight.sun_up > 0.4, "light height {}", midnight.sun_up);
        assert!(
            (midnight.sun_up + sun_below.y).abs() < 1e-3,
            "negated sun vector"
        );
        assert!(midnight.sun_lux > 100.0, "{}", midnight.sun_lux);
        // night is darker: the native table colours (the synthetic fixture here)
        assert!(midnight.sun_lux < noon.sun_lux / 3.0);
        assert!(midnight.ambient < noon.ambient / 2.0);
        // the sky is the native dome's job (`native_sky.rs`): the scene is cleared to the native
        // linear (127, 127, 255, 255) / 255 (JC2-203) at every hour
        assert_eq!(noon.clear, SCENE_CLEAR);
        assert_eq!(midnight.clear, SCENE_CLEAR);
    }

    /// The light's colour and the camera ambient are the native table values at the hour,
    /// through the exposure identity (`sky_light.rs`).
    #[test]
    fn the_lights_take_their_colour_from_the_zone_tables() {
        use crate::sky_light::{light_state, synthetic};
        let g = synthetic();
        let mut a = app(10.0, 1.0);
        for hour in [12.0, 0.0] {
            let seen = at(&mut a, hour);
            let want = light_state(&g, 7, 3.0, hour).to_bevy();
            assert!(
                (seen.sun_lux - want.sun_illuminance).abs() < 0.01 * want.sun_illuminance,
                "{hour}: {} vs {}",
                seen.sun_lux,
                want.sun_illuminance
            );
            assert!(
                (seen.ambient - want.ambient_brightness).abs() < 0.01 * want.ambient_brightness
            );
            let w = a.world_mut();
            let l = w
                .query_filtered::<&DirectionalLight, With<Sun>>()
                .single(w)
                .unwrap();
            let (got, exp) = (l.color.to_linear(), want.sun_color.to_linear());
            assert!(
                (got.red - exp.red).abs()
                    + (got.green - exp.green).abs()
                    + (got.blue - exp.blue).abs()
                    < 5e-3
            );
        }
    }

    /// The flip happens where the sun crosses the horizon and only negates the vector.
    #[test]
    fn the_light_flips_when_the_sun_height_goes_negative() {
        for hour in [0.0, 3.0, 5.0, 19.0, 22.0] {
            let st = SkyState::at(hour);
            assert!(st.sun_dir.y < 0.0);
            assert!((st.light_dir + st.sun_dir).length() < 1e-5, "{hour}");
        }
        for hour in [6.5, 9.0, 12.0, 17.0] {
            let st = SkyState::at(hour);
            assert!(st.sun_dir.y > 0.0);
            assert!((st.light_dir - st.sun_dir).length() < 1e-5, "{hour}");
        }
    }

    /// The morning sun comes up in the east, which is +x (the map's right, `coords.rs`),
    /// and goes down in the west; at noon it stands in the south (+z).
    #[test]
    fn the_morning_sun_rises_in_the_east() {
        let dawn = SkyState::at(6.5);
        assert!(dawn.sun_elevation > 0.0 && dawn.sun_elevation < 20.0);
        assert!(dawn.sun_dir.x > 0.7, "dawn sun {:?}", dawn.sun_dir);
        let dusk = SkyState::at(17.5);
        assert!(dusk.sun_dir.x < -0.7, "dusk sun {:?}", dusk.sun_dir);
        let noon = SkyState::at(12.0);
        assert!(noon.sun_dir.z > 0.7 && noon.sun_dir.x.abs() < 1e-3);
        // east on the map: walking towards the morning sun moves the map arrow right
        let here = crate::map::bevy_to_map(Vec3::ZERO);
        let east = crate::map::bevy_to_map(Vec3::new(dawn.sun_dir.x, 0.0, dawn.sun_dir.z) * 500.0);
        assert!(east.x > here.x, "{here:?} → {east:?}");
    }

    #[test]
    fn dusk_is_between() {
        let mut a = app(10.0, 1.0);
        let dusk = at(&mut a, 18.5);
        let noon = at(&mut a, 12.0);
        let night = at(&mut a, 23.0);
        assert!(dusk.sun_lux < noon.sun_lux / 2.0 && night.sun_lux < noon.sun_lux / 3.0);
    }

    #[test]
    fn the_clock_runs_at_its_speed_and_wraps() {
        // 60 game minutes per real second, 0.1 s a frame: 6 game minutes a frame
        let mut a = app(23.0, 60.0);
        for _ in 0..10 {
            a.update();
        }
        let h = a.world().resource::<TimeOfDay>().hours;
        // first app.update() ran with a zero delta; 10 frames of 0.1 s later: +1 hour
        assert!((h - 0.0).abs() < 0.15 || (h - 24.0).abs() < 0.15, "{h}");
        for _ in 0..10 {
            a.update();
        }
        let h = a.world().resource::<TimeOfDay>().hours;
        assert!((h - 1.0).abs() < 0.15, "{h}");
        // paused: it stays
        a.world_mut().resource_mut::<TimeOfDay>().paused = true;
        a.update();
        assert_eq!(a.world().resource::<TimeOfDay>().hours, h);
    }

    #[test]
    fn the_default_pace_is_a_game_minute_a_second() {
        let mut t = TimeOfDay {
            hours: 10.0,
            speed: 1.0,
            paused: false,
        };
        t.advance(60.0);
        assert!((t.hours - 11.0).abs() < 1e-4);
        t.scrub(-0.5);
        assert!((t.hours - 10.5).abs() < 1e-4);
        t.scrub(14.0);
        assert!((t.hours - 0.5).abs() < 1e-4);
        assert_eq!(t.clock(), "00:30");
    }

    #[test]
    fn keys_scrub_and_pause() {
        let mut a = app(10.0, 1.0);
        a.world_mut().resource_mut::<TimeOfDay>().speed = 0.0;
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::BracketRight);
        keys.press(KeyCode::KeyT);
        a.insert_resource(keys);
        a.update();
        let t = a.world().resource::<TimeOfDay>();
        assert!((t.hours - 10.5).abs() < 0.01, "{}", t.hours);
        assert!(t.paused);
    }
}
