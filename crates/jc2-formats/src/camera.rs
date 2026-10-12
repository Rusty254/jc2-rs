//! The gameplay camera's data (JC2-141 research brief `research/formats/camera-fov-next.md`):
//! `settings/camera_settings.bin` (93 camera settings by id), `camera_defaults.bin` (the per-type
//! defaults, applied first), `camera_transitions.bin` (the FOV blend between two settings) and the
//! `cameras` / sniper-zoom sections of `settings/player.bin`.
//!
//! Native readers: the settings loader `007d44b0` / `00808070` (type defaults first, then the
//! entry's own keys; angles converted from degrees with the literal `0.017453292`), the
//! struct default `005852e0` (`Fov` = 45 deg), the transition loader `00660b20`, the player
//! loader `006f77a0` / `00660000`. Keys not named here are raw hashes (the names are unknown).

use crate::error::{Error, Result};
use crate::hash::hash_str;
use crate::property::{Node, PropertyFile, Variant};
use crate::vfs::GameFs;
use std::collections::BTreeMap;

/// The literal of every native degree-to-radian conversion (`007d44b0`).
pub const DEG_TO_RAD: f32 = 0.017_453_292;
/// `Fov` of a camera setting no key sets (`005852e0`: `0x42340000`), degrees.
pub const STRUCT_DEFAULT_FOV_DEG: f32 = 45.0;
/// The cap of the state FOV (`00838720`), radians (80.21 degrees).
pub const STATE_FOV_CAP: f32 = 1.4;
/// `_DAT_010655a0` (`/fovfactor=f` on the command line), initial and retail value.
pub const FOV_FACTOR: f32 = 1.0;
/// The manager's default transition (`00762eb0`: `[0x18] = [0x19] = 0.5`), seconds.
pub const DEFAULT_TRANSITION_SECONDS: f32 = 0.5;

const SETTINGS_GROUP: u32 = 0x30a7_489a;
const K_NAME: u32 = 0x54ab_6349;
const K_TYPE: u32 = 0x8cc2_f0ff;
const K_ID: u32 = 0xc7a6_2efe;
const K_FILTER: u32 = 0xfb71_b57e;
const K_SPEED_FOV: u32 = 0xcc21_87d9;
const K_SPEED_RANGE: u32 = 0xf8ae_9bfd;
const K_SPRING: u32 = 0xa3b4_2ba2;
const K_TRANS_POS: u32 = 0xe380_ec1d;
const K_TRANS_MODE: u32 = 0x77fd_4815;
const K_TRANS_FOV: u32 = 0xaa77_3667;
const K_TRANS_DELAY: u32 = 0x2d94_e355;
// Placement keys (`00808070`, `research/formats/parachute-camera-placement.md`, JC2-b78c).
const K_PIVOT: u32 = 0x93d9_cae6;
const K_CAMERA_OFFSET: u32 = 0x80cf_2614;
const K_TARGET_OFFSET: u32 = 0x15ed_1fea;
const K_YAW_LIMITS: u32 = 0x82bb_c61c;
const K_PITCH_LIMITS: u32 = 0x8fc2_7e6c;
const K_SMOOTH_TYPE: u32 = 0x5069_2f83;
const K_SMOOTH_RATE: u32 = 0x8e26_0e93;
const K_TARGET_NODE: u32 = 0x3e72_7c53;
const K_CAMERA_NODE: u32 = 0x0a4c_2bdc;
const K_MOVE_A: u32 = 0xc4af_b170;
const K_MOVE_B: u32 = 0xc26c_0ec2;
const K_TARGET_MODE: u32 = 0x1aca_be1c;
const K_COLLISION: u32 = 0x2945_85fd;
const K_INPUT_FILTER: u32 = 0xd318_5ed6;
const PLAYER_NODE: u32 = 0xa0f5_dc0e;
const SNIPER_SECTION: u32 = 0xa271_2e04;

/// A native `Fov`-style angle: degrees to radians as the f32 product with the native literal.
pub fn deg_to_rad(deg: f32) -> f32 {
    deg * DEG_TO_RAD
}

/// The filter type of a camera setting (key `0xfb71b57e`, factory `005d9760`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraFilter {
    /// 0 (the reverse cameras): none.
    None,
    /// 2: the vehicle speed filter (`00699c70` / `0068d720`).
    Speed,
    /// 4: the water filter ("Swimming").
    Water,
    /// 1 (combat FOV) and 3 (parachute): in the factory, selected by no retail setting.
    Other(i32),
}

