//! FMOD Designer event projects: `sound\fmod\*.fev`, `FEV1` version `0x3b0000` (FMOD
//! Designer / FMOD Event 4.x). Each file is one project (`Weapons`, `Vehicles`, …): the
//! wave banks it plays from (`Weapons_Common` → `weapons_common.fsb`), the category tree
//! that mixes it (`master/sfx/weapons/shots`), the event groups and their events, the
//! sound-definition templates, the sound definitions with their waveforms (bank + sample
//! index + weight) and the reverb presets. Everything is little-endian; a string is a
//! `u32` length that counts its terminating NUL, then the bytes.
//!
//! An event (`Single/Pistol_Shot`) is either *simple* (kind `0x10`: one sound) or
//! *complex* (kind `0x8`): layers, each driven by one of the event's parameters
//! (`distance`, `rpm`, `load`, `variation`, `(listener angle)`…). A layer holds sound
//! instances (a sound definition placed on the parameter's range, with crossfades,
//! loop mode and automatic pitch) and envelopes (an effect — volume, pitch, a DSP's
//! setting … — as a curve over one parameter). Every byte of the 17 retail files is
//! read; what the fields mean is in `research/formats/audio.md`, with the ones still
//! unexplained kept as raw values so parsing stays aligned.
//!
//! [`Event::voices`] evaluates an event the way the FMOD Event runtime would for one
//! start: which sound instances sound at the given parameter values, how loud, at what
//! pitch, looping or not, and which waveform of each sound definition plays.

use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"FEV1";
/// The version JC2's files carry.
pub const VERSION: u32 = 0x003b_0000;

/// `Event::mode` bits: FMOD Ex's `FMOD_MODE`.
pub const MODE_2D: u32 = 0x8;
pub const MODE_3D: u32 = 0x10;
pub const MODE_3D_HEADRELATIVE: u32 = 0x4_0000;
pub const MODE_3D_WORLDRELATIVE: u32 = 0x8_0000;
pub const MODE_3D_LOGROLLOFF: u32 = 0x10_0000;
pub const MODE_3D_LINEARROLLOFF: u32 = 0x20_0000;
pub const MODE_3D_CUSTOMROLLOFF: u32 = 0x400_0000;
pub const MODE_3D_IGNOREGEOMETRY: u32 = 0x4000_0000;

/// `Event::kind`.
pub const EVENT_COMPLEX: u32 = 0x8;
pub const EVENT_SIMPLE: u32 = 0x10;

/// `Envelope::kind` with the low three bits masked off (`ENVELOPE_EFFECT_MASK`): the
/// effect the envelope drives. Volume and pitch are certain (their curves sit at 1 and
/// 0.5 when idle); the others are named from the parameters they hang on (see
/// `research/formats/audio.md`).
pub const ENVELOPE_EFFECT_MASK: u32 = !0x7;
pub const ENVELOPE_VOLUME: u32 = 0x8;
pub const ENVELOPE_PITCH: u32 = 0x10;
/// 0..1 on `(distance)` (0 up close, 1 from a few metres): how much the 3D position pans
/// the sound ("3D pan level", assumed; it is what keeps a gun at Rico's feet centred).
pub const ENVELOPE_PAN_LEVEL: u32 = 0x400;
/// 0..1 on `(listener angle)` (0 in front, 0.4 behind) and `(distance)`: occlusion, how
/// much the sound is muffled by a low-pass filter (assumed from its shape).
pub const ENVELOPE_OCCLUSION: u32 = 0x2000;

/// A pitch in FMOD Event's raw units is this many octaves (assumed: the designers'
/// values are multiples of 0.0025 = 0.01 octave).
pub const OCTAVES_PER_PITCH_UNIT: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct WaveBank {
    /// `Weapons_Common`; the file is the name in lower case plus `.fsb`.
    pub name: String,
    /// `0x200` decompress into memory (the sound effects), `0x80` stream from disk
    /// (music, dialogue, cutscenes).
    pub load_mode: u32,
    pub max_streams: u32,
}

