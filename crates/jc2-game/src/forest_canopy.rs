//! The forest canopy: the terrain's `TerrainShaderForest` layer (JC2-160, research
//! `research/formats/forest-canopy-next.md`, probe `research/tools/forest_canopy_probe.py`).
//!
//! The native game draws the terrain patch mesh a second time, 10 m higher, as an **opaque
//! alpha-tested cutout** (blend ONE / ZERO, alpha test reference 0.5, depth test and write,
//! back-face culling): a pixel either survives and overwrites the terrain colour or is
//! discarded. It is *not* alpha blended. Per pixel
//!
//! ```text
//! alpha = data.dds.x * forest_dif.a * fade          discard when alpha < 0.5
//! fade  = min(sat((c - 450 / f) / 350), 1 - sat((c - 32000 / f) / 35000))
//! f     = max(fov^2, 0.38)                           c = horizontal Chebyshev distance
//! ```
//!
//! This file holds the pure functions (fade, alpha, coverage), the canopy material and the
//! systems that put the canopy over the horizon chunks and the loaded tiles. The shader is
//! `forest_canopy.wgsl`.

// The pure functions below are the CPU mirror of forest_canopy.wgsl, checked by the tests.
#![cfg_attr(not(test), allow(dead_code))]

/// Renderer `+0xd1c`: where the canopy starts to fade in, metres at `f = 1` (the same float
/// pair as the tree impostors' Top card, `tree-impostors-next.md`).
pub const FADE_IN_START: f32 = 450.0;
/// Renderer `+0xd20`: the width of that fade-in, metres at `f = 1`.
pub const FADE_IN_WIDTH: f32 = 350.0;
/// Where the canopy starts to fade out again (`32000 / f` m): far beyond a 32 km world.
pub const FADE_OUT_START: f32 = 32_000.0;
/// The width of the fade-out (`35000` m at `f = 1`; the `f` scales only the start).
pub const FADE_OUT_WIDTH: f32 = 35_000.0;
/// The lower clamp of `f` (`f = max(fov^2, 0.38)`, the tree impostors' `ctx + 0xca8`).
pub const MIN_FADE_SCALE: f32 = 0.38;
/// Alpha-test reference (`FUN_00949aa0(7, 0x80)` -> `255 / 510`): a pixel with a smaller
/// alpha is discarded.
pub const ALPHA_REFERENCE: f32 = 0.5;
/// How far above the terrain the canopy mesh is drawn (`cb2[0].y = 10 - cam.y + 1100`).
pub const LIFT: f32 = 10.0;
/// The canopy-only factor on the sun's direct term (`sun * 0.7`).
#[allow(dead_code)] // documents the constant the shader carries
pub const SUN_SCALE: f32 = 0.7;
/// The gradient-to-normal factor of the canopy's lighting normal:
/// `N = normalize(8.56031 (0.5 - x), 0.5, 8.56031 (0.5 - y))` from `globalslopes`.
#[allow(dead_code)] // documents the constant the shader carries
pub const SLOPE_FACTOR: f32 = 8.56031;
/// Repeat of `forest_dif.dds` in metres (`uv = world.xz / 400`).
#[allow(dead_code)] // documents the constant the shader carries
pub const FOREST_REPEAT: f32 = 400.0;
/// The world edge: the global UV is `(world.xz + 16384) / 32768`.
pub const WORLD_EXTENT: f32 = 32_768.0;
/// The smallest terrain patch side the canopy is built for (patch level >= 8, 256 m).
#[allow(dead_code)] // documents the constant the shader carries
pub const MIN_PATCH_SIDE: f32 = 256.0;

/// `f = max(fov^2, 0.38)`: the distance scale shared with the tree impostors, `fov` the
/// camera's vertical field of view in radians (the realised FOV of
/// [`crate::terrain_material::TerrainLod`]).
pub fn fade_scale(fov: f32) -> f32 {
    (fov * fov).max(MIN_FADE_SCALE)
}

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// `sat((c - 450 / f) / 350)`: 0 up to `450 / f` m, 1 from `450 / f + 350` m. Evaluated as
/// the vertex shader's `c * y - x` with `x = 450 / (350 f)` and `y = 1 / 350`.
pub fn fade_in(c: f32, f: f32) -> f32 {
    sat(c * (1.0 / FADE_IN_WIDTH) - FADE_IN_START / (FADE_IN_WIDTH * f))
}

