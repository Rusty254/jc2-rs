//! Terrain height fields: the global 512×512 map (`terrain_win32.dat`) and the 64×64
//! streamed tiles (`terrain\terrain_win32_XX_ZZ.dat`), read the way the original engine
//! reads them.
//!
//! World layout, from the original executable (`CGameZoneMapStreamPatch`; functions
//! `FUN_00a5e990` tile index, `FUN_00a5f390` cell lookup, `FUN_00a5f210` global sample,
//! `FUN_00a56b70` patch reset, constants at `0x0108ab34..0x0108ab44`):
//!
//! * The world is 32 768 m square; x and z run from −16 384 to +16 384, y is up.
//! * Tile `(X, Z)` covers `x ∈ [512·X − 16384, +512)`, and the same for `z`;
//!   `X = floor((x + 16384) / 512)`. 64 × 64 tiles.
//! * A tile's height field is 132 × 132 `u16` samples 4 m apart: its own 128 cells plus a
//!   two-sample margin on every side that duplicates the neighbours (so seams match).
//!   Row = z, column = x: sample `(r, c)` sits at `x = x0 + 4·(c − 2)`, `z = z0 + 4·(r − 2)`.
//! * Height in metres = `u16 × 2200 / 65535` (the engine's `max_terrain_height`).
//!   The patch reset value 300 (≈ 10 m) marks "no data" (open sea).
//! * After the heights come 132 × 132 bytes: the gameplay material of every sample, an
//!   index into `climate\terrain_materials.bin` (`FUN_00a64830` reads one u32, then
//!   `0x4410` u16 heights, then `0x4410` bytes; `FUN_00a563a0` looks a position up in
//!   that byte map). The rest of the file is a sequence of sections, each a u32 version
//!   (12) followed by DDS textures prefixed with their byte size: the render layers
//!   (the gradient map `A8L8` with 8 mips — `128 + Δraw/m` in x and z, see
//!   [`TerrainTile::layer_gradient`] — meta-material slots `A4R4G4B4`, slot weights
//!   `A4R4G4B4`, all 128 × 128, texel `(r, c)` on height sample `(r + 2, c + 2)`), then
//!   the PDA map tile (`DXT1` 128 × 128), then the terrain mesh data
//!   (`CTerrainDataStreamPatch`, `FUN_00a562c0`: 1 + 4 + 16 `STerrainMeshDataHi` pieces,
//!   each a bit array `u32 count, u32[count / 32 + 1]` plus four `u16`) and the game-zone
//!   map (`CGameZoneMapStreamPatch`, `FUN_00a560b0`: 64 × 64 bytes, 8 m cells), each
//!   behind its own `u32 12`; the file ends with one more `u32 12`. See
//!   `research/formats/terrain.md` and [`crate::landscape`] for how the layers are drawn.
//! * The global map is "VER\0", version 12, then 512 × 512 `u16` at 64 m with the same
//!   scale; row = `floor((z + 16384) / 64)`, column likewise from x; sampled bilinearly,
//!   300 outside the map.

use crate::dds::DdsImage;
use crate::error::{Error, Result};
use crate::reader::Reader;

/// Metres from one edge of the world to the other.
pub const WORLD_SIZE: f32 = 32768.0;
/// The lowest x and z coordinate of the world.
pub const WORLD_MIN: f32 = -16384.0;
/// Edge length of one streamed tile.
pub const TILE_SIZE: f32 = 512.0;
/// Tiles per world edge.
pub const TILES_PER_SIDE: u32 = 64;
/// Distance between two height samples in a tile.
pub const CELL_SIZE: f32 = 4.0;
/// Cells a tile owns per edge.
pub const TILE_CELLS: usize = 128;
/// Samples stored per edge (own cells plus the margin on both sides).
pub const TILE_SAMPLES: usize = 132;
/// Samples of margin on each side of a tile.
pub const TILE_MARGIN: usize = 2;
/// Metres per raw height unit.
pub const HEIGHT_SCALE: f32 = 2200.0 / 65535.0;
/// Raw height the engine fills patches with before data arrives (and for the sea floor
/// outside the map).
pub const NO_DATA: u16 = 300;
/// Samples per edge of the global map.
pub const GLOBAL_SAMPLES: usize = 512;
/// Distance between two samples of the global map.
pub const GLOBAL_CELL: f32 = 64.0;

