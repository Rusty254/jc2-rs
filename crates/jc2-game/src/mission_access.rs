//! Shared mission catalogue, launch policy and resource/lifetime contract.
//! Retail employer/PDA routing is not decoded here. Giver/PDA callers use this same
//! service; normal requests wait for a generation-specific streaming acknowledgement.

use super::*;
use jc2_formats::property::{Node, PropertyFile, Variant};
use std::collections::{HashMap, VecDeque};

/// One run's ownership identity, including colliders, queued models and debris.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MissionOwner(pub u64);

/// Retain the original icon node: visible/enabled are raw defaults, not unlocks.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Catalogue metadata is retained for the forthcoming mission browser.
pub struct MissionGiver {
    pub id: String,
    pub position: [f32; 3],
    pub kind: i32,
    pub raw: Node,
    /// A discovered archive path, absent when the filename index has no match.
    pub container: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CatalogMission {
    pub record: MissionRecord,
    pub name: String,
    pub description: String,
    pub script_container: Option<String>,
    pub giver: Option<MissionGiver>,
}

#[derive(Debug, Default)]
pub struct MissionCatalog(pub Vec<CatalogMission>);

impl MissionCatalog {
    pub fn load(fs: &jc2_formats::GameFs, db: &MissionDb) -> Self {
        let files: HashMap<String, String> = fs
            .entries()
            .iter()
            .filter_map(|e| fs.names.file(e.name_hash))
            .map(|n| (bare_name(n).to_ascii_lowercase(), n.to_string()))
            .collect();
        let icons = fs
            .read_inflated("icons_global.bin")
            .ok()
            .and_then(|b| PropertyFile::parse_any(&b).ok());
        let mut givers = HashMap::new();
        if let Some(icons) = icons {
            for root in &icons.nodes {
                root.walk(&mut |n| {
                    let Some(id) = n.str("id") else { return };
                    let lower = id.to_ascii_lowercase();
                    let Some(mission) = lower.strip_suffix("emp") else {
                        return;
                    };
                    let Some(Variant::Vec3(position)) = n.value_hash(0x01df_c541) else {
                        return;
                    };
                    let kind = match n.value_hash(0xc9f3_6d94) {
                        Some(Variant::Int(n)) => *n,
                        _ => 0,
                    };
                    givers.insert(
                        mission.to_string(),
                        MissionGiver {
                            id: id.to_string(),
                            position: *position,
                            kind,
                            raw: n.clone(),
                            container: files.get(&format!("{lower}.nlz")).cloned(),
                        },
                    );
                });
            }
        }
        Self(
            db.table
                .iter()
                .filter(|r| playable_kind(r.kind))
                .map(|record| {
                    let id = record.id.to_ascii_lowercase();
                    let bare = format!("{id}.blz");
                    CatalogMission {
                        record: record.clone(),
                        name: db.text(&format!("{id}_name_txt"), &id),
                        description: db.text(&format!("{id}_intfile_desc_txt"), ""),
                        script_container: files
                            .get(&bare)
                            .cloned()
                            .or_else(|| fs.contains(&bare).then_some(bare)),
                        giver: givers.get(&id).cloned(),
                    }
                })
                .collect(),
        )
    }

    pub fn get(&self, id: &str) -> Option<&CatalogMission> {
        self.0.iter().find(|m| m.record.id.eq_ignore_ascii_case(id))
    }
}

fn playable_kind(kind: MissionKind) -> bool {
    matches!(
        kind,
        MissionKind::Agency
            | MissionKind::FactionMission
            | MissionKind::FactionTask
            | MissionKind::Stronghold
    )
}

/// Current jc2-rs policy, not a claim about retail replay/status-router semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    UnknownMission,
    UnsupportedKind,
    MissingScript,
    Completed,
    ActiveMission(String),
    Preparing(String),
    Prerequisite(String),
    Chaos { required: u32, current: u32 },
    InvalidDebugOverride,
    InvalidGiver,
    LoadFailed(String),
    ResourcesFailed(String),
}

pub fn mission_completed(id: &str, done: &MissionsDone, holds: &Strongholds) -> bool {
    done.0.iter().any(|m| m.eq_ignore_ascii_case(id))
        || holds
            .0
            .iter()
            .any(|s| s.phase == Phase::Taken && s.data.mission.eq_ignore_ascii_case(id))
}

#[allow(dead_code)] // Entry points are staged for the future giver/PDA integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOrigin {
    Giver { employer: String },
    Pda,
    Debug,
}

#[derive(Debug, Clone)]
pub struct MissionLaunchRequest {
    pub id: String,
    pub origin: LaunchOrigin,
    /// Only Debug can bypass prerequisite/chaos/completion. Never replaces a run,
    /// bypasses a missing script or repeats completion rewards.
    pub debug_override: bool,
}

impl MissionLaunchRequest {
    pub fn debug(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            origin: LaunchOrigin::Debug,
            debug_override: true,
        }
    }
}

pub fn availability(
    db: &MissionDb,
    request: &MissionLaunchRequest,
    chaos: u32,
    done: &MissionsDone,
    holds: &Strongholds,
    active: Option<&str>,
    preparing: Option<&str>,
) -> Availability {
    if request.debug_override && request.origin != LaunchOrigin::Debug {
        return Availability::InvalidDebugOverride;
    }
    let Some(record) = db.record(&request.id) else {
        return Availability::UnknownMission;
    };
    if !playable_kind(record.kind) {
        return Availability::UnsupportedKind;
    }
    let Some(entry) = db.catalog.get(&request.id) else {
        return Availability::UnknownMission;
    };
    if entry.script_container.is_none() {
        return Availability::MissingScript;
    }
    if let LaunchOrigin::Giver { employer } = &request.origin
        && !entry
            .giver
            .as_ref()
            .is_some_and(|g| g.id.eq_ignore_ascii_case(employer))
    {
        return Availability::InvalidGiver;
    }
    if let Some(id) = active {
        return Availability::ActiveMission(id.to_string());
    }
    if let Some(id) = preparing {
        return Availability::Preparing(id.to_string());
    }
    if !request.debug_override {
        if mission_completed(&record.id, done, holds) {
            return Availability::Completed;
        }
        if let Some(id) = record.prerequisite()
            && !mission_completed(id, done, holds)
        {
            return Availability::Prerequisite(id.to_string());
        }
        if chaos < record.chaos {
            return Availability::Chaos {
                required: record.chaos,
                current: chaos,
            };
        }
    }
    Availability::Available
}

