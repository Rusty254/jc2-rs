//! How the game's numeric sound ids name FMOD events.
//!
//! The game never asks FMOD for an event by path. Weapons, effects, characters,
//! vehicles, the zones and the music carry two numbers, `soundbank_id` (also written
//! `sound_bank_id` and `bank_id`) and `sound_id`, and `sound\settings_core.bin` says what
//! they mean:
//!
//! * A **bank** is an event *group* of one of the projects (`weapons.fev` group 0 `Auto`
//!   is bank 0, group 1 `Single` bank 1, `effects.fev`'s `Explosions` bank 11, …). The
//!   `soundbanks` list gives each bank its global id, the project (`project_id`, which
//!   `projects` maps to a `.fev` file), the group's position in that project
//!   (`eventgroup_id`) and the `.fsb` it plays from.
//! * A **sound** is the event's position in that group, in file order
//!   (`fev::EventFile::event_in_group`). `Single` event 5 is `Pistol_Shot`.
//!
//! Everything that plays a sound is a table of such pairs, in the `settings_smod_*.bin`
//! files ("sound modules"): [`WeaponSounds`] (a `sound_fire_id` of `weapons.bin` names a
//! *sound set*: the shot, the reload, the shell, the tail), [`CharacterSounds`] (events
//! such as `footstep` and `jump_land`, with a sound per ground material),
//! [`VehicleSounds`] (an engine's `key_on`, `sustain` and `key_off` per preset),
//! [`Environment`] (the ambience of each climate zone and game zone, the rain),
//! [`Music`] (the adaptive music: track groups chosen by the time of day and the chaos).
//! Effects (`effects\*.bin`) hold a `Sound` component with the pair, one per ground
//! material if need be: [`effect_sounds`]. The ground materials are `game_materials.bin`:
//! [`GameMaterials`], and a terrain material's `GameMaterial` (`landscape`) is one of
//! their ids.
//!
//! Vehicle export EPEs select numeric module/preset pairs. Preset keys hash numbered
//! `preset_00` ... `preset_99` names; display names are not unique identities.

use crate::hash::hash_str;
use crate::property::{Key, Node, PropertyFile, Variant};
use crate::vfs::GameFs;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// A bank and a sound in it: what an id pair in the data means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundRef {
    pub bank: u32,
    pub sound: u32,
}

/// An event named by position: the project (as `.fev` file), the top-level group and
/// the event in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRef {
    pub file: String,
    pub group: usize,
    pub index: usize,
}

/// One entry of `settings_core.bin`'s `soundbanks`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bank {
    pub id: u32,
    /// `eventgroup_name`: `auto`, `single`, `explosions` …
    pub name: String,
    pub project: u32,
    /// `eventgroup_id`: the position of the group in its project.
    pub group: u32,
    /// The sample bank the group plays from (`weapons_common.fsb`), when it has one.
    pub fsb: String,
}

fn node_ref(n: &Node, h: u32) -> Option<&Node> {
    n.children
        .iter()
        .find(|(k, _)| k.hash() == h)
        .map(|(_, c)| c)
}

fn val<'a>(n: &'a Node, name: &str) -> Option<&'a Variant> {
    n.value(name)
}

fn as_i(v: &Variant) -> Option<i32> {
    match v {
        Variant::Int(i) => Some(*i),
        Variant::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn as_f(v: &Variant) -> Option<f32> {
    match v {
        Variant::Int(i) => Some(*i as f32),
        Variant::Float(f) => Some(*f),
        _ => None,
    }
}

fn int(n: &Node, name: &str) -> Option<i32> {
    val(n, name).and_then(as_i)
}

fn flt(n: &Node, name: &str) -> Option<f32> {
    val(n, name).and_then(as_f)
}

fn int_h(n: &Node, h: u32) -> Option<i32> {
    n.value_hash(h).and_then(as_i)
}

/// The key of a child as a name when it is one, else `#hash`.
fn key_hash(k: &Key) -> u32 {
    k.hash()
}

/// The `id` the property files write as the unnamed key `#6a9dfd76` (a node's own id in
/// the sound lists).
const ID: u32 = 0x6a9d_fd76;

/// A bank/sound pair in `n`, whichever way it spells the bank.
fn sound_ref(n: &Node) -> Option<SoundRef> {
    let sound = int(n, "sound_id")?;
    let bank = int(n, "soundbank_id")
        .or_else(|| int(n, "sound_bank_id"))
        .or_else(|| int(n, "bank_id"))?;
    (sound >= 0 && bank >= 0).then_some(SoundRef {
        bank: bank as u32,
        sound: sound as u32,
    })
}

fn root<'a>(file: &'a PropertyFile, what: &str) -> Result<&'a Node> {
    file.nodes
        .first()
        .ok_or_else(|| Error::format(format!("{what}: empty")))
}

/// The module settings node of an `settings_smod_*.bin`.
fn module<'a>(file: &'a PropertyFile, what: &str) -> Result<&'a Node> {
    root(file, what)?
        .child("sound_module_settings")
        .ok_or_else(|| Error::format(format!("{what}: no sound_module_settings")))
}

// ───────────────────────────── settings_core.bin ─────────────────────────────

/// `sound\settings_core.bin`: the projects and the banks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Core {
    /// Project id → its `.fev` file name (`weapons.fev`).
    pub projects: BTreeMap<u32, String>,
    pub banks: BTreeMap<u32, Bank>,
}

impl Core {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let avalanche = root(&file, "settings_core")?
            .children
            .first()
            .map(|(_, c)| c)
            .ok_or_else(|| Error::format("settings_core: no project"))?;
        let mut core = Core::default();
        if let Some(fmod) = avalanche
            .children
            .iter()
            .map(|(_, c)| c)
            .find(|c| c.child("projects").is_some() && c.child("soundbanks").is_some())
        {
            for (_, p) in &fmod
                .child("projects")
                .map(|n| n.children.clone())
                .unwrap_or_default()
            {
                if let (Some(id), Some(name)) = (int(p, "id"), p.str("name")) {
                    core.projects.insert(id as u32, name.to_ascii_lowercase());
                }
            }
            for (_, b) in &fmod
                .child("soundbanks")
                .map(|n| n.children.clone())
                .unwrap_or_default()
            {
                let (Some(id), Some(name)) = (int_h(b, ID), b.str("eventgroup_name")) else {
                    continue;
                };
                let fsb = b
                    .child("samplebanks")
                    .and_then(|s| s.children.first())
                    .and_then(|(_, c)| {
                        c.str("name").or_else(|| match c.values.first() {
                            Some((_, Variant::Str(s))) => Some(s.as_str()),
                            _ => None,
                        })
                    })
                    .unwrap_or("")
                    .to_string();
                core.banks.insert(
                    id as u32,
                    Bank {
                        id: id as u32,
                        name: name.to_string(),
                        project: int(b, "project_id").unwrap_or(0) as u32,
                        group: int(b, "eventgroup_id").unwrap_or(0) as u32,
                        fsb,
                    },
                );
            }
        }
        if core.banks.is_empty() {
            return Err(Error::format("settings_core: no soundbanks"));
        }
        Ok(core)
    }

    /// The event a bank/sound pair names.
    pub fn event(&self, r: SoundRef) -> Option<EventRef> {
        let b = self.banks.get(&r.bank)?;
        Some(EventRef {
            file: self.projects.get(&b.project)?.clone(),
            group: b.group as usize,
            index: r.sound as usize,
        })
    }
}

