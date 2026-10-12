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
//! **Global mode** (JC2-157): the horizon chunks beyond the streamed tiles use the same
//! material with the shader def `TERRAIN_GLOBAL` ([`SplatContext::global_material`]): the
//! game's `FastTerrainShader_Global` is the Lod2 maths on the global layers
//! (`globalmetamaterials` slots, `globalmetaweights`, `globalslopes`, `texturezonemap`), see
//! `research/formats/terrain-terms-next.md`.
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
use jc2_formats::landscape::ZoneMaps;
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
/// The graphics-engine camera object's constructor FOV (`FUN_00924d80`, `0x3f91361e`, 65°).
/// It is **not** a gameplay FOV: the camera manager overwrites it on its first frame
/// (JC2-141; on foot the game runs 40°, see `gameplay_camera.rs`). Kept as a fixture value
/// for the LOD-scale tests only.
#[cfg(test)]
pub const ENGINE_CAMERA_CTOR_FOV: f32 = 1.134_464;

/// The FOV scale of the LOD radii: `min(1 / fov², 999.99994)`, `fov` the camera's
/// vertical field of view in radians (`FUN_00a61220`, camera `+0x618`).
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of terrain_splat.wgsl, checked by the tests
pub fn lod_scale(fov: f32) -> f32 {
    (1.0 / (fov * fov)).min(LOD_SCALE_MAX)
}

/// The terrain LOD scale of the camera as it is this frame (JC2-148): the CPU view of the
/// quantity `terrain_splat.wgsl` derives from the view's projection
/// (`fov = 2 atan(1 / clip_from_view[1][1])`, the same FOV [`crate::gameplay_camera::CameraFov`]
/// holds). On foot (40 degrees) `scale` = 2.0517542, LOD 0 ends at 51.29 m and LOD 1 at
/// 512.9 m; aim (20 / 15 degrees) pushes them out to 212 / 364.8 m and 2124 / 3648 m.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TerrainLod {
    /// The realised vertical FOV in radians.
    pub fov: f32,
    /// `min(1 / fov^2, 999.99994)`.
    pub scale: f32,
}

impl TerrainLod {
    pub fn at(fov: f32) -> Self {
        Self {
            fov,
            scale: lod_scale(fov),
        }
    }

    /// Chebyshev radius (m) of pure LOD 0.
    pub fn lod0_radius(&self) -> f32 {
        LOD0_RADIUS * self.scale
    }

    /// Chebyshev radius (m) of pure LOD 1.
    pub fn lod1_radius(&self) -> f32 {
        LOD1_RADIUS * self.scale
    }
}

impl Default for TerrainLod {
    fn default() -> Self {
        Self::at(std::f32::consts::FRAC_PI_4)
    }
}

/// Follow the realised camera FOV (after the gameplay camera system, like the vegetation
/// fade): the shader reads the same FOV from the projection the camera renders with.
pub(crate) fn update_terrain_lod(
    cams: Query<&Projection, With<Camera3d>>,
    camera_fov: Option<Res<crate::gameplay_camera::CameraFov>>,
    mut lod: ResMut<TerrainLod>,
) {
    if let Some(fov) =
        crate::gameplay_camera::realised_fov(camera_fov.as_deref(), cams.iter().next())
        && fov != lod.fov
    {
        *lod = TerrainLod::at(fov);
    }
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
    pub table: [UVec4; TABLE_VEC4],
    pub colours: [UVec4; TABLE_VEC4],
}

/// Texture zones the tables carry (Jungle / Arctic use zone 0, DesertCity zone 1, JC2-101:
/// `Landscape::texture_zone_count`); a tile fills the first half of the uniform tables with
/// its own zone, the global mode fills both.
pub const TABLE_ZONES: usize = 2;
/// Entries in one zone's slot x band table.
const ZONE_ENTRIES: usize = META_SLOTS * SLOPE_SLOTS;
/// `vec4`s in the uniform tables.
pub const TABLE_VEC4: usize = ZONE_ENTRIES * TABLE_ZONES / 4;
/// `vec4`s in one zone's table.
const ZONE_VEC4: usize = ZONE_ENTRIES / 4;

