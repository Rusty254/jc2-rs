//! Shared location geometry and gameplay source lifetime. Readiness is geometry only;
//! registry bounds do not certify mission teams, scripts, audio or cutscenes.
use crate::{
    Game, scenery,
    world::{self, LoadedTile, Streamer, WorldConfig},
};
use bevy::prelude::*;
use jc2_formats::location::{LocationRegistry, canonical};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum LocationOwner {
    Tile((u32, u32)),
    /// A loaded terrain tile reaching a non-city registry location: geometry only
    /// (its actors, scripts and events are not activated here; JC2-111).
    Scenery((u32, u32)),
    ExtraTile((u32, u32)),
    Mission(crate::missions::MissionOwner),
    Giver(u64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationState {
    Accepted,
    Registering,
    Ready,
    Failed(String),
    Unloading,
}
struct Record {
    owners: HashSet<LocationOwner>,
    anchor: (u32, u32),
    container: String,
    generation: u64,
    state: LocationState,
}
#[derive(Resource, Default)]
pub struct Locations {
    pub registry: LocationRegistry,
    records: HashMap<String, Record>,
    next_generation: u64,
}
impl Locations {
    /// Canonical full paths prevent distinct same-basename archives sharing a ticket.
    #[allow(dead_code)]
    pub fn request(&mut self, owner: LocationOwner, container: &str, anchor: (u32, u32)) -> u64 {
        let identity = canonical(container);
        if let Some(r) = self.records.get_mut(&identity) {
            if r.state == LocationState::Unloading && r.owners.is_empty() {
                self.next_generation = self
                    .next_generation
                    .checked_add(1)
                    .expect("location generation exhausted");
                r.generation = self.next_generation;
            }
            r.owners.insert(owner);
            return r.generation;
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("location generation exhausted");
        let generation = self.next_generation;
        self.records.insert(
            identity.clone(),
            Record {
                owners: [owner].into_iter().collect(),
                anchor,
                container: identity,
                generation,
                state: LocationState::Accepted,
            },
        );
        generation
    }
    #[allow(dead_code)]
    pub fn release(&mut self, owner: LocationOwner) {
        for r in self.records.values_mut() {
            r.owners.remove(&owner);
        }
    }
    #[allow(dead_code)]
    pub fn state(&self, container: &str, generation: u64) -> Option<&LocationState> {
        self.records
            .get(&canonical(container))
            .filter(|r| r.generation == generation)
            .map(|r| &r.state)
    }
    #[cfg(test)]
    pub(crate) fn has_owner(&self, container: &str, owner: LocationOwner) -> bool {
        self.records
            .get(&canonical(container))
            .is_some_and(|r| r.owners.contains(&owner))
    }
}
pub struct LocationsPlugin;
impl Plugin for LocationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Locations>()
            .add_systems(Startup, load_registry)
            .add_systems(
                Update,
                reconcile
                    .in_set(LocationReconciled)
                    .after(world::stream_tiles)
                    .before(world::spawn_models),
            );
    }
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct LocationReconciled;
fn load_registry(game: Res<Game>, mut locations: ResMut<Locations>) {
    match LocationRegistry::load(&game.fs) {
        Ok(registry) => locations.registry = registry,
        Err(e) => warn!("world location registry: {e}"),
    }
}
/// Parse all placement/collision/CGD members before beginning object registration.
/// Existing actor consumers reconcile owned metadata separately. Geometry registration
/// does not certify mission team/controller/script/cutscene resource readiness.
fn parse_geometry(
    st: &mut Streamer,
    game: &Game,
    container: &str,
) -> jc2_formats::Result<Vec<jc2_formats::property::PropertyFile>> {
    let local = st
        .index
        .container(game, container)
        .ok_or_else(|| jc2_formats::Error::format("missing or invalid location archive"))?;
    let mut files = Vec::new();
    for member in &local.sarc.entries {
        let lower = member.name.to_ascii_lowercase();
        if !(lower.ends_with(".blo") || lower.ends_with(".cgd") || lower.ends_with(".pfi")) {
            continue;
        }
        let bytes = local
            .member(&member.name)
            .ok_or_else(|| jc2_formats::Error::format("unreadable location member"))?;
        let inflated = jc2_formats::compress::maybe_inflate(bytes)?;
        if lower.ends_with(".blo") {
            files.push(jc2_formats::property::PropertyFile::parse_any(&inflated)?);
        }
        if lower.ends_with(".cgd") {
            jc2_formats::cgd::CompiledGeometry::parse(&inflated)?;
        }
        if lower.ends_with(".pfi") {
            jc2_formats::collision::CollisionInstances::parse(&inflated)?;
        }
    }
    if files.len() > 1
        || local
            .sarc
            .entries
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".pfi"))
            .count()
            > 1
    {
        return Err(jc2_formats::Error::format(
            "multiple placement/collision members unsupported",
        ));
    }
    if files.is_empty() {
        return Err(jc2_formats::Error::format(
            "location has no placement member",
        ));
    }
    Ok(files)
}
#[allow(clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    cfg: Res<WorldConfig>,
    st: Option<ResMut<Streamer>>,
    mut locations: ResMut<Locations>,
    entities: Query<(Entity, Option<&world::Piece>)>,
    extra: Option<Res<world::ExtraContainers>>,
) {
    let Some(mut st) = st else { return };
    // Reconcile registry geometry against already-loaded tiles, with independent
    // owners for all intersecting tiles. Exact retail near/far margins are unknown.
    let loaded: HashSet<_> = st.loaded.keys().copied().collect();
    let mut extra_wanted = HashSet::new();
    if let Some(extra) = extra {
        for (&key, names) in &extra.0 {
            if !loaded.contains(&key) {
                continue;
            }
            for name in names {
                if let Ok(identity) = jc2_formats::location::container_identity(&game.fs, name)
                    && !st.loaded.values().any(|t| t.containers.contains(&identity))
                {
                    extra_wanted.insert((key, identity));
                }
            }
        }
    }
    for r in locations.records.values_mut() {
        r.owners.retain(|o| match o {
            LocationOwner::Tile(k) | LocationOwner::Scenery(k) => loaded.contains(k),
            LocationOwner::ExtraTile(k) => extra_wanted.contains(&(*k, r.container.clone())),
            _ => true,
        });
    }
    let mut wanted: Vec<_> = extra_wanted
        .into_iter()
        .map(|(k, n)| (LocationOwner::ExtraTile(k), n, k))
        .collect();
    for loc in &locations.registry.0 {
        // `wc/` tiles are the terrain streamer's. Every other registry location is a
        // native world location (`FUN_007c4840` registers all of them, enabled unless
        // `conditions.bin` says otherwise): key-mission bases such as the start base
        // `keymissions/km01/km01.base`, strongholds, general interests, roads...
        // City gameplay/settlement containers remain with the gameplay milestone.
        if loc.identity.starts_with("wc/") || !loc.enabled_at_start {
            continue;
        }
        let city = loc.identity.starts_with("metropolis/");
        if city && (loc.identity.contains("gameplay") || loc.identity.contains("settlement")) {
            continue;
        }
        for &key in &loaded {
            let (x, z) = jc2_formats::terrain::tile_origin(key.0, key.1);
            if loc.near_intersects_xz(
                [x, z],
                [
                    x + jc2_formats::terrain::TILE_SIZE,
                    z + jc2_formats::terrain::TILE_SIZE,
                ],
            ) {
                let name = loc.near_container();
                if st.loaded.values().any(|t| t.containers.contains(&name)) {
                    continue;
                }
                let owner = if city {
                    LocationOwner::Tile(key)
                } else {
                    LocationOwner::Scenery(key)
                };
                wanted.push((owner, name, key));
            }
        }
    }
    for (owner, name, key) in wanted {
        locations.request(owner, &name, key);
    }
    let keys: Vec<_> = locations.records.keys().cloned().collect();
    let mut began = false;
    for identity in keys {
        let record = locations.records.get_mut(&identity).unwrap();
        if record.owners.is_empty() || record.state == LocationState::Unloading {
            if let Some((_, tile)) = st.location_tiles.get_mut(&identity) {
                tile.gameplay_enabled = false;
                tile.pending.clear();
                if record.state != LocationState::Unloading {
                    for &entity in &tile.entities {
                        commands.entity(entity).try_despawn();
                    }
                    record.state = LocationState::Unloading;
                    continue;
                }
                if tile.entities.iter().any(|e| entities.contains(*e)) {
                    continue;
                }
            }
            if let Some((_, tile)) = st.location_tiles.remove(&identity) {
                for path in tile.models {
                    if let Some((_, n)) = st.handles.get_mut(&path) {
                        *n -= 1;
                        if *n == 0 {
                            st.handles.remove(&path);
                        }
                    }
                }
            }
            st.index.forget(&record.container);
            if record.owners.is_empty() {
                locations.records.remove(&identity);
            } else {
                record.state = LocationState::Accepted;
            }
            continue;
        }
        if record.state == LocationState::Accepted && !began {
            began = true;
            if !cfg.models {
                record.state = LocationState::Failed("models disabled".into());
                continue;
            }
            if let Err(e) = parse_geometry(&mut st, &game, &record.container) {
                warn!("location {identity}: {e}");
                record.state = LocationState::Failed(e.to_string());
                continue;
            }
            let mut tile = LoadedTile::default();
            tile.source_key = Some(st.next_source());
            tile.source_container = record.container.clone();
            world::load_container(
                &mut st,
                &mut tile,
                record.anchor.0,
                record.anchor.1,
                &record.container,
                &mut commands,
                &assets,
                &game,
                &cfg,
            );
            let result = scenery::place_container_statics(
                &mut st,
                &mut tile,
                record.anchor,
                &record.container,
                &assets,
                &game,
                &cfg,
            );
            record.state = match result {
                Err(e) => LocationState::Failed(e.to_string()),
                Ok(_) if tile.unresolved > 0 || tile.collision_missing > 0 => {
                    LocationState::Failed(format!(
                        "unresolved model/collision dependencies: {} models, {} collisions; {:?}",
                        tile.unresolved, tile.collision_missing, st.unresolved
                    ))
                }
                Ok(_) => LocationState::Registering,
            };
            if let LocationState::Failed(e) = &record.state {
                warn!("location {identity}: {e}");
            }
            st.location_tiles
                .insert(identity.clone(), (record.anchor, tile));
        } else if record.state == LocationState::Registering
            && let Some((_, tile)) = st.location_tiles.get(&identity)
        {
            if tile.objects_failed > 0 {
                record.state = LocationState::Failed("model asset initialization failed".into());
            } else {
                let mut assets_ready = true;
                for path in &tile.models {
                    let Some((handle, _)) = st.handles.get(path) else {
                        record.state = LocationState::Failed("lost model reference".into());
                        break;
                    };
                    if let Some(bevy::asset::RecursiveDependencyLoadState::Failed(e)) =
                        assets.get_recursive_dependency_load_state(handle.id())
                    {
                        record.state = LocationState::Failed(format!("asset dependency: {e}"));
                        break;
                    }
                    assets_ready &= assets.is_loaded_with_dependencies(handle.id());
                }
                if record.state == LocationState::Registering
                    && assets_ready
                    && tile.pending.is_empty()
                    && tile.objects_spawned == tile.objects_total
                    && tile.entities.iter().all(|e| entities.contains(*e))
                {
                    record.state = LocationState::Ready;
                }
            }
        }
        let enable = record.state == LocationState::Ready
            && record
                .owners
                .iter()
                .any(|o| !matches!(o, LocationOwner::Giver(_) | LocationOwner::Scenery(_)));
        let renew = enable
            && st
                .location_tiles
                .get(&identity)
                .is_some_and(|(_, t)| !t.gameplay_enabled && t.gameplay_started);
        let new_key = renew.then(|| st.next_source());
        if let Some((_, tile)) = st.location_tiles.get_mut(&identity) {
            if let Some(key) = new_key {
                tile.source_key = Some(key);
                for &entity in &tile.entities {
                    if let Ok((_, Some(piece))) = entities.get(entity) {
                        commands.entity(entity).insert(world::Piece {
                            source: Some(key),
                            ..*piece
                        });
                    }
                }
            }
            tile.gameplay_enabled = enable;
            tile.gameplay_started |= enable;
        }
        if matches!(record.state, LocationState::Failed(_))
            && let Some((_, tile)) = st.location_tiles.remove(&identity)
        {
            for entity in tile.entities {
                commands.entity(entity).try_despawn();
            }
            for path in tile.models {
                if let Some((_, n)) = st.handles.get_mut(&path) {
                    *n -= 1;
                    if *n == 0 {
                        st.handles.remove(&path);
                    }
                }
            }
            st.index.forget(&record.container);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_deduplicates_and_stale_generation_cannot_observe_retry() {
        let mut locations = Locations::default();
        let a = LocationOwner::Mission(crate::missions::MissionOwner(1));
        let b = LocationOwner::Mission(crate::missions::MissionOwner(2));
        let first = locations.request(a, "LOCATIONS\\City.NLZ", (1, 2));
        assert_eq!(first, locations.request(b, "locations/city.nlz", (3, 4)));
        locations.release(a);
        assert_eq!(locations.records["locations/city.nlz"].owners.len(), 1);
        locations.release(b);
        locations
            .records
            .get_mut("locations/city.nlz")
            .unwrap()
            .state = LocationState::Unloading;
        let second = locations.request(b, "locations/city.nlz", (1, 2));
        assert_ne!(first, second);
        assert!(locations.state("locations/city.nlz", first).is_none());
        assert_eq!(
            locations.state("locations/city.nlz", second),
            Some(&LocationState::Unloading)
        );
    }
    /// Leaves the registry empty, so only the test's explicit owners load locations.
    pub(crate) fn without_registry_streaming(app: &mut App) {
        app.add_systems(
            Startup,
            (|mut l: ResMut<Locations>| l.registry.0.clear()).after(load_registry),
        );
    }
    fn wait_ready(app: &mut App, name: &str, generation: u64) {
        let started = std::time::Instant::now();
        loop {
            app.update();
            let state = app
                .world()
                .resource::<Locations>()
                .state(name, generation)
                .cloned();
            if state == Some(LocationState::Ready) {
                break;
            }
            assert!(
                !matches!(state, Some(LocationState::Failed(_))),
                "{state:?}"
            );
            assert!(
                started.elapsed().as_secs() < 90,
                "location did not register: {state:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    fn retail_city_geometry_shared_registration_cleanup_and_reload() {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return;
        }
        let mut app = world::tests::tile_app(|app| {
            app.add_plugins(LocationsPlugin);
        })
        .unwrap();
        app.world_mut()
            .resource_mut::<Locations>()
            .registry
            .0
            .clear();
        let name = "locations/metropolis/city.district.b1.houses.nlz";
        let a = LocationOwner::Mission(crate::missions::MissionOwner(81));
        let b = LocationOwner::Mission(crate::missions::MissionOwner(82));
        let generation = app
            .world_mut()
            .resource_mut::<Locations>()
            .request(a, name, (6, 29));
        assert_eq!(
            generation,
            app.world_mut()
                .resource_mut::<Locations>()
                .request(b, name, (7, 29))
        );
        assert_eq!(
            app.world().resource::<Locations>().state(name, generation),
            Some(&LocationState::Accepted)
        );
        wait_ready(&mut app, name, generation);
        let (models, entities, paths) = {
            let st = app.world().resource::<Streamer>();
            let tile = &st.location_tiles[name].1;
            assert_eq!(tile.objects_total, tile.objects_spawned);
            assert_eq!(tile.objects_failed, 0);
            assert!(
                tile.objects_spawned > 100,
                "retail houses and CGD are initialized"
            );
            let statics = tile
                .entities
                .iter()
                .filter(|e| {
                    app.world()
                        .get::<Name>(**e)
                        .is_some_and(|n| n.starts_with("static "))
                })
                .count();
            assert!(statics > 0, "extra-container CGD is actually registered");
            (
                tile.objects_total,
                tile.entities.clone(),
                tile.models.clone(),
            )
        };
        app.world_mut().resource_mut::<Locations>().release(a);
        app.update();
        assert_eq!(
            app.world().resource::<Streamer>().location_tiles[name]
                .1
                .objects_total,
            models
        );
        assert!(entities.iter().all(|e| app.world().get_entity(*e).is_ok()));
        let before: HashMap<_, _> = paths
            .iter()
            .map(|p| (p.clone(), app.world().resource::<Streamer>().handles[p].1))
            .collect();
        app.world_mut().resource_mut::<Locations>().release(b);
        app.update();
        assert_eq!(
            app.world().resource::<Locations>().state(name, generation),
            Some(&LocationState::Unloading)
        );
        app.update();
        assert!(
            app.world()
                .resource::<Locations>()
                .state(name, generation)
                .is_none()
        );
        assert!(
            !app.world()
                .resource::<Streamer>()
                .location_tiles
                .contains_key(name)
        );
        assert!(entities.iter().all(|e| app.world().get_entity(*e).is_err()));
        for (path, count) in before {
            let removed = paths.iter().filter(|p| **p == path).count();
            let retained = app
                .world()
                .resource::<Streamer>()
                .handles
                .get(&path)
                .map_or(0, |(_, n)| *n);
            assert_eq!(retained, count - removed, "shared references survive");
        }
        let retry = app
            .world_mut()
            .resource_mut::<Locations>()
            .request(a, name, (6, 29));
        assert_ne!(retry, generation);
        wait_ready(&mut app, name, retry);
        assert_eq!(
            app.world().resource::<Streamer>().location_tiles[name]
                .1
                .objects_total,
            models
        );
    }
    /// JC2-111: the start base Kem Gunung Dataran Tinggi is the registry location
    /// `keymissions/km01/km01.base`, reached by tile 29,18 (no `wc` container there).
    #[test]
    fn retail_start_base_streams_as_scenery_geometry() {
        let mut app = match world::tests::tile_app_with((29, 18), Vec::new(), |_| {}) {
            Some(app) => app,
            None => return,
        };
        let name = "locations/keymissions/km01/km01.base.nlz";
        let started = std::time::Instant::now();
        let generation = loop {
            app.update();
            let locations = app.world().resource::<Locations>();
            if let Some(r) = locations.records.get(name) {
                assert!(r.owners.contains(&LocationOwner::Scenery((29, 18))));
                break r.generation;
            }
            assert!(started.elapsed().as_secs() < 30, "start base not requested");
        };
        wait_ready(&mut app, name, generation);
        let st = app.world().resource::<Streamer>();
        let tile = &st.location_tiles[name].1;
        assert_eq!(tile.objects_failed, 0);
        assert_eq!(tile.unresolved, 0);
        assert!(!tile.gameplay_enabled, "scenery owners enable no actors");
        let statics = tile
            .entities
            .iter()
            .filter(|e| {
                app.world()
                    .get::<Name>(**e)
                    .is_some_and(|n| n.starts_with("static "))
            })
            .count();
        eprintln!(
            "km01.base: {} objects, {} CGD statics",
            tile.objects_spawned, statics
        );
        // 280 near + 246 far CGD instances + the 469 visible model placements of the .blo
        assert_eq!(tile.objects_spawned, 749 + 246);
        assert_eq!(
            statics,
            280 + 246,
            "every km01.base.nl.cgd and .fl.cgd instance"
        );
    }
    #[test]
    fn missing_location_fails_instead_of_acknowledging_loaded_terrain() {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return;
        }
        let mut app = world::tests::tile_app(|app| {
            app.add_plugins(LocationsPlugin);
        })
        .unwrap();
        let owner = LocationOwner::Mission(crate::missions::MissionOwner(9));
        let generation = app.world_mut().resource_mut::<Locations>().request(
            owner,
            "locations/absent.nlz",
            (31, 14),
        );
        app.update();
        assert!(matches!(
            app.world()
                .resource::<Locations>()
                .state("locations/absent.nlz", generation),
            Some(LocationState::Failed(_))
        ));
        assert!(
            !app.world()
                .resource::<Streamer>()
                .location_tiles
                .contains_key("locations/absent.nlz")
        );
        app.world_mut().resource_mut::<Locations>().release(owner);
        app.update();
        assert!(
            !app.world()
                .resource::<Locations>()
                .records
                .contains_key("locations/absent.nlz")
        );
    }
    #[test]
    fn late_legacy_extra_registration_loads_cgd_once() {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return;
        }
        let mut app = world::tests::tile_app(|_| {}).unwrap();
        let name = "locations/metropolis/city.district.b1.houses.nlz";
        let owner = LocationOwner::Mission(crate::missions::MissionOwner(880));
        let generation = app
            .world_mut()
            .resource_mut::<Locations>()
            .request(owner, name, (31, 14));
        app.insert_resource(world::ExtraContainers(
            [((31, 14), vec!["city.district.b1.houses.nlz".into(); 2])]
                .into_iter()
                .collect(),
        ));
        wait_ready(&mut app, name, generation);
        let after = app.world().resource::<Streamer>().location_tiles[name]
            .1
            .objects_total;
        app.update();
        assert_eq!(
            app.world().resource::<Streamer>().location_tiles[name]
                .1
                .objects_total,
            after,
            "extra and mission owners share geometry"
        );
        let t = &app.world().resource::<Streamer>().location_tiles[name].1;
        assert!(t.entities.iter().any(|e| {
            app.world()
                .get::<Name>(*e)
                .is_some_and(|n| n.starts_with("static "))
        }));
        assert!(
            app.world()
                .resource::<Locations>()
                .has_owner(name, LocationOwner::ExtraTile((31, 14)))
        );
    }
    #[test]
    fn retail_gameplay_sources_late_owners_cleanup_and_giver_reactivation() {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return;
        }
        let mut app = world::tests::tile_app_with((2, 26), Vec::new(), |app| {
            // explicit owners only: tile 2,26 would also hold f1s07.base as scenery
            without_registry_streaming(app);
            app.init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<crate::vehicle::CameraMode>()
                .init_resource::<crate::missions::WorldFlags>()
                .insert_resource(crate::fleet::Fleet::new(1, true))
                .add_plugins((
                    crate::fleet::FleetPlugin,
                    crate::npc::NpcPlugin,
                    crate::combat::CombatPlugin,
                ));
        })
        .unwrap();
        let name = jc2_formats::location::container_identity(
            &app.world().resource::<Game>().fs,
            "f1s07.base.nlz",
        )
        .unwrap();
        let giver = LocationOwner::Giver(9030);
        let mission = LocationOwner::Mission(crate::missions::MissionOwner(9030));
        let generation = app
            .world_mut()
            .resource_mut::<Locations>()
            .request(giver, &name, (2, 26));
        wait_ready(&mut app, &name, generation);
        let initial = app.world().resource::<Streamer>().location_tiles[&name]
            .1
            .source_key
            .unwrap();
        assert!(
            !app.world().resource::<Streamer>().source_alive(initial),
            "giver scenery does not enable actors"
        );
        app.world_mut()
            .resource_mut::<crate::missions::WorldFlags>()
            .fired
            .insert("f1s07.before.enabled".into());
        app.insert_resource(world::ExtraContainers(
            [((2, 26), vec!["f1s07.base.nlz".into(); 2])]
                .into_iter()
                .collect(),
        ));
        assert_eq!(
            generation,
            app.world_mut()
                .resource_mut::<Locations>()
                .request(mission, &name, (2, 26))
        );
        let started = std::time::Instant::now();
        let source_entities = loop {
            app.update();
            let st = app.world().resource::<Streamer>();
            let tile = &st.location_tiles[&name].1;
            assert!(tile.gameplay_enabled);
            assert_eq!(tile.counters.get("f1s07.CountUp"), Some(&14));
            let mut q = app.world_mut().query::<(Entity, &world::SourceOrigin)>();
            let owned: Vec<_> = q
                .iter(app.world())
                .filter(|(_, o)| o.key == initial)
                .map(|(e, _)| e)
                .collect();
            let npcs = owned
                .iter()
                .filter(|e| app.world().get::<crate::npc::Npc>(**e).is_some())
                .count();
            let pickups = owned
                .iter()
                .filter(|e| app.world().get::<crate::pickups::Pickup>(**e).is_some())
                .count();
            let vehicles = owned
                .iter()
                .filter(|e| app.world().get::<crate::fleet::Container>(**e).is_some())
                .count();
            if npcs > 0 && pickups == 6 && vehicles == 6 {
                eprintln!(
                    "shared f1s07 gameplay: {npcs} NPCs, {pickups} pickups, {vehicles} vehicles"
                );
                break owned;
            }
            assert!(
                started.elapsed().as_secs() < 90,
                "late actors did not spawn: NPCs={npcs} pickups={pickups} vehicles={vehicles}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let geometry = app.world().resource::<Streamer>().location_tiles[&name]
            .1
            .entities
            .clone();
        let refs = app
            .world()
            .resource::<Streamer>()
            .handles
            .iter()
            .map(|(p, (_, n))| (p.clone(), *n))
            .collect::<HashMap<_, _>>();
        app.world_mut().resource_mut::<Locations>().release(mission);
        app.update();
        assert!(
            source_entities
                .iter()
                .all(|e| app.world().get_entity(*e).is_ok()),
            "extra owner retains actors"
        );
        let vehicles: Vec<_> = source_entities
            .iter()
            .copied()
            .filter(|e| app.world().get::<crate::fleet::Container>(*e).is_some())
            .collect();
        let player_vehicle = vehicles[0];
        let ai_vehicle = vehicles[1];
        app.world_mut()
            .entity_mut(player_vehicle)
            .insert(crate::vehicle::Occupied);
        app.world_mut()
            .entity_mut(ai_vehicle)
            .insert(crate::vehicle::AiDriven);
        app.world_mut()
            .resource_mut::<world::ExtraContainers>()
            .0
            .clear();
        app.update();
        assert!(!app.world().resource::<Streamer>().source_alive(initial));
        assert!(
            source_entities
                .iter()
                .all(|e| *e == player_vehicle || app.world().get_entity(*e).is_err()),
            "last gameplay owner removes ordinary actors"
        );
        assert!(
            app.world().get_entity(ai_vehicle).is_err(),
            "ordinary AI ownership does not retain a released vehicle"
        );
        assert!(
            app.world().get_entity(player_vehicle).is_ok(),
            "player-held vehicle transfers out of source ownership"
        );
        assert!(
            app.world()
                .get::<world::SourceOrigin>(player_vehicle)
                .is_none()
        );
        app.world_mut()
            .entity_mut(player_vehicle)
            .remove::<crate::vehicle::Occupied>();
        assert!(
            geometry.iter().all(|e| app.world().get_entity(*e).is_ok()),
            "giver keeps geometry"
        );
        assert_eq!(
            refs,
            app.world()
                .resource::<Streamer>()
                .handles
                .iter()
                .map(|(p, (_, n))| (p.clone(), *n))
                .collect::<HashMap<_, _>>()
        );
        app.world_mut()
            .resource_mut::<Locations>()
            .request(mission, &name, (2, 26));
        app.update();
        let next = app.world().resource::<Streamer>().location_tiles[&name]
            .1
            .source_key
            .unwrap();
        assert_ne!(next, initial);
        assert!(!app.world().resource::<Streamer>().source_alive(initial));
        assert!(app.world().resource::<Streamer>().source_alive(next));
        let (health, index) = {
            let t = &app.world().resource::<Streamer>().location_tiles[&name].1;
            let (i, u) = t
                .units
                .iter()
                .enumerate()
                .find(|(_, u)| u.health > 0.0)
                .unwrap();
            (u.health, i)
        };
        app.world_mut()
            .resource_mut::<crate::destruction::StructureHits>()
            .queue
            .push((
                world::Piece {
                    tile: (2, 26),
                    unit: index as u32,
                    source: Some(initial),
                },
                health + 1000.0,
            ));
        app.update();
        assert_eq!(
            health,
            app.world().resource::<Streamer>().location_tiles[&name]
                .1
                .units[index]
                .health,
            "stale generation damage cannot address reactivated geometry"
        );

        assert!(
            geometry
                .iter()
                .filter_map(|e| app.world().get::<world::Piece>(*e))
                .all(|p| p.source == Some(next)),
            "surviving collision and visual pieces belong to fresh generation"
        );
        app.world_mut()
            .resource_mut::<crate::destruction::StructureHits>()
            .queue
            .push((
                world::Piece {
                    tile: (2, 26),
                    unit: index as u32,
                    source: Some(next),
                },
                health + 1000.0,
            ));
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world().resource::<Streamer>().location_tiles[&name]
                .1
                .units[index]
                .destroyed,
            "fresh source routes actual damage"
        );
        let debris = app.world().resource::<Streamer>().location_tiles[&name]
            .1
            .entities
            .iter()
            .copied()
            .filter(|e| {
                app.world()
                    .get::<Name>(*e)
                    .is_some_and(|n| n.starts_with("debris of "))
            })
            .collect::<Vec<_>>();
        app.world_mut().resource_mut::<Locations>().release(mission);
        app.world_mut().resource_mut::<Locations>().release(giver);
        for _ in 0..4 {
            app.update();
        }
        assert!(
            !app.world()
                .resource::<Streamer>()
                .location_tiles
                .contains_key(&name)
        );
        assert!(geometry.iter().all(|e| app.world().get_entity(*e).is_err()));
        assert!(!app.world().resource::<Streamer>().source_alive(next));
        assert!(
            debris.iter().all(|e| app.world().get_entity(*e).is_err()),
            "release cleans live debris and tolerates already destroyed parts"
        );
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut q = app.world_mut().query::<&world::SourceOrigin>();
        assert!(
            q.iter(app.world())
                .all(|o| o.key != initial && o.key != next),
            "late asset arrivals never recreate retired actor roots"
        );
    }
    #[test]
    fn retail_extra_terrain_identity_is_not_registered_twice() {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return;
        }
        let mut app = world::tests::tile_app(|_| {}).unwrap();
        let before = app.world().resource::<Streamer>().loaded[&(31, 14)].objects_total;
        app.insert_resource(world::ExtraContainers(
            [((31, 14), vec!["31x14.nlz".into()])].into_iter().collect(),
        ));
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            before,
            app.world().resource::<Streamer>().loaded[&(31, 14)].objects_total
        );
        assert!(
            app.world()
                .resource::<Locations>()
                .records
                .values()
                .all(|r| !r.container.ends_with("/31x14.nlz"))
        );
    }
}
