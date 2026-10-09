//! Every vehicle in the world goes through here: the ones the command line asks for
//! (`--drive`, `--heli`, `--boat`) and the ones the tiles' spawn sets park
//! (`jc2_formats::placement::vehicle_spawns`: the village's attack helicopter, trucks,
//! an APC). A request names a container; its doll says whether it is a car, a
//! helicopter or a boat; its parts load through the asset server; once they are in, the
//! kind's builder (`vehicle::build_car`, `heli::build_heli`, `boat::build_boat`) puts it
//! in the world. Vehicles a tile spawned leave with the tile — unless someone is in one.

use crate::Game;
use crate::boat::{build_boat, find_open_water};
use crate::coords::{bevy_xyz, engine_xz, mat_to_bevy};
use crate::gameplay_camera::VehicleCamera;
use crate::heli::build_heli;
use crate::rbm_loader::RbmAsset;
use crate::vehicle::{
    CameraMode, DollParts, HeroEnabled, Occupied, VehicleSpec, build_car, load_doll_parts,
};
use crate::world::{GameplaySourceKey, SourceOrigin};
use crate::world::{SEA_LEVEL, SpawnPoint, Streamer};
use avian3d::prelude::*;
use bevy::asset::LoadState;
use bevy::prelude::*;
use jc2_formats::placement::bare_name;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::terrain::GlobalTerrain;
use jc2_formats::vehicle::{Airfoil, Boat, Helicopter, VehicleDoll};
use std::collections::{HashMap, HashSet};

/// Where a requested vehicle goes.
#[derive(Clone, Debug)]
pub enum Place {
    /// Next to the start, on the ground (cars, helicopters) — this offset from the
    /// spawn point, in Bevy axes.
    NearStart(Vec3),
    /// On the nearest open water to the start (boats).
    OpenWater,
    /// Exactly here (a spawn set), standing on the ground below.
    At(Transform),
}

#[derive(Clone, Debug)]
pub struct Request {
    pub container: String,
    pub place: Place,
    /// The tile whose spawn set asked; the vehicle leaves when the tile does.
    pub tile: Option<(u32, u32)>,
    /// Without Rico, the player drives this one.
    pub drive: bool,
    /// Put a driver in it (`crew.rs`).
    pub crew: Option<crate::crew::CrewKind>,
}

pub enum Kind {
    Car(VehicleSpec),
    Heli(Helicopter),
    Boat(Boat),
    /// The helicopter modules, the wings, and the wheels (centre, radius) in body axes.
    Plane(Helicopter, Vec<Airfoil>, Vec<(Vec3, f32)>),
}

impl Kind {
    fn label(&self) -> &'static str {
        match self {
            Kind::Car(_) => "car",
            Kind::Heli(_) => "helicopter",
            Kind::Boat(_) => "boat",
            Kind::Plane(..) => "plane",
        }
    }
}

struct Pending {
    origin: Option<SourceOrigin>,
    req: Request,
    name: String,
    kind: Kind,
    parts: DollParts,
    /// The driver seat controller's camera setting (`camera_id`), when the doll has one.
    camera: Option<VehicleCamera>,
}

#[derive(Resource)]
pub struct Fleet {
    pub lod: usize,
    pub requests: Vec<Request>,
    pending: Vec<Pending>,
    source_requests: Vec<(Request, SourceOrigin)>,
    /// Spawned vehicles and the tile that owns them.
    pub spawned: Vec<(Entity, Option<(u32, u32)>)>,
    /// Tiles whose spawn sets were already requested.
    tiles_done: HashSet<GameplaySourceKey>,
    /// Decoded dolls by container, so a second truck of a kind is cheap.
    dolls: HashMap<String, Option<VehicleDoll>>,
    global: Option<GlobalTerrain>,
    /// Also park the spawn sets' vehicles (off with `--no-traffic`).
    pub world_vehicles: bool,
}

