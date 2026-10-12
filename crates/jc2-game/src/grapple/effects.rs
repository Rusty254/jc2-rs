//! The grapple's effect calls (board task 7bd60e82; research `research/formats/grapple-tether-visuals.md`
//! sections 1.2, 3.2, 5 and 7), made through the effect spawner (`crate::effects`).
//!
//! Native (read from `JustCause2.exe` and `settings/grapplingparams.bin` block `#ca400f12`):
//! * the device holds eight effect ids: one fire-and-forget one-shot (213 `hook_detach`) and
//!   seven slots with a persistent controller each: 203, 203, 204, -1, 215, 410, 214;
//! * `PlaySlot(i)` (`0x00735700`) plays the controller only if it is not playing, `StopSlot(i)`
//!   (`0x005E9670`) stops it (graceful) only if it is playing;
//! * the hook's hit plays 212 `hook_impact` through the surface-aware path (`0x005F6660`);
//! * Rico's animation event `grapplefire` (0.19 s of `grpl_fire.event_trackb`) plays 11
//!   `flying_hook` on the bone `AttachHandLeft`.
//!
//! Non-native (labelled again where used):
//! * the transform of the slot controllers: no traced native code sets one (doc gap 2, board
//!   task 16c666c0); the port places a slot at the call site's point (the hook end or the cut
//!   end) so a particle or a 3D sound has somewhere to be;
//! * the surface id of a hook hit: the port has no per-collider surface id, so the default
//!   matrix entry (empty interaction list) is used; `FxCall::HookImpact` carries the field;
//! * `grapplefire` plays as a fire-and-forget at the hand's position at that moment (the
//!   native attaches it to the bone), and the clip time is the time since the shot.
//!
//! The call sites push [`FxCall`]s (the rig's own outbox for the rig functions, the
//! [`GrappleFx`] resource for the tether code); [`apply_grapple_fx`] drains them into the
//! spawner. Particles, decals and sounds ride on the adapters of the spawner: until those
//! exist the calls still reach it and are counted there (stub starts).

use super::{GrappleRig, Phase};
use crate::effects::system::{EffectController, EffectSystem, PlayRequest, STATE_DEAD};
use crate::effects::Interaction;
use bevy::prelude::*;

/// `hook_detach` (`0x00000000` field of the device block, key `#6bcc7c5a`).
pub const HOOK_DETACH: u32 = 213;
/// `hook_impact`, the hook projectile's impact id (`weapons.bin`, weapon 52, `#469ba70d`).
pub const HOOK_IMPACT: u32 = 212;
/// `flying_hook`, Rico's `grapplefire` slot (`RicosEffects.bin` slot 7).
pub const FLYING_HOOK: u32 = 11;
/// The `grapplefire` event time inside `grpl_fire.event_trackb`, seconds.
pub const GRAPPLEFIRE_AT: f32 = 0.19;
/// Ids of the seven device slots (struct offsets 0x04..0x1c), -1 = none.
pub const SLOT_IDS: [i32; 7] = [203, 203, 204, -1, 215, 410, 214];

/// A device effect slot (struct offset `4 * (n + 1)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// 203 `char_reelin_sparks`, started by the reel start (`0x008233C0`).
    Reel = 0,
    /// 203, started by the attach impulse step (`0x00740260`).
    Attach = 1,
    /// 204 `char_slingshot_sparks`, the parachute grapple pull (`0x005A9950`).
    Parachute = 2,
    /// 215 `dual_hook_deployed`, played at the tether completion (`0x00538470`).
    Deployed = 4,
    /// 410 `hook_detach_dual`, played at the cut.
    DetachDual = 5,
    /// 214 `hook_miss`, played by the device detach (`0x005A0530`).
    Miss = 6,
}

impl Slot {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One request to the spawner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxCall {
    /// `PlaySlot(slot)`: only if it is not playing. `at` is non-native (doc gap 2).
    PlaySlot { slot: Slot, at: Vec3 },
    /// `StopSlot(slot)`: only if it is playing.
    StopSlot(Slot),
    /// `PlayOneShot(field 0)`: 213 `hook_detach` at a wire end with the end's surface id.
    HookDetach { at: Vec3, surface: Option<u32> },
    /// 212 `hook_impact` where the hook hit; `surface` / `water` pick the variations.
    HookImpact {
        at: Vec3,
        normal: Vec3,
        surface: Option<u32>,
        water: bool,
    },
    /// 11 `flying_hook`: the `grapplefire` animation event.
    FlyingHook { at: Vec3 },
}

/// The pending calls, the history (bounded) and the slot controllers.
#[derive(Resource, Default)]
pub struct GrappleFx {
    pending: Vec<FxCall>,
    /// Every call that reached the spawner (or would have), the newest last; for tests and
    /// diagnostics. Bounded at [`LOG_CAP`].
    pub log: Vec<FxCall>,
    controllers: [Option<EffectController>; 7],
}

/// The history keeps this many calls.
const LOG_CAP: usize = 256;

impl GrappleFx {
    pub fn push(&mut self, call: FxCall) {
        self.pending.push(call);
    }

    pub fn extend(&mut self, calls: impl IntoIterator<Item = FxCall>) {
        self.pending.extend(calls);
    }

    /// The state word of a slot's controller (native `[controller + 0x10]`); `None` when the
    /// slot has none (yet).
    #[cfg(test)]
    pub fn slot_state(&self, fx: &EffectSystem, slot: Slot) -> Option<u8> {
        self.controllers[slot.index()]
            .as_ref()
            .and_then(|c| fx.state(c))
    }

