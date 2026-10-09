//! The per-hour atmosphere tables of `textures\atmosphere\gradients.bmp`, evaluated once
//! a frame (JC2-118; research `research/formats/light-tables-next.md`, JC2-100).
//!
//! [`AtmosphereState`] holds what the exe's table readers produce for the current hour and
//! the weather-variant weights. Native consumers and what this crate feeds:
//!
//! | value | native consumer | here |
//! |---|---|---|
//! | `cloud_lit` / `cloud_shaded` (`C`, `D`) | cloud card colours (`FUN_00a53a80`) | the cumulus cards' tint (`clouds.rs`, lit colour) |
//! | `haze_a/b/e/f` (`A`, `B`, `E`, `F`) | haze `FUN_00a432d0` -> `cb0[13]`/`cb0[14]` | `A`/`B` variants 1/2 only in the rain/snow fog colour (`haze.rs`, weights 0 until JC2-156); variant 0 is never read natively. `E`/`F` are the **water** colour `cb0[14]` (JC2-116), **stored only**, not fog |
//! | `point_light_scale` (`S`) | `FUN_0091dcc0`: point-light colour `* S * CRenderEngine+0x830` | muzzle-flash and explosion lights, whose base strength is in native colour units at 1 m (`native_units::point_lumens`, policy value; `+0x830` is constant 1.0 as far as a displacement sweep shows, `night-light-next.md` section 6) |
//! | `facade_night` (`S0`) | pixel `cb2[0].x` of the merged/facade render blocks (inferred: a "lights on" factor) | **stored only**: this crate has no facade/merged-block material |
//!
//! The six RGB tables of `gradients.bmp` do **not** feed the sun colour or the ambient. Those
//! are driven by `textures\landscape\gamezone_gradients.bmp` (JC2-115, `sky_light.rs`, JC2-122);
//! nothing here touches them. Interaction (gap): gamezone rows 2/3 also feed the cloud object,
//! so the cloud card colours set here from table `C` may be overridden or combined natively.
//!
//! **Weather variant weights.** `w1 = (1 - snow) * precip`, `w2 = snow * precip`
//! (`FUN_00a40080`). `precip` (precipitation object `+0x48`) and `snow` (`+0x50`) come from
//! the position-dependent weather-zone map and the cloud column (gaps G3, G7), neither of
//! which exists here, so both stay 0 and only variant 0 is used. They are plain fields so a
//! later producer can set them; nothing in this crate invents a zone map.

use crate::Game;
use crate::coords::to_bevy;
use crate::sky::TimeOfDay;
use bevy::prelude::*;
use jc2_formats::weather::{GradientTables, RgbTable, sun_direction, variant_weights};

/// The tables, when the files could be read.
#[derive(Resource, Default)]
pub struct GradientData(pub Option<GradientTables>);

/// The tables evaluated at the current hour (linear RGB).
#[derive(Resource, Clone, Debug)]
pub struct AtmosphereState {
    /// Hour 0..24 (`CRenderEngine+0xac`).
    #[allow(dead_code)] // the evaluation inputs, kept for inspection and tests
    pub hour: f32,
    /// Precipitation object `+0x48`; 0 until the zone map / cloud column exist (G3, G7).
    pub precip: f32,
    /// Precipitation object `+0x50` (snow factor); 0 as above.
    pub snow: f32,
    /// `(1 - snow) * precip`, `snow * precip`.
    #[allow(dead_code)] // the variant weights already applied to the values below
    pub w1: f32,
    #[allow(dead_code)]
    pub w2: f32,
    /// Whether the tables were read. When false every table value is the exe's constructor
    /// value (1.0) and consumers keep their project colours.
    pub loaded: bool,
    /// Unit vector towards the sun (Bevy axes), before the night flip to the moon.
    pub sun_dir: Vec3,
    /// Cloud card lit / shaded colour (`C`, `D`).
    pub cloud_lit: Vec3,
    pub cloud_shaded: Vec3,
    /// Haze tables `A`, `B`, `E`, `F` (variant-weighted).
    pub haze_a: Vec3,
    pub haze_b: Vec3,
    pub haze_e: Vec3,
    pub haze_f: Vec3,
    /// Point-light scale `S` (1.0 at night, 0.25 by day in variant 0), without `+0x830`.
    pub point_light_scale: f32,
    /// `S0`: 0 from 08:00 to 15:00, about 0.25 at night (inferred: facade lights factor).
    pub facade_night: f32,
}