impl CameraFilter {
    fn of(v: i32) -> Self {
        match v {
            0 => Self::None,
            2 => Self::Speed,
            4 => Self::Water,
            n => Self::Other(n),
        }
    }
}

/// One entry of `camera_settings.bin` with its type defaults applied.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraSetting {
    pub id: i32,
    pub name: String,
    /// The type string (`Normal`, `Car`, `Motorcycle`, `Boat`, `Chopper`, `Plane`, ...).
    pub kind: Option<String>,
    /// The effective `Fov` in degrees: own key, else the type default, else 45.
    pub fov_deg: f32,
    pub filter: Option<CameraFilter>,
    /// Speed filter: `[smin, smax]` in m/s (key `0xf8ae9bfd`).
    pub speed_range: Option<[f32; 2]>,
    /// Speed filter: the FOV at `smax`, degrees (key `0xcc2187d9`).
    pub speed_fov_deg: Option<f32>,
    /// Speed filter: spring `[k, c]` (key `0xa3b42ba2`).
    pub spring: Option<[f32; 2]>,
    /// The placement vectors and limits (`00838720` / `00688380`), when the record has any.
    pub placement: Placement,
}

/// The placement data of a camera record (board b78ce9ba; `parachute-camera-placement.md`).
/// Angles are radians (`007d44b0`). Every field is `None` when no key sets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    /// Key `0x93d9cae6` (`def+0x04`): pivot in the node's local frame. The native loader
    /// reads it only when the target and camera node ids are equal and non-zero; this
    /// field is filled under that same condition.
    pub pivot: Option<[f32; 3]>,
    /// Key `0x80cf2614` (`def+0x84`): camera offset from the pivot, camera-local, +Z behind.
    pub camera_offset: Option<[f32; 3]>,
    /// Key `0x15ed1fea` (`def+0x30`): target (look-at) offset.
    pub target_offset: Option<[f32; 3]>,
    /// Key `0x82bbc61c`: `[min, max]` yaw offset, radians.
    pub yaw_limits: Option<[f32; 2]>,
    /// Key `0x8fc27e6c`: `[min, max]` pitch offset, radians.
    pub pitch_limits: Option<[f32; 2]>,
    /// Key `0x50692f83`: camera pivot smoothing type (2 = interpolation, `00660f40`).
    pub smoothing_type: Option<i32>,
    /// Key `0x8e260e93`: smoothing rate, 1/s.
    pub smoothing_rate: Option<f32>,
    /// Keys `0x3e727c53` / `0x0a4c2bdc`: target node id, camera node id.
    pub nodes: Option<(i32, i32)>,
    /// Key `0xc4afb170`: movement parameter A (Parachute: yaw recentre seconds, 12.0).
    pub move_a: Option<f32>,
    /// Key `0xc26c0ec2`: movement parameter B (0.1; Freefall: input filter timer).
    pub move_b: Option<f32>,
    /// Key `0x1acabe1c` (record `+0xe8`, `00640fd0`): the target-matrix rotation mode of the
    /// movement class. 1 = the target is also rotated by the yaw and pitch offsets (SkyDive),
    /// 2 yaw only, 3 pitch only; absent (0) = no rotation.
    pub target_mode: Option<i32>,
    /// Key `0x294585fd` (`+0xbc`): collision filter class (1 Full, 8 Parachute; `00771930`).
    pub collision: Option<i32>,
    /// Key `0xd3185ed6` (`+0x104`): input filter class (1 Nonlinear, 2 NonlinearAccel;
    /// `00816230`).
    pub input_filter: Option<i32>,
}

impl Placement {
    /// The interpolation smoothing rate (type 2 only, `00576360`), 1/s.
    pub fn pivot_smoothing_rate(&self) -> Option<f32> {
        (self.smoothing_type == Some(2)).then_some(self.smoothing_rate).flatten()
    }
}

impl CameraSetting {
    /// The definition's FOV in radians (`def+0xc0`).
    pub fn fov(&self) -> f32 {
        deg_to_rad(self.fov_deg)
    }

    /// The controller state's FOV, `min(def.fov * fovfactor, 1.4)` (`00838720`).
    pub fn state_fov(&self, fov_factor: f32) -> f32 {
        (self.fov() * fov_factor).min(STATE_FOV_CAP)
    }

    /// The vehicle speed filter of this setting, when it has one with all its data.
    pub fn speed_filter(&self) -> Option<SpeedFilter> {
        if self.filter != Some(CameraFilter::Speed) {
            return None;
        }
        let [smin, smax] = self.speed_range?;
        let [k, c] = self.spring?;
        Some(SpeedFilter {
            smin,
            smax,
            base: self.fov(),
            peak: deg_to_rad(self.speed_fov_deg?),
            k,
            c,
        })
    }
}

