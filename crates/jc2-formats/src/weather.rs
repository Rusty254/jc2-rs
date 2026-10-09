//! The sky: where the sun and moon are at a given hour, what colour the sky is, and the
//! cloud settings (`research/formats/weather.md`).
//!
//! * `weathersettings.bin` holds the cloud system's constants (cloud base and height,
//!   mist, wind, default cloud cover). It has nothing about the time of day.
//! * The hour of the day (`CWorldTime`, a float 0..24) drives everything else: the
//!   exe turns it into the sun's direction ([`sun_direction`]) and uses it as the
//!   horizontal coordinate of `textures\atmosphere\skygradient_dif.dds` (a 32 × 128
//!   table: hour/24 across, zenith at the top row to horizon at the bottom).
//! * `fog_gradient.dds` is a 4 × 128 alpha ramp (fog amount); its colour is constant.
//!
//! The sun's light colours and intensities are not in `gradients.bmp`. That file is **not** an editor image: the exe loads it into ten
//! per-hour tables ([`GradientTables`], `research/formats/light-tables-next.md`, JC2-100),
//! which feed the cloud cards, the haze/fog colours, a point-light scale and a facade
//! factor, not the sun or the ambient. The sun/moon colour and ambients are driven by
//! `textures\landscape\gamezone_gradients.bmp` ([`GameZoneGradients`], with the constants of
//! `settings\landscape_effects.bin` in [`LandscapeEffects`]; JC2-115, JC2-122).

use crate::Result;
use crate::dds::DdsImage;
use crate::property::{Key, Node, PropertyFile, Variant};
use crate::vfs::GameFs;

pub const WEATHER_SETTINGS_FILE: &str = "weathersettings.bin";
pub const CLOUD_SHADOW_SETTINGS_FILE: &str = "cloudshadows.bin";
pub const SKY_GRADIENT_FILE: &str = "skygradient_dif.dds";
pub const FOG_GRADIENT_FILE: &str = "fog_gradient.dds";
/// `textures\atmosphere\gradients.bmp` (files are matched by name): the per-hour tables.
pub const GRADIENTS_FILE: &str = "gradients.bmp";

// weathersettings.bin keys (names found by guessing and hashing: see weather.md)
const CLOUD_BASE: u32 = 0x4763_06ad; // `cloud_base`
const CLOUD_HEIGHT: u32 = 0xc4af_68f5; // `cloud_height`
const MIST_COUNT: u32 = 0x21fc_2f7c; // `mist_count`
const MIST_ALTITUDE: u32 = 0x4f38_5ead; // `mist_altitude`
const K_B8B11972: u32 = 0xb8b1_1972;
const K_7ECE1F17: u32 = 0x7ece_1f17;
const K_D1E88637: u32 = 0xd1e8_8637;
const OVERCAST: u32 = 0x97d8_53b2;
const CLOUD_SHADOW_INTENSITY: u32 = 0xdd25_aaff;

/// The sole recovered name in `cloudshadows.bin`, with all remaining properties
/// retained by their original keys and variant types. Their meanings are unresolved.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudShadowSettings {
    pub intensity: f32,
    pub unknown: Vec<(Key, Variant)>,
}

impl CloudShadowSettings {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(CLOUD_SHADOW_SETTINGS_FILE)?)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        if file.nodes.len() != 1 || !file.nodes[0].children.is_empty() {
            return Err(crate::Error::format("cloudshadows.bin needs one flat node"));
        }
        let root = &file.nodes[0];
        let intensity = num(root, CLOUD_SHADOW_INTENSITY)
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                crate::Error::format("cloudshadows.bin needs finite cloud_shadow_intensity")
            })?;
        if root
            .values
            .iter()
            .filter(|(k, _)| k.hash() == CLOUD_SHADOW_INTENSITY)
            .count()
            != 1
        {
            return Err(crate::Error::format("duplicate cloud_shadow_intensity"));
        }
        Ok(Self {
            intensity,
            unknown: root
                .values
                .iter()
                .filter(|(k, _)| k.hash() != CLOUD_SHADOW_INTENSITY)
                .cloned()
                .collect(),
        })
    }

    /// Renderer policy, not a decoded original formula: bound opacity to 0..1 and
    /// disable it with zero/non-finite cover. The raw intensity remains unchanged.
    pub fn max_opacity(&self, cover: f32) -> f32 {
        if !cover.is_finite() || !self.intensity.is_finite() {
            return 0.0;
        }
        self.intensity.clamp(0.0, 1.0) * cover.clamp(0.0, 1.0)
    }
}

/// `weathersettings.bin`: the arguments of the cloud system's constructor.
///
/// Names are the real ones where the hash matched (`cloud_base`, `cloud_height`,
/// `mist_count`, `mist_altitude`, `acceleration`); the rest are named for what the
/// constructor does with them and marked assumed.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherSettings {
    /// `cloud_base`: 2000 (m above the sea, assumed).
    pub cloud_base: f32,
    /// `cloud_height`: 1320 (m, assumed: the clouds' vertical extent).
    pub cloud_height: f32,
    /// `mist_count`: 555 (particles).
    pub mist_count: f32,
    /// `mist_altitude`: 420 (m, assumed).
    pub mist_altitude: f32,
    /// `#b8b11972`: 555 (a count; assumed: soft-cloud billboards).
    pub k_b8b11972: f32,
    /// `#7ece1f17`: 250 (a count; assumed).
    pub k_7ece1f17: f32,
    /// `#d1e88637`: 20000 (m; assumed: the radius of the cloud field).
    pub k_d1e88637: f32,
    /// `acceleration`: 1e-6 (assumed: cloud drift).
    pub acceleration: f32,
    /// `#97d853b2`: 0.42 (assumed: default cloud cover / overcast, 0..1; the exe stores
    /// it at the weather object's `+0x85c`, which missions overwrite).
    pub overcast: f32,
}

impl Default for WeatherSettings {
    /// What the unmodded game's `settings\weathersettings.bin` (archive `pc0`) says. A mod in
    /// `dropzone\` may override it: the Definitive Edition mod ships cloud height 1320,
    /// mist 555 at 420 m, 555/250, acceleration 1e-6 and overcast 0.42.
    fn default() -> Self {
        Self {
            cloud_base: 2000.0,
            cloud_height: 800.0,
            mist_count: 500.0,
            mist_altitude: 550.0,
            k_b8b11972: 100.0,
            k_7ece1f17: 200.0,
            k_d1e88637: 20000.0,
            acceleration: 1.0,
            overcast: 1.0,
        }
    }
}

fn num(n: &Node, h: u32) -> Option<f32> {
    match n.value_hash(h)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

impl WeatherSettings {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = file
            .nodes
            .first()
            .ok_or_else(|| crate::Error::format("weathersettings.bin is empty"))?;
        let d = Self::default();
        Ok(Self {
            cloud_base: num(root, CLOUD_BASE).unwrap_or(d.cloud_base),
            cloud_height: num(root, CLOUD_HEIGHT).unwrap_or(d.cloud_height),
            mist_count: num(root, MIST_COUNT).unwrap_or(d.mist_count),
            mist_altitude: num(root, MIST_ALTITUDE).unwrap_or(d.mist_altitude),
            k_b8b11972: num(root, K_B8B11972).unwrap_or(d.k_b8b11972),
            k_7ece1f17: num(root, K_7ECE1F17).unwrap_or(d.k_7ece1f17),
            k_d1e88637: num(root, K_D1E88637).unwrap_or(d.k_d1e88637),
            acceleration: root.float("acceleration").unwrap_or(d.acceleration),
            overcast: num(root, OVERCAST).unwrap_or(d.overcast),
        })
    }
}

/// A small RGBA gradient texture, level 0 of a DDS.
#[derive(Clone, Debug)]
pub struct Gradient {
    pub width: usize,
    pub height: usize,
    /// RGBA8, row-major, row 0 on top.
    pub rgba: Vec<u8>,
}

impl Gradient {
    pub fn parse(dds: &[u8]) -> Result<Self> {
        let img = DdsImage::parse(dds)?;
        let rgba = img
            .decode_rgba(0)
            .ok_or_else(|| crate::Error::format("gradient: unsupported DDS format"))?;
        Ok(Self {
            width: img.width as usize,
            height: img.height as usize,
            rgba,
        })
    }

    /// One texel as 0..1 floats.
    pub fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        let i = (y.min(self.height - 1) * self.width + x.min(self.width - 1)) * 4;
        let p = &self.rgba[i..i + 4];
        [
            p[0] as f32 / 255.0,
            p[1] as f32 / 255.0,
            p[2] as f32 / 255.0,
            p[3] as f32 / 255.0,
        ]
    }

    /// Bilinear sample at `(u, v)` in 0..1 with texel centres at half steps, `u`
    /// wrapping (the sky table wraps at midnight) and `v` clamped.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let fx = u * self.width as f32 - 0.5;
        let fy = (v * self.height as f32 - 0.5).clamp(0.0, self.height as f32 - 1.0);
        let x0 = fx.floor();
        let tx = fx - x0;
        let w = self.width as i32;
        let xa = (x0 as i32).rem_euclid(w) as usize;
        let xb = (x0 as i32 + 1).rem_euclid(w) as usize;
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(self.height - 1);
        let ty = fy - y0 as f32;
        let mut out = [0.0; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let top = self.texel(xa, y0)[c] * (1.0 - tx) + self.texel(xb, y0)[c] * tx;
            let bot = self.texel(xa, y1)[c] * (1.0 - tx) + self.texel(xb, y1)[c] * tx;
            *o = top * (1.0 - ty) + bot * ty;
        }
        out
    }
}

/// Everything the sky needs from the data files.
#[derive(Clone, Debug)]
pub struct Atmosphere {
    pub settings: WeatherSettings,
    /// 32 × 128: columns are hours (`hour / 24`, wrapping), rows go from the zenith
    /// (0) to the horizon (1).
    pub sky: Gradient,
    /// 4 × 128: fog amount in the alpha channel, down the rows (assumed: distance or
    /// height); the colour is constant (255, 246, 255).
    pub fog: Gradient,
}

