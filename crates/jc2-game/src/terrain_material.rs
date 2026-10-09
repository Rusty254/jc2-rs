//! The terrain's real materials: the game's meta-material layers drawn with the game's
//! atlas textures the way its `FastTer_Lod0` pixel shader draws them (see
//! `research/formats/terrain.md`, "The terrain shader", and `jc2_formats::landscape`).
//!
//! Two levels, so there is always something to see:
//!
//! * **Vertex colours** ([`vertex_colour`]): every terrain vertex gets the mean colour
//!   of the two atlas textures the shader would blend in its cell (`blend_layers`).
//!   Cheap, needs no shader, and already puts roads, rock, snow and sand where they
//!   belong. Used when the splat material cannot be built (no atlas in the data).
//! * **Splat material** ([`TerrainSplat`], an `ExtendedMaterial` over
//!   `StandardMaterial` with `terrain_splat.wgsl`): per tile, the slot and weight
//!   layers go to the GPU as two 128 × 128 textures, the zone's atlas as a 32-layer
//!   texture array (the 256 px tiles of the 2048 × 1024 DXT1 atlas, mip chain kept),
//!   the zone's normal atlas likewise, and the zone's slot × slope-band texture table
//!   as a uniform. The fragment shader repeats the game's selection: weight curve, the
//!   two heaviest slots, each split over the slope band and the next, the two heaviest
//!   of those four, sampled in world space at the game's 3.05 m repeat and mixed; the
//!   normal atlas bends the slope normal in the game's frame
//!   (`N (1 + n.z) + 4 n.x T + 4 n.y B`). With distance the game's LOD shaders take over
//!   by its own blend ([`lod_blend`]): the 21.3 m repeat (`Lod1`, no normal atlas), then
//!   the per-texture colour table (`Lod2`).
//!
//! The slope comes from the tile's A8L8 gradient layer ([`gradient_image`]), not the
//! mesh. The displacement atlas (`globaldisplaceatlas.dds`) bends the slope before the
//! second band selection and the lighting normal, as the game's shader does (terrain.md,
//! "LOD blend distances, mips and lighting terms"). Not done: the game's own light
//! colours (their producers are not recovered); Bevy's PBR lights the native normal and
//! albedo, with the game's hollows term as ambient-only occlusion.

use crate::Game;
use crate::sun_horizon::{HorizonShadow, update_horizon};
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use jc2_formats::dds::{DdsFormat, DdsImage};
use jc2_formats::landscape::{
    ATLAS_COLUMNS, ATLAS_REPEAT_FAR, ATLAS_REPEAT_NEAR, ATLAS_ROWS, ATLAS_TEXTURES, Landscape,
    META_SLOTS, SLOPE_SLOTS, SLOPE_UNIT, diffuse_atlas_name, gradient_from_up, normal_atlas_name,
};
use jc2_formats::terrain::{MetaCell, TILE_CELLS, TerrainTile};
use std::sync::Arc;

/// How far the normal atlas bends the normal: the game scales the decoded xy by 4.
pub const NORMAL_STRENGTH: f32 = 4.0;

/// The terrain LOD blend of the game (`CTerrainRenderBlockType` constructor
/// `FUN_00a60d30`, renderer `+0x4b4`, `+0x4b0`, `+0x4bc`, `+0x4b8`): radius of pure LOD 0
/// before the FOV scale, the width of its blend into LOD 1, and the same for LOD 1 into 2,
/// in metres of Chebyshev distance from the camera. Mirrored in `terrain_splat.wgsl`.
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const LOD0_RADIUS: f32 = 25.0;
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const LOD0_WIDTH: f32 = 25.0;
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const LOD1_RADIUS: f32 = 250.0;
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const LOD1_WIDTH: f32 = 50.0;
/// Upper clamp of the FOV scale (`FUN_00a61220`).
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const LOD_SCALE_MAX: f32 = 999.99994;
/// The game camera's default vertical FOV (camera constructor, `0x3f91361e`, 65°).
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub const NATIVE_FOV: f32 = 1.134_464;

/// The FOV scale of the LOD radii: `min(1 / fov², 999.99994)`, `fov` the camera's
/// vertical field of view in radians (`FUN_00a61220`, camera `+0x618`).
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub fn lod_scale(fov: f32) -> f32 {
    (1.0 / (fov * fov)).min(LOD_SCALE_MAX)
}

/// The two LOD weights the game's terrain vertex shader (`fastter_sh*`) passes on for a
/// point at camera-relative offset `rel`: `t0` (1 = LOD 0, the near atlas) and `t1`
/// (1 = LOD 1 or nearer, 0 = the colour table), `1 - saturate((d - r k) / w)` of the
/// Chebyshev distance `d`.
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub fn lod_blend(rel: Vec3, fov: f32) -> (f32, f32) {
    let d = rel.abs().max_element();
    let k = lod_scale(fov);
    let t = |r: f32, w: f32| 1.0 - ((d - r * k) / w).clamp(0.0, 1.0);
    (t(LOD0_RADIUS, LOD0_WIDTH), t(LOD1_RADIUS, LOD1_WIDTH))
}

