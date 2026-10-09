//! The gameplay camera's field of view (JC2-147; research `research/formats/camera-fov-next.md`).
//!
//! Native chain: the camera manager builds a controller *state* when a camera setting is
//! selected (`SetCamera`, `00838720`): `state.fov = min(setting.fov * fovfactor, 1.4)`, the
//! setting's `Fov` being per camera id (on foot 40 degrees, vertical; a setting no key sets
//! gets the struct default of 45). Every frame `0067c330` writes the engine camera's FOV as
//! `fov = now + w * (state.fov - now)`, `w` the weight of the current transition, ramped by
//! `dt / duration` (`0053efe0`; duration from `camera_transitions.bin`, 0.5 s when no group
//! maps the pair of settings). The vehicle speed filter (`0068d720`, vehicles and aircraft)
//! then moves `state.fov` from `Fov` towards a peak with speed through a spring. The
//! projection's aspect is the window's; the FOV is vertical (`004d0870`).
//!
//! What is native here: the per-id values, the cap, the per-frame blend formula with real
//! `dt`, the transition table, the speed filter and the order of the two per frame. What is
//! not: *which* camera id is selected for a jc2-rs state (the mapping in [`select_camera`]
//! is ours; its non-native parts are marked), the reverse "dolly" of the speed filter, the
//! aim, crouch, cover, ladder, mounted-gun and reverse cameras (no such states in jc2-rs),
//! and the 1-frame ordering between the matrix blend and the FOV (kept as in the brief).
//!
//! Only the gameplay camera is touched: the model viewer and capture cameras build their
//! own projections. [`CameraFov`] holds the realised vertical FOV for the consumers that
//! scale with it (terrain LOD radii by `1 / fov^2`, vegetation by `max(fov^2, 0.38)`).

use crate::boat::Watercraft;
use crate::character::Hero;
use crate::heli::Heli;
use crate::plane::Plane;
use crate::vehicle::{Occupied, Vehicle};
use crate::world::FlyCamera;
use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use jc2_formats::camera::{
    CameraData, FOV_FACTOR, PlayerCameras, SpeedFilter, SpeedFilterState, Transition,
};

/// The realised vertical field of view of the camera the player sees through, radians
/// (the native `camera+0x618`). The free fly camera and any camera nobody drives report
/// the projection's own value (Bevy's default 45 degrees).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct CameraFov {
    pub fov_y: f32,
}

impl Default for CameraFov {
    fn default() -> Self {
        Self {
            fov_y: std::f32::consts::FRAC_PI_4,
        }
    }
}

/// The realised vertical FOV the FOV-scaled consumers (terrain LOD radii, vegetation fades)
/// use this frame: [`CameraFov`] when the gameplay camera plugin is running, else the
/// camera's own perspective projection (JC2-148). `None` for no camera / an orthographic one.
pub fn realised_fov(camera_fov: Option<&CameraFov>, projection: Option<&Projection>) -> Option<f32> {
    camera_fov.map(|c| c.fov_y).or_else(|| match projection? {
        Projection::Perspective(p) => Some(p.fov),
        _ => None,
    })
}

/// The camera setting a vehicle's driver seat names (`camera_id`, key `0x742cdd23`) and the
/// one for reversing (`camera_id_reverse`; unused, see the Gaps in the brief).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleCamera {
    pub id: i32,
    pub reverse: Option<i32>,
}

/// Fallback camera ids by vehicle class when a vehicle has no `camera_id` (11 retail
/// vehicles have no `.mvdoll`; non-native choice): the Sedan, SmallBoat, Chopper Medium
/// and Small Plane settings, one per `Type` of `camera_settings.bin`.
pub const FALLBACK_CAR: i32 = 4;
pub const FALLBACK_BOAT: i32 = 33;
pub const FALLBACK_HELI: i32 = 44;
pub const FALLBACK_PLANE: i32 = 34;

/// What jc2-rs knows about the player when a camera is chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerView {
    /// Driving or flying: the camera id of the vehicle.
    pub vehicle: Option<i32>,
    pub downed: bool,
    pub parachute: bool,
    pub hanging: bool,
    pub on_wire: bool,
    pub swimming: bool,
    pub skydiving: bool,
}

