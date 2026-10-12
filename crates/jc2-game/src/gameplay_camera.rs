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
pub fn realised_fov(
    camera_fov: Option<&CameraFov>,
    projection: Option<&Projection>,
) -> Option<f32> {
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
    /// `CBaseJumpController`'s state (`S_JUMP_TO_SKYDIVE`): the `BaseJump` camera.
    pub base_jump: bool,
}

/// The foot-machine states served by `CParachuteController` (camera `Parachute`, id 1;
/// table in `foot-conditions-next.md` section 4, from the player registration `00766260`).
pub fn is_parachute_controller_state(state: &str) -> bool {
    matches!(
        state,
        "S_UNFOLD_PARACHUTE_VERTICAL" | "S_UNFOLD_PARACHUTE_HORIZONTAL" | "S_PARACHUTE"
    )
}

/// Sets the controller-selected flags of `view` from the main-layer foot-machine state: one
/// controller serves each state, so at most one of the three is set. Native: the camera id
/// is the active controller's (`controller+4`, filled by `006f77a0`). Inferred: that the
/// manager follows the active controller each frame (`foot-conditions-next.md`).
pub fn apply_foot_state(view: &mut PlayerView, state: &str) {
    view.parachute = is_parachute_controller_state(state);
    view.skydiving = crate::hud::is_skydive_controller_state(state);
    view.base_jump = state == "S_JUMP_TO_SKYDIVE";
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
    } else if v.base_jump {
        get("BaseJump")
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

/// The active camera's right axis (`camera+0x24`, row 0 of its matrix), the input of the
/// parachute's camera-steered turn (`S+4`, `parachute_flight::camera_angle`).
pub fn camera_right(tf: &Transform) -> Vec3 {
    tf.rotation * Vec3::X
}

/// Fallback only, for a hero without a running foot machine (its data missing): falling for
/// more than 1.5 s faster than 12 m/s. This is a jc2-rs heuristic, not native; with the
/// machine running the camera and the dive pose follow its state ([`apply_foot_state`]).
pub fn is_skydiving(hero: &Hero, vertical_speed: f32) -> bool {
    !hero.grounded && hero.air_time > 1.5 && vertical_speed < -12.0
}

// ---- chute and skydive camera placement (board b78ce9ba) -------------------------------------
//
// Native model (research `research/formats/parachute-camera-placement.md`, board 3eac0c46):
// state builder `00838720` takes the pivot (`def+0x04`), camera offset (`def+0x84`) and target
// offset (`def+0x30`); the per-frame update `0062be20` smooths the pivot, the movement class
// composes the orientation and `00688380` places the camera at `A.pos + A.R * camera offset` and
// the target at `B.pos + B.R * target offset`. Everything the document marks "inferred" or
// "gap" is labelled so below.

/// Which native movement class the chute-family camera runs (`def+0x108`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChuteKind {
    /// Camera id `Parachute` (1): `CCameraMovementParachute` (`006b26a0`).
    Parachute,
    /// Camera ids `SkyDive` / `BaseJump` (2): `CCameraMovementFreefall` (`00652160`).
    Freefall,
}

/// The extra offset's approach speed, units per second (`004f0a30`, rate 0.5), and its targets
/// (`0x0106a928`, `0x0106a924`): up 0.5, back 1.3 (`006b26a0`).
pub const EXTRA_RATE: f32 = 0.5;
pub const EXTRA_UP: f32 = 0.5;
pub const EXTRA_BACK: f32 = 1.3;
/// Freefall tilt spring `k = c = 10` and the tilt magnitude 0.4 rad (`00652160`, constructor
/// `006ebed0`); thresholds 0.8 / 0.6 / 0.4 / 0.2 on `f = 1 - P`.
pub const TILT_SPRING: f32 = 10.0;
pub const TILT_MAX: f32 = 0.4;

fn wrap_pi(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    a - tau * ((a + std::f32::consts::PI) / tau).floor()
}

fn approach(x: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let step = rate * dt;
    if x < target {
        (x + step).min(target)
    } else {
        (x - step).max(target)
    }
}

/// The cubic bezier with control values 0, 0, 1, 1 (`004c1ae0`): `3t^2 - 2t^3`.
pub fn bezier_ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    3.0 * t * t - 2.0 * t * t * t
}

/// The freefall tilt target from `f = 1 - P` (`P` the skydive pitch pose, `S+0x148`), radians.
pub fn freefall_tilt_target(f: f32) -> f32 {
    if f >= 0.8 {
        TILT_MAX
    } else if f > 0.6 {
        TILT_MAX * (f - 0.6) / 0.2
    } else if f >= 0.4 {
        0.0
    } else if f > 0.2 {
        -TILT_MAX * (0.4 - f) / 0.2
    } else {
        -TILT_MAX
    }
}

/// The result of one rig step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChutePose {
    pub camera: Vec3,
    pub target: Vec3,
    /// The smoothed pivot the camera hangs from.
    pub pivot: Vec3,
    /// Absolute orientation angles in the project's convention
    /// (`Quat::from_euler(YXZ, yaw, pitch, 0)`).
    pub yaw: f32,
    pub pitch: f32,
}

/// One frame of inputs to [`ChuteRig::step`].
#[derive(Clone, Copy, Debug)]
pub struct ChuteFrame {
    /// The player character's world matrix at the feet (camera node id 2, research addendum A:
    /// `character + 0x15c`): position and the full orientation (canopy basis while the canopy
    /// flies, the skydive pose while falling, the plain heading otherwise).
    pub node_pos: Vec3,
    pub node_rot: Quat,
    /// The current `(yaw, pitch)` the input system has moved; its change since the last step
    /// is the input delta (the native offsets accumulate the input deltas).
    pub orbit: (f32, f32),
    /// `|S+4| > 0.9` (`parachute_flight::camera_angle`).
    pub turn_gate: bool,
    /// `1 - P` of the skydive pose (Freefall only).
    pub tilt_f: f32,
    /// `D+0x40`, the canopy yaw rate w34 in rad/s (`CanopyFlight::yaw_rate`; 0 outside the
    /// canopy): the Parachute yaw adds it unscaled (addendum B).
    pub yaw_rate: f32,
    pub dt: f32,
}

impl ChuteFrame {
    /// A frame at `node_pos` with the node turned `heading` about Y and nothing else set.
    #[cfg(test)]
    pub fn at(node_pos: Vec3, heading: f32, orbit: (f32, f32), dt: f32) -> Self {
        Self {
            node_pos,
            node_rot: Quat::from_rotation_y(heading),
            orbit,
            turn_gate: false,
            tilt_f: 0.5,
            yaw_rate: 0.0,
            dt,
        }
    }
}

/// Which native collision filter the record names (`def+0xbc`, factory `00771930`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChuteCollision {
    /// Class 1 `CCameraCollisionFull` (`00633c00`): snap in, damped spring out.
    Full,
    /// Class 8 `CCameraCollisionParachute` (`00536800`): constant-speed fraction.
    Parachute,
}

