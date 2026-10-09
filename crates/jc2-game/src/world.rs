//! Milestone 3: the world. Streams the terrain tiles around whatever you are flying or
//! driving — terrain mesh and collider, the tile's placed models, its Havok collision
//! instances — loading one tile a frame as you move and dropping the ones left behind,
//! draws the sea, and gives you a free-flying camera.
//!
//! `jc2 --world 31,14 [--radius 1] [--lod 1] [--no-models] [--no-physics]`
//!
//! Controls: right-drag to look, WASD to move, Q/E or Space/C down/up, Shift ×10,
//! Alt ×0.1, wheel changes the base speed, `R` returns to the start.

use crate::Game;
use crate::coords::{bevy_xyz, engine_xz, mat_to_bevy, to_bevy, to_engine};
use crate::missions::{ActiveMission, MissionOwner};
use crate::physics::{TerrainBody, colliders_from_body, static_mesh_collider, terrain_collider};
use crate::rbm_loader::RbmAsset;
use crate::terrain_material::{SplatContext, TerrainAssets, vertex_colour};
use crate::vehicle::CameraMode;
use avian3d::prelude::{Collider, RigidBody};
use bevy::asset::LoadState;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use jc2_formats::areaset::AreaSets;
use jc2_formats::collision::{CollisionInstances, parse_pfx};
use jc2_formats::landscape::Landscape;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::placement::{bare_name, placements};
use jc2_formats::property::PropertyFile;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::terrain::{
    CELL_SIZE, TILE_CELLS, TILE_MARGIN, TILE_SIZE, TILES_PER_SIDE, TerrainTile,
    tile_container_name, tile_file_name, tile_origin,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Avian colliders of one Havok shape with their placements (see `colliders_from_body`).
pub(crate) type ColliderParts = Vec<(Vec3, Quat, Collider)>;

/// Height of the water surface in world units (the original checks "at sea level" as
/// `|y − 200| ≤ 5`).
pub const SEA_LEVEL: f32 = 200.0;

/// Container with the props every location may reference without shipping a copy.
const SHARED_CONTAINER: &str = "general.blz";

/// Clear colour and fog tint.
const SKY: Color = Color::srgb(0.58, 0.72, 0.90);

#[derive(Resource, Clone)]
pub struct WorldConfig {
    /// Where to start.
    pub tile: (u32, u32),
    /// Tiles kept loaded in every direction around the focus (1 → 3 × 3).
    pub radius: u32,
    pub lod: usize,
    pub models: bool,
    /// Give the terrain and the placed models colliders.
    pub physics: bool,
    /// Colour the terrain by height and slope instead of the game's materials.
    pub flat_terrain: bool,
}

/// Where things get dropped into the world: on the ground at the start tile's middle
/// (Bevy axes). Inserted once that tile is loaded, so there is ground to land on.
#[derive(Resource, Clone, Copy)]
pub struct SpawnPoint(pub Vec3);

/// Start here instead of the start tile's middle: engine `(x, z)` on that tile
/// (`--mission` puts Rico at the mission's restart point).
#[derive(Resource, Clone, Copy, Debug)]
pub struct StartAt(pub [f32; 2]);

/// Location containers that are not a tile's own (`locations\wc\…`) but load with it:
/// the strongholds' bases (`f1s07.base.nlz` on the stronghold's tile; `missions.rs`).
#[derive(Resource, Default, Debug)]
pub struct ExtraContainers(pub HashMap<(u32, u32), Vec<String>>);

/// A model asked from the asset server, spawned by `spawn_models` once it arrives.
/// `scenery` (vegetation, static geometry) queues these too.
pub(crate) struct PendingModel {
    pub(crate) handle: Handle<RbmAsset>,
    pub(crate) transform: Transform,
    pub(crate) name: String,
    /// Give the render meshes trimesh colliders (only when the tile has no `.pfi` list
    /// of real collision shapes).
    pub(crate) mesh_colliders: bool,
    /// The destructible unit (`destruction.rs`) this object is an intact part of.
    pub(crate) piece: Option<u32>,
    /// Distances (m) between which the model fades out of view (`VisibilityRange`):
    /// `scenery.rs` gives its plants one, the distant trees are impostors then.
    pub(crate) vis_range: Option<(f32, f32)>,
    pub(crate) mission_owner: Option<MissionOwner>,
    /// Native LOD slots of a plant (`plant_lod.rs`); `handle` is then the set's far model.
    pub(crate) lod: Option<std::sync::Arc<crate::plant_lod::PlantLodSet>>,
}

/// Part of a destructible unit of a tile: goes when the unit is destroyed.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub(crate) source: Option<GameplaySourceKey>,
    pub tile: (u32, u32),
    pub unit: u32,
}

/// A piece of debris a destroyed unit throws: the model and the shape, placed.
pub(crate) struct DebrisData {
    pub(crate) model: Option<String>,
    pub(crate) transform: Transform,
    pub(crate) shape: Option<ColliderParts>,
    pub(crate) mass: Option<f32>,
    /// The placement's authored `grappling_weight_category` (JC2-037).
    pub(crate) grapple_weight: Option<u8>,
}

/// A destructible unit of a tile (`jc2_formats::destruction`).
pub(crate) struct UnitData {
    pub(crate) name: String,
    pub(crate) health: f32,
    pub(crate) center: Vec3,
    pub(crate) explodes: bool,
    pub(crate) debris: Vec<DebrisData>,
    pub(crate) destroyed: bool,
    /// The activity type its destruction counts as (`chaos.rs`).
    pub(crate) activity: Option<u32>,
    /// The settlement it counts towards (`31x14`).
    pub(crate) counter: Option<String>,
    /// What its damage controls fire (`e{…}::destroyed gsy.activity.type13`): a mission
    /// script listens (`missions.rs`).
    pub(crate) fires: Vec<String>,
    pub(crate) mission_owner: Option<MissionOwner>,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GameplaySourcesReconciled;

/// Ephemeral generation identity. Durable authored object identities are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GameplaySourceKey(pub(crate) u64);
#[derive(Component, Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct SourceOrigin {
    pub(crate) key: GameplaySourceKey,
    pub(crate) container: String,
    pub(crate) object: String,
    pub(crate) ordinal: usize,
}

/// Everything one loaded tile owns.
#[derive(Default)]
pub(crate) struct LoadedTile {
    pub(crate) source_key: Option<GameplaySourceKey>,
    pub(crate) source_container: String,
    pub(crate) gameplay_enabled: bool,
    pub(crate) gameplay_started: bool,
    pub(crate) entities: Vec<Entity>,
    pub(crate) containers: std::collections::HashSet<String>,
    pub(crate) pending: Vec<PendingModel>,
    /// Asset paths this tile asked for (reference-counted in `Streamer::handles`).
    pub(crate) models: Vec<String>,
    /// References introduced by each mission generation, removable without touching
    /// models that this tile or another run owns.
    pub(crate) mission_models: HashMap<MissionOwner, Vec<String>>,
    pub(crate) terrain_triangles: usize,
    pub(crate) objects_total: usize,
    pub(crate) objects_spawned: usize,
    pub(crate) objects_failed: usize,
    model_triangles: usize,
    collision_instances: usize,
    pub(crate) collision_missing: usize,
    pub(crate) unresolved: usize,
    /// Vehicles the tile's spawn sets park here (`fleet.rs` spawns them).
    pub(crate) vehicle_spawns: Vec<jc2_formats::placement::VehicleSpawn>,
    /// Exported entities its spawn sets place: weapon stashes (`pickups.rs`).
    pub(crate) exported_spawns: Vec<jc2_formats::placement::ExportedSpawn>,
    /// The soldiers its team spawn sets place, their routes and guard areas (`npc.rs`).
    pub(crate) ai: jc2_formats::character::AiPlacements,
    /// Its destructible units, indexed by `Piece::unit`.
    pub(crate) units: Vec<UnitData>,
    /// How many things count towards each settlement's completion here (`chaos.rs`).
    pub(crate) counters: std::collections::BTreeMap<String, u32>,
    /// Its resource crates: weapon, vehicle and armour parts, cash (`pickups.rs`).
    pub(crate) resources: Vec<jc2_formats::pickups::ResourceItem>,
    /// The colonels its spawn sets place (`npc.rs` spawns them, `missions.rs` pays).
    pub(crate) colonels: Vec<jc2_formats::missions::Colonel>,
    /// Its 64 x 64 game zone bytes (8 m cells, row = z), for the light set under the camera
    /// (`sky_light.rs`); empty when the terrain file did not load.
    pub(crate) gamezone: Vec<u8>,
}