/// `1 - sat((c - 32000 / f) / 35000)`.
pub fn fade_out(c: f32, f: f32) -> f32 {
    1.0 - sat(c * (1.0 / FADE_OUT_WIDTH) - FADE_OUT_START / (FADE_OUT_WIDTH * f))
}

/// The canopy fade of a vertex at horizontal Chebyshev distance `c` (m) from the camera.
pub fn fade(c: f32, f: f32) -> f32 {
    fade_in(c, f).min(fade_out(c, f))
}

/// Horizontal Chebyshev distance of the offset `(dx, dz)` (the canopy ignores height).
pub fn chebyshev(dx: f32, dz: f32) -> f32 {
    dx.abs().max(dz.abs())
}

/// `data.x * forest_dif.a * fade`.
pub fn alpha(density: f32, texture_alpha: f32, fade: f32) -> f32 {
    density * texture_alpha * fade
}

/// True when the alpha test keeps the pixel (`alpha >= 0.5`).
pub fn passes(alpha: f32) -> bool {
    alpha >= ALPHA_REFERENCE
}

/// The three distances (m) of the canopy ramp at `f`: where the fade starts, the nearest
/// distance at which a pixel of full density and texture alpha can pass the test, and where
/// the fade reaches 1.
pub fn ramp(f: f32) -> (f32, f32, f32) {
    let start = FADE_IN_START / f;
    (
        start,
        start + FADE_IN_WIDTH * ALPHA_REFERENCE,
        start + FADE_IN_WIDTH,
    )
}

/// The share of `texture_alphas` (a `forest_dif` mip, alpha 0..1) that survives the test at
/// `density` and `fade`.
pub fn coverage(texture_alphas: &[f32], density: f32, fade: f32) -> f32 {
    if texture_alphas.is_empty() {
        return 0.0;
    }
    let kept = texture_alphas
        .iter()
        .filter(|&&a| passes(alpha(density, a, fade)))
        .count();
    kept as f32 / texture_alphas.len() as f32
}

// ------------------------------------------------------------------------------------
// The material, its data and the systems
// ------------------------------------------------------------------------------------

use crate::Game;
use crate::horizon::{CHUNK_CELLS, CHUNK_SIZE, Horizon, chunk_origin};
use crate::terrain_material::SplatContext;
use crate::world::Streamer;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use jc2_formats::dds::{DdsFormat, DdsImage};
use jc2_formats::terrain::{GLOBAL_CELL, GlobalTerrain, TILE_SIZE};
use std::collections::HashMap;

/// The density map in the archives (`FUN_00a67ef0` loads `Landscape\data.dds`).
pub const DENSITY_MAP: &str = "textures\\landscape\\data.dds";
/// The canopy colour and cutout texture.
pub const FOREST_TEXTURE: &str = "textures\\landscape\\forest_dif.dds";
/// One density texel in metres (1024 texels over 32768 m).
const DENSITY_TEXEL: f32 = WORLD_EXTENT / 1024.0;
/// A canopy mesh is skipped when no density texel it can see reaches this byte: the alpha test
/// needs `density >= 0.5` (texture alpha and fade are at most 1), so 120 / 255 is a safe margin
/// under 0.5 for the BC1 decode. An exact culling, never a visible change.
const DENSITY_CULL: u8 = 120;

/// The canopy material (`TerrainShaderForest`): opaque, alpha tested in the shader, no blending.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ForestCanopy {
    /// `data.dds` (t0), `.x` = density; BC1 UNORM, repeat, bilinear.
    #[texture(0)]
    #[sampler(1)]
    pub density: Handle<Image>,
    /// `forest_dif.dds` (t1): colour (rgb, sRGB inferred) and cutout (a); BC3 with mips, repeat,
    /// trilinear.
    #[texture(2)]
    #[sampler(3)]
    pub forest: Handle<Image>,
    /// `globalslopes.dds` (t2): the lighting normal; shared with the horizon chunks.
    #[texture(4)]
    #[sampler(5)]
    pub slopes: Handle<Image>,
    /// The AO / horizon map (t11), sampled with the slopes' clamp / linear sampler.
    #[texture(6)]
    pub ao: Handle<Image>,
    /// The per-frame sun, ambient tilt and fog constants (`sun_horizon.rs`).
    #[storage(7, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
}

