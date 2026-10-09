//! Procedural vegetation: trees, bushes and rocks are not stored anywhere — the
//! engine grows them from the terrain's material map with a fixed-seed random
//! generator, so every machine sees the same forest. This module re-implements that
//! generator (`CVegetationManager`, `FUN_00a85eb0` in the decompilation; see
//! `research/formats/world.md`, "Vegetation").
//!
//! Inputs, all under `climate\`:
//!
//! * `num_zones.bin` — number of climate zones (4: jungle, arctic, desert, and a second
//!   jungle), and `textures\landscape\climatezonemap.dds`, a 64 × 64 A8R8G8B8 image
//!   whose red channel is the zone of each 512 m tile.
//! * `zone_N\vegetation_types_N.bin` — the plant types of a zone: `.lod` model, slope and
//!   altitude ranges, noise coverage, exclusion radius, collision/breakability flags.
//! * `zone_N\vegetation_set_N.bin` — named sets ("Arctic_PineForest"): a list of
//!   (type, group, probability). Each set is turned into three 64-entry lookup tables
//!   (tree, bush, grass): a type gets `probability × 64` slots, the rest are empty, and
//!   the tables are shuffled with the engine's `rand()` seeded with 3.
//! * `terrain_materials.bin` — the terrain material table; each material names the
//!   vegetation set that grows on it (looked up by name through the zones in order).
//!
//! Generation runs per 64 m patch (`pz, px` multiples of 64 from −16 384) and is seeded
//! with the patch origin, `(z / 4) << 13 + x / 4`, so it is deterministic. The engine's
//! `rand()` is the MSVC LCG (`seed × 214013 + 2531011`, value = bits 16..30).
//!
//! * Trees: 4 × 4 cells of 16 m; per cell two draws give the position (`cell + rand × 12 m`
//!   on each axis) and the yaw (`rand2 × 2π`); the tree table is indexed with the low six
//!   bits of the first draw.
//! * Bushes (and rocks): the same seed again, 15 × 15 cells of 4 m (`cell + rand × 3 m`),
//!   skipping every cell whose indices are both multiples of 4, the bush table indexed
//!   the same way; a candidate is dropped when it overlaps an earlier instance in a
//!   window of the bookkeeping array (trees occupy rows 0..3, bushes their own cell).
//! * A candidate survives when the terrain slope and height at its position are inside
//!   the type's ranges and, if the type's coverage is below 1, when
//!   `(fBm(floor(x) × f, floor(z) × f) + 1) / 2 ≤ coverage` — Perlin fBm with the
//!   engine's own tables (`CNoise`, seed 3; H 0.1, lacunarity 2.3, 3 octaves).
//! * Grass is a third table used by a separate 8 m-patch generator (`grass.rs`).
//!
//! Terrain lookups go through the [`Ground`] trait so the caller decides where the
//! height field comes from.

use crate::error::{Error, Result};
use crate::hash::hash_str;
use crate::placement::Transform3;
use crate::property::{Node, PropertyFile, Variant};
use crate::terrain::{TILE_SAMPLES, TerrainTile};
use std::f32::consts::TAU;

/// Edge of a vegetation patch in metres.
pub const PATCH_SIZE: f32 = 64.0;
/// Patches per tile edge.
pub const PATCHES_PER_TILE: u32 = 8;

// ---------------------------------------------------------------------------------
// The engine's rand()

/// One step of the MSVC `rand()` generator the engine uses everywhere.
pub fn lcg(seed: i32) -> i32 {
    seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3)
}

/// The 15-bit value `rand()` returns for a state.
fn rand15(seed: i32) -> i32 {
    (seed >> 16) & 0x7fff
}

// ---------------------------------------------------------------------------------
// Perlin noise and fBm (CNoise / CfBmFractal)

const B: usize = 256;

/// Ken Perlin's reference `noise2`, with the gradient and permutation tables built from
/// the engine's `rand()` (`CNoise::CNoise(seed)`, `FUN_004d4d90`).
pub struct Noise {
    p: Vec<usize>,
    g2: Vec<[f32; 2]>,
}

impl Noise {
    pub fn new(seed: i32) -> Self {
        let mut s = seed;
        let mut p = vec![0usize; 2 * B + 2];
        let mut g2 = vec![[0.0f32; 2]; 2 * B + 2];
        let comp = |r: i32| (((r >> 16) & 0x1ff) - 0x100) as f32 * 0.00390625;
        for (i, (pi, gi)) in p.iter_mut().zip(g2.iter_mut()).enumerate().take(B) {
            *pi = i;
            s = lcg(s); // g1 (unused here)
            s = lcg(s);
            let gx = comp(s);
            s = lcg(s);
            let gy = comp(s);
            let l = 1.0 / (gx * gx + gy * gy).sqrt();
            *gi = [gx * l, gy * l];
            // g3: three more draws, normalised, unused by 2-D noise
            s = lcg(s);
            s = lcg(s);
            s = lcg(s);
        }
        for i in (1..B).rev() {
            let k = p[i];
            s = lcg(s);
            let j = ((s >> 16) & 0xff) as usize;
            p[i] = p[j];
            p[j] = k;
        }
        for i in 0..B + 2 {
            p[B + i] = p[i];
            g2[B + i] = g2[i];
        }
        Self { p, g2 }
    }

    pub fn noise2(&self, x: f32, y: f32) -> f32 {
        let t = x + 4096.0;
        let ti = t as i32;
        let bx0 = (ti & 0xff) as usize;
        let rx0 = t - ti as f32;
        let rx1 = rx0 - 1.0;
        let bx1 = (bx0 + 1) & 0xff;
        let t = y + 4096.0;
        let ti = t as i32;
        let by0 = (ti & 0xff) as usize;
        let ry0 = t - ti as f32;
        let ry1 = ry0 - 1.0;
        let by1 = (by0 + 1) & 0xff;
        let i = self.p[bx0];
        let j = self.p[bx1];
        let b00 = self.p[i + by0];
        let b10 = self.p[j + by0];
        let b01 = self.p[i + by1];
        let b11 = self.p[j + by1];
        let sx = (3.0 - 2.0 * rx0) * rx0 * rx0;
        let sy = (3.0 - 2.0 * ry0) * ry0 * ry0;
        let u = rx0 * self.g2[b00][0] + ry0 * self.g2[b00][1];
        let v = rx1 * self.g2[b10][0] + ry0 * self.g2[b10][1];
        let a = u + sx * (v - u);
        let u = rx0 * self.g2[b01][0] + ry1 * self.g2[b01][1];
        let v = rx1 * self.g2[b11][0] + ry1 * self.g2[b11][1];
        let b = u + sx * (v - u);
        a + sy * (b - a)
    }
}