// ───────────────────────────── game materials ─────────────────────────────

/// A ground material (`game_materials.bin`): the surface a sound or a particle effect
/// is chosen by.
#[derive(Clone, Debug, PartialEq)]
pub struct GameMaterial {
    pub id: u32,
    pub name: String,
    pub friction: f32,
}

/// `settings\game_materials.bin`: 35 materials, ids 0 `Sand` … 34 `Grass_Field`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameMaterials {
    pub list: Vec<GameMaterial>,
}

impl GameMaterials {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut list = Vec::new();
        for r in &file.nodes {
            r.walk(&mut |n| {
                if let (Some(name), Some(id)) = (n.str("name"), int_h(n, ID))
                    && n.value("friction").is_some()
                {
                    list.push(GameMaterial {
                        id: id as u32,
                        name: name.to_string(),
                        friction: flt(n, "friction").unwrap_or(1.0),
                    });
                }
            });
        }
        list.sort_by_key(|m| m.id);
        list.dedup_by_key(|m| m.id);
        if list.is_empty() {
            return Err(Error::format("game_materials: none"));
        }
        Ok(Self { list })
    }

    pub fn get(&self, id: u32) -> Option<&GameMaterial> {
        self.list.iter().find(|m| m.id == id)
    }

    /// The material whose name hashes to `h` (the keys of the per-material tables).
    pub fn by_hash(&self, h: u32) -> Option<&GameMaterial> {
        self.list.iter().find(|m| hash_str(&m.name) == h)
    }

    pub fn by_name(&self, name: &str) -> Option<&GameMaterial> {
        self.list.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

// ───────────────────────────────── weapons ─────────────────────────────────

/// What a layer of a weapon's sound set does (`event_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerEvent {
    /// 0: when the trigger is pulled (the shot; a loop for the automatics).
    KeyOn,
    /// 1: when it is released (the tail, the slap-back).
    KeyOff,
    /// 2: at a reload.
    Reload,
}

/// One sound of a weapon's sound set.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponLayer {
    /// The layer's key: `fire_shot`, `fire_loop`, `fire_shell`, `fire_reload` or the
    /// unnamed tail layer.
    pub key: u32,
    pub event: LayerEvent,
    pub looping: bool,
    pub sound: SoundRef,
    /// `max_playbacks`: how many of it may sound at once.
    pub max_playbacks: u32,
    /// `setup_id`: which of the module's `connection_setups` (event parameters) it uses.
    pub setup: u32,
}

impl WeaponLayer {
    /// `fire_shot`, `fire_loop`, `fire_shell`, `fire_reload`, or `tail`/`other`.
    pub fn name(&self) -> &'static str {
        for n in ["fire_shot", "fire_loop", "fire_shell", "fire_reload"] {
            if hash_str(n) == self.key {
                return n;
            }
        }
        match self.event {
            LayerEvent::KeyOff => "tail",
            _ => "other",
        }
    }

    pub fn is_shell(&self) -> bool {
        self.name() == "fire_shell"
    }
}

/// A weapon's sound set (`sound_sets` of `settings_smod_sfx_weapon.bin`).
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponSet {
    pub id: u32,
    /// The set's key, the hash of its name (`pistol_fire`, `minigun_fire` … see
    /// [`WeaponSet::name`]).
    pub key: u32,
    /// `hold_ms`: how long the module keeps the weapon's sound alive after the last
    /// shot (the tail waits this long).
    pub hold_ms: u32,
    pub layers: Vec<WeaponLayer>,
}

impl WeaponSet {
    /// The set's name when it is one of the few found by hashing.
    pub fn name(&self) -> Option<&'static str> {
        const NAMES: &[&str] = &[
            "pistol_fire",
            "revolver_fire",
            "shotgun_fire",
            "sawed_off_shotgun",
            "sniper_fire",
            "minigun_fire",
            "vulcan_fire",
            "sam_fire",
            "sentry_gun_fire",
            "cannon",
            "flak",
            "launcher",
            "grenade_launcher",
        ];
        NAMES.iter().copied().find(|n| hash_str(n) == self.key)
    }

    /// The layer called `name` (`fire_shot`, `fire_loop`, `fire_shell`, `fire_reload`).
    pub fn layer(&self, name: &str) -> Option<&WeaponLayer> {
        self.layers.iter().find(|l| l.name() == name)
    }

    /// The shot: the `fire_shot` or `fire_loop` layer that sounds when the trigger is
    /// pulled (an automatic's is a loop, flagged `looping`).
    pub fn fire(&self) -> Option<&WeaponLayer> {
        self.layers
            .iter()
            .find(|l| l.event == LayerEvent::KeyOn && matches!(l.name(), "fire_shot" | "fire_loop"))
    }

    pub fn is_automatic(&self) -> bool {
        self.fire().is_some_and(|l| l.looping)
    }

    /// The tail: what plays when the trigger is released.
    pub fn tail(&self) -> Option<&WeaponLayer> {
        self.layers.iter().find(|l| l.event == LayerEvent::KeyOff)
    }
}

/// `settings_smod_sfx_weapon.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponSounds {
    pub sets: BTreeMap<u32, WeaponSet>,
    /// `ground_hardness_maps`: how hard each [`GameMaterial`] is (0 water … 1 metal): the
    /// `hardness` the shell sounds are played with.
    pub hardness: BTreeMap<u32, f32>,
    /// The hardness of a material the map does not list.
    pub default_hardness: f32,
}

impl WeaponSounds {
    /// The ground hardness of material `id`.
    pub fn hardness_of(&self, id: Option<u32>) -> f32 {
        id.and_then(|m| self.hardness.get(&m).copied())
            .unwrap_or(self.default_hardness)
    }
}

impl WeaponSounds {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let m = module(&file, "weapon sounds")?;
        let sets_node = m
            .child("sound_sets")
            .ok_or_else(|| Error::format("weapon sounds: no sound_sets"))?;
        let mut sets = BTreeMap::new();
        for (k, s) in &sets_node.children {
            let Some(id) = int(s, "sound_set_id") else {
                continue;
            };
            let mut layers = Vec::new();
            for (lk, l) in &s.children {
                let Some(sound) = sound_ref(l) else { continue };
                layers.push(WeaponLayer {
                    key: key_hash(lk),
                    event: match int(l, "event_type").unwrap_or(0) {
                        1 => LayerEvent::KeyOff,
                        2 => LayerEvent::Reload,
                        _ => LayerEvent::KeyOn,
                    },
                    looping: int(l, "looping").unwrap_or(0) != 0,
                    sound,
                    max_playbacks: int(l, "max_playbacks").unwrap_or(1).max(0) as u32,
                    setup: int(l, "setup_id").unwrap_or(0).max(0) as u32,
                });
            }
            sets.insert(
                id as u32,
                WeaponSet {
                    id: id as u32,
                    key: key_hash(k),
                    hold_ms: int(s, "hold_ms").unwrap_or(0).max(0) as u32,
                    layers,
                },
            );
        }
        let mut hardness = BTreeMap::new();
        let mut default_hardness = 0.5;
        if let Some(maps) = m.child("ground_hardness_maps") {
            default_hardness = flt(maps, "default").unwrap_or(0.5);
            for (_, e) in &maps.children {
                if let (Some(id), Some(h)) = (int(e, "material_id"), flt(e, "ground_hardness")) {
                    hardness.insert(id as u32, h);
                }
            }
        }
        Ok(Self {
            sets,
            hardness,
            default_hardness,
        })
    }
}