/// The explicit mip levels of `FastTer_*`: `D = 0.5 log2` of the summed squared screen
/// derivatives of world x and z; displacement atlas `D + 0.5`, near atlas `D + 5.5`, far
/// atlas `D + 2.8`, each clamped to 0..6.
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub fn native_mips(derivs_sq: f32) -> (f32, f32, f32) {
    let d = 0.5 * derivs_sq.log2();
    let c = |o: f32| (d + o).clamp(0.0, 6.0);
    (c(0.5), c(5.5), c(2.8))
}

/// Where the game samples displacement tile `tile` for a world point at mip `mip`: the
/// tile's quarter of the atlas, inset by one texel of that mip on every side.
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub fn displace_uv(tile: u32, x: f32, z: f32, mip: f32) -> (f32, f32) {
    let m = mip.exp2();
    let shrink = 1.0 - m / 512.0;
    let fx = (x / 256.0).rem_euclid(1.0);
    let fz = (z / 256.0).rem_euclid(1.0);
    (
        tile as f32 / 4.0 + m / 4096.0 + 0.25 * fx * shrink,
        m / 1024.0 + fz * shrink,
    )
}

/// The terrain material: StandardMaterial lighting with our splat base colour.
pub type TerrainSplatMaterial = ExtendedMaterial<StandardMaterial, TerrainSplat>;

/// Per-tile uniform: `origin` = (tile x0, tile z0 in engine metres, near repeat, far
/// repeat), `slope` = (slope unit, bands, normal-map strength or 0 without a normal
/// atlas, displacement on/off), the slope table of the tile's texture zone:
/// `table[slot * 16 + band] = atlas texture index` (bit 16 set when the slot is undefined
/// in this zone), and the game's colour table for the same entries (`colours`, sRGB
/// RGBA8: `MetaMaterialColors`, what `FastTer_Lod2` draws).
#[derive(ShaderType, Clone, Debug)]
pub struct SplatParams {
    pub origin: Vec4,
    pub slope: Vec4,
    pub table: [UVec4; META_SLOTS * SLOPE_SLOTS / 4],
    pub colours: [UVec4; META_SLOTS * SLOPE_SLOTS / 4],
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct TerrainSplat {
    #[uniform(100)]
    pub params: SplatParams,
    /// The zone's diffuse atlas as 32 layers.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub atlas: Handle<Image>,
    /// 128 × 128 `Rgba8Uint`: the four slots of every cell.
    #[texture(103, sample_type = "u_int")]
    pub slots: Handle<Image>,
    /// 128 × 128 `Rgba8Unorm`: their weights.
    #[texture(104)]
    #[sampler(105)]
    pub weights: Handle<Image>,
    /// The zone's normal atlas as 32 layers (the diffuse atlas again, unused, when the
    /// zone has none — `params.slope.z` is 0 then).
    #[texture(106, dimension = "2d_array")]
    #[sampler(107)]
    pub normals: Handle<Image>,
    /// `globaldisplaceatlas.dds` (4096 × 1024 BC3, four tiles): the normal-map relief
    /// that bends the slope bands and the lighting (a 1 × 1 stand-in without it,
    /// `params.slope.w` is 0 then).
    #[texture(108)]
    #[sampler(109)]
    pub displace: Handle<Image>,
    /// 128 x 128 `Rg8Unorm` with mips: the tile's A8L8 gradient layer (L, A), the slope
    /// the game's shader reads (`t5`, `x 2 - 1`).
    #[texture(110)]
    #[sampler(111)]
    pub gradient: Handle<Image>,
    /// `ambientocclusionmap.dds`, shared (sampled with the weights' clamp / linear
    /// sampler): see `sun_horizon.rs`.
    #[texture(112)]
    pub ao: Handle<Image>,
    /// The per-frame horizon constants (one `SunHorizon`), shared.
    #[storage(113, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
}

impl MaterialExtension for TerrainSplat {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("terrain_splat.wgsl")).with_source("embedded"),
        )
    }
}

/// Registers the material and its shader.
pub struct TerrainMaterialPlugin;

/// Keeps the embedded splat shader loaded for as long as the app runs.
#[derive(Resource)]
pub struct TerrainShader(#[allow(dead_code)] pub Handle<Shader>);

impl Plugin for TerrainMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain_splat.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "terrain_splat.wgsl");
        bevy::asset::embedded_asset!(app, "sun_horizon.wgsl");
        let horizon: Handle<Shader> = load_embedded_asset!(app, "sun_horizon.wgsl");
        app.insert_resource(TerrainShader(shader));
        app.insert_resource(SunHorizonShader(horizon));
        app.add_plugins(MaterialPlugin::<TerrainSplatMaterial>::default());
        app.add_systems(Update, update_horizon);
    }
}