impl ChuteCollision {
    pub fn from_class(class: Option<i32>) -> Option<Self> {
        match class {
            Some(1) => Some(Self::Full),
            Some(8) => Some(Self::Parachute),
            _ => None,
        }
    }
}

/// `CCameraCollisionFull` spring `k = c = 10` (`007d13c0`) and the floor on the fraction
/// (`0x3a83126f`, `00633c00`).
pub const COLLISION_SPRING: f32 = 10.0;
pub const COLLISION_MIN_FRACTION: f32 = 0.000_999;
/// `CCameraCollisionParachute` constants (`0057e730`): fraction speed closing 3.8 / opening
/// 0.9 per second, the Y dip 2.0 and the floor 0.5 (`00536800`).
pub const PARACHUTE_CLOSE_SPEED: f32 = 3.8;
pub const PARACHUTE_OPEN_SPEED: f32 = 0.9;
pub const PARACHUTE_DIP: f32 = 2.0;
pub const PARACHUTE_FLOOR: f32 = 0.5;

/// The frame state of the chute-family camera: smoothed pivot, accumulated yaw / pitch offsets,
/// the recentre timers, the extra-offset ease-in, the freefall tilt spring, the collision
/// fraction and the input-filter hold timer.
#[derive(Resource, Clone, Debug)]
pub struct ChuteRig {
    /// The camera setting this state was built for (the native state is rebuilt on a change).
    pub built_for: Option<i32>,
    smoothed: Option<Vec3>,
    pub yaw_off: f32,
    pub pitch_off: f32,
    /// The node's own yaw (the Euler angle of the incoming matrix, `004de960`) of the last step.
    base_yaw: f32,
    /// The orbit angles written last frame; a difference is this frame's input delta.
    last_orbit: Option<(f32, f32)>,
    recentre: f32,
    recentre_from: f32,
    extra_up: f32,
    extra_back: f32,
    tilt: f32,
    tilt_v: f32,
    /// Freefall idle-recentre timer (`+0x30` of `00534e70`).
    idle: f32,
    /// Collision fraction `f` (`+0x24`, starts 1.0) and its spring velocity.
    coll_f: f32,
    coll_v: f32,
    // read only by `look_delta`, which has no stick source yet
    #[allow(dead_code)]
    accel: NonlinearAccel,
}

impl Default for ChuteRig {
    fn default() -> Self {
        Self {
            built_for: None,
            smoothed: None,
            yaw_off: 0.0,
            pitch_off: 0.0,
            base_yaw: 0.0,
            last_orbit: None,
            recentre: 0.0,
            recentre_from: 0.0,
            extra_up: 0.0,
            extra_back: 0.0,
            tilt: 0.0,
            tilt_v: 0.0,
            idle: 0.0,
            coll_f: 1.0,
            coll_v: 0.0,
            accel: NonlinearAccel::default(),
        }
    }
}

