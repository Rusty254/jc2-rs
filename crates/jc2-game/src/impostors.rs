//! Tree impostors: the forests out to the horizon.
//!
//! The tiles around the camera grow their trees as models (`scenery.rs`), whose foliage
//! fades out over 55..110 m (native, `veg_fade.rs`). Beyond that, and over every tile
//! within `--impostor-radius` tiles that is not streamed in at all, each tree is two
//! cards cut out of the game's atlases (`jc2_formats::impostors`): a crown from
//! `TopAtlas_DIF.dds` and a trunk from `TrunkAtlas_DIF.dds`, centred where
//! `treeimpostors.bin` says and as big as it says.
//!
//! * The trees come from the same generator as the near ones (`generate_tile_trees`:
//!   only the trees, which the bushes never move), so an impostor stands where the
//!   model it replaces stands.
//! * A tile is built on a worker thread (read, parse, grow, mesh: tens of ms) and
//!   arrives as one mesh pair per 128 m patch (crowns, trunks), so a patch is two draw
//!   calls. The native per-card fade (JC2-140: crown in from ~55 m and clipped past ~799 m,
//!   trunk in from ~53 m and clipped past ~600 m Chebyshev at FOV factor 1, both stretching
//!   with the camera FOV) runs per card in the vertex shader from
//!   [`crate::veg_fade::VegetationFade`].
//! * `impostor.wgsl` builds the cards: the trunk turns to face the camera, the crown
//!   tips over onto the ground as the camera climbs above the tree.
//!
//! `--no-impostors` leaves them out (and keeps every plant model in view out to the end
//! of the loaded tiles), `--impostor-radius N` is how many tiles around the camera get
//! them (default 5, 2.7 km; the native clip is 800 / f m, f = FOV factor).

use crate::Game;
use crate::vegdata::VegData;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::camera::primitives::{Aabb, MeshAabb};
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};
use jc2_formats::dds::DdsImage;
use jc2_formats::impostors::{ImpostorQuad, TreeImpostors};
use jc2_formats::terrain::{TILE_SIZE, TerrainTile, WORLD_MIN, tile_file_name, tile_origin};
use jc2_formats::vegetation::{Fbm, TileGround, Vegetation};
use std::collections::HashMap;

/// Impostor patch edge (m): a tile is 4 × 4 of them.
pub const PATCH: f32 = 128.0;
/// Tiles built at once.
const IN_FLIGHT: usize = 2;

#[derive(Resource, Clone, Debug)]
pub struct ImpostorConfig {
    /// Tiles (Chebyshev) around the camera's tile that get impostors.
    pub radius: u32,
}

impl Default for ImpostorConfig {
    fn default() -> Self {
        Self { radius: 5 }
    }
}

/// The impostor material: StandardMaterial lighting and a vertex shader that builds
/// the cards.
pub type ImpostorMaterial = ExtendedMaterial<StandardMaterial, ImpostorExt>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ImpostorExt {
    #[uniform(100)]
    pub params: ImpostorParams,
}

#[derive(ShaderType, Clone, Debug, Default)]
pub struct ImpostorParams {
    /// x: 0 for trunks, 1 for crowns
    pub mode: Vec4,
    /// The native `cb1[1]` of this card kind (`VegetationFade::top` / `trunk`); zero = no fade.
    pub fade: Vec4,
}

impl MaterialExtension for ImpostorExt {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn vertex_shader() -> ShaderRef {
        shader()
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("impostor.wgsl")).with_source("embedded"),
    )
}

#[derive(Resource)]
struct ImpostorShader(#[allow(dead_code)] Handle<Shader>);

/// What a worker hands back for a tile: per patch, the meshes of its crowns and trunks.
pub struct PatchMeshes {
    /// Patch origin (engine/Bevy axes, the corner with the smallest x and z).
    pub origin: Vec2,
    pub crowns: Option<Mesh>,
    pub trunks: Option<Mesh>,
    pub trees: usize,
}

type TileTask = Task<Vec<PatchMeshes>>;

enum TileState {
    Building(TileTask),
    Built(Vec<Entity>),
}

#[derive(Resource)]
pub struct Impostors {
    tiles: HashMap<(u32, u32), TileState>,
    crown_material: Handle<ImpostorMaterial>,
    trunk_material: Handle<ImpostorMaterial>,
    pub trees: usize,
    pub built_tiles: usize,
}

#[derive(Component)]
pub struct ImpostorPatch;

pub struct ImpostorPlugin;

impl Plugin for ImpostorPlugin {
    fn build(&self, app: &mut App) {
        crate::vegdata::ensure(app);
        app.init_resource::<crate::veg_fade::VegetationFade>();
        if !app.world().contains_resource::<ImpostorConfig>() {
            app.insert_resource(ImpostorConfig::default());
        }
        bevy::asset::embedded_asset!(app, "impostor.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "impostor.wgsl");
        app.insert_resource(ImpostorShader(shader))
            .add_plugins(MaterialPlugin::<ImpostorMaterial>::default())
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    stream_impostors,
                    push_fade.after(crate::veg_fade::VegetationFadeSet),
                ),
            );
    }
}