/// Keeps the shared `jc2::sun_horizon` shader module loaded.
#[derive(Resource)]
pub struct SunHorizonShader(#[allow(dead_code)] pub Handle<Shader>);

/// The shared pieces of the splat material: the tables and one atlas array per
/// texture zone. Built once at start-up from the game data.
pub struct SplatContext {
    pub land: Arc<Landscape>,
    /// Diffuse atlas array image per texture zone (`None` when that atlas is missing).
    pub atlases: Vec<Option<Handle<Image>>>,
    /// Normal atlas array image per texture zone, when present.
    pub normal_atlases: Vec<Option<Handle<Image>>>,
    /// The displacement atlas, shared by every tile; `None` when it is missing.
    pub displace: Option<Handle<Image>>,
    /// A stand-in bound when there is no displacement atlas.
    pub blank: Handle<Image>,
    /// The horizon / AO map and constants every tile's material binds.
    pub horizon: HorizonShadow,
}

impl SplatContext {
    /// Load the climate tables and slice the atlases. `None` when the tables are not
    /// in the data (the vertex-colour fallback is used then).
    pub fn new(
        game: &Game,
        images: &mut Assets<Image>,
        buffers: Option<&mut Assets<ShaderBuffer>>,
    ) -> Option<Self> {
        let land = match Landscape::load(&game.fs) {
            Ok(l) => l,
            Err(e) => {
                warn!("terrain materials unavailable ({e}); terrain gets height colours");
                return None;
            }
        };
        let zones = land.metas.texture_zone_count().max(1);
        let mut atlases = Vec::with_capacity(zones);
        let mut normal_atlases = Vec::with_capacity(zones);
        let mut load = |name: String, srgb: bool| {
            game.fs
                .read_inflated(&name)
                .ok()
                .and_then(|d| DdsImage::parse(&d).ok())
                .and_then(|img| match atlas_array_image(&img, srgb) {
                    Ok(i) => Some(images.add(i)),
                    Err(e) => {
                        warn!("{name}: {e}");
                        None
                    }
                })
        };
        for zone in 0..zones as u8 {
            let atlas = load(diffuse_atlas_name(zone), true);
            if atlas.is_none() {
                warn!(
                    "{} not found: texture zone {zone} gets vertex colours",
                    diffuse_atlas_name(zone)
                );
            }
            atlases.push(atlas);
            normal_atlases.push(load(normal_atlas_name(zone), false));
        }
        // JC2_NO_DISPLACE=1 draws without it (to compare)
        let displace = game
            .fs
            .read_inflated(DISPLACE_ATLAS)
            .ok()
            .filter(|_| std::env::var("JC2_NO_DISPLACE").is_err())
            .and_then(|d| DdsImage::parse(&d).ok())
            .and_then(|img| match displace_image(&img) {
                Ok(i) => Some(images.add(i)),
                Err(e) => {
                    warn!("{DISPLACE_ATLAS}: {e}");
                    None
                }
            });
        let blank = images.add(Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![128, 128, 255, 128],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        ));
        Some(Self {
            land: Arc::new(land),
            atlases,
            normal_atlases,
            displace,
            blank,
            horizon: HorizonShadow::load(game, images, buffers),
        })
    }

    /// The colour table of texture zone `zone` (`MetaMaterialColors`: per slot x band the
    /// `globalcolorpixelmap` texel of the entry's atlas texture) packed as sRGB RGBA8.
    pub fn colours(&self, zone: u8) -> [UVec4; META_SLOTS * SLOPE_SLOTS / 4] {
        colour_table(&self.land, zone)
    }

    /// The slope table of texture zone `zone` packed for the uniform.
    fn table(&self, zone: u8) -> [UVec4; META_SLOTS * SLOPE_SLOTS / 4] {
        let slots = self.land.metas.slots(zone);
        let mut flat = [0u32; META_SLOTS * SLOPE_SLOTS];
        for (s, meta) in slots.iter().enumerate() {
            for k in 0..SLOPE_SLOTS {
                flat[s * SLOPE_SLOTS + k] = match meta {
                    Some(m) => pack_entry(&m.slopes[k]),
                    // an undefined slot: texture 0, flagged
                    None => 0x1_0000,
                };
            }
        }
        pack_flat(&flat)
    }

    /// The material of tile `(x, z)`: its layers as textures plus the zone's atlas.
    /// `None` when the tile has no layers or the zone's atlas is missing.
    pub fn material_for_tile(
        &self,
        tile: &TerrainTile,
        x: u32,
        z: u32,
        images: &mut Assets<Image>,
        materials: &mut Assets<TerrainSplatMaterial>,
    ) -> Option<Handle<TerrainSplatMaterial>> {
        if !tile.has_layers() {
            return None;
        }
        let zone = self.land.texture_zone(x, z);
        let atlas = self.atlases.get(zone as usize)?.clone()?;
        let normals = self.normal_atlases.get(zone as usize).cloned().flatten();
        let normal_strength = if normals.is_some() {
            NORMAL_STRENGTH
        } else {
            0.0
        };
        let normals = normals.unwrap_or_else(|| atlas.clone());
        let (slots, weights) = layer_images(tile);
        let (x0, z0) = jc2_formats::terrain::tile_origin(x, z);
        let material = TerrainSplatMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 1.0,
                metallic: 0.0,
                reflectance: 0.1,
                ..default()
            },
            extension: TerrainSplat {
                params: SplatParams {
                    origin: Vec4::new(x0, z0, ATLAS_REPEAT_NEAR, ATLAS_REPEAT_FAR),
                    slope: Vec4::new(
                        SLOPE_UNIT,
                        jc2_formats::landscape::SLOPE_BANDS,
                        normal_strength,
                        if self.displace.is_some() { 1.0 } else { 0.0 },
                    ),
                    table: self.table(zone),
                    colours: self.colours(zone),
                },
                atlas,
                slots: images.add(slots),
                weights: images.add(weights),
                normals,
                displace: self.displace.clone().unwrap_or_else(|| self.blank.clone()),
                gradient: images.add(gradient_image(tile)),
                ao: self.horizon.ao.clone(),
                sun_horizon: self.horizon.params.clone(),
            },
        };
        Some(materials.add(material))
    }
}

