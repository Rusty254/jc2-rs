//! RBMDL render-block models (`.rbm`) — the mesh format for vehicles, props
//! and characters.
//!
//! ```text
//! u32 5, "RBMDL", i32 major=1, i32 minor=13, i32 revision
//! f32 bbox min xyz, f32 bbox max xyz
//! i32 block_count
//! repeat:
//!   u32 type_hash            hashlittle("CarPaint") etc.
//!   block body               layout depends on the type (see below)
//!   u32 0x89ABCDEF           footer
//! ```
//!
//! Block bodies are not length-prefixed, so an unknown block type ends
//! parsing; everything before it is kept and `truncated_at` records the type.
//!
//! Vertex attributes are stored packed. Positions are either `f32` or
//! `i16` snorm scaled by a header float; normals/tangents are a single `f32`
//! whose integer and fractional parts carry three bytes (see
//! [`unpack_normal`]); the sign of the float carries tangent handedness.
//! Layouts were worked out against real files (see `research/`) with the
//! Gibbed.Avalanche block definitions as the starting point.

use crate::error::{Error, Result};
use crate::hash::hash_str;
use crate::reader::{Endian, Reader};
use std::sync::OnceLock;

pub const FOOTER: u32 = 0x89AB_CDEF;

/// Every render block type name the engine knows, for diagnostics.
pub const BLOCK_TYPE_NAMES: &[&str] = &[
    "2DTex1",
    "2DTex2",
    "3DText",
    "AOBox",
    "Beam",
    "BillboardFoliage",
    "Box",
    "Bullet",
    "CarPaint",
    "CarPaintSimple",
    "CirrusClouds",
    "Clouds",
    "Creatures",
    "DecalDeformable",
    "DecalSimple",
    "DecalSkinned",
    "DeformableWindow",
    "Facade",
    "Flag",
    "FogGradient",
    "Font",
    "General",
    "Grass",
    "GuiAnark",
    "Halo",
    "Lambert",
    "Leaves",
    "Lights",
    "Line",
    "Merged",
    "NvWaterHighEnd",
    "Occluder",
    "Open",
    "Particle",
    "Skidmarks",
    "SkinnedGeneral",
    "SkyGradient",
    "SoftClouds",
    "SplineRoad",
    "Stars",
    "Terrain",
    "TerrainForest",
    "TerrainForestFin",
    "TreeImpostorTrunk",
    "TreeImpostorTop",
    "Triangle",
    "VegetationBark",
    "VegetationFoliage",
    "WaterGodrays",
    "WaterHighEnd",
    "WaterWaves",
    "Weather",
    "Window",
];

pub fn block_type_name(hash: u32) -> Option<&'static str> {
    static TABLE: OnceLock<Vec<(u32, &'static str)>> = OnceLock::new();
    let t = TABLE.get_or_init(|| BLOCK_TYPE_NAMES.iter().map(|n| (hash_str(n), *n)).collect());
    t.iter().find(|(h, _)| *h == hash).map(|(_, n)| *n)
}

/// Texture slots of a block material, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Material {
    pub diffuse: String,
    pub normal: String,
    /// "mpm": metallic / paint mask / properties
    pub properties: String,
    pub deformed_diffuse: String,
    pub deformed_normal: String,
    pub deformed_properties: String,
    pub normal_ex: String,
    pub shadow: String,
    pub flags: u32,
}

impl Material {
    pub fn textures(&self) -> [&str; 8] {
        [
            &self.diffuse,
            &self.normal,
            &self.properties,
            &self.deformed_diffuse,
            &self.deformed_normal,
            &self.deformed_properties,
            &self.normal_ex,
            &self.shadow,
        ]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    /// Decoded from the packed attribute when the block has one; empty otherwise.
    pub normals: Vec<[f32; 3]>,
    /// xyz + handedness sign, when present.
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    /// General/Lambert vertex colour stored as a packed float. Preserve the
    /// integer representation; General VS188280 decodes its fractional RGB.
    pub packed_colours: Vec<u32>,
    /// Triangle list.
    pub indices: Vec<u16>,
    /// Skinning (`SkinnedGeneral`): four skeleton bone indices and weights per vertex,
    /// the batch-local indices already mapped through the batch bone lists.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

impl MeshData {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut it = self.positions.iter();
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        }))
    }
    /// Area-weighted vertex normals from the triangles — the fallback when a
    /// block has no decodable normal attribute.
    pub fn compute_normals(&mut self) {
        let mut n = vec![[0f32; 3]; self.positions.len()];
        for t in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            if a >= n.len() || b >= n.len() || c >= n.len() {
                continue;
            }
            let (pa, pb, pc) = (self.positions[a], self.positions[b], self.positions[c]);
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let f = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            for i in [a, b, c] {
                n[i][0] += f[0];
                n[i][1] += f[1];
                n[i][2] += f[2];
            }
        }
        for v in &mut n {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if l > 1e-12 {
                *v = [v[0] / l, v[1] / l, v[2] / l];
            } else {
                *v = [0.0, 1.0, 0.0];
            }
        }
        self.normals = n;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderBlock {
    pub type_name: String,
    pub type_hash: u32,
    pub version: u8,
    pub material: Material,
    pub mesh: MeshData,
    /// Raw header floats of the block, for research.
    pub params: Vec<f32>,
    /// Verified VegetationFoliage +0x3c flags, separate from five header floats.
    pub foliage_flags: Option<u32>,
    /// General's exact header, independent of the material texture trailer.
    pub general: Option<GeneralHeader>,
    /// Deform / skin / colour attributes that are parsed past but not yet
    /// interpreted are counted here so nothing is silently lost.
    pub extra_attribute_streams: u8,
    /// Merged "batch" blocks carry no geometry of their own: they place
    /// `<name>_part<N>.rbm` models from the same container.
    pub instances: Vec<Instance>,
}

/// CRenderBlockGeneral +34..7f, verified by importer00935840/reader00973da0.
/// Packing extents/colour are compiler metadata; do not reinterpret integers as
/// floats or apply colour_extent to VS188280's fractional colour decode.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralHeader {
    pub channel_texture_mask: [f32; 4],
    pub channel_ao_mask: [f32; 3],
    pub channel_ao_factor: f32,
    pub depth_offset: f32,
    pub specular_power: f32,
    pub vertex_format: u32,
    pub position_extent: f32,
    pub uv0_extent: [f32; 2],
    pub uv1_extent: [f32; 2],
    pub colour_extent: f32,
    pub packed_colour: u32,
    pub render_flags: u32,
}
impl GeneralHeader {
    pub fn channel_textures(&self) -> bool {
        self.render_flags & 0x20 != 0
    }
    pub fn alpha_test(&self) -> bool {
        self.render_flags & 0x100 != 0
    }
    pub fn no_culling(&self) -> bool {
        self.render_flags & 1 != 0
    }
    /// Native first-positive channel selector00949b50; retains all mask values.
    pub fn variant_key(&self) -> u8 {
        let texture = if self.channel_textures() {
            self.channel_texture_mask
                .iter()
                .position(|v| *v > 0.0)
                .map_or(0, |i| i as u8 + 1)
        } else {
            0
        };
        let ao = if self.render_flags & 0x200 != 0 {
            self.channel_ao_mask
                .iter()
                .position(|v| *v > 0.0)
                .map_or(0, |i| (i as u8 + 1) * 8)
        } else {
            0
        };
        texture | ao
    }
}

/// General VS188280: frc(c * [1,1/64,1/4096]), with instance alpha supplied
/// separately. `colour_extent` is not multiplied in this consumer.
pub fn general_vertex_colour(raw: u32) -> [f32; 4] {
    let c = f32::from_bits(raw);
    let fraction = |v: f32| v - v.floor();
    [fraction(c), fraction(c / 64.0), fraction(c / 4096.0), 1.0]
}