/// The uniform's tables: `first` in the first half, `second` (or zeros) in the second.
fn join_zones(
    first: [UVec4; ZONE_VEC4],
    second: Option<[UVec4; ZONE_VEC4]>,
) -> [UVec4; TABLE_VEC4] {
    let mut out = [UVec4::ZERO; TABLE_VEC4];
    out[..ZONE_VEC4].copy_from_slice(&first);
    if let Some(second) = second {
        out[ZONE_VEC4..].copy_from_slice(&second);
    }
    out
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(TerrainSplatKey)]
pub struct TerrainSplat {
    /// The global layers (horizon chunks) instead of a tile's: selects the
    /// `TERRAIN_GLOBAL` shader def, see [`TerrainSplatKey`].
    pub global: bool,
    #[uniform(100)]
    pub params: SplatParams,
    /// The zone's diffuse atlas as 32 layers.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub atlas: Handle<Image>,
    /// 128 × 128 `Rgba8Uint`: the four slots of every cell (global mode: 1024 × 1024, the
    /// `globalmetamaterials` nibbles R, G, B, A = slot 0..3).
    #[texture(103, sample_type = "u_int")]
    pub slots: Handle<Image>,
    /// 128 × 128 `Rgba8Unorm`: their weights (global mode: the BC1 `globalmetaweights`).
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
    /// the game's shader reads (`t5`, `x 2 - 1`); global mode: the BC1 `globalslopes`.
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
    /// 64 × 64 `R8Uint`: the texture zone of every tile (`texturezonemap.dds`, point); only
    /// the global mode reads it.
    #[texture(114, sample_type = "u_int")]
    pub zones: Handle<Image>,
    /// The frame's selected point lights and the 4 m cell grid (`point_lights.rs`), shared.
    #[storage(120, read_only)]
    pub point_lights: Handle<ShaderBuffer>,
    #[storage(121, read_only)]
    pub point_cells: Handle<ShaderBuffer>,
}

/// The pipeline key of [`TerrainSplat`]: tile or global mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerrainSplatKey {
    pub global: bool,
}

impl From<&TerrainSplat> for TerrainSplatKey {
    fn from(m: &TerrainSplat) -> Self {
        Self { global: m.global }
    }
}

/// The splat material in global mode, shared by every horizon chunk (inserted by
/// `world::setup` when the data has the global layers).
#[derive(Resource, Clone)]
pub struct GlobalTerrainMaterial(pub Handle<TerrainSplatMaterial>);

/// The shader def of the global mode.
pub const TERRAIN_GLOBAL_DEF: &str = "TERRAIN_GLOBAL";

impl MaterialExtension for TerrainSplat {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("terrain_splat.wgsl")).with_source("embedded"),
        )
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if key.bind_group_data.global
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            fragment.shader_defs.push(TERRAIN_GLOBAL_DEF.into());
        }
        Ok(())
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
        crate::haze::ensure_shader(app);
        crate::point_lights::ensure_shader(app);
        app.insert_resource(TerrainShader(shader));
        app.insert_resource(SunHorizonShader(horizon));
        app.add_plugins(MaterialPlugin::<TerrainSplatMaterial>::default());
        app.init_resource::<TerrainLod>().add_systems(
            Update,
            (
                update_horizon,
                update_terrain_lod.after(crate::gameplay_camera::drive_fov),
            ),
        );
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
    /// The texture zone map as a GPU texture (bound by every material; the global mode
    /// reads it).
    pub zones: Handle<Image>,
    /// The global layers of the horizon chunks, when all of them are in the data.
    pub global: Option<GlobalLayers>,
}

