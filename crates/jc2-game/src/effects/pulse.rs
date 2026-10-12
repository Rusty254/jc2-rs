//! Adapters: `PhysicalForcePulse` (file type 4, `CECD_PhysicalForcePulse` 0x00678de0, Start
//! 0x005fc5f0), `CameraShake` (type 8, 0x006b67e0, Start 0x0062b300) and `HapticsRumble` (type 5,
//! 0x008469c0, Start 0x007992e0). All three act inside `Start` and are never alive (research
//! `effects-pipeline.md` 1.3, `explosion-effects.md` section 2).
//!
//! The components only queue a [`PulseOp`]; [`apply_pulse_ops`] (a Bevy system) evaluates it
//! against the camera and writes the native accumulators [`CameraShakeState`] and
//! [`RumbleState`].
//!
//! # Native (proven in the research doc)
//! * Force pulse fields: `#41874e97` = A, `#4402eb79` = C, `#fb8c4ec3` = half extents. Start
//!   builds a `CForcePulse` at the effect transform and applies it (0x0060f680). The camera
//!   coupling of the apply is [`pulse_shake`] and [`pulse_rumble`] (constants 3.0, 0.5, 0.2,
//!   0.1, 1.5, -0.1, 0.02 and the pulse defaults +0xc4 = 0.5, +0xc8 = 1.0).
//! * Camera shake fields and Start: [`shake_intensity`] (`scale`, `distance_scale`,
//!   `#73440d3e` flag, `#27750b42` gain, `speed`, `#6afb428a` decay; the loader defaults are in
//!   [`ShakeParams::DEFAULT`]).
//! * Setters 0x0070d460 / 0x00698de0: [`CameraShakeState::set`] stores `max(old, new)` intensity
//!   and overwrites speed and decay.
//!
//! # Not implemented (open gaps, nothing guessed)
//! * The shake accumulator is consumed by `camera_shake.rs` (0x005e7b90, task b6762d43). The
//!   rumble accumulator's reader (`CRumbleManager`, 0x0118fb98 +0x1c) is not traced: [`RumbleState`]
//!   is filled but nothing reads it (PC keyboard play has no rumble device).
//! * What the bodies in the pulse volume do with A, B, C (the virtual at body vtable +0x20/+0x24
//!   called from 0x008074e0): gap b73c5209. No impulse is applied; the pulse only counts the
//!   rigid bodies inside its box ([`PulseStats`]). Blast pushes stay with `explosion.rs`.
//! * Haptics: PC build, no rumble device; the four floats are parsed, the start is counted.
//!   Which pair is left / right motor is inferred from the pairing of the loader stores.
//! * The pulse's owner (-1 for effect pulses) and the stimulus notification 0x008e5610 (AI side).

use super::component::{ComponentInit, EffectComponent};
use avian3d::prelude::RigidBody;
use bevy::prelude::*;
use jc2_formats::property::{Node, Variant};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const KEY_PULSE_A: u32 = 0x4187_4e97;
pub const KEY_PULSE_C: u32 = 0x4402_eb79;
pub const KEY_PULSE_EXTENTS: u32 = 0xfb8c_4ec3;

pub const KEY_SHAKE_GAIN: u32 = 0x2775_0b42;
pub const KEY_SHAKE_DECAY: u32 = 0x6afb_428a;
pub const KEY_SHAKE_ATTENUATE: u32 = 0x7344_0d3e;

pub const KEY_HAPTIC_A: u32 = 0x0151_28a1;
pub const KEY_HAPTIC_B: u32 = 0x2635_80f8;
pub const KEY_HAPTIC_C: u32 = 0x3f1d_2ee2;
pub const KEY_HAPTIC_D: u32 = 0xbe0b_5e5f;

/// Camera-shake coupling of a pulse (0x0060f680 step 2): the camera is checked against
/// `3.0 x max(extent)` (constant 0x0106fcf8).
pub const PULSE_SHAKE_RANGE: f32 = 3.0;
/// `CForcePulse` +0xc4 and +0xc8 (0x0062e5b0 defaults, never overwritten by the effect path).
pub const PULSE_C4: f32 = 0.5;
pub const PULSE_C8: f32 = 1.0;
/// The pulse hands `+0xc4 * 0.5` to 0x00698de0, speed 1.5 and decay -0.1.
pub const PULSE_SHAKE_INPUT_SCALE: f32 = 0.5;
pub const PULSE_SHAKE_SPEED: f32 = 1.5;
pub const PULSE_SHAKE_DECAY: f32 = -0.1;
/// 0x00698de0: `(input / max(1, 0.1 * distance)) * 0.2` (constants 0x0106a73c / 0x0106a738).
pub const SHAKE_DISTANCE_SCALE: f32 = 0.1;
pub const SHAKE_GAIN: f32 = 0.2;
/// The rumble manager input: `+0xc8 * +0xb4 * 0.02 / max(1, distance)`.
pub const PULSE_RUMBLE_SCALE: f32 = 0.02;

