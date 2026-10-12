//! Rico's two air models, native: the canopy flight model (`0x005a9950`) and the freefall
//! ("skydive") model (`0x006b9290`). Pure maths on `Vec3`s so the headless tests can fly them
//! without a world. Every constant cites its source in
//! `research/formats/parachute-canopy-landing-next.md` (JC2-243/244) and
//! `foot-controller-next.md` section 8; what is inferred is said so at the use.
//!
//! Frames: the world is the native frame (Y up, the character faces -Z at yaw 0; `coords.rs`),
//! so the native matrices are used as they are. A matrix is three row vectors `[R0, R1, R2]`
//! (side, wing normal / up, back; forward is `-R2`), composed as row-vector products.

use bevy::prelude::Vec3;
#[cfg(test)]
use bevy::prelude::Quat;
use std::f32::consts::PI;

/// Native gravity of both air models (`0x00e48bc0`).
pub const G: f32 = 9.82;
/// The force-to-acceleration scale, 1/90 (`0x010776cc`, `0x01077c5c`).
const MASS_SCALE: f32 = 1.0 / 90.0;

pub type Rows = [Vec3; 3];

/// `Rx(a)` (`0x004dbce0`).
pub fn rx(a: f32) -> Rows {
    let (s, c) = a.sin_cos();
    [Vec3::X, Vec3::new(0.0, c, s), Vec3::new(0.0, -s, c)]
}
/// `Ry(a)` (`0x004eaad0`).
pub fn ry(a: f32) -> Rows {
    let (s, c) = a.sin_cos();
    [Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c)]
}
/// `Rz(a)` (`0x004f1790`).
pub fn rz(a: f32) -> Rows {
    let (s, c) = a.sin_cos();
    [Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::Z]
}
/// `out = p1 * p2` with row-vector convention (`0x00491e50`).
pub fn mul(a: &Rows, b: &Rows) -> Rows {
    let row = |r: Vec3| r.x * b[0] + r.y * b[1] + r.z * b[2];
    [row(a[0]), row(a[1]), row(a[2])]
}

/// The Euler extraction `0x004de960` (normal arm; the gimbal arm is dead code, its threshold
/// is the float 9.99999): returns `(pitch a30, yaw a34, bank a38)` with
/// `a30 = asin(clamp(-R2.y))`, `a34 = atan2(R2.x, R2.z)`, `a38 = atan2(R0.y, R1.y)`: the exact
/// inverse of `M = Rz(a38) * Rx(a30) * Ry(a34)`.
pub fn euler_from_rows(m: &Rows) -> (f32, f32, f32) {
    let pitch = (-m[2].y).clamp(-1.0, 1.0).asin();
    (pitch, m[2].x.atan2(m[2].z), m[0].y.atan2(m[1].y))
}

fn wrap_pi(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

fn table(xs: &[f32], ys: &[f32], x: f32, outside: f32) -> f32 {
    if x < xs[0] || x > xs[xs.len() - 1] {
        return outside;
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return ys[i - 1] + (ys[i] - ys[i - 1]) * t;
        }
    }
    outside
}

/// Drag coefficient against the angle (degrees, absolute) of the wind to the back axis
/// (`0x00843900`; x `0x010773ac`, y `0x01077388`); 0 beyond 180.
pub fn canopy_cd(deg: f32) -> f32 {
    table(
        &[0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 90.0, 180.0],
        &[0.032, 0.052, 0.0904, 0.124, 0.194, 0.25, 0.328, 0.8, 1.0],
        deg.abs(),
        0.0,
    )
}

/// Lift coefficient against the angle (degrees) of the wind to the wing plane (`0x0056fca0`;
/// x `0x01076e00`, y `0x01076e2c`); -0.54 outside.
pub fn canopy_cl(deg: f32) -> f32 {
    table(
        &[-8.0, -4.0, 0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 54.0, 80.0, 180.0],
        &[-0.54, -0.2, 0.2, 0.57, 0.92, 1.21, 1.43, 1.4, 1.0, 0.0, 0.0],
        deg,
        -0.54,
    )
}

/// The keys the canopy reads: `pitch` = FORWARD - BACKWARD, `roll` = RIGHT - LEFT (analog,
/// no smoothing; `foot-controller-next.md` section 8).
#[derive(Clone, Copy, Debug, Default)]
pub struct CanopyInput {
    pub pitch: f32,
    pub roll: f32,
    /// `S+4`: the signed yaw of the character against the active camera ([`camera_angle`]).
    pub camera_angle: f32,
    /// Setting 0x26 ("PPI", parachute pitch invert: inferred expansion): swaps FORWARD and
    /// BACKWARD. Code default 0; the shipped option default is unknown (task 6f204aa8).
    pub pitch_invert: bool,
    /// FIRE_GRAPPLE (input id 0x51) is pressed or held (`0x005aa4d4-0x005aa504`): with a wire
    /// attached this switches the hook assist off (the 40 m/s clamp stays).
    pub fire_grapple: bool,
}

/// Camera-steer threshold on `|S+4|` (section 10.5 of the landing doc).
pub const CAMERA_TURN_THRESHOLD: f32 = 0.9;
/// Gain from `S+4` to the chute turn target (`0.46`, section 10.5).
pub const CAMERA_TURN_GAIN: f32 = 0.46;
/// Rate at which `D+0x2c` approaches its target, units per second (`0x010776d0`).
pub const CAMERA_TURN_RATE: f32 = 0.7;

/// `S+4` (producer `0x0077fc80`): the signed planar angle from the character's right axis `h`
/// to the active camera's right axis `c`, positive when the camera is turned towards the
/// character's left (a positive yaw). Both are projected to the XZ plane and normalised;
/// a degenerate vector gives 0 (the native NaN becomes 0). The aim-mode yaw offset (vtable
/// slot 2, `0x005a7310`) is 0 outside the aim modes 1..4 and is not modelled.
pub fn camera_angle(hero_right: Vec3, camera_right: Vec3) -> f32 {
    let (hx, hz) = (hero_right.x, hero_right.z);
    let (cx, cz) = (camera_right.x, camera_right.z);
    let (hl, cl) = (hx.hypot(hz), cx.hypot(cz));
    if hl < 1e-6 || cl < 1e-6 {
        return 0.0;
    }
    let (hx, hz, cx, cz) = (hx / hl, hz / hl, cx / cl, cz / cl);
    let angle = (hx * cx + hz * cz).clamp(-1.0, 1.0).acos();
    let a = if hz * cx - cz * hx >= 0.0 { angle } else { -angle };
    wrap_pi(if a.is_nan() { 0.0 } else { a })
}