impl Fleet {
    pub fn new(lod: usize, world_vehicles: bool) -> Self {
        Self {
            lod,
            requests: Vec::new(),
            pending: Vec::new(),
            source_requests: Vec::new(),
            spawned: Vec::new(),
            tiles_done: HashSet::new(),
            dolls: HashMap::new(),
            global: None,
            world_vehicles,
        }
    }
}

pub struct FleetPlugin;

impl Plugin for FleetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_plugins((
                crate::vehicle::VehiclePlugin,
                crate::heli::HeliPlugin,
                crate::boat::BoatPlugin,
                crate::plane::PlanePlugin,
            ))
            .add_systems(
                Update,
                (tile_spawn_sets, request_parts, spawn_ready, drop_with_tiles)
                    .chain()
                    .after(crate::locations::LocationReconciled)
                    .in_set(crate::world::GameplaySourcesReconciled),
            );
    }
}

/// Queue the spawn sets of tiles that just loaded.
fn tile_spawn_sets(mut fleet: ResMut<Fleet>, streamer: Option<Res<Streamer>>) {
    if !fleet.world_vehicles {
        return;
    }
    let Some(st) = streamer else { return };
    for (source, key, tile) in st.gameplay_sources() {
        if fleet.tiles_done.contains(&source) {
            continue;
        }
        for (ordinal, s) in tile.vehicle_spawns.iter().enumerate() {
            fleet.source_requests.push((
                Request {
                    container: bare_name(&s.container).to_string(),
                    place: Place::At(mat_to_bevy(&s.world)),
                    tile: Some(key),
                    drive: false,
                    crew: None,
                },
                SourceOrigin {
                    key: source,
                    container: tile.source_container.clone(),
                    object: s.name.clone(),
                    ordinal,
                },
            ));
        }
        fleet.tiles_done.insert(source);
    }
    fleet.tiles_done.retain(|k| st.source_alive(*k));
}

/// Read the doll of every new request and ask for its models.
fn request_parts(mut fleet: ResMut<Fleet>, assets: Res<AssetServer>, game: Res<Game>) {
    let mut requests: Vec<_> = std::mem::take(&mut fleet.requests)
        .into_iter()
        .map(|r| (r, None))
        .collect();
    requests.extend(
        std::mem::take(&mut fleet.source_requests)
            .into_iter()
            .map(|(r, o)| (r, Some(o))),
    );
    for (req, origin) in requests {
        let container = bare_name(&req.container).to_string();
        let Ok(data) = game.fs.read_inflated(&container) else {
            warn!("{container}: not found");
            continue;
        };
        let Ok(sarc) = SmallArchive::parse(&data) else {
            warn!("{container}: not a container");
            continue;
        };
        let doll = fleet
            .dolls
            .entry(container.clone())
            .or_insert_with(|| {
                sarc.entries
                    .iter()
                    .find(|e| e.name.to_ascii_lowercase().ends_with("vdoll"))
                    .and_then(|e| sarc.data(&data, e).ok())
                    .and_then(|d| VehicleDoll::parse(d).ok())
            })
            .clone();
        let Some(doll) = doll else {
            warn!("{container}: no vehicle doll inside");
            continue;
        };
        let kind = if let (true, Some(h)) = (doll.plane, doll.heli.clone()) {
            let wheels = doll
                .wheels()
                .iter()
                .map(|w| {
                    let r = w
                        .wheel
                        .and_then(|n| doll.axle_for_wheel(n))
                        .map_or(0.3, |a| a.radius);
                    (mat_to_bevy(&w.world).translation, r)
                })
                .collect();
            Kind::Plane(h, doll.wings.clone(), wheels)
        } else if let Some(h) = doll.heli.clone() {
            Kind::Heli(h)
        } else if !doll.axles.is_empty() {
            Kind::Car(VehicleSpec::from_doll(&container, &doll))
        } else if let Some(b) = doll.boat.clone().filter(|b| !b.floats.is_empty()) {
            Kind::Boat(b)
        } else {
            warn!("{container}: neither car, helicopter, plane nor boat");
            continue;
        };
        let lod = fleet.lod;
        let Some(parts) = load_doll_parts(&container, &sarc, &data, &doll, &game, &assets, lod)
        else {
            warn!("{container}: no body part");
            continue;
        };
        info!("{container}: a {} requested", kind.label());
        let camera = doll
            .driver_seat()
            .and_then(|s| Some((s.camera_id?, s.camera_id_reverse)))
            .map(|(id, reverse)| VehicleCamera { id, reverse });
        fleet.pending.push(Pending {
            origin,
            req,
            name: container,
            kind,
            parts,
            camera,
        });
    }
}