/// The camera setting id for the player's state, through `settings/player.bin` `cameras`
/// (`006f77a0`): on foot `OnFoot`, `Parachute`, `SkyDive`, `Swim`, `Ragdoll`, `ReeledIn`,
/// `ReeledHang`. The precedence between those states, and using `Ragdoll` for the downed
/// body, are ours (the native states are separate controllers); crouch (69), aim (26, 3, 5),
/// cover, ladder, mounted guns and the reverse cameras have no jc2-rs state yet.
pub fn select_camera(p: &PlayerCameras, v: &PlayerView) -> i32 {
    let get = |k: &str| p.id(k).unwrap_or(0);
    if let Some(id) = v.vehicle {
        id
    } else if v.downed {
        get("Ragdoll")
    } else if v.parachute {
        get("Parachute")
    } else if v.hanging {
        get("ReeledHang")
    } else if v.on_wire {
        get("ReeledIn")
    } else if v.swimming {
        get("Swim")
    } else if v.skydiving {
        get("SkyDive")
    } else {
        get("OnFoot")
    }
}

/// The weight ramp of the current transition (`CCameraTransition`, `0053efe0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FovBlend {
    /// Transitions started so far (`manager+0x20`): with none, the target is written directly.
    started: u32,
    /// The weight `w` (transition `+0x10`).
    weight: f32,
    active: bool,
    transition: Transition,
    delay_elapsed: f32,
}

impl Default for FovBlend {
    fn default() -> Self {
        Self {
            started: 0,
            weight: 0.0,
            active: false,
            transition: Transition::DEFAULT,
            delay_elapsed: 0.0,
        }
    }
}

impl FovBlend {
    /// A camera change starts a transition (`006c7500`): weight 0, active.
    pub fn start(&mut self, t: Transition) {
        self.started += 1;
        self.weight = 0.0;
        self.active = true;
        self.transition = t;
        self.delay_elapsed = 0.0;
    }

    #[cfg(test)]
    pub fn weight(&self) -> f32 {
        self.weight
    }

    /// `CCameraTransition::update` (`0053efe0`): the weight only changes inside the ramp
    /// branch. With no FOV difference on a frame the transition ends and leaves the weight
    /// where it was, even at 0 (the "freeze quirk", decoded from the code only).
    fn update(&mut self, diff: f32, dt: f32) {
        if self.weight < 1.0 && self.active && diff != 0.0 {
            let t = self.transition;
            if t.delay > 0.0 && self.weight == 0.0 {
                self.delay_elapsed += dt;
                if self.delay_elapsed < t.delay {
                    return;
                }
                self.delay_elapsed = 0.0;
            }
            if t.mode == 1 {
                // FUN_004f0a30(w, 1.0, 1 / duration, dt, clamp)
                self.weight = if t.fov_seconds > 0.0 {
                    (self.weight + dt / t.fov_seconds).min(1.0)
                } else {
                    1.0
                };
            } else {
                self.weight = 1.0;
            }
            if self.weight == 1.0 {
                self.active = false;
            }
        } else {
            self.active = false;
        }
    }

    /// `0067c330`: the FOV to write to the camera this frame, or `None` to leave it.
    pub fn apply(&mut self, now: f32, target: f32, dt: f32) -> Option<f32> {
        if self.started == 0 {
            // no transition has ever been started: the target goes straight in
            return (target != now).then_some(target);
        }
        self.update(target - now, dt);
        Some(self.weight * (target - now) + now)
    }
}

/// The camera manager's FOV state: the active setting, its state FOV and filter, the blend.
#[derive(Resource, Debug)]
pub struct GameplayCamera {
    data: CameraData,
    pub fov_factor: f32,
    /// The active camera setting id (`controller+4`), once one is selected.
    pub active: Option<i32>,
    /// `state+0x70` of the active controller state, radians.
    pub state_fov: f32,
    filter: Option<SpeedFilter>,
    filter_state: SpeedFilterState,
    blend: FovBlend,
}

impl GameplayCamera {
    pub fn new(data: CameraData) -> Self {
        Self {
            data,
            fov_factor: FOV_FACTOR,
            active: None,
            state_fov: 0.0,
            filter: None,
            filter_state: SpeedFilterState::default(),
            blend: FovBlend::default(),
        }
    }

