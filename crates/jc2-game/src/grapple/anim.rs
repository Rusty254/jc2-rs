//! The grapple's animation graph: which action the device starts (reel or pull), which
//! states Rico's body (layer 0) and left arm (layer 2) pass through, and which clip each
//! one plays (JC2-250; research `grapple-action-selection-next.md` and
//! `animation-statemachines-next.md`).
//!
//! Nothing here touches Bevy resources: the machine is plain data so a headless test can
//! drive it and print the trace of every state it enters. The runtime (`grapple.rs`) posts
//! the same actions the original posts (`ACT_REEL`, `LA_ACT_HOOK_HIT`, ...) and
//! `character.rs` plays the clip of the state the machine is in.
//!
//! **Native**: the selector `0x007f3db0`, the arrival classification `0x00549490`, the
//! graph edges of `basestatemachine.afsm` and `leftarmstatemachine.afsm`, the state to clip
//! binding of `kc001_base.asb` / `kc001_leftarm.asb` (clip, blend time, playback speed),
//! the NPC `CGrappledCondition` bands. **Inferred / unknown** items are marked `Gap` in
//! the doc comments and collected in `research/formats/grapple-animation-runtime.md`.

use super::class::{GrappleClass, GrappleClassification};
use crate::rig::Clip;

// ---------------------------------------------------------------- constants

/// `#00992cfb` of `grapplingparams.bin` (`0x00802ec0` loads `device+0xf4 = cos` of it):
/// a hit normal this close to straight up is a floor (JC2-169).
pub const FLOOR_ANGLE_DEG: f32 = 50.0;
/// `#8e43b31e` (`device+0xf0`): a normal beyond this from straight up is a ceiling.
pub const CEILING_ANGLE_DEG: f32 = 140.0;
/// `CPullGrappledAction` constructor default `+0x38` (`0x0070f830`): in the air the pull
/// ends when the wire is this much (m) shorter than at the start (`0x00510a60`). Whether
/// the data overrides it is not recorded (the property loader `0x0089f290` does not read it).
pub const PULL_WIRE_SHORTENING: f32 = 6.0;
/// `0x010803fc`: the S_REEL_START blend input is the signed angle to the hook over this.
pub const REEL_START_ANGLE: f32 = 2.4435;
/// `CGrappledCondition` bands of `npc_combat_base.afsm` (`689d08f9` / `d7f292c6`).
pub const GRAPPLED_ATLEAST_ONCE: (u32, u32) = (1, 42);

/// **Inferred (Gap)**: Rico on foot after `S_REEL_END_LAND` leaves the state once he runs
/// this fast (m/s); the native exit of the state has no graph edge (it is internal).
pub const LAND_ENDS_AT_SPEED: f32 = 2.0;

/// `cos 50 deg`: at or above is a floor.
pub fn cos_floor() -> f32 {
    FLOOR_ANGLE_DEG.to_radians().cos()
}

/// `cos 140 deg`: at or below is a ceiling.
pub fn cos_ceiling() -> f32 {
    CEILING_ANGLE_DEG.to_radians().cos()
}

// ---------------------------------------------------------------- the action set

/// The six actions of Rico's `CGrapplingActionSet`, in the order `0x00842080` creates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetAction {
    FireHook = 0,
    ReelIn = 1,
    PullGrappled = 2,
    DeployParachute = 3,
    ReeledInJump = 4,
    ReeledInRelease = 5,
}

/// What the hook hit, as the selector sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookTarget {
    /// No game object: terrain, static geometry or a `CBreakableTreePatch`.
    Terrain,
    /// A `CCharacter`.
    Character,
    /// Any other game object, with its authored `grappling_weight_category` (7 bit).
    Object(u8),
}

/// The selector's input for a body of the game (`class.rs`' classification, `None` = no
/// game object). Authored categories pass through (light 1, heavy 2). **Gaps** (JC2-168
/// Gap 1): the category of a vehicle is not located, so a vehicle counts as heavy (2); a
/// body with no usable category keeps the mass fallback of `class.rs` (non-native), light
/// = 1, else 2. Native would do nothing for category 0 and 3..127.
pub fn hook_target(c: Option<GrappleClassification>) -> HookTarget {
    match c {
        None => HookTarget::Terrain,
        Some(c) => match c.class {
            GrappleClass::Static => HookTarget::Terrain,
            GrappleClass::Character => HookTarget::Character,
            GrappleClass::LightProp => HookTarget::Object(1),
            GrappleClass::HeavyProp | GrappleClass::Vehicle => HookTarget::Object(2),
            GrappleClass::Unclassified => HookTarget::Object(if c.pull { 1 } else { 2 }),
        },
    }
}

/// What the device chose to do at attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Reel,
    Pull,
    /// Only FireHook stays enabled: the hook holds, nothing moves.
    Neither,
    /// A second hook (dual hook): Reel, Pull and the parachute are switched off.
    Dual,
}