/// Raw height unit → metres.
pub fn height_to_metres(raw: u16) -> f32 {
    raw as f32 * HEIGHT_SCALE
}

/// The tile that contains world position `(x, z)`, or `None` outside the world.
pub fn tile_index(x: f32, z: f32) -> Option<(u32, u32)> {
    let tx = ((x - WORLD_MIN) / TILE_SIZE).floor();
    let tz = ((z - WORLD_MIN) / TILE_SIZE).floor();
    let n = TILES_PER_SIDE as f32;
    if tx < 0.0 || tz < 0.0 || tx >= n || tz >= n {
        return None;
    }
    Some((tx as u32, tz as u32))
}

/// World position of tile `(tx, tz)`'s corner with the smallest x and z.
pub fn tile_origin(tx: u32, tz: u32) -> (f32, f32) {
    (
        WORLD_MIN + tx as f32 * TILE_SIZE,
        WORLD_MIN + tz as f32 * TILE_SIZE,
    )
}

/// Archive path of a tile's height-field file.
pub fn tile_file_name(tx: u32, tz: u32) -> String {
    format!("terrain\\terrain_win32_{tx:02}_{tz:02}.dat")
}

/// Archive path of the location container (`.nlz`: models, placements, collision) for
/// tile `(tx, tz)`. Only tiles with content have one.
pub fn tile_container_name(tx: u32, tz: u32) -> String {
    format!("locations\\wc\\{tx:02}\\{tx:02}x{tz:02}.nlz")
}

/// Version word that introduces every stream patch of a tile file.
pub const PATCH_VERSION: u32 = 12;
/// Cells per edge of the game-zone map (8 m each).
pub const GAMEZONE_SAMPLES: usize = 64;

/// One piece of the terrain mesh data (`STerrainMeshDataHi`, read by `FUN_00a56230`):
/// a bit array and four `u16`. The quadtree has 1 + 4 + 16 pieces per tile; what the
/// bits mean (triangulation / hole flags of the high-detail terrain mesh, judging by
/// the owner class) is not decoded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshPiece {
    pub bits: u32,
    pub words: Vec<u32>,
    pub extra: [u16; 4],
}

/// Material slots and weights the renderer blends in one 4 m cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetaCell {
    /// Four meta-material slots (0..16; see [`crate::landscape::MetaMaterials`]).
    pub slots: [u8; 4],
    /// Their weights, 0..15, summing to 15 in the shipped data.
    pub weights: [u8; 4],
}

impl MetaCell {
    /// Unpack a texel of the `A4R4G4B4` slot and weight textures: channel order
    /// `A R G B` = nibbles `15..12, 11..8, 7..4, 3..0`; slot *i* goes with weight *i*.
    pub fn unpack(slots: u16, weights: u16) -> Self {
        let nib = |v: u16, i: usize| ((v >> (12 - 4 * i)) & 0xf) as u8;
        Self {
            slots: [nib(slots, 0), nib(slots, 1), nib(slots, 2), nib(slots, 3)],
            weights: [
                nib(weights, 0),
                nib(weights, 1),
                nib(weights, 2),
                nib(weights, 3),
            ],
        }
    }

    /// The slot with the largest weight.
    pub fn dominant(&self) -> u8 {
        let mut best = 0;
        for i in 1..4 {
            if self.weights[i] > self.weights[best] {
                best = i;
            }
        }
        self.slots[best]
    }

    /// `(slot, weight / 15)` for every slot with a non-zero weight.
    pub fn blend(&self) -> impl Iterator<Item = (u8, f32)> + '_ {
        (0..4)
            .filter(|&i| self.weights[i] > 0)
            .map(|i| (self.slots[i], self.weights[i] as f32 / 15.0))
    }
}

/// One section of a tile file after the game patch: its version and its textures.
#[derive(Clone, Debug)]
pub struct TileSection {
    pub version: u32,
    pub textures: Vec<DdsImage>,
}

