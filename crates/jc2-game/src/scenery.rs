//! Scenery: what `world::load_tile` does not place.
//!
//! * **Vegetation** — trees, bushes and rocks. The original stores none of them; it
//!   grows them from the terrain material map with a fixed-seed generator
//!   (`jc2_formats::vegetation`, see `research/formats/world.md`). Every loaded tile
//!   is run through the same generator here and the plants are queued on the tile's
//!   `pending` list like the placements, so they arrive through the asset server and
//!   are despawned with the tile. Types with a collision flag get their own
//!   `<model>_COL.pfx` Havok shape (trees you cannot drive through, rocks you can climb).
//! * **Static geometry** — the `XXxZZ.nl.cgd` list of a location container: the
//!   fences, crates, poles and building bodies that have no object of their own in the
//!   placement file (`jc2_formats::cgd`). Their collision already comes with the tile's
//!   `.pfi` list.
//!
//! Both run as one system, a frame after the streamer loads a tile, one tile per frame.

use crate::Game;
use crate::coords::mat_to_bevy;
use crate::physics::colliders_from_body;
use crate::plant_lod::PlantLodSet;
use crate::world::{self, Container, LoadedTile, PendingModel, Streamer, WorldConfig};
use avian3d::prelude::RigidBody;
use bevy::prelude::*;
use jc2_formats::cgd::CompiledGeometry;
use jc2_formats::collision::parse_pfx;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::placement::bare_name;
use jc2_formats::terrain::{TerrainTile, tile_container_name, tile_file_name};
use jc2_formats::vegetation::{Fbm, Group, TileGround, Vegetation};
use jc2_formats::veglod::SlotPlan;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

#[derive(Resource, Clone, Debug)]
pub struct SceneryConfig {
    /// Grow the vegetation.
    pub vegetation: bool,
    /// 0 = the original's per-asset LOD selection from the camera distance
    /// (`plant_lod.rs`); N >= 1 forces that single `.lod` level for every plant (1 = full
    /// detail; 2 is a third of the triangles).
    pub vegetation_lod: usize,
    /// Give the trees and rocks that have one their collision shape (needs physics).
    pub vegetation_physics: bool,
    /// Share of the bushes and rocks to keep, 0..=1 (the trees always stay): a jungle
    /// tile grows 4–5 000 of them, which is a lot of entities for a 3 × 3 of tiles.
    pub bush_density: f32,
    /// Place the `.cgd` static geometry.
    pub static_geometry: bool,
    /// Hide the plant models beyond [`TREE_RANGE`] / [`BUSH_RANGE`] (the distant trees
    /// are `impostors.rs`'s billboards then).
    pub far_lod: bool,
}

/// Where a tree with an impostor stops being drawn as a model: fully drawn up to the
/// first number, gone at the second (`VisibilityRange` fades it in between).
pub const TREE_RANGE: (f32, f32) = (520.0, 580.0);
/// The same for bushes, rocks and trees without an impostor.
pub const BUSH_RANGE: (f32, f32) = (380.0, 440.0);

impl Default for SceneryConfig {
    fn default() -> Self {
        Self {
            vegetation: true,
            vegetation_lod: 0,
            vegetation_physics: true,
            bush_density: 1.0,
            static_geometry: true,
            far_lod: true,
        }
    }
}

/// Running totals, for the HUD and the tests.
#[derive(Default, Debug)]
pub struct SceneryStats {
    pub tiles: usize,
    pub trees: usize,
    pub bushes: usize,
    pub statics: usize,
    pub vegetation_colliders: usize,
    /// Plant types whose model could not be found, with how often they came up.
    pub unresolved: BTreeMap<String, usize>,
}

/// One plant type's resolved assets.
#[derive(Clone)]
struct PlantModel {
    /// `jc2://container/model.rbm`
    path: String,
    /// The zone container the shapes live in.
    container: Arc<Container>,
    /// Bare `<model>_COL.pfx` name when the container has it.
    collision: Option<String>,
    align: bool,
    name: String,
    /// Native slot plan and the `jc2://` path of each of its models (`vegetation_lod == 0`
    /// and more than one distinct model); `path` is then the plan's far model.
    lod: Option<(SlotPlan, Vec<String>)>,
}

