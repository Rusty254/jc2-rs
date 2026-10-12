//! Bullet impact effects per surface (board task 2974c809, FX-GUN-2; research
//! `research/formats/weapon-fire-effects.md` sections 1 and 3).
//!
//! The weapon type's impact id (`weapons.bin` `#469ba70d`, type +0x2e4) names an
//! `effectsettings.bin` effect. The effect's own interaction matrix picks the particle, light,
//! decal and sound variant by the (flag, material) pair the hit hands it: water = `(0, 1)`, ground
//! = `(1, surface material id)` (native `CBullet` terrain hit 0x00613eb0, water hit 0x0068fec0).
//! This module only builds that pair and plays the effect through the spawner; selection, the
//! particle, light, decal, force-pulse and sound adapters live in `effects` (not touched here).
//!
//! Labelled gaps (research Gap 1, 7):
//! * the surface material id comes from the terrain tile under the hit (`audio::Ground`); a hit on
//!   a prop, building or vehicle has no collider surface id in this port, so it plays with no
//!   interaction (the effect's fallback selection `#a28df775`);
//! * hits on characters play `character_hit*` natively (selection not traced), so they are skipped;
//! * the surface record's play probability (+0x60) and ricochet (+0x5c) are unknown: every hit plays;
//! * the matrix handed to the effect has the opposite of the bullet direction as its Y axis (an
//!   inferred stand-in for the surface normal, the ray hit's normal is not read here);
//! * soldiers' shots at Rico do not ray-cast the world, so they raise no impacts.

use crate::effects::{EffectSystem, Interaction, PlayRequest};
use bevy::prelude::*;

/// A bullet hit waiting for the effect system.
#[derive(Clone, Copy, Debug)]
pub struct ImpactShot {
    /// `weapons.bin` `#469ba70d` (never -1 here).
    pub effect: u32,
    pub point: Vec3,
    /// The bullet's direction.
    pub direction: Vec3,
    /// The collider hit is the terrain, so its tile material id applies.
    pub terrain: bool,
    /// The water hit: the pair `(0, 1)`.
    pub water: bool,
}

/// Where the segment `from -> to` goes through the sea surface from above, if it does.
/// Non-native: `SEA_LEVEL` is the port's single water plane.
pub fn water_crossing(from: Vec3, to: Vec3) -> Option<Vec3> {
    let sea = crate::world::SEA_LEVEL;
    if from.y <= sea || to.y >= sea {
        return None;
    }
    let t = (from.y - sea) / (from.y - to.y);
    Some(from.lerp(to, t))
}

/// The interaction pair for a hit; `surface` is the terrain material (`None` when unknown).
pub fn interaction_for(shot: &ImpactShot, surface: Option<u32>) -> Option<Interaction> {
    if shot.water {
        Some(Interaction::WATER)
    } else if shot.terrain {
        surface.map(Interaction::surface)
    } else {
        None
    }
}

/// The matrix of the played effect: at the hit, Y against the bullet (inferred normal).
pub fn impact_matrix(shot: &ImpactShot) -> Mat4 {
    let normal = if shot.water {
        Vec3::Y
    } else {
        -shot.direction.normalize_or(Vec3::NEG_Y)
    };
    Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, normal), shot.point)
}

/// Drains `Arsenal::impact_shots` into the effect system.
pub fn drive_impact_effects(
    arsenal: Option<ResMut<crate::combat::Arsenal>>,
    fx: Option<ResMut<EffectSystem>>,
    mut ground: Option<ResMut<crate::audio::Ground>>,
    game: Option<Res<crate::Game>>,
) {
    let Some(mut a) = arsenal else { return };
    let shots = std::mem::take(&mut a.impact_shots);
    let Some(mut fx) = fx else { return };
    for shot in &shots {
        let surface = match (shot.terrain, ground.as_mut(), game.as_ref()) {
            (true, Some(g), Some(game)) => g.surface_at(&game.fs, shot.point).map(|s| s.material),
            _ => None,
        };
        let mut req = PlayRequest::new(shot.effect).at(impact_matrix(shot));
        if let Some(i) = interaction_for(shot, surface) {
            req = req.interaction(i);
        }
        fx.play(req);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(terrain: bool, water: bool) -> ImpactShot {
        ImpactShot {
            effect: 36,
            point: Vec3::new(1.0, 2.0, 3.0),
            direction: Vec3::NEG_Z,
            terrain,
            water,
        }
    }

    #[test]
    fn pair_is_water_ground_or_none() {
        assert_eq!(
            interaction_for(&shot(false, true), Some(8)),
            Some(Interaction::WATER)
        );
        assert_eq!(
            interaction_for(&shot(true, false), Some(8)),
            Some(Interaction::surface(8))
        );
        assert_eq!(interaction_for(&shot(true, false), None), None);
        assert_eq!(interaction_for(&shot(false, false), Some(8)), None);
    }

    #[test]
    fn water_crossing_only_from_above() {
        let s = crate::world::SEA_LEVEL;
        let p = water_crossing(
            Vec3::new(0.0, s + 10.0, 0.0),
            Vec3::new(10.0, s - 10.0, 0.0),
        )
        .unwrap();
        assert!((p.y - s).abs() < 1e-4 && (p.x - 5.0).abs() < 1e-4);
        assert!(water_crossing(Vec3::new(0.0, s - 1.0, 0.0), Vec3::new(0.0, s - 5.0, 0.0)).is_none());
        assert!(water_crossing(Vec3::new(0.0, s + 1.0, 0.0), Vec3::new(0.0, s + 5.0, 0.0)).is_none());
    }

    #[test]
    fn matrix_sits_at_the_hit_and_faces_the_shooter() {
        let m = impact_matrix(&shot(true, false));
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5);
        assert!((m.transform_vector3(Vec3::Y) - Vec3::Z).length() < 1e-5);
    }

    /// The assault rifle's impact id plays on metal and water and spawns particle emitters
    /// (needs JC2_TEST_DIR).
    #[test]
    fn retail_impacts_spawn_per_surface() {
        use crate::effects::particle::ParticleManager;
        use crate::effects::{EffectHost, default_adapters};
        use std::sync::Arc;
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        let weapons =
            jc2_formats::weapons::parse(&fs.read_inflated("weapons.bin").unwrap()).unwrap();
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
        let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
        let w = weapons.iter().find(|w| w.id == 11).unwrap();
        assert_eq!(w.impact_effect, 36);
        let mut counts = Vec::new();
        for (terrain, water, surface) in [(true, false, 8), (true, false, 1), (false, true, 0)] {
            let s = shot(terrain, water);
            let before = host.particles.lock().unwrap().emitter_count();
            let mut req = PlayRequest::new(w.impact_effect as u32).at(impact_matrix(&s));
            if let Some(i) = interaction_for(&s, Some(surface)) {
                req = req.interaction(i);
            }
            assert!(fx.play(req).is_some());
            for _ in 0..3 {
                fx.tick(1.0 / 60.0);
            }
            counts.push(host.particles.lock().unwrap().emitter_count() - before);
        }
        // metal and water have retail particle systems; soil medium may be missing and is skipped
        assert!(counts[0] > 0, "metal: {counts:?}");
        assert!(counts[2] > 0, "water: {counts:?}");
    }
}