/// The enable flags (`CGrapplingActionSet`, bytes `+0x48+i`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionSet {
    pub enabled: [bool; 6],
}

impl Default for ActionSet {
    /// `0x005beed0` enables only index 0 (FireHook).
    fn default() -> Self {
        let mut enabled = [false; 6];
        enabled[SetAction::FireHook as usize] = true;
        Self { enabled }
    }
}

impl ActionSet {
    pub fn is(&self, a: SetAction) -> bool {
        self.enabled[a as usize]
    }

    fn set(&mut self, a: SetAction, on: bool) {
        self.enabled[a as usize] = on;
    }

    /// `0x007f3db0`, called once from the hook-hit handler `0x00538470` before the wire
    /// exists. `wire_exists` is `device+0x128 != 0` (a second hook). Mass is never compared.
    pub fn select(&mut self, target: HookTarget, wire_exists: bool) -> Choice {
        self.set(SetAction::FireHook, true);
        if wire_exists {
            self.set(SetAction::PullGrappled, false);
            self.set(SetAction::ReelIn, false);
            self.set(SetAction::DeployParachute, false);
            return Choice::Dual;
        }
        match target {
            HookTarget::Terrain => self.reel(),
            HookTarget::Character => self.pull(),
            HookTarget::Object(1) => self.pull(),
            HookTarget::Object(2) => self.reel(),
            HookTarget::Object(_) => Choice::Neither,
        }
    }

    fn reel(&mut self) -> Choice {
        self.set(SetAction::ReelIn, true);
        Choice::Reel
    }

    fn pull(&mut self) -> Choice {
        self.set(SetAction::PullGrappled, true);
        Choice::Pull
    }

    /// `0x005a0530` (device detach) clears Pull, Reel and Deploy; `0x005fbcb0` (cancel)
    /// restores the initial state.
    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    /// Reel update case 0 (`0x006b8890`): once in the flight state, parachute while reeling.
    fn flying(&mut self) {
        self.set(SetAction::DeployParachute, true);
    }

    /// The arrival dispatchers (section 5 of the brief).
    fn arrived(&mut self, class: ArrivalClass) {
        self.set(SetAction::DeployParachute, false);
        self.set(SetAction::ReelIn, false);
        match class {
            // case 3 clears Deploy, Jump and Release
            ArrivalClass::Floor => {
                self.set(SetAction::ReeledInJump, false);
                self.set(SetAction::ReeledInRelease, false);
            }
            // `0x00564b20`: Release and Jump on
            ArrivalClass::Wall => {
                self.set(SetAction::ReeledInRelease, true);
                self.set(SetAction::ReeledInJump, true);
            }
            // `0x005e6b90`: Release on
            ArrivalClass::Ceiling => self.set(SetAction::ReeledInRelease, true),
        }
    }
}

// ---------------------------------------------------------------- arrival

/// How the reel ended, by the end surface's normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrivalClass {
    /// `normal.y >= cos 50`: `ACT_REEL_LAND`.
    Floor,
    /// `cos 140 < y < cos 50`: `ACT_REEL_IMPACT_VERTICAL`.
    Wall,
    /// `y <= cos 140`: `ACT_REEL_IMPACT_HORIZONTAL`.
    Ceiling,
}

/// `0x00549490`: classify the hit normal's Y (the first wire end; characters give +Y).
pub fn classify_arrival(normal_y: f32) -> ArrivalClass {
    if normal_y >= cos_floor() {
        ArrivalClass::Floor
    } else if normal_y <= cos_ceiling() {
        ArrivalClass::Ceiling
    } else {
        ArrivalClass::Wall
    }
}

/// `CReelInAction::start` (`0x008098c0`) in `S_REEL_START`: the blend input
/// `controller+0x3c = clamp(signedAngle / 140 deg, -1, 1)`. `signed_angle` is the angle
/// from his facing to the direction to the hook (radians; sign from the cross product's y).
///
/// Not consumed yet (**Gap**): `reel_in_start` is one clip and how `CJumpControllerState`
/// uses the input (a directional blend of other takes?) is not decoded.
#[allow(dead_code)]
pub fn reel_start_blend(signed_angle: f32) -> f32 {
    (signed_angle / REEL_START_ANGLE).clamp(-1.0, 1.0)
}

// ---------------------------------------------------------------- layer 0: the body

/// What Rico's body is doing when an action reaches layer 0 (his non-grapple states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyCtx {
    /// Standing, walking, running, landing: `ACT_REEL` -> `S_REEL_START`.
    Ground,
    /// `S_FALL`, a jump in flight, the skydive: `ACT_REEL` -> `S_REEL_FLIGHT`.
    Air,
    /// Swimming (`T_REEL_SWIMMING`): `ACT_REEL` -> `S_REEL_FLIGHT`; pull accepted from the dive.
    Swim,
    /// A stunt position on a vehicle roof: reel accepted, pull not.
    Stunt,
}