/// Spawn what has loaded and has somewhere to stand.
#[allow(clippy::too_many_arguments)]
fn spawn_ready(
    mut commands: Commands,
    mut fleet: ResMut<Fleet>,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    meshes: Res<Assets<Mesh>>,
    game: Res<Game>,
    spawn: Option<Res<SpawnPoint>>,
    hero: Option<Res<HeroEnabled>>,
    spatial: SpatialQuery,
    mut mode: ResMut<CameraMode>,
    streamer: Option<Res<Streamer>>,
) {
    if fleet.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut fleet.pending);
    let mut keep = Vec::new();
    for p in pending {
        if p.origin
            .as_ref()
            .is_some_and(|o| streamer.as_ref().is_none_or(|st| !st.source_alive(o.key)))
        {
            continue;
        }
        let failed = |h: &Handle<RbmAsset>| {
            matches!(assets.get_load_state(h.id()), Some(LoadState::Failed(_)))
        };
        if failed(&p.parts.body) {
            warn!("{}: the body model failed to load", p.name);
            continue;
        }
        let ready = rbms.get(&p.parts.body).is_some()
            && p.parts
                .parts
                .iter()
                .all(|pp| rbms.get(&pp.handle).is_some() || failed(&pp.handle));
        if !ready {
            keep.push(p);
            continue;
        }
        // where it goes
        let ground_below = |at: Vec3| -> Option<Vec3> {
            spatial
                .cast_ray(
                    at + Vec3::Y * 300.0,
                    Dir3::NEG_Y,
                    800.0,
                    true,
                    &SpatialQueryFilter::default(),
                )
                .map(|h| Vec3::new(at.x, at.y + 300.0 - h.distance, at.z))
        };
        let spot: Option<(Vec3, Quat)> = match &p.req.place {
            Place::NearStart(offset) => spawn
                .as_ref()
                .and_then(|s| ground_below(s.0 + *offset))
                .map(|g| (g, Quat::IDENTITY)),
            Place::At(tf) if matches!(p.kind, Kind::Boat(_)) => Some((tf.translation, tf.rotation)),
            Place::At(tf) => ground_below(tf.translation).map(|g| (g, tf.rotation)),
            Place::OpenWater => {
                if fleet.global.is_none() {
                    fleet.global = game
                        .fs
                        .read_inflated("terrain_win32.dat")
                        .ok()
                        .and_then(|d| GlobalTerrain::parse(&d).ok());
                }
                match (&spawn, &fleet.global) {
                    (Some(s), Some(map)) => {
                        let (sx, sz) = engine_xz(s.0);
                        match find_open_water(map, sx, sz, 6000.0, 3.0) {
                            Some((x, z)) => Some((bevy_xyz(x, SEA_LEVEL, z), Quat::IDENTITY)),
                            None => {
                                warn!("{}: no open water within 6 km of the start", p.name);
                                continue;
                            }
                        }
                    }
                    _ => None,
                }
            }
        };
        let Some((at, rot)) = spot else {
            // the ground under it is not there yet
            keep.push(p);
            continue;
        };
        let Some(body) = rbms.get(&p.parts.body) else {
            continue;
        };
        let collision = p.parts.body_collision.as_ref();
        let parts = &p.parts.parts;
        let id = match &p.kind {
            Kind::Car(spec) => build_car(
                &mut commands,
                spec,
                body,
                &meshes,
                &rbms,
                parts,
                collision,
                at,
                rot,
            ),
            Kind::Heli(h) => build_heli(
                &mut commands,
                h,
                &p.name,
                body,
                &meshes,
                &rbms,
                parts,
                collision,
                at,
                rot,
            ),
            Kind::Plane(h, wings, wheels) => crate::plane::build_plane(
                &mut commands,
                h,
                wings,
                wheels,
                &p.name,
                body,
                &meshes,
                &rbms,
                parts,
                collision,
                at,
                rot,
            ),
            Kind::Boat(b) => {
                // afloat where it was asked for (only x and z count)
                build_boat(
                    &mut commands,
                    b,
                    &p.name,
                    body,
                    &meshes,
                    &rbms,
                    parts,
                    collision,
                    at,
                    rot,
                )
            }
        };
        commands.entity(id).insert(Container(
            jc2_formats::placement::bare_name(&p.req.container).to_ascii_lowercase(),
        ));
        if let Some(kind) = p.req.crew {
            commands.entity(id).insert(crate::crew::CrewRequest(kind));
        }
        if p.req.drive && hero.is_none() {
            commands.entity(id).insert(Occupied);
            *mode = CameraMode::Drive;
        }
        if let Some(origin) = p.origin {
            commands.entity(id).insert(origin);
        }
        if let Some(camera) = p.camera {
            commands.entity(id).insert(camera);
        }
        fleet.spawned.push((id, p.req.tile));
    }
    fleet.pending.extend(keep);
}