/// The free-flight target of `D+0x2c`: `0.46 * S+4` when `|S+4| > 0.9` (and `CPlayer+0x114e`,
/// treated as always set: inferred), else 0.
pub fn camera_turn_target(s4: f32) -> f32 {
    if s4.abs() > CAMERA_TURN_THRESHOLD {
        CAMERA_TURN_GAIN * s4
    } else {
        0.0
    }
}

/// The grapple hook the canopy assists (`0x005a9950`, section 2 of the brief): the world
/// position of the wire end.
#[derive(Clone, Copy, Debug)]
pub struct HookAssist {
    pub target: Vec3,
}

/// Grapple device speed limit (`device+0xfc`, `grapplingparams.bin` key `0x7a4ef581`) and the
/// assist gain (`device+0xf8`, key `0x320b8cd2`), read by the loader `0x00802ec0`.
pub const HOOK_SPEED_LIMIT: f32 = 40.0;
pub const HOOK_GAIN: f32 = 50.0;

/// The flight data block: the three Euler angles and their rates (`D+0x30..0x44`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CanopyFlight {
    pub a30: f32,
    pub a34: f32,
    pub a38: f32,
    w30: f32,
    w34: f32,
    w38: f32,
    /// `D+0x2c`: the chute turn, approaching `0.46 * S+4` (free flight only).
    pub turn: f32,
    /// The animation inputs of the frame (`D+4`, `D+8`, ...; section 10.3 of the landing doc).
    pub pose: CanopyPose,
}

/// The flare offset `D+0x28` the canopy opening writes (`0x0074edd0`, constant `0x01077980`).
pub const FLARE_ON_OPEN: f32 = -0.825;
/// Stick filter coefficient per call (`avatar+0x38`, `0x00e290bc`); the native tick rate is unknown.
const STICK_FILTER: f32 = 0.1;

/// `0x004f0a30(cur, target, rate, dt)`: linear approach, never overshooting.
fn approach(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    if cur == target {
        return cur;
    }
    let t = (rate / (cur - target).abs() * dt).clamp(0.0, 1.0);
    (target - cur) * t + cur
}

/// The animation-side flight data of `0x0071a190` and `0x0050aa00` (`D+4`, `D+8`, `D+0x24`,
/// `D+0x28`, `D+0x48/+0x4c`, `S+0x3c`, `S+0x40`, and the stick filter `S+0x8c/+0x90`).
#[derive(Clone, Copy, Debug)]
pub struct CanopyPose {
    /// Pitch axis `D+4` in [-1, 1]: forward (nose down) positive.
    pub d4: f32,
    /// Bank axis `D+8` in [-1, 1]: a right bank positive.
    pub d8: f32,
    /// Glide ratio `D+0x24`.
    pub d24: f32,
    /// Flare offset `D+0x28`.
    pub d28: f32,
    d48: f32,
    d4c: f32,
    /// `S+0x3c` (bank) and `S+0x40` (pitch): above 0.5 while the angle grows.
    pub s3c: f32,
    pub s40: f32,
    filt: [f32; 2],
    primed: bool,
}

impl Default for CanopyPose {
    fn default() -> Self {
        // `S+0x3c/+0x40` are 0.5 at rest (the hysteresis term vanishes); the constructor
        // value is unknown (inferred), `D+0x28` and the rest are zero-initialised (`0x005a25e0`)
        Self {
            d4: 0.0,
            d8: 0.0,
            d24: 0.0,
            d28: 0.0,
            d48: 0.0,
            d4c: 0.0,
            s3c: 0.5,
            s40: 0.5,
            filt: [0.0; 2],
            primed: false,
        }
    }
}

impl CanopyPose {
    /// One flight frame of `0x0071a190`, after the spring has updated the angles. `stick` =
    /// (LEFT - RIGHT, BACKWARD - FORWARD) as given to the producer; `turn` = `D+0x2c`;
    /// `wire` = a hook wire is attached with FIRE_GRAPPLE not held (the hook assist is active); `thruster` = the thruster object is in state 1 (no thruster ships).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, f: &CanopyFlight, stick: [f32; 2], turn: f32, wire: bool, thruster: bool, dt: f32) {
        // the first-order stick filter (`0x004d6150`): the first sample snaps (`rico-locomotion-motion.md`)
        if self.primed {
            for (v, x) in self.filt.iter_mut().zip(stick) {
                *v = (1.0 - STICK_FILTER) * *v + STICK_FILTER * x;
                if v.abs() < 1e-4 {
                    *v = 0.0;
                }
            }
        } else {
            self.filt = stick;
            self.primed = true;
        }
        let mut d4 = (-0.9 * f.w30 - 0.9 * f.a30).clamp(-1.0, 1.0);
        let mut d8 = (-0.5 * f.a38 - 1.0 * f.w38).clamp(-1.0, 1.0);
        // (D+0x48, D+0x4c) follows the filtered stick at 1.0 units per second (`0x004d6090`)
        let (dx, dy) = (self.d48 - self.filt[0], self.d4c - self.filt[1]);
        let dist = dx.hypot(dy);
        if dist > 0.0 {
            let t = (dt / dist).clamp(0.0, 1.0);
            self.d48 = self.d48 * (1.0 - t) + self.filt[0] * t;
            self.d4c = self.d4c * (1.0 - t) + self.filt[1] * t;
        }
        let (target, rate) = if thruster {
            (0.78, 1.55)
        } else if wire {
            (-0.8, 0.7)
        } else {
            (0.0, 0.7)
        };
        self.d28 = approach(self.d28, target, rate, dt);
        d4 = (self.d4c * ((1.0 - d4.abs()) * -0.7) + d4 + self.d28).clamp(-1.0, 1.0);
        d8 = (turn * -0.5 + self.d48 * ((1.0 - d8.abs()) * -0.7) + d8).clamp(-1.0, 1.0);
        self.s40 = (3.5 * f.w30 * f.a30 + 0.5).clamp(0.0, 1.0);
        self.s3c = (3.5 * f.w38 * f.a38 + 0.5).clamp(0.0, 1.0);
        self.d4 = d4;
        self.d8 = d8;
    }

    /// `0x0050aa00`: `D+0x24 = min(10, horizontal speed / |vy|) * 0.1` of the velocity just
    /// written back (0 when `vy` is 0).
    pub fn set_glide(&mut self, v: Vec3) {
        let ay = v.y.abs();
        let r = if ay > 0.0 { (v.x.hypot(v.z) / ay).max(0.0) } else { 0.0 };
        self.d24 = r.min(10.0) * 0.1;
    }
}