/// The tile streamer: what is loaded, and the caches shared between tiles.
#[derive(Resource)]
pub(crate) struct Streamer {
    next_gameplay_generation: u64,
    pub(crate) loaded: HashMap<(u32, u32), LoadedTile>,
    pub(crate) mission_tiles: HashMap<MissionOwner, std::collections::HashSet<(u32, u32)>>,
    pub(crate) location_tiles: HashMap<String, ((u32, u32), LoadedTile)>,
    pub(crate) index: ModelIndex,
    /// Model asset handles by path, with how many loaded tiles use them.
    pub(crate) handles: HashMap<String, (Handle<RbmAsset>, usize)>,
    pub(crate) colliders: HashMap<String, Option<ColliderParts>>,
    /// Fallback material (vertex colours) for tiles the splat material cannot draw.
    terrain_material: Handle<StandardMaterial>,
    /// The game's terrain materials and atlases; `None` without the climate tables.
    splat: Option<SplatContext>,
    sea: Option<Entity>,
    sea_centre: (u32, u32),
    pub(crate) focus: (u32, u32),
    start: Transform,
    spawn_point_set: bool,
    pub(crate) unresolved: BTreeMap<String, usize>,
    loads: usize,
    unloads: usize,
    /// How long the last tile took to build (ms), for the HUD.
    last_load_ms: f32,
}

impl Streamer {
    pub(crate) fn next_source(&mut self) -> GameplaySourceKey {
        self.next_gameplay_generation = self
            .next_gameplay_generation
            .checked_add(1)
            .expect("gameplay generation exhausted");
        GameplaySourceKey(self.next_gameplay_generation)
    }
    pub(crate) fn gameplay_sources(
        &self,
    ) -> impl Iterator<Item = (GameplaySourceKey, (u32, u32), &LoadedTile)> {
        self.loaded
            .iter()
            .map(|(k, t)| (*k, t))
            .chain(self.location_tiles.values().map(|(k, t)| (*k, t)))
            .filter(|(_, t)| t.gameplay_enabled)
            .filter_map(|(k, t)| t.source_key.map(|s| (s, k, t)))
    }
    pub(crate) fn source_alive(&self, key: GameplaySourceKey) -> bool {
        self.gameplay_sources().any(|(s, _, _)| s == key)
    }
    pub(crate) fn tile_for_piece_mut(&mut self, piece: Piece) -> Option<&mut LoadedTile> {
        match piece.source {
            Some(key) => self
                .loaded
                .values_mut()
                .chain(self.location_tiles.values_mut().map(|(_, t)| t))
                .find(|t| t.source_key == Some(key) && t.gameplay_enabled),
            None => self.loaded.get_mut(&piece.tile),
        }
    }

    /// The handle of a model asset path, reference-counted per loaded tile: the caller
    /// records the path in its `LoadedTile::models` so `unload_tile` can let go of it.
    pub(crate) fn model_handle(&mut self, assets: &AssetServer, path: &str) -> Handle<RbmAsset> {
        match self.handles.get_mut(path) {
            Some((h, n)) => {
                *n += 1;
                h.clone()
            }
            None => {
                let h: Handle<RbmAsset> = assets.load(path.to_string());
                self.handles.insert(path.to_string(), (h.clone(), 1));
                h
            }
        }
    }

    pub(crate) fn stat(&self, f: impl Fn(&LoadedTile) -> usize) -> usize {
        self.loaded.values().map(f).sum()
    }
}

#[derive(Component)]
pub struct FlyCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub speed: f32,
}

#[derive(Component)]
struct Hud;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (stream_tiles, spawn_models, fly_camera, hud).chain(),
        );
    }
}

/// Colour of a terrain vertex from its height and slope.
pub(crate) fn terrain_colour(height: f32, up: f32) -> [f32; 4] {
    // albedo values; the scene is lit at Bevy's calibrated daylight, so keep these in
    // the range real ground sits in (grass ≈ 0.1–0.2, dry soil ≈ 0.3, fresh snow ≈ 0.8)
    let grass = Vec3::new(0.16, 0.26, 0.09);
    let dry = Vec3::new(0.36, 0.31, 0.18);
    let rock = Vec3::new(0.30, 0.28, 0.26);
    let snow = Vec3::new(0.82, 0.84, 0.88);
    let sand = Vec3::new(0.58, 0.52, 0.36);
    let seabed = Vec3::new(0.18, 0.26, 0.22);
    let above = height - SEA_LEVEL;
    let mut c = if above < -2.0 {
        seabed
    } else if above < 6.0 {
        sand
    } else if above < 400.0 {
        grass.lerp(dry, ((above - 60.0) / 340.0).clamp(0.0, 1.0) * 0.5)
    } else if above < 1300.0 {
        dry.lerp(rock, ((above - 400.0) / 900.0).clamp(0.0, 1.0))
    } else {
        rock.lerp(snow, ((above - 1300.0) / 250.0).clamp(0.0, 1.0))
    };
    // steep → rock
    let steep = ((0.80 - up) / 0.25).clamp(0.0, 1.0);
    c = c.lerp(rock, steep);
    [c.x, c.y, c.z, 1.0]
}