    /// `SetCamera` (`00548f30`): build the state for setting `id`; a change starts the
    /// transition its (from, to) pair maps to. Unknown ids are ignored (the native table
    /// holds disabled records for them).
    pub fn set_camera(&mut self, id: i32) {
        if self.active == Some(id) {
            return;
        }
        let Some(def) = self.data.settings.get(id) else {
            return;
        };
        info!(
            "camera: setting {id} {:?}, fov {:.1} deg",
            def.name,
            def.state_fov(self.fov_factor).to_degrees()
        );
        self.state_fov = def.state_fov(self.fov_factor);
        self.filter = def.speed_filter();
        self.filter_state = SpeedFilterState::default();
        if let Some(old) = self.active {
            let t = self.data.transitions.lookup(old, id);
            self.blend.start(t);
        }
        self.active = Some(id);
    }

    /// One frame of the manager update (`0081c960`): the FOV application with `now`, the
    /// engine camera's FOV, then the filters (which therefore act one frame late).
    /// `speed` is the player's speed in m/s. Returns the FOV to write.
    pub fn step(&mut self, now: f32, speed: f32, dt: f32) -> Option<f32> {
        self.active?;
        let out = self.blend.apply(now, self.state_fov, dt);
        if let Some(f) = &self.filter {
            self.state_fov = f.step(self.state_fov, speed, dt, &mut self.filter_state);
        }
        out
    }
}

/// jc2-rs's own trigger for the skydive pose (`Clip::Dive`, `character.rs`): falling for
/// more than 1.5 s faster than 12 m/s. The native SkyDive controller's entry condition is
/// not decoded (Gap); both settings 1 and 2 are 45 degrees, so only the transition group
/// depends on this.
pub fn is_skydiving(hero: &Hero, vertical_speed: f32) -> bool {
    !hero.grounded && hero.air_time > 1.5 && vertical_speed < -12.0
}

pub struct GameplayCameraPlugin;

impl Plugin for GameplayCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraFov>()
            .add_systems(Startup, load_camera_data)
            .add_systems(Update, drive_fov);
    }
}

fn load_camera_data(mut commands: Commands, game: Option<Res<crate::Game>>) {
    let Some(game) = game else { return };
    match CameraData::load(&game.fs) {
        Ok(data) => {
            info!(
                "camera: {} settings, {} transition groups, {} player camera keys",
                data.settings.settings.len(),
                data.transitions.groups.len(),
                data.player.ids.len()
            );
            commands.insert_resource(GameplayCamera::new(data));
        }
        Err(e) => warn!("camera settings unreadable ({e}); keeping the default projection"),
    }
}

type VehicleQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static VehicleCamera>,
        &'static LinearVelocity,
        Has<Vehicle>,
        Has<Watercraft>,
        Has<Heli>,
        Has<Plane>,
    ),
>;