/// One clip of the canopy blend: instance weight (`+0x5c`), time (`+0x28`) and, for the idle
/// clip only, the rate (`+0x60`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipBlend {
    pub weight: f32,
    pub time: f32,
    pub rate: Option<f32>,
}

/// `CParachuteBlendState` update (`0x00778c50`): slots 0..4 = `chute_turn_TB_UP`,
/// `chute_turn_TB_DN`, `chute_speed_TB_UP`, `chute_speed_TB_DN`, `chute_add_idle`.
/// `durations` are the clip lengths in seconds, `idle_time` the idle clip's time.
pub fn blend_update(p: &CanopyPose, durations: [f32; 5], idle_time: f32, dt: f32) -> [ClipBlend; 5] {
    let a = p.d8.abs();
    let w = [p.s3c * a, (1.0 - p.s3c) * a, p.s40 * (1.0 - a), (1.0 - p.s40) * (1.0 - a)];
    let pos_turn = (p.d8 + 1.0) * 0.5;
    let pos_speed = (p.d4 + 1.0) * 0.5;
    let t = [
        durations[0] * pos_turn,
        durations[1] * pos_turn,
        durations[2] * pos_speed,
        durations[3] * pos_speed,
    ];
    let w4 = 0.25 + (1.0 - p.d24) * (0.85 - 0.25);
    let rate4 = 1.5 + p.d24 * (5.0 - 1.5);
    // wrapping: the new time is `time - length` (the pre-dt time), as the native does
    let t4 = if idle_time + dt >= durations[4] { idle_time - durations[4] } else { idle_time };
    let tb = |i: usize| ClipBlend { weight: w[i], time: t[i], rate: None };
    [tb(0), tb(1), tb(2), tb(3), ClipBlend { weight: w4, time: t4, rate: Some(rate4) }]
}

impl CanopyFlight {
    /// The opening frame: angles from the character matrix, rates zero. The character is
    /// upright when the canopy opens here (inferred; the native pose is the unfolding
    /// state's, Gap 2), so only the yaw is taken.
    pub fn open(yaw: f32) -> Self {
        let mut f = Self { a34: yaw, ..Self::default() };
        f.pose.d28 = FLARE_ON_OPEN;
        f
    }

    /// The opening frame from the character matrix (`0x005a9a72` calling the Euler extraction
    /// `0x004de960`, `research/formats/parachute-body-orientation.md` section 3): angles from
    /// `rows`, rates zero. A canopy opened from the dive therefore starts head down.
    /// Whether the matrix of an open from outside the skydive controller is yaw only is
    /// inferred (the on-foot matrix); see `open` for that case.
    pub fn open_from_matrix(rows: &Rows) -> Self {
        let (a30, a34, a38) = euler_from_rows(rows);
        let mut f = Self { a30, a34, a38, ..Self::default() };
        f.pose.d28 = FLARE_ON_OPEN;
        f
    }

    /// `D+0x40`: the yaw rate w34, rad/s. The Parachute camera adds it to its yaw as if it
    /// were an angle (`006b26a0`, research addendum B).
    pub fn yaw_rate(&self) -> f32 {
        self.w34
    }

    /// `M = Rz(a38) * Rx(a30) * Ry(a34)` (`0x005ab3f2-0x005ab421`).
    pub fn matrix(&self) -> Rows {
        mul(&mul(&rz(self.a38), &rx(self.a30)), &ry(self.a34))
    }