/// One tile as a 129×129 mesh (its own 128 cells; the extra row/column comes from the
/// margin so neighbouring tiles share their edge vertices exactly). With the landscape
/// tables and the tile's texture zone, vertices are coloured by the materials the
/// game draws there (mean atlas colours); otherwise by height and slope.
fn terrain_mesh(
    tile: &TerrainTile,
    x0: f32,
    z0: f32,
    palette: Option<(&Landscape, u8)>,
) -> (Mesh, usize) {
    let n = TILE_CELLS + 1;
    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut colours = Vec::with_capacity(n * n);
    let palette = palette.filter(|_| tile.has_layers());
    for cz in 0..n {
        for cx in 0..n {
            let (r, c) = (cz + TILE_MARGIN, cx + TILE_MARGIN);
            let h = tile.height(r, c);
            // central differences, margin makes neighbours always available
            let dx = (tile.height(r, c + 1) - tile.height(r, c - 1)) / (2.0 * CELL_SIZE);
            let dz = (tile.height(r + 1, c) - tile.height(r - 1, c)) / (2.0 * CELL_SIZE);
            let normal = to_bevy([-dx, 1.0, -dz]).normalize();
            positions.push(
                bevy_xyz(x0 + cx as f32 * CELL_SIZE, h, z0 + cz as f32 * CELL_SIZE).to_array(),
            );
            normals.push(normal.to_array());
            let colour = palette
                .and_then(|(land, zone)| {
                    let cell = tile.meta_cell(cz.min(TILE_CELLS - 1), cx.min(TILE_CELLS - 1));
                    vertex_colour(land, zone, &cell, normal.y)
                })
                .unwrap_or_else(|| terrain_colour(h, normal.y));
            colours.push(colour);
        }
    }
    let mut indices: Vec<u32> = Vec::with_capacity(TILE_CELLS * TILE_CELLS * 6);
    for cz in 0..TILE_CELLS {
        for cx in 0..TILE_CELLS {
            let a = (cz * n + cx) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // counter-clockwise seen from above (+x east, +z south: a → c → b)
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let tris = indices.len() / 3;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
    (mesh, tris)
}

/// A parsed, inflated SARC container.
pub(crate) struct Container {
    pub(crate) name: String,
    pub(crate) sarc: SmallArchive,
    data: Vec<u8>,
}

impl Container {
    fn open(game: &Game, name: &str) -> Option<Arc<Container>> {
        let data = game.fs.read_inflated(name).ok()?;
        let sarc = SmallArchive::parse(&data).ok()?;
        Some(Arc::new(Container {
            name: jc2_formats::location::canonical(name),
            sarc,
            data,
        }))
    }

    pub(crate) fn member(&self, name: &str) -> Option<&[u8]> {
        let e = self.sarc.get(name)?;
        self.sarc.data(&self.data, e).ok()
    }
}

/// The far (`.flz`) companion of a near location container (`.nlz`): the exe opens
/// every world location as a near and a far `CLocation` (`FUN_00830560`, suffix `.nl` /
/// `.fl` by `FUN_005f4780`), and the far one's `.fl.cgd` carries the location's large
/// static structures (research/formats/location-static-geometry-next.md).
pub(crate) fn far_companion(near: &str) -> Option<String> {
    let lower = near.to_ascii_lowercase();
    lower
        .ends_with(".nlz")
        .then(|| format!("{}.flz", &near[..near.len() - 4]))
}

/// Resolves the `filename` references of location files to `(container, model)`:
/// the tile's own container, then `general.blz`, then the tile's area set, then the
/// other area sets (see `jc2_formats::areaset`). Containers open lazily and stay open.
pub(crate) struct ModelIndex {
    areasets: Option<AreaSets>,
    open: HashMap<String, Option<Arc<Container>>>,
    aliases: HashMap<String, Option<String>>,
}

impl ModelIndex {
    fn new(game: &Game) -> Self {
        let areasets = game
            .fs
            .read_inflated("areasets.bin")
            .ok()
            .zip(game.fs.read_inflated("areasets.bmp").ok())
            .and_then(|(bin, bmp)| AreaSets::parse(&bin, &bmp).ok());
        if areasets.is_none() {
            warn!("areasets.bin/.bmp not readable: regional props may be missing");
        }
        let mut aliases: HashMap<String, Option<String>> = HashMap::new();
        for name in game
            .fs
            .entries()
            .iter()
            .filter_map(|e| game.fs.names.file(e.name_hash))
        {
            let path = jc2_formats::location::canonical(name);
            let bare = bare_name(&path).to_string();
            aliases
                .entry(bare)
                .and_modify(|old| {
                    if old.as_ref().is_some_and(|o| *o != path) {
                        *old = None;
                    }
                })
                .or_insert(Some(path));
        }
        Self {
            areasets,
            open: HashMap::new(),
            aliases,
        }
    }

    /// The cache key of a container name: full paths as they are, bare names through
    /// the archive index when it names exactly one file (`None` when ambiguous).
    fn key_of(&self, name: &str) -> Option<String> {
        let normalized = jc2_formats::location::canonical(name);
        if normalized.contains('/') {
            return Some(normalized);
        }
        match self.aliases.get(&normalized) {
            Some(Some(path)) => Some(path.clone()),
            Some(None) => None,
            None => Some(normalized),
        }
    }

    pub(crate) fn container(&mut self, game: &Game, name: &str) -> Option<Arc<Container>> {
        let key = self.key_of(name)?;
        self.open
            .entry(key.clone())
            .or_insert_with(|| Container::open(game, &key))
            .clone()
    }

    /// Drop a location's own container, and its far companion, from the cache (shared
    /// ones stay).
    pub(crate) fn forget(&mut self, name: &str) {
        for n in std::iter::once(name.to_string()).chain(far_companion(name)) {
            if let Some(key) = self.key_of(&n) {
                self.open.remove(&key);
            }
            self.open.remove(&jc2_formats::location::canonical(&n));
        }
    }

    /// Shared containers to search for tile `(x, z)`, most likely first.
    fn shared_for(&self, x: u32, z: u32) -> Vec<String> {
        let mut out = vec![SHARED_CONTAINER.to_string()];
        if let Some(a) = &self.areasets {
            if let Some(set) = a.set_for_tile(x, z)
                && set.folder != "general"
            {
                out.push(set.container());
            }
            for c in a.regional_containers() {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&c)) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Bytes of a collision file (`.pfx`, bare name) from the tile's container or the
    /// shared ones.
    pub(crate) fn shape_bytes(
        &mut self,
        game: &Game,
        local: &Arc<Container>,
        tile: (u32, u32),
        bare: &str,
    ) -> Option<Vec<u8>> {
        if let Some(b) = local.member(bare) {
            return Some(b.to_vec());
        }
        if let Ok(b) = game.fs.read_inflated(bare) {
            return Some(b);
        }
        for name in self.shared_for(tile.0, tile.1) {
            if let Some(c) = self.container(game, &name)
                && let Some(b) = c.member(bare)
            {
                return Some(b.to_vec());
            }
        }
        None
    }

    pub(crate) fn resolve(
        &mut self,
        game: &Game,
        local: &Arc<Container>,
        tile: (u32, u32),
        filename: &str,
        lod: usize,
    ) -> Option<(String, String)> {
        let bare = bare_name(filename);
        if bare.to_ascii_lowercase().ends_with(".rbm") {
            return Some((local.name.clone(), bare.to_string()));
        }
        let mut searched: Vec<Arc<Container>> = vec![local.clone()];
        let shared = self.shared_for(tile.0, tile.1);
        let mut shared_iter = shared.iter();
        let mut i = 0;
        loop {
            let Some(c) = searched.get(i).cloned() else {
                // open the next shared container on demand
                let Some(next) = shared_iter.next() else {
                    break;
                };
                if let Some(c) = self.container(game, next) {
                    searched.push(c);
                }
                continue;
            };
            i += 1;
            let Some(bytes) = c.member(bare) else {
                continue;
            };
            let d = LodDescriptor::parse(&String::from_utf8_lossy(bytes));
            let model = d.model_for_level(lod)?.to_string();
            // the model normally sits next to its descriptor; otherwise wherever it is
            if c.member(&model).is_some() {
                return Some((c.name.clone(), model));
            }
            for other in &searched {
                if other.member(&model).is_some() {
                    return Some((other.name.clone(), model));
                }
            }
            for name in shared.iter() {
                if let Some(other) = self.container(game, name)
                    && other.member(&model).is_some()
                {
                    return Some((other.name.clone(), model));
                }
            }
            return Some((c.name.clone(), model));
        }
        None
    }
}

/// Load one tile: terrain mesh + collider, the collision instances, and the model
/// placements (queued until their assets arrive).
#[allow(clippy::too_many_arguments)]
fn load_tile(
    st: &mut Streamer,
    x: u32,
    z: u32,
    commands: &mut Commands,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    tex: &mut TerrainAssets,
    game: &Game,
    cfg: &WorldConfig,
) -> LoadedTile {
    let mut tile = LoadedTile {
        source_key: Some(st.next_source()),
        source_container: jc2_formats::location::container_identity(
            &game.fs,
            &tile_container_name(x, z),
        )
        .unwrap_or_else(|_| tile_container_name(x, z)),
        gameplay_enabled: true,
        ..Default::default()
    };
    let (x0, z0) = tile_origin(x, z);
    let name = tile_file_name(x, z);
    match game
        .fs
        .read_inflated(&name)
        .and_then(|d| TerrainTile::parse(&d))
    {
        Ok(terrain) => {
            let palette = st
                .splat
                .as_ref()
                .map(|s| (&*s.land, s.land.texture_zone(x, z)));
            let (mesh, tris) = terrain_mesh(&terrain, x0, z0, palette);
            // the game's materials through the splat shader, else vertex colours
            let splat = st.splat.as_ref().and_then(|s| {
                s.material_for_tile(&terrain, x, z, &mut tex.images, &mut tex.materials)
            });
            let mut e = commands.spawn((
                Mesh3d(meshes.add(mesh)),
                Transform::IDENTITY,
                Name::new(format!("terrain {x},{z}")),
            ));
            match splat {
                Some(m) => {
                    e.insert(MeshMaterial3d(m));
                }
                None => {
                    e.insert(MeshMaterial3d(st.terrain_material.clone()));
                }
            }
            let e = e.id();
            tile.entities.push(e);
            if cfg.physics {
                let (collider, tf) = terrain_collider(&terrain, x0, z0);
                let e = commands
                    .spawn((
                        RigidBody::Static,
                        collider,
                        tf,
                        TerrainBody,
                        Name::new(format!("terrain collider {x},{z}")),
                    ))
                    .id();
                tile.entities.push(e);
            }
            tile.terrain_triangles = tris;
            tile.gamezone = terrain.gamezone.clone();
        }
        Err(e) => warn!("{name}: {e}"),
    }
    if !cfg.models {
        return tile;
    }
    let container = tile_container_name(x, z);
    load_container(st, &mut tile, x, z, &container, commands, assets, game, cfg);
    info!(
        "tile {x},{z}: {} models, {} collision shapes",
        tile.objects_total, tile.collision_instances
    );
    tile
}

/// One location container's objects into a tile: its collision list, spawn sets, soldiers,
/// destructible units, crates, colonels and models. The tile's own `XXxZZ.nlz`, then the
/// `ExtraContainers` that load with it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_container(
    st: &mut Streamer,
    tile: &mut LoadedTile,
    x: u32,
    z: u32,
    container: &str,
    commands: &mut Commands,
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
) {
    let Ok(identity) = jc2_formats::location::container_identity(&game.fs, container) else {
        return;
    };
    if tile.containers.contains(&identity) {
        return;
    }
    if !game.fs.contains(container) {
        return;
    }
    let Some(local) = st.index.container(game, container) else {
        warn!("{container}: not a SARC container");
        return;
    };
    let Some(blo) = local
        .sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".blo"))
    else {
        return;
    };
    let file = match local
        .member(&blo.name)
        .ok_or_else(|| jc2_formats::Error::format("unreadable member"))
        .and_then(|b| jc2_formats::compress::maybe_inflate(b).map(|c| c.into_owned()))
        .and_then(|b| PropertyFile::parse_any(&b))
    {
        Ok(f) => f,
        Err(e) => {
            warn!("{container}/{}: {e}", blo.name);
            return;
        }
    };
    tile.containers.insert(identity);
    // The container's own collision: Havok shapes placed by its .pfi list (rocks and other
    // landscape props that are not placed objects)…
    let mut has_pfi = false;
    if cfg.physics
        && let Some(pfi) = local
            .sarc
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".pfi"))
        && let Some(bytes) = local.member(&pfi.name)
    {
        match CollisionInstances::parse(bytes) {
            Ok(list) => {
                has_pfi = true;
                for i in 0..list.instances.len() {
                    let tf = mat_to_bevy(&list.world(i));
                    place_shape(
                        st,
                        tile,
                        commands,
                        game,
                        &local,
                        (x, z),
                        &list.instances[i].pfx,
                        tf,
                        None,
                    );
                }
            }
            Err(e) => warn!("{container}/{}: {e}", pfi.name),
        }
    }
    tile.vehicle_spawns
        .extend(jc2_formats::placement::vehicle_spawns(&file));
    tile.exported_spawns
        .extend(jc2_formats::placement::exported_spawns(&file));
    let ai = jc2_formats::character::AiPlacements::from_property(&file);
    tile.ai.teams.extend(ai.teams);
    tile.ai.routes.extend(ai.routes);
    tile.ai.areas.extend(ai.areas);
    for (id, n) in jc2_formats::destruction::settlement_totals(&file) {
        *tile.counters.entry(id).or_insert(0) += n;
    }
    tile.resources
        .extend(jc2_formats::pickups::resource_items(&file));
    tile.colonels.extend(jc2_formats::missions::colonels(&file));
    place_objects(
        st,
        tile,
        (x, z),
        &local,
        &file,
        None,
        has_pfi,
        commands,
        assets,
        game,
        cfg,
    );
}

