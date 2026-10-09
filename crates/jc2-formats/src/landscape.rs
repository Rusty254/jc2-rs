//! The terrain's materials: what the per-tile slot and weight layers mean and which
//! texture they end up drawing.
//!
//! Three layers of indirection, all read off `JustCause2.exe` and the data (see
//! `research/formats/terrain.md`, section "Materials"):
//!
//! 1. **Gameplay materials** (`climate\terrain_materials.bin`, `CTerrainMapGame`
//!    constructor `FUN_00a74e40` → `FUN_00a740e0`): up to 256 entries addressed by their
//!    `Material` index, which is what the 132 × 132 byte map of every tile stores. Each
//!    names a vegetation set, a game material (sounds/particles), displacement
//!    parameters and, through `MetaMaterial`, which render material it is.
//! 2. **Meta materials** (`climate\meta_materials.bin`, `CLandscapeManager`
//!    `FUN_00a70fb0`): the 16 render materials a tile's slot layer can refer to. Each
//!    belongs to a climate zone (Jungle, Arctic, DesertCity, City), occupies a slot
//!    (`meta_index`, 0..16) and carries `num_slope_slots` = 16 **slope slots**: for a
//!    ground steepness band, which texture of the zone's atlas to draw and three more
//!    parameters. The engine packs these into the 16 × 128 `terrain_meta_material_uv_atlas`
//!    texture (row = texture zone × 16 + slot, column = slope slot, texel = `(f0, f1,
//!    f2·255, f3·255)` as bytes).
//! 3. **Texture zones / atlases** (`Landscape\texturezonemap.dds`, 64 × 64, one texel
//!    per tile): which `climate\zone_N\diffuseatlas_N_dif.dds` (2048 × 1024 DXT1, 8 × 4
//!    tiles of 256 px) a tile draws from. A climate zone's `#787b04b5` float4 lists the
//!    texture zones it is drawn with; a meta material is available in every texture zone
//!    of its climate zone. In the shipped data slots 0..10 are the Jungle set in both
//!    atlases, slots 11..14 Arctic in atlas 0 and 11..15 DesertCity in atlas 1.
//!
//! How a cell turns into pixels is read off the game's terrain pixel shader
//! (`FastTer_Lod0ShNone` in `DX10_Shaders_F.shader_bundle`, disassembled; see
//! `research/formats/terrain.md`, "The terrain shader"): [`blend_layers`] is that
//! shader's selection of the two (slot, slope band) pairs it draws, [`slope_band`] its
//! slope measure, [`ATLAS_REPEAT_NEAR`] its tiling.
//!
//! Verified against the data: in the five fixture tiles the gameplay byte map's
//! material → `MetaMaterial` → `meta_index` equals the dominant slot of the render
//! layer in 97–100 % of the cells (`jc2tool terrain X Z --materials` prints the
//! agreement), and the tile's normal layer is `128 + Δraw/m` of its own heights
//! (`jc2tool terrain X Z` prints the fit).

use crate::dds::DdsImage;
use crate::error::{Error, Result};
use crate::property::{Node, PropertyFile, Variant};
use crate::terrain::{HEIGHT_SCALE, MetaCell, TILES_PER_SIDE};
use crate::vfs::GameFs;

/// Meta-material slots a tile layer can address (4 bits).
pub const META_SLOTS: usize = 16;
/// Slope bands per meta material (`num_slope_slots`): the columns of the uv atlas.
pub const SLOPE_SLOTS: usize = 16;
/// Textures per atlas row and column (2048 × 1024 → 8 × 4 tiles of 256 px).
pub const ATLAS_COLUMNS: u32 = 8;
pub const ATLAS_ROWS: u32 = 4;
/// Textures in one atlas.
pub const ATLAS_TEXTURES: usize = (ATLAS_COLUMNS * ATLAS_ROWS) as usize;

/// World-space repeat of an atlas texture on near terrain: the shader tiles every
/// texture with `frac(world.xz × 0.328125)`, i.e. one 256 px texture per 3.05 m, the same
/// for every material (`FastTer_Lod0`).
pub const ATLAS_REPEAT_NEAR: f32 = 1.0 / 0.328125;
/// The repeat the LOD 1 shader uses instead (`× 0.046875`): seven times coarser, so the
/// pattern does not shimmer at a distance (`FastTer_Lod1`; `Lod01` blends the two).
pub const ATLAS_REPEAT_FAR: f32 = 1.0 / 0.046875;

