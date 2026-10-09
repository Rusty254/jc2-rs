//! Sound: the game's FMOD Designer events (`sound\fmod\*.fev`, `jc2_formats::fev`)
//! played from its sample banks (`*.fsb`, `jc2_formats::fsb`) through Bevy's audio.
//!
//! **Which event.** The game's data names sounds by two numbers, a bank and a sound
//! (`jc2_formats::soundmap`): a weapon's `sound_fire_id` is a sound set whose layers (shot,
//! tail, shell, reload) are such pairs; an effect file holds one per ground material; the
//! character module has one per ground material for each footstep; a vehicle module one
//! per engine preset; the environment module one per zone set. [`Sounds`] turns a pair into
//! an event key (`weapons.fev#1.5`) and loads the event, its sound definitions and its
//! samples the first time it is asked for. What the data has no id for stays on the by-name
//! table (`EVENTS`, `FALLBACK`): the canopy's flap and the wind of a fall, and every sound
//! whose id does not resolve; the first use of each is logged.
//!
//! **How it plays.** A start of an event is an [`Instance`] that runs the event the way FMOD
//! Event would, in small: its parameters (`rpm`, `load`, `speed`, `(distance)`, `(listener
//! angle)`, `timeofday`, the timelines that move by themselves), its layers' volume and
//! pitch envelopes, the sound instances that sound where the layer's parameter is
//! (crossfades, automatic pitch), the waveform picked by weight, volume and pitch
//! randomisation, loops, re-triggered "spawning" definitions, the 3D rolloff, the event's
//! fade in and fade out, its **max playbacks** (an instance over the limit steals the oldest,
//! the newest or the quietest, or fails, as the event says) and the DSP the envelopes name:
//! the 3D pan level (a gun at Rico's feet stays centred) and the occlusion (a low-pass; the
//! sounds behind the listener are dulled). The DSP runs in `audio_dsp`. One-shots (shots,
//! blasts, steps) end by themselves; held instances (an automatic gun, the engine, the canopy,
//! the wind, the ambience) follow what the game sets each frame.

mod vehicle_fx;

