//! Native point-light data (JC2-221, `research/formats/point-light-model.md`): the keyed light
//! component (type 7, `CECI_LightSource`) of the effect files and the placed
//! `CDynamicLightObject` lamps of the location containers. Decoding only; selection and
//! shading live in the game crate.
//!
//! Labels: **native** = read from the exe / data by the research probes; **inferred** = said so.

use crate::hash::hash_str;
use crate::placement::{Transform3, node_transform};
use crate::property::{Node, PropertyFile, Variant};
use crate::{GameFs, Result};

/// Key names whose strings were not recovered (hashes; model doc section 2 and 9.2).
const KEY_C: u32 = 0xc5ec_85ba;
const KEY_L: u32 = 0x12ca_621e;
const KEY_Q: u32 = 0x599f_e7f1;
const KEY_TIME: u32 = 0x103d_27f0;
const KEY_PRIORITY: u32 = 0x4367_6e89;
const KEY_OFFSET: u32 = 0x1b20_850d;
const KEY_MULTIPLIER: u32 = 0x51e2_e0de;
const KEY_FADE_IN: u32 = 0xd93f_d02e;

/// The radius the exe derives from the attenuation constants (`0x00957450`): the positive root
/// of `c + l r + q r^2 = 4`, 0 when there is none. Native (probe-verified over all keys).
pub fn derived_radius(c: f32, l: f32, q: f32) -> f32 {
    let (c, l, q) = (c as f64, l as f64, q as f64);
    let r = if q == 0.0 {
        if l > 0.0 { (4.0 - c) / l } else { 0.0 }
    } else {
        let h = l / q * 0.5;
        let disc = h * h - (c - 4.0) / q;
        if disc >= 0.0 { disc.sqrt() - h } else { 0.0 }
    };
    r as f32
}

/// One key of a light track (`node_i`): colour is `diffuse * multiplier`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightKey {
    pub time: f32,
    pub colour: [f32; 3],
    /// The explicit `radius` (> 0), `None` when the key derives it from `clq`.
    pub radius: Option<f32>,
    /// Constant, linear and quadratic attenuation (defaults 1, 1, 1).
    pub clq: [f32; 3],
}

impl LightKey {
    pub fn radius_here(&self) -> f32 {
        self.radius
            .unwrap_or_else(|| derived_radius(self.clq[0], self.clq[1], self.clq[2]))
    }
}

/// A keyed light of an effect: one `type = 7` component group.
#[derive(Clone, Debug, PartialEq)]
pub struct LightTrack {
    /// Added to the effect transform.
    pub offset: [f32; 3],
    pub looping: bool,
    /// Sort priority (key word 12 of the first node; default 1; no retail effect sets it).
    pub priority: i16,
    /// In `node_0..node_N` order.
    pub keys: Vec<LightKey>,
}

impl LightTrack {
    /// Total duration (the last key time).
    pub fn duration(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.time)
    }

    /// Colour and radius at clock `t` (`0x00959590`): linear between key k and k+1 with
    /// fraction `(t - t_k) / (t_k+1 - t_k)`, the index advancing when the interval is <= 0 or
    /// the fraction reaches 1. Radius is interpolated linearly only when both keys carry an
    /// explicit radius; otherwise c, l, q are interpolated and the radius derived. `None` once
    /// the clock is past the last key of a non-looping track (the light is removed); a looping
    /// track wraps the clock.
    pub fn evaluate(&self, t: f32) -> Option<([f32; 3], f32)> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        let mut t = t;
        let end = self.duration();
        if t >= end {
            if !self.looping || end <= 0.0 {
                return None;
            }
            t %= end;
        }
        let mut i = 0;
        while i + 1 < n
            && (self.keys[i + 1].time - self.keys[i].time <= 0.0 || t >= self.keys[i + 1].time)
        {
            i += 1;
        }
        if i + 1 >= n {
            let k = &self.keys[n - 1];
            return Some((k.colour, k.radius_here()));
        }
        let (a, b) = (&self.keys[i], &self.keys[i + 1]);
        let f = ((t - a.time) / (b.time - a.time)).clamp(0.0, 1.0);
        let lerp = |x: f32, y: f32| x + (y - x) * f;
        let colour = [
            lerp(a.colour[0], b.colour[0]),
            lerp(a.colour[1], b.colour[1]),
            lerp(a.colour[2], b.colour[2]),
        ];
        let radius = match (a.radius, b.radius) {
            (Some(x), Some(y)) => lerp(x, y),
            _ => derived_radius(
                lerp(a.clq[0], b.clq[0]),
                lerp(a.clq[1], b.clq[1]),
                lerp(a.clq[2], b.clq[2]),
            ),
        };
        Some((colour, radius))
    }
}

