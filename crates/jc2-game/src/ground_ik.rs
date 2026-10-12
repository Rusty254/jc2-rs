//! Ground foot placement (JC2-075): the original's per-leg `CIKFootPlacementImpl`, a Havok
//! `hkaFootPlacementIkSolver` wrapper, as far as `research/formats/ground-ik-activation-next.md`
//! (JC2-093) proves it.
//!
//! What is native (JustCause2.exe sha256 df99a479...b74f80):
//! * the tuning: constructor defaults at `0x004bb480` (the reload `0x004bedf0` has no caller and
//!   its 18 keys occur in no retail archive entry), see [`Tuning`];
//! * the enable switch `0x00899c90(flag)`: 1 on entering the rooftop stunt, cling-to-roof, stunt
//!   jump, grappling jump and rocket-baby states, 0 on their exits; both legs off by default
//!   (`0x0087dc80`). **Ordinary on-foot walking has no enable site: retail does not run ground
//!   placement while walking**, so by default [`GroundIkMode::Native`] runs it only where the
//!   original enables it that this runtime has a state for;
//! * left leg then right leg, each with private state and no feedback between them (`+0x138` is
//!   never written, `+0x130/+0x134` never consumed, `+0x13c` never read by the solver);
//! * a fresh vertical ray every update from `ankle + up*1` to `ankle - up*1` (`0x00b09ce0`), a hit
//!   only when the fraction is strictly below 1.0.
//!
//! What is inferred from the decompiled solver `0x00b09fd0` (not executed; each is marked
//! "inferred" below and listed as a Gap in the board hand-in): the smoothing/clamp arithmetic,
//! the ankle target `(S - ownerH) + ankle height`, the second (toe) ray's plane intersection,
//! and how the ankle rotation is blended to the ground normal. Not recovered at all: the
//! collision-filter predicates 1/3 (here: everything but Rico himself), the support identity
//! (retail keeps none: `solver+0x94` is a world height) and the animation-update rate the
//! per-call gains assume.
//!
//! [`GroundIkMode::AllGrounded`] (`--ground-ik all`) runs the same solver whenever Rico is on the
//! ground. That is **not native**: it exists because the user wants feet planted on slopes and
//! stairs while walking, which the original achieves only through its authored animation.

use crate::character::{Hero, HeroModel};
use crate::rig::{Animator, Joint, animate, model_space, rotate_bone, two_bone_solve};
use avian3d::prelude::*;
use bevy::prelude::*;

/// `CIKFootPlacementImpl` defaults, decoded from the immediates of the constructor `0x004bb480`
/// (names are the exact-hash reload keys at `0x00f246b8..`). Units are skeleton distance units,
/// treated as metres (Rico's rig is unit scale; the code never converts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// `FootPlantedAnkleHeight` (+0x84): low end of the height-blend ramp.
    pub foot_planted_ankle_height: f32,
    /// `FootRaisedAnkleHeight` (+0x88): high end of the ramp.
    pub foot_raised_ankle_height: f32,
    /// `MaxAnkleHeight` (+0x8c): the target ankle height is pulled down to this (model up).
    pub max_ankle_height: f32,
    /// `MinAnkleHeight` (+0x90): below this the leg is rejected.
    pub min_ankle_height: f32,
    /// `CosineMaxKneeAngle` (+0x94), handed to the two-joint solve unchanged.
    pub cosine_max_knee_angle: f32,
    /// `CosineMinKneeAngle` (+0x98).
    pub cosine_min_knee_angle: f32,
    /// `RaycastDistanceUp` (+0x9c): the ray starts this far above the ankle.
    pub raycast_distance_up: f32,
    /// `RaycastDistanceDown` (+0xa0): and ends this far below it.
    pub raycast_distance_down: f32,
    /// `OnOffGain` (+0x118): blend gain per solver call.
    pub on_off_gain: f32,
    /// `GroundAscendingGain` (+0x11c).
    pub ground_ascending_gain: f32,
    /// `GroundDescendingGain` (+0x120).
    pub ground_descending_gain: f32,
    /// `FootPlantedGain` (+0x124): upper clamp of the height blend factor.
    pub foot_planted_gain: f32,
    /// `FootRaisedGain` (+0x128): lower clamp of it.
    pub foot_raised_gain: f32,
    /// `FootEnd` (+0x50): the toe point in the ankle bone's frame.
    pub foot_end: Vec3,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            foot_planted_ankle_height: 0.0642,
            foot_raised_ankle_height: 0.35,
            max_ankle_height: 0.35,
            min_ankle_height: -0.35,
            cosine_max_knee_angle: -0.9,
            cosine_min_knee_angle: 0.8,
            raycast_distance_up: 1.0,
            raycast_distance_down: 1.0,
            on_off_gain: 0.3,
            ground_ascending_gain: 1.0,
            ground_descending_gain: 1.0,
            foot_planted_gain: 1.0,
            foot_raised_gain: 0.2,
            foot_end: Vec3::new(0.0, 0.0647, -0.1172),
        }
    }
}