/// The states of `basestatemachine.afsm` that the grapple enters. `Free` is every other
/// state: the locomotion code decides the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BodyState {
    #[default]
    Free,
    /// `reel_in_start`, then `S_REEL_FLIGHT`.
    ReelStart,
    /// `grpl_reel_flight_part2` (`CVelocityTimeVariableState`).
    ReelFlight,
    /// Same clip as the reel flight; the only exit is `ACT_FALL`.
    PullInAirFlight,
    ReelEndLand,
    ReelImpactVertical,
    ReeledInIdle,
    ReeledInJump,
    ReeledInReleaseDrop,
    ReelImpactHorizontal,
    ReeledInToHang,
    HangStuntIdle,
    HangToFall,
}

/// Actions posted to layer 0 (`0x0073f7f0(id, 0)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyAction {
    Reel,
    GrapplePull,
    ReelLand,
    ReelImpactVertical,
    ReelImpactHorizontal,
    Detach,
    Fall,
    ReeledInJump,
    ReeledInRelease,
}

impl BodyAction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Reel => "ACT_REEL",
            Self::GrapplePull => "ACT_GRAPPLE_PULL",
            Self::ReelLand => "ACT_REEL_LAND",
            Self::ReelImpactVertical => "ACT_REEL_IMPACT_VERTICAL",
            Self::ReelImpactHorizontal => "ACT_REEL_IMPACT_HORIZONTAL",
            Self::Detach => "ACT_DETACH",
            Self::Fall => "ACT_FALL",
            Self::ReeledInJump => "ACT_REELED_IN_JUMP",
            Self::ReeledInRelease => "ACT_REELED_IN_RELEASE",
        }
    }
}

impl BodyState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Free => "(free)",
            Self::ReelStart => "S_REEL_START",
            Self::ReelFlight => "S_REEL_FLIGHT",
            Self::PullInAirFlight => "S_PULL_IN_AIR_FLIGHT",
            Self::ReelEndLand => "S_REEL_END_LAND",
            Self::ReelImpactVertical => "S_REEL_IMPACT_VERTICAL",
            Self::ReeledInIdle => "S_REELED_IN_IDLE",
            Self::ReeledInJump => "S_REELED_IN_JUMP",
            Self::ReeledInReleaseDrop => "S_REELED_IN_RELEASE_DROP",
            Self::ReelImpactHorizontal => "S_REEL_IMPACT_HORIZONTAL",
            Self::ReeledInToHang => "S_REELED_IN_TO_HANG",
            Self::HangStuntIdle => "S_HANGSTUNT_IDLE",
            Self::HangToFall => "S_HANG_TO_FALL",
        }
    }

    /// `CanReelInState` (`stateruleflags` Flag15, bit `0x8000`, tested per layer by
    /// `0x00575950`), read from `basestatemachine.afsm` by `grapple_reel_flight_probe.py`
    /// (JC2-246 section 5). `Free` stands for every state this module does not model;
    /// their flag is **inferred true** (the upright idle/move/fall/jump/swim states carry it;
    /// dash, climb, ladder and hard landing do not, and are not modelled here).
    pub fn can_reel_in_state(self) -> bool {
        !matches!(
            self,
            Self::ReelImpactHorizontal | Self::ReeledInToHang | Self::HangStuntIdle | Self::HangToFall
        )
    }

    /// The clip of `kc001_base.asb` bound to the state (`None`: not the grapple's).
    pub fn clip(self) -> Option<Clip> {
        Some(match self {
            Self::Free => return None,
            Self::ReelStart => Clip::ReelStart,
            Self::ReelFlight | Self::PullInAirFlight => Clip::Reel,
            Self::ReelEndLand => Clip::ReelEndLand,
            Self::ReelImpactVertical => Clip::ReelImpactVertical,
            Self::ReeledInIdle => Clip::ReeledInIdle,
            Self::ReeledInJump => Clip::GrappleClimb,
            Self::ReeledInReleaseDrop => Clip::ReeledInReleaseDrop,
            Self::ReelImpactHorizontal => Clip::ReelImpactHorizontal,
            Self::ReeledInToHang => Clip::ReeledInToHang,
            Self::HangStuntIdle => Clip::HangStuntIdle,
            Self::HangToFall => Clip::HangToFall,
        })
    }

    /// The `.asb` blend-in (s) from the previous state (`57ab7197` read as the blend time,
    /// the same reading `character.rs` already uses for fall 0.3 / skydive 0.4).
    pub fn blend(self) -> f32 {
        match self {
            Self::Free => 0.0,
            Self::ReelStart => 0.1,
            Self::ReelFlight | Self::PullInAirFlight => 0.2,
            Self::ReelEndLand => 0.4,
            Self::ReelImpactVertical | Self::ReelImpactHorizontal => 0.07,
            Self::ReeledInIdle
            | Self::ReeledInJump
            | Self::ReeledInReleaseDrop
            | Self::ReeledInToHang
            | Self::HangToFall => 0.0,
            Self::HangStuntIdle => 0.2,
        }
    }

    /// `playback_speed` of the `.asb` row (only `S_REELED_IN_TO_HANG` differs from 1).
    pub fn speed(self) -> f32 {
        if self == Self::ReeledInToHang {
            0.9
        } else {
            1.0
        }
    }

    /// A `CVelocityTimeVariableState` / time-blend state: the clip is a pose picked by a
    /// parameter, not played forward (the existing reel flight treatment). **Gap**: the
    /// parameter (velocity? sway?) and its mapping to clip time are undecoded; the pose is
    /// held at the middle of the clip as `character.rs` already did for the reel flight.
    pub fn held_pose(self) -> bool {
        matches!(
            self,
            Self::ReelFlight | Self::PullInAirFlight | Self::ReeledInIdle | Self::HangStuntIdle
        )
    }

    /// States that end with their clip, and where they go.
    fn on_finish(self) -> Option<BodyState> {
        match self {
            Self::ReelStart => Some(Self::ReelFlight),
            // `S_REEL_IMPACT_VERTICAL` -> `_INTERRUPT` (`CSetStartTimeState`, same clip) ->
            // `S_REELED_IN_IDLE`: the interrupt state is collapsed (Gap: its start time).
            Self::ReelImpactVertical => Some(Self::ReeledInIdle),
            Self::ReelImpactHorizontal => Some(Self::ReeledInToHang),
            Self::ReeledInToHang => Some(Self::HangStuntIdle),
            // `S_REELED_IN_JUMP` -> `S_FALL` (`C_WAIT_FOR_ANIM_TO_FINISH`)
            Self::ReeledInJump => Some(Self::Free),
            // no native exit is listed for these three (Gap): the clip's end hands back
            // to the locomotion states.
            Self::ReelEndLand | Self::ReeledInReleaseDrop | Self::HangToFall => Some(Self::Free),
            _ => None,
        }
    }

    /// The wire-held states: they make no sense without the wire.
    pub fn needs_wire(self) -> bool {
        matches!(
            self,
            Self::ReelImpactVertical
                | Self::ReeledInIdle
                | Self::ReelImpactHorizontal
                | Self::ReeledInToHang
                | Self::HangStuntIdle
        )
    }
}