impl ChuteRig {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One frame. The sign of the native pitch against the project's (positive = camera
    /// below, looking up) is inferred.
    pub fn step(
        &mut self,
        kind: ChuteKind,
        pl: &jc2_formats::camera::Placement,
        f: &ChuteFrame,
    ) -> ChutePose {
        let dt = f.dt;
        let [ymin, ymax] = pl
            .yaw_limits
            .unwrap_or([-std::f32::consts::PI, std::f32::consts::PI]);
        let [pmin, pmax] = pl.pitch_limits.unwrap_or([-1.5, 1.5]);
        // base yaw / pitch: the Euler angles of the incoming matrix (`004de960`)
        let (base_yaw, base_pitch, _) = f.node_rot.to_euler(EulerRot::YXZ);
        self.base_yaw = base_yaw;
        let input = match self.last_orbit {
            Some((ly, lp)) => {
                let d = (wrap_pi(f.orbit.0 - ly), f.orbit.1 - lp);
                self.yaw_off += d.0;
                self.pitch_off += d.1;
                d.0 != 0.0 || d.1 != 0.0
            }
            None => {
                // first frame: the offsets are whatever puts the camera where the orbit is
                self.yaw_off = wrap_pi(f.orbit.0 - base_yaw);
                self.pitch_off = f.orbit.1;
                false
            }
        };
        if kind == ChuteKind::Freefall {
            // idle recentre (`00534e70`, addendum C): with both input deltas exactly 0 a timer
            // runs; after the delay (`+0x2c`, key `0xc26c0ec2` = 0.1 s) the offsets move
            // towards the rest pose 0 by `offset += (0 - offset) * w` each frame with
            // `w = clamp((timer - delay) / duration)`, duration = key `0xc4afb170` (2.0 s).
            // Any non-zero input resets the timer.
            let delay = pl.move_b.unwrap_or(0.0);
            if delay > 0.0 {
                if input {
                    self.idle = 0.0;
                } else {
                    self.idle += dt;
                    if self.idle > delay {
                        let dur = pl.move_a.unwrap_or(0.0);
                        let w = if dur > 0.0 {
                            ((self.idle - delay) / dur).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        self.yaw_off -= self.yaw_off * w;
                        self.pitch_off -= self.pitch_off * w;
                    }
                }
            }
        }
        self.yaw_off = wrap_pi(self.yaw_off).clamp(ymin, ymax);
        self.pitch_off = self.pitch_off.clamp(pmin, pmax);

        let node_rot = f.node_rot;
        let cam_off = Vec3::from(pl.camera_offset.unwrap_or([0.0; 3]));
        let tgt_off = Vec3::from(pl.target_offset.unwrap_or([0.0; 3]));
        let pivot = f.node_pos + node_rot * Vec3::from(pl.pivot.unwrap_or([0.0; 3]));
        // pivot smoothing (`00576360`): exponential in the node's local frame; none when the
        // record has no interpolation smoothing
        let smoothed = match (pl.pivot_smoothing_rate(), self.smoothed) {
            (Some(rate), Some(prev)) => {
                let k = 1.0 - (rate * dt).clamp(0.0, 1.0);
                pivot + node_rot * (node_rot.inverse() * (prev - pivot) * k)
            }
            _ => pivot,
        };
        self.smoothed = Some(smoothed);

        let (rot, extra, target_rot) = match kind {
            ChuteKind::Parachute => {
                // auto recentre (`006b26a0`): with no input and |S+4| > 0.9 a timer runs,
                // `t = timer / 12 s`, and the yaw offset is reduced by `offset * bezier(t)`,
                // 0 at `t >= 1`. INFERRED: the doc does not say whether `offset` is the value
                // at the timer start (read here: the offset glides to 0 along the bezier over
                // the 12 s) or the current value each frame (that compounds and would be
                // done in about a second, which a 12 s duration does not suggest).
                if !input && f.turn_gate {
                    if self.recentre == 0.0 {
                        self.recentre_from = self.yaw_off;
                    }
                    self.recentre += dt;
                    if let Some(dur) = pl.move_a.filter(|d| *d > 0.0) {
                        let t = self.recentre / dur;
                        if t < 1.0 {
                            self.yaw_off = self.recentre_from * (1.0 - bezier_ease(t));
                        } else {
                            self.yaw_off = 0.0;
                        }
                    }
                } else {
                    self.recentre = 0.0;
                }
                self.extra_up = approach(self.extra_up, EXTRA_UP, EXTRA_RATE, dt);
                self.extra_back = approach(self.extra_back, EXTRA_BACK, EXTRA_RATE, dt);
                // yaw = w34 + yaw offset + base yaw; pitch = base pitch + pitch offset; the
                // orientation is replaced by it (roll dropped), `006b26a0` / addendum B
                let rot = Quat::from_euler(
                    EulerRot::YXZ,
                    base_yaw + self.yaw_off + f.yaw_rate,
                    base_pitch + self.pitch_off,
                    0.0,
                );
                // no target-rotation mode for this record (mode 0): B keeps the node basis
                (rot, Vec3::new(0.0, self.extra_up, self.extra_back), node_rot)
            }
            ChuteKind::Freefall => {
                // damped spring about the target (`00652160`, dt capped at 0.1)
                let target = freefall_tilt_target(f.tilt_f);
                let sdt = dt.min(jc2_formats::camera::SPRING_MAX_DT);
                let x = self.tilt - target;
                self.tilt_v += (-TILT_SPRING * x - TILT_SPRING * self.tilt_v) * sdt;
                let mut x = x + self.tilt_v * sdt;
                if x.abs() < jc2_formats::camera::SPRING_DEAD_BAND {
                    x = 0.0;
                }
                self.tilt = x + target;
                let input_rot = Quat::from_euler(EulerRot::YXZ, self.yaw_off, self.pitch_off, 0.0);
                // `00640fd0`: the target matrix is also rotated by the input offsets, mode 1
                // yaw and pitch, 2 yaw only, 3 pitch only, else not at all (addendum C). The
                // operand order follows the camera's (not independently verified).
                let target_rot = match pl.target_mode {
                    Some(1) => node_rot * input_rot,
                    Some(2) => node_rot * Quat::from_rotation_y(self.yaw_off),
                    Some(3) => node_rot * Quat::from_rotation_x(self.pitch_off),
                    _ => node_rot,
                };
                (node_rot * Quat::from_rotation_x(self.tilt) * input_rot, Vec3::ZERO, target_rot)
            }
        };
        let (yaw, pitch, _) = rot.to_euler(EulerRot::YXZ);
        self.last_orbit = Some(self.orbit());
        ChutePose {
            camera: smoothed + rot * (cam_off + extra),
            // the target side has no smoothing (type 0, `006b78e0`): B.pos is the plain pivot
            target: pivot + target_rot * tgt_off,
            pivot: smoothed,
            yaw,
            pitch,
        }
    }

    /// The orbit angles the input system carries: the node's yaw plus the clamped offsets.
    pub fn orbit(&self) -> (f32, f32) {
        (self.base_yaw + self.yaw_off, self.pitch_off)
    }

    /// The collision filter's post slot (addendum D) for this frame's pose. `hit` is the
    /// fraction of the target-to-camera line that is free (1 = nothing in the way; the
    /// project's ray cast stands in for the native shape cast `007b9fe0`, which stays a gap).
    /// Returns the final `(camera, target)`; `class` `None` (a record without a collision
    /// filter) passes the pose through.
    pub fn collide(
        &mut self,
        class: Option<ChuteCollision>,
        pose: &ChutePose,
        hit: f32,
        dt: f32,
    ) -> (Vec3, Vec3) {
        let hit = hit.clamp(0.0, 1.0);
        let line = pose.camera - pose.target;
        let d = line.length();
        match class {
            None => (pose.camera, pose.target),
            Some(ChuteCollision::Full) => {
                let f = collision_full_step(&mut self.coll_f, &mut self.coll_v, hit, dt);
                (pose.target + line * f, pose.target)
            }
            Some(ChuteCollision::Parachute) => {
                let speed = if hit < self.coll_f {
                    PARACHUTE_CLOSE_SPEED
                } else {
                    PARACHUTE_OPEN_SPEED
                };
                let raw = approach(self.coll_f, hit, speed, dt);
                let f = raw.clamp(PARACHUTE_FLOOR, 1.0);
                self.coll_f = f;
                let u = line.normalize_or_zero();
                // the camera at `f^2 D` from the old target, the target moved back (away from
                // the camera) by `(f - f^2) D`: separation `f D`
                let mut target = pose.target - u * ((f - f * f) * d);
                let mut camera = pose.target + u * (f * f * d);
                if raw < PARACHUTE_FLOOR {
                    // floored: re-placed `1 + D (0.5 - f)` from the new target (INFERRED that
                    // `f` here is the unfloored value)
                    camera = target + u * (1.0 + d * (PARACHUTE_FLOOR - raw));
                }
                let dip = Vec3::Y * ((1.0 - f) * PARACHUTE_DIP);
                target -= dip;
                camera -= dip;
                (camera, target)
            }
        }
    }

    /// The look-input deltas `(yaw, pitch)` in radians of the record's input filter from the
    /// raw look stick (addendum E): Nonlinear (class 1) or NonlinearAccel (class 2).
    /// `scale` is the per-axis device scale `(X, Y)` and `sens` the record sensitivity `s`
    /// (key `0x24868714`, 1.0 when absent; times 0.6 for camera id 2 without option 0x10).
    /// The device scales and the mouse mapping are a gap (nothing in the project yields a
    /// stick yet), so nothing calls this at runtime.
    #[allow(dead_code)]
    pub fn look_delta(
        &mut self,
        class: Option<i32>,
        stick: (f32, f32),
        scale: (f32, f32),
        sens: f32,
        dt: f32,
    ) -> (f32, f32) {
        let out = match class {
            Some(1) => nonlinear(stick),
            Some(2) => self.accel.filter(stick, scale, dt),
            _ => stick,
        };
        (scale.0 * sens * out.0 * dt, scale.1 * sens * out.1 * dt)
    }
}

/// `CCameraCollisionFull` post update (`00633c00`): the fraction `f` snaps down to the free
/// fraction `h` at once; when `h >= f` it relaxes with a damped spring `k = c = 10`
/// (`dt' = min(dt, 0.1)`), settling to `h` within `1e-5`, never below `0.000999`.
fn collision_full_step(f: &mut f32, v: &mut f32, h: f32, dt: f32) -> f32 {
    if h < *f {
        // the velocity is left as is: the doc does not say it is cleared (unknown)
        *f = h;
    } else {
        let dt = dt.min(jc2_formats::camera::SPRING_MAX_DT);
        let a = -COLLISION_SPRING * (*f - h) - COLLISION_SPRING * *v;
        *v += a * dt;
        *f += *v * dt;
        if (*f - h).abs() < 1e-5 {
            // INFERRED: settling also stops the motion
            *f = h;
            *v = 0.0;
        }
    }
    *f = f.max(COLLISION_MIN_FRACTION);
    *f
}

/// `CCameraInputNonlinear` (`0074e830`): `t = clamp(m^4 + 0.3, 0, 1)`, `out = 1.5 * stick * t`.
#[allow(dead_code)] // unwired until a look-stick source exists (research Gap 3)
pub fn nonlinear(stick: (f32, f32)) -> (f32, f32) {
    let m = stick.0.hypot(stick.1);
    if m == 0.0 {
        return (0.0, 0.0);
    }
    let t = (m.powi(4) + 0.3).clamp(0.0, 1.0);
    (1.5 * stick.0 * t, 1.5 * stick.1 * t)
}

/// `CCameraInputNonlinearAccel` (`0079b680`): dead zone `m > 0.01`; `out = lerp(0.05 dir,
/// 1.2 dir, clamp(m^2))`; holding the stick at full deflection in a steady direction
/// (`m > 0.98`, dot of the previous and current stick above 0.98) grows a timer, and over
/// 0.75 s the output blends to `(4.0 + (2.5 - 4.0) * scale / 2) * stick` per axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NonlinearAccel {
    hold: f32,
    prev: (f32, f32),
}