/// VegetationBark (32 B layout) vertex colour: the vertex shader's `frc(w * (1, 1/256, 1/65536))`
/// on the packed float `w` of `v0` (`vegetationbark` VS, `o6`), multiplied into the diffuse by the
/// pixel shader (`mul r1.xyz, diffuse, v6`). Alpha is not part of it.
pub fn vegetation_bark_colour(raw: u32) -> [f32; 4] {
    let c = f32::from_bits(raw);
    let fraction = |v: f32| v - v.floor();
    [
        fraction(c),
        fraction(c * (1.0 / 256.0)),
        fraction(c * (1.0 / 65536.0)),
        1.0,
    ]
}

/// One placement from a Merged batch block.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// Row-major 3×4: rows are [r0 r1 r2 t] for x, y, z.
    pub matrix: [f32; 12],
    /// Index of the part model this instance draws (see `.batchlod`).
    pub part: u16,
    pub extra: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RbmModel {
    pub version: (i32, i32, i32),
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub blocks: Vec<RenderBlock>,
    /// Set when parsing stopped at an unsupported block type: (block index, type name).
    pub truncated_at: Option<(usize, String)>,
}

pub fn is_rbm(data: &[u8]) -> bool {
    data.len() >= 9
        && &data[4..9] == b"RBMDL"
        && (data[0..4] == [5, 0, 0, 0] || data[0..4] == [0, 0, 0, 5])
}

/// Decode the engine's packed 3-component attribute: the fractional part and
/// the two low bytes of the integer part are three bytes mapped to [-1, 1].
/// The sign of `v` is returned separately (tangent handedness).
pub fn unpack_normal(v: f32) -> ([f32; 3], f32) {
    let sign = if v < 0.0 { -1.0 } else { 1.0 };
    let a = v.abs();
    let int = a.floor();
    let frac = a - int;
    let i = int as u32;
    let b0 = frac * 256.0;
    let b1 = (i & 0xff) as f32;
    let b2 = ((i >> 8) & 0xff) as f32;
    ([b0 / 127.5 - 1.0, b1 / 127.5 - 1.0, b2 / 127.5 - 1.0], sign)
}

/// Bytes x, y, z of a packed `u32` mapped to [-1, 1] (SkinnedGeneral normal, tangent, bitangent).
fn unpack_bytes3(v: u32) -> [f32; 3] {
    [
        (v & 0xff) as f32 / 127.5 - 1.0,
        ((v >> 8) & 0xff) as f32 / 127.5 - 1.0,
        ((v >> 16) & 0xff) as f32 / 127.5 - 1.0,
    ]
}

fn snorm(v: i16) -> f32 {
    if v == -32768 {
        -1.0
    } else {
        v as f32 / 32767.0
    }
}

/// IEEE half-precision to f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let frac = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            let mut e: i32 = 127 - 15 + 1;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((f & 0x3ff) << 13)
        }
    } else if exp == 31 {
        (sign << 31) | 0x7f80_0000 | (frac << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (frac << 13)
    };
    f32::from_bits(bits)
}

fn read_str32(r: &mut Reader) -> Result<String> {
    let n = r.u32()? as usize;
    if n > 1024 {
        return Err(Error::format(format!(
            "string length {n} too long at {}",
            r.pos()
        )));
    }
    r.str(n)
}

fn read_material(r: &mut Reader) -> Result<Material> {
    Ok(Material {
        diffuse: read_str32(r)?,
        normal: read_str32(r)?,
        properties: read_str32(r)?,
        deformed_diffuse: read_str32(r)?,
        deformed_normal: read_str32(r)?,
        deformed_properties: read_str32(r)?,
        normal_ex: read_str32(r)?,
        shadow: read_str32(r)?,
        flags: r.u32()?,
    })
}

fn read_faces(r: &mut Reader) -> Result<Vec<u16>> {
    let n = r.i32()?;
    if n < 0 {
        return Err(Error::format("negative face count"));
    }
    let n = n as usize;
    if !n.is_multiple_of(3) {
        return Err(Error::format(format!(
            "face index count {n} not a multiple of 3"
        )));
    }
    let bytes = r.bytes(n * 2)?;
    Ok(match r.endian {
        Endian::Little => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect(),
        Endian::Big => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect(),
    })
}

fn read_count(r: &mut Reader, elem_size: usize) -> Result<usize> {
    let n = r.i32()?;
    if n < 0 {
        return Err(Error::format("negative vertex count"));
    }
    let n = n as usize;
    if n * elem_size > r.remaining() {
        return Err(Error::format(format!(
            "vertex array of {n} x {elem_size} bytes exceeds data at {}",
            r.pos()
        )));
    }
    Ok(n)
}

/// The sibling model a Merged mode-0 block (instance list) draws from: `<stem>_base_<N>.rbm`
/// with `N` the block's `flags` word (`params[1]`), `<stem>` the model's name without its
/// `_batchN` / `_partN` suffix. Observed in retail: the base model's own `flags` equals its
/// `N` and every instance range fits its index list. `None` for any other block or name.
pub fn merged_base_name(model_name: &str, block: &RenderBlock) -> Option<String> {
    if block.type_name != "Merged" || block.instances.is_empty() {
        return None;
    }
    let lower = model_name.to_ascii_lowercase();
    let stem_end = lower.rfind("_batch").or_else(|| lower.rfind("_part"))?;
    let flags = *block.params.get(1)?;
    (flags >= 0.0 && flags.fract() == 0.0).then(|| format!("{}_base_{}.rbm", &model_name[..stem_end], flags as u32))
}

/// Merged mode 0 (`*_batchN`, `*_partN`): every instance draws `extra` triangles of the base
/// model's index list starting at index `part`, placed by its 3x4 matrix. Native draw loop of
/// the Merged block `0x9458a0` (stride 0x34 per instance: matrix rows set as constants, then
/// the draw call `0x9263e0` with the two `u16` fields at +0x30/+0x32 as start index and
/// triangle count, as its other caller passes (0, triangles)); the `part`/`extra` roles are
/// confirmed by retail data (start + 3 * count never exceeds the base model's index count,
/// and reaches it exactly for the last range). Returns the instances as ordinary geometry,
/// split so each block stays within 16-bit indices; instances that fall outside the base
/// model are skipped.
pub fn expand_merged_instances(batch: &RenderBlock, base: &RenderBlock) -> Vec<RenderBlock> {
    let src = &base.mesh;
    let mut out: Vec<RenderBlock> = Vec::new();
    let fresh = || {
        let mut b = batch.clone();
        b.instances.clear();
        b.mesh = MeshData::default();
        b.params = base.params.clone();
        b
    };
    let mut cur = fresh();
    for inst in &batch.instances {
        let start = inst.part as usize;
        let end = start + inst.extra as usize * 3;
        if end > src.indices.len() {
            continue;
        }
        let m = &inst.matrix;
        let place = |p: [f32; 3]| {
            [
                m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
                m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
                m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
            ]
        };
        let turn = |n: [f32; 3]| {
            [
                m[0] * n[0] + m[1] * n[1] + m[2] * n[2],
                m[4] * n[0] + m[5] * n[1] + m[6] * n[2],
                m[8] * n[0] + m[9] * n[1] + m[10] * n[2],
            ]
        };
        let tris = &src.indices[start..end];
        if cur.mesh.positions.len() + tris.len() > u16::MAX as usize {
            out.push(std::mem::replace(&mut cur, fresh()));
        }
        // one copy of each distinct source vertex per instance
        let mut remap = std::collections::HashMap::new();
        for &i in tris {
            let i = i as usize;
            let to = *remap.entry(i).or_insert_with(|| {
                let n = cur.mesh.positions.len() as u16;
                cur.mesh.positions.push(place(src.positions[i]));
                if let Some(v) = src.normals.get(i) {
                    cur.mesh.normals.push(turn(*v));
                }
                if let Some(v) = src.uv0.get(i) {
                    cur.mesh.uv0.push(*v);
                }
                if let Some(v) = src.uv1.get(i) {
                    cur.mesh.uv1.push(*v);
                }
                n
            });
            cur.mesh.indices.push(to);
        }
    }
    if !cur.mesh.indices.is_empty() {
        out.push(cur);
    }
    out
}