    /// One frame: forces on `v`, then the steering spring. `pos` is the character position
    /// (only the hook assist reads it). `wire` = the attached hook wire's end, if any: the
    /// assist runs only while `input.fire_grapple` is clear (`assist = device && !flag &&
    /// device+0x128`, `0x005aa4f0-0x005aa504`), the 40 m/s clamp whenever a wire is attached
    /// (`0x005aaa4e-0x005aaafd`). Returns the new velocity.
    pub fn step(&mut self, mut v: Vec3, input: CanopyInput, wire: Option<HookAssist>, pos: Vec3, dt: f32) -> Vec3 {
        let hook = if input.fire_grapple { None } else { wire };
        let m = self.matrix();
        let (r0, r1, r2) = (m[0], m[1], m[2]);
        let a = forward_axis(&m);
        let va = v + 12.25 * a; // 3.5 squared (`0x010776f4`)
        let s2 = va.length_squared();
        let w = (-v).normalize_or_zero();
        let thl = w.dot(r1).clamp(-1.0, 1.0).asin().to_degrees();
        let thd = w.dot(r2).clamp(-1.0, 1.0).acos().to_degrees();
        let p = w.dot(r0);
        let wp = w - p * r0;
        let fd = wp * s2 * 3.0 * canopy_cd(thd) + r0 * p * s2 * 10.0;
        let ld = ((w.cross(r1)).cross(w)).normalize_or_zero();
        let fl = ld * s2 * 1.7 * canopy_cl(thl);
        let wr1 = w.dot(r1);
        let mut thrust = if wr1 >= 0.7 { wr1.powi(16) * 400.0 } else { 1.0 };
        let mut ft = a * thrust;
        // the hook assist replaces the free-flight thrust (`0x005aa6e1`)
        let mut pitch_target = 0.0;
        // free flight: `D+0x2c` approaches its camera-steered target; the hook assist keeps
        // the previous value (section 10.5)
        if hook.is_none() {
            self.turn = approach(self.turn, camera_turn_target(input.camera_angle), CAMERA_TURN_RATE, dt);
        }
        // setting 0x26 swaps FORWARD and BACKWARD
        let input = CanopyInput { pitch: if input.pitch_invert { -input.pitch } else { input.pitch }, ..input };
        let mut turn = self.turn;
        if let Some(h) = hook {
            let to = h.target - pos;
            let dist = to.length();
            let d = to.normalize_or_zero();
            thrust = HOOK_GAIN * dist;
            ft = d * thrust;
            pitch_target = (1.0 + d.y).clamp(0.0, 1.0);
            let heading = r2.x.atan2(r2.z);
            turn = 1.4 * wrap_pi((-d.x).atan2(-d.z) - heading);
        }
        let mut acc = (fd + fl + ft) * MASS_SCALE;
        acc.y -= G;
        v += acc * dt;
        if wire.is_some() {
            v = v.normalize_or_zero() * v.length().min(HOOK_SPEED_LIMIT);
        }
        // the canopy never flies backwards
        v -= a * a.dot(v).min(0.0);

        // the damped spring on the three angles (statics 0x011fcef8..f00, 0x010776bc, ...)
        let heading = r2.x.atan2(r2.z);
        let e30 = wrap_pi((pitch_target - input.pitch) * 0.65 - self.a30);
        let e34 = wrap_pi((turn - input.roll) * 0.75 + heading - self.a34);
        let e38 = wrap_pi((turn - input.roll) * 0.65 - self.a38);
        let damp30 = 0.2 + (1.8 - 0.2) * self.a30.abs() * (2.0 / PI);
        let damp38 = 0.2 + (1.8 - 0.2) * self.a38.abs() * (2.0 / PI);
        self.w30 *= (-damp30 * dt).exp();
        self.w34 *= (-0.95 * dt).exp();
        self.w38 *= (-damp38 * dt).exp();
        self.w30 += 1.2 * e30 * dt;
        self.w34 += 1.0 * e34 * dt;
        self.w38 += 1.2 * e38 * dt;
        self.a30 += self.w30 * dt;
        self.a34 += self.w34 * dt;
        self.a38 += self.w38 * dt;
        // the animation inputs (`0x0071a190`, then the glide ratio `0x0050aa00`); the hook
        // assist keeps the previous `D+0x2c`
        let mut pose = self.pose;
        pose.update(self, [-input.roll, -input.pitch], self.turn, hook.is_some(), false, dt);
        pose.set_glide(v);
        self.pose = pose;
        v
    }
}

/// `A = unit(-R2)`; falls back to `R1` when `-R2` has no horizontal part (section 8).
fn forward_axis(m: &Rows) -> Vec3 {
    let f = -m[2];
    if f.x * f.x + f.z * f.z < 1e-12 {
        m[1].normalize_or_zero()
    } else {
        f.normalize_or_zero()
    }
}

/// Pitch pose range, roll pose range and the other freefall constants (section 7).
const AREA_BASE: (f32, f32) = (0.8, 1.1);
const AREA_CROSS: (f32, f32) = (0.4, 0.8);
const AIR_INFLUENCE: f32 = 0.645;
const ROLL_CLAMP: f32 = 2.199_114_8;
const CAP_FACTOR: f32 = 0.4;

/// `Ca(P)`, multiplies the lift-like direction `L` (statics `0x01077984..90`).
pub fn skydive_ca(p: f32) -> f32 {
    if p <= 0.25 {
        0.5 + 0.3 * p
    } else if p <= 0.5 {
        0.8 - 0.5 * p
    } else {
        0.3 + 0.1 * p
    }
}
/// `Cb(P)`, multiplies the drag direction `W = -unit(v)` (statics `0x01077994..a0`).
pub fn skydive_cb(p: f32) -> f32 {
    if p <= 0.25 {
        0.6 + 0.3 * p
    } else if p <= 0.5 {
        0.9 + 0.5 * p
    } else {
        1.4 - 0.4 * p
    }
}

/// The base jump's target speed (`f32[0x00e3aa6c]`, stored at `S+0x38` by `0x005952e0`).
pub const BASE_JUMP_SPEED: f32 = 12.0;

/// `CBaseJumpController` update `0x00756ff0` (`parachute-state-velocities.md` section 1):
/// `speed = v0 + (12 - v0) * w`, `v0` the speed at the controller start, `w` the progress of
/// the clip (0 to 1). The lift is `forward * speed`; no gravity.
pub fn base_jump_speed(v0: f32, w: f32) -> f32 {
    v0 + (BASE_JUMP_SPEED - v0) * w.clamp(0.0, 1.0)
}

/// The freefall controller's state: pitch pose `P` and roll pose `R` (with their static
/// targets), the character matrix and the vertical-speed cap `S+0x150`.
#[derive(Clone, Copy, Debug)]
pub struct Skydive {
    pub p: f32,
    p_target: f32,
    pub r: f32,
    r_target: f32,
    pub cap: f32,
    pub matrix: Rows,
}

impl Skydive {
    /// The start routine (`0x0065c9a0`): cap zeroed, pose 0.5; the body keeps its velocity
    /// and takes the character's current yaw.
    pub fn start(yaw: f32) -> Self {
        Self {
            p: 0.5,
            p_target: 0.5,
            r: 0.0,
            r_target: 0.0,
            cap: 0.0,
            matrix: ry(yaw),
        }
    }