/// One 512 m terrain tile.
#[derive(Clone, Debug)]
pub struct TerrainTile {
    pub version: u32,
    /// Always 123 in the shipped data.
    pub kind: u32,
    /// 132 × 132 raw heights, row-major, row = z.
    pub heights: Vec<u16>,
    /// 132 × 132 gameplay material indices (`climate\terrain_materials.bin`, the
    /// `Material` field), same layout as `heights`. Zero-filled when the file lacks them.
    pub materials: Vec<u8>,
    /// Render layers: `[normals, slots, weights]` as DDS images when present.
    pub layers: Vec<DdsImage>,
    /// Meta-material slot texels of the tile's 128 × 128 cells (`A4R4G4B4`, row = z,
    /// texel `(r, c)` sits on height sample `(r + 2, c + 2)`), zeros when absent.
    pub meta_slots: Vec<u16>,
    /// Slot weights, same layout as `meta_slots`.
    pub meta_weights: Vec<u16>,
    /// The 128 × 128 `DXT1` PDA map tile (baked colours), when present.
    pub map: Option<DdsImage>,
    /// Every section after the game patch, in file order.
    pub sections: Vec<TileSection>,
    /// The 21 terrain mesh data pieces (`CTerrainDataStreamPatch`); empty when absent.
    pub mesh_data: Vec<MeshPiece>,
    /// 64 × 64 game-zone bytes, 8 m per cell, row = z (`CGameZoneMapStreamPatch`);
    /// empty when the patch is missing.
    pub gamezone: Vec<u8>,
    /// Undecoded remainder of the file (the zone map and mesh data sections).
    pub tail: Vec<u8>,
}