/// What `weapons.bin` says about a weapon's sound: `sound_fire_id` is the weapon sound
/// set; the upgrade levels at which a faster set takes over come from the weapon's
/// `#1c6b76f9` list (`#06ea0642` the level, `sound_set_id` the set).
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponSoundIds {
    /// The weapon's `id` and `name`.
    pub id: i32,
    pub name: String,
    /// `sound_fire_id`, −1 for none.
    pub fire: i32,
    /// (upgrade level, set): from this upgrade level on, this set sounds instead.
    pub upgrades: Vec<(u32, u32)>,
}

impl WeaponSoundIds {
    /// The set to play at an upgrade level (0 = as bought): the base set, or the set of
    /// the highest upgrade level reached.
    pub fn set_at(&self, level: u32) -> Option<u32> {
        let mut best = (0u32, u32::try_from(self.fire).ok()?);
        for &(lv, set) in &self.upgrades {
            if lv <= level && lv >= best.0 && (lv > best.0 || best.0 == 0) {
                best = (lv, set);
            }
        }
        Some(best.1)
    }

    pub fn parse_all(data: &[u8]) -> Result<Vec<Self>> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Vec::new();
        for r in &file.nodes {
            r.walk(&mut |n| {
                let (Some(id), Some(name), Some(fire)) =
                    (int(n, "id"), n.str("name"), int(n, "sound_fire_id"))
                else {
                    return;
                };
                let mut upgrades = Vec::new();
                for (_, c) in &n.children {
                    for (_, u) in &c.children {
                        if let (Some(lv), Some(set)) =
                            (int_h(u, 0x06ea_0642), int(u, "sound_set_id"))
                            && lv >= 0
                            && set >= 0
                        {
                            upgrades.push((lv as u32, set as u32));
                        }
                    }
                }
                upgrades.sort();
                out.push(WeaponSoundIds {
                    id,
                    name: name.to_string(),
                    fire,
                    upgrades,
                });
            });
        }
        Ok(out)
    }
}

// ───────────────────────────────── characters ─────────────────────────────────

/// A character sound set (`primary_sounds`): one sound, or one per ground material.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterSet {
    pub id: u32,
    pub key: u32,
    /// The sound for any ground (`id_set`, or the `default` of an `id_set_map`).
    pub default: Option<SoundRef>,
    /// The sound for a [`GameMaterial`] id (`id_set_map`'s `map`).
    pub by_material: BTreeMap<u32, SoundRef>,
    /// `trigger_rules`' lower threshold: the strength an event needs to sound.
    pub min_value: f32,
}

impl CharacterSet {
    /// The sound for ground material `material` (any when the set has none for it).
    pub fn sound_for(&self, material: Option<u32>) -> Option<SoundRef> {
        material
            .and_then(|m| self.by_material.get(&m).copied())
            .or(self.default)
    }
}

/// An event of the character module: `footstep`, `jump_land`, `swim` …
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterEvent {
    pub name: String,
    /// The set it plays.
    pub set: u32,
    /// `interaction_param`: a ground material the event forces (21, `Metal_Car`, for
    /// the steps on a car's roof; 8 `Metal`; 22 the ladder's).
    pub material: Option<u32>,
}

/// `settings_smod_sfx_characters.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterSounds {
    pub sets: BTreeMap<u32, CharacterSet>,
    pub events: BTreeMap<String, CharacterEvent>,
}

impl CharacterSounds {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let m = module(&file, "character sounds")?;
        let mut out = Self::default();
        let primaries = m
            .child("sound_descriptions")
            .and_then(|d| d.child("primary_sounds"))
            .ok_or_else(|| Error::format("character sounds: no primary_sounds"))?;
        for (k, s) in &primaries.children {
            let Some(id) = int_h(s, ID) else { continue };
            let mut set = CharacterSet {
                id: id as u32,
                key: key_hash(k),
                default: None,
                by_material: BTreeMap::new(),
                min_value: s
                    .child("trigger_rules")
                    .and_then(|t| t.children.first())
                    .and_then(|(_, r)| flt(r, "min_value"))
                    .unwrap_or(0.5),
            };
            if let Some(one) = s.child("id_set") {
                set.default = sound_ref(one);
            }
            if let Some(map) = s.child("id_set_map") {
                set.default = map.child("default").and_then(sound_ref).or(set.default);
                if let Some(per) = map.child("map") {
                    for (_, e) in &per.children {
                        if let (Some(slot), Some(r)) = (int(e, "map_slot_id"), sound_ref(e)) {
                            set.by_material.insert(slot as u32, r);
                        }
                    }
                }
            }
            out.sets.insert(id as u32, set);
        }
        if let Some(table) = m.child("event_lookup_table") {
            for (_, e) in &table.children {
                let (Some(name), Some(set)) = (e.str("event_name"), int(e, "sound_set_id")) else {
                    continue;
                };
                out.events.insert(
                    name.to_string(),
                    CharacterEvent {
                        name: name.to_string(),
                        set: set as u32,
                        material: int(e, "interaction_param").map(|v| v as u32),
                    },
                );
            }
        }
        Ok(out)
    }

    /// The sound of character event `name` on ground material `material`.
    pub fn sound(&self, name: &str, material: Option<u32>) -> Option<SoundRef> {
        let e = self.events.get(name)?;
        self.sets.get(&e.set)?.sound_for(e.material.or(material))
    }
}

// ───────────────────────────────── vehicles ─────────────────────────────────

