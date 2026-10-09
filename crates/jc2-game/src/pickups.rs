//! Weapon pickups. The locations' weapon stashes (`CSpawnSet` +
//! `CExportedEntityComponent` → `weaponstashes\mod.stash.<weapon>.ee`: a crate with the
//! gun lying in it, `jc2_formats::pickups`) appear with their tile, and every soldier
//! drops his weapon where he falls. Rico collects one by walking over it: a gun he
//! carries tops up its reserve, a new one joins his guns (`6`–`9`), grenades add to his
//! count. How much a pickup gives is its `pickup_count` in magazines (the
//! `exported\pickups\pickup.<weapon>.ee` value for dropped guns, 1 when there is none) —
//! the unit is an assumption (`research/formats/spawning.md`).
//!
//! The locations' **resource crates** (`CResourceItem`: weapon parts, vehicle parts,
//! armour parts, cash — `jc2_formats::pickups::resource_items`) appear the same way, as
//! their `exported\pickups\pickup.boost.*.ee` box; walking over one adds a part (or
//! $2500) to the [`Wallet`] and counts towards the settlement's completion.

use crate::Game;
use crate::chaos::{Progress, Wallet};
use crate::character::{HEIGHT, Hero};
use crate::combat::{Arsenal, Dead, Gunner};
use crate::coords::mat_to_bevy;
use crate::npc::Npc;
use crate::rbm_loader::RbmAsset;
use crate::world::Streamer;
use crate::world::{GameplaySourceKey, SourceOrigin};
use bevy::prelude::*;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::pickups::{PickupEntity, PickupKind, entity_models};
use jc2_formats::placement::bare_name;
use jc2_formats::sarc::SmallArchive;
use std::collections::{HashMap, HashSet};

/// How close (m, on the ground plane) Rico's middle comes to a pickup to take it.
const REACH: f32 = 1.4;
/// Dropped weapons lie this long (s).
const DROP_SECONDS: f32 = 90.0;
/// Magazines in a dropped gun without a `pickup.<weapon>.ee` of its own.
const DEFAULT_COUNT: i32 = 1;

/// A resource crate: its tile and index in the tile's list.
type CrateKey = (String, usize);

/// Something to pick up.
#[derive(Component, Clone, Debug)]
pub struct Pickup {
    pub kind: PickupKind,
    /// Magazines (guns) or items (grenades); ≤ 0 counts as one.
    pub count: i32,
    /// The stash it came from (tile, index of the spawn), so a taken stash stays empty.
    stash: Option<CrateKey>,
    /// Seconds left for a dropped gun.
    life: Option<f32>,
    /// A resource crate: (tile, index in the tile's list), the settlement it counts for.
    resource: Option<(CrateKey, Option<String>)>,
}

impl Pickup {
    #[cfg(test)]
    pub fn weapon(name: &str, count: i32) -> Self {
        Self {
            kind: PickupKind::Weapon(name.to_string()),
            count,
            stash: None,
            life: None,
            resource: None,
        }
    }
}

/// The models of a pickup's item (hidden once it is taken).
#[derive(Component)]
struct PickupModel;

#[derive(Resource, Default)]
pub struct Pickups {
    /// `pickup_count` of each weapon's `pickup.<weapon>.ee`, by `weapons.bin` name.
    pub counts: HashMap<String, i32>,
    stashes: HashMap<String, Option<PickupEntity>>,
    tiles_done: HashSet<GameplaySourceKey>,
    spawned: Vec<(Entity, GameplaySourceKey)>,
    taken: HashSet<CrateKey>,
    /// Resource crates taken (tile, index).
    resources_taken: HashSet<CrateKey>,
    /// The box model of each resource entity (`None`: none found).
    boxes: HashMap<String, Option<String>>,
    /// Taken so far, for the tests.
    pub collected: u32,
}

pub struct PickupPlugin;

impl Plugin for PickupPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pickups>()
            .add_systems(Startup, read_pickups)
            .add_systems(
                Update,
                (spawn_stashes, drop_weapons, dress_pickups, collect, expire)
                    .chain()
                    .after(crate::locations::LocationReconciled)
                    .in_set(crate::world::GameplaySourcesReconciled),
            );
    }
}