fn pack_flat(flat: &[u32; META_SLOTS * SLOPE_SLOTS]) -> [UVec4; META_SLOTS * SLOPE_SLOTS / 4] {
    let mut out = [UVec4::ZERO; META_SLOTS * SLOPE_SLOTS / 4];
    for (v, c) in out.iter_mut().zip(flat.as_chunks::<4>().0) {
        *v = UVec4::from_array(*c);
    }
    out
}

/// `MetaMaterialColors` of texture zone `zone` as the uniform carries it: entry
/// `slot * 16 + band` is the `globalcolorpixelmap` texel of that entry's atlas texture
/// (`FUN_00a70fb0`; [`Landscape::slot_colour`]), packed by [`pack_colour`].
pub fn colour_table(land: &Landscape, zone: u8) -> [UVec4; META_SLOTS * SLOPE_SLOTS / 4] {
    let mut flat = [0u32; META_SLOTS * SLOPE_SLOTS];
    for (i, v) in flat.iter_mut().enumerate() {
        let (slot, band) = (i / SLOPE_SLOTS, i % SLOPE_SLOTS);
        *v = pack_colour(land.slot_colour(zone, slot as u8, band));
    }
    pack_flat(&flat)
}

/// An sRGB colour (0..1) as the RGBA8 word the shader unpacks; magenta when unknown.
pub fn pack_colour(c: Option<[f32; 3]>) -> u32 {
    let [r, g, b] = c.unwrap_or([1.0, 0.0, 1.0]);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(r) | (byte(g) << 8) | (byte(b) << 16) | 0xff00_0000
}