/// Gradient (metres of height per metre: `tan` of the slope) that counts as 1 in the
/// slope measure: `127 × HEIGHT_SCALE` (the shader's `4.26337`). The normal layer of a
/// tile stores `128 + Δraw per metre` per axis, which the shader scales back with it.
pub const SLOPE_UNIT: f32 = 127.0 * HEIGHT_SCALE;
/// The slope measure is `min(|gradient| / SLOPE_UNIT, 1) × 36`; its integer part picks
/// the slope band, the fraction blends with the next band. Only the first 16 bands have
/// table entries (the uv atlas is 16 wide), steeper ground uses the last one.
pub const SLOPE_BANDS: f32 = 36.0;

/// Slope band and the fraction towards the next for a ground gradient (`|∇h|`, m/m):
/// `floor` and `frac` of `min(|g| / SLOPE_UNIT, 1) × 36` (`FastTer_Lod0`: `sqrt(min(dot(g,
/// g), 1)) × 36`, `round_ni`, `frc`). Band `k` starts at `atan(k × 0.1184)`: 6.8° for
/// 1, 25° for 4, 43° for 8, 61° for 15.
pub fn slope_band(gradient: f32) -> (usize, f32) {
    let s = (gradient.abs() / SLOPE_UNIT).min(1.0) * SLOPE_BANDS;
    let i = s.floor();
    (i as usize, s - i)
}

/// The table column a slope band reads: bands past the last column clamp to it (the uv
/// atlas is addressed with `band / 15.5` and, we assume, a clamping sampler).
pub fn band_column(band: usize) -> usize {
    band.min(SLOPE_SLOTS - 1)
}

/// Gradient magnitude of a surface whose unit normal has vertical component `up`.
pub fn gradient_from_up(up: f32) -> f32 {
    let up = up.clamp(1e-4, 1.0);
    (1.0 - up * up).max(0.0).sqrt() / up
}

/// The shader's weight curve: a layer weight below one half is divided by 8, above it is
/// stretched to reach 1 (`w × 1.875 − 0.875`), so a cell shows its dominant material and
/// only blends near an even split.
pub fn weight_curve(w: f32) -> f32 {
    if w < 0.5 {
        w * 0.125
    } else {
        w * 1.875 - 0.875
    }
}

/// One of the two (slot, slope band) pairs the shader draws in a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layer {
    pub slot: u8,
    /// Slope band (`0..36`; [`band_column`] clamps it for table lookups).
    pub band: u8,
    /// Blend weight; the two layers' weights sum to 1.
    pub weight: f32,
}

/// The two layers `FastTer_Lod0` blends in a cell with gradient `|∇h|`: the two
/// heaviest slots after [`weight_curve`], each split between the slope band and the next
/// by the band fraction, and of those four candidates again the two heaviest,
/// normalised. A cell with one material thus blends that material's two neighbouring
/// slope bands; a 50/50 cell blends the two materials at the nearer band.
pub fn blend_layers(cell: &MetaCell, gradient: f32) -> [Layer; 2] {
    // the shader reads the R, G, B weights and sets the fourth to 1 − their sum; the
    // stored weights sum to 15, so the stored fourth nibble is the same thing
    let mut w: Vec<(u8, f32)> = (0..4)
        .map(|i| (cell.slots[i], weight_curve(cell.weights[i] as f32 / 15.0)))
        .collect();
    w.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (sa, wa) = w[0];
    let (sb, wb) = w[1];
    let total = (wa + wb).max(1e-6);
    let (wa, wb) = (wa / total, wb / total);
    let (band, f) = slope_band(gradient);
    let (b0, b1) = (band as u8, (band + 1) as u8);
    let mut c = [
        Layer {
            slot: sa,
            band: b0,
            weight: wa * (1.0 - f),
        },
        Layer {
            slot: sb,
            band: b0,
            weight: wb * (1.0 - f),
        },
        Layer {
            slot: sa,
            band: b1,
            weight: wa * f,
        },
        Layer {
            slot: sb,
            band: b1,
            weight: wb * f,
        },
    ];
    c.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let total = (c[0].weight + c[1].weight).max(1e-6);
    c[0].weight /= total;
    c[1].weight /= total;
    [c[0], c[1]]
}

