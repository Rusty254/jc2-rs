//! What the grappling hook does to the thing it hits: pull it to Rico, or reel Rico to it.
//!
//! Evidence (see `research/formats/character.md`, "Grapple target classes"):
//!
//! * `grappling_weight_category` (`#692808c5`) is an integer authored on game objects
//!   (the base reader `FUN_008af820` stores it in a 7-bit field). Shipped values are 0, 1
//!   and 2; there are no names for them. `CSimpleRigidObject`: 1 on small movable props
//!   (barrels, tubes: 389 of 408 nodes are dynamic), 2 on structure pieces and heavy parts
//!   (200 of 511 are fixed), 0 on default/destroyed/scenery nodes. Vehicle smart objects
//!   (`CSmartObject` of every vehicle doll), `CItem` and `CWeapon` are always 0;
//!   `CSAMSite`, `CBigCannon`, `CDamageableObject`, `CVehicleComponent` are 2. Characters
//!   never carry the key.
//! * No code in the exe compares the category (or a mass against Rico's) at the points the
//!   decompiler could follow. The pull/reel split is in the undecoded state machines:
//!   `S_GRAPPLED`/`T_HIT_BY_GRAPPLINGHOOK` exist only in the soldier machines
//!   (`npc_combat_*.afsm`), `LA_ACT_GRAPPLE_PULL` pulls, `LA_ACT_FIRE_GRAPPLE` and
//!   `LA_ACT_FIRE_GRAPPLE_SO` (vehicle smart objects) reel Rico in.
//!
//! So: a soldier is pulled and a vehicle is an anchor ([`Basis::States`]); a body that
//! cannot move is an anchor ([`Basis::Physics`]); categories 1 and 2 follow the authored
//! data ([`Basis::Authored`], light = pulled, heavy = anchor; that mapping is the
//! best-correlated reading of the data and is **not** read from a native branch); a
//! category 0, an absent or an unknown one falls back to the old mass threshold
//! ([`Basis::Fallback`], explicitly unknown). "Lighter than Rico is pulled" is not proven.

use bevy::prelude::*;

/// The authored `grappling_weight_category` of an object, when its data has one.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrappleWeight(pub u8);

/// Things lighter than this (kg) are pulled when nothing in the data says otherwise.
/// **Unknown fallback**: no native value was found; it is only used for bodies with no
/// usable `grappling_weight_category` (see [`classify`]).
pub const FALLBACK_LIGHT_MASS: f32 = 400.0;

/// Which category an authored value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightCategory {
    /// 0: the default; carries no information about weight.
    Default,
    /// 1: small movable props.
    Light,
    /// 2: structure pieces and heavy parts.
    Heavy,
    /// Anything else (not present in the shipped data).
    Other(u8),
}

impl WeightCategory {
    pub fn from_authored(v: u8) -> Self {
        match v {
            0 => Self::Default,
            1 => Self::Light,
            2 => Self::Heavy,
            n => Self::Other(n),
        }
    }
}

/// What a target is, for the hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrappleClass {
    /// A soldier or pedestrian.
    Character,
    /// A vehicle (its smart object is the anchor).
    Vehicle,
    /// An authored light prop (category 1).
    LightProp,
    /// An authored heavy body (category 2).
    HeavyProp,
    /// A body with no usable category: judged by mass ([`Basis::Fallback`]).
    Unclassified,
    /// A body that does not move (fixed, sleeping scenery, terrain).
    Static,
}

/// How firmly the classification is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// The target's own state machine / smart object decides (soldier, vehicle).
    States,
    /// The body does not move, so only Rico can.
    Physics,
    /// The authored category; the 1 = pull, 2 = anchor reading is data-correlated.
    Authored,
    /// Unknown rule: the mass threshold [`FALLBACK_LIGHT_MASS`].
    Fallback,
}

/// What the classifier looks at.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClassInput {
    pub character: bool,
    pub vehicle: bool,
    /// A dynamic rigid body (not fixed or kinematic).
    pub dynamic: bool,
    /// The authored `grappling_weight_category`, if any.
    pub category: Option<u8>,
    /// The body's mass in kg, if known.
    pub mass: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrappleClassification {
    pub class: GrappleClass,
    /// The target is pulled to Rico; otherwise Rico is reeled to it.
    pub pull: bool,
    pub basis: Basis,
}