/// Keep the two card materials on the current native fade constants (they change only with
/// the camera FOV): crowns take the Top vector, trunks the Trunk vector.
fn push_fade(
    fade: Res<crate::veg_fade::VegetationFade>,
    state: Option<Res<Impostors>>,
    mut materials: ResMut<Assets<ImpostorMaterial>>,
) {
    let Some(st) = state else { return };
    for (handle, want) in [
        (&st.crown_material, fade.top),
        (&st.trunk_material, fade.trunk),
    ] {
        if materials
            .get(handle)
            .is_some_and(|m| m.extension.params.fade != want)
            && let Some(mut m) = materials.get_mut(handle)
        {
            m.extension.params.fade = want;
        }
    }
}

/// An atlas DDS out of the archives as an RGBA8 texture with its mip chain (decoded on
/// the CPU: the atlases are 2048 × 1024 at most).
pub(crate) fn atlas_image(game: &Game, name: &str, srgb: bool) -> Option<Image> {
    let bytes = game
        .fs
        .read_inflated(&name.to_ascii_lowercase())
        .map_err(|e| warn!("{name}: {e}"))
        .ok()?;
    let dds = DdsImage::parse(&bytes)
        .map_err(|e| warn!("{name}: {e}"))
        .ok()?;
    let mips = dds.mips.max(1);
    let mut data = Vec::new();
    let mut levels = 0;
    for level in 0..mips {
        let Some(px) = dds.decode_rgba(level) else {
            break;
        };
        data.extend_from_slice(&px);
        levels += 1;
        let (w, h) = dds.level_size(level);
        if w <= 1 && h <= 1 {
            break;
        }
    }
    if levels == 0 {
        warn!("{name}: cannot decode {:?}", dds.format);
        return None;
    }
    let mut image = Image::new(
        Extent3d {
            width: dds.width,
            height: dds.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        if srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..default()
    });
    Some(image)
}

fn setup(
    mut commands: Commands,
    game: Res<Game>,
    data: Res<VegData>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ImpostorMaterial>>,
) {
    let Some(imp) = &data.impostors else {
        warn!("treeimpostors.bin not readable: no tree impostors");
        return;
    };
    let mut make = |atlas: &str, mode: f32| {
        let texture = atlas_image(&game, atlas, true).map(|i| images.add(i));
        materials.add(ImpostorMaterial {
            base: StandardMaterial {
                base_color: Color::srgb(0.9 + 0.3 * mode, 0.9 + 0.3 * mode, 0.9 + 0.3 * mode),
                base_color_texture: texture,
                perceptual_roughness: 1.0,
                metallic: 0.0,
                reflectance: 0.05,
                // the atlases' alpha is 0.5..1 noise inside the shapes
                alpha_mode: AlphaMode::Mask(0.4),
                cull_mode: None,
                double_sided: false,
                ..default()
            },
            extension: ImpostorExt {
                params: ImpostorParams {
                    mode: Vec4::new(mode, 0.0, 0.0, 0.0),
                    fade: Vec4::ZERO,
                },
            },
        })
    };
    let crown = make(&imp.top_atlas, 1.0);
    let trunk = make(&imp.trunk_atlas, 0.0);
    commands.insert_resource(Impostors {
        tiles: HashMap::new(),
        crown_material: crown,
        trunk_material: trunk,
        trees: 0,
        built_tiles: 0,
    });
}

