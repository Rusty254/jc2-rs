//! The on-foot crouch and the evasive roll (JC2-051), the parts around the foot machine:
//! the evade key's press / tap-release rule and how the machine's crouch and evade states
//! move him. The states, edges, clips and blend times come from `basestatemachine.afsm` and
//! `kc001_base.asb` through `foot_machine.rs`; nothing about them is typed here.
//!
//! Native facts used (`research/formats/foot-controller-next.md` sections 3 and 6,
//! `rico-controller-next.md` section 6):
//!
//! * the on-foot input handler (`007b6520`) calls the evade executor (`00a3d500`) when EVADE is
//!   released after a press of at most the controller's `+0x30c` (constructor default 0.25 s,
//!   `00e2c7d0`), or on the press itself with the keyboard/mouse device, or when the layer-0
//!   state is exactly `S_EVADE` (`00507a10` compares the one state id) and the press is younger
//!   than 0.2 s (`01077708`); the executor needs `ACT_EVADE` accepted;
//! * sprint never starts within `+0x30c` seconds of an EVADE press (`005ff2f0`);
//! * the crouch routine (`005d4e60`) dispatches `ACT_CROUCH` every frame the key is down (hold
//!   mode, the default of setting `0x28`); `S_CROUCH`, `S_CROUCH_WALK`, `S_DOWN_TO_CROUCH` and
//!   `S_UP_FROM_CROUCH` turn at rate 5.0 (twice that angular speed, the table at `006c7380`),
//!   `S_EVADE` at pi/2;
//! * what moves him in these states is the playing clip's root motion (the law of
//!   `rico-locomotion-motion.md`, JC2-241): `crouch_DN` and `cr_idle` have none, `crouch_UP` 0.23 m,
//!   `evade_fwd` 5.69 m over its 0.933 s. That the character motion driver (`+0x9c8`) applies
//!   the clip's root motion in these states is **inferred** from the same law.

use crate::rig::{Clip, RigData};

/// The controller's tap threshold (`+0x30c`, constructor default `00e2c7d0`): a press shorter
/// than this, released, is an evade; a sprint never starts this soon after an evade press. The
/// field may be overridden by data (Gap 5 of `foot-controller-next.md`).
pub const EVADE_TAP_SECONDS: f32 = 0.25;
/// A press younger than this, in `S_EVADE`, is retried every frame (`01077708`).
pub const EVADE_QUEUE_SECONDS: f32 = 0.2;
/// Turn rates (rad/s) while the state is the roll or a crouch state: twice the per-state `rate`
/// of the table at `006c7380` (pi/2 and 5.0), because the slerp step is on the half angle.
pub const EVADE_TURN: f32 = std::f32::consts::PI;
pub const CROUCH_TURN: f32 = 10.0;

/// How the foot machine's state moves him on the ground, instead of the stick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootDrive {
    /// Horizontal speed along his facing (m/s), the clip's root motion over the next frame.
    pub speed: f32,
    /// Turn rate (rad/s) towards the stick direction (the "face the desired direction" step of
    /// the locomotion intent, `0063b110`).
    pub turn: f32,
}

/// The ground drive of a foot-machine state: `None` for the states the stick drives.
/// `state_time` is the time since the state was entered, `dt` the length of the frame the speed
/// is for; the speed is the root-motion distance of `[state_time, state_time + dt]` over `dt`,
/// so the total travelled does not depend on the frame rate.
pub fn root_drive(state: &str, state_time: f32, dt: f32, rig: &RigData) -> Option<RootDrive> {
    let over = |clip: Clip, turn: f32| {
        let dist = rig.root_distance(clip, state_time + dt) - rig.root_distance(clip, state_time);
        RootDrive {
            speed: if dt > 0.0 { dist / dt } else { 0.0 },
            turn,
        }
    };
    match state {
        // (both clips are in place: `crouch_DN` has no root motion, `cr_idle` is a 2-frame pose)
        "S_CROUCH" | "S_DOWN_TO_CROUCH" => Some(RootDrive {
            speed: 0.0,
            turn: CROUCH_TURN,
        }),
        "S_UP_FROM_CROUCH" => Some(over(Clip::CrouchUp, CROUCH_TURN)),
        "S_EVADE" => Some(over(Clip::EvadeFwd, EVADE_TURN)),
        // the interrupt state's clip is the same `evade_fwd`, already played out (it is
        // treated as not replaying, like the other `_INTERRUPT` states: Gap 6)
        _ => None,
    }
}

/// The evade key as the on-foot handler reads it.
#[derive(Clone, Debug)]
pub struct EvadeKey {
    down: bool,
    held: f32,
    /// Seconds since the last press (0 on the pressed frame); large until the first press.
    pub since_press: f32,
}