impl Atmosphere {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Ok(Self {
            settings: WeatherSettings::parse(&fs.read_inflated(WEATHER_SETTINGS_FILE)?)?,
            sky: Gradient::parse(&fs.read_inflated(SKY_GRADIENT_FILE)?)?,
            fog: Gradient::parse(&fs.read_inflated(FOG_GRADIENT_FILE)?)?,
        })
    }

    /// The sky's colour (0..1, sRGB as stored) at `hour` looking `v` of the way from
    /// the zenith (0) to the horizon (1).
    pub fn sky_colour(&self, hour: f32, v: f32) -> [f32; 3] {
        let c = self.sky.sample(hour.rem_euclid(24.0) / 24.0, v);
        [c[0], c[1], c[2]]
    }
}

/// Hours per table (columns of `gradients.bmp`); every table has one more entry (24 = entry 0).
pub const GRADIENT_HOURS: usize = 24;
const GRADIENT_ROWS: usize = 22;
/// `pow(x, 2.2)` with the double at 0x00e4aab0, the float 2.2 promoted (JC2-100, section 1).
const GRADIENT_GAMMA: f64 = 2.200_000_047_683_716;

/// The six RGB tables of the weather object (`FUN_00a46bd0`), named by their offsets in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RgbTable {
    /// `+0x660`: haze table 0 (role in clear weather unresolved, gap G2).
    A,
    /// `+0xb10`: haze table 1 (as `A`).
    B,
    /// `+0xfc0`: cloud card lit colour (`FUN_00a53a80`).
    C,
    /// `+0x1470`: cloud card shaded colour.
    D,
    /// `+0x1920`: haze table above the sea (`cb0[14]` in clear weather).
    E,
    /// `+0x1dd0`: haze table when the camera is below 200 m.
    F,
}

/// The ten per-hour tables the exe builds from `textures\atmosphere\gradients.bmp`
/// (a 24 x 22, 24 bpp picture: 24 hours across), exactly as `FUN_00a46bd0` does
/// (`research/formats/light-tables-next.md` section 1).
///
/// * `s0`: memory row 0, red only, raw.
/// * `s[v]`: memory row `1 + 7v`, red only, raw (point-light scale).
/// * `v[t][variant]`: memory rows `2 + 7 variant + t` (`t` = [`RgbTable`] order), RGB
///   `** 2.2` (linear), `w` = 1.
///
/// Memory row 0 is the top of the picture (the BMP is bottom-up); entry 24 duplicates
/// entry 0. Channels: R = byte 2, G = byte 1, B = byte 0, each `/ 255`.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientTables {
    pub s0: [f32; GRADIENT_HOURS + 1],
    pub s: [[f32; GRADIENT_HOURS + 1]; 3],
    pub v: [[[[f32; 4]; GRADIENT_HOURS + 1]; 3]; 6],
}

/// The weights of variants 1 and 2 (`FUN_00a40080`): `precip` is the precipitation
/// object's `+0x48`, `snow` its `+0x50`. Variant 0 is the base.
pub fn variant_weights(precip: f32, snow: f32) -> (f32, f32) {
    ((1.0 - snow) * precip, snow * precip)
}

/// `i = trunc(hour)`, `j = i + 1` wrapping to 0 above 23, `f = hour - i`: every table
/// reader in the exe (`0x00a53a9d`) interpolates linearly between those two entries.
fn hour_pair(hour: f32) -> (usize, usize, f32) {
    let h = if hour.is_finite() {
        hour.rem_euclid(24.0)
    } else {
        0.0
    };
    let i = (h as usize).min(GRADIENT_HOURS - 1);
    let j = if i + 1 > GRADIENT_HOURS - 1 { 0 } else { i + 1 };
    (i, j, (h - i as f32).clamp(0.0, 1.0))
}

impl GradientTables {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(GRADIENTS_FILE)?)
    }

    pub fn parse(bmp: &[u8]) -> Result<Self> {
        let bad = |m: &str| crate::Error::format(format!("gradients.bmp: {m}"));
        let u32_at = |o: usize| -> Result<u32> {
            bmp.get(o..o + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| bad("truncated header"))
        };
        if bmp.get(..2) != Some(b"BM") {
            return Err(bad("not a BMP"));
        }
        let off = u32_at(10)? as usize;
        let (w, h) = (u32_at(18)? as i32, u32_at(22)? as i32);
        let bpp = bmp
            .get(28..30)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| bad("truncated header"))?;
        if bpp != 24 || u32_at(30)? != 0 {
            return Err(bad("needs an uncompressed 24 bpp picture"));
        }
        if w != GRADIENT_HOURS as i32 || h != GRADIENT_ROWS as i32 {
            return Err(bad("needs 24 x 22 pixels (24 hours, 1 + 3 x 7 rows)"));
        }
        let pitch = (GRADIENT_HOURS * 3 + 3) & !3;
        if bmp.len() < off.saturating_add(pitch * GRADIENT_ROWS) {
            return Err(bad("truncated pixels"));
        }
        // memory row r is file row (h - 1 - r); pixel -> (R, G, B) = bytes (2, 1, 0) / 255
        let px = |r: usize, c: usize| -> [f32; 3] {
            let o = off + (GRADIENT_ROWS - 1 - r) * pitch + c * 3;
            [
                bmp[o + 2] as f32 / 255.0,
                bmp[o + 1] as f32 / 255.0,
                bmp[o] as f32 / 255.0,
            ]
        };
        let lin = |x: f32| (x as f64).powf(GRADIENT_GAMMA) as f32;
        let mut t = Self {
            s0: [0.0; GRADIENT_HOURS + 1],
            s: [[0.0; GRADIENT_HOURS + 1]; 3],
            v: [[[[0.0; 4]; GRADIENT_HOURS + 1]; 3]; 6],
        };
        for c in 0..GRADIENT_HOURS {
            t.s0[c] = px(0, c)[0];
            for var in 0..3 {
                t.s[var][c] = px(1 + 7 * var, c)[0];
                for tab in 0..6 {
                    let p = px(2 + 7 * var + tab, c);
                    t.v[tab][var][c] = [lin(p[0]), lin(p[1]), lin(p[2]), 1.0];
                }
            }
        }
        t.s0[GRADIENT_HOURS] = t.s0[0];
        for var in 0..3 {
            t.s[var][GRADIENT_HOURS] = t.s[var][0];
            for tab in 0..6 {
                t.v[tab][var][GRADIENT_HOURS] = t.v[tab][var][0];
            }
        }
        Ok(t)
    }

    fn scalar(tab: &[f32; GRADIENT_HOURS + 1], hour: f32) -> f32 {
        let (i, j, f) = hour_pair(hour);
        tab[i] + (tab[j] - tab[i]) * f
    }

    /// `S0` at `hour`, linear (the facade/merged-block factor, `CRenderEngine+0x29b4`).
    pub fn s0_at(&self, hour: f32) -> f32 {
        Self::scalar(&self.s0, hour)
    }

    /// The point-light scale table of one weather variant at `hour`.
    pub fn scalar_at(&self, variant: usize, hour: f32) -> f32 {
        Self::scalar(&self.s[variant.min(2)], hour)
    }

    /// The point-light scale with the variants mixed by `(w1, w2)` (`FUN_00a472f0`).
    pub fn scalar_blend(&self, hour: f32, w1: f32, w2: f32) -> f32 {
        let mut x = self.scalar_at(0, hour);
        for (var, w) in [(1, w1), (2, w2)] {
            if w > 0.0 {
                x += (self.scalar_at(var, hour) - x) * w;
            }
        }
        x
    }

    /// One RGB table of one weather variant at `hour` (linear RGB).
    pub fn rgb_at(&self, table: RgbTable, variant: usize, hour: f32) -> [f32; 3] {
        let tab = &self.v[table as usize][variant.min(2)];
        let (i, j, f) = hour_pair(hour);
        std::array::from_fn(|c| tab[i][c] + (tab[j][c] - tab[i][c]) * f)
    }

    /// An RGB table with the variants mixed by `(w1, w2)` (`FUN_00a53a80`, `FUN_00a432d0`):
    /// variant 0 is lerped toward variant 1 by `w1`, then toward variant 2 by `w2`.
    pub fn rgb_blend(&self, table: RgbTable, hour: f32, w1: f32, w2: f32) -> [f32; 3] {
        let mut x = self.rgb_at(table, 0, hour);
        for (var, w) in [(1, w1), (2, w2)] {
            if w > 0.0 {
                let y = self.rgb_at(table, var, hour);
                for c in 0..3 {
                    x[c] += (y[c] - x[c]) * w;
                }
            }
        }
        x
    }
}

// ---------------------------------------------------------------------------------------------
// gamezone_gradients.bmp: the native sun/moon colour and sky/ground ambient (JC2-115, JC2-122)
// ---------------------------------------------------------------------------------------------

/// `textures\landscape\gamezone_gradients.bmp` (named by `settings\landscape_effects.bin`; files
/// are matched by name).
pub const GAMEZONE_GRADIENTS_FILE: &str = "gamezone_gradients.bmp";
/// `settings\landscape_effects.bin`, matched by name.
pub const LANDSCAPE_EFFECTS_FILE: &str = "landscape_effects.bin";
/// Gradient sets that hold data: 0..7 the game zone types, 8 height, 9 underwater, 10 rain,
/// 11 snow. The picture has 16; sets 12..15 are black filler (night-light-next.md section 3).
pub const ZONE_SETS: usize = 12;
const ZONE_PICTURE_SETS: usize = 16;
/// Colour rows per set. Row 0 sun/moon colour, row 1 colour 2, rows 2/3 cloud colours, row 4 the
/// sky ambient (`cb0[10]`, up-facing), row 5 the ground ambient (`cb0[11]`, down-facing).
pub const ZONE_ROWS: usize = 6;
pub const ROW_SUN: usize = 0;
pub const ROW_SKY_AMBIENT: usize = 4;
pub const ROW_GROUND_AMBIENT: usize = 5;
/// The game zone type `Default` of `terrain\gamezone.bin` (id 7); its gradient set is 7.
pub const ZONE_TYPE_DEFAULT: usize = 7;
const ZONE_VARIANTS: usize = 2;
const ZONE_PICTURE_W: i32 = (ZONE_VARIANTS * GRADIENT_HOURS) as i32; // 48
const ZONE_PICTURE_H: i32 = (ZONE_PICTURE_SETS * ZONE_ROWS) as i32; // 96
/// `1 / 2.2` as the double at `0x00e5b860` (the float 2.2 promoted).
const GRADIENT_INV_GAMMA: f64 = 0.454_545_438_289_642_33;