/// A preparation manifest, not a transitive dependency inventory. A streaming service
/// must resolve locations/dependencies and acknowledge its supported contract. Merely
/// loading the script, submitting loads or seeing the start tile is insufficient.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Streaming integration consumes this preparation manifest later.
pub struct MissionPreparation {
    pub owner: MissionOwner,
    pub id: String,
    pub origin: LaunchOrigin,
    pub script_container: String,
    /// Original named completion uses the mission's .location identity. Only derive
    /// it from a discovered full archive path; a bare fallback cannot identify a location.
    pub location_name: Option<String>,
    pub start_position: Option<[f32; 3]>,
    pub giver_container: Option<String>,
    pub unsupported_classes: Vec<String>,
    pub team_spawn_sets: usize,
}

#[derive(Debug)]
struct PreparedMission {
    manifest: MissionPreparation,
    run: MissionRun,
    /// The caller asserts the supported resource closure is usable, not just queued.
    receipt: Option<Result<(), String>>,
    debug_override: bool,
    initialized_resources: bool,
    prepared_events: Vec<String>,
}

#[allow(dead_code)] // Cancellation is exposed for forthcoming gameplay/UI callers.
#[derive(Debug)]
enum LaunchCommand {
    Launch(MissionLaunchRequest),
    Cancel(MissionOwner),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchStatus {
    WaitingForResources {
        owner: MissionOwner,
        id: String,
    },
    Started {
        owner: MissionOwner,
        id: String,
        origin: LaunchOrigin,
    },
    Rejected {
        id: String,
        reason: Availability,
    },
    Cancelled {
        owner: MissionOwner,
        id: String,
    },
}

#[derive(Resource, Default, Debug)]
pub struct MissionLaunches {
    commands: VecDeque<LaunchCommand>,
    pending: Option<PreparedMission>,
    next_owner: u64,
    pub history: VecDeque<LaunchStatus>,
}

impl MissionLaunches {
    pub fn request(&mut self, request: MissionLaunchRequest) {
        self.commands.push_back(LaunchCommand::Launch(request));
    }

    #[allow(dead_code)]
    pub fn cancel(&mut self, owner: MissionOwner) {
        self.commands.push_back(LaunchCommand::Cancel(owner));
    }

    #[allow(dead_code)]
    pub fn preparation(&self) -> Option<&MissionPreparation> {
        self.pending.as_ref().map(|p| &p.manifest)
    }

    /// Only the current preparation can receive an acknowledgement. The resource
    /// owner calls this after successfully realizing its supported dependency closure.
    /// Stale/duplicate acknowledgements cannot start a subsequent retry.
    #[allow(dead_code)]
    pub fn acknowledge(&mut self, owner: MissionOwner, result: Result<(), String>) -> bool {
        let Some(p) = self.pending.as_mut().filter(|p| p.manifest.owner == owner) else {
            return false;
        };
        if p.receipt.is_some() {
            return false;
        }
        p.receipt = Some(result);
        true
    }