/// Name of the terrain diffuse atlas of texture zone `zone` (0 or 1 in the shipped game).
/// The engine formats `climate\zone_%d\diffuseatlas_%d_DIF.dds` (`FUN_00a69370`); the
/// archive stores it under the hash of the bare file name, which is what `GameFs` keys on.
pub fn diffuse_atlas_name(zone: u8) -> String {
    format!("climate\\zone_{zone}\\diffuseatlas_{zone}_dif.dds")
}

/// Name of the matching normal atlas.
pub fn normal_atlas_name(zone: u8) -> String {
    format!("climate\\zone_{zone}\\normalatlas_{zone}.dds")
}

/// One gameplay material (`climate\terrain_materials.bin`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMaterial {
    /// `Material`: the index the tile byte map stores.
    pub index: u8,
    /// `MaterialName`, e.g. `road_jungle_asphalt`.
    pub name: String,
    /// `VegetationSet`, e.g. `Rain_Forest` (empty for bare ground).
    pub vegetation_set: String,
    /// `MetaMaterial`: the render material's `global_index`.
    pub meta_material: u32,
    /// `GameMaterial`: the game's surface material id (sounds, particles, friction).
    pub game_material: u32,
    /// `Road`: 1 for road surfaces.
    pub road: bool,
    /// `DefaultMaterialMask`: 1 for the zone's default (non-"paintable") ground.
    pub default: bool,
    /// `#884c9d87`: 2 for natural ground, 3 for roads, 0/1 for flat man-made surfaces.
    pub kind: u32,
    /// `#ae498a0f`: 8 for roads, 4 pavement/dirt, 2 road edges, 0 otherwise.
    pub priority: u32,
    /// `#bd3ceeab`: displacement slot 0..3.
    pub displacement_slot: u32,
    /// `#02bae781`: displacement amplitude (the engine clamps `×1.875` to 0..15).
    pub displacement: f32,
    /// `#2ebeb969`: displacement offset (always `−amplitude / 2`).
    pub displacement_offset: f32,
    /// `#7da08d72`: set on the arctic ground materials (snow?).
    pub flag_7da08d72: bool,
}

/// All gameplay materials, addressable by index.
#[derive(Clone, Debug, Default)]
pub struct TerrainMaterials {
    pub materials: Vec<TerrainMaterial>,
}

const KEY_MATERIALS: u32 = 0x3000_fcd6; // "materials"

impl TerrainMaterials {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = file
            .nodes
            .first()
            .ok_or_else(|| Error::format("terrain_materials: empty"))?;
        let list = child_hash(root, KEY_MATERIALS)
            .ok_or_else(|| Error::format("terrain_materials: no materials list"))?;
        let mut materials = Vec::new();
        for (_, n) in &list.children {
            let index = n.int("Material").unwrap_or(-1);
            if !(0..=255).contains(&index) {
                continue;
            }
            materials.push(TerrainMaterial {
                index: index as u8,
                name: n.str("MaterialName").unwrap_or("").to_string(),
                vegetation_set: n.str("VegetationSet").unwrap_or("").to_string(),
                meta_material: n.int("MetaMaterial").unwrap_or(0) as u32,
                game_material: n.int("GameMaterial").unwrap_or(0) as u32,
                road: n.int("Road").unwrap_or(0) != 0,
                default: n.int("DefaultMaterialMask").unwrap_or(0) != 0,
                kind: int_hash(n, 0x884c_9d87).unwrap_or(0) as u32,
                priority: int_hash(n, 0xae49_8a0f).unwrap_or(0) as u32,
                displacement_slot: int_hash(n, 0xbd3c_eeab).unwrap_or(0) as u32,
                displacement: float_hash(n, 0x02ba_e781).unwrap_or(0.0),
                displacement_offset: float_hash(n, 0x2ebe_b969).unwrap_or(0.0),
                flag_7da08d72: int_hash(n, 0x7da0_8d72).unwrap_or(0) != 0,
            });
        }
        materials.sort_by_key(|m| m.index);
        Ok(Self { materials })
    }

    /// Load `climate\terrain_materials.bin` from the game.
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated("climate\\terrain_materials.bin")?)
    }

    pub fn get(&self, index: u8) -> Option<&TerrainMaterial> {
        self.materials.iter().find(|m| m.index == index)
    }

    pub fn by_name(&self, name: &str) -> Option<&TerrainMaterial> {
        self.materials
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// One slope band of a meta material: the four floats of its `slope_slots` entry, which
/// the engine packs as bytes `(f0, f1, f2 × 255, f3 × 255)` into the uv atlas texel and
/// the pixel shader reads back as follows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlopeSlot {
    /// `f0`: which of the four 1024 px tiles of `Landscape\globaldisplaceatlas.dds`
    /// (4096 × 1024, each tiled every 256 m) perturbs this band's normal
    /// (`frac(f0 × 63.75)` = `(f0 mod 4) / 4` in the shader).
    pub displace_tile: u8,
    /// `f1`: the texture's index in the atlas (`row = i / 8`, `column = i % 8`).
    pub texture: u8,
    /// `f2`: how much the displacement normal bends the gradient before the slope band
    /// is chosen a second time (0..0.6 in the data).
    pub band_bump: f32,
    /// `f3`: how much the displacement normal bends the lighting normal (`× 1.5`), and
    /// how dark its hollows get (`1 − 0.6 × f3 × (1 − n.z) / 2`).
    pub light_bump: f32,
}