use crate::Game;
use crate::audio_dsp::{Shared, Voice, occlusion_cutoff};
use crate::boat::Watercraft;
use crate::character::Hero;
use crate::combat::Arsenal;
use crate::destruction::StructureHits;
use crate::explosion::{Blasts, Projectile};
use crate::heli::Heli;
use crate::plane::Plane;
use crate::vehicle::{CameraMode, Occupied, Vehicle};
use crate::world::FlyCamera;
use avian3d::prelude::LinearVelocity;
use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings};
use bevy::prelude::*;
use jc2_formats::fev::{self, EventFile, MODE_3D_CUSTOMROLLOFF, WaveKind};
use jc2_formats::fsb::SoundBank;
use jc2_formats::soundmap::{
    EffectSound, SoundMap, SoundRef, VehicleSelector, VehicleSoundSelection,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// An event as the game asks for it: a name from `EVENTS` or an id key (`weapons.fev#1.5`).
pub type Key = &'static str;

/// (key, project, event path): the by-name table. Only the sounds the data has no id for
/// and the sounds whose id does not resolve play from here (see `Sounds::key`).
const EVENTS: &[(&str, &str, &str)] = &[
    ("pistol", "weapons.fev", "Single/Pistol_Shot"),
    ("smg", "weapons.fev", "Auto/SMG_HighRate"),
    ("shotgun", "weapons.fev", "Single/Shotgun_Shot"),
    ("rifle", "weapons.fev", "Auto/AR_MidRate"),
    ("sniper", "weapons.fev", "Single/Sniper"),
    ("grenade_launcher", "weapons.fev", "Single/Grenade Launcher"),
    ("rocket_fire", "weapons.fev", "Single/Rocket Launcher Shot"),
    ("rocket_loop", "weapons.fev", "Single/Rocket Launcher Loop"),
    (
        "enemy_rifle",
        "weapons.fev",
        "Acoustics/EarlyReflection_AR_Ridge",
    ),
    (
        "explosion_small",
        "effects.fev",
        "Explosions/13 - Grenade Explo",
    ),
    (
        "explosion_medium",
        "effects.fev",
        "Explosions/11 - Explosion Medium",
    ),
    ("explosion_big", "effects.fev", "Explos/Barrel Explode"),
    ("debris", "effects.fev", "PostEffects/Debris_Metal_Large"),
    ("step", "character.fev", "Foley/Footsteps_Dirt"),
    (
        "grapple_fire",
        "parachute.fev",
        "grappling_hook/grappling_fire_wire",
    ),
    (
        "grapple_detach",
        "parachute.fev",
        "grappling_hook/grappling_detatch",
    ),
    ("chute_open", "parachute.fev", "Parachute/Parachute_Vert"),
    ("chute_loop", "parachute.fev", "Parachute/Parachute_steer"),
    ("wind", "parachute.fev", "Parachute/Wind_skydive"),
    ("car_jeep", "vehicles.fev", "Engines/Jeep"),
    ("car_sedan", "vehicles.fev", "Engines/Small Sedan"),
    ("car_truck", "vehicles.fev", "Engines/BigTruck Engine"),
    ("car_bus", "vehicles.fev", "Engines/Bus"),
    ("car_sport", "vehicles.fev", "Engines/Corvette"),
    ("bike", "vehicles.fev", "Engines/Dirtbike"),
    ("heli", "vehicles.fev", "Engines/Jet helicopter"),
    (
        "heli_transport",
        "vehicles.fev",
        "Engines/Transport Helicopter",
    ),
    ("boat", "vehicles.fev", "Engines/Speedboat"),
    ("prop", "vehicles.fev", "Engines/Cessna"),
    ("jet", "vehicles.fev", "Engines/Jet_JAS"),
    ("amb_jungle", "zones.fev", "Zones/Jungle_Amb"),
    ("amb_arctic", "zones.fev", "Zones/Arctic_Amb"),
    ("amb_desert", "zones.fev", "Zones/Desert_Amb"),
    ("amb_city", "zones.fev", "Zones/City_Amb"),
    ("spr_jungle", "zones.fev", "Zones/Jungle_Sprinkle"),
    ("spr_arctic", "zones.fev", "Zones/Arctic_Sprinkle"),
    ("spr_desert", "zones.fev", "Zones/Desert_Sprinkle"),
    ("spr_city", "zones.fev", "Zones/City_Sprinkle"),
];

/// When a key's event does not resolve: (key, bank, sample name — or the start of one
/// for a set of variations), as picked by name before the events were read.
const FALLBACK: &[(&str, &str, &str)] = &[
    ("pistol", "weapons_common.fsb", "Pistol_New_Full_2_HiLo.wav"),
    ("smg", "weapons_common.fsb", "SMG_1200_HiLo_L.wav"),
    ("shotgun", "weapons_common.fsb", "Shotgun_HiLo_02.wav"),
    ("rifle", "weapons_common.fsb", "AR_625_HiLo_L.wav"),
    ("sniper", "weapons_common.fsb", "Pistol_New_Full_2_HiLo.wav"),
    (
        "grenade_launcher",
        "weapons_common.fsb",
        "Shotgun_HiLo_02.wav",
    ),
    (
        "rocket_fire",
        "weapons_common.fsb",
        "RocketLauncher_Shot_New_6_HiLo",
    ),
    (
        "rocket_loop",
        "weapons_common.fsb",
        "Missile_loop_01_stereo.wav",
    ),
    (
        "enemy_rifle",
        "weapons_common.fsb",
        "Assault_Rifle_2_ER_Ridge_Mid",
    ),
    (
        "explosion_small",
        "effects_common.fsb",
        "Explosion_Small_01_Hi.wav",
    ),
    (
        "explosion_medium",
        "effects_common.fsb",
        "Explosion_Medium_02_Hi.wav",
    ),
    (
        "explosion_big",
        "effects_common.fsb",
        "2009_Explo_Large_Firey_Long_02",
    ),
    ("debris", "effects_common.fsb", "Debris_Large_Metal_"),
    ("step", "character_foley.fsb", "Footstep_Dirt_Run_"),
    ("grapple_fire", "parachute.fsb", "grappling_fire_02.wav"),
    (
        "grapple_detach",
        "parachute.fsb",
        "grappling_detatch_01.wav",
    ),
    ("chute_open", "parachute.fsb", "Para_Flap_Open_05.wav"),
    (
        "chute_loop",
        "parachute.fsb",
        "Para_FlappingSlow_exp 001.wav",
    ),
    ("wind", "parachute.fsb", "Steady wind 10.wav"),
    ("car_jeep", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("car_sedan", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("car_truck", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("car_bus", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("car_sport", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("bike", "vehicles_common.fsb", "jc7_onlow_eng1.wav"),
    ("heli", "vehicles_common.fsb", "Rotor_03.wav"),
    ("heli_transport", "vehicles_common.fsb", "Rotor_03.wav"),
    ("boat", "vehicles_common.fsb", "speedboateng_onlow.wav"),
    ("prop", "vehicles_common.fsb", "Cessna_onhighprop.wav"),
    ("jet", "vehicles_common.fsb", "jet_afterburn_loop.wav"),
];

/// A loop of an instance starts once its gain passes this and stops below it.
const AUDIBLE: f32 = 0.002;
/// The ambience bed under everything (the events are already quiet; this keeps it a
/// bed).
const AMBIENCE_GAIN: f32 = 0.8;
/// Most sample players sounding at once (the original's software mixer has 64 channels).
pub const MAX_VOICES: usize = 48;
/// A pan never goes all the way to one ear.
const MAX_PAN: f32 = 0.85;

// ───────────────────────────────── keys ─────────────────────────────────

fn interner() -> &'static Mutex<HashMap<String, Key>> {
    static I: OnceLock<Mutex<HashMap<String, Key>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// A string kept for the life of the program (the set of events is a few hundred).
fn intern(s: &str) -> Key {
    let mut m = interner().lock().unwrap();
    if let Some(k) = m.get(s) {
        return k;
    }
    let k: Key = Box::leak(s.to_string().into_boxed_str());
    m.insert(s.to_string(), k);
    k
}

/// The key of event `index` of group `group` of a project (`weapons.fev#1.5`).
pub fn ev_key(file: &str, group: usize, index: usize) -> Key {
    intern(&format!("{file}#{group}.{index}"))
}

fn parse_ev_key(key: &str) -> Option<(&str, usize, usize)> {
    let (f, r) = key.split_once('#')?;
    let (g, i) = r.split_once('.')?;
    Some((f, g.parse().ok()?, i.parse().ok()?))
}

// ───────────────────────────────── the sounds ─────────────────────────────────

/// A decoded sample (MP3 frames) and how long it plays at speed 1.
#[derive(Clone)]
struct Clip {
    bytes: Arc<[u8]>,
    secs: f32,
}

struct LoadedEvent {
    project: usize,
    event: fev::Event,
    /// (sound definition, waveform) → the sample.
    clips: HashMap<(usize, usize), Clip>,
}

/// A weapon's sounds, as event keys.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponKeys {
    /// The shot (a loop for an automatic).
    pub fire: Key,
    pub looping: bool,
    /// What sounds when the trigger is released (the slap-back, the early reflection).
    pub tail: Option<Key>,
    pub shell: Option<Key>,
    pub reload: Option<Key>,
    /// Seconds the tail waits after the last shot.
    pub hold: f32,
    /// The module's `max_playbacks` of the shot.
    pub limit: u32,
}

/// An engine's three events.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineKeys {
    pub key_on: Option<Key>,
    pub sustain: Key,
    pub key_off: Option<Key>,
}

/// The ambience of a zone set: its bed and the details around the listener.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneKeys {
    pub name: String,
    pub bed: Key,
    pub detail: Option<Key>,
    pub sprinkle: Option<Key>,
}

/// The sounds: the id tables, the loaded events, and the samples picked by name for the
/// keys whose event did not resolve.
#[derive(Resource, Default)]
pub struct Sounds {
    fs: Option<Arc<jc2_formats::GameFs>>,
    pub map: Option<SoundMap>,
    projects: Vec<EventFile>,
    project_of: HashMap<String, usize>,
    banks: Banks,
    events: HashMap<Key, Option<LoadedEvent>>,
    /// Logical name → the id key that replaces it.
    alias: HashMap<&'static str, Key>,
    /// Samples picked by name, by key.
    by_name: HashMap<&'static str, Vec<Clip>>,
    /// Names played from the by-name table so far (each logged once).
    pub named: HashSet<&'static str>,
    effects: HashMap<&'static str, Vec<EffectSound>>,
    rng: u32,
}

impl Sounds {
    pub fn new(fs: Arc<jc2_formats::GameFs>) -> Self {
        let map = match SoundMap::load(&fs) {
            Ok(m) => Some(m),
            Err(e) => {
                warn!(
                    "audio: the sound id tables did not load ({e}); every sound is picked by name"
                );
                None
            }
        };
        let mut s = Self {
            fs: Some(fs),
            map,
            rng: 0x1234_5678,
            ..Default::default()
        };
        s.build_aliases();
        s
    }

    fn rand(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }

    /// Resolve the event a bank/sound pair names to its key.
    pub fn ref_key(&self, r: SoundRef) -> Option<Key> {
        let e = self.map.as_ref()?.event(r)?;
        Some(ev_key(&e.file, e.group, e.index))
    }

    /// The key to play for a logical name: the id key that replaced it, else the name.
    pub fn key(&self, name: Key) -> Key {
        self.alias.get(name).copied().unwrap_or(name)
    }

    fn read_effect(&mut self, name: &'static str) -> Option<Vec<EffectSound>> {
        let fs = self.fs.as_ref()?;
        let data = fs.read_inflated(&format!("effects\\{name}.bin")).ok()?;
        let v = jc2_formats::soundmap::effect_sounds(&data).ok()?;
        (!v.is_empty()).then_some(v)
    }

    /// The sound an effect file plays on a ground material (a `GameMaterial` id; `None`
    /// for any).
    pub fn effect_key(&self, effect: &str, material: Option<u32>) -> Option<Key> {
        let v = self.effects.get(effect)?;
        let map = self.map.as_ref()?;
        let hash = material
            .and_then(|m| map.materials.get(m))
            .map(|m| jc2_formats::hash::hash_str(&m.name));
        self.ref_key(jc2_formats::soundmap::pick_effect_sound(v, hash)?)
    }

    fn build_aliases(&mut self) {
        // effects whose Sound component names the sound
        for (logical, effect) in [
            ("explosion_small", "grenade_explosion"),
            ("explosion_medium", "explosion_fuel_depot_small"),
            ("explosion_big", "explosion_lave"),
            ("explosion_barrel", "explosion_barrel"),
            ("explosion_missile", "missile_explosion"),
            ("debris", "destruction_default"),
            ("rocket_loop", "missile_fire"),
            ("grapple_fire", "flying_hook"),
            ("grapple_detach", "hook_detach"),
        ] {
            if let Some(v) = self.read_effect(effect) {
                self.effects.insert(effect, v);
                if let Some(k) = self.effect_key(effect, None) {
                    self.alias.insert(logical, k);
                }
            }
        }
        // the character module's events
        let step_ref = self
            .map
            .as_ref()
            .and_then(|m| m.characters.sound("parachute_open_vertical", None));
        if let Some(k) = step_ref.and_then(|r| self.ref_key(r)) {
            self.alias.insert("chute_open", k);
        }
        // the shots of the weapons the soldiers and the rocket use
        let fire_of = |s: &Sounds, name: &str| s.weapon_keys(name, 0).map(|k| k.fire);
        if let Some(k) = fire_of(self, "RocketLauncher") {
            self.alias.insert("rocket_fire", k);
        }
        if let Some(k) = fire_of(self, "Assault") {
            self.alias.insert("enemy_rifle", k);
        }
        // the sounds the game asks for by name that the data gives no id for
        let by_name: Vec<&str> = [
            "explosion_small",
            "explosion_medium",
            "explosion_big",
            "debris",
            "rocket_loop",
            "rocket_fire",
            "enemy_rifle",
            "grapple_fire",
            "grapple_detach",
            "chute_open",
            "chute_loop",
            "wind",
        ]
        .into_iter()
        .filter(|n| !self.alias.contains_key(n))
        .collect();
        info!(
            "audio: {} sounds by id ({} effects); still by name: {}",
            self.alias.len(),
            self.effects.len(),
            by_name.join(", ")
        );
    }

    // ----- the id tables, as keys -----

    /// A weapon's sounds by its name in `weapons.bin` at an upgrade level.
    pub fn weapon_keys(&self, weapon: &str, level: u32) -> Option<WeaponKeys> {
        let map = self.map.as_ref()?;
        let ids = map.weapon_ids_by_name(weapon)?;
        let set = map.weapon_set(ids, level)?;
        let fire = set.fire()?;
        let key_of = |name: &str| set.layer(name).and_then(|l| self.ref_key(l.sound));
        Some(WeaponKeys {
            fire: self.ref_key(fire.sound)?,
            looping: fire.looping,
            tail: set.tail().and_then(|l| self.ref_key(l.sound)),
            shell: key_of("fire_shell"),
            reload: key_of("fire_reload"),
            hold: set.hold_ms as f32 / 1000.0,
            limit: fire.max_playbacks.max(1),
        })
    }

    /// The events of an engine preset of a vehicle module (`lave_engine`, `seve_engine`,
    /// `arve_engine`).
    pub fn engine_keys(&self, module: &str, preset: &str) -> Option<EngineKeys> {
        let p = self.map.as_ref()?.vehicles.engine(module, preset)?;
        Some(EngineKeys {
            key_on: p.key_on.and_then(|r| self.ref_key(r)),
            sustain: self.ref_key(p.sustain?)?,
            key_off: p.key_off.and_then(|r| self.ref_key(r)),
        })
    }

    /// Secondary event of the effects preset chosen for this vehicle.
    fn vehicle_effect(&self, name: &str, role: &str) -> Option<Key> {
        let (module, preset) = vehicle_fx::effects_preset(name)?;
        let p = self.map.as_ref()?.vehicles.module(module)?.preset(preset)?;
        self.ref_key(p.secondary.iter().find(|(n, _)| n == role)?.1)
    }

    /// The sound of a character event on a ground material (`footstep`, `jump_land` …).
    pub fn character_key(&self, event: &str, material: Option<u32>) -> Option<Key> {
        let r = self.map.as_ref()?.characters.sound(event, material)?;
        self.ref_key(r)
    }

    /// The ambience of a climate zone, overridden by the game zone under the listener.
    pub fn zone_keys(&self, climate: u32, game_zone: Option<u32>) -> Option<ZoneKeys> {
        let z = self
            .map
            .as_ref()?
            .environment
            .zone_set(climate, game_zone)?;
        Some(ZoneKeys {
            name: z.name.clone(),
            bed: self.ref_key(z.bed)?,
            detail: z.detail_looping.and_then(|r| self.ref_key(r)),
            sprinkle: z.sprinkle.and_then(|r| self.ref_key(r)),
        })
    }

    /// The hardness of a ground material, for the shells.
    pub fn hardness(&self, material: Option<u32>) -> f32 {
        self.map
            .as_ref()
            .map_or(0.5, |m| m.weapons.hardness_of(material))
    }

    // ----- loading -----

    fn project(&mut self, file: &str) -> Option<usize> {
        if let Some(&i) = self.project_of.get(file) {
            return Some(i);
        }
        let fs = self.fs.as_ref()?;
        let f = EventFile::parse(&fs.read(file).ok()?).ok()?;
        self.projects.push(f);
        self.project_of
            .insert(file.to_string(), self.projects.len() - 1);
        Some(self.projects.len() - 1)
    }

    /// Load the event `key` names (its samples too) unless it is loaded: whether it plays.
    pub fn ensure(&mut self, key: Key) -> bool {
        if let Some(e) = self.events.get(key) {
            return e.is_some();
        }
        let loaded = self.load_event(key);
        if loaded.is_none() {
            if !EVENTS.iter().any(|e| e.0 == key) && parse_ev_key(key).is_some() {
                warn!("audio: {key} does not resolve to an event");
            }
            self.load_fallback(key);
        }
        let ok = loaded.is_some();
        self.events.insert(key, loaded);
        ok
    }

    fn load_event(&mut self, key: Key) -> Option<LoadedEvent> {
        let fs = self.fs.clone()?;
        let (pi, ev) = if let Some((file, g, i)) = parse_ev_key(key) {
            let pi = self.project(file)?;
            (pi, self.projects[pi].event_in_group(g, i)?.clone())
        } else {
            let &(_, project, path) = EVENTS.iter().find(|e| e.0 == key)?;
            let pi = self.project(project)?;
            if self.named.insert(key) {
                warn!("audio: {key} plays by name ({project} {path})");
            }
            (pi, self.projects[pi].event(path)?.clone())
        };
        let wanted = resolve_event(&fs, &mut self.banks, &self.projects[pi], &ev);
        let clips: HashMap<(usize, usize), Clip> = wanted
            .into_iter()
            .filter_map(|(k, (bank, i))| Some((k, self.banks.clip(&fs, &bank, i)?)))
            .collect();
        if clips.is_empty() {
            warn!("audio: {key} resolves to no sample");
            return None;
        }
        Some(LoadedEvent {
            project: pi,
            event: ev,
            clips,
        })
    }

    fn load_fallback(&mut self, key: Key) {
        let Some(&(k, bank, name)) = FALLBACK.iter().find(|f| f.0 == key) else {
            return;
        };
        let Some(fs) = self.fs.clone() else { return };
        let picks: Vec<usize> = match self.banks.bank(&fs, bank) {
            Some((_, b)) => match b.find(name) {
                Some(i) => vec![i],
                None => b.starting_with(name).collect(),
            },
            None => Vec::new(),
        };
        let clips: Vec<Clip> = picks
            .into_iter()
            .filter_map(|i| self.banks.clip(&fs, bank, i))
            .collect();
        if self.named.insert(k) {
            warn!("audio: {k} falls back to samples picked by name");
        }
        self.by_name.insert(k, clips);
    }

    fn pick_named(&mut self, key: &str) -> Option<Clip> {
        let n = self.by_name.get(key).map_or(0, Vec::len);
        if n == 0 {
            return None;
        }
        let i = (self.rand() * n as f32) as usize % n;
        Some(self.by_name[key][i].clone())
    }

    /// A project file (`music.fev`), loaded on first use.
    pub(crate) fn fev(&mut self, file: &str) -> Option<&EventFile> {
        let i = self.project(file)?;
        self.projects.get(i)
    }

    /// The event of `key`, when loaded.
    fn event(&self, key: Key) -> Option<&LoadedEvent> {
        self.events.get(key)?.as_ref()
    }

    /// The loaded event `key`, for the tests.
    #[cfg(test)]
    pub fn loaded_event(&self, key: Key) -> Option<(&EventFile, &fev::Event)> {
        let le = self.event(key)?;
        Some((&self.projects[le.project], &le.event))
    }
}

/// Where an instance sounds from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// In Rico's ears: not spatial, at distance 0.
    Ears,
    At(Vec3),
    /// Follows an entity; the instance ends when it is gone.
    On(Entity),
}

/// Event parameters a caller sets: (name, value in the parameter's units).
type Params = Vec<(&'static str, f32)>;
/// A held slot's wish: (key, volume, parameters).
type HoldWant<'a> = Option<(Key, f32, &'a [(&'static str, f32)])>;
type SinkQuery<'w, 's> = Query<'w, 's, (&'static mut AudioSink, &'static Want), Changed<Want>>;

/// What gameplay asks of the sound this frame.
enum Ask {
    Fire {
        key: Key,
        at: Place,
        volume: f32,
        set: Params,
        /// A limit on how many may sound at once, under the event's own.
        limit: Option<u32>,
    },
    Hold {
        slot: &'static str,
        want: Option<(Key, f32, Params)>,
    },
}

/// One sample player owned by an Instance, including its entity lifetime.
/// PlaybackMode::Once prevents Bevy from removing it before instance stop/expiry.
struct Shot {
    entity: Entity,
    shared: Arc<Shared>,
    gain: f32,
    ends: f32,
}

/// A loop playing: its entity, its DSP block, the random gain and speed it started with.
struct LoopVoice {
    entity: Entity,
    shared: Arc<Shared>,
    rg: f32,
    rs: f32,
}

/// A running start of an event.
struct Instance {
    key: Key,
    /// Held instances have a slot (`engine`, `wind` …); one-shots none.
    slot: Option<&'static str>,
    at: Place,
    volume: f32,
    /// In the event's parameter order and units.
    params: Vec<f32>,
    /// Explicit caller parameters override automatic FMOD parameter motion.
    controlled: HashSet<usize>,
    loops: HashMap<(usize, usize), LoopVoice>,
    shots: Vec<Shot>,
    /// One-shot sounds inside their range already fired.
    fired: HashSet<(usize, usize)>,
    /// Spawning sounds: seconds to the next trigger.
    spawn: HashMap<(usize, usize), f32>,
    /// When the last one-shot it started ends (s of `age`).
    busy_until: f32,
    age: f32,
    /// The order instances started in.
    serial: u64,
    /// The loudest sound it made lately, for stealing the quietest.
    loudness: f32,
    /// A one-shot with no timeline plays its loops once too.
    loops_once: bool,
    /// Asked to stop (by the game or stolen): it fades out, then is gone.
    stop: bool,
    /// Seconds of fade-out left once stopping.
    stopping: Option<f32>,
    finished: bool,
    /// No event: a sample picked by name, looped for held instances.
    fallback: Option<(Entity, Arc<Shared>)>,
}

#[derive(Resource, Default)]
struct Playing {
    asks: Vec<Ask>,
    /// Asks that wait: seconds left, the ask.
    later: Vec<(f32, Ask)>,
    list: Vec<Instance>,
    serial: u64,
}

impl Playing {
    fn fire(&mut self, key: Key, at: Place, volume: f32) {
        self.fire_with(key, at, volume, &[]);
    }

    fn fire_with(&mut self, key: Key, at: Place, volume: f32, set: &[(&'static str, f32)]) {
        self.asks.push(Ask::Fire {
            key,
            at,
            volume,
            set: set.to_vec(),
            limit: None,
        });
    }

    fn fire_limited(&mut self, key: Key, at: Place, volume: f32, limit: u32) {
        self.asks.push(Ask::Fire {
            key,
            at,
            volume,
            set: Vec::new(),
            limit: Some(limit),
        });
    }

    /// Fire after `delay` seconds.
    fn fire_after(
        &mut self,
        delay: f32,
        key: Key,
        at: Place,
        volume: f32,
        set: &[(&'static str, f32)],
    ) {
        self.later.push((
            delay,
            Ask::Fire {
                key,
                at,
                volume,
                set: set.to_vec(),
                limit: None,
            },
        ));
    }

    /// Keep `slot` playing `key` with these parameters (`None` stops it).
    fn hold(&mut self, slot: &'static str, want: HoldWant) {
        self.asks.push(Ask::Hold {
            slot,
            want: want.map(|(k, v, s)| (k, v, s.to_vec())),
        });
    }

    /// Instances of `key` that are not on their way out.
    #[cfg(test)]
    pub fn count(&self, key: Key) -> usize {
        self.list.iter().filter(|i| i.key == key && !i.stop).count()
    }
}

/// Where the listener is (the camera) and which way it faces.
#[derive(Resource)]
pub struct Ear {
    pub pos: Vec3,
    pub right: Vec3,
    pub fwd: Vec3,
}

impl Default for Ear {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            right: Vec3::X,
            fwd: Vec3::NEG_Z,
        }
    }
}

impl Ear {
    /// The pan (−1 left … 1 right) of a sound at `pos`, scaled by the event's pan level.
    pub fn pan_of(&self, pos: Vec3, level: f32) -> f32 {
        let rel = pos - self.pos;
        let d = rel.length();
        if d < 1.0 {
            return 0.0;
        }
        (rel.dot(self.right) / d * level).clamp(-MAX_PAN, MAX_PAN)
    }

    /// FMOD's `(listener angle)`: degrees from straight ahead, to the right positive.
    pub fn angle_of(&self, pos: Vec3) -> f32 {
        let rel = pos - self.pos;
        rel.dot(self.right).atan2(rel.dot(self.fwd)).to_degrees()
    }
}

/// No ambience bed (`--no-ambience`).
#[derive(Resource)]
pub struct NoAmbience;

/// Master and music volume, and whether the music plays (`--volume`, `--music-volume`,
/// `--no-music`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct AudioLevels {
    pub master: f32,
    pub music: f32,
    pub music_on: bool,
}

impl Default for AudioLevels {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 1.0,
            music_on: true,
        }
    }
}

/// The climate zones per tile, for the ambience.
#[derive(Resource, Default)]
struct Zones(Option<jc2_formats::landscape::ZoneMaps>);

/// The speed (pitch) of a sample player; the sink needs a frame to exist, so the instance
/// hands it over through this.
#[derive(Component)]
struct Want {
    speed: f32,
}

pub struct JcAudioPlugin;

impl Plugin for JcAudioPlugin {
    fn build(&self, app: &mut App) {
        use bevy::audio::AddAudioSource;
        app.add_audio_source::<Voice>()
            .init_resource::<Sounds>()
            .init_resource::<Playing>()
            .init_resource::<Ear>()
            .init_resource::<Zones>()
            .init_resource::<Ground>()
            .init_resource::<AudioLevels>()
            .add_systems(Startup, load_sounds)
            .add_systems(PostUpdate, vehicle_fx::vehicle_effects)
            .add_systems(
                Update,
                (
                    (
                        listener,
                        ground_probe,
                        gunfire,
                        blasts.after(crate::explosion::detonate),
                        rockets,
                        rico,
                        engines,
                        ambience,
                    ),
                    run_instances,
                    apply_wants,
                )
                    .chain(),
            );
        crate::music::add(app);
    }
}

// ───────────────────────────────── banks ─────────────────────────────────

/// Which sample of `bank` a waveform plays, decoded once and shared.
#[derive(Default)]
struct Banks {
    banks: HashMap<String, Option<(Vec<u8>, SoundBank)>>,
    clips: HashMap<(String, usize), Option<Clip>>,
}

impl Banks {
    fn bank(&mut self, fs: &jc2_formats::GameFs, file: &str) -> Option<&(Vec<u8>, SoundBank)> {
        self.banks
            .entry(file.to_string())
            .or_insert_with(|| {
                let d = fs.read(file).ok()?;
                let b = SoundBank::parse(&d).ok()?;
                Some((d, b))
            })
            .as_ref()
    }

    fn clip(&mut self, fs: &jc2_formats::GameFs, file: &str, i: usize) -> Option<Clip> {
        let key = (file.to_string(), i);
        if let Some(c) = self.clips.get(&key) {
            return c.clone();
        }
        let c = self.bank(fs, file).and_then(|(d, b)| {
            let s = &b.samples[i];
            let secs = s.length_samples as f32 / s.frequency.max(1) as f32;
            b.mpeg(d, i).map(|mp3| Clip {
                bytes: mp3.into(),
                secs,
            })
        });
        self.clips.insert(key, c.clone());
        c
    }
}

/// Resolve every waveform `e` can play to a sample of its bank (`fev::resolve_sample`
/// checks the index against the name). Returns (sound def, waveform) → (bank file,
/// sample index).
fn resolve_event(
    fs: &jc2_formats::GameFs,
    banks: &mut Banks,
    file: &EventFile,
    e: &fev::Event,
) -> HashMap<(usize, usize), (String, usize)> {
    let mut out = HashMap::new();
    for sd in file.sound_defs_of(e) {
        for (wi, w) in file.sound_defs[sd].waveforms.iter().enumerate() {
            let WaveKind::Sample {
                file: name,
                bank,
                index,
                ..
            } = &w.kind
            else {
                continue;
            };
            let bank_file = format!("{}.fsb", bank.to_ascii_lowercase());
            let Some((_, b)) = banks.bank(fs, &bank_file) else {
                continue;
            };
            if let Some(i) = fev::resolve_sample(name, *index, b) {
                out.insert((sd, wi), (bank_file, i));
            }
        }
    }
    out
}

fn load_sounds(game: Res<Game>, mut zones: ResMut<Zones>, mut commands: Commands) {
    let sounds = Sounds::new(game.fs.clone());
    zones.0 = jc2_formats::landscape::ZoneMaps::load(&game.fs).ok();
    info!(
        "audio: id tables {}",
        if sounds.map.is_some() {
            "loaded"
        } else {
            "missing"
        }
    );
    commands.insert_resource(sounds);
}

/// The camera is where Rico hears from.
fn listener(cams: Query<&GlobalTransform, With<FlyCamera>>, mut ear: ResMut<Ear>) {
    if let Some(tf) = cams.iter().next() {
        ear.pos = tf.translation();
        ear.right = tf.right().as_vec3();
        ear.fwd = tf.forward().as_vec3();
    }
}

// ───────────────────────────────── the ground ─────────────────────────────────

/// What the ground under a position is, for the sounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Surface {
    /// The terrain tile's gameplay material byte.
    pub terrain: u8,
    /// Its `GameMaterial`: the id the footsteps, impacts and shells are chosen by
    /// (`game_materials.bin`).
    pub material: u32,
    /// The game zone the material belongs to (`gamezone.bin`: 1 arctic, 2 desert, 3 jungle,
    /// 4 sea, 5 city, 6 grass, 7 default).
    pub game_zone: u32,
}

type TileMaterials = ((u32, u32), Option<Vec<u8>>);

/// The terrain's gameplay materials under Rico, read from the tile files on a worker
/// thread as he moves between tiles. `here` is the surface under him.
#[derive(Resource)]
pub struct Ground {
    tiles: HashMap<(u32, u32), Option<Vec<u8>>>,
    asked: HashSet<(u32, u32)>,
    tx: std::sync::mpsc::Sender<TileMaterials>,
    rx: Mutex<std::sync::mpsc::Receiver<TileMaterials>>,
    materials: Option<jc2_formats::landscape::TerrainMaterials>,
    zones: Option<jc2_formats::civilians::GameZones>,
    pub here: Option<Surface>,
}

impl Default for Ground {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            tiles: HashMap::new(),
            asked: HashSet::new(),
            tx,
            rx: Mutex::new(rx),
            materials: None,
            zones: None,
            here: None,
        }
    }
}

/// The game zone ids of `terrain\gamezone.bin`'s name table (`#3183ca98`).
pub fn game_zone_id(name: &str) -> u32 {
    match name {
        "arctic" => 1,
        "desert" => 2,
        "jungle" => 3,
        "sea" => 4,
        "city" => 5,
        "grass" => 6,
        _ => 7,
    }
}

impl Ground {
    /// The surface at a Bevy position, when its tile has been read; asks for the tile if
    /// not (the answer comes a few frames later).
    pub fn surface_at(&mut self, fs: &Arc<jc2_formats::GameFs>, pos: Vec3) -> Option<Surface> {
        while let Ok((k, m)) = self.rx.lock().unwrap().try_recv() {
            self.tiles.insert(k, m);
        }
        if self.materials.is_none() {
            self.materials = jc2_formats::landscape::TerrainMaterials::load(fs).ok();
            self.zones = fs
                .read_inflated("gamezone.bin")
                .ok()
                .and_then(|d| jc2_formats::civilians::GameZones::parse(&d).ok());
        }
        let (ex, ez) = crate::coords::engine_xz(pos);
        let key = jc2_formats::terrain::tile_index(ex, ez)?;
        if !self.tiles.contains_key(&key) {
            if self.asked.insert(key) {
                let (fs, tx) = (fs.clone(), self.tx.clone());
                std::thread::spawn(move || {
                    let m = fs
                        .read_inflated(&jc2_formats::terrain::tile_file_name(key.0, key.1))
                        .ok()
                        .and_then(|d| jc2_formats::terrain::TerrainTile::parse(&d).ok())
                        .map(|t| t.materials);
                    let _ = tx.send((key, m));
                });
            }
            return None;
        }
        let mats = self.tiles.get(&key)?.as_ref()?;
        let (x0, z0) = jc2_formats::terrain::tile_origin(key.0, key.1);
        Some(self.surface_in(mats, ex - x0, ez - z0))
    }

    fn surface_in(&self, mats: &[u8], lx: f32, lz: f32) -> Surface {
        let n = jc2_formats::terrain::TILE_SAMPLES as i32;
        let c = ((lx * 0.25 + 1.0) as i32 + 1).clamp(0, n - 1) as usize;
        let r = ((lz * 0.25 + 1.0) as i32 + 1).clamp(0, n - 1) as usize;
        let terrain = mats.get(r * n as usize + c).copied().unwrap_or(0);
        Surface {
            terrain,
            material: self
                .materials
                .as_ref()
                .and_then(|m| m.get(terrain))
                .map_or(1, |m| m.game_material),
            game_zone: game_zone_id(
                self.zones
                    .as_ref()
                    .map_or("default", |z| z.of_material(terrain)),
            ),
        }
    }

    /// A tile's materials handed in directly (the tests).
    #[cfg(test)]
    pub fn insert_tile(&mut self, key: (u32, u32), materials: Vec<u8>) {
        self.tiles.insert(key, Some(materials));
    }

    #[cfg(test)]
    pub fn set_tables(
        &mut self,
        materials: jc2_formats::landscape::TerrainMaterials,
        zones: Option<jc2_formats::civilians::GameZones>,
    ) {
        self.materials = Some(materials);
        self.zones = zones;
    }
}

/// The surface under Rico, ten times a second.
fn ground_probe(
    time: Res<Time>,
    game: Res<Game>,
    mut ground: ResMut<Ground>,
    heroes: Query<&GlobalTransform, With<Hero>>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < 0.1 {
        return;
    }
    *since = 0.0;
    let Ok(tf) = heroes.single() else { return };
    let here = ground.surface_at(&game.fs, tf.translation());
    if here.is_some() {
        ground.here = here;
    }
}

// ───────────────────────────────── playing ─────────────────────────────────

/// The value a parameter starts at when the game does not set it.
fn default_param(p: &fev::Param, rand: f32) -> f32 {
    match p.name.as_str() {
        // everything Rico hears is what he attends to
        "focus" => p.max.min(1.0).max(p.min),
        "variation" | "var" => p.min + (p.max - p.min) * rand,
        "depth" | "timeofday" => (p.min + p.max) / 2.0,
        "(listener angle)" => 0.0f32.clamp(p.min, p.max),
        // a hard floor is the default for the shells
        "hardness" => 0.5f32.clamp(p.min, p.max),
        _ => p.min,
    }
}

fn is_distance(name: &str) -> bool {
    matches!(name, "(distance)" | "distance" | "average distance")
}

/// What the max-playbacks rule says about a new instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    Start,
    /// Start it after stopping the instance at this index.
    Steal(usize),
    Reject,
}