impl NonlinearAccel {
    #[allow(dead_code)] // unwired until a look-stick source exists (research Gap 3)
    pub fn filter(&mut self, stick: (f32, f32), scale: (f32, f32), dt: f32) -> (f32, f32) {
        let m = stick.0.hypot(stick.1);
        let prev = self.prev;
        self.prev = stick;
        if m <= 0.01 {
            self.hold = 0.0;
            return (0.0, 0.0);
        }
        let dir = (stick.0 / m, stick.1 / m);
        let t = (m * m).clamp(0.0, 1.0);
        let gain = 0.05 + (1.2 - 0.05) * t;
        let mut out = (dir.0 * gain, dir.1 * gain);
        let dot = prev.0 * stick.0 + prev.1 * stick.1;
        if m > 0.98 && dot > 0.98 {
            self.hold += dt;
            let w = (self.hold / 0.75).clamp(0.0, 1.0);
            let fast = (
                (4.0 + (2.5 - 4.0) * scale.0 / 2.0) * stick.0,
                (4.0 + (2.5 - 4.0) * scale.1 / 2.0) * stick.1,
            );
            out = (out.0 + (fast.0 - out.0) * w, out.1 + (fast.1 - out.1) * w);
        } else {
            self.hold = 0.0;
        }
        out
    }
}

impl GameplayCamera {
    /// The chute-family placement of the active camera, when it is the Parachute (id from
    /// `player.bin`) or the SkyDive / BaseJump one.
    pub fn chute_placement(&self) -> Option<(i32, ChuteKind, &jc2_formats::camera::Placement)> {
        let id = self.active?;
        let kind = if Some(id) == self.data.player.id("Parachute") {
            ChuteKind::Parachute
        } else if Some(id) == self.data.player.id("SkyDive")
            || Some(id) == self.data.player.id("BaseJump")
        {
            ChuteKind::Freefall
        } else {
            return None;
        };
        Some((id, kind, &self.data.settings.get(id)?.placement))
    }
}

// ---- on-foot camera placement (board c0c2758e; research `research/formats/onfoot-camera-placement.md`) ----
//
// Native: camera id `OnFoot` (0) is a record of type `Normal` with camera node = target node = 2
// (the character matrix at the feet), no pivot, no smoothing, camera offset (0.55, 1.55, 3.0)
// and target offset (0.55, 1.55, 0.0), FOV 40 deg, pitch limits -55 .. +85 deg, movement class
// 1 `CCameraMovementFree` (`0053b930`, update `0063d0f0`) and target-matrix mode 2 (yaw only,
// `00640fd0`). `00688380` composes `camera = A.pos + A.R * camera offset` with `A.R` the orbit
// rotation (yaw, pitch) and `target = B.pos + Ry(yaw) * target offset`. The record names the
// offset filter type 5 (`0053e7d0`): `CCameraRunFilter` (`006faa50`, update `007a17f0`), which
// adds `run` metres to the camera offset's Z (the distance behind) every frame.

/// `CCameraRunFilter` constants (`006faa50`: `+4` = 10.0, `+0xc` = `DAT_01066a84` = 1.5, `+0x10`
/// = `DAT_01066a80` = 2.5): the squared speed above which the camera pulls back, and the extra
/// distance for running and for dashing (`S_DASH`, `S_DASH_TURN_180`, `S_INSTANT_DASH`).
pub const RUN_SPEED_SQ: f32 = 10.0;
pub const RUN_EXTRA: f32 = 1.5;
pub const DASH_EXTRA: f32 = 2.5;
/// The approach speed is the remaining distance clamped to 0.2 .. 1.0 m/s (`007a17f0`).
pub const RUN_RATE_MIN: f32 = 0.2;
pub const RUN_RATE_MAX: f32 = 1.0;
/// The `OnFoot` record's pitch limits in radians (`Normal` default -55 .. +85 degrees).
pub const FOOT_PITCH_LIMITS: [f32; 2] = [-0.959_931_1, 1.483_529_8];

/// The foot-machine states `00615e60` tests (`S_DASH` family, `rico-locomotion-motion.md`).
pub fn is_dash_state(state: &str) -> bool {
    matches!(state, "S_DASH" | "S_DASH_TURN_180" | "S_INSTANT_DASH")
}

/// The run filter's target distance (`007a17f0`): dashing 2.5, else 1.5 when the squared speed
/// is above 10, else 0. INFERRED: the speed vector is the character's velocity (the decompiler
/// shows the vector filled by a call it does not resolve).
pub fn run_target(dashing: bool, speed_sq: f32) -> f32 {
    if dashing {
        DASH_EXTRA
    } else if speed_sq > RUN_SPEED_SQ {
        RUN_EXTRA
    } else {
        0.0
    }
}

/// The run filter's state (`filter+8`): the extra distance behind, 0 at standstill.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct FootRig {
    pub run: f32,
}

impl FootRig {
    /// `007a17f0`: the extra moves toward `target` at a constant speed of `|target - run|`
    /// clamped to 0.2 .. 1.0 per second (`004f0a30`, no overshoot). Returns the new extra.
    pub fn step(&mut self, target: f32, dt: f32) -> f32 {
        let rate = (target - self.run).abs().clamp(RUN_RATE_MIN, RUN_RATE_MAX);
        self.run = approach(self.run, target, rate, dt);
        self.run
    }
}