impl Default for AtmosphereState {
    fn default() -> Self {
        Self::evaluate(None, 12.0, 0.0, 0.0)
    }
}

impl AtmosphereState {
    /// The tables at `hour` with the precipitation inputs `precip`/`snow`.
    pub fn evaluate(tables: Option<&GradientTables>, hour: f32, precip: f32, snow: f32) -> Self {
        let (w1, w2) = variant_weights(precip, snow);
        let mut s = Self {
            hour,
            precip,
            snow,
            w1,
            w2,
            loaded: tables.is_some(),
            sun_dir: to_bevy(sun_direction(hour)),
            cloud_lit: Vec3::ONE,
            cloud_shaded: Vec3::ONE,
            haze_a: Vec3::ONE,
            haze_b: Vec3::ONE,
            haze_e: Vec3::ONE,
            haze_f: Vec3::ONE,
            point_light_scale: 1.0,
            facade_night: 1.0,
        };
        if let Some(t) = tables {
            let rgb = |tab| Vec3::from(t.rgb_blend(tab, hour, w1, w2));
            s.cloud_lit = rgb(RgbTable::C);
            s.cloud_shaded = rgb(RgbTable::D);
            s.haze_a = rgb(RgbTable::A);
            s.haze_b = rgb(RgbTable::B);
            s.haze_e = rgb(RgbTable::E);
            s.haze_f = rgb(RgbTable::F);
            s.point_light_scale = t.scalar_blend(hour, w1, w2);
            s.facade_night = t.s0_at(hour);
        }
        s
    }

    /// The point-light scale for consumers: the table value, or 1 without tables so the
    /// project's own light intensities stand.
    pub fn point_light_factor(&self) -> f32 {
        if self.loaded {
            self.point_light_scale
        } else {
            1.0
        }
    }
}

pub fn load_gradients(game: Option<Res<Game>>, mut data: ResMut<GradientData>) {
    let Some(game) = game else { return };
    match GradientTables::load(&game.fs) {
        Ok(t) => data.0 = Some(t),
        Err(e) => warn!("atmosphere: gradients.bmp not read ({e}); keeping built-in colours"),
    }
}

/// A point light whose project-tuned intensity is multiplied by the native per-hour
/// point-light scale each frame ([`AtmosphereState::point_light_factor`]). The base
/// intensity (Bevy lumens) is the project's: built with `native_units::point_lumens`, i.e. in
/// native colour units at 1 m, but the strength itself is a policy value (the native colour
/// is also capped at 8 per channel and falls off with the light's radius, not decoded).
#[derive(Component, Clone, Copy, Debug)]
pub struct PointLightBase(pub f32);

pub fn scale_point_lights(
    state: Res<AtmosphereState>,
    mut lights: Query<(&PointLightBase, &mut PointLight)>,
) {
    let k = state.point_light_factor();
    for (base, mut light) in &mut lights {
        let target = base.0 * k;
        if light.intensity != target {
            light.intensity = target;
        }
    }
}