/// The model-space bounds in an `.rbm` header, without reading its blocks.
pub fn header_bounds(data: &[u8]) -> Result<([f32; 3], [f32; 3])> {
    let mut r = Reader::new(data);
    let ml = r.u32()?;
    let endian = if ml == 5 {
        Endian::Little
    } else if ml.swap_bytes() == 5 {
        Endian::Big
    } else {
        return Err(Error::format("not an RBMDL file"));
    };
    if r.bytes(5)? != b"RBMDL" {
        return Err(Error::format("not an RBMDL file (magic)"));
    }
    r.endian = endian;
    r.bytes(12)?;
    Ok((
        [r.f32()?, r.f32()?, r.f32()?],
        [r.f32()?, r.f32()?, r.f32()?],
    ))
}

impl RbmModel {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let ml = r.u32()?;
        let endian = if ml == 5 {
            Endian::Little
        } else if ml.swap_bytes() == 5 {
            Endian::Big
        } else {
            return Err(Error::format("not an RBMDL file"));
        };
        if r.bytes(5)? != b"RBMDL" {
            return Err(Error::format("not an RBMDL file (magic)"));
        }
        r.endian = endian;
        let version = (r.i32()?, r.i32()?, r.i32()?);
        if version.0 != 1 || version.1 != 13 {
            return Err(Error::format(format!(
                "unsupported RBMDL version {}.{}.{}",
                version.0, version.1, version.2
            )));
        }
        let bbox_min = [r.f32()?, r.f32()?, r.f32()?];
        let bbox_max = [r.f32()?, r.f32()?, r.f32()?];
        let count = r.i32()?.max(0) as usize;
        let mut model = Self {
            version,
            bbox_min,
            bbox_max,
            blocks: Vec::with_capacity(count),
            truncated_at: None,
        };
        for i in 0..count {
            let type_hash = r.u32()?;
            let name = block_type_name(type_hash)
                .map(str::to_string)
                .unwrap_or_else(|| format!("0x{type_hash:08x}"));
            let block = match name.as_str() {
                "CarPaint" => parse_car_paint(&mut r),
                "CarPaintSimple" => parse_car_paint_simple(&mut r),
                "DeformableWindow" => parse_deformable_window(&mut r),
                "General" => parse_general(&mut r),
                "Lambert" => parse_lambert(&mut r),
                "SkinnedGeneral" => parse_skinned_general(&mut r),
                "Facade" => parse_facade(&mut r),
                "Window" => parse_window(&mut r),
                "VegetationBark" => parse_vegetation_bark(&mut r),
                "VegetationFoliage" => parse_vegetation_foliage(&mut r),
                "BillboardFoliage" => parse_billboard_foliage(&mut r),
                "Halo" => parse_halo(&mut r),
                "Merged" => parse_merged(&mut r),
                _ => {
                    model.truncated_at = Some((i, name));
                    break;
                }
            };
            let mut block = block.map_err(|e| Error::format(format!("block {i} ({name}): {e}")))?;
            block.type_name = name;
            block.type_hash = type_hash;
            let footer = r.u32()?;
            if footer != FOOTER {
                return Err(Error::format(format!(
                    "block {i} ({}): bad footer {footer:#x} at {} — layout misread",
                    block.type_name,
                    r.pos() - 4
                )));
            }
            model.blocks.push(block);
        }
        Ok(model)
    }

    pub fn vertex_count(&self) -> usize {
        self.blocks.iter().map(|b| b.mesh.vertex_count()).sum()
    }
    pub fn triangle_count(&self) -> usize {
        self.blocks.iter().map(|b| b.mesh.triangle_count()).sum()
    }
}

fn new_block(version: u8, material: Material, params: Vec<f32>) -> RenderBlock {
    RenderBlock {
        type_name: String::new(),
        type_hash: 0,
        version,
        material,
        mesh: MeshData::default(),
        params,
        foliage_flags: None,
        general: None,
        extra_attribute_streams: 0,
        instances: Vec::new(),
    }
}