/// The container a vehicle was built from (`lave.v014_jeep.ee`).
#[derive(Component, Clone, Debug)]
pub struct Container(pub String);

/// A tile's vehicles go when it unloads, unless someone is in one (then the vehicle
/// is the player's now and stays).
fn drop_with_tiles(
    mut commands: Commands,
    mut fleet: ResMut<Fleet>,
    streamer: Option<Res<Streamer>>,
    occupied: Query<(), With<Occupied>>,
    origins: Query<&SourceOrigin>,
) {
    let Some(st) = streamer else { return };
    let mut keep = Vec::with_capacity(fleet.spawned.len());
    for (e, tile) in std::mem::take(&mut fleet.spawned) {
        match tile {
            Some(t)
                if origins
                    .get(e)
                    .map_or_else(|_| !st.loaded.contains_key(&t), |o| !st.source_alive(o.key)) =>
            {
                if occupied.get(e).is_ok() {
                    commands.entity(e).remove::<SourceOrigin>();
                    keep.push((e, None));
                } else {
                    commands.entity(e).try_despawn();
                }
            }
            _ => keep.push((e, tile)),
        }
    }
    fleet.spawned = keep;
    // pending requests for tiles that left are dropped too
    fleet.pending.retain(|p| {
        p.origin.as_ref().map_or_else(
            || p.req.tile.is_none_or(|t| st.loaded.contains_key(&t)),
            |o| st.source_alive(o.key),
        )
    });
    fleet
        .source_requests
        .retain(|(_, o)| st.source_alive(o.key));
}

