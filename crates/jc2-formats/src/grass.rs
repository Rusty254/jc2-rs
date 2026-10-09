//! Grass: the ground cover the engine scatters around the camera, 1 m cells in 8 m
//! patches (`FUN_00a82620`, called through `UpdateVegetationGrass` → `FUN_00a8c880`; see
//! `research/formats/world.md`, "Grass").
//!
//! Data:
//!
//! * `climate\zone_N\grass_types_N.bin` — one record per grass type (`Grass_Thin`,
//!   `Jungle_Grass_Leafs_Red`, `Poppy_Field_Grass_Thick`…): a quad's size, its UV
//!   rectangle in the texture atlas `global_grass_DIF.dds` (`general.blz`, 1024 × 512
//!   DXT5, 2 × 4 cells of 512 × 128) and a few ranges.
//! * the vegetation sets (`vegetation_set_N.bin`) list grass entries (`group = "grass"`,
//!   `#df8d2697` slots out of 64) next to the trees and bushes; a set's third table is
//!   the grass lookup, shuffled like the other two.
//! * the terrain material under a cell picks the set (`terrain_materials.bin`).
//!
//! The generator (one 8 m patch, origin `(px, pz)` a multiple of 8 from −16384):
//!
//! * seed `(int)(pz × 435 + px × 231)` stepped with the MSVC LCG; four passes of 16
//!   cells (the original draws only as many passes as its density setting wants), cell
//!   `(row, col)` with `col = ((row & 3) > 1) + pass + 2 row` mod 4, and `+ 4` again;
//! * per cell six draws: (a) table slot, (b) a phase byte, (c) the size lerp, (d) the
//!   yaw, (e, f) the position jitter `±0.5 m`; all six are drawn even for cells that
//!   end up empty;
//! * the ground there must be above the sea (`y > 200`), have a material with a set in
//!   a zone that has grass types; slot `a mod count` (count = grass types of the zone)
//!   then the next slots in turn until one is non-empty and its slope range accepts the
//!   ground (`max(|sin n.x|, |sin n.z|) ≤ slope_max`, `min(…) ≥ slope_min`, the angles
//!   in the type's degrees);
//! * one blade is three quads 120° apart, each `2 × width × size` wide and
//!   `height × size` tall (`size = lerp(size_min, size_max, c)`), sheared by `lean`,
//!   every vertex standing on the terrain at its own position.

use crate::property::{Node, PropertyFile, Variant};
use crate::vegetation::{Ground, Vegetation};
use std::f32::consts::TAU;

/// Edge of a grass patch in metres.
pub const GRASS_PATCH: f32 = 8.0;
/// Cells per patch edge (1 m cells).
pub const GRASS_CELLS: u32 = 8;
/// The terrain below this height is sea: no grass.
pub const SEA_LEVEL: f32 = 200.0;
/// Quads standing around one blade's centre.
pub const QUADS_PER_BLADE: usize = 3;
/// How far a quad's vertices sit above the terrain (m).
pub const GROUND_LIFT: f32 = 0.05;
/// Sin of the tilt (45°) the engine rotates its quads by before taking the lean.
pub const LEAN_TILT: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Patches the engine keeps around the camera (a 13 × 13 ring buffer: ±52 m).
pub const ENGINE_PATCH_WINDOW: u32 = 13;

const KEY_NAME: &str = "name";
const KEY_WIDTH: &str = "width";
const KEY_HEIGHT: &str = "height";
const KEY_BILLBOARD: &str = "billboard";
const KEY_TEXTURE: u32 = 0xfbdb_7d47;
const KEY_TYPES: u32 = 0xd99a_4e2d;
const KEY_U0: u32 = 0x30f8_f3e1;
const KEY_V0: u32 = 0x4a58_4a79;
const KEY_U1: u32 = 0xd838_54c9;
const KEY_V1: u32 = 0x090d_5ee6;
const KEY_SIZE_MIN: u32 = 0x1460_14af;
const KEY_SIZE_MAX: u32 = 0x2ca4_9d2e;
const KEY_LEAN: u32 = 0xf873_e6d4;
const KEY_SWAY: u32 = 0x87ee_b199;
const KEY_SLOPE_MIN: u32 = 0x33f3_ef7b;
const KEY_SLOPE_MAX: u32 = 0x2ead_7fc9;
const KEY_ALIGN: u32 = 0x8643_db23;

/// One grass type.
#[derive(Clone, Debug, PartialEq)]
pub struct GrassType {
    pub name: String,
    /// Half width and height of the quad before the size factor (`width`, `height`).
    pub width: f32,
    pub height: f32,
    /// The quad's rectangle in the atlas: `[u0, v0, u1, v1]` (v0 is the top).
    pub uv: [f32; 4],
    /// The size factor is drawn between these.
    pub size_min: f32,
    pub size_max: f32,
    /// How far the top leans out of the quad's plane, in heights.
    pub lean: f32,
    /// How far the tip sways (0..1 of the vertex weight).
    pub sway: f32,
    /// Accepted ground slopes in radians (see the module docs).
    pub slope_min: f32,
    pub slope_max: f32,
    /// How much the blade follows the ground normal (1 everywhere in the data).
    pub align: f32,
}