fn vehicle_camera(vehicles: &VehicleQuery, entity: Entity) -> Option<(i32, f32)> {
    let (cam, vel, car, boat, heli, plane) = vehicles.get(entity).ok()?;
    let id = cam.map(|c| c.id).unwrap_or(if plane {
        FALLBACK_PLANE
    } else if heli {
        FALLBACK_HELI
    } else if boat {
        FALLBACK_BOAT
    } else if car {
        FALLBACK_CAR
    } else {
        return None;
    });
    Some((id, vel.0.length()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_fov(
    time: Res<Time>,
    state: Option<ResMut<GameplayCamera>>,
    mut fov: ResMut<CameraFov>,
    heroes: Query<(&Hero, Option<&LinearVelocity>)>,
    vehicles: VehicleQuery,
    occupied: Query<Entity, With<Occupied>>,
    mut cams: Query<&mut Projection, With<FlyCamera>>,
) {
    let Ok(mut projection) = cams.single_mut() else {
        return;
    };
    let Projection::Perspective(p) = &mut *projection else {
        return;
    };
    let Some(mut state) = state else {
        fov.fov_y = p.fov;
        return;
    };
    // who the player is: Rico, in a vehicle or not, or the lone driven vehicle (`--no-hero`)
    let hero = heroes.single().ok();
    let driven = hero
        .and_then(|(h, _)| h.in_vehicle)
        .or_else(|| occupied.single().ok());
    let vehicle = driven.and_then(|e| vehicle_camera(&vehicles, e));
    if hero.is_none() && vehicle.is_none() {
        // nobody to look through: the free camera keeps its own projection
        fov.fov_y = p.fov;
        return;
    }
    let mut view = PlayerView {
        vehicle: vehicle.map(|(id, _)| id),
        ..default()
    };
    let mut speed = vehicle.map(|(_, s)| s).unwrap_or(0.0);
    if let Some((h, vel)) = hero
        && vehicle.is_none()
    {
        let vy = vel.map_or(0.0, |v| v.0.y);
        speed = vel.map_or(0.0, |v| v.0.length());
        view.downed = h.downed;
        view.parachute = h.parachute;
        view.hanging = h.hanging;
        view.on_wire = h.grapple.is_some();
        view.swimming = h.swimming;
        view.skydiving = is_skydiving(h, vy);
    }
    let id = select_camera(&state.data.player, &view);
    state.set_camera(id);
    if let Some(next) = state.step(p.fov, speed, time.delta_secs()) {
        p.fov = next;
    }
    fov.fov_y = p.fov;
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::camera::deg_to_rad;
    use std::time::Duration;

    fn data() -> Option<CameraData> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        Some(CameraData::load(&GameFs::open(dir).ok()?).expect("camera data"))
    }

    #[test]
    fn blend_formula_and_ramp_follow_the_native_code() {
        let t = Transition {
            fov_seconds: 0.4,
            mode: 1,
            delay: 0.0,
        };
        let mut b = FovBlend::default();
        // no transition yet: the target is written directly, nothing if it already is
        assert_eq!(b.apply(0.5, 0.7, 0.016), Some(0.7));
        assert_eq!(b.apply(0.7, 0.7, 0.016), None);
        b.start(t);
        // each frame w += dt / duration, fov = now + w (target - now)
        let (mut now, target, dt) = (0.7_f32, 0.35_f32, 1.0 / 60.0);
        let mut w = 0.0_f32;
        for _ in 0..12 {
            w = (w + dt / 0.4).min(1.0);
            let want = now + w * (target - now);
            now = b.apply(now, target, dt).unwrap();
            assert!((now - want).abs() < 1e-6);
            assert!((b.weight() - w).abs() < 1e-6);
        }
        // the weight multiplies the remaining difference every frame, so the FOV reaches
        // the target (to the last f32 bit) before the weight does: a later target change
        // is then followed with the weight the ramp had reached, never beyond
        for _ in 0..48 {
            now = b.apply(now, target, dt).unwrap();
        }
        assert_eq!(now, target);
        assert!(b.weight() <= 1.0 && b.weight() > 0.5, "{}", b.weight());
        let next = b.apply(now, 0.5, dt).unwrap();
        let w = b.weight();
        assert!((next - (now + w * (0.5 - now))).abs() < 1e-7);
        // mode 0 (reverse cameras) is instant
        let mut b = FovBlend::default();
        b.start(Transition {
            fov_seconds: 0.0,
            mode: 0,
            delay: 0.0,
        });
        assert_eq!(b.apply(0.5, 0.9, dt), Some(0.9));
    }

    #[test]
    fn a_zero_difference_first_frame_freezes_the_weight() {
        // Inferred 3 of the brief: equal FOV on the first frame ends the transition at w = 0
        let mut b = FovBlend::default();
        b.start(Transition::DEFAULT);
        assert_eq!(b.apply(0.7, 0.7, 0.016), Some(0.7));
        assert_eq!(b.weight(), 0.0);
        // later state FOV changes (the speed filter) do not reach the camera
        assert_eq!(b.apply(0.7, 0.9, 0.016), Some(0.7));
    }

    #[test]
    fn blend_is_the_same_curve_at_any_frame_rate_only_in_time() {
        // w depends on elapsed time, so the weight after 0.25 s of a 0.5 s ramp is 0.5
        for fps in [32.0_f32, 64.0, 128.0] {
            let mut b = FovBlend::default();
            b.start(Transition::DEFAULT);
            let n = (0.25 * fps).round();
            let mut now = 0.7;
            for _ in 0..n as usize {
                now = b.apply(now, 0.8, 1.0 / fps).unwrap();
            }
            assert!((b.weight() - 0.5).abs() < 1e-5, "{fps}: {}", b.weight());
        }
    }

    #[test]
    fn camera_ids_follow_player_cameras() {
        let Some(d) = data() else { return };
        let p = &d.player;
        let v = PlayerView::default();
        assert_eq!(select_camera(p, &v), 0);
        let with = |f: fn(&mut PlayerView)| {
            let mut v = PlayerView::default();
            f(&mut v);
            select_camera(p, &v)
        };
        assert_eq!(with(|v| v.parachute = true), 1);
        assert_eq!(with(|v| v.skydiving = true), 2);
        assert_eq!(with(|v| v.swimming = true), 38);
        assert_eq!(with(|v| v.downed = true), 25);
        assert_eq!(with(|v| v.on_wire = true), 30);
        assert_eq!(with(|v| v.hanging = true), 65);
        assert_eq!(with(|v| v.vehicle = Some(62)), 62);
        // the retail on-foot FOV is 40 degrees, vertical
        let mut g = GameplayCamera::new(d.clone());
        g.set_camera(0);
        assert_eq!(g.state_fov, 0.698_131_7);
        assert_eq!(
            g.step(std::f32::consts::FRAC_PI_4, 0.0, 0.016),
            Some(0.698_131_7)
        );
        // the consumers' fixtures at the native on-foot FOV (probe: terrain k 2.0517542,
        // vegetation f 0.48738784, horizontal FOV 65.81 degrees at 16:9)
        let k = crate::terrain_material::lod_scale(g.state_fov);
        assert!((k - 2.051_754_2).abs() < 1e-5, "{k}");
        assert!((25.0 * k - 51.29).abs() < 0.01 && (250.0 * k - 512.9).abs() < 0.1);
        let f = (g.state_fov * g.state_fov).max(0.38);
        assert!((f - 0.487_387_84).abs() < 1e-6, "{f}");
        let hfov = 2.0 * ((g.state_fov / 2.0).tan() * 16.0 / 9.0).atan();
        assert!(
            (hfov.to_degrees() - 65.81).abs() < 0.01,
            "{}",
            hfov.to_degrees()
        );
    }

    #[test]
    fn mode_changes_ramp_by_the_transition_table() {
        let Some(d) = data() else { return };
        let mut g = GameplayCamera::new(d.clone());
        g.set_camera(0);
        let mut now = g.step(0.9, 0.0, 0.016).unwrap();
        assert_eq!(now, deg_to_rad(40.0));
        // 0 -> 2 (freefall group, 0.5 s) to 45 degrees
        g.set_camera(2);
        let target = deg_to_rad(45.0);
        let dt = 1.0 / 60.0;
        let mut frames = 0;
        while (now - target).abs() > 1e-4 && frames < 600 {
            now = g.step(now, 0.0, dt).unwrap();
            frames += 1;
        }
        // the weight ramps over 0.5 s = 30 frames but multiplies the remaining difference, so
        // the FOV settles in about 18 frames at 60 Hz: an ease-out (brief, Inferred 2)
        assert!((10..=30).contains(&frames), "{frames}");
        // on foot -> shoulder aim (closeup_aim, 0.4 s) uses its own duration
        assert_eq!(d.transitions.lookup(0, 26).fov_seconds, 0.4);
        assert_eq!(d.transitions.lookup(0, 3).fov_seconds, 0.1);
    }

    #[test]
    fn vehicle_filter_widens_car_fov_with_speed() {
        let Some(d) = data() else { return };
        let mut g = GameplayCamera::new(d);
        g.set_camera(4);
        assert_eq!(g.state_fov, deg_to_rad(50.0));
        let mut now = g.step(0.7, 0.0, 0.016).unwrap();
        for _ in 0..1200 {
            now = g.step(now, 40.0, 1.0 / 60.0).unwrap_or(now);
        }
        assert!((now - deg_to_rad(65.0)).abs() < 1e-3, "{now}");
        // the 1.4 rad cap applies to the state FOV (sports peak 90 degrees = 1.57 rad)
        let mut g = GameplayCamera::new(data().unwrap());
        g.set_camera(64);
        for _ in 0..3000 {
            g.step(0.9, 60.0, 1.0 / 60.0);
        }
        assert!(g.state_fov > 1.3, "{}", g.state_fov);
    }

    /// Headless: a camera, a hero, the manager. On foot the projection is 40 degrees
    /// vertical at once; a parachute ramps to 45 degrees over the 0.5 s transition.
    #[test]
    fn projection_follows_the_hero_state() {
        let Some(d) = data() else { return };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<CameraFov>()
            .insert_resource(GameplayCamera::new(d))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(16),
            ))
            .add_systems(Update, drive_fov);
        let cam = app
            .world_mut()
            .spawn((
                FlyCamera {
                    yaw: 0.0,
                    pitch: 0.0,
                    speed: 1.0,
                },
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        let hero = app.world_mut().spawn(Hero::default()).id();
        for _ in 0..3 {
            app.update();
        }
        let fov = |app: &App| match app.world().get::<Projection>(cam).unwrap() {
            Projection::Perspective(p) => p.fov,
            _ => unreachable!(),
        };
        assert_eq!(fov(&app), 0.698_131_7);
        assert_eq!(app.world().resource::<CameraFov>().fov_y, 0.698_131_7);
        app.world_mut().get_mut::<Hero>(hero).unwrap().parachute = true;
        app.update();
        let first = fov(&app);
        assert!(first > 0.698_131_7 && first < deg_to_rad(45.0), "{first}");
        for _ in 0..80 {
            app.update();
        }
        assert!((fov(&app) - deg_to_rad(45.0)).abs() < 1e-3, "{}", fov(&app));
    }

    /// JC2-148, end to end: the manager's FOV reaches the terrain LOD scale and the vegetation
    /// fade in the same frame. On foot 40 degrees: LOD0 51.29 m, LOD1 512.9 m, foliage fade
    /// 112.8..167.8 m; the parachute's 45 degrees then moves them to 40.5 m / 405.3 m and
    /// 89.2..144.2 m.
    #[test]
    fn consumers_follow_the_realised_fov() {
        use crate::terrain_material::TerrainLod;
        use crate::veg_fade::{VegetationFade, VegetationFadePlugin};
        let Some(d) = data() else { return };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(VegetationFadePlugin)
            .init_resource::<CameraFov>()
            .init_resource::<TerrainLod>()
            .insert_resource(GameplayCamera::new(d))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(16),
            ))
            .add_systems(Update, drive_fov);
        app.add_systems(Update, crate::terrain_material::update_terrain_lod.after(drive_fov));
        app.world_mut().spawn((
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection::default()),
        ));
        let hero = app.world_mut().spawn(Hero::default()).id();
        for _ in 0..3 {
            app.update();
        }
        let fade = |app: &App| {
            let f = app.world().resource::<VegetationFade>();
            (f.foliage.x / f.foliage.y, f.foliage.x / f.foliage.y + 1.0 / f.foliage.y)
        };
        let lod = *app.world().resource::<TerrainLod>();
        assert_eq!(lod.fov, 0.698_131_7);
        assert!((lod.lod0_radius() - 51.29).abs() < 0.01 && (lod.lod1_radius() - 512.9).abs() < 0.1);
        let (a, b) = fade(&app);
        assert!((a - 112.84).abs() < 0.05 && (b - 167.84).abs() < 0.05, "{a} {b}");
        app.world_mut().get_mut::<Hero>(hero).unwrap().parachute = true;
        for _ in 0..80 {
            app.update();
        }
        let lod = *app.world().resource::<TerrainLod>();
        assert!((lod.lod0_radius() - 40.53).abs() < 0.1, "{}", lod.lod0_radius());
        assert!((lod.lod1_radius() - 405.3).abs() < 1.0);
        let (a, b) = fade(&app);
        assert!((a - 89.16).abs() < 0.3 && (b - 144.16).abs() < 0.3, "{a} {b}");
    }
}