/// Verified export property pairs, in original loader order.
pub const VEHICLE_SOUND_SELECTORS: [(u32, u32); 3] = [
    (0xd0f5_0183, 0x9f99_6ed1),
    (0x1894_a3e8, 0x78bd_a2b6),
    (0x0c95_6561, 0x834b_3da2),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModulePreset {
    pub module_id: u32,
    pub preset_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VehicleSelector {
    Absent,
    Disabled,
    Selected(ModulePreset),
    Invalid,
}

/// Raw fields are retained for diagnostics; floats are never coerced to selectors.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSoundSelection {
    pub slots: [VehicleSelector; 3],
    pub raw_fields: [(Option<Variant>, Option<Variant>); 3],
}

impl VehicleSoundSelection {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        if file.nodes.len() != 1 {
            return Err(Error::format("vehicle sound export: expected one root"));
        }
        Ok(Self::from_node(&file.nodes[0]))
    }

    pub fn from_node(node: &Node) -> Self {
        let raw_fields = VEHICLE_SOUND_SELECTORS
            .map(|(m, p)| (node.value_hash(m).cloned(), node.value_hash(p).cloned()));
        let slots = std::array::from_fn(|i| {
            let (mh, ph) = VEHICLE_SOUND_SELECTORS[i];
            if [mh, ph]
                .iter()
                .any(|h| node.values.iter().filter(|(k, _)| k.hash() == *h).count() > 1)
            {
                return VehicleSelector::Invalid;
            }
            match &raw_fields[i] {
                (None, None) => VehicleSelector::Absent,
                // Native defaults a missing module to -1: an orphan preset is not
                // a valid selection, and must not enable a guessed engine.
                (Some(Variant::Int(m)), Some(Variant::Int(p))) if *m < 0 || *p < 0 => {
                    VehicleSelector::Disabled
                }
                (Some(Variant::Int(m)), Some(Variant::Int(p))) if *p < 100 => {
                    VehicleSelector::Selected(ModulePreset {
                        module_id: *m as u32,
                        preset_index: *p as u32,
                    })
                }
                _ => VehicleSelector::Invalid,
            }
        });
        Self { slots, raw_fields }
    }

    /// Engine scope starts from pair 0. Later positive assignments to the same
    /// module replace its preset, as the native module-indexed vector does.
    /// Other engine modules in later slots are unsupported, not guessed.
    pub fn engine_selector(&self) -> VehicleSelector {
        let mut selected = self.slots[0].clone();
        for i in 1..3 {
            match &self.slots[i] {
                VehicleSelector::Selected(next) if matches!(next.module_id, 0 | 2 | 3) => {
                    match selected {
                        VehicleSelector::Selected(prior) if prior.module_id == next.module_id => {
                            selected = VehicleSelector::Selected(*next);
                        }
                        _ => return VehicleSelector::Invalid,
                    }
                }
                VehicleSelector::Invalid
                    if matches!(self.raw_fields[i].0, Some(Variant::Int(0 | 2 | 3))) =>
                {
                    return VehicleSelector::Invalid;
                }
                // Native ignores a negative preset without clearing the previous
                // module slot. Negative module values skip the entire pair.
                _ => {}
            }
        }
        selected
    }

    /// Conventional top-level vehicle EPE. VFS resolves mounted overrides first;
    /// inheritance and per-placement overrides remain unsupported.
    pub fn load(fs: &GameFs, container: &str) -> Result<Self> {
        let blob = fs.read_inflated(container)?;
        let sarc = crate::sarc::SmallArchive::parse(&blob)?;
        let bare = container.rsplit(['/', '\\']).next().unwrap_or(container);
        let member = format!("{bare}.epe");
        let entry = sarc.get(&member).ok_or(Error::NotFound(member))?;
        let data = crate::compress::maybe_inflate(sarc.data(&blob, entry)?)?;
        Self::parse(&data)
    }
}

/// One engine (or siren, or horn …) preset of a vehicle module, with its sounds.
#[derive(Clone, Debug, PartialEq)]
pub struct VehiclePreset {
    /// The hash key the module uses for it.
    pub key: u32,
    /// `hummer`, `Corvette`, `bus` … (the preset's name).
    pub name: String,
    /// The engine started (`key_on`), running (`sustain`: it takes `rpm` and `load`)
    /// and stopped (`key_off`).
    pub key_on: Option<SoundRef>,
    pub sustain: Option<SoundRef>,
    pub key_off: Option<SoundRef>,
    /// The module's secondary sounds for this preset by their names (`horn`,
    /// `door_open`, `gear_change`, `load_keyoff`).
    pub secondary: Vec<(String, SoundRef)>,
}

/// A module of `settings_smod_sfx_vehicle.bin`: `lave_engine` (land), `seve_engine`
/// (sea), `arve_engine` (air), their `_effects` and `siren`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleModule {
    pub id: u32,
    pub name: String,
    pub presets: Vec<VehiclePreset>,
}

impl VehicleModule {
    /// A preset by name, ignoring case (several presets may share one, `sedan`: the
    /// first is returned).
    pub fn preset(&self, name: &str) -> Option<&VehiclePreset> {
        self.presets
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// `settings_smod_sfx_vehicle.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleSounds {
    pub modules: Vec<VehicleModule>,
    /// The traffic bed (`traffic_module`): cars passing by.
    pub traffic: Option<SoundRef>,
}

/// The sound id a preset key has in `sub` (a number, or a node with `sound_id`).
fn preset_sound(sub: &Node, key: u32) -> Option<SoundRef> {
    let presets = sub.child("presets")?;
    let bank = int(sub, "soundbank_id")?;
    let sound = match presets.value_hash(key) {
        Some(v) => as_i(v)?,
        None => int(node_ref(presets, key)?, "sound_id")?,
    };
    (sound >= 0 && bank >= 0).then_some(SoundRef {
        bank: bank as u32,
        sound: sound as u32,
    })
}

impl VehicleSounds {
    /// Numeric export identity, independent of ambiguous display names.
    pub fn numbered(&self, selection: ModulePreset) -> Option<&VehiclePreset> {
        if selection.preset_index >= 100 {
            return None;
        }
        let mut modules = self.modules.iter().filter(|m| m.id == selection.module_id);
        let module = modules.next()?;
        if modules.next().is_some() {
            return None;
        }
        let key = hash_str(&format!("preset_{:02}", selection.preset_index));
        let mut presets = module.presets.iter().filter(|p| p.key == key);
        let preset = presets.next()?;
        presets.next().is_none().then_some(preset)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let m = module(&file, "vehicle sounds")?;
        let list = m
            .child("component_list")
            .ok_or_else(|| Error::format("vehicle sounds: no component_list"))?;
        let mut modules = Vec::new();
        for (_, c) in &list.children {
            let (Some(name), Some(id)) = (c.str("name"), int_h(c, ID)) else {
                continue;
            };
            let names = c.child("presets");
            let primary = c.child("primary_sound");
            let secondary = c.child("secondary_sounds");
            let mut presets = Vec::new();
            for (k, v) in names.map(|n| n.values.as_slice()).unwrap_or(&[]) {
                let Variant::Str(pname) = v else { continue };
                let key = key_hash(k);
                let part = |role: &str| {
                    primary
                        .and_then(|p| p.child(role))
                        .and_then(|s| preset_sound(s, key))
                };
                let mut extra = Vec::new();
                if let Some(sec) = secondary {
                    for (_, s) in &sec.children {
                        if let (Some(sn), Some(r)) = (s.str("name"), preset_sound(s, key)) {
                            extra.push((sn.to_string(), r));
                        }
                    }
                }
                presets.push(VehiclePreset {
                    key,
                    name: pname.clone(),
                    key_on: part("key_on"),
                    sustain: part("sustain"),
                    key_off: part("key_off"),
                    secondary: extra,
                });
            }
            modules.push(VehicleModule {
                id: id as u32,
                name: name.to_string(),
                presets,
            });
        }
        let traffic = m.child("traffic_module").and_then(sound_ref);
        Ok(Self { modules, traffic })
    }

    pub fn module(&self, name: &str) -> Option<&VehicleModule> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// The engine preset `name` of the land (`lave_engine`), sea (`seve_engine`) or air
    /// (`arve_engine`) module.
    pub fn engine(&self, module: &str, name: &str) -> Option<&VehiclePreset> {
        self.module(module)?.preset(name)
    }
}

// ───────────────────────────────── environment ─────────────────────────────────

/// One zone sound set of `settings_smod_sfx_environment.bin` (`field`, `jungle`, `city`
/// …): a bed and the details sprinkled around the listener.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneSet {
    pub name: String,
    /// The bed (`sound_id` in the zones bank).
    pub bed: SoundRef,
    /// `detail_looping`: a loop spawned near the listener.
    pub detail_looping: Option<SoundRef>,
    /// `detail_non_looping`: the sprinkle of one-shots (birds, crickets), with the
    /// seconds between spawns (`spawn_dt`).
    pub sprinkle: Option<SoundRef>,
    pub sprinkle_dt: f32,
    /// `rain_sound_set`: which rain sounds it uses (`rain_land_hard`).
    pub rain: String,
}

