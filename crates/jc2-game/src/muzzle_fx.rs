//! Muzzle flash effects (board task 446ad726, FX-GUN-1; research
//! `research/formats/weapon-fire-effects.md` sections 1 and 2).
//!
//! Native 0x006fae80: when the weapon type's muzzle id (`weapons.bin` key `#04374f9d`, type
//! +0x2e8) is not -1, a `shared_ptr<IEffectController>` is created once from that id (weapon+0xe10)
//! and every shot restarts it with the transform of the emission frame. This module keeps that
//! shape: one controller per weapon (the player's gun slot, a soldier's own gun), restarted on each
//! shot through the effect spawner (`effects::EffectSystem`).
//!
//! Hand guns carry no muzzle light (retail `muzzle_*` graphs 115/255/432 only); the pistol ground
//! light is the unresolved `bullet_fire` producer (research Gap 2) and is not invented here.

use crate::effects::{EffectController, EffectSystem};
use bevy::prelude::*;
use std::collections::HashMap;

/// Which weapon fired: the controller's owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MuzzleOwner {
    /// One of Rico's guns (index into `Arsenal::guns`).
    Player(usize),
    /// A soldier (his own gun).
    Soldier(Entity),
}

/// A trigger pull waiting for the effect system.
#[derive(Clone, Copy, Debug)]
pub struct MuzzleShot {
    pub owner: MuzzleOwner,
    /// `weapons.bin` `#04374f9d` (never -1 here).
    pub effect: u32,
    /// The emission frame (`weapon_attach::emission_frame`).
    pub origin: Vec3,
    pub direction: Vec3,
}

/// The matrix handed to the effect: the emission frame's origin and barrel direction (-Z), yaw/pitch
/// only. The native matrix (weapon+0x164, 0x00631d00) is the weapon's own frame; its roll and which
/// effect-local axis the leaf graphs treat as forward are unresolved (inferred: the flash leaves are
/// billboards, so only the shell and smoke drift depends on it).
pub fn shot_matrix(origin: Vec3, direction: Vec3) -> Mat4 {
    let rot = Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize_or(Vec3::NEG_Z));
    Mat4::from_rotation_translation(rot, origin)
}

/// The persistent controllers.
#[derive(Resource, Default)]
pub struct MuzzleControllers {
    held: HashMap<MuzzleOwner, (u32, EffectController)>,
}

impl MuzzleControllers {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.held.len()
    }
}

/// Restart `shot.owner`'s controller at the shot's frame, creating it on the first shot (0x006fae80).
/// `false` when the spawner has no such effect (the native getter returns null: no flash).
pub fn restart(held: &mut MuzzleControllers, fx: &mut EffectSystem, shot: &MuzzleShot) -> bool {
    let stale = held
        .held
        .get(&shot.owner)
        .is_some_and(|(id, _)| *id != shot.effect);
    if stale {
        held.held.remove(&shot.owner);
    }
    if let std::collections::hash_map::Entry::Vacant(v) = held.held.entry(shot.owner) {
        let Some(c) = fx.create(shot.effect) else {
            return false;
        };
        v.insert((shot.effect, c));
    }
    let (_, c) = &held.held[&shot.owner];
    fx.set_transform(c, shot_matrix(shot.origin, shot.direction));
    fx.play_controller(c, &[], true)
}

/// Drains the shots the combat systems queued (`Arsenal::muzzle_shots`) into the effect system, and
/// drops the controllers of soldiers that are gone.
pub fn drive_muzzle_effects(
    arsenal: Option<ResMut<crate::combat::Arsenal>>,
    fx: Option<ResMut<EffectSystem>>,
    mut held: ResMut<MuzzleControllers>,
    npcs: Query<(), With<crate::npc::Npc>>,
) {
    let Some(mut a) = arsenal else { return };
    // The placeholder flash stays only while there is no effect system (no retail data).
    a.native_muzzle = fx.is_some();
    let shots = std::mem::take(&mut a.muzzle_shots);
    let Some(mut fx) = fx else { return };
    held.held.retain(|k, _| match k {
        MuzzleOwner::Soldier(e) => npcs.contains(*e),
        MuzzleOwner::Player(_) => true,
    });
    for shot in &shots {
        restart(&mut held, &mut fx, shot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_puts_the_origin_and_points_minus_z_along_the_barrel() {
        let m = shot_matrix(Vec3::new(1.0, 2.0, 3.0), Vec3::X);
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5);
        assert!((m.transform_vector3(Vec3::NEG_Z) - Vec3::X).length() < 1e-5);
    }

    /// Every player gun's muzzle id restarts one persistent controller per shot and spawns
    /// emitters each time (needs JC2_TEST_DIR).
    #[test]
    fn retail_muzzle_controllers_restart_per_shot() {
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
        let mut held = MuzzleControllers::default();
        for (slot, id) in [2, 5, 11, 13, 16].into_iter().enumerate() {
            let w = weapons.iter().find(|w| w.id == id).unwrap();
            let effect = u32::try_from(w.muzzle_effect).expect("a muzzle effect");
            let shot = MuzzleShot {
                owner: MuzzleOwner::Player(slot),
                effect,
                origin: Vec3::new(slot as f32, 1.0, 0.0),
                direction: Vec3::NEG_Z,
            };
            assert!(restart(&mut held, &mut fx, &shot), "weapon {id} effect {effect}");
            for _ in 0..3 {
                fx.tick(1.0 / 60.0);
            }
            assert!(
                host.particles.lock().unwrap().emitter_count() > 0,
                "weapon {id}: no emitters"
            );
            let flash_births = |host: &EffectHost| -> u64 {
                let m = host.particles.lock().unwrap();
                m.sim_emitters()
                    .filter(|(_, e)| e.texture_member == "muzzleflashes.dds")
                    .map(|(_, e)| e.births)
                    .sum()
            };
            let before = flash_births(&host);
            assert!(before > 0, "weapon {id}: the flash leaves birth nothing");
            // let the 0.09 s emission window pass; the 1 s shell casings are still flying
            for _ in 0..12 {
                fx.tick(1.0 / 60.0);
            }
            // a second pull restarts the same controller (no new controller) and flashes again
            assert!(restart(&mut held, &mut fx, &shot));
            // the new emitters hold their births for the first job (smoke-effects.md section 4)
            fx.tick(1.0 / 60.0);
            fx.tick(1.0 / 60.0);
            assert_eq!(held.len(), slot + 1);
            // (the first flash's emitters finished and were removed meanwhile)
            assert!(flash_births(&host) > 0, "weapon {id}: no second flash");
            for _ in 0..3 {
                fx.tick(1.0 / 60.0);
            }
        }
    }
}