/// The epe of an exported entity container.
fn read_entity(game: &Game, container: &str) -> Option<PickupEntity> {
    let data = game.fs.read_inflated(container).ok()?;
    let sarc = SmallArchive::parse(&data).ok()?;
    let e = sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".epe"))?;
    PickupEntity::parse(sarc.data(&data, e).ok()?).ok()?
}

/// Every `exported\pickups\pickup.*.ee` (not the crated ones): what a weapon's pickup
/// gives.
fn read_pickups(mut pickups: ResMut<Pickups>, game: Res<Game>) {
    for e in game.fs.entries() {
        let Some(name) = game.fs.names.file(e.name_hash) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.starts_with("exported\\pickups\\pickup.")
            || !lower.ends_with(".ee")
            || lower.contains(".crated.")
        {
            continue;
        }
        if let Some(p) = read_entity(&game, bare_name(&lower))
            && let PickupKind::Weapon(w) = p.item.kind
        {
            pickups.counts.entry(w).or_insert(p.item.count);
        }
    }
    info!("pickups: {} weapons with a pickup", pickups.counts.len());
}

/// A `.lod`'s model as an asset path: from the container, else `general.blz`.
fn lod_model(game: &Game, container: &str, lod: &str) -> Option<String> {
    let lod = bare_name(lod).to_ascii_lowercase();
    for home in [container, "general.blz"] {
        let Ok(data) = game.fs.read_inflated(home) else {
            continue;
        };
        let Ok(sarc) = SmallArchive::parse(&data) else {
            continue;
        };
        let Some(text) = sarc.get(&lod).and_then(|e| sarc.data(&data, e).ok()) else {
            continue;
        };
        let d = LodDescriptor::parse(&String::from_utf8_lossy(text));
        if let Some(rbm) = d.model_for_level(1) {
            return Some(format!("jc2://{home}/{}", rbm.to_ascii_lowercase()));
        }
    }
    None
}

/// The box a resource entity draws (`pickup.boost.weapon.ee` → its `pu04-a.lod` model).
fn box_model(game: &Game, container: &str) -> Option<String> {
    let data = game.fs.read_inflated(container).ok()?;
    let sarc = SmallArchive::parse(&data).ok()?;
    let e = sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".epe"))?;
    let models = entity_models(sarc.data(&data, e).ok()?).ok()?;
    models
        .iter()
        .find_map(|(lod, _)| lod_model(game, container, lod))
}

/// The model of a gun by its `weapons.bin` name (`general.blz`, like Rico's).
fn gun_model(arsenal: &Arsenal, name: &str) -> Option<String> {
    let w = arsenal.table.iter().find(|w| w.name == name)?;
    let file = bare_name(&w.model).to_ascii_lowercase();
    file.ends_with(".rbm")
        .then(|| format!("jc2://general.blz/{file}"))
}

/// Asks for a pickup's models; `dress_pickups` hangs them on when they arrive.
#[derive(Component)]
struct Dress(Vec<(Handle<RbmAsset>, Transform, bool)>);