/// The 12 data sets x 6 rows x 2 variants x 24 hours of `gamezone_gradients.bmp`
/// (`FUN_00a632b0`): pixel `(set * 6 + row, variant * 24 + hour)`, R = byte 2, memory row 0 =
/// top of the picture, `rgb ** 2.2` (linear); entry 24 of each variant = entry 0 (the day
/// wraps). Sets 12..15 are black filler and not kept. Index: `t[set][row][variant][hour]`.
pub type ZoneTable = [[[[[f32; 3]; GRADIENT_HOURS + 1]; ZONE_VARIANTS]; ZONE_ROWS]; ZONE_SETS];

#[derive(Clone, Debug, PartialEq)]
pub struct GameZoneGradients {
    pub t: Box<ZoneTable>,
}

impl GameZoneGradients {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(GAMEZONE_GRADIENTS_FILE)?)
    }

    /// A table from a function of `(set, row, variant, hour)` (hour 0..24; entry 24 repeats
    /// entry 0). For tests and synthetic fixtures.
    pub fn from_fn(f: impl Fn(usize, usize, usize, usize) -> [f32; 3]) -> Self {
        let mut t =
            Box::new([[[[[0.0; 3]; GRADIENT_HOURS + 1]; ZONE_VARIANTS]; ZONE_ROWS]; ZONE_SETS]);
        for (s, set) in t.iter_mut().enumerate() {
            for (r, row) in set.iter_mut().enumerate() {
                for (v, var) in row.iter_mut().enumerate() {
                    for (h, entry) in var.iter_mut().take(GRADIENT_HOURS).enumerate() {
                        *entry = f(s, r, v, h);
                    }
                    var[GRADIENT_HOURS] = var[0];
                }
            }
        }
        Self { t }
    }

    pub fn parse(bmp: &[u8]) -> Result<Self> {
        let bad = |m: &str| crate::Error::format(format!("gamezone_gradients.bmp: {m}"));
        let u32_at = |o: usize| -> Result<u32> {
            bmp.get(o..o + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| bad("truncated header"))
        };
        if bmp.get(..2) != Some(b"BM") {
            return Err(bad("not a BMP"));
        }
        let off = u32_at(10)? as usize;
        let (w, h) = (u32_at(18)? as i32, u32_at(22)? as i32);
        let bpp = bmp
            .get(28..30)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| bad("truncated header"))?;
        if bpp != 24 || u32_at(30)? != 0 {
            return Err(bad("needs an uncompressed 24 bpp picture"));
        }
        if w != ZONE_PICTURE_W || h != ZONE_PICTURE_H {
            return Err(bad("needs 48 x 96 pixels (16 sets x 6 rows, 2 x 24 hours)"));
        }
        let pitch = (w as usize * 3 + 3) & !3;
        if bmp.len() < off.saturating_add(pitch * h as usize) {
            return Err(bad("truncated pixels"));
        }
        // memory row r is file row (h - 1 - r); pixel -> (R, G, B) = bytes (2, 1, 0) / 255
        let px = |r: usize, c: usize| -> [f32; 3] {
            let o = off + (h as usize - 1 - r) * pitch + c * 3;
            [
                bmp[o + 2] as f32 / 255.0,
                bmp[o + 1] as f32 / 255.0,
                bmp[o] as f32 / 255.0,
            ]
        };
        let lin = |x: f32| (x as f64).powf(GRADIENT_GAMMA) as f32;
        Ok(Self::from_fn(|s, r, v, hour| {
            let p = px(s * ZONE_ROWS + r, v * GRADIENT_HOURS + hour);
            [lin(p[0]), lin(p[1]), lin(p[2])]
        }))
    }

    /// One row of one variant at `hour`, linear between hour `i` and `i + 1` (entry 24 wraps).
    pub fn row_at(&self, set: usize, row: usize, variant: usize, hour: f32) -> [f32; 3] {
        let tab = &self.t[set.min(ZONE_SETS - 1)][row.min(ZONE_ROWS - 1)][variant.min(1)];
        let (i, j, f) = hour_pair(hour);
        std::array::from_fn(|c| tab[i][c] + (tab[j][c] - tab[i][c]) * f)
    }

    /// The six rows of a set at `hour` for the variant weight `wv` (0..1):
    /// `wv * variant 0 + (1 - wv) * variant 1`, the base case of `FUN_00a5d390` (on the ground,
    /// no rain or snow; night-light-next.md section 4). That variant 0 is the high-intensity
    /// one is inferred; it only matters in sets 2, 3 and 9.
    pub fn rows(&self, set: usize, wv: f32, hour: f32) -> [[f32; 3]; ZONE_ROWS] {
        let wv = wv.clamp(0.0, 1.0);
        std::array::from_fn(|r| {
            let a = self.row_at(set, r, 0, hour);
            let b = self.row_at(set, r, 1, hour);
            std::array::from_fn(|c| a[c] * wv + b[c] * (1.0 - wv))
        })
    }
}

/// The final transform of the sun colour (vector 0 only; exe `0x00a5df12..0x00a5df52`):
/// `x -> ((x ** (1 / 2.2)) * gain) ** 2.2` per channel. With the picture's 255 and gain 2.2 the
/// maximum is `2.2 ** 2.2` = 5.667. The other vectors (colour 2, the ambients) are used raw.
pub fn sun_colour_transform(rgb: [f32; 3], gain: f32) -> [f32; 3] {
    rgb.map(|x| {
        let x = x.max(0.0) as f64;
        ((x.powf(GRADIENT_INV_GAMMA) * gain as f64).powf(GRADIENT_GAMMA)) as f32
    })
}

// settings\landscape_effects.bin keys. Names where the hash matched (`gradient_for_zone_NN`,
// `game_zones`, `max_gradient_N`); the rest are the hashes whose values the exe's loader
// `FUN_00a69830` reads from the exe key table (exe address and destination in the comments;
// night-light-next.md section 2).
const LE_GRADIENT_BLOCK: u32 = 0xe675_b7c8; // the block holding the gradient keys
const LE_IMAGE_PATH: u32 = 0x0c47_9c2f; // 0x00f1223c: "textures\landscape\gamezone_gradients.bmp"
const LE_ALTITUDE_A: u32 = 0x12c1_139f; // 0x00f12240 -> Z+0x1aec: 30 m
const LE_ALTITUDE_B: u32 = 0xe2ac_8f85; // 0x00f12244 -> Z+0x1af0: 300 m
const LE_ALTITUDE_C: u32 = 0x7157_f3a4; // 0x00f12248 -> Z+0x1ae4: 1000 m
const LE_UNDERWATER_DEPTH: u32 = 0xd91f_594f; // 0x00f1224c -> Z+0x1ae8: 120 m
const LE_SET_ALTITUDE: u32 = 0x5aa8_f383; // 0x00f12250 -> Z+0x1ad0: 8
const LE_SET_UNDERWATER: u32 = 0x33c9_5e4b; // 0x00f12254 -> Z+0x1ad4: 9
const LE_SET_RAIN: u32 = 0x49c4_c7d7; // 0x00f12258 -> Z+0x1ad8: 10
const LE_SET_SNOW: u32 = 0xa4b4_4b37; // 0x00f1225c -> Z+0x1adc: 11
const LE_GAIN: u32 = 0x9c93_19fc; // 0x00f121ec -> Z+0x147e0: 2.2 (constructor default 1.7)
const LE_SMOOTHING: u32 = 0xa08d_04c3; // root value -> Z+0x1ac0: 1.0 (the smoothing rate, 1/s)
/// Zone-type count of `gradient_for_zone_NN` (zone type N -> gradient set).
pub const ZONE_TYPES: usize = 8;
/// Entries of `game_zones` (`max_gradient_0..9`).
const MAX_GRADIENT_KEYS: usize = 10;

/// What `settings\landscape_effects.bin` gives the gradient system (`FUN_00a69830`).
#[derive(Clone, Debug, PartialEq)]
pub struct LandscapeEffects {
    /// The picture (`textures\landscape\gamezone_gradients.bmp`).
    pub image: String,
    /// `gradient_for_zone_NN`: the gradient set of game zone type N (identity in the retail
    /// file: Arctic 1, Desert 2, Jungle 3, Sea 4, City 5, Grass 6, Default 7, None 0).
    pub zone_set: [u32; ZONE_TYPES],
    /// The gain of the sun-colour transform (2.2 in the file; the exe constructor's own
    /// default is 1.7).
    pub gain: f32,
    /// The smoothing rate (key `#a08d04c3`, 1.0): the state moves toward the target by
    /// `clamp(rate * dt)` per frame.
    pub smoothing_rate: f32,
    /// `max_gradient_0..9` (3 each). The variant weight of a zone cell is
    /// `intensity / max_gradient` (probe reading of `Z[0x1a80 + set * 4]`).
    pub max_gradient: [f32; MAX_GRADIENT_KEYS],
    /// Sets used by the branches that are not implemented here (night-light-next.md gap 2):
    /// altitude 8, underwater 9, rain 10, snow 11.
    pub set_altitude: u32,
    pub set_underwater: u32,
    pub set_rain: u32,
    pub set_snow: u32,
    /// The altitude thresholds 30 / 300 / 1000 m and the underwater depth 120 m (same gap).
    pub altitude: [f32; 3],
    pub underwater_depth: f32,
}