impl Material for ForestCanopy {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("forest_canopy.wgsl")).with_source("embedded"),
        )
    }
    fn alpha_mode(&self) -> AlphaMode {
        // opaque: the shader discards, nothing is blended (state of `FUN_00a7d5c0`)
        AlphaMode::Opaque
    }
    // Which native passes draw the canopy is unresolved (Gap 3): it is drawn in the main pass
    // only (no depth pre-pass, no shadow cascades).
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

/// The shared canopy material.
#[derive(Resource, Clone)]
pub struct CanopyMaterial(pub Handle<ForestCanopy>);

/// Keeps the embedded shader loaded.
#[derive(Resource)]
struct CanopyShader(#[allow(dead_code)] Handle<Shader>);

/// `data.dds` on the CPU (one byte per 32 m texel) for the exact culling of canopy meshes that
/// cannot pass the alpha test.
#[derive(Resource, Clone)]
pub struct DensityMap {
    texels: Vec<u8>,
}

impl DensityMap {
    pub fn from_dds(img: &DdsImage) -> Option<Self> {
        if (img.width, img.height) != (1024, 1024) {
            return None;
        }
        let rgba = img.decode_rgba(0)?;
        Some(Self {
            texels: rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect(),
        })
    }

    fn at(&self, tx: i32, tz: i32) -> u8 {
        let c = |v: i32| v.clamp(0, 1023) as usize;
        self.texels[c(tz) * 1024 + c(tx)]
    }

    /// The largest density byte a bilinear sample can see inside the engine-space box
    /// `[x0, x1] x [z0, z1]` (one texel of margin).
    pub fn max_in(&self, x0: f32, z0: f32, x1: f32, z1: f32) -> u8 {
        let t = |v: f32| ((v + WORLD_EXTENT / 2.0) / DENSITY_TEXEL).floor() as i32;
        let mut max = 0;
        for tz in t(z0) - 1..=t(z1) + 1 {
            for tx in t(x0) - 1..=t(x1) + 1 {
                max = max.max(self.at(tx, tz));
            }
        }
        max
    }

    /// True when some pixel of the box can survive the alpha test.
    pub fn can_pass(&self, x0: f32, z0: f32, x1: f32, z1: f32) -> bool {
        self.max_in(x0, z0, x1, z1) >= DENSITY_CULL
    }
}

fn sampler(address: ImageAddressMode, mip: ImageFilterMode) -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: mip,
        ..default()
    })
}

