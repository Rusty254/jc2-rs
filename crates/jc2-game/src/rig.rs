//! Rico's skeleton and animation: the game's `biped.bsk` becomes a joint hierarchy
//! that skins the `SkinnedGeneral` meshes, and the `.ban` clips (spline-compressed
//! Havok animation, `jc2_formats::animation`) drive it.
//!
//! An [`Animator`] plays a **base** clip for the whole body (crossfaded from the last
//! one) and up to four **layers** over it, in the game's own split
//! (`research/formats/animation.md`): the upper-body clips (`two_handed_run_fwd`,
//! `rifle_aim_con`, `ub_reload_rifle`, …) carry tracks for the spine, arms and fingers
//! only, so a layer overrides exactly the bones its clip has tracks for — the track set
//! *is* the mask, the legs keep running underneath. Last comes the **aim constraint**
//! ([`AimIk`]): it turns the spine so the weapon's barrel points where the camera aims,
//! within limits, the way the game's `*_con` clips are bent by their `.acs` constraints.

use crate::coords::qs_to_bevy;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};

use bevy::prelude::*;
use jc2_formats::GameFs;
use jc2_formats::animation::{QsTransform, Skeleton, SplineAnimation};
use std::collections::HashMap;
use std::sync::Arc;

/// The clips the characters use and the files they come from (all members of
/// `global\animations.blz`, or dropzone files of the same name). Which state plays
/// which, and the layers, are in `research/formats/animation.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clip {
    // --- full body (the base layer) ---
    Idle,
    Walk,
    Run,
    /// Sprinting (`dash_fwd`, 4.17 m/s of root motion).
    Sprint,
    /// Running sideways while aiming the other way (`runstrafe_*`, `wkstrafe_*`).
    RunStrafeLeft,
    RunStrafeRight,
    WalkStrafeLeft,
    WalkStrafeRight,
    /// Walking with a one-handed gun (`one_handed_wk_fwd`: the whole body).
    Walk1H,
    Jump,
    Fall,
    /// The landing out of a run (`ANIM_JUMP_RUNNING_LAND`: `run_jump_to_run`) and from a
    /// standing jump or a short fall (`ANIM_JUMP_LAND`: `std_landing_light`).
    Land,
    LandLight,
    /// A hard landing from high up (`std_landing_heavy`) …
    LandHeavy,
    /// … the roll out of a fast one (`ANIM_LANDING_ROLL`: `land_chute_roll`) …
    LandRoll,
    /// … and the chute's hard landing (`ANIM_LANDING_HARD`: `land_chute_hard`).
    LandHard,
    Hang,
    /// Reeled in by the grappling hook, flying at the anchor (`grpl_reel_flight_part2`).
    Reel,
    /// Climbing over a ledge out of the hang (`ANIM_REELED_IN_JUMP`: `grpl_reel_bounce`).
    GrappleClimb,
    /// Under the canopy (`ANIM_PARACHUTE`): a two-axis time blend; the speed axis
    /// (`rico_speed_tb_up`: 0.5 s is hands on the handles, 0 leaning back, 1 diving) …
    Chute,
    /// … and the turn axis (`rico_turn_tb_up`: 0 left, 1 right).
    ChuteTurn,
    ChuteLand,
    SwimIdle,
    Swim,
    /// Free fall: the skydive (`ANIM_SKYDIVE`: `mhfreefall`).
    Dive,
    /// Crouched on a speeding vehicle's roof (`cr_idle`).
    Crouch,
    /// Sitting at the wheel (`mv_driving_fwd`): soldiers driving and flying.
    Drive,
    // --- upper-body layers: how a gun is held (spine, head, arms, fingers) ---
    /// The pistol or SMG held while running after a fight (`one_handed_run_fwd`, in step
    /// with `run_fwd`) and standing alert (`ub_idle_alerted_var1`); otherwise only the
    /// fingers close on it (`UB_ANIM_WIELDING_IDLE` is `fingers_armed_rgt` alone).
    Wield1HRun,
    Wield1HAlert,
    /// Rifles, shotgun: standing, walking, running (`two_handed_*`).
    Wield2HIdle,
    Wield2HWalk,
    Wield2HRun,
    /// The rocket launcher on the shoulder (`rpg_idle`, `two_handed_rpg_*`).
    WieldRpgIdle,
    WieldRpgWalk,
    WieldRpgRun,
    /// Crouched on a roof with a gun (`cr_idle_1handed`, `_2handed`, `_rpg`).
    Crouch1H,
    Crouch2H,
    CrouchRpg,
    /// The right hand's fingers closed round a grip (`fingers_armed_rgt`).
    FingersRight,
    FingersLeft,
    // --- aim and fire (the `UB_ANIM_AIM_AND_FIRE*` sets: an aim pose and its recoil) ---
    Aim1H,
    Recoil1H,
    Aim1HLeft,
    Recoil1HLeft,
    /// Rifles (`UB_ANIM_AIM_AND_FIRE_2H`: `rifle_aim_con` + `two_handed_recoil_con`) …
    Aim2H,
    Recoil2H,
    /// … and crouched on a roof (`_2H_LOW`: `rifle_aim_con_low` + `rifle_recoil_con_low`).
    Aim2HLow,
    Recoil2HLow,
    AimRpg,
    // --- reloads and throws ---
    /// One gun in the right hand, pistol or SMG (`UB_ANIM_RELOAD_RH`: `ub_single_pistol_reload`).
    ReloadPistol,
    ReloadRifle,
    ReloadShotgun,
    ReloadRpg,
    /// The grenade, thrown over the arm with the left hand (`LA_ANIM_OVER_THROW_GRENADE`,
    /// on the left-arm layer: masked to the left arm) …
    Throw,
    /// … and under the canopy.
    ThrowChute,
    // --- the grappling hook, fired from the left forearm ---
    /// The left arm held out at the target (`grpl_fire_aim_con`) …
    GrappleAim,
    /// … the shot (`LA_ANIM_RAISE_GRAPPLE`: `grpl_fire_ani_con`) …
    GrappleFire,
    /// … and in the air (`grpl_freefall_fire_ani_con`).
    GrappleFireAir,
    // --- soldiers (on the same `biped.bsk`) ---
    NpcIdle,
    NpcWalk,
    NpcRun,
    NpcAimRifle,
    NpcRecoilRifle,
    NpcAimSmg,
    NpcRecoilSmg,
    NpcReload,
    /// A soldier dragged by the hook (`UB_ANIM_GRAPPLED`: `stumble_grpl`).
    NpcStumble,
    /// The civilians' own clips, on `civilian_biped.bsk` (not in `ALL`: `Rig::load_with`).
    CivIdle,
    CivWalk,
    CivRun,
    /// A civilian at the wheel (`civ_driving_fwd`).
    CivDrive,
}

