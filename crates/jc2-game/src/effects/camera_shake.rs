//! The camera-shake consumer: the per-frame waveform behind the accumulators that the force pulse
//! (0x0060f680 -> 0x00698de0) and the camera-shake effect component (0x0062b300 -> 0x0070d460) fill
//! ([`CameraShakeState`]). Research: `research/formats/camera-shake.md`, probe
//! `research/tools/camera_shake_probe.py`.
//!
//! # Native (proven, executable fingerprint in the probe)
//! The camera manager's per-frame update (vtable slot 6 of `CCameraControlManager`, 0x0081c960)
//! builds the final camera matrix (0x0062be20), runs the camera-stack steps and then calls
//! **0x005e7b90** with that matrix and the frame delta (0x004d3690). That function is the only
//! reader of the accumulators, so the shake applies to **every camera mode** (it sits after the
//! mode blend). The matrix is then stored into the camera object (+0x24 and +0x264). 0x005e7b90:
//!
//! * runs only while `intensity` (+0x24) is positive;
//! * keeps a persistent offset `S` and target `T` (three floats each, globals 0x011971d8 and
//!   0x011971cc, both start at zero). When `S == T` on all three axes it draws a new target
//!   `T_i = axis_i * intensity * (rand() * 6.103702e-05 - 1)` (`axis_i` = +0x30/+0x34/+0x38, always
//!   1.0 after a setter), i.e. uniform in `[-intensity, +intensity]`;
//! * moves each axis toward its target at a constant `speed` (+0x28) units per second
//!   (0x004f0a30, clamped so it never overshoots; it does not reset when the shake ends);
//! * rotates the camera matrix by the Euler angles `(S_x, S_y, S_z)` radians (yaw, pitch, roll,
//!   0x004e1470, multipliers 1.0 at 0x0106a740..48) **in the camera's own frame** (`R * M` in the
//!   row-vector convention) and adds `S` **in metres** to the world position;
//! * decays `intensity += decay * dt` (+0x2c) and clears it when it reaches zero or below.
//!
//! # Not native (labelled)
//! * The MSVC `rand()` seed (set elsewhere in the executable, unresolved) - a fixed seed is used.
//! * The rumble accumulator (0x0118fb98 +0x1c, `CRumbleManager`): its reader is not traced
//!   (gap in the research doc); PC keyboard play has no device, so nothing consumes it here.
//! * The native pause flag in 0x004d3690 (dt = 1e-6 while paused) is represented by Bevy's own
//!   paused virtual clock (dt = 0).

use super::pulse::CameraShakeState;
use bevy::prelude::*;

/// `6.103702e-05` at 0x00e2f620: maps `rand()` (0..32767) onto `[0, 2]`.
pub const RAND_SCALE: f32 = 6.103_702e-5;

/// MSVC `rand()` (a 32-bit LCG; the high 15 bits). The seed the executable uses is unknown, so
/// the seed here is arbitrary (*non-native*).
#[derive(Clone, Copy, Debug)]
pub struct MsvcRand(u32);

impl Default for MsvcRand {
    fn default() -> Self {
        Self(1)
    }
}

impl MsvcRand {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.0 >> 16) & 0x7fff
    }
}

const TWO_PI: f32 = 6.283_185_5; // 0x01081d9c
const HALF_PI: f32 = 1.570_796_4; // 0x01081da0
const INV_PI: f32 = 0.318_309_87; // 0x01081da4