/// A headless app that can spawn vehicles through the fleet, for the vehicle tests.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::physics::terrain_collider;
    use crate::rbm_loader::RbmLoader;
    use bevy::asset::AssetApp;
    use bevy::asset::io::AssetSourceBuilder;
    use jc2_formats::GameFs;
    use jc2_formats::terrain::TerrainTile;
    use std::sync::Arc;

    pub fn flat_tile(raw: u16) -> TerrainTile {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for _ in 0..132 * 132 {
            d.extend_from_slice(&raw.to_le_bytes());
        }
        d.extend(std::iter::repeat_n(0u8, 4 * 128 * 128 * 2));
        TerrainTile::parse(&d).unwrap()
    }

    /// The app, the ground height of the flat tile it stands on (tile 31,14's corner),
    /// and the spawn point in the middle of it. `None` without the test data.
    pub fn app(needs: &[&str], setup: impl FnOnce(&mut App)) -> Option<(App, f32, Vec3)> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = Arc::new(GameFs::open(dir).ok()?);
        if needs.iter().any(|n| !fs.contains(n)) {
            return None;
        }
        let tile = flat_tile(32000);
        let ground = tile.cell_height(0, 0);
        let (collider, tf) = terrain_collider(&tile, -512.0, -9216.0);
        let reader_fs = fs.clone();
        let mut app = App::new();
        app.register_asset_source(
            "jc2",
            AssetSourceBuilder::new(move || Box::new(JcAssetReader::new(reader_fs.clone()))),
        );
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugins::default(),
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 64.0),
        ))
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        // Headless fixtures still decode required retail BC colour images.
        .register_asset_loader(bevy::image::ImageLoader::new(
            bevy::image::CompressedImageFormats::BC,
        ))
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .init_asset::<crate::general_material::GeneralMaterial>()
        .register_asset_loader(RbmLoader::default())
        .insert_resource(Game { fs })
        .insert_resource(Fleet::new(1, false))
        .init_resource::<CameraMode>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Update, (request_parts, spawn_ready).chain());
        let spawn = Vec3::new(-256.0, ground, -8960.0);
        app.insert_resource(SpawnPoint(spawn));
        app.world_mut()
            .spawn((RigidBody::Static, collider, tf, Name::new("ground")));
        setup(&mut app);
        app.finish();
        Some((app, ground, spawn))
    }

    /// Frames, one at a time, until `done`. Whatever loads on other threads (models, Rico)
    /// shows up after a number of frames that depends on the machine's load, so a test
    /// waits for it by the wall clock (a minute at most) instead of stepping a fixed
    /// number of frames and then looking for it.
    pub fn wait_until(app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
        let start = std::time::Instant::now();
        loop {
            app.update();
            if done(app) {
                return;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(60),
                "timed out waiting for {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Run until Rico exists, has been placed (his `Settling` is over) and stands on the
    /// ground.
    pub fn wait_for_hero(app: &mut App) {
        use crate::character::{Hero, Settling};
        wait_until(app, "rico", |app| {
            let mut q = app.world_mut().query_filtered::<&Hero, Without<Settling>>();
            q.single(app.world()).is_ok_and(|h| h.grounded)
        });
    }

    /// Ask for a vehicle and run until it is in the world.
    pub fn spawn(app: &mut App, container: &str, place: Place) -> Entity {
        spawn_crewed(app, container, place, None)
    }

    /// Ask for a vehicle (with a driver when `crew` says so) and run until it is in the
    /// world. Without Rico in the app the player is put in it unless it is crewed.
    pub fn spawn_crewed(
        app: &mut App,
        container: &str,
        place: Place,
        crew: Option<crate::crew::CrewKind>,
    ) -> Entity {
        let before = app.world().resource::<Fleet>().spawned.len();
        app.world_mut()
            .resource_mut::<Fleet>()
            .requests
            .push(Request {
                container: container.into(),
                place,
                tile: None,
                drive: crew.is_none(),
                crew,
            });
        // the models load on other threads: wait by the wall clock, not for a number of
        // frames (a loaded machine takes many more of them)
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(60) {
            app.update();
            let fleet = app.world().resource::<Fleet>();
            if fleet.spawned.len() > before
                && let Some((e, _)) = fleet.spawned.last()
            {
                return *e;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("{container} never spawned");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::rbm_loader::RbmLoader;
    use crate::world::{self, WorldConfig};
    use bevy::asset::AssetApp;
    use bevy::asset::io::AssetSourceBuilder;
    use jc2_formats::GameFs;
    use std::sync::Arc;

    /// Streaming tile 31,14 queues the vehicles its spawn sets park (the attack
    /// helicopter, the trucks, the APC), once. Needs JC2_TEST_DIR with `31x14.nlz` and the
    /// tile's terrain.
    #[test]
    fn tiles_queue_their_vehicles() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        if !fs.contains("31x14.nlz") || !fs.contains("terrain_win32_31_14.dat") {
            return;
        }
        let reader_fs = fs.clone();
        let mut app = App::new();
        app.register_asset_source(
            "jc2",
            AssetSourceBuilder::new(move || Box::new(JcAssetReader::new(reader_fs.clone()))),
        )
        .add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            crate::physics::tests::headless_physics(),
        ))
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        // Headless fixtures still decode required retail BC colour images.
        .register_asset_loader(bevy::image::ImageLoader::new(
            bevy::image::CompressedImageFormats::BC,
        ))
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .init_asset::<crate::general_material::GeneralMaterial>()
        .register_asset_loader(RbmLoader::default())
        .insert_resource(Game { fs })
        .insert_resource(WorldConfig {
            tile: (31, 14),
            radius: 0,
            lod: 1,
            models: true,
            physics: true,
            flat_terrain: true,
        })
        .insert_resource(Fleet::new(1, true))
        .init_asset::<crate::terrain_material::TerrainSplatMaterial>()
        .add_systems(Startup, world::setup)
        .add_systems(Update, (world::stream_tiles, tile_spawn_sets).chain());
        app.finish();
        for _ in 0..20 {
            app.update();
        }
        let fleet = app.world().resource::<Fleet>();
        let asked: Vec<String> = fleet
            .source_requests
            .iter()
            .map(|(r, _)| r.container.clone())
            .collect();
        let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
        assert!(asked.contains(&"arve.v060_attackheli.ee"), "{asked:?}");
        assert!(asked.contains(&"lave.v016_military_apc.ee"), "{asked:?}");
        assert!(
            fleet
                .source_requests
                .iter()
                .all(|(r, _)| r.tile == Some((31, 14)) && !r.drive)
        );
        let n = asked.len();
        app.update();
        assert_eq!(
            app.world().resource::<Fleet>().source_requests.len(),
            n,
            "asked twice"
        );
        eprintln!("tile 31,14 parks {n} vehicles: {asked:?}");
    }
    #[test]
    fn retired_source_vehicle_assets_cannot_spawn_but_external_requests_can() {
        let Some((mut app, _, _)) = testing::app(&["lave.v016_military_apc.ee"], |_| {}) else {
            return;
        };
        let request = Request {
            container: "lave.v016_military_apc.ee".into(),
            place: Place::NearStart(Vec3::ZERO),
            tile: Some((31, 14)),
            drive: false,
            crew: None,
        };
        app.world_mut()
            .resource_mut::<Fleet>()
            .source_requests
            .push((
                request.clone(),
                SourceOrigin {
                    key: GameplaySourceKey(9030),
                    container: "retired/source.nlz".into(),
                    object: request.container.clone(),
                    ordinal: 0,
                },
            ));
        app.update();
        assert!(app.world().resource::<Fleet>().pending.is_empty());
        assert!(app.world().resource::<Fleet>().spawned.is_empty());
        app.world_mut()
            .resource_mut::<Fleet>()
            .requests
            .push(Request {
                tile: None,
                ..request
            });
        testing::wait_until(&mut app, "ordinary APC", |app| {
            !app.world().resource::<Fleet>().spawned.is_empty()
        });
        assert_eq!(
            app.world().resource::<Fleet>().spawned.len(),
            1,
            "retired assets did not recreate their actor"
        );
        let entity = app.world().resource::<Fleet>().spawned[0].0;
        assert!(app.world().get::<SourceOrigin>(entity).is_none());
    }
}