/// The vehicle death pulse (`CVehicle` slot 78, 0x00559150): half extents 7 (0x00ebeddc),
/// A = B = 300, C = 500. Native.
pub const VEHICLE_DEATH_PULSE: PulseParams = PulseParams { a: 300.0, c: 500.0, extents: Vec3::splat(7.0) };

/// Vehicle death flash keys (0x00559150 via 0x00942ec0): black at 0, (2.0, 1.4, 0.6) at 0.05 s,
/// black at 1.5 s, (c, l, q) = (0.1, 0.008, 0.04). Native.
pub const VEHICLE_FLASH_PEAK: [f32; 3] = [2.0, 1.4, 0.6];
pub const VEHICLE_FLASH_TIMES: [f32; 3] = [0.0, 0.05, 1.5];
pub const VEHICLE_FLASH_CLQ: [f32; 3] = [0.1, 0.008, 0.04];
/// NON-NATIVE: the flash radius is uninitialised stack residue in the original (key word 10,
/// Gap 4 / JC2-210). This is the radius derived from (c, l, q) (9.77 m), the value the key would
/// have if the word were -1.0; replace when the dynamic recipe in `vehicle-fire-explosion.md`
/// section 7 yields the real one.
pub const VEHICLE_FLASH_RADIUS: f32 = 9.77;

/// The three-key flash of a vehicle death as a light track.
pub fn vehicle_death_flash() -> jc2_formats::point_light::LightTrack {
    use jc2_formats::point_light::{LightKey, LightTrack};
    let key = |i: usize, colour: [f32; 3]| LightKey {
        time: VEHICLE_FLASH_TIMES[i],
        colour,
        radius: Some(VEHICLE_FLASH_RADIUS),
        clq: VEHICLE_FLASH_CLQ,
    };
    LightTrack {
        offset: [0.0; 3],
        looping: false,
        priority: 0,
        keys: vec![key(0, [0.0; 3]), key(1, VEHICLE_FLASH_PEAK), key(2, [0.0; 3])],
    }
}

/// Force pulse variation fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulseParams {
    /// `#41874e97`, stored to +0xb4 and +0xb8.
    pub a: f32,
    /// `#4402eb79`, stored to +0xbc.
    pub c: f32,
    /// `#fb8c4ec3`, the half extents of the pulse box.
    pub extents: Vec3,
}

impl PulseParams {
    pub fn from_node(n: &Node) -> Self {
        let f = |k: u32| match n.value_hash(k) {
            Some(Variant::Float(f)) => *f,
            Some(Variant::Int(i)) => *i as f32,
            _ => 0.0,
        };
        Self {
            a: f(KEY_PULSE_A),
            c: f(KEY_PULSE_C),
            extents: match n.value_hash(KEY_PULSE_EXTENTS) {
                Some(Variant::Vec3(v)) => Vec3::from(*v),
                _ => Vec3::ZERO,
            },
        }
    }
}

/// Camera shake variation fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeParams {
    pub speed: f32,
    pub scale: f32,
    /// `distance_scale` (hash 0xf1e162dd): attenuation per metre.
    pub distance_scale: f32,
    /// `#27750b42`.
    pub gain: f32,
    /// `#6afb428a`.
    pub decay: f32,
    /// `#73440d3e`: attenuate by distance.
    pub attenuate: bool,
}

impl ShakeParams {
    /// The loader's stack defaults (0x0052f700) for fields the file leaves out.
    pub const DEFAULT: ShakeParams = ShakeParams {
        speed: 5.0,
        scale: 0.5,
        distance_scale: 0.1,
        gain: 0.2,
        decay: -0.1,
        attenuate: true,
    };

    pub fn from_node(n: &Node) -> Self {
        let d = Self::DEFAULT;
        let hf = |k: u32, d: f32| match n.value_hash(k) {
            Some(Variant::Float(f)) => *f,
            _ => d,
        };
        Self {
            speed: n.float("speed").unwrap_or(d.speed),
            scale: n.float("scale").unwrap_or(d.scale),
            distance_scale: n.float("distance_scale").unwrap_or(d.distance_scale),
            gain: hf(KEY_SHAKE_GAIN, d.gain),
            decay: hf(KEY_SHAKE_DECAY, d.decay),
            attenuate: match n.value_hash(KEY_SHAKE_ATTENUATE) {
                Some(Variant::Int(i)) => *i != 0,
                _ => d.attenuate,
            },
        }
    }
}