/// `00810360(id, 0)`: would layer 0 accept the action now (and into which state)?
pub fn body_accepts(state: BodyState, ctx: BodyCtx, a: BodyAction) -> Option<BodyState> {
    use BodyAction as A;
    use BodyState as S;
    match (state, a) {
        // reel: from ground states `S_REEL_START`, from the airborne, swim and stunt ones
        // (and from the hang-to-fall) `S_REEL_FLIGHT`
        (S::Free, A::Reel) => Some(if ctx == BodyCtx::Ground {
            S::ReelStart
        } else {
            S::ReelFlight
        }),
        (S::HangToFall, A::Reel) => Some(S::ReelFlight),
        // the pull is offered from S_FALL, S_JUMP_FLIGHT_STANDING, S_JUMP_TO_SKYDIVE,
        // S_SKYDIVE, S_SWIM_DIVE_IDLE, S_BLEND_TO_SWIM_IDLE, S_REEL_FLIGHT only
        (S::Free, A::GrapplePull) if matches!(ctx, BodyCtx::Air | BodyCtx::Swim) => {
            Some(S::PullInAirFlight)
        }
        (S::ReelFlight, A::GrapplePull) => Some(S::PullInAirFlight),
        (S::ReelFlight, A::ReelLand) => Some(S::ReelEndLand),
        // `S_FALL` also offers the vertical impact (index 11)
        (S::ReelFlight, A::ReelImpactVertical) => Some(S::ReelImpactVertical),
        (S::Free, A::ReelImpactVertical) if ctx == BodyCtx::Air => Some(S::ReelImpactVertical),
        (S::ReelFlight, A::ReelImpactHorizontal) => Some(S::ReelImpactHorizontal),
        (S::ReelFlight | S::ReelStart, A::Detach) => Some(S::Free),
        (S::PullInAirFlight, A::Fall) => Some(S::Free),
        (S::ReeledInIdle, A::ReeledInJump) => Some(S::ReeledInJump),
        (S::ReeledInIdle, A::ReeledInRelease) => Some(S::ReeledInReleaseDrop),
        (S::HangStuntIdle, A::ReeledInRelease) => Some(S::HangToFall),
        _ => None,
    }
}

// ---------------------------------------------------------------- layer 2: the left arm

/// States of `leftarmstatemachine.afsm` the hook passes through (`LA_S_IDLE` also stands
/// for the wielding and aiming states of a gun: the gun code owns those).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ArmState {
    #[default]
    Idle,
    RaiseGrapple,
    WaitingForHook,
    HookHit,
    Grapple,
    GrapplePull,
    GrappleDetach,
    HookMissed,
    GrappleToIdle,
    DualGrapple,
}