    fn record(&mut self, status: LaunchStatus) {
        self.history.push_back(status);
        while self.history.len() > 32 {
            self.history.pop_front();
        }
    }
}

fn report(world: &mut World, service: &mut MissionLaunches, id: String, reason: Availability) {
    warn!("mission {id} rejected: {reason:?}");
    if let Some(mut notices) = world.get_resource_mut::<Notices>() {
        notices.push(format!("Mission {id}: {reason:?}"));
    }
    service.record(LaunchStatus::Rejected { id, reason });
}

/// Runs before mission events. CLI is translated once into the same validated path.
pub(crate) fn service_launches(world: &mut World) {
    if !world.contains_resource::<MissionDb>() || !world.contains_resource::<Game>() {
        return;
    }
    let fs = world.resource::<Game>().fs.clone();
    let mut service = world
        .remove_resource::<MissionLaunches>()
        .unwrap_or_default();
    if let Some(start) = world.remove_resource::<MissionStart>() {
        service.request(MissionLaunchRequest::debug(start.0));
    }
    world.resource_scope(|world, db: Mut<MissionDb>| {
        for command in std::mem::take(&mut service.commands) {
            match command {
                LaunchCommand::Cancel(owner) => {
                    let pending = service.pending.as_ref().is_some_and(|p| p.manifest.owner == owner);
                    let running = world.resource::<ActiveMission>().0.as_ref().is_some_and(|r| r.owner == owner);
                    let id = if pending {
                        service.pending.take().unwrap().manifest.id
                    } else if running {
                        let run = world.resource_mut::<ActiveMission>().0.take().unwrap();
                        clear_run_events(world, &run);
                        run.id
                    } else { continue };
                    cleanup_owned(world, owner);
                    if running {
                        clear_presentation(world);
                        world.resource_mut::<MissionBanner>().active = false;
                    }
                    service.record(LaunchStatus::Cancelled { owner, id });
                }
                LaunchCommand::Launch(request) => {
                    let id = request.id.to_ascii_lowercase();
                    let state = availability(
                        &db, &request,
                        world.get_resource::<Chaos>().map_or(0, |c| c.total),
                        world.resource::<MissionsDone>(), world.resource::<Strongholds>(),
                        world.resource::<ActiveMission>().0.as_ref().map(|r| r.id.as_str()),
                        service.pending.as_ref().map(|p| p.manifest.id.as_str()),
                    );
                    if state != Availability::Available { report(world, &mut service, id, state); continue; }
                    let file = match jc2_formats::missions::mission_file(&fs, &id) {
                        Ok(file) => file,
                        Err(e) => { report(world, &mut service, id, Availability::LoadFailed(e.to_string())); continue; }
                    };
                    let objects = script_objects(&file);
                    let entry = db.catalog.get(&id).unwrap();
                    service.next_owner = service.next_owner.checked_add(1).expect("mission generation exhausted");
                    let owner = MissionOwner(service.next_owner);
                    let mut unsupported: Vec<String> = objects.iter()
                        .filter(|o| matches!(o.kind, ScriptKind::Other))
                        .map(|o| o.class.clone()).collect();
                    unsupported.sort(); unsupported.dedup();
                    let manifest = MissionPreparation {
                        owner, id: id.clone(), origin: request.origin.clone(),
                        script_container: entry.script_container.clone().unwrap(),
                        location_name: entry.script_container.as_ref().and_then(|path| {
                            let normalized = path.replace('\\', "/").to_ascii_lowercase();
                            normalized.contains('/').then(|| normalized.strip_suffix(".blz").map(|stem| format!("{stem}.location"))).flatten()
                        }),
                        start_position: jc2_formats::missions::start_point(&objects),
                        giver_container: entry.giver.as_ref().and_then(|g| g.container.clone()),
                        unsupported_classes: unsupported,
                        team_spawn_sets: objects.iter().filter(|o| matches!(o.kind, ScriptKind::Spawn { team: true, .. })).count(),
                    };
                    let mut run = MissionRun::new(&id, entry.record.kind, &db.rules, objects);
                    run.owner = owner;
                    let receipt = (request.origin == LaunchOrigin::Debug).then_some(Ok(()));
                    if receipt.is_some() {
                        warn!("mission {id}: DebugScriptOnly readiness; mission locations/teams/cutscenes are not certified loaded");
                    } else if let Some(mut notices) = world.get_resource_mut::<Notices>() {
                        notices.push(format!("Mission {id}: waiting for location resources"));
                    }
                    service.pending = Some(PreparedMission { manifest, run, receipt, debug_override: request.debug_override, initialized_resources: false, prepared_events: Vec::new() });
                    service.record(LaunchStatus::WaitingForResources { owner, id });
                }
            }
        }
        let settled = !world.contains_resource::<crate::hud::HudStrings>() || {
            let mut heroes = world.query_filtered::<Entity, (With<Hero>, Without<crate::character::Settling>)>();
            heroes.iter(world).next().is_some()
        };
        // Failed preparation must release resources even while Rico is settling.
        let failed = service.pending.as_ref().is_some_and(|p| matches!(p.receipt, Some(Err(_))));
        if (!settled && !failed) || !service.pending.as_ref().is_some_and(|p| p.receipt.is_some()) { return; }
        let mut prepared = service.pending.take().unwrap();
        if let Err(reason) = prepared.receipt.take().unwrap() {
            cleanup_owned(world, prepared.manifest.owner);
            report(world, &mut service, prepared.manifest.id, Availability::ResourcesFailed(reason));
            return;
        }
        // Availability can change during resource loading; validate again at activation.
        let request = MissionLaunchRequest {
            id: prepared.manifest.id.clone(), origin: prepared.manifest.origin.clone(),
            debug_override: prepared.debug_override,
        };
        let state = availability(
            &db, &request, world.get_resource::<Chaos>().map_or(0, |c| c.total),
            world.resource::<MissionsDone>(), world.resource::<Strongholds>(),
            world.resource::<ActiveMission>().0.as_ref().map(|r| r.id.as_str()), None,
        );
        if state != Availability::Available {
            cleanup_owned(world, prepared.manifest.owner);
            report(world, &mut service, prepared.manifest.id, state); return;
        }
        let mut events = prepared.run.start(&db.rules);
        events.extend(prepared.run.resources_loaded(&db.rules));
        events.extend(prepared.prepared_events);
        world.resource_mut::<EventBus>().queue.extend(events);
        let (key, fallback) = prepared.run.kind.started_text();
        world.resource_mut::<MissionBanner>().show(db.text(key, fallback), db.text(&format!("{}_name_txt", prepared.run.id), &prepared.run.id), false, Vec::new());
        service.record(LaunchStatus::Started { owner: prepared.manifest.owner, id: prepared.manifest.id, origin: prepared.manifest.origin });
        world.resource_mut::<ActiveMission>().0 = Some(prepared.run);
    });
    world.insert_resource(service);
}

/// Production readiness for the existing f2t10 exported-geometry proof only. This
/// does not certify warning/distance-trigger behavior, audio, teams or cutscenes.
/// Geometry is registered under the pending generation before RESOURCE_LOADED.
pub(crate) fn prepare_location_resources(world: &mut World) {
    let Some(mut service) = world.remove_resource::<MissionLaunches>() else {
        return;
    };
    let Some(prepared) = service.pending.as_mut() else {
        world.insert_resource(service);
        return;
    };
    if prepared.receipt.is_some() || prepared.manifest.origin == LaunchOrigin::Debug {
        world.insert_resource(service);
        return;
    }
    if !world.contains_resource::<Streamer>() || !world.contains_resource::<AssetServer>() {
        world.insert_resource(service);
        return;
    }
    let owner = prepared.manifest.owner;
    let supported = normal_launch_supported(&prepared.manifest.id)
        && prepared.manifest.team_spawn_sets == 0
        && prepared
            .manifest
            .unsupported_classes
            .iter()
            .all(|c| matches!(c.as_str(), "CDistanceTrigger" | "CMissionFailWarning"));
    if !supported {
        prepared.receipt=Some(Err("resource closure unsupported: only f2t10 exported geometry is verified; mission teams/cutscenes require their own providers".into()));
        world.insert_resource(service);
        return;
    }
    let cfg = world.resource::<WorldConfig>().clone();
    if !cfg.models || !cfg.physics {
        prepared.receipt = Some(Err("f2t10 readiness requires models and physics".into()));
        world.insert_resource(service);
        return;
    }
    let game = world.resource::<Game>().clone();
    let assets = world.resource::<AssetServer>().clone();
    let resources: Vec<_> = prepared
        .run
        .objects
        .iter()
        .enumerate()
        .filter_map(|(i, o)| {
            if let ScriptKind::Spawn {
                entity: Some(entity),
                team: false,
                ..
            } = &o.obj.kind
            {
                Some((i, entity.clone(), o.world))
            } else {
                None
            }
        })
        .collect();
    let mut keys = HashSet::new();
    for (_, _, base) in &resources {
        let p = base.translation();
        let Some(key) = jc2_formats::terrain::tile_index(p[0], p[2]) else {
            prepared.receipt = Some(Err("exported resource outside world".into()));
            world.insert_resource(service);
            return;
        };
        if !game
            .fs
            .contains(&jc2_formats::terrain::tile_file_name(key.0, key.1))
        {
            prepared.receipt = Some(Err("required terrain resource missing".into()));
            world.insert_resource(service);
            return;
        }
        keys.insert(key);
    }
    let mut queue = bevy::ecs::world::CommandQueue::default();
    world.resource_scope(|world, mut st: Mut<Streamer>| {
        st.mission_tiles.insert(owner, keys.clone());
        if keys
            .iter()
            .any(|k| !st.loaded.get(k).is_some_and(|t| t.terrain_triangles > 0))
        {
            return;
        }
        if !prepared.initialized_resources {
            for (index, entity, base) in &resources {
                if !prepared.run.objects[*index].enabled {
                    prepared.receipt = Some(Err("unsupported deferred resource spawn".into()));
                    return;
                }
                let file = match game
                    .fs
                    .read_inflated(bare_name(entity))
                    .ok()
                    .and_then(|b| first_property_member(&b, &[".epe"]))
                {
                    Some(file) => file,
                    None => {
                        prepared.receipt =
                            Some(Err(format!("{entity}: unreadable exported entity")));
                        return;
                    }
                };
                let entity_objects = script_objects(&file);
                if entity_objects.iter().any(|o| {
                    matches!(o.kind, ScriptKind::Spawn { .. })
                        || o.class.to_ascii_lowercase().contains("cutscene")
                }) {
                    prepared.receipt = Some(Err(format!(
                        "{entity}: nested spawn/team/cutscene closure unsupported"
                    )));
                    return;
                }
                let p = base.translation();
                let key = jc2_formats::terrain::tile_index(p[0], p[2]).unwrap();
                let before = (
                    st.loaded[&key].unresolved,
                    st.loaded[&key].collision_missing,
                );
                let units = {
                    let mut commands = Commands::new(&mut queue, world);
                    place_entity(
                        &mut st,
                        key,
                        entity,
                        base,
                        &mut commands,
                        &assets,
                        &game,
                        &cfg,
                        owner,
                    )
                };
                let Some(units) = units else {
                    prepared.receipt = Some(Err(format!("{entity}: initialization failed")));
                    return;
                };
                if st.loaded[&key].unresolved > before.0
                    || st.loaded[&key].collision_missing > before.1
                {
                    prepared.receipt = Some(Err(format!("{entity}: unresolved model/collision")));
                    return;
                }
                let o = &mut prepared.run.objects[*index];
                o.spawned = true;
                o.units = units
                    .into_iter()
                    .map(|unit| crate::world::Piece {
                        tile: key,
                        unit,
                        source: st.loaded[&key].source_key,
                    })
                    .collect();
                prepared.prepared_events.extend(o.fire.iter().cloned());
                let scope = prepared.run.add_entity(entity_objects, base);
                prepared.run.objects[*index].scope = Some(scope);
            }
            prepared.initialized_resources = true;
            return; // command registration must be applied before a readiness receipt
        }
        let mut dependencies_ready = true;
        let paths: Vec<_> = st
            .loaded
            .values()
            .filter_map(|t| t.mission_models.get(&owner))
            .flatten()
            .collect();
        if paths.is_empty() {
            prepared.receipt = Some(Err("no exported geometry registered".into()));
            return;
        }
        for path in paths {
            let Some((handle, _)) = st.handles.get(path) else {
                prepared.receipt = Some(Err("lost resource reference".into()));
                return;
            };
            if let Some(bevy::asset::LoadState::Failed(e)) = assets.get_load_state(handle.id()) {
                prepared.receipt = Some(Err(format!("{path}: {e}")));
                return;
            }
            if let Some(bevy::asset::RecursiveDependencyLoadState::Failed(e)) =
                assets.get_recursive_dependency_load_state(handle.id())
            {
                prepared.receipt = Some(Err(format!("{path}: dependency failed: {e}")));
                return;
            }
            if !assets.is_loaded_with_dependencies(handle.id()) {
                dependencies_ready = false;
            }
        }
        if !dependencies_ready {
            return;
        }
        if st
            .loaded
            .values()
            .any(|t| t.pending.iter().any(|p| p.mission_owner == Some(owner)))
        {
            return;
        }
        let mut owners = world.query::<&MissionOwner>();
        if owners.iter(world).any(|o| *o == owner) {
            prepared.receipt = Some(Ok(()));
        }
    });
    queue.apply(world);
    world.insert_resource(service);
}
pub(super) fn clear_presentation(world: &mut World) {
    world.resource_mut::<Subtitle>().0 = None;
    if let Some(mut o) = world.get_resource_mut::<Objective>() {
        o.0 = None;
    }
    if let Some(mut m) = world.get_resource_mut::<ExtraMarkers>() {
        m.0.clear();
    }
    if let Some(mut b) = world.get_resource_mut::<ExtraBlips>() {
        b.0.clear();
    }
    if let Some(mut p) = world.get_resource_mut::<MapPins>() {
        p.0.clear();
    }
}

pub(super) fn clear_run_events(world: &mut World, run: &MissionRun) {
    world
        .resource_mut::<WorldFlags>()
        .fired
        .retain(|e| !run.event_flags.contains(e));
    world
        .resource_mut::<EventBus>()
        .queue
        .retain(|e| !run.owns_event(e));
}

/// Drop only this generation. Shared models keep their remaining tile references;
/// destructible indices stay stable (tombstones), so another run's Pieces remain valid.
pub(super) fn cleanup_owned(world: &mut World, owner: MissionOwner) {
    if let Some(mut locations) = world.get_resource_mut::<crate::locations::Locations>() {
        locations.release(crate::locations::LocationOwner::Mission(owner));
    }
    let mut query = world.query::<(Entity, &MissionOwner)>();
    let entities: HashSet<Entity> = query
        .iter(world)
        .filter_map(|(e, o)| (*o == owner).then_some(e))
        .collect();
    for &entity in &entities {
        let _ = world.despawn(entity);
    }
    if let Some(mut st) = world.get_resource_mut::<Streamer>() {
        st.mission_tiles.remove(&owner);
        let mut released = Vec::new();
        for tile in st.loaded.values_mut() {
            tile.entities.retain(|e| !entities.contains(e));
            tile.pending.retain(|p| p.mission_owner != Some(owner));
            for unit in &mut tile.units {
                if unit.mission_owner == Some(owner) {
                    unit.destroyed = true;
                    unit.fires.clear();
                    unit.debris.clear();
                }
            }
            if let Some(paths) = tile.mission_models.remove(&owner) {
                for path in paths {
                    if let Some(index) = tile.models.iter().position(|p| *p == path) {
                        tile.models.swap_remove(index);
                        released.push(path);
                    }
                }
            }
        }
        for path in released {
            if let Some((_, count)) = st.handles.get_mut(&path) {
                *count -= 1;
                if *count == 0 {
                    st.handles.remove(&path);
                }
            }
        }
    }
}

/// The bounded resource provider currently supports this geometric mission fixture.
pub(crate) fn normal_launch_supported(id: &str) -> bool {
    id.eq_ignore_ascii_case("f2t10")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless() -> Option<App> {
        let fs = std::sync::Arc::new(
            jc2_formats::GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()?,
        );
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Game { fs })
            .init_resource::<Chaos>()
            .init_resource::<Wallet>()
            .init_resource::<Objective>()
            .init_resource::<Notices>()
            .init_resource::<ExtraMarkers>()
            .init_resource::<ExtraBlips>()
            .init_resource::<MapPins>()
            .add_plugins(MissionsPlugin);
        app.update();
        Some(app)
    }