/// The on-foot camera's pose from the record: camera `node + R(yaw, pitch) * (offset + (0,0,run))`
/// and target `node + B * target offset`, `B` by the record's target mode (`00640fd0`: 1 yaw and
/// pitch, 2 yaw only, 3 pitch only, otherwise the node's own rotation).
pub fn foot_pose(
    pl: &jc2_formats::camera::Placement,
    node_pos: Vec3,
    node_rot: Quat,
    (yaw, pitch): (f32, f32),
    run: f32,
) -> (Vec3, Vec3) {
    let rot = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    let cam_off = Vec3::from(pl.camera_offset.unwrap_or([0.0; 3])) + Vec3::Z * run;
    let tgt_off = Vec3::from(pl.target_offset.unwrap_or([0.0; 3]));
    let target_rot = match pl.target_mode {
        Some(1) => rot,
        Some(2) => Quat::from_rotation_y(yaw),
        Some(3) => Quat::from_rotation_x(pitch),
        _ => node_rot,
    };
    (node_pos + rot * cam_off, node_pos + target_rot * tgt_off)
}

impl GameplayCamera {
    /// The placement of the active camera when it is the on-foot one (`OnFoot`, id 0).
    pub fn foot_placement(&self) -> Option<&jc2_formats::camera::Placement> {
        let id = self.active?;
        if Some(id) != self.data.player.id("OnFoot") {
            return None;
        }
        Some(&self.data.settings.get(id)?.placement)
    }
}

pub struct GameplayCameraPlugin;

