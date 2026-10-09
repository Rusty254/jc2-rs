//! Native weapon attachment ownership and the supported socket service (JC2-045).
//!
//! Implements only what `research/formats/weapon-ownership-next.md` (JC2-092) and
//! `weapon-sockets-next.md` (JC2-017) prove, from `JustCause2.exe` (SHA256 df99a479..):
//!
//! * the weapon's world is `Correction * Bone * CharacterWorld` (row vectors, 0x005e3100):
//!   a gun is a child of the bone joint whose local transform is the correction, so
//!   [`bone_local`] is that correction in this engine's column convention;
//! * the bone is `weapon+0x254`, an `NBones::EBoneIndex` of biped.bsk: held guns on
//!   `AttachHandRight` (0x19), the dual-wield left gun on `AttachHandLeft` (0x18), stowed
//!   slot 0/1/2 on `LeftUpLeg` (4) / `RightUpLeg` (7) / `Harness` (0x4d);
//! * hand rotations `R(-pi/2,0,pi/2)` / `R(pi/2,0,-pi/2)`, thigh corrections and the
//!   harness correction are **radians, unscaled**; only the authored A and B rotations
//!   are half-turns, converted by pi exactly once in [`authored_transform`];
//! * the emission frame is V carried into the weapon's world; the shot direction is that
//!   frame's -Z row ([`emission_frame`]);
//! * two-handed guns drive the left hand to `A * weapon_world` (draw state 3).
//!
//! Everything the evidence leaves open is listed in [`UNSUPPORTED`] and logged at start.

use crate::character::Hero;
use crate::combat::{Arsenal, GunModel};
use crate::poses::{Stow, is_reload};
use crate::ragdoll::{PendingRagdoll, PhysicsPose};
use crate::rig::{Animator, Clip, Slot, SupportHand, model_space};
use bevy::prelude::*;
use jc2_formats::weapon_mounts::{AuthoredTransform, WeaponMounts};
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

// biped.bsk bone indices written to `weapon+0x254` (verified writers, §1 of the brief).
pub const HIPS: usize = 0x01;
pub const LEFT_UP_LEG: usize = 0x04;
pub const RIGHT_UP_LEG: usize = 0x07;
pub const ATTACH_HAND_LEFT: usize = 0x18;
pub const ATTACH_HAND_RIGHT: usize = 0x19;
pub const HARNESS: usize = 0x4d;

/// The bone's name in biped.bsk; the indices are valid for that skeleton only.
pub fn bone_name(index: usize) -> Option<&'static str> {
    match index {
        HIPS => Some("Hips"),
        LEFT_UP_LEG => Some("LeftUpLeg"),
        RIGHT_UP_LEG => Some("RightUpLeg"),
        ATTACH_HAND_LEFT => Some("AttachHandLeft"),
        ATTACH_HAND_RIGHT => Some("AttachHandRight"),
        HARNESS => Some("Harness"),
        _ => None,
    }
}

/// The skeleton's bone for a native index, only when the skeleton really is biped.bsk-ordered
/// at that index (the civilian skeleton has another order and is rejected).
pub fn resolve_bone(skeleton: &jc2_formats::animation::Skeleton, index: usize) -> Option<usize> {
    let found = skeleton.bone_index(bone_name(index)?)?;
    (found == index).then_some(found)
}

/// 0x004e1470 with the angles in radians: the rotation as the native flat row-vector matrix
/// (`m[0..3]`, `m[4..6]`, `m[8..10]` are the images of the model's x, y, z axes), which are
/// exactly the columns of this engine's column-vector rotation.
pub fn native_rotation(r: [f32; 3]) -> Quat {
    let (a, b) = r[0].sin_cos();
    let (c, d) = r[1].sin_cos();
    let (e, f) = r[2].sin_cos();
    let row0 = Vec3::new(e * c * a + f * b, e * d, e * c * b - f * a);
    let row1 = Vec3::new(f * c * a - e * b, f * d, f * c * b + e * a);
    let row2 = Vec3::new(d * a, -c, d * b);
    Quat::from_mat3(&Mat3::from_cols(row0, row1, row2)).normalize()
}

/// A native correction matrix: rotation from radians (unscaled), translation set.
pub fn correction(rotation_radians: [f32; 3], translation: [f32; 3]) -> Transform {
    Transform::from_translation(Vec3::from(translation))
        .with_rotation(native_rotation(rotation_radians))
}

/// An authored A/B transform. The half-turn rotation is multiplied by pi **here, once**.
pub fn authored_transform(a: &AuthoredTransform) -> Transform {
    correction(a.rotation_half_turns.map(|h| h * PI), a.translation)
}

