//! Destruction: the placement files' damage controls (`jc2_formats::destruction`) made
//! to work. Every intact part of a destructible unit carries a `Piece`; bullets that
//! hit one and blasts that reach one take its health down, and at zero the intact parts
//! go and the `_dst` debris the file placed hidden falls out as dynamic bodies with
//! their masses, pushed out from the middle. The red barrels and anything else with an
//! explosion effect on its events blow up too.

use crate::explosion::{Blast, Blasts, detonate, falloff};
use crate::rbm_loader::RbmAsset;
use crate::world::{Piece, Streamer};
use avian3d::prelude::*;
use bevy::prelude::*;

/// What an exploding unit (a red barrel) does; its effect is not decoded.
const UNIT_BLAST: Blast = Blast {
    at: Vec3::ZERO,
    radius: 7.0,
    damage: 350.0,
    impulse: 1200.0,
    effect: Some(crate::explosion::effect_id::BARREL),
};
const DEBRIS_SPEED: f32 = 5.0;
const DEFAULT_DEBRIS_MASS: f32 = 100.0;

/// Damage to destructible pieces, collected by the guns and applied here.
#[derive(Resource, Default)]
pub struct StructureHits {
    pub queue: Vec<(Piece, f32)>,
    /// Units destroyed so far, for the HUD and the tests.
    pub destroyed: u32,
}

/// A destructible unit just went: who it was and what it counts as (`chaos.rs` awards
/// its chaos and money and counts it towards its settlement).
#[derive(Message, Clone, Debug, PartialEq)]
pub struct UnitDestroyed {
    pub source_container: String,
    pub piece: Piece,
    pub name: String,
    pub at: Vec3,
    /// `gsy.activity.typeNN`.
    pub activity: Option<u32>,
    /// The settlement it counts towards.
    pub counter: Option<String>,
    /// What its damage controls fire (`missions.rs` hands them to the mission scripts).
    pub fires: Vec<String>,
    pub mission_owner: Option<crate::missions::MissionOwner>,
}

/// Debris whose model is still loading.
#[derive(Component)]
struct DebrisModel(Handle<RbmAsset>);

pub struct DestructionPlugin;

impl Plugin for DestructionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StructureHits>()
            .add_message::<UnitDestroyed>()
            .add_systems(
                Update,
                (blast_units, destroy, dress_debris).chain().after(detonate),
            );
    }
}

/// Blasts that went off this frame reach the units around them.
fn blast_units(blasts: Res<Blasts>, st: Option<Res<Streamer>>, mut hits: ResMut<StructureHits>) {
    let Some(st) = st else { return };
    for b in &blasts.fired {
        for (_, tile, t) in st.gameplay_sources() {
            for (i, u) in t.units.iter().enumerate() {
                if u.destroyed {
                    continue;
                }
                // units are buildings and props: measure to roughly their skin
                let d = (u.center.distance(b.at) - 1.5).max(0.0);
                let k = falloff(d, b.radius);
                if k > 0.0 {
                    hits.queue.push((
                        Piece {
                            source: t.source_key,
                            tile,
                            unit: i as u32,
                        },
                        b.damage * k,
                    ));
                }
            }
        }
    }
}

fn destroy(
    mut commands: Commands,
    assets: Res<AssetServer>,
    st: Option<ResMut<Streamer>>,
    mut hits: ResMut<StructureHits>,
    mut blasts: ResMut<Blasts>,
    pieces: Query<(Entity, &Piece)>,
    mut destroyed: MessageWriter<UnitDestroyed>,
) {
    let Some(mut st) = st else {
        hits.queue.clear();
        return;
    };
    let queue = std::mem::take(&mut hits.queue);
    let mut gone = Vec::new();
    for (piece, damage) in queue {
        let Some(tile) = st.tile_for_piece_mut(piece) else {
            continue;
        };
        let Some(u) = tile.units.get_mut(piece.unit as usize) else {
            continue;
        };
        if u.destroyed {
            continue;
        }
        u.health -= damage;
        if u.health <= 0.0 {
            u.destroyed = true;
            gone.push(piece);
        }
    }
    for piece in gone {
        hits.destroyed += 1;
        for (e, p) in &pieces {
            if *p == piece {
                commands.entity(e).despawn();
            }
        }
        let tile = st.tile_for_piece_mut(piece).unwrap();
        let u = &tile.units[piece.unit as usize];
        destroyed.write(UnitDestroyed {
            source_container: tile.source_container.clone(),
            piece,
            name: u.name.clone(),
            at: u.center,
            activity: u.activity,
            counter: u.counter.clone(),
            fires: u.fires.clone(),
            mission_owner: u.mission_owner,
        });
        info!(
            "{} destroyed: {} pieces of debris{}",
            u.name,
            u.debris.len(),
            if u.explodes { ", and it blows up" } else { "" }
        );
        if u.explodes {
            blasts.queue.push(Blast {
                at: u.center + Vec3::Y * 0.5,
                ..UNIT_BLAST
            });
        }
        let center = u.center;
        let mut spawned = Vec::new();
        for (k, d) in u.debris.iter().enumerate() {
            let collider = match &d.shape {
                Some(parts) if parts.len() == 1 && parts[0].0 == Vec3::ZERO => parts[0].2.clone(),
                Some(parts) => {
                    Collider::compound(parts.iter().map(|(p, r, c)| (*p, *r, c.clone())).collect())
                }
                None => Collider::cuboid(0.6, 0.6, 0.6),
            };
            // out from the middle and up, a little different for each piece
            let out = (d.transform.translation - center).normalize_or(Vec3::X);
            let spin = (k as f32 * 2.399).sin();
            let mut e = commands.spawn((
                RigidBody::Dynamic,
                collider,
                Mass(d.mass.unwrap_or(DEFAULT_DEBRIS_MASS)),
                d.transform,
                LinearVelocity((out + Vec3::Y * 0.8) * DEBRIS_SPEED),
                AngularVelocity(Vec3::new(spin, 0.5 * spin, -spin) * 2.0),
                Visibility::default(),
                Name::new(format!("debris of {}", u.name)),
            ));
            if let Some(path) = &d.model {
                e.insert(DebrisModel(assets.load::<RbmAsset>(path.clone())));
            }
            if let Some(w) = d.grapple_weight {
                e.insert(crate::grapple::GrappleWeight(w));
            }
            if let Some(owner) = u.mission_owner {
                e.insert(owner);
            }
            spawned.push(e.id());
        }
        tile.entities.extend(spawned);
    }
}