/// The executable's sine (inlined in 0x004e1470) for an angle already reduced to `[-pi, pi]`:
/// the parabola `4/pi x - 4/pi^2 x|x|` with the 0.225 correction.
fn parabola_sine(mut x: f32) -> f32 {
    if std::f32::consts::PI - x < 0.0 {
        x -= TWO_PI;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let y = (1.0 - INV_PI * sign * x) * x * (INV_PI * 4.0);
    (y * y * sign - y) * 0.225 + y
}

/// `(sin a, cos a)` as 0x004e1470 computes them: range reduction (truncating division by 2 pi,
/// then a wrap into `[-pi, pi]`), the sine of the reduced angle and the sine of `reduced + pi/2`.
pub fn native_sin_cos(a: f32) -> (f32, f32) {
    let mut x = a - (INV_PI * a * 0.5).trunc() * TWO_PI;
    if std::f32::consts::PI - x < 0.0 {
        x -= TWO_PI;
    }
    if x + std::f32::consts::PI < 0.0 {
        x += TWO_PI;
    }
    (parabola_sine(x), parabola_sine(x + HALF_PI))
}

/// The three basis rows of the rotation matrix 0x004e1470 builds from the angles
/// `(a, b, c)` (row-vector convention; `a` yaw about Y, `b` pitch, `c` roll).
pub fn native_rotation_rows(a: f32, b: f32, c: f32) -> [Vec3; 3] {
    let (sa, ca) = native_sin_cos(a);
    let (sb, cb) = native_sin_cos(b);
    let (sc, cc) = native_sin_cos(c);
    [
        Vec3::new(sc * sb * sa + cc * ca, sc * cb, sc * sb * ca - cc * sa),
        Vec3::new(cc * sb * sa - sc * ca, cc * cb, cc * sb * ca + sc * sa),
        Vec3::new(cb * sa, -sb, cb * ca),
    ]
}

/// 0x004f0a30 with the clamp flag set: move `from` toward `to` by at most `speed * dt` (it
/// returns `to` when the step reaches it).
pub fn approach(from: f32, to: f32, speed: f32, dt: f32) -> f32 {
    if from == to {
        return from;
    }
    let k = ((speed / (from - to).abs()) * dt).clamp(0.0, 1.0);
    if k >= 1.0 {
        return to;
    }
    (to - from) * k + from
}

/// The persistent offset and target of 0x005e7b90 (globals 0x011971d8 / 0x011971cc).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ShakeWave {
    /// Current offset `S`: radians for the three rotation angles, metres for the position.
    pub offset: Vec3,
    pub target: Vec3,
    pub rng: MsvcRand,
}

impl ShakeWave {
    /// One frame of 0x005e7b90 against the accumulators. Returns the offset to apply when the
    /// shake is active (`intensity > 0`), after which the intensity has decayed.
    pub fn step(&mut self, state: &mut CameraShakeState, dt: f32) -> Option<Vec3> {
        if state.intensity <= 0.0 {
            return None;
        }
        if self.offset == self.target {
            // axis multipliers +0x30/+0x34/+0x38 are 1.0 after every setter
            let mut draw = || state.intensity * (self.rng.next() as f32 * RAND_SCALE - 1.0);
            let x = draw();
            let y = draw();
            let z = draw();
            self.target = Vec3::new(x, y, z);
        }
        self.offset = Vec3::new(
            approach(self.offset.x, self.target.x, state.speed, dt),
            approach(self.offset.y, self.target.y, state.speed, dt),
            approach(self.offset.z, self.target.z, state.speed, dt),
        );
        let applied = self.offset;
        let next = state.decay * dt + state.intensity;
        state.intensity = if next <= 0.0 { 0.0 } else { next };
        Some(applied)
    }
}

/// Apply the offset to a camera transform: rotate in the camera's own frame by the Euler angles
/// and add the offset to the world position (0x005e7b90 after 0x0081c960 built the matrix).
pub fn shaken(tf: &Transform, s: Vec3) -> Transform {
    let rows = native_rotation_rows(s.x, s.y, s.z);
    // engine rows are Bevy columns (`coords::mat_to_bevy`): `R * M` becomes `q_cam * q_R`
    let q = Quat::from_mat3(&Mat3::from_cols(rows[0], rows[1], rows[2])).normalize();
    Transform {
        translation: tf.translation + s,
        rotation: (tf.rotation * q).normalize(),
        scale: tf.scale,
    }
}

/// The camera transform before and after the shake of the last frame, so the shake never leaks
/// into the systems that place the camera from its previous transform.
#[derive(Resource, Default)]
pub struct ShakeApplied {
    entity: Option<Entity>,
    base: Transform,
    shaken: Transform,
}

type ShakeCamera = (With<Camera3d>, With<crate::post_effects::PostPass>);

/// First: take last frame's shake back off the camera (only if nothing else moved it since).
pub fn restore_camera(mut applied: ResMut<ShakeApplied>, mut cams: Query<&mut Transform, ShakeCamera>) {
    if let Some(e) = applied.entity.take()
        && let Ok(mut tf) = cams.get_mut(e)
        && *tf == applied.shaken
    {
        *tf = applied.base;
    }
}