/// CarPaint v3/v4: vehicle body panels.
/// header: 13 f32 + u32; [v4: deform table 256 u32]; material;
/// data0[n]: f32 xyz + f32 w(packed deform weights) + 4 i16 (deform vectors)
/// data1[n]: f32 u, v, light_index/-1, packed normal, packed deformed normal, packed tangent (signed), packed bitangent
/// faces; [v<=3: deform table 256 u32]
fn parse_car_paint(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if !(1..=4).contains(&version) {
        return Err(Error::format(format!(
            "CarPaint version {version} unhandled"
        )));
    }
    if version < 3 {
        return Err(Error::format(format!(
            "CarPaint version {version} vertex layout unknown"
        )));
    }
    let mut params = Vec::with_capacity(14);
    for _ in 0..13 {
        params.push(r.f32()?);
    }
    params.push(r.u32()? as f32);
    if version >= 4 {
        r.bytes(256 * 4)?;
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    let n0 = read_count(r, 24)?;
    let mut positions = Vec::with_capacity(n0);
    for _ in 0..n0 {
        positions.push([r.f32()?, r.f32()?, r.f32()?]);
        r.f32()?; // packed deform weights
        r.bytes(8)?; // 4 x i16 deform data
    }
    let n1 = read_count(r, 28)?;
    let mut uv0 = Vec::with_capacity(n1);
    let mut normals = Vec::with_capacity(n1);
    let mut tangents = Vec::with_capacity(n1);
    for _ in 0..n1 {
        let u = r.f32()?;
        let v = r.f32()?;
        let _light = r.f32()?;
        let (n, _) = unpack_normal(r.f32()?);
        let _deformed_normal = r.f32()?;
        let (t, s) = unpack_normal(r.f32()?);
        let _bitangent = r.f32()?;
        uv0.push([u, v]);
        normals.push(n);
        tangents.push([t[0], t[1], t[2], s]);
    }
    b.mesh.indices = read_faces(r)?;
    if version <= 3 {
        r.bytes(256 * 4)?;
    }
    if n1 != n0 {
        // streams are per-vertex and must agree; keep geometry, drop attributes
        uv0.clear();
        normals.clear();
        tangents.clear();
    }
    b.mesh.positions = positions;
    b.mesh.uv0 = uv0;
    b.mesh.normals = normals;
    b.mesh.tangents = tangents;
    b.extra_attribute_streams = 2;
    Ok(b)
}

/// CarPaintSimple v1: simpler body parts (no deformation).
/// header: 9 f32 + 5 u32; 8 texture names + u32; vertices[n]: f32 xyz w, f32 u v, packed normal, packed tangent; faces
fn parse_car_paint_simple(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 1 {
        return Err(Error::format(format!(
            "CarPaintSimple version {version} unhandled"
        )));
    }
    // 14 floats (the reference tools read the last five as u32, but they hold
    // float bit patterns such as 0.2 / 0.4 / 1.0)
    let mut params = Vec::with_capacity(14);
    for _ in 0..14 {
        params.push(r.f32()?);
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    let n = read_count(r, 32)?;
    for _ in 0..n {
        b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
        r.f32()?;
        b.mesh.uv0.push([r.f32()?, r.f32()?]);
        let (nn, _) = unpack_normal(r.f32()?);
        let (t, s) = unpack_normal(r.f32()?);
        b.mesh.normals.push(nn);
        b.mesh.tangents.push([t[0], t[1], t[2], s]);
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// DeformableWindow v0-2: glass.
/// material; [v2: u32 + deform table]; vertices[n]: f32 xyz w, 4 i16 deform, f32 packed normal,
/// packed deformed normal, packed tangent, packed bitangent, f32 u v; faces; [v0/1: deform table]; [v1: u32]
fn parse_deformable_window(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version > 2 {
        return Err(Error::format(format!(
            "DeformableWindow version {version} unhandled"
        )));
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, Vec::new());
    if version >= 2 {
        b.params.push(r.u32()? as f32);
        r.bytes(256 * 4)?;
    }
    let n = read_count(r, 48)?;
    for _ in 0..n {
        b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
        r.f32()?;
        r.bytes(8)?;
        let (nn, _) = unpack_normal(r.f32()?);
        let _deformed = r.f32()?;
        let (t, s) = unpack_normal(r.f32()?);
        let _bitangent = r.f32()?;
        b.mesh.uv0.push([r.f32()?, r.f32()?]);
        b.mesh.normals.push(nn);
        b.mesh.tangents.push([t[0], t[1], t[2], s]);
    }
    b.mesh.indices = read_faces(r)?;
    if version <= 1 {
        r.bytes(256 * 4)?;
    }
    if version == 1 {
        b.params.push(r.u32()? as f32);
    }
    b.extra_attribute_streams = 1;
    Ok(b)
}

/// Shared vertex layout of General and Lambert.
/// big:   f32 xyz, f32 u0 v0 u1 v1, packed normal, packed tangent, packed colour
/// small: i16 u0 v0 u1 v1 (snorm, scaled by uv scales), packed normal, packed tangent, packed colour, i16 xyz w (snorm * scale)
fn read_general_vertices(
    r: &mut Reader,
    big: bool,
    scale: f32,
    uv0_scale: [f32; 2],
    uv1_scale: [f32; 2],
    mesh: &mut MeshData,
) -> Result<()> {
    if big {
        let n = read_count(r, 40)?;
        for _ in 0..n {
            mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
            mesh.uv0.push([r.f32()?, r.f32()?]);
            mesh.uv1.push([r.f32()?, r.f32()?]);
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, s) = unpack_normal(r.f32()?);
            mesh.packed_colours.push(r.u32()?);
            mesh.normals.push(nn);
            mesh.tangents.push([t[0], t[1], t[2], s]);
        }
    } else {
        let n = read_count(r, 28)?;
        for _ in 0..n {
            let (a, b, c, d) = (
                snorm(r.i16()?),
                snorm(r.i16()?),
                snorm(r.i16()?),
                snorm(r.i16()?),
            );
            mesh.uv0.push([a * uv0_scale[0], b * uv0_scale[1]]);
            mesh.uv1.push([c * uv1_scale[0], d * uv1_scale[1]]);
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, s) = unpack_normal(r.f32()?);
            mesh.packed_colours.push(r.u32()?);
            let (x, y, z) = (snorm(r.i16()?), snorm(r.i16()?), snorm(r.i16()?));
            let _w = r.i16()?;
            mesh.positions.push([x * scale, y * scale, z * scale]);
            mesh.normals.push(nn);
            mesh.tangents.push([t[0], t[1], t[2], s]);
        }
    }
    Ok(())
}

/// General v2/v3: static props and vehicle details.
/// header: 10 f32, u32 big_vertices(0=big), 6 f32 (vertex scale, uv0 scale xy, uv1 scale xy, ?), [v3: 2 i32]; material; vertices; faces
fn parse_general(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if !(2..=3).contains(&version) {
        return Err(Error::format(format!(
            "General version {version} unhandled"
        )));
    }
    let mut params = Vec::with_capacity(19);
    for _ in 0..10 {
        params.push(r.f32()?);
    }
    let flag = r.u32()?;
    params.push(flag as f32);
    let mut scales = [0f32; 6];
    for s in &mut scales {
        *s = r.f32()?;
        params.push(*s);
    }
    let (packed_colour, render_flags) = if version == 3 {
        let colour = r.u32()?;
        let flags = r.u32()?;
        // Legacy params remain diagnostic only; typed fields preserve all bits.
        params.push(colour as i32 as f32);
        params.push(flags as i32 as f32);
        (colour, flags)
    } else {
        // Reader00973da0 defaults the absent v2 fields to ffffffff/40.
        (u32::MAX, 0x40)
    };
    let header = GeneralHeader {
        channel_texture_mask: params[0..4].try_into().unwrap(),
        channel_ao_mask: params[4..7].try_into().unwrap(),
        channel_ao_factor: params[7],
        depth_offset: params[8],
        specular_power: params[9],
        vertex_format: flag,
        position_extent: scales[0],
        uv0_extent: [scales[1], scales[2]],
        uv1_extent: [scales[3], scales[4]],
        colour_extent: scales[5],
        packed_colour,
        render_flags,
    };
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    b.general = Some(header);
    let big = flag == 0;
    read_general_vertices(
        r,
        big,
        scales[0],
        [scales[1], scales[2]],
        [scales[3], scales[4]],
        &mut b.mesh,
    )?;
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// Lambert v4: unlit/simple geometry (building details, props).
/// header: u32 small_vertices(1=small), f32 vertex scale, f32 uv0 scale xy, f32 uv1 scale xy, 5 unused f32; material; vertices; faces
fn parse_lambert(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 4 {
        return Err(Error::format(format!(
            "Lambert version {version} unhandled"
        )));
    }
    let flag = r.u32()?;
    let mut params = vec![flag as f32];
    let mut floats = [0f32; 10];
    for f in &mut floats {
        *f = r.f32()?;
        params.push(*f);
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    let big = flag == 0;
    read_general_vertices(
        r,
        big,
        floats[0],
        [floats[1], floats[2]],
        [floats[3], floats[4]],
        &mut b.mesh,
    )?;
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// SkinnedGeneral v0/v3: characters.
/// u32 flags (0x80000 = big vertices, v3 only), 6 f32; material;
/// data0[n]: f32 xyz + 8 bytes (bone weights/indices) [+ 8 bytes when big];
/// data1[n]: u32 packed normal/tangent/bitangent, f32 u v; skin batches; faces
fn parse_skinned_general(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 && version != 3 {
        return Err(Error::format(format!(
            "SkinnedGeneral version {version} unhandled"
        )));
    }
    let flags = r.u32()?;
    let mut params = vec![flags as f32];
    for _ in 0..6 {
        params.push(r.f32()?);
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    let big = version >= 3 && (flags & 0x80000) != 0;
    let stride = if big { 28 } else { 20 };
    let n0 = read_count(r, stride)?;
    // per vertex: 4 weights then 4 batch-local bone indices (bytes); "big" vertices
    // carry a second set of four, which is not used yet
    let mut local_joints: Vec<[u8; 4]> = Vec::with_capacity(n0);
    for _ in 0..n0 {
        b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
        let w = r.bytes(4)?;
        let j = r.bytes(4)?;
        b.mesh.weights.push([
            w[0] as f32 / 255.0,
            w[1] as f32 / 255.0,
            w[2] as f32 / 255.0,
            w[3] as f32 / 255.0,
        ]);
        local_joints.push([j[0], j[1], j[2], j[3]]);
        if big {
            // "big" vertices (`0x80000`: Rico's head, neck and chest skin) keep their four
            // batch-local bone indices in the FIRST four of these eight bytes (retail Rico:
            // the four bytes after the weights are identical for every vertex and would bind
            // the whole head rigidly to one bone; the first four extra bytes pair with the
            // weights, a zero weight with index 0 or 13). The other eight bytes (four after
            // the weights, four after the indices) are unknown (gap, `rbm.md`).
            let e = r.bytes(8)?;
            let last = local_joints.len() - 1;
            local_joints[last] = [e[0], e[1], e[2], e[3]];
        }
    }
    let n1 = read_count(r, 20)?;
    for _ in 0..n1 {
        let n = r.u32()?;
        let t = r.u32()?;
        let bt = r.u32()?;
        b.mesh.uv0.push([r.f32()?, r.f32()?]);
        // bytes x,y,z of the u32 map to [-1,1]
        let nn = unpack_bytes3(n);
        b.mesh.normals.push(nn);
        // tangent frame (board 2280444f): handedness from the stored bitangent, B = sign * N x T
        let tt = unpack_bytes3(t);
        let bb = unpack_bytes3(bt);
        let cross = [
            nn[1] * tt[2] - nn[2] * tt[1],
            nn[2] * tt[0] - nn[0] * tt[2],
            nn[0] * tt[1] - nn[1] * tt[0],
        ];
        let sign = if cross[0] * bb[0] + cross[1] * bb[1] + cross[2] * bb[2] < 0.0 { -1.0 } else { 1.0 };
        b.mesh.tangents.push([tt[0], tt[1], tt[2], sign]);
    }
    // skin batches: a run of faces and the skeleton bones their local indices mean
    let batches = r.i32()?.max(0) as usize;
    let mut batch_list = Vec::with_capacity(batches);
    for _ in 0..batches {
        let face_count = r.i32()?.max(0) as usize;
        let face_index = r.i32()?.max(0) as usize;
        let bones = r.i32()?.max(0) as usize;
        let mut map = Vec::with_capacity(bones);
        for _ in 0..bones {
            map.push(r.u16()?);
        }
        batch_list.push((face_index, face_count, map));
    }
    b.mesh.indices = read_faces(r)?;
    if n1 != n0 {
        b.mesh.uv0.clear();
        b.mesh.normals.clear();
        b.mesh.tangents.clear();
    }
    // resolve the local joints through the batch each vertex is drawn by
    let mut joints = vec![[0u16; 4]; n0];
    let mut seen = vec![false; n0];
    for (face_index, face_count, map) in &batch_list {
        let start = face_index * 3;
        let end = (start + face_count * 3).min(b.mesh.indices.len());
        for &vi in b.mesh.indices.get(start..end).unwrap_or(&[]) {
            let vi = vi as usize;
            if vi >= n0 || seen[vi] {
                continue;
            }
            seen[vi] = true;
            let l = local_joints[vi];
            joints[vi] = [
                map.get(l[0] as usize).copied().unwrap_or(0),
                map.get(l[1] as usize).copied().unwrap_or(0),
                map.get(l[2] as usize).copied().unwrap_or(0),
                map.get(l[3] as usize).copied().unwrap_or(0),
            ];
        }
    }
    if batch_list.is_empty() {
        for (j, l) in joints.iter_mut().zip(&local_joints) {
            *j = [l[0] as u16, l[1] as u16, l[2] as u16, l[3] as u16];
        }
    }
    b.mesh.joints = joints;
    b.extra_attribute_streams = if big { 1 } else { 0 };
    Ok(b)
}

/// Facade v1: building fronts.
/// header: 12 f32 (f[11] = 16.0), u32 small_vertices, f32 vertex_scale, u32; material; vertices; faces
/// small (40 B): i16 x y z w (snorm × scale); f32 u0 v0 u1 v1; packed normal, tangent, colour; f32 extra
/// big   (44 B): f32 x y z; f32 u0 v0 u1 v1; packed normal, tangent, colour; f32 extra
fn parse_facade(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 1 {
        return Err(Error::format(format!("Facade version {version} unhandled")));
    }
    let mut params = Vec::with_capacity(15);
    for _ in 0..12 {
        params.push(r.f32()?);
    }
    let flag = r.u32()?;
    params.push(flag as f32);
    let scale = r.f32()?;
    params.push(scale);
    params.push(r.u32()? as f32);
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    if flag == 0 {
        let n = read_count(r, 44)?;
        for _ in 0..n {
            b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
            b.mesh.uv0.push([r.f32()?, r.f32()?]);
            b.mesh.uv1.push([r.f32()?, r.f32()?]);
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, sg) = unpack_normal(r.f32()?);
            let _colour = r.f32()?;
            let _extra = r.f32()?;
            b.mesh.normals.push(nn);
            b.mesh.tangents.push([t[0], t[1], t[2], sg]);
        }
    } else {
        let n = read_count(r, 40)?;
        for _ in 0..n {
            let (x, y, z) = (snorm(r.i16()?), snorm(r.i16()?), snorm(r.i16()?));
            let _w = r.i16()?;
            b.mesh.positions.push([x * scale, y * scale, z * scale]);
            b.mesh.uv0.push([r.f32()?, r.f32()?]);
            b.mesh.uv1.push([r.f32()?, r.f32()?]);
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, sg) = unpack_normal(r.f32()?);
            let _colour = r.f32()?;
            let _extra = r.f32()?;
            b.mesh.normals.push(nn);
            b.mesh.tangents.push([t[0], t[1], t[2], sg]);
        }
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// Window v0: building glass. header: f32 (16.0), u32; material; General big vertices (40 B); faces
fn parse_window(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!("Window version {version} unhandled")));
    }
    let params = vec![r.f32()?, r.u32()? as f32];
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    read_general_vertices(r, true, 1.0, [1.0, 1.0], [1.0, 1.0], &mut b.mesh)?;
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// VegetationBark v0: trunks and rocks. u32 flags; material; vertices; faces
/// flags & 4 == 0 (40 B): f32 x y z; f32 u0 v0 (tiling bark); f32 u1 v1; packed normal, tangent, bitangent
/// flags & 4 != 0 (32 B): f32 x y z; **packed colour**, packed normal, packed tangent (sign = bitangent sign); f32 u v
///
/// The 32 B order is the input layout of the native `vegetationbark` vertex shader (JC2-lighting
/// research `research/formats/lighting-gaps-next.md`): `v0 = (x y z, w)` with `w` the packed vertex colour
/// (`frc(w * (1, 1/256, 1/65536))`), `v1 = (packed normal -> o9, packed tangent with sign -> o7 / o8, u, v)`.
/// Checked on the retail Jungle canopy trunks: the second packed float agrees with the geometric normal
/// (median |dot| 0.87 to 0.997 over four blocks), the first does not (0.44 to 0.56) and its decoded colour
/// spans 0.28..1.0. Earlier code read the colour as the normal and the normal as the tangent.
fn parse_vegetation_bark(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!(
            "VegetationBark version {version} unhandled"
        )));
    }
    let flags = r.u32()?;
    let material = read_material(r)?;
    let mut b = new_block(version, material, vec![flags as f32]);
    if flags & 4 == 0 {
        let n = read_count(r, 40)?;
        for _ in 0..n {
            b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
            b.mesh.uv0.push([r.f32()?, r.f32()?]);
            b.mesh.uv1.push([r.f32()?, r.f32()?]);
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, sg) = unpack_normal(r.f32()?);
            let _bt = r.f32()?;
            b.mesh.normals.push(nn);
            b.mesh.tangents.push([t[0], t[1], t[2], sg]);
        }
    } else {
        let n = read_count(r, 32)?;
        for _ in 0..n {
            b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
            b.mesh.packed_colours.push(r.f32()?.to_bits());
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, sg) = unpack_normal(r.f32()?);
            b.mesh.uv0.push([r.f32()?, r.f32()?]);
            b.mesh.normals.push(nn);
            b.mesh.tangents.push([t[0], t[1], t[2], sg]);
        }
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// VegetationFoliage v0: leaves. header 5 f32 + u32 flags; material; 40 B vertices: f32 x y z; f32 u0 v0 u1 v1; packed normal, tangent, bitangent; faces
fn parse_vegetation_foliage(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!(
            "VegetationFoliage version {version} unhandled"
        )));
    }
    let mut params = Vec::with_capacity(5);
    for _ in 0..5 {
        params.push(r.f32()?);
    }
    let flags = r.u32()?;
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
    b.foliage_flags = Some(flags);
    let n = read_count(r, 40)?;
    for _ in 0..n {
        b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
        b.mesh.uv0.push([r.f32()?, r.f32()?]);
        b.mesh.uv1.push([r.f32()?, r.f32()?]);
        let (nn, _) = unpack_normal(r.f32()?);
        let (t, _) = unpack_normal(r.f32()?);
        let (bt, _) = unpack_normal(r.f32()?);
        // The VS (`vegetationfoliage`) decodes tangent and bitangent as two independent packed vectors
        // (o3, o4); Bevy rebuilds B = w * cross(N, T), so w is the handedness of the stored bitangent.
        let c = [
            nn[1] * t[2] - nn[2] * t[1],
            nn[2] * t[0] - nn[0] * t[2],
            nn[0] * t[1] - nn[1] * t[0],
        ];
        let w = if c[0] * bt[0] + c[1] * bt[1] + c[2] * bt[2] < 0.0 { -1.0 } else { 1.0 };
        b.mesh.normals.push(nn);
        b.mesh.tangents.push([t[0], t[1], t[2], w]);
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// BillboardFoliage v0: camera-facing canopy sprites. material; 28 B vertices: f32 centre x y z; f32 u v;
/// f32 offset x y (corner offset in view space); faces. Positions here are the centres plus the
/// offset applied in the XY plane so the bounds and the legacy route have area; `uv1` carries
/// the corner offset itself (the native vertex input `v1.zw`), so the centre is
/// `position - (uv1, 0)` and the native vertex shader (`foliage_billboard.wgsl`) can rebuild
/// the camera-facing card.
fn parse_billboard_foliage(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!(
            "BillboardFoliage version {version} unhandled"
        )));
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, Vec::new());
    let n = read_count(r, 28)?;
    for _ in 0..n {
        let (x, y, z) = (r.f32()?, r.f32()?, r.f32()?);
        let uv = [r.f32()?, r.f32()?];
        let (ox, oy) = (r.f32()?, r.f32()?);
        b.mesh.positions.push([x + ox, y + oy, z]);
        b.mesh.uv0.push(uv);
        b.mesh.uv1.push([ox, oy]);
    }
    b.mesh.indices = read_faces(r)?;
    b.extra_attribute_streams = 1;
    Ok(b)
}