    /// The start routine with the character matrix given: the freefall start keeps the
    /// character's current matrix, which after a cut-away is the canopy's `M`, so the dive
    /// begins with the canopy's pitch and bank (the start routine itself writes only P, R and
    /// the cap; board 4d7f7642).
    pub fn start_from_matrix(matrix: Rows) -> Self {
        Self { matrix, ..Self::start(0.0) }
    }

    /// The yaw of the character (as `coords::yaw_toward` of its forward `-R2`).
    pub fn yaw(&self) -> f32 {
        (self.matrix[2].x).atan2(self.matrix[2].z)
    }

    /// One frame. `pitch_in` = FORWARD - BACKWARD in [-1,1] (W positive), `roll_in` = LEFT - RIGHT
    /// (A positive). `aim` is the grapple aim yaw term (0 unless the left arm is in a grapple
    /// state, Gap 4 of the brief).
    pub fn step(&mut self, mut v: Vec3, pitch_in: f32, roll_in: f32, aim: f32, dt: f32) -> Vec3 {
        // poses (`0x00517090`, `0x007687c0`)
        let want_p = ((-pitch_in) + 1.0) * 0.5;
        self.p_target += (want_p - self.p_target).clamp(-1.5 * dt, 1.5 * dt);
        self.p = (self.p + (self.p_target - self.p) * dt * 4.0).clamp(0.0, 1.0);
        let want_r = 2.0 * roll_in;
        self.r_target += (want_r - self.r_target).clamp(-4.0 * dt, 4.0 * dt);
        self.r = (self.r + (self.r_target - self.r) * dt * 3.0).clamp(-1.0, 1.0);

        let c = self.matrix;
        let (r0, r2) = (c[0], c[2]);
        let p = self.p;
        // forces
        if v.length_squared() == 0.0 {
            v = 0.1 * r2;
        }
        let w = -v.normalize();
        let s2 = v.length_squared();
        let area = (AREA_BASE.0 + AREA_BASE.1 * p) * (AREA_CROSS.0 + AREA_CROSS.1 * p);
        let q = area * s2 * AIR_INFLUENCE;
        let u = if r0.dot(w).powi(2) <= r2.dot(w).powi(2) {
            r0.cross(w).normalize_or_zero()
        } else {
            (w.cross(r2)).cross(w).normalize_or_zero()
        };
        let l = -u;
        let f = skydive_ca(p) * q * l + skydive_cb(p) * q * w;
        let mut acc = f * MASS_SCALE;
        acc.y -= G;
        v += acc * dt;
        // the vertical cap (`0x006b9b2b-0x006b9ba6`)
        if v.y < self.cap * 2.5 {
            self.cap = CAP_FACTOR * v.y;
        }
        if v.y >= self.cap {
            v.y = self.cap;
        }

        // orientation (`0x006b9ef8-0x006ba139`)
        let th_y = (self.r + aim).clamp(-ROLL_CLAMP, ROLL_CLAMP) * dt;
        let th_r = (self.r - 1.5 * r0.y) * dt;
        let th_p = ((p - 0.5) * 5.0 + 3.0 * r2.y) * dt;
        self.matrix = mul(&mul(&rx(th_p), &rz(th_r)), &mul(&c, &ry(th_y)));
        v
    }
}

/// The forward ray fan of the cut-chute edges (`CForwardRayCastFanCondition`, `0x00799600`):
/// `n` rays from the planar forward to straight down, ray i = `unit(lerp(forward, down,
/// i/(n-1)))`. `forward` is the character's planar forward `unit(-(R2.x, 0, R2.z))`.
pub fn forward_fan(forward: Vec3, n: usize) -> Vec<Vec3> {
    (0..n)
        .map(|i| {
            let t = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
            (forward + (Vec3::NEG_Y - forward) * t).normalize_or_zero()
        })
        .collect()
}
/// Fan origin offset above the body position (`0x01077c8c`) and ray count (`ba4ef577`).
pub const FAN_ORIGIN_UP: f32 = 0.25;
pub const FAN_RAYS: usize = 10;

#[cfg(test)]
mod tests {
    use super::*;

    const DUR: [f32; 5] = [1.0, 1.0, 1.0, 1.0, 1.333];

    fn pose(d4: f32, d8: f32, d24: f32, s3c: f32, s40: f32) -> CanopyPose {
        CanopyPose { d4, d8, d24, s3c, s40, ..Default::default() }
    }

    /// Probe fixture (`parachute_blend_probe.py`, landing doc 10.2): neutral glide.
    #[test]
    fn blend_neutral_glide_fixture() {
        let b = blend_update(&pose(0.0, 0.0, 0.1, 0.5, 0.5), DUR, 0.0, 1.0 / 60.0);
        let w: Vec<f32> = b.iter().map(|c| c.weight).collect();
        assert!((w[0]).abs() < 1e-6 && (w[1]).abs() < 1e-6 && (w[2] - 0.5).abs() < 1e-6 && (w[3] - 0.5).abs() < 1e-6);
        assert!((b[0].time - 0.5).abs() < 1e-6 && (b[3].time - 0.5).abs() < 1e-6);
        assert!((w[4] - 0.79).abs() < 1e-5, "{}", w[4]);
        assert!((b[4].rate.unwrap() - 1.85).abs() < 1e-5);
    }

    /// The base-jump law (`0x00756ff0`): the entry speed at progress 0, 12 m/s at progress 1.
    #[test]
    fn base_jump_speed_ramps_to_twelve() {
        assert_eq!(base_jump_speed(4.0, 0.0), 4.0);
        assert!((base_jump_speed(4.0, 0.5) - 8.0).abs() < 1e-6);
        assert_eq!(base_jump_speed(4.0, 1.0), BASE_JUMP_SPEED);
        // faster than 12 at entry: it slows to 12
        assert!((base_jump_speed(20.0, 1.0) - 12.0).abs() < 1e-6);
        assert_eq!(base_jump_speed(4.0, 3.0), BASE_JUMP_SPEED);
    }

