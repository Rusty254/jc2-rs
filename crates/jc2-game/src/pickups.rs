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
use crate::identity::{
    Books, Event, IdentitySynced, ObjectLedger, ObjectRef, Outcome, Reward, Table,
};
use crate::npc::Npc;
use crate::rbm_loader::RbmAsset;
use crate::world::Streamer;
use crate::world::{GameplaySourceKey, SourceOrigin};
use bevy::prelude::*;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::pickups::{PickupEntity, PickupKind, ResourceKind, entity_models};
use jc2_formats::placement::bare_name;
use jc2_formats::sarc::SmallArchive;
use std::collections::{HashMap, HashSet};

/// How close (m, on the ground plane) Rico's middle comes to a pickup to take it.
const REACH: f32 = 1.4;
/// Dropped weapons lie this long (s).
const DROP_SECONDS: f32 = 90.0;
/// Magazines in a dropped gun without a `pickup.<weapon>.ee` of its own.
const DEFAULT_COUNT: i32 = 1;

/// Something to pick up.
#[derive(Component, Clone, Debug)]
pub struct Pickup {
    pub kind: PickupKind,
    /// Magazines (guns) or items (grenades); ≤ 0 counts as one.
    pub count: i32,
    /// The durable object it is (a stash or resource crate of a registered source set,
    /// `identity.rs`): taking it goes through the ledger, so it is paid once and stays
    /// taken across unload/reload. Dropped guns have none.
    object: Option<ObjectRef>,
    /// Seconds left for a dropped gun.
    life: Option<f32>,
    /// A resource crate: the settlement it counts for.
    counter: Option<String>,
}