impl WaveBank {
    /// The `.fsb` this bank is in: `weapons_common.fsb`.
    pub fn file_name(&self) -> String {
        format!("{}.fsb", self.name.to_ascii_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    /// Two words, zero in every retail file.
    pub unknown: [u32; 2],
    pub children: Vec<Category>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Int(i32),
    Float(f32),
    String(String),
}

/// A user property (none in JC2's files; the layout is xoreos's).
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: PropertyValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventGroup {
    pub name: String,
    pub properties: Vec<Property>,
    pub groups: Vec<EventGroup>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    /// The groups and the name: `Single/Pistol_Shot` (names can hold a `/` themselves:
    /// `Acoustics/EarlyReflection_Cannon/FLAK_Ridge`).
    pub path: String,
    /// `EVENT_COMPLEX` or `EVENT_SIMPLE`.
    pub kind: u32,
    /// 16 bytes after the name; shared by events made from the same template.
    pub guid: [u8; 16],
    /// Linear gain.
    pub volume: f32,
    /// Raw pitch units (`OCTAVES_PER_PITCH_UNIT`).
    pub pitch: f32,
    /// ± this much at random on each start (raw pitch units).
    pub pitch_randomization: f32,
    /// Always 0 but in two events.
    pub volume_randomization: f32,
    pub priority: u32,
    pub max_playbacks: u32,
    /// 0..10000.
    pub steal_priority: u32,
    /// `MODE_*` bits: 2D or 3D, world or head relative, the rolloff.
    pub mode: u32,
    /// 3D: full volume inside, inaudible past (with a custom rolloff the `(distance)`
    /// envelopes shape the fall and `max_distance` is the end of that parameter).
    pub min_distance: f32,
    pub max_distance: f32,
    /// More mode-like bits (`0x80000`, `0x80000000`, `0x40080000`); unexplained.
    pub flags: u32,
    /// 2D speaker levels: L, R, C, LFE, LR, RR, LS, RS.
    pub speaker_levels: [f32; 8],
    pub cone_inside: f32,
    pub cone_outside: f32,
    pub cone_outside_volume: f32,
    pub max_playbacks_behavior: u32,
    pub doppler: f32,
    pub reverb_dry: f32,
    pub reverb_wet: f32,
    pub speaker_spread: f32,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub spawn_intensity: f32,
    pub spawn_intensity_randomization: f32,
    pub pan_level: f32,
    pub position_randomization: u32,
    pub layers: Vec<Layer>,
    pub params: Vec<Param>,
    pub properties: Vec<Property>,
    /// The category that mixes it: `sfx/weapons/shots`.
    pub category: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// 2 in every layer.
    pub flags: u16,
    /// -1 or 128.
    pub priority: i16,
    /// The parameter (index into `Event::params`) the sounds are laid out on.
    pub param: u16,
    pub sounds: Vec<SoundInstance>,
    pub envelopes: Vec<Envelope>,
}

/// A sound definition placed on a layer, over `start .. start + length` of the layer's
/// parameter (both normalised to 0..1).
#[derive(Clone, Debug, PartialEq)]
pub struct SoundInstance {
    /// Index into `EventFile::sound_defs`.
    pub sound_def: u16,
    pub start: f32,
    pub length: f32,
    pub start_mode: u32,
    /// `LOOP_*`.
    pub loop_mode: u32,
    /// -1: for ever.
    pub loop_count: i32,
    /// Automatic pitch: the pitch follows the parameter, 1 at `autopitch_reference`
    /// (normalised), `autopitch_at_zero` at 0.
    pub autopitch: u32,
    pub autopitch_reference: f32,
    pub autopitch_at_zero: f32,
    /// Assumed cents.
    pub fine_tune: f32,
    pub volume: f32,
    /// Crossfade widths in normalised parameter units, -1 when none.
    pub fade_in: f32,
    pub fade_out: f32,
    pub fade_in_shape: u32,
    pub fade_out_shape: u32,
}

pub const LOOP_AND_CUT_OFF: u32 = 0;
pub const LOOP_ONESHOT: u32 = 1;
pub const LOOP_AND_PLAY_TO_END: u32 = 2;

impl SoundInstance {
    pub fn loops(&self) -> bool {
        self.loop_mode != LOOP_ONESHOT
    }

    /// How much of it sounds at `x` (0..1 on the layer's parameter): 0 outside, ramps
    /// over the crossfades.
    pub fn coverage(&self, x: f32) -> f32 {
        let end = self.start + self.length;
        // a sound that reaches the end of the range covers the end itself
        if x < self.start || x > end || (x == end && end < 0.999) {
            return 0.0;
        }
        let mut g = 1.0f32;
        if self.fade_in > 0.0 {
            g = g.min((x - self.start) / self.fade_in);
        }
        if self.fade_out > 0.0 {
            g = g.min((end - x) / self.fade_out);
        }
        g.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Normalised parameter value.
    pub x: f32,
    pub y: f32,
    /// Curve to the next point (1 linear, 2 …); interpolation here is linear.
    pub shape: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// The layer's DSP unit it drives, -1 for built-in effects.
    pub dsp: i32,
    /// The DSP effect's name (`Channel Mix`, `FMOD Distortion`), empty otherwise.
    pub dsp_name: String,
    /// The DSP's parameter.
    pub dsp_param: u32,
    /// The effect (`ENVELOPE_VOLUME`, `ENVELOPE_PITCH`, …) in the high bits.
    pub kind: u32,
    /// Zero everywhere.
    pub unknown: u32,
    pub points: Vec<Point>,
    /// Index into `Event::params`.
    pub param: u32,
    /// 0 or 1; unexplained.
    pub flags: u32,
}

impl Envelope {
    pub fn effect(&self) -> u32 {
        self.kind & ENVELOPE_EFFECT_MASK
    }

    /// The curve at `x` (normalised), linear between the points, flat outside them.
    pub fn value_at(&self, x: f32) -> f32 {
        let Some(first) = self.points.first() else {
            return 1.0;
        };
        if x <= first.x {
            return first.y;
        }
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            if x <= b.x {
                if b.x <= a.x {
                    return b.y;
                }
                return a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x);
            }
        }
        self.points.last().map_or(1.0, |p| p.y)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    /// Units per second the parameter moves by itself (its "velocity").
    pub velocity: f32,
    pub min: f32,
    pub max: f32,
    /// 2, 3, 5, 8, 9 …; unexplained.
    pub flags: u32,
    pub unknown: [u32; 2],
    /// Key-off points.
    pub sustain_points: Vec<f32>,
}

impl Param {
    /// `v` in the parameter's units → 0..1.
    pub fn normalize(&self, v: f32) -> f32 {
        if self.max > self.min {
            ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// The settings sound definitions share (60 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct SoundDefTemplate {
    /// 0–4 (sequential, random, random without repeat, …: the order is not certain).
    pub play_mode: u32,
    /// With a spawn time the definition re-triggers a waveform every
    /// `spawn_min_ms..spawn_max_ms` (birds in an ambience) instead of looping one.
    pub spawn_min_ms: u32,
    pub spawn_max_ms: u32,
    pub max_spawned: u32,
    pub volume: f32,
    pub unknown_a: [u32; 3],
    /// The lowest random gain, 1 = no randomisation.
    pub volume_randomization: f32,
    /// Raw pitch units.
    pub pitch: f32,
    pub unknown_b: [u32; 3],
    /// ± raw pitch units.
    pub pitch_randomization: f32,
    /// Metres.
    pub position_randomization: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WaveKind {
    /// A sample in a bank: `index` into the `.fsb`.
    Sample {
        file: String,
        bank: String,
        index: u32,
        length_ms: u32,
    },
    Oscillator {
        shape: u32,
        frequency: f32,
    },
    /// "Don't play": a weighted gap.
    Silence,
    /// Chosen by the game at run time (dialogue).
    Programmer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    /// Relative chance (0..100).
    pub weight: u32,
    pub kind: WaveKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundDef {
    /// `/Single Shots/Pistol_Final_2_Mid`.
    pub name: String,
    /// Index into `EventFile::templates`.
    pub template: u32,
    pub waveforms: Vec<Waveform>,
}

impl SoundDef {
    /// The waveform a uniform `r` in 0..1 lands on, by weight. `None` when there is none
    /// or every weight is 0.
    pub fn pick(&self, r: f32) -> Option<usize> {
        let total: u64 = self.waveforms.iter().map(|w| w.weight as u64).sum();
        if total == 0 {
            return None;
        }
        let mut t = (r.clamp(0.0, 0.999_999) as f64 * total as f64) as u64;
        for (i, w) in self.waveforms.iter().enumerate() {
            if t < w.weight as u64 {
                return Some(i);
            }
            t -= w.weight as u64;
        }
        Some(self.waveforms.len() - 1)
    }
}

/// A reverb preset: the name, then 132 bytes (EAX-like room, decay, reflections …; the
/// first fields follow xoreos's layout).
#[derive(Clone, Debug, PartialEq)]
pub struct Reverb {
    pub name: String,
    pub room: i32,
    pub room_hf: i32,
    pub decay_time: f32,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventFile {
    pub version: u32,
    /// Two words that grow with the file (`0x534`, `0x654` in `parachute.fev`);
    /// unexplained, perhaps memory sizes.
    pub header: [u32; 2],
    pub project: String,
    pub banks: Vec<WaveBank>,
    pub category: Category,
    pub groups: Vec<EventGroup>,
    pub templates: Vec<SoundDefTemplate>,
    pub sound_defs: Vec<SoundDef>,
    pub reverbs: Vec<Reverb>,
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = self
            .d
            .get(self.p..self.p + n)
            .ok_or_else(|| Error::format(format!("fev: truncated at {:#x}", self.p)))?;
        self.p += n;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    /// A count that must fit in what is left of the file at `min` bytes per item.
    fn count(&mut self, min: usize) -> Result<usize> {
        let at = self.p;
        let n = self.u32()? as usize;
        if n.saturating_mul(min.max(1)) > self.d.len() - self.p {
            return Err(Error::format(format!("fev: bad count {n} at {at:#x}")));
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.count(1)?;
        let b = self.bytes(n)?;
        let b = b.split(|&c| c == 0).next().unwrap_or(&[]);
        Ok(b.iter().map(|&c| c as char).collect())
    }
}

fn category(r: &mut Rd, depth: u32) -> Result<Category> {
    if depth > 32 {
        return Err(Error::format("fev: categories too deep"));
    }
    let name = r.string()?;
    let volume = r.f32()?;
    let pitch = r.f32()?;
    let unknown = [r.u32()?, r.u32()?];
    let n = r.count(24)?;
    let children = (0..n)
        .map(|_| category(r, depth + 1))
        .collect::<Result<_>>()?;
    Ok(Category {
        name,
        volume,
        pitch,
        unknown,
        children,
    })
}

fn properties(r: &mut Rd) -> Result<Vec<Property>> {
    let n = r.count(12)?;
    (0..n)
        .map(|_| {
            let name = r.string()?;
            let value = match r.u32()? {
                0 => PropertyValue::Int(r.i32()?),
                1 => PropertyValue::Float(r.f32()?),
                2 => PropertyValue::String(r.string()?),
                t => return Err(Error::format(format!("fev: property type {t}"))),
            };
            Ok(Property { name, value })
        })
        .collect()
}

fn sound_instance(r: &mut Rd) -> Result<SoundInstance> {
    Ok(SoundInstance {
        sound_def: r.u16()?,
        start: r.f32()?,
        length: r.f32()?,
        start_mode: r.u32()?,
        loop_mode: r.u32()?,
        loop_count: r.i32()?,
        autopitch: r.u32()?,
        autopitch_reference: r.f32()?,
        autopitch_at_zero: r.f32()?,
        fine_tune: r.f32()?,
        volume: r.f32()?,
        fade_in: r.f32()?,
        fade_out: r.f32()?,
        fade_in_shape: r.u32()?,
        fade_out_shape: r.u32()?,
    })
}

fn envelope(r: &mut Rd) -> Result<Envelope> {
    let dsp = r.i32()?;
    let dsp_name = r.string()?;
    let dsp_param = r.u32()?;
    let kind = r.u32()?;
    let unknown = r.u32()?;
    let n = r.count(12)?;
    let points = (0..n)
        .map(|_| {
            Ok(Point {
                x: r.f32()?,
                y: r.f32()?,
                shape: r.u32()?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Envelope {
        dsp,
        dsp_name,
        dsp_param,
        kind,
        unknown,
        points,
        param: r.u32()?,
        flags: r.u32()?,
    })
}

fn param(r: &mut Rd) -> Result<Param> {
    let name = r.string()?;
    let velocity = r.f32()?;
    let min = r.f32()?;
    let max = r.f32()?;
    let flags = r.u32()?;
    let unknown = [r.u32()?, r.u32()?];
    let n = r.count(4)?;
    let sustain_points = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
    Ok(Param {
        name,
        velocity,
        min,
        max,
        flags,
        unknown,
        sustain_points,
    })
}

fn event(r: &mut Rd, group_path: &str) -> Result<Event> {
    let kind = r.u32()?;
    if kind != EVENT_COMPLEX && kind != EVENT_SIMPLE {
        return Err(Error::format(format!(
            "fev: event kind {kind:#x} at {:#x}",
            r.p - 4
        )));
    }
    let name = r.string()?;
    let mut guid = [0u8; 16];
    guid.copy_from_slice(r.bytes(16)?);
    let mut e = Event {
        path: format!("{group_path}/{name}"),
        name,
        kind,
        guid,
        volume: r.f32()?,
        pitch: r.f32()?,
        pitch_randomization: r.f32()?,
        volume_randomization: r.f32()?,
        priority: r.u32()?,
        max_playbacks: r.u32()?,
        steal_priority: r.u32()?,
        mode: r.u32()?,
        min_distance: r.f32()?,
        max_distance: r.f32()?,
        flags: r.u32()?,
        speaker_levels: [0.0; 8],
        cone_inside: 0.0,
        cone_outside: 0.0,
        cone_outside_volume: 0.0,
        max_playbacks_behavior: 0,
        doppler: 0.0,
        reverb_dry: 0.0,
        reverb_wet: 0.0,
        speaker_spread: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
        spawn_intensity: 0.0,
        spawn_intensity_randomization: 0.0,
        pan_level: 0.0,
        position_randomization: 0,
        layers: Vec::new(),
        params: Vec::new(),
        properties: Vec::new(),
        category: String::new(),
    };
    for s in &mut e.speaker_levels {
        *s = r.f32()?;
    }
    e.cone_inside = r.f32()?;
    e.cone_outside = r.f32()?;
    e.cone_outside_volume = r.f32()?;
    e.max_playbacks_behavior = r.u32()?;
    e.doppler = r.f32()?;
    e.reverb_dry = r.f32()?;
    e.reverb_wet = r.f32()?;
    e.speaker_spread = r.f32()?;
    e.fade_in_ms = r.u32()?;
    e.fade_out_ms = r.u32()?;
    e.spawn_intensity = r.f32()?;
    e.spawn_intensity_randomization = r.f32()?;
    e.pan_level = r.f32()?;
    e.position_randomization = r.u32()?;
    if kind == EVENT_SIMPLE {
        // one layer without a header, parameters or envelopes
        let n = r.count(58)?;
        let sounds = (0..n).map(|_| sound_instance(r)).collect::<Result<_>>()?;
        e.layers.push(Layer {
            flags: 0,
            priority: -1,
            param: 0,
            sounds,
            envelopes: Vec::new(),
        });
    } else {
        let n = r.count(10)?;
        for _ in 0..n {
            let flags = r.u16()?;
            let priority = r.u16()? as i16;
            let param = r.u16()?;
            let ns = r.u16()? as usize;
            let ne = r.u16()? as usize;
            let sounds = (0..ns).map(|_| sound_instance(r)).collect::<Result<_>>()?;
            let envelopes = (0..ne).map(|_| envelope(r)).collect::<Result<_>>()?;
            e.layers.push(Layer {
                flags,
                priority,
                param,
                sounds,
                envelopes,
            });
        }
        let n = r.count(32)?;
        e.params = (0..n).map(|_| param(r)).collect::<Result<_>>()?;
        e.properties = properties(r)?;
    }
    // the number of category names: 1 in every event
    let ncat = r.u32()?;
    if ncat != 1 {
        return Err(Error::format(format!(
            "fev: {} has {ncat} categories",
            e.path
        )));
    }
    e.category = r.string()?;
    Ok(e)
}

fn group(r: &mut Rd, parent: &str, depth: u32) -> Result<EventGroup> {
    if depth > 32 {
        return Err(Error::format("fev: groups too deep"));
    }
    let name = r.string()?;
    let path = if parent.is_empty() {
        name.clone()
    } else {
        format!("{parent}/{name}")
    };
    let properties = properties(r)?;
    let ngroups = r.count(16)?;
    let nevents = r.count(100)?;
    let groups = (0..ngroups)
        .map(|_| group(r, &path, depth + 1))
        .collect::<Result<_>>()?;
    let events = (0..nevents)
        .map(|_| event(r, &path))
        .collect::<Result<_>>()?;
    Ok(EventGroup {
        name,
        properties,
        groups,
        events,
    })
}

impl EventFile {
    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.get(0..4) != Some(MAGIC) {
            return Err(Error::format("not an FMOD event file (FEV1)"));
        }
        let mut r = Rd { d, p: 4 };
        let version = r.u32()?;
        if version != VERSION {
            return Err(Error::format(format!("fev: version {version:#x}")));
        }
        let header = [r.u32()?, r.u32()?];
        let project = r.string()?;
        let n = r.count(12)?;
        let banks = (0..n)
            .map(|_| {
                Ok(WaveBank {
                    load_mode: r.u32()?,
                    max_streams: r.u32()?,
                    name: r.string()?,
                })
            })
            .collect::<Result<_>>()?;
        let category = category(&mut r, 0)?;
        let n = r.count(16)?;
        let groups = (0..n)
            .map(|_| group(&mut r, "", 0))
            .collect::<Result<_>>()?;
        let n = r.count(60)?;
        let templates = (0..n)
            .map(|_| {
                Ok(SoundDefTemplate {
                    play_mode: r.u32()?,
                    spawn_min_ms: r.u32()?,
                    spawn_max_ms: r.u32()?,
                    max_spawned: r.u32()?,
                    volume: r.f32()?,
                    unknown_a: [r.u32()?, r.u32()?, r.u32()?],
                    volume_randomization: r.f32()?,
                    pitch: r.f32()?,
                    unknown_b: [r.u32()?, r.u32()?, r.u32()?],
                    pitch_randomization: r.f32()?,
                    position_randomization: r.f32()?,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count(12)?;
        let sound_defs = (0..n)
            .map(|_| {
                let name = r.string()?;
                let template = r.u32()?;
                let n = r.count(8)?;
                let waveforms = (0..n)
                    .map(|_| {
                        let kind = r.u32()?;
                        let weight = r.u32()?;
                        let kind = match kind {
                            0 => WaveKind::Sample {
                                file: r.string()?,
                                bank: r.string()?,
                                index: r.u32()?,
                                length_ms: r.u32()?,
                            },
                            1 => WaveKind::Oscillator {
                                shape: r.u32()?,
                                frequency: r.f32()?,
                            },
                            2 => WaveKind::Silence,
                            3 => WaveKind::Programmer,
                            k => {
                                return Err(Error::format(format!(
                                    "fev: waveform kind {k} in {name}"
                                )));
                            }
                        };
                        Ok(Waveform { weight, kind })
                    })
                    .collect::<Result<_>>()?;
                Ok(SoundDef {
                    name,
                    template,
                    waveforms,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count(136)?;
        let reverbs = (0..n)
            .map(|_| {
                let name = r.string()?;
                let raw = r.bytes(132)?.to_vec();
                let w = |i: usize| u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
                Ok(Reverb {
                    name,
                    room: w(0) as i32,
                    room_hf: w(4) as i32,
                    decay_time: f32::from_bits(w(12)),
                    raw,
                })
            })
            .collect::<Result<_>>()?;
        // a closing chunk header: size 8, `comp`, nothing in it
        if r.bytes(8)? != b"\x08\0\0\0comp" || r.p != d.len() {
            return Err(Error::format(format!(
                "fev: {} bytes past the reverbs, expected the `comp` trailer",
                d.len() - (r.p - 8).min(d.len())
            )));
        }
        let file = Self {
            version,
            header,
            project,
            banks,
            category,
            groups,
            templates,
            sound_defs,
            reverbs,
        };
        file.check()?;
        Ok(file)
    }

    /// Every index points somewhere.
    fn check(&self) -> Result<()> {
        for e in self.events() {
            for l in &e.layers {
                // (empty placeholder events have a layer but no parameters)
                if !e.params.is_empty() && l.param as usize >= e.params.len() {
                    return Err(Error::format(format!("fev: {}: layer parameter", e.path)));
                }
                for s in &l.sounds {
                    if s.sound_def as usize >= self.sound_defs.len() {
                        return Err(Error::format(format!("fev: {}: sound definition", e.path)));
                    }
                }
                for v in &l.envelopes {
                    if v.param as usize >= e.params.len() {
                        return Err(Error::format(format!(
                            "fev: {}: envelope parameter",
                            e.path
                        )));
                    }
                }
            }
        }
        for s in &self.sound_defs {
            if s.template as usize >= self.templates.len() {
                return Err(Error::format(format!("fev: {}: template", s.name)));
            }
        }
        Ok(())
    }

    /// Every event, depth first, in file order.
    pub fn events(&self) -> Vec<&Event> {
        fn walk<'a>(g: &'a EventGroup, out: &mut Vec<&'a Event>) {
            for s in &g.groups {
                walk(s, out);
            }
            out.extend(g.events.iter());
        }
        let mut out = Vec::new();
        for g in &self.groups {
            walk(g, &mut out);
        }
        out
    }

    /// The event number `index` of top-level group `group`, in file order. This is how
    /// the game's own ids name an event (`crate::soundmap`): the group by its position in
    /// the project, the event by its position in the group. No retail group has
    /// subgroups, so the group's own events are all there is.
    pub fn event_in_group(&self, group: usize, index: usize) -> Option<&Event> {
        self.groups.get(group)?.events.get(index)
    }

    /// The event at `path` (`Single/Pistol_Shot`, or with the project in front:
    /// `Weapons/Single/Pistol_Shot`), ignoring case.
    pub fn event(&self, path: &str) -> Option<&Event> {
        let path = path.trim_matches('/');
        let rest = path
            .split_once('/')
            .filter(|(p, _)| p.eq_ignore_ascii_case(&self.project))
            .map(|(_, r)| r);
        self.events().into_iter().find(|e| {
            e.path.eq_ignore_ascii_case(path)
                || rest.is_some_and(|r| e.path.eq_ignore_ascii_case(r))
        })
    }

    pub fn template_of(&self, sd: &SoundDef) -> &SoundDefTemplate {
        &self.templates[sd.template as usize]
    }

    /// The product of the volumes from `master` down to `path` (`sfx/weapons/shots`).
    pub fn category_volume(&self, path: &str) -> f32 {
        let mut v = self.category.volume;
        let mut c = &self.category;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            match c
                .children
                .iter()
                .find(|k| k.name.eq_ignore_ascii_case(part))
            {
                Some(k) => {
                    v *= k.volume;
                    c = k;
                }
                None => break,
            }
        }
        v
    }

    /// The sound definitions `e` can play, each once.
    pub fn sound_defs_of(&self, e: &Event) -> Vec<usize> {
        let mut out: Vec<usize> = e
            .layers
            .iter()
            .flat_map(|l| l.sounds.iter().map(|s| s.sound_def as usize))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The `.fev` files of the retail game, in `sound\fmod\` (`dialog_*` per language).
pub const RETAIL_FILES: [&str; 17] = [
    "weapons.fev",
    "vehicles.fev",
    "zones.fev",
    "effects.fev",
    "character.fev",
    "parachute.fev",
    "gui.fev",
    "structures.fev",
    "music.fev",
    "cutscenes.fev",
    "crowds.fev",
    "dialog_eng.fev",
    "dialog_fre.fev",
    "dialog_ger.fev",
    "dialog_ita.fev",
    "dialog_rus.fev",
    "dialog_spa.fev",
];

/// The sample of `bank` a waveform plays. The index is checked against the sample's
/// name (the `.fsb` keeps 30 characters of it): a few of the retail events name
/// samples the bank was rebuilt without, and their index points at something else.
/// Falls back to a search by name; `None` when the bank has no such sample.
pub fn resolve_sample(file: &str, index: u32, bank: &crate::fsb::SoundBank) -> Option<usize> {
    let base = file
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file)
        .to_ascii_lowercase();
    let same = |name: &str| {
        let n = name.to_ascii_lowercase();
        n == base || (n.len() >= 29 && base.starts_with(&n))
    };
    match bank.samples.get(index as usize) {
        Some(s) if same(&s.name) => Some(index as usize),
        _ => bank.samples.iter().position(|s| same(&s.name)),
    }
}

/// The DSP a layer applies at some parameter values: see [`Event::layer_dsp`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerDsp {
    pub pan_level: f32,
    pub occlusion: f32,
}

/// One sound an event start makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub layer: usize,
    pub sound: usize,
    /// Index into `EventFile::sound_defs`.
    pub sound_def: usize,
    /// The waveform picked; `None` when the definition has no waveform with weight.
    pub waveform: Option<usize>,
    /// Linear gain: event × category × layer envelopes × sound × template × crossfade ×
    /// the random part.
    pub gain: f32,
    /// Playback speed (1 = as recorded): event, template and envelope pitch, fine tune
    /// and automatic pitch.
    pub speed: f32,
    pub looping: bool,
    /// Re-trigger every `min..max` ms instead of looping (ambient sprinkles).
    pub spawn_ms: Option<(u32, u32)>,
}

impl Event {
    pub fn is_3d(&self) -> bool {
        self.mode & MODE_3D != 0
    }

    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Gain and pitch (octaves) the envelopes of layer `l` give at these parameter
    /// values (`values[i]` in parameter `i`'s units). Volume envelopes multiply,
    /// pitch envelopes add `(y − 0.5) × 2` octaves (assumed range); the other effects
    /// are not applied.
    pub fn layer_mix(&self, l: usize, values: &[f32]) -> (f32, f32) {
        let mut gain = 1.0;
        let mut octaves = 0.0;
        for v in &self.layers[l].envelopes {
            if v.dsp >= 0 {
                continue;
            }
            let Some(p) = self.params.get(v.param as usize) else {
                continue;
            };
            let x = p.normalize(values.get(v.param as usize).copied().unwrap_or(p.min));
            match v.effect() {
                ENVELOPE_VOLUME => gain *= v.value_at(x).max(0.0),
                ENVELOPE_PITCH => octaves += (v.value_at(x) - 0.5) * 2.0,
                _ => {}
            }
        }
        (gain, octaves)
    }

    /// The pan level (1 = the position pans fully, 0 = centred; the product of the
    /// layer's [`ENVELOPE_PAN_LEVEL`] envelopes) and the occlusion (0 = clear, 1 = fully
    /// muffled; the strongest of its [`ENVELOPE_OCCLUSION`] envelopes) of layer `l` at
    /// these parameter values.
    pub fn layer_dsp(&self, l: usize, values: &[f32]) -> LayerDsp {
        let mut out = LayerDsp {
            pan_level: 1.0,
            occlusion: 0.0,
        };
        for v in &self.layers[l].envelopes {
            if v.dsp >= 0 {
                continue;
            }
            let Some(p) = self.params.get(v.param as usize) else {
                continue;
            };
            let x = p.normalize(values.get(v.param as usize).copied().unwrap_or(p.min));
            match v.effect() {
                ENVELOPE_PAN_LEVEL => out.pan_level *= v.value_at(x).clamp(0.0, 1.0),
                ENVELOPE_OCCLUSION => {
                    out.occlusion = out.occlusion.max(v.value_at(x).clamp(0.0, 1.0))
                }
                _ => {}
            }
        }
        out
    }

    /// What one start of the event plays at these parameter values (`values[i]` in
    /// parameter `i`'s units; missing ones sit at their minimum). `rand` gives uniform
    /// numbers in 0..1 for the waveform picks and the randomisation.
    pub fn voices(
        &self,
        file: &EventFile,
        values: &[f32],
        rand: &mut dyn FnMut() -> f32,
    ) -> Vec<Voice> {
        let mut out = Vec::new();
        for (li, layer) in self.layers.iter().enumerate() {
            for (si, s) in layer.sounds.iter().enumerate() {
                let (gain, speed) = self.sound_level(file, li, si, values);
                if gain <= 0.0001 {
                    continue;
                }
                let (rgain, rspeed) = self.randomize(file, s.sound_def as usize, rand);
                let sd = &file.sound_defs[s.sound_def as usize];
                let t = file.template_of(sd);
                let spawn = (t.spawn_max_ms > 0).then_some((t.spawn_min_ms, t.spawn_max_ms));
                out.push(Voice {
                    layer: li,
                    sound: si,
                    sound_def: s.sound_def as usize,
                    waveform: sd.pick(rand()),
                    gain: gain * rgain,
                    speed: (speed * rspeed).clamp(0.05, 8.0),
                    looping: s.loops() && spawn.is_none(),
                    spawn_ms: spawn,
                });
            }
        }
        out
    }

    /// Sound `si` of layer `li` at these parameter values, without the random part:
    /// the gain (event × category × the layer's volume envelopes × sound × template ×
    /// crossfade) and the speed (event and template pitch, pitch envelopes, fine tune,
    /// automatic pitch). A running instance re-evaluates this as its parameters move.
    pub fn sound_level(
        &self,
        file: &EventFile,
        li: usize,
        si: usize,
        values: &[f32],
    ) -> (f32, f32) {
        let layer = &self.layers[li];
        let s = &layer.sounds[si];
        let complex = self.kind == EVENT_COMPLEX;
        let (lgain, loct) = if complex {
            self.layer_mix(li, values)
        } else {
            (1.0, 0.0)
        };
        let x = match self.params.get(layer.param as usize) {
            Some(p) if complex => {
                p.normalize(values.get(layer.param as usize).copied().unwrap_or(p.min))
            }
            _ => 0.0,
        };
        let cover = if complex { s.coverage(x) } else { 1.0 };
        let sd = &file.sound_defs[s.sound_def as usize];
        let t = file.template_of(sd);
        let gain = self.volume
            * file.category_volume(&self.category)
            * lgain
            * s.volume
            * t.volume
            * cover;
        let mut speed = 2f32
            .powf((self.pitch + t.pitch) * OCTAVES_PER_PITCH_UNIT + loct + s.fine_tune / 1200.0);
        if s.autopitch != 0 && s.autopitch_reference > 0.0 {
            let k = x / s.autopitch_reference;
            speed *= (s.autopitch_at_zero + (1.0 - s.autopitch_at_zero) * k).max(0.05);
        }
        (gain.max(0.0), speed.clamp(0.05, 8.0))
    }

    /// The random part of one start of sound definition `sd`: a gain (the template's
    /// volume randomisation: uniform between it and 1) and a speed factor (± the event's
    /// and the template's pitch randomisation).
    pub fn randomize(
        &self,
        file: &EventFile,
        sd: usize,
        rand: &mut dyn FnMut() -> f32,
    ) -> (f32, f32) {
        let t = file.template_of(&file.sound_defs[sd]);
        let vr = t.volume_randomization.clamp(0.0, 1.0);
        let gain = if vr < 1.0 {
            vr + (1.0 - vr) * rand()
        } else {
            1.0
        };
        let units = self.pitch_randomization * (2.0 * rand() - 1.0)
            + t.pitch_randomization * (2.0 * rand() - 1.0);
        (gain, 2f32.powf(units * OCTAVES_PER_PITCH_UNIT))
    }

    /// The gain of the `(distance)` / `distance` volume envelopes at `metres`, or the
    /// FMOD rolloff the mode names (linear or inverse between min and max distance).
    /// 0 past the end of the range.
    pub fn rolloff(&self, metres: f32) -> f32 {
        if !self.is_3d() {
            return 1.0;
        }
        if self.mode & MODE_3D_CUSTOMROLLOFF != 0 {
            if metres > self.max_distance.max(1.0) {
                return 0.0;
            }
            // the custom curve is drawn on the distance parameter; take the loudest
            // layer's (the layers share it in practice)
            let Some(pi) = self.params.iter().position(|p| {
                matches!(
                    p.name.as_str(),
                    "(distance)" | "distance" | "average distance"
                )
            }) else {
                return 1.0;
            };
            let x = self.params[pi].normalize(metres);
            let mut best: Option<f32> = None;
            for l in &self.layers {
                let mut g: Option<f32> = None;
                for v in &l.envelopes {
                    if v.dsp < 0 && v.effect() == ENVELOPE_VOLUME && v.param as usize == pi {
                        g = Some(g.unwrap_or(1.0) * v.value_at(x));
                    }
                }
                if let Some(g) = g {
                    best = Some(best.map_or(g, |b: f32| b.max(g)));
                }
            }
            return best.unwrap_or(1.0 - x).clamp(0.0, 1.0);
        }
        let (min, max) = (self.min_distance.max(0.01), self.max_distance.max(0.02));
        if metres <= min {
            1.0
        } else if metres >= max {
            0.0
        } else if self.mode & MODE_3D_LINEARROLLOFF != 0 {
            (max - metres) / (max - min)
        } else {
            min / metres
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fs() -> Option<crate::GameFs> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = crate::GameFs::open(dir).ok()?;
        fs.contains("weapons.fev").then_some(fs)
    }

    #[test]
    fn envelope_and_coverage() {
        let e = Envelope {
            dsp: -1,
            dsp_name: String::new(),
            dsp_param: 0,
            kind: 0xc,
            unknown: 0,
            points: vec![
                Point {
                    x: 0.0,
                    y: 1.0,
                    shape: 1,
                },
                Point {
                    x: 0.5,
                    y: 0.0,
                    shape: 2,
                },
            ],
            param: 0,
            flags: 0,
        };
        assert_eq!(e.effect(), ENVELOPE_VOLUME);
        assert_eq!(e.value_at(-1.0), 1.0);
        assert!((e.value_at(0.25) - 0.5).abs() < 1e-6);
        assert_eq!(e.value_at(0.9), 0.0);
        let s = SoundInstance {
            sound_def: 0,
            start: 0.2,
            length: 0.5,
            start_mode: 0,
            loop_mode: 1,
            loop_count: -1,
            autopitch: 0,
            autopitch_reference: 0.0,
            autopitch_at_zero: 0.0,
            fine_tune: 0.0,
            volume: 1.0,
            fade_in: 0.1,
            fade_out: -1.0,
            fade_in_shape: 1,
            fade_out_shape: 1,
        };
        assert_eq!(s.coverage(0.1), 0.0);
        assert!((s.coverage(0.25) - 0.5).abs() < 1e-5);
        assert_eq!(s.coverage(0.5), 1.0);
        assert_eq!(s.coverage(0.8), 0.0);
        assert!(!s.loops());
    }

    #[test]
    fn weighted_pick() {
        let w = |weight| Waveform {
            weight,
            kind: WaveKind::Silence,
        };
        let sd = SoundDef {
            name: "x".into(),
            template: 0,
            waveforms: vec![w(6), w(0), w(100), w(94)],
        };
        assert_eq!(sd.pick(0.0), Some(0));
        assert_eq!(sd.pick(0.0299), Some(0));
        assert_eq!(sd.pick(0.031), Some(2));
        assert_eq!(sd.pick(0.52), Some(2));
        assert_eq!(sd.pick(0.54), Some(3));
        assert_eq!(sd.pick(1.0), Some(3));
        let empty = SoundDef {
            name: "y".into(),
            template: 0,
            waveforms: vec![w(0)],
        };
        assert_eq!(empty.pick(0.5), None);
    }

    /// Every `.fev` in the game parses to its last byte (`parse` fails on anything left
    /// over or any index out of range). Needs JC2_TEST_DIR.
    #[test]
    fn every_event_file() {
        let Some(fs) = test_fs() else { return };
        let mut events = 0;
        let mut sound_defs = 0;
        for name in RETAIL_FILES {
            let d = fs.read(name).unwrap();
            let f = EventFile::parse(&d).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!f.banks.is_empty(), "{name}");
            assert_eq!(f.category.name, "master");
            events += f.events().len();
            sound_defs += f.sound_defs.len();
            for e in f.events() {
                assert!(e.kind == EVENT_COMPLEX || e.kind == EVENT_SIMPLE);
                assert!(e.mode & (MODE_2D | MODE_3D) != 0, "{}", e.path);
                assert!(f.event(&e.path).is_some(), "{}", e.path);
            }
            for sd in &f.sound_defs {
                for w in &sd.waveforms {
                    if let WaveKind::Sample { bank, .. } = &w.kind {
                        assert!(
                            f.banks.iter().any(|b| b.name.eq_ignore_ascii_case(bank)),
                            "{name}: {} plays from {bank}",
                            sd.name
                        );
                    }
                }
            }
        }
        assert_eq!(events, 1699);
        assert!(sound_defs > 1500);
    }

    /// The pistol's shot resolves to its samples in `weapons_common.fsb`, by index and
    /// by name.
    #[test]
    fn pistol_shot() {
        let Some(fs) = test_fs() else { return };
        let f = EventFile::parse(&fs.read("weapons.fev").unwrap()).unwrap();
        assert_eq!(f.project, "Weapons");
        assert_eq!(f.banks[0].file_name(), "weapons_common.fsb");
        let e = f.event("Weapons/Single/Pistol_Shot").unwrap();
        assert!(f.event("single/pistol_shot").is_some());
        assert!(e.is_3d() && e.mode & MODE_3D_CUSTOMROLLOFF != 0);
        assert_eq!(e.max_distance, 300.0);
        assert_eq!(e.category, "sfx/weapons/shots");
        assert_eq!(e.params[0].name, "distance");
        let bank =
            crate::fsb::SoundBank::parse(&fs.read(&f.banks[0].file_name()).unwrap()).unwrap();
        let mut names: Vec<&str> = Vec::new();
        for sd in f.sound_defs_of(e) {
            for w in &f.sound_defs[sd].waveforms {
                let WaveKind::Sample { file, index, .. } = &w.kind else {
                    panic!("not a sample");
                };
                let s = &bank.samples[*index as usize];
                let base = file.rsplit('/').next().unwrap();
                assert!(base.starts_with(s.name.trim_end_matches(".wav")), "{base}");
                names.push(&s.name);
            }
        }
        names.sort();
        assert_eq!(
            names,
            ["Pistol_New_Full_2_HiLo.wav", "Pistol_New_Full_2_Mid.wav"]
        );
        // close: both layers; far off only the distant one
        let mut n = 0.5f32;
        let mut rand = || {
            n = (n * 9.73).fract();
            n
        };
        let near = e.voices(&f, &[5.0, 0.5, 1.0, 0.0], &mut rand);
        assert_eq!(near.len(), 2);
        assert!(near.iter().all(|v| !v.looping && v.gain > 0.0));
        let far = e.voices(&f, &[280.0, 0.5, 1.0, 0.0], &mut rand);
        assert_eq!(far.len(), 1);
        assert!(e.rolloff(10.0) > e.rolloff(200.0));
        assert_eq!(e.rolloff(400.0), 0.0);
    }

    /// The waveforms of the sound-effect projects resolve to samples of their banks,
    /// checked by name; the few that do not are the ones the banks were rebuilt
    /// without.
    #[test]
    fn waveforms_resolve() {
        let Some(fs) = test_fs() else { return };
        let mut banks: std::collections::HashMap<String, crate::fsb::SoundBank> =
            Default::default();
        let (mut ok, mut bad) = (0, Vec::new());
        for name in &RETAIL_FILES[..11] {
            let f = EventFile::parse(&fs.read(name).unwrap()).unwrap();
            for sd in &f.sound_defs {
                for w in &sd.waveforms {
                    let WaveKind::Sample {
                        file, bank, index, ..
                    } = &w.kind
                    else {
                        continue;
                    };
                    let key = format!("{}.fsb", bank.to_ascii_lowercase());
                    if !banks.contains_key(&key) {
                        let Ok(d) = fs.read(&key) else { continue };
                        banks.insert(key.clone(), crate::fsb::SoundBank::parse(&d).unwrap());
                    }
                    match resolve_sample(file, *index, &banks[&key]) {
                        Some(_) => ok += 1,
                        None => bad.push(format!("{name}: {} → {file}", sd.name)),
                    }
                }
            }
        }
        eprintln!("{ok} waveforms resolve, {} do not: {bad:#?}", bad.len());
        // 1822 and 31 (stale GUI sounds, the flak, a siren, a tractor layer…)
        assert!(ok > 1800);
        assert!(bad.len() * 50 < ok, "{bad:#?}");
    }

    /// Picks follow the weights: the jungle crickets' definition (silences weighted 6
    /// and 1 next to three crickets at 100) over many starts. And every pick of every
    /// sound definition of the SFX projects lands on a waveform that has weight.
    #[test]
    fn picks_follow_weights() {
        let Some(fs) = test_fs() else { return };
        let f = EventFile::parse(&fs.read("zones.fev").unwrap()).unwrap();
        let sd = f
            .sound_defs
            .iter()
            .find(|s| s.name == "/Insects/Insect_single_ex_night_02")
            .unwrap();
        let total: u32 = sd.waveforms.iter().map(|w| w.weight).sum();
        let mut counts = vec![0u32; sd.waveforms.len()];
        let mut seed = 0x1234_5678u32;
        let n = 200_000;
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let r = (seed >> 8) as f32 / (1u32 << 24) as f32;
            counts[sd.pick(r).unwrap()] += 1;
        }
        for (w, c) in sd.waveforms.iter().zip(&counts) {
            let want = n as f64 * w.weight as f64 / total as f64;
            let sigma = want.sqrt().max(1.0);
            assert!(
                (*c as f64 - want).abs() < 5.0 * sigma,
                "weight {} picked {c} times, expected {want:.0}",
                w.weight
            );
        }
        for name in &RETAIL_FILES[..11] {
            let f = EventFile::parse(&fs.read(name).unwrap()).unwrap();
            for sd in &f.sound_defs {
                for k in 0..50 {
                    if let Some(i) = sd.pick(k as f32 / 50.0) {
                        assert!(sd.waveforms[i].weight > 0, "{}", sd.name);
                    }
                }
            }
        }
    }

    /// The parachute's events: the canopy opening is a one-shot, the flapping and the
    /// wind loop.
    #[test]
    fn parachute_loops() {
        let Some(fs) = test_fs() else { return };
        let f = EventFile::parse(&fs.read("parachute.fev").unwrap()).unwrap();
        assert_eq!(f.events().len(), 18);
        let mut r = || 0.5;
        let open = f.event("Parachute/Parachute_Horiz").unwrap();
        let v = open.voices(&f, &[0.0, 1.0], &mut r);
        assert!(!v.is_empty() && v.iter().all(|v| !v.looping));
        assert_eq!(f.sound_defs[v[0].sound_def].name, "/Para_Flap_Open");
        let wind = f.event("Parachute/Wind_skydive").unwrap();
        assert!(!wind.is_3d());
        let mut vals: Vec<f32> = wind.params.iter().map(|p| p.max).collect();
        vals[0] = 20.0;
        assert!(wind.voices(&f, &vals, &mut r).iter().any(|v| v.looping));
    }
}