/// One render material (`climate\meta_materials.bin`).
#[derive(Clone, Debug, PartialEq)]
pub struct MetaMaterial {
    /// `global_index`: what `TerrainMaterial::meta_material` refers to.
    pub id: u32,
    /// `meta_index`: the slot (0..16) a tile layer stores.
    pub slot: u8,
    /// `zone_index`: the climate zone this material belongs to.
    pub climate_zone: u8,
    pub slopes: [SlopeSlot; SLOPE_SLOTS],
}

/// A climate zone: a name and the texture zones (atlases) it is drawn with.
#[derive(Clone, Debug, PartialEq)]
pub struct ClimateZone {
    pub index: u8,
    pub name: String,
    pub texture_zones: Vec<u8>,
}

/// The render materials and the per-texture-zone slot tables built from them.
#[derive(Clone, Debug, Default)]
pub struct MetaMaterials {
    pub materials: Vec<MetaMaterial>,
    pub zones: Vec<ClimateZone>,
}

impl MetaMaterials {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = file
            .nodes
            .first()
            .ok_or_else(|| Error::format("meta_materials: empty"))?;
        let mut materials = Vec::new();
        if let Some(list) = child_hash(root, KEY_MATERIALS) {
            for (_, n) in &list.children {
                let floats = match n.value("slope_slots") {
                    Some(Variant::Floats(f)) => f.clone(),
                    _ => continue,
                };
                let mut slopes = [SlopeSlot::default(); SLOPE_SLOTS];
                for (i, s) in slopes.iter_mut().enumerate() {
                    let g = &floats[(i * 4).min(floats.len())..((i + 1) * 4).min(floats.len())];
                    if g.len() == 4 {
                        *s = SlopeSlot {
                            displace_tile: (g[0].clamp(0.0, 255.0) as u8) % 4,
                            texture: g[1].clamp(0.0, 255.0) as u8,
                            band_bump: g[2],
                            light_bump: g[3],
                        };
                    }
                }
                materials.push(MetaMaterial {
                    id: n.int("global_index").unwrap_or(0) as u32,
                    slot: n.int("meta_index").unwrap_or(0).clamp(0, 15) as u8,
                    climate_zone: n.int("zone_index").unwrap_or(0).clamp(0, 255) as u8,
                    slopes,
                });
            }
        }
        let mut zones = Vec::new();
        if let Some(list) = child_hash(root, 0xa964_9a63) {
            // "zones"
            for (_, n) in &list.children {
                let texture_zones = match n.value_hash(0x787b_04b5) {
                    Some(Variant::Vec4(v)) => {
                        v.iter().filter(|x| **x >= 0.0).map(|x| *x as u8).collect()
                    }
                    _ => Vec::new(),
                };
                zones.push(ClimateZone {
                    index: n.int("zone_index").unwrap_or(0).clamp(0, 255) as u8,
                    name: n.str("zone_name").unwrap_or("").to_string(),
                    texture_zones,
                });
            }
        }
        materials.sort_by_key(|m| (m.climate_zone, m.slot));
        zones.sort_by_key(|z| z.index);
        Ok(Self { materials, zones })
    }

    /// Load `climate\meta_materials.bin` from the game.
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated("climate\\meta_materials.bin")?)
    }

    pub fn by_id(&self, id: u32) -> Option<&MetaMaterial> {
        self.materials.iter().find(|m| m.id == id)
    }

    /// Number of texture zones (atlases) any climate zone refers to.
    pub fn texture_zone_count(&self) -> usize {
        self.zones
            .iter()
            .flat_map(|z| z.texture_zones.iter())
            .map(|z| *z as usize + 1)
            .max()
            .unwrap_or(0)
    }

    /// The 16 slots of texture zone `zone`: the meta materials of every climate zone
    /// drawn with that atlas, by slot. (Mirrors how `FUN_00a70fb0` fills the uv atlas.)
    pub fn slots(&self, zone: u8) -> [Option<&MetaMaterial>; META_SLOTS] {
        let mut out = [None; META_SLOTS];
        for cz in self
            .zones
            .iter()
            .filter(|z| z.texture_zones.contains(&zone))
        {
            for m in self.materials.iter().filter(|m| m.climate_zone == cz.index) {
                out[m.slot as usize] = Some(m);
            }
        }
        out
    }

    /// The slope-slot entry of `slot` in texture zone `zone` for slope band `band`
    /// (bands past the table clamp to its last column).
    pub fn texture(&self, zone: u8, slot: u8, band: usize) -> Option<&SlopeSlot> {
        self.slots(zone)[(slot as usize).min(META_SLOTS - 1)].map(|m| &m.slopes[band_column(band)])
    }
}