impl Clip {
    /// Everything on `biped.bsk`: Rico's clips and the soldiers'.
    pub const ALL: &[Clip] = &[
        Clip::Dive,
        Clip::Walk,
        Clip::Idle,
        Clip::Run,
        Clip::Sprint,
        Clip::RunStrafeLeft,
        Clip::RunStrafeRight,
        Clip::WalkStrafeLeft,
        Clip::WalkStrafeRight,
        Clip::Walk1H,
        Clip::Jump,
        Clip::Fall,
        Clip::Land,
        Clip::LandLight,
        Clip::LandHeavy,
        Clip::LandRoll,
        Clip::LandHard,
        Clip::Hang,
        Clip::Reel,
        Clip::GrappleClimb,
        Clip::Chute,
        Clip::ChuteTurn,
        Clip::ChuteLand,
        Clip::SwimIdle,
        Clip::Swim,
        Clip::Crouch,
        Clip::Drive,
        Clip::Wield1HRun,
        Clip::Wield1HAlert,
        Clip::Wield2HIdle,
        Clip::Wield2HWalk,
        Clip::Wield2HRun,
        Clip::WieldRpgIdle,
        Clip::WieldRpgWalk,
        Clip::WieldRpgRun,
        Clip::Crouch1H,
        Clip::Crouch2H,
        Clip::CrouchRpg,
        Clip::FingersRight,
        Clip::FingersLeft,
        Clip::Aim1H,
        Clip::Recoil1H,
        Clip::Aim1HLeft,
        Clip::Recoil1HLeft,
        Clip::Aim2H,
        Clip::Recoil2H,
        Clip::Aim2HLow,
        Clip::Recoil2HLow,
        Clip::AimRpg,
        Clip::ReloadPistol,
        Clip::ReloadRifle,
        Clip::ReloadShotgun,
        Clip::ReloadRpg,
        Clip::Throw,
        Clip::ThrowChute,
        Clip::GrappleAim,
        Clip::GrappleFire,
        Clip::GrappleFireAir,
        Clip::NpcIdle,
        Clip::NpcWalk,
        Clip::NpcRun,
        Clip::NpcAimRifle,
        Clip::NpcRecoilRifle,
        Clip::NpcAimSmg,
        Clip::NpcRecoilSmg,
        Clip::NpcReload,
        Clip::NpcStumble,
    ];