/// Append one card to a mesh's arrays: four vertices that share the card's centre
/// (relative to the patch), the tree's yaw and each carry their own corner.
struct Cards {
    centre: Vec<[f32; 3]>,
    /// (cos yaw, sin yaw, 1 for a card that lies flat)
    yaw: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    corner: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Cards {
    fn new() -> Self {
        Self {
            centre: Vec::new(),
            yaw: Vec::new(),
            uv: Vec::new(),
            corner: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn push(&mut self, centre: [f32; 3], yaw: f32, q: &ImpostorQuad) {
        let base = self.centre.len() as u32;
        let (s, c) = yaw.sin_cos();
        let [u0, v0, u1, v1] = q.uv;
        let (hw, hh) = (q.size[0] * 0.5, q.size[1] * 0.5);
        let c3 = [centre[0], centre[1] + q.offset_y, centre[2]];
        for (cx, cy, u, v) in [
            (-hw, -hh, u0, v1),
            (hw, -hh, u1, v1),
            (hw, hh, u1, v0),
            (-hw, hh, u0, v0),
        ] {
            self.centre.push(c3);
            self.yaw.push([c, s, if q.flat { 1.0 } else { 0.0 }]);
            self.uv.push([u, v]);
            self.corner.push([cx, cy]);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn into_mesh(self) -> Option<Mesh> {
        if self.centre.is_empty() {
            return None;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.centre);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.yaw);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.corner);
        mesh.insert_indices(Indices::U32(self.indices));
        Some(mesh)
    }
}

/// The impostor meshes of one tile, by 128 m patch. Pure: tests call it directly.
pub fn build_tile(
    veg: &Vegetation,
    imp: &TreeImpostors,
    fbm: &Fbm,
    terrain: &TerrainTile,
    tx: u32,
    tz: u32,
) -> Vec<PatchMeshes> {
    let ground = TileGround::new(terrain, tx, tz);
    let trees = veg.generate_tile_trees(tx, tz, &ground, fbm);
    let (x0, z0) = tile_origin(tx, tz);
    let n = (TILE_SIZE / PATCH) as usize;
    let mut crowns: Vec<Cards> = (0..n * n).map(|_| Cards::new()).collect();
    let mut trunks: Vec<Cards> = (0..n * n).map(|_| Cards::new()).collect();
    let mut counts = vec![0usize; n * n];
    for t in &trees {
        let Some(ty) = veg.type_of(t) else { continue };
        let Some(i) = imp.get(ty.billboard) else {
            continue;
        };
        let pi = (((t.position[0] - x0) / PATCH) as usize).min(n - 1);
        let pj = (((t.position[2] - z0) / PATCH) as usize).min(n - 1);
        let k = pj * n + pi;
        let local = [
            t.position[0] - (x0 + pi as f32 * PATCH),
            t.position[1],
            t.position[2] - (z0 + pj as f32 * PATCH),
        ];
        if let Some(q) = &i.top {
            crowns[k].push(local, t.yaw, q);
        }
        if let Some(q) = &i.trunk {
            trunks[k].push(local, t.yaw, q);
        }
        counts[k] += 1;
    }
    let mut out = Vec::new();
    for (k, (c, tr)) in crowns.into_iter().zip(trunks).enumerate() {
        if counts[k] == 0 {
            continue;
        }
        out.push(PatchMeshes {
            origin: Vec2::new(x0 + (k % n) as f32 * PATCH, z0 + (k / n) as f32 * PATCH),
            crowns: c.into_mesh(),
            trunks: tr.into_mesh(),
            trees: counts[k],
        });
    }
    out
}

/// Build the tiles around the camera (a couple at a time, on worker threads) and drop
/// the ones it has left.
#[allow(clippy::too_many_arguments)]
fn stream_impostors(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    game: Res<Game>,
    data: Res<VegData>,
    cfg: Res<ImpostorConfig>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    state: Option<ResMut<Impostors>>,
) {
    let Some(mut st) = state else { return };
    let (Some(veg), Some(imp)) = (data.veg.clone(), data.impostors.clone()) else {
        return;
    };
    let Ok(cam) = cams.single() else { return };
    let eye = cam.translation();
    let ctx = ((eye.x - WORLD_MIN) / TILE_SIZE).floor() as i64;
    let ctz = ((eye.z - WORLD_MIN) / TILE_SIZE).floor() as i64;
    let r = cfg.radius as i64;
    let dist = |k: (u32, u32)| (k.0 as i64 - ctx).abs().max((k.1 as i64 - ctz).abs());
    // drop what the camera has left (one tile of hysteresis)
    let gone: Vec<(u32, u32)> = st
        .tiles
        .keys()
        .copied()
        .filter(|k| dist(*k) > r + 1)
        .collect();
    for k in gone {
        if let Some(TileState::Built(es)) = st.tiles.remove(&k) {
            for e in es {
                commands.entity(e).despawn();
            }
        }
    }
    // collect finished tiles
    let (crown_mat, trunk_mat) = (st.crown_material.clone(), st.trunk_material.clone());
    let mut finished: Vec<((u32, u32), Vec<PatchMeshes>)> = Vec::new();
    for (k, s) in st.tiles.iter_mut() {
        if let TileState::Building(task) = s
            && let Some(patches) = block_on(future::poll_once(task))
        {
            finished.push((*k, patches));
        }
    }
    for (k, patches) in finished {
        let mut entities = Vec::new();
        let mut trees = 0;
        for p in patches {
            trees += p.trees;
            let origin = Vec3::new(p.origin.x, 0.0, p.origin.y);
            for (mesh, mat) in [(p.crowns, &crown_mat), (p.trunks, &trunk_mat)] {
                let Some(mesh) = mesh else { continue };
                // the cards reach beyond their centres: pad the box the culling uses
                let aabb = mesh.compute_aabb().map(|a| {
                    Aabb::from_min_max(
                        Vec3::from(a.center - a.half_extents) - Vec3::splat(22.0),
                        Vec3::from(a.center + a.half_extents) + Vec3::splat(22.0),
                    )
                });
                let mut e = commands.spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_translation(origin),
                    ImpostorPatch,
                    Name::new(format!("tree impostors {},{}", p.origin.x, p.origin.y)),
                ));
                if let Some(a) = aabb {
                    e.insert(a);
                }
                entities.push(e.id());
            }
        }
        info!(
            "impostors {},{}: {} entities, {} trees",
            k.0,
            k.1,
            entities.len(),
            trees
        );
        st.trees += trees;
        st.built_tiles += 1;
        st.tiles.insert(k, TileState::Built(entities));
    }
    // start the nearest missing tiles
    let building = st
        .tiles
        .values()
        .filter(|s| matches!(s, TileState::Building(_)))
        .count();
    if building >= IN_FLIGHT {
        return;
    }
    let mut wanted: Vec<(u32, u32)> = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            let (x, z) = (ctx + dx, ctz + dz);
            if !(0..64).contains(&x) || !(0..64).contains(&z) {
                continue;
            }
            let k = (x as u32, z as u32);
            if !st.tiles.contains_key(&k) {
                wanted.push(k);
            }
        }
    }
    wanted.sort_by_key(|k| dist(*k));
    let pool = AsyncComputeTaskPool::get();
    for k in wanted.into_iter().take(IN_FLIGHT - building) {
        let (fs, veg, imp, fbm) = (game.fs.clone(), veg.clone(), imp.clone(), data.fbm.clone());
        let task = pool.spawn(async move {
            let Some(terrain) = fs
                .read_inflated(&tile_file_name(k.0, k.1))
                .ok()
                .and_then(|d| TerrainTile::parse(&d).ok())
                .filter(|t| !t.is_empty())
            else {
                return Vec::new();
            };
            build_tile(&veg, &imp, &fbm, &terrain, k.0, k.1)
        });
        st.tiles.insert(k, TileState::Building(task));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;

    /// Tile 40,40 (jungle): every impostor tree gets two cards in the patch it stands
    /// in; the cards' centres sit on the ground plus the table's offsets.
    #[test]
    fn a_jungle_tile_gets_crowns_and_trunks() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = GameFs::open(dir) else { return };
        let (Ok(veg), Ok(imp)) = (Vegetation::load(&fs), TreeImpostors::load(&fs)) else {
            return;
        };
        let Ok(bytes) = fs.read_inflated(&tile_file_name(40, 40)) else {
            return;
        };
        let terrain = TerrainTile::parse(&bytes).unwrap();
        let fbm = Fbm::vegetation();
        let started = std::time::Instant::now();
        let patches = build_tile(&veg, &imp, &fbm, &terrain, 40, 40);
        eprintln!("tile 40,40 impostors built in {:.1?}", started.elapsed());
        let trees: usize = patches.iter().map(|p| p.trees).sum();
        assert!(trees > 300, "{trees} trees");
        let (x0, z0) = tile_origin(40, 40);
        for p in &patches {
            assert!(p.origin.x >= x0 && p.origin.x < x0 + TILE_SIZE);
            assert!(p.origin.y >= z0 && p.origin.y < z0 + TILE_SIZE);
            let crowns = p.crowns.as_ref().expect("jungle trees have crowns");
            // two cards' worth of vertices per tree at most: a crown and a trunk
            assert!(crowns.count_vertices() <= p.trees * 4);
            let pos = crowns
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
                .unwrap();
            for c in pos {
                assert!(c[0] >= -1.0 && c[0] <= PATCH + 1.0 && c[2] >= -1.0 && c[2] <= PATCH + 1.0);
                // 200 m is the sea; a crown hangs metres above the ground
                assert!(c[1] > 200.0, "{c:?}");
            }
        }
        // only tile-local patches: 16 at most
        assert!(patches.len() <= 16);
    }
}