impl LandscapeEffects {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(LANDSCAPE_EFFECTS_FILE)?)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let missing = |what: &str| {
            crate::Error::format(format!("landscape_effects.bin: missing or bad {what}"))
        };
        let file = PropertyFile::parse_any(data)?;
        let root = file
            .nodes
            .first()
            .ok_or_else(|| crate::Error::format("landscape_effects.bin is empty"))?;
        let block = root
            .children
            .iter()
            .find(|(k, _)| k.hash() == LE_GRADIENT_BLOCK)
            .map(|(_, n)| n)
            .ok_or_else(|| missing("gradient block #e675b7c8"))?;
        let image = match block.value_hash(LE_IMAGE_PATH) {
            Some(Variant::Str(s)) => s.clone(),
            _ => return Err(missing("gradient image path")),
        };
        let get = |n: &Node, h: u32, what: &str| num(n, h).ok_or_else(|| missing(what));
        let mut zone_set = [0u32; ZONE_TYPES];
        for (i, slot) in zone_set.iter_mut().enumerate() {
            let name = format!("gradient_for_zone_{i:02}");
            let v = get(block, crate::hash::hash_str(&name), &name)?;
            if !(0.0..ZONE_SETS as f32).contains(&v) {
                return Err(missing(&name));
            }
            *slot = v as u32;
        }
        let zones = root
            .child("game_zones")
            .ok_or_else(|| missing("game_zones"))?;
        let mut max_gradient = [0.0; MAX_GRADIENT_KEYS];
        for (i, slot) in max_gradient.iter_mut().enumerate() {
            let name = format!("max_gradient_{i}");
            *slot = get(zones, crate::hash::hash_str(&name), &name)?;
            if *slot <= 0.0 {
                return Err(missing(&name));
            }
        }
        let gain = get(block, LE_GAIN, "gradient gain")?;
        let smoothing_rate = get(root, LE_SMOOTHING, "smoothing rate #a08d04c3")?;
        Ok(Self {
            image,
            zone_set,
            gain,
            smoothing_rate,
            max_gradient,
            set_altitude: get(block, LE_SET_ALTITUDE, "altitude set")? as u32,
            set_underwater: get(block, LE_SET_UNDERWATER, "underwater set")? as u32,
            set_rain: get(block, LE_SET_RAIN, "rain set")? as u32,
            set_snow: get(block, LE_SET_SNOW, "snow set")? as u32,
            altitude: [
                get(block, LE_ALTITUDE_A, "altitude 30")?,
                get(block, LE_ALTITUDE_B, "altitude 300")?,
                get(block, LE_ALTITUDE_C, "altitude 1000")?,
            ],
            underwater_depth: get(block, LE_UNDERWATER_DEPTH, "underwater depth")?,
        })
    }

    /// The gradient set of a game zone type (0..7; larger types clamp to `Default`).
    pub fn set_for_zone_type(&self, zone_type: usize) -> usize {
        self.zone_set
            .get(zone_type)
            .copied()
            .unwrap_or(self.zone_set[ZONE_TYPE_DEFAULT]) as usize
    }

    /// The variant weight of the zone intensity (0..3, bits 6..7 of the cell byte) in the
    /// base case: `clamp(intensity / max_gradient[set])`. Sets above 9 have no weight in the
    /// file; the last one stands (they enter through the rain/snow weights natively).
    pub fn variant_weight(&self, set: usize, intensity: f32) -> f32 {
        let m = self.max_gradient[set.min(MAX_GRADIENT_KEYS - 1)];
        (intensity / m).clamp(0.0, 1.0)
    }
}

/// The exe's sun: the unit vector `a = (0, 1, 1) / √2` is turned about the axis
/// `b = (0, −1, 0.8)` (normalised) by `(hour − 12) · 15°`.
const SUN_REST: [f32; 3] = [0.0, 1.0, 1.0];
const SUN_AXIS: [f32; 3] = [0.0, -1.0, 0.8];

fn normalised(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Rodrigues rotation of `v` about the unit axis `n` by `angle`.
fn rotate(v: [f32; 3], n: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    let dot = v[0] * n[0] + v[1] * n[1] + v[2] * n[2];
    let cross = [
        n[1] * v[2] - n[2] * v[1],
        n[2] * v[0] - n[0] * v[2],
        n[0] * v[1] - n[1] * v[0],
    ];
    [
        v[0] * c + cross[0] * s + n[0] * dot * (1.0 - c),
        v[1] * c + cross[1] * s + n[1] * dot * (1.0 - c),
        v[2] * c + cross[2] * s + n[2] * dot * (1.0 - c),
    ]
}

/// Unit vector from the ground towards the sun, in the engine's axes (x east, y up,
/// z south: the map's axes, right-handed), at `hour` (0..24, wraps). At 12:00 the sun is
/// 45° up on the +z side; it rises on the +x side (about 5:40) and sets on the −x side
/// (about 18:20) and is 32° below the horizon at midnight.
///
/// Sign of the turn: the exe's matrix code was read as a row-vector rotation; the
/// direction was fixed by the sun having to come up on the +x side in the morning
/// (assumed: x is east).
pub fn sun_direction(hour: f32) -> [f32; 3] {
    let angle = (hour - 12.0) * std::f32::consts::PI / 12.0;
    normalised(rotate(normalised(SUN_REST), normalised(SUN_AXIS), angle))
}

/// The moon the exe draws twelve hours behind the sun on the same orbit.
pub fn moon_direction(hour: f32) -> [f32; 3] {
    sun_direction(hour + 12.0)
}

/// Degrees above the horizon of a direction (negative below).
pub fn elevation_degrees(dir: [f32; 3]) -> f32 {
    dir[1].clamp(-1.0, 1.0).asin().to_degrees()
}

/// `"HH:MM"`, `"H"`, or `"H.h"` → hours 0..24.
pub fn parse_clock(s: &str) -> Option<f32> {
    let s = s.trim();
    let h = if let Some((a, b)) = s.split_once(':') {
        let h: f32 = a.trim().parse().ok()?;
        let m: f32 = b.trim().parse().ok()?;
        if !(0.0..60.0).contains(&m) {
            return None;
        }
        h + m / 60.0
    } else {
        s.parse().ok()?
    };
    (0.0..=24.0).contains(&h).then_some(h % 24.0)
}

// ------------------------------------------------------------------------------------------
// The hour-dependent post-effect blocks (JC2-124 brief `night-exposure-next.md` section 2;
// JC2-134). `settings\landscape_effects.bin`, container `post_effects` (hash `d69d4965`).
// ------------------------------------------------------------------------------------------

/// The size of the post-effect manager's setting array `S(n)` (the floats at `P + 0x44 + 4 n`).
/// A block fills `S0..=S17`, `S25`, `S26`; the rest stay 0 (keys absent from the file).
pub const POST_SETTINGS: usize = 27;

/// Setting indices of the keys the blocks carry (brief section 2.2; the block parser
/// `FUN_0091a970`, the settings loader `FUN_0096f920` and the writer `FUN_0091d160` agree).
pub mod post {
    /// HighPass saturation mix.
    pub const HIGHPASS_SATURATION: usize = 0;
    /// HighPass exponent.
    pub const HIGHPASS_EXPONENT: usize = 1;
    /// Overlay strength, bright branch.
    pub const OVERLAY_BRIGHT: usize = 2;
    /// Overlay strength, dark branch.
    pub const OVERLAY_DARK: usize = 3;
    /// `blur`: blur blend.
    pub const BLUR: usize = 4;
    /// Depth-of-field far focus start (m).
    pub const DOF_FAR: usize = 5;
    /// Depth-of-field near focus (0 = off).
    pub const DOF_NEAR: usize = 6;
    /// Exposure offset + 1 (`cb1[2].x = S7 - 1`).
    pub const EXPOSURE_ONE: usize = 7;
    /// Auto-exposure x (weight of the darkest tap).
    pub const AUTO_X: usize = 8;
    /// Auto-exposure y (strength).
    pub const AUTO_Y: usize = 9;
    /// Auto-exposure key.
    pub const AUTO_KEY: usize = 10;
    /// Tint R.
    pub const TINT_R: usize = 11;
    /// Tint G.
    pub const TINT_G: usize = 12;
    /// Tint B.
    pub const TINT_B: usize = 13;
    /// `saturation`: saturation clamp end.
    pub const SATURATION_END: usize = 14;
    /// Saturation offset.
    pub const SATURATION_OFFSET: usize = 15;
    /// Saturation slope against luminance.
    pub const SATURATION_SLOPE: usize = 16;
    /// The other clamp end of the saturation factor.
    pub const SATURATION_OTHER_END: usize = 17;
    /// Depth-of-field blend gain.
    pub const DOF_GAIN: usize = 25;
    /// Heat-haze selector.
    pub const HEAT_HAZE: usize = 26;
}

/// A block of post-effect settings indexed by `S(n)`.
pub type PostBlock = [f32; POST_SETTINGS];

const PE_CONTAINER: &str = "post_effects";
const PE_NIGHT: u32 = 0x0a10_a074;
const PE_DAY: u32 = 0x0a48_aa9b;
const PE_HOURS: u32 = 0x9623_2764;
/// The four hour thresholds, in order 4.5, 5.5, 18.5, 19.5.
const PE_HOUR_KEYS: [u32; 4] = [0xf10e_7ec1, 0x2201_37a5, 0x88d3_6ba1, 0xf56f_fcb7];
/// The per-zone override container's gradient-set key and its four slot containers
/// (slot 1 and 2 patch the day pair, 3 and 4 the night pair).
const PE_ZONE_SET: u32 = 0x5767_7bdc;
const PE_SLOTS: [u32; 4] = [0x8eba_4969, 0x2e67_4fd1, 0xde6c_5fe4, 0x1eeb_7740];
/// Key hash of each setting `S(n)` (brief section 2.2). `blur` and `saturation` are names
/// whose hashes are `0xc07a98af` and `0x8b639e1e` (test `post_effect_key_names_hash_as_the_brief_says`).
const PE_KEYS: [(usize, u32); 20] = [
    (0, 0x84a1_ec42),
    (1, 0x1418_74c9),
    (2, 0xb989_17f0),
    (3, 0x2098_968f),
    (4, 0xc07a_98af),
    (5, 0xae11_0681),
    (6, 0xb862_814f),
    (7, 0x463e_ece2),
    (8, 0xa4d3_1fce),
    (9, 0x7772_4563),
    (10, 0x566b_d67c),
    (11, 0xab26_27cc),
    (12, 0xe57d_0a03),
    (13, 0x679d_ef2a),
    (14, 0x8b63_9e1e),
    (15, 0xcc44_e277),
    (16, 0xfad8_9ab2),
    (17, 0x15ba_5c5b),
    (25, 0xbd54_de7b),
    (26, 0x91fe_df25),
];

/// A zone's overrides: for the gradient set it names, single keys patched into slots 1..4
/// of the set's blocks (`FUN_00a6a550`; slot 4 follows slot 3 by sequence, brief section 2.1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostZoneOverride {
    /// The gradient set (key `#57677bdc`).
    pub set: usize,
    /// `(S index, value)` patches of slots 1, 2 (day) and 3, 4 (night).
    pub slots: [Vec<(usize, f32)>; 4],
}