/// The destructible units and the models of a placement file, into `tile`: the intact
/// parts tagged with their unit, the debris kept for later. `base` places a file whose
/// objects are relative (an exported entity put down by a spawn set). Returns the units
/// it added (indices into `tile.units`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_objects(
    st: &mut Streamer,
    tile: &mut LoadedTile,
    xz: (u32, u32),
    local: &Arc<Container>,
    file: &PropertyFile,
    base: Option<&jc2_formats::placement::Transform3>,
    has_pfi: bool,
    commands: &mut Commands,
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
) -> Vec<u32> {
    let (x, z) = xz;
    // what can be destroyed: the intact parts are tagged, the debris kept for later
    let mut all = placements(file);
    if let Some(b) = base {
        for p in &mut all {
            p.world = p.world.then(b);
        }
    }
    let mut piece_of: HashMap<usize, u32> = HashMap::new();
    let mut added = Vec::new();
    for d in jc2_formats::destruction::destructibles(file) {
        let unit = tile.units.len() as u32;
        added.push(unit);
        for &i in &d.intact {
            piece_of.insert(i, unit);
        }
        let mut debris = Vec::new();
        for &(i, mass) in &d.debris {
            let p = &all[i];
            let model = p.model.as_ref().and_then(|m| {
                st.index
                    .resolve(game, local, (x, z), m, cfg.lod)
                    .map(|(home, rbm)| format!("jc2://{home}/{rbm}"))
            });
            let shape = match &p.collision {
                Some(c) if cfg.physics => shape_parts(st, game, local, (x, z), c),
                _ => None,
            };
            debris.push(DebrisData {
                model,
                transform: mat_to_bevy(&p.world),
                shape,
                mass,
                grapple_weight: p.grapple_weight,
            });
        }
        let center = base.map_or(d.center, |b| b.transform_point(d.center));
        tile.units.push(UnitData {
            name: d.name,
            health: d.health,
            center: to_bevy(center),
            explodes: d.explodes,
            debris,
            destroyed: false,
            activity: d.activity,
            counter: d.counter,
            fires: d.fires,
            mission_owner: None,
        });
    }
    for (index, p) in all.into_iter().enumerate() {
        let Some(model) = &p.model else { continue };
        if !p.visible {
            continue;
        }
        let piece = piece_of.get(&index).copied();
        let Some((home, rbm)) = st.index.resolve(game, local, (x, z), model, cfg.lod) else {
            tile.unresolved += 1;
            *st.unresolved
                .entry(bare_name(model).to_string())
                .or_default() += 1;
            continue;
        };
        let path = format!("jc2://{home}/{rbm}");
        let handle = match st.handles.get_mut(&path) {
            Some((h, n)) => {
                *n += 1;
                h.clone()
            }
            None => {
                let h: Handle<RbmAsset> = assets.load(path.clone());
                st.handles.insert(path.clone(), (h.clone(), 1));
                h
            }
        };
        tile.models.push(path);
        let transform = mat_to_bevy(&p.world);
        // …and every placed object carries its own shape (`collision` in the .blo)
        let own_shape = match &p.collision {
            Some(c) if cfg.physics => {
                place_shape(st, tile, commands, game, local, (x, z), c, transform, piece)
            }
            _ => false,
        };
        tile.pending.push(PendingModel {
            handle,
            transform,
            name: format!("{} ({})", p.name, rbm),
            mesh_colliders: cfg.physics && !has_pfi && !own_shape,
            piece,
            vis_range: None,
            mission_owner: None,
            lod: None,
        });
        tile.objects_total += 1;
    }
    added
}

