//! Grass: the engine's ground cover, drawn around the camera.
//!
//! The data and the generator are `jc2_formats::grass` (`research/formats/world.md`,
//! "Grass"): 1 m cells in 8 m patches, one blade (three crossed quads) per cell where
//! the terrain material's vegetation set has a grass type that the ground's slope
//! accepts, textured from the atlas `global_grass_DIF.dds`.
//!
//! The original rebuilds a 13 × 13 ring of 8 m patches (±52 m) around the camera into
//! one dynamic vertex buffer. Here the world is cut into 32 m blocks (4 × 4 patches);
//! the blocks within `radius` of the camera are generated into one static mesh each
//! (a few hundred microseconds apiece) and dropped again when the camera has moved on,
//! a couple per frame. Every vertex stands on the terrain at its own position the way
//! the engine's do, so the blades follow the ground; the lighting normal is the ground
//! normal, so grass shades like the terrain under it. One draw call per block, no
//! entity per blade; true GPU instancing does not apply because no two blades share a
//! vertex shape. The material is `AlphaToCoverage` with a stochastic distance fade
//! (`grass.wgsl`) and the tips sway in the wind (`sway` of the type).
//!
//! `--no-grass` leaves it out, `--grass-density 0.5` draws half of the engine's four
//! passes, `--grass-radius 60` shortens the range.

use crate::Game;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use jc2_formats::grass::{GRASS_PATCH, GrassBlade, QUADS_PER_BLADE, blade_quad};
use jc2_formats::terrain::{TILE_SIZE, TerrainTile, WORLD_MIN, tile_file_name};
use jc2_formats::vegetation::{Ground, GroundSample, TileGround, Vegetation};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

/// Edge of a block in metres (4 patches).
pub const BLOCK: f32 = 32.0;
/// Time one frame may spend building blocks (seconds).
const BUILD_BUDGET: f32 = 0.004;

#[derive(Resource, Clone, Debug)]
pub struct GrassConfig {
    /// Share of the engine's four passes to draw, 0..=1 (0 draws nothing).
    pub density: f32,
    /// How far from the camera grass is built (m); it fades out over the last 40 %.
    pub radius: f32,
}

impl Default for GrassConfig {
    fn default() -> Self {
        Self {
            density: 1.0,
            radius: 80.0,
        }
    }
}

impl GrassConfig {
    /// Passes (of four) the density asks for.
    pub fn passes(&self) -> u8 {
        (self.density.clamp(0.0, 1.0) * 4.0).ceil() as u8
    }
}

/// Running totals for the HUD and the tests.
#[derive(Default, Debug, Clone, Copy)]
pub struct GrassStats {
    pub blocks: usize,
    pub blades: usize,
    pub built_total: usize,
    pub last_build_ms: f32,
}

/// The grass material: StandardMaterial lighting with a distance fade and wind.
pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassExt>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct GrassExt {
    /// x: fade start, y: fade end (metres from the camera)
    #[uniform(100)]
    pub fade: GrassFade,
    /// The shared per-frame constants (`sun_horizon::SunHorizon`); grass reads only the haze fog
    /// fields (JC2-163). Filled by the system `haze::add_haze_binding` registers.
    #[storage(101, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
    /// `t11`: `ambientocclusionmap.dds`, filled by `sun_horizon::add_ao_binding` (white = AO 1
    /// until then). The grass pixel shader reads it for the ambient, the horizon shadow and the
    /// `(AO / 2 + 1/2)` factor of the Sun (lighting-gaps-next.md G2/G3).
    #[texture(102)]
    #[sampler(103)]
    pub ao_map: Handle<Image>,
}

impl crate::sun_horizon::AoBound for GrassMaterial {
    fn ao_map(&self) -> &Handle<Image> {
        &self.extension.ao_map
    }
    fn set_ao_map(&mut self, map: Handle<Image>) {
        self.extension.ao_map = map;
    }
}

impl crate::haze::HazeBound for GrassMaterial {
    fn haze_buffer(&self) -> &Handle<ShaderBuffer> {
        &self.extension.sun_horizon
    }
    fn set_haze_buffer(&mut self, buffer: Handle<ShaderBuffer>) {
        self.extension.sun_horizon = buffer;
    }
}

#[derive(ShaderType, Clone, Debug, Default)]
pub struct GrassFade {
    pub range: Vec4,
}