/// Actions posted to layer 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmAction {
    FireGrapple,
    HookHit,
    HookMissed,
    Reel,
    GrapplePull,
    Detach,
    DualGrapple,
    /// `LA_ACT_IDLE`, posted by the air pull start.
    Idle,
}

impl ArmAction {
    pub fn name(self) -> &'static str {
        match self {
            Self::FireGrapple => "LA_ACT_FIRE_GRAPPLE",
            Self::HookHit => "LA_ACT_HOOK_HIT",
            Self::HookMissed => "LA_ACT_HOOK_MISSED",
            Self::Reel => "LA_ACT_REEL",
            Self::GrapplePull => "LA_ACT_GRAPPLE_PULL",
            Self::Detach => "LA_ACT_DETACH",
            Self::DualGrapple => "LA_ACT_DUAL_GRAPPLE",
            Self::Idle => "LA_ACT_IDLE",
        }
    }
}

impl ArmState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "LA_S_IDLE",
            Self::RaiseGrapple => "LA_S_RAISE_GRAPPLE",
            Self::WaitingForHook => "LA_S_WAITING_FOR_HOOK",
            Self::HookHit => "LA_S_HOOK_HIT",
            Self::Grapple => "LA_S_GRAPPLE",
            Self::GrapplePull => "LA_S_GRAPPLE_PULL",
            Self::GrappleDetach => "LA_S_GRAPPLE_DETACH",
            Self::HookMissed => "LA_S_HOOK_MISSED",
            Self::GrappleToIdle => "LA_S_GRAPPLE_TO_IDLE",
            Self::DualGrapple => "LA_S_DUAL_GRAPPLE",
        }
    }

    /// The clip of `kc001_leftarm.asb` (`None`: the arm is Rico's normal one).
    pub fn clip(self) -> Option<Clip> {
        match self {
            Self::Idle => None,
            Self::RaiseGrapple => Some(Clip::GrappleFire),
            Self::WaitingForHook | Self::HookHit | Self::Grapple => Some(Clip::GrappleAim),
            Self::GrapplePull => Some(Clip::GrapplePull),
            Self::GrappleDetach | Self::HookMissed | Self::GrappleToIdle => {
                Some(Clip::GrappleRecoil)
            }
            Self::DualGrapple => Some(Clip::GrappleDual),
        }
    }

    /// The `.asb` blend time (s): raise 0.2, aim 0, pull 0.14, recoil 0.2, dual 0.07.
    pub fn blend(self) -> f32 {
        match self {
            Self::Idle | Self::WaitingForHook | Self::HookHit | Self::Grapple => 0.0,
            Self::RaiseGrapple | Self::GrappleDetach | Self::HookMissed | Self::GrappleToIdle => {
                0.2
            }
            Self::GrapplePull => 0.14,
            Self::DualGrapple => 0.07,
        }
    }

    /// Plays once and returns to idle (`C_WAIT_FOR_LEFTARM_TO_FINISH`).
    fn on_finish(self) -> Option<ArmState> {
        match self {
            Self::RaiseGrapple => Some(Self::WaitingForHook),
            Self::HookHit => Some(Self::Grapple),
            Self::GrapplePull
            | Self::GrappleDetach
            | Self::HookMissed
            | Self::GrappleToIdle
            | Self::DualGrapple => Some(Self::Idle),
            Self::Idle | Self::WaitingForHook | Self::Grapple => None,
        }
    }
}

/// `leftarmstatemachine.afsm` edges for the actions the hook posts.
pub fn arm_accepts(state: ArmState, a: ArmAction) -> Option<ArmState> {
    use ArmAction as A;
    use ArmState as S;
    match (state, a) {
        (S::Idle, A::FireGrapple) => Some(S::RaiseGrapple),
        (S::RaiseGrapple | S::WaitingForHook, A::HookHit) => Some(S::HookHit),
        (S::RaiseGrapple | S::WaitingForHook, A::HookMissed) => Some(S::HookMissed),
        (S::RaiseGrapple | S::WaitingForHook | S::HookHit, A::Detach) => Some(S::GrappleDetach),
        (S::HookHit, A::Reel) => Some(S::Grapple),
        (S::HookHit | S::Grapple, A::GrapplePull) => Some(S::GrapplePull),
        (S::HookHit | S::Grapple, A::DualGrapple) => Some(S::DualGrapple),
        (S::Grapple, A::Detach) => Some(S::GrappleToIdle),
        (S::Grapple, A::Idle) => Some(S::Idle),
        _ => None,
    }
}

// ---------------------------------------------------------------- the machine

/// One recorded state change (for headless traces).
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub t: f32,
    pub layer: u8,
    pub action: &'static str,
    pub accepted: bool,
    pub from: &'static str,
    pub to: &'static str,
    pub clip: Option<Clip>,
}