/// The two 64 × 64 per-tile zone maps.
#[derive(Clone, Debug)]
pub struct ZoneMaps {
    /// Climate zone per tile (0 Jungle, 1 Arctic, 2 DesertCity, 3 City), row = Z.
    pub climate: Vec<u8>,
    /// Texture zone (atlas) per tile, row = Z.
    pub texture: Vec<u8>,
}

impl ZoneMaps {
    /// Decode `texturezonemap.dds` and `climatezonemap.dds` (64 × 64 `A8R8G8B8`, the
    /// zone in the red byte; `FUN_00a555a0` reads the packed 4-bit copy of it).
    pub fn parse(texture: &[u8], climate: &[u8]) -> Result<Self> {
        let decode = |bytes: &[u8]| -> Result<Vec<u8>> {
            let img = DdsImage::parse(bytes)?;
            let n = TILES_PER_SIDE as usize;
            if img.width as usize != n || img.height as usize != n {
                return Err(Error::format(format!(
                    "zone map is {}×{}, expected {n}×{n}",
                    img.width, img.height
                )));
            }
            let rgba = img
                .decode_rgba(0)
                .ok_or_else(|| Error::format("zone map pixel format"))?;
            Ok(rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect())
        };
        Ok(Self {
            texture: decode(texture)?,
            climate: decode(climate)?,
        })
    }

    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(
            &read_landscape_file(fs, "textures\\landscape\\texturezonemap.dds")?,
            &read_landscape_file(fs, "textures\\landscape\\climatezonemap.dds")?,
        )
    }
}

/// A `Landscape\...` file: loose in the archives, or a member of `general.blz` (which
/// carries copies of the zone maps and global maps).
pub fn read_landscape_file(fs: &GameFs, name: &str) -> Result<Vec<u8>> {
    if let Ok(d) = fs.read_inflated(name) {
        return Ok(d);
    }
    let bare = crate::placement::bare_name(name);
    let general = fs.read_inflated("general.blz")?;
    let sarc = crate::sarc::SmallArchive::parse(&general)?;
    let entry = sarc
        .get(bare)
        .ok_or_else(|| Error::NotFound(name.to_string()))?;
    let bytes = sarc.data(&general, entry)?;
    Ok(crate::compress::maybe_inflate(bytes)?.into_owned())
}

impl ZoneMaps {
    pub fn texture_zone(&self, tx: u32, tz: u32) -> u8 {
        self.texture[(tz * TILES_PER_SIDE + tx) as usize]
    }

    pub fn climate_zone(&self, tx: u32, tz: u32) -> u8 {
        self.climate[(tz * TILES_PER_SIDE + tx) as usize]
    }
}