/// The blend decays to exactly zero below this (pseudo of `0x00b09fd0`).
const BLEND_SNAP: f32 = 0.01;
/// The toe ray is only cast when the toe point is more than this from the ankle on some axis.
const FOOT_END_EPS: f32 = 0.001;

/// Where ground placement runs.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GroundIkMode {
    /// Only in the states whose entry functions enable it in the original (native).
    #[default]
    Native,
    /// Whenever Rico is grounded or on a roof. Not native.
    AllGrounded,
    /// Never (before/after captures).
    Off,
}

/// One leg's private solver state (the original's solver object, `0xb09fd0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LegState {
    /// 0..1 weight of the IK result over the animation (`solver` blend).
    pub blend: f32,
    /// Smoothed ground height in world up (`solver+0x94`): a world height, not support-relative.
    pub prior: f32,
    /// The last ankle offset along model up (`solver+0xb0`), held while the leg is inactive.
    pub offset: f32,
    /// Vertical error `groundH - (ownerH + 0)` (`impl+0x130`); diagnostic, never fed to the other leg.
    pub error: f32,
    /// Whether the ray and the height limits accepted the leg this call (`impl+0x134`).
    pub accepted: bool,
}

impl LegState {
    /// The ankle's applied displacement along model up.
    pub fn applied(&self) -> f32 {
        self.offset * self.blend
    }
}

/// One solver call for one leg. `ankle_h` is the animated ankle's height along model up,
/// `owner_h` the owner's world height, `ground_h` the world height under the ankle from the ray
/// (`None`: disabled, or the ray missed). Inferred from the pseudo of `0x00b09fd0` (see the
/// module docs and `ground-ik-activation-next.md` section 4).
pub fn solve_leg(
    t: &Tuning,
    s: &mut LegState,
    ankle_h: f32,
    owner_h: f32,
    ground_h: Option<f32>,
) {
    let mut accepted = false;
    if let Some(g) = ground_h {
        s.error = g - owner_h;
        let ramp = t.foot_raised_ankle_height - t.foot_planted_ankle_height;
        let k = (1.0 - (ankle_h - t.foot_planted_ankle_height) / ramp)
            .clamp(t.foot_raised_gain, t.foot_planted_gain);
        let smoothed = if s.blend != 0.0 {
            let gain = if s.prior <= g {
                t.ground_ascending_gain
            } else {
                t.ground_descending_gain
            };
            s.prior + gain * k * (g - s.prior)
        } else {
            g
        };
        let target = ((smoothed - owner_h) + ankle_h).min(t.max_ankle_height);
        if target >= t.min_ankle_height {
            accepted = true;
            s.prior = smoothed;
            s.offset = target - ankle_h;
        }
    } else {
        s.error = 0.0;
    }
    s.accepted = accepted;
    let want = if accepted { 1.0 } else { 0.0 };
    s.blend += t.on_off_gain * (want - s.blend);
    if s.blend < BLEND_SNAP {
        s.blend = 0.0;
    }
}

/// The three bones of a leg.
#[derive(Clone, Copy, Debug)]
pub struct LegBones {
    pub hip: usize,
    pub knee: usize,
    pub ankle: usize,
}

/// What the ray found, in world space: fraction along the ray (0 at the start) and the surface
/// normal.
pub type RayHit = Option<(f32, Vec3)>;

/// Ground placement for one character.
#[derive(Component, Clone, Debug, Default)]
pub struct GroundIk {
    /// Left then right, as the original's vector order.
    pub legs: [LegState; 2],
    pub tuning: Tuning,
    bones: Option<[LegBones; 2]>,
    /// The switch the original calls `0x00899c90(flag)`; both legs together.
    pub enabled: bool,
}

impl GroundIk {
    pub fn for_rig(skeleton: &jc2_formats::animation::Skeleton) -> Self {
        let leg = |h: &str, k: &str, a: &str| {
            Some(LegBones {
                hip: skeleton.bone_index(h)?,
                knee: skeleton.bone_index(k)?,
                ankle: skeleton.bone_index(a)?,
            })
        };
        let bones = (|| {
            Some([
                leg("LeftUpLeg", "LeftLeg", "LeftFoot")?,
                leg("RightUpLeg", "RightLeg", "RightFoot")?,
            ])
        })();
        Self {
            bones,
            ..default()
        }
    }