impl Step {
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn line(&self) -> String {
        let clip = self.clip.map_or("-", |c| c.file());
        if self.accepted {
            format!(
                "{:6.2}s L{} {:<28} {} -> {}  [{}]",
                self.t, self.layer, self.action, self.from, self.to, clip
            )
        } else {
            format!(
                "{:6.2}s L{} {:<28} rejected in {}",
                self.t, self.layer, self.action, self.from
            )
        }
    }
}

/// The air pull's bookkeeping (`CPullGrappledAction`, sub-state 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirPull {
    /// The wire length (m) at which the pull ends: the start length minus
    /// [`PULL_WIRE_SHORTENING`].
    pub end_length: f32,
}

/// How a hit played out in the machine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attach {
    pub choice: Choice,
    /// A pull while Rico was in the air: he flies at the object too.
    pub air_pull: bool,
}

/// Rico's grapple animation state: the action set and both layers.
#[derive(Clone, Debug, Default)]
pub struct GrappleAnim {
    pub actions: ActionSet,
    pub body: BodyState,
    pub arm: ArmState,
    /// Seconds in the current body / arm state.
    pub body_time: f32,
    pub arm_time: f32,
    /// Seconds since the machine started (for traces).
    pub clock: f32,
    pub air_pull: Option<AirPull>,
    /// Every post and change, kept only when `record` is set.
    pub record: bool,
    pub steps: Vec<Step>,
    /// Bumped each time the arm enters `RaiseGrapple` (the shot animation restarts).
    pub shots: u32,
}

impl GrappleAnim {
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn recording() -> Self {
        Self {
            record: true,
            ..Self::default()
        }
    }

    fn log(&mut self, layer: u8, action: &'static str, accepted: bool, from: &'static str) {
        if !self.record {
            return;
        }
        let (to, clip) = if layer == 0 {
            (self.body.name(), self.body.clip())
        } else {
            (self.arm.name(), self.arm.clip())
        };
        self.steps.push(Step {
            t: self.clock,
            layer,
            action,
            accepted,
            from,
            to,
            clip,
        });
    }

    fn note(&mut self, layer: u8, what: &'static str, from: &'static str) {
        self.log(layer, what, true, from);
    }

    /// `0x0073f7f0(id, 0)`: post an action to the body layer.
    pub fn post_body(&mut self, a: BodyAction, ctx: BodyCtx) -> bool {
        let from = self.body.name();
        match body_accepts(self.body, ctx, a) {
            Some(next) => {
                self.set_body(next);
                self.log(0, a.name(), true, from);
                true
            }
            None => {
                self.log(0, a.name(), false, from);
                false
            }
        }
    }

    /// `00810360(id, 0)`: would the body layer accept it?
    pub fn body_would_accept(&self, a: BodyAction, ctx: BodyCtx) -> bool {
        body_accepts(self.body, ctx, a).is_some()
    }

    /// `0x0073f7f0(id, 2)`: post an action to the left arm.
    pub fn post_arm(&mut self, a: ArmAction) -> bool {
        let from = self.arm.name();
        match arm_accepts(self.arm, a) {
            Some(next) => {
                self.set_arm(next);
                self.log(2, a.name(), true, from);
                true
            }
            None => {
                self.log(2, a.name(), false, from);
                false
            }
        }
    }

    fn set_body(&mut self, next: BodyState) {
        if next != self.body {
            self.body = next;
            self.body_time = 0.0;
        }
    }

    fn set_arm(&mut self, next: ArmState) {
        if next != self.arm {
            self.arm = next;
            self.arm_time = 0.0;
            if next == ArmState::RaiseGrapple {
                self.shots += 1;
            }
        }
    }

    /// The shot: `CFireHookAction::update` posts `LA_ACT_FIRE_GRAPPLE`.
    pub fn fire(&mut self) -> bool {
        self.post_arm(ArmAction::FireGrapple)
    }

    /// The flight ended without a hit (`0x0083fe20`).
    pub fn miss(&mut self) {
        self.post_arm(ArmAction::HookMissed);
    }

    /// The hook hit (`0x00538470`): the selector runs, the arm is told, then the chosen
    /// action starts (forced triggers start in the same frame).
    pub fn attach(&mut self, target: HookTarget, wire_exists: bool, ctx: BodyCtx) -> Attach {
        let choice = self.actions.select(target, wire_exists);
        self.post_arm(ArmAction::HookHit);
        let mut air_pull = false;
        match choice {
            Choice::Reel => {
                self.reel_start(ctx);
            }
            Choice::Pull => {
                air_pull = self.pull_start(ctx);
            }
            Choice::Dual => {
                self.post_arm(ArmAction::DualGrapple);
            }
            Choice::Neither => {}
        }
        Attach { choice, air_pull }
    }

    /// The state-flag term of reel availability (`0x00575950`, `CanReelInState` on every
    /// layer's current state). Layer 3 (upper body) and the additive layer are true in
    /// every state; the left arm is false in `LA_S_GRAPPLE_PULL` (and `LA_S_GRAPPLE_REEL_OUT`,
    /// not modelled). Other terms of `0x005c2df0` (`character+0x324`, `0x0099a440`, parachute
    /// `device+0x40`) are not part of this gate.
    pub fn can_reel_in_state(&self) -> bool {
        self.body.can_reel_in_state() && self.arm != ArmState::GrapplePull
    }