/// The hand rotations of 0x005e3100: `R(-pi/2,0,pi/2)` on the right, `R(pi/2,0,-pi/2)` on
/// the left (pi/2 floats at 0x00e2f898 and 0x00e48a74).
pub fn hand_rotation(left: bool) -> Quat {
    if left {
        native_rotation([FRAC_PI_2, 0.0, -FRAC_PI_2])
    } else {
        native_rotation([-FRAC_PI_2, 0.0, FRAC_PI_2])
    }
}

/// The thigh corrections (cases 4/1 and 7 of 0x005e3100): translation and rotation.
pub const LEFT_THIGH_CORRECTION: ([f32; 3], [f32; 3]) = ([-0.11, 0.06, 0.06], [0.0, -1.7, 0.0]);
pub const RIGHT_THIGH_CORRECTION: ([f32; 3], [f32; 3]) = ([0.14, 0.03, 0.06], [0.2, -1.7, 0.0]);

/// The weapon's transform in its attach bone's frame: 0x005e3100's per-bone correction.
/// Bones the function does not special-case get the identity (`Bone * CharWorld`), as does
/// a two-handed-less `Harness` (null correction pointers) and a hand with the flag at 0.
pub fn bone_local(bone: usize, mounts: Option<&WeaponMounts>) -> Transform {
    match bone {
        ATTACH_HAND_LEFT => Transform::from_rotation(hand_rotation(true)),
        ATTACH_HAND_RIGHT => {
            // settings+0x1ca defaults to 1 and is authored 0 only on vehicle guns
            if mounts.is_none_or(|m| m.right_hand_rotation) {
                Transform::from_rotation(hand_rotation(false))
            } else {
                Transform::IDENTITY
            }
        }
        HIPS | LEFT_UP_LEG => correction(LEFT_THIGH_CORRECTION.1, LEFT_THIGH_CORRECTION.0),
        RIGHT_UP_LEG => correction(RIGHT_THIGH_CORRECTION.1, RIGHT_THIGH_CORRECTION.0),
        HARNESS => mounts
            .and_then(|m| m.harness)
            .map_or(Transform::IDENTITY, |h| {
                correction(h.rotation_radians, h.translation)
            }),
        _ => Transform::IDENTITY,
    }
}

// ---- the holder: five slots, a draw state (0x005d6710 / 0x005c0ba0 / 0x007b6f20) ----

// The game has no holder state machine yet (the draw state is Rico's current gun and gait),
// so this model is exercised by the tests only: see the `state-5-mounted-and-b` and
// `dual-wield-gameplay` diagnostics.
#[allow(dead_code)]
pub const HOLDER_SLOTS: usize = 5;

/// Draw state -> (slot, bone) written by 0x005d6710. State 4 is the dual-wield pair.
#[allow(dead_code)]
pub fn draw_bones(state: u8) -> &'static [(usize, usize)] {
    match state {
        1 => &[(0, ATTACH_HAND_RIGHT)],
        2 => &[(1, ATTACH_HAND_RIGHT)],
        3 => &[(2, ATTACH_HAND_RIGHT)],
        4 => &[(0, ATTACH_HAND_LEFT), (1, ATTACH_HAND_RIGHT)],
        5 => &[(4, ATTACH_HAND_RIGHT)],
        _ => &[],
    }
}

/// Stow bone of a slot (0x005c0ba0 / 0x00648f20); slot 3 has none, slot 4 is the state-5 gun.
#[allow(dead_code)]
pub fn stow_bone(slot: usize) -> Option<usize> {
    match slot {
        0 => Some(LEFT_UP_LEG),
        1 => Some(RIGHT_UP_LEG),
        2 => Some(HARNESS),
        4 => Some(ATTACH_HAND_RIGHT),
        _ => None,
    }
}

/// The draw: `Some(state)` with the bones each slot goes to, or `None` (a no-op, the state
/// unchanged) when a required slot is empty.
#[allow(dead_code)]
pub fn draw(slots: &[bool; HOLDER_SLOTS], state: u8) -> Option<(u8, Vec<(usize, usize)>)> {
    let need = draw_bones(state);
    if need.is_empty() || need.iter().any(|(slot, _)| !slots[*slot]) {
        return None;
    }
    Some((state, need.to_vec()))
}

/// 0x007b6f20: the active slot, or 5 (an empty pointer) when state and slot disagree.
#[allow(dead_code)]
pub fn active_index(slots: &[bool; HOLDER_SLOTS], state: u8) -> usize {
    match state {
        3 if slots[2] => 2,
        4 | 2 if slots[1] => 1,
        1 if slots[0] => 0,
        5 if slots[4] => 4,
        _ => 5,
    }
}