impl Pickup {
    #[cfg(test)]
    pub fn weapon(name: &str, count: i32) -> Self {
        Self {
            kind: PickupKind::Weapon(name.to_string()),
            count,
            object: None,
            life: None,
            counter: None,
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
    /// The box model of each resource entity (`None`: none found).
    boxes: HashMap<String, Option<String>>,
    /// Taken so far, for the tests.
    pub collected: u32,
}

pub struct PickupPlugin;

impl Plugin for PickupPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::identity::IdentityPlugin>() {
            app.add_plugins(crate::identity::IdentityPlugin);
        }
        app.init_resource::<Pickups>()
            .add_systems(Startup, read_pickups)
            .add_systems(
                Update,
                (spawn_stashes, drop_weapons, dress_pickups, collect, expire)
                    .chain()
                    .after(crate::locations::LocationReconciled)
                    .after(IdentitySynced)
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
    ledger: Res<ObjectLedger>,
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
            // an object the ledger does not know (its source set was refused) or that is
            // already taken does not appear
            let object = ObjectRef::new(source, Table::Stash, i as u32);
            if ledger.0.fact_of(object).is_none_or(|f| f.collected) {
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
                        object: Some(object),
                        life: None,
                        counter: None,
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
            let object = ObjectRef::new(source, Table::Resource, i as u32);
            if ledger.0.fact_of(object).is_none_or(|f| f.collected) {
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
                        object: Some(object),
                        life: None,
                        counter: r.counter.clone(),
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
                    object: None,
                    life: Some(DROP_SECONDS),
                    counter: None,
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

/// A gun or grenade into Rico's arsenal; false when he cannot take it.
fn into_arsenal(arsenal: &mut Arsenal, assets: &AssetServer, kind: &PickupKind, n: u32) -> bool {
    match kind {
        PickupKind::Weapon(w) => {
            let rounds = n * arsenal.magazine_of(w);
            arsenal.pick_up(Some(assets), w, rounds).is_some()
        }
        PickupKind::Item(lod) => {
            // the grenade (`wea33`); the triggered charge is not a weapon here yet
            lod.to_ascii_lowercase().contains("wea33") && arsenal.pick_up_grenades(n) > 0
        }
        PickupKind::Resource(_) => false,
    }
}

/// What a resource crate puts in the wallet (`Wallet::take`).
fn crate_reward(kind: ResourceKind, counter: Option<String>) -> Reward {
    let n = kind.amount();
    let mut r = Reward {
        settlement: counter,
        ..Default::default()
    };
    match kind {
        ResourceKind::Weapon => r.weapon_parts = n,
        ResourceKind::Vehicle => r.vehicle_parts = n,
        ResourceKind::Armor => r.armor_parts = n,
        ResourceKind::Cash => r.money = n,
    }
    r
}

/// A resource crate taken: the wallet and the settlement counter change with the ledger
/// or not at all (`Ledger::commit`).
fn take_resource(
    ledger: &mut crate::identity::Ledger,
    object: ObjectRef,
    kind: ResourceKind,
    counter: Option<String>,
    wallet: Option<&mut Wallet>,
    progress: Option<&mut Progress>,
) -> Outcome {
    let reward = crate_reward(kind, counter);
    let mut books = Books {
        chaos: None,
        wallet,
        progress,
    };
    ledger.commit(object, Event::Collect, Some((&reward, &mut books)), || true)
}

/// Rico walks over a pickup and takes it. Stashes and resource crates are durable
/// objects: the take goes through the ledger (`identity.rs`), which pays the wallet and
/// the settlement counter atomically and only once, however often or in whatever order
/// the pickup and its tile come and go.
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
    mut ledger: ResMut<ObjectLedger>,
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
        // `Some(true)`: taken now; `Some(false)`: already taken before (clean up only)
        let took = match (p.object, &p.kind) {
            (Some(h), PickupKind::Resource(kind)) => {
                match take_resource(
                    &mut ledger.0,
                    h,
                    *kind,
                    p.counter.clone(),
                    wallet.as_deref_mut(),
                    progress.as_deref_mut(),
                ) {
                    Outcome::Applied => {
                        info!("rico takes a {kind:?} crate (+{})", kind.amount());
                        Some(true)
                    }
                    Outcome::AlreadyDone => Some(false),
                    Outcome::Stale | Outcome::Rejected(_) => None,
                }
            }
            (Some(h), kind) => {
                match ledger.0.commit(h, Event::Collect, None, || {
                    into_arsenal(&mut arsenal, &assets, kind, n)
                }) {
                    Outcome::Applied => Some(true),
                    Outcome::AlreadyDone => Some(false),
                    Outcome::Stale | Outcome::Rejected(_) => None,
                }
            }
            (None, PickupKind::Resource(kind)) => match wallet.as_mut() {
                Some(w) => {
                    let got = w.take(*kind);
                    info!("rico takes a {kind:?} crate (+{got})");
                    Some(true)
                }
                None => None,
            },
            (None, kind) => into_arsenal(&mut arsenal, &assets, kind, n).then_some(true),
        };
        let Some(took) = took else { continue };
        if took {
            pickups.collected += 1;
        }
        match p.object {
            Some(h) if h.table == Table::Stash => {
                // the crate stays, empty
                for c in children.into_iter().flatten() {
                    if models.contains(*c) {
                        commands.entity(*c).despawn();
                    }
                }
                commands.entity(e).remove::<Pickup>();
            }
            _ => commands.entity(e).despawn(),
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
                object: None,
                life: None,
                counter: None,
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

    /// Headless, tile 31,14 (eight resource crates): a crate pays its part once however
    /// often it is taken; after the tile unloads and reloads (a new source generation) the
    /// same crate is still taken and pays nothing, the old generation's handle is stale,
    /// and the settlement count does not move. Needs the tile's test data.
    #[test]
    fn resource_crates_pay_once_across_unload_and_reload() {
        use crate::chaos::ChaosPlugin;
        use crate::world::FlyCamera;
        use bevy::ecs::system::RunSystemOnce;
        use jc2_formats::terrain::TILE_SIZE;
        let Some(mut app) = crate::world::tests::tile_app(|app| {
            app.add_plugins(ChaosPlugin);
        }) else {
            return;
        };
        let (key, index, kind, counter) = {
            let st = app.world().resource::<Streamer>();
            let t = &st.loaded[&(31, 14)];
            assert!(!t.resources.is_empty(), "the village has resource crates");
            let r = &t.resources[0];
            (t.source_key.unwrap(), 0u32, r.kind, r.counter.clone())
        };
        let object = ObjectRef::new(key, Table::Resource, index);
        let take = |app: &mut App, object: ObjectRef| {
            let counter = counter.clone();
            app.world_mut()
                .run_system_once(
                    move |mut ledger: ResMut<ObjectLedger>,
                          mut wallet: ResMut<Wallet>,
                          mut progress: ResMut<Progress>| {
                        take_resource(
                            &mut ledger.0,
                            object,
                            kind,
                            counter.clone(),
                            Some(&mut wallet),
                            Some(&mut progress),
                        )
                    },
                )
                .unwrap()
        };
        let totals = |app: &App| {
            let w = app.world().resource::<Wallet>().clone();
            let done = counter
                .as_deref()
                .map_or(0, |c| app.world().resource::<Progress>().done(c));
            (w, done)
        };
        let start = totals(&app);
        assert_eq!(take(&mut app, object), Outcome::Applied);
        let paid = totals(&app);
        assert_ne!(paid.0, start.0, "the crate paid");
        assert_eq!(take(&mut app, object), Outcome::AlreadyDone);
        assert_eq!(totals(&app), paid);
        // away and back
        let mut cams = app
            .world_mut()
            .query_filtered::<&mut Transform, With<FlyCamera>>();
        cams.single_mut(app.world_mut()).unwrap().translation.x += 3.0 * TILE_SIZE;
        for _ in 0..4 {
            app.update();
        }
        assert!(
            !app.world()
                .resource::<Streamer>()
                .loaded
                .contains_key(&(31, 14))
        );
        assert!(
            !app.world().resource::<ObjectLedger>().0.has_source(key.0),
            "the unloaded generation's handles are released"
        );
        cams.single_mut(app.world_mut()).unwrap().translation.x -= 3.0 * TILE_SIZE;
        let started = std::time::Instant::now();
        while !app
            .world()
            .resource::<Streamer>()
            .loaded
            .contains_key(&(31, 14))
        {
            app.update();
            assert!(started.elapsed().as_secs() < 60);
        }
        app.update();
        let new_key = app.world().resource::<Streamer>().loaded[&(31, 14)]
            .source_key
            .unwrap();
        assert_ne!(new_key, key);
        let again = ObjectRef::new(new_key, Table::Resource, index);
        assert!(
            app.world()
                .resource::<ObjectLedger>()
                .0
                .fact_of(again)
                .unwrap()
                .collected,
            "the reloaded crate is already taken"
        );
        assert_eq!(take(&mut app, again), Outcome::AlreadyDone);
        assert_eq!(take(&mut app, object), Outcome::Stale, "old token");
        assert_eq!(totals(&app), paid, "nothing was paid again");
        // another crate of the tile is a different object and still pays
        if app.world().resource::<Streamer>().loaded[&(31, 14)]
            .resources
            .len()
            > 1
        {
            let other = ObjectRef::new(new_key, Table::Resource, 1);
            assert_eq!(take(&mut app, other), Outcome::Applied);
        }
    }
}