    /// The freefall start from a matrix keeps the matrix (the canopy's pitch and bank after a
    /// cut-away) and resets only P, R and the cap.
    #[test]
    fn skydive_start_from_matrix_keeps_the_pitch() {
        let m = mul(&rx(0.4), &ry(1.0));
        let sd = Skydive::start_from_matrix(m);
        assert_eq!(sd.matrix, m);
        assert_eq!((sd.p, sd.r, sd.cap), (0.5, 0.0, 0.0));
        let plain = Skydive::start(1.0);
        assert_eq!(plain.matrix, ry(1.0));
        assert!((sd.yaw() - plain.yaw()).abs() < 0.1);
    }

    /// Probe fixture: full bank, `D+4 = 1`, `S+0x3c = 1`, `D+0x24 = 1`.
    #[test]
    fn blend_full_bank_fixture() {
        let b = blend_update(&pose(1.0, -1.0, 1.0, 1.0, 0.5), DUR, 0.0, 1.0 / 60.0);
        assert_eq!((b[0].weight, b[0].time), (1.0, 0.0));
        assert_eq!(b[1].weight, 0.0);
        assert!(b[2].weight.abs() < 1e-6 && b[3].weight.abs() < 1e-6);
        assert_eq!((b[2].time, b[3].time), (1.0, 1.0));
        assert!((b[4].weight - 0.25).abs() < 1e-6 && (b[4].rate.unwrap() - 5.0).abs() < 1e-6);
    }

    /// The idle time wraps to `time - length` (pre-dt time) when `time + dt >= length`.
    #[test]
    fn blend_idle_wrap_uses_pre_dt_time() {
        let b = blend_update(&pose(0.0, 0.0, 0.1, 0.5, 0.5), DUR, 1.32, 0.02);
        assert!((b[4].time - (1.32 - 1.333)).abs() < 1e-5);
        let b = blend_update(&pose(0.0, 0.0, 0.1, 0.5, 0.5), DUR, 1.0, 0.02);
        assert_eq!(b[4].time, 1.0);
    }