/// Musgrave's fBm over [`Noise`] (`CfBmFractal`).
pub struct Fbm {
    noise: Noise,
    lacunarity: f32,
    octaves: f32,
    exponents: Vec<f32>,
}

impl Fbm {
    pub fn new(h: f32, lacunarity: f32, octaves: f32, seed: i32) -> Self {
        let n = octaves as i32;
        let mut exponents = Vec::with_capacity(n as usize + 1);
        let mut frequency = 1.0f32;
        for _ in 0..=n {
            exponents.push((frequency as f64).powf(-h as f64) as f32);
            frequency *= lacunarity;
        }
        Self {
            noise: Noise::new(seed),
            lacunarity,
            octaves,
            exponents,
        }
    }

    /// The vegetation generator's fractal: H 0.1, lacunarity 2.3, 3 octaves, seed 3.
    pub fn vegetation() -> Self {
        Self::new(0.1, 2.3, 3.0, 3)
    }

    pub fn value(&self, mut x: f32, mut y: f32) -> f32 {
        let n = self.octaves as i32;
        let mut v = 0.0f32;
        for i in 0..n.max(0) as usize {
            v += self.noise.noise2(x, y) * self.exponents[i];
            x *= self.lacunarity;
            y *= self.lacunarity;
        }
        let rem = self.octaves - n as f32;
        if rem != 0.0 {
            v += self.noise.noise2(x, y) * self.exponents[n as usize] * rem;
        }
        v
    }
}

// ---------------------------------------------------------------------------------
// Types, sets, materials

/// Group a set entry belongs to; selects which table it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Tree,
    Bush,
    Grass,
}

impl Group {
    fn from_name(s: &str) -> Option<Self> {
        match s {
            "tree" => Some(Self::Tree),
            "bush" => Some(Self::Bush),
            "grass" => Some(Self::Grass),
            _ => None,
        }
    }
}

/// One plant type (`vegetation_types_N.bin`).
#[derive(Clone, Debug, PartialEq)]
pub struct VegetationType {
    pub name: String,
    /// Bare `.lod` name of the whole plant (`#cfa2e112`).
    pub model: String,
    /// `.lod` of the stump left when a breakable tree falls (`#6a510b0f`).
    pub stump: Option<String>,
    /// Trunk pieces (`#efa84452`, `;`-separated).
    pub trunks: Vec<String>,
    /// Accepted ground slope as cosines: `slope_cos_min ≤ normal.y ≤ slope_cos_max`
    /// (from `slope_range` in degrees, quantised to 0.001 like the engine).
    pub slope_cos_max: f32,
    pub slope_cos_min: f32,
    /// Accepted ground height (`height_range`).
    pub height_min: f32,
    pub height_max: f32,
    /// Noise frequency multiplier (`#a3809ad6`).
    pub noise_frequency: f32,
    /// Noise coverage threshold (`#b282def7`); ≥ 1 grows everywhere.
    pub coverage: f32,
    /// Exclusion radius against other instances (`#ad1572e6`).
    pub radius: f32,
    pub has_collision: bool,
    pub breakable: bool,
    /// `#bf2041b7`: set on rocks; probably "align to the ground normal".
    pub align_to_ground: bool,
    pub sway_factor: f32,
    /// `billboard`: the id of the tree's impostor in `treeimpostors.bin` (`jc2_formats::
    /// impostors`), −1 for plants that have none.
    pub billboard: i32,
}

/// A set entry as written in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct SetEntry {
    pub type_name: String,
    pub group: Group,
    pub probability: f32,
    /// Grass: slots given outright (`#df8d2697`).
    pub count: i32,
}

/// A named vegetation set with its three shuffled lookup tables.
#[derive(Clone, Debug, PartialEq)]
pub struct VegetationSet {
    pub name: String,
    pub entries: Vec<SetEntry>,
    /// Type index per table slot: `[tree, bush, grass]`.
    pub tables: [[Option<u16>; 64]; 3],
}

impl VegetationSet {
    pub fn table(&self, g: Group) -> &[Option<u16>; 64] {
        &self.tables[g as usize]
    }
}

/// One climate zone: its types and sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zone {
    pub index: usize,
    pub types: Vec<VegetationType>,
    pub sets: Vec<VegetationSet>,
    /// `grass_types_N.bin`: what the sets' grass entries and tables refer to.
    pub grass: crate::grass::GrassTypes,
}

impl Zone {
    /// Container with the zone's models (`climate\zone_N\vegetation_N.blz`).
    pub fn container(&self) -> String {
        format!(
            "climate\\zone_{}\\vegetation_{}.blz",
            self.index, self.index
        )
    }

    pub fn type_index(&self, name: &str) -> Option<u16> {
        self.types
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
            .map(|i| i as u16)
    }
}

/// Which set grows on a terrain material.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaterialSet {
    pub zone: u8,
    pub set: u16,
}

fn key_str(n: &Node, hash: u32) -> Option<&str> {
    match n.value_hash(hash)? {
        Variant::Str(s) => Some(s),
        _ => None,
    }
}