/// The game's global material layers as GPU textures (JC2-157): what the far terrain
/// shader `FastTerrainShader_Global` reads (`terrain-terms-next.md`).
#[derive(Clone, Debug)]
pub struct GlobalLayers {
    /// `globalmetamaterials.dds`, repacked by [`global_meta_image`].
    pub slots: Handle<Image>,
    /// `globalmetaweights.dds`, BC1, linear filter.
    pub weights: Handle<Image>,
    /// `globalslopes.dds`, BC1, linear filter.
    pub gradient: Handle<Image>,
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
        let zones = images.add(zone_image(land.zones.as_ref()));
        let global = load_global_layers(game, images);
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
            zones,
            global,
        })
    }

    /// The colour table of texture zone `zone` (`MetaMaterialColors`: per slot x band the
    /// `globalcolorpixelmap` texel of the entry's atlas texture) packed as sRGB RGBA8.
    pub fn colours(&self, zone: u8) -> [UVec4; ZONE_VEC4] {
        colour_table(&self.land, zone)
    }

    /// The slope table of texture zone `zone` packed for the uniform.
    fn table(&self, zone: u8) -> [UVec4; ZONE_VEC4] {
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
                // the native fog is applied in `terrain_splat.wgsl` (`haze_fog.wgsl`)
                fog_enabled: false,
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
                    table: join_zones(self.table(zone), None),
                    colours: join_zones(self.colours(zone), None),
                },
                global: false,
                zones: self.zones.clone(),
                atlas,
                slots: images.add(slots),
                weights: images.add(weights),
                normals,
                displace: self.displace.clone().unwrap_or_else(|| self.blank.clone()),
                gradient: images.add(gradient_image(tile)),
                ao: self.horizon.ao.clone(),
                sun_horizon: self.horizon.params.clone(),
                point_lights: crate::point_lights::LIGHT_BUFFER,
                point_cells: crate::point_lights::CELL_BUFFER,
            },
        };
        Some(materials.add(material))
    }

    /// The material of the horizon chunks (JC2-157): the splat shader in global mode on the
    /// game's global layers, with both texture zones' tables. `None` when a global layer
    /// or every diffuse atlas is missing from the data (the atlas array is bound but never
    /// sampled: the global shader runs the colour-table branch only).
    pub fn global_material(
        &self,
        materials: &mut Assets<TerrainSplatMaterial>,
    ) -> Option<Handle<TerrainSplatMaterial>> {
        let layers = self.global.as_ref()?;
        let atlas = self.atlases.iter().flatten().next()?.clone();
        let both = |f: &dyn Fn(u8) -> [UVec4; ZONE_VEC4]| join_zones(f(0), Some(f(1)));
        let material = TerrainSplatMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 1.0,
                metallic: 0.0,
                reflectance: 0.1,
                // the native fog is applied in `terrain_splat.wgsl` (`haze_fog.wgsl`)
                fog_enabled: false,
                ..default()
            },
            extension: TerrainSplat {
                global: true,
                params: SplatParams {
                    origin: Vec4::new(0.0, 0.0, ATLAS_REPEAT_NEAR, ATLAS_REPEAT_FAR),
                    slope: Vec4::new(
                        SLOPE_UNIT,
                        jc2_formats::landscape::SLOPE_BANDS,
                        0.0,
                        if self.displace.is_some() { 1.0 } else { 0.0 },
                    ),
                    table: both(&|z| self.table(z)),
                    colours: both(&|z| self.colours(z)),
                },
                normals: atlas.clone(),
                atlas,
                slots: layers.slots.clone(),
                weights: layers.weights.clone(),
                displace: self.displace.clone().unwrap_or_else(|| self.blank.clone()),
                gradient: layers.gradient.clone(),
                ao: self.horizon.ao.clone(),
                sun_horizon: self.horizon.params.clone(),
                point_lights: crate::point_lights::LIGHT_BUFFER,
                point_cells: crate::point_lights::CELL_BUFFER,
                zones: self.zones.clone(),
            },
        };
        Some(materials.add(material))
    }
}

/// The path of a global layer in the archives.
fn global_layer_name(name: &str) -> String {
    format!("textures\\landscape\\{name}.dds")
}

/// Read and convert the global layers; `None` (with a warning) when one is missing.
fn load_global_layers(game: &Game, images: &mut Assets<Image>) -> Option<GlobalLayers> {
    let read = |name: &str| -> Option<DdsImage> {
        let path = global_layer_name(name);
        match jc2_formats::landscape::read_landscape_file(&game.fs, &path)
            .map_err(|e| e.to_string())
            .and_then(|d| DdsImage::parse(&d).map_err(|e| e.to_string()))
        {
            Ok(i) => Some(i),
            Err(e) => {
                warn!("{path}: {e}; no global terrain material");
                None
            }
        }
    };
    let slots = read("globalmetamaterials")?;
    let weights = read("globalmetaweights")?;
    let gradient = read("globalslopes")?;
    let mut convert = |r: Result<Image, String>, name: &str| match r {
        Ok(i) => Some(images.add(i)),
        Err(e) => {
            warn!("{name}: {e}; no global terrain material");
            None
        }
    };
    Some(GlobalLayers {
        slots: convert(global_meta_image(&slots), "globalmetamaterials")?,
        weights: convert(global_bc1_image(&weights), "globalmetaweights")?,
        gradient: convert(global_bc1_image(&gradient), "globalslopes")?,
    })
}