/// PostUpdate, before transform propagation: the consumer.
pub fn apply_camera_shake(
    time: Res<Time>,
    mut state: ResMut<CameraShakeState>,
    mut wave: ResMut<ShakeWave>,
    mut applied: ResMut<ShakeApplied>,
    mut cams: Query<(Entity, &mut Transform), ShakeCamera>,
) {
    let Some((entity, mut tf)) = cams.iter_mut().next() else {
        return;
    };
    let Some(s) = wave.step(&mut state, time.delta_secs()) else {
        return;
    };
    let base = *tf;
    let out = shaken(&base, s);
    *tf = out;
    *applied = ShakeApplied { entity: Some(entity), base, shaken: out };
}

/// Registration used by [`super::EffectsPlugin`]. `JC2_NO_CAMERA_SHAKE=1` leaves the consumer out
/// (before captures). Opt-in capture fixture `JC2_SHAKE_FIXTURE=<delay seconds>` (with
/// `JC2_SHAKE_FIXTURE_PATH=<png>`): once, at that time, sets the accumulators to what the retail
/// effect `rumble_and_shake_large` (207) stores (intensity 1 x 0.08, speed 5, decay -0.6), requests a
/// screenshot of that same frame and quits four real seconds later. Non-native, capture aid only.
pub fn add_to(app: &mut App) {
    app.init_resource::<ShakeWave>().init_resource::<ShakeApplied>();
    if std::env::var_os("JC2_NO_CAMERA_SHAKE").is_none() {
        app.add_systems(First, restore_camera).add_systems(
            PostUpdate,
            apply_camera_shake.before(bevy::transform::TransformSystems::Propagate),
        );
    }
    if let (Some(delay), Ok(path)) = (
        std::env::var("JC2_SHAKE_FIXTURE").ok().and_then(|v| v.parse::<f32>().ok()),
        std::env::var("JC2_SHAKE_FIXTURE_PATH"),
    ) {
        app.add_systems(
            PostUpdate,
            (move |time: Res<Time>,
                   real: Res<Time<Real>>,
                   mut state: ResMut<CameraShakeState>,
                   mut commands: Commands,
                   mut exit: MessageWriter<AppExit>,
                   mut requested: Local<Option<f32>>| {
                if requested.is_none() && time.elapsed_secs() >= delay {
                    state.set(1.0 * 0.08, 5.0, -0.6);
                    info!("shake fixture: screenshot with shake intensity 0.08");
                    commands
                        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                        .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
                    *requested = Some(real.elapsed_secs());
                }
                if requested.is_some_and(|r| real.elapsed_secs() - r > 4.0) {
                    exit.write(AppExit::Success);
                }
            })
            .before(apply_camera_shake),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rand_matches_the_msvc_lcg_range() {
        let mut r = MsvcRand::default();
        let v: Vec<u32> = (0..1000).map(|_| r.next()).collect();
        assert!(v.iter().all(|x| *x <= 32767));
        assert_eq!(MsvcRand(1).next(), 41, "first rand() after srand(1) is 41 on MSVC");
        // 32767 maps to 2.0 (the draw spans -1..+1)
        assert!((32767.0 * RAND_SCALE - 2.0).abs() < 1e-4);
    }

    #[test]
    fn parabola_sine_is_the_executables_approximation() {
        for i in -40..=40 {
            let a = i as f32 * 0.15;
            let (s, c) = native_sin_cos(a);
            assert!((s - a.sin()).abs() < 1.2e-3, "sin {a}: {s}");
            assert!((c - a.cos()).abs() < 1.2e-3, "cos {a}: {c}");
        }
    }

    #[test]
    fn rows_are_a_rotation_and_zero_is_identity() {
        let rows = native_rotation_rows(0.0, 0.0, 0.0);
        assert!((rows[0] - Vec3::X).length() < 1e-5 && (rows[1] - Vec3::Y).length() < 1e-5);
        assert!((rows[2] - Vec3::Z).length() < 1e-5);
        let r = native_rotation_rows(0.05, -0.03, 0.04);
        assert!((r[0].length() - 1.0).abs() < 2e-3 && r[0].dot(r[1]).abs() < 2e-3);
        // yaw 0.1 turns the forward row (z) sideways: row 2 = (sin a, 0, cos a)
        let yaw = native_rotation_rows(0.1, 0.0, 0.0);
        assert!((yaw[2].x - 0.1f32.sin()).abs() < 1e-3 && yaw[2].y.abs() < 1e-5);
    }

    #[test]
    fn approach_moves_at_constant_speed_and_never_overshoots() {
        assert_eq!(approach(0.0, 1.0, 1.5, 0.1), 0.15);
        assert_eq!(approach(0.0, 0.1, 1.5, 0.1), 0.1, "reaches the target and stops");
        assert_eq!(approach(0.3, 0.3, 5.0, 1.0), 0.3);
        assert_eq!(approach(0.0, 1.0, 5.0, 0.0), 0.0, "paused clock: no motion");
    }

    #[test]
    fn a_default_pulse_shakes_within_five_centimetres_and_dies_in_half_a_second() {
        let mut state = CameraShakeState::default();
        state.set(0.05, 1.5, -0.1);
        let mut wave = ShakeWave::default();
        let dt = 1.0 / 60.0;
        let mut peak = 0.0f32;
        let mut frames = 0;
        while let Some(s) = wave.step(&mut state, dt) {
            peak = peak.max(s.abs().max_element());
            frames += 1;
            assert!(frames < 200);
        }
        assert!(peak > 0.0 && peak <= 0.05 + 1e-6, "peak {peak}");
        // intensity 0.05 falls 0.1 per second: gone after 0.5 s (30 frames, give or take rounding)
        assert!((29..=31).contains(&frames), "{frames}");
        assert_eq!(state.intensity, 0.0);
        assert!(wave.step(&mut state, dt).is_none());
    }

    #[test]
    fn a_new_target_is_drawn_only_when_every_axis_arrived() {
        let mut state = CameraShakeState::default();
        state.set(1.0, 0.5, 0.0);
        let mut wave = ShakeWave::default();
        wave.step(&mut state, 0.1);
        let first = wave.target;
        wave.step(&mut state, 0.1);
        assert_eq!(wave.target, first, "still travelling");
        for _ in 0..200 {
            wave.step(&mut state, 0.1);
        }
        assert_ne!(wave.target, first, "arrived, so a new target was drawn");
    }

    #[test]
    fn shake_rotates_in_the_camera_frame_and_translates_in_the_world() {
        let cam = Transform::from_xyz(10.0, 5.0, -3.0).looking_to(Vec3::X, Vec3::Y);
        let out = shaken(&cam, Vec3::new(0.0, 0.0, 0.0));
        assert!((out.translation - cam.translation).length() < 1e-6);
        assert!(out.rotation.angle_between(cam.rotation) < 1e-3);
        let out = shaken(&cam, Vec3::new(0.02, 0.0, 0.0));
        assert!((out.translation - cam.translation - Vec3::new(0.02, 0.0, 0.0)).length() < 1e-6);
        // yaw 0.02 turns the view by that angle about the camera's up axis
        let turn = out.rotation.angle_between(cam.rotation);
        assert!((turn - 0.02).abs() < 1e-3, "{turn}");
        let local = cam.rotation.inverse() * out.rotation;
        assert!(local.to_scaled_axis().y.abs() > 0.019, "rotation is about the camera's own Y");
    }

    #[test]
    fn the_shake_is_taken_off_before_the_next_frame() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<CameraShakeState>()
            .init_resource::<ShakeWave>()
            .init_resource::<ShakeApplied>()
            .add_systems(First, restore_camera)
            .add_systems(PostUpdate, apply_camera_shake);
        let base = Transform::from_xyz(1.0, 2.0, 3.0);
        let cam = app
            .world_mut()
            .spawn((Camera3d::default(), crate::post_effects::PostPass::default(), base))
            .id();
        app.world_mut().resource_mut::<CameraShakeState>().set(0.5, 5.0, -0.1);
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_millis(16));
        app.update();
        let moved = *app.world().get::<Transform>(cam).unwrap();
        assert_ne!(moved, base, "shaken after the first frame");
        app.world_mut().resource_mut::<CameraShakeState>().intensity = 0.0;
        app.update();
        assert_eq!(*app.world().get::<Transform>(cam).unwrap(), base, "restored, no shake left");
    }
}