    pub fn file(self) -> &'static str {
        match self {
            Clip::Idle => "std_idle.ban",
            Clip::Walk => "wk_fwd.ban",
            Clip::Run => "run_fwd.ban",
            Clip::Sprint => "dash_fwd.ban",
            Clip::RunStrafeLeft => "runstrafe_lft.ban",
            Clip::RunStrafeRight => "runstrafe_rgt.ban",
            Clip::WalkStrafeLeft => "wkstrafe_lft.ban",
            Clip::WalkStrafeRight => "wkstrafe_rgt.ban",
            Clip::Walk1H => "one_handed_wk_fwd.ban",
            Clip::Jump => "run_jump_flight.ban",
            Clip::Fall => "fall.ban",
            Clip::Land => "run_jump_to_run.ban",
            Clip::LandLight => "std_landing_light.ban",
            Clip::LandHeavy => "std_landing_heavy.ban",
            Clip::LandRoll => "land_chute_roll.ban",
            Clip::LandHard => "land_chute_hard.ban",
            Clip::Hang => "grpl_hang.ban",
            Clip::Reel => "grpl_reel_flight_part2.ban",
            Clip::GrappleClimb => "grpl_reel_bounce.ban",
            Clip::Chute => "rico_speed_tb_up.ban",
            Clip::ChuteTurn => "rico_turn_tb_up.ban",
            Clip::Dive => "mhfreefall.ban",
            Clip::ChuteLand => "land_chute.ban",
            Clip::SwimIdle => "swim_idle.ban",
            Clip::Swim => "swim_fwd.ban",
            Clip::Crouch => "cr_idle.ban",
            Clip::Drive => "mv_driving_fwd.ban",
            Clip::Wield1HRun => "one_handed_run_fwd.ban",
            Clip::Wield1HAlert => "ub_idle_alerted_var1.ban",
            Clip::Wield2HIdle => "two_handed_idle.ban",
            Clip::Wield2HWalk => "two_handed_wk_fwd.ban",
            Clip::Wield2HRun => "two_handed_run_fwd.ban",
            Clip::WieldRpgIdle => "rpg_idle.ban",
            Clip::WieldRpgWalk => "two_handed_rpg_wk_fwd.ban",
            Clip::WieldRpgRun => "two_handed_rpg_run_fwd.ban",
            Clip::Crouch1H => "cr_idle_1handed.ban",
            Clip::Crouch2H => "cr_idle_2handed.ban",
            Clip::CrouchRpg => "cr_idle_rpg.ban",
            Clip::FingersRight => "fingers_armed_rgt.ban",
            Clip::FingersLeft => "fingers_armed_lft.ban",
            Clip::Aim1H => "one_handed_rgt_aim.ban",
            Clip::Recoil1H => "one_handed_rgt_recoil.ban",
            Clip::Aim1HLeft => "one_handed_lft_aim.ban",
            Clip::Recoil1HLeft => "one_handed_lft_recoil.ban",
            Clip::Aim2H => "rifle_aim_con.ban",
            Clip::Recoil2H => "two_handed_recoil_con.ban",
            Clip::Aim2HLow => "rifle_aim_con_low.ban",
            Clip::Recoil2HLow => "rifle_recoil_con_low.ban",
            Clip::AimRpg => "rpg_aim.ban",
            Clip::ReloadPistol => "ub_single_pistol_reload.ban",
            Clip::ReloadRifle => "ub_reload_rifle.ban",
            Clip::ReloadShotgun => "ub_reload_shotgun_pump.ban",
            Clip::ReloadRpg => "rpg_reload.ban",
            Clip::Throw => "throw_grenade_over_arm.ban",
            Clip::ThrowChute => "ub_parachute_throw_grenade.ban",
            Clip::GrappleAim => "grpl_fire_aim_con.ban",
            Clip::GrappleFire => "grpl_fire_ani_con.ban",
            Clip::GrappleFireAir => "grpl_freefall_fire_ani_con.ban",
            Clip::NpcIdle => "npc_std_idle.ban",
            Clip::NpcWalk => "npc_wk_fwd.ban",
            Clip::NpcRun => "npc_tactical_fwd.ban",
            Clip::NpcAimRifle => "npc_rifle_aim_con.ban",
            Clip::NpcRecoilRifle => "npc_two_handed_single_recoil_con.ban",
            Clip::NpcAimSmg => "npc_two_handed_smg_aim_con.ban",
            Clip::NpcRecoilSmg => "npc_two_handed_smg_recoil_con.ban",
            Clip::NpcReload => "npc_ub_single_pistol_reload.ban",
            Clip::NpcStumble => "stumble_grpl.ban",
            Clip::CivIdle => "pose_shared_idle.ban",
            Clip::CivWalk => "pose_shared_walk.ban",
            Clip::CivRun => "pose_shared_panicrun.ban",
            Clip::CivDrive => "civ_driving_fwd.ban",
        }
    }

    /// `grpl_hang.ban` is authored facing the other way: its root is turned 180° about the
    /// vertical against every other clip (his lateral axis points the opposite way, the
    /// hips face the camera in a chase view). Without correction he turns round to look
    /// back along the wire (`research/formats/character.md`).
    pub fn faces_backwards(self) -> bool {
        self.file() == "grpl_hang.ban"
    }

    /// Which way the body faces inside the take (yaw about +y, radians): the strafes are
    /// authored travelling along −z with the body side-on (`runstrafe_lft` faces +x and
    /// runs to its left), so the model is turned by the opposite to keep him facing his way.
    pub fn body_yaw(self) -> f32 {
        match self {
            Clip::RunStrafeLeft | Clip::WalkStrafeLeft => -std::f32::consts::FRAC_PI_2,
            Clip::RunStrafeRight | Clip::WalkStrafeRight => std::f32::consts::FRAC_PI_2,
            _ => 0.0,
        }
    }

    pub fn looping(self) -> bool {
        !matches!(
            self,
            Clip::Jump
                | Clip::Land
                | Clip::LandLight
                | Clip::LandHeavy
                | Clip::LandRoll
                | Clip::LandHard
                | Clip::GrappleClimb
                | Clip::ChuteLand
                | Clip::Recoil1H
                | Clip::Recoil1HLeft
                | Clip::Recoil2H
                | Clip::Recoil2HLow
                | Clip::ReloadPistol
                | Clip::ReloadRifle
                | Clip::ReloadRpg
                | Clip::Throw
                | Clip::ThrowChute
                | Clip::GrappleFire
                | Clip::GrappleFireAir
                | Clip::NpcRecoilRifle
                | Clip::NpcRecoilSmg
                | Clip::NpcReload
        )
    }
}

pub struct LoadedClip {
    pub anim: SplineAnimation,
    /// Track index → bone index in the skeleton.
    pub bones: Vec<Option<usize>>,
    /// Ground speed the clip was made for, from its root motion (the run: 3.33 m/s).
    pub natural_speed: Option<f32>,
}

/// A skeleton and the clips mapped onto it.
pub struct RigData {
    /// The skeleton's file name (`biped.bsk`).
    pub file: String,
    pub skeleton: Skeleton,
    pub clips: HashMap<Clip, LoadedClip>,
}

impl RigData {
    pub fn duration(&self, clip: Clip) -> f32 {
        self.clips.get(&clip).map_or(0.0, |c| c.anim.duration)
    }

    pub fn has(&self, clip: Clip) -> bool {
        self.clips.contains_key(&clip)
    }
}

/// Rico's rig, loaded once at startup (NPC rigs live in `npc::NpcRigs`).
#[derive(Resource, Clone)]
pub struct Rig(pub Arc<RigData>);

impl std::ops::Deref for Rig {
    type Target = RigData;
    fn deref(&self) -> &RigData {
        &self.0
    }
}

/// A skeleton or clip by file name: at the top level (dropzone override) or inside
/// `animations.blz` (`GameFs::read_any`).
pub fn read_animation_file(fs: &GameFs, name: &str) -> Option<Vec<u8>> {
    fs.read_any(name).ok()
}

impl Rig {
    /// Load the skeleton named by the player settings (`biped.bsk`) and every clip
    /// that can be found; `None` without a skeleton.
    pub fn load(fs: &GameFs, skeleton_file: &str) -> Option<Self> {
        Self::load_with(fs, skeleton_file, Clip::ALL)
    }