/// An instance as the stealing rule sees it: (serial, loudness, held), in list order.
pub type Rival = (u64, f32, bool);

/// FMOD Event's max-playbacks rule: with `limit` instances of the event sounding, a new
/// one starts anyway by stealing, as the event's behaviour says (0 the oldest, 1 the
/// newest, 2 the quietest, 3 just fails, 4 fails if it would be the quietest). Held
/// instances (an engine) are never stolen. `rivals` are the sounding instances of the event.
pub fn admit(rivals: &[Rival], limit: u32, behaviour: u32, loudness: f32) -> Admit {
    if (rivals.len() as u32) < limit.max(1) {
        return Admit::Start;
    }
    let free = || rivals.iter().enumerate().filter(|(_, r)| !r.2);
    let pick = |best: Option<(usize, &Rival)>| best.map_or(Admit::Reject, |(i, _)| Admit::Steal(i));
    match behaviour {
        0 => pick(free().min_by_key(|(_, r)| r.0)),
        1 => pick(free().max_by_key(|(_, r)| r.0)),
        2 => pick(free().min_by(|a, b| a.1.1.total_cmp(&b.1.1))),
        4 => match free().min_by(|a, b| a.1.1.total_cmp(&b.1.1)) {
            Some((i, r)) if loudness >= r.1 => Admit::Steal(i),
            _ => Admit::Reject,
        },
        _ => Admit::Reject,
    }
}