/// Re-evaluate every frame from the clock (the exe's readers do too).
pub fn update_atmosphere(
    time: Res<TimeOfDay>,
    data: Res<GradientData>,
    mut state: ResMut<AtmosphereState>,
) {
    let (precip, snow) = (state.precip, state.snow);
    *state = AtmosphereState::evaluate(data.0.as_ref(), time.hours, precip, snow);
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::weather::GRADIENTS_FILE;

    fn retail() -> Option<GradientTables> {
        let fs = GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()?;
        fs.contains(GRADIENTS_FILE)
            .then(|| GradientTables::load(&fs).expect("gradients.bmp parses"))
    }

    #[test]
    fn without_tables_the_constructor_values_stand_and_consumers_keep_their_look() {
        let s = AtmosphereState::evaluate(None, 3.0, 0.5, 0.0);
        assert!(!s.loaded);
        assert_eq!(s.cloud_lit, Vec3::ONE);
        assert_eq!(s.point_light_factor(), 1.0);
        assert_eq!((s.w1, s.w2), (0.5, 0.0));
    }

    #[test]
    fn the_state_follows_the_clock_and_keeps_variant_zero_without_a_zone_map() {
        let Some(t) = retail() else { return };
        let noon = AtmosphereState::evaluate(Some(&t), 12.0, 0.0, 0.0);
        assert_eq!((noon.w1, noon.w2), (0.0, 0.0));
        assert!((noon.cloud_lit - Vec3::new(0.560_499, 0.547_993, 0.517_401)).length() < 1e-4);
        assert!((noon.point_light_scale - 0.250_980_4).abs() < 1e-5);
        assert_eq!(noon.facade_night, 0.0);
        let midnight = AtmosphereState::evaluate(Some(&t), 0.0, 0.0, 0.0);
        assert_eq!(midnight.point_light_scale, 1.0);
        assert!((midnight.facade_night - 63.0 / 255.0).abs() < 1e-5);
        assert!((midnight.haze_f - Vec3::new(0.0, 0.026_549, 0.044_553)).length() < 1e-4);
        // 23.5 interpolates toward hour 0 (wrap), and precipitation mixes the variants in
        let late = AtmosphereState::evaluate(Some(&t), 23.5, 0.0, 0.0);
        assert!((late.cloud_lit.x - 0.100_115).abs() < 1e-4);
        let rain = AtmosphereState::evaluate(Some(&t), 12.0, 1.0, 0.0);
        assert_eq!((rain.w1, rain.w2), (1.0, 0.0));
        assert!((rain.cloud_lit.x - 0.015_175).abs() < 1e-4, "variant 1");
        assert!((rain.point_light_scale - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tagged_point_lights_follow_the_scale_and_untagged_ones_do_not() {
        let mut app = App::new();
        app.init_resource::<AtmosphereState>()
            .add_systems(Update, scale_point_lights);
        let tagged = app
            .world_mut()
            .spawn((PointLight::default(), PointLightBase(1000.0)))
            .id();
        let plain = app
            .world_mut()
            .spawn(PointLight {
                intensity: 7.0,
                ..default()
            })
            .id();
        // no tables: the project's intensity stands
        app.update();
        assert_eq!(
            app.world().get::<PointLight>(tagged).unwrap().intensity,
            1000.0
        );
        // tables at noon (variant 0): 0.25 of night
        let mut s = AtmosphereState::evaluate(None, 12.0, 0.0, 0.0);
        s.loaded = true;
        s.point_light_scale = 0.25;
        app.insert_resource(s);
        app.update();
        assert_eq!(
            app.world().get::<PointLight>(tagged).unwrap().intensity,
            250.0
        );
        assert_eq!(app.world().get::<PointLight>(plain).unwrap().intensity, 7.0);
    }

    #[test]
    fn the_system_re_evaluates_each_frame_and_keeps_the_precipitation_inputs() {
        let mut app = App::new();
        app.insert_resource(TimeOfDay {
            hours: 12.0,
            speed: 0.0,
            paused: true,
        })
        .init_resource::<GradientData>()
        .init_resource::<AtmosphereState>()
        .add_systems(Update, update_atmosphere);
        app.world_mut().resource_mut::<AtmosphereState>().precip = 0.5;
        app.world_mut().resource_mut::<TimeOfDay>().hours = 6.0;
        app.update();
        let s = app.world().resource::<AtmosphereState>();
        assert_eq!((s.hour, s.precip, s.w1), (6.0, 0.5, 0.5));
        assert!(
            s.sun_dir.y > 0.0 && s.sun_dir.x > 0.7,
            "dawn sun {:?}",
            s.sun_dir
        );
    }
}