impl TerrainTile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.u32()?;
        let kind = r.u32()?;
        if version != 12 {
            return Err(Error::format(format!("terrain tile version {version}")));
        }
        let n = TILE_SAMPLES * TILE_SAMPLES;
        let heights = r.u16_vec(n)?;
        let materials = if r.remaining() >= n {
            r.bytes(n)?.to_vec()
        } else {
            vec![0; n]
        };
        // sections: u32 version, then (u32 size, DDS)* until the next thing that is not
        // a sized DDS; the first section without any texture ends the decoded part
        let mut sections = Vec::new();
        loop {
            let at = r.pos();
            let Ok(v) = r.u32() else { break };
            let mut textures = Vec::new();
            while r.remaining() >= 8 && &r.peek(8)[4..8] == b"DDS " {
                let size = r.u32()? as usize;
                let bytes = r.bytes(size)?;
                textures.push(DdsImage::parse(bytes)?);
            }
            if textures.is_empty() {
                r.seek(at)?;
                break;
            }
            sections.push(TileSection {
                version: v,
                textures,
            });
        }
        // the mesh data and game-zone patches, when they parse
        let at = r.pos();
        let (mesh_data, gamezone) = match Self::parse_mesh_and_zones(&mut r) {
            Ok(v) => v,
            Err(_) => {
                r.seek(at)?;
                (Vec::new(), Vec::new())
            }
        };
        let tail = r.bytes(r.remaining())?.to_vec();
        let layers = sections
            .first()
            .map(|s| s.textures.clone())
            .unwrap_or_default();
        let cells = TILE_CELLS * TILE_CELLS;
        let texels = |i: usize| -> Vec<u16> {
            layers
                .get(i)
                .filter(|d| d.width as usize == TILE_CELLS && d.height as usize == TILE_CELLS)
                .and_then(|d| d.texels_u16(0))
                .unwrap_or_else(|| vec![0; cells])
        };
        let meta_slots = texels(1);
        let meta_weights = texels(2);
        let map = sections
            .get(1)
            .and_then(|s| s.textures.first())
            .filter(|d| d.width as usize == TILE_CELLS)
            .cloned();
        Ok(Self {
            version,
            kind,
            heights,
            materials,
            layers,
            meta_slots,
            meta_weights,
            map,
            sections,
            mesh_data,
            gamezone,
            tail,
        })
    }

    fn parse_mesh_and_zones(r: &mut Reader) -> Result<(Vec<MeshPiece>, Vec<u8>)> {
        let version = |r: &mut Reader| -> Result<()> {
            let v = r.u32()?;
            if v != PATCH_VERSION {
                return Err(Error::format(format!("stream patch version {v}")));
            }
            Ok(())
        };
        version(r)?;
        let mut pieces = Vec::with_capacity(21);
        for _ in 0..21 {
            let bits = r.u32()?;
            let words = if bits == 0 {
                Vec::new()
            } else {
                let n = (bits as usize >> 5) + 1;
                let mut w = Vec::with_capacity(n);
                for _ in 0..n {
                    w.push(r.u32()?);
                }
                w
            };
            let mut extra = [0u16; 4];
            for e in extra.iter_mut() {
                *e = r.u16()?;
            }
            pieces.push(MeshPiece { bits, words, extra });
        }
        version(r)?;
        let zones = r.bytes(GAMEZONE_SAMPLES * GAMEZONE_SAMPLES)?.to_vec();
        Ok((pieces, zones))
    }

    /// Material at a position relative to the tile origin, the way the engine's ground
    /// query looks it up (`FUN_00a56400`: `floor(local / 4) + 2` into the 132 grid, clamped).
    pub fn material_local(&self, lx: f32, lz: f32) -> u8 {
        let max = (TILE_SAMPLES - 1) as i32;
        let c = ((lx * 0.25 + 1.0) as i32 + 1).clamp(0, max) as usize;
        let r = ((lz * 0.25 + 1.0) as i32 + 1).clamp(0, max) as usize;
        self.material(r, c)
    }

    /// Game zone byte at a position relative to the tile origin (8 m cells), or `None`
    /// when the tile has no zone map.
    pub fn gamezone_local(&self, lx: f32, lz: f32) -> Option<u8> {
        if self.gamezone.len() != GAMEZONE_SAMPLES * GAMEZONE_SAMPLES {
            return None;
        }
        let max = (GAMEZONE_SAMPLES - 1) as i32;
        let c = ((lx / 8.0) as i32).clamp(0, max) as usize;
        let r = ((lz / 8.0) as i32).clamp(0, max) as usize;
        Some(self.gamezone[r * GAMEZONE_SAMPLES + c])
    }

    /// Gameplay material index of sample `(row, col)` in the 132 × 132 grid.
    pub fn material(&self, row: usize, col: usize) -> u8 {
        self.materials[row * TILE_SAMPLES + col]
    }

    /// Gameplay material of the tile's own cell `(cz, cx)`, `0..128`.
    pub fn cell_material(&self, cz: usize, cx: usize) -> u8 {
        self.material(cz + TILE_MARGIN, cx + TILE_MARGIN)
    }

    /// Render slots and weights of cell `(cz, cx)`, `0..128`.
    pub fn meta_cell(&self, cz: usize, cx: usize) -> MetaCell {
        let i = cz * TILE_CELLS + cx;
        MetaCell::unpack(self.meta_slots[i], self.meta_weights[i])
    }

    /// True when the file carried the render layers (slots and weights).
    pub fn has_layers(&self) -> bool {
        self.layers.len() >= 3
    }

    /// The tile's normal-map layer, when present.
    pub fn normal_map(&self) -> Option<&DdsImage> {
        self.layers.first()
    }

    /// The ground gradient `(∂h/∂x, ∂h/∂z)` in metres per metre stored in the normal
    /// layer for cell `(cz, cx)`: the `A8L8` texel holds `128 + Δraw per metre` for x in
    /// L and for z in A, clamped to a byte (the shader reads `(byte / 255 × 2 − 1) ×
    /// 127 × HEIGHT_SCALE`). `None` without the layer.
    pub fn layer_gradient(&self, cz: usize, cx: usize) -> Option<(f32, f32)> {
        let layer = self.normal_map()?;
        if layer.format != crate::dds::DdsFormat::A8L8
            || layer.width as usize != TILE_CELLS
            || layer.height as usize != TILE_CELLS
        {
            return None;
        }
        let data = layer.level(0)?;
        let i = (cz * TILE_CELLS + cx) * 2;
        let g = |b: u8| (b as f32 / 255.0 * 2.0 - 1.0) * 127.0 * HEIGHT_SCALE;
        Some((g(data[i]), g(data[i + 1])))
    }

    /// The same gradient computed from the heights: the central difference across the
    /// cell's two neighbours, per metre (what the normal layer was baked from).
    pub fn cell_gradient(&self, cz: usize, cx: usize) -> (f32, f32) {
        let (r, c) = (cz + TILE_MARGIN, cx + TILE_MARGIN);
        let dx = (self.height(r, c + 1) - self.height(r, c - 1)) / (2.0 * CELL_SIZE);
        let dz = (self.height(r + 1, c) - self.height(r - 1, c)) / (2.0 * CELL_SIZE);
        (dx, dz)
    }

    /// Raw height of sample `(row, col)` in the 132 × 132 grid (margin included).
    pub fn raw(&self, row: usize, col: usize) -> u16 {
        self.heights[row * TILE_SAMPLES + col]
    }

    /// Height in metres of sample `(row, col)` in the 132 × 132 grid.
    pub fn height(&self, row: usize, col: usize) -> f32 {
        height_to_metres(self.raw(row, col))
    }

    /// Height in metres of the tile's own cell `(cz, cx)`, `0..=128` (128 is the first
    /// margin sample, i.e. the neighbour's edge, so a 129-vertex mesh is seamless).
    pub fn cell_height(&self, cz: usize, cx: usize) -> f32 {
        self.height(cz + TILE_MARGIN, cx + TILE_MARGIN)
    }

    /// Bilinear height at a position relative to the tile origin, in metres. Positions a
    /// little outside the tile are served from the margin.
    pub fn sample_local(&self, lx: f32, lz: f32) -> Option<f32> {
        let c = lx / CELL_SIZE + TILE_MARGIN as f32;
        let r = lz / CELL_SIZE + TILE_MARGIN as f32;
        let max = (TILE_SAMPLES - 1) as f32;
        if !(0.0..max).contains(&c) || !(0.0..max).contains(&r) {
            return None;
        }
        let (c0, r0) = (c.floor() as usize, r.floor() as usize);
        let (fc, fr) = (c - c0 as f32, r - r0 as f32);
        let h00 = self.height(r0, c0);
        let h01 = self.height(r0, c0 + 1);
        let h10 = self.height(r0 + 1, c0);
        let h11 = self.height(r0 + 1, c0 + 1);
        let top = h00 + (h01 - h00) * fc;
        let bottom = h10 + (h11 - h10) * fc;
        Some(top + (bottom - top) * fr)
    }

    /// Lowest and highest raw height in the tile.
    pub fn raw_range(&self) -> (u16, u16) {
        let lo = self.heights.iter().copied().min().unwrap_or(0);
        let hi = self.heights.iter().copied().max().unwrap_or(0);
        (lo, hi)
    }

    /// True when every sample is the engine's fill value: open sea, nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.heights.iter().all(|&h| h == NO_DATA)
    }

    /// Check that this tile's margin matches the neighbour to the +x side (`east`) and
    /// the neighbour to the +z side (`south`): returns the number of mismatching samples.
    pub fn seam_mismatches(
        &self,
        east: Option<&TerrainTile>,
        south: Option<&TerrainTile>,
    ) -> usize {
        let n = TILE_SAMPLES;
        let m = TILE_MARGIN;
        let mut bad = 0;
        if let Some(e) = east {
            // our columns 128..132 are the neighbour's columns 0..4
            for r in 0..n {
                for k in 0..2 * m {
                    if self.raw(r, n - 2 * m + k) != e.raw(r, k) {
                        bad += 1;
                    }
                }
            }
        }
        if let Some(s) = south {
            for c in 0..n {
                for k in 0..2 * m {
                    if self.raw(n - 2 * m + k, c) != s.raw(k, c) {
                        bad += 1;
                    }
                }
            }
        }
        bad
    }
}