    /// The skeleton and the given clips (the civilians' on `civilian_biped.bsk`).
    pub fn load_with(fs: &GameFs, skeleton_file: &str, wanted: &[Clip]) -> Option<Self> {
        let bytes = read_animation_file(fs, skeleton_file)?;
        let skeleton = match Skeleton::parse(&bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!("{skeleton_file}: {e}");
                return None;
            }
        };
        let mut clips = HashMap::new();
        for &clip in wanted {
            if clips.contains_key(&clip) {
                continue;
            }
            let Some(bytes) = read_animation_file(fs, clip.file()) else {
                warn!("{}: not found", clip.file());
                continue;
            };
            match SplineAnimation::parse(&bytes) {
                Ok(anim) => {
                    let bones = anim
                        .track_names
                        .iter()
                        .map(|n| skeleton.bone_index(n))
                        .collect();
                    let natural_speed = anim.root_speed();
                    clips.insert(
                        clip,
                        LoadedClip {
                            anim,
                            bones,
                            natural_speed,
                        },
                    );
                }
                Err(e) => warn!("{}: {e}", clip.file()),
            }
        }
        info!(
            "rig: {} ({} bones), {} clips",
            skeleton.name,
            skeleton.bones.len(),
            clips.len()
        );
        Some(Self(Arc::new(RigData {
            file: skeleton_file.to_ascii_lowercase(),
            skeleton,
            clips,
        })))
    }
}

/// Turn a root transform half way round the vertical axis (in the engine's axes, which
/// are Bevy's: x and z change sign).
fn turn_round(t: &QsTransform) -> QsTransform {
    // (0, 1, 0, 0) * q
    let [x, y, z, w] = t.rotation;
    QsTransform {
        translation: [-t.translation[0], t.translation[1], -t.translation[2]],
        rotation: [z, w, -x, -y],
        scale: t.scale,
    }
}

/// The bone poses (in bone order, `None` where the clip has no track) of `clip` at
/// `time`, with the root of the clips that face backwards turned round.
pub fn clip_bones(
    rig: &RigData,
    id: Clip,
    time: f32,
    scratch: &mut Vec<QsTransform>,
) -> Option<Vec<Option<QsTransform>>> {
    let clip = rig.clips.get(&id)?;
    clip.anim.sample(time, scratch);
    let mut current: Vec<Option<QsTransform>> = vec![None; rig.skeleton.bones.len()];
    for (ti, qs) in scratch.iter().enumerate() {
        if let Some(Some(b)) = clip.bones.get(ti) {
            current[*b] = Some(*qs);
        }
    }
    if id.faces_backwards()
        && let Some(root) = rig.skeleton.reference_pose.first()
    {
        current[0] = Some(turn_round(&current[0].unwrap_or(*root)));
    }
    Some(current)
}

/// Runs the animators once a frame, before transforms propagate. Added by whoever
/// needs it first (the character, the NPCs).
pub struct RigPlugin;

impl Plugin for RigPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            animate.before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// A joint entity: its bone index (for inspection).
#[derive(Component)]
#[allow(dead_code)]
pub struct Joint(pub usize);

/// The layers over the base clip, applied in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// How the gun is carried: the upper body of a wielding clip, in step with the legs.
    Wield = 0,
    /// The aim pose.
    Aim = 1,
    /// Recoil, reloads, throws, the grapple shot: played once over everything.
    Action = 2,
    /// The fingers round the grip.
    Fingers = 3,
}

pub const SLOTS: usize = 4;

/// One layer: a clip over the base that overrides the bones its clip has tracks for,
/// faded in and out by `weight`.
#[derive(Clone, Debug, Default)]
pub struct Layer {
    pub clip: Option<Clip>,
    pub time: f32,
    pub rate: f32,
    /// 0 … 1: how much of the clip shows.
    pub weight: f32,
    /// Seconds to fade in and out (the `.asb`'s blend times).
    pub blend: f32,
    /// Follows the base clip's phase (upper-body locomotion: `two_handed_run_fwd` with
    /// `run_fwd`, both 1.2 s).
    pub sync: bool,
    /// Plays once, fades out at its end and is cleared.
    pub once: bool,
    /// Only the bones under this one (the left-arm layer's clips are whole-body takes).
    pub mask: Option<usize>,
    /// Asked to fade out.
    off: bool,
    /// The clip it replaced (its time and weight), kept underneath until the new one is
    /// fully in, then faded out.
    prev: Option<FadingLayer>,
}

/// A fading layer retains its bone mask and playback rate until it is gone.
#[derive(Clone, Copy, Debug)]
struct FadingLayer {
    clip: Clip,
    time: f32,
    weight: f32,
    mask: Option<usize>,
    rate: f32,
}

/// The aim constraint: turns the spine so that `axis` of `bone` (the barrel of the gun
/// in that hand) points along `dir`.
#[derive(Clone, Debug)]
pub struct AimIk {
    /// The wanted direction, in the model's space (−z ahead of him, +x his right).
    pub dir: Vec3,
    /// 0 … 1, easing towards `target` over [`AIM_BLEND`].
    pub weight: f32,
    pub target: f32,
    /// The bone that holds the weapon (`AttachHandRight`) and the barrel's axis in its
    /// frame (`WEAPON_AXIS_RIGHT`).
    pub bone: Option<usize>,
    pub axis: Vec3,
    /// The bones that turn, and their shares of the turn (the `.acsb` weights: the 2H
    /// aim turns `Spine1`, the RPG `Spine` and `Spine1`, the one-handed aim the right
    /// shoulder); the last one takes what the sharing left over.
    pub chain: Vec<(usize, f32)>,
    /// The head looks along the aim (the `.acsb`s weight `Head` 100 in every aim).
    pub head: Option<usize>,
    /// What it did last frame: the chain's yaw and pitch (radians), for the tests.
    pub applied: (f32, f32),
}

impl Default for AimIk {
    fn default() -> Self {
        Self {
            dir: Vec3::NEG_Z,
            weight: 0.0,
            target: 0.0,
            bone: None,
            axis: WEAPON_AXIS_RIGHT,
            chain: Vec::new(),
            head: None,
            applied: (0.0, 0.0),
        }
    }
}

/// The support-hand constraint (JC2-045): the left hand is driven to the weapon's authored
/// A target. The native solver (0x008f09c0) and its weighting are not traced; this is a
/// plain two-bone solve of `LeftArm`/`LeftForeArm` to `AttachHandLeft` at full weight, and
/// it drives the hand's position only.
#[derive(Clone, Debug)]
pub struct SupportHand {
    /// A, in the weapon's space (`AuthoredTransform` converted once).
    pub target: Transform,
    /// The weapon's frame in `AttachHandRight`'s (the hand rotation).
    pub weapon_in_hand: Transform,
}