/// Give the debris its meshes once they have loaded.
fn dress_debris(
    mut commands: Commands,
    rbms: Res<Assets<RbmAsset>>,
    assets: Res<AssetServer>,
    q: Query<(Entity, &DebrisModel)>,
) {
    for (e, m) in &q {
        if let Some(rbm) = rbms.get(&m.0) {
            commands
                .entity(e)
                .remove::<DebrisModel>()
                .with_children(|p| {
                    for b in &rbm.blocks {
                        p.spawn((Mesh3d(b.mesh.clone()), b.material_component()));
                    }
                });
        } else if assets
            .get_load_state(m.0.id())
            .is_some_and(|s| s.is_failed())
        {
            commands.entity(e).remove::<DebrisModel>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explosion::ExplosionPlugin;
    use crate::vehicle::CameraMode;

    /// Headless: shoot a red barrel in the village to zero and it goes — its intact
    /// parts despawn, its two halves fall out as debris with their models, and it
    /// blows up. Needs the tile-31,14 test data.
    #[test]
    fn a_red_barrel_blows_up() {
        let Some(mut app) = crate::world::tests::tile_app(|app| {
            app.init_resource::<CameraMode>()
                .add_plugins((ExplosionPlugin, DestructionPlugin));
        }) else {
            return;
        };
        let st = app.world().resource::<Streamer>();
        let unit = st.loaded[&(31, 14)]
            .units
            .iter()
            .position(|u| u.name == "dac.barrel.1")
            .unwrap() as u32;
        let piece = Piece {
            source: st.loaded[&(31, 14)].source_key,
            tile: (31, 14),
            unit,
        };
        let count = |app: &mut App| {
            let mut q = app.world_mut().query::<&Piece>();
            q.iter(app.world()).filter(|p| **p == piece).count()
        };
        assert!(count(&mut app) >= 1, "the barrel is in the world");
        // 86 health over its two stages: 30 is not enough
        app.world_mut()
            .resource_mut::<StructureHits>()
            .queue
            .push((piece, 30.0));
        app.update();
        assert!(count(&mut app) >= 1);
        app.world_mut()
            .resource_mut::<StructureHits>()
            .queue
            .push((piece, 60.0));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(count(&mut app), 0);
        assert!(app.world().resource::<StructureHits>().destroyed >= 1);
        assert!(app.world().resource::<Blasts>().count >= 1, "no explosion");
        let st = app.world().resource::<Streamer>();
        assert!(st.loaded[&(31, 14)].units[unit as usize].destroyed);
        // the debris, dressed in its models once they load
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 60 {
            app.update();
            let mut q = app.world_mut().query::<&DebrisModel>();
            if q.iter(app.world()).count() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut q = app.world_mut().query::<(&Name, &Children)>();
        let debris = q
            .iter(app.world())
            .filter(|(n, _)| n.as_str() == "debris of dac.barrel.1")
            .count();
        assert!(debris >= 2, "{debris} pieces of debris");
        // the authored grappling weight reaches the debris bodies. This tile's barrel
        // halves author category 0 (no weight information), so they are judged by the
        // unknown mass fallback: 50 kg is pulled. Categories 1 and 2 are covered by
        // `grapple::class` and the placement reader; no placement on this tile has them.
        let mut q = app
            .world_mut()
            .query::<(&Name, &crate::grapple::GrappleWeight, &RigidBody, &Mass)>();
        let authored: Vec<_> = q
            .iter(app.world())
            .filter(|(n, ..)| n.as_str() == "debris of dac.barrel.1")
            .map(|(_, w, rb, m)| (w.0, *rb, m.0))
            .collect();
        assert!(!authored.is_empty(), "no debris carries a GrappleWeight");
        for (w, rb, mass) in authored {
            assert_eq!(w, 0);
            let c = crate::grapple::classify(crate::grapple::ClassInput {
                dynamic: rb == RigidBody::Dynamic,
                category: Some(w),
                mass: Some(mass),
                ..default()
            });
            assert_eq!(c.basis, crate::grapple::Basis::Fallback);
            assert!(
                c.pull,
                "{mass} kg of debris should be pulled by the fallback"
            );
        }
    }
}