/// The data of the speed filter (`00699c70` build, `0068d720` per frame); angles in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedFilter {
    pub smin: f32,
    pub smax: f32,
    pub base: f32,
    pub peak: f32,
    pub k: f32,
    pub c: f32,
}

/// Native dead band of the filter springs (`in_ECX[8] = 0x3727c5ac`).
pub const SPRING_DEAD_BAND: f32 = 1e-5;
/// Native cap of the spring step (`006e14c0`, `0068d720`).
pub const SPRING_MAX_DT: f32 = 0.1;

/// The spring state of one filter instance (`in_ECX[11]`, the velocity).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpeedFilterState {
    pub velocity: f32,
}

impl SpeedFilter {
    /// One frame of `0068d720` for the FOV: `fov` is the state FOV (`state+0x70`), `speed` the
    /// player character's |velocity| in m/s. Returns the new state FOV. The filter also moves
    /// the camera back by the `cot` difference (the "reverse dolly"); that is not modelled.
    pub fn step(&self, fov: f32, speed: f32, dt: f32, st: &mut SpeedFilterState) -> f32 {
        if speed <= self.smin {
            // spring back to the base (`006e14c0`)
            if fov != self.base {
                return self.spring(fov, self.base, dt, st);
            }
            return fov;
        }
        let t = ((speed - self.smin) / (self.smax - self.smin)).clamp(0.0, 1.0);
        let target = (self.peak - self.base) * t + self.base;
        self.spring(fov, target, dt, st)
    }

    fn spring(&self, fov: f32, target: f32, dt: f32, st: &mut SpeedFilterState) -> f32 {
        let dt = dt.min(SPRING_MAX_DT);
        let x = fov - target;
        let v = (-self.k * x - self.c * st.velocity) * dt + st.velocity;
        st.velocity = v;
        let mut x = v * dt + x;
        if x.abs() < SPRING_DEAD_BAND {
            x = 0.0;
        }
        x + target
    }
}

/// `camera_settings.bin` with `camera_defaults.bin` applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraSettings {
    pub settings: BTreeMap<i32, CameraSetting>,
}