/// The support-hand IK state: what to reach (set by `weapon_attach`), and what it did.
#[derive(Clone, Debug, Default)]
pub struct SupportIk {
    pub hand: Option<SupportHand>,
    /// `LeftArm`, `LeftForeArm`, `AttachHandLeft`, `AttachHandRight`.
    bones: Option<[usize; 4]>,
    /// Last frame: the target (model space) and how far the hand ended from it. `None`
    /// when the constraint was off or rejected the target as out of reach.
    pub solved: Option<(Vec3, f32)>,
    /// How far the pose put the hand from the target before the solve (m).
    pub moved: f32,
}

/// A target further than this many arm lengths from the shoulder is rejected (the data
/// does not fit the pose), rather than stretching the arm towards it.
pub const SUPPORT_REACH_LIMIT: f32 = 1.25;

/// New elbow and hand positions that put the hand of a two-bone chain on `target` (or as
/// near as the arm reaches), keeping the elbow on the side it was.
pub fn two_bone_solve(root: Vec3, mid: Vec3, end: Vec3, target: Vec3) -> Option<(Vec3, Vec3)> {
    let (l1, l2) = ((mid - root).length(), (end - mid).length());
    let to = target - root;
    let dist = to.length();
    if l1 < 1e-4 || l2 < 1e-4 || !dist.is_finite() || dist > (l1 + l2) * SUPPORT_REACH_LIMIT {
        return None;
    }
    let d = dist.clamp((l1 - l2).abs() + 1e-4, l1 + l2 - 1e-4);
    let dir = if dist > 1e-5 {
        to / dist
    } else {
        (end - root).normalize_or(Vec3::X)
    };
    let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).sqrt();
    let p = mid - root;
    let side = p - dir * p.dot(dir);
    let side = if side.length() < 1e-5 {
        dir.any_orthonormal_vector()
    } else {
        side.normalize()
    };
    Some((
        root + dir * (l1 * cos_a) + side * (l1 * sin_a),
        root + dir * d,
    ))
}

/// Drive the left hand to the weapon's A target. Returns the target and the remaining
/// distance of `AttachHandLeft` from it.
fn support_hand_ik(
    skeleton: &Skeleton,
    local: &mut [Transform],
    bones: [usize; 4],
    s: &SupportHand,
) -> Option<(Vec3, f32, f32)> {
    let [arm, fore, hand_l, hand_r] = bones;
    let mut global = model_space(skeleton, local);
    let weapon = global[hand_r] * s.weapon_in_hand;
    let target = weapon.transform_point(s.target.translation);
    let moved = global[hand_l].translation.distance(target);
    let (mid, end) = two_bone_solve(
        global[arm].translation,
        global[fore].translation,
        global[hand_l].translation,
        target,
    )?;
    let root = global[arm].translation;
    let turn = Quat::from_rotation_arc(
        (global[fore].translation - root).normalize_or(Vec3::X),
        (mid - root).normalize_or(Vec3::X),
    );
    rotate_bone(skeleton, local, &mut global, arm, turn);
    let elbow = global[fore].translation;
    let turn = Quat::from_rotation_arc(
        (global[hand_l].translation - elbow).normalize_or(Vec3::X),
        (end - elbow).normalize_or(Vec3::X),
    );
    rotate_bone(skeleton, local, &mut global, fore, turn);
    let error = global[hand_l].translation.distance(target);
    Some((target, error, moved))
}

/// How far the spine turns to aim before the legs have to follow (radians).
pub const AIM_YAW_LIMIT: f32 = 1.6;
pub const AIM_PITCH_UP: f32 = 1.2;
pub const AIM_PITCH_DOWN: f32 = 1.1;
/// Seconds to bring the aim in or out.
pub const AIM_BLEND: f32 = 0.12;
/// How far the head turns on from the chest to look along the aim (radians).
const HEAD_LIMIT: f32 = 1.0;

/// Where a gun sits in the hand. Every weapon model is made with its barrel along −z,
/// its top +y and its origin in the grip; `AttachHandRight` (6 cm along the hand, 3 cm
/// towards the palm) holds that origin, and the hand's own axes (+x along the fingers,
/// −y out of the palm, −z the thumb's side) give the turn: the barrel along +x, the top
/// towards −z. Derived from the two-handed clips (in every one the left hand sits
/// 30 cm along the right hand's +x and 10 cm towards its −z: the rifle's handguard)
/// and the pistol aim (the barrel then points where the arm does). The exe proves the same
/// turn, `R(-pi/2,0,pi/2)` (`weapon_attach::hand_rotation`); this is its test oracle.
#[cfg(test)]
pub fn weapon_in_right_hand() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::NEG_Z, Vec3::NEG_X))
}

/// The barrel's direction in the attach bone's frame.
pub const WEAPON_AXIS_RIGHT: Vec3 = Vec3::X;

/// Drives the joints of one skinned model.
#[derive(Component)]
pub struct Animator {
    pub rig: Arc<RigData>,
    pub joints: Vec<Entity>,
    pub clip: Clip,
    pub time: f32,
    pub rate: f32,
    /// The clip being faded out, its time, and how much of it is left (0..1).
    pub fading: Option<(Clip, f32, f32)>,
    /// Seconds the next base crossfade takes (`FADE_SECONDS` unless a caller sets it).
    pub fade: f32,
    pub layers: [Layer; SLOTS],
    pub aim: AimIk,
    /// The left hand on a two-handed gun's A target.
    pub support: SupportIk,
    pose: Vec<QsTransform>,
    pose_fade: Vec<QsTransform>,
    /// The bones' local transforms after the last step.
    pub local: Vec<Transform>,
}