/// Everything needed to turn a tile's layers into textures and colours.
#[derive(Clone, Debug, Default)]
pub struct Landscape {
    pub materials: TerrainMaterials,
    pub metas: MetaMaterials,
    pub zones: Option<ZoneMaps>,
    /// Mean RGB (0..1, as stored, i.e. sRGB) of every atlas texture, per texture zone;
    /// empty for atlases that were not found.
    pub atlas_colours: Vec<Vec<[f32; 3]>>,
}

impl Landscape {
    /// Load the material tables, the zone maps and the atlases' mean colours. Missing
    /// pieces degrade gracefully (empty tables, no zone map → zone 0), so a partial
    /// game folder still works.
    pub fn load(fs: &GameFs) -> Result<Self> {
        let materials = TerrainMaterials::load(fs)?;
        let metas = MetaMaterials::load(fs)?;
        let zones = ZoneMaps::load(fs).ok();
        let pixel_map = read_landscape_file(fs, "textures\\landscape\\globalcolorpixelmap.dds")
            .ok()
            .and_then(|d| colour_pixel_map(&d));
        let mut atlas_colours = Vec::new();
        for zone in 0..metas.texture_zone_count().max(1) as u8 {
            let colours = pixel_map
                .as_ref()
                .and_then(|rows| rows.get(zone as usize).cloned())
                .or_else(|| {
                    fs.read_inflated(&diffuse_atlas_name(zone))
                        .ok()
                        .and_then(|d| DdsImage::parse(&d).ok())
                        .map(|img| atlas_mean_colours(&img))
                })
                .unwrap_or_default();
            atlas_colours.push(colours);
        }
        Ok(Self {
            materials,
            metas,
            zones,
            atlas_colours,
        })
    }

    pub fn texture_zone(&self, tx: u32, tz: u32) -> u8 {
        self.zones
            .as_ref()
            .map(|z| z.texture_zone(tx, tz))
            .unwrap_or(0)
    }

    /// Mean colour (sRGB 0..1) of the texture that `slot` draws in texture zone `zone`
    /// on ground of slope band `band`; `None` when the slot or atlas is unknown.
    pub fn slot_colour(&self, zone: u8, slot: u8, band: usize) -> Option<[f32; 3]> {
        let tex = self.metas.texture(zone, slot, band)?.texture as usize;
        self.atlas_colours
            .get(zone as usize)
            .and_then(|c| c.get(tex))
            .copied()
    }

    /// Mean colour (sRGB 0..1) the shader would draw in a cell of texture zone `zone`
    /// with ground gradient `gradient`: the two layers of [`blend_layers`] mixed. `None`
    /// when the tables do not cover a layer's slot.
    pub fn cell_colour(&self, zone: u8, cell: &MetaCell, gradient: f32) -> Option<[f32; 3]> {
        let mut out = [0.0f32; 3];
        for layer in blend_layers(cell, gradient) {
            let c = self.slot_colour(zone, layer.slot, layer.band as usize)?;
            for k in 0..3 {
                out[k] += c[k] * layer.weight;
            }
        }
        Some(out)
    }
}

/// The game's own table of mean atlas colours, `Landscape\globalcolorpixelmap.dds`:
/// `A8R8G8B8`, one row per texture zone, one column per atlas texture (32). The engine
/// copies its texels into the colour table the far terrain shaders draw with
/// (`FUN_00a70fb0`, second buffer). Returns the rows as sRGB 0..1; `None` when the
/// image is not that shape.
pub fn colour_pixel_map(data: &[u8]) -> Option<Vec<Vec<[f32; 3]>>> {
    let img = DdsImage::parse(data).ok()?;
    if img.width as usize != ATLAS_TEXTURES {
        return None;
    }
    let rgba = img.decode_rgba(0)?;
    Some(
        rgba.as_chunks::<{ ATLAS_TEXTURES * 4 }>()
            .0
            .iter()
            .map(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| {
                        [
                            p[0] as f32 / 255.0,
                            p[1] as f32 / 255.0,
                            p[2] as f32 / 255.0,
                        ]
                    })
                    .collect()
            })
            .collect(),
    )
}