/// A DDS level chain as a GPU image of `format` (BC1 or BC3 as stored).
pub fn bc_image(
    img: &DdsImage,
    format: TextureFormat,
    sampler: ImageSampler,
) -> Result<Image, String> {
    let bytes = img.chain_bytes().ok_or("mip chain is short")?;
    if img.data.len() < bytes {
        return Err("texture data is short".into());
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = img.mips.max(1);
    image.data = Some(img.data[..bytes].to_vec());
    image.sampler = sampler;
    Ok(image)
}

fn read_dds(game: &Game, path: &str, format: DdsFormat) -> Option<DdsImage> {
    let img = jc2_formats::landscape::read_landscape_file(&game.fs, path)
        .map_err(|e| e.to_string())
        .and_then(|d| DdsImage::parse(&d).map_err(|e| e.to_string()))
        .inspect_err(|e| warn!("{path}: {e}; no forest canopy"))
        .ok()?;
    if img.format != format {
        warn!(
            "{path}: {:?}, expected {format:?}; no forest canopy",
            img.format
        );
        return None;
    }
    Some(img)
}

/// Build the canopy material and the CPU density map from the game data and the splat
/// context (the global slope layer and the horizon constants are shared with it). Inserts
/// [`CanopyMaterial`] and [`DensityMap`]; nothing when a texture is missing or
/// `JC2_NO_CANOPY` is set (a before / after aid).
pub fn setup_material(
    commands: &mut Commands,
    game: &Game,
    splat: &SplatContext,
    images: &mut Assets<Image>,
    materials: &mut Assets<ForestCanopy>,
) {
    if std::env::var("JC2_NO_CANOPY").is_ok() {
        return;
    }
    let Some(layers) = splat.global.as_ref() else {
        return;
    };
    let Some(data) = read_dds(game, DENSITY_MAP, DdsFormat::Dxt1) else {
        return;
    };
    let Some(forest) = read_dds(game, FOREST_TEXTURE, DdsFormat::Dxt5) else {
        return;
    };
    let Some(map) = DensityMap::from_dds(&data) else {
        warn!("{DENSITY_MAP}: not 1024 x 1024; no forest canopy");
        return;
    };
    // t0: plain UNORM; t1: colour, sRGB decode (the project's diffuse policy, Gap 2)
    let density = bc_image(
        &data,
        TextureFormat::Bc1RgbaUnorm,
        sampler(ImageAddressMode::Repeat, ImageFilterMode::Nearest),
    );
    let forest = bc_image(
        &forest,
        TextureFormat::Bc3RgbaUnormSrgb,
        sampler(ImageAddressMode::Repeat, ImageFilterMode::Linear),
    );
    let (Ok(density), Ok(forest)) = (density, forest) else {
        warn!("forest canopy textures could not be converted; no forest canopy");
        return;
    };
    let material = materials.add(ForestCanopy {
        density: images.add(density),
        forest: images.add(forest),
        slopes: layers.gradient.clone(),
        ao: splat.horizon.ao.clone(),
        sun_horizon: splat.horizon.params.clone(),
    });
    commands.insert_resource(CanopyMaterial(material));
    commands.insert_resource(map);
}

/// Marks the terrain mesh entity of a loaded tile: the canopy is drawn over it as a child.
#[derive(Component)]
pub struct TileTerrainMesh(pub (u32, u32));

/// Marks a canopy mesh entity.
#[derive(Component)]
pub struct CanopyMesh;

/// Canopy entities alive, for the captures.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct CanopyStats {
    pub chunks: usize,
    pub tiles: usize,
}

pub struct ForestCanopyPlugin;

impl Plugin for ForestCanopyPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "forest_canopy.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "forest_canopy.wgsl");
        app.insert_resource(CanopyShader(shader))
            .init_resource::<CanopyStats>()
            .init_resource::<ChunkCanopies>()
            .add_plugins(MaterialPlugin::<ForestCanopy>::default())
            // after the horizon streamer: its chunk entities are spawned through commands, and the
            // canopy parents its meshes to them (unordered, the parent could still be unspawned)
            .add_systems(
                Update,
                (
                    canopy_tiles,
                    canopy_chunks.after(crate::horizon::stream_horizon),
                ),
            );
    }
}

/// The canopy over a tile's terrain mesh: the same mesh, 10 m higher (a child entity).
fn canopy_tiles(
    mut commands: Commands,
    material: Option<Res<CanopyMaterial>>,
    density: Option<Res<DensityMap>>,
    tiles: Query<(Entity, &Mesh3d, &TileTerrainMesh), Added<TileTerrainMesh>>,
    mut stats: ResMut<CanopyStats>,
) {
    let (Some(material), Some(density)) = (material, density) else {
        return;
    };
    for (entity, mesh, tile) in &tiles {
        let (x0, z0) = jc2_formats::terrain::tile_origin(tile.0.0, tile.0.1);
        if !density.can_pass(x0, z0, x0 + TILE_SIZE, z0 + TILE_SIZE) {
            continue;
        }
        commands.entity(entity).with_child((
            Mesh3d(mesh.0.clone()),
            MeshMaterial3d(material.0.clone()),
            Transform::from_xyz(0.0, LIFT, 0.0),
            NotShadowCaster,
            CanopyMesh,
            Name::new(format!("canopy {},{}", tile.0.0, tile.0.1)),
        ));
        stats.tiles += 1;
    }
}