fn float(n: &Node, hash: u32) -> Option<f32> {
    match n.value_hash(hash)? {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn vec3(n: &Node, hash: u32) -> Option<[f32; 3]> {
    match n.value_hash(hash)? {
        Variant::Vec3(v) => Some(*v),
        Variant::Vec4(v) => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn key_of(n: &Node) -> LightKey {
    let diffuse = vec3(n, hash_str("diffuse")).unwrap_or([0.0; 3]);
    let mult = float(n, KEY_MULTIPLIER).unwrap_or(1.0);
    LightKey {
        time: float(n, KEY_TIME).unwrap_or(1.0),
        colour: diffuse.map(|c| c * mult),
        radius: float(n, hash_str("radius")).filter(|r| *r > 0.0),
        clq: [
            float(n, KEY_C).unwrap_or(1.0),
            float(n, KEY_L).unwrap_or(1.0),
            float(n, KEY_Q).unwrap_or(1.0),
        ],
    }
}

fn track_groups(n: &Node, node_hash: &[u32], out: &mut Vec<LightTrack>) {
    let mut keys: Vec<(usize, &Node)> = n
        .children
        .iter()
        .filter_map(|(k, c)| node_hash.iter().position(|h| *h == k.hash()).map(|i| (i, c)))
        .collect();
    if !keys.is_empty() {
        keys.sort_by_key(|(i, _)| *i);
        out.push(LightTrack {
            offset: vec3(n, KEY_OFFSET).unwrap_or([0.0; 3]),
            looping: matches!(n.value_hash(hash_str("looping")), Some(Variant::Int(1))),
            priority: keys
                .first()
                .and_then(|(_, k)| match k.value_hash(KEY_PRIORITY) {
                    Some(Variant::Int(p)) => i16::try_from(*p).ok(),
                    _ => None,
                })
                .unwrap_or(1),
            keys: keys.iter().map(|(_, k)| key_of(k)).collect(),
        });
        return;
    }
    for (_, c) in &n.children {
        track_groups(c, node_hash, out);
    }
}

/// The light tracks of one `type = 7` component variation node (the effect spawner builds one
/// light component per resolved variation).
pub fn variation_light_tracks(variation: &Node) -> Vec<LightTrack> {
    let node_hash: Vec<u32> = (0..64).map(|i| hash_str(&format!("node_{i}"))).collect();
    let mut out = Vec::new();
    track_groups(variation, &node_hash, &mut out);
    out
}

/// Every light track of a parsed effect file: each group of `node_i` children under a
/// `type = 7` component.
pub fn effect_light_tracks(file: &PropertyFile) -> Vec<LightTrack> {
    let node_hash: Vec<u32> = (0..64).map(|i| hash_str(&format!("node_{i}"))).collect();
    let type_hash = hash_str("type");
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if matches!(n.value_hash(type_hash), Some(Variant::Int(7))) {
                for (_, c) in &n.children {
                    track_groups(c, &node_hash, &mut out);
                }
            }
        });
    }
    out
}

/// The light tracks of the effect file `name` (for example `muzzle_bigcannon`); empty when the
/// file has no light component.
pub fn load_effect_lights(fs: &GameFs, name: &str) -> Result<Vec<LightTrack>> {
    let file = if name.ends_with(".bin") {
        name.to_string()
    } else {
        format!("{name}.bin")
    };
    let bytes = fs.read_inflated(&file)?;
    Ok(effect_light_tracks(&PropertyFile::parse_any(&bytes)?))
}

/// A placed `CDynamicLightObject` (native fields, model doc 9.2).
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicLight {
    /// Engine axes, absolute (matrix translation; no offset).
    pub position: [f32; 3],
    pub enabled: bool,
    /// `diffuse` bytes `(int)(x * 255)` as colours 0..1 (alpha is not read by any shader).
    pub diffuse: [f32; 3],
    pub multiplier: f32,
    /// The explicit radius, or the one derived from c, l, q when the key is absent / -1.
    pub radius: f32,
    pub fade_in: f32,
    /// -1 = never expires.
    pub duration: f32,
    pub priority: i16,
}

impl DynamicLight {
    /// `diffuse bytes / 255 * multiplier` (intensity 1).
    pub fn colour(&self) -> [f32; 3] {
        self.diffuse.map(|c| c * self.multiplier)
    }
}

fn walk_dynamic(n: &Node, parent: &Transform3, out: &mut Vec<DynamicLight>) {
    let mut here = *parent;
    let class = n.str("_class");
    if class.is_some()
        && let Some(local) = node_transform(n)
    {
        here = local.then(parent);
    }
    if class == Some("CDynamicLightObject") {
        let diffuse = match n.value("diffuse") {
            Some(Variant::Vec4(v)) => [v[0], v[1], v[2]],
            Some(Variant::Vec3(v)) => *v,
            _ => [0.0; 3],
        };
        let f = |name: &str, d: f32| n.float(name).unwrap_or(d);
        let explicit = f("radius", -1.0);
        out.push(DynamicLight {
            position: here.translation(),
            enabled: n.int("enabled").is_none_or(|e| e != 0),
            // 0x007d3e70 stores (int)(x * 255) bytes and divides by 255 again
            diffuse: diffuse.map(|c| ((c * 255.0) as i32).clamp(0, 255) as f32 / 255.0),
            multiplier: f("multiplier", 1.0),
            radius: if explicit != -1.0 {
                explicit
            } else {
                derived_radius(
                    f("constant_attenuation", 1.0),
                    f("linear_attenuation", 0.1),
                    f("quadratic_attenuation", 0.01),
                )
            },
            fade_in: float(n, KEY_FADE_IN).unwrap_or(0.2),
            duration: f("duration", -1.0),
            priority: n
                .int("priority")
                .and_then(|p| i16::try_from(p).ok())
                .unwrap_or(0),
        });
    }
    for (_, c) in &n.children {
        walk_dynamic(c, &here, out);
    }
}

/// Every `CDynamicLightObject` in a placement file (depth-first, transforms composed).
pub fn dynamic_lights(file: &PropertyFile) -> Vec<DynamicLight> {
    let mut out = Vec::new();
    for root in &file.nodes {
        walk_dynamic(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Key;

    fn key(time: f32, rgb: [f32; 3], radius: Option<f32>) -> LightKey {
        LightKey {
            time,
            colour: rgb,
            radius,
            clq: [1.0, 0.04, 0.001],
        }
    }

    #[test]
    fn derived_radius_matches_the_probe_examples() {
        // explosion_laser (1, 0.04, 0.001) -> 38.31 m; a fresh light's (0, 0.1, 0.01) -> 15.6 m;
        // the lamp default (1, 0.1, 0.01) -> 13.03 m; q = 0 -> (4 - c) / l; no root -> 0
        assert!((derived_radius(1.0, 0.04, 0.001) - 38.31).abs() < 0.01);
        assert!((derived_radius(0.0, 0.1, 0.01) - 15.6).abs() < 0.05);
        assert!((derived_radius(1.0, 0.1, 0.01) - 13.03).abs() < 0.01);
        assert_eq!(derived_radius(1.0, 0.1, 0.0), 30.0);
        assert_eq!(derived_radius(9.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn tracks_interpolate_wrap_and_expire() {
        let t = LightTrack {
            offset: [0.0, 1.0, 0.0],
            looping: false,
            priority: 1,
            keys: vec![
                key(0.0, [8.0, 4.0, 0.0], Some(10.0)),
                key(0.2, [0.0, 0.0, 0.0], Some(20.0)),
            ],
        };
        let (c, r) = t.evaluate(0.1).unwrap();
        assert_eq!((c[0], c[1], r), (4.0, 2.0, 15.0));
        assert!(
            t.evaluate(0.2).is_none(),
            "past the last key a non-looping light is removed"
        );
        let looped = LightTrack {
            looping: true,
            ..t.clone()
        };
        let (c, _) = looped.evaluate(0.3).unwrap();
        assert!((c[0] - 4.0).abs() < 1e-5, "the clock wraps");
        // a key without an explicit radius interpolates c, l, q and derives
        let mixed = LightTrack {
            keys: vec![key(0.0, [1.0; 3], None), key(1.0, [1.0; 3], Some(5.0))],
            ..t
        };
        assert!((mixed.evaluate(0.5).unwrap().1 - 38.31).abs() < 0.01);
    }

    #[test]
    fn dynamic_light_fields_and_defaults() {
        let mut n = Node::default();
        let put = |n: &mut Node, k: &str, v: Variant| n.values.push((Key::Name(k.into()), v));
        put(&mut n, "_class", Variant::Str("CDynamicLightObject".into()));
        put(&mut n, "diffuse", Variant::Vec4([1.0, 1.0, 0.6, 1.0]));
        put(&mut n, "multiplier", Variant::Float(3.5));
        put(&mut n, "radius", Variant::Float(11.0));
        put(
            &mut n,
            "world",
            Variant::Mat4x3([1., 0., 0., 0., 1., 0., 0., 0., 1., 5., 6., 7.]),
        );
        let l = &dynamic_lights(&PropertyFile {
            nodes: vec![n.clone()],
        })[0];
        assert_eq!(l.position, [5.0, 6.0, 7.0]);
        assert!(l.enabled);
        assert_eq!(
            (l.radius, l.priority, l.duration, l.fade_in),
            (11.0, 0, -1.0, 0.2)
        );
        assert!((l.colour()[0] - 3.5).abs() < 1e-6 && (l.colour()[2] - 0.6 * 3.5).abs() < 0.02);
        // radius -1: derived from the default (1, 0.1, 0.01)
        let mut m = n;
        m.values.retain(|(k, _)| k.hash() != hash_str("radius"));
        let l = &dynamic_lights(&PropertyFile { nodes: vec![m] })[0];
        assert!((l.radius - 13.03).abs() < 0.01);
    }

    #[test]
    fn retail_muzzle_bigcannon_and_missile_explosion() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).expect("game folder");
        let t = load_effect_lights(&fs, "muzzle_bigcannon").expect("muzzle_bigcannon.bin");
        // model doc section 5: offset (0,1,0), t0 (1, 0.65, 0.2) x8 r10; t0.2 black
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].offset, [0.0, 1.0, 0.0]);
        assert!((t[0].keys[0].colour[1] - 5.2).abs() < 1e-5);
        assert_eq!(t[0].keys[0].radius, Some(10.0));
        assert_eq!(t[0].keys[1].colour, [0.0; 3]);
        assert!((t[0].keys[1].time - 0.2).abs() < 1e-6);
        assert_eq!(t[0].priority, 1);
        let m = load_effect_lights(&fs, "missile_explosion").unwrap();
        assert_eq!(m[0].keys.len(), 3);
        assert_eq!(m[0].keys[1].colour, [8.0, 4.0, 0.0]);
        assert!(load_effect_lights(&fs, "muzzle_smg").is_ok_and(|t| t.is_empty()));
    }
}
