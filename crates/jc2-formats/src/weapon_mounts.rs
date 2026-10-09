//! The attachment inputs of `settings\weapons.bin`, read by the original loader
//! `0x006e83a0` (JC2-017 / JC2-092, `research/formats/weapon-sockets-next.md` and
//! `weapon-ownership-next.md`): the emission point V, the hand-IK transforms A and B and
//! the harness (back) stow correction. Kept apart from [`crate::weapons::Weapon`] on
//! purpose: raw authored units are preserved here and converted exactly once by the
//! consumer (`jc2-game/src/weapon_attach.rs`).
//!
//! Units: A and B rotations are authored in **half-turns** (the loader multiplies them by
//! pi); the harness rotation is **radians**, unscaled; translations and V are model-space
//! floats. Missing A/B fields are zero (identity), like the loader.

use crate::property::{Node, PropertyFile, Variant};
use std::collections::HashMap;

/// `#70f54065, #7619481b, #05cc8d39`: V, the emission point (settings +0xd4).
const V: [u32; 3] = [0x70f5_4065, 0x7619_481b, 0x05cc_8d39];
/// A translation and rotation (half-turns).
const A_T: [u32; 3] = [0x4399_2e3c, 0x02c6_9df6, 0x4816_5d1b];
const A_R: [u32; 3] = [0x0c57_90dc, 0x1df6_f7f6, 0xa0b4_0053];
/// B translation and rotation (half-turns).
const B_T: [u32; 3] = [0x3c2e_2ddf, 0xc3d7_56d0, 0x0a6c_9c51];
const B_R: [u32; 3] = [0xa2f8_6494, 0xda1f_f260, 0x0d3a_dfae];
/// `#2477f09c`: two-handed category (settings +0x1c9). Allocates the harness corrections.
const TWO_HANDED: u32 = 0x2477_f09c;
/// `#982afd0c`: right-hand rotation flag (settings +0x1ca), default true.
const HAND_ROTATION: u32 = 0x982a_fd0c;
/// `#a05ef628` / `#1116e1e0`: harness translation / rotation (settings +0x3b8 / +0x3bc).
const HARNESS_T: u32 = 0xa05e_f628;
const HARNESS_R: u32 = 0x1116_e1e0;

/// What `0x006e83a0` loads into the harness corrections when the weapon is two-handed and
/// authors nothing (five MOVSS loads, checked by `weapon_ownership_probe.py`). The
/// rotation is in radians and 3.14 is a literal, not pi.
pub const HARNESS_DEFAULT_TRANSLATION: [f32; 3] = [-0.15, 0.15, 0.15];
#[allow(clippy::approx_constant)]
pub const HARNESS_DEFAULT_ROTATION: [f32; 3] = [1.5, -1.9, 3.14];

/// An authored transform: translation and a rotation in half-turns (x pi on load).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AuthoredTransform {
    pub translation: [f32; 3],
    pub rotation_half_turns: [f32; 3],
}

/// The harness stow correction: translation, and rotation in **radians**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HarnessCorrection {
    pub translation: [f32; 3],
    pub rotation_radians: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponMounts {
    /// `#118bd8f2`: the weapon id (26 is the vehicle-gun selector domain).
    pub id: i32,
    /// V, absent when none of its three fields is authored.
    pub emission_v: Option<[f32; 3]>,
    /// The support-hand IK target (settings +0xec).
    pub hand_a: AuthoredTransform,
    /// The mounted/state-5 target (settings +0x12c).
    pub hand_b: AuthoredTransform,
    /// settings +0x1c9.
    pub two_handed: bool,
    /// settings +0x1ca: the right-hand rotation applies (authored 0 on vehicle guns).
    pub right_hand_rotation: bool,
    /// Present only for two-handed weapons.
    pub harness: Option<HarnessCorrection>,
}

fn float(n: &Node, h: u32) -> Option<f32> {
    match n.value_hash(h)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn triple(n: &Node, keys: [u32; 3]) -> Option<[f32; 3]> {
    if keys.iter().all(|k| n.value_hash(*k).is_none()) {
        return None;
    }
    Some(keys.map(|k| float(n, k).unwrap_or(0.0)))
}

fn vec3(n: &Node, h: u32) -> Option<[f32; 3]> {
    match n.value_hash(h)? {
        Variant::Vec3(v) => Some(*v),
        _ => None,
    }
}

impl WeaponMounts {
    fn from_node(n: &Node) -> Option<Self> {
        let two_handed = float(n, TWO_HANDED).unwrap_or(0.0) != 0.0;
        let authored = |t: [u32; 3], r: [u32; 3]| AuthoredTransform {
            translation: triple(n, t).unwrap_or_default(),
            rotation_half_turns: triple(n, r).unwrap_or_default(),
        };
        Some(Self {
            id: n.int("id")?,
            emission_v: triple(n, V),
            hand_a: authored(A_T, A_R),
            hand_b: authored(B_T, B_R),
            two_handed,
            right_hand_rotation: float(n, HAND_ROTATION).is_none_or(|f| f != 0.0),
            harness: two_handed.then(|| HarnessCorrection {
                translation: vec3(n, HARNESS_T).unwrap_or(HARNESS_DEFAULT_TRANSLATION),
                rotation_radians: vec3(n, HARNESS_R).unwrap_or(HARNESS_DEFAULT_ROTATION),
            }),
        })
    }
}

/// Every weapon's mounts, by its `weapons.bin` name.
pub fn parse(data: &[u8]) -> crate::Result<HashMap<String, WeaponMounts>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = HashMap::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if n.value("id").is_some()
                && let Some(name) = n.str("name")
                && let Some(m) = WeaponMounts::from_node(n)
            {
                out.insert(name.to_string(), m);
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three fixtures of `weapon-sockets-next.md` / `weapon-ownership-next.md`, read
    /// from the retail table. Needs JC2_TEST_DIR.
    #[test]
    fn retail_fixtures() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("weapons.bin") else {
            return;
        };
        let m = parse(&data).unwrap();
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        let beretta = &m["Beretta Handgun"];
        assert_eq!(beretta.id, 2);
        assert!(near(beretta.emission_v.unwrap(), [0.0, 0.11, -0.35]));
        assert_eq!(beretta.hand_a, AuthoredTransform::default());
        assert!(!beretta.two_handed && beretta.harness.is_none());
        assert!(beretta.right_hand_rotation);
        let assault = &m["Assault"];
        assert_eq!(assault.id, 11);
        assert!(near(assault.emission_v.unwrap(), [-0.03, 0.115, -0.75]));
        assert!(near(assault.hand_a.translation, [-0.06, 0.08, -0.35]));
        assert!(near(assault.hand_a.rotation_half_turns, [0.0, 0.0, 0.92]));
        assert_eq!(assault.hand_b, AuthoredTransform::default());
        let h = assault.harness.unwrap();
        assert!(near(h.translation, HARNESS_DEFAULT_TRANSLATION));
        assert!(near(h.rotation_radians, HARNESS_DEFAULT_ROTATION));
        let rpg = &m["RocketLauncher"];
        assert_eq!(rpg.id, 16);
        assert!(near(rpg.emission_v.unwrap(), [0.0, 0.1, 0.7]));
        assert!(near(rpg.hand_a.translation, [-0.1, 0.06, 0.1]));
        assert!(near(rpg.hand_a.rotation_half_turns, [0.7, -0.2, -0.15]));
        let h = rpg.harness.unwrap();
        assert!(near(h.translation, [0.05, 0.175, 0.07]));
        assert!(near(h.rotation_radians, [1.8, 1.5, 1.5]));
    }
}