/// The weapon stashes of tiles that just loaded; gone with their tile.
#[allow(clippy::too_many_arguments)]
fn spawn_stashes(
    mut commands: Commands,
    mut pickups: ResMut<Pickups>,
    streamer: Option<Res<Streamer>>,
    arsenal: Option<Res<Arsenal>>,
    game: Res<Game>,
    assets: Res<AssetServer>,
) {
    let (Some(st), Some(arsenal)) = (streamer, arsenal) else {
        return;
    };
    for (source, _, tile) in st.gameplay_sources() {
        if pickups.tiles_done.contains(&source) {
            continue;
        }
        pickups.tiles_done.insert(source);
        for (i, s) in tile.exported_spawns.iter().enumerate() {
            let key = (tile.source_container.clone(), i);
            if pickups.taken.contains(&key) {
                continue;
            }
            let container = bare_name(&s.entity).to_ascii_lowercase();
            let entity = pickups
                .stashes
                .entry(container.clone())
                .or_insert_with(|| read_entity(&game, &container))
                .clone();
            let Some(entity) = entity else { continue };
            let mut dress = Vec::new();
            for (lod, local) in &entity.models {
                if let Some(path) = lod_model(&game, &container, lod) {
                    dress.push((assets.load(path), mat_to_bevy(local), false));
                }
            }
            let item = match &entity.item.kind {
                PickupKind::Weapon(w) => gun_model(&arsenal, w),
                PickupKind::Item(lod) => lod_model(&game, &container, lod),
                PickupKind::Resource(_) => None,
            };
            if let Some(path) = item {
                dress.push((assets.load(path), mat_to_bevy(&entity.item.local), true));
            }
            let e = commands
                .spawn((
                    mat_to_bevy(&s.world),
                    Visibility::default(),
                    Pickup {
                        kind: entity.item.kind.clone(),
                        count: entity.item.count,
                        stash: Some(key),
                        life: None,
                        resource: None,
                    },
                    Dress(dress),
                    Name::new(format!("stash {container}")),
                ))
                .id();
            commands.entity(e).insert(SourceOrigin {
                key: source,
                container: tile.source_container.clone(),
                object: s.name.clone(),
                ordinal: i,
            });
            pickups.spawned.push((e, source));
        }
        // the resource crates
        for (i, r) in tile.resources.iter().enumerate() {
            let key = (tile.source_container.clone(), i);
            if pickups.resources_taken.contains(&key) {
                continue;
            }
            let container = bare_name(&r.entity).to_ascii_lowercase();
            let model = pickups
                .boxes
                .entry(container.clone())
                .or_insert_with(|| box_model(&game, &container))
                .clone();
            let dress = model
                .map(|m| vec![(assets.load(m), Transform::IDENTITY, true)])
                .unwrap_or_default();
            let e = commands
                .spawn((
                    mat_to_bevy(&r.world),
                    Visibility::default(),
                    Pickup {
                        kind: PickupKind::Resource(r.kind),
                        count: 1,
                        stash: None,
                        life: None,
                        resource: Some((key, r.counter.clone())),
                    },
                    Dress(dress),
                    Name::new(format!("resource {}", r.name)),
                ))
                .id();
            commands.entity(e).insert(SourceOrigin {
                key: source,
                container: tile.source_container.clone(),
                object: r.name.clone(),
                ordinal: i,
            });
            pickups.spawned.push((e, source));
        }
    }
    pickups.tiles_done.retain(|k| st.source_alive(*k));
    let mut keep = Vec::new();
    for (e, tile) in std::mem::take(&mut pickups.spawned) {
        if st.source_alive(tile) {
            keep.push((e, tile));
        } else if let Ok(mut ec) = commands.get_entity(e) {
            ec.despawn();
        }
    }
    pickups.spawned = keep;
}

/// A soldier who falls drops his gun at his feet.
#[allow(clippy::type_complexity)]
fn drop_weapons(
    mut commands: Commands,
    mut pickups: ResMut<Pickups>,
    arsenal: Option<Res<Arsenal>>,
    assets: Res<AssetServer>,
    fallen: Query<(&Transform, &Gunner, &Npc, Option<&SourceOrigin>), Added<Dead>>,
) {
    let Some(arsenal) = arsenal else { return };
    for (tf, gun, npc, origin) in &fallen {
        let Some(sg) = arsenal.soldier_guns.get(gun.gun) else {
            continue;
        };
        let name = sg.weapon.name.clone();
        let count = pickups.counts.get(&name).copied().unwrap_or(DEFAULT_COUNT);
        // beside him, on the ground, lying on its side
        let side = Quat::from_rotation_y(npc.facing) * Vec3::X * 0.6;
        let at = tf.translation + side - Vec3::Y * (HEIGHT / 2.0 - 0.06);
        let dress = gun_model(&arsenal, &name)
            .map(|p| {
                vec![(
                    assets.load(p),
                    Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                    true,
                )]
            })
            .unwrap_or_default();
        let e = commands
            .spawn((
                Transform::from_translation(at).with_rotation(Quat::from_rotation_y(npc.facing)),
                Visibility::default(),
                Pickup {
                    kind: PickupKind::Weapon(name.clone()),
                    count,
                    stash: None,
                    life: Some(DROP_SECONDS),
                    resource: None,
                },
                Dress(dress),
                Name::new(format!("dropped {name}")),
            ))
            .id();
        if let Some(origin) = origin {
            commands.entity(e).insert(origin.clone());
            pickups.spawned.push((e, origin.key));
        }
    }
}