/// Put an exported entity (`factiontaskresources\f2t10\f2t10.gb017b.ee`: its `.epe`
/// objects) down on loaded tile `key` at `base` (engine axes): its models, collision and
/// destructible units join the tile's. The units it added, `None` when the tile is not
/// loaded or the entity is unreadable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_entity(
    st: &mut Streamer,
    key: (u32, u32),
    entity: &str,
    base: &jc2_formats::placement::Transform3,
    commands: &mut Commands,
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
    owner: MissionOwner,
) -> Option<Vec<u32>> {
    if !st.loaded.contains_key(&key) {
        return None;
    }
    let local = st.index.container(game, bare_name(entity))?;
    let member = local
        .sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".epe"))?
        .name
        .clone();
    let file = local
        .member(&member)
        .and_then(|b| {
            jc2_formats::compress::maybe_inflate(b)
                .ok()
                .map(|c| c.into_owned())
        })
        .and_then(|b| PropertyFile::parse_any(&b).ok())?;
    let mut tile = st.loaded.remove(&key)?;
    let entities_before = tile.entities.len();
    let pending_before = tile.pending.len();
    let models_before = tile.models.len();
    let units = place_objects(
        st,
        &mut tile,
        key,
        &local,
        &file,
        Some(base),
        false,
        commands,
        assets,
        game,
        cfg,
    );
    for &entity in &tile.entities[entities_before..] {
        commands.entity(entity).insert(owner);
    }
    for pending in &mut tile.pending[pending_before..] {
        pending.mission_owner = Some(owner);
    }
    for &unit in &units {
        tile.units[unit as usize].mission_owner = Some(owner);
    }
    tile.mission_models
        .entry(owner)
        .or_default()
        .extend_from_slice(&tile.models[models_before..]);
    st.loaded.insert(key, tile);
    Some(units)
}

/// Spawn the static colliders of one Havok `.pfx` at a world transform (cached per
/// shape name). Returns whether the shape was found.
#[allow(clippy::too_many_arguments)]
fn place_shape(
    st: &mut Streamer,
    tile: &mut LoadedTile,
    commands: &mut Commands,
    game: &Game,
    local: &Arc<Container>,
    xz: (u32, u32),
    pfx: &str,
    tf: Transform,
    piece: Option<u32>,
) -> bool {
    let key = bare_name(pfx).to_ascii_lowercase();
    let Some(parts) = shape_parts(st, game, local, xz, pfx) else {
        tile.collision_missing += 1;
        *st.unresolved.entry(key).or_default() += 1;
        return false;
    };
    for (p, r, collider) in parts {
        let part_tf = Transform {
            translation: tf.transform_point(p),
            rotation: tf.rotation * r,
            scale: Vec3::ONE,
        };
        let mut e = commands.spawn((
            RigidBody::Static,
            collider,
            part_tf,
            Name::new(format!("collision {}", bare_name(pfx))),
        ));
        if let Some(unit) = piece {
            e.insert(Piece {
                tile: xz,
                unit,
                source: tile.source_key,
            });
        }
        tile.entities.push(e.id());
    }
    tile.collision_instances += 1;
    true
}

/// The collider parts of a Havok `.pfx` (cached per shape name).
pub(crate) fn shape_parts(
    st: &mut Streamer,
    game: &Game,
    local: &Arc<Container>,
    xz: (u32, u32),
    pfx: &str,
) -> Option<ColliderParts> {
    let bare = bare_name(pfx).to_ascii_lowercase();
    let key = format!("{}/{}", local.name, bare);
    match st.colliders.get(&key) {
        Some(p) => p.clone(),
        None => {
            let bytes = st.index.shape_bytes(game, local, xz, &bare);
            let p = bytes
                .and_then(|b| parse_pfx(&b).ok())
                .and_then(|bodies| bodies.first().map(colliders_from_body))
                .filter(|p| !p.is_empty());
            st.colliders.insert(key, p.clone());
            p
        }
    }
}

fn unload_tile(st: &mut Streamer, key: (u32, u32), commands: &mut Commands) {
    let Some(tile) = st.loaded.remove(&key) else {
        return;
    };
    for e in tile.entities {
        commands.entity(e).try_despawn();
    }
    for path in tile.models {
        if let Some((_, n)) = st.handles.get_mut(&path) {
            *n -= 1;
            if *n == 0 {
                st.handles.remove(&path);
            }
        }
    }
    st.index.forget(&tile_container_name(key.0, key.1));
    st.unloads += 1;
}

pub(crate) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: Option<ResMut<Assets<bevy::render::storage::ShaderBuffer>>>,
    game: Res<Game>,
    cfg: Res<WorldConfig>,
) {
    let (tx, tz) = cfg.tile;
    let splat = if cfg.flat_terrain {
        None
    } else {
        SplatContext::new(&game, &mut images, buffers.as_deref_mut())
    };
    if let Some(splat) = &splat {
        commands.insert_resource(splat.horizon.clone());
    }
    let terrain_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.1,
        ..default()
    });
    if !game.fs.contains(SHARED_CONTAINER) {
        warn!("{SHARED_CONTAINER} not found: shared props will be missing");
    }
    // ground height at the start, for the camera and the spawn point
    let (cx0, cz0) = tile_origin(tx, tz);
    let centre_height = game
        .fs
        .read_inflated(&tile_file_name(tx, tz))
        .ok()
        .and_then(|d| TerrainTile::parse(&d).ok())
        .and_then(|t| t.sample_local(TILE_SIZE / 2.0, TILE_SIZE / 2.0));
    let ground = centre_height.unwrap_or(SEA_LEVEL).max(SEA_LEVEL);

    // the sea: a plane out to beyond the fog, following the focus tile
    let span = 2.0 * (crate::horizon::FAR + 2.0 * TILE_SIZE);
    let sea = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(span, span))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(0.05, 0.25, 0.45, 0.75),
                perceptual_roughness: 0.15,
                metallic: 0.0,
                alpha_mode: AlphaMode::Blend,
                ..default()
            })),
            Transform::from_translation(bevy_xyz(
                cx0 + TILE_SIZE / 2.0,
                SEA_LEVEL,
                cz0 + TILE_SIZE / 2.0,
            )),
            Name::new("sea"),
        ))
        .id();

    // camera: south of the start tile's middle and above it, looking north (−z, yaw 0),
    // so the map's north is up the screen and east (+x) to the right
    // JC2_CAM_HEIGHT (metres above the tile middle's ground, default 160) for aerial shots
    let height = std::env::var("JC2_CAM_HEIGHT")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(160.0);
    let eye = bevy_xyz(
        cx0 + TILE_SIZE / 2.0,
        ground + height,
        cz0 + TILE_SIZE / 2.0 + 200.0,
    );
    // JC2_CAM_PITCH / JC2_CAM_YAW (degrees) aim the free camera's start (smoke shots of the sky)
    let pitch = std::env::var("JC2_CAM_PITCH")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(f32::to_radians)
        .unwrap_or(-0.45);
    let yaw = std::env::var("JC2_CAM_YAW")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(f32::to_radians)
        .unwrap_or(0.0);
    let cam = FlyCamera {
        yaw,
        pitch,
        speed: 60.0,
    };
    let mut tf = Transform::from_translation(eye);
    tf.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    commands.insert_resource(Streamer {
        next_gameplay_generation: 0,
        loaded: HashMap::new(),
        location_tiles: HashMap::new(),
        mission_tiles: HashMap::new(),
        index: ModelIndex::new(&game),
        handles: HashMap::new(),
        colliders: HashMap::new(),
        terrain_material,
        splat,
        sea: Some(sea),
        sea_centre: (tx, tz),
        focus: (tx, tz),
        start: tf,
        spawn_point_set: false,
        unresolved: BTreeMap::new(),
        loads: 0,
        unloads: 0,
        last_load_ms: 0.0,
    });
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            far: 30_000.0,
            ..default()
        }),
        tf,
        cam,
        // the native final chain (`post_effects.rs`): an 8-bit sRGB scene target (no HDR),
        // no tone-mapping curve and no dither (the exe has neither), the camera exposure the
        // light units are derived from (`sky_light.rs`), and the final pass
        Tonemapping::None,
        DebandDither::Disabled,
        Exposure::BLENDER,
        crate::post_effects::PostPass::default(),
        AmbientLight {
            color: Color::srgb(0.70, 0.80, 1.0),
            brightness: 140.0,
            ..default()
        },
        // haze: hides the edge of the loaded region and gives depth to the ridges
        DistanceFog {
            color: SKY,
            directional_light_color: Color::srgba(1.0, 0.95, 0.85, 0.5),
            directional_light_exponent: 30.0,
            falloff: FogFalloff::from_visibility_colors(
                12_000.0,
                Color::srgb(0.35, 0.45, 0.55),
                Color::srgb(0.75, 0.82, 0.90),
            ),
        },
    ));
    commands.insert_resource(ClearColor(SKY));
    // the sun/moon light and the ambient are `sky.rs`'s (`sky_light.rs`): they follow the time of day
    commands.spawn((
        Text::new("loading…"),
        TextFont::from_font_size(14.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(8.0),
            ..default()
        },
        Hud,
        crate::hud::DevText,
    ));
}