impl Default for EvadeKey {
    fn default() -> Self {
        Self {
            down: false,
            held: 0.0,
            since_press: 1.0e6,
        }
    }
}

impl EvadeKey {
    /// One frame. Returns whether the executor is to be tried on a press or a release (the
    /// queued retry in `S_EVADE` is the caller's: it needs the state). `keyboard` is the input
    /// device (setting `0x10` equal to 0): the keyboard and mouse evade on the press, a pad on
    /// the tap-release only.
    pub fn update(&mut self, dt: f32, down: bool, keyboard: bool) -> bool {
        let pressed = down && !self.down;
        let released = !down && self.down;
        if pressed {
            self.since_press = 0.0;
            self.held = 0.0;
        } else {
            self.since_press += dt;
        }
        let tap = released && self.held <= EVADE_TAP_SECONDS;
        if down {
            self.held += dt;
        }
        self.down = down;
        tap || (keyboard && pressed)
    }

    /// Sprint may start (`005ff2f0`): more than the tap threshold since the last press.
    pub fn sprint_allowed(&self) -> bool {
        self.since_press > EVADE_TAP_SECONDS
    }

    /// The retry window of `S_EVADE`: a press younger than 0.2 s.
    pub fn queued(&self) -> bool {
        self.since_press < EVADE_QUEUE_SECONDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn keyboard_evades_on_the_press_and_the_pad_on_the_tap_release() {
        let mut k = EvadeKey::default();
        assert!(k.update(DT, true, true), "keyboard: the press");
        assert!(!k.update(DT, true, true));
        let mut p = EvadeKey::default();
        assert!(!p.update(DT, true, false), "pad: nothing on the press");
        for _ in 0..10 {
            assert!(!p.update(DT, true, false));
        }
        assert!(p.update(DT, false, false), "pad: a 0.18 s tap fires on the release");
        // held too long: the release is no tap (it was a sprint)
        let mut p = EvadeKey::default();
        p.update(DT, true, false);
        for _ in 0..20 {
            p.update(DT, true, false);
        }
        assert!(!p.update(DT, false, false), "held 0.35 s");
    }

    #[test]
    fn sprint_waits_for_the_tap_threshold_after_a_press() {
        let mut k = EvadeKey::default();
        assert!(k.sprint_allowed());
        k.update(DT, true, true);
        assert!(!k.sprint_allowed());
        for _ in 0..14 {
            k.update(DT, false, true);
        }
        assert!(!k.sprint_allowed(), "0.23 s");
        for _ in 0..3 {
            k.update(DT, false, true);
        }
        assert!(k.sprint_allowed(), "0.28 s");
        assert!(!k.queued());
    }

    /// The roll's total travel is the root motion of `evade_fwd` (5.69 m, 0.933 s), whatever
    /// the frame rate; the rise from a crouch travels 0.23 m; the crouch and the descent stay.
    #[test]
    fn the_drive_follows_the_clips_root_motion() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let Some(rig) = crate::rig::Rig::load(&fs, "biped.bsk") else { return };
        for dt in [1.0 / 30.0, 1.0 / 64.0, 1.0 / 144.0] {
            let evade = (0..)
                .map(|i| i as f32 * dt)
                .take_while(|t| *t < 1.2)
                .map(|t| root_drive("S_EVADE", t, dt, &rig).unwrap().speed * dt)
                .sum::<f32>();
            assert!((evade - 5.69).abs() < 0.05, "evade at {dt}: {evade}");
            let up = (0..)
                .map(|i| i as f32 * dt)
                .take_while(|t| *t < 0.7)
                .map(|t| root_drive("S_UP_FROM_CROUCH", t, dt, &rig).unwrap().speed * dt)
                .sum::<f32>();
            assert!((up - 0.23).abs() < 0.03, "crouch up at {dt}: {up}");
        }
        assert_eq!(root_drive("S_CROUCH", 0.3, DT, &rig).unwrap().speed, 0.0);
        assert_eq!(root_drive("S_DOWN_TO_CROUCH", 0.1, DT, &rig).unwrap().speed, 0.0);
        assert!(root_drive("S_UPRIGHT_IDLE", 0.0, DT, &rig).is_none());
        assert!(root_drive("S_EVADE_INTERRUPT", 0.0, DT, &rig).is_none());
        // the roll peaks near 15.7 m/s (`rico-locomotion-motion.md` section 4.4)
        let peak = (0..60)
            .map(|i| root_drive("S_EVADE", i as f32 / 64.0, 1.0 / 64.0, &rig).unwrap().speed)
            .fold(0.0, f32::max);
        assert!((14.0..17.5).contains(&peak), "peak {peak}");
    }
}