    /// Weights of the four scrubbed clips always sum to one.
    #[test]
    fn blend_weights_sum_to_one() {
        for (d8, s3c, s40) in [(0.3, 0.2, 0.9), (-0.8, 0.7, 0.1), (0.0, 0.5, 0.5)] {
            let b = blend_update(&pose(0.2, d8, 0.4, s3c, s40), DUR, 0.0, 0.0);
            let sum: f32 = b[..4].iter().map(|c| c.weight).sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    /// Opening sets the flare offset -0.825; it decays linearly at 0.7/s to 0 (71 frames at 60 fps).
    #[test]
    fn flare_offset_decays_after_opening() {
        let mut c = CanopyFlight::open(0.0);
        assert_eq!(c.pose.d28, FLARE_ON_OPEN);
        let mut v = Vec3::new(0.0, -5.0, -8.0);
        let mut frames = 0;
        while c.pose.d28 != 0.0 && frames < 200 {
            v = c.step(v, CanopyInput::default(), None, Vec3::ZERO, 1.0 / 60.0);
            frames += 1;
        }
        assert!((70..=72).contains(&frames), "{frames}");
    }

    /// With a wire attached the flare target is -0.8 (not held fire).
    #[test]
    fn wire_pulls_flare_to_minus_point_eight() {
        let mut p = CanopyPose::default();
        let f = CanopyFlight::default();
        for _ in 0..200 {
            p.update(&f, [0.0; 2], 0.0, true, false, 1.0 / 60.0);
        }
        assert!((p.d28 + 0.8).abs() < 1e-6);
    }

    /// Hand-derived steady states of the physical terms (landing doc 10.3): a30 = 0.65 with no
    /// rate gives `D+4 = -0.585`; a38 = -0.65 gives `D+8 = 0.325`.
    #[test]
    fn pose_axes_follow_physical_angles() {
        let f = CanopyFlight { a30: 0.65, a38: -0.65, ..Default::default() };
        let mut p = CanopyPose::default();
        p.update(&f, [0.0; 2], 0.0, false, false, 1.0 / 60.0);
        assert!((p.d4 + 0.585).abs() < 1e-5, "{}", p.d4);
        assert!((p.d8 - 0.325).abs() < 1e-5, "{}", p.d8);
        assert!((p.s3c - 0.5).abs() < 1e-6 && (p.s40 - 0.5).abs() < 1e-6);
    }

    /// Glide ratio: neutral glide (9.60, -1.69) gives 0.568, the full dive (45.1, -33.0) 0.137.
    #[test]
    fn glide_ratio_fixtures() {
        let mut p = CanopyPose::default();
        p.set_glide(Vec3::new(9.60, -1.69, 0.0));
        assert!((p.d24 - 0.568).abs() < 1e-3);
        p.set_glide(Vec3::new(45.1, -33.0, 0.0));
        assert!((p.d24 - 0.137).abs() < 1e-3);
        p.set_glide(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(p.d24, 0.0);
    }

    /// Back stick feeds `D+4` negative (nose up), left stick feeds `D+8` negative, through the
    /// filtered stick feed-forward.
    #[test]
    fn stick_feed_forward_signs() {
        let mut c = CanopyFlight::open(0.0);
        c.pose.d28 = 0.0;
        let mut v = Vec3::new(0.0, -5.0, -8.0);
        for _ in 0..120 {
            v = c.step(v, CanopyInput { pitch: -1.0, roll: -1.0, ..CanopyInput::default() }, None, Vec3::ZERO, 1.0 / 60.0);
        }
        assert!(c.pose.d4 < -0.3, "{}", c.pose.d4);
        assert!(c.pose.d8 < -0.05, "{}", c.pose.d8);
    }

    #[test]
    fn camera_angle_signs_and_wrap() {
        let x = Vec3::X;
        assert!(camera_angle(x, x).abs() < 1e-6);
        // camera right rotated by +0.5 rad about Y (towards the left / positive yaw)
        let c = Quat::from_rotation_y(0.5) * x;
        assert!((camera_angle(x, c) - 0.5).abs() < 1e-5);
        let c = Quat::from_rotation_y(-1.2) * x;
        assert!((camera_angle(x, c) + 1.2).abs() < 1e-5);
        assert!((camera_angle(x, -x).abs() - PI).abs() < 1e-4);
        assert_eq!(camera_angle(x, Vec3::Y), 0.0);
        // height is ignored
        let c = Quat::from_rotation_y(0.7) * x + Vec3::Y * 3.0;
        assert!((camera_angle(x, c) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn camera_turn_target_threshold() {
        assert_eq!(camera_turn_target(0.9), 0.0);
        assert_eq!(camera_turn_target(-0.5), 0.0);
        assert!((camera_turn_target(1.0) - 0.46).abs() < 1e-6);
        assert!((camera_turn_target(-PI) + 0.46 * PI).abs() < 1e-5);
    }

    /// The turn approaches the target at 0.7/s without overshoot, the hook assist freezes it,
    /// and a camera turned to the left yaws the canopy towards it.
    #[test]
    fn camera_steer_turns_canopy() {
        let mut c = CanopyFlight::open(0.0);
        let mut v = Vec3::new(0.0, -5.0, -8.0);
        let inp = CanopyInput { camera_angle: 2.0, ..CanopyInput::default() };
        v = c.step(v, inp, None, Vec3::ZERO, 0.5);
        assert!((c.turn - 0.35).abs() < 1e-5, "{}", c.turn);
        for _ in 0..120 {
            v = c.step(v, inp, None, Vec3::ZERO, 1.0 / 60.0);
        }
        assert!((c.turn - 0.92).abs() < 1e-5, "{}", c.turn);
        assert!(c.a34 > 0.1, "yaw towards positive camera angle: {}", c.a34);
        let held = c.turn;
        let hook = HookAssist { target: Vec3::new(0.0, -50.0, -50.0) };
        c.step(v, CanopyInput::default(), Some(hook), Vec3::ZERO, 0.1);
        assert_eq!(c.turn, held);
        c.step(v, CanopyInput::default(), None, Vec3::ZERO, 0.1);
        assert!(c.turn < held);
    }

    /// Held FIRE_GRAPPLE (id 0x51) with a wire attached: no assist (free-flight thrust, turn
    /// follows the camera, no hook pull) but the 40 m/s clamp stays (`0x005aa4f0`, `0x005aaa4e`).
    #[test]
    fn fire_grapple_disables_assist_keeps_clamp() {
        let hook = HookAssist { target: Vec3::new(0.0, -50.0, -50.0) };
        let (mut free, mut held, mut assisted) =
            (CanopyFlight::open(0.0), CanopyFlight::open(0.0), CanopyFlight::open(0.0));
        let v0 = Vec3::new(0.0, -5.0, -8.0);
        let fire = CanopyInput { fire_grapple: true, camera_angle: 2.0, ..CanopyInput::default() };
        let plain = CanopyInput { camera_angle: 2.0, ..CanopyInput::default() };
        let vf = free.step(v0, plain, None, Vec3::ZERO, 0.1);
        let vh = held.step(v0, fire, Some(hook), Vec3::ZERO, 0.1);
        assisted.step(v0, plain, Some(hook), Vec3::ZERO, 0.1);
        assert_eq!(vf, vh);
        assert_eq!(free.turn, held.turn);
        assert_eq!(free.pose.d28, held.pose.d28);
        assert_ne!(assisted.turn, held.turn);
        // the clamp is wire-gated only
        let mut c = CanopyFlight::open(0.0);
        let v = c.step(Vec3::new(0.0, -90.0, -80.0), fire, Some(hook), Vec3::ZERO, 0.01);
        assert!(v.length() <= HOOK_SPEED_LIMIT + 1e-3, "{}", v.length());
    }

    #[test]
    fn pitch_invert_swaps_forward_back() {
        let (a, _) = settle(CanopyInput { pitch: 1.0, pitch_invert: true, ..CanopyInput::default() }, 60.0, 90.0);
        let (b, _) = settle(CanopyInput { pitch: -1.0, ..CanopyInput::default() }, 60.0, 90.0);
        assert!((a.a30 - b.a30).abs() < 1e-4);
    }

    /// Fly the canopy with held inputs until it settles (60 fps unless said).
    fn settle(input: CanopyInput, fps: f32, seconds: f32) -> (CanopyFlight, Vec3) {
        let dt = 1.0 / fps;
        let mut c = CanopyFlight::open(0.0);
        let mut v = Vec3::new(0.0, -5.0, -8.0);
        for _ in 0..(seconds * fps) as usize {
            v = c.step(v, input, None, Vec3::ZERO, dt);
        }
        (c, v)
    }

    /// Neutral glide: 9.60 m/s forward, 1.69 m/s sink (brief section 5), at both frame rates.
    #[test]
    fn neutral_glide_is_native() {
        for fps in [60.0, 30.0] {
            let (_, v) = settle(CanopyInput::default(), fps, 60.0);
            assert!((-v.z - 9.60).abs() < 0.05, "{fps}: forward {}", -v.z);
            assert!((-v.y - 1.69).abs() < 0.03, "{fps}: sink {}", -v.y);
        }
    }

    /// Full back (nose up) and full forward (dive): the other two fixed points.
    #[test]
    fn pitch_fixed_points() {
        let (c, v) = settle(CanopyInput { pitch: -1.0, roll: 0.0, ..CanopyInput::default() }, 60.0, 90.0);
        assert!((c.a30 - 0.65).abs() < 0.01, "a30 {}", c.a30);
        assert!((-v.z - 7.24).abs() < 0.1 && (-v.y - 9.52).abs() < 0.1, "back {v:?}");
        let (c, v) = settle(CanopyInput { pitch: 1.0, roll: 0.0, ..CanopyInput::default() }, 60.0, 90.0);
        assert!((c.a30 + 0.65).abs() < 0.01, "a30 {}", c.a30);
        assert!((-v.z - 45.1).abs() < 0.5 && (-v.y - 33.0).abs() < 0.5, "forward {v:?}");
    }

    /// Full side input: the yaw rate settles near 0.75 / 0.95 = 0.789 rad/s with 0.65 rad of
    /// bank (right = negative yaw natively).
    #[test]
    fn steering_rate_and_bank() {
        let (mut c, mut v) = settle(CanopyInput { pitch: 0.0, roll: 1.0, ..CanopyInput::default() }, 60.0, 30.0);
        let a = c.a34;
        for _ in 0..60 {
            v = c.step(v, CanopyInput { pitch: 0.0, roll: 1.0, ..CanopyInput::default() }, None, Vec3::ZERO, 1.0 / 60.0);
        }
        let rate = (c.a34 - a).abs();
        assert!((rate - 0.789).abs() < 0.02, "yaw rate {rate}");
        assert!((c.a38.abs() - 0.65).abs() < 0.03, "bank {}", c.a38);
        assert!(c.a34 < a, "right input must turn right (negative yaw)");
    }

    /// The hook assist fixture: char (0,100,0), hook (30,60,40): the pitch target and the clamp.
    #[test]
    fn hook_assist_clamps_to_forty() {
        let mut c = CanopyFlight::open(0.0);
        let mut v = Vec3::new(0.0, -5.0, -8.0);
        let hook = HookAssist { target: Vec3::new(30.0, 60.0, 40.0) };
        for _ in 0..600 {
            v = c.step(v, CanopyInput::default(), Some(hook), Vec3::new(0.0, 100.0, 0.0), 1.0 / 60.0);
            assert!(v.length() <= HOOK_SPEED_LIMIT + 1e-3, "{}", v.length());
        }
        assert!(v.length() > 30.0, "the assist must pull: {}", v.length());
    }

    /// Terminal velocities of the freefall for the five pitch poses (brief section 7).
    #[test]
    fn skydive_terminal_velocities() {
        // (pitch input, horizontal, vertical). The curves are discontinuous at P = 0.25 and
        // 0.5, so the settled pose P = 0.25 (pitch 0.5) approached from above is not the
        // brief's value; P = 0.2 and 0.3 are checked against an independent replica instead.
        let cases = [
            (1.0, 47.40, -56.88),
            (0.6, 34.06, -40.14),
            (0.4, 20.62, -33.30),
            (0.0, 13.61, -28.46),
            (-0.5, 8.69, -25.50),
            (-1.0, 8.77, -21.93),
        ];
        for (pitch, h, vy) in cases {
            let mut s = Skydive::start(0.0);
            let mut v = Vec3::new(0.0, -10.0, -5.0);
            for _ in 0..(120 * 60) {
                v = s.step(v, pitch, 0.0, 0.0, 1.0 / 60.0);
            }
            let hs = v.x.hypot(v.z);
            assert!((hs - h).abs() < 1.0 && (v.y - vy).abs() < 1.0, "pitch {pitch}: {hs} / {} (want {h} / {vy})", v.y);
        }
    }

    #[test]
    fn coefficient_curves() {
        assert!((skydive_cb(0.0) - 0.6).abs() < 1e-6 && (skydive_cb(0.5) - 1.15).abs() < 1e-6);
        assert!((skydive_ca(0.0) - 0.5).abs() < 1e-6 && (skydive_ca(1.0) - 0.4).abs() < 1e-6);
        assert!((canopy_cd(8.0) - 0.0904).abs() < 1e-6 && canopy_cd(200.0) == 0.0);
        assert!((canopy_cl(0.0) - 0.2).abs() < 1e-6 && canopy_cl(-20.0) == -0.54);
    }

    #[test]
    fn fan_runs_from_forward_to_down() {
        let rays = forward_fan(Vec3::NEG_Z, FAN_RAYS);
        assert_eq!(rays.len(), 10);
        assert!((rays[0] - Vec3::NEG_Z).length() < 1e-6);
        assert!((rays[9] - Vec3::NEG_Y).length() < 1e-6);
        assert!(rays.iter().all(|r| (r.length() - 1.0).abs() < 1e-5));
    }

    /// `flight_matrix -> euler -> flight_matrix` (`0x004de960`, probe: 1,050 triples).
    #[test]
    fn euler_round_trips_the_flight_matrix() {
        for &a30 in &[-1.2f32, -0.985, -0.3, 0.0, 0.4, 1.1] {
            for &a34 in &[-3.0f32, -1.0, 0.0, 0.7, 2.5] {
                for &a38 in &[-1.0f32, -0.2, 0.0, 0.65, 1.3] {
                    let f = CanopyFlight { a30, a34, a38, ..Default::default() };
                    let (p, y, b) = euler_from_rows(&f.matrix());
                    assert!((p - a30).abs() < 2e-5 && (y - a34).abs() < 2e-5 && (b - a38).abs() < 2e-5, "{a30} {a34} {a38} -> {p} {y} {b}");
                }
            }
        }
    }

    /// Probe fixture: the terminal dive (`P = 0`, `R2.y = 5/6`) gives a30 = -0.98511 (56.44
    /// degrees head down), bank 0; the opening takes it with the rates at zero.
    #[test]
    fn canopy_opens_head_down_from_the_dive_matrix() {
        // R2 = back axis = (0, -sin a30, cos a30): pitched head down means R2.y > 0
        let a30 = -(5.0f32 / 6.0).asin();
        let rows = rx(a30);
        assert!((rows[2].y - 5.0 / 6.0).abs() < 1e-6);
        let f = CanopyFlight::open_from_matrix(&rows);
        assert!((f.a30 + 0.98511).abs() < 1e-4, "{}", f.a30);
        assert!(f.a34.abs() < 1e-6 && f.a38.abs() < 1e-6);
        assert_eq!((f.w30, f.w34, f.w38), (0.0, 0.0, 0.0));
        // a level body with R0.y = 2/3 (the steady state of a full roll): bank 41.81 degrees
        let r1y = (1.0f32 - 4.0 / 9.0).sqrt();
        let (_, _, b) = euler_from_rows(&[Vec3::new(1.0, 2.0 / 3.0, 0.0), Vec3::new(0.0, r1y, 0.0), Vec3::Z]);
        assert!((b.to_degrees() - 41.81).abs() < 0.01, "{}", b.to_degrees());
    }
}