/// The 512 × 512 overview height map (`terrain_win32.dat`), 64 m per sample.
#[derive(Clone, Debug)]
pub struct GlobalTerrain {
    pub version: u32,
    pub heights: Vec<u16>,
    /// Undecoded remainder (about 1 MB of further records).
    pub tail: Vec<u8>,
}

impl GlobalTerrain {
    pub const MAGIC: &'static [u8; 4] = b"VER\0";

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.bytes(4)? != Self::MAGIC {
            return Err(Error::format("not a global terrain map (no VER magic)"));
        }
        let version = r.u32()?;
        let heights = r.u16_vec(GLOBAL_SAMPLES * GLOBAL_SAMPLES)?;
        let tail = r.bytes(r.remaining())?.to_vec();
        Ok(Self {
            version,
            heights,
            tail,
        })
    }

    pub fn raw(&self, row: usize, col: usize) -> u16 {
        self.heights[row * GLOBAL_SAMPLES + col]
    }

    /// Height in metres at world `(x, z)`, bilinear, exactly as the engine's coarse
    /// query does it (`FUN_00a5f210`): the fill value outside the map.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let u = (x - WORLD_MIN) / GLOBAL_CELL;
        let v = (z - WORLD_MIN) / GLOBAL_CELL;
        let (iu, iv) = (u.floor(), v.floor());
        let (fu, fv) = (u - iu, v - iv);
        if iu < 0.0 || iv < 0.0 || iu >= GLOBAL_SAMPLES as f32 || iv >= GLOBAL_SAMPLES as f32 {
            return height_to_metres(NO_DATA);
        }
        let (c, r) = (iu as usize, iv as usize);
        let mask = GLOBAL_SAMPLES - 1;
        let h = |r: usize, c: usize| self.raw(r & mask, c & mask) as f32;
        let top = h(r, c) + (h(r, c + 1) - h(r, c)) * fu;
        let bottom = h(r + 1, c) + (h(r + 1, c + 1) - h(r + 1, c)) * fu;
        (top + (bottom - top) * fv) * HEIGHT_SCALE
    }
}