/// Hang the models on as they arrive.
fn dress_pickups(
    mut commands: Commands,
    rbms: Res<Assets<RbmAsset>>,
    assets: Res<AssetServer>,
    mut q: Query<(Entity, &mut Dress)>,
) {
    for (e, mut dress) in &mut q {
        dress.0.retain(|(h, tf, item)| {
            let Some(rbm) = rbms.get(h) else {
                // keep waiting unless it failed
                return !matches!(
                    assets.get_load_state(h.id()),
                    Some(bevy::asset::LoadState::Failed(_))
                );
            };
            for b in &rbm.blocks {
                let mut c = commands.spawn((Mesh3d(b.mesh.clone()), b.material_component(), *tf));
                if *item {
                    c.insert(PickupModel);
                }
                let id = c.id();
                commands.entity(e).add_child(id);
            }
            false
        });
        if dress.0.is_empty() {
            commands.entity(e).remove::<Dress>();
        }
    }
}

/// Rico walks over a pickup and takes it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn collect(
    mut commands: Commands,
    mut pickups: ResMut<Pickups>,
    arsenal: Option<ResMut<Arsenal>>,
    assets: Res<AssetServer>,
    heroes: Query<(&Hero, &Transform), Without<Dead>>,
    q: Query<(Entity, &Pickup, &GlobalTransform, Option<&Children>)>,
    models: Query<(), With<PickupModel>>,
    mut wallet: Option<ResMut<Wallet>>,
    mut progress: Option<ResMut<Progress>>,
) {
    let Some(mut arsenal) = arsenal else { return };
    let Ok((hero, htf)) = heroes.single() else {
        return;
    };
    if hero.in_vehicle.is_some() {
        return;
    }
    let me = htf.translation;
    for (e, p, gtf, children) in &q {
        let at = gtf.translation();
        if Vec2::new(at.x - me.x, at.z - me.z).length() > REACH || (at.y - me.y).abs() > HEIGHT {
            continue;
        }
        let n = p.count.max(1) as u32;
        let took = match &p.kind {
            PickupKind::Weapon(w) => {
                let rounds = n * arsenal.magazine_of(w);
                arsenal.pick_up(Some(&assets), w, rounds).is_some()
            }
            PickupKind::Item(lod) => {
                // the grenade (`wea33`); the triggered charge is not a weapon here yet
                lod.to_ascii_lowercase().contains("wea33") && arsenal.pick_up_grenades(n) > 0
            }
            PickupKind::Resource(kind) => match wallet.as_mut() {
                Some(w) => {
                    let got = w.take(*kind);
                    info!("rico takes a {kind:?} crate (+{got})");
                    true
                }
                None => false,
            },
        };
        if !took {
            continue;
        }
        pickups.collected += 1;
        if let Some((key, counter)) = &p.resource {
            pickups.resources_taken.insert(key.clone());
            if let (Some(id), Some(pr)) = (counter, progress.as_mut()) {
                pr.count(id, format!("crate {} {}", key.0, key.1));
            }
            commands.entity(e).despawn();
            continue;
        }
        match &p.stash {
            Some(key) => {
                // the crate stays, empty
                pickups.taken.insert(key.clone());
                for c in children.into_iter().flatten() {
                    if models.contains(*c) {
                        commands.entity(*c).despawn();
                    }
                }
                commands.entity(e).remove::<Pickup>();
            }
            None => commands.entity(e).despawn(),
        }
    }
}