impl Animator {
    pub fn new(rig: Arc<RigData>, joints: Vec<Entity>) -> Self {
        let bones = rig.skeleton.bones.len();
        let aim = AimIk {
            bone: rig.skeleton.bone_index("AttachHandRight"),
            head: rig.skeleton.bone_index("Head"),
            ..default()
        };
        let support_bones = (|| {
            let sk = &rig.skeleton;
            Some([
                sk.bone_index("LeftArm")?,
                sk.bone_index("LeftForeArm")?,
                sk.bone_index("AttachHandLeft")?,
                sk.bone_index("AttachHandRight")?,
            ])
        })();
        Self {
            rig,
            joints,
            clip: Clip::Idle,
            time: 0.0,
            rate: 1.0,
            fading: None,
            fade: FADE_SECONDS,
            layers: Default::default(),
            aim,
            support: SupportIk {
                bones: support_bones,
                ..default()
            },
            pose: Vec::new(),
            pose_fade: Vec::new(),
            local: vec![Transform::IDENTITY; bones],
        }
    }

    /// Switch clips with a crossfade (no-op when already playing it).
    pub fn play(&mut self, clip: Clip) {
        if self.clip == clip {
            return;
        }
        self.fading = Some((self.clip, self.time, 1.0));
        self.clip = clip;
        self.time = 0.0;
        self.fade = FADE_SECONDS;
    }

    /// Switch clips with a crossfade of `seconds`.
    pub fn play_blend(&mut self, clip: Clip, seconds: f32) {
        if self.clip == clip {
            return;
        }
        self.play(clip);
        self.fade = seconds.max(0.01);
    }

    pub fn layer(&self, slot: Slot) -> &Layer {
        &self.layers[slot as usize]
    }

    /// Loop `clip` on `slot` (or fade the slot out with `None`), crossfading over
    /// `blend` seconds; `sync` keeps it in step with the base clip.
    pub fn set_layer(&mut self, slot: Slot, clip: Option<Clip>, blend: f32, sync: bool) {
        let rig = self.rig.clone();
        let l = &mut self.layers[slot as usize];
        let clip = clip.filter(|c| rig.has(*c));
        match clip {
            None => {
                if l.clip.is_some() {
                    l.off = true;
                    l.blend = blend.max(0.01);
                }
            }
            Some(c) if l.clip == Some(c) && !l.once => {
                l.off = false;
                l.sync = sync;
                l.blend = blend.max(0.01);
            }
            Some(c) => {
                if let Some(old) = l.clip
                    && l.weight > 0.0
                {
                    l.prev = Some(FadingLayer {
                        clip: old,
                        time: l.time,
                        weight: l.weight,
                        mask: l.mask,
                        rate: l.rate,
                    });
                }
                l.clip = Some(c);
                l.time = 0.0;
                l.rate = 1.0;
                l.weight = 0.0;
                l.blend = blend.max(0.01);
                l.sync = sync;
                l.once = false;
                l.off = false;
                l.mask = None;
            }
        }
    }

    /// Hold `clip` on `slot` at `time` (a pose picked by a parameter, the game's
    /// play mode 0: the chute's turn axis).
    pub fn hold_layer(&mut self, slot: Slot, clip: Clip, time: f32, blend: f32) {
        self.set_layer(slot, Some(clip), blend, false);
        let l = &mut self.layers[slot as usize];
        if l.clip == Some(clip) {
            l.rate = 0.0;
            l.time = time;
        }
    }

    /// Hold the base clip at `time` (rate 0): the chute's speed axis.
    pub fn hold(&mut self, clip: Clip, time: f32, blend: f32) {
        self.play_blend(clip, blend);
        self.rate = 0.0;
        self.time = time;
    }

    /// Play `clip` once on `slot` from its start (restarting it if it is already
    /// playing), at `rate`, fading in and out over `blend` seconds.
    pub fn play_once(&mut self, slot: Slot, clip: Clip, rate: f32, blend: f32) {
        if !self.rig.has(clip) {
            return;
        }
        let l = &mut self.layers[slot as usize];
        if let Some(old) = l.clip
            && l.weight > 0.0
            && old != clip
        {
            l.prev = Some(FadingLayer {
                clip: old,
                time: l.time,
                weight: l.weight,
                mask: l.mask,
                rate: l.rate,
            });
        }
        // a restarted recoil keeps its weight: no dip between shots
        if l.clip != Some(clip) {
            l.weight = 0.0;
        }
        l.clip = Some(clip);
        l.time = 0.0;
        l.rate = rate.max(0.01);
        l.blend = blend.max(0.01);
        l.sync = false;
        l.once = true;
        l.off = false;
        l.mask = None;
    }

    /// `play_once`, on the bones under `root` only.
    pub fn play_once_masked(&mut self, slot: Slot, clip: Clip, rate: f32, blend: f32, root: &str) {
        let mask = self.rig.skeleton.bone_index(root);
        self.play_once(slot, clip, rate, blend);
        let l = &mut self.layers[slot as usize];
        if l.clip == Some(clip) {
            l.mask = mask;
        }
    }