#[derive(Resource)]
pub struct Scenery {
    veg: Option<Arc<Vegetation>>,
    fbm: Arc<Fbm>,
    impostors: Option<Arc<jc2_formats::impostors::TreeImpostors>>,
    /// Tiles already dressed.
    done: HashSet<(u32, u32)>,
    /// Per (zone, type index): the model, or `None` when it could not be resolved.
    plants: HashMap<(u8, u16), Option<PlantModel>>,
    pub stats: SceneryStats,
}

pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<SceneryConfig>() {
            app.insert_resource(SceneryConfig::default());
        }
        crate::vegdata::ensure(app);
        app.add_plugins(crate::plant_lod::PlantLodPlugin);
        app.add_systems(Startup, setup).add_systems(
            Update,
            dress_tiles
                .after(world::stream_tiles)
                .before(world::spawn_models),
        );
    }
}

fn setup(mut commands: Commands, data: Res<crate::vegdata::VegData>, cfg: Res<SceneryConfig>) {
    let veg = if cfg.vegetation {
        data.veg.clone()
    } else {
        None
    };
    if let Some(v) = &veg {
        let zones = v.zones.iter().filter(|z| z.is_some()).count();
        let sets: usize = v.zones.iter().flatten().map(|z| z.sets.len() / 3).sum();
        info!("vegetation: {zones} climate zones, {sets} sets");
    }
    commands.insert_resource(Scenery {
        veg,
        fbm: data.fbm.clone(),
        impostors: data.impostors.clone(),
        done: HashSet::new(),
        plants: HashMap::new(),
        stats: SceneryStats::default(),
    });
}

/// Dress the loaded tile nearest the focus that has not been dressed yet.
pub(crate) fn dress_tiles(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    cfg: Res<WorldConfig>,
    scfg: Res<SceneryConfig>,
    st: Option<ResMut<Streamer>>,
    sc: Option<ResMut<Scenery>>,
) {
    let (Some(mut st), Some(mut sc)) = (st, sc) else {
        return;
    };
    if !cfg.models {
        return;
    }
    // forget the tiles the streamer dropped
    let loaded: HashSet<(u32, u32)> = st.loaded.keys().copied().collect();
    sc.done.retain(|k| loaded.contains(k));
    let (fx, fz) = st.focus;
    let Some(key) = loaded
        .iter()
        .copied()
        .filter(|k| !sc.done.contains(k))
        .min_by_key(|k| {
            (k.0 as i64 - fx as i64)
                .abs()
                .max((k.1 as i64 - fz as i64).abs())
        })
    else {
        return;
    };
    sc.done.insert(key);
    // take the tile out of the streamer while we work on it, so the streamer's caches
    // and the tile can be borrowed together
    let Some(mut tile) = st.loaded.remove(&key) else {
        return;
    };
    if scfg.vegetation {
        grow(
            &mut st,
            &mut sc,
            &mut tile,
            key,
            &mut commands,
            &assets,
            &game,
            &cfg,
            &scfg,
        );
    }
    if scfg.static_geometry {
        place_statics(&mut st, &mut sc, &mut tile, key, &assets, &game, &cfg);
    }
    sc.stats.tiles += 1;
    st.loaded.insert(key, tile);
}