/// Whose A matrix drives the left hand: slot 2 in state 3, slot 4 in state 5, else none
/// (an identity target; how identity is weighted is unresolved).
#[allow(dead_code)]
pub fn a_target_owner(slots: &[bool; HOLDER_SLOTS], state: u8) -> Option<usize> {
    match state {
        3 if slots[2] => Some(2),
        5 if slots[4] => Some(4),
        _ => None,
    }
}

// ---- the emission frame ----

/// The V emission frame carried into the weapon's world (0x006f5d50), and the shot
/// direction, the frame's -Z row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmissionFrame {
    pub origin: Vec3,
    pub direction: Vec3,
}

pub fn emission_frame(weapon_world: &Transform, v: Vec3) -> EmissionFrame {
    EmissionFrame {
        origin: weapon_world.transform_point(v),
        direction: (weapon_world.rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
    }
}

/// V in the weapon's own frame; the model-bounds guess only when the settings author none.
pub fn emission_point(mounts: Option<&WeaponMounts>, bbox_min: Vec3, bbox_max: Vec3) -> Vec3 {
    mounts
        .and_then(|m| m.emission_v)
        .map(Vec3::from)
        .unwrap_or_else(|| crate::poses::muzzle_of(bbox_min, bbox_max))
}

// ---- stow smoothing (0x00908940 + slerp 0x004f1af0) ----

/// The slerp factor of a stow bone (constants at 0x01063c90..9c).
pub fn slerp_t(bone: usize) -> Option<f32> {
    match bone {
        HIPS | LEFT_UP_LEG => Some(0.4),
        RIGHT_UP_LEG => Some(0.5),
        HARNESS => Some(0.3),
        _ => None,
    }
}

/// The process-global bone-rotation caches (0x01195f90, 0x01195ff0 and the analogous
/// statics), one per stow case and shared by every character. Seeding on first use and
/// writing the result back are assumptions (see [`UNSUPPORTED`]).
#[derive(Resource, Debug)]
pub struct StowSmoothing {
    pub enabled: bool,
    cache: [Option<Quat>; 4],
}

impl Default for StowSmoothing {
    fn default() -> Self {
        Self {
            enabled: true,
            cache: [None; 4],
        }
    }
}

impl StowSmoothing {
    fn index(bone: usize) -> Option<usize> {
        match bone {
            HIPS => Some(0),
            LEFT_UP_LEG => Some(1),
            RIGHT_UP_LEG => Some(2),
            HARNESS => Some(3),
            _ => None,
        }
    }

    /// `slerp(cached, bone_q, t)`, stored back; the bone's own rotation when disabled.
    pub fn filter(&mut self, bone: usize, q: Quat) -> Quat {
        let (Some(t), Some(i)) = (slerp_t(bone), Self::index(bone)) else {
            return q;
        };
        if !self.enabled {
            return q;
        }
        let next = self.cache[i].map_or(q, |cached| cached.slerp(q, t));
        self.cache[i] = Some(next);
        next
    }
}

// ---- explicit diagnostics ----

/// An item the evidence does not settle. None of these is guessed into the runtime beyond
/// what the `how` column says.
pub struct Unsupported {
    pub id: &'static str,
    pub what: &'static str,
    pub how: &'static str,
}

pub const UNSUPPORTED: &[Unsupported] = &[
    Unsupported {
        id: "pickup-slot-assignment",
        what: "which pickup goes in holder slot 0, 1 or 2 (the slot-insertion writer was not found)",
        how: "kept the earlier order: first one-handed gun slot 1 (RightUpLeg), second slot 0 (LeftUpLeg), two-handed slot 2 (Harness)",
    },
    Unsupported {
        id: "character-0x728-gate",
        what: "the character+0x728 flag that gates +0xe00 clearing and the 0x00741620 world update",
        how: "not modelled",
    },
    Unsupported {
        id: "ik-weighting",
        what: "how the A/B targets (identity for pistols) are weighted in the IK solve",
        how: "pistols and dual wield get no IK; two-handed guns are solved at full weight, not in reload/throw",
    },
    Unsupported {
        id: "solver-0x008f09c0",
        what: "the native two-joint solver and its end-effector orientation",
        how: "plain two-bone solve (LeftArm, LeftForeArm to AttachHandLeft) of the hand's position only",
    },
    Unsupported {
        id: "stow-slerp-predicate",
        what: "predicate 0x006ca590 (always treated true) and whether the global cache is written back",
        how: "low-pass: cache seeded by the first sample, result stored back; Rico only; StowSmoothing::enabled",
    },
    Unsupported {
        id: "thigh-correction-gate",
        what: "bit 4 of [weapon+0x98]+0xc gating the 4/1/7 corrections",
        how: "treated as set",
    },
    Unsupported {
        id: "state-5-mounted-and-b",
        what: "draw state 5 (slot 4), the B target and the id-26 (vehicle gun) IK branch",
        how: "no runtime state; the mapping is tested only",
    },
    Unsupported {
        id: "dual-wield-gameplay",
        what: "alternating fire of two guns and a dual-wield draw state in the game",
        how: "the state-4 bones are implemented and tested; the game never draws it",
    },
    Unsupported {
        id: "soldier-slots",
        what: "soldier holder state, stow and support-hand ownership",
        how: "soldiers get the proven held hand rotation and V; no stow, no support IK",
    },
    Unsupported {
        id: "aim-endpoints",
        what: "writers of weapon+0x200/+0x20c/+0x218, the rocket/grenade/pellet use of the M1 rows, tracer from M2",
        how: "origin is V in the weapon world and direction its -Z row; the aim endpoint stays with combat (JC2-046)",
    },
    Unsupported {
        id: "interrupted-events",
        what: "cleanup of draw/stow/reload mid-shot and animation events (JC2-026)",
        how: "none beyond the IK being off during a reload",
    },
];

// ---- runtime ----

/// `settings\weapons.bin` mounts by weapon name.
#[derive(Resource, Default, Debug)]
pub struct WeaponMountTable(pub HashMap<String, WeaponMounts>);

pub struct WeaponAttachPlugin;

impl Plugin for WeaponAttachPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StowSmoothing>()
            .init_resource::<WeaponMountTable>()
            .add_systems(Startup, load_mounts)
            .add_systems(Update, drive_support_hand.after(crate::combat::attach_guns))
            .add_systems(
                PostUpdate,
                place_guns
                    .after(crate::rig::animate)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn load_mounts(game: Option<Res<crate::Game>>, mut table: ResMut<WeaponMountTable>) {
    for u in UNSUPPORTED {
        info!(
            "weapon_attach unsupported [{}]: {}; {}",
            u.id, u.what, u.how
        );
    }
    if let Some(m) = game
        .and_then(|g| g.fs.read_inflated("weapons.bin").ok())
        .and_then(|d| jc2_formats::weapon_mounts::parse(&d).ok())
    {
        table.0 = m;
    }
}

/// Hero guns: the held ones at their hand correction, the stowed ones at their slot
/// correction with the global slerp smoothing. Runs after the pose and before the transforms
/// propagate, so the shot frame and the draw agree.
#[allow(clippy::type_complexity)]
fn place_guns(
    mut smoothing: ResMut<StowSmoothing>,
    heroes: Query<&Animator, (With<Hero>, Without<PhysicsPose>, Without<PendingRagdoll>)>,
    mut guns: Query<(&GunModel, &mut Transform)>,
) {
    let Ok(an) = heroes.single() else { return };
    let mut model = None;
    for (gun, mut tf) in &mut guns {
        let mut want = gun.base;
        if let Some(bone) = gun.stow.bone()
            && !gun.stow.in_hand()
            && smoothing.enabled
        {
            let model = model.get_or_insert_with(|| model_space(&an.rig.skeleton, &an.local));
            if let Some(b) = model.get(bone) {
                // Bone' keeps the bone's translation and scale; only its rotation is the
                // slerped one, so the gun is the correction turned by q^-1 q'.
                let q = b.rotation;
                let delta = q.inverse() * smoothing.filter(bone, q);
                want.translation = delta * want.translation;
                want.rotation = delta * want.rotation;
            }
        }
        if *tf != want {
            *tf = want;
        }
    }
}

/// The left hand follows A on a two-handed gun in the hand.
fn drive_support_hand(
    arsenal: Option<Res<Arsenal>>,
    table: Res<WeaponMountTable>,
    mut heroes: Query<&mut Animator, With<Hero>>,
    guns: Query<&GunModel>,
) {
    let Some(a) = arsenal else { return };
    let Ok(mut an) = heroes.single_mut() else {
        return;
    };
    let held = guns
        .iter()
        .any(|g| g.slot == a.current && g.stow == Stow::Hand);
    let reloading = an
        .layer(Slot::Action)
        .clip
        .is_some_and(|c| is_reload(c) || c == Clip::Throw);
    let mounts = a
        .guns
        .get(a.current)
        .and_then(|g| table.0.get(&g.weapon.name));
    an.support.hand = match mounts {
        Some(m) if held && !reloading && m.two_handed => Some(SupportHand {
            target: authored_transform(&m.hand_a),
            weapon_in_hand: bone_local(ATTACH_HAND_RIGHT, Some(m)),
        }),
        _ => None,
    };
}

#[cfg(test)]
mod tests;