/// What a climate zone plays by default and by game zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClimateSounds {
    pub default_set: String,
    /// `game_zone_id` → zone set name.
    pub by_game_zone: BTreeMap<u32, String>,
}

/// `settings_smod_sfx_environment.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Environment {
    pub zone_sets: Vec<ZoneSet>,
    /// By `climate_zone_id` (the order of `climatezonemap.dds`).
    pub climates: BTreeMap<u32, ClimateSounds>,
    /// The rain sets: `rain_land_hard`, `rain_land_soft`, `rain_water`.
    pub rain: BTreeMap<String, SoundRef>,
    pub thunder: Option<SoundRef>,
}

impl Environment {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let m = module(&file, "environment sounds")?;
        let mut out = Self::default();
        let zones = m
            .child("zones")
            .ok_or_else(|| Error::format("environment sounds: no zones"))?;
        if let Some(sets) = zones.child("sound_sets") {
            for (_, s) in &sets.children {
                let (Some(name), Some(bed)) = (s.str("name"), sound_ref(s)) else {
                    continue;
                };
                let bank = bed.bank;
                let sub = |n: &str| {
                    s.child(n).and_then(|d| {
                        int(d, "sound_id").map(|id| SoundRef {
                            bank,
                            sound: id as u32,
                        })
                    })
                };
                out.zone_sets.push(ZoneSet {
                    name: name.to_string(),
                    bed,
                    detail_looping: sub("detail_looping"),
                    sprinkle: sub("detail_non_looping"),
                    sprinkle_dt: s
                        .child("detail_non_looping")
                        .and_then(|d| flt(d, "spawn_dt"))
                        .unwrap_or(20.0),
                    rain: s.str("rain_sound_set").unwrap_or("").to_string(),
                });
            }
        }
        if let Some(lookup) = zones.child("lookup_table") {
            for (_, c) in &lookup.children {
                let Some(zone) = int(c, "climate_zone_id") else {
                    continue;
                };
                let mut cs = ClimateSounds {
                    default_set: c.str("default_sound_set").unwrap_or("").to_string(),
                    by_game_zone: BTreeMap::new(),
                };
                if let Some(g) = c.child("game_zone_sound_sets") {
                    for (_, e) in &g.children {
                        if let (Some(id), Some(set)) = (int(e, "game_zone_id"), e.str("sound_set"))
                        {
                            cs.by_game_zone.insert(id as u32, set.to_string());
                        }
                    }
                }
                out.climates.insert(zone as u32, cs);
            }
        }
        if let Some(w) = m.child("weather_content") {
            if let Some(sets) = w
                .children
                .iter()
                .map(|(_, c)| c)
                .find_map(|c| c.child("sound_sets"))
            {
                for (_, s) in &sets.children {
                    if let (Some(name), Some(r)) = (s.str("name"), sound_ref(s)) {
                        out.rain.insert(name.to_string(), r);
                    }
                }
            }
            out.thunder = w.child("thunder").and_then(sound_ref);
        }
        Ok(out)
    }

    pub fn set(&self, name: &str) -> Option<&ZoneSet> {
        self.zone_sets.iter().find(|z| z.name == name)
    }

    /// The zone set for a climate zone, overridden by the game zone when the table
    /// names one.
    pub fn zone_set(&self, climate: u32, game_zone: Option<u32>) -> Option<&ZoneSet> {
        let c = self.climates.get(&climate)?;
        let name = game_zone
            .and_then(|g| c.by_game_zone.get(&g))
            .unwrap_or(&c.default_set);
        self.set(name).or_else(|| self.set(&c.default_set))
    }
}

// ───────────────────────────────── music ─────────────────────────────────

/// A track group of the adaptive music: three tracks that play in step and are mixed by
/// the player's alert state.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicGroup {
    pub id: u32,
    /// By `slot_id`: 0 undetected, 1 combat, 2 evasion (the events are named
    /// `…_Undetected`, `…_Combat`, `…_Evasion`).
    pub slots: [Option<SoundRef>; 3],
}

/// Which groups may play in a situation: `conditions = "<time of day>:<chaos>"`.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicSelection {
    /// Condition of the time input: 0 day, 1 night (`Nig_Scary` is in the night's set).
    pub time: u32,
    /// Condition of the chaos input: 0 … 5.
    pub chaos: u32,
    /// Group ids.
    pub groups: Vec<u32>,
}

/// `settings_smod_music.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Music {
    pub bank: u32,
    pub groups: Vec<MusicGroup>,
    pub selections: Vec<MusicSelection>,
    /// The chaos input: (resulting condition, lower threshold) in the game's chaos units.
    pub chaos_rules: Vec<(u32, f32)>,
    /// The time input: (resulting condition, lower threshold in hours); the default
    /// condition applies below the first threshold.
    pub time_rules: Vec<(u32, f32)>,
    pub time_default: u32,
    /// `state_hold`: the seconds the music stays in an alert state before it may change.
    pub state_hold: f32,
}