/// The vegetation of one tile.
#[allow(clippy::too_many_arguments)]
fn grow(
    st: &mut Streamer,
    sc: &mut Scenery,
    tile: &mut LoadedTile,
    (x, z): (u32, u32),
    commands: &mut Commands,
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
    scfg: &SceneryConfig,
) {
    let Some(veg) = sc.veg.clone() else { return };
    let name = tile_file_name(x, z);
    let terrain = match game
        .fs
        .read_inflated(&name)
        .and_then(|d| TerrainTile::parse(&d))
    {
        Ok(t) => t,
        Err(e) => {
            warn!("{name}: {e}");
            return;
        }
    };
    if terrain.is_empty() {
        return;
    }
    let ground = TileGround::new(&terrain, x, z);
    let instances = veg.generate_tile(x, z, &ground, &sc.fbm);
    let mut trees = 0;
    let mut bushes = 0;
    let mut colliders = 0;
    // per plant type: the held model handles of its LOD slots, shared by the tile's instances
    let mut lod_sets: HashMap<(u8, u16), Arc<PlantLodSet>> = HashMap::new();
    for inst in &instances {
        let Some(ty) = veg.type_of(inst) else {
            continue;
        };
        if inst.group != Group::Tree && scfg.bush_density < 1.0 {
            // a deterministic subset: hash the position so the same bushes stay
            let h = jc2_formats::hash::hashlittle(
                &inst.position[0].to_bits().to_le_bytes(),
                inst.position[2].to_bits(),
            );
            if (h % 1000) as f32 >= scfg.bush_density * 1000.0 {
                continue;
            }
        }
        let key = (inst.zone, inst.type_index);
        if !sc.plants.contains_key(&key) {
            // the type's own zone container first, then the other zones': the zones
            // share one type list and a zone container only ships the models its own
            // sets use (the city zone's sets are jungle sets, whose trees live in
            // vegetation_0.blz)
            let mut containers: Vec<String> = veg
                .zone(inst.zone)
                .map(|z| z.container())
                .into_iter()
                .collect();
            for z in veg.zones.iter().flatten() {
                if z.index != inst.zone as usize {
                    containers.push(z.container());
                }
            }
            let model = containers
                .iter()
                .find_map(|c| resolve_plant(st, game, c, ty, scfg.vegetation_lod));
            if model.is_none() {
                *sc.stats.unresolved.entry(ty.model.clone()).or_default() += 1;
                *st.unresolved.entry(ty.model.clone()).or_default() += 1;
            }
            sc.plants.insert(key, model);
        }
        let Some(plant) = sc.plants.get(&key).cloned().flatten() else {
            continue;
        };
        let handle = st.model_handle(assets, &plant.path);
        tile.models.push(plant.path.clone());
        let transform = mat_to_bevy(&inst.world(plant.align));
        if cfg.physics
            && scfg.vegetation_physics
            && ty.has_collision
            && let Some(pfx) = &plant.collision
            && place_collision(
                st,
                tile,
                commands,
                game,
                &plant.container,
                (x, z),
                pfx,
                transform,
            )
        {
            colliders += 1;
        }
        // beyond the model range a tree is its impostor (`impostors.rs`), a bush or rock
        // just goes
        let has_impostor = inst.group == Group::Tree
            && sc
                .impostors
                .as_ref()
                .is_some_and(|i| i.get(ty.billboard).is_some());
        let vis_range = scfg
            .far_lod
            .then_some(if has_impostor { TREE_RANGE } else { BUSH_RANGE });
        let lod = plant.lod.as_ref().map(|(plan, paths)| {
            lod_sets
                .entry(key)
                .or_insert_with(|| {
                    // one reference per model for the tile's lifetime (`unload_tile`)
                    let handles = paths
                        .iter()
                        .map(|p| {
                            tile.models.push(p.clone());
                            st.model_handle(assets, p)
                        })
                        .collect();
                    Arc::new(PlantLodSet {
                        plan: plan.clone(),
                        handles,
                    })
                })
                .clone()
        });
        tile.pending.push(PendingModel {
            handle,
            transform,
            name: plant.name.clone(),
            mesh_colliders: false,
            piece: None,
            vis_range,
            mission_owner: None,
            lod,
        });
        tile.objects_total += 1;
        match inst.group {
            Group::Tree => trees += 1,
            _ => bushes += 1,
        }
    }
    sc.stats.trees += trees;
    sc.stats.bushes += bushes;
    sc.stats.vegetation_colliders += colliders;
    info!("tile {x},{z}: {trees} trees, {bushes} bushes and rocks, {colliders} with collision");
}