/// Halo v0: light glow sprites. material; 20 B vertices: f32 x y z + 8 bytes (colour / size, undecoded); faces.
/// Kept as point positions; the viewer does not draw halos.
fn parse_halo(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!("Halo version {version} unhandled")));
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, Vec::new());
    let n = read_count(r, 20)?;
    for _ in 0..n {
        b.mesh.positions.push([r.f32()?, r.f32()?, r.f32()?]);
        r.bytes(8)?;
    }
    b.mesh.indices = read_faces(r)?;
    b.extra_attribute_streams = 1;
    Ok(b)
}

/// Merged v0: pre-merged distant geometry (city districts, roads, bridges).
/// material; u8 mode; u32 flags;
/// mode 1 (`*_base_N`, `*_part_N`): i32 n; n × { f32 x y z; packed normal; f16 u0 v0; f16 u1 v1 }; faces.
///   Entry 0 of the vertex array is an in-memory header the exporter wrote out; faces start at 1.
/// mode 0 (`*_batchN`): i32 n; n × { f32 3×4 matrix; u16 part; u16 extra } — instance placements, no geometry.
fn parse_merged(r: &mut Reader) -> Result<RenderBlock> {
    let version = r.u8()?;
    if version != 0 {
        return Err(Error::format(format!("Merged version {version} unhandled")));
    }
    let material = read_material(r)?;
    let mode = r.u8()?;
    let flags = r.u32()?;
    let mut b = new_block(version, material, vec![mode as f32, flags as f32]);
    if mode == 0 {
        let n = read_count(r, 52)?;
        for _ in 0..n {
            let mut m = [0f32; 12];
            for v in &mut m {
                *v = r.f32()?;
            }
            let part = r.u16()?;
            let extra = r.u16()?;
            b.instances.push(Instance {
                matrix: m,
                part,
                extra,
            });
        }
        return Ok(b);
    }
    let n = read_count(r, 24)?;
    for i in 0..n {
        let p = [r.f32()?, r.f32()?, r.f32()?];
        let nn = unpack_normal(r.f32()?).0;
        let uv0 = [half_to_f32(r.u16()?), half_to_f32(r.u16()?)];
        let uv1 = [half_to_f32(r.u16()?), half_to_f32(r.u16()?)];
        if i == 0 {
            // dummy header entry
            b.mesh.positions.push([0.0; 3]);
            b.mesh.normals.push([0.0, 1.0, 0.0]);
            b.mesh.uv0.push([0.0; 2]);
            b.mesh.uv1.push([0.0; 2]);
        } else {
            b.mesh.positions.push(p);
            b.mesh.normals.push(nn);
            b.mesh.uv0.push(uv0);
            b.mesh.uv1.push(uv1);
        }
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_typed_header_preserves_integer_bits_and_native_v2_defaults() {
        for version in [2, 3] {
            let mut bytes = vec![version];
            for value in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 120.0] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(1u32.to_le_bytes());
            for value in [1.0f32, 2.0, 3.0, 4.0, 5.0, 0.4241] {
                bytes.extend(value.to_le_bytes());
            }
            if version == 3 {
                bytes.extend(0xff959515u32.to_le_bytes());
                bytes.extend(0x80000221u32.to_le_bytes());
            }
            bytes.extend([0u8; 32]); // eight empty texture names
            bytes.extend(3u32.to_le_bytes()); // distinct texture trailer
            bytes.extend([0u8; 8]); // no vertices/faces
            let block = parse_general(&mut Reader::new(&bytes)).unwrap();
            let header = block.general.unwrap();
            assert_eq!(header.specular_power, 120.0);
            assert_eq!(header.vertex_format, 1);
            assert_eq!(header.uv0_extent, [2.0, 3.0]);
            assert_eq!(header.uv1_extent, [4.0, 5.0]);
            assert_eq!(header.colour_extent, 0.4241);
            assert_eq!(block.material.flags, 3);
            if version == 3 {
                assert_eq!(header.packed_colour, 0xff959515);
                assert_eq!(header.render_flags, 0x80000221);
                assert_eq!(header.variant_key(), 3 | 16);
                assert!(header.no_culling());
            } else {
                assert_eq!(header.packed_colour, u32::MAX);
                assert_eq!(header.render_flags, 0x40);
                assert_eq!(header.variant_key(), 0);
            }
        }
        // Literal VS188280 fractional decode, with no colour-extent multiplier.
        assert_eq!(
            general_vertex_colour(3104.25f32.to_bits()),
            [0.25, 0.50390625, 0.75787354, 1.0]
        );
        assert_eq!(
            general_vertex_colour((-0.25f32).to_bits()),
            [0.75, 0.99609375, 0.99993896, 1.0]
        );
    }

    /// Board 2280444f (`rico-model-detail.md`): every Rico `SkinnedGeneral` vertex carries a unit tangent
    /// orthogonal to its normal and a handedness sign.
    #[test]
    fn rico_skinned_general_tangents_are_decoded() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated("mc01_lod1-rico.rbm").unwrap();
        let model = RbmModel::parse(&bytes).unwrap();
        assert_eq!(model.blocks.len(), 16);
        for block in &model.blocks {
            let m = &block.mesh;
            assert_eq!(m.tangents.len(), m.positions.len());
            // individual vertices may be off (up to ~0.4 seen); the mean is what the shader frame needs
            let mut sum_dot = 0.0;
            for (t, n) in m.tangents.iter().zip(&m.normals) {
                let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
                assert!((0.9..1.1).contains(&len), "tangent length {len}");
                sum_dot += (t[0] * n[0] + t[1] * n[1] + t[2] * n[2]).abs();
                assert!(t[3] == 1.0 || t[3] == -1.0);
            }
            assert!(sum_dot / (m.tangents.len() as f32) < 0.05, "mean |T.N| {}", sum_dot / m.tangents.len() as f32);
        }
    }

    /// Lighting research (7625eb04): the VegetationFoliage tangent follows the UV u direction (the
    /// native normal map basis is the stored tangent/bitangent pair) and the handedness `w` derived
    /// from the stored bitangent is consistent with the UV layout (always opposite to the sign of
    /// `cross(N, T) . dP/dv`, the v axis running down the image).
    #[test]
    fn jungle_vegetation_foliage_basis_follows_the_uv_layout() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated("vegetation_0.blz").unwrap();
        let archive = crate::sarc::SmallArchive::parse(&bytes).unwrap();
        for name in ["jungle_T16_palmCLL_LOD1-Whole.rbm"] {
            let model = RbmModel::parse(archive.data(&bytes, archive.get(name).unwrap()).unwrap()).unwrap();
            for block in &model.blocks {
                if block.type_name != "VegetationFoliage" {
                    continue;
                }
                let m = &block.mesh;
                let n = m.positions.len();
                let mut dpdu = vec![[0.0f32; 3]; n];
                let mut dpdv = vec![[0.0f32; 3]; n];
                for t in m.indices.as_chunks::<3>().0 {
                    let i = [t[0] as usize, t[1] as usize, t[2] as usize];
                    let e1: Vec<f32> = (0..3).map(|k| m.positions[i[1]][k] - m.positions[i[0]][k]).collect();
                    let e2: Vec<f32> = (0..3).map(|k| m.positions[i[2]][k] - m.positions[i[0]][k]).collect();
                    let (du1, dv1) = (m.uv0[i[1]][0] - m.uv0[i[0]][0], m.uv0[i[1]][1] - m.uv0[i[0]][1]);
                    let (du2, dv2) = (m.uv0[i[2]][0] - m.uv0[i[0]][0], m.uv0[i[2]][1] - m.uv0[i[0]][1]);
                    let det = du1 * dv2 - du2 * dv1;
                    if det.abs() < 1e-12 {
                        continue;
                    }
                    for k in 0..3 {
                        let pu = (e1[k] * dv2 - e2[k] * dv1) / det;
                        let pv = (e2[k] * du1 - e1[k] * du2) / det;
                        for &v in &i {
                            dpdu[v][k] += pu;
                            dpdv[v][k] += pv;
                        }
                    }
                }
                let dot = |a: &[f32; 3], b: &[f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
                let norm = |a: [f32; 3]| {
                    let l = dot(&a, &a).sqrt().max(1e-9);
                    [a[0] / l, a[1] / l, a[2] / l]
                };
                let (mut tu, mut tn, mut pos, mut neg, mut agree_w) = (0.0f64, 0.0f64, 0, 0, 0);
                for v in 0..n {
                    let t = [m.tangents[v][0], m.tangents[v][1], m.tangents[v][2]];
                    let nn = m.normals[v];
                    tn += dot(&t, &nn).abs() as f64;
                    let du = norm(dpdu[v]);
                    let d = dot(&t, &du);
                    tu += d as f64;
                    if d > 0.0 { pos += 1 } else { neg += 1 }
                    // handedness of the uv map vs w
                    let c = [
                        nn[1] * t[2] - nn[2] * t[1],
                        nn[2] * t[0] - nn[0] * t[2],
                        nn[0] * t[1] - nn[1] * t[0],
                    ];
                    let dv = dpdv[v];
                    let hand = if dot(&c, &dv) >= 0.0 { 1.0 } else { -1.0 };
                    if hand == m.tangents[v][3] { agree_w += 1 }
                }
                assert_eq!(block.material.normal, "jungle_fanpalm_foliage_nrm.dds");
                assert_eq!(block.material.properties, "dummy_foliage128_aoc.dds", "t2 = the properties slot");
                assert!(tn / (n as f64) < 0.05, "{name}: tangent is not orthogonal to the normal");
                assert!(tu / (n as f64) > 0.95 && neg == 0, "{name}: tangent does not follow dP/du ({pos}/{neg})");
                assert_eq!(agree_w, 0, "{name}: the stored-bitangent handedness is inconsistent with the UV layout");
            }
        }
    }

    #[test]
    fn vegetation_bark_colour_is_the_vertex_shader_fraction_chain() {
        // frc(w * (1, 1/256, 1/65536)): 0.5 + 3 / 256 in the second byte, 2 / 65536 in the third
        let w: f32 = 515.5;
        let c = vegetation_bark_colour(w.to_bits());
        assert_eq!(c[0], 0.5);
        assert!((c[1] - (515.5f32 / 256.0).fract()).abs() < 1e-6);
        assert!((c[2] - (515.5f32 / 65536.0).fract()).abs() < 1e-6);
        assert_eq!(c[3], 1.0);
    }

    /// BillboardFoliage (f012a36f): every card is four vertices that share one centre
    /// (`position - uv1`) and whose offsets are the four corners of a rectangle; the native loader
    /// (`0x960490`) averages the centres into block +0x28 (the vertex shader's `cb2[5].xyz`).
    #[test]
    fn jungle_billboard_foliage_cards_share_a_centre_and_carry_corner_offsets() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP jungle billboard: JC2_TEST_DIR absent");
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated("vegetation_0.blz").unwrap();
        let archive = crate::sarc::SmallArchive::parse(&bytes).unwrap();
        let model = RbmModel::parse(
            archive
                .data(
                    &bytes,
                    archive.get("Jungle_T02_CanopyM_LOD1-Whole.rbm").unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
        let mut seen = 0;
        for block in model.blocks.iter().filter(|b| b.type_name == "BillboardFoliage") {
            let m = &block.mesh;
            assert_eq!(m.uv1.len(), m.positions.len());
            assert_eq!(m.positions.len() % 4, 0, "cards are four vertices");
            for quad in 0..m.positions.len() / 4 {
                let centre = |i: usize| {
                    let (p, o) = (m.positions[i], m.uv1[i]);
                    [p[0] - o[0], p[1] - o[1], p[2]]
                };
                let c0 = centre(quad * 4);
                let mut corners = std::collections::BTreeSet::new();
                for k in 0..4 {
                    let c = centre(quad * 4 + k);
                    for a in 0..3 {
                        assert!((c[a] - c0[a]).abs() < 1e-3, "quad {quad} centre differs");
                    }
                    let o = m.uv1[quad * 4 + k];
                    assert!(o[0].abs() > 1e-4 && o[1].abs() > 1e-4, "{o:?}");
                    corners.insert((o[0] > 0.0, o[1] > 0.0));
                }
                assert_eq!(corners.len(), 4, "quad {quad}: one vertex per corner");
            }
            seen += 1;
        }
        assert!(seen > 0, "the canopy model has billboard blocks");
    }

    /// Lighting research (f3772304): the 32-byte VegetationBark layout is `x y z, colour, normal,
    /// tangent, u, v` (the native `vegetationbark` vertex shader's input order). The decoded normal must
    /// follow the triangles' geometric normal, the colour must be a baked darkening factor in 0.2..1.
    #[test]
    fn jungle_vegetation_bark_32_byte_layout_normal_follows_geometry() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP jungle bark: JC2_TEST_DIR absent");
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated("vegetation_0.blz").unwrap();
        let archive = crate::sarc::SmallArchive::parse(&bytes).unwrap();
        let model = RbmModel::parse(
            archive
                .data(
                    &bytes,
                    archive.get("Jungle_T02_CanopyM_LOD1-Whole.rbm").unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
        let block = &model.blocks[1];
        assert_eq!(block.type_name, "VegetationBark");
        assert_eq!(block.params[0] as u32 & 4, 4, "the 32-byte layout");
        let m = &block.mesh;
        assert_eq!(m.packed_colours.len(), m.positions.len());
        // accumulate area-weighted face normals
        let mut geo = vec![[0.0f32; 3]; m.positions.len()];
        for t in m.indices.as_chunks::<3>().0 {
            let p = |i: u16| m.positions[i as usize];
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            for i in t {
                for k in 0..3 {
                    geo[*i as usize][k] += n[k];
                }
            }
        }
        let mut dots: Vec<f32> = geo
            .iter()
            .zip(&m.normals)
            .map(|(g, n)| {
                let l = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
                (g[0] * n[0] + g[1] * n[1] + g[2] * n[2]).abs() / l
            })
            .collect();
        dots.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(dots[dots.len() / 2] > 0.9, "median |dot| {}", dots[dots.len() / 2]);
        for raw in &m.packed_colours {
            let c = vegetation_bark_colour(*raw);
            assert!(c[..3].iter().all(|v| (0.2..=1.0).contains(v)), "{c:?}");
        }
    }

    #[test]
    fn original_general_hook_and_opaque_crate_headers_colours_and_textures() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP General originals: JC2_TEST_DIR absent");
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        for (container, member, count, power, flags) in [
            ("general.blz", "wea04_lod1-grapplinghook.rbm", 146, 120.0, 0),
            ("obj.cratesx2.ee", "go003_lod1-a.rbm", 364, 16.0, 0x60),
        ] {
            let bytes = fs.read_inflated(container).unwrap();
            let archive = crate::sarc::SmallArchive::parse(&bytes).unwrap();
            let model =
                RbmModel::parse(archive.data(&bytes, archive.get(member).unwrap()).unwrap())
                    .unwrap();
            assert!(model.truncated_at.is_none());
            let block = &model.blocks[0];
            let header = block.general.as_ref().unwrap();
            assert_eq!(header.specular_power, power);
            assert_eq!(header.render_flags, flags);
            assert_eq!(header.vertex_format, 1);
            assert_eq!(block.mesh.vertex_count(), count);
            assert_eq!(block.mesh.packed_colours.len(), count);
            assert_eq!(block.mesh.uv1.len(), count);
            assert_eq!(block.material.flags, 3);
            assert!(
                block
                    .mesh
                    .packed_colours
                    .iter()
                    .all(|c| general_vertex_colour(*c).iter().all(|v| v.is_finite()))
            );
            if flags == 0 {
                assert_eq!(header.packed_colour, 4287993237);
                assert_eq!(block.material.properties, "hook_mpm.dds");
                assert_eq!(block.material.deformed_diffuse, "hook_dummy.dds");
                assert_eq!(
                    general_vertex_colour(block.mesh.packed_colours[0]),
                    [0.359375, 0.36499023, 0.36507797, 1.0]
                );
            } else {
                assert_eq!(header.channel_texture_mask, [0.0, 0.0, 1.0, 0.0]);
                assert_eq!(header.variant_key(), 3);
                assert_eq!(block.material.properties, "g_specular_1_mpm.dds");
                assert_eq!(block.material.deformed_diffuse, "g_dirt_8.dds");
            }
        }
    }

    #[test]
    fn foliage_parameter_flags_retain_all_raw_bits_and_float_fields() {
        for flags in [0x40u32, 0x60, 0x80000040, 0xffffffff] {
            let mut bytes = vec![0u8];
            for value in [0.0f32, 0.0, 1.6, 1.8, 1.0] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(flags.to_le_bytes());
            // Empty eight material names, trailer3, no vertices/faces.
            bytes.extend([0u8; 32]);
            bytes.extend(3u32.to_le_bytes());
            bytes.extend([0u8; 8]);
            let block = parse_vegetation_foliage(&mut Reader::new(&bytes)).unwrap();
            assert_eq!(block.foliage_flags, Some(flags));
            assert_eq!(block.params, vec![0.0, 0.0, 1.6, 1.8, 1.0]);
            assert_eq!(block.material.flags, 3);
        }
    }
    #[test]
    fn packed_normal_decode() {
        // (127,127,127) -> ~zero vector; 255 -> +1, 0 -> -1
        let (n, s) = unpack_normal(32639.0 + 127.0 / 256.0);
        assert!(n.iter().all(|c| c.abs() < 0.01), "{n:?}");
        assert_eq!(s, 1.0);
        let (n, s) = unpack_normal(-(255.0 * 256.0 + 0.0 + 255.0 / 256.0));
        assert!(
            (n[0] - 1.0).abs() < 1e-3 && (n[1] + 1.0).abs() < 1e-3 && (n[2] - 1.0).abs() < 1e-3,
            "{n:?}"
        );
        assert_eq!(s, -1.0);
    }

    #[test]
    fn half_floats() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert!((half_to_f32(0x3555) - 0.333).abs() < 1e-3);
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert!((half_to_f32(0x0001) - 5.96e-8).abs() < 1e-9);
    }

    #[test]
    fn block_names_hash() {
        assert_eq!(block_type_name(hash_str("CarPaint")), Some("CarPaint"));
        assert_eq!(block_type_name(0), None);
    }

    #[test]
    fn rejects_non_rbm() {
        assert!(!is_rbm(b"DDS "));
        assert!(RbmModel::parse(b"\x05\x00\x00\x00RBMDLxxxx").is_err());
    }

    /// Retail: every Merged mode-0 instance of every metropolis far container fits its
    /// `*_base_N` model's index list (start + 3 * count <= index count), which fixes the
    /// roles of the two `u16` fields, and the expansion yields geometry.
    #[test]
    fn retail_merged_instances_are_index_ranges_of_their_base() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (mut models, mut instances) = (0, 0);
        for l in fs.entries() {
            let Some(name) = fs.names.file(l.name_hash) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.contains("metropolis") || !lower.ends_with(".flz") {
                continue;
            }
            let data = fs.read_inflated(name).unwrap();
            let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
            let read = |n: &str| {
                let m = sarc.get(n)?;
                let raw = crate::compress::maybe_inflate(sarc.data(&data, m).ok()?).ok()?;
                RbmModel::parse(&raw).ok()
            };
            for e in &sarc.entries {
                if !e.name.ends_with(".rbm") {
                    continue;
                }
                let model = read(&e.name).unwrap();
                for block in &model.blocks {
                    let Some(base_name) = merged_base_name(&e.name, block) else {
                        continue;
                    };
                    let base = read(&base_name).unwrap_or_else(|| panic!("{name}: {base_name}"));
                    let base = &base.blocks[0];
                    for i in &block.instances {
                        assert!(
                            i.part as usize + 3 * i.extra as usize <= base.mesh.indices.len(),
                            "{name}/{}",
                            e.name
                        );
                    }
                    let out = expand_merged_instances(block, base);
                    let tris: usize = out.iter().map(|b| b.mesh.indices.len() / 3).sum();
                    assert_eq!(
                        tris,
                        block
                            .instances
                            .iter()
                            .map(|i| i.extra as usize)
                            .sum::<usize>()
                    );
                    models += 1;
                    instances += block.instances.len();
                }
            }
        }
        assert!(models > 1000 && instances > 10000, "{models} {instances}");
    }
}