fn f32_of(n: &Node, hash: u32) -> Option<f32> {
    match n.value_hash(hash)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

impl GrassType {
    fn parse(n: &Node) -> Option<Self> {
        let name = n.str(KEY_NAME)?.to_string();
        // the quad is a rectangle in the atlas: u0 v0 / u1 v1
        let uv = [
            f32_of(n, KEY_U0)?,
            f32_of(n, KEY_V0)?,
            f32_of(n, KEY_U1)?,
            f32_of(n, KEY_V1)?,
        ];
        Some(Self {
            name,
            width: n.float(KEY_WIDTH).unwrap_or(1.0),
            height: n.float(KEY_HEIGHT).unwrap_or(0.5),
            uv,
            size_min: f32_of(n, KEY_SIZE_MIN).unwrap_or(1.0),
            size_max: f32_of(n, KEY_SIZE_MAX).unwrap_or(1.0),
            lean: f32_of(n, KEY_LEAN).unwrap_or(0.5),
            sway: f32_of(n, KEY_SWAY).unwrap_or(0.0).clamp(0.0, 1.0),
            slope_min: f32_of(n, KEY_SLOPE_MIN).unwrap_or(0.0).to_radians(),
            slope_max: f32_of(n, KEY_SLOPE_MAX).unwrap_or(90.0).to_radians(),
            align: f32_of(n, KEY_ALIGN).unwrap_or(1.0),
        })
    }

    /// The type's `billboard` flag is 1 on every shipped type; kept for completeness.
    pub fn is_billboard(n: &Node) -> bool {
        n.int(KEY_BILLBOARD).unwrap_or(0) != 0
    }
}

/// A zone's `grass_types_N.bin`: the atlas it names and its types in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrassTypes {
    pub texture: String,
    pub types: Vec<GrassType>,
}

impl GrassTypes {
    pub fn parse(file: &PropertyFile) -> Self {
        let mut out = Self::default();
        for root in &file.nodes {
            if let Some(Variant::Str(s)) = root.value_hash(KEY_TEXTURE) {
                out.texture = s.clone();
            }
            if let Some((_, list)) = root.children.iter().find(|(k, _)| k.hash() == KEY_TYPES) {
                out.types.extend(
                    list.children
                        .iter()
                        .filter_map(|(_, n)| GrassType::parse(n)),
                );
            }
        }
        out
    }

    pub fn index_of(&self, name: &str) -> Option<u16> {
        self.types
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
            .map(|i| i as u16)
    }
}

/// One grass blade (three quads) the generator placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassBlade {
    /// Zone whose grass types `type_index` indexes.
    pub zone: u8,
    pub type_index: u16,
    /// The pass (0..4) that drew it: a density setting keeps the first passes.
    pub pass: u8,
    /// Engine axes, on the ground.
    pub position: [f32; 3],
    /// Yaw of the first quad (the others follow at 120° steps).
    pub yaw: f32,
    /// The size factor: `lerp(size_min, size_max, size)` of the type.
    pub size: f32,
    /// Ground normal under the blade.
    pub normal: [f32; 3],
    /// The sway phase byte (200..256).
    pub phase: u8,
}

impl GrassBlade {
    /// The size factor in metres-per-unit for a type.
    pub fn scale(&self, ty: &GrassType) -> f32 {
        ty.size_min + (ty.size_max - ty.size_min) * self.size
    }
}

/// The vertices of one quad of a blade, bottom left, bottom right, top right, top left,
/// as `(xz offset from the blade's centre, height above the ground there, uv)`. `ground`
/// gives the terrain height at a world position (`y` of the vertex is that plus
/// [`GROUND_LIFT`] plus the quad height). `k` is the quad (0..3).
pub struct QuadVertex {
    pub position: [f32; 3],
    /// Distance along the blade (0 at the ground, 1 at the tip), the sway weight.
    pub along: f32,
    pub uv: [f32; 2],
}