/// One byte of the game zone map (`CGameZoneMap`, 8 m cells, last section of a terrain tile;
/// `research/formats/gamezone-map-next.md`): `intensity (2 bits) | type (3 bits) | low3`.
/// The accessors are `FUN_00a5e400` (bits 6..7), `FUN_00a5e3f0` (bits 3..5), `FUN_00a5e410`
/// (bits 0..2). The type is the gradient light set (identity map in `landscape_effects.bin`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameZoneCell(pub u8);

impl GameZoneCell {
    /// Bits 3..5: 0 None, 1 Arctic, 2 Desert, 3 Jungle, 4 Sea, 5 City, 6 Grass, 7 Default.
    pub fn zone_type(self) -> u8 {
        (self.0 >> 3) & 7
    }
    /// Bits 6..7 (0..3).
    pub fn intensity(self) -> u8 {
        self.0 >> 6
    }
    /// Bits 0..2 (a sub-zone field; not used by lighting).
    pub fn low3(self) -> u8 {
        self.0 & 7
    }
}

/// The byte the engine's patch reset fills with (`FUN_00a56090`: `memset(..., 0xe0, 0x1000)`),
/// Sea at intensity 3. **Non-native as a fallback** for tiles that are not loaded or positions
/// outside the world: the engine reads an unrelated ring slot there.
pub const GAMEZONE_RESET_BYTE: u8 = 0xE0;