    /// `CReelInAction::start` (`0x008098c0`): `ACT_REEL` on layer 0 (it ends at once when
    /// refused), `LA_ACT_REEL` on layer 2, the Reel flag off. Returns whether it started.
    pub fn reel_start(&mut self, ctx: BodyCtx) -> bool {
        let ok = self.can_reel_in_state()
            && (self.body == BodyState::ReelFlight || self.post_body(BodyAction::Reel, ctx));
        self.actions.set(SetAction::ReelIn, false);
        if !ok {
            return false;
        }
        self.post_arm(ArmAction::Reel);
        // reel update case 0: the flight state enables the parachute action
        if self.body == BodyState::ReelFlight {
            self.actions.flying();
        }
        true
    }

    /// `CPullGrappledAction::start` (`0x007ae2e0`). On the ground the left arm alone pulls
    /// (`LA_ACT_GRAPPLE_PULL`); where layer 0 accepts `ACT_GRAPPLE_PULL` the body flies
    /// (`S_PULL_IN_AIR_FLIGHT`) and the arm goes idle. Returns whether the body flies.
    pub fn pull_start(&mut self, ctx: BodyCtx) -> bool {
        if self.body_would_accept(BodyAction::GrapplePull, ctx) {
            self.post_body(BodyAction::GrapplePull, ctx);
            self.post_arm(ArmAction::Idle);
            true
        } else {
            self.post_arm(ArmAction::GrapplePull);
            false
        }
    }

    /// Reel update: the end surface was reached. Dispatch the arrival action and switch
    /// the follow-up actions (`0x00564b20`, `0x005e6b90`).
    pub fn arrive(&mut self, class: ArrivalClass, ctx: BodyCtx) -> bool {
        let act = match class {
            ArrivalClass::Floor => BodyAction::ReelLand,
            ArrivalClass::Wall => BodyAction::ReelImpactVertical,
            ArrivalClass::Ceiling => BodyAction::ReelImpactHorizontal,
        };
        let ok = self.post_body(act, ctx);
        self.actions.arrived(class);
        ok
    }

    /// `ACT_DETACH` (reel update abort, the device detach `0x005a0530`, the monitor
    /// `0x005d2c10`): the body falls if the graph offers it, the arm drops, the action
    /// flags are reset.
    pub fn detach(&mut self) {
        // (the detach edges do not look at what Rico was doing: the context is moot)
        self.post_body(BodyAction::Detach, BodyCtx::Air);
        self.post_arm(ArmAction::Detach);
        self.actions.cancel();
        self.air_pull = None;
        if self.body.needs_wire() {
            // Gap: the graph has no ACT_DETACH edge from the hang states; `CReeledInController`
            // slot 6 posts it when the wire is gone and the exit is native-internal.
            let from = self.body.name();
            self.set_body(BodyState::Free);
            self.note(0, "(wire gone: forced)", from);
        }
    }

    /// **Inferred (Gap)**: the graph offers the jump and the release only from the idle
    /// hang states, but the actions are enabled at arrival (`0x00564b20`, `0x005e6b90`) and
    /// how the original treats an input while the impact clip still plays is not decoded.
    /// An input then cuts the impact short (skips to the hang state) instead of being lost.
    fn skip_impact(&mut self) {
        loop {
            let next = match self.body {
                BodyState::ReelImpactVertical => BodyState::ReeledInIdle,
                BodyState::ReelImpactHorizontal => BodyState::ReeledInToHang,
                BodyState::ReeledInToHang => BodyState::HangStuntIdle,
                _ => return,
            };
            let from = self.body.name();
            self.set_body(next);
            self.note(0, "(input during impact: skipped ahead)", from);
        }
    }

    /// Space on a wall (`CReeledInJumpAction`): the bounce out of the hang.
    pub fn reeled_in_jump(&mut self, ctx: BodyCtx) -> bool {
        self.skip_impact();
        self.post_body(BodyAction::ReeledInJump, ctx)
    }

    /// Ctrl (`CReeledInReleaseAction`): let go of a wall or a ceiling.
    pub fn reeled_in_release(&mut self, ctx: BodyCtx) -> bool {
        self.skip_impact();
        let ok = self.post_body(BodyAction::ReeledInRelease, ctx);
        self.post_arm(ArmAction::Detach);
        self.actions.cancel();
        ok
    }

    /// The air pull ended: `ACT_FALL`, the hook is cancelled.
    pub fn end_air_pull(&mut self, ctx: BodyCtx) {
        self.post_body(BodyAction::Fall, ctx);
        self.air_pull = None;
        self.actions.cancel();
    }