    /// Advance by `dt` and pose the skeleton: `self.local` holds every bone's local
    /// transform afterwards.
    pub fn step(&mut self, dt: f32) {
        let rig = self.rig.clone();
        let Some(clip) = rig.clips.get(&self.clip) else {
            return;
        };
        // advance the base
        self.time += dt * self.rate;
        let base_duration = clip.anim.duration;
        if self.clip.looping() {
            if base_duration > 0.0 {
                self.time = self.time.rem_euclid(base_duration);
            }
        } else {
            self.time = self.time.min(base_duration);
        }
        let phase = if base_duration > 0.0 {
            self.time / base_duration
        } else {
            0.0
        };
        // the pose of the current clip, in bone order (reference pose where no track)
        self.local.clear();
        self.local
            .extend(rig.skeleton.reference_pose.iter().map(qs_to_bevy));
        let Some(mut current) = clip_bones(&rig, self.clip, self.time, &mut self.pose) else {
            return;
        };
        // crossfade from the previous clip
        if let Some((prev_clip, prev_time, left)) = self.fading {
            let left = left - dt / self.fade.max(0.01);
            if left <= 0.0 {
                self.fading = None;
            } else {
                if let Some(prev) = clip_bones(&rig, prev_clip, prev_time, &mut self.pose_fade) {
                    for (b, qs) in prev.iter().enumerate() {
                        if let Some(qs) = qs {
                            let cur = current[b].unwrap_or(rig.skeleton.reference_pose[b]);
                            current[b] = Some(lerp_qs(&cur, qs, left));
                        }
                    }
                }
                self.fading = Some((prev_clip, prev_time, left));
            }
        }
        // the layers
        for l in &mut self.layers {
            advance_layer(&rig, l, dt, phase);
            if let Some(prev) = l.prev {
                overlay(
                    &rig,
                    &mut current,
                    prev.clip,
                    prev.time,
                    prev.weight,
                    prev.mask,
                    &mut self.pose_fade,
                );
            }
            if let Some(c) = l.clip
                && l.weight > 0.0
            {
                overlay(
                    &rig,
                    &mut current,
                    c,
                    l.time,
                    l.weight,
                    l.mask,
                    &mut self.pose_fade,
                );
            }
        }
        for (b, qs) in current.iter().enumerate() {
            if let Some(qs) = qs {
                self.local[b] = qs_to_bevy(qs);
            }
        }
        // the aim constraint
        let a = &mut self.aim;
        let toward = a.target.clamp(0.0, 1.0);
        let step = dt / AIM_BLEND;
        a.weight = if a.weight < toward {
            (a.weight + step).min(toward)
        } else {
            (a.weight - step).max(toward)
        };
        a.applied = (0.0, 0.0);
        if a.weight > 0.0
            && let Some(bone) = a.bone
            && !a.chain.is_empty()
        {
            a.applied = aim_spine(&rig.skeleton, &mut self.local, bone, a);
        }
        // the support hand, after the weapon's world is known from the pre-IK pose
        let solved = match (&self.support.hand, self.support.bones) {
            (Some(hand), Some(bones)) => {
                support_hand_ik(&rig.skeleton, &mut self.local, bones, hand)
            }
            _ => None,
        };
        self.support.moved = solved.map_or(0.0, |s| s.2);
        self.support.solved = solved.map(|s| (s.0, s.1));
    }

    /// Which bones the aim turns (`AimIk::chain`), by name and share.
    pub fn set_aim_chain(&mut self, chain: &[(&str, f32)]) {
        let sk = &self.rig.skeleton;
        let want: Vec<(usize, f32)> = chain
            .iter()
            .filter_map(|(n, s)| Some((sk.bone_index(n)?, *s)))
            .collect();
        if want != self.aim.chain {
            self.aim.chain = want;
        }
    }
}

/// Move a layer's clock and weight on by `dt`.
fn advance_layer(rig: &RigData, l: &mut Layer, dt: f32, phase: f32) {
    if let Some(mut prev) = l.prev {
        prev.time += dt * prev.rate;
        let d = rig.duration(prev.clip);
        if prev.clip.looping() && d > 0.0 {
            prev.time = prev.time.rem_euclid(d);
        } else {
            prev.time = prev.time.min(d);
        }
        // held while the new clip comes in, then faded out
        let w = if l.clip.is_some() && l.weight < 1.0 && !l.off {
            prev.weight
        } else {
            prev.weight - dt / l.blend.max(0.01)
        };
        prev.weight = w;
        l.prev = (w > 0.0).then_some(prev);
    }
    let Some(c) = l.clip else { return };
    let d = rig.duration(c);
    if l.sync {
        l.time = phase * d;
    } else {
        l.time += dt * l.rate;
        if c.looping() && !l.once {
            if d > 0.0 {
                l.time = l.time.rem_euclid(d);
            }
        } else {
            l.time = l.time.min(d);
        }
    }
    let ending = l.once && (d - l.time) / l.rate.max(0.01) <= l.blend;
    let target = if l.off || ending { 0.0 } else { 1.0 };
    let step = dt / l.blend.max(0.01);
    if l.weight < target {
        l.weight = (l.weight + step).min(1.0);
    } else if l.weight > target || target == 0.0 {
        l.weight = (l.weight - step).max(0.0);
        if l.weight <= 0.0 && (l.off || ending) {
            l.clip = None;
            l.off = false;
            l.once = false;
        }
    }
}

/// Whether bone `b` is `root` or under it.
fn under(skeleton: &Skeleton, mut b: usize, root: usize) -> bool {
    loop {
        if b == root {
            return true;
        }
        match skeleton.bones[b].parent {
            Some(p) if p < b => b = p,
            _ => return false,
        }
    }
}

/// Blend `clip` at `time` over `pose` by `weight`, on the bones it has tracks for (and
/// that are under `mask`, when there is one).
fn overlay(
    rig: &RigData,
    pose: &mut [Option<QsTransform>],
    clip: Clip,
    time: f32,
    weight: f32,
    mask: Option<usize>,
    scratch: &mut Vec<QsTransform>,
) {
    let Some(bones) = clip_bones(rig, clip, time, scratch) else {
        return;
    };
    let w = weight.clamp(0.0, 1.0);
    for (b, qs) in bones.iter().enumerate() {
        if mask.is_some_and(|m| !under(&rig.skeleton, b, m)) {
            continue;
        }
        if let Some(qs) = qs {
            let under = pose[b].unwrap_or(rig.skeleton.reference_pose[b]);
            pose[b] = Some(if w >= 1.0 {
                *qs
            } else {
                lerp_qs(&under, qs, w)
            });
        }
    }
}

/// Every bone's transform in the model's space, from the local ones.
pub fn model_space(skeleton: &Skeleton, local: &[Transform]) -> Vec<Transform> {
    let mut global: Vec<Transform> = Vec::with_capacity(local.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let l = local.get(i).copied().unwrap_or_default();
        global.push(match bone.parent {
            Some(p) if p < i => global[p] * l,
            _ => l,
        });
    }
    global
}