/// Classify a hook target. The order is the strength of the evidence.
pub fn classify(i: ClassInput) -> GrappleClassification {
    let make = |class, pull, basis| GrappleClassification { class, pull, basis };
    if i.character {
        return make(GrappleClass::Character, true, Basis::States);
    }
    if i.vehicle {
        return make(GrappleClass::Vehicle, false, Basis::States);
    }
    if !i.dynamic {
        return make(GrappleClass::Static, false, Basis::Physics);
    }
    match i.category.map(WeightCategory::from_authored) {
        Some(WeightCategory::Light) => make(GrappleClass::LightProp, true, Basis::Authored),
        Some(WeightCategory::Heavy) => make(GrappleClass::HeavyProp, false, Basis::Authored),
        _ => {
            let light = i
                .mass
                .is_some_and(|m| m.is_finite() && m > 0.0 && m <= FALLBACK_LIGHT_MASS);
            make(GrappleClass::Unclassified, light, Basis::Fallback)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(category: Option<u8>, mass: Option<f32>) -> ClassInput {
        ClassInput {
            dynamic: true,
            category,
            mass,
            ..default()
        }
    }

    #[test]
    fn representative_fixtures() {
        // an explosive barrel / gas canister: authored 1, ~80 kg
        let barrel = classify(prop(Some(1), Some(80.0)));
        assert_eq!(
            (barrel.class, barrel.pull, barrel.basis),
            (GrappleClass::LightProp, true, Basis::Authored)
        );
        // a soldier: no category key at all
        let soldier = classify(ClassInput {
            character: true,
            dynamic: true,
            mass: Some(80.0),
            ..default()
        });
        assert_eq!(
            (soldier.class, soldier.pull, soldier.basis),
            (GrappleClass::Character, true, Basis::States)
        );
        // light and heavy vehicles are both anchors, whatever their mass
        for mass in [900.0, 12000.0] {
            let v = classify(ClassInput {
                vehicle: true,
                dynamic: true,
                category: Some(0),
                mass: Some(mass),
                ..default()
            });
            assert_eq!(
                (v.class, v.pull, v.basis),
                (GrappleClass::Vehicle, false, Basis::States)
            );
        }
        // a heavy authored body stays put even when its mass is tiny
        let heavy = classify(prop(Some(2), Some(50.0)));
        assert_eq!((heavy.class, heavy.pull), (GrappleClass::HeavyProp, false));
        // a static surface
        let wall = classify(ClassInput::default());
        assert_eq!(
            (wall.class, wall.pull, wall.basis),
            (GrappleClass::Static, false, Basis::Physics)
        );
    }

    #[test]
    fn authored_category_beats_mass_both_ways() {
        assert!(classify(prop(Some(1), Some(5000.0))).pull);
        assert!(classify(prop(Some(1), None)).pull);
        assert!(!classify(prop(Some(2), Some(1.0))).pull);
    }

    #[test]
    fn a_body_that_cannot_move_is_never_pulled() {
        let fixed_light = classify(ClassInput {
            dynamic: false,
            category: Some(1),
            mass: Some(10.0),
            ..default()
        });
        assert_eq!(
            (fixed_light.class, fixed_light.pull),
            (GrappleClass::Static, false)
        );
    }

    #[test]
    fn fallback_boundaries_and_missing_data() {
        let at = |m| classify(prop(None, Some(m)));
        assert!(
            at(FALLBACK_LIGHT_MASS).pull,
            "the threshold itself is light"
        );
        assert!(!at(FALLBACK_LIGHT_MASS + 0.01).pull);
        assert!(at(0.01).pull);
        for m in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            assert!(!at(m).pull, "{m} kg is not a usable mass");
        }
        // missing mass: anchor, and flagged as the unknown rule
        let none = classify(prop(None, None));
        assert_eq!(
            (none.class, none.pull, none.basis),
            (GrappleClass::Unclassified, false, Basis::Fallback)
        );
        // category 0 and unknown values carry no weight information
        for c in [0u8, 3, 7, 127] {
            let r = classify(prop(Some(c), Some(100.0)));
            assert_eq!(
                (r.class, r.pull, r.basis),
                (GrappleClass::Unclassified, true, Basis::Fallback),
                "category {c}"
            );
            assert!(!classify(prop(Some(c), Some(900.0))).pull);
        }
    }

    #[test]
    fn category_values() {
        assert_eq!(WeightCategory::from_authored(0), WeightCategory::Default);
        assert_eq!(WeightCategory::from_authored(1), WeightCategory::Light);
        assert_eq!(WeightCategory::from_authored(2), WeightCategory::Heavy);
        assert_eq!(WeightCategory::from_authored(9), WeightCategory::Other(9));
    }
}