/// Haptics variation fields (no device on PC; stored for completeness).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HapticsParams {
    /// `#be0b5e5f` and `#015128a1` always carry the same value in retail data (the larger pair).
    pub pair_a: [f32; 2],
    /// `#263580f8` and `#3f1d2ee2` (the smaller pair).
    pub pair_b: [f32; 2],
}

impl HapticsParams {
    pub fn from_node(n: &Node) -> Self {
        let f = |k: u32| match n.value_hash(k) {
            Some(Variant::Float(f)) => *f,
            _ => 0.0,
        };
        Self {
            pair_a: [f(KEY_HAPTIC_D), f(KEY_HAPTIC_A)],
            pair_b: [f(KEY_HAPTIC_B), f(KEY_HAPTIC_C)],
        }
    }
}

/// Shake intensity of a pulse at `distance` from the camera, or `None` when the camera is not
/// closer than `3.0 x max(extent)` (0x0060f680). Returns `(intensity, speed, decay)`.
pub fn pulse_shake(extents: Vec3, distance: f32) -> Option<(f32, f32, f32)> {
    let reach = PULSE_SHAKE_RANGE * extents.max_element();
    if distance >= reach {
        return None;
    }
    let input = PULSE_C4 * PULSE_SHAKE_INPUT_SCALE;
    let intensity = (input / (SHAKE_DISTANCE_SCALE * distance).max(1.0)) * SHAKE_GAIN;
    Some((intensity, PULSE_SHAKE_SPEED, PULSE_SHAKE_DECAY))
}

/// What a pulse hands the rumble manager (0x008e1280): `c8 * A * 0.02 / max(1, distance)`.
pub fn pulse_rumble(a: f32, distance: f32) -> f32 {
    PULSE_C8 * a * PULSE_RUMBLE_SCALE / distance.max(1.0)
}

/// `CameraShake` Start (0x0062b300): `scale`, divided by `max(1, distance * distance_scale)` when
/// the attenuate flag is set, times the gain.
pub fn shake_intensity(p: &ShakeParams, distance: f32) -> f32 {
    let mut i = p.scale;
    if p.attenuate {
        i /= (distance * p.distance_scale).max(1.0);
    }
    i * p.gain
}

/// The accumulators on the native camera manager (`CCameraControlManager` global 0x011900f8).
/// Written as the setters 0x0070d460 / 0x00698de0 do; read and decayed every frame by
/// `camera_shake::apply_camera_shake` (0x005e7b90). The setters' axis words (+0x30..+0x38) are
/// always 1.0 and are not stored.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraShakeState {
    /// +0x24, max-accumulated.
    pub intensity: f32,
    /// +0x28.
    pub speed: f32,
    /// +0x2c.
    pub decay: f32,
    /// Shakes applied (diagnostics).
    pub applied: u64,
}

impl CameraShakeState {
    pub fn set(&mut self, intensity: f32, speed: f32, decay: f32) {
        self.intensity = self.intensity.max(intensity);
        self.speed = speed;
        self.decay = decay;
        self.applied += 1;
    }
}

/// The last value handed to the rumble manager (0x0118fb98 via 0x008e1280). How the manager
/// accumulates and decays it (+0x1c) is unknown (reader not traced); no device reads it on PC.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct RumbleState {
    pub last_input: f32,
    pub applied: u64,
}

/// Diagnostics of the pulses: bodies found in the pulse boxes (no impulse is applied).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct PulseStats {
    pub pulses: u64,
    pub bodies_in_box: u64,
}

/// A request from one of the three components.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PulseOp {
    Pulse { at: Vec3, params: PulseParams },
    Shake { at: Vec3, params: ShakeParams },
    Haptics { params: HapticsParams },
}

/// The outbox shared by the components and [`apply_pulse_ops`].
#[derive(Default)]
pub struct PulseQueue {
    ops: Mutex<Vec<PulseOp>>,
    pub pulses_started: AtomicU64,
    pub shakes_started: AtomicU64,
    /// Haptics starts: no rumble device on PC, so they are counted only.
    pub haptics_started: AtomicU64,
}

impl PulseQueue {
    /// Queue a code-built pulse (the native code constructs `CForcePulse` directly for explosive
    /// detonations and vehicle death, 0x005569a0 / 0x00559150; `explosion-effects.md` section 4).
    pub fn queue_pulse(&self, at: Vec3, params: PulseParams) {
        self.pulses_started.fetch_add(1, Ordering::Relaxed);
        self.push(PulseOp::Pulse { at, params });
    }
    fn push(&self, op: PulseOp) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(op);
        }
    }
    pub fn drain(&self) -> Vec<PulseOp> {
        self.ops.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
    }
}