impl MaterialExtension for GrassExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("grass.wgsl")).with_source("embedded"),
        )
    }
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("grass.wgsl")).with_source("embedded"),
        )
    }
    // JC2-237: no depth pre-pass and no shadow cascades. The material is `AlphaToCoverage`; with
    // MSAA Bevy's prepass (and shadow) pipeline has no alpha discard for that mode (`MAY_DISCARD`
    // is only set for `Mask`, or A2C with MSAA off), so every grass card wrote its whole quad
    // into the depth buffer and the terrain behind the transparent texels failed the main pass's
    // depth test: the scene clear colour showed through as flat patches. Whether the native grass
    // draws in a depth pre-pass or casts shadows is not traced (non-native choice, Gap).
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

#[derive(Resource)]
struct GrassShader(#[allow(dead_code)] Handle<Shader>);

#[derive(Component)]
pub struct GrassBlock;

/// Terrain tiles kept for the ground queries, and what is built.
#[derive(Resource)]
pub struct Grass {
    veg: Option<Arc<Vegetation>>,
    tiles: HashMap<(u32, u32), Option<Arc<TerrainTile>>>,
    blocks: HashMap<(i32, i32), (Entity, usize)>,
    /// Blocks known to have no grass (so they are not rebuilt every frame).
    empty: HashSet<(i32, i32)>,
    material: Handle<GrassMaterial>,
    pub stats: GrassStats,
}

pub struct GrassPlugin;

impl Plugin for GrassPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<GrassConfig>() {
            app.insert_resource(GrassConfig::default());
        }
        crate::vegdata::ensure(app);
        bevy::asset::embedded_asset!(app, "grass.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "grass.wgsl");
        crate::haze::ensure_shader(app);
        crate::haze::add_haze_binding::<GrassMaterial>(app);
        crate::sun_horizon::ensure_correction_shader(app);
        crate::sun_horizon::add_ao_binding::<GrassMaterial>(app);
        app.insert_resource(GrassShader(shader))
            .add_plugins(MaterialPlugin::<GrassMaterial>::default())
            .add_systems(Startup, setup)
            .add_systems(Update, update_grass);
    }
}

fn setup(
    mut commands: Commands,
    data: Res<crate::vegdata::VegData>,
    cfg: Res<GrassConfig>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<GrassMaterial>>,
) {
    let veg = data.veg.clone();
    // the atlas every zone's grass types name (they all say global_grass_DIF.dds)
    // sampler: CLAMP in U and V, fields (7,7) with the user anisotropy (`FUN_00a821d0`)
    let texture: Handle<Image> = assets
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler =
                crate::native_sampler::SlotState::new(crate::native_sampler::CLAMP, 7, 7, true)
                    .sampler();
        })
        .load("jc2://general.blz/global_grass_dif.dds");
    let material = materials.add(GrassMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.05,
            alpha_mode: AlphaMode::AlphaToCoverage,
            cull_mode: None,
            double_sided: false,
            // the native fog is applied in `grass.wgsl` (`haze_fog.wgsl`), not Bevy's
            fog_enabled: false,
            ..default()
        },
        extension: GrassExt {
            sun_horizon: Handle::default(),
            ao_map: Handle::default(),
            fade: GrassFade {
                range: Vec4::new(cfg.radius * 0.6, cfg.radius, 0.0, 0.0),
            },
        },
    });
    commands.insert_resource(Grass {
        veg,
        tiles: HashMap::new(),
        blocks: HashMap::new(),
        empty: HashSet::new(),
        material,
        stats: GrassStats::default(),
    });
}

/// The terrain tiles' own height fields as one [`Ground`]: the tile under a position
/// answers, so a block that straddles a tile border still lines up.
pub struct TilesGround<'a> {
    pub tiles: &'a HashMap<(u32, u32), Option<Arc<TerrainTile>>>,
}

impl TilesGround<'_> {
    fn tile_of(&self, x: f32, z: f32) -> Option<(&TerrainTile, u32, u32)> {
        let tx = ((x - WORLD_MIN) / TILE_SIZE).floor();
        let tz = ((z - WORLD_MIN) / TILE_SIZE).floor();
        if !(0.0..64.0).contains(&tx) || !(0.0..64.0).contains(&tz) {
            return None;
        }
        let (tx, tz) = (tx as u32, tz as u32);
        let t = self.tiles.get(&(tx, tz))?.as_deref()?;
        Some((t, tx, tz))
    }
}