/// Mean colour (sRGB 0..1) of each of the 32 textures of an atlas, from a small mip:
/// what [`colour_pixel_map`] holds, computed, for when the file is missing.
pub fn atlas_mean_colours(img: &DdsImage) -> Vec<[f32; 3]> {
    // a level where every tile is still ≥ 8 px wide
    let tile = img.width / ATLAS_COLUMNS;
    let mut level = 0;
    while level + 1 < img.mips && (tile >> (level + 1)) >= 8 {
        level += 1;
    }
    let (w, h) = img.level_size(level);
    let (tw, th) = (w / ATLAS_COLUMNS, h / ATLAS_ROWS);
    let Some(rgba) = img.decode_rgba(level) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(ATLAS_TEXTURES);
    for i in 0..ATLAS_TEXTURES as u32 {
        let (row, col) = (i / ATLAS_COLUMNS, i % ATLAS_COLUMNS);
        let mut sum = [0.0f64; 3];
        let mut n = 0.0;
        for y in row * th..(row + 1) * th {
            for x in col * tw..(col + 1) * tw {
                let p = &rgba[((y * w + x) * 4) as usize..];
                sum[0] += p[0] as f64;
                sum[1] += p[1] as f64;
                sum[2] += p[2] as f64;
                n += 1.0;
            }
        }
        out.push([
            (sum[0] / n / 255.0) as f32,
            (sum[1] / n / 255.0) as f32,
            (sum[2] / n / 255.0) as f32,
        ]);
    }
    out
}

fn child_hash(n: &Node, hash: u32) -> Option<&Node> {
    n.children
        .iter()
        .find(|(k, _)| k.hash() == hash)
        .map(|(_, c)| c)
}