/// `globalmetamaterials.dds` (1024 x 1024 `A4R4G4B4`) as the slot texture the shader loads:
/// `Rgba8Uint`, the nibbles R, G, B, A in channels x, y, z, w = slots 0..3 (the shader's
/// `round(sample * 15)` of the game's UNORM sample, `terrain-terms-next.md`).
pub fn global_meta_image(img: &DdsImage) -> Result<Image, String> {
    if img.format != DdsFormat::Argb4444 {
        return Err(format!("expected A4R4G4B4, got {:?}", img.format));
    }
    let texels = img.texels_u16(0).ok_or("level 0 is short")?;
    if texels.len() != (img.width * img.height) as usize {
        return Err("level 0 is short".into());
    }
    let data = texels
        .iter()
        .flat_map(|&v| global_slots(v))
        .collect::<Vec<u8>>();
    Ok(Image::new(
        Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Uint,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// The four slots of one `A4R4G4B4` texel of `globalmetamaterials`: R, G, B, A nibble =
/// slot 0..3 (the global shader reads the sample's .xyzw as the slots; the tile layers use
/// the order A, R, G, B, [`MetaCell::unpack`]).
pub fn global_slots(texel: u16) -> [u8; 4] {
    let nib = |shift: u32| ((texel >> shift) & 0xf) as u8;
    [nib(8), nib(4), nib(0), nib(12)]
}

/// A 1024 x 1024 BC1 global layer (`globalmetaweights`, `globalslopes`) with its mip chain
/// (the retail files have none), linear filter, clamped (`s0` / `s3` of the game).
pub fn global_bc1_image(img: &DdsImage) -> Result<Image, String> {
    if img.format != DdsFormat::Dxt1 {
        return Err(format!("expected DXT1, got {:?}", img.format));
    }
    let bytes = img.chain_bytes().ok_or("mip chain is short")?;
    let mut image = Image::new_uninit(
        Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bc1RgbaUnorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = img.mips.max(1);
    image.data = Some(img.data[..bytes].to_vec());
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    Ok(image)
}

/// The texture zone of every tile as a 64 x 64 `R8Uint` texture (all zero without the zone
/// map, the same fallback as [`Landscape::texture_zone`]).
pub fn zone_image(zones: Option<&ZoneMaps>) -> Image {
    let n = jc2_formats::terrain::TILES_PER_SIDE;
    let data = zones.map_or_else(|| vec![0u8; (n * n) as usize], |z| z.texture.clone());
    Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R8Uint,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn pack_flat(flat: &[u32; META_SLOTS * SLOPE_SLOTS]) -> [UVec4; ZONE_VEC4] {
    let mut out = [UVec4::ZERO; META_SLOTS * SLOPE_SLOTS / 4];
    for (v, c) in out.iter_mut().zip(flat.as_chunks::<4>().0) {
        *v = UVec4::from_array(*c);
    }
    out
}

/// `MetaMaterialColors` of texture zone `zone` as the uniform carries it: entry
/// `slot * 16 + band` is the `globalcolorpixelmap` texel of that entry's atlas texture
/// (`FUN_00a70fb0`; [`Landscape::slot_colour`]), packed by [`pack_colour`].
pub fn colour_table(land: &Landscape, zone: u8) -> [UVec4; ZONE_VEC4] {
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

    /// Preprocess `terrain_splat.wgsl` the way Bevy does for the shader defs `defs`
    /// (`#ifdef` / `#ifndef` / `#else` / `#endif`, nested), dropping the `#import` lines
    /// and flattening Bevy's module paths into the test stubs.
    fn preprocess_splat(src: &str, defs: &[&str]) -> String {
        let mut body = String::new();
        let mut skip = false;
        // one entry per open conditional: (this branch is live, the enclosing region is live)
        let mut stack: Vec<(bool, bool)> = Vec::new();
        let live = |stack: &[(bool, bool)]| stack.last().is_none_or(|&(on, _)| on);
        for line in src.lines() {
            let t = line.trim();
            if t.starts_with("#import") {
                skip = t.ends_with('{');
                continue;
            }
            if skip {
                skip = t != "}";
                continue;
            }
            if let Some(name) = t.strip_prefix("#ifdef ") {
                let outer = live(&stack);
                stack.push((outer && defs.contains(&name.trim()), outer));
                continue;
            }
            if let Some(name) = t.strip_prefix("#ifndef ") {
                let outer = live(&stack);
                stack.push((outer && !defs.contains(&name.trim()), outer));
                continue;
            }
            if t == "#else" {
                let (on, outer) = stack.pop().expect("#else without #ifdef");
                stack.push((outer && !on, outer));
                continue;
            }
            if t == "#endif" {
                stack.pop().expect("#endif without #ifdef");
                continue;
            }
            if !live(&stack) {
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
        assert!(stack.is_empty(), "unbalanced #ifdef");
        body
    }

    /// The shader cannot run here (no GPU), so at least parse and validate it with naga:
    /// Bevy's imports are replaced by stubs of the pieces the shader touches, the
    /// prepass branch is dropped.
    #[test]
    fn splat_shader_validates() {
        validate_splat_shader(&[]);
    }

    /// JC2-157: the global mode compiles with the shader def the specialisation pushes.
    #[test]
    fn splat_shader_validates_in_global_mode() {
        validate_splat_shader(&[TERRAIN_GLOBAL_DEF]);
    }

    fn validate_splat_shader(defs: &[&str]) {
        let body = preprocess_splat(include_str!("terrain_splat.wgsl"), defs);
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
        // the shared modules, minus their import paths
        let shared: String = [
            include_str!("sun_horizon.wgsl"),
            include_str!("haze_fog.wgsl"),
            include_str!("point_lights.wgsl"),
        ]
        .iter()
        .flat_map(|m| m.lines())
        .filter(|l| !l.starts_with("#define_import_path"))
        .map(|l| l.replace("#{MATERIAL_BIND_GROUP}", "2"))
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
        let k = lod_scale(ENGINE_CAMERA_CTOR_FOV);
        assert!((k - 0.777_003).abs() < 1e-5, "{k}");
        let (r0, r1) = (LOD0_RADIUS * k, LOD1_RADIUS * k);
        assert!((r0 - 19.425).abs() < 1e-3 && (r1 - 194.251).abs() < 1e-2);
        let at = |d: f32| lod_blend(Vec3::new(d, 0.0, 0.0), ENGINE_CAMERA_CTOR_FOV);
        assert_eq!(at(0.0), (1.0, 1.0));
        assert_eq!(at(r0 - 0.01).0, 1.0);
        assert!((at(r0 + 12.5).0 - 0.5).abs() < 1e-4);
        assert_eq!(at(r0 + LOD0_WIDTH + 0.01).0, 0.0);
        assert_eq!(at(r1 - 0.01).1, 1.0);
        assert!((at(r1 + 25.0).1 - 0.5).abs() < 1e-4);
        assert_eq!(at(r1 + LOD1_WIDTH + 0.01), (0.0, 0.0));
        // Chebyshev, not Euclidean: (30, 5, -30) is 30 m away, height counts too
        let (t0, _) = lod_blend(Vec3::new(30.0, 5.0, -30.0), ENGINE_CAMERA_CTOR_FOV);
        assert!((t0 - (1.0 - (30.0 - r0) / 25.0)).abs() < 1e-5);
        let (t0, _) = lod_blend(Vec3::new(0.0, 30.0, 0.0), ENGINE_CAMERA_CTOR_FOV);
        assert!(t0 < 0.6);
        // Bevy's default camera (45°): 40.5 / 65.5 m and 405 / 455 m
        let k45 = lod_scale(std::f32::consts::FRAC_PI_4);
        assert!((LOD0_RADIUS * k45 - 40.528).abs() < 1e-2);
        assert!((LOD1_RADIUS * k45 - 405.28).abs() < 1e-1);
        // the scale's clamp
        assert_eq!(lod_scale(1e-4), LOD_SCALE_MAX);
    }

    /// JC2-148: the radii at the realised gameplay FOVs (`f32(deg * 0.017453292)`, the
    /// JC2-141 probe's `terrain_lod` fixtures): on foot 40 degrees, aim 20 and 15, vehicles
    /// 50, the free camera / parachute 45, and the graphics-engine constructor's 65.
    #[test]
    fn terrain_radii_follow_each_mode_fov() {
        let rad = |deg: f32| deg * 0.017_453_292;
        // (degrees, scale, LOD0 radius, LOD1 radius)
        for (deg, k, r0, r1) in [
            (40.0, 2.051_754_2, 51.29, 512.9),
            (20.0, 8.207_02, 205.18, 2051.75),
            (15.0, 14.590_3, 364.8, 3647.6),
            (50.0, 1.313_15, 32.83, 328.3),
            (45.0, 1.621_139, 40.53, 405.28),
            (65.0, 0.777_003, 19.43, 194.25),
        ] {
            let t = TerrainLod::at(rad(deg));
            assert!((t.scale - k).abs() < 2e-4 * k, "{deg}: {}", t.scale);
            assert!((t.lod0_radius() - r0).abs() < 0.02 * r0.max(10.0), "{deg}");
            assert!((t.lod1_radius() - r1).abs() < 0.02 * r1.max(10.0), "{deg}");
        }
        // the quantity the shader derives from the camera matrix is the same FOV
        for deg in [40.0_f32, 20.0, 15.0, 50.0, 45.0, 65.0] {
            let p = PerspectiveProjection {
                fov: rad(deg),
                aspect_ratio: 16.0 / 9.0,
                ..default()
            };
            let m = bevy::camera::CameraProjection::get_clip_from_view(&p);
            let fov = 2.0 * (1.0_f32 / m.y_axis.y).atan();
            assert!(
                (lod_scale(fov) / lod_scale(rad(deg)) - 1.0).abs() < 1e-4,
                "{deg}"
            );
        }
        // no hard cap in range: the clamp only bites below ~1.8 milliradians
        assert_eq!(TerrainLod::at(1e-4).scale, LOD_SCALE_MAX);
    }

    /// The resource follows the camera's projection when no gameplay camera drives
    /// `CameraFov`, and `CameraFov` when it does.
    #[test]
    fn terrain_lod_resource_follows_the_realised_fov() {
        use crate::gameplay_camera::CameraFov;
        let mut app = App::new();
        app.init_resource::<TerrainLod>()
            .add_systems(Update, update_terrain_lod);
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: 0.698_131_7,
                    ..default()
                }),
            ))
            .id();
        app.update();
        let t = *app.world().resource::<TerrainLod>();
        assert!((t.scale - 2.051_754_2).abs() < 1e-5, "{}", t.scale);
        assert!((t.lod0_radius() - 51.29).abs() < 0.01);
        // a new projection FOV is picked up the next frame
        *app.world_mut().get_mut::<Projection>(cam).unwrap() =
            Projection::Perspective(PerspectiveProjection {
                fov: 0.349_065_85,
                ..default()
            });
        app.update();
        assert!((app.world().resource::<TerrainLod>().lod0_radius() - 205.18).abs() < 0.1);
        // with the gameplay camera's resource present, it is the source
        app.insert_resource(CameraFov { fov_y: 0.261_799_4 });
        app.update();
        assert!((app.world().resource::<TerrainLod>().lod0_radius() - 364.8).abs() < 0.3);
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
    fn global_slots_unpack_r_g_b_a_in_that_order() {
        // A4R4G4B4: alpha nibble 15..12, red 11..8, green 7..4, blue 3..0; the global shader
        // reads the sample's .xyzw = R, G, B, A as slots 0..3 (the tile layers use A, R, G, B)
        assert_eq!(global_slots(0xABCD), [0xB, 0xC, 0xD, 0xA]);
        let cell = MetaCell::unpack(0xABCD, 0);
        assert_eq!(cell.slots, [0xA, 0xB, 0xC, 0xD]);
        let dds = write_dds(2, 1, 1, DdsFormat::Argb4444, &[0xCD, 0xAB, 0x21, 0x03]);
        let image = global_meta_image(&DdsImage::parse(&dds).unwrap()).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Uint);
        assert_eq!(
            image.data.as_ref().unwrap(),
            &[0xB, 0xC, 0xD, 0xA, 0x3, 0x2, 0x1, 0x0]
        );
    }

    #[test]
    fn global_bc1_layers_are_linear_clamped_and_zones_point() {
        let block = [0u8; 8];
        let dds = write_dds(8, 8, 1, DdsFormat::Dxt1, &block.repeat(4));
        let image = global_bc1_image(&DdsImage::parse(&dds).unwrap()).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Bc1RgbaUnorm);
        assert_eq!(image.texture_descriptor.mip_level_count, 1);
        let ImageSampler::Descriptor(d) = &image.sampler else {
            panic!("sampler");
        };
        assert_eq!(d.address_mode_u, ImageAddressMode::ClampToEdge);
        assert_eq!(d.mag_filter, ImageFilterMode::Linear);
        assert!(
            global_bc1_image(
                &DdsImage::parse(&write_dds(4, 4, 1, DdsFormat::Dxt5, &[0; 16])).unwrap()
            )
            .is_err()
        );
        // the zone map keeps its texture zone bytes (no sampler: it is loaded by texel)
        let n = jc2_formats::terrain::TILES_PER_SIDE as usize;
        let zones = ZoneMaps {
            climate: vec![0; n * n],
            texture: (0..n * n).map(|i| (i % 2) as u8).collect(),
        };
        let image = zone_image(Some(&zones));
        assert_eq!(image.texture_descriptor.format, TextureFormat::R8Uint);
        assert_eq!(image.data.as_ref().unwrap(), &zones.texture);
        assert!(
            zone_image(None)
                .data
                .as_ref()
                .unwrap()
                .iter()
                .all(|&z| z == 0)
        );
    }

    #[test]
    fn tables_fill_the_first_half_for_a_tile_and_both_for_the_global_mode() {
        let a = [UVec4::splat(1); ZONE_VEC4];
        let b = [UVec4::splat(2); ZONE_VEC4];
        let tile = join_zones(a, None);
        assert_eq!(tile[ZONE_VEC4 - 1], UVec4::splat(1));
        assert_eq!(tile[ZONE_VEC4], UVec4::ZERO);
        let both = join_zones(a, Some(b));
        assert_eq!(
            (both[0], both[ZONE_VEC4], both[TABLE_VEC4 - 1]),
            (a[0], b[0], b[0])
        );
        // the shader's arrays and zone stride are the same size
        let src = include_str!("terrain_splat.wgsl");
        assert!(src.contains(&format!("table: array<vec4<u32>, {TABLE_VEC4}>")));
        assert!(src.contains(&format!("colours: array<vec4<u32>, {TABLE_VEC4}>")));
        assert!(src.contains(&format!("const ZONE_ENTRIES: u32 = {ZONE_ENTRIES}u;")));
        assert_eq!(TABLE_VEC4, 128);
    }

    /// CPU mirror of the global mode of `terrain_splat.wgsl` for the first pass (no
    /// displacement, nearest texels), the maths of the JC2-101 probe's `Native.colour`:
    /// the colour-table (Lod2) colour at world (x, z) and the texture zone used.
    struct GlobalFar {
        land: Landscape,
        /// 1024 x 1024 x 4 slot nibbles as uploaded ([`global_meta_image`]).
        slots: Vec<u8>,
        /// BC1 level 0 as stored (decoded per texel in float, as the GPU does).
        weights: Vec<u8>,
        slopes: Vec<u8>,
    }

    /// RGB (0..1, UNORM: 5/6/5 bits over 31/63/31, no 8-bit rounding) of texel (x, y) of a
    /// 1024-wide BC1 level 0, the decode the GPU filters.
    fn bc1_texel(data: &[u8], x: usize, y: usize) -> [f32; 3] {
        let block = &data[((y / 4) * 256 + x / 4) * 8..][..8];
        let (c0, c1) = (
            u16::from_le_bytes([block[0], block[1]]),
            u16::from_le_bytes([block[2], block[3]]),
        );
        let idx = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        let rgb = |c: u16| {
            [
                (c >> 11) as f32 / 31.0,
                ((c >> 5) & 63) as f32 / 63.0,
                (c & 31) as f32 / 31.0,
            ]
        };
        let (p0, p1) = (rgb(c0), rgb(c1));
        let sel = (idx >> (2 * ((y & 3) * 4 + (x & 3)))) & 3;
        let lerp = |t: f32| [0, 1, 2].map(|k| p0[k] * (1.0 - t) + p1[k] * t);
        match (sel, c0 > c1) {
            (0, _) => p0,
            (1, _) => p1,
            (2, true) => lerp(1.0 / 3.0),
            (3, true) => lerp(2.0 / 3.0),
            (2, false) => lerp(0.5),
            _ => [0.0; 3],
        }
    }

    impl GlobalFar {
        fn load(fs: &jc2_formats::GameFs) -> Option<Self> {
            let land = Landscape::load(fs).ok()?;
            let read = |n: &str| {
                jc2_formats::landscape::read_landscape_file(fs, &global_layer_name(n))
                    .ok()
                    .and_then(|d| DdsImage::parse(&d).ok())
            };
            let meta = global_meta_image(&read("globalmetamaterials")?).ok()?;
            Some(Self {
                land,
                slots: meta.data?,
                weights: read("globalmetaweights")?.level(0)?.to_vec(),
                slopes: read("globalslopes")?.level(0)?.to_vec(),
            })
        }

        fn colour(&self, x: f32, z: f32) -> ([f32; 3], u8) {
            let (u, v) = (x / 32768.0 + 0.5, z / 32768.0 + 0.5);
            let tex =
                |n: f32, size: i32| ((n * size as f32).floor() as i32).clamp(0, size - 1) as usize;
            let (tx, ty) = (tex(u, 1024), tex(v, 1024));
            let i = ty * 1024 + tx;
            let zone = self
                .land
                .zones
                .as_ref()
                .map_or(0, |zm| zm.texture[tex(v, 64) * 64 + tex(u, 64)]);
            let slots: [u8; 4] = self.slots[i * 4..i * 4 + 4].try_into().unwrap();
            let w3 = bc1_texel(&self.weights, tx, ty);
            let w4 = [
                w3[0],
                w3[1],
                w3[2],
                1.0 - (w3[0] + w3[1] + w3[2]).clamp(0.0, 1.0),
            ];
            let cw = w4.map(jc2_formats::landscape::weight_curve);
            let mut order = [0usize, 1, 2, 3];
            order.sort_by(|&a, &b| cw[b].total_cmp(&cw[a]));
            let (a, b) = (order[0], order[1]);
            let t = (cw[a] + cw[b]).max(1e-6);
            let (wa, wb) = (cw[a] / t, cw[b] / t);
            let sl = bc1_texel(&self.slopes, tx, ty);
            let g = [sl[0] * 2.0 - 1.0, sl[1] * 2.0 - 1.0];
            let s = g[0].hypot(g[1]).min(1.0) * jc2_formats::landscape::SLOPE_BANDS;
            let band = s.floor() as usize;
            let fr = s - s.floor();
            let mut cand = [
                (slots[a], band, wa * (1.0 - fr)),
                (slots[b], band, wb * (1.0 - fr)),
                (slots[a], band + 1, wa * fr),
                (slots[b], band + 1, wb * fr),
            ];
            cand.sort_by(|p, q| q.2.total_cmp(&p.2));
            let t = (cand[0].2 + cand[1].2).max(1e-6);
            let col = |(slot, band, _): (u8, usize, f32)| {
                self.land
                    .slot_colour(zone, slot, band.min(15))
                    .unwrap_or([1.0, 0.0, 1.0])
                    .map(srgb_to_linear)
            };
            let (c1, c2) = (col(cand[0]), col(cand[1]));
            let mix = |k: usize| (cand[0].2 * c1[k] + cand[1].2 * c2[k]) / t;
            ([mix(0), mix(1), mix(2)], zone)
        }
    }

    /// JC2-157: the global mode reproduces the native far colours of the JC2-101 probe
    /// (`research/tools/terrain_terms_probe.py`, `Native.colour`): golden points and the
    /// 2500-point statistics of the highlands north of tile 40,40 (median linear luminance
    /// 0.0382 against the old palette's 0.2265).
    #[test]
    fn global_far_colours_match_the_native_probe() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = jc2_formats::GameFs::open(dir) else {
            return;
        };
        let Some(far) = GlobalFar::load(&fs) else {
            return;
        };
        // (x, z, texture zone, linear rgb) from the probe's Native.colour
        let golden: [(f32, f32, u8, [f32; 3]); 10] = [
            (3500.0, 1800.0, 0, [0.052624, 0.033823, 0.014706]),
            (4380.0, 2760.0, 0, [0.026890, 0.031005, 0.005964]),
            (5460.0, 3720.0, 0, [0.014444, 0.026241, 0.005182]),
            (-12000.0, -9000.0, 1, [0.051745, 0.031728, 0.012983]),
            (9000.0, -3000.0, 0, [0.052043, 0.036329, 0.024457]),
            (-300.0, -8960.0, 0, [0.038675, 0.026683, 0.017217]),
            (15000.0, 12000.0, 1, [0.104616, 0.074214, 0.049707]),
            (-20000.0, 5000.0, 1, [0.048939, 0.034323, 0.022991]),
            (200.0, 200.0, 0, [0.263496, 0.178887, 0.111607]),
            (1000.0, 20000.0, 1, [0.045186, 0.031896, 0.021219]),
        ];
        for (x, z, zone, want) in golden {
            let (c, got_zone) = far.colour(x, z);
            assert_eq!(got_zone, zone, "zone at {x},{z}");
            for k in 0..3 {
                assert!((c[k] - want[k]).abs() < 1e-4, "{x},{z}: {c:?} vs {want:?}");
            }
        }
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let mut cols = Vec::new();
        for zi in 0..50 {
            for xi in 0..50 {
                cols.push(
                    far.colour(3500.0 + 40.0 * xi as f32, 1800.0 + 40.0 * zi as f32)
                        .0,
                );
            }
        }
        let mut lums: Vec<f32> = cols.iter().map(|&c| lum(c)).collect();
        lums.sort_by(f32::total_cmp);
        let (median, p95) = (lums[lums.len() / 2], lums[lums.len() * 95 / 100]);
        let mean: Vec<f32> = (0..3)
            .map(|k| cols.iter().map(|c| c[k]).sum::<f32>() / cols.len() as f32)
            .collect();
        eprintln!("native far ground: median {median:.4} p95 {p95:.4} mean {mean:.4?}");
        // probe: median 0.0382, p95 0.0789, mean (0.0595, 0.0481, 0.0288)
        assert!((median - 0.0382).abs() < 5e-4, "{median}");
        assert!((p95 - 0.0789).abs() < 1e-3, "{p95}");
        for (m, w) in mean.iter().zip([0.0595, 0.0481, 0.0288]) {
            assert!((m - w).abs() < 1e-3, "{mean:?}");
        }
        // the old palette's darkest land colour (grass) is ~6x brighter
        let grass = crate::world::terrain_colour(300.0, 1.0);
        assert!(lum([grass[0], grass[1], grass[2]]) / median > 4.0);
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