impl Ground for TilesGround<'_> {
    fn sample(&self, x: f32, z: f32) -> Option<GroundSample> {
        let (t, tx, tz) = self.tile_of(x, z)?;
        TileGround::new(t, tx, tz).sample(x, z)
    }
}

/// The mesh of one block, or `None` when nothing grows there. Vertices are relative to
/// the block's corner `(bx, bz)`.
pub fn build_block(
    veg: &Vegetation,
    ground: &dyn Ground,
    bx: f32,
    bz: f32,
    passes: u8,
) -> Option<(Mesh, usize)> {
    let patches = (BLOCK / GRASS_PATCH) as usize;
    let mut blades: Vec<GrassBlade> = Vec::new();
    for j in 0..patches {
        for i in 0..patches {
            blades.extend(veg.generate_grass_patch(
                bx + i as f32 * GRASS_PATCH,
                bz + j as f32 * GRASS_PATCH,
                passes,
                ground,
            ));
        }
    }
    if blades.is_empty() {
        return None;
    }
    let quads = blades.len() * QUADS_PER_BLADE;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(quads * 4);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(quads * 4);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(quads * 4);
    let mut colours: Vec<[f32; 4]> = Vec::with_capacity(quads * 4);
    let mut wind: Vec<[f32; 2]> = Vec::with_capacity(quads * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(quads * 6);
    let height = |x: f32, z: f32| ground.sample(x, z).map(|g| g.height).unwrap_or(0.0);
    let mut kept = 0;
    for blade in &blades {
        let Some(ty) = veg
            .zone(blade.zone)
            .and_then(|z| z.grass.types.get(blade.type_index as usize))
        else {
            continue;
        };
        kept += 1;
        // a little brightness variation from the blade's phase byte
        let shade = 0.82 + 0.36 * ((blade.phase as f32 - 200.0) / 56.0);
        for k in 0..QUADS_PER_BLADE {
            let base = positions.len() as u32;
            for v in blade_quad(blade, ty, k, &height) {
                positions.push([v.position[0] - bx, v.position[1], v.position[2] - bz]);
                normals.push(blade.normal);
                uvs.push(v.uv);
                colours.push([shade, shade, shade, 1.0]);
                // the tips sway: weight along the blade × the type's factor, phase
                wind.push([v.along * ty.sway, blade.phase as f32]);
            }
            // counter-clockwise from the front: bl, br, tr, tl; the material is
            // two-sided
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    if kept == 0 {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, wind);
    mesh.insert_indices(Indices::U32(indices));
    Some((mesh, kept))
}

fn block_origin(key: (i32, i32)) -> (f32, f32) {
    (
        WORLD_MIN + key.0 as f32 * BLOCK,
        WORLD_MIN + key.1 as f32 * BLOCK,
    )
}

fn load_tile(game: &Game, x: u32, z: u32) -> Option<Arc<TerrainTile>> {
    game.fs
        .read_inflated(&tile_file_name(x, z))
        .ok()
        .and_then(|d| TerrainTile::parse(&d).ok())
        .filter(|t| !t.is_empty())
        .map(Arc::new)
}

/// Keep the blocks near the camera built, drop the ones it has left.
#[allow(clippy::too_many_arguments)]
fn update_grass(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    game: Res<Game>,
    cfg: Res<GrassConfig>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    grass: Option<ResMut<Grass>>,
) {
    let Some(mut g) = grass else { return };
    let Ok(cam) = cams.single() else { return };
    if g.veg.is_none() {
        return;
    }
    let eye = cam.translation();
    let passes = cfg.passes();
    // tiles near the camera stay parsed, the others go
    let tx = ((eye.x - WORLD_MIN) / TILE_SIZE).floor() as i32;
    let tz = ((eye.z - WORLD_MIN) / TILE_SIZE).floor() as i32;
    g.tiles
        .retain(|k, _| (k.0 as i32 - tx).abs() <= 1 && (k.1 as i32 - tz).abs() <= 1);
    if passes == 0 {
        return;
    }
    // blocks to drop: out of range, judged in 3-D against the block's own ground
    let range = cfg.radius;
    let gone: Vec<(i32, i32)> = g
        .blocks
        .iter()
        .filter(|(k, (_, _))| {
            let (x0, z0) = block_origin(**k);
            let d = rect_distance(eye.x, eye.z, x0, z0);
            d > range + BLOCK
        })
        .map(|(k, _)| *k)
        .collect();
    for k in gone {
        if let Some((e, _)) = g.blocks.remove(&k) {
            commands.entity(e).despawn();
        }
    }
    g.empty.retain(|k| {
        let (x0, z0) = block_origin(*k);
        rect_distance(eye.x, eye.z, x0, z0) <= range + 2.0 * BLOCK
    });

    // wanted blocks, nearest first
    let reach = (range / BLOCK).ceil() as i32 + 1;
    let (cbx, cbz) = (
        ((eye.x - WORLD_MIN) / BLOCK).floor() as i32,
        ((eye.z - WORLD_MIN) / BLOCK).floor() as i32,
    );
    let mut wanted: Vec<((i32, i32), f32)> = Vec::new();
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            let key = (cbx + dx, cbz + dz);
            if key.0 < 0 || key.1 < 0 || key.0 >= 1024 || key.1 >= 1024 {
                continue;
            }
            if g.blocks.contains_key(&key) || g.empty.contains(&key) {
                continue;
            }
            let (x0, z0) = block_origin(key);
            let d = rect_distance(eye.x, eye.z, x0, z0);
            if d <= range {
                wanted.push((key, d));
            }
        }
    }
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let started = Instant::now();
    for (key, _) in wanted {
        if started.elapsed().as_secs_f32() > BUILD_BUDGET {
            break;
        }
        let (x0, z0) = block_origin(key);
        // the tiles the block (and its blades' reach) touches
        for (cx, cz) in [(x0 - 4.0, z0 - 4.0), (x0 + BLOCK + 4.0, z0 + BLOCK + 4.0)] {
            let tx = ((cx - WORLD_MIN) / TILE_SIZE).floor();
            let tz = ((cz - WORLD_MIN) / TILE_SIZE).floor();
            if !(0.0..64.0).contains(&tx) || !(0.0..64.0).contains(&tz) {
                continue;
            }
            let k = (tx as u32, tz as u32);
            g.tiles
                .entry(k)
                .or_insert_with(|| load_tile(&game, k.0, k.1));
        }
        // (the second corner's tile can be a different one than the first's: both are
        // in the map now)
        let built = {
            let ground = TilesGround { tiles: &g.tiles };
            let veg = g.veg.as_ref().unwrap();
            build_block(veg, &ground, x0, z0, passes)
        };
        match built {
            Some((mesh, blades)) => {
                let e = commands
                    .spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(g.material.clone()),
                        Transform::from_xyz(x0, 0.0, z0),
                        GrassBlock,
                        Name::new(format!("grass {},{}", key.0, key.1)),
                    ))
                    .id();
                g.blocks.insert(key, (e, blades));
                g.stats.built_total += 1;
            }
            None => {
                g.empty.insert(key);
            }
        }
    }
    g.stats.blocks = g.blocks.len();
    g.stats.blades = g.blocks.values().map(|b| b.1).sum();
    g.stats.last_build_ms = started.elapsed().as_secs_f32() * 1000.0;
}