/// One component class of the three; they differ only in what they queue.
pub struct OneShot {
    queue: Arc<PulseQueue>,
    at: Vec3,
    enabled: bool,
    op: fn(Vec3, &Node) -> PulseOp,
    node: Node,
}

impl OneShot {
    fn build(init: &ComponentInit, queue: Arc<PulseQueue>, op: fn(Vec3, &Node) -> PulseOp) -> Box<dyn EffectComponent> {
        Box::new(Self {
            queue,
            at: Vec3::ZERO,
            enabled: true,
            op,
            node: init.variation.node.clone(),
        })
    }

    /// Constructor for kind 4.
    pub fn force_pulse(init: &ComponentInit, queue: Arc<PulseQueue>) -> Box<dyn EffectComponent> {
        Self::build(init, queue, |at, n| PulseOp::Pulse { at, params: PulseParams::from_node(n) })
    }
    /// Constructor for kind 8.
    pub fn camera_shake(init: &ComponentInit, queue: Arc<PulseQueue>) -> Box<dyn EffectComponent> {
        Self::build(init, queue, |at, n| PulseOp::Shake { at, params: ShakeParams::from_node(n) })
    }
    /// Constructor for kind 5.
    pub fn haptics(init: &ComponentInit, queue: Arc<PulseQueue>) -> Box<dyn EffectComponent> {
        Self::build(init, queue, |_, n| PulseOp::Haptics { params: HapticsParams::from_node(n) })
    }
}

impl EffectComponent for OneShot {
    fn start(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        let op = (self.op)(self.at, &self.node);
        let counter = match op {
            PulseOp::Pulse { .. } => &self.queue.pulses_started,
            PulseOp::Shake { .. } => &self.queue.shakes_started,
            PulseOp::Haptics { .. } => &self.queue.haptics_started,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.queue.push(op);
        true
    }
    fn stop(&mut self, _immediate: bool) {}
    /// Always false (the one-shot classes, doc 1.3).
    fn is_alive(&self) -> bool {
        false
    }
    fn update(&mut self, _dt: f32) {}
    fn set_transform(&mut self, m: &Mat4) {
        self.at = m.w_axis.truncate();
    }
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// Evaluate the queued ops against the camera. Without a camera (the native manager does not
/// exist) the shake and rumble parts are skipped.
pub fn apply_pulse_ops(
    host: Res<super::EffectHost>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut shake: ResMut<CameraShakeState>,
    mut rumble: ResMut<RumbleState>,
    mut stats: ResMut<PulseStats>,
    bodies: Query<(&RigidBody, &GlobalTransform)>,
) {
    let ops = host.pulses.drain();
    if ops.is_empty() {
        return;
    }
    let cam = camera.iter().next().map(|t| t.translation());
    for op in ops {
        match op {
            PulseOp::Pulse { at, params } => {
                stats.pulses += 1;
                stats.bodies_in_box += bodies
                    .iter()
                    .filter(|(_, t)| {
                        let d = (t.translation() - at).abs();
                        d.x <= params.extents.x && d.y <= params.extents.y && d.z <= params.extents.z
                    })
                    .count() as u64;
                if let Some(cam) = cam {
                    let dist = cam.distance(at);
                    if let Some((i, s, d)) = pulse_shake(params.extents, dist) {
                        shake.set(i, s, d);
                        // inferred: the rumble call sits in the same camera-range branch as the shake
                        rumble.last_input = pulse_rumble(params.a, dist);
                        rumble.applied += 1;
                    }
                }
            }
            PulseOp::Shake { at, params } => {
                if let Some(cam) = cam {
                    shake.set(shake_intensity(&params, cam.distance(at)), params.speed, params.decay);
                }
            }
            PulseOp::Haptics { .. } => {}
        }
    }
}

#[cfg(test)]
mod code_pulse_tests {
    use super::*;

    #[test]
    fn vehicle_death_shakes_inside_21_metres() {
        assert!(pulse_shake(VEHICLE_DEATH_PULSE.extents, 20.9).is_some());
        assert!(pulse_shake(VEHICLE_DEATH_PULSE.extents, 21.1).is_none());
    }

    #[test]
    fn vehicle_flash_has_three_keys_and_expires() {
        let t = vehicle_death_flash();
        assert_eq!(t.keys.len(), 3);
        assert_eq!(t.duration(), 1.5);
        let (c, _) = t.evaluate(0.05).unwrap();
        assert!((c[0] - 2.0).abs() < 1e-4);
        assert!(t.evaluate(1.6).is_none());
    }
}