type SeaQuery<'w, 's> = Query<
    'w,
    's,
    &'static mut Transform,
    (
        Without<FlyCamera>,
        Without<crate::vehicle::Occupied>,
        Without<crate::character::Hero>,
    ),
>;

/// Keep the tiles around the focus (the driven vehicle, else the camera) loaded: drop
/// what fell out of range, load the nearest missing tile — one per frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stream_tiles(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tex: TerrainAssets,
    game: Res<Game>,
    cfg: Res<WorldConfig>,
    st: Option<ResMut<Streamer>>,
    heroes: Query<&Transform, With<crate::character::Hero>>,
    players: Query<&Transform, With<crate::vehicle::Occupied>>,
    cams: Query<&Transform, With<FlyCamera>>,
    mut seas: SeaQuery,
    start_at: Option<Res<StartAt>>,
) {
    let Some(mut st) = st else { return };
    // Rico (he rides along in the car), else the car, else the camera
    let focus_pos = heroes
        .single()
        .ok()
        .or_else(|| players.single().ok())
        .or_else(|| cams.single().ok())
        .map(|t| t.translation);
    if let Some(p) = focus_pos
        && let Some(t) = {
            let (x, z) = engine_xz(p);
            jc2_formats::terrain::tile_index(x, z)
        }
    {
        st.focus = t;
    }
    let (fx, fz) = st.focus;
    let r = cfg.radius as i64;
    let dist = |k: (u32, u32)| {
        (k.0 as i64 - fx as i64)
            .abs()
            .max((k.1 as i64 - fz as i64).abs())
    };
    // unload what is further than radius + 1
    let gone: Vec<(u32, u32)> = st
        .loaded
        .keys()
        .copied()
        .filter(|k| {
            dist(*k) > r + 1
                && st.loaded[k].mission_models.is_empty()
                && !st.mission_tiles.values().any(|keys| keys.contains(k))
        })
        .collect();
    for k in gone {
        unload_tile(&mut st, k, &mut commands);
    }
    // load the nearest missing tile
    let mut wanted: Vec<(u32, u32)> = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            let (x, z) = (fx as i64 + dx, fz as i64 + dz);
            if x < 0 || z < 0 || x >= TILES_PER_SIDE as i64 || z >= TILES_PER_SIDE as i64 {
                continue;
            }
            let k = (x as u32, z as u32);
            if !st.loaded.contains_key(&k) {
                wanted.push(k);
            }
        }
    }
    for &key in st.mission_tiles.values().flatten() {
        if !st.loaded.contains_key(&key) && !wanted.contains(&key) {
            wanted.push(key);
        }
    }
    wanted.sort_by_key(|k| dist(*k));
    if let Some(k) = wanted.first().copied() {
        let t0 = std::time::Instant::now();
        let tile = load_tile(
            &mut st,
            k.0,
            k.1,
            &mut commands,
            &assets,
            &mut meshes,
            &mut tex,
            &game,
            &cfg,
        );
        st.last_load_ms = t0.elapsed().as_secs_f32() * 1000.0;
        st.loaded.insert(k, tile);
        st.loads += 1;
        if k == cfg.tile && !st.spawn_point_set {
            let (x0, z0) = tile_origin(k.0, k.1);
            // JC2_START_OFFSET=dx,dz moves the start from the tile's middle (smoke tests)
            let (ox, oz) = start_offset();
            let (sx, sz) = match start_at.as_deref() {
                Some(StartAt([ax, az])) => (ax - x0, az - z0),
                None => (TILE_SIZE / 2.0 + ox, TILE_SIZE / 2.0 + oz),
            };
            let ground = game
                .fs
                .read_inflated(&tile_file_name(k.0, k.1))
                .ok()
                .and_then(|d| TerrainTile::parse(&d).ok())
                .and_then(|t| t.sample_local(sx, sz))
                .unwrap_or(SEA_LEVEL)
                .max(SEA_LEVEL);
            commands.insert_resource(SpawnPoint(to_bevy([x0 + sx, ground, z0 + sz])));
            st.spawn_point_set = true;
        }
    }
    // the sea follows the focus
    if st.sea_centre != st.focus
        && let Some(sea) = st.sea
        && let Ok(mut tf) = seas.get_mut(sea)
    {
        let (x0, z0) = tile_origin(fx, fz);
        tf.translation = bevy_xyz(x0 + TILE_SIZE / 2.0, SEA_LEVEL, z0 + TILE_SIZE / 2.0);
        st.sea_centre = st.focus;
    }
}

/// `JC2_START_OFFSET=dx,dz`: metres from the start tile's middle (for smoke tests).
fn start_offset() -> (f32, f32) {
    std::env::var("JC2_START_OFFSET")
        .ok()
        .and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        })
        .unwrap_or((0.0, 0.0))
}

pub(crate) fn spawn_models(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    st: Option<ResMut<Streamer>>,
    mission: Option<Res<ActiveMission>>,
    launches: Option<Res<crate::missions::access::MissionLaunches>>,
    lodfactor: Option<Res<crate::veg_fade::LodFactor>>,
) {
    let Some(mut st) = st else { return };
    let lodfactor = lodfactor.map(|l| *l).unwrap_or_default();
    let Streamer {
        loaded,
        location_tiles,
        ..
    } = &mut *st;
    for (key, tile) in loaded
        .iter_mut()
        .map(|(k, t)| (*k, t))
        .chain(location_tiles.values_mut().map(|(k, t)| (*k, t)))
    {
        if tile.pending.is_empty() {
            continue;
        }
        let mut keep = Vec::with_capacity(tile.pending.len());
        let pending = std::mem::take(&mut tile.pending);
        for p in pending {
            // Prepared resources can realize before activation. A queued model must
            // not appear after that generation was cancelled, ended or replaced.
            if p.mission_owner.is_some_and(|owner| {
                let active = mission
                    .as_ref()
                    .is_some_and(|m| m.0.as_ref().is_some_and(|r| r.owner == owner));
                let preparing = launches
                    .as_ref()
                    .is_some_and(|l| l.preparation().is_some_and(|p| p.owner == owner));
                !active && !preparing
            }) {
                continue;
            }
            let id = p.handle.id();
            if let Some(LoadState::Failed(e)) = assets.get_load_state(id) {
                if tile.objects_failed < 5 {
                    error!("{}: {e}", p.name);
                }
                tile.objects_failed += 1;
                continue;
            }
            let Some(rbm) = rbms.get(&p.handle) else {
                keep.push(p);
                continue;
            };
            let mut tris = 0;
            let mut entity = commands.spawn((
                p.transform,
                Visibility::default(),
                Name::new(p.name.clone()),
            ));
            if let Some(unit) = p.piece {
                entity.insert(Piece {
                    tile: key,
                    unit,
                    source: tile.source_key,
                });
            }
            if let Some(owner) = p.mission_owner {
                entity.insert(owner);
            }
            if let Some(set) = &p.lod {
                entity.insert(crate::plant_lod::PlantLod {
                    set: set.clone(),
                    shown: set.plan.far_model().unwrap_or(0),
                    vis_range: p.vis_range,
                });
            }
            if p.mesh_colliders {
                entity.insert(RigidBody::Static);
            }
            entity.with_children(|parent| {
                for b in &rbm.blocks {
                    let mut child = parent.spawn((
                        Mesh3d(b.mesh.clone()),
                        b.material_component(),
                        Transform::IDENTITY,
                    ));
                    if p.mesh_colliders && b.triangles > 0 {
                        child.insert(static_mesh_collider());
                    }
                    // foliage-material meshes fade out natively (`veg_fade.rs`, JC2-140): no
                    // hand-over range, only a lossless cull; the others keep the project range
                    if let Some(range) =
                        crate::veg_fade::plant_visibility_range(
                            b.foliage.is_some(),
                            p.vis_range,
                            lodfactor.0,
                        )
                    {
                        child.insert(range);
                    }
                    tris += b.triangles;
                }
            });
            tile.entities.push(entity.id());
            tile.objects_spawned += 1;
            tile.model_triangles += tris;
        }
        tile.pending = keep;
    }
}