/// Build the four vertices of quad `k` of `blade`.
pub fn blade_quad(
    blade: &GrassBlade,
    ty: &GrassType,
    k: usize,
    ground_height: &dyn Fn(f32, f32) -> f32,
) -> [QuadVertex; 4] {
    let size = blade.scale(ty);
    let w = ty.width * size;
    let h = ty.height * size;
    let theta = blade.yaw + k as f32 * (TAU / 3.0);
    let (s, c) = theta.sin_cos();
    // the quad's horizontal axis and its (horizontal) normal
    let across = [c, -s];
    let normal = [s, c];
    let lean = h * ty.lean * LEAN_TILT;
    let [u0, v0, u1, v1] = ty.uv;
    // (vertical coordinate 0|1, horizontal −1|+1, u, v): the engine's V0..V3
    let corners = [
        (0.0, -1.0, u0, v1),
        (0.0, 1.0, u1, v1),
        (1.0, 1.0, u1, v0),
        (1.0, -1.0, u0, v0),
    ];
    corners.map(|(vx, vy, u, v)| {
        let mut x = blade.position[0] + across[0] * vy * w + normal[0] * lean * vx;
        let mut z = blade.position[2] + across[1] * vy * w + normal[1] * lean * vx;
        let gy = ground_height(x, z);
        // the blade follows the ground's tilt
        x += blade.normal[0] * ty.align * h * vx;
        z += blade.normal[2] * ty.align * h * vx;
        QuadVertex {
            position: [x, gy + GROUND_LIFT + h * vx, z],
            along: vx,
            uv: [u, v],
        }
    })
}

fn rand15(seed: i32) -> i32 {
    (seed >> 16) & 0x7fff
}

impl Vegetation {
    /// Grass of one 8 m patch whose corner with the smallest x and z is `(px, pz)`;
    /// `passes` (1..=4) is how many of the four 16-cell passes are drawn (4 is the
    /// engine's full density).
    pub fn generate_grass_patch(
        &self,
        px: f32,
        pz: f32,
        passes: u8,
        ground: &dyn Ground,
    ) -> Vec<GrassBlade> {
        let mut out = Vec::new();
        let mut seed = (pz * 435.0 + px * 231.0) as i32;
        for pass in 0..4u8 {
            for row in 0..GRASS_CELLS as i32 {
                let z0 = pz + row as f32;
                let col0 = (((row & 3) > 1) as i32 + pass as i32 + row * 2) & 3;
                for col in [col0, col0 + 4] {
                    let x0 = px + col as f32;
                    // six draws per cell, used or not
                    let a = crate::vegetation::lcg(seed);
                    let b = crate::vegetation::lcg(a);
                    let c = crate::vegetation::lcg(b);
                    let d = crate::vegetation::lcg(c);
                    let e = crate::vegetation::lcg(d);
                    let f = crate::vegetation::lcg(e);
                    seed = f;
                    if pass >= passes {
                        continue;
                    }
                    let size = rand15(c) as f32 * (1.0 / 32767.0);
                    let yaw = (rand15(d) as f32 * (2.0 / 32767.0) - 1.0) * TAU;
                    let x = (rand15(e) as f32 * (2.0 / 32767.0) - 1.0) * 0.5 + x0;
                    let z = (rand15(f) as f32 * (2.0 / 32767.0) - 1.0) * 0.5 + z0;
                    let Some(g) = ground.sample(x, z) else {
                        continue;
                    };
                    if g.height <= SEA_LEVEL {
                        continue;
                    }
                    let Some((zone, type_index)) = self.grass_type_at(&g, rand15(a)) else {
                        continue;
                    };
                    out.push(GrassBlade {
                        zone,
                        type_index,
                        pass,
                        position: [x, g.height, z],
                        yaw,
                        size,
                        normal: g.normal,
                        phase: (rand15(b) % 56 + 200) as u8,
                    });
                }
            }
        }
        out
    }

    /// The grass type the ground sample grows, given the cell's first draw: the
    /// engine's slot walk (see the module docs). `None` when the material has no set,
    /// its zone no grass, or no slot the ground's slope accepts.
    pub fn grass_type_at(
        &self,
        g: &crate::vegetation::GroundSample,
        draw: i32,
    ) -> Option<(u8, u16)> {
        let ms = self.materials.get(g.material as usize).copied().flatten()?;
        let zone = self.zone(ms.zone)?;
        let count = zone.grass.types.len();
        if count == 0 {
            return None;
        }
        let set = zone.sets.get(ms.set as usize)?;
        let table = set.table(crate::vegetation::Group::Grass);
        let sin_x = g.normal[0].sin().abs();
        let sin_z = g.normal[2].sin().abs();
        let (hi, lo) = (sin_x.max(sin_z), sin_x.min(sin_z));
        let accepts = |i: u16| {
            let t = &zone.grass.types[i as usize];
            hi <= t.slope_max && t.slope_min <= lo
        };
        if count < 2 {
            let i = table[0]?;
            return accepts(i).then_some((ms.zone, i));
        }
        let mut slot = draw as usize % count;
        for _ in 0..count {
            slot += 1;
            if slot == count {
                slot = 0;
            }
            if let Some(i) = table[slot & 0x3f]
                && (i as usize) < count
                && accepts(i)
            {
                return Some((ms.zone, i));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "grass_tests.rs"]
mod tests;