fn variant_f32(v: &Variant) -> Option<f32> {
    match v {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn variant_pair(v: &Variant) -> Option<[f32; 2]> {
    match v {
        Variant::Vec2(p) => Some(*p),
        Variant::Floats(f) if f.len() >= 2 => Some([f[0], f[1]]),
        _ => None,
    }
}

fn val(n: &Node, key: u32) -> Option<&Variant> {
    n.value_hash(key)
}

fn int(n: &Node, key: u32) -> Option<i32> {
    match val(n, key)? {
        Variant::Int(i) => Some(*i),
        Variant::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn text(n: &Node, key: u32) -> Option<String> {
    match val(n, key)? {
        Variant::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// An entry's keys over the type defaults (`00808070`: the type section first).
#[derive(Default)]
struct Merged {
    fov: Option<f32>,
    filter: Option<i32>,
    speed_range: Option<[f32; 2]>,
    speed_fov: Option<f32>,
    spring: Option<[f32; 2]>,
    placement: Placement,
    pivot: Option<[f32; 3]>,
}

fn variant_vec3(v: &Variant) -> Option<[f32; 3]> {
    match v {
        Variant::Vec3(p) => Some(*p),
        Variant::Floats(f) if f.len() >= 3 => Some([f[0], f[1], f[2]]),
        _ => None,
    }
}

fn degrees_pair(v: &Variant) -> Option<[f32; 2]> {
    variant_pair(v).map(|[a, b]| [deg_to_rad(a), deg_to_rad(b)])
}

impl Merged {
    fn apply(&mut self, n: &Node) {
        if let Some(f) = val(n, hash_str("Fov")).and_then(variant_f32) {
            self.fov = Some(f);
        }
        if let Some(f) = int(n, K_FILTER) {
            self.filter = Some(f);
        }
        if let Some(p) = val(n, K_SPEED_RANGE).and_then(variant_pair) {
            self.speed_range = Some(p);
        }
        if let Some(f) = val(n, K_SPEED_FOV).and_then(variant_f32) {
            self.speed_fov = Some(f);
        }
        if let Some(p) = val(n, K_SPRING).and_then(variant_pair) {
            self.spring = Some(p);
        }
        let pl = &mut self.placement;
        if let Some(v) = val(n, K_PIVOT).and_then(variant_vec3) {
            self.pivot = Some(v);
        }
        if let Some(v) = val(n, K_CAMERA_OFFSET).and_then(variant_vec3) {
            pl.camera_offset = Some(v);
        }
        if let Some(v) = val(n, K_TARGET_OFFSET).and_then(variant_vec3) {
            pl.target_offset = Some(v);
        }
        if let Some(v) = val(n, K_YAW_LIMITS).and_then(degrees_pair) {
            pl.yaw_limits = Some(v);
        }
        if let Some(v) = val(n, K_PITCH_LIMITS).and_then(degrees_pair) {
            pl.pitch_limits = Some(v);
        }
        if let Some(v) = int(n, K_SMOOTH_TYPE) {
            pl.smoothing_type = Some(v);
        }
        if let Some(v) = val(n, K_SMOOTH_RATE).and_then(variant_f32) {
            pl.smoothing_rate = Some(v);
        }
        if let (Some(t), Some(c)) = (int(n, K_TARGET_NODE), int(n, K_CAMERA_NODE)) {
            pl.nodes = Some((t, c));
        }
        if let Some(v) = val(n, K_MOVE_A).and_then(variant_f32) {
            pl.move_a = Some(v);
        }
        if let Some(v) = val(n, K_MOVE_B).and_then(variant_f32) {
            pl.move_b = Some(v);
        }
        if let Some(v) = int(n, K_TARGET_MODE) {
            pl.target_mode = Some(v);
        }
        if let Some(v) = int(n, K_COLLISION) {
            pl.collision = Some(v);
        }
        if let Some(v) = int(n, K_INPUT_FILTER) {
            pl.input_filter = Some(v);
        }
    }
}

impl CameraSettings {
    pub fn parse(settings: &[u8], defaults: &[u8]) -> Result<Self> {
        let s = PropertyFile::parse_any(settings)?;
        let d = PropertyFile::parse_any(defaults)?;
        Self::from_property(&s, &d)
    }

    pub fn from_property(settings: &PropertyFile, defaults: &PropertyFile) -> Result<Self> {
        let sroot = settings
            .nodes
            .first()
            .ok_or_else(|| Error::format("camera_settings.bin: empty"))?;
        let droot = defaults.nodes.first();
        let group = sroot
            .children
            .iter()
            .find(|(k, _)| k.hash() == SETTINGS_GROUP)
            .map(|(_, n)| n)
            .ok_or_else(|| Error::format("camera_settings.bin: no settings group"))?;
        let mut out = Self::default();
        for (_, n) in &group.children {
            let Some(id) = int(n, K_ID) else { continue };
            let kind = text(n, K_TYPE);
            let mut m = Merged::default();
            if let (Some(t), Some(root)) = (&kind, droot) {
                // the type's section of camera_defaults.bin
                let th = hash_str(t);
                for (k, tn) in &root.children {
                    if k.hash() == th {
                        m.apply(tn);
                    }
                }
            }
            m.apply(n);
            let mut placement = m.placement.clone();
            // `00808070` reads the pivot only when both node ids are equal and non-zero
            placement.pivot = match placement.nodes {
                Some((t, c)) if t == c && t != 0 => m.pivot,
                _ => None,
            };
            out.settings.insert(
                id,
                CameraSetting {
                    id,
                    name: text(n, K_NAME).unwrap_or_default(),
                    kind,
                    fov_deg: m.fov.unwrap_or(STRUCT_DEFAULT_FOV_DEG),
                    filter: m.filter.map(CameraFilter::of),
                    speed_range: m.speed_range,
                    speed_fov_deg: m.speed_fov,
                    spring: m.spring,
                    placement,
                },
            );
        }
        Ok(out)
    }

    pub fn get(&self, id: i32) -> Option<&CameraSetting> {
        self.settings.get(&id)
    }
}

/// One group of `camera_transitions.bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionGroup {
    pub name: String,
    /// Position blend seconds (`+4`, key `0xe380ec1d`).
    pub pos_blend: f32,
    /// FOV weight ramp seconds (`+8`, key `0xaa773667`); 0 when absent (`00800d60`).
    pub fov_seconds: f32,
    /// Weight mode (`+0x28`, key `0x77fd4815`): 1 ramps, anything else jumps to 1.
    pub mode: i32,
    /// Start delay seconds (`+0x1c`, key `0x2d94e355`), 0 when absent.
    pub delay: f32,
    /// The `(From, To)` setting-id pairs.
    pub pairs: Vec<(i32, i32)>,
}

/// What a camera change starts: the FOV weight ramp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub fov_seconds: f32,
    pub mode: i32,
    pub delay: f32,
}

impl Transition {
    /// No mapping found: the manager's default 0.5 s ramp (`00762eb0`; mode 1 assumed).
    pub const DEFAULT: Transition = Transition {
        fov_seconds: DEFAULT_TRANSITION_SECONDS,
        mode: 1,
        delay: 0.0,
    };
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraTransitions {
    pub groups: Vec<TransitionGroup>,
}

impl CameraTransitions {
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::from_property(&PropertyFile::parse_any(data)?)
    }

    pub fn from_property(file: &PropertyFile) -> Result<Self> {
        let root = file
            .nodes
            .first()
            .ok_or_else(|| Error::format("camera_transitions.bin: empty"))?;
        let list = root
            .child("transitions")
            .ok_or_else(|| Error::format("camera_transitions.bin: no transitions"))?;
        let mut groups = Vec::new();
        for (_, n) in &list.children {
            let mut pairs = Vec::new();
            for (k, m) in &n.children {
                if k.hash() == hash_str("Mapping") {
                    for (_, e) in &m.children {
                        if let (Some(from), Some(to)) = (e.int("From"), e.int("To")) {
                            pairs.push((from, to));
                        }
                    }
                }
            }
            groups.push(TransitionGroup {
                name: text(n, K_NAME).unwrap_or_default(),
                pos_blend: val(n, K_TRANS_POS).and_then(variant_f32).unwrap_or(0.0),
                fov_seconds: val(n, K_TRANS_FOV).and_then(variant_f32).unwrap_or(0.0),
                mode: int(n, K_TRANS_MODE).unwrap_or(0),
                delay: val(n, K_TRANS_DELAY).and_then(variant_f32).unwrap_or(0.0),
                pairs,
            });
        }
        Ok(Self { groups })
    }

    /// The group that maps `from` to `to`, the first in file order.
    pub fn group(&self, from: i32, to: i32) -> Option<&TransitionGroup> {
        self.groups.iter().find(|g| g.pairs.contains(&(from, to)))
    }

    /// The transition a change `from` -> `to` starts (the manager's default when unmapped).
    pub fn lookup(&self, from: i32, to: i32) -> Transition {
        match self.group(from, to) {
            Some(g) => Transition {
                fov_seconds: g.fov_seconds,
                mode: g.mode,
                delay: g.delay,
            },
            None => Transition::DEFAULT,
        }
    }
}

/// The `cameras` and sniper sections of `settings/player.bin`'s `player` node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerCameras {
    /// `cameras` keys by hash (`OnFoot`, `Parachute`, ... and the raw ones).
    pub ids: BTreeMap<u32, i32>,
    /// Sniper zoom (section `0xa2712e04`): min FOV, max FOV (radians, `* 0.017453292`) and the
    /// zoom rate in rad/s. `None` when the section is absent.
    pub sniper: Option<SniperZoom>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SniperZoom {
    pub min_fov: f32,
    pub max_fov: f32,
    pub rate: f32,
}

/// The key hashes of the aim cameras (`006f77a0`; the key table at exe `0x00ec2150`).
pub const AIM_KEYS: [u32; 4] = [0xba61_3cde, 0x1535_a5d3, 0x18dc_14e2, 0xb930_65e2];
/// The crouch camera key and the unmounted-gun key.
pub const CROUCH_KEY: u32 = 0xf11f_6034;
pub const UNMOUNTED_GUN_KEY: u32 = 0x34bc_e195;

impl PlayerCameras {
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::from_property(&PropertyFile::parse_any(data)?)
    }

    pub fn from_property(file: &PropertyFile) -> Result<Self> {
        let root = file
            .nodes
            .first()
            .ok_or_else(|| Error::format("player.bin: empty"))?;
        let player = root
            .children
            .iter()
            .find(|(k, _)| k.hash() == PLAYER_NODE)
            .map(|(_, n)| n)
            .ok_or_else(|| Error::format("player.bin: no player node"))?;
        let mut out = Self::default();
        if let Some(c) = player.child("cameras") {
            for (k, v) in &c.values {
                if let Variant::Int(i) = v {
                    out.ids.insert(k.hash(), *i);
                }
            }
        }
        if let Some((_, s)) = player
            .children
            .iter()
            .find(|(k, _)| k.hash() == SNIPER_SECTION)
        {
            let f = |key| val(s, key).and_then(variant_f32);
            if let (Some(min), Some(max), Some(rate)) =
                (f(0x9f2c_04d4), f(0x94e9_5f3c), f(0x3631_a212))
            {
                out.sniper = Some(SniperZoom {
                    min_fov: deg_to_rad(min),
                    max_fov: deg_to_rad(max),
                    rate,
                });
            }
        }
        Ok(out)
    }

    /// A named `cameras` key (`OnFoot`, `Parachute`, `SkyDive`, `BaseJump`, `Swim`, `Dive`,
    /// `Ragdoll`, `ReeledIn`, `ReeledHang`, `Ladder`).
    pub fn id(&self, name: &str) -> Option<i32> {
        self.ids.get(&hash_str(name)).copied()
    }

    pub fn id_hash(&self, key: u32) -> Option<i32> {
        self.ids.get(&key).copied()
    }
}

/// Everything the gameplay camera needs, read from the game folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraData {
    pub settings: CameraSettings,
    pub transitions: CameraTransitions,
    pub player: PlayerCameras,
}

impl CameraData {
    pub fn load(fs: &GameFs) -> Result<Self> {
        let read = |name: &str| {
            fs.read_inflated(name)
                .or_else(|_| fs.read_inflated(&format!("settings/{name}")))
        };
        Ok(Self {
            settings: CameraSettings::parse(
                &read("camera_settings.bin")?,
                &read("camera_defaults.bin")?,
            )?,
            transitions: CameraTransitions::parse(&read("camera_transitions.bin")?)?,
            player: PlayerCameras::parse(&read("player.bin")?)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<CameraData> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        Some(CameraData::load(&fs).expect("camera data"))
    }

    fn f32_bits(deg: f32) -> f32 {
        deg_to_rad(deg)
    }

    #[test]
    fn speed_filter_springs_to_the_peak_and_back() {
        let f = SpeedFilter {
            smin: 10.0,
            smax: 40.0,
            base: f32_bits(50.0),
            peak: f32_bits(65.0),
            k: 20.0,
            c: 20.0,
        };
        let mut st = SpeedFilterState::default();
        let mut fov = f.base;
        // at the minimum speed the FOV stays on the base
        assert_eq!(f.step(fov, 10.0, 1.0 / 60.0, &mut st), f.base);
        for _ in 0..600 {
            fov = f.step(fov, 40.0, 1.0 / 60.0, &mut st);
        }
        assert!((fov - f.peak).abs() < 1e-4, "{fov}");
        for _ in 0..600 {
            fov = f.step(fov, 0.0, 1.0 / 60.0, &mut st);
        }
        assert!((fov - f.base).abs() < 1e-4, "{fov}");
        // half way between the speeds: half way between the FOVs once it settles
        for _ in 0..600 {
            fov = f.step(fov, 25.0, 1.0 / 60.0, &mut st);
        }
        assert!((fov - (f.base + f.peak) / 2.0).abs() < 1e-4, "{fov}");
        // the spring step is capped at 0.1 s
        let mut a = SpeedFilterState::default();
        let mut b = SpeedFilterState::default();
        assert_eq!(
            f.step(f.base, 40.0, 0.1, &mut a),
            f.step(f.base, 40.0, 5.0, &mut b)
        );
    }

    #[test]
    fn retail_settings_match_the_probe_fixtures() {
        let Some(d) = data() else { return };
        let s = &d.settings;
        assert_eq!(s.settings.len(), 93);
        // (id, name, type, effective FOV deg)
        let want: [(i32, &str, Option<&str>, f32); 25] = [
            (0, "Onfoot Camera", Some("Normal"), 40.0),
            (1, "Parachute Camera", None, 45.0),
            (2, "Freefall Camera", None, 45.0),
            (3, "Sniper1", Some("Normal"), 15.0),
            (4, "Sedan", Some("Car"), 50.0),
            (5, "Sniper on roof", Some("Normal"), 15.0),
            (12, "ATV", Some("Motorcycle"), 50.0),
            (24, "Reeling from wall", Some("Reeling"), 45.0),
            (25, "Ragdoll", None, 45.0),
            (26, "Shoulder cam", Some("Normal"), 20.0),
            (27, "Mounted Minigun", None, 40.0),
            (28, "Mounted Machinegun", None, 40.0),
            (30, "Reeled-in wall", None, 50.0),
            (33, "SmallBoat", Some("Boat"), 50.0),
            (34, "Small Plane", Some("Plane"), 45.0),
            (38, "Swimming", None, 45.0),
            (39, "Diving", None, 45.0),
            (40, "Ladder", None, 45.0),
            (44, "Chopper Medium", Some("Chopper"), 45.0),
            (65, "Reeled-in hang", None, 50.0),
            (67, "Roof shoulder cam", Some("Normal"), 25.0),
            (69, "Crouch Camera", Some("Normal"), 40.0),
            (79, "Arve Reverse", Some("Plane"), 65.0),
            (87, "Big Arve Reverse", Some("Plane"), 55.0),
            (89, "V010 Reverse", Some("Plane"), 65.0),
        ];
        for (id, name, kind, deg) in want {
            let c = s.get(id).unwrap_or_else(|| panic!("camera {id}"));
            assert_eq!(c.name, name, "{id}");
            assert_eq!(c.kind.as_deref(), kind, "{id}");
            assert_eq!(c.fov_deg, deg, "{id}");
        }
        // on foot: 40 degrees = 0.6981317 rad (f32 0.6981316804885864)
        assert_eq!(s.get(0).unwrap().fov(), 0.698_131_7);
        assert_eq!(s.get(0).unwrap().state_fov(FOV_FACTOR), 0.698_131_7);
        // the speed filter data
        let car = s.get(4).unwrap();
        assert_eq!(car.speed_range, Some([10.0, 40.0]));
        assert_eq!(car.speed_fov_deg, Some(65.0));
        assert_eq!(car.spring, Some([20.0, 20.0]));
        let boat = s.get(33).unwrap();
        assert_eq!(boat.speed_range, Some([5.0, 60.0]));
        assert_eq!(boat.speed_fov_deg, Some(75.0));
        let plane = s.get(34).unwrap();
        assert_eq!(plane.speed_range, Some([10.0, 80.0]));
        assert_eq!(plane.speed_fov_deg, Some(85.0));
        assert_eq!(plane.spring, Some([20.0, 10.0]));
        assert_eq!(s.get(44).unwrap().speed_fov_deg, Some(75.0));
        let bike = s.get(12).unwrap();
        assert_eq!(bike.speed_fov_deg, Some(60.0));
        assert_eq!(bike.spring, Some([25.0, 10.0]));
        assert_eq!(s.get(71).unwrap().filter, Some(CameraFilter::None));
        assert_eq!(s.get(38).unwrap().filter, Some(CameraFilter::Water));
        // every speed-filter setting has all its data; no setting selects the unused types
        for c in s.settings.values() {
            match c.filter {
                Some(CameraFilter::Speed) => assert!(c.speed_filter().is_some(), "{}", c.id),
                Some(CameraFilter::Other(n)) => panic!("{}: filter type {n}", c.id),
                _ => {}
            }
        }
    }

    #[test]
    fn retail_chute_camera_placement() {
        let Some(d) = data() else { return };
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        let p = &d.settings.get(1).unwrap().placement;
        assert!(near(p.pivot.unwrap(), [0.0, 3.75, -0.5]));
        assert!(near(p.camera_offset.unwrap(), [0.0, 0.0, 6.75]));
        assert!(near(p.target_offset.unwrap(), [0.0, -1.0, 0.0]));
        assert_eq!(p.yaw_limits, Some([deg_to_rad(-70.0), deg_to_rad(70.0)]));
        assert_eq!(p.pitch_limits, Some([deg_to_rad(-65.0), deg_to_rad(75.0)]));
        assert_eq!(p.pivot_smoothing_rate(), Some(5.0));
        assert_eq!(p.nodes, Some((2, 2)));
        assert_eq!(p.move_a, Some(12.0));
        let p = &d.settings.get(2).unwrap().placement;
        assert_eq!(p.pivot, None);
        assert!(near(p.camera_offset.unwrap(), [0.0, 3.0, 2.5]));
        assert!(near(p.target_offset.unwrap(), [0.0, 0.5, -1.0]));
        assert_eq!(p.yaw_limits, Some([deg_to_rad(-85.0), deg_to_rad(85.0)]));
        assert_eq!(p.pitch_limits, Some([deg_to_rad(-25.0), deg_to_rad(45.0)]));
        assert_eq!(p.pivot_smoothing_rate(), None);
        // on foot: 3.0 m behind, no pivot
        let p = &d.settings.get(0).unwrap().placement;
        assert!(near(p.camera_offset.unwrap(), [0.55, 1.55, 3.0]));
        assert_eq!(p.pivot, None);
    }

    #[test]
    fn retail_transitions_and_player_cameras() {
        let Some(d) = data() else { return };
        let t = &d.transitions;
        // 33 entries, 32 distinct names ("Roof_to_ClingJump" appears twice)
        assert_eq!(t.groups.len(), 33);
        let mut names: Vec<_> = t.groups.iter().map(|g| g.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 32);
        // a (From, To) pair listed by two groups with different timings: the native lookup
        // order (the registration in `00660b20` / `005f70b0`) is not decoded, so the first
        // group in file order wins here (Gap). None involves the on-foot, parachute, swim,
        // ragdoll or reeling settings jc2-rs selects between.
        let mut conflicts = Vec::new();
        for g in &t.groups {
            for &(f, to) in &g.pairs {
                let first = t.lookup(f, to);
                if t.groups.iter().any(|h| {
                    h.pairs.contains(&(f, to))
                        && (h.fov_seconds, h.mode) != (first.fov_seconds, first.mode)
                }) {
                    conflicts.push((f, to));
                }
            }
        }
        conflicts.sort();
        conflicts.dedup();
        eprintln!("camera_transitions: conflicting pairs {conflicts:?}");
        let states = [0, 1, 2, 25, 30, 38, 39, 65];
        assert!(
            conflicts
                .iter()
                .all(|(f, to)| !(states.contains(f) && states.contains(to))),
            "{conflicts:?}"
        );
        let by = |n: &str| t.groups.iter().find(|g| g.name == n).unwrap();
        for (name, pos, fov, mode) in [
            ("closeup_aim", 0.3, 0.4, 1),
            ("Sniper", 0.1, 0.1, 1),
            ("Crouch", 0.2, 0.2, 1),
            ("freefall", 0.5, 0.5, 1),
            ("mounted", 0.2, 0.3, 1),
            ("swimming", 0.5, 0.5, 1),
            ("diving", 0.5, 0.5, 1),
            ("ragdoll", 0.5, 0.5, 1),
            ("Hard_Cuts", 0.1, 0.5, 1),
            ("reverse_cam", 0.0, 0.0, 0),
        ] {
            let g = by(name);
            assert_eq!(
                (g.pos_blend, g.fov_seconds, g.mode),
                (pos, fov, mode),
                "{name}"
            );
        }
        assert_eq!(t.lookup(0, 26).fov_seconds, 0.4);
        assert_eq!(t.lookup(26, 0).fov_seconds, 0.4);
        assert_eq!(t.lookup(0, 3).fov_seconds, 0.1);
        assert_eq!(t.lookup(0, 69).fov_seconds, 0.2);
        assert_eq!(t.lookup(0, 2).fov_seconds, 0.5);
        assert_eq!(t.lookup(0, 27).fov_seconds, 0.3);
        // unmapped pair: the manager default
        assert_eq!(t.lookup(0, 1), Transition::DEFAULT);
        // entering a vehicle ramps in 0.25 s, leaving it in 0.5 s (groups enter_car, exit_car)
        assert_eq!(t.group(0, 4).unwrap().name, "enter_car");
        assert_eq!(t.lookup(0, 4).fov_seconds, 0.25);
        assert_eq!(t.lookup(0, 62).fov_seconds, 0.25);
        assert_eq!(t.group(4, 0).unwrap().name, "exit_car");
        assert_eq!(t.lookup(4, 0).fov_seconds, 0.5);
        // boats have no on-foot mapping: the default
        assert_eq!(t.lookup(0, 33), Transition::DEFAULT);
        assert_eq!(by("reverse_cam").pairs.len(), 44);
        // only the roof-stunt cameras (6, 7, 8, ...) carry a start delay (key 0x2d94e355)
        let delayed: Vec<_> = t
            .groups
            .iter()
            .filter(|g| g.delay != 0.0)
            .map(|g| (g.name.as_str(), g.delay))
            .collect();
        assert_eq!(
            delayed,
            [
                ("Cling_to_Roof", 0.1),
                ("Roof_to_Driver", 0.1),
                ("TestSpline", 0.15)
            ]
        );

        let p = &d.player;
        assert_eq!(p.ids.len(), 16);
        for (k, id) in [
            ("OnFoot", 0),
            ("Ladder", 0),
            ("Parachute", 1),
            ("SkyDive", 2),
            ("BaseJump", 2),
            ("Swim", 38),
            ("Dive", 39),
            ("Ragdoll", 25),
            ("ReeledIn", 30),
            ("ReeledHang", 65),
        ] {
            assert_eq!(p.id(k), Some(id), "{k}");
        }
        assert_eq!(
            AIM_KEYS.map(|k| p.id_hash(k)),
            [Some(26), Some(3), Some(5), Some(0)]
        );
        assert_eq!(p.id_hash(CROUCH_KEY), Some(69));
        assert_eq!(p.id_hash(UNMOUNTED_GUN_KEY), Some(0));
        let z = p.sniper.expect("sniper section");
        assert_eq!(z.min_fov, deg_to_rad(3.0));
        assert_eq!(z.max_fov, deg_to_rad(15.0));
        assert_eq!(z.rate, 0.4);
    }
}