impl Plugin for GameplayCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraFov>()
            .init_resource::<ChuteRig>()
            .init_resource::<FootRig>()
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
    heroes: Query<(
        &Hero,
        Option<&LinearVelocity>,
        Option<&crate::foot_machine::FootDriver>,
    )>,
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
        .and_then(|(h, _, _)| h.in_vehicle)
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
    if let Some((h, vel, driver)) = hero
        && vehicle.is_none()
    {
        let vy = vel.map_or(0.0, |v| v.0.y);
        speed = vel.map_or(0.0, |v| v.0.length());
        view.downed = h.downed;
        view.parachute = h.parachute;
        view.hanging = h.hanging;
        view.on_wire = h.grapple.is_some();
        view.swimming = h.swimming;
        match driver.filter(|d| d.stepped()) {
            // the controller serving the foot-machine state owns the camera id
            Some(d) => apply_foot_state(&mut view, &d.state),
            None => view.skydiving = is_skydiving(h, vy),
        }
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
        assert_eq!(with(|v| v.base_jump = true), p.id("BaseJump").unwrap());
        // the foot-machine state picks the controller's camera
        let by_state = |s: &str| {
            let mut v = PlayerView::default();
            apply_foot_state(&mut v, s);
            select_camera(p, &v)
        };
        for s in crate::hud::SKYDIVE_CONTROLLER_STATES {
            assert_eq!(by_state(s), 2, "{s}");
        }
        for s in [
            "S_UNFOLD_PARACHUTE_VERTICAL",
            "S_UNFOLD_PARACHUTE_HORIZONTAL",
            "S_PARACHUTE",
        ] {
            assert_eq!(by_state(s), 1, "{s}");
        }
        assert_eq!(by_state("S_FALL"), 0);
        assert_eq!(by_state("S_PARACHUTE_LANDING"), 0);
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
        app.add_systems(
            Update,
            crate::terrain_material::update_terrain_lod.after(drive_fov),
        );
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
            (
                f.foliage.x / f.foliage.y,
                f.foliage.x / f.foliage.y + 1.0 / f.foliage.y,
            )
        };
        let lod = *app.world().resource::<TerrainLod>();
        assert_eq!(lod.fov, 0.698_131_7);
        assert!(
            (lod.lod0_radius() - 51.29).abs() < 0.01 && (lod.lod1_radius() - 512.9).abs() < 0.1
        );
        let (a, b) = fade(&app);
        assert!(
            (a - 112.84).abs() < 0.05 && (b - 167.84).abs() < 0.05,
            "{a} {b}"
        );
        app.world_mut().get_mut::<Hero>(hero).unwrap().parachute = true;
        for _ in 0..80 {
            app.update();
        }
        let lod = *app.world().resource::<TerrainLod>();
        assert!(
            (lod.lod0_radius() - 40.53).abs() < 0.1,
            "{}",
            lod.lod0_radius()
        );
        assert!((lod.lod1_radius() - 405.3).abs() < 1.0);
        let (a, b) = fade(&app);
        assert!(
            (a - 89.16).abs() < 0.3 && (b - 144.16).abs() < 0.3,
            "{a} {b}"
        );
    }

    // ---- chute placement fixtures (probe values of `chute_camera_probe.py`) -------------------

    fn chute_placement() -> jc2_formats::camera::Placement {
        jc2_formats::camera::Placement {
            pivot: Some([0.0, 3.75, -0.5]),
            camera_offset: Some([0.0, 0.0, 6.75]),
            target_offset: Some([0.0, -1.0, 0.0]),
            yaw_limits: Some([deg_to_rad(-70.0), deg_to_rad(70.0)]),
            pitch_limits: Some([deg_to_rad(-65.0), deg_to_rad(75.0)]),
            smoothing_type: Some(2),
            smoothing_rate: Some(5.0),
            nodes: Some((2, 2)),
            move_a: Some(12.0),
            move_b: Some(0.1),
            target_mode: None,
            collision: Some(8),
            input_filter: Some(2),
        }
    }

    fn skydive_placement() -> jc2_formats::camera::Placement {
        jc2_formats::camera::Placement {
            camera_offset: Some([0.0, 3.0, 2.5]),
            target_offset: Some([0.0, 0.5, -1.0]),
            yaw_limits: Some([deg_to_rad(-85.0), deg_to_rad(85.0)]),
            pitch_limits: Some([deg_to_rad(-25.0), deg_to_rad(45.0)]),
            move_a: Some(2.0),
            move_b: Some(0.1),
            target_mode: Some(1),
            collision: Some(1),
            input_filter: Some(1),
            ..Default::default()
        }
    }

    const DT: f32 = 1.0 / 60.0;

    fn chute_step(rig: &mut ChuteRig, pos: Vec3, orbit: (f32, f32), gate: bool, dt: f32) -> ChutePose {
        let f = ChuteFrame { turn_gate: gate, ..ChuteFrame::at(pos, 0.0, orbit, dt) };
        rig.step(ChuteKind::Parachute, &chute_placement(), &f)
    }

    fn free_step(rig: &mut ChuteRig, pos: Vec3, orbit: (f32, f32), tilt_f: f32, dt: f32) -> ChutePose {
        let f = ChuteFrame { tilt_f, ..ChuteFrame::at(pos, 0.0, orbit, dt) };
        rig.step(ChuteKind::Freefall, &skydive_placement(), &f)
    }

    #[test]
    fn extra_offset_eases_in_over_one_and_two_point_six_seconds() {
        let mut rig = ChuteRig::default();
        for _ in 0..60 {
            chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, DT);
        }
        assert!((rig.extra_up - 0.5).abs() < 1e-4, "up reached after 1 s");
        assert!((rig.extra_back - 0.5).abs() < 1e-4, "back is 0.5 after 1 s");
        let mut rig = ChuteRig::default();
        for _ in 0..155 {
            chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, DT);
        }
        assert!(rig.extra_back < 1.3, "not yet at 2.6 s: {}", rig.extra_back);
        for _ in 0..3 {
            chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, DT);
        }
        assert!((rig.extra_back - 1.3).abs() < 1e-6);
    }

    #[test]
    fn parachute_camera_settles_at_the_native_offsets() {
        let mut rig = ChuteRig::default();
        let mut pose = None;
        for _ in 0..300 {
            pose = Some(chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, DT));
        }
        let pose = pose.unwrap();
        // heading 0, level: pivot (0, 3.75, -0.5) in the node frame, camera (0, 0.5, 6.75 + 1.3)
        // behind it (+Z), target = pivot + (0, -1, 0)
        let node_pivot = Vec3::new(0.0, 3.75, -0.5);
        assert!((pose.pivot - node_pivot).length() < 1e-5);
        assert!((pose.camera - (node_pivot + Vec3::new(0.0, 0.5, 8.05))).length() < 1e-4);
        assert!((pose.target - (node_pivot + Vec3::new(0.0, -1.0, 0.0))).length() < 1e-5);
    }

    #[test]
    fn pivot_smoothing_lags_by_one_minus_rate_dt_in_the_character_frame() {
        let dt = 0.02;
        let mut rig = ChuteRig::default();
        chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, dt);
        // the node jumps 1 m along +x: the smoothed pivot keeps (1 - 5 dt) of the lag
        let pose = chute_step(&mut rig, Vec3::X, (0.0, 0.0), false, dt);
        let want_x = 1.0 - (1.0 - 5.0 * dt);
        assert!((pose.pivot.x - want_x).abs() < 1e-6, "{}", pose.pivot.x);
        // SkyDive has no smoothing: the pivot is the node at once
        let mut rig = ChuteRig::default();
        free_step(&mut rig, Vec3::ZERO, (0.0, 0.0), 0.5, dt);
        let pose = free_step(&mut rig, Vec3::X, (0.0, 0.0), 0.5, dt);
        assert_eq!(pose.pivot, Vec3::X);
    }

    #[test]
    fn input_accumulates_and_clamps_to_the_limits() {
        let mut rig = ChuteRig::default();
        chute_step(&mut rig, Vec3::ZERO, (0.0, 0.0), false, DT);
        let (mut y, mut p) = (0.0_f32, 0.0_f32);
        for _ in 0..200 {
            y += 0.05;
            p -= 0.05;
            chute_step(&mut rig, Vec3::ZERO, (y, p), false, DT);
            // the input system carries the clamped value back, as `hero_camera` does
            (y, p) = rig.orbit();
        }
        assert!((rig.yaw_off - deg_to_rad(70.0)).abs() < 1e-5);
        assert!((rig.pitch_off - deg_to_rad(-65.0)).abs() < 1e-5);
    }

    #[test]
    fn recentre_needs_no_input_and_the_gate_and_takes_twelve_seconds() {
        let seed = |rig: &mut ChuteRig| {
            chute_step(rig, Vec3::ZERO, (0.0, 0.0), false, DT);
            rig.yaw_off = 1.0;
            rig.last_orbit = Some(rig.orbit());
        };
        // gate closed: the offset stays
        let mut rig = ChuteRig::default();
        seed(&mut rig);
        for _ in 0..100 {
            let o = rig.orbit();
            chute_step(&mut rig, Vec3::ZERO, o, false, DT);
        }
        assert_eq!(rig.yaw_off, 1.0);
        // gate open: the offset falls monotonically and is 0 once the 12 s timer ends
        let mut rig = ChuteRig::default();
        seed(&mut rig);
        let mut last = 1.0;
        for i in 0..730 {
            let o = rig.orbit();
            chute_step(&mut rig, Vec3::ZERO, o, true, DT);
            assert!(rig.yaw_off <= last + 1e-6, "frame {i}");
            last = rig.yaw_off;
            if i == 600 {
                assert!(rig.yaw_off > 0.0, "still recentring before 12 s");
            }
        }
        assert_eq!(rig.yaw_off, 0.0);
        // any input resets the timer
        let mut rig = ChuteRig::default();
        seed(&mut rig);
        for _ in 0..30 {
            let o = rig.orbit();
            chute_step(&mut rig, Vec3::ZERO, o, true, DT);
        }
        assert!(rig.recentre > 0.0);
        let o = rig.orbit();
        chute_step(&mut rig, Vec3::ZERO, (o.0 + 0.01, o.1), true, DT);
        assert_eq!(rig.recentre, 0.0);
    }

    #[test]
    fn freefall_tilt_targets_and_spring() {
        for (f, want) in [
            (1.0, 0.4),
            (0.8, 0.4),
            (0.7, 0.2),
            (0.6, 0.0),
            (0.5, 0.0),
            (0.4, 0.0),
            (0.3, -0.2),
            (0.2, -0.4),
            (0.0, -0.4),
        ] {
            assert!((freefall_tilt_target(f) - want).abs() < 1e-6, "{f}");
        }
        // k = c = 10 is overdamped (c^2 > 4k): the tilt approaches 0.4 without overshoot
        let mut rig = ChuteRig::default();
        for _ in 0..600 {
            free_step(&mut rig, Vec3::ZERO, (0.0, 0.0), 1.0, DT);
            assert!(rig.tilt <= 0.4 + 1e-5);
        }
        assert!((rig.tilt - 0.4).abs() < 1e-4);
    }

    #[test]
    fn skydive_camera_sits_above_and_behind() {
        let mut rig = ChuteRig::default();
        let pose = free_step(&mut rig, Vec3::ZERO, (0.0, 0.0), 0.5, DT);
        // f = 0.5: no tilt; camera (0, 3, 2.5) from the node, target (0, 0.5, -1)
        assert!((pose.camera - Vec3::new(0.0, 3.0, 2.5)).length() < 1e-5);
        assert!((pose.target - Vec3::new(0.0, 0.5, -1.0)).length() < 1e-5);
    }

    #[test]
    fn retail_chute_cameras_build_from_the_settings() {
        let Some(d) = data() else { return };
        let mut g = GameplayCamera::new(d);
        assert_eq!(g.data.player.id("Parachute"), Some(1));
        assert_eq!(g.data.player.id("SkyDive"), Some(2));
        g.set_camera(1);
        let (id, kind, pl) = g.chute_placement().unwrap();
        assert_eq!((id, kind), (1, ChuteKind::Parachute));
        assert_eq!(pl, &chute_placement());
        g.set_camera(2);
        let (_, kind, pl) = g.chute_placement().unwrap();
        assert_eq!(kind, ChuteKind::Freefall);
        let want = skydive_placement();
        assert_eq!(pl.camera_offset, want.camera_offset);
        assert_eq!(pl.target_offset, want.target_offset);
        assert_eq!(pl.yaw_limits, want.yaw_limits);
        assert_eq!(pl.pitch_limits, want.pitch_limits);
        assert_eq!(pl.pivot, None);
        assert_eq!(pl.pivot_smoothing_rate(), None);
        assert_eq!((pl.target_mode, pl.collision, pl.input_filter), (Some(1), Some(1), Some(1)));
        g.set_camera(0);
        assert!(g.chute_placement().is_none());
    }

    // ---- addendum A-E (board 72909638) ---------------------------------------------------------

    #[test]
    fn pivot_and_offsets_follow_the_full_character_matrix() {
        // addendum A: the pivot (0, 3.75, -0.5) is character-local, so a body pitched 90 deg
        // about X (nose down) carries it with the whole matrix
        let mut rig = ChuteRig::default();
        let rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let f = ChuteFrame {
            node_rot: rot,
            ..ChuteFrame::at(Vec3::new(1.0, 2.0, 3.0), 0.0, (0.0, 0.0), DT)
        };
        let pose = rig.step(ChuteKind::Parachute, &chute_placement(), &f);
        let want = Vec3::new(1.0, 2.0, 3.0) + rot * Vec3::new(0.0, 3.75, -0.5);
        assert!((pose.pivot - want).length() < 1e-5, "{:?} vs {:?}", pose.pivot, want);
    }

    #[test]
    fn parachute_yaw_adds_the_canopy_yaw_rate_unscaled() {
        // addendum B: yaw = w34 + yaw offset + base yaw; the orbit the input system carries
        // does not include w34
        let mut rig = ChuteRig::default();
        let f = ChuteFrame { yaw_rate: 0.3, ..ChuteFrame::at(Vec3::ZERO, 0.0, (0.0, 0.0), DT) };
        let pose = rig.step(ChuteKind::Parachute, &chute_placement(), &f);
        assert!((pose.yaw - 0.3).abs() < 1e-5, "{}", pose.yaw);
        assert!(rig.orbit().0.abs() < 1e-6);
        // and the base yaw is the matrix's own
        let f = ChuteFrame { yaw_rate: 0.3, ..ChuteFrame::at(Vec3::ZERO, 1.0, (1.0, 0.0), DT) };
        let mut rig = ChuteRig::default();
        let pose = rig.step(ChuteKind::Parachute, &chute_placement(), &f);
        assert!((pose.yaw - 1.3).abs() < 1e-5, "{}", pose.yaw);
    }

    #[test]
    fn skydive_target_orbits_with_the_input_and_recentres_when_idle() {
        // addendum C: mode 1 rotates the target by the offsets; 0.1 s delay, 2.0 s duration
        let mut rig = ChuteRig::default();
        free_step(&mut rig, Vec3::ZERO, (0.0, 0.0), 0.5, DT);
        let pose = free_step(&mut rig, Vec3::ZERO, (0.5, 0.0), 0.5, DT);
        let want = Quat::from_rotation_y(0.5) * Vec3::new(0.0, 0.5, -1.0);
        assert!((pose.target - want).length() < 1e-5, "{:?}", pose.target);
        // no mode: the target keeps the node frame
        let mut plain = skydive_placement();
        plain.target_mode = None;
        let mut rig2 = ChuteRig::default();
        let f = ChuteFrame::at(Vec3::ZERO, 0.0, (0.0, 0.0), DT);
        rig2.step(ChuteKind::Freefall, &plain, &f);
        let f = ChuteFrame::at(Vec3::ZERO, 0.0, (0.5, 0.0), DT);
        let pose = rig2.step(ChuteKind::Freefall, &plain, &f);
        assert!((pose.target - Vec3::new(0.0, 0.5, -1.0)).length() < 1e-5);
        // idle: nothing for the first 0.1 s (5 frames of 1/60 s), then the offset decays to 0
        let mut y = rig.yaw_off;
        for i in 0..5 {
            let o = rig.orbit();
            free_step(&mut rig, Vec3::ZERO, o, 0.5, DT);
            assert_eq!(rig.yaw_off, y, "inside the delay, frame {i}");
        }
        for _ in 0..200 {
            let o = rig.orbit();
            free_step(&mut rig, Vec3::ZERO, o, 0.5, DT);
            assert!(rig.yaw_off <= y + 1e-7);
            y = rig.yaw_off;
        }
        assert!(rig.yaw_off.abs() < 1e-3, "{}", rig.yaw_off);
        // any input resets the timer
        rig.idle = 1.0;
        let o = rig.orbit();
        free_step(&mut rig, Vec3::ZERO, (o.0 + 0.01, o.1), 0.5, DT);
        assert_eq!(rig.idle, 0.0);
    }

    fn pose_with_line(len: f32) -> ChutePose {
        ChutePose {
            camera: Vec3::new(0.0, 0.0, len),
            target: Vec3::ZERO,
            pivot: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    #[test]
    fn full_collision_snaps_in_and_eases_out_overdamped() {
        let pose = pose_with_line(8.0);
        let mut rig = ChuteRig::default();
        let (cam, tgt) = rig.collide(Some(ChuteCollision::Full), &pose, 0.25, DT);
        assert!((cam.z - 2.0).abs() < 1e-5, "snap-in {}", cam.z);
        assert_eq!(tgt, Vec3::ZERO);
        // free again: the spring k = c = 10 is overdamped, so it rises without overshoot
        let mut last = 0.25;
        for _ in 0..360 {
            let (cam, _) = rig.collide(Some(ChuteCollision::Full), &pose, 1.0, DT);
            let f = cam.z / 8.0;
            assert!(f >= last - 1e-6 && f <= 1.0 + 1e-6, "{f}");
            last = f;
        }
        assert!(last > 0.99, "{last}");
        // first free frame: v = 10 * 0.75 / 60, f moves by v / 60
        let mut rig = ChuteRig::default();
        rig.collide(Some(ChuteCollision::Full), &pose, 0.25, DT);
        let (cam, _) = rig.collide(Some(ChuteCollision::Full), &pose, 1.0, DT);
        let v = 10.0 * 0.75 * DT;
        assert!((cam.z / 8.0 - (0.25 + v * DT)).abs() < 1e-6);
    }

    #[test]
    fn parachute_collision_fraction_runs_3_8_in_and_0_9_out_with_floor_and_dip() {
        let pose = pose_with_line(8.0);
        let mut rig = ChuteRig::default();
        // closing: 3.8 per second from 1.0 towards 0.7
        rig.collide(Some(ChuteCollision::Parachute), &pose, 0.7, 0.05);
        assert!((rig.coll_f - (1.0 - 3.8 * 0.05)).abs() < 1e-6);
        // never overshoots the hit fraction
        for _ in 0..30 {
            rig.collide(Some(ChuteCollision::Parachute), &pose, 0.7, 0.05);
        }
        assert!((rig.coll_f - 0.7).abs() < 1e-6);
        // pose: camera at f^2 D, target back by (f - f^2) D, both lowered by (1 - f) * 2
        let (cam, tgt) = rig.collide(Some(ChuteCollision::Parachute), &pose, 0.7, 0.05);
        let dip = (1.0 - 0.7) * 2.0;
        assert!((cam.z - 0.49 * 8.0).abs() < 1e-4, "{}", cam.z);
        assert!((tgt.z + (0.7 - 0.49) * 8.0).abs() < 1e-4, "{}", tgt.z);
        assert!((cam.y + dip).abs() < 1e-5 && (tgt.y + dip).abs() < 1e-5);
        assert!(((cam - tgt).length() - 0.7 * 8.0).abs() < 1e-4, "separation f D");
        // opening: 0.9 per second
        rig.collide(Some(ChuteCollision::Parachute), &pose, 1.0, 0.1);
        assert!((rig.coll_f - (0.7 + 0.09)).abs() < 1e-6);
        // the floor 0.5
        let mut rig = ChuteRig::default();
        for _ in 0..60 {
            rig.collide(Some(ChuteCollision::Parachute), &pose, 0.1, 0.1);
        }
        assert_eq!(rig.coll_f, 0.5);
        // no filter: untouched
        let (cam, tgt) = rig.collide(None, &pose, 0.1, DT);
        assert_eq!((cam, tgt), (pose.camera, pose.target));
        assert_eq!(ChuteCollision::from_class(Some(8)), Some(ChuteCollision::Parachute));
        assert_eq!(ChuteCollision::from_class(Some(1)), Some(ChuteCollision::Full));
        assert_eq!(ChuteCollision::from_class(None), None);
    }

    #[test]
    fn input_curves_match_the_native_filters() {
        // Nonlinear (`0074e830`): t = clamp(m^4 + 0.3), out = 1.5 * stick * t
        assert_eq!(nonlinear((0.0, 0.0)), (0.0, 0.0));
        assert!((nonlinear((1.0, 0.0)).0 - 1.5).abs() < 1e-6);
        assert!((nonlinear((0.5, 0.0)).0 - 1.5 * 0.5 * (0.0625 + 0.3)).abs() < 1e-6);
        // NonlinearAccel (`0079b680`): dead zone 0.01, lerp(0.05, 1.2, m^2)
        let mut a = NonlinearAccel::default();
        assert_eq!(a.filter((0.005, 0.0), (1.0, 1.0), DT), (0.0, 0.0));
        let out = a.filter((0.5, 0.0), (1.0, 1.0), DT);
        assert!((out.0 - (0.05 + 1.15 * 0.25)).abs() < 1e-6 && out.1 == 0.0);
        // full deflection held 0.75 s: output blends to 4.0 - 0.75 * scale per axis
        let mut a = NonlinearAccel::default();
        let mut out = (0.0, 0.0);
        for _ in 0..80 {
            out = a.filter((1.0, 0.0), (1.0, 1.0), DT);
        }
        assert!((out.0 - 3.25).abs() < 1e-5, "{}", out.0);
        // a change of direction resets the hold
        a.filter((0.0, 1.0), (1.0, 1.0), DT);
        assert_eq!(a.hold, 0.0);
        // the deltas scale by X * s * out * dt
        let mut rig = ChuteRig::default();
        let d = rig.look_delta(Some(1), (1.0, 0.0), (0.5, 0.5), 0.6, 0.02);
        assert!((d.0 - 0.5 * 0.6 * 1.5 * 0.02).abs() < 1e-7 && d.1 == 0.0);
    }

    /// Time for the run filter to go from `from` to `to` at 60 Hz, and to come within `tol`.
    fn run_time(from: f32, to: f32, tol: f32) -> (f32, f32) {
        let mut f = FootRig { run: from };
        let (mut t, mut near) = (0.0, None);
        while f.run != to && t < 20.0 {
            f.step(to, 1.0 / 60.0);
            t += 1.0 / 60.0;
            if near.is_none() && (f.run - to).abs() <= tol {
                near = Some(t);
            }
        }
        (t, near.unwrap_or(0.0))
    }

    #[test]
    fn run_filter_blend_times_follow_007a17f0() {
        // 0 -> 1.5 (run): 0.5 s at 1 m/s down to 1.0 remaining, then rate = remaining down to
        // 0.2 (ln 5 = 1.61 s), then 0.2 m/s for the last 0.2 m (1 s): 3.1 s in all
        let (t, near) = run_time(0.0, RUN_EXTRA, 0.2);
        assert!((t - 3.11).abs() < 0.05, "{t}");
        assert!((near - 2.11).abs() < 0.05, "{near}");
        // and back to standstill the same way (the model is symmetric)
        let (t, _) = run_time(RUN_EXTRA, 0.0, 0.2);
        assert!((t - 3.11).abs() < 0.05, "{t}");
        // dash: 2.5 m, 1 s more at the full rate
        let (t, _) = run_time(0.0, DASH_EXTRA, 0.2);
        assert!((t - 4.11).abs() < 0.05, "{t}");
        // targets: standing 0, running 1.5, dashing 2.5 (a dash wins over speed)
        assert_eq!(run_target(false, 0.0), 0.0);
        assert_eq!(run_target(false, 10.0), 0.0);
        assert_eq!(run_target(false, 10.1), RUN_EXTRA);
        assert_eq!(run_target(true, 0.0), DASH_EXTRA);
        assert!(is_dash_state("S_DASH") && !is_dash_state("S_UPRIGHT_BASIC_NAVIGATION"));
    }

    #[test]
    fn foot_pose_matches_the_onfoot_record() {
        let Some(d) = data() else { return };
        let pl = &d.settings.get(d.player.id("OnFoot").unwrap()).unwrap().placement;
        assert_eq!(pl.camera_offset, Some([0.55, 1.55, 3.0]));
        assert_eq!(pl.target_offset, Some([0.55, 1.55, 0.0]));
        assert_eq!(pl.target_mode, Some(2));
        assert_eq!(pl.pivot, None);
        assert_eq!(pl.pitch_limits, Some(FOOT_PITCH_LIMITS));
        // looking along -Z at standstill: 3 m behind (+Z), 1.55 up, 0.55 to the right (+X)
        let (cam, target) = foot_pose(pl, Vec3::ZERO, Quat::IDENTITY, (0.0, 0.0), 0.0);
        assert!((cam - Vec3::new(0.55, 1.55, 3.0)).length() < 1e-5, "{cam}");
        assert!((target - Vec3::new(0.55, 1.55, 0.0)).length() < 1e-5, "{target}");
        // running adds the run extra behind
        let (cam, _) = foot_pose(pl, Vec3::ZERO, Quat::IDENTITY, (0.0, 0.0), RUN_EXTRA);
        assert!((cam.z - 4.5).abs() < 1e-5);
        // the target ignores pitch (mode 2, yaw only)
        let (_, t2) = foot_pose(pl, Vec3::ZERO, Quat::IDENTITY, (0.0, 0.5), 0.0);
        assert!((t2 - target).length() < 1e-5);
    }
}