/// The night and day post-effect blocks of `settings\landscape_effects.bin`, the hour
/// windows and the per-zone overrides. [`PostEffectBlocks::params`] is the exe's per-frame
/// block selection and blend (`FUN_00a5e010`).
#[derive(Clone, Debug, PartialEq)]
pub struct PostEffectBlocks {
    /// Block `#0a10a074`: before the first and from the last threshold.
    pub night: PostBlock,
    /// Block `#0a48aa9b`: between the second and third threshold.
    pub day: PostBlock,
    /// `#96232764`: 4.5, 5.5, 18.5, 19.5.
    pub hours: [f32; 4],
    pub overrides: Vec<PostZoneOverride>,
}

impl PostEffectBlocks {
    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(LANDSCAPE_EFFECTS_FILE)?)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = file
            .nodes
            .first()
            .ok_or_else(|| crate::Error::format("landscape_effects.bin is empty"))?;
        Self::from_root(root)
    }

    fn from_root(root: &Node) -> Result<Self> {
        let missing = |what: &str| {
            crate::Error::format(format!("landscape_effects.bin: missing or bad {what}"))
        };
        let post = root
            .child(PE_CONTAINER)
            .ok_or_else(|| missing("post_effects"))?;
        let block = |h: u32, what: &str| -> Result<PostBlock> {
            let node = child_by_hash(post, h).ok_or_else(|| missing(what))?;
            let mut b = [0.0; POST_SETTINGS];
            for (n, key) in PE_KEYS {
                b[n] = num(node, key).ok_or_else(|| missing(what))?;
            }
            Ok(b)
        };
        let night = block(PE_NIGHT, "night block #0a10a074")?;
        let day = block(PE_DAY, "day block #0a48aa9b")?;
        let hours_node = child_by_hash(post, PE_HOURS).ok_or_else(|| missing("hours #96232764"))?;
        let mut hours = [0.0; 4];
        for (slot, key) in hours.iter_mut().zip(PE_HOUR_KEYS) {
            *slot = num(hours_node, key).ok_or_else(|| missing("hour threshold"))?;
        }
        if !hours.windows(2).all(|w| w[0] < w[1]) {
            return Err(missing("hour thresholds (not increasing)"));
        }
        let mut overrides = Vec::new();
        for (_, zone) in &post.children {
            let Some(set) = num(zone, PE_ZONE_SET) else {
                continue; // the blocks and the hour container
            };
            let mut o = PostZoneOverride {
                set: set as usize,
                ..Default::default()
            };
            for (patch, slot_key) in o.slots.iter_mut().zip(PE_SLOTS) {
                if let Some(slot) = child_by_hash(zone, slot_key) {
                    for (n, key) in PE_KEYS {
                        if let Some(v) = num(slot, key) {
                            patch.push((n, v));
                        }
                    }
                }
            }
            overrides.push(o);
        }
        Ok(Self {
            night,
            day,
            hours,
            overrides,
        })
    }

    /// Slot `i` (0..4 = slots 1..4) of gradient `set`: the day (slots 1, 2) or night
    /// (3, 4) block with the zone's patches applied.
    fn slot(&self, set: usize, i: usize) -> PostBlock {
        let mut b = if i < 2 { self.day } else { self.night };
        for o in self.overrides.iter().filter(|o| o.set == set) {
            for &(n, v) in &o.slots[i] {
                b[n] = v;
            }
        }
        b
    }

    /// The settings the exe applies at `hour` (0..24) for gradient `set` (`FUN_00a5e010`):
    /// with `n` the number of thresholds `<= hour`, `n` in {2, 3} takes the day pair
    /// (slot 2 weight `1 - w`, slot 1 weight `w`) and otherwise the night pair (slot 4,
    /// slot 3); for odd `n` that is blended with the other pair across the window
    /// (`t = (hour - T[n-1]) / (T[n] - T[n-1])`: dawn night to day, dusk day to night).
    ///
    /// `w` is the intensity weight (`intensity byte * Z[0x1a80 + set * 4]`); it is the
    /// variant weight of [`LandscapeEffects::variant_weight`] here (**inferred**), and has
    /// no effect in the shipped data because both slots of a pair are identical except in
    /// set 8.
    pub fn params(&self, set: usize, w: f32, hour: f32) -> PostBlock {
        let pair = |day: bool| -> PostBlock {
            let (hi, lo) = if day { (0, 1) } else { (2, 3) };
            lerp_block(&self.slot(set, lo), &self.slot(set, hi), w.clamp(0.0, 1.0))
        };
        let n = self.hours.iter().filter(|&&t| t <= hour).count();
        let day = n == 2 || n == 3;
        let cur = pair(day);
        if n == 1 || n == 3 {
            let (lo, hi) = (self.hours[n - 1], self.hours[n]);
            let t = ((hour - lo) / (hi - lo)).clamp(0.0, 1.0);
            lerp_block(&cur, &pair(!day), t)
        } else {
            cur
        }
    }
}

/// A direct child container by key hash.
fn child_by_hash(n: &Node, h: u32) -> Option<&Node> {
    n.children
        .iter()
        .find(|(k, _)| k.hash() == h)
        .map(|(_, c)| c)
}