    /// Advance the clocks; a state whose clip has played out takes its `ACT_UPDATE` exit.
    /// `duration` gives a clip's length in seconds (0 when unknown: the state holds).
    pub fn tick(&mut self, dt: f32, duration: &dyn Fn(Clip) -> f32) {
        self.clock += dt;
        self.body_time += dt;
        self.arm_time += dt;
        if let (Some(clip), Some(next)) = (self.body.clip(), self.body.on_finish()) {
            let d = duration(clip) / self.body.speed();
            if d > 0.0 && self.body_time >= d {
                let from = self.body.name();
                self.set_body(next);
                self.note(0, "ACT_UPDATE (clip finished)", from);
                if next == BodyState::ReelFlight && self.actions.is(SetAction::FireHook) {
                    // reel update case 0: the flight state enables the parachute action
                    self.actions.flying();
                }
            }
        }
        if let (Some(clip), Some(next)) = (self.arm.clip(), self.arm.on_finish()) {
            let d = duration(clip);
            if d > 0.0 && self.arm_time >= d {
                let from = self.arm.name();
                self.set_arm(next);
                self.note(2, "LA_ACT_UPDATE (clip finished)", from);
            }
        }
    }

    /// Leave the grapple's body states by other means (a parachute, a seat, a fall to the
    /// ground); the left arm is not touched.
    pub fn force_free(&mut self, why: &'static str) {
        if self.body != BodyState::Free {
            let from = self.body.name();
            self.set_body(BodyState::Free);
            self.note(0, why, from);
        }
    }

    /// Whether the body layer is in one of the grapple's states.
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn owns_body(&self) -> bool {
        self.body != BodyState::Free
    }

    /// The human-readable trace so far.
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn trace(&self) -> String {
        self.steps
            .iter()
            .map(Step::line)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------------------------------------------------------------- the grappled victim

/// `char+0x9e8`: how many hooks hold a character (`0x007513d0(4)` increments it in the
/// hook-hit handler, `0x0069b380` decrements it). **Gap**: the decrement's caller is not
/// located; the runtime decrements it when the wire to the soldier ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GrappledCount(pub u32);

impl GrappledCount {
    pub fn hooked(&mut self) {
        self.0 += 1;
    }

    pub fn unhooked(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }

    /// `C_GRAPPLED_ATLEAST_ONCE`: `1 <= count <= 42`.
    pub fn atleast_once(self) -> bool {
        (GRAPPLED_ATLEAST_ONCE.0..=GRAPPLED_ATLEAST_ONCE.1).contains(&self.0)
    }

    /// `C_NOT_GRAPPLED`: `0 <= count <= 0`.
    pub fn not_grappled(self) -> bool {
        self.0 == 0
    }
}

/// The NPC machine's grapple states (`npc_combat_base.afsm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VictimState {
    #[default]
    Free,
    /// `S_GRAPPLED` (upper body `UB_S_GRAPPLED`: `stumble_grpl`).
    Grappled,
    /// `S_REMOVE_GRAPPLINGHOOK` (`UB_S_REMOVE_GRAPPLINGHOOK`: `stumble_grpl_release`).
    RemoveGrapplingHook,
}

impl VictimState {
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn name(self) -> &'static str {
        match self {
            Self::Free => "S_AI_BASIC_NAVIGATION",
            Self::Grappled => "S_GRAPPLED",
            Self::RemoveGrapplingHook => "S_REMOVE_GRAPPLINGHOOK",
        }
    }

    /// `ACT_UPDATE` with the counter: `T_HIT_BY_GRAPPLINGHOOK` enters `S_GRAPPLED` on
    /// `C_GRAPPLED_ATLEAST_ONCE`; `S_GRAPPLED` leaves on `C_NOT_GRAPPLED`.
    /// `upper_body_done` is `C_WAIT_FOR_UPPERBODY_TO_FINISH` for the removal state.
    pub fn update(self, count: GrappledCount, upper_body_done: bool) -> Self {
        match self {
            Self::Free if count.atleast_once() => Self::Grappled,
            Self::Grappled if count.not_grappled() => Self::RemoveGrapplingHook,
            Self::RemoveGrapplingHook if upper_body_done => Self::Free,
            s => s,
        }
    }

    /// `ACT_FORCE_RELEASE_GRAPPLINGHOOK`: straight to idle.
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn force_release(self) -> Self {
        Self::Free
    }

    /// `ACT_DIE`: a grappled victim goes to `S_DEAD`, a free one to the airborne death.
    #[allow(dead_code)] // trace and test API (headless traces, `grapple/anim/tests.rs`)
    pub fn dies_in_place(self, count: GrappledCount) -> bool {
        count.atleast_once()
    }

    pub fn clip(self) -> Option<Clip> {
        match self {
            Self::Free => None,
            Self::Grappled => Some(Clip::NpcStumble),
            Self::RemoveGrapplingHook => Some(Clip::NpcStumbleRelease),
        }
    }
}

#[cfg(test)]
mod tests;