    /// The legs have no bones to work on (a rig without the leg chain).
    pub fn is_usable(&self) -> bool {
        self.bones.is_some()
    }
}

/// Cast the ankle ray (and the toe ray) and return the ground height along world up and the
/// surface normal. The hit rule is the native one: the fraction must be strictly below 1.
/// `cast(start, length)` casts straight down from `start`.
fn probe_ground(
    t: &Tuning,
    ankle_w: Vec3,
    toe_w: Vec3,
    cast: &mut impl FnMut(Vec3, f32) -> RayHit,
) -> Option<(f32, Vec3)> {
    let len = t.raycast_distance_up + t.raycast_distance_down;
    let start = ankle_w + Vec3::Y * t.raycast_distance_up;
    let (mut fraction, mut normal) = cast(start, len).filter(|(f, _)| *f < 1.0)?;
    // inferred: a second ray at the toe point; its plane (normal.up > 0) meets the ankle's
    // vertical line, and the nearer (higher) fraction wins
    if (toe_w - ankle_w).abs().max_element() > FOOT_END_EPS {
        let toe_start = toe_w + Vec3::Y * t.raycast_distance_up;
        if let Some((tf, tn)) = cast(toe_start, len)
            && tf < 1.0
            && tn.y > 0.0
        {
            let hit = toe_start.y - tf * len;
            // plane through (toe.x, hit, toe.z) with normal tn, height at the ankle's x/z
            let plane_y = hit - (tn.x * (ankle_w.x - toe_w.x) + tn.z * (ankle_w.z - toe_w.z)) / tn.y;
            let plane_f = (start.y - plane_y) / len;
            if (0.0..1.0).contains(&plane_f) && plane_f < fraction {
                fraction = plane_f;
                normal = tn;
            }
        }
    }
    Some((start.y - fraction * len, normal))
}

/// Run both legs once on `local` (bone-local transforms of the animated pose, which are
/// corrected in place). `world_from_model` carries the model into the world (yaw and translation
/// only: the model's up is the world's). `cast` is the world ray query.
pub fn apply(
    skeleton: &jc2_formats::animation::Skeleton,
    local: &mut [Transform],
    ik: &mut GroundIk,
    world_from_model: &Transform,
    mut cast: impl FnMut(Vec3, f32) -> RayHit,
) {
    let Some(bones) = ik.bones else { return };
    let owner_h = world_from_model.translation.y;
    for (leg, b) in bones.iter().enumerate() {
        let global = model_space(skeleton, local);
        let ankle_model = global[b.ankle].translation;
        let ankle_h = ankle_model.y;
        let ground = if ik.enabled {
            let toe_model = global[b.ankle].transform_point(ik.tuning.foot_end);
            let ankle_w = world_from_model.transform_point(ankle_model);
            let toe_w = world_from_model.transform_point(toe_model);
            probe_ground(&ik.tuning, ankle_w, toe_w, &mut cast)
        } else {
            None
        };
        let state = &mut ik.legs[leg];
        solve_leg(&ik.tuning, state, ankle_h, owner_h, ground.map(|g| g.0));
        if state.blend == 0.0 {
            continue;
        }
        let normal_model = ground
            .map(|g| world_from_model.rotation.inverse() * g.1)
            .unwrap_or(Vec3::Y);
        correct_leg(
            skeleton,
            local,
            *b,
            &ik.tuning,
            state.applied(),
            state.blend,
            normal_model,
            ground.is_some(),
        );
    }
}