/// Distance from a point to an axis-aligned block (0 inside).
fn rect_distance(px: f32, pz: f32, x0: f32, z0: f32) -> f32 {
    let dx = (x0 - px).max(0.0).max(px - (x0 + BLOCK));
    let dz = (z0 - pz).max(0.0).max(pz - (z0 + BLOCK));
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::vegetation::GroundSample;

    /// JC2-237: an A2C grass card in the depth pre-pass / shadow pass writes its whole quad
    /// (no alpha discard there under MSAA), which hides the terrain behind the transparent texels.
    #[test]
    fn grass_stays_out_of_the_depth_prepass_and_shadows() {
        assert!(!<GrassMaterial as bevy::pbr::Material>::enable_prepass());
        assert!(!<GrassMaterial as bevy::pbr::Material>::enable_shadows());
        let m = GrassMaterial {
            base: StandardMaterial {
                alpha_mode: AlphaMode::AlphaToCoverage,
                ..default()
            },
            extension: GrassExt {
                sun_horizon: Handle::default(),
                ao_map: Handle::default(),
                fade: GrassFade::default(),
            },
        };
        assert!(matches!(
            bevy::pbr::Material::alpha_mode(&m),
            AlphaMode::AlphaToCoverage
        ));
    }

    fn veg() -> Option<(GameFs, Vegetation)> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        if !fs.contains(r"climate\zone_0\grass_types_0.bin") {
            return None;
        }
        let v = Vegetation::load(&fs).ok()?;
        Some((fs, v))
    }

    /// Rain-forest ground (material 0), gently tilted so the slope ranges pass, above
    /// the sea.
    fn jungle_ground(material: u8, height: f32) -> impl Fn(f32, f32) -> Option<GroundSample> {
        move |_x, _z| {
            Some(GroundSample {
                height,
                normal: [0.12, 0.98, -0.1],
                material,
            })
        }
    }

    #[test]
    fn a_block_of_grass_has_quads_on_the_ground() {
        let Some((_, v)) = veg() else { return };
        let ground = jungle_ground(0, 300.0);
        let (mesh, blades) = build_block(&v, &ground, 0.0, 0.0, 4).expect("grass on jungle ground");
        // 1 m cells in 8 m patches: every cell of a 32 m block grows a blade when the
        // zone has grass and the ground suits it; at least half of the 1024 here
        assert!(blades > 500, "{blades} blades");
        let n = mesh.count_vertices();
        assert_eq!(n, blades * QUADS_PER_BLADE * 4);
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        // every vertex stands between the ground and a couple of metres above it, and
        // inside the block (plus the blades' reach)
        for p in pos {
            assert!(p[1] >= 300.0 && p[1] < 303.0, "{p:?}");
            assert!(p[0] > -4.0 && p[0] < BLOCK + 4.0 && p[2] > -4.0 && p[2] < BLOCK + 4.0);
        }
        // fewer passes, fewer blades
        let (_, quarter) = build_block(&v, &ground, 0.0, 0.0, 1).unwrap();
        assert!(
            quarter * 3 < blades * 2 && quarter > 50,
            "{quarter} vs {blades}"
        );
    }

    #[test]
    fn no_grass_on_road_rock_snow_or_sea() {
        let Some((_, v)) = veg() else { return };
        let count = |ground: &dyn Ground| {
            build_block(&v, ground, 0.0, 0.0, 4)
                .map(|b| b.1)
                .unwrap_or(0)
        };
        // materials without a vegetation set (asphalt, pavement, bare rock: the table
        // names them "" or leaves them out) grow nothing, however gentle the ground
        let mut bare = 0;
        let mut grassy = 0;
        for i in 0..=255u8 {
            let named = v.material_names[i as usize].is_some();
            if v.materials[i as usize].is_none() && named {
                assert_eq!(count(&jungle_ground(i, 300.0)), 0, "material {i}");
                bare += 1;
            }
            // and no material grows grass on a cliff (22 degrees at the very most)
            let cliff = |_x: f32, _z: f32| {
                Some(GroundSample {
                    height: 300.0,
                    normal: [0.8, 0.6, 0.0],
                    material: i,
                })
            };
            assert_eq!(count(&cliff), 0, "material {i} grows grass on a cliff");
            if v.materials[i as usize].is_some() && count(&jungle_ground(i, 300.0)) > 0 {
                grassy += 1;
            }
        }
        assert!(bare >= 5, "{bare} materials without a set");
        assert!(grassy >= 5, "{grassy} materials grow grass");
        // the road material named in the tile listing: asphalt
        let road = v
            .material_names
            .iter()
            .position(|n| n.as_deref() == Some(""))
            .expect("a material with an empty set name");
        assert_eq!(count(&jungle_ground(road as u8, 300.0)), 0);
        // below the waterline nothing grows
        assert_eq!(count(&jungle_ground(0, 150.0)), 0);
        // the arctic's one grass type wants 11..15 degrees on both axes: not on this
        // gentle slope
        let arctic = v
            .material_names
            .iter()
            .position(|n| n.as_deref() == Some("Arctic_PineForest"))
            .expect("the arctic pine forest material");
        assert_eq!(count(&jungle_ground(arctic as u8, 900.0)), 0);
        eprintln!("{bare} bare materials, {grassy} grassy ones");
    }
}