    fn fixture_request(app: &mut App) -> MissionLaunchRequest {
        let (prerequisite, chaos) = {
            let record = app.world().resource::<MissionDb>().record("f2t10").unwrap();
            (record.prerequisite().map(str::to_string), record.chaos)
        };
        if let Some(p) = prerequisite {
            app.world_mut().resource_mut::<MissionsDone>().0.push(p);
        }
        app.world_mut().resource_mut::<Chaos>().total = chaos;
        MissionLaunchRequest {
            id: "F2T10".into(),
            origin: LaunchOrigin::Pda,
            debug_override: false,
        }
    }

    fn launch(app: &mut App, request: MissionLaunchRequest) {
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request);
        app.update();
    }

    fn owner(app: &App) -> MissionOwner {
        app.world()
            .resource::<ActiveMission>()
            .0
            .as_ref()
            .unwrap()
            .owner
    }

    #[test]
    fn proof_rules_wait_for_resource_acknowledgement() {
        let Some(app) = headless() else { return };
        let db = app.world().resource::<MissionDb>();
        let mut run = MissionRun::new("f2t10", MissionKind::FactionTask, &db.rules, vec![]);
        assert!(run.start(&db.rules).is_empty());
        assert!(run.current.is_none());
        assert!(!run.vars.contains("f2t10.resource_loaded"));
        let fired = run.resources_loaded(&db.rules);
        assert_eq!(run.current.as_deref(), Some("f2t10Obj01"));
        assert!(fired.contains(&"msy.f2t10obj01.activated".into()));
        assert!(run.resources_loaded(&db.rules).is_empty());
    }

    #[test]
    fn catalog_joins_real_employers_and_uses_bmf_faction() {
        let Some(app) = headless() else { return };
        let db = app.world().resource::<MissionDb>();
        assert_eq!(db.catalog.0.len(), 65);
        let task = db.catalog.get("F2T10").unwrap();
        assert_eq!(task.name, "Offensive Action");
        assert_eq!(
            task.record.faction(),
            Some(jc2_formats::missions::Faction::Roaches)
        );
        assert!(
            task.script_container
                .as_ref()
                .unwrap()
                .ends_with("f2t10.blz")
        );
        let giver = task.giver.as_ref().unwrap();
        assert_eq!(giver.id, "f2t10Emp");
        assert_eq!(giver.kind, 1);
        assert!((giver.position[0] - 6013.292).abs() < 0.01);
        assert_eq!(giver.raw.int("enabled"), Some(1));
        assert!(giver.container.as_ref().unwrap().ends_with("f2t10emp.nlz"));
        assert!(db.catalog.get("csk01").is_none());
    }

    #[test]
    fn unlock_boundary_self_prerequisite_and_taken_stronghold_share_policy() {
        let Some(mut app) = headless() else { return };
        let request = fixture_request(&mut app);
        let db = app.world().resource::<MissionDb>();
        let record = db.record("f2t10").unwrap();
        let required = record.chaos;
        assert!(required > 0);
        let prerequisite = record.prerequisite().unwrap();
        let mut holds = Strongholds::default();
        holds.0.push(StrongholdState {
            data: db
                .strongholds
                .iter()
                .find(|s| s.mission == prerequisite)
                .unwrap()
                .clone(),
            phase: Phase::Taken,
            defend_left: 0.0,
        });
        let query = |chaos, done: &MissionsDone, holds: &Strongholds| {
            availability(db, &request, chaos, done, holds, None, None)
        };
        assert_eq!(
            query(required, &MissionsDone::default(), &Strongholds::default()),
            Availability::Prerequisite(prerequisite.into())
        );
        assert_eq!(
            query(required - 1, &MissionsDone::default(), &holds),
            Availability::Chaos {
                required,
                current: required - 1
            }
        );
        assert_eq!(
            query(required, &MissionsDone::default(), &holds),
            Availability::Available
        );
        assert_eq!(
            query(required + 1, &MissionsDone(vec!["F2T10".into()]), &holds),
            Availability::Completed
        );
        let first = MissionLaunchRequest {
            id: "km01".into(),
            origin: LaunchOrigin::Pda,
            debug_override: false,
        };
        assert!(db.record("km01").unwrap().prerequisite().is_none());
        assert_eq!(
            availability(db, &first, 0, &MissionsDone::default(), &holds, None, None),
            Availability::Available
        );
        let hold = MissionLaunchRequest {
            id: prerequisite.into(),
            origin: LaunchOrigin::Pda,
            debug_override: false,
        };
        assert_eq!(
            availability(
                db,
                &hold,
                u32::MAX,
                &MissionsDone::default(),
                &holds,
                None,
                None
            ),
            Availability::Completed
        );
    }

    #[test]
    fn debug_override_is_explicit_and_cannot_replace_or_name_unknown_missions() {
        let Some(mut app) = headless() else { return };
        let mut request = MissionLaunchRequest::debug("f2t10");
        request.origin = LaunchOrigin::Pda;
        launch(&mut app, request);
        assert!(matches!(
            app.world().resource::<MissionLaunches>().history.back(),
            Some(LaunchStatus::Rejected {
                reason: Availability::InvalidDebugOverride,
                ..
            })
        ));
        launch(&mut app, MissionLaunchRequest::debug("not_a_mission"));
        assert!(matches!(
            app.world().resource::<MissionLaunches>().history.back(),
            Some(LaunchStatus::Rejected {
                reason: Availability::UnknownMission,
                ..
            })
        ));
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        let generation = owner(&app);
        launch(&mut app, MissionLaunchRequest::debug("km01"));
        assert_eq!(owner(&app), generation);
        assert!(
            matches!(app.world().resource::<MissionLaunches>().history.back(), Some(LaunchStatus::Rejected { reason: Availability::ActiveMission(id), .. }) if id == "f2t10")
        );
    }

    #[test]
    fn normal_launch_waits_and_duplicate_requests_do_not_overwrite_preparation() {
        let Some(mut app) = headless() else { return };
        let mut request = fixture_request(&mut app);
        request.origin = LaunchOrigin::Giver {
            employer: "f2t10Emp".into(),
        };
        launch(&mut app, request.clone());
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        let preparation = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .clone();
        assert!(preparation.start_position.is_some());
        assert!(preparation.giver_container.is_some());
        assert_eq!(preparation.origin, request.origin);
        launch(&mut app, request);
        assert_eq!(
            app.world()
                .resource::<MissionLaunches>()
                .preparation()
                .unwrap()
                .owner,
            preparation.owner
        );
        assert!(matches!(
            app.world().resource::<MissionLaunches>().history.back(),
            Some(LaunchStatus::Rejected {
                reason: Availability::Preparing(_),
                ..
            })
        ));
        assert!(
            app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(preparation.owner, Ok(()))
        );
        assert!(
            !app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(preparation.owner, Ok(()))
        );
        app.update();
        let run = app.world().resource::<ActiveMission>().0.as_ref().unwrap();
        assert_eq!(run.owner, preparation.owner);
        assert!(run.vars.contains("f2t10.resource_loaded"));
        assert_eq!(run.current.as_deref(), Some("f2t10Obj01"));
    }

    #[test]
    fn cancel_pending_retry_rejects_stale_receipts_and_rechecks_completion() {
        let Some(mut app) = headless() else { return };
        let request = fixture_request(&mut app);
        launch(&mut app, request.clone());
        let old = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .cancel(old);
        app.update();
        launch(&mut app, request);
        let new = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        assert_ne!(old, new);
        assert!(
            !app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(old, Ok(()))
        );
        app.world_mut()
            .resource_mut::<MissionsDone>()
            .0
            .push("f2t10".into());
        assert!(
            app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(new, Ok(()))
        );
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(matches!(
            app.world().resource::<MissionLaunches>().history.back(),
            Some(LaunchStatus::Rejected {
                reason: Availability::Completed,
                ..
            })
        ));
    }

    #[test]
    fn resource_failure_is_visible_and_does_not_start_objectives() {
        let Some(mut app) = headless() else { return };
        // A gameplay HUD with no settled hero must not delay failure cleanup.
        app.init_resource::<crate::hud::HudStrings>();
        let request = fixture_request(&mut app);
        launch(&mut app, request);
        let generation = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        let prepared_entity = app.world_mut().spawn(generation).id();
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .acknowledge(generation, Err("missing mission location".into()));
        app.update();
        assert!(app.world().get_entity(prepared_entity).is_err());
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(
            app.world()
                .resource::<MissionLaunches>()
                .preparation()
                .is_none()
        );
        assert!(
            app.world()
                .resource::<Notices>()
                .0
                .iter()
                .any(|n| n.text.contains("missing mission location"))
        );
    }

    #[test]
    fn cancel_running_restores_flags_entities_and_presentation_then_retry_is_fresh() {
        let Some(mut app) = headless() else { return };
        app.world_mut()
            .resource_mut::<WorldFlags>()
            .fired
            .insert("f2t10.base.entered".into());
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        let old = owner(&app);
        let entity = app
            .world_mut()
            .spawn((old, Name::new("old mission collider")))
            .id();
        let child = app
            .world_mut()
            .spawn((ChildOf(entity), Name::new("mesh collider child")))
            .id();
        let grandchild = app
            .world_mut()
            .spawn((ChildOf(child), Name::new("nested collider")))
            .id();
        let shared = app.world_mut().spawn(Name::new("shared world object")).id();
        app.world_mut()
            .resource_mut::<EventBus>()
            .fire("f2t10.destructable.spawned unrelated.world.event");
        app.update();
        app.world_mut().resource_mut::<Subtitle>().0 = Some("playing".into());
        app.world_mut().resource_mut::<Objective>().0 = Some("old objective".into());
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .cancel(old);
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(app.world().get_entity(entity).is_err());
        assert!(app.world().get_entity(child).is_err());
        assert!(app.world().get_entity(grandchild).is_err());
        assert!(app.world().get_entity(shared).is_ok());
        assert!(app.world().resource::<Subtitle>().0.is_none());
        assert!(app.world().resource::<Objective>().0.is_none());
        assert!(app.world().resource::<ExtraMarkers>().0.is_empty());
        assert!(app.world().resource::<ExtraBlips>().0.is_empty());
        assert!(app.world().resource::<MapPins>().0.is_empty());
        let flags = &app.world().resource::<WorldFlags>().fired;
        assert!(!flags.contains("f2t10.destructable.spawned"));
        assert!(!flags.contains("msy.f2t10obj01.activated"));
        assert!(flags.contains("f2t10.base.entered"));
        assert!(flags.contains("unrelated.world.event"));
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        assert_ne!(owner(&app), old);
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .cancel(old);
        app.update();
        assert_ne!(
            owner(&app),
            old,
            "stale cancellation cannot cancel the retry"
        );
        let run = app.world().resource::<ActiveMission>().0.as_ref().unwrap();
        assert!(run.completed.is_empty());
        assert!(run.objects.iter().all(|o| !o.spawned));
        app.world_mut()
            .resource_mut::<EventBus>()
            .fire("msy.mission.fail");
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(
            !app.world()
                .resource::<WorldFlags>()
                .fired
                .contains("msy.mission.fail")
        );
        assert_eq!(app.world().resource::<Wallet>().money, 0);
        assert!(app.world().resource::<MissionsDone>().0.is_empty());
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        assert_ne!(owner(&app), old);
    }

    #[test]
    fn completion_rewards_are_idempotent_even_for_debug_replay_and_taken_stronghold() {
        let Some(mut app) = headless() else { return };
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        app.world_mut()
            .resource_mut::<ActiveMission>()
            .0
            .as_mut()
            .unwrap()
            .state = RunState::Completed;
        app.update();
        assert_eq!(app.world().resource::<Wallet>().money, 10_000);
        let paid = app.world().resource::<Wallet>().clone();
        let chaos = app.world().resource::<Chaos>().total;
        launch(&mut app, MissionLaunchRequest::debug("f2t10"));
        app.world_mut()
            .resource_mut::<ActiveMission>()
            .0
            .as_mut()
            .unwrap()
            .state = RunState::Completed;
        app.update();
        assert_eq!(*app.world().resource::<Wallet>(), paid);
        assert_eq!(app.world().resource::<Chaos>().total, chaos);
        assert_eq!(app.world().resource::<MissionsDone>().0, ["f2t10"]);
        assert!(app.world().resource::<MissionBanner>().rewards.is_empty());
        app.world_mut()
            .resource_mut::<Strongholds>()
            .0
            .iter_mut()
            .find(|s| s.data.mission == "f1s07")
            .unwrap()
            .phase = Phase::Taken;
        launch(&mut app, MissionLaunchRequest::debug("f1s07"));
        app.world_mut()
            .resource_mut::<ActiveMission>()
            .0
            .as_mut()
            .unwrap()
            .state = RunState::Completed;
        app.update();
        assert_eq!(*app.world().resource::<Wallet>(), paid);
    }

    #[test]
    fn owned_models_pending_at_cancellation_cannot_spawn_into_retry() {
        let Some(mut app) = crate::world::tests::tile_app(|_| {}) else {
            return;
        };
        let old = MissionOwner(11);
        let new = MissionOwner(12);
        let shared = app.world_mut().spawn(Name::new("shared collider")).id();
        let owned = app
            .world_mut()
            .spawn((old, Name::new("owned collider")))
            .id();
        let retry = app
            .world_mut()
            .spawn((new, Name::new("retry collider")))
            .id();
        let handle = Handle::<crate::rbm_loader::RbmAsset>::default();
        {
            let mut st = app.world_mut().resource_mut::<Streamer>();
            let tile = st.loaded.get_mut(&(31, 14)).unwrap();
            let pending = |mission_owner| crate::world::PendingModel {
                handle: handle.clone(),
                transform: Transform::IDENTITY,
                name: "not loaded".into(),
                mesh_colliders: false,
                piece: None,
                vis_range: None,
                mission_owner,
                lod: None,
            };
            tile.entities.extend([shared, owned, retry]);
            tile.pending
                .extend([pending(None), pending(Some(old)), pending(Some(new))]);
            tile.models
                .extend(["shared-model".into(), "shared-model".into()]);
            tile.mission_models.insert(old, vec!["shared-model".into()]);
            st.handles.insert("shared-model".into(), (handle, 2));
        }
        cleanup_owned(app.world_mut(), old);
        {
            let st = app.world().resource::<Streamer>();
            let tile = &st.loaded[&(31, 14)];
            assert_eq!(tile.pending.len(), 2);
            assert!(tile.pending.iter().all(|p| p.mission_owner != Some(old)));
            assert!(tile.entities.contains(&shared));
            assert!(tile.entities.contains(&retry));
            assert!(!tile.entities.contains(&owned));
            assert_eq!(st.handles["shared-model"].1, 1);
            assert_eq!(
                tile.models
                    .iter()
                    .filter(|p| p.as_str() == "shared-model")
                    .count(),
                1
            );
        }
        let mut run = MissionRun::new("fixture", MissionKind::FactionTask, &[], vec![]);
        run.owner = new;
        app.insert_resource(ActiveMission(Some(run)));
        app.update();
        assert!(app.world().get_entity(owned).is_err());
        assert!(app.world().get_entity(shared).is_ok());
        assert!(app.world().get_entity(retry).is_ok());
        let tile = &app.world().resource::<Streamer>().loaded[&(31, 14)];
        assert_eq!(tile.pending.len(), 2, "shared and retry loads stay queued");
    }

    #[test]
    fn real_exported_entity_ownership_covers_colliders_pending_models_and_units() {
        use bevy::ecs::system::RunSystemOnce;
        let Some(mut app) = crate::world::tests::tile_app(|_| {}) else {
            return;
        };
        let fs = app.world().resource::<Game>().fs.clone();
        let file = jc2_formats::missions::mission_file(&fs, "f2t10").unwrap();
        let entity = script_objects(&file)
            .into_iter()
            .find_map(|o| match o.kind {
                ScriptKind::Spawn {
                    entity: Some(entity),
                    ..
                } => Some(entity),
                _ => None,
            })
            .expect("proof mission exports a silo");
        let generation = MissionOwner(77);
        let before = {
            let tile = &app.world().resource::<Streamer>().loaded[&(31, 14)];
            (tile.entities.len(), tile.units.len(), tile.models.len())
        };
        let units = app
            .world_mut()
            .run_system_once(
                move |mut commands: Commands,
                      mut st: ResMut<Streamer>,
                      assets: Res<AssetServer>,
                      game: Res<Game>,
                      cfg: Res<WorldConfig>| {
                    place_entity(
                        &mut st,
                        (31, 14),
                        &entity,
                        &Transform3::IDENTITY,
                        &mut commands,
                        &assets,
                        &game,
                        &cfg,
                        generation,
                    )
                },
            )
            .unwrap()
            .expect("silo's EPE places");
        assert!(!units.is_empty());
        let owned_entities = {
            let tile = &app.world().resource::<Streamer>().loaded[&(31, 14)];
            assert!(!tile.pending.is_empty());
            assert!(
                tile.pending
                    .iter()
                    .all(|p| p.mission_owner == Some(generation))
            );
            assert!(
                units
                    .iter()
                    .all(|&u| tile.units[u as usize].mission_owner == Some(generation))
            );
            assert_eq!(
                tile.mission_models[&generation].len(),
                tile.models.len() - before.2
            );
            tile.entities[before.0..].to_vec()
        };
        assert!(!owned_entities.is_empty(), "silo has colliders");
        assert!(
            owned_entities
                .iter()
                .all(|&e| app.world().get::<MissionOwner>(e) == Some(&generation))
        );
        cleanup_owned(app.world_mut(), generation);
        let tile = &app.world().resource::<Streamer>().loaded[&(31, 14)];
        assert!(tile.pending.is_empty());
        assert_eq!(tile.entities.len(), before.0);
        assert_eq!(tile.models.len(), before.2);
        assert!(
            tile.units[before.1..]
                .iter()
                .all(|u| u.destroyed && u.fires.is_empty() && u.debris.is_empty())
        );
        assert!(
            owned_entities
                .iter()
                .all(|&e| app.world().get_entity(e).is_err())
        );
    }
    fn streaming_app() -> Option<App> {
        if std::env::var("JC2_TEST_DIR").is_err() {
            return None;
        }
        let mut app = crate::world::tests::tile_app(|app| {
            app.init_resource::<Chaos>()
                .init_resource::<Wallet>()
                .init_resource::<Objective>()
                .init_resource::<Notices>()
                .init_resource::<ExtraMarkers>()
                .init_resource::<ExtraBlips>()
                .init_resource::<MapPins>()
                .add_plugins(MissionsPlugin);
        })
        .unwrap();
        app.update();
        Some(app)
    }
    #[test]
    fn retail_preparation_registers_exported_geometry_before_activation() {
        let Some(mut app) = streaming_app() else {
            return;
        };
        let request = fixture_request(&mut app);
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request);
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        let owner = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        let start = std::time::Instant::now();
        while app.world().resource::<ActiveMission>().0.is_none() {
            app.update();
            assert!(
                start.elapsed().as_secs() < 90,
                "{:?}",
                app.world().resource::<MissionLaunches>().history
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let run = app.world().resource::<ActiveMission>().0.as_ref().unwrap();
        assert_eq!(run.owner, owner);
        assert_eq!(
            run.objects
                .iter()
                .filter(|o| o.spawned && !o.units.is_empty())
                .count(),
            5
        );
        assert_eq!(
            run.objects
                .iter()
                .filter(|o| o.spawned
                    && matches!(
                        o.obj.kind,
                        ScriptKind::Spawn {
                            entity: Some(_),
                            ..
                        }
                    ))
                .count(),
            8,
            "all five silos and three stash geometry resources register once"
        );
        let st = app.world().resource::<Streamer>();
        assert!(!st.mission_tiles[&owner].is_empty());
        assert!(
            st.loaded
                .values()
                .all(|t| t.pending.iter().all(|p| p.mission_owner != Some(owner)))
        );
        assert!(
            st.loaded
                .values()
                .filter_map(|t| t.mission_models.get(&owner))
                .flatten()
                .all(|p| {
                    app.world()
                        .resource::<AssetServer>()
                        .is_loaded_with_dependencies(st.handles[p].0.id())
                })
        );
    }
    #[test]
    fn cancel_registered_preparation_rejects_late_model_work_and_retry_generation() {
        let Some(mut app) = streaming_app() else {
            return;
        };
        let request = fixture_request(&mut app);
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request.clone());
        let start = std::time::Instant::now();
        loop {
            app.update();
            if app
                .world()
                .resource::<MissionLaunches>()
                .pending
                .as_ref()
                .is_some_and(|p| p.initialized_resources)
            {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 90,
                "resource preparation stalled"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        let old = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .cancel(old);
        app.update();
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(
            !app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(old, Ok(()))
        );
        let st = app.world().resource::<Streamer>();
        assert!(!st.mission_tiles.contains_key(&old));
        assert!(
            st.loaded
                .values()
                .all(|t| !t.mission_models.contains_key(&old)
                    && t.pending.iter().all(|p| p.mission_owner != Some(old)))
        );
        let mut query = app.world_mut().query::<&MissionOwner>();
        assert!(query.iter(app.world()).all(|o| *o != old));
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request);
        app.update();
        let retry = app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        assert_ne!(old, retry);
        assert!(
            !app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(old, Ok(()))
        );
        assert!(app.world().resource::<ActiveMission>().0.is_none());
    }
    #[test]
    fn partial_preparation_read_failure_releases_generation_resources() {
        let Some(mut app) = streaming_app() else {
            return;
        };
        let request = fixture_request(&mut app);
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request);
        app.update();
        let owner = {
            let mut launches = app.world_mut().resource_mut::<MissionLaunches>();
            let prepared = launches.pending.as_mut().unwrap();
            let second = prepared
                .run
                .objects
                .iter_mut()
                .filter(|o| {
                    matches!(
                        o.obj.kind,
                        ScriptKind::Spawn {
                            entity: Some(_),
                            ..
                        }
                    )
                })
                .nth(1)
                .unwrap();
            if let ScriptKind::Spawn { entity, .. } = &mut second.obj.kind {
                *entity = Some("missing-proof-resource.ee".into());
            }
            prepared.manifest.owner
        };
        let start = std::time::Instant::now();
        while app
            .world()
            .resource::<MissionLaunches>()
            .preparation()
            .is_some()
        {
            app.update();
            assert!(start.elapsed().as_secs() < 90, "failure never reported");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(
            app.world()
                .resource::<MissionLaunches>()
                .history
                .iter()
                .any(|s| matches!(
                    s,
                    LaunchStatus::Rejected {
                        reason: Availability::ResourcesFailed(_),
                        ..
                    }
                ))
        );
        let st = app.world().resource::<Streamer>();
        assert!(!st.mission_tiles.contains_key(&owner));
        assert!(
            st.loaded
                .values()
                .all(|t| !t.mission_models.contains_key(&owner)
                    && t.pending.iter().all(|p| p.mission_owner != Some(owner)))
        );
        let mut query = app.world_mut().query::<&MissionOwner>();
        assert!(query.iter(app.world()).all(|o| *o != owner));
        assert!(
            !app.world_mut()
                .resource_mut::<MissionLaunches>()
                .acknowledge(owner, Ok(()))
        );
    }
    #[test]
    fn resource_provider_rejects_unsupported_team_closure() {
        let Some(mut app) = streaming_app() else {
            return;
        };
        let request = fixture_request(&mut app);
        app.world_mut()
            .resource_mut::<MissionLaunches>()
            .request(request);
        app.update();
        let owner = {
            let mut service = app.world_mut().resource_mut::<MissionLaunches>();
            let prepared = service.pending.as_mut().unwrap();
            prepared.manifest.team_spawn_sets = 1;
            prepared.manifest.owner
        };
        app.update();
        app.update();
        assert!(app.world().resource::<ActiveMission>().0.is_none());
        assert!(
            app.world()
                .resource::<MissionLaunches>()
                .preparation()
                .is_none()
        );
        assert!(app.world().resource::<MissionLaunches>().history.iter().any(|s| matches!(s,LaunchStatus::Rejected { reason: Availability::ResourcesFailed(e), .. } if e.contains("unsupported"))));
        assert!(
            !app.world()
                .resource::<Streamer>()
                .mission_tiles
                .contains_key(&owner)
        );
    }
}