fn key_f32(n: &Node, hash: u32) -> Option<f32> {
    match n.value_hash(hash)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn key_i32(n: &Node, hash: u32) -> Option<i32> {
    match n.value_hash(hash)? {
        Variant::Int(i) => Some(*i),
        Variant::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn key_range(n: &Node, hash: u32) -> Option<(f32, f32)> {
    match n.value_hash(hash)? {
        Variant::Vec2([a, b]) => Some((*a, *b)),
        Variant::Floats(v) if v.len() >= 2 => Some((v[0], v[1])),
        _ => None,
    }
}

/// `cos(degrees)` quantised to a thousandth, the way the type loader stores slopes.
fn slope_cos(deg: f32) -> f32 {
    ((deg.to_radians().cos() * 1000.0) as i32) as f32 * 0.001
}

const KEY_MODEL: u32 = 0xcfa2_e112;
const KEY_STUMP: u32 = 0x6a51_0b0f;
const KEY_TRUNKS: u32 = 0xefa8_4452;
const KEY_SLOPE_RANGE: u32 = 0x5955_dcde;
const KEY_HEIGHT_RANGE: u32 = 0x8538_92e8;
const KEY_NOISE_FREQUENCY: u32 = 0xa380_9ad6;
const KEY_COVERAGE: u32 = 0xb282_def7;
const KEY_RADIUS: u32 = 0xad15_72e6;
const KEY_HAS_COLLISION: u32 = 0x5272_e003;
const KEY_BREAKABLE: u32 = 0x6803_c8c0;
const KEY_ALIGN: u32 = 0xbf20_41b7;
const KEY_SWAY: u32 = 0x87ee_b199;
const KEY_TYPES_ROOT: u32 = 0xa653_deb7;
const KEY_SET_TYPES: u32 = 0x5c32_6dec;
const KEY_GROUP: u32 = 0xd228_2449;
const KEY_PROBABILITY: u32 = 0xd8a0_3db0;
const KEY_COUNT: u32 = 0xdf8d_2697;
const KEY_MATERIAL_INDEX: u32 = 0x89dd_cd69;
const KEY_MATERIAL_SET: u32 = 0x259a_5bb8;
const KEY_NUM_ZONES: u32 = 0x837b_8c6a;

impl VegetationType {
    fn parse(n: &Node) -> Option<Self> {
        let name = n.str("name")?.to_string();
        let model = key_str(n, KEY_MODEL)?.to_string();
        let (s_min, s_max) = key_range(n, KEY_SLOPE_RANGE).unwrap_or((0.0, 90.0));
        let (h_min, h_max) = key_range(n, KEY_HEIGHT_RANGE).unwrap_or((-1e9, 1e9));
        Some(Self {
            name,
            model,
            stump: key_str(n, KEY_STUMP).map(str::to_string),
            trunks: key_str(n, KEY_TRUNKS)
                .map(|s| {
                    s.split(';')
                        .filter(|p| !p.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            slope_cos_max: slope_cos(s_min),
            slope_cos_min: slope_cos(s_max),
            height_min: h_min,
            height_max: h_max,
            noise_frequency: key_f32(n, KEY_NOISE_FREQUENCY).unwrap_or(0.0),
            coverage: key_f32(n, KEY_COVERAGE).unwrap_or(1.0),
            radius: key_f32(n, KEY_RADIUS).unwrap_or(0.0),
            has_collision: key_i32(n, KEY_HAS_COLLISION).unwrap_or(0) != 0,
            breakable: key_i32(n, KEY_BREAKABLE).unwrap_or(0) != 0,
            align_to_ground: key_i32(n, KEY_ALIGN).unwrap_or(0) != 0,
            sway_factor: key_f32(n, KEY_SWAY).unwrap_or(0.0),
            billboard: n.int("billboard").unwrap_or(-1),
        })
    }
}

/// Types of one zone, in file order (the order the engine's tables index).
pub fn parse_types(file: &PropertyFile) -> Vec<VegetationType> {
    let mut out = Vec::new();
    for root in &file.nodes {
        let list = root
            .children
            .iter()
            .find(|(k, _)| k.hash() == KEY_TYPES_ROOT)
            .map(|(_, n)| n)
            .unwrap_or(root);
        for (_, n) in &list.children {
            if let Some(t) = VegetationType::parse(n) {
                out.push(t);
            }
        }
    }
    out
}

/// Sets of one zone as the engine keeps them: every set of the file, then its
/// `_notrees` variant (grass only) and its `_noveg` variant (empty), tables built and
/// shuffled.
pub fn parse_sets(
    file: &PropertyFile,
    types: &[VegetationType],
    grass: &crate::grass::GrassTypes,
) -> Vec<VegetationSet> {
    let zone = Zone {
        index: 0,
        types: types.to_vec(),
        sets: Vec::new(),
        grass: grass.clone(),
    };
    let mut raw = Vec::new();
    for root in &file.nodes {
        for (_, n) in &root.children {
            let Some(name) = n.str("name") else { continue };
            let mut entries = Vec::new();
            if let Some(list) = n
                .children
                .iter()
                .find(|(k, _)| k.hash() == KEY_SET_TYPES)
                .map(|(_, l)| l)
            {
                for (_, e) in &list.children {
                    let Some(type_name) = n_type(e) else { continue };
                    let Some(group) = key_str(e, KEY_GROUP).and_then(Group::from_name) else {
                        continue;
                    };
                    entries.push(SetEntry {
                        type_name: type_name.to_string(),
                        group,
                        probability: key_f32(e, KEY_PROBABILITY).unwrap_or(0.0),
                        count: key_i32(e, KEY_COUNT).unwrap_or(0),
                    });
                }
            }
            raw.push((name.to_string(), entries));
        }
    }
    let mut out = Vec::with_capacity(raw.len() * 3);
    for variant in 0..3 {
        for (name, entries) in &raw {
            let suffix = ["", "_notrees", "_noveg"][variant];
            let kept: Vec<SetEntry> = entries
                .iter()
                .filter(|e| match variant {
                    0 => true,
                    1 => e.group == Group::Grass,
                    _ => false,
                })
                .cloned()
                .collect();
            out.push(VegetationSet {
                name: format!("{name}{suffix}"),
                tables: build_tables(&kept, &zone),
                entries: kept,
            });
        }
    }
    out
}

fn n_type(e: &Node) -> Option<&str> {
    e.str("type")
}

/// The three 64-slot tables of a set: filled in entry order, padded with empty slots,
/// shuffled with `rand()` seeded 3 (`FUN_00a917a0`).
fn build_tables(entries: &[SetEntry], zone: &Zone) -> [[Option<u16>; 64]; 3] {
    let mut tables = [[None; 64]; 3];
    let mut fill = [0usize; 3];
    for e in entries {
        let g = e.group as usize;
        let count = e.count + (e.probability * 64.0) as i32;
        let end = (fill[g] as i32 + count).min(64);
        if end > fill[g] as i32 {
            // grass entries name grass types, the others vegetation types
            let idx = if e.group == Group::Grass {
                zone.grass.index_of(&e.type_name)
            } else {
                zone.type_index(&e.type_name)
            };
            for slot in &mut tables[g][fill[g]..end as usize] {
                *slot = idx;
            }
            fill[g] = end as usize;
        }
    }
    let mut seed = 3i32;
    for t in tables.iter_mut() {
        for i in 0..64 {
            seed = lcg(seed);
            let j = ((seed >> 16) & 0x3f) as usize;
            t.swap(i, j);
        }
    }
    tables
}

// ---------------------------------------------------------------------------------
// The world: zones, materials, climate map

/// Everything the generator needs, loaded once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vegetation {
    /// Index = zone number; `None` where the zone's files are missing.
    pub zones: Vec<Option<Zone>>,
    /// Index = terrain material byte.
    pub materials: Vec<Option<MaterialSet>>,
    /// Zone per tile, `[tz × 64 + tx]`.
    pub climate: Vec<u8>,
    /// Material index → set name, for listings.
    pub material_names: Vec<Option<String>>,
}

/// What the generator asks about a position on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSample {
    pub height: f32,
    /// Unit ground normal, engine axes (`[0, 1, 0]` = flat).
    pub normal: [f32; 3],
    /// Terrain material byte at the position.
    pub material: u8,
}

impl GroundSample {
    /// y of the normal: the cosine of the slope, what the type ranges test.
    pub fn normal_y(&self) -> f32 {
        self.normal[1]
    }
}

/// Height field access for the generator. World coordinates, engine axes.
pub trait Ground {
    fn sample(&self, x: f32, z: f32) -> Option<GroundSample>;
}

impl<F: Fn(f32, f32) -> Option<GroundSample>> Ground for F {
    fn sample(&self, x: f32, z: f32) -> Option<GroundSample> {
        self(x, z)
    }
}

/// One grown plant.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub zone: u8,
    pub type_index: u16,
    pub group: Group,
    /// Engine axes.
    pub position: [f32; 3],
    /// Rotation about y in radians.
    pub yaw: f32,
    /// Ground normal under the plant (engine axes).
    pub normal: [f32; 3],
}

impl Instance {
    /// The instance's world matrix the way `CVegetationModelPatch` builds it
    /// (`FUN_00a85820` + `FUN_004e1470`): a yaw about y, and for types that align to the
    /// ground (`align` — bushes/rocks with the flag) a pitch and roll that lean the plant
    /// onto the ground normal: `pitch = atan((nz·cos − nx·sin) / ny)`,
    /// `roll = atan(−(nx·cos + nz·sin) / ny)`. Row-vector convention like the `.blo`
    /// matrices: rows are the x, y, z axes, then the translation.
    pub fn world(&self, align: bool) -> Transform3 {
        let (s1, c1) = self.yaw.sin_cos();
        let (pitch, roll) = if align {
            let [nx, ny, nz] = self.normal;
            // the engine takes sin/cos of −yaw here
            let (sn, cn) = (-self.yaw).sin_cos();
            let inv = 1.0 / ny.max(1e-4);
            (
                ((nz * cn - nx * sn) * inv).atan(),
                (-(nx * cn + nz * sn) * inv).atan(),
            )
        } else {
            (0.0, 0.0)
        };
        let (s2, c2) = pitch.sin_cos();
        let (s3, c3) = roll.sin_cos();
        let [x, y, z] = self.position;
        Transform3::from_mat4x3([
            s3 * s2 * s1 + c3 * c1,
            s3 * c2,
            s3 * s2 * c1 - c3 * s1,
            c3 * s2 * s1 - s3 * c1,
            c3 * c2,
            c3 * s2 * c1 + s3 * s1,
            c2 * s1,
            -s2,
            c2 * c1,
            x,
            y,
            z,
        ])
    }
}

impl Vegetation {
    /// Parse the climate zone map DDS (64 × 64 A8R8G8B8): red = zone.
    pub fn parse_climate_map(dds: &[u8]) -> Result<Vec<u8>> {
        if dds.len() < 128 || &dds[..4] != b"DDS " {
            return Err(Error::format("climatezonemap: not a DDS"));
        }
        let h = u32::from_le_bytes(dds[12..16].try_into().unwrap()) as usize;
        let w = u32::from_le_bytes(dds[16..20].try_into().unwrap()) as usize;
        let bits = u32::from_le_bytes(dds[88..92].try_into().unwrap());
        if w != 64 || h != 64 || bits != 32 || dds.len() < 128 + w * h * 4 {
            return Err(Error::format(format!(
                "climatezonemap: expected 64×64 32-bit, got {w}×{h} {bits}-bit"
            )));
        }
        Ok((0..w * h).map(|i| dds[128 + i * 4 + 2]).collect())
    }

    /// Build from the parsed files. `zones[i]` = (types file, sets file) of zone `i`.
    pub fn new(
        zones: Vec<Option<(PropertyFile, PropertyFile, Option<PropertyFile>)>>,
        materials: &PropertyFile,
        climate: Vec<u8>,
    ) -> Self {
        let zones: Vec<Option<Zone>> = zones
            .into_iter()
            .enumerate()
            .map(|(i, z)| {
                z.map(|(types, sets, grass)| {
                    let types = parse_types(&types);
                    let grass = grass
                        .map(|g| crate::grass::GrassTypes::parse(&g))
                        .unwrap_or_default();
                    let sets = parse_sets(&sets, &types, &grass);
                    Zone {
                        index: i,
                        types,
                        sets,
                        grass,
                    }
                })
            })
            .collect();
        let mut mats = vec![None; 256];
        let mut names = vec![None; 256];
        for root in &materials.nodes {
            root.walk(&mut |n| {
                let (Some(index), Some(set)) =
                    (key_i32(n, KEY_MATERIAL_INDEX), key_str(n, KEY_MATERIAL_SET))
                else {
                    return;
                };
                if !(0..256).contains(&index) {
                    return;
                }
                names[index as usize] = Some(set.to_string());
                mats[index as usize] = Self::find_set(&zones, set);
            });
        }
        Self {
            zones,
            materials: mats,
            climate,
            material_names: names,
        }
    }

    /// `FUN_00a89b30`: the first zone, in order, with a set of that name.
    fn find_set(zones: &[Option<Zone>], name: &str) -> Option<MaterialSet> {
        if name.is_empty() {
            return None;
        }
        let h = hash_str(name);
        for (zi, z) in zones.iter().enumerate() {
            let Some(z) = z else { continue };
            if let Some(si) = z.sets.iter().position(|s| hash_str(&s.name) == h) {
                return Some(MaterialSet {
                    zone: zi as u8,
                    set: si as u16,
                });
            }
        }
        None
    }

    /// Load from a game folder: `climate\num_zones.bin`, the zones' set and type files
    /// (zones whose files are missing are skipped), `climate\terrain_materials.bin` and
    /// the climate zone map.
    pub fn load(fs: &crate::GameFs) -> Result<Self> {
        let num = PropertyFile::parse_any(&fs.read_inflated("climate\\num_zones.bin")?)?;
        let n = num
            .nodes
            .iter()
            .find_map(|r| key_i32(r, KEY_NUM_ZONES))
            .unwrap_or(4)
            .clamp(1, 8) as usize;
        let mut zones = Vec::with_capacity(n);
        for i in 0..n {
            let types = fs.read_inflated(&format!("climate\\zone_{i}\\vegetation_types_{i}.bin"));
            let sets = fs.read_inflated(&format!("climate\\zone_{i}\\vegetation_set_{i}.bin"));
            let grass = fs
                .read_inflated(&format!("climate\\zone_{i}\\grass_types_{i}.bin"))
                .ok()
                .and_then(|g| PropertyFile::parse_any(&g).ok());
            zones.push(match (types, sets) {
                (Ok(t), Ok(s)) => Some((
                    PropertyFile::parse_any(&t)?,
                    PropertyFile::parse_any(&s)?,
                    grass,
                )),
                _ => None,
            });
        }
        let materials =
            PropertyFile::parse_any(&fs.read_inflated("climate\\terrain_materials.bin")?)?;
        let climate = fs
            .read_inflated("textures\\landscape\\climatezonemap.dds")
            .ok()
            .map(|d| Self::parse_climate_map(&d))
            .transpose()?
            .unwrap_or_else(|| vec![0; 64 * 64]);
        Ok(Self::new(zones, &materials, climate))
    }

    /// Climate zone of a tile.
    pub fn zone_of_tile(&self, tx: u32, tz: u32) -> u8 {
        self.climate
            .get((tz.min(63) * 64 + tx.min(63)) as usize)
            .copied()
            .unwrap_or(0)
    }

    pub fn zone(&self, i: u8) -> Option<&Zone> {
        self.zones.get(i as usize).and_then(|z| z.as_ref())
    }

    /// The type an instance refers to.
    pub fn type_of(&self, inst: &Instance) -> Option<&VegetationType> {
        self.zone(inst.zone)?.types.get(inst.type_index as usize)
    }

    /// Origins of the 8 × 8 patches of a tile.
    pub fn tile_patches(tx: u32, tz: u32) -> impl Iterator<Item = (f32, f32)> {
        let (x0, z0) = crate::terrain::tile_origin(tx, tz);
        (0..PATCHES_PER_TILE).flat_map(move |j| {
            (0..PATCHES_PER_TILE)
                .map(move |i| (x0 + i as f32 * PATCH_SIZE, z0 + j as f32 * PATCH_SIZE))
        })
    }

    /// Grow one 64 m patch whose corner with the smallest x and z is `(px, pz)`;
    /// `climate_zone` is the zone of the patch (see [`Self::zone_of_tile`]).
    pub fn generate_patch(
        &self,
        px: f32,
        pz: f32,
        climate_zone: u8,
        ground: &dyn Ground,
        fbm: &Fbm,
    ) -> Vec<Instance> {
        self.generate_patch_groups(px, pz, climate_zone, ground, fbm, true)
    }

    /// [`Self::generate_patch`], optionally only the trees (the bushes and rocks are
    /// placed after them and never move them: the trees come out the same).
    pub fn generate_patch_groups(
        &self,
        px: f32,
        pz: f32,
        climate_zone: u8,
        ground: &dyn Ground,
        fbm: &Fbm,
        bushes: bool,
    ) -> Vec<Instance> {
        let mut out = Vec::new();
        // (x, z, radius) of what has been placed; trees in rows 0..3, bushes by cell
        let mut slots = [(f32::MAX, f32::MAX, 0.0f32); 256];
        let seed0 = ((pz as i32) >> 2)
            .wrapping_shl(13)
            .wrapping_add((px as i32) >> 2);

        // trees: 4 × 4 cells of 16 m
        let mut seed = seed0;
        for row in 0..4 {
            let cz = pz + row as f32 * 16.0;
            for col in 0..4 {
                let cx = px + col as f32 * 16.0;
                let r1 = lcg(seed);
                let r2 = lcg(r1);
                seed = r2;
                let x = rand15(r1) as f32 * (12.0 / 32767.0) + cx;
                let f2 = rand15(r2) as f32 * (1.0 / 32767.0);
                let z = f2 * 12.0 + cz;
                let slot = ((r1 >> 16) & 0x3f) as usize;
                if let Some((inst, radius)) =
                    self.try_grow(x, z, f2 * TAU, climate_zone, Group::Tree, slot, ground, fbm)
                {
                    out.push(inst);
                    slots[row * 16 + col] = (x, z, radius);
                }
            }
        }

        if !bushes {
            return out;
        }
        // bushes and rocks: 15 × 15 cells of 4 m, the same seed
        let mut seed = seed0;
        for i in 2..=16usize {
            let cz = pz + (i - 2) as f32 * 4.0;
            for j in 2..=16usize {
                let cx = px + (j - 2) as f32 * 4.0;
                if (j - 2) & 3 == 0 && (i - 2) & 3 == 0 {
                    continue;
                }
                let r1 = lcg(seed);
                let r2 = lcg(r1);
                seed = r2;
                let x = rand15(r1) as f32 * (3.0 / 32767.0) + cx;
                let f2 = rand15(r2) as f32 * (1.0 / 32767.0);
                let z = f2 * 3.0 + cz;
                let slot = ((r1 >> 16) & 0x3f) as usize;
                let Some(ty) = self.slot_type(x, z, climate_zone, Group::Bush, slot, ground) else {
                    continue;
                };
                // overlap test against a window of the bookkeeping array, indexed with
                // the raw loop counters like the original
                let radius = ty.radius;
                let mut blocked = false;
                'window: for rr in i.saturating_sub(4)..=i.min(15) {
                    for cc in j.saturating_sub(4)..=j.min(15) {
                        let (ex, ez, er) = slots[rr * 16 + cc];
                        let d = (ez - z) * (ez - z) + (ex - x) * (ex - x)
                            - (er + radius) * (er + radius);
                        if d < 0.0 {
                            blocked = true;
                            break 'window;
                        }
                    }
                }
                if blocked {
                    continue;
                }
                if let Some((inst, radius)) =
                    self.try_grow(x, z, f2 * TAU, climate_zone, Group::Bush, slot, ground, fbm)
                {
                    out.push(inst);
                    slots[(i - 2) * 16 + (j - 2)] = (x, z, radius);
                }
            }
        }
        out
    }

    /// The type a table slot selects at a position, if the ground's material grows a set
    /// of the right zone and the slot is not empty.
    fn slot_type(
        &self,
        x: f32,
        z: f32,
        climate_zone: u8,
        group: Group,
        slot: usize,
        ground: &dyn Ground,
    ) -> Option<&VegetationType> {
        let g = ground.sample(x, z)?;
        let ms = self.materials.get(g.material as usize).copied().flatten()?;
        if ms.zone != climate_zone && ms.zone != 0 {
            return None;
        }
        let zone = self.zone(ms.zone)?;
        let set = zone.sets.get(ms.set as usize)?;
        let ti = set.table(group)[slot]?;
        zone.types.get(ti as usize)
    }

    /// Everything after the table lookup: slope, altitude and coverage tests.
    #[allow(clippy::too_many_arguments)]
    fn try_grow(
        &self,
        x: f32,
        z: f32,
        yaw: f32,
        climate_zone: u8,
        group: Group,
        slot: usize,
        ground: &dyn Ground,
        fbm: &Fbm,
    ) -> Option<(Instance, f32)> {
        let g = ground.sample(x, z)?;
        let ms = self.materials.get(g.material as usize).copied().flatten()?;
        if ms.zone != climate_zone && ms.zone != 0 {
            return None;
        }
        let zone = self.zone(ms.zone)?;
        let set = zone.sets.get(ms.set as usize)?;
        let ti = set.table(group)[slot]?;
        let ty = zone.types.get(ti as usize)?;
        if !(g.normal_y() <= ty.slope_cos_max
            && ty.slope_cos_min <= g.normal_y()
            && ty.height_min <= g.height
            && g.height <= ty.height_max)
        {
            return None;
        }
        if ty.coverage < 1.0 {
            let n = fbm.value(
                (x as i32) as f32 * ty.noise_frequency,
                (z as i32) as f32 * ty.noise_frequency,
            );
            if (n + 1.0) * 0.5 > ty.coverage {
                return None;
            }
        }
        Some((
            Instance {
                zone: ms.zone,
                type_index: ti,
                group,
                position: [x, g.height, z],
                yaw,
                normal: g.normal,
            },
            ty.radius,
        ))
    }

    /// Only the trees of every patch of a tile (what the distant impostors need).
    pub fn generate_tile_trees(
        &self,
        tx: u32,
        tz: u32,
        ground: &dyn Ground,
        fbm: &Fbm,
    ) -> Vec<Instance> {
        let zone = self.zone_of_tile(tx, tz);
        let mut out = Vec::new();
        for (px, pz) in Self::tile_patches(tx, tz) {
            out.extend(self.generate_patch_groups(px, pz, zone, ground, fbm, false));
        }
        out
    }

    /// Grow every patch of a tile.
    pub fn generate_tile(&self, tx: u32, tz: u32, ground: &dyn Ground, fbm: &Fbm) -> Vec<Instance> {
        let zone = self.zone_of_tile(tx, tz);
        let mut out = Vec::new();
        for (px, pz) in Self::tile_patches(tx, tz) {
            out.extend(self.generate_patch(px, pz, zone, ground, fbm));
        }
        out
    }
}

// ---------------------------------------------------------------------------------
// Ground from a terrain tile

/// [`Ground`] over one tile (positions outside its margin give `None`). The material
/// comes from the tile's own 132 × 132 material map (`TerrainTile::materials`, the byte
/// map that follows the heights in the tile file).
pub struct TileGround<'a> {
    pub tile: &'a TerrainTile,
    pub x0: f32,
    pub z0: f32,
}

impl<'a> TileGround<'a> {
    pub fn new(tile: &'a TerrainTile, tx: u32, tz: u32) -> Self {
        let (x0, z0) = crate::terrain::tile_origin(tx, tz);
        Self { tile, x0, z0 }
    }
}

impl Ground for TileGround<'_> {
    fn sample(&self, x: f32, z: f32) -> Option<GroundSample> {
        let (lx, lz) = (x - self.x0, z - self.z0);
        let height = self.tile.sample_local(lx, lz)?;
        // the engine's cell lookup: floor(local / 4) + 2 into the 132 grid
        let c = ((lx * 0.25 + 1.0) as i32 + 1).clamp(0, TILE_SAMPLES as i32 - 1) as usize;
        let r = ((lz * 0.25 + 1.0) as i32 + 1).clamp(0, TILE_SAMPLES as i32 - 1) as usize;
        let material = self.tile.material(r, c);
        let h = |rr: usize, cc: usize| self.tile.height(rr, cc);
        let (rc, cc) = (r.clamp(1, TILE_SAMPLES - 2), c.clamp(1, TILE_SAMPLES - 2));
        // central differences over the 4 m grid; the engine differentiates its bicubic
        // patch instead, which agrees to within a few degrees
        let dx = (h(rc, cc + 1) - h(rc, cc - 1)) / 8.0;
        let dz = (h(rc + 1, cc) - h(rc - 1, cc)) / 8.0;
        let inv = 1.0 / (dx * dx + 1.0 + dz * dz).sqrt();
        Some(GroundSample {
            height,
            normal: [-dx * inv, inv, -dz * inv],
            material,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameFs;

    #[test]
    fn rand_matches_msvc() {
        // srand(1); rand() → 41, 18467, 6334 (the classic MSVC sequence)
        let s = lcg(1);
        assert_eq!(rand15(s), 41);
        let s = lcg(s);
        assert_eq!(rand15(s), 18467);
        let s = lcg(s);
        assert_eq!(rand15(s), 6334);
    }

    #[test]
    fn noise_is_zero_on_the_lattice_and_bounded() {
        let n = Noise::new(3);
        assert_eq!(n.noise2(10.0, -20.0), 0.0);
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..2000 {
            let v = n.noise2(i as f32 * 0.37 - 300.0, i as f32 * 0.11 + 7.0);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo < -0.2 && hi > 0.2 && lo > -1.0 && hi < 1.0, "{lo} {hi}");
        let f = Fbm::vegetation();
        assert_eq!(f.exponents.len(), 4);
        assert!((f.exponents[1] - 2.3f32.powf(-0.1)).abs() < 1e-5);
        let v = f.value(123.0 * 50.0, -4567.0 * 50.0);
        assert!(v.abs() < 2.0);
    }

    #[test]
    fn tables_fill_and_shuffle() {
        let plant = |name: &str, radius: f32| VegetationType {
            name: name.into(),
            model: format!("{name}.lod"),
            stump: None,
            trunks: vec![],
            slope_cos_max: 1.0,
            slope_cos_min: 0.0,
            height_min: 0.0,
            height_max: 1e9,
            noise_frequency: 0.0,
            coverage: 1.0,
            radius,
            has_collision: false,
            breakable: false,
            align_to_ground: false,
            sway_factor: 0.0,
            billboard: -1,
        };
        let types = vec![plant("pine", 1.0), plant("rock", 2.0)];
        let zone = Zone {
            index: 0,
            types,
            sets: vec![],
            grass: Default::default(),
        };
        let entries = vec![
            SetEntry {
                type_name: "pine".into(),
                group: Group::Tree,
                probability: 0.35,
                count: 0,
            },
            SetEntry {
                type_name: "rock".into(),
                group: Group::Bush,
                probability: 0.15,
                count: 0,
            },
            SetEntry {
                type_name: "missing".into(),
                group: Group::Bush,
                probability: 0.1,
                count: 0,
            },
        ];
        let t = build_tables(&entries, &zone);
        assert_eq!(t[0].iter().filter(|s| **s == Some(0)).count(), 22);
        assert_eq!(t[0].iter().filter(|s| s.is_none()).count(), 42);
        assert_eq!(t[1].iter().filter(|s| **s == Some(1)).count(), 9);
        assert_eq!(t[1].iter().filter(|s| s.is_none()).count(), 55);
        assert!(t[2].iter().all(|s| s.is_none()));
        // shuffled: not all pines at the front
        assert!(t[0][..22].iter().any(|s| s.is_none()));
    }

    #[test]
    fn patch_is_deterministic_and_seeded_by_origin() {
        let veg = Vegetation::default();
        let ground = |_x: f32, _z: f32| None::<GroundSample>;
        let fbm = Fbm::vegetation();
        assert!(
            veg.generate_patch(-512.0, -9216.0, 1, &ground, &fbm)
                .is_empty()
        );
        let n: usize = Vegetation::tile_patches(31, 14).count();
        assert_eq!(n, 64);
        assert_eq!(
            Vegetation::tile_patches(31, 14).next(),
            Some((-512.0, -9216.0))
        );
    }

    #[test]
    fn instance_matrix() {
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4);
        let mut inst = Instance {
            zone: 1,
            type_index: 0,
            group: Group::Tree,
            position: [10.0, 20.0, 30.0],
            yaw: std::f32::consts::FRAC_PI_2,
            normal: [0.0, 1.0, 0.0],
        };
        // yaw only: x axis turns towards −z, z axis towards +x (a right-handed turn about
        // +y, row vectors)
        let w = inst.world(false);
        assert!(close(w.row(0), [0.0, 0.0, -1.0]), "{:?}", w.row(0));
        assert!(close(w.row(1), [0.0, 1.0, 0.0]));
        assert!(close(w.row(2), [1.0, 0.0, 0.0]), "{:?}", w.row(2));
        assert_eq!(w.translation(), [10.0, 20.0, 30.0]);
        // aligned to a slope: the y axis follows the ground normal
        inst.yaw = 0.3;
        let n = [0.2f32, 0.9, -0.3];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        inst.normal = [n[0] / l, n[1] / l, n[2] / l];
        let w = inst.world(true);
        let up = w.row(1);
        let dot = up[0] * inst.normal[0] + up[1] * inst.normal[1] + up[2] * inst.normal[2];
        assert!(dot > 0.99, "up {up:?} vs normal {:?}", inst.normal);
        // the rows stay orthonormal
        let x = w.row(0);
        assert!((x[0] * up[0] + x[1] * up[1] + x[2] * up[2]).abs() < 1e-3);
        assert!((x[0] * x[0] + x[1] * x[1] + x[2] * x[2] - 1.0).abs() < 1e-3);
    }

    /// Needs the fixture (JC2_TEST_DIR) with `climate\num_zones.bin`,
    /// `climate\terrain_materials.bin`, `climate\zone_1\vegetation_set_1.bin` and
    /// `vegetation_types_1.bin`, `textures\landscape\climatezonemap.dds` (dropzone: the
    /// bare names) and `terrain_win32_31_14.dat`.
    #[test]
    fn fixture_arctic_village() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        if !fs.contains("vegetation_set_1.bin") || !fs.contains("terrain_win32_31_14.dat") {
            return;
        }
        let veg = Vegetation::load(&fs).unwrap();
        assert_eq!(veg.zone_of_tile(31, 14), 1, "31,14 is arctic");
        let z1 = veg.zone(1).expect("zone 1 loaded");
        assert!(z1.types.len() > 100, "{}", z1.types.len());
        let pine = z1
            .sets
            .iter()
            .find(|s| s.name == "Arctic_PineForest")
            .expect("Arctic_PineForest set");
        assert_eq!(pine.entries.len(), 12);
        let t20 = z1.type_index("Arctic_T20_PineS").unwrap();
        assert_eq!(
            pine.table(Group::Tree)
                .iter()
                .filter(|s| **s == Some(t20))
                .count(),
            22
        );
        assert_eq!(
            pine.table(Group::Tree)
                .iter()
                .filter(|s| s.is_none())
                .count(),
            1
        );
        assert_eq!(
            pine.table(Group::Bush)
                .iter()
                .filter(|s| s.is_none())
                .count(),
            4
        );
        assert!(
            z1.sets
                .iter()
                .any(|s| s.name == "Arctic_PineForest_notrees")
        );
        // material 0x41 "default_arctic" grows the pine forest of zone 1
        assert_eq!(veg.materials[0x41], Some(MaterialSet { zone: 1, set: 0 }));
        assert_eq!(veg.materials[0x52], None, "road_arctic_asphalt has no set");
        let rock = &z1.types[z1.type_index("Arctic_G01_RocksM").unwrap() as usize];
        assert!(rock.has_collision && !rock.breakable && rock.align_to_ground);
        assert!((rock.radius - 4.6).abs() < 1e-5 && (rock.coverage - 0.3).abs() < 1e-5);
        assert_eq!(rock.model, "Arctic_G01_RocksM-Whole.lod");
        assert!(
            (rock.slope_cos_min - 0.707).abs() < 1e-5,
            "{}",
            rock.slope_cos_min
        );

        let tile =
            TerrainTile::parse(&fs.read_inflated("terrain_win32_31_14.dat").unwrap()).unwrap();
        let ground = TileGround::new(&tile, 31, 14);
        assert_eq!(tile.materials.len(), 132 * 132);
        // the village's road is asphalt, the slopes around it arctic ground
        assert_eq!(ground.sample(-450.0, -9100.0).unwrap().material, 0x41);
        let fbm = Fbm::vegetation();
        let inst = veg.generate_tile(31, 14, &ground, &fbm);
        let trees = inst.iter().filter(|i| i.group == Group::Tree).count();
        let bushes = inst.len() - trees;
        assert!(
            trees > 200 && bushes > 300,
            "{trees} trees, {bushes} bushes"
        );
        for i in &inst {
            let t = veg.type_of(i).unwrap();
            assert!(i.position[0] >= -512.0 && i.position[0] < 12.0);
            assert!(i.position[1] >= t.height_min && i.position[1] <= t.height_max);
            assert!(i.yaw >= 0.0 && i.yaw <= TAU);
        }
        // the same patch twice gives the same forest
        let a = veg.generate_patch(-512.0, -9216.0, 1, &ground, &fbm);
        let b = veg.generate_patch(-512.0, -9216.0, 1, &ground, &fbm);
        assert_eq!(a, b);
        eprintln!("31,14: {trees} trees, {bushes} bushes/rocks");
    }

    /// The other climate zones. Needs, besides the files above, the fixture's
    /// `vegetation_set_{0,2,3}.bin`, `vegetation_types_{0,2,3}.bin` and the tiles
    /// `terrain_win32_40_40.dat` (jungle), `terrain_win32_10_50.dat` (desert) and
    /// `terrain_win32_08_24.dat` (the city zone).
    #[test]
    fn fixture_other_zones() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        if !fs.contains("vegetation_set_3.bin") || !fs.contains("terrain_win32_40_40.dat") {
            return;
        }
        let veg = Vegetation::load(&fs).unwrap();
        assert_eq!(veg.zones.len(), 4);
        assert!(veg.zones.iter().all(|z| z.is_some()), "all four zones load");
        // the zones share one type list (zone 3's is shorter); the sets differ
        assert_eq!(veg.zone(0).unwrap().types.len(), 101);
        assert_eq!(veg.zone(3).unwrap().types.len(), 69);
        assert_eq!(veg.zone(0).unwrap().sets.len(), 19 * 3);
        assert_eq!(veg.zone(2).unwrap().sets.len(), 5 * 3);
        // a jungle material resolves to zone 0, the first zone with a set of that name
        assert_eq!(veg.materials[0x00], Some(MaterialSet { zone: 0, set: 0 }));
        assert_eq!(
            veg.materials[0x1b].map(|m| m.zone),
            Some(0),
            "Jungle_To_Arctic"
        );
        let fbm = Fbm::vegetation();
        let expect = [
            ((40u32, 40u32), 0u8, "Jungle_", 400usize),
            ((10, 50), 2, "Desert_", 40),
            ((8, 24), 3, "Jungle_", 300),
        ];
        for ((tx, tz), zone, prefix, min_trees) in expect {
            assert_eq!(veg.zone_of_tile(tx, tz), zone, "{tx},{tz}");
            let tile = TerrainTile::parse(
                &fs.read_inflated(&format!("terrain_win32_{tx:02}_{tz:02}.dat"))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(tile.tail.len(), 4, "{tx},{tz}: every stream patch parsed");
            assert_eq!(tile.gamezone.len(), 64 * 64);
            let ground = TileGround::new(&tile, tx, tz);
            let inst = veg.generate_tile(tx, tz, &ground, &fbm);
            let trees = inst.iter().filter(|i| i.group == Group::Tree).count();
            assert!(trees >= min_trees, "{tx},{tz}: {trees} trees");
            assert!(
                inst.iter()
                    .any(|i| veg.type_of(i).unwrap().name.starts_with(prefix)),
                "{tx},{tz}: no {prefix} plants"
            );
            for i in &inst {
                assert!(i.position[1] >= veg.type_of(i).unwrap().height_min);
            }
            eprintln!(
                "{tx},{tz}: zone {zone}, {trees} trees, {} bushes/rocks",
                inst.len() - trees
            );
        }
        // an arctic tile on the jungle border grows both: zone 0 sets are allowed anywhere
        if fs.contains("terrain_win32_38_12.dat") {
            let tile =
                TerrainTile::parse(&fs.read_inflated("terrain_win32_38_12.dat").unwrap()).unwrap();
            let ground = TileGround::new(&tile, 38, 12);
            let inst = veg.generate_tile(38, 12, &ground, &fbm);
            let zones: std::collections::BTreeSet<u8> = inst.iter().map(|i| i.zone).collect();
            assert_eq!(zones.into_iter().collect::<Vec<_>>(), vec![0, 1]);
        }
    }
}