/// Dropped guns go after a while.
fn expire(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Pickup)>) {
    for (e, mut p) in &mut q {
        if let Some(l) = p.life.as_mut() {
            *l -= time.delta_secs();
            if *l <= 0.0 {
                commands.entity(e).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatPlugin;

    /// Headless: Rico steps onto a Beretta pickup (2 magazines) and his reserve fills by
    /// 24; onto a revolver and it joins his guns; onto a grenade pickup. Needs
    /// JC2_TEST_DIR with Rico and the weapon tables.
    #[test]
    fn walking_over_pickups() {
        use crate::character::CharacterPlugin;
        use crate::world::FlyCamera;
        use avian3d::prelude::*;
        use bevy::input::mouse::AccumulatedMouseMotion;
        let Some((mut app, _, spawn)) = crate::fleet::testing::app(
            &[
                "mc01_lod1-rico.rbm",
                "weapons.bin",
                "weapon_upgrades.bin",
                "ammunition.bin",
            ],
            |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins((CharacterPlugin, CombatPlugin));
                app.world_mut().spawn((
                    Transform::from_xyz(-250.0, 0.0, -8955.0),
                    FlyCamera {
                        yaw: 0.0,
                        pitch: 0.0,
                        speed: 1.0,
                    },
                ));
            },
        ) else {
            return;
        };
        // Rico appears once his model has loaded on the asset threads, which takes longer
        // when the whole suite runs in parallel: wait by the clock, then let him settle.
        crate::fleet::testing::wait_for_hero(&mut app);
        for _ in 0..20 {
            app.update();
        }
        let mut q = app.world_mut().query_filtered::<&Transform, With<Hero>>();
        let rico = q.single(app.world()).unwrap().translation;
        assert!(rico.distance(spawn) < 20.0, "{rico} {spawn}");
        {
            let mut a = app.world_mut().resource_mut::<Arsenal>();
            a.guns[0].ammo.reserve = 100;
            a.grenades = 2;
        }
        let ground = rico - Vec3::Y * (HEIGHT / 2.0);
        // three pickups in a row, 5 m apart, ahead of him
        let place = |i: f32| ground + Vec3::new(5.0 * i, 0.05, 0.0);
        app.world_mut().spawn((
            Transform::from_translation(place(1.0)),
            Pickup::weapon("Beretta Handgun", 2),
        ));
        app.world_mut().spawn((
            Transform::from_translation(place(2.0)),
            Pickup::weapon("Revolver", 1),
        ));
        app.world_mut().spawn((
            Transform::from_translation(place(3.0)),
            Pickup {
                kind: PickupKind::Item("weapons\\wea33\\wea33-wea33.lod".into()),
                count: 5,
                stash: None,
                life: None,
                resource: None,
            },
        ));
        app.update();
        // nothing within reach yet
        assert_eq!(app.world().resource::<Pickups>().collected, 0);
        // he walks along the row
        for i in 1..=3 {
            let to = place(i as f32) + Vec3::Y * (HEIGHT / 2.0);
            let mut q = app
                .world_mut()
                .query_filtered::<(&mut Transform, &mut Position), With<Hero>>();
            let (mut tf, mut pos) = q.single_mut(app.world_mut()).unwrap();
            tf.translation = to;
            pos.0 = to;
            for _ in 0..4 {
                app.update();
            }
        }
        let a = app.world().resource::<Arsenal>();
        assert_eq!(a.guns[0].ammo.reserve, 124, "two Beretta magazines");
        let r = a
            .guns
            .iter()
            .find(|g| g.weapon.name == "Revolver")
            .expect("the revolver joined his guns");
        assert_eq!(a.guns.len(), 6);
        assert_eq!(r.ammo.magazine, r.ammo.magazine_size);
        assert_eq!(
            r.ammo.reserve, r.ammo.magazine_size,
            "one magazine in reserve"
        );
        assert_eq!(a.grenades, 7);
        assert_eq!(app.world().resource::<Pickups>().collected, 3);
        let mut left = app.world_mut().query::<&Pickup>();
        assert_eq!(left.iter(app.world()).count(), 0);
    }
}