/// `FUN_00a5eb50`: the tile and cell of a world position, in float32 exactly as the engine does
/// it (no bounds check: `x = 16384` gives tile 64, `x < -16384` tile -1). Returns
/// `(tile_x, tile_z, cell_x, cell_z)`, cells 0..63 (cell = row `z` * 64 + column `x`).
pub fn zone_cell(x: f32, z: f32) -> (i32, i32, usize, usize) {
    fn split(v: f32) -> (i32, usize) {
        // (v - (-16384)) * (1/512); cvttss2si truncates, then 1 is subtracted when the
        // fraction is negative
        let u = (v - WORLD_MIN) * 0.001_953_125;
        let t = u as i32;
        let mut frac = u - t as f32;
        let mut tile = t;
        if frac < 0.0 {
            tile -= 1;
            frac += 1.0;
        }
        (tile, ((frac * 64.0) as i32).clamp(0, 63) as usize)
    }
    let (tx, cx) = split(x);
    let (tz, cz) = split(z);
    (tx, tz, cx, cz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_with(f: impl Fn(usize, usize) -> u16) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for r in 0..TILE_SAMPLES {
            for c in 0..TILE_SAMPLES {
                d.extend_from_slice(&f(r, c).to_le_bytes());
            }
        }
        d.extend(std::iter::repeat_n(7u8, TILE_SAMPLES * TILE_SAMPLES));
        d.extend_from_slice(b"tail");
        d
    }

    #[test]
    fn index_and_origin() {
        assert_eq!(tile_index(-16384.0, -16384.0), Some((0, 0)));
        assert_eq!(tile_index(-500.0, -9000.0), Some((31, 14)));
        assert_eq!(tile_index(16383.0, 0.0), Some((63, 32)));
        assert_eq!(tile_index(16384.0, 0.0), None);
        assert_eq!(tile_origin(31, 14), (-512.0, -9216.0));
        assert_eq!(tile_file_name(3, 14), "terrain\\terrain_win32_03_14.dat");
        assert_eq!(tile_container_name(31, 14), "locations\\wc\\31\\31x14.nlz");
    }

    #[test]
    fn parse_and_sample() {
        // height grows 1 raw unit per column: metres = col * scale
        let data = tile_with(|_, c| c as u16 * 100);
        let t = TerrainTile::parse(&data).unwrap();
        assert_eq!(t.kind, 123);
        assert_eq!(t.tail, b"tail");
        assert_eq!(t.material(3, 3), 7);
        assert!(!t.has_layers());
        assert_eq!(t.meta_cell(0, 0), MetaCell::default());
        assert_eq!(t.raw(5, 7), 700);
        assert_eq!(t.cell_height(0, 0), height_to_metres(200));
        // local x = 0 is column 2; x = 6 m is column 3.5
        let h = t.sample_local(6.0, 100.0).unwrap();
        assert!((h - height_to_metres(350)).abs() < 1e-3, "{h}");
        assert!(t.sample_local(-9.0, 0.0).is_none());
        assert!(t.sample_local(520.0, 0.0).is_none());
        assert_eq!(t.raw_range(), (0, 13100));
    }

    /// A tile with the render sections: heights, materials, then a layer section
    /// (normals, slots, weights) and a map section, the way the shipped files are laid
    /// out (`jc2tool terrain 31 14` shows the real one).
    #[test]
    fn sections_and_meta_cells() {
        use crate::dds::{DdsFormat, write_dds};
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        // heights rise 40 raw units per column: a gradient of 10 raw/m in x
        for _ in 0..TILE_SAMPLES {
            for c in 0..TILE_SAMPLES {
                d.extend_from_slice(&(c as u16 * 40).to_le_bytes());
            }
        }
        d.extend((0..TILE_SAMPLES * TILE_SAMPLES).map(|i| (i % 7) as u8));
        let cells = TILE_CELLS * TILE_CELLS;
        let normals = write_dds(128, 128, 1, DdsFormat::A8L8, &[138u8, 128u8].repeat(cells));
        let mut slots = Vec::new();
        let mut weights = Vec::new();
        for i in 0..cells {
            // slot nibbles A R G B = 11, 0, 5, 3; weights 2, 0, 13, 0 → dominant slot 5
            slots.extend_from_slice(&0xb053u16.to_le_bytes());
            weights.extend_from_slice(&(if i == 0 { 0x20d0u16 } else { 0x000f }).to_le_bytes());
        }
        let slots = write_dds(128, 128, 1, DdsFormat::Argb4444, &slots);
        let weights = write_dds(128, 128, 1, DdsFormat::Argb4444, &weights);
        let map = write_dds(128, 128, 1, DdsFormat::Dxt1, &vec![0u8; 32 * 32 * 8]);
        d.extend_from_slice(&12u32.to_le_bytes());
        for t in [&normals, &slots, &weights] {
            d.extend_from_slice(&(t.len() as u32).to_le_bytes());
            d.extend_from_slice(t);
        }
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&(map.len() as u32).to_le_bytes());
        d.extend_from_slice(&map);
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(b"rest of the file");
        let t = TerrainTile::parse(&d).unwrap();
        assert_eq!(t.sections.len(), 2);
        assert!(t.has_layers());
        assert_eq!(t.normal_map().unwrap().format, DdsFormat::A8L8);
        assert!(t.map.is_some());
        assert_eq!(t.cell_material(0, 0), ((2 * TILE_SAMPLES + 2) % 7) as u8);
        let c = t.meta_cell(0, 0);
        assert_eq!(c.slots, [11, 0, 5, 3]);
        assert_eq!(c.weights, [2, 0, 13, 0]);
        assert_eq!(c.dominant(), 5);
        let b: Vec<(u8, f32)> = c.blend().collect();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].0, 11);
        assert!((b[1].1 - 13.0 / 15.0).abs() < 1e-6);
        assert_eq!(t.meta_cell(0, 1).dominant(), 3);
        assert_eq!(&t.tail[..4], &12u32.to_le_bytes());
        // the normal layer's 138 is +10 raw per metre; the heights say the same
        let (gx, gz) = t.layer_gradient(5, 5).unwrap();
        assert!((gx - 10.0 * HEIGHT_SCALE).abs() < 0.02, "{gx}");
        assert!(gz.abs() < 0.02, "{gz}");
        let (hx, hz) = t.cell_gradient(5, 5);
        assert!((hx - gx).abs() < 0.02, "{hx} vs {gx}");
        assert_eq!(hz, 0.0);
    }

    #[test]
    fn seams() {
        let a = TerrainTile::parse(&tile_with(|_, c| c as u16)).unwrap();
        // neighbour whose first four columns equal our last four (128..132)
        let e = TerrainTile::parse(&tile_with(|_, c| c as u16 + 128)).unwrap();
        assert_eq!(a.seam_mismatches(Some(&e), None), 0);
        let wrong = TerrainTile::parse(&tile_with(|_, c| c as u16 + 1)).unwrap();
        assert_eq!(a.seam_mismatches(Some(&wrong), None), 4 * TILE_SAMPLES);
    }

    #[test]
    fn global() {
        let mut d = Vec::new();
        d.extend_from_slice(b"VER\0");
        d.extend_from_slice(&12u32.to_le_bytes());
        for r in 0..GLOBAL_SAMPLES {
            for _ in 0..GLOBAL_SAMPLES {
                d.extend_from_slice(&(r as u16).to_le_bytes());
            }
        }
        let g = GlobalTerrain::parse(&d).unwrap();
        // row 10 at z = -16384 + 640; half-way to row 11
        let h = g.height_at(0.0, -16384.0 + 640.0 + 32.0);
        assert!((h - 10.5 * HEIGHT_SCALE).abs() < 1e-4, "{h}");
        assert_eq!(g.height_at(-20000.0, 0.0), height_to_metres(NO_DATA));
    }
    #[test]
    fn zone_cell_follows_the_engines_float_sequence() {
        // the probe's fixtures (gamezone-map-next.md section 5)
        assert_eq!(zone_cell(-1280.0, -6712.0), (29, 18, 32, 57));
        assert_eq!(zone_cell(-16384.0, -16384.0), (0, 0, 0, 0));
        assert_eq!(zone_cell(-16000.0, -16000.0), (0, 0, 48, 48));
        assert_eq!(zone_cell(16384.0, 0.0).0, 64, "no bounds check: tile 64");
        assert_eq!(zone_cell(-16385.0, 0.0).0, -1, "below the world: tile -1");
        assert_eq!(zone_cell(-16385.0, 0.0).2, 63);
        // the byte layout
        let c = GameZoneCell(0xC8);
        assert_eq!((c.zone_type(), c.intensity(), c.low3()), (1, 3, 0));
        let c = GameZoneCell(0x3A);
        assert_eq!((c.zone_type(), c.intensity(), c.low3()), (7, 0, 2));
        let sea = GameZoneCell(GAMEZONE_RESET_BYTE);
        assert_eq!((sea.zone_type(), sea.intensity()), (4, 3));
    }

    /// The probe's lookups on the real tiles (needs the game files): position -> byte.
    #[test]
    fn real_zone_bytes_match_the_probe_fixtures() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = crate::vfs::GameFs::open(dir) else {
            return;
        };
        #[rustfmt::skip]
        let fixtures: [(f32, f32, u8); 12] = [
            (-1280.0, -6712.0, 0xC8),   // start base: Arctic, intensity 3
            (3164.6, -3777.0, 0xC8),    // research facility
            (6884.9, -3508.6, 0x0C),    // military base 45x25: Arctic, intensity 0, low3 4
            (-9493.4, 5583.0, 0xD0),    // desert base
            (-15092.0, -14524.0, 0xD8), // jungle interior
            (-16000.0, -16000.0, 0xE0), // open ocean
            (-15364.0, -3092.0, 0xE8),  // city interior
            (-3749.1, 9594.4, 0xF0),    // grass village
            (12473.3, 11692.6, 0xF8),   // Default base
            (-6635.8, -3585.5, 0x3A),   // airport: Default, intensity 0, low3 2
            (-2730.4, 4622.7, 0x50),    // village 26x41: Desert, intensity 1
            (-16384.0, -16384.0, 0xE0), // world corner
        ];
        for (x, z, want) in fixtures {
            let (tx, tz, cx, cz) = zone_cell(x, z);
            let name = tile_file_name(tx as u32, tz as u32);
            let data = fs.read_inflated(&name).expect("terrain tile is readable");
            let tile = TerrainTile::parse(&data).expect("tile parses");
            assert_eq!(tile.gamezone.len(), 4096, "{name}");
            assert_eq!(tile.gamezone[cz * 64 + cx], want, "({x}, {z}) in {name}");
        }
    }
}