/// Yaw of a direction about +y, 0 straight ahead (−z), positive to his left.
pub fn yaw_of_dir(v: Vec3) -> f32 {
    (-v.x).atan2(-v.z)
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// The aim constraint: share the turn that brings the barrel onto `aim.dir` between the
/// chain's bones (within the limits), take out what is left on the last one, then turn
/// the head to look along the aim. Returns the yaw and pitch it applied.
fn aim_spine(skeleton: &Skeleton, local: &mut [Transform], bone: usize, aim: &AimIk) -> (f32, f32) {
    let want = aim.dir.normalize_or(Vec3::NEG_Z);
    let global = model_space(skeleton, local);
    let barrel = (global[bone].rotation * aim.axis).normalize_or(Vec3::NEG_Z);
    let w = aim.weight.clamp(0.0, 1.0);
    let yaw = wrap(yaw_of_dir(want) - yaw_of_dir(barrel)).clamp(-AIM_YAW_LIMIT, AIM_YAW_LIMIT) * w;
    let pitch = (want.y.clamp(-1.0, 1.0).asin() - barrel.y.clamp(-1.0, 1.0).asin())
        .clamp(-AIM_PITCH_DOWN, AIM_PITCH_UP)
        * w;
    // where the barrel should end up
    let (ty, tp) = (
        yaw_of_dir(barrel) + yaw,
        barrel.y.clamp(-1.0, 1.0).asin() + pitch,
    );
    let target = Vec3::new(-ty.sin() * tp.cos(), tp.sin(), -ty.cos() * tp.cos());
    let right = Quat::from_rotation_y(ty) * Vec3::X;
    let mut global = global;
    for &(b, share) in &aim.chain {
        let delta =
            Quat::from_axis_angle(right, pitch * share) * Quat::from_rotation_y(yaw * share);
        rotate_bone(skeleton, local, &mut global, b, delta);
    }
    // what the sharing left over goes on the last bone
    let barrel = (global[bone].rotation * aim.axis).normalize_or(Vec3::NEG_Z);
    let rest = Quat::from_rotation_arc(barrel, target);
    if let Some(&(last, _)) = aim.chain.last()
        && rest.angle_between(Quat::IDENTITY) < 0.6
    {
        rotate_bone(skeleton, local, &mut global, last, rest);
    }
    // the head looks along the aim
    if let Some(h) = aim.head {
        let face = (global[h].rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        let turn = Quat::from_rotation_arc(face, want);
        let (axis, angle) = turn.to_axis_angle();
        if angle.is_finite() && axis.is_finite() {
            let delta = Quat::from_axis_angle(axis, angle.min(HEAD_LIMIT) * w);
            rotate_bone(skeleton, local, &mut global, h, delta);
        }
    }
    (yaw, pitch)
}

/// Turn bone `b` by `delta` (in the model's space, about the bone's own origin), keeping
/// its children attached; `global` is brought up to date.
fn rotate_bone(
    skeleton: &Skeleton,
    local: &mut [Transform],
    global: &mut Vec<Transform>,
    b: usize,
    delta: Quat,
) {
    let parent_rot = match skeleton.bones[b].parent {
        Some(p) if p < b => global[p].rotation,
        _ => Quat::IDENTITY,
    };
    let new_global = delta * global[b].rotation;
    local[b].rotation = (parent_rot.inverse() * new_global).normalize();
    *global = model_space(skeleton, local);
}

const FADE_SECONDS: f32 = 0.15;

/// Build the joint hierarchy under `root` in the reference pose and the inverse bind
/// matrices that go with it. Returns the joints in bone order.
pub fn spawn_joints(
    commands: &mut Commands,
    root: Entity,
    skeleton: &Skeleton,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
) -> (Vec<Entity>, Handle<SkinnedMeshInverseBindposes>) {
    let n = skeleton.bones.len();
    let mut joints = Vec::with_capacity(n);
    let mut global = Vec::with_capacity(n);
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let local = qs_to_bevy(&skeleton.reference_pose[i]);
        let parent_global: Mat4 = match bone.parent {
            Some(p) if p < i => global[p],
            _ => Mat4::IDENTITY,
        };
        let g = parent_global * local.to_matrix();
        global.push(g);
        let e = commands
            .spawn((
                local,
                Visibility::default(),
                Joint(i),
                Name::new(bone.name.clone()),
            ))
            .id();
        joints.push(e);
    }
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let parent = match bone.parent {
            Some(p) if p < i => joints[p],
            _ => root,
        };
        commands.entity(parent).add_child(joints[i]);
    }
    let inverse: Vec<Mat4> = global.iter().map(|g| g.inverse()).collect();
    let handle = bindposes.add(SkinnedMeshInverseBindposes::from(inverse));
    (joints, handle)
}

/// The `SkinnedMesh` component for a mesh of this rig.
pub fn skin(joints: &[Entity], bindposes: &Handle<SkinnedMeshInverseBindposes>) -> SkinnedMesh {
    SkinnedMesh {
        inverse_bindposes: bindposes.clone(),
        joints: joints.to_vec(),
    }
}

fn lerp_qs(a: &QsTransform, b: &QsTransform, t: f32) -> QsTransform {
    let qa = Quat::from_array(a.rotation);
    let qb = Quat::from_array(b.rotation);
    let q = qa.slerp(qb, t);
    let tr = Vec3::from(a.translation).lerp(Vec3::from(b.translation), t);
    let sc = Vec3::from(a.scale).lerp(Vec3::from(b.scale), t);
    QsTransform {
        translation: tr.to_array(),
        rotation: q.to_array(),
        scale: sc.to_array(),
    }
}

/// Advance every animator and write the joints' local transforms.
pub fn animate(
    time: Res<Time>,
    mut animators: Query<
        &mut Animator,
        (
            Without<crate::ragdoll::PhysicsPose>,
            Without<crate::ragdoll::PendingRagdoll>,
        ),
    >,
    mut joints: Query<&mut Transform, With<Joint>>,
) {
    let dt = time.delta_secs();
    for mut an in &mut animators {
        an.step(dt);
        let an = &*an;
        for (b, &e) in an.joints.iter().enumerate() {
            if let Ok(mut tf) = joints.get_mut(e) {
                *tf = an.local[b];
            }
        }
    }
}

#[cfg(test)]
mod tests;