/// Find a plant type's model (`.lod` in the zone container, then the LOD level's
/// `.rbm` next to it) and its collision shape.
fn resolve_plant(
    st: &mut Streamer,
    game: &Game,
    container: &str,
    ty: &jc2_formats::vegetation::VegetationType,
    lod: usize,
) -> Option<PlantModel> {
    let c = st.index.container(game, container)?;
    let lod_text = c.member(&ty.model)?;
    let d = LodDescriptor::parse(&String::from_utf8_lossy(lod_text));
    // native: every slot of the descriptor, only the ones the container has
    let mut lod_plan = None;
    let rbm = if lod == 0 {
        let slots = d.slots().map(|s| s.filter(|n| c.member(n).is_some()));
        let plan = SlotPlan::new(d.lod_scale(), slots);
        let far = plan.models.get(plan.far_model()?)?.clone();
        if plan.switches() {
            let paths = plan
                .models
                .iter()
                .map(|m| format!("jc2://{}/{}", c.name, m))
                .collect();
            lod_plan = Some((plan, paths));
        }
        far
    } else {
        let rbm = d.model_for_level(lod)?.to_string();
        c.member(&rbm)?;
        rbm
    };
    let stem = ty
        .model
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&ty.model);
    let pfx = format!("{stem}_COL.pfx");
    let collision = c.member(&pfx).is_some().then_some(pfx);
    Some(PlantModel {
        path: format!("jc2://{}/{}", c.name, rbm),
        container: c,
        collision,
        align: ty.align_to_ground,
        name: format!("{} ({rbm})", ty.name),
        lod: lod_plan,
    })
}

/// Spawn the static colliders of one Havok `.pfx` at a world transform, cached per shape
/// name in the streamer. Returns whether the shape was found. (The same thing
/// `world` does for the `.pfi` instances.)
#[allow(clippy::too_many_arguments)]
fn place_collision(
    st: &mut Streamer,
    tile: &mut LoadedTile,
    commands: &mut Commands,
    game: &Game,
    local: &Arc<Container>,
    xz: (u32, u32),
    pfx: &str,
    tf: Transform,
) -> bool {
    let key = bare_name(pfx).to_ascii_lowercase();
    let parts = match st.colliders.get(&key) {
        Some(p) => p.clone(),
        None => {
            let bytes = st.index.shape_bytes(game, local, xz, &key);
            let p = bytes
                .and_then(|b| parse_pfx(&b).ok())
                .and_then(|bodies| bodies.first().map(colliders_from_body))
                .filter(|p| !p.is_empty());
            st.colliders.insert(key.clone(), p.clone());
            p
        }
    };
    let Some(parts) = parts else {
        *st.unresolved.entry(key).or_default() += 1;
        return false;
    };
    for (p, r, collider) in parts {
        let part_tf = Transform {
            translation: tf.transform_point(p),
            rotation: tf.rotation * r,
            scale: Vec3::ONE,
        };
        let e = commands
            .spawn((
                RigidBody::Static,
                collider,
                part_tf,
                Name::new(format!("collision {}", bare_name(pfx))),
            ))
            .id();
        tile.entities.push(e);
    }
    true
}

/// The `.cgd` static geometry of one tile's location container.
fn place_statics(
    st: &mut Streamer,
    sc: &mut Scenery,
    tile: &mut LoadedTile,
    (x, z): (u32, u32),
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
) {
    match place_container_statics(
        st,
        tile,
        (x, z),
        &tile_container_name(x, z),
        assets,
        game,
        cfg,
    ) {
        Ok(placed) => sc.stats.statics += placed,
        Err(e) => warn!("tile {x},{z} static geometry: {e}"),
    }
}