    /// Send the pending calls to the spawner (or only to the history without one).
    pub fn flush(&mut self, mut fx: Option<&mut EffectSystem>) {
        for call in std::mem::take(&mut self.pending) {
            if let Some(fx) = fx.as_mut() {
                self.apply(fx, call);
            }
            self.log.push(call);
        }
        if self.log.len() > LOG_CAP {
            let extra = self.log.len() - LOG_CAP;
            self.log.drain(..extra);
        }
    }

    fn controller(&mut self, fx: &mut EffectSystem, slot: Slot) -> Option<EffectController> {
        let i = slot.index();
        // a controller the spawner freed (its state is gone) is made again
        if self.controllers[i].as_ref().is_some_and(|c| fx.state(c).is_none()) {
            self.controllers[i] = None;
        }
        if self.controllers[i].is_none() && SLOT_IDS[i] >= 0 {
            self.controllers[i] = fx.create(SLOT_IDS[i] as u32);
        }
        self.controllers[i].clone()
    }

    fn apply(&mut self, fx: &mut EffectSystem, call: FxCall) {
        match call {
            FxCall::PlaySlot { slot, at } => {
                let Some(c) = self.controller(fx, slot) else { return };
                // PlaySlot 0x00735700: only if `IsPlaying` (the state word) is 0
                if fx.state(&c).is_some_and(|s| s != STATE_DEAD) {
                    return;
                }
                // NON-NATIVE position (doc gap 2)
                fx.set_transform(&c, Mat4::from_translation(at));
                fx.play_controller(&c, &[], false);
            }
            FxCall::StopSlot(slot) => {
                let Some(c) = self.controllers[slot.index()].clone() else { return };
                // StopSlot 0x005E9670: only if playing
                if fx.state(&c).is_some_and(|s| s != STATE_DEAD) {
                    fx.stop(&c, false);
                }
            }
            FxCall::HookDetach { at, surface } => {
                let mut req = PlayRequest::new(HOOK_DETACH).at_point(at);
                if let Some(s) = surface {
                    req = req.interaction(Interaction::surface(s));
                }
                fx.play(req);
            }
            FxCall::HookImpact {
                at,
                normal,
                surface,
                water,
            } => {
                // the matrix's second row is the surface normal (effects-pipeline, inferred)
                let rot = Quat::from_rotation_arc(Vec3::Y, normal.normalize_or(Vec3::Y));
                let mut req =
                    PlayRequest::new(HOOK_IMPACT).at(Mat4::from_rotation_translation(rot, at));
                if water {
                    req = req.interaction(Interaction::WATER);
                } else if let Some(s) = surface {
                    req = req.interaction(Interaction::surface(s));
                }
                fx.play(req);
            }
            FxCall::FlyingHook { at } => {
                fx.play(PlayRequest::new(FLYING_HOOK).at_point(at));
            }
        }
    }
}

/// The cancel (`0x005FBCB0`): slots 0..2 stop.
pub fn cancel_calls() -> [FxCall; 3] {
    [Slot::Reel, Slot::Attach, Slot::Parachute].map(FxCall::StopSlot)
}

/// The device detach (`0x005A0530`): slot 6 `hook_miss` and the one-shot 213 at the hook.
pub fn detach_calls(at: Vec3) -> [FxCall; 2] {
    [
        FxCall::PlaySlot {
            slot: Slot::Miss,
            at,
        },
        FxCall::HookDetach { at, surface: None },
    ]
}

/// The cut of a two-ended wire (native `0x0083FE20`, doc 3.2): 213 at both ends with each end's
/// surface id, stop 215, play 410.
pub fn cut_calls(a: Vec3, b: Vec3, surfaces: [Option<u32>; 2]) -> [FxCall; 4] {
    [
        FxCall::HookDetach {
            at: a,
            surface: surfaces[0],
        },
        FxCall::HookDetach {
            at: b,
            surface: surfaces[1],
        },
        FxCall::StopSlot(Slot::Deployed),
        FxCall::PlaySlot {
            slot: Slot::DetachDual,
            at: a,
        },
    ]
}

/// The tether completion (`0x00539762`): slot 4 plays.
pub fn tether_complete_call(at: Vec3) -> FxCall {
    FxCall::PlaySlot {
        slot: Slot::Deployed,
        at,
    }
}

impl GrappleRig {
    /// The rig's side of the cancel: stop slots 0..2, then the device detach. Only when the
    /// wire was out (a second cancel does nothing).
    pub(super) fn fx_detach(&mut self) {
        if matches!(self.phase, Phase::Idle | Phase::Retract) {
            return;
        }
        self.fx.extend(cancel_calls());
        self.fx.extend(detach_calls(self.hook));
        self.fire_clock = None;
    }
}

/// Move the rig outboxes into the resource and send everything to the spawner.
pub(super) fn apply_grapple_fx(
    mut state: ResMut<GrappleFx>,
    mut rigs: Query<&mut GrappleRig>,
    fx: Option<ResMut<EffectSystem>>,
) {
    for mut rig in &mut rigs {
        if !rig.fx.is_empty() {
            let calls = std::mem::take(&mut rig.fx);
            state.extend(calls);
        }
    }
    state.flush(fx.map(|f| f.into_inner()));
}