impl Music {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let m = module(&file, "music")?;
        let adaptive = m
            .child("adaptive_music")
            .ok_or_else(|| Error::format("music: no adaptive_music"))?;
        let bank = int(m, "soundbank_id").unwrap_or(25) as u32;
        let mut out = Music {
            bank,
            state_hold: flt(adaptive, "state_hold").unwrap_or(5.0),
            ..Default::default()
        };
        if let Some(tg) = adaptive.child("track_groups") {
            for (_, g) in &tg.children {
                let Some(id) = int_h(g, ID) else { continue };
                let mut slots = [None; 3];
                for (_, s) in &g.children {
                    let (Some(slot), Some(sound)) = (s.str("slot_id"), int(s, "sound_id")) else {
                        continue;
                    };
                    if let Ok(i) = slot.parse::<usize>()
                        && i < 3
                    {
                        slots[i] = Some(SoundRef {
                            bank,
                            sound: sound as u32,
                        });
                    }
                }
                out.groups.push(MusicGroup {
                    id: id as u32,
                    slots,
                });
            }
            out.groups.sort_by_key(|g| g.id);
        }
        if let Some(sel) = adaptive.child("track_groups_selections") {
            for (_, s) in &sel.children {
                let Some(cond) = s.str("conditions") else {
                    continue;
                };
                let Some((a, b)) = cond.split_once(':') else {
                    continue;
                };
                let (Ok(time), Ok(chaos)) = (a.parse(), b.parse()) else {
                    continue;
                };
                let mut groups: Vec<u32> = s
                    .child("track_groups")
                    .map(|t| {
                        t.values
                            .iter()
                            .filter_map(|(_, v)| as_i(v))
                            .map(|v| v as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                groups.sort();
                out.selections.push(MusicSelection {
                    time,
                    chaos,
                    groups,
                });
            }
            out.selections.sort_by_key(|s| (s.time, s.chaos));
        }
        if let Some(inputs) = adaptive.child("inputs") {
            for (_, i) in &inputs.children {
                let rules = |n: &Node| -> Vec<(u32, f32)> {
                    let mut v: Vec<(u32, f32)> = n
                        .child("translation_rules")
                        .map(|t| {
                            t.children
                                .iter()
                                .filter_map(|(_, r)| {
                                    Some((
                                        int(r, "resulting_condition")? as u32,
                                        flt(r, "lower_threshold")?,
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    v.sort_by(|a, b| a.1.total_cmp(&b.1));
                    v
                };
                match int_h(i, ID) {
                    Some(2) => out.chaos_rules = rules(i),
                    Some(0) => {
                        out.time_rules = rules(i);
                        out.time_default = int(i, "default_condition").unwrap_or(0) as u32;
                    }
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    /// The time condition (0 day, 1 night) at `hour`.
    pub fn time_condition(&self, hour: f32) -> u32 {
        self.time_rules
            .iter()
            .rev()
            .find(|(_, t)| hour >= *t)
            .map_or(self.time_default, |(c, _)| *c)
    }

    /// The chaos condition (0 … 5) at a chaos total.
    pub fn chaos_condition(&self, chaos: f32) -> u32 {
        self.chaos_rules
            .iter()
            .rev()
            .find(|(_, t)| chaos >= *t)
            .map_or(0, |(c, _)| *c)
    }

    /// The groups that may play at `hour` with this much chaos.
    pub fn groups_for(&self, hour: f32, chaos: f32) -> Option<&MusicSelection> {
        let (t, c) = (self.time_condition(hour), self.chaos_condition(chaos));
        self.selections.iter().find(|s| s.time == t && s.chaos == c)
    }

    pub fn group(&self, id: u32) -> Option<&MusicGroup> {
        self.groups.iter().find(|g| g.id == id)
    }
}

// ───────────────────────────────── effects ─────────────────────────────────

/// A sound an effect file (`effects\*.bin`) plays: for any ground, or for one material.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSound {
    /// The variant's key: the hash of a [`GameMaterial`] name, or another for "any".
    pub key: u32,
    pub sound: SoundRef,
}

/// The `Sound` component's variants of an effect file (empty when it has none).
pub fn effect_sounds(data: &[u8]) -> Result<Vec<EffectSound>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for r in &file.nodes {
        fn visit(n: &Node, in_sound: bool, out: &mut Vec<EffectSound>) {
            for (k, c) in &n.children {
                // a sound component is `type = 0` under its name; its variants have the pair
                let sound_component =
                    in_sound || (int(c, "type") == Some(0) && node_ref(c, 0x1f1d_50e6).is_some());
                if sound_component
                    && c.value("sound_id").is_some()
                    && let Some(sound) = sound_ref(c)
                {
                    out.push(EffectSound {
                        key: key_hash(k),
                        sound,
                    });
                }
                if !in_sound || c.value("sound_id").is_none() {
                    visit(c, sound_component, out);
                }
            }
        }
        visit(r, false, &mut out);
    }
    Ok(out)
}

/// The sound to play for an effect on ground material `material` (a game material
/// name hashing to a variant's key), else the variant for any ground, else the first.
pub fn pick_effect_sound(variants: &[EffectSound], material_hash: Option<u32>) -> Option<SoundRef> {
    material_hash
        .and_then(|h| variants.iter().find(|v| v.key == h))
        .or_else(|| {
            variants.iter().find(|v| {
                v.key == hash_str("Default") || v.key == hash_str("default") || v.key == 0xdef9_ae75
            })
        })
        .or(variants.first())
        .map(|v| v.sound)
}

// ───────────────────────────────── the lot ─────────────────────────────────

/// Everything: the id tables of the game.
#[derive(Clone, Debug, Default)]
pub struct SoundMap {
    pub core: Core,
    pub materials: GameMaterials,
    pub weapons: WeaponSounds,
    pub weapon_ids: Vec<WeaponSoundIds>,
    pub characters: CharacterSounds,
    pub vehicles: VehicleSounds,
    pub environment: Environment,
    pub music: Music,
}

impl SoundMap {
    pub fn load(fs: &GameFs) -> Result<Self> {
        let read = |n: &str| fs.read_inflated(n);
        Ok(Self {
            core: Core::parse(&read("settings_core.bin")?)?,
            materials: GameMaterials::parse(&read("game_materials.bin")?)?,
            weapons: WeaponSounds::parse(&read("settings_smod_sfx_weapon.bin")?)?,
            weapon_ids: WeaponSoundIds::parse_all(&read("weapons.bin")?)?,
            characters: CharacterSounds::parse(&read("settings_smod_sfx_characters.bin")?)?,
            vehicles: VehicleSounds::parse(&read("settings_smod_sfx_vehicle.bin")?)?,
            environment: Environment::parse(&read("settings_smod_sfx_environment.bin")?)?,
            music: Music::parse(&read("settings_smod_music.bin")?)?,
        })
    }

    /// The event a bank/sound pair names.
    pub fn event(&self, r: SoundRef) -> Option<EventRef> {
        self.core.event(r)
    }

    /// The weapon with this `id` (the table's, not the `weapon_N` index).
    pub fn weapon_ids(&self, id: i32) -> Option<&WeaponSoundIds> {
        self.weapon_ids.iter().find(|w| w.id == id)
    }

    pub fn weapon_ids_by_name(&self, name: &str) -> Option<&WeaponSoundIds> {
        self.weapon_ids.iter().find(|w| w.name == name)
    }

    /// The weapon sound set a weapon plays at an upgrade level.
    pub fn weapon_set(&self, w: &WeaponSoundIds, level: u32) -> Option<&WeaponSet> {
        self.weapons.sets.get(&w.set_at(level)?)
    }

    /// The material a [`GameMaterial`] hash names.
    pub fn material_of_hash(&self, h: u32) -> Option<u32> {
        self.materials.by_hash(h).map(|m| m.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fev::EventFile;

    #[test]
    fn original_vehicle_numeric_selectors() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        for (name, module_id, preset_index, sounds) in [
            ("lave.v014_jeep.ee", 0, 5, [21, 20, 22]),
            ("lave.v047_jeep_small.ee", 0, 8, [36, 35, 37]),
            ("lave.v024_sportscar.ee", 0, 18, [79, 78, 80]),
            ("seve.v082_motorboat_standard_small.ee", 2, 4, [88, 87, 105]),
            ("arve.v059_civilian_helicopter.ee", 3, 3, [99, 70, 100]),
            ("arve.v010_personal_propellerplane.ee", 3, 4, [85, 84, 86]),
            ("arve.v009_military_helicopter.ee", 3, 2, [42, 41, 43]),
            ("arve.v071_jet_fighter.ee", 3, 1, [101, 18, 102]),
        ] {
            let selection = VehicleSoundSelection::load(&fs, name).unwrap();
            let pair = ModulePreset {
                module_id,
                preset_index,
            };
            assert_eq!(
                selection.slots[0],
                VehicleSelector::Selected(pair),
                "{name}"
            );
            let preset = map.vehicles.numbered(pair).unwrap();
            assert_eq!(
                [preset.key_on, preset.sustain, preset.key_off],
                sounds.map(|sound| Some(SoundRef { bank: 5, sound })),
                "{name}"
            );
            // Retail may retain the third preset property without its module;
            // the native missing-module default skips that orphan selector.
            assert!(selection.raw_fields[2].0.is_none());
            assert!(matches!(
                selection.slots[2],
                VehicleSelector::Absent | VehicleSelector::Invalid
            ));
        }
        let disabled = VehicleSoundSelection::load(&fs, "arve.v062_cutscene.ee").unwrap();
        assert_eq!(disabled.slots[0], VehicleSelector::Disabled);
        assert_eq!(
            disabled.raw_fields[0],
            (Some(Variant::Int(-1)), Some(Variant::Int(-1)))
        );
        let a = map
            .vehicles
            .numbered(ModulePreset {
                module_id: 0,
                preset_index: 2,
            })
            .unwrap();
        let b = map
            .vehicles
            .numbered(ModulePreset {
                module_id: 0,
                preset_index: 19,
            })
            .unwrap();
        assert_eq!(a.name, b.name);
        assert_ne!(a.key, b.key);
        assert_ne!(a.secondary, b.secondary);
        assert!(
            map.vehicles
                .numbered(ModulePreset {
                    module_id: 99,
                    preset_index: 0
                })
                .is_none()
        );
        assert!(
            map.vehicles
                .numbered(ModulePreset {
                    module_id: 0,
                    preset_index: 100
                })
                .is_none()
        );
    }

    #[test]
    fn selector_absence_disable_and_malformed_are_distinct() {
        let (m, p) = VEHICLE_SOUND_SELECTORS[0];
        let mut node = Node::default();
        assert_eq!(
            VehicleSoundSelection::from_node(&node).slots[0],
            VehicleSelector::Absent
        );
        for (module, preset, expected) in [
            (
                Variant::Int(-1),
                Variant::Int(-1),
                VehicleSelector::Disabled,
            ),
            (Variant::Int(0), Variant::Int(-1), VehicleSelector::Disabled),
            (
                Variant::Int(0),
                Variant::Float(5.0),
                VehicleSelector::Invalid,
            ),
            (Variant::Int(0), Variant::Int(100), VehicleSelector::Invalid),
            (
                Variant::Int(0),
                Variant::Int(5),
                VehicleSelector::Selected(ModulePreset {
                    module_id: 0,
                    preset_index: 5,
                }),
            ),
        ] {
            node.values = vec![(Key::Hash(m), module), (Key::Hash(p), preset)];
            assert_eq!(VehicleSoundSelection::from_node(&node).slots[0], expected);
        }
        node.values.push((Key::Hash(m), Variant::Int(0)));
        assert_eq!(
            VehicleSoundSelection::from_node(&node).slots[0],
            VehicleSelector::Invalid
        );
        node.values = vec![(Key::Hash(p), Variant::Int(5))];
        assert_eq!(
            VehicleSoundSelection::from_node(&node).slots[0],
            VehicleSelector::Invalid
        );
        assert!(VehicleSoundSelection::parse(&[]).is_err());
        let (m1, p1) = VEHICLE_SOUND_SELECTORS[1];
        let mut repeated = Node {
            values: vec![
                (Key::Hash(m), Variant::Int(0)),
                (Key::Hash(p), Variant::Int(2)),
                (Key::Hash(m1), Variant::Int(0)),
                (Key::Hash(p1), Variant::Int(19)),
            ],
            ..Default::default()
        };
        assert_eq!(
            VehicleSoundSelection::from_node(&repeated).engine_selector(),
            VehicleSelector::Selected(ModulePreset {
                module_id: 0,
                preset_index: 19
            })
        );
        repeated.values[3].1 = Variant::Int(-1);
        assert_eq!(
            VehicleSoundSelection::from_node(&repeated).engine_selector(),
            VehicleSelector::Selected(ModulePreset {
                module_id: 0,
                preset_index: 2
            })
        );
        repeated.values[2].1 = Variant::Int(3);
        repeated.values[3].1 = Variant::Int(1);
        assert_eq!(
            VehicleSoundSelection::from_node(&repeated).engine_selector(),
            VehicleSelector::Invalid
        );
        assert!(
            VehicleSoundSelection::parse(
                &PropertyFile {
                    nodes: vec![node.clone(), node]
                }
                .to_bytes()
            )
            .is_err()
        );
    }

    fn test_fs() -> Option<GameFs> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        fs.contains("settings_core.bin").then_some(fs)
    }

    fn name(map: &SoundMap, fs: &GameFs, r: SoundRef) -> String {
        let e = map
            .event(r)
            .unwrap_or_else(|| panic!("{r:?} names no event"));
        let f = EventFile::parse(&fs.read(&e.file).unwrap()).unwrap();
        let ev = f
            .event_in_group(e.group, e.index)
            .unwrap_or_else(|| panic!("{r:?}: no event {e:?}"));
        format!("{}/{}", f.project, ev.path)
    }

    /// The ids of a few known sounds resolve to the events the designers meant.
    #[test]
    fn known_ids_resolve() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        // a bank is a group of a project
        let b = &map.core.banks[&1];
        assert_eq!((b.name.as_str(), b.project, b.group), ("single", 0, 1));
        assert_eq!(map.core.projects[&0], "weapons.fev");
        assert_eq!(map.core.banks[&11].name, "explosions");
        // the Beretta: sound_fire_id 6 → the pistol's set → Pistol_Shot
        let beretta = map.weapon_ids_by_name("Beretta Handgun").unwrap();
        assert_eq!(beretta.fire, 6);
        let set = map.weapon_set(beretta, 0).unwrap();
        assert_eq!(set.name(), Some("pistol_fire"));
        let shot = set.fire().unwrap();
        assert_eq!(name(&map, &fs, shot.sound), "Weapons/Single/Pistol_Shot");
        assert!(!set.is_automatic());
        // its reload and its tail
        assert_eq!(
            name(&map, &fs, set.layer("fire_reload").unwrap().sound),
            "Weapons/Handling/PistolReload"
        );
        assert_eq!(
            name(&map, &fs, set.tail().unwrap().sound),
            "Weapons/Acoustics/SlapBack_OpenField_FarOff"
        );
        // the assault rifle speeds up with its upgrades: AR_LowRate, _MidRate, _HighRate
        let ar = map.weapon_ids_by_name("Assault").unwrap();
        let rate = |lv| {
            let s = map.weapon_set(ar, lv).unwrap();
            assert!(s.is_automatic());
            name(&map, &fs, s.fire().unwrap().sound)
        };
        assert_eq!(rate(0), "Weapons/Auto/AR_LowRate");
        assert_eq!(rate(2), "Weapons/Auto/AR_MidRate");
        assert_eq!(rate(3), "Weapons/Auto/AR_MidRate");
        assert_eq!(rate(4), "Weapons/Auto/AR_HighRate");
        // the engines: the SmJeep preset is the Jeep, the Corvette the Corvette
        let lave = map.vehicles.module("lave_engine").unwrap();
        let jeep = lave.preset("SmJeep").unwrap();
        assert_eq!(
            name(&map, &fs, jeep.sustain.unwrap()),
            "Vehicles/Engines/Jeep"
        );
        assert_eq!(
            name(&map, &fs, jeep.key_on.unwrap()),
            "Vehicles/Engines/Jeep startup"
        );
        assert_eq!(
            name(&map, &fs, jeep.key_off.unwrap()),
            "Vehicles/Engines/Jeep engstop"
        );
        let vette = lave.preset("Corvette").unwrap();
        assert_eq!(
            name(&map, &fs, vette.sustain.unwrap()),
            "Vehicles/Engines/Corvette"
        );
        let heli = map
            .vehicles
            .engine("arve_engine", "heli_transport")
            .unwrap();
        assert_eq!(
            name(&map, &fs, heli.sustain.unwrap()),
            "Vehicles/Engines/Transport Helicopter"
        );
    }

    #[test]
    fn vehicle_secondary_effects_keep_roles_and_optional_doors() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        let effects = map.vehicles.module("lave_effects").unwrap();
        for (preset, role, sound, path) in [
            ("medium_vehicle", "horn", 0, "Vehicles/FX/Horn medium"),
            ("big_vehicle", "horn", 12, "Vehicles/FX/Horn big"),
            ("small_vehicle", "horn", 13, "Vehicles/FX/Horn small"),
            ("truck", "horn", 14, "Vehicles/FX/Horn truck 1"),
            ("truck2", "horn", 15, "Vehicles/FX/Horn truck 2"),
            (
                "small_vehicle",
                "door_open",
                3,
                "Vehicles/FX/Door Small Open",
            ),
            (
                "small_vehicle",
                "door_close",
                4,
                "Vehicles/FX/Door Small Close",
            ),
        ] {
            let p = effects.preset(preset).unwrap();
            let r = p.secondary.iter().find(|(n, _)| n == role).unwrap().1;
            assert_eq!(r, SoundRef { bank: 6, sound });
            assert_eq!(name(&map, &fs, r), path);
        }
        let motorcycle = effects.preset("motorcycle").unwrap();
        assert_eq!(motorcycle.secondary.len(), 1);
        assert_eq!(motorcycle.secondary[0].0, "horn");
        assert!(
            map.vehicles
                .module("seve_effects")
                .unwrap()
                .presets
                .is_empty()
        );
    }

    /// Footsteps: the character module's `footstep` event has a sound per ground
    /// material, and the terrain's own materials lead to them.
    #[test]
    fn footsteps_by_material() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        let mats = &map.materials;
        assert_eq!(mats.list.len(), 35);
        assert_eq!(mats.get(0).unwrap().name, "Sand");
        assert_eq!(mats.get(30).unwrap().name, "Asphalt");
        assert_eq!(mats.get(34).unwrap().name, "Grass_Field");
        let step = |name: &str| {
            let m = mats.by_name(name).unwrap().id;
            self::name(
                &map,
                &fs,
                map.characters.sound("footstep", Some(m)).unwrap(),
            )
        };
        assert_eq!(step("Soil"), "Character/Foley/Footsteps_Dirt");
        assert_eq!(step("Sand"), "Character/Foley/Footsteps_Sand_Dry");
        assert_eq!(step("Snow"), "Character/Foley/Footsteps_Snow");
        assert_eq!(step("Gravel"), "Character/Foley/Footsteps_Gravel");
        assert_eq!(step("Metal"), "Character/Foley/Footsteps_Metal");
        assert_eq!(step("Wood_Solid"), "Character/Foley/Footsteps_Wood_Solid");
        assert_eq!(step("Asphalt"), "Character/Foley/Footsteps_Stone");
        // a material the table does not list falls back to the default (dirt)
        let default = map.characters.sound("footstep", None).unwrap();
        assert_eq!(name(&map, &fs, default), "Character/Foley/Footsteps_Dirt");
        // landing and the car roof
        assert!(map.characters.events.contains_key("jump_land"));
        assert_eq!(map.characters.events["footstep_car"].material, Some(21));
    }

    /// Every id pair in the settings and in a sample of effects names an event that exists.
    #[test]
    fn every_pair_resolves() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        let mut files: BTreeMap<String, EventFile> = BTreeMap::new();
        let mut check = |r: SoundRef, what: &str| {
            let e = map
                .event(r)
                .unwrap_or_else(|| panic!("{what}: {r:?} names no event"));
            let f = files
                .entry(e.file.clone())
                .or_insert_with(|| EventFile::parse(&fs.read(&e.file).unwrap()).unwrap());
            assert!(
                f.event_in_group(e.group, e.index).is_some(),
                "{what}: {r:?} → {e:?}"
            );
        };
        for s in map.weapons.sets.values() {
            for l in &s.layers {
                check(l.sound, "weapon set");
            }
        }
        for s in map.characters.sets.values() {
            for r in s.default.iter().chain(s.by_material.values()) {
                check(*r, "character set");
            }
        }
        for m in &map.vehicles.modules {
            for p in &m.presets {
                for r in p.key_on.iter().chain(&p.sustain).chain(&p.key_off) {
                    check(*r, &m.name);
                }
                for (n, r) in &p.secondary {
                    check(*r, n);
                }
            }
        }
        for z in &map.environment.zone_sets {
            check(z.bed, &z.name);
            for r in z.detail_looping.iter().chain(&z.sprinkle) {
                check(*r, &z.name);
            }
        }
        for r in map.environment.rain.values() {
            check(*r, "rain");
        }
        assert_eq!(map.music.groups.len(), 14);
        for g in &map.music.groups {
            for r in g.slots.iter().flatten() {
                check(*r, "music");
            }
        }
        // every effect file
        let mut n = 0;
        for e in fs.entries() {
            let Some(name) = fs
                .names
                .file(e.name_hash)
                .filter(|n| n.starts_with("effects\\") && n.ends_with(".bin"))
            else {
                continue;
            };
            let Ok(data) = fs.read_inflated(name) else {
                continue;
            };
            let Ok(vs) = effect_sounds(&data) else {
                continue;
            };
            for v in vs {
                check(v.sound, name);
                n += 1;
            }
        }
        assert!(n > 100, "{n} effect sounds");
    }

    /// Weapon ids: each automatic's set has a loop, and the speed-ups exist.
    #[test]
    fn weapon_sound_ids() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        assert!(map.weapon_ids.len() >= 45);
        let mg = map.weapon_ids_by_name("MachineGun").unwrap();
        assert_eq!(mg.upgrades, vec![(2, 20), (4, 21)]);
        assert_eq!(mg.set_at(0), Some(17));
        assert_eq!(mg.set_at(1), Some(17));
        assert_eq!(mg.set_at(2), Some(20));
        assert_eq!(mg.set_at(9), Some(21));
        assert!(map.weapon_set(mg, 4).unwrap().is_automatic());
    }

    /// The adaptive music: 14 groups of three, picked by the hour and the chaos.
    #[test]
    fn music_selection() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        let m = &map.music;
        assert_eq!(m.time_condition(12.0), 0);
        assert_eq!(m.time_condition(22.0), 1);
        assert_eq!(m.time_condition(3.0), 1);
        assert_eq!(m.chaos_condition(0.0), 0);
        assert_eq!(m.chaos_condition(120_000.0), 2);
        assert_eq!(m.chaos_condition(500_000.0), 5);
        let day = m.groups_for(12.0, 0.0).unwrap();
        let night = m.groups_for(23.0, 0.0).unwrap();
        assert!(!day.groups.contains(&1) && night.groups.contains(&1));
        let g = m.group(day.groups[0]).unwrap();
        let names: Vec<String> = g
            .slots
            .iter()
            .map(|s| name(&map, &fs, s.unwrap()))
            .collect();
        assert!(names[0].ends_with("_Undetected"), "{names:?}");
        assert!(names[1].ends_with("_Combat"), "{names:?}");
        assert!(names[2].ends_with("_Evasion"), "{names:?}");
    }
}