/// The tile's gradient layer as the shader samples it: `Rg8Unorm` (L = x, A = z) with the
/// file's mip chain. A tile whose layer is not a 128 x 128 A8L8 gets the same bytes
/// computed from its heights (the central difference the layer was baked from), one mip.
pub fn gradient_image(tile: &TerrainTile) -> Image {
    let size = Extent3d {
        width: TILE_CELLS as u32,
        height: TILE_CELLS as u32,
        depth_or_array_layers: 1,
    };
    let layer = tile
        .normal_map()
        .filter(|l| {
            l.format == DdsFormat::A8L8
                && l.width as usize == TILE_CELLS
                && l.height as usize == TILE_CELLS
        })
        .and_then(|l| Some((l, l.chain_bytes()?)))
        .filter(|(l, bytes)| l.data.len() >= *bytes);
    let mut image = match layer {
        Some((l, bytes)) => {
            let mut image = Image::new_uninit(
                size,
                TextureDimension::D2,
                TextureFormat::Rg8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.mip_level_count = l.mips.max(1);
            image.data = Some(l.data[..bytes].to_vec());
            image
        }
        None => {
            let unit = jc2_formats::terrain::HEIGHT_SCALE;
            let byte = |g: f32| (128.0 + g / unit).round().clamp(0.0, 255.0) as u8;
            let mut data = Vec::with_capacity(TILE_CELLS * TILE_CELLS * 2);
            for cz in 0..TILE_CELLS {
                for cx in 0..TILE_CELLS {
                    let (gx, gz) = tile.cell_gradient(cz, cx);
                    data.extend_from_slice(&[byte(gx), byte(gz)]);
                }
            }
            Image::new(
                size,
                TextureDimension::D2,
                data,
                TextureFormat::Rg8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            )
        }
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The game's displacement atlas.
pub const DISPLACE_ATLAS: &str = "textures\\landscape\\globaldisplaceatlas.dds";

/// One slot × band table entry as the shader reads it: the atlas texture (bits 0..7), the
/// displacement tile (8..9), `f2` (17..24, × 255) and `f3` (25..31, × 200); bit 16 is set
/// by the caller for undefined slots.
pub fn pack_entry(s: &jc2_formats::landscape::SlopeSlot) -> u32 {
    let f2 = (s.band_bump.clamp(0.0, 1.0) * 255.0).round() as u32;
    let f3 = (s.light_bump.clamp(0.0, 0.63) * 200.0).round() as u32;
    s.texture as u32 | ((s.displace_tile as u32 & 3) << 8) | (f2 << 17) | (f3 << 25)
}

/// The displacement atlas as one BC3 texture with its whole mip chain.
pub fn displace_image(img: &DdsImage) -> Result<Image, String> {
    if img.format != DdsFormat::Dxt5 {
        return Err(format!("expected DXT5, got {:?}", img.format));
    }
    let bytes = img.chain_bytes().ok_or("mip chain is short")?;
    let mut image = Image::new_uninit(
        Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bc3RgbaUnorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = img.mips.max(1);
    image.data = Some(img.data[..bytes].to_vec());
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    Ok(image)
}

/// The slot and weight layers of a tile as GPU textures (`Rgba8Uint`, `Rgba8Unorm`).
pub fn layer_images(tile: &TerrainTile) -> (Image, Image) {
    let n = TILE_CELLS * TILE_CELLS;
    let mut slots = Vec::with_capacity(n * 4);
    let mut weights = Vec::with_capacity(n * 4);
    for cz in 0..TILE_CELLS {
        for cx in 0..TILE_CELLS {
            let c = tile.meta_cell(cz, cx);
            slots.extend_from_slice(&c.slots);
            weights.extend(c.weights.iter().map(|w| w * 17));
        }
    }
    let size = Extent3d {
        width: TILE_CELLS as u32,
        height: TILE_CELLS as u32,
        depth_or_array_layers: 1,
    };
    let slots = Image::new(
        size,
        TextureDimension::D2,
        slots,
        TextureFormat::Rgba8Uint,
        RenderAssetUsages::RENDER_WORLD,
    );
    let mut weights = Image::new(
        size,
        TextureDimension::D2,
        weights,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    weights.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    (slots, weights)
}

/// Slice a DXT1 atlas (8 × 4 square tiles) into a 32-layer BC1 array texture (sRGB for
/// colours, linear for normals), keeping every mip level down to one 4 × 4 block per
/// tile.
pub fn atlas_array_image(img: &DdsImage, srgb: bool) -> Result<Image, String> {
    if img.format != DdsFormat::Dxt1 {
        return Err(format!("atlas is {:?}, expected DXT1", img.format));
    }
    let tile = img.width / ATLAS_COLUMNS;
    if tile == 0 || tile * ATLAS_ROWS != img.height || !tile.is_multiple_of(4) {
        return Err(format!(
            "atlas is {}×{}, expected 8 × 4 square tiles",
            img.width, img.height
        ));
    }
    // mips while the tile is still ≥ 4 px (one block) and the file has them
    let mut mips = 1;
    while mips < img.mips && (tile >> mips) >= 4 {
        mips += 1;
    }
    let mut data = Vec::new();
    for k in 0..ATLAS_TEXTURES as u32 {
        let (row, col) = (k / ATLAS_COLUMNS, k % ATLAS_COLUMNS);
        for level in 0..mips {
            let src = img.level(level).ok_or("atlas mip chain is short")?;
            let (lw, _) = img.level_size(level);
            let blocks_per_row = lw / 4;
            let tb = (tile >> level) / 4; // blocks per tile edge at this level
            for by in 0..tb {
                let start = (((row * tb + by) * blocks_per_row + col * tb) * 8) as usize;
                data.extend_from_slice(&src[start..start + (tb * 8) as usize]);
            }
        }
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: tile,
            height: tile,
            depth_or_array_layers: ATLAS_TEXTURES as u32,
        },
        TextureDimension::D2,
        if srgb {
            TextureFormat::Bc1RgbaUnormSrgb
        } else {
            TextureFormat::Bc1RgbaUnorm
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mips;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    Ok(image)
}

/// sRGB byte-scale colour (0..1 as stored in the atlas) → linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear RGBA of a terrain vertex: the mean colours of the two atlas textures the
/// game's shader blends in its cell on ground with vertical component `up`. `None` when
/// the tables do not cover the cell (no atlas for the zone, unknown slot).
pub fn vertex_colour(land: &Landscape, zone: u8, cell: &MetaCell, up: f32) -> Option<[f32; 4]> {
    let c = land.cell_colour(zone, cell, gradient_from_up(up))?;
    Some([
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
        1.0,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::dds::write_dds;

    #[test]
    fn atlas_slices_keep_each_tile_and_its_mips() {
        // 32×16 atlas → 4×4 tiles, 2 mips (4 px, then one block of 2×2 would be too small)
        let (w, h) = (32u32, 16u32);
        let mut data = Vec::new();
        // mip 0: block (bx, by) gets colour c0 = bx + by * 8 so tiles are distinguishable
        for by in 0..h / 4 {
            for bx in 0..w / 4 {
                data.extend_from_slice(&((bx + by * 8) as u16).to_le_bytes());
                data.extend_from_slice(&0u16.to_le_bytes());
                data.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        // mip 1: 16×8 → 4×2 blocks
        for i in 0..8u16 {
            data.extend_from_slice(&(100 + i).to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
        }
        let file = write_dds(w, h, 2, DdsFormat::Dxt1, &data);
        let img = DdsImage::parse(&file).unwrap();
        let image = atlas_array_image(&img, true).unwrap();
        assert_eq!(image.texture_descriptor.mip_level_count, 1);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 32);
        assert_eq!(image.texture_descriptor.size.width, 4);
        let d = image.data.as_ref().unwrap();
        assert_eq!(d.len(), 32 * 8);
        // layer k = tile (row k/8, col k%8) = block (col, row) → c0 = col + row*8 = k
        for k in 0..32u16 {
            let c0 = u16::from_le_bytes([d[k as usize * 8], d[k as usize * 8 + 1]]);
            assert_eq!(c0, k);
        }
    }

    /// The shader cannot run here (no GPU), so at least parse and validate it with naga:
    /// Bevy's imports are replaced by stubs of the pieces the shader touches, the
    /// prepass branch is dropped.
    #[test]
    fn splat_shader_validates() {
        let src = include_str!("terrain_splat.wgsl");
        let mut body = String::new();
        let mut skip = 0;
        let mut in_prepass = false;
        for line in src.lines() {
            let t = line.trim();
            if t.starts_with("#import") {
                skip = if t.ends_with('{') { 1 } else { 0 };
                continue;
            }
            if skip > 0 {
                if t == "}" {
                    skip = 0;
                }
                continue;
            }
            match t {
                "#ifdef PREPASS_PIPELINE" => {
                    in_prepass = true;
                    continue;
                }
                "#ifndef PREPASS_PIPELINE" => continue,
                "#else" => {
                    in_prepass = false;
                    continue;
                }
                "#endif" => continue,
                _ => {}
            }
            if in_prepass {
                continue;
            }
            // Bevy's module paths are flat in the stubs
            let line = line
                .replace("#{MATERIAL_BIND_GROUP}", "2")
                .replace("view_bindings::", "")
                .replace("mesh_view_types::", "")
                .replace("lighting::", "")
                .replace("shadows::", "");
            body.push_str(&line);
            body.push('\n');
        }
        let stubs = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
}
struct FragmentOutput { @location(0) color: vec4<f32> }
struct View {
    world_position: vec3<f32>,
    clip_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    exposure: f32,
}
@group(0) @binding(0) var<uniform> view: View;
struct DirectionalLight { direction_to_light: vec3<f32>, flags: u32 }
struct Lights { n_directional_lights: u32, directional_lights: array<DirectionalLight, 4> }
@group(0) @binding(1) var<uniform> lights: Lights;
const DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT: u32 = 1u;
const MESH_FLAGS_SHADOW_RECEIVER_BIT: u32 = 536870912u;
const LAYER_BASE: u32 = 0u;
struct Layer {
    NdotV: f32, N: vec3<f32>, R: vec3<f32>, perceptual_roughness: f32, roughness: f32,
}
struct LightingInput {
    layers: array<Layer, 1>,
    P: vec3<f32>, V: vec3<f32>, diffuse_color: vec3<f32>, metallic: f32,
    F0_dielectric: vec3<f32>, F0_metallic: vec3<f32>, F_ab: vec2<f32>,
}
fn perceptualRoughnessToRoughness(r: f32) -> f32 { return r * r; }
fn F_AB(r: f32, n: f32) -> vec2<f32> { return vec2<f32>(r, n); }
fn directional_light(i: u32, input: ptr<function, LightingInput>, d: bool) -> vec3<f32> {
    return (*input).diffuse_color;
}
fn fetch_directional_shadow(i: u32, p: vec4<f32>, n: vec3<f32>, z: f32, xy: vec2<f32>) -> f32 {
    return 1.0;
}
fn calculate_diffuse_color(c: vec3<f32>, m: f32, st: f32, dt: f32) -> vec3<f32> { return c; }
fn calculate_F0_dielectric(r: vec3<f32>) -> vec3<f32> { return r; }
struct Mat {
    base_color: vec4<f32>, perceptual_roughness: f32, metallic: f32,
    specular_transmission: f32, diffuse_transmission: f32, reflectance: vec3<f32>,
}
struct PbrInput {
    material: Mat, N: vec3<f32>, V: vec3<f32>, diffuse_occlusion: vec3<f32>, flags: u32,
    frag_coord: vec4<f32>, world_normal: vec3<f32>, world_position: vec4<f32>,
}
fn pbr_input_from_standard_material(in: VertexOutput, f: bool) -> PbrInput {
    var p: PbrInput; p.material.base_color = vec4<f32>(1.0); p.N = in.world_normal; return p;
}
fn alpha_discard(m: Mat, c: vec4<f32>) -> vec4<f32> { return c; }
fn apply_pbr_lighting(p: PbrInput) -> vec4<f32> { return p.material.base_color; }
fn main_pass_post_lighting_processing(p: PbrInput, c: vec4<f32>) -> vec4<f32> { return c; }
"#;
        // the shared module, minus its import path
        let shared: String = include_str!("sun_horizon.wgsl")
            .lines()
            .filter(|l| !l.starts_with("#define_import_path"))
            .collect::<Vec<_>>()
            .join("\n");
        let full = format!("{stubs}\n{shared}\n{body}");
        let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| {
            panic!("{}", e.emit_to_string(&full));
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        if let Err(e) = validator.validate(&module) {
            panic!("{}", e.emit_to_string(&full));
        }
    }

    /// Golden boundaries of the game's LOD blend (FUN_00a60d30 / FUN_00a61220 / the
    /// `fastter_sh*` vertex shader) at its own 65° camera and at Bevy's 45° default.
    #[test]
    fn lod_blend_matches_the_native_boundaries() {
        let k = lod_scale(NATIVE_FOV);
        assert!((k - 0.777_003).abs() < 1e-5, "{k}");
        let (r0, r1) = (LOD0_RADIUS * k, LOD1_RADIUS * k);
        assert!((r0 - 19.425).abs() < 1e-3 && (r1 - 194.251).abs() < 1e-2);
        let at = |d: f32| lod_blend(Vec3::new(d, 0.0, 0.0), NATIVE_FOV);
        assert_eq!(at(0.0), (1.0, 1.0));
        assert_eq!(at(r0 - 0.01).0, 1.0);
        assert!((at(r0 + 12.5).0 - 0.5).abs() < 1e-4);
        assert_eq!(at(r0 + LOD0_WIDTH + 0.01).0, 0.0);
        assert_eq!(at(r1 - 0.01).1, 1.0);
        assert!((at(r1 + 25.0).1 - 0.5).abs() < 1e-4);
        assert_eq!(at(r1 + LOD1_WIDTH + 0.01), (0.0, 0.0));
        // Chebyshev, not Euclidean: (30, 5, -30) is 30 m away, height counts too
        let (t0, _) = lod_blend(Vec3::new(30.0, 5.0, -30.0), NATIVE_FOV);
        assert!((t0 - (1.0 - (30.0 - r0) / 25.0)).abs() < 1e-5);
        let (t0, _) = lod_blend(Vec3::new(0.0, 30.0, 0.0), NATIVE_FOV);
        assert!(t0 < 0.6);
        // Bevy's default camera (45°): 40.5 / 65.5 m and 405 / 455 m
        let k45 = lod_scale(std::f32::consts::FRAC_PI_4);
        assert!((LOD0_RADIUS * k45 - 40.528).abs() < 1e-2);
        assert!((LOD1_RADIUS * k45 - 405.28).abs() < 1e-1);
        // the scale's clamp
        assert_eq!(lod_scale(1e-4), LOD_SCALE_MAX);
    }

    #[test]
    fn native_mips_and_displacement_inset() {
        // 1 m per pixel in x and in y: D = 0.5 log2(2) = 0.5
        let (disp, near, far) = native_mips(2.0);
        assert!((disp - 1.0).abs() < 1e-6 && (near - 6.0).abs() < 1e-6);
        assert!((far - 3.3).abs() < 1e-6);
        // 1 cm per pixel: everything at mip 0
        assert_eq!(native_mips(2e-4), (0.0, 0.0, 0.0));
        // far away: everything clamped at 6
        assert_eq!(native_mips(1e6), (6.0, 6.0, 6.0));
        // tile 2 at its corner, mip 0: one texel in
        let (u, v) = displace_uv(2, 0.0, 0.0, 0.0);
        assert!((u - (0.5 + 1.0 / 4096.0)).abs() < 1e-7 && (v - 1.0 / 1024.0).abs() < 1e-7);
        // the far corner stays one texel short of the next tile; negative x wraps
        let (u, _) = displace_uv(2, -1e-3, 0.0, 0.0);
        assert!(u < 0.75 - 1.0 / 4096.0 + 1e-6 && u > 0.749);
        // at mip 3 the inset is 8 texels of mip 0
        let (u, v) = displace_uv(0, 0.0, 0.0, 3.0);
        assert!((u - 8.0 / 4096.0).abs() < 1e-7 && (v - 8.0 / 1024.0).abs() < 1e-7);
    }

    /// The shader carries the same native constants and none of the old policy values.
    #[test]
    fn shader_mirrors_the_native_constants() {
        let src = include_str!("terrain_splat.wgsl");
        for line in [
            format!("const LOD0_RADIUS: f32 = {LOD0_RADIUS:?};"),
            format!("const LOD0_WIDTH: f32 = {LOD0_WIDTH:?};"),
            format!("const LOD1_RADIUS: f32 = {LOD1_RADIUS:?};"),
            format!("const LOD1_WIDTH: f32 = {LOD1_WIDTH:?};"),
            format!("const LOD_SCALE_MAX: f32 = {LOD_SCALE_MAX:?};"),
        ] {
            assert!(src.contains(&line), "{line}");
        }
        for mip in ["d_mip + 0.5", "d_mip + 5.5", "d_mip + 2.8", "1.5 * f3 * dv"] {
            assert!(src.contains(mip), "{mip}");
        }
        for old in ["DETAIL_LIGHT", "FAR_START", "DISP_FADE", "in.world_normal"] {
            assert!(!src.contains(old), "{old}");
        }
    }

    #[test]
    fn colours_pack_as_srgb_bytes() {
        assert_eq!(pack_colour(Some([1.0, 0.5, 0.0])), 0xff00_80ff);
        assert_eq!(pack_colour(None), 0xffff_00ff);
    }

    /// Original fixtures, one tile per climate (0 Jungle, 1 Arctic, 2 DesertCity): the
    /// gradient texture is the file's A8L8 layer byte for byte with its chain, decodes to
    /// the height field's central difference, and the colour table has the
    /// `globalcolorpixelmap` texel of every slot the tile uses.
    #[test]
    fn original_climate_fixtures() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = jc2_formats::GameFs::open(dir) else {
            return;
        };
        let Ok(land) = Landscape::load(&fs) else {
            return;
        };
        let Some(zones) = land.zones.as_ref() else {
            return;
        };
        for climate in 0..3u8 {
            let found = (0..64u32 * 64).find_map(|i| {
                let (tx, tz) = (i % 64, i / 64);
                if zones.climate_zone(tx, tz) != climate {
                    return None;
                }
                let bytes = fs
                    .read_inflated(&jc2_formats::terrain::tile_file_name(tx, tz))
                    .ok()?;
                let tile = TerrainTile::parse(&bytes).ok()?;
                // a land tile (centre above the 200 m sea) with layers and some relief
                let relief = (0..TILE_CELLS)
                    .map(|c| tile.cell_gradient(64, c).0.abs())
                    .fold(0.0f32, f32::max);
                (tile.has_layers() && relief > 0.3 && tile.cell_height(64, 64) > 220.0)
                    .then_some((tx, tz, tile))
            });
            let (tx, tz, tile) = found.unwrap_or_else(|| panic!("no tile of climate {climate}"));
            let image = gradient_image(&tile);
            assert_eq!(image.texture_descriptor.format, TextureFormat::Rg8Unorm);
            assert_eq!(image.texture_descriptor.mip_level_count, 8, "{tx},{tz}");
            let data = image.data.as_ref().unwrap();
            let file = tile.normal_map().unwrap().level(0).unwrap();
            assert_eq!(&data[..file.len()], file);
            // decoded as the shader does, x SLOPE_UNIT, against the heights
            let mut err = 0.0f64;
            for cz in 2..TILE_CELLS - 2 {
                for cx in 2..TILE_CELLS - 2 {
                    let i = (cz * TILE_CELLS + cx) * 2;
                    let g = |b: u8| (b as f32 / 255.0 * 2.0 - 1.0) * SLOPE_UNIT;
                    let (hx, hz) = tile.cell_gradient(cz, cx);
                    err += ((g(data[i]) - hx).abs() + (g(data[i + 1]) - hz).abs()) as f64;
                }
            }
            let mean = err / (2.0 * ((TILE_CELLS - 4) * (TILE_CELLS - 4)) as f64);
            assert!(mean < 0.05, "climate {climate} tile {tx},{tz}: {mean}");
            // every slot the tile shows has its table colour, not the magenta stand-in
            let zone = land.texture_zone(tx, tz);
            let table = colour_table(&land, zone);
            let flat: Vec<u32> = table.iter().flat_map(|v| v.to_array()).collect();
            for cz in (0..TILE_CELLS).step_by(8) {
                for cx in (0..TILE_CELLS).step_by(8) {
                    let slot = tile.meta_cell(cz, cx).dominant() as usize;
                    for band in 0..SLOPE_SLOTS {
                        let c = land.slot_colour(zone, slot as u8, band);
                        assert!(c.is_some(), "climate {climate}: slot {slot} band {band}");
                        assert_eq!(flat[slot * SLOPE_SLOTS + band], pack_colour(c));
                    }
                }
            }
            eprintln!("climate {climate}: tile {tx},{tz} zone {zone}, gradient error {mean:.4}");
        }
    }

    #[test]
    fn table_entries_pack_the_displacement_data() {
        use jc2_formats::landscape::SlopeSlot;
        let slot = SlopeSlot {
            displace_tile: 3,
            texture: 27,
            band_bump: 0.4,
            light_bump: 0.6,
        };
        let e = pack_entry(&slot);
        assert_eq!(e & 0xff, 27);
        assert_eq!((e >> 8) & 3, 3);
        assert_eq!(e & 0x1_0000, 0, "not flagged undefined");
        assert_eq!((e >> 17) & 0xff, 102);
        assert_eq!((e >> 25) & 0x7f, 120);
    }

    /// The real atlas: 4096 × 1024 DXT5 with its whole chain becomes one BC3 texture.
    #[test]
    fn the_displacement_atlas_loads() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = jc2_formats::GameFs::open(dir) else {
            return;
        };
        let Ok(bytes) = fs.read_inflated(DISPLACE_ATLAS) else {
            return;
        };
        let img = DdsImage::parse(&bytes).unwrap();
        let image = displace_image(&img).unwrap();
        assert_eq!(image.texture_descriptor.size.width, 4096);
        assert_eq!(image.texture_descriptor.size.height, 1024);
        assert_eq!(image.texture_descriptor.mip_level_count, 13);
        assert_eq!(image.texture_descriptor.format, TextureFormat::Bc3RgbaUnorm);
        assert!(image.data.as_ref().unwrap().len() > 5_000_000);
    }

    #[test]
    fn layer_images_carry_slots_and_weights() {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        d.extend(std::iter::repeat_n(0u8, 132 * 132 * 3));
        let cells = 128 * 128;
        let normals = write_dds(128, 128, 1, DdsFormat::A8L8, &vec![128u8; cells * 2]);
        let slots = write_dds(
            128,
            128,
            1,
            DdsFormat::Argb4444,
            &0x1234u16.to_le_bytes().repeat(cells),
        );
        let weights = write_dds(
            128,
            128,
            1,
            DdsFormat::Argb4444,
            &0x0f00u16.to_le_bytes().repeat(cells),
        );
        d.extend_from_slice(&12u32.to_le_bytes());
        for t in [&normals, &slots, &weights] {
            d.extend_from_slice(&(t.len() as u32).to_le_bytes());
            d.extend_from_slice(t);
        }
        let tile = TerrainTile::parse(&d).unwrap();
        let (s, w) = layer_images(&tile);
        assert_eq!(s.texture_descriptor.format, TextureFormat::Rgba8Uint);
        assert_eq!(&s.data.as_ref().unwrap()[..4], &[1, 2, 3, 4]);
        assert_eq!(&w.data.as_ref().unwrap()[..4], &[0, 255, 0, 0]);
    }
}

/// The asset stores a tile's splat material needs, bundled so `world.rs` passes one
/// parameter around.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TerrainAssets<'w> {
    pub images: ResMut<'w, Assets<Image>>,
    pub materials: ResMut<'w, Assets<TerrainSplatMaterial>>,
}