#[allow(clippy::too_many_arguments)]
fn fly_camera(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    st: Option<Res<Streamer>>,
    mode: Option<Res<CameraMode>>,
    mut cams: Query<(&mut FlyCamera, &mut Transform)>,
) {
    if mode.is_some_and(|m| *m != CameraMode::Free) {
        return;
    }
    for (mut cam, mut tf) in &mut cams {
        if keys.just_pressed(KeyCode::KeyR)
            && let Some(st) = &st
        {
            *tf = st.start;
            cam.yaw = 0.0;
            cam.pitch = -0.45;
            continue;
        }
        if buttons.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO {
            cam.yaw -= motion.delta.x * 0.0025;
            cam.pitch = (cam.pitch - motion.delta.y * 0.0025).clamp(-1.55, 1.55);
            tf.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
        }
        if scroll.delta.y != 0.0 {
            let step = match scroll.unit {
                MouseScrollUnit::Line => scroll.delta.y * 0.15,
                MouseScrollUnit::Pixel => scroll.delta.y * 0.005,
            };
            cam.speed = (cam.speed * (1.0 + step)).clamp(1.0, 3000.0);
        }
        let mut dir = Vec3::ZERO;
        let forward = tf.forward().as_vec3();
        let right = tf.right().as_vec3();
        if keys.pressed(KeyCode::KeyW) {
            dir += forward;
        }
        if keys.pressed(KeyCode::KeyS) {
            dir -= forward;
        }
        if keys.pressed(KeyCode::KeyD) {
            dir += right;
        }
        if keys.pressed(KeyCode::KeyA) {
            dir -= right;
        }
        if keys.pressed(KeyCode::KeyE) || keys.pressed(KeyCode::Space) {
            dir += Vec3::Y;
        }
        if keys.pressed(KeyCode::KeyQ) || keys.pressed(KeyCode::KeyC) {
            dir -= Vec3::Y;
        }
        if dir != Vec3::ZERO {
            let mut speed = cam.speed;
            if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
                speed *= 10.0;
            }
            if keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight) {
                speed *= 0.1;
            }
            tf.translation += dir.normalize() * speed * time.delta_secs();
        }
    }
}