/// Runs the asks and every instance: parameters, envelopes, which sounds play, loops,
/// one-shots, spawns, rolloff, DSP, fades, max playbacks.
#[allow(clippy::too_many_arguments)]
fn run_instances(
    mut commands: Commands,
    time: Res<Time>,
    mut sounds: ResMut<Sounds>,
    mut playing: ResMut<Playing>,
    mut voices: ResMut<Assets<Voice>>,
    ear: Res<Ear>,
    levels: Res<AudioLevels>,
    clock: Option<Res<crate::sky::TimeOfDay>>,
    places: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    let sounds = &mut *sounds;
    let playing = &mut *playing;
    // asks that waited long enough
    let mut due = Vec::new();
    let mut keep = Vec::new();
    for (t, ask) in std::mem::take(&mut playing.later) {
        if t <= dt {
            due.push(ask);
        } else {
            keep.push((t - dt, ask));
        }
    }
    playing.later = keep;
    let asks: Vec<Ask> = std::mem::take(&mut playing.asks)
        .into_iter()
        .chain(due)
        .collect();
    for ask in asks {
        match ask {
            Ask::Fire {
                key,
                at,
                volume,
                set,
                limit,
            } => {
                if !sounds.ensure(key) {
                    // a sample picked by name
                    if let Some(clip) = sounds.pick_named(key) {
                        let shared = Shared::new(volume * levels.master, 0.0, 0.0);
                        let h = voices.add(Voice {
                            bytes: clip.bytes,
                            shared,
                            looping: false,
                        });
                        commands.spawn((AudioPlayer(h), PlaybackSettings::DESPAWN));
                    }
                    continue;
                }
                if let Ok(i) = start_instance(sounds, playing, key, None, at, volume, &set, limit) {
                    playing.list.push(i);
                }
            }
            Ask::Hold { slot, want } => {
                let cur = playing
                    .list
                    .iter()
                    .position(|i| i.slot == Some(slot) && !i.stop);
                match (cur, want) {
                    (Some(c), Some((key, volume, set))) if playing.list[c].key == key => {
                        let inst = &mut playing.list[c];
                        inst.volume = volume;
                        inst.controlled.clear();
                        if let Some(le) = sounds.event(key) {
                            for (name, v) in set {
                                if let Some(pi) = le.event.param_index(name) {
                                    inst.params[pi] = v;
                                    inst.controlled.insert(pi);
                                }
                            }
                        }
                    }
                    (cur, want) => {
                        if let Some(c) = cur {
                            playing.list[c].stop = true;
                        }
                        if let Some((key, volume, set)) = want {
                            if sounds.ensure(key) {
                                if let Ok(i) = start_instance(
                                    sounds,
                                    playing,
                                    key,
                                    Some(slot),
                                    Place::Ears,
                                    volume,
                                    &set,
                                    None,
                                ) {
                                    playing.list.push(i);
                                }
                            } else {
                                // a sample picked by name, looped
                                let fallback = sounds.pick_named(key).map(|clip| {
                                    let shared = Shared::new(volume * levels.master, 0.0, 0.0);
                                    let h = voices.add(Voice {
                                        bytes: clip.bytes,
                                        shared: shared.clone(),
                                        looping: true,
                                    });
                                    (
                                        commands
                                            .spawn((AudioPlayer(h), PlaybackSettings::ONCE))
                                            .id(),
                                        shared,
                                    )
                                });
                                playing.serial += 1;
                                let serial = playing.serial;
                                playing.list.push(Instance {
                                    key,
                                    slot: Some(slot),
                                    at: Place::Ears,
                                    volume,
                                    params: Vec::new(),
                                    controlled: HashSet::new(),
                                    loops: HashMap::new(),
                                    shots: Vec::new(),
                                    fired: HashSet::new(),
                                    spawn: HashMap::new(),
                                    busy_until: 0.0,
                                    age: 0.0,
                                    serial,
                                    loudness: volume,
                                    loops_once: false,
                                    stop: false,
                                    stopping: None,
                                    finished: false,
                                    fallback,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    let hours = clock.map(|c| c.hours);
    let mut rng = sounds.rng;
    let mut budget: usize = playing
        .list
        .iter()
        .map(|i| i.loops.len() + i.shots.len())
        .sum();
    for inst in &mut playing.list {
        step(
            &mut commands,
            &mut voices,
            sounds,
            &mut rng,
            inst,
            dt,
            &ear,
            levels.master,
            hours,
            &places,
            &mut budget,
        );
    }
    sounds.rng = rng;
    // the finished ones
    playing.list.retain_mut(|inst| {
        if !inst.finished {
            return true;
        }
        for (_, l) in inst.loops.drain() {
            commands.entity(l.entity).despawn();
        }
        for s in inst.shots.drain(..) {
            s.shared.set_gain(0.0);
            commands.entity(s.entity).despawn();
        }
        if let Some((e, _)) = inst.fallback.take() {
            commands.entity(e).despawn();
        }
        false
    });
}

fn place_of(at: Place, places: &Query<&GlobalTransform>) -> Option<Vec3> {
    match at {
        Place::Ears => None,
        Place::At(p) => Some(p),
        Place::On(e) => places.get(e).ok().map(|t| t.translation()),
    }
}

/// Start an instance of the loaded event `key`, applying its max playbacks: `Err` when it
/// fails to start. A stolen instance is told to stop (it fades out).
#[allow(clippy::too_many_arguments)]
fn start_instance(
    sounds: &mut Sounds,
    playing: &mut Playing,
    key: Key,
    slot: Option<&'static str>,
    at: Place,
    volume: f32,
    set: &[(&'static str, f32)],
    limit: Option<u32>,
) -> Result<Instance, ()> {
    let (max, behaviour, n) = {
        let le = sounds.event(key).ok_or(())?;
        (
            le.event.max_playbacks.max(1),
            le.event.max_playbacks_behavior,
            le.event.params.len(),
        )
    };
    let limit = limit.map_or(max, |l| l.min(max)).max(1);
    // the instances of this event now sounding
    let (mut idx, mut rivals): (Vec<usize>, Vec<Rival>) = playing
        .list
        .iter()
        .enumerate()
        .filter(|(_, i)| i.key == key && !i.stop)
        .map(|(n, i)| (n, (i.serial, i.loudness, i.slot.is_some())))
        .unzip();
    // steal until there is room (a lowered limit may need several)
    loop {
        match admit(&rivals, limit, behaviour, volume) {
            Admit::Start => break,
            Admit::Steal(i) => {
                playing.list[idx[i]].stop = true;
                idx.remove(i);
                rivals.remove(i);
            }
            Admit::Reject => return Err(()),
        }
    }
    let mut params = Vec::with_capacity(n);
    for i in 0..n {
        let r = sounds.rand();
        params.push(default_param(
            &sounds.event(key).ok_or(())?.event.params[i],
            r,
        ));
    }
    let le = sounds.event(key).ok_or(())?;
    let mut controlled = HashSet::new();
    for (name, v) in set {
        if let Some(pi) = le.event.param_index(name) {
            params[pi] = *v;
            controlled.insert(pi);
        }
    }
    let timeline = le.event.params.iter().any(is_timeline);
    playing.serial += 1;
    Ok(Instance {
        key,
        slot,
        at,
        volume,
        params,
        controlled,
        loops: HashMap::new(),
        shots: Vec::new(),
        fired: HashSet::new(),
        spawn: HashMap::new(),
        busy_until: 0.0,
        age: 0.0,
        serial: playing.serial,
        loudness: volume,
        loops_once: slot.is_none() && !timeline,
        stop: false,
        stopping: None,
        finished: false,
        fallback: None,
    })
}

/// One frame of an instance.
fn xorshift(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

/// A parameter that runs once from its start to its end (an explosion's debris and
/// fire come in along it); other self-moving parameters wrap.
fn is_timeline(p: &fev::Param) -> bool {
    p.velocity > 0.0 && matches!(p.name.as_str(), "timeline" | "time")
}

#[allow(clippy::too_many_arguments)]
fn step(
    commands: &mut Commands,
    voices: &mut Assets<Voice>,
    sounds: &Sounds,
    rng: &mut u32,
    inst: &mut Instance,
    dt: f32,
    ear: &Ear,
    master: f32,
    hours: Option<f32>,
    places: &Query<&GlobalTransform>,
    budget: &mut usize,
) {
    if inst.finished {
        return;
    }
    if let Some((_, shared)) = &inst.fallback {
        // a sample picked by name: its volume follows the ask; it stops at once
        if inst.stop {
            inst.finished = true;
        } else {
            shared.set_gain(inst.volume * master);
        }
        return;
    }
    let Some(le) = sounds.event(inst.key) else {
        inst.finished = true;
        return;
    };
    let e = &le.event;
    let pos = place_of(inst.at, places);
    if matches!(inst.at, Place::On(_)) && pos.is_none() {
        inst.finished = true;
        return;
    }
    let dist = pos.map_or(0.0, |p| p.distance(ear.pos));
    // fades: in over the event's fade-in, out once told to stop
    let fade_in_s = e.fade_in_ms as f32 / 1000.0;
    let fade_out_s = e.fade_out_ms as f32 / 1000.0;
    if inst.stop && inst.stopping.is_none() {
        inst.stopping = Some(fade_out_s);
    }
    let mut fade = if fade_in_s > 0.0 {
        (inst.age / fade_in_s).clamp(0.0, 1.0)
    } else {
        1.0
    };
    if let Some(left) = &mut inst.stopping {
        fade *= if fade_out_s > 0.0 {
            (*left / fade_out_s).clamp(0.0, 1.0)
        } else {
            0.0
        };
        *left -= dt;
        if *left <= 0.0 {
            inst.finished = true;
        }
    }
    let stopping = inst.stopping.is_some();
    // parameters: the distance, the angle, the clock, the ones that move by themselves
    let mut moving = false;
    for (i, p) in e.params.iter().enumerate() {
        if is_distance(&p.name) {
            inst.params[i] = dist.clamp(p.min, p.max);
        } else if p.name == "(listener angle)" {
            inst.params[i] = pos.map_or(0.0, |q| ear.angle_of(q)).clamp(p.min, p.max);
        } else if p.name == "timeofday" {
            if let Some(h) = hours {
                inst.params[i] = h.clamp(p.min, p.max);
            }
        } else if p.velocity > 0.0 && inst.age > 0.0 && !inst.controlled.contains(&i) {
            let v = inst.params[i] + p.velocity * dt;
            if is_timeline(p) {
                inst.params[i] = v.min(p.max);
                moving |= v < p.max;
            } else {
                let span = (p.max - p.min).max(1e-3);
                inst.params[i] = p.min + (v - p.min).rem_euclid(span);
            }
        }
    }
    let file = &sounds.projects[le.project];
    // 3D: the rolloff the mode names (custom curves come through the distance
    // parameter's envelopes), nothing past the end
    let far = if e.is_3d() && pos.is_some() {
        if e.mode & MODE_3D_CUSTOMROLLOFF != 0 {
            if dist > e.max_distance.max(1.0) {
                0.0
            } else {
                1.0
            }
        } else {
            e.rolloff(dist)
        }
    } else {
        1.0
    };
    let mut fire: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut start_loops: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut loudest = 0.0f32;
    for (li, layer) in e.layers.iter().enumerate() {
        let dsp = e.layer_dsp(li, &inst.params);
        let cutoff = occlusion_cutoff(dsp.occlusion);
        let pan = pos.map_or(0.0, |p| ear.pan_of(p, dsp.pan_level));
        for (si, s) in layer.sounds.iter().enumerate() {
            let id = (li, si);
            let (gain, speed) = e.sound_level(file, li, si, &inst.params);
            let gain = gain * far * inst.volume * master;
            loudest = loudest.max(gain);
            let sd = &file.sound_defs[s.sound_def as usize];
            let t = file.template_of(sd);
            if let Some(l) = inst.loops.get(&id) {
                // a loop that plays follows the values
                if gain > AUDIBLE * 0.5 || stopping {
                    l.shared.set_gain(gain * l.rg * fade);
                    l.shared.set_pan(pan);
                    l.shared.set_cutoff(cutoff);
                    commands.entity(l.entity).insert(Want {
                        speed: speed * l.rs,
                    });
                } else {
                    commands.entity(l.entity).despawn();
                    inst.loops.remove(&id);
                }
                continue;
            }
            if stopping {
                continue;
            }
            if t.spawn_max_ms > 0 {
                // re-triggered while it sounds
                if gain > AUDIBLE {
                    let left = inst.spawn.entry(id).or_insert(0.0);
                    *left -= dt;
                    if *left <= 0.0 {
                        fire.push((li, si, gain, speed));
                    }
                } else {
                    inst.spawn.remove(&id);
                }
            } else if s.loops() && !inst.loops_once {
                if gain > AUDIBLE {
                    start_loops.push((li, si, gain, speed));
                }
            } else if gain > AUDIBLE {
                // a one-shot fires as the parameter enters its range
                if inst.fired.insert(id) {
                    fire.push((li, si, gain, speed));
                }
            } else {
                inst.fired.remove(&id);
            }
        }
    }
    inst.loudness = loudest;
    let mut r = || xorshift(rng);
    for (li, si, gain, speed) in fire {
        let sd = e.layers[li].sounds[si].sound_def as usize;
        let (rg, rs) = e.randomize(file, sd, &mut r);
        let pick = file.sound_defs[sd].pick(r());
        let t = file.template_of(&file.sound_defs[sd]);
        if t.spawn_max_ms > 0 {
            let lo = t.spawn_min_ms.min(t.spawn_max_ms) as f32;
            let hi = t.spawn_max_ms as f32;
            let next = (lo + (hi - lo) * r()) / 1000.0;
            inst.spawn.insert((li, si), next.max(0.03));
        }
        let Some(clip) = pick.and_then(|w| le.clips.get(&(sd, w))) else {
            continue; // a weighted silence, or a sample the bank lacks
        };
        if *budget >= MAX_VOICES {
            continue; // the mixer is full
        }
        *budget += 1;
        let dsp = e.layer_dsp(li, &inst.params);
        let speed = (speed * rs).clamp(0.05, 8.0);
        inst.busy_until = inst.busy_until.max(inst.age + clip.secs / speed);
        let shared = Shared::new(
            gain * rg * fade,
            pos.map_or(0.0, |p| ear.pan_of(p, dsp.pan_level)),
            occlusion_cutoff(dsp.occlusion),
        );
        let h = voices.add(Voice {
            bytes: clip.bytes.clone(),
            shared: shared.clone(),
            looping: false,
        });
        let ent = commands
            .spawn((AudioPlayer(h), PlaybackSettings::ONCE.with_speed(speed)))
            .id();
        inst.shots.push(Shot {
            entity: ent,
            shared,
            gain: gain * rg,
            ends: inst.age + clip.secs / speed + 0.1,
        });
    }
    for (li, si, gain, speed) in start_loops {
        let sd = e.layers[li].sounds[si].sound_def as usize;
        let (rg, rs) = e.randomize(file, sd, &mut r);
        let pick = file.sound_defs[sd].pick(r());
        let Some(clip) = pick.and_then(|w| le.clips.get(&(sd, w))) else {
            continue;
        };
        if *budget >= MAX_VOICES {
            continue;
        }
        *budget += 1;
        let dsp = e.layer_dsp(li, &inst.params);
        let shared = Shared::new(
            gain * rg * fade,
            pos.map_or(0.0, |p| ear.pan_of(p, dsp.pan_level)),
            occlusion_cutoff(dsp.occlusion),
        );
        let h = voices.add(Voice {
            bytes: clip.bytes.clone(),
            shared: shared.clone(),
            looping: true,
        });
        let ent = commands
            .spawn((
                AudioPlayer(h),
                PlaybackSettings {
                    mode: PlaybackMode::Once,
                    speed: speed * rs,
                    ..default()
                },
            ))
            .id();
        inst.loops.insert(
            (li, si),
            LoopVoice {
                entity: ent,
                shared,
                rg,
                rs,
            },
        );
    }
    // the one-shots in the air fade with the instance and are forgotten when done
    for s in &inst.shots {
        if stopping {
            s.shared.set_gain(s.gain * fade);
        }
    }
    let age = inst.age;
    // These players are owned by the Instance, not Bevy's natural audio cleanup.
    // Retire each expired shot here even when its held event continues indefinitely.
    // Removing it from bookkeeping prevents finished-instance cleanup owning it twice.
    inst.shots.retain(|s| {
        if s.ends > age {
            true
        } else {
            commands.entity(s.entity).despawn();
            false
        }
    });
    inst.age += dt;
    // a one-shot ends when its timeline has run and its sounds have played
    if inst.slot.is_none() && !moving && inst.age > 0.05 && inst.age >= inst.busy_until {
        inst.finished = true;
    }
}

/// Hand the instances' speed to the sinks.
fn apply_wants(mut sinks: SinkQuery) {
    for (a, w) in &mut sinks {
        a.set_speed(w.speed);
    }
}

// ───────────────────────────────── the game's sounds ─────────────────────────────────

#[derive(Default)]
struct GunState {
    shots: u32,
    since: f32,
    firing: bool,
    reloading: bool,
    shell_cool: f32,
    name: String,
    level: u32,
    keys: Option<WeaponKeys>,
}

/// The event for one of Rico's guns by the weapon table's name: the by-name fallback
/// when the id tables lack the weapon.
fn gun_sound(name: &str, automatic: bool) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("rocket") {
        "rocket_fire"
    } else if n.contains("grenade") {
        "grenade_launcher"
    } else if n.contains("shotgun") || n.contains("sawed") {
        "shotgun"
    } else if n.contains("sniper") {
        "sniper"
    } else if n.contains("smg") || n.contains("submachine") || n.contains("scorpion") {
        "smg"
    } else if automatic || n.contains("assault") || n.contains("rifle") || n.contains("mg") {
        "rifle"
    } else {
        "pistol"
    }
}

/// Rico's shots (the automatic's loop held while it keeps firing, its tail when it stops;
/// a single shot's tail after its hold; the shell on the ground; the reload) and the
/// soldiers'.
fn gunfire(
    time: Res<Time>,
    mut playing: ResMut<Playing>,
    arsenal: Option<ResMut<Arsenal>>,
    sounds: Res<Sounds>,
    ground: Res<Ground>,
    mut st: Local<GunState>,
) {
    let Some(mut a) = arsenal else { return };
    let dt = time.delta_secs();
    st.since += dt;
    st.shell_cool -= dt;
    let fired = a.shots != st.shots;
    st.shots = a.shots;
    let gun = a.gun();
    let (name, level, interval, automatic) = (
        gun.weapon.name.clone(),
        gun.level as u32,
        gun.interval,
        gun.automatic,
    );
    let reloading = gun.ammo.reloading.is_some();
    if st.name != name || st.level != level || (st.keys.is_none() && sounds.map.is_some()) {
        st.keys = sounds.weapon_keys(&name, level);
        st.name = name.clone();
        st.level = level;
    }
    let material = ground.here.map(|s| s.material);
    match st.keys.clone() {
        Some(k) => {
            if k.looping {
                if fired {
                    st.since = 0.0;
                }
                let firing = st.since < interval * 2.0 + 0.05;
                playing.hold("auto", firing.then_some((k.fire, 1.0, &[][..])));
                if st.firing
                    && !firing
                    && let Some(t) = k.tail
                {
                    playing.fire_after(k.hold, t, Place::Ears, 1.0, &[]);
                }
                st.firing = firing;
            } else {
                playing.hold("auto", None);
                st.firing = false;
                if fired {
                    playing.fire_limited(k.fire, Place::Ears, 1.0, k.limit);
                    if let Some(t) = k.tail {
                        playing.fire_after(k.hold.max(0.05), t, Place::Ears, 1.0, &[]);
                    }
                }
            }
            if fired
                && st.shell_cool <= 0.0
                && let Some(sh) = k.shell
            {
                st.shell_cool = 0.12;
                let h = sounds.hardness(material);
                playing.fire_after(0.35, sh, Place::Ears, 1.0, &[("hardness", h)]);
            }
            if reloading
                && !st.reloading
                && let Some(r) = k.reload
            {
                playing.fire(r, Place::Ears, 1.0);
            }
        }
        None => {
            let key = sounds.key(gun_sound(&name, automatic));
            if automatic {
                if fired {
                    st.since = 0.0;
                }
                let firing = st.since < interval * 2.0 + 0.05;
                playing.hold("auto", firing.then_some((key, 1.0, &[][..])));
            } else {
                playing.hold("auto", None);
                if fired {
                    playing.fire(key, Place::Ears, 1.0);
                }
            }
        }
    }
    st.reloading = reloading;
    let enemy = sounds.key("enemy_rifle");
    for at in std::mem::take(&mut a.enemy_shots) {
        playing.fire_limited(enemy, Place::At(at), 1.0, 3);
    }
}

/// The blast's effect by its size: a grenade's, a rocket's, a barrel's, a vehicle's.
fn blast_key(sounds: &Sounds, b: &crate::explosion::Blast) -> Key {
    if b.damage >= 450.0 && b.radius >= 9.0 {
        sounds.key("explosion_big")
    } else if b.radius >= 7.0 && b.damage < 400.0 {
        sounds
            .effect_key("explosion_barrel", None)
            .unwrap_or_else(|| sounds.key("explosion_medium"))
    } else if b.damage >= 400.0 {
        sounds.key("explosion_medium")
    } else {
        sounds.key("explosion_small")
    }
}

fn blasts(
    mut playing: ResMut<Playing>,
    sounds: Res<Sounds>,
    blasts: Res<Blasts>,
    hits: Option<Res<StructureHits>>,
    mut destroyed: Local<u32>,
) {
    for b in &blasts.fired {
        playing.fire(blast_key(&sounds, b), Place::At(b.at), 1.0);
    }
    if let Some(h) = hits
        && h.destroyed != *destroyed
    {
        *destroyed = h.destroyed;
        playing.fire(sounds.key("debris"), Place::Ears, 1.0);
    }
}

/// A rocket roars as it goes.
fn rockets(
    mut playing: ResMut<Playing>,
    sounds: Res<Sounds>,
    new: Query<(Entity, &Transform), Added<Projectile>>,
) {
    for (e, tf) in &new {
        playing.fire(sounds.key("rocket_fire"), Place::At(tf.translation), 1.0);
        playing.fire_with(
            sounds.key("rocket_loop"),
            Place::On(e),
            1.0,
            &[("load", 1.0)],
        );
    }
}

#[derive(Default)]
struct RicoState {
    chute: bool,
    grapple: bool,
    step: f32,
    grounded: bool,
    air: f32,
}

/// Footsteps by the ground under him, the jump and the landing, the swim, the grapple's
/// shot and release, the canopy opening and flapping, the wind of a fall.
#[allow(clippy::too_many_arguments)]
fn rico(
    time: Res<Time>,
    mut playing: ResMut<Playing>,
    sounds: Res<Sounds>,
    ground: Res<Ground>,
    heroes: Query<(&Hero, Option<&LinearVelocity>)>,
    mut st: Local<RicoState>,
) {
    let Ok((h, vel)) = heroes.single() else {
        return;
    };
    let dt = time.delta_secs();
    let grapple = h.grapple.is_some();
    if grapple != st.grapple {
        let key = if grapple {
            sounds.key("grapple_fire")
        } else {
            sounds.key("grapple_detach")
        };
        playing.fire(key, Place::Ears, 1.0);
        st.grapple = grapple;
    }
    if h.parachute && !st.chute {
        playing.fire(sounds.key("chute_open"), Place::Ears, 1.0);
    }
    st.chute = h.parachute;
    playing.hold(
        "canopy",
        h.parachute
            .then_some((sounds.key("chute_loop"), 1.0, &[("steer", 0.0)][..])),
    );
    // the wind of falling or gliding
    let v = vel.map_or(Vec3::ZERO, |v| v.0);
    let airborne = !h.grounded && h.in_vehicle.is_none() && !h.swimming && h.air_time > 0.3;
    let wind = [("speed", v.length()), ("speed vertical", v.y.abs())];
    playing.hold(
        "wind",
        airborne.then_some((sounds.key("wind"), 1.0, &wind[..])),
    );
    let material = ground.here.map(|s| s.material);
    // the jump and the landing
    if h.in_vehicle.is_none() {
        let landed = h.grounded && !st.grounded && st.air > 0.35 && !h.swimming;
        let jumped = !h.grounded && st.grounded && v.y > 2.0;
        let (event_land, event_jump) = if h.stunt.is_some() {
            ("jump_land_car", "jump_start_car")
        } else {
            ("jump_land", "jump_start")
        };
        let params = [("speed", v.length())];
        if landed && let Some(k) = sounds.character_key(event_land, material) {
            playing.fire_with(k, Place::Ears, 1.0, &params);
        }
        if jumped && let Some(k) = sounds.character_key(event_jump, material) {
            playing.fire_with(k, Place::Ears, 1.0, &params);
        }
    }
    st.air = if h.grounded { 0.0 } else { st.air + dt };
    st.grounded = h.grounded;
    // swimming
    let swim_params = [("speed", h.speed)];
    playing.hold(
        "swim",
        sounds
            .character_key("swim", None)
            .filter(|_| h.swimming && h.speed > 0.4)
            .map(|k| (k, 1.0, &swim_params[..])),
    );
    // a step every stride; the event crossfades walk and run on its speed
    if h.grounded && h.in_vehicle.is_none() && !h.swimming && h.speed > 0.6 {
        st.step -= dt;
        if st.step <= 0.0 {
            let run = h.speed > 3.0;
            st.step = (if run { 1.5 } else { 1.0 } / h.speed).clamp(0.22, 0.7);
            let event = if h.stunt.is_some() {
                "footstep_car"
            } else {
                "footstep"
            };
            let key = sounds
                .character_key(event, material)
                .unwrap_or_else(|| sounds.key("step"));
            playing.fire_with(key, Place::Ears, 1.0, &[("speed", h.speed)]);
        }
    } else {
        st.step = 0.0;
    }
}

/// Explicit legacy fallback for absent export support only. Original selections
/// use numeric EPE pairs; malformed, disabled and unresolved pairs never use this.
pub fn engine_preset(name: &str) -> (&'static str, &'static str) {
    let n = name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    if n.starts_with("seve") {
        let p = if has("hover") {
            "hovercraft_dlc"
        } else if has("swamp") {
            "swampboat"
        } else if has("attack") || has("military") {
            "military_big"
        } else if has("djonk") {
            "longtail"
        } else if has("sailboat")
            || has("houseboat")
            || has("yacht")
            || has("charter")
            || has("barge")
            || has("indo")
        {
            "bigboat"
        } else {
            "speedboat"
        };
        return ("seve_engine", p);
    }
    if n.starts_with("arve") {
        let p = if has("propeller") {
            "cessna"
        } else if has("jet_fighter") {
            "jet_plane_afterburner"
        } else if has("jumbo") || has("cargo_plane") {
            "big_jet_plane"
        } else if has("personal_jet") || has("speedplane") || has("blackplane") {
            "small_jet_plane"
        } else if has("military_helicopter") || has("transport") {
            "heli_transport"
        } else if has("attackheli") || has("chopper") {
            "jet_heli"
        } else {
            "heli_r44"
        };
        return ("arve_engine", p);
    }
    let p = if has("monstertruck") {
        "monstertruck_dlc"
    } else if has("v056") || has("atv") {
        "atv"
    } else if has("tractor") {
        "traktor"
    } else if has("v012") || has("v016") || has("tank") || has("apc") {
        "tank"
    } else if has("v004") || has("truck") || has("lorry") || has("v050") || has("v108") {
        "truck"
    } else if has("bus") {
        "bus"
    } else if has("v052_motorcross") {
        "dirtbike"
    } else if has("v007_classic") || has("v007_heavy") || has("v052_classic") {
        "harley"
    } else if has("v007") || has("v054") || has("sportbike") {
        "kawasaki"
    } else if has("v006") || has("v055") || has("moped") || has("vespa") || has("tuktuk") {
        "new moped"
    } else if has("v036") || has("v047") || has("buggy") {
        "SmJeep"
    } else if has("v014") || has("v015") || has("v017") || has("v018") || has("v022") || has("jeep")
    {
        "hummer"
    } else if has("v030") || has("super_sport") {
        "ferrari"
    } else if has("v025") || has("v028") {
        "astonmartin"
    } else if has("v024") || has("sports") {
        "Corvette"
    } else if has("v020") || has("v023") || has("v035") {
        "challenger"
    } else if has("v109") || has("limo") || has("v002_sport") {
        "passat"
    } else if has("v003") {
        "SUV"
    } else {
        "sedan"
    };
    ("lave_engine", p)
}

/// The by-name event for a vehicle when the id tables lack its engine.
fn car_event(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("bus") {
        "car_bus"
    } else if n.contains("truck") || n.contains("lorry") || n.contains("tanker") {
        "car_truck"
    } else if n.contains("bike") || n.contains("motor") || n.contains("moped") {
        "bike"
    } else if n.contains("sport") || n.contains("race") || n.contains("super") {
        "car_sport"
    } else if n.contains("jeep") || n.contains("4x4") || n.contains("offroad") {
        "car_jeep"
    } else {
        "car_sedan"
    }
}

/// The engine of the vehicle being driven: its sustain event with `rpm` and `load` from
/// the vehicle, started and stopped by the preset's `key_on` and `key_off`.
#[derive(Default)]
struct EngineState {
    entity: Option<Entity>,
    keys: Option<EngineKeys>,
    sustain: Option<Key>,
}

#[derive(Component, Clone, Debug)]
struct VehicleEngine {
    /// Full bare VFS name within the immutable mounted GameFs in Sounds. Keep
    /// the name, not just its 32-bit archive hash, as the cache identity.
    asset: String,
    keys: Option<EngineKeys>,
    sustain: Option<Key>,
    provenance: String,
}

impl Sounds {
    fn vehicle_engine(&self, name: &str) -> VehicleEngine {
        let selection = self
            .fs
            .as_ref()
            .map(|fs| VehicleSoundSelection::load(fs, name));
        self.engine_export(name, selection)
    }

    fn engine_export(
        &self,
        name: &str,
        selection: Option<jc2_formats::Result<VehicleSoundSelection>>,
    ) -> VehicleEngine {
        let asset = jc2_formats::hash::file_name(name).to_string();
        let mut result = VehicleEngine {
            asset,
            keys: None,
            sustain: None,
            provenance: String::new(),
        };
        let legacy = match selection {
            Some(Ok(selection)) => match &selection.engine_selector() {
                VehicleSelector::Absent => Some("absent engine selector".to_string()),
                VehicleSelector::Disabled => {
                    result.provenance = "export explicitly disables engine".into();
                    None
                }
                VehicleSelector::Invalid => {
                    result.provenance = format!(
                        "invalid/unsupported engine selector {:?}",
                        selection.raw_fields
                    );
                    None
                }
                VehicleSelector::Selected(pair) => {
                    result.provenance = format!(
                        "export module={} preset={}",
                        pair.module_id, pair.preset_index
                    );
                    // Pair 0 must select an engine module. Chassis/effects are not
                    // silently treated as a propulsion sound.
                    let preset = self
                        .map
                        .as_ref()
                        .filter(|_| matches!(pair.module_id, 0 | 2 | 3))
                        .and_then(|m| m.vehicles.numbered(*pair));
                    result.keys = preset.and_then(|p| {
                        Some(EngineKeys {
                            key_on: p.key_on.and_then(|r| self.ref_key(r)),
                            sustain: self.ref_key(p.sustain?)?,
                            key_off: p.key_off.and_then(|r| self.ref_key(r)),
                        })
                    });
                    result.sustain = result.keys.as_ref().map(|k| k.sustain);
                    if result.sustain.is_none() {
                        result
                            .provenance
                            .push_str(" unresolved or silent sustain; no fallback");
                    }
                    None
                }
            },
            Some(Err(jc2_formats::Error::NotFound(missing))) => {
                Some(format!("missing export support: {missing}"))
            }
            Some(Err(error)) => {
                result.provenance = format!("malformed/unreadable export: {error}; no fallback");
                None
            }
            None => Some("no mounted export filesystem".into()),
        };
        if let Some(reason) = legacy {
            let (module, preset) = engine_preset(name);
            result.keys = self.engine_keys(module, preset);
            result.sustain = Some(result.keys.as_ref().map_or_else(
                || {
                    self.key(match name.to_ascii_lowercase() {
                        n if n.starts_with("seve") => "boat",
                        n if n.contains("transport") || n.contains("cargo") => "heli_transport",
                        n if n.starts_with("arve")
                            && (n.contains("heli") || n.contains("chopper")) =>
                        {
                            "heli"
                        }
                        n if n.starts_with("arve") && n.contains("jet") => "jet",
                        n if n.starts_with("arve") => "prop",
                        _ => car_event(name),
                    })
                },
                |k| k.sustain,
            ));
            result.provenance = format!("legacy fallback ({reason}): {module}/{preset}");
        }
        result
    }
}

impl EngineState {
    fn transition(
        &mut self,
        entity: Option<Entity>,
        next: Option<&VehicleEngine>,
        playing: &mut Playing,
    ) {
        if self.entity == entity {
            return;
        }
        if let Some(off) = self.keys.take().and_then(|k| k.key_off) {
            playing.fire(off, Place::Ears, 1.0);
        }
        // Stop before start even when two vehicle instances share the same event.
        playing.hold("engine", None);
        self.entity = entity;
        self.keys = next.and_then(|n| n.keys.clone());
        self.sustain = next.and_then(|n| n.sustain);
        if let Some(on) = self.keys.as_ref().and_then(|k| k.key_on) {
            playing.fire(on, Place::Ears, 1.0);
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn engines(
    mut commands: Commands,
    mode: Res<CameraMode>,
    sounds: Res<Sounds>,
    mut playing: ResMut<Playing>,
    cars: Query<(Entity, &Vehicle, Option<&VehicleEngine>), With<Occupied>>,
    helis: Query<(Entity, &Heli, Option<&VehicleEngine>), With<Occupied>>,
    boats: Query<(Entity, &Watercraft, &LinearVelocity, Option<&VehicleEngine>), With<Occupied>>,
    planes: Query<(Entity, &Plane, &LinearVelocity, Option<&VehicleEngine>), With<Occupied>>,
    mut st: Local<EngineState>,
    mut cache: Local<HashMap<String, VehicleEngine>>,
) {
    let driving = *mode == CameraMode::Drive;
    // what is driven: its name and how hard the engine works (rpm share, load)
    let drive = if let (true, Ok((entity, v, selection))) = (driving, cars.single()) {
        let share = v.rpm / v.spec.rpm_max.max(1000.0);
        Some((
            entity,
            v.spec.name.as_str(),
            share,
            v.throttle.abs(),
            selection,
        ))
    } else if let (true, Ok((entity, h, selection))) = (driving, helis.single()) {
        Some((
            entity,
            h.name.as_str(),
            h.spool,
            h.collective.abs().max(0.2 * h.spool),
            selection,
        ))
    } else if let (true, Ok((entity, b, lin, selection))) = (driving, boats.single()) {
        let n = (lin.length() / 22.0).clamp(0.0, 1.0);
        let t = b.throttle.abs();
        Some((
            entity,
            b.name.as_str(),
            0.15 + 0.5 * t + 0.35 * n,
            t,
            selection,
        ))
    } else if let (true, Ok((entity, p, lin, selection))) = (driving, planes.single()) {
        let n = (lin.length() / Plane::top_speed(&p.params)).clamp(0.0, 1.0);
        Some((
            entity,
            p.name.as_str(),
            0.2 + 0.6 * p.throttle + 0.2 * n,
            p.throttle,
            selection,
        ))
    } else {
        None
    };
    let Some((entity, name, share, load, attached)) = drive else {
        st.transition(None, None, &mut playing);
        playing.hold("engine", None);
        return;
    };
    let asset = jc2_formats::hash::file_name(name).to_string();
    let resolved = attached.filter(|s| s.asset == asset).unwrap_or_else(|| {
        cache.entry(asset.clone()).or_insert_with(|| {
            let selection = sounds.vehicle_engine(name);
            info!("audio engine {name}: {}", selection.provenance);
            selection
        })
    });
    if attached.is_none_or(|s| s.asset != asset) {
        commands.entity(entity).insert(resolved.clone());
    }
    st.transition(Some(entity), Some(resolved), &mut playing);
    let Some(key) = st.sustain else {
        playing.hold("engine", None);
        return;
    };
    // rpm as a share of the range → the event's rpm units
    let max = sounds
        .event(key)
        .and_then(|le| le.event.params.iter().find(|p| p.name == "rpm"))
        .map_or(1.0, |p| p.max);
    let set = [("rpm", share.clamp(0.0, 1.0) * max), ("load", load)];
    playing.hold("engine", Some((key, 1.0, &set[..])));
}

/// A bed of the zone set the climate zone and the ground under Rico name (the jungle's,
/// the field's, the desert's, the city's …) and its details and sprinkles around the
/// listener, by the hour.
fn ambience(
    mut playing: ResMut<Playing>,
    sounds: Res<Sounds>,
    zones: Res<Zones>,
    ground: Res<Ground>,
    ear: Res<Ear>,
    off: Option<Res<NoAmbience>>,
    cams: Query<(), With<FlyCamera>>,
) {
    if off.is_some() || cams.is_empty() {
        playing.hold("ambience", None);
        playing.hold("sprinkle", None);
        playing.hold("detail", None);
        return;
    }
    // the tile under the listener (engine axes are Bevy's, `coords.rs`)
    let zone = zones
        .0
        .as_ref()
        .and_then(|z| {
            let (ex, ez) = crate::coords::engine_xz(ear.pos);
            jc2_formats::terrain::tile_index(ex, ez).map(|(tx, tz)| z.climate_zone(tx, tz))
        })
        .unwrap_or(0);
    let game_zone = ground.here.map(|s| s.game_zone);
    if let Some(k) = sounds.zone_keys(zone as u32, game_zone) {
        playing.hold("ambience", Some((k.bed, AMBIENCE_GAIN, &[][..])));
        playing.hold("sprinkle", k.sprinkle.map(|s| (s, AMBIENCE_GAIN, &[][..])));
        playing.hold("detail", k.detail.map(|s| (s, AMBIENCE_GAIN, &[][..])));
        return;
    }
    let (amb, spr) = match zone {
        1 => ("amb_arctic", "spr_arctic"),
        2 => ("amb_desert", "spr_desert"),
        3 => ("amb_city", "spr_city"),
        _ => ("amb_jungle", "spr_jungle"),
    };
    playing.hold("ambience", Some((amb, AMBIENCE_GAIN, &[][..])));
    playing.hold("sprinkle", Some((spr, AMBIENCE_GAIN, &[][..])));
    playing.hold("detail", None);
}

// ───────────────────────────────── tests ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::audio::{AudioSource, Decodable};

    pub(super) fn test_fs() -> Option<Arc<jc2_formats::GameFs>> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = jc2_formats::GameFs::open(dir).ok()?;
        fs.contains("weapons.fev").then(|| Arc::new(fs))
    }

    fn event_path(s: &Sounds, key: Key) -> String {
        let (f, e) = s
            .loaded_event(key)
            .unwrap_or_else(|| panic!("{key} not loaded"));
        format!("{}/{}", f.project, e.path)
    }

    /// Every event of the by-name table is in its project and resolves to samples of its
    /// bank, checked by name; every one of those samples decodes at the rate and channel
    /// count the bank states (a sample that would not decode would panic the game when it
    /// plays). Needs JC2_TEST_DIR.
    #[test]
    fn the_events_play() {
        let Some(fs) = test_fs() else { return };
        let mut banks = Banks::default();
        let mut projects: HashMap<&str, EventFile> = HashMap::new();
        let mut checked = HashSet::new();
        for &(key, project, path) in EVENTS {
            let f = projects
                .entry(project)
                .or_insert_with(|| EventFile::parse(&fs.read(project).unwrap()).unwrap());
            let e = f
                .event(path)
                .unwrap_or_else(|| panic!("{key}: no {path} in {project}"));
            let samples = resolve_event(&fs, &mut banks, f, e);
            assert!(!samples.is_empty(), "{key}: {path} resolves to nothing");
            // every waveform with weight that names a sample resolves, by name
            for sd in f.sound_defs_of(e) {
                for (wi, w) in f.sound_defs[sd].waveforms.iter().enumerate() {
                    let WaveKind::Sample { file, .. } = &w.kind else {
                        continue;
                    };
                    let Some((bank, i)) = samples.get(&(sd, wi)) else {
                        panic!("{key}: {} → {file} not in its bank", f.sound_defs[sd].name);
                    };
                    let (_, b) = banks.bank(&fs, bank).unwrap();
                    let base = file.rsplit('/').next().unwrap().to_ascii_lowercase();
                    let name = b.samples[*i].name.to_ascii_lowercase();
                    assert!(
                        base.starts_with(name.trim_end_matches(".wav")),
                        "{key}: {base} vs {name}"
                    );
                }
            }
            for (bank, i) in samples.into_values() {
                if !checked.insert((bank.clone(), i)) {
                    continue;
                }
                let (data, b) = banks.bank(&fs, &bank).unwrap();
                let src = AudioSource {
                    bytes: b.mpeg(data, i).unwrap().into(),
                };
                let dec = src.decoder();
                let s = &b.samples[i];
                assert_eq!(
                    bevy::audio::Source::sample_rate(&dec).get(),
                    s.frequency,
                    "{key}"
                );
                assert_eq!(
                    bevy::audio::Source::channels(&dec).get(),
                    s.channels,
                    "{key}"
                );
                let n = dec.count();
                let want = s.length_samples as usize * s.channels as usize;
                assert!(
                    n + 2304 * 2 >= want && n <= want + 2304 * 2,
                    "{key}: {n} of {want}"
                );
            }
        }
        assert!(checked.len() > 100, "{}", checked.len());
    }

    /// The samples picked by name when an event does not resolve are in the banks.
    #[test]
    fn the_fallbacks_exist() {
        let Some(fs) = test_fs() else { return };
        let mut banks = Banks::default();
        for &(key, bank, name) in FALLBACK {
            let (_, b) = banks.bank(&fs, bank).unwrap();
            assert!(
                b.find(name).is_some() || b.starting_with(name).next().is_some(),
                "{key}: {name} not in {bank}"
            );
        }
        for &(key, ..) in EVENTS {
            if !key.starts_with("amb_") && !key.starts_with("spr_") {
                assert!(FALLBACK.iter().any(|f| f.0 == key), "{key} has no fallback");
            }
        }
    }

    /// Rico's guns come from the data's ids: the pistol is `Pistol_Shot`, the assault rifle
    /// loops and speeds up with its upgrades, and a weapon the tables lack falls back to a name.
    #[test]
    fn gun_events_by_id() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs);
        let pistol = s.weapon_keys("Beretta Handgun", 0).unwrap();
        assert!(!pistol.looping && pistol.tail.is_some() && pistol.shell.is_some());
        assert!(pistol.reload.is_some());
        assert!(s.ensure(pistol.fire));
        assert_eq!(event_path(&s, pistol.fire), "Weapons/Single/Pistol_Shot");
        assert!(s.ensure(pistol.reload.unwrap()));
        assert_eq!(
            event_path(&s, pistol.reload.unwrap()),
            "Weapons/Handling/PistolReload"
        );
        let rifle = |s: &mut Sounds, lv| {
            let k = s.weapon_keys("Assault", lv).unwrap();
            assert!(k.looping);
            assert!(s.ensure(k.fire));
            event_path(s, k.fire)
        };
        assert_eq!(rifle(&mut s, 0), "Weapons/Auto/AR_LowRate");
        assert_eq!(rifle(&mut s, 3), "Weapons/Auto/AR_MidRate");
        assert_eq!(rifle(&mut s, 5), "Weapons/Auto/AR_HighRate");
        assert!(s.weapon_keys("No Such Gun", 0).is_none());
        assert_eq!(gun_sound("Beretta Handgun", false), "pistol");
        assert_eq!(gun_sound("Scorpion_SMG", true), "smg");
        assert_eq!(gun_sound("Shotgun (pump action)", false), "shotgun");
        assert_eq!(gun_sound("RocketLauncher", false), "rocket_fire");
        // the soldiers' and the rocket's sounds are the weapons' too
        let enemy = s.key("enemy_rifle");
        assert!(s.ensure(enemy));
        assert_eq!(event_path(&s, enemy), "Weapons/Auto/AR_LowRate");
        let rocket = s.key("rocket_fire");
        assert!(s.ensure(rocket));
        assert_eq!(
            event_path(&s, rocket),
            "Weapons/Single/Rocket Launcher Shot"
        );
    }

    /// Legacy fallback remains explicit for exports that cannot be found. These
    /// heuristic choices do not describe original vehicle assignments.
    #[test]
    fn engines_by_id() {
        assert_eq!(
            engine_preset("lave.v014_jeep.ee"),
            ("lave_engine", "hummer")
        );
        assert_eq!(
            engine_preset("lave.v047_jeep_small.ee"),
            ("lave_engine", "SmJeep")
        );
        assert_eq!(
            engine_preset("lave.v046_buscoach.ee"),
            ("lave_engine", "bus")
        );
        assert_eq!(
            engine_preset("lave.v024_sportscar.ee"),
            ("lave_engine", "Corvette")
        );
        assert_eq!(
            engine_preset("lave.v002_sedan.ee"),
            ("lave_engine", "sedan")
        );
        assert_eq!(
            engine_preset("seve.v082_motorboat_standard_small.ee").0,
            "seve_engine"
        );
        assert_eq!(
            engine_preset("arve.v071_jet_fighter.ee"),
            ("arve_engine", "jet_plane_afterburner")
        );
        assert_eq!(car_event("lave.v014_jeep.ee"), "car_jeep");
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs.clone());
        // every vehicle of the game has a preset that has a sustain event
        let map = s.map.clone().unwrap();
        let mut seen = 0;
        for e in fs.entries() {
            let Some(n) = fs.names.file(e.name_hash) else {
                continue;
            };
            let n = n.rsplit('\\').next().unwrap();
            if !(n.ends_with(".ee") && n.contains(".v")) {
                continue;
            }
            let (m, p) = engine_preset(n);
            assert!(map.vehicles.engine(m, p).is_some(), "{n} → {m}/{p}");
            seen += 1;
        }
        assert!(seen > 80, "{seen} vehicles");
        let k = s.engine_keys("lave_engine", "SmJeep").unwrap();
        assert!(s.ensure(k.sustain));
        assert_eq!(event_path(&s, k.sustain), "Vehicles/Engines/Jeep");
        let on = k.key_on.unwrap();
        assert!(s.ensure(on));
        assert_eq!(event_path(&s, on), "Vehicles/Engines/Jeep startup");
        let hummer = s.engine_keys("lave_engine", "hummer").unwrap();
        assert!(s.ensure(hummer.sustain));
        assert_eq!(
            event_path(&s, hummer.sustain),
            "Vehicles/Engines/Hummer Engine"
        );
        let rpm = s.loaded_event(hummer.sustain).unwrap().1.param_index("rpm");
        assert!(rpm.is_some());
    }

    #[test]
    fn engines_use_export_identity_and_keep_disabled_silent() {
        let Some(fs) = test_fs() else { return };
        let mut sounds = Sounds::new(fs);
        for (name, expected) in [
            ("lave.v014_jeep.ee", 20),
            ("lave.v047_jeep_small.ee", 35),
            ("lave.v024_sportscar.ee", 78),
            ("seve.v082_motorboat_standard_small.ee", 87),
            ("arve.v059_civilian_helicopter.ee", 70),
            ("arve.v010_personal_propellerplane.ee", 84),
            ("arve.v009_military_helicopter.ee", 41),
            ("arve.v071_jet_fighter.ee", 18),
        ] {
            let selection = sounds.vehicle_engine(name);
            assert!(
                !selection.provenance.contains("fallback"),
                "{name}: {}",
                selection.provenance
            );
            assert_eq!(
                selection.sustain,
                sounds.ref_key(SoundRef {
                    bank: 5,
                    sound: expected
                })
            );
            assert!(sounds.ensure(selection.sustain.unwrap()));
        }
        let disabled = sounds.vehicle_engine("arve.v062_cutscene.ee");
        assert!(disabled.keys.is_none() && disabled.sustain.is_none());
        assert!(disabled.provenance.contains("explicitly disables"));
        let missing = sounds.vehicle_engine("lave.unavailable_export.ee");
        assert!(
            missing
                .provenance
                .starts_with("legacy fallback (missing export support")
        );
        assert!(missing.sustain.is_some());
        // Even a familiar container must not guess when an override supplies
        // malformed/unknown data. Inject the loaded result without disk writes.
        for loaded in [
            Err(jc2_formats::Error::format("bad override export")),
            Ok(VehicleSoundSelection {
                slots: [
                    VehicleSelector::Invalid,
                    VehicleSelector::Absent,
                    VehicleSelector::Absent,
                ],
                raw_fields: std::array::from_fn(|_| (None, None)),
            }),
            Ok(VehicleSoundSelection {
                slots: [
                    VehicleSelector::Selected(jc2_formats::soundmap::ModulePreset {
                        module_id: 99,
                        preset_index: 0,
                    }),
                    VehicleSelector::Absent,
                    VehicleSelector::Absent,
                ],
                raw_fields: std::array::from_fn(|_| (None, None)),
            }),
            Ok(VehicleSoundSelection {
                slots: [
                    VehicleSelector::Selected(jc2_formats::soundmap::ModulePreset {
                        module_id: 0,
                        preset_index: 99,
                    }),
                    VehicleSelector::Absent,
                    VehicleSelector::Absent,
                ],
                raw_fields: std::array::from_fn(|_| (None, None)),
            }),
        ] {
            let rejected = sounds.engine_export("lave.v014_jeep.ee", Some(loaded));
            assert!(rejected.sustain.is_none() && rejected.keys.is_none());
            assert!(!rejected.provenance.starts_with("legacy fallback"));
        }
    }

    #[test]
    fn identical_vehicle_transfer_restarts_once_and_exit_stops_once() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let selection = VehicleEngine {
            asset: "fixture.ee".into(),
            keys: Some(EngineKeys {
                key_on: Some("start"),
                sustain: "running",
                key_off: Some("stop"),
            }),
            sustain: Some("running"),
            provenance: "export".into(),
        };
        let mut state = EngineState::default();
        let mut playing = Playing::default();
        state.transition(Some(a), Some(&selection), &mut playing);
        assert_eq!(playing.asks.len(), 2);
        playing.asks.clear();
        state.transition(Some(a), Some(&selection), &mut playing);
        assert!(playing.asks.is_empty());
        state.transition(Some(b), Some(&selection), &mut playing);
        assert_eq!(playing.asks.len(), 3);
        assert!(matches!(&playing.asks[0], Ask::Fire { key: "stop", .. }));
        assert!(matches!(
            &playing.asks[1],
            Ask::Hold {
                slot: "engine",
                want: None
            }
        ));
        assert!(matches!(&playing.asks[2], Ask::Fire { key: "start", .. }));
        playing.asks.clear();
        state.transition(None, None, &mut playing);
        assert_eq!(playing.asks.len(), 2);
        playing.asks.clear();
        state.transition(None, None, &mut playing);
        assert!(playing.asks.is_empty());
        let disabled = VehicleEngine {
            keys: None,
            sustain: None,
            ..selection
        };
        state.transition(Some(a), Some(&disabled), &mut playing);
        assert!(state.sustain.is_none());
        assert!(
            playing
                .asks
                .iter()
                .all(|ask| matches!(ask, Ask::Hold { want: None, .. }))
        );
    }

    /// The footstep follows the ground: sand, soil, snow, gravel, metal and asphalt are
    /// six different events, and the car roof forces the metal car.
    #[test]
    fn footsteps_follow_the_material() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs);
        let map = s.map.clone().unwrap();
        let id = |n: &str| map.materials.by_name(n).unwrap().id;
        let mut seen = HashSet::new();
        for n in [
            "Sand",
            "Soil",
            "Snow",
            "Gravel",
            "Metal",
            "Asphalt",
            "Wood_Solid",
            "Ice",
        ] {
            let k = s.character_key("footstep", Some(id(n))).unwrap();
            assert!(s.ensure(k), "{n}");
            seen.insert(k);
        }
        // asphalt and stone share the stone step; the rest differ
        assert!(seen.len() >= 7, "{seen:?}");
        let sand = s.character_key("footstep", Some(id("Sand"))).unwrap();
        let dirt = s.character_key("footstep", Some(id("Soil"))).unwrap();
        assert_ne!(sand, dirt);
        assert_eq!(event_path(&s, sand), "Character/Foley/Footsteps_Sand_Dry");
        let roof = s.character_key("footstep_car", Some(id("Sand"))).unwrap();
        assert_eq!(
            roof,
            s.character_key("footstep_car", Some(id("Wood"))).unwrap()
        );
        assert!(s.ensure(roof));
        assert_eq!(event_path(&s, roof), "Character/Foley/Footsteps_Metal_Car");
        // a sound for any ground when the material is unknown
        let any = s.character_key("footstep", None).unwrap();
        assert_eq!(any, dirt);
        // shells ring harder on metal than on sand
        assert!(s.hardness(Some(id("Metal"))) > s.hardness(Some(id("Sand"))));
    }

    /// The terrain tile under a position gives the surface: a tile's bytes → the terrain
    /// material → its `GameMaterial` → the footstep.
    #[test]
    fn ground_surface_from_the_tile() {
        let Some(fs) = test_fs() else { return };
        let mut g = Ground::default();
        let mats = jc2_formats::landscape::TerrainMaterials::load(&fs).unwrap();
        g.set_tables(
            mats.clone(),
            fs.read_inflated("gamezone.bin")
                .ok()
                .and_then(|d| jc2_formats::civilians::GameZones::parse(&d).ok()),
        );
        // a made-up tile: left half "default_desert" (sand), right half a jungle road
        let desert = mats.by_name("default_desert").unwrap();
        let road = mats.by_name("road_jungle_dirt").unwrap();
        let n = jc2_formats::terrain::TILE_SAMPLES;
        let mut bytes = vec![desert.index; n * n];
        for r in 0..n {
            for c in n / 2..n {
                bytes[r * n + c] = road.index;
            }
        }
        g.insert_tile((31, 14), bytes);
        // tile (31,14) starts at engine (-512, -9216)
        let mut at = |ex: f32, ez: f32| {
            let p = crate::coords::bevy_xyz(ex, 10.0, ez);
            g.surface_at(&fs, p).unwrap()
        };
        let sand = at(-512.0 + 20.0, -9216.0 + 256.0);
        let dirt = at(-512.0 + 480.0, -9216.0 + 256.0);
        assert_eq!(sand.material, desert.game_material);
        assert_eq!(dirt.material, road.game_material);
        assert_ne!(sand.material, dirt.material);
        let s = Sounds::new(fs);
        let a = s.character_key("footstep", Some(sand.material)).unwrap();
        let b = s.character_key("footstep", Some(dirt.material)).unwrap();
        assert_ne!(a, b);
    }

    /// The zone sets: the climate zone's default, overridden by the game zone.
    #[test]
    fn ambience_by_zone() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs);
        let field = s.zone_keys(0, None).unwrap();
        assert_eq!(field.name, "field");
        let jungle = s.zone_keys(0, Some(game_zone_id("jungle"))).unwrap();
        assert_eq!(jungle.name, "jungle");
        assert!(s.ensure(jungle.bed));
        assert_eq!(event_path(&s, jungle.bed), "Zones/Zones/Jungle_Amb");
        let arctic = s.zone_keys(1, None).unwrap();
        assert_eq!(arctic.name, "mountain");
        assert_eq!(s.zone_keys(3, None).unwrap().name, "city");
    }

    /// Effects give the sound by their own file: a grenade, a barrel, the grapple.
    #[test]
    fn effects_name_sounds() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs);
        for (logical, want) in [
            ("explosion_small", "Effects/Explosions/13 - Grenade Explo"),
            ("explosion_big", "Effects/Explos/Lave Explosion"),
            ("rocket_loop", "Weapons/Single/Rocket Launcher Loop"),
            (
                "grapple_fire",
                "Parachute/grappling_hook/grappling_fire_wire",
            ),
            (
                "grapple_detach",
                "Parachute/grappling_hook/grappling_detatch",
            ),
            ("chute_open", "Parachute/Parachute/Parachute_Vert"),
        ] {
            let k = s.key(logical);
            assert!(k.contains('#'), "{logical} still by name");
            assert!(s.ensure(k), "{logical}");
            assert_eq!(event_path(&s, k), want, "{logical}");
        }
        let barrel = s.effect_key("explosion_barrel", None).unwrap();
        assert!(s.ensure(barrel));
        assert_eq!(event_path(&s, barrel), "Effects/Explos/Barrel Explode");
    }

    /// The max-playbacks rule: under the limit everything starts; over it the oldest, the
    /// newest or the quietest is stolen, or the new one fails.
    #[test]
    fn max_playbacks_rule() {
        let rivals: Vec<Rival> = vec![(5, 0.5, false), (3, 0.2, false), (9, 0.8, false)];
        assert_eq!(admit(&rivals[..2], 3, 0, 1.0), Admit::Start);
        assert_eq!(admit(&rivals, 3, 0, 1.0), Admit::Steal(1)); // oldest = serial 3
        assert_eq!(admit(&rivals, 3, 1, 1.0), Admit::Steal(2)); // newest = serial 9
        assert_eq!(admit(&rivals, 3, 2, 1.0), Admit::Steal(1)); // quietest = 0.2
        assert_eq!(admit(&rivals, 3, 3, 1.0), Admit::Reject);
        assert_eq!(admit(&rivals, 3, 4, 0.1), Admit::Reject);
        assert_eq!(admit(&rivals, 3, 4, 0.9), Admit::Steal(1));
        // an engine is never stolen
        let held: Vec<Rival> = vec![(1, 0.5, true)];
        assert_eq!(admit(&held, 1, 0, 1.0), Admit::Reject);
    }

    /// The listener's pan and angle: a sound to the right pans right and sits at +90°.
    #[test]
    fn ear_pan_and_angle() {
        let ear = Ear::default();
        let p = Vec3::new(10.0, 0.0, 0.0);
        assert!(ear.pan_of(p, 1.0) > 0.8);
        assert!((ear.angle_of(p) - 90.0).abs() < 0.01);
        assert!(ear.pan_of(Vec3::new(-10.0, 0.0, 0.0), 1.0) < -0.8);
        // straight ahead and behind
        assert!(ear.pan_of(Vec3::new(0.0, 0.0, -10.0), 1.0).abs() < 1e-5);
        assert!((ear.angle_of(Vec3::new(0.0, 0.0, 10.0)).abs() - 180.0).abs() < 0.01);
        // a pan level of 0 centres it; up close it is centred anyway
        assert_eq!(ear.pan_of(p, 0.0), 0.0);
        assert_eq!(ear.pan_of(Vec3::new(0.5, 0.0, 0.0), 1.0), 0.0);
    }

    /// The pan level and the occlusion come out of the events' own envelopes: a rifle shot
    /// pans fully a few metres out but stays centred up close, and a sound behind the
    /// listener is dulled.
    #[test]
    fn dsp_envelopes_from_the_events() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs);
        let k = s.weapon_keys("Beretta Handgun", 0).unwrap().fire;
        assert!(s.ensure(k));
        let (_, e) = s.loaded_event(k).unwrap();
        let di = e.param_index("distance").unwrap();
        let ai = e.param_index("(listener angle)").unwrap();
        let mut v: Vec<f32> = e.params.iter().map(|p| p.min).collect();
        v[di] = 1.0;
        let near = e.layer_dsp(0, &v);
        v[di] = 60.0;
        let far = e.layer_dsp(0, &v);
        assert!(
            near.pan_level < 0.2 && far.pan_level > 0.9,
            "{near:?} {far:?}"
        );
        v[ai] = 0.0;
        let front = e.layer_dsp(0, &v);
        v[ai] = 180.0;
        let back = e.layer_dsp(0, &v);
        assert!(back.occlusion > front.occlusion + 0.2, "{front:?} {back:?}");
        assert!(occlusion_cutoff(back.occlusion) > 0.0);
        assert_eq!(occlusion_cutoff(front.occlusion), 0.0);
    }

    /// The engine: at idle the low layers sound and the high ones do not, flat out the
    /// other way round, and pitch rises with rpm.
    #[test]
    fn engine_layers_follow_rpm() {
        let Some(fs) = test_fs() else { return };
        let f = EventFile::parse(&fs.read("vehicles.fev").unwrap()).unwrap();
        let e = f.event("Engines/Jeep").unwrap();
        let mut vals: Vec<f32> = e.params.iter().map(|p| default_param(p, 0.5)).collect();
        let rpm = e.param_index("rpm").unwrap();
        let load = e.param_index("load").unwrap();
        let level = |vals: &[f32]| -> Vec<(f32, f32)> {
            e.layers
                .iter()
                .enumerate()
                .flat_map(|(li, l)| (0..l.sounds.len()).map(move |si| (li, si)))
                .map(|(li, si)| e.sound_level(&f, li, si, vals))
                .collect()
        };
        vals[load] = 1.0;
        vals[rpm] = 0.05 * e.params[rpm].max;
        let idle = level(&vals);
        vals[rpm] = 0.95 * e.params[rpm].max;
        let high = level(&vals);
        let loud = |v: &[(f32, f32)]| v.iter().filter(|(g, _)| *g > AUDIBLE).count();
        assert!(loud(&idle) > 0 && loud(&high) > 0);
        assert_ne!(
            idle.iter().map(|(g, _)| *g > AUDIBLE).collect::<Vec<_>>(),
            high.iter().map(|(g, _)| *g > AUDIBLE).collect::<Vec<_>>()
        );
    }

    /// Held horn envelopes remain in the sustained section even with slow frames;
    /// releasing the wish fades the instance out rather than leaving a loop alive.
    #[test]
    fn held_horns_pin_envelope_motion_and_release_stops_instances() {
        let Some(fs) = test_fs() else { return };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Voice>()
            .init_resource::<Playing>()
            .init_resource::<Ear>()
            .init_resource::<AudioLevels>()
            .insert_resource(Sounds::new(fs))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .add_systems(Update, run_instances);
        for name in [
            "lave.v002_sedan.ee",
            "lave.v014_jeep.ee",
            "lave.v047_jeep_small.ee",
            "lave.v004_truck.ee",
        ] {
            let key = app
                .world()
                .resource::<Sounds>()
                .vehicle_effect(name, "horn")
                .unwrap();
            for _ in 0..30 {
                app.world_mut()
                    .resource_mut::<Playing>()
                    .hold("horn", Some((key, 1.0, &[("velocity", 0.03)])));
                app.update();
                let sounds = app.world().resource::<Sounds>();
                let pi = sounds
                    .loaded_event(key)
                    .unwrap()
                    .1
                    .param_index("velocity")
                    .unwrap();
                let p = app.world().resource::<Playing>();
                let inst = p
                    .list
                    .iter()
                    .find(|i| i.slot == Some("horn") && !i.stop)
                    .unwrap();
                assert_eq!(inst.params[pi], 0.03);
                assert!(
                    !inst.loops.is_empty(),
                    "{name}: sustained horn has no voice"
                );
                assert!(
                    inst.fired.is_empty(),
                    "acoustic tail fired during held horn"
                );
            }
            app.world_mut().resource_mut::<Playing>().hold("horn", None);
            app.update();
            app.update();
            assert_eq!(app.world().resource::<Playing>().count(key), 0);
        }
    }

    #[derive(Component)]
    struct TestEndedVoice;

    // Bevy's actual PostUpdate policy, with decoder EOS supplied deterministically
    // instead of opening an audio device in a unit test.
    fn natural_voice_cleanup(
        mut commands: Commands,
        voices: Query<(Entity, &PlaybackSettings), With<TestEndedVoice>>,
    ) {
        for (entity, settings) in &voices {
            if matches!(settings.mode, PlaybackMode::Despawn) {
                commands.entity(entity).despawn();
            }
        }
    }
    fn ownership_app(fs: Arc<jc2_formats::GameFs>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Voice>()
            .init_resource::<Playing>()
            .init_resource::<Ear>()
            .init_resource::<AudioLevels>()
            .insert_resource(Sounds::new(fs))
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::ZERO,
            ))
            .insert_resource(bevy::ecs::error::FallbackErrorHandler(
                bevy::ecs::error::panic,
            ))
            .add_systems(Update, run_instances)
            .add_systems(PostUpdate, natural_voice_cleanup);
        app
    }
    fn shot_key(app: &mut App) -> Key {
        let mut sounds = app.world_mut().resource_mut::<Sounds>();
        let key = sounds.character_key("footstep", None).unwrap();
        assert!(sounds.ensure(key));
        key
    }
    #[test]
    fn voice_ownership_natural_completion_and_entity_generation_reuse() {
        let Some(fs) = test_fs() else {
            return;
        };
        let mut app = ownership_app(fs);
        let key = shot_key(&mut app);
        app.world_mut()
            .resource_mut::<Playing>()
            .fire(key, Place::Ears, 1.0);
        app.update();
        let voices: Vec<_> = app
            .world()
            .resource::<Playing>()
            .list
            .iter()
            .flat_map(|i| i.shots.iter().map(|s| s.entity))
            .collect();
        assert!(!voices.is_empty());
        assert!(app.world().resource::<Playing>().list[0].loops_once);
        for voice in &voices {
            app.world_mut().entity_mut(*voice).insert(TestEndedVoice);
        }
        // Natural decoder completion precedes the instance's estimated end.
        app.update();
        assert!(
            voices.iter().all(|e| app.world().get_entity(*e).is_ok()),
            "Bevy and Instance must not both own voice despawn"
        );
        for inst in &mut app.world_mut().resource_mut::<Playing>().list {
            inst.age = inst.busy_until + 1.0;
        }
        app.update();
        assert!(app.world().resource::<Playing>().list.is_empty());
        assert!(voices.iter().all(|e| app.world().get_entity(*e).is_err()));
        // Bevy batches freed indices; ordinary scene churn flushes that batch.
        let churn: Vec<_> = (0..256)
            .map(|_| app.world_mut().spawn_empty().id())
            .collect();
        for e in churn {
            assert!(app.world_mut().despawn(e));
        }
        let replacements: Vec<_> = (0..256)
            .map(|_| app.world_mut().spawn_empty().id())
            .collect();
        assert!(replacements.iter().any(|e| {
            voices
                .iter()
                .any(|old| e.index() == old.index() && e.generation() != old.generation())
        }));
        app.update();
        app.update();
        assert!(
            replacements
                .iter()
                .all(|e| app.world().get_entity(*e).is_ok()),
            "retired voice commands must not target a reused generation"
        );
    }
    #[test]
    fn voice_ownership_held_expiry_stop_steal_and_untracked_fallback() {
        let Some(fs) = test_fs() else {
            return;
        };
        let mut app = ownership_app(fs);
        let key = shot_key(&mut app);
        app.world_mut()
            .resource_mut::<Playing>()
            .hold("held-shot", Some((key, 1.0, &[])));
        app.update();
        let voices: Vec<_> = app.world().resource::<Playing>().list[0]
            .shots
            .iter()
            .map(|s| s.entity)
            .collect();
        assert!(!voices.is_empty());
        {
            let mut p = app.world_mut().resource_mut::<Playing>();
            let inst = &mut p.list[0];
            assert!(!inst.loops_once);
            inst.age = inst.busy_until + 1.0;
        }
        app.update();
        assert_eq!(app.world().resource::<Playing>().count(key), 1);
        assert!(app.world().resource::<Playing>().list[0].shots.is_empty());
        assert!(
            voices.iter().all(|e| app.world().get_entity(*e).is_err()),
            "held event must retire expired players, not only forget their handles"
        );
        app.world_mut()
            .resource_mut::<Playing>()
            .hold("held-shot", None);
        app.world_mut().resource_mut::<Playing>().list[0].stopping = Some(0.0);
        app.update();
        assert!(app.world().resource::<Playing>().list.is_empty());
        // Steal an actively playing original footstep using the existing oldest policy.
        app.world_mut()
            .resource_mut::<Sounds>()
            .events
            .get_mut(key)
            .unwrap()
            .as_mut()
            .unwrap()
            .event
            .max_playbacks_behavior = 0;
        app.world_mut()
            .resource_mut::<Playing>()
            .fire_limited(key, Place::Ears, 1.0, 1);
        app.update();
        let retired: Vec<_> = app.world().resource::<Playing>().list[0]
            .shots
            .iter()
            .map(|s| (s.entity, Arc::clone(&s.shared)))
            .collect();
        let serial = app.world().resource::<Playing>().list[0].serial;
        app.world_mut()
            .resource_mut::<Playing>()
            .fire_limited(key, Place::Ears, 1.0, 1);
        app.update();
        {
            let mut p = app.world_mut().resource_mut::<Playing>();
            if let Some(old) = p.list.iter_mut().find(|i| i.serial == serial) {
                assert!(old.stop);
                old.stopping = Some(0.0);
            }
        }
        app.update();
        assert!(
            retired
                .iter()
                .all(|(e, shared)| app.world().get_entity(*e).is_err() && shared.gain() == 0.0)
        );
        assert_eq!(app.world().resource::<Playing>().count(key), 1);
        let budget: usize = app
            .world()
            .resource::<Playing>()
            .list
            .iter()
            .map(|i| i.loops.len() + i.shots.len())
            .sum();
        assert!(budget <= MAX_VOICES);
        // A named fallback fire has no Instance owner: actual natural cleanup owns it.
        let clip = app
            .world()
            .resource::<Sounds>()
            .event(key)
            .unwrap()
            .clips
            .values()
            .next()
            .unwrap()
            .clone();
        {
            let mut sounds = app.world_mut().resource_mut::<Sounds>();
            sounds.events.insert("cleanup-test-fallback", None);
            sounds.by_name.insert("cleanup-test-fallback", vec![clip]);
        }
        app.world_mut()
            .resource_mut::<Playing>()
            .fire("cleanup-test-fallback", Place::Ears, 1.0);
        app.update();
        let mut q = app.world_mut().query::<(Entity, &PlaybackSettings)>();
        let fallbacks: Vec<_> = q
            .iter(app.world())
            .filter(|(_, settings)| matches!(settings.mode, PlaybackMode::Despawn))
            .map(|(e, _)| e)
            .collect();
        assert_eq!(fallbacks.len(), 1);
        for fallback in &fallbacks {
            app.world_mut().entity_mut(*fallback).insert(TestEndedVoice);
        }
        app.update();
        assert!(
            fallbacks
                .iter()
                .all(|e| app.world().get_entity(*e).is_err())
        );
    }

    /// A headless app with the instance runner: fifty pistol shots in one frame start no
    /// more than the event's max playbacks, a machine gun does not stack voices, and the
    /// mixer never holds more than `MAX_VOICES` sample players.
    #[test]
    fn instances_respect_max_playbacks() {
        let Some(fs) = test_fs() else { return };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Voice>()
            .init_resource::<Playing>()
            .init_resource::<Ear>()
            .init_resource::<AudioLevels>()
            .insert_resource(Sounds::new(fs))
            .add_systems(Update, run_instances);
        let pistol = app
            .world()
            .resource::<Sounds>()
            .weapon_keys("Beretta Handgun", 0)
            .unwrap()
            .fire;
        app.world_mut().resource_mut::<Sounds>().ensure(pistol);
        let max = app
            .world()
            .resource::<Sounds>()
            .loaded_event(pistol)
            .unwrap()
            .1
            .max_playbacks as usize;
        assert!(max > 1 && max < 50);
        for _ in 0..50 {
            app.world_mut()
                .resource_mut::<Playing>()
                .fire(pistol, Place::Ears, 1.0);
        }
        app.update();
        let live = app.world().resource::<Playing>().count(pistol);
        assert!(live >= 1 && live <= max, "{live} of {max}");
        // the module's own limit of two for the shot
        app.world_mut().resource_mut::<Playing>().list.clear();
        for _ in 0..50 {
            app.world_mut()
                .resource_mut::<Playing>()
                .fire_limited(pistol, Place::Ears, 1.0, 2);
        }
        app.update();
        assert!(app.world().resource::<Playing>().count(pistol) <= 2);
        // an event with a hundred layers' worth of loops never passes the mixer's budget
        let ak = app
            .world()
            .resource::<Sounds>()
            .weapon_keys("Assault", 0)
            .unwrap()
            .fire;
        for slot in ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10"] {
            app.world_mut()
                .resource_mut::<Playing>()
                .hold(slot, Some((ak, 1.0, &[][..])));
        }
        for _ in 0..5 {
            app.update();
        }
        let p = app.world().resource::<Playing>();
        let max = app
            .world()
            .resource::<Sounds>()
            .loaded_event(ak)
            .unwrap()
            .1
            .max_playbacks as usize;
        assert!(p.count(ak) <= max, "{} of {max}", p.count(ak));
        let voices: usize = p.list.iter().map(|i| i.loops.len() + i.shots.len()).sum();
        assert!(voices <= MAX_VOICES, "{voices}");
    }
}