/// Move the ankle `lift` along model up (the solved target, already scaled by the blend), bend
/// the knee to reach it and tilt the foot onto the ground's normal by the blend.
#[allow(clippy::too_many_arguments)]
fn correct_leg(
    skeleton: &jc2_formats::animation::Skeleton,
    local: &mut [Transform],
    b: LegBones,
    t: &Tuning,
    lift: f32,
    blend: f32,
    normal_model: Vec3,
    have_normal: bool,
) {
    let mut global = model_space(skeleton, local);
    let (hip, knee, ankle) = (
        global[b.hip].translation,
        global[b.knee].translation,
        global[b.ankle].translation,
    );
    let foot_rot = global[b.ankle].rotation;
    let (l1, l2) = ((knee - hip).length(), (ankle - knee).length());
    if l1 < 1e-4 || l2 < 1e-4 {
        return;
    }
    // the knee angle limits (cosines at the knee), widened to where the animation already is:
    // inferred, so a straight-legged idle is not bent by the limit on flat ground
    let d_of = |cos_k: f32| (l1 * l1 + l2 * l2 - 2.0 * l1 * l2 * cos_k).max(0.0).sqrt();
    let d0 = (ankle - hip).length();
    let lo = d_of(t.cosine_min_knee_angle).min(d0);
    let hi = d_of(t.cosine_max_knee_angle).max(d0);
    let want = ankle + Vec3::Y * lift;
    let to = want - hip;
    let dist = to.length();
    if !dist.is_finite() || dist < 1e-5 {
        return;
    }
    let target = hip + to / dist * dist.clamp(lo, hi);
    if let Some((new_knee, new_ankle)) = two_bone_solve(hip, knee, ankle, target) {
        let turn = Quat::from_rotation_arc(
            (knee - hip).normalize_or(Vec3::NEG_Y),
            (new_knee - hip).normalize_or(Vec3::NEG_Y),
        );
        rotate_bone(skeleton, local, &mut global, b.hip, turn);
        let knee_now = global[b.knee].translation;
        let turn = Quat::from_rotation_arc(
            (global[b.ankle].translation - knee_now).normalize_or(Vec3::NEG_Y),
            (new_ankle - knee_now).normalize_or(Vec3::NEG_Y),
        );
        rotate_bone(skeleton, local, &mut global, b.knee, turn);
    }
    // inferred: the foot keeps its animated orientation in the model, then leans from model up
    // onto the ground's normal by the blend (the native arithmetic is a Gap)
    let tilt = if have_normal {
        let n = normal_model.normalize_or(Vec3::Y);
        Quat::IDENTITY.slerp(Quat::from_rotation_arc(Vec3::Y, n), blend)
    } else {
        Quat::IDENTITY
    };
    let desired = tilt * foot_rot;
    let delta = desired * global[b.ankle].rotation.inverse();
    rotate_bone(skeleton, local, &mut global, b.ankle, delta);
}

/// Whether ground placement is on for `hero` under `mode`. The native enable states
/// (`ground-ik-activation-next.md` section 1): standing on a vehicle's roof is the rooftop-stunt
/// controller (`0x00701f30`); the stunt-jump, grappling-jump, cling-to-roof and rocket-baby
/// states have no counterpart in this runtime yet (Gap).
fn wants_ik(mode: GroundIkMode, hero: &Hero) -> bool {
    let native = hero.stunt.is_some() && hero.stunt_clip.is_none();
    match mode {
        GroundIkMode::Native => native,
        GroundIkMode::AllGrounded => native || (hero.grounded && !hero.swimming && !hero.parachute),
        GroundIkMode::Off => false,
    }
}

/// Give every hero with a rig a [`GroundIk`], switch it by [`GroundIkMode`] and apply it after
/// the animation step, before transforms propagate.
#[allow(clippy::type_complexity)]
fn ground_ik(
    mut commands: Commands,
    mode: Res<GroundIkMode>,
    spatial: SpatialQuery,
    mut heroes: Query<(
        Entity,
        &Hero,
        &Transform,
        &mut Animator,
        Option<&mut GroundIk>,
        &Children,
    )>,
    models: Query<&Transform, (With<HeroModel>, Without<Hero>)>,
    mut joints: Query<&mut Transform, (With<Joint>, Without<Hero>, Without<HeroModel>)>,
) {
    for (entity, hero, tf, mut an, ik, children) in &mut heroes {
        let Some(mut ik) = ik else {
            commands
                .entity(entity)
                .insert(GroundIk::for_rig(&an.rig.skeleton));
            continue;
        };
        if !ik.is_usable() || hero.downed {
            continue;
        }
        ik.enabled = wants_ik(*mode, hero);
        if !ik.enabled && ik.legs.iter().all(|l| l.blend == 0.0) {
            continue;
        }
        let Some(model_tf) = children.iter().find_map(|c| models.get(c).ok()) else {
            continue;
        };
        let world_from_model = tf.mul_transform(*model_tf);
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let an = &mut *an;
        let rig = an.rig.clone();
        apply(
            &rig.skeleton,
            &mut an.local,
            &mut ik,
            &world_from_model,
            |start, length| {
                spatial
                    .cast_ray(start, Dir3::NEG_Y, length, true, &filter)
                    .map(|h| (h.distance / length, h.normal))
            },
        );
        // write back only the three bones of each leg
        if let Some(bones) = ik.bones {
            for b in bones {
                for i in [b.hip, b.knee, b.ankle] {
                    if let Some(e) = an.joints.get(i)
                        && let Ok(mut j) = joints.get_mut(*e)
                    {
                        *j = an.local[i];
                    }
                }
            }
        }
    }
}

pub struct GroundIkPlugin;

impl Plugin for GroundIkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroundIkMode>().add_systems(
            PostUpdate,
            ground_ik
                .after(animate)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[cfg(test)]
mod tests;
