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
//! The sun's light colours and intensities are not in the data files that could be
//! found (the per-hour tables inside the exe's weather object start out as 1.0 and
//! `gradients.bmp` is an editor image); the game crate picks its own.

use crate::Result;
use crate::dds::DdsImage;
use crate::property::{Key, Node, PropertyFile, Variant};
use crate::vfs::GameFs;

pub const WEATHER_SETTINGS_FILE: &str = "weathersettings.bin";
pub const CLOUD_SHADOW_SETTINGS_FILE: &str = "cloudshadows.bin";
pub const SKY_GRADIENT_FILE: &str = "skygradient_dif.dds";
pub const FOG_GRADIENT_FILE: &str = "fog_gradient.dds";

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
}