fn lerp_block(a: &PostBlock, b: &PostBlock, t: f32) -> PostBlock {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shadow_bytes(values: Vec<(Key, Variant)>) -> Vec<u8> {
        PropertyFile {
            nodes: vec![Node {
                values,
                ..Default::default()
            }],
        }
        .to_bytes()
    }

    #[test]
    fn shadow_settings_preserve_unknown_keys_and_types() {
        let unknown = vec![
            (Key::Hash(0x3440_8d49), Variant::Vec2([0.6, 0.8])),
            (Key::Hash(0x0056_15e6), Variant::Int(15)),
            (
                Key::Name("mod_extra".into()),
                Variant::Str("untouched".into()),
            ),
        ];
        let mut values = unknown.clone();
        values.push((
            Key::Name("cloud_shadow_intensity".into()),
            Variant::Float(0.85),
        ));
        let parsed = CloudShadowSettings::parse(&shadow_bytes(values)).unwrap();
        assert_eq!(parsed.intensity, 0.85);
        // Serialization groups named/hash sections, so the original mixed list
        // order is not part of the contract. Keys and typed values must survive.
        assert_eq!(parsed.unknown.len(), unknown.len());
        for property in unknown {
            assert!(parsed.unknown.contains(&property));
        }
    }

    #[test]
    fn shadow_settings_reject_missing_invalid_or_duplicate_intensity() {
        assert!(CloudShadowSettings::parse(&[]).is_err());
        assert!(CloudShadowSettings::parse(&shadow_bytes(vec![])).is_err());
        for value in [
            Variant::Float(f32::NAN),
            Variant::Float(f32::INFINITY),
            Variant::Vec2([0.8, 1.0]),
        ] {
            assert!(
                CloudShadowSettings::parse(&shadow_bytes(vec![(
                    Key::Hash(CLOUD_SHADOW_INTENSITY),
                    value
                )]))
                .is_err()
            );
        }
        assert!(
            CloudShadowSettings::parse(&shadow_bytes(vec![
                (Key::Hash(CLOUD_SHADOW_INTENSITY), Variant::Float(0.85)),
                (Key::Name("cloud_shadow_intensity".into()), Variant::Int(1)),
            ]))
            .is_err()
        );
    }

    #[test]
    fn shadow_opacity_is_bounded_and_clear_sky_is_off() {
        for intensity in [-1.0, 0.85, 4.0, f32::NAN] {
            let s = CloudShadowSettings {
                intensity,
                unknown: vec![],
            };
            for cover in [-1.0, 0.0, 0.42, 1.0, 2.0, f32::NAN] {
                assert!((0.0..=1.0).contains(&s.max_opacity(cover)));
            }
            assert_eq!(s.max_opacity(0.0), 0.0);
        }
        let s = CloudShadowSettings::parse(&shadow_bytes(vec![(
            Key::Hash(CLOUD_SHADOW_INTENSITY),
            Variant::Float(4.0),
        )]))
        .unwrap();
        assert_eq!(
            s.intensity, 4.0,
            "decoder must retain mod values before renderer clamping"
        );
        assert_eq!(s.max_opacity(1.0), 1.0);
    }

    #[test]
    fn real_cloud_shadow_settings() {
        let Some(fs) = game() else { return };
        let s = CloudShadowSettings::load(&fs).expect("plain-game cloudshadows.bin parses");
        assert!((s.intensity - 0.85).abs() < 1e-6);
        assert_eq!(s.unknown.len(), 10);
        assert_eq!(
            s.unknown
                .iter()
                .filter(|(_, v)| matches!(v, Variant::Vec2(_)))
                .count(),
            4
        );
        assert!(s.unknown.iter().any(
            |(k, v)| k.hash() == 0x7e6d_3167 && matches!(v, Variant::Float(x) if *x == 6000.0)
        ));
    }

    fn game() -> Option<GameFs> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        GameFs::open(dir).ok()
    }

    #[test]
    fn sun_orbit() {
        let noon = sun_direction(12.0);
        assert!((elevation_degrees(noon) - 45.0).abs() < 0.1);
        assert!(noon[0].abs() < 1e-4 && noon[2] > 0.7);
        let midnight = sun_direction(0.0);
        assert!(elevation_degrees(midnight) < -30.0);
        // up in the morning on +x, down in the evening on −x, symmetric about noon
        let (am, pm) = (sun_direction(8.0), sun_direction(16.0));
        assert!(am[0] > 0.8 && pm[0] < -0.8);
        assert!((am[1] - pm[1]).abs() < 1e-4);
        assert!(elevation_degrees(sun_direction(5.0)) < 0.0);
        assert!(elevation_degrees(sun_direction(7.0)) > 0.0);
        assert!(elevation_degrees(sun_direction(17.0)) > 0.0);
        assert!(elevation_degrees(sun_direction(19.0)) < 0.0);
        // wraps
        let a = sun_direction(3.0);
        let b = sun_direction(27.0);
        assert!((a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
        // unit length
        for h in 0..24 {
            let d = sun_direction(h as f32);
            assert!((d[0] * d[0] + d[1] * d[1] + d[2] * d[2] - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn clock_strings() {
        assert_eq!(parse_clock("22:00"), Some(22.0));
        assert_eq!(parse_clock("18:30"), Some(18.5));
        assert_eq!(parse_clock("7"), Some(7.0));
        assert_eq!(parse_clock("24:00"), Some(0.0));
        assert_eq!(parse_clock("25:00"), None);
        assert_eq!(parse_clock("10:75"), None);
        assert_eq!(parse_clock("noon"), None);
    }

    #[test]
    fn gradient_wraps_and_blends() {
        // 2 × 2: left column black, right column white
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]);
        }
        let g = Gradient {
            width: 2,
            height: 2,
            rgba,
        };
        let mid = g.sample(0.5, 0.5);
        assert!((mid[0] - 0.5).abs() < 1e-3);
        // u = 0 lies between the right column (wrapped) and the left one
        let wrap = g.sample(0.0, 0.5);
        assert!((wrap[0] - 0.5).abs() < 1e-3);
        assert!((g.sample(0.25, 0.5)[0]).abs() < 1e-3);
    }

    #[test]
    fn real_files() {
        let Some(fs) = game() else { return };
        if !fs.contains(WEATHER_SETTINGS_FILE) || !fs.contains(SKY_GRADIENT_FILE) {
            return;
        }
        let a = Atmosphere::load(&fs).expect("weather files parse");
        assert_eq!(a.settings, WeatherSettings::default());
        assert_eq!((a.sky.width, a.sky.height), (32, 128));
        assert_eq!((a.fog.width, a.fog.height), (4, 128));
        // noon horizon is pale, midnight horizon is grey-dark, dawn horizon is orange
        let noon = a.sky_colour(12.0, 0.95);
        let night = a.sky_colour(0.0, 0.95);
        let dawn = a.sky_colour(6.4, 0.95);
        assert!(noon[2] > 0.8, "{noon:?}");
        assert!(night[2] < 0.4, "{night:?}");
        assert!(dawn[0] > dawn[2] + 0.2, "{dawn:?}");
        // zenith is darker than the horizon at noon
        let zen = a.sky_colour(12.0, 0.0);
        assert!(zen[0] < noon[0]);
        // the fog ramp fades out
        assert!(a.fog.texel(0, 0)[3] > 0.9 && a.fog.texel(0, 127)[3] < 0.1);
    }

    // Fixtures below: values printed by `research/tools/light_tables_probe.py` (JC2-100).
    fn tables() -> Option<GradientTables> {
        let fs = game()?;
        if !fs.contains(GRADIENTS_FILE) {
            return None;
        }
        Some(GradientTables::load(&fs).expect("gradients.bmp parses"))
    }

    fn near3(a: [f32; 3], b: [f32; 3]) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-5, "{a:?} vs {b:?}");
        }
    }

    fn blank_bmp() -> Vec<u8> {
        let mut bmp = vec![0u8; 54 + 72 * 22];
        bmp[..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&24i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&22i32.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp
    }

    #[test]
    fn gradients_reject_malformed_pictures() {
        assert!(GradientTables::parse(&[]).is_err());
        assert!(GradientTables::parse(b"BM").is_err());
        let mut bmp = blank_bmp();
        assert!(GradientTables::parse(&bmp).is_ok());
        bmp[18..22].copy_from_slice(&23i32.to_le_bytes());
        assert!(GradientTables::parse(&bmp).is_err());
        assert!(GradientTables::parse(&blank_bmp()[..100]).is_err());
    }

    /// Row map, flip, channel order, gamma and the entry 24 copy on a synthetic picture.
    #[test]
    fn gradients_map_rows_channels_and_gamma() {
        let mut bmp = blank_bmp();
        // memory row m is file row 21 - m; bytes are B, G, R
        let mut set = |mem_row: usize, col: usize, r: u8, g: u8, b: u8| {
            let o = 54 + (21 - mem_row) * 72 + col * 3;
            bmp[o..o + 3].copy_from_slice(&[b, g, r]);
        };
        set(0, 5, 51, 0, 0); // S0, hour 5: 0.2
        set(8, 0, 255, 0, 0); // variant 1 scalar row (1 + 7)
        set(2 + 7 + 2, 3, 255, 128, 0); // variant 1, table C
        set(7, 0, 0, 255, 0); // variant 0, table F
        let t = GradientTables::parse(&bmp).unwrap();
        assert!((t.s0[5] - 0.2).abs() < 1e-6);
        assert_eq!(t.s[1][0], 1.0);
        assert_eq!(t.s[1][GRADIENT_HOURS], 1.0, "entry 24 = entry 0");
        let c = t.v[RgbTable::C as usize][1][3];
        assert_eq!(c[0], 1.0);
        assert!((c[1] - (128.0f32 / 255.0).powf(2.2)).abs() < 1e-5);
        assert_eq!((c[2], c[3]), (0.0, 1.0));
        assert_eq!(t.v[RgbTable::F as usize][0][0], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(t.v[RgbTable::F as usize][0][24], [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn variant_weights_follow_the_precipitation_object() {
        assert_eq!(variant_weights(0.5, 0.0), (0.5, 0.0));
        assert_eq!(variant_weights(0.5, 1.0), (0.0, 0.5));
        assert_eq!(variant_weights(0.0, 0.7), (0.0, 0.0));
        let (w1, w2) = variant_weights(0.8, 0.25);
        assert!((w1 - 0.6).abs() < 1e-6 && (w2 - 0.2).abs() < 1e-6);
    }

    #[test]
    fn hours_interpolate_linearly_and_wrap_23_to_0() {
        let mut t = GradientTables::parse(&blank_bmp()).unwrap();
        t.s0[23] = 0.4;
        t.s0[0] = 0.2;
        t.s0[24] = 0.2;
        assert!((t.s0_at(23.5) - 0.3).abs() < 1e-6);
        assert!((t.s0_at(23.0) - 0.4).abs() < 1e-6);
        assert!((t.s0_at(24.0) - 0.2).abs() < 1e-6, "24 wraps to 0");
        assert!((t.s0_at(-0.5) - 0.3).abs() < 1e-6);
        // variant blend: variant 0 -> 1 by w1, then toward 2 by w2
        t.v[2][0][12] = [0.0, 0.0, 0.0, 1.0];
        t.v[2][1][12] = [1.0, 1.0, 1.0, 1.0];
        t.v[2][2][12] = [0.0, 0.5, 1.0, 1.0];
        near3(t.rgb_blend(RgbTable::C, 12.0, 0.0, 0.0), [0.0; 3]);
        near3(t.rgb_blend(RgbTable::C, 12.0, 0.5, 0.0), [0.5; 3]);
        near3(t.rgb_blend(RgbTable::C, 12.0, 0.5, 0.5), [0.25, 0.5, 0.75]);
    }

    /// Probe values at 0/6/12/18 and 23.5 (clear weather, variant 0 unless noted; linear RGB).
    #[test]
    fn real_gradient_tables_match_the_probe() {
        let Some(t) = tables() else { return };
        assert!((t.s0_at(0.0) - 63.0 / 255.0).abs() < 1e-6);
        assert!((t.s0_at(6.0) - 0.184_313_73).abs() < 1e-6);
        assert_eq!(t.s0_at(12.0), 0.0);
        assert!((t.s0_at(18.0) - 0.196_078_43).abs() < 1e-6);
        assert!(
            (t.s0_at(23.5) - 0.264_705_88).abs() < 1e-6,
            "hour 23.5 wraps"
        );
        assert_eq!(t.scalar_at(0, 0.0), 1.0);
        assert!((t.scalar_at(0, 12.0) - 0.250_980_4).abs() < 1e-6);
        assert!(
            (t.scalar_blend(12.0, 1.0, 0.0) - 1.0).abs() < 1e-6,
            "variant 1 is 1.0"
        );
        // C = cloud lit, D = shaded
        near3(
            t.rgb_at(RgbTable::C, 0, 12.0),
            [0.560_499, 0.547_993, 0.517_401],
        );
        near3(
            t.rgb_at(RgbTable::D, 0, 12.0),
            [0.399_293, 0.388_91, 0.363_604],
        );
        near3(t.rgb_at(RgbTable::C, 0, 6.0), [1.0, 0.302_125, 0.136_099]);
        near3(t.rgb_at(RgbTable::C, 0, 18.0), [1.0, 0.388_91, 0.065_754]);
        near3(
            t.rgb_at(RgbTable::D, 0, 0.0),
            [0.011_881, 0.018_913, 0.026_549],
        );
        near3(t.rgb_at(RgbTable::A, 0, 12.0), [0.0, 0.320_382, 0.940_601]);
        near3(
            t.rgb_at(RgbTable::B, 0, 6.0),
            [0.141_98, 0.157_281, 0.197_516],
        );
        near3(
            t.rgb_at(RgbTable::E, 0, 12.0),
            [0.008_373, 0.130_352, 0.130_352],
        );
        near3(t.rgb_at(RgbTable::F, 0, 0.0), [0.0, 0.026_549, 0.044_553]);
        // variants 1 and 2, and the 23.5 wrap
        near3(
            t.rgb_at(RgbTable::A, 1, 6.0),
            [0.080_219, 0.082_414, 0.096_266],
        );
        near3(t.rgb_at(RgbTable::C, 1, 12.0), [0.015_175; 3]);
        near3(t.rgb_at(RgbTable::C, 0, 23.5), [0.100_115; 3]);
        near3(
            t.rgb_at(RgbTable::A, 2, 23.5),
            [0.035_629, 0.042_219, 0.046_962],
        );
        // the probe's variant mix of C: 12:00 with (w1, w2) = (0.5, 0.25), and 23.5
        near3(
            t.rgb_blend(RgbTable::C, 12.0, 0.5, 0.25),
            [0.356_003, 0.348_187, 0.329_067],
        );
        near3(t.rgb_blend(RgbTable::C, 23.5, 0.3, 0.6), [0.089_922; 3]);
        // gradients.bmp is 1640 bytes (probe BMP_SHA256 b66bf510...)
        let fs = game().unwrap();
        assert_eq!(fs.read_inflated(GRADIENTS_FILE).unwrap().len(), 1640);
    }

    // ---- gamezone_gradients.bmp / landscape_effects.bin (JC2-122) -------------------------

    /// A 48 x 96 24 bpp BMP whose pixel `(mem_row, x)` is `f(mem_row, x)` as (R, G, B) bytes;
    /// memory row 0 is the top of the picture = the last file row.
    fn zone_bmp(f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let (w, h, pitch) = (48usize, 96usize, 144usize);
        let mut bmp = vec![0u8; 54 + pitch * h];
        bmp[..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        bmp[22..26].copy_from_slice(&(h as i32).to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        for r in 0..h {
            for x in 0..w {
                let [red, g, b] = f(r, x);
                let o = 54 + (h - 1 - r) * pitch + x * 3;
                bmp[o..o + 3].copy_from_slice(&[b, g, red]);
            }
        }
        bmp
    }

    #[test]
    fn zone_picture_layout_channels_and_wrap() {
        // pixel (mem_row, x): R = 2 row, G = 5 x, B = 7
        let bmp = zone_bmp(|r, x| [r as u8 * 2, x as u8 * 5, 7]);
        let g = GameZoneGradients::parse(&bmp).unwrap();
        // set 2 row 3 = memory row 15; variant 1 hour 4 = column 28
        let e = g.t[2][3][1][4];
        let lin = |b: f32| ((b / 255.0) as f64).powf(2.200_000_047_683_716) as f32;
        near3(e, [lin(30.0), lin(140.0), lin(7.0)]);
        // entry 24 repeats entry 0; hour 23.5 sits halfway between them
        assert_eq!(g.t[0][0][0][24], g.t[0][0][0][0]);
        let mid = g.row_at(0, 0, 0, 23.5);
        for (c, m) in mid.iter().enumerate() {
            let want = (g.t[0][0][0][23][c] + g.t[0][0][0][0][c]) / 2.0;
            assert!((m - want).abs() < 1e-6);
        }
        // rows(): wv mixes variant 0 and 1
        let lo = g.rows(2, 0.0, 4.0)[3];
        let hi = g.rows(2, 1.0, 4.0)[3];
        near3(lo, g.row_at(2, 3, 1, 4.0));
        near3(hi, g.row_at(2, 3, 0, 4.0));
        let half = g.rows(2, 0.5, 4.0)[3];
        for c in 0..3 {
            assert!((half[c] - (lo[c] + hi[c]) / 2.0).abs() < 1e-6);
        }
        // sizes and bit depth are checked
        let mut small = bmp.clone();
        small[18..22].copy_from_slice(&24i32.to_le_bytes());
        assert!(GameZoneGradients::parse(&small).is_err());
        assert!(GameZoneGradients::parse(&bmp[..bmp.len() - 1]).is_err());
        assert!(GameZoneGradients::parse(b"nope").is_err());
    }

    #[test]
    fn the_sun_transform_peaks_at_2_2_to_the_2_2() {
        let top = sun_colour_transform([1.0, 1.0, 1.0], 2.2);
        for c in top {
            assert!((c - 5.666_696).abs() < 2e-3, "{top:?}");
        }
        assert_eq!(sun_colour_transform([0.0; 3], 2.2), [0.0; 3]);
        // gain 1: the 1/2.2 and 2.2 powers cancel
        let id = sun_colour_transform([0.25, 0.5, 0.75], 1.0);
        for (a, b) in id.iter().zip([0.25, 0.5, 0.75]) {
            assert!((a - b).abs() < 1e-5);
        }
        // a blended pixel p becomes (2.2 p) ** 2.2 in the linear domain
        let p = 0.5f64;
        let want = (2.2 * p).powf(2.2) as f32;
        assert!((sun_colour_transform([(p.powf(2.2)) as f32; 3], 2.2)[0] - want).abs() < 1e-3);
    }

    /// The probe's table (`night_light_probe.py`, brief section 5): set, hour, sun/moon colour
    /// after the transform, sky ambient `cb0[10]`, ground ambient `cb0[11]`, intensity 3.
    #[test]
    fn zone_gradients_match_the_probe_rows() {
        let Some(fs) = game() else { return };
        if !fs.contains(GAMEZONE_GRADIENTS_FILE) || !fs.contains(LANDSCAPE_EFFECTS_FILE) {
            return;
        }
        let raw = fs.read_inflated(GAMEZONE_GRADIENTS_FILE).unwrap();
        // 13,880 B, probe BMP_SHA256 b43e8c1b...
        assert_eq!(raw.len(), 13_880);
        let g = GameZoneGradients::parse(&raw).expect("gamezone_gradients.bmp parses");
        let le = LandscapeEffects::load(&fs).expect("landscape_effects.bin parses");
        type Row = (usize, f32, [f32; 3], [f32; 3], [f32; 3]);
        #[rustfmt::skip]
        let rows: [Row; 12] = [
            (1, 0.0,  [0.532, 0.616, 0.707], [0.048, 0.114, 0.170], [0.006, 0.006, 0.032]),
            (1, 4.0,  [0.532, 0.616, 0.707], [0.122, 0.201, 0.298], [0.006, 0.006, 0.032]),
            (1, 6.0,  [2.698, 0.909, 0.467], [0.284, 0.251, 0.298], [0.235, 0.263, 0.293]),
            (1, 9.0,  [2.383, 2.175, 1.662], [0.000, 0.080, 0.085], [0.723, 0.789, 0.745]),
            (1, 12.0, [1.922, 1.763, 1.376], [0.000, 0.021, 0.070], [0.883, 0.941, 0.867]),
            (1, 18.0, [1.977, 0.631, 0.164], [0.148, 0.231, 0.284], [0.247, 0.164, 0.099]),
            (1, 21.0, [0.532, 0.616, 0.707], [0.048, 0.114, 0.170], [0.006, 0.006, 0.032]),
            (7, 0.0,  [0.330, 0.723, 1.223], [0.008, 0.145, 0.316], [0.006, 0.006, 0.032]),
            (7, 6.0,  [4.646, 1.563, 0.805], [0.284, 0.251, 0.298], [0.028, 0.028, 0.028]),
            (7, 12.0, [5.667, 5.051, 3.697], [0.000, 0.017, 0.041], [0.730, 0.554, 0.263]),
            (7, 18.0, [3.394, 1.079, 0.280], [0.148, 0.231, 0.284], [0.145, 0.062, 0.024]),
            (7, 21.0, [0.330, 0.723, 1.223], [0.008, 0.145, 0.316], [0.006, 0.006, 0.032]),
        ];
        for (set, hour, sun, sky, gnd) in rows {
            let wv = le.variant_weight(set, 3.0);
            assert_eq!(wv, 1.0, "intensity 3 of max_gradient 3");
            let r = g.rows(set, wv, hour);
            let s = sun_colour_transform(r[ROW_SUN], le.gain);
            for c in 0..3 {
                let at = format!("set {set} hour {hour} channel {c}");
                assert!((s[c] - sun[c]).abs() < 6e-4, "{at} sun {s:?} vs {sun:?}");
                assert!((r[ROW_SKY_AMBIENT][c] - sky[c]).abs() < 6e-4, "{at} sky");
                assert!(
                    (r[ROW_GROUND_AMBIENT][c] - gnd[c]).abs() < 6e-4,
                    "{at} ground"
                );
            }
        }
        // sets 0..8 hold one constant night colour for hours 19..23 and 0..4 (row 0)
        for set in 0..9 {
            let night = g.t[set][ROW_SUN][0][0];
            for h in (0..5).chain(19..24) {
                assert_eq!(g.t[set][ROW_SUN][0][h], night, "set {set} hour {h}");
            }
        }
        // the variants differ in sets 2 (row 4) and 3 (row 5), not in 7 or 1
        let differs = |set: usize, row: usize| g.t[set][row][0] != g.t[set][row][1];
        assert!(differs(2, 4) && differs(3, 5) && !differs(7, 0) && !differs(1, 4));
    }

    #[test]
    fn landscape_effects_carry_the_zone_map_gain_and_weights() {
        let Some(fs) = game() else { return };
        if !fs.contains(LANDSCAPE_EFFECTS_FILE) {
            return;
        }
        // 2,637 B, probe LE_SHA256 43fa8c17...
        assert_eq!(
            fs.read_inflated(LANDSCAPE_EFFECTS_FILE).unwrap().len(),
            2_637
        );
        let le = LandscapeEffects::load(&fs).expect("landscape_effects.bin parses");
        assert_eq!(le.image, "textures\\landscape\\gamezone_gradients.bmp");
        // zone type N uses gradient set N
        assert_eq!(le.zone_set, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(le.set_for_zone_type(ZONE_TYPE_DEFAULT), 7);
        assert_eq!(
            le.set_for_zone_type(99),
            7,
            "unknown types fall back to Default"
        );
        assert!((le.gain - 2.2).abs() < 1e-6);
        assert_eq!(le.smoothing_rate, 1.0);
        assert_eq!(le.max_gradient, [3.0; 10]);
        assert_eq!(
            (le.set_altitude, le.set_underwater, le.set_rain, le.set_snow),
            (8, 9, 10, 11)
        );
        assert_eq!(le.altitude, [30.0, 300.0, 1000.0]);
        assert_eq!(le.underwater_depth, 120.0);
        // wv = intensity / 3
        assert_eq!(le.variant_weight(7, 3.0), 1.0);
        assert_eq!(le.variant_weight(7, 0.0), 0.0);
        assert!((le.variant_weight(7, 1.0) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(le.variant_weight(7, 9.0), 1.0, "clamped");
    }

    #[test]
    fn landscape_effects_reject_missing_keys() {
        assert!(LandscapeEffects::parse(&[]).is_err());
        let one = PropertyFile {
            nodes: vec![Node {
                values: vec![(Key::Hash(LE_SMOOTHING), Variant::Float(1.0))],
                ..Default::default()
            }],
        }
        .to_bytes();
        assert!(LandscapeEffects::parse(&one).is_err());
    }

    // ---- post-effect blocks (JC2-134; fixtures: night_exposure_probe.py data checks) ----

    use post::*;

    fn retail_blocks() -> Option<PostEffectBlocks> {
        let fs = game()?;
        fs.contains(LANDSCAPE_EFFECTS_FILE).then(|| {
            PostEffectBlocks::load(&fs).expect("landscape_effects.bin post_effects parses")
        })
    }

    #[test]
    fn post_effect_key_names_hash_as_the_brief_says() {
        assert_eq!(crate::hash::hash_str("blur"), 0xc07a_98af);
        assert_eq!(crate::hash::hash_str("saturation"), 0x8b63_9e1e);
        assert_eq!(crate::hash::hash_str("post_effects"), 0xd69d_4965);
    }

    #[test]
    fn retail_night_and_day_blocks_match_the_probe() {
        let Some(b) = retail_blocks() else { return };
        assert_eq!(b.hours, [4.5, 5.5, 18.5, 19.5]);
        let n = &b.night;
        // night #0a10a074: S8 0, S9 0.08, S10 0.32, S7 1, tint (1, 1, 1.6), S14 0.95, S15 0.6, S16 10, S17 0
        assert_eq!((n[AUTO_X], n[AUTO_Y], n[AUTO_KEY]), (0.0, 0.08, 0.32));
        assert_eq!(n[EXPOSURE_ONE], 1.0);
        assert_eq!((n[TINT_R], n[TINT_G], n[TINT_B]), (1.0, 1.0, 1.6));
        assert_eq!(
            (
                n[SATURATION_END],
                n[SATURATION_OFFSET],
                n[SATURATION_SLOPE],
                n[SATURATION_OTHER_END]
            ),
            (0.95, 0.6, 10.0, 0.0)
        );
        assert_eq!((n[OVERLAY_BRIGHT], n[OVERLAY_DARK]), (0.1, 0.8));
        assert_eq!((n[DOF_FAR], n[DOF_GAIN], n[BLUR]), (150.0, 0.3, 0.0));
        let d = &b.day;
        // day #0a48aa9b: S8 0.3, S9 0.1, S10 0.32, S7 1, tint (1,1,1), S14 1, S15 0.9, S16 -0.8, S17 1
        assert_eq!((d[AUTO_X], d[AUTO_Y], d[AUTO_KEY]), (0.3, 0.1, 0.32));
        assert_eq!(d[EXPOSURE_ONE], 1.0);
        assert_eq!((d[TINT_R], d[TINT_G], d[TINT_B]), (1.0, 1.0, 1.0));
        assert_eq!(
            (
                d[SATURATION_END],
                d[SATURATION_OFFSET],
                d[SATURATION_SLOPE],
                d[SATURATION_OTHER_END]
            ),
            (1.0, 0.9, -0.8, 1.0)
        );
        assert_eq!((d[OVERLAY_BRIGHT], d[OVERLAY_DARK]), (0.1, 0.7));
        assert_eq!((d[DOF_FAR], d[DOF_GAIN]), (120.0, 0.4));
        // the settings that differ between the blocks (probe): S3, S5, S8, S9, S13..S17, S25
        let diff: Vec<usize> = (0..POST_SETTINGS).filter(|&i| n[i] != d[i]).collect();
        assert_eq!(diff, [3, 5, 8, 9, 13, 14, 15, 16, 17, 25]);
    }

    fn sorted(v: &[(usize, f32)]) -> Vec<(usize, f32)> {
        let mut v = v.to_vec();
        v.sort_by_key(|p| p.0);
        v
    }

    #[test]
    fn retail_overrides_are_the_probes() {
        let Some(b) = retail_blocks() else { return };
        let zone = |set: usize| b.overrides.iter().find(|o| o.set == set);
        // City, Sea, Jungle, Grass and Default are present and empty
        for set in [3, 4, 5, 6, 7] {
            assert!(zone(set).unwrap().slots.iter().all(Vec::is_empty), "{set}");
        }
        let arctic = zone(1).unwrap();
        let day = vec![
            (OVERLAY_BRIGHT, 0.25),
            (AUTO_Y, 0.04),
            (TINT_B, 1.25),
            (SATURATION_END, 0.9),
        ];
        let mut want = day;
        want.sort_by_key(|p| p.0);
        assert_eq!(sorted(&arctic.slots[0]), want);
        assert_eq!(sorted(&arctic.slots[1]), want);
        // night slots 3, 4: tint reset to (1, 1, 1)
        let neutral = vec![(TINT_R, 1.0), (TINT_G, 1.0), (TINT_B, 1.0)];
        assert_eq!(sorted(&arctic.slots[2]), neutral);
        assert_eq!(sorted(&arctic.slots[3]), neutral);
        let desert = zone(2).unwrap();
        assert_eq!(desert.slots[0].len(), 12);
        assert!(desert.slots[2].is_empty() && desert.slots[3].is_empty());
        assert!(desert.slots[0].contains(&(HEAT_HAZE, 1.0)));
        assert!(desert.slots[0].contains(&(DOF_FAR, 50.0)));
        let water = zone(9).unwrap();
        let want_water = vec![
            (BLUR, 0.15),
            (DOF_FAR, 8.0),
            (TINT_B, 1.3),
            (DOF_GAIN, 0.65),
        ];
        for slot in &water.slots {
            assert_eq!(sorted(slot), want_water);
        }
        // height set 8: S5 150 in slots 2 and 4, 1500 in slots 1 and 3
        let height = zone(8).unwrap();
        assert_eq!(height.slots[0], [(DOF_FAR, 1500.0)]);
        assert_eq!(height.slots[1], [(DOF_FAR, 150.0)]);
        assert_eq!(height.slots[2], [(DOF_FAR, 1500.0)]);
        assert_eq!(height.slots[3], [(DOF_FAR, 150.0)]);
    }

    /// The probe's hour model (`block_params`): night before 4:30 and from 19:30, day from
    /// 5:30 to 18:30, linear 4:30..5:30 and 18:30..19:30; 5:00 and 19:00 are 50 % blends.
    #[test]
    fn the_hour_blend_follows_the_thresholds() {
        let Some(b) = retail_blocks() else { return };
        // a zone without overrides: Default, set 7
        let at = |h: f32| b.params(7, 1.0, h);
        for h in [0.0, 3.0, 4.0, 4.5] {
            assert_eq!(at(h), b.night, "night at {h}");
        }
        for h in [5.5, 6.0, 12.0, 18.0, 18.5] {
            assert_eq!(at(h), b.day, "day at {h}");
        }
        for h in [19.5, 21.0, 23.0, 23.99] {
            assert_eq!(at(h), b.night, "night at {h}");
        }
        // 5:00 and 19:00: half way (probe: S9 0.09, S13 1.3)
        for h in [5.0, 19.0] {
            let p = at(h);
            assert!((p[AUTO_Y] - 0.09).abs() < 1e-6, "{h}: {}", p[AUTO_Y]);
            assert!((p[TINT_B] - 1.3).abs() < 1e-6);
            assert!((p[SATURATION_OFFSET] - 0.75).abs() < 1e-6);
            assert!((p[SATURATION_SLOPE] - 4.6).abs() < 1e-5);
            assert!((p[AUTO_X] - 0.15).abs() < 1e-6);
        }
        // linear: a quarter of the way from night to day at 4:45, three quarters at 5:15
        assert!((at(4.75)[AUTO_Y] - 0.085).abs() < 1e-6);
        assert!((at(5.25)[AUTO_Y] - 0.095).abs() < 1e-6);
        // dusk goes the other way: 18:45 is a quarter of the way toward night
        assert!((at(18.75)[AUTO_Y] - 0.095).abs() < 1e-6);
        assert!((at(19.25)[AUTO_Y] - 0.085).abs() < 1e-6);
    }

    #[test]
    fn the_arctic_overrides_apply_to_the_day_and_night_pairs() {
        let Some(b) = retail_blocks() else { return };
        // set 1 = Arctic: day tint B 1.25, S9 0.04, saturation end 0.9, S2 0.25; night neutral
        let noon = b.params(1, 1.0, 12.0);
        assert_eq!(
            (
                noon[TINT_B],
                noon[AUTO_Y],
                noon[SATURATION_END],
                noon[OVERLAY_BRIGHT]
            ),
            (1.25, 0.04, 0.9, 0.25)
        );
        // other keys keep the day block's
        assert_eq!(noon[SATURATION_OFFSET], b.day[SATURATION_OFFSET]);
        let night = b.params(1, 1.0, 0.0);
        assert_eq!(
            (night[TINT_R], night[TINT_G], night[TINT_B]),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(night[SATURATION_END], b.night[SATURATION_END]);
        assert_eq!(night[AUTO_Y], b.night[AUTO_Y]);
        // dawn blends the two overridden blocks: B (1.0 + 1.25) / 2 at 5:00
        assert!((b.params(1, 1.0, 5.0)[TINT_B] - 1.125).abs() < 1e-6);
        // Desert: heat haze flag, S5 50 by day only
        let desert = b.params(2, 1.0, 12.0);
        assert_eq!((desert[HEAT_HAZE], desert[DOF_FAR]), (1.0, 50.0));
        assert_eq!(b.params(2, 1.0, 0.0)[HEAT_HAZE], 0.0);
        // a set without an override container is the plain block
        assert_eq!(b.params(0, 1.0, 12.0), b.day);
    }

    /// Slot 2 weight `1 - w`, slot 1 weight `w` (day); slot 4 `1 - w`, slot 3 `w` (night).
    #[test]
    fn the_intensity_weight_mixes_the_slots_of_a_pair() {
        let mut day = [0.0; POST_SETTINGS];
        day[AUTO_Y] = 0.1;
        let b = PostEffectBlocks {
            night: [0.0; POST_SETTINGS],
            day,
            hours: [4.5, 5.5, 18.5, 19.5],
            overrides: vec![PostZoneOverride {
                set: 8,
                slots: [
                    vec![(DOF_FAR, 1500.0)],
                    vec![(DOF_FAR, 150.0)],
                    vec![(DOF_FAR, 1500.0)],
                    vec![(DOF_FAR, 150.0)],
                ],
            }],
        };
        assert_eq!(b.params(8, 1.0, 12.0)[DOF_FAR], 1500.0);
        assert_eq!(b.params(8, 0.0, 12.0)[DOF_FAR], 150.0);
        assert!((b.params(8, 0.25, 12.0)[DOF_FAR] - (150.0 + 0.25 * 1350.0)).abs() < 1e-3);
        assert_eq!(b.params(8, 1.0, 0.0)[DOF_FAR], 1500.0);
        // the other sets are untouched
        assert_eq!(b.params(7, 0.5, 12.0)[DOF_FAR], 0.0);
        assert_eq!(b.params(7, 0.5, 12.0)[AUTO_Y], 0.1);
    }

    #[test]
    fn post_effect_blocks_reject_missing_containers() {
        assert!(PostEffectBlocks::parse(&[]).is_err());
        let one = PropertyFile {
            nodes: vec![Node {
                values: vec![(Key::Hash(LE_SMOOTHING), Variant::Float(1.0))],
                ..Default::default()
            }],
        }
        .to_bytes();
        assert!(PostEffectBlocks::parse(&one).is_err());
    }
}