/// A horizon chunk's canopy: its parent chunk entity, the tiles of the chunk that were loaded
/// when the mesh was built (bit `a + 2 b` = tile `a, b` of the chunk) and the canopy entity.
struct ChunkCanopy {
    parent: Entity,
    covered: u8,
    mesh: Handle<Mesh>,
    entity: Option<Entity>,
}

#[derive(Resource, Default)]
struct ChunkCanopies(HashMap<(i32, i32), ChunkCanopy>);

/// Chunks (re)built per frame.
const CHUNKS_PER_FRAME: usize = 6;

/// The canopy mesh of a horizon chunk: the global terrain at its true height `+ LIFT` (the
/// chunk itself sinks up to 18 m, the canopy is applied to the unsunk height), without the
/// quads of the tiles in `covered` (those draw their own canopy), or `None` when nothing is
/// left. Positions only: the lighting normal comes from `globalslopes`.
pub fn chunk_canopy_mesh(
    map: &GlobalTerrain,
    chunk: (i32, i32),
    step: usize,
    covered: u8,
) -> Option<Mesh> {
    let (x0, z0) = chunk_origin(chunk);
    let cell = GLOBAL_CELL * step as f32;
    let n = CHUNK_CELLS / step + 1;
    let mut positions = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (x0 + i as f32 * cell, z0 + j as f32 * cell);
            positions.push(crate::coords::bevy_xyz(x, map.height_at(x, z) + LIFT, z).to_array());
        }
    }
    let tiles_per_cell = cell / TILE_SIZE;
    let mut indices: Vec<u32> = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let (ta, tb) = (
                (i as f32 * tiles_per_cell) as u32,
                (j as f32 * tiles_per_cell) as u32,
            );
            if covered & (1 << (ta + 2 * tb)) != 0 {
                continue;
            }
            let a = (j * n + i) as u32;
            let (b, c) = (a + 1, a + n as u32);
            // counter-clockwise seen from above, like the chunk mesh
            indices.extend_from_slice(&[a, c, b, b, c, c + 1]);
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// Which of the four tiles of a chunk are loaded (bit `a + 2 b`).
fn covered_mask(streamer: &Streamer, chunk: (i32, i32)) -> u8 {
    let per = (CHUNK_SIZE / TILE_SIZE) as i32;
    let mut mask = 0;
    for b in 0..per {
        for a in 0..per {
            let t = ((chunk.0 * per + a) as u32, (chunk.1 * per + b) as u32);
            if streamer.loaded.contains_key(&t) {
                mask |= 1 << (a + per * b);
            }
        }
    }
    mask
}

