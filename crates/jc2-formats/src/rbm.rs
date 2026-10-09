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
    /// Deform / skin / colour attributes that are parsed past but not yet
    /// interpreted are counted here so nothing is silently lost.
    pub extra_attribute_streams: u8,
    /// Merged "batch" blocks carry no geometry of their own: they place
    /// `<name>_part<N>.rbm` models from the same container.
    pub instances: Vec<Instance>,
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
            let _colour = r.f32()?;
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
            let _colour = r.f32()?;
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
    if version == 3 {
        params.push(r.i32()? as f32);
        params.push(r.i32()? as f32);
    }
    let material = read_material(r)?;
    let mut b = new_block(version, material, params);
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
            r.bytes(8)?;
        }
    }
    let n1 = read_count(r, 20)?;
    for _ in 0..n1 {
        let n = r.u32()?;
        let _t = r.u32()?;
        let _bt = r.u32()?;
        b.mesh.uv0.push([r.f32()?, r.f32()?]);
        // bytes x,y,z of the u32 map to [-1,1]
        let nn = [
            (n & 0xff) as f32 / 127.5 - 1.0,
            ((n >> 8) & 0xff) as f32 / 127.5 - 1.0,
            ((n >> 16) & 0xff) as f32 / 127.5 - 1.0,
        ];
        b.mesh.normals.push(nn);
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
/// flags & 4 != 0 (32 B): f32 x y z; packed normal, tangent, bitangent; f32 u v
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
            let (nn, _) = unpack_normal(r.f32()?);
            let (t, sg) = unpack_normal(r.f32()?);
            let _bt = r.f32()?;
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
        let (t, sg) = unpack_normal(r.f32()?);
        let _bt = r.f32()?;
        b.mesh.normals.push(nn);
        b.mesh.tangents.push([t[0], t[1], t[2], sg]);
    }
    b.mesh.indices = read_faces(r)?;
    Ok(b)
}

/// BillboardFoliage v0: camera-facing canopy sprites. material; 28 B vertices: f32 centre x y z; f32 u v;
/// f32 offset x y (corner offset in view space); faces. Positions here are the centres plus the
/// offset applied in the XY plane so the quads at least have area without a billboard shader.
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
}