fn hud(
    st: Option<Res<Streamer>>,
    cfg: Res<WorldConfig>,
    cams: Query<(&FlyCamera, &Transform)>,
    lod: Option<Res<crate::terrain_material::TerrainLod>>,
    fade: Option<Res<crate::veg_fade::VegetationFade>>,
    mut q: Query<&mut Text, With<Hud>>,
) {
    let (Some(st), Ok((cam, tf))) = (st, cams.single()) else {
        return;
    };
    // the realised camera FOV and what scales with it (JC2-148)
    let fov_line = match (lod.as_deref(), fade.as_deref()) {
        (Some(l), Some(f)) => format!(
            "fov {:.1} deg   terrain LOD0 {:.1} m / LOD1 {:.1} m   foliage fade {:.0}..{:.0} m   lodfactor {}\n",
            l.fov.to_degrees(),
            l.lod0_radius(),
            l.lod1_radius(),
            f.foliage.x / f.foliage.y,
            f.foliage.x / f.foliage.y + 1.0 / f.foliage.y,
            f.lodfactor,
        ),
        _ => String::new(),
    };
    // engine coordinates
    let [ex, ey, ez] = to_engine(tf.translation);
    let tile = jc2_formats::terrain::tile_index(ex, ez)
        .map(|(x, z)| format!("{x},{z}"))
        .unwrap_or_else(|| "outside".into());
    let pending = st.stat(|t| t.pending.len());
    let failed = st.stat(|t| t.objects_failed);
    let missing = st.stat(|t| t.collision_missing);
    for mut t in &mut q {
        **t = format!(
            "jc2-rs world   start {},{} radius {} lod {}   {} tiles loaded ({} loaded, {} dropped so far, last took {:.0} ms)\n\
             {} terrain tris   {}/{} objects placed{}{}   {} model tris   {} collision shapes{}   {} refs unresolved\n\
             position {ex:.0} {ey:.0} {ez:.0}  (tile {tile})   {:.0} m above sea   speed {:.0} m/s\n\
             {fov_line}right-drag look · WASD move · Space/C up/down · Shift fast · Alt slow · wheel speed · R reset · B ball",
            cfg.tile.0,
            cfg.tile.1,
            cfg.radius,
            cfg.lod,
            st.loaded.len(),
            st.loads,
            st.unloads,
            st.last_load_ms,
            st.stat(|t| t.terrain_triangles),
            st.stat(|t| t.objects_spawned),
            st.stat(|t| t.objects_total),
            if pending == 0 { "" } else { " (loading)" },
            if failed == 0 {
                String::new()
            } else {
                format!(", {failed} failed")
            },
            st.stat(|t| t.model_triangles),
            st.stat(|t| t.collision_instances),
            if missing == 0 {
                String::new()
            } else {
                format!(" ({missing} missing)")
            },
            st.stat(|t| t.unresolved),
            ey - SEA_LEVEL,
            cam.speed,
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::rbm_loader::RbmLoader;
    use bevy::asset::io::AssetSourceBuilder;
    use bevy::asset::{AssetApp, AssetPlugin};
    use jc2_formats::GameFs;
    use jc2_formats::placement::Transform3;
    use std::sync::Arc;

    #[test]
    fn placement_transform_is_the_engines() {
        // rotate 90° about y (engine, row-vector), translate to (10, 20, 30)
        let t = Transform3::from_mat4x3([
            0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 10.0, 20.0, 30.0,
        ]);
        let tf = mat_to_bevy(&t);
        assert!((tf.translation - Vec3::new(10.0, 20.0, 30.0)).length() < 1e-5);
        // engine: the x axis maps to (0,0,-1); a point (1,0,0) → (10,20,30)+(0,0,-1)
        let p = tf.transform_point(Vec3::X);
        assert!((p - Vec3::new(10.0, 20.0, 29.0)).length() < 1e-4, "{p:?}");
        assert!((to_bevy(t.transform_point([1.0, 0.0, 0.0])) - p).length() < 1e-4);
        assert!((tf.scale - Vec3::ONE).length() < 1e-5);
    }

    #[test]
    fn terrain_mesh_shape() {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for _ in 0..132 * 132 {
            d.extend_from_slice(&6000u16.to_le_bytes());
        }
        d.extend(std::iter::repeat_n(0u8, 4 * 128 * 128 * 2));
        let tile = TerrainTile::parse(&d).unwrap();
        let (mesh, tris) = terrain_mesh(&tile, -512.0, -9216.0, None);
        assert_eq!(tris, 128 * 128 * 2);
        assert_eq!(mesh.count_vertices(), 129 * 129);
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap();
        assert_eq!(
            pos[0],
            [-512.0, 6000.0 * jc2_formats::terrain::HEIGHT_SCALE, -9216.0]
        );
        assert_eq!(pos[129 * 129 - 1][0], 0.0);
        assert_eq!(pos[129 * 129 - 1][2], -9216.0 + 512.0);
        // every triangle faces up (counter-clockwise seen from above)
        let idx: Vec<u32> = mesh.indices().unwrap().iter().map(|i| i as u32).collect();
        for &[ia, ib, ic] in idx.as_chunks::<3>().0.iter().take(1000) {
            let [a, b, c] = [ia, ib, ic].map(|i| Vec3::from(pos[i as usize]));
            assert!((b - a).cross(c - a).y > 0.0);
        }
    }

    /// Tile 31,14 streamed in headless with its models and collision (and whatever
    /// `extra` adds), run until everything has spawned. `None` without the test data.
    pub(crate) fn tile_app(extra: impl FnOnce(&mut App)) -> Option<App> {
        tile_app_at((31, 14), extra)
    }

    /// [`tile_app`] on another tile.
    pub(crate) fn tile_app_at(tile: (u32, u32), extra: impl FnOnce(&mut App)) -> Option<App> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        if !fs.contains(&format!("{:02}x{:02}.nlz", tile.0, tile.1)) {
            return None;
        }
        tile_app_with(tile, Vec::new(), extra)
    }

    /// [`tile_app_at`] with location containers that load with the tile
    /// ([`ExtraContainers`]: a stronghold's base); the tile needs no container of its own.
    pub(crate) fn tile_app_with(
        tile: (u32, u32),
        containers: Vec<String>,
        extra: impl FnOnce(&mut App),
    ) -> Option<App> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = Arc::new(GameFs::open(dir).ok()?);
        if !fs.contains(&format!("terrain_win32_{:02}_{:02}.dat", tile.0, tile.1))
            || containers.iter().any(|c| !fs.contains(c))
        {
            return None;
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
        .register_asset_loader(bevy::image::ImageLoader::new(
            bevy::image::CompressedImageFormats::BC,
        ))
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .init_asset::<crate::general_material::GeneralMaterial>()
        .register_asset_loader(RbmLoader::default())
        .insert_resource(Game { fs })
        .insert_resource(WorldConfig {
            tile,
            radius: 0,
            lod: 1,
            models: true,
            physics: true,
            flat_terrain: false,
        })
        .init_asset::<crate::terrain_material::TerrainSplatMaterial>()
        .add_systems(Startup, setup)
        .add_systems(Update, (stream_tiles, spawn_models).chain());
        if !containers.is_empty() {
            app.insert_resource(ExtraContainers([(tile, containers)].into_iter().collect()));
        }
        extra(&mut app);
        if !app.is_plugin_added::<crate::locations::LocationsPlugin>() {
            app.add_plugins(crate::locations::LocationsPlugin);
        }
        app.finish();
        // a camera stands in for the focus
        // (the models load on other threads: wait by the wall clock, not a frame count)
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 60 {
            app.update();
            let s = app.world().resource::<Streamer>();
            if !s.loaded.is_empty()
                && s.stat(|t| t.pending.len()) == 0
                && app
                    .world()
                    .get_resource::<ExtraContainers>()
                    .is_none_or(|e| {
                        e.0.iter()
                            .filter(|(key, _)| s.loaded.contains_key(*key))
                            .flat_map(|(_, names)| names)
                            .all(|name| {
                                let game = app.world().resource::<Game>();
                                let identity =
                                    jc2_formats::location::container_identity(&game.fs, name)
                                        .unwrap();
                                s.loaded.values().any(|t| t.containers.contains(&identity))
                                    || s.location_tiles
                                        .get(&identity)
                                        .is_some_and(|(_, t)| t.gameplay_enabled)
                            })
                    })
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Some(app)
    }

    /// Headless: run the world setup on tile 31,14 of a test game folder and make sure
    /// terrain meshes and placed models come out. Needs JC2_TEST_DIR with the
    /// terrain tiles and `31x14.nlz` (+ `general.blz`) in its dropzone.
    #[test]
    fn builds_a_tile_headless() {
        let Some(mut app) = tile_app(|_| {}) else {
            return;
        };
        let s = app.world().resource::<Streamer>();
        assert_eq!(s.loaded.len(), 1);
        let t = s.loaded.get(&(31, 14)).expect("start tile loaded");
        assert_eq!(t.terrain_triangles, 128 * 128 * 2);
        assert!(t.objects_total > 100, "{}", t.objects_total);
        assert!(t.pending.is_empty(), "{} still pending", t.pending.len());
        assert_eq!(t.objects_failed, 0);
        assert_eq!(t.objects_spawned, t.objects_total);
        assert!(t.model_triangles > 10_000, "{}", t.model_triangles);
        // 167 from the .pfi list plus the placed objects' own shapes
        assert_eq!(
            t.collision_instances,
            167 + 110,
            "{} missing",
            t.collision_missing
        );
        assert_eq!(t.collision_missing, 0);
        assert!(app.world().get_resource::<SpawnPoint>().is_some());
        // its destructibles: five red barrels among them, their parts tagged
        let barrels = t.units.iter().filter(|u| u.name == "dac.barrel.1").count();
        assert_eq!(barrels, 5);
        let tagged = {
            let mut q = app.world_mut().query::<&Piece>();
            q.iter(app.world()).count()
        };
        assert!(tagged >= 17, "{tagged} tagged pieces");
        let s = app.world().resource::<Streamer>();
        let t = s.loaded.get(&(31, 14)).unwrap();
        eprintln!(
            "tile 31,14: {} objects, {} model triangles, {} collision shapes, {} unresolved",
            t.objects_spawned, t.model_triangles, t.collision_instances, t.unresolved
        );
        // move the camera two tiles away: the tile unloads, its entities go
        let entities = t.entities.len();
        let retired = Piece {
            tile: (31, 14),
            unit: 0,
            source: t.source_key,
        };
        let mut cams = app
            .world_mut()
            .query_filtered::<&mut Transform, With<FlyCamera>>();
        let mut cam = cams.single_mut(app.world_mut()).unwrap();
        cam.translation.x += 3.0 * TILE_SIZE;
        for _ in 0..4 {
            app.update();
        }
        let s = app.world().resource::<Streamer>();
        assert!(!s.loaded.contains_key(&(31, 14)), "tile still loaded");
        assert!(entities > 150);
        let mut q = app.world_mut().query::<&Name>();
        let terrain_left = q
            .iter(app.world())
            .filter(|n| n.as_str().starts_with("terrain 31,14"))
            .count();
        assert_eq!(terrain_left, 0);
        let mut cams = app
            .world_mut()
            .query_filtered::<&mut Transform, With<FlyCamera>>();
        cams.single_mut(app.world_mut()).unwrap().translation.x -= 3.0 * TILE_SIZE;
        let started = std::time::Instant::now();
        loop {
            app.update();
            if app
                .world()
                .resource::<Streamer>()
                .loaded
                .contains_key(&(31, 14))
            {
                break;
            }
            assert!(started.elapsed().as_secs() < 60);
        }
        let mut st = app.world_mut().resource_mut::<Streamer>();
        assert_ne!(st.loaded[&(31, 14)].source_key, retired.source);
        assert!(
            st.tile_for_piece_mut(retired).is_none(),
            "retired terrain hit cannot address a new unit at the same index"
        );
    }
}