/// Keep one canopy mesh per horizon chunk that has forest in it, rebuilt when the chunk
/// entity is replaced (new detail) or the loaded tiles over it change.
#[allow(clippy::too_many_arguments)]
fn canopy_chunks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut state: ResMut<ChunkCanopies>,
    mut stats: ResMut<CanopyStats>,
    horizon: Option<Res<Horizon>>,
    material: Option<Res<CanopyMaterial>>,
    density: Option<Res<DensityMap>>,
    streamer: Option<Res<Streamer>>,
) {
    let (Some(hz), Some(material), Some(density)) = (horizon, material, density) else {
        return;
    };
    // forget chunks the horizon dropped (their canopy went with the chunk entity)
    state.0.retain(|c, _| hz.chunks.contains_key(c));
    let mut built = 0;
    for (&c, &(parent, step)) in &hz.chunks {
        let covered = streamer.as_ref().map_or(0, |s| covered_mask(s, c));
        let old = state.0.get(&c);
        let same_parent = old.is_some_and(|k| k.parent == parent);
        if same_parent && old.is_some_and(|k| k.covered == covered) {
            continue;
        }
        if built >= CHUNKS_PER_FRAME {
            break;
        }
        built += 1;
        let (x0, z0) = chunk_origin(c);
        let mesh = density
            .can_pass(x0, z0, x0 + CHUNK_SIZE, z0 + CHUNK_SIZE)
            .then(|| chunk_canopy_mesh(&hz.map, c, step, covered))
            .flatten();
        let existing = old
            .filter(|_| same_parent)
            .and_then(|k| k.entity.map(|e| (e, k.mesh.clone())));
        let (entity, handle) = match (existing, mesh) {
            // same chunk entity, new mesh contents
            (Some((e, h)), Some(mesh)) => {
                let _ = meshes.insert(h.id(), mesh);
                (Some(e), h)
            }
            (Some((e, _)), None) => {
                commands.entity(e).despawn();
                (None, Handle::default())
            }
            (None, Some(mesh)) => {
                let handle = meshes.add(mesh);
                let e = commands
                    .spawn((
                        Mesh3d(handle.clone()),
                        MeshMaterial3d(material.0.clone()),
                        Transform::IDENTITY,
                        NotShadowCaster,
                        CanopyMesh,
                        Name::new(format!("canopy chunk {},{}", c.0, c.1)),
                    ))
                    .id();
                // a child of the chunk: it follows the chunk's visibility and its despawn
                commands.entity(parent).add_child(e);
                (Some(e), handle)
            }
            (None, None) => (None, Handle::default()),
        };
        state.0.insert(
            c,
            ChunkCanopy {
                parent,
                covered,
                mesh: handle,
                entity,
            },
        );
    }
    stats.chunks = state.0.values().filter(|k| k.entity.is_some()).count();
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::dds::DdsImage;

    fn near(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() < eps, "{a} vs {b}");
    }

    fn landscape(name: &str) -> Option<DdsImage> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        let path = format!("textures\\landscape\\{name}.dds");
        let bytes = jc2_formats::landscape::read_landscape_file(&fs, &path).ok()?;
        DdsImage::parse(&bytes).ok()
    }

    /// The probe's draw-range table: the fade is 0 at `450 / f`, 0.5 at +175 m, 1 at +350 m
    /// for `f` = 1, the 40 degree on-foot FOV and the clamp.
    #[test]
    fn the_fade_ramp_matches_the_probe_table() {
        let on_foot = fade_scale(40f32.to_radians());
        near(on_foot, 0.4874, 5e-4);
        for (f, start, half, full) in [
            (1.0, 450.0, 625.0, 800.0),
            (on_foot, 923.3, 1098.3, 1273.3),
            (0.38, 1184.2, 1359.2, 1534.2),
        ] {
            let (s, h, e) = ramp(f);
            near(s, start, 0.1);
            near(h, half, 0.1);
            near(e, full, 0.1);
            near(fade(s, f), 0.0, 1e-5);
            near(fade(h, f), 0.5, 1e-4);
            near(fade(e, f), 1.0, 1e-5);
            // inside the start it is flat 0, beyond the end flat 1, and still 1 at 20 km
            assert_eq!(fade(0.0, f), 0.0);
            assert_eq!(fade(s * 0.5, f), 0.0);
            assert_eq!(fade(20_000.0, f), 1.0);
        }
    }

    /// `f` clamps at 0.38 for any FOV up to 0.6164 rad (35.3 degrees) and follows `fov^2` above.
    #[test]
    fn the_fade_scale_clamps_at_038() {
        assert_eq!(fade_scale(0.2), 0.38);
        assert_eq!(fade_scale(0.6164), 0.38);
        near(fade_scale(1.0), 1.0, 1e-6);
        near(fade_scale(std::f32::consts::FRAC_PI_4), 0.61685, 1e-4);
    }

    /// The far ramp: 1 up to `32000 / f`, 0 at `32000 / f + 35000`.
    #[test]
    fn the_far_fade_is_gone_beyond_67_km() {
        near(fade_out(32_000.0, 1.0), 1.0, 1e-6);
        near(fade_out(32_000.0 + 17_500.0, 1.0), 0.5, 1e-5);
        near(fade_out(67_000.0, 1.0), 0.0, 1e-5);
        assert_eq!(fade(70_000.0, 1.0), 0.0);
        // the minimum of the two
        near(fade(40_000.0, 1.0), fade_out(40_000.0, 1.0), 1e-6);
    }

    /// The canopy takes over where the Top impostor card fades out: its fade-in is the card's
    /// fade-out complement over the whole ramp (probe P6, same constant pair).
    #[test]
    fn the_canopy_fade_in_complements_the_card_fade_out() {
        let card_fade_out = |c: f32| 1.0 - sat(c * (1.0 / 350.0) - 450.0 / 350.0);
        for c in (0..1200).step_by(25) {
            let c = c as f32;
            near(card_fade_out(c) + fade(c, 1.0), 1.0, 1e-5);
        }
    }

    #[test]
    fn the_distance_is_horizontal_chebyshev() {
        assert_eq!(chebyshev(-300.0, 120.0), 300.0);
        assert_eq!(chebyshev(5.0, -40.0), 40.0);
        // the camera's height does not enter: the function has no y argument
        assert_eq!(chebyshev(0.0, 0.0), 0.0);
    }

    #[test]
    fn the_alpha_test_is_a_cutout_at_one_half() {
        assert!(!passes(alpha(1.0, 1.0, 0.499)));
        assert!(passes(alpha(1.0, 1.0, 0.5)));
        // a density of 0.84 needs a texture alpha of 0.5952: the far mip-average look
        assert!(!passes(alpha(0.84, 0.5952, 1.0)));
        assert!(passes(alpha(0.84, 0.5953, 1.0)));
        // no density, no canopy
        assert!(!passes(alpha(0.0, 1.0, 1.0)));
    }

    /// The probe's coverage fixture (`forest_dif` mip 0 alpha, full density, `f` = 1):
    /// 0 % at 600 m, 27.4 % at 650 m, 47.4 % at 700 m, 63.6 % at 800 m.
    #[test]
    fn the_coverage_matches_the_probe_fixtures() {
        let Some(img) = landscape("forest_dif") else {
            return;
        };
        assert_eq!((img.width, img.height, img.mips), (256, 256, 9));
        let rgba = img.decode_rgba(0).unwrap();
        let alphas: Vec<f32> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3] as f32 / 255.0)
            .collect();
        let at = |c: f32| coverage(&alphas, 1.0, fade(c, 1.0));
        assert_eq!(at(600.0), 0.0);
        near(at(650.0), 0.274, 0.004);
        near(at(700.0), 0.474, 0.004);
        near(at(750.0), 0.569, 0.004);
        near(at(800.0), 0.636, 0.004);
        // mean alpha 0.596 (the mip-averaged look), 63.7 % of texels at or above 0.5
        near(
            alphas.iter().sum::<f32>() / alphas.len() as f32,
            0.596,
            0.003,
        );
        near(coverage(&alphas, 1.0, 1.0), 0.637, 0.003);
    }

    /// The probe's `data.dds` fixtures: tile 40,40 (16 x 16 texels) is dense jungle, the
    /// start base tile 31,13 has no canopy at all.
    #[test]
    fn the_density_map_matches_the_probe_fixtures() {
        let Some(img) = landscape("data") else {
            return;
        };
        assert_eq!((img.width, img.height, img.mips), (1024, 1024, 1));
        let rgba = img.decode_rgba(0).unwrap();
        let tile = |tx: u32, tz: u32| -> Vec<f32> {
            let mut v = Vec::new();
            for j in 0..16 {
                for i in 0..16 {
                    let (x, z) = (tx * 16 + i, tz * 16 + j);
                    v.push(rgba[((z * 1024 + x) * 4) as usize] as f32 / 255.0);
                }
            }
            v
        };
        let jungle = tile(40, 40);
        near(jungle.iter().sum::<f32>() / 256.0, 0.792, 0.01);
        let ge = |v: &[f32], t: f32| v.iter().filter(|&&d| d >= t).count() as f32 / v.len() as f32;
        near(ge(&jungle, 0.5), 0.789, 0.01);
        near(ge(&jungle, 0.84), 0.652, 0.02);
        let base = tile(31, 13);
        assert!(
            base.iter().all(|&d| d == 0.0),
            "the start base has no canopy"
        );
    }
}