fn int_hash(n: &Node, hash: u32) -> Option<i32> {
    match n.value_hash(hash)? {
        Variant::Int(i) => Some(*i),
        Variant::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn float_hash(n: &Node, hash: u32) -> Option<f32> {
    match n.value_hash(hash)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{TerrainTile, tile_file_name};

    #[test]
    fn slope_bands_follow_the_shader() {
        assert_eq!(slope_band(0.0), (0, 0.0));
        // one slope unit is band 36 (clamped), half of it band 18
        assert_eq!(slope_band(SLOPE_UNIT).0, 36);
        assert_eq!(slope_band(SLOPE_UNIT * 0.5).0, 18);
        assert_eq!(slope_band(10.0).0, 36);
        let (b, f) = slope_band(SLOPE_UNIT * 1.5 / 36.0);
        assert_eq!(b, 1);
        assert!((f - 0.5).abs() < 1e-4);
        // 45° ground: gradient 1 → 1 / 4.263 × 36 = 8.44 → band 8
        assert_eq!(slope_band(gradient_from_up(0.5f32.sqrt())).0, 8);
        assert_eq!(band_column(36), 15);
        assert_eq!(band_column(7), 7);
        assert!((weight_curve(1.0) - 1.0).abs() < 1e-6);
        assert!((weight_curve(0.5) - 0.0625).abs() < 1e-6);
        assert!((weight_curve(0.4) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn blend_picks_the_two_heaviest_candidates() {
        // one material: its two neighbouring bands, split by the fraction
        let cell = MetaCell {
            slots: [3, 0, 0, 0],
            weights: [15, 0, 0, 0],
        };
        let g = SLOPE_UNIT * 2.25 / 36.0; // band 2.25
        let [a, b] = blend_layers(&cell, g);
        assert_eq!((a.slot, a.band), (3, 2));
        assert_eq!((b.slot, b.band), (3, 3));
        assert!((a.weight - 0.75).abs() < 1e-4 && (b.weight - 0.25).abs() < 1e-4);
        // an even split of two materials on flat ground: both at band 0
        let cell = MetaCell {
            slots: [3, 7, 0, 0],
            weights: [8, 7, 0, 0],
        };
        let [a, b] = blend_layers(&cell, 0.0);
        assert_eq!((a.slot, a.band), (3, 0));
        assert_eq!((b.slot, b.band), (7, 0));
        assert!((a.weight + b.weight - 1.0).abs() < 1e-5);
        assert!(a.weight > b.weight);
        // a 10/5 split is nearly all the first after the weight curve
        let cell = MetaCell {
            slots: [3, 7, 0, 0],
            weights: [10, 5, 0, 0],
        };
        let [a, _] = blend_layers(&cell, 0.0);
        assert!(a.weight > 0.85, "{}", a.weight);
    }

    /// Needs JC2_TEST_DIR with `terrain_materials.bin`, `meta_materials.bin`,
    /// `general.blz` (the zone maps), `diffuseatlas_0_dif.dds` and
    /// `globalcolorpixelmap.dds` (all from `textures\landscape\` or `climate\`) in its
    /// dropzone.
    #[test]
    fn material_tables_from_the_game() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        if !fs.contains("climate\\terrain_materials.bin") {
            return;
        }
        let land = Landscape::load(&fs).unwrap();
        let m = &land.materials;
        assert_eq!(m.materials.len(), 66);
        let road = m.by_name("road_jungle_asphalt").unwrap();
        assert_eq!(road.index, 18);
        assert!(road.road);
        assert_eq!(road.meta_material, 5);
        let arctic = m.get(65).unwrap();
        assert_eq!(arctic.name, "default_arctic");
        assert_eq!(arctic.vegetation_set, "Arctic_PineForest");
        assert_eq!(arctic.meta_material, 20);
        assert!(arctic.default);

        let metas = &land.metas;
        assert_eq!(metas.materials.len(), 20);
        assert_eq!(metas.zones.len(), 4);
        assert_eq!(metas.texture_zone_count(), 2);
        let snow = metas.by_id(20).unwrap();
        assert_eq!((snow.slot, snow.climate_zone), (11, 1));
        assert_eq!(snow.slopes[0].texture, 16);
        assert_eq!(snow.slopes[15].texture, 26);
        // atlas 0 holds the jungle slots and the arctic ones, atlas 1 the desert ones
        let a0 = metas.slots(0);
        let a1 = metas.slots(1);
        assert_eq!(a0[0].unwrap().id, 0);
        assert_eq!(a0[11].unwrap().id, 20);
        assert_eq!(a1[0].unwrap().id, 0);
        assert_eq!(a1[11].unwrap().id, 30);
        assert!(a0[15].is_none());
        assert_eq!(metas.texture(0, 8, 0).unwrap().texture, 30); // asphalt

        if let Some(z) = &land.zones {
            assert_eq!(z.climate_zone(31, 14), 1);
            assert_eq!(z.texture_zone(31, 14), 0);
            assert_eq!(z.texture_zone(5, 50), 1);
        }
        if !land.atlas_colours[0].is_empty() {
            assert_eq!(land.atlas_colours[0].len(), 32);
            // snow is bright, asphalt dark
            assert!(land.atlas_colours[0][16][0] > 0.7);
            assert!(land.atlas_colours[0][30][0] < 0.4);
        }
        // the game's colour table and the atlas means agree (it is a little desaturated)
        if let (Ok(pm), Ok(atlas)) = (
            read_landscape_file(&fs, "textures\\landscape\\globalcolorpixelmap.dds"),
            fs.read_inflated(&diffuse_atlas_name(0)),
        ) {
            let rows = colour_pixel_map(&pm).unwrap();
            assert_eq!(rows.len(), 2);
            let means = atlas_mean_colours(&DdsImage::parse(&atlas).unwrap());
            for (table, mean) in rows[0].iter().zip(&means) {
                for k in 0..3 {
                    assert!((table[k] - mean[k]).abs() < 0.12, "{table:?} vs {mean:?}");
                }
            }
            assert_eq!(land.atlas_colours[0], rows[0]);
        }

        // the gameplay byte map agrees with the render layer's dominant slot
        if fs.contains("terrain_win32_31_14.dat") {
            let t =
                TerrainTile::parse(&fs.read_inflated(&tile_file_name(31, 14)).unwrap()).unwrap();
            assert!(t.has_layers());
            let (mut agree, mut total) = (0, 0);
            for cz in 0..128 {
                for cx in 0..128 {
                    let expect = m
                        .get(t.cell_material(cz, cx))
                        .and_then(|g| metas.by_id(g.meta_material))
                        .map(|mm| mm.slot);
                    total += 1;
                    if expect == Some(t.meta_cell(cz, cx).dominant()) {
                        agree += 1;
                    }
                }
            }
            assert!(agree * 100 / total >= 95, "{agree}/{total}");
        }
    }
}