/// Static geometry from the actual owner container, including mission/city extras.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_container_statics(
    st: &mut Streamer,
    tile: &mut LoadedTile,
    (x, z): (u32, u32),
    container: &str,
    assets: &AssetServer,
    game: &Game,
    cfg: &WorldConfig,
) -> jc2_formats::Result<usize> {
    let local = st
        .index
        .container(game, container)
        .ok_or_else(|| jc2_formats::Error::format(format!("{container}: unreadable archive")))?;
    let mut geometries = Vec::new();
    for cgd in local
        .sarc
        .entries
        .iter()
        .filter(|e| e.name.to_ascii_lowercase().ends_with(".cgd"))
    {
        let bytes = local
            .member(&cgd.name)
            .ok_or_else(|| jc2_formats::Error::format("unreadable CGD"))?;
        geometries.push(CompiledGeometry::parse(
            &jc2_formats::compress::maybe_inflate(bytes)?,
        )?);
    }
    // The far companion's `.fl.cgd` (JC2-111): the location's large structures (the
    // start base's key036 towers, the gb223 kit of 31,14...). Its models' descriptors and
    // full-detail meshes are in the near container (the far one ships the low LODs only),
    // so they resolve as the near ones do (near, then the shared sets), then far.
    let far = world::far_companion(container).and_then(|n| st.index.container(game, &n));
    let near_count = geometries.len();
    if let Some(far) = &far {
        for cgd in far
            .sarc
            .entries
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".cgd"))
        {
            let bytes = far
                .member(&cgd.name)
                .ok_or_else(|| jc2_formats::Error::format("unreadable far CGD"))?;
            geometries.push(CompiledGeometry::parse(
                &jc2_formats::compress::maybe_inflate(bytes)?,
            )?);
        }
    }
    let mut placed = 0;
    for (g, geometry) in geometries.iter().enumerate() {
        for inst in &geometry.instances {
            let model = geometry.model_of(inst);
            let resolved = st
                .index
                .resolve(game, &local, (x, z), model, cfg.lod)
                .or_else(|| {
                    let far = far.as_ref().filter(|_| g >= near_count)?;
                    st.index.resolve(game, far, (x, z), model, cfg.lod)
                });
            let Some((home, rbm)) = resolved else {
                *st.unresolved.entry(model.to_string()).or_default() += 1;
                tile.unresolved += 1;
                continue;
            };
            let path = format!("jc2://{home}/{rbm}");
            let handle = st.model_handle(assets, &path);
            tile.models.push(path);
            tile.pending.push(PendingModel {
                handle,
                transform: mat_to_bevy(&inst.world),
                name: format!("static {model} ({rbm})"),
                // the tile's .pfi list carries these shapes already
                mesh_colliders: false,
                piece: None,
                vis_range: None,
                mission_owner: None,
                lod: None,
            });
            tile.objects_total += 1;
            placed += 1;
        }
    }
    Ok(placed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::rbm_loader::{RbmAsset, RbmLoader};
    use bevy::asset::io::AssetSourceBuilder;
    use bevy::asset::{AssetApp, AssetPlugin};
    use jc2_formats::GameFs;

    /// Headless: tile 31,14 with its vegetation and static geometry. Needs
    /// JC2_TEST_DIR with the terrain tiles, `31x14.nlz`, `general.blz`, the climate
    /// files (`num_zones.bin`, `terrain_materials.bin`, `vegetation_set_1.bin`,
    /// `vegetation_types_1.bin`, `climatezonemap.dds`) and `vegetation_1.blz`.
    #[test]
    fn dresses_a_tile_headless() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        if !fs.contains("terrain_win32_31_14.dat")
            || !fs.contains("31x14.nlz")
            || !fs.contains("vegetation_1.blz")
            || !fs.contains("vegetation_set_1.bin")
        {
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
        .init_asset::<crate::terrain_material::TerrainSplatMaterial>()
        .add_systems(Startup, world::setup)
        .add_systems(Update, (world::stream_tiles, world::spawn_models).chain())
        .add_plugins(SceneryPlugin);
        app.finish();
        let started = std::time::Instant::now();
        // (the models load on other threads: wait by the wall clock, not a frame count)
        while started.elapsed().as_secs() < 90 {
            app.update();
            let s = app.world().resource::<Streamer>();
            let sc = app.world().resource::<Scenery>();
            if sc.stats.tiles == 1 && s.stat(|t| t.pending.len()) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let s = app.world().resource::<Streamer>();
        let sc = app.world().resource::<Scenery>();
        assert_eq!(sc.stats.tiles, 1);
        let t = s.loaded.get(&(31, 14)).expect("start tile loaded");
        assert!(t.pending.is_empty(), "{} still pending", t.pending.len());
        // 652 trees and 3100 bushes/rocks on this tile (see jc2-formats' test), 100 + 81
        // static geometry instances
        assert_eq!(sc.stats.trees, 652);
        assert_eq!(sc.stats.bushes, 3100);
        assert_eq!(sc.stats.statics, 100 + 81, "31x14.nl.cgd + 31x14.fl.cgd");
        assert!(sc.stats.unresolved.is_empty(), "{:?}", sc.stats.unresolved);
        assert_eq!(
            sc.stats.vegetation_colliders,
            sc.stats.trees + sc.stats.bushes,
            "every arctic type has a _COL.pfx"
        );
        assert_eq!(t.objects_spawned, t.objects_total);
        assert_eq!(t.objects_failed, 0);
        let (trees, bushes, statics, colliders) = (
            sc.stats.trees,
            sc.stats.bushes,
            sc.stats.statics,
            sc.stats.vegetation_colliders,
        );
        // plants stand on the ground, upright, inside the tile
        let mut plants = 0;
        let mut q = app.world_mut().query::<(&Name, &Transform)>();
        for (name, tf) in q.iter(app.world()) {
            if name.starts_with("Arctic_") {
                plants += 1;
                assert!(
                    tf.translation.x >= -512.0 && tf.translation.x < 12.0,
                    "{name}"
                );
                assert!(
                    tf.translation.y > 700.0 && tf.translation.y < 1200.0,
                    "{name}"
                );
                let up = tf.rotation * Vec3::Y;
                assert!(up.y > 0.6, "{name} leans: {up:?}");
            }
        }
        assert_eq!(plants, 652 + 3100);

        // Native LOD (JC2-086): plants whose descriptor has more than one model carry a
        // slot plan and start on the far model; a camera 5 m from one switches it to the
        // near model without adding an entity, a child set or a collider.
        use crate::plant_lod::PlantLod;
        let (target, target_pos) = {
            let mut q = app.world_mut().query::<(Entity, &Transform, &PlantLod)>();
            let n = q.iter(app.world()).count();
            assert!(n > 0 && n < plants, "{n} of {plants} plants switch");
            let (e, tf, lod) = q.iter(app.world()).next().unwrap();
            assert_eq!(Some(lod.shown), lod.set.plan.far_model());
            (e, tf.translation)
        };
        // (the world setup spawned the camera; move it)
        let mut cams = app
            .world_mut()
            .query_filtered::<&mut Transform, With<Camera3d>>();
        for mut tf in cams.iter_mut(app.world_mut()) {
            tf.translation = target_pos + Vec3::new(0.0, 2.0, 5.0);
        }
        let started = std::time::Instant::now();
        let near = loop {
            app.update();
            let lod = app.world().get::<PlantLod>(target).unwrap();
            let want = lod
                .set
                .plan
                .model_at(29.0)
                .expect("a camera 5 m away selects a model");
            if lod.shown == want {
                break want;
            }
            if started.elapsed().as_secs() >= 60 {
                let shown = lod.shown;
                let state = app
                    .world()
                    .resource::<AssetServer>()
                    .get_load_state(lod.set.handles[want].id());
                let plant = app.world().get::<GlobalTransform>(target).map(|g| g.translation());
                let cams: Vec<Vec3> = app
                    .world_mut()
                    .query_filtered::<&GlobalTransform, With<Camera3d>>()
                    .iter(app.world())
                    .map(|g| g.translation())
                    .collect();
                panic!(
                    "near model never loaded: shown {shown} want {want} state {state:?} plant {plant:?} cameras {cams:?}"
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        app.update();
        let lod = app.world().get::<PlantLod>(target).unwrap();
        assert_ne!(Some(near), lod.set.plan.far_model(), "near differs from far");
        let blocks = app
            .world()
            .resource::<Assets<RbmAsset>>()
            .get(&lod.set.handles[near])
            .unwrap()
            .blocks
            .len();
        let children = app.world().get::<Children>(target).unwrap().len();
        assert_eq!(children, blocks, "the swap replaced the meshes, not added");
        eprintln!(
            "tile 31,14 dressed in {:.1?}: {trees} trees, {bushes} bushes/rocks, {statics} statics, {colliders} colliders",
            started.elapsed(),
        );
    }
}
