//! Effect descriptions: the registry `effects/effectsettings.bin` and the per-effect files
//! `effects/<name>.bin` (research: `research/formats/effects-pipeline.md`, sections 1 and 3;
//! native parser 0x007f6070, registry loader 0x0073d250, interaction matrix 0x00746c00,
//! resolve 0x00671bf0).
//!
//! * [`EffectRegistry`]: the cap on live effects (`#6a32433a` in `settings`, 255) and the effect
//!   list (`#4cbb1cc1`): per entry the `filename` and the id (`#6a9dfd76`). Ids are dense
//!   `0..468` in retail; 8 entries name `.xml` files that are not in the install (their slots are
//!   never parsed, so `Play` fails for them).
//! * [`EffectDescription::parse`]: `settings`, the typed components (`#f6a9f638`, each with a
//!   numeric `uid`, a `type` 0..10 and numbered variations `#1f1d50e6`), the interaction matrix
//!   (`#84085477`) and the optional float `inputs`. Variation bodies stay raw property nodes
//!   (unknown keys preserved); the game-side adapters read the fields they know.
//!
//! The module has no per-effect content: callers pass an id and read the registry.

use crate::error::{Error, Result};
use crate::hash::hash_str;
use crate::property::{Node, PropertyFile, Variant};
use crate::vfs::GameFs;
use std::collections::BTreeMap;

/// Registry root `#ee927e3d`.
pub const REGISTRY_ROOT: u32 = 0xee92_7e3d;
const KEY_LIVE_CAP: u32 = 0x6a32_433a;
/// Registry effect list.
const KEY_EFFECT_LIST: u32 = 0x4cbb_1cc1;
/// The effect id / component uid / variation uid / surface id key.
pub const KEY_UID: u32 = 0x6a9d_fd76;
/// Effect root `#e62de7fa`.
pub const EFFECT_ROOT: u32 = 0xe62d_e7fa;
const KEY_MAX_INSTANCES: u32 = 0x488c_291b;
const KEY_TAIL: u32 = 0x50be_431d;
const KEY_MODE: u32 = 0x553b_d311;
const KEY_MODE_FLOAT_23: u32 = 0x1cdc_8b90;
const KEY_MODE_FLOAT_1: u32 = 0x8bd4_af70;
const KEY_COMPONENTS: u32 = 0xf6a9_f638;
const KEY_VARIATIONS: u32 = 0x1f1d_50e6;
const KEY_MATRIX: u32 = 0x8408_5477;
const KEY_MATRIX_DEFAULT: u32 = 0xa28d_f775;
const KEY_MATRIX_FLAGGED: u32 = 0x55ba_2d5a;
const KEY_MATERIALS: u32 = 0xea0e_1560;

/// Component `type` values (file value; the native `CECD_*` class of each is in the doc, 1.3).
pub mod kind {
    pub const SOUND: u32 = 0;
    pub const PARTICLE: u32 = 1;
    pub const POST_EFFECT: u32 = 2;
    pub const DECAL: u32 = 3;
    pub const FORCE_PULSE: u32 = 4;
    pub const HAPTICS: u32 = 5;
    pub const CUTSCENE: u32 = 6;
    pub const LIGHT: u32 = 7;
    pub const CAMERA_SHAKE: u32 = 8;
    pub const SOUND_POST: u32 = 9;
    pub const AREA_DAMAGE: u32 = 10;
}

/// One registry entry.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectEntry {
    /// `#6a9dfd76`: the index into the native slot table.
    pub id: u32,
    /// The file name without directory and extension (`vehicle_collision_light`).
    pub name: String,
    /// The registry's `filename` as written.
    pub filename: String,
}

impl EffectEntry {
    /// The name to open in the file system: lower-case, as the engine VFS is case-insensitive
    /// while `jc2tool`/`GameFs` hash the exact spelling.
    pub fn file(&self) -> String {
        crate::hash::file_name(&self.filename).to_ascii_lowercase()
    }

    /// The 8 retail entries name `.xml` files that do not exist in the install (cutscene-only).
    pub fn is_xml(&self) -> bool {
        self.filename.to_ascii_lowercase().ends_with(".xml")
    }
}

/// `effects/effectsettings.bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRegistry {
    /// Cap on simultaneously live effects (native: low 31 bits of `CEffectSystem+0xc`).
    pub live_cap: u32,
    /// Sorted by id.
    pub entries: Vec<EffectEntry>,
}

fn effect_root(file: &PropertyFile, hash: u32) -> Option<&Node> {
    for n in &file.nodes {
        if let Some((_, c)) = n.children.iter().find(|(k, _)| k.hash() == hash) {
            return Some(c);
        }
    }
    None
}

fn int_of(v: &Variant) -> Option<i64> {
    match v {
        Variant::Int(i) => Some(i64::from(*i)),
        Variant::Float(f) => Some(*f as i64),
        _ => None,
    }
}

fn float_of(v: &Variant) -> Option<f32> {
    match v {
        Variant::Float(f) => Some(*f),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn uint(n: &Node, hash: u32) -> Option<u32> {
    n.value_hash(hash).and_then(int_of).and_then(|v| u32::try_from(v).ok())
}

impl EffectRegistry {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = effect_root(&file, REGISTRY_ROOT)
            .ok_or_else(|| Error::Format("effectsettings: no root #ee927e3d".into()))?;
        let live_cap = root
            .child("settings")
            .and_then(|s| uint(s, KEY_LIVE_CAP))
            .ok_or_else(|| Error::Format("effectsettings: no live cap #6a32433a".into()))?;
        let list = root
            .children
            .iter()
            .find(|(k, _)| k.hash() == KEY_EFFECT_LIST)
            .map(|(_, n)| n)
            .ok_or_else(|| Error::Format("effectsettings: no effect list #4cbb1cc1".into()))?;
        let mut entries = Vec::new();
        for (_, e) in &list.children {
            let (Some(id), Some(filename)) = (uint(e, KEY_UID), e.str("filename")) else {
                continue;
            };
            let base = crate::hash::file_name(filename);
            let name = base.rsplit_once('.').map_or(base, |(s, _)| s).to_string();
            entries.push(EffectEntry {
                id,
                name,
                filename: filename.to_string(),
            });
        }
        entries.sort_by_key(|e| e.id);
        Ok(Self { live_cap, entries })
    }

    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated("effectsettings.bin")?)
    }

    pub fn get(&self, id: u32) -> Option<&EffectEntry> {
        let e = self.entries.get(id as usize).filter(|e| e.id == id);
        e.or_else(|| self.entries.iter().find(|e| e.id == id))
    }

    /// Slot count (the native `CEffectSystem+0x14`): one past the highest id.
    pub fn slot_count(&self) -> u32 {
        self.entries.last().map_or(0, |e| e.id + 1)
    }

    /// Read and parse the description of `id`; `None` when the id is unknown or its file is not in
    /// the install (the 8 `.xml` names): native `Play` returns 0 for those.
    pub fn load_description(&self, fs: &GameFs, id: u32) -> Option<EffectDescription> {
        let entry = self.get(id)?;
        if entry.is_xml() {
            return None;
        }
        let bytes = fs.read_inflated(&entry.file()).ok()?;
        EffectDescription::parse(id, &bytes).ok()
    }
}

/// `settings` of an effect file (native `CEffectDescription` fields, doc 1.2).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSettings {
    /// `priority` (0x43676e89): 0 in every retail file; no native reader found.
    pub priority: i32,
    /// `#488c291b`: no native reader found (doc gap 1).
    pub max_instances: u32,
    /// `#553bd311` (1..3), or 1 when `#8bd4af70` > 0, else 0: bits 27..30 of `desc+0xc`; no reader found.
    pub throttle_mode: u8,
    /// `#1cdc8b90` (mode 2/3) or `#8bd4af70` (mode 1): `desc+0x14`; no reader found.
    pub throttle_float: f32,
    /// `looping` (bit 31 of `desc+0xc`): the effect ends only through Stop.
    pub looping: bool,
    /// `#50be431d`: seconds the state machine waits after the last component dies (and in Stop).
    pub tail_time: f32,
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            priority: 0,
            max_instances: 0,
            throttle_mode: 0,
            throttle_float: 0.0,
            looping: false,
            tail_time: 0.0,
        }
    }
}

/// One numbered variation of a component: the raw property node (all keys preserved).
#[derive(Clone, Debug, PartialEq)]
pub struct Variation {
    pub uid: u32,
    pub node: Node,
}

/// One typed component with its variations, keyed by its `uid` in [`EffectDescription`].
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentDesc {
    pub uid: u32,
    /// The hash of the role name the file uses for the component (`Sound`, `#54236b22`, ...).
    pub role: u32,
    /// `type` (see [`kind`]); 0..10 natively.
    pub kind: u32,
    pub variations: BTreeMap<u32, Variation>,
}

/// Role key hash -> `(component uid, variation uid)`; `None` is the file's `"-1"` (no instance).
pub type PairMap = BTreeMap<u32, Option<(u32, u32)>>;

/// An interaction entry passed with a Play: kind 0 uid != 0 is the "flagged" (water) matrix,
/// kind 1 a surface id (doc 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interaction {
    pub kind: u32,
    pub uid: u32,
}

impl Interaction {
    /// The water flag the bullet impacts pass: `(0, 1)`.
    pub const WATER: Interaction = Interaction { kind: 0, uid: 1 };

    pub fn surface(id: u32) -> Self {
        Interaction { kind: 1, uid: id }
    }
}

/// `interaction_matrix` (0x84085477).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractionMatrix {
    /// `#a28df775`.
    pub default: PairMap,
    /// `#55ba2d5a`; absent in 425 of 460 retail files.
    pub flagged: Option<PairMap>,
    /// `material_interactions` (0xea0e1560): surface id -> override map.
    pub materials: BTreeMap<u32, PairMap>,
}

fn parse_pair(s: &str) -> Option<(u32, u32)> {
    let (c, v) = s.split_once(':')?;
    Some((c.trim().parse().ok()?, v.trim().parse().ok()?))
}

fn pair_map(n: &Node) -> PairMap {
    let mut out = PairMap::new();
    for (k, v) in &n.values {
        if let Variant::Str(s) = v {
            out.insert(k.hash(), parse_pair(s));
        }
    }
    out
}

impl InteractionMatrix {
    fn parse(n: &Node) -> Self {
        let mut m = Self::default();
        for (k, c) in &n.children {
            match k.hash() {
                KEY_MATRIX_DEFAULT => m.default = pair_map(c),
                KEY_MATRIX_FLAGGED => m.flagged = Some(pair_map(c)),
                KEY_MATERIALS => {
                    for (_, s) in &c.children {
                        let Some(surface) = uint(s, KEY_UID) else { continue };
                        let map = s
                            .children
                            .iter()
                            .find(|(k, _)| k.hash() == KEY_MATRIX_DEFAULT)
                            .map(|(_, n)| pair_map(n))
                            .unwrap_or_default();
                        m.materials.insert(surface, map);
                    }
                }
                _ => {}
            }
        }
        m
    }

    /// Native 0x00671bf0 (called by Begin's builder 0x0078bd40): start from the default map; for each
    /// interaction: kind 0 with uid != 0 applies the flagged map and stops; kind 1 applies the
    /// surface's override on top. Returns `component uid -> variation uid`; a component with no
    /// entry is not instantiated. The override rule (later entry wins per role key, `"-1"` removes the
    /// role) is inferred from the file shape; the native merge order is not decompiled in detail.
    pub fn resolve(&self, interactions: &[Interaction]) -> BTreeMap<u32, u32> {
        let mut roles = self.default.clone();
        for it in interactions {
            match it.kind {
                0 if it.uid != 0 => {
                    if let Some(f) = &self.flagged {
                        roles.extend(f.iter().map(|(k, v)| (*k, *v)));
                    }
                    break;
                }
                1 => {
                    if let Some(m) = self.materials.get(&it.uid) {
                        roles.extend(m.iter().map(|(k, v)| (*k, *v)));
                    }
                }
                _ => {}
            }
        }
        roles.values().flatten().map(|(c, v)| (*c, *v)).collect()
    }
}

/// One parsed effect file.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDescription {
    /// The registry id this was parsed for (native `desc+4`).
    pub id: u32,
    pub settings: EffectSettings,
    /// By component uid.
    pub components: BTreeMap<u32, ComponentDesc>,
    pub matrix: InteractionMatrix,
    /// `inputs`: key name hash -> default value.
    pub inputs: BTreeMap<u32, f32>,
}

impl EffectDescription {
    /// Mirror of 0x007f6070.
    pub fn parse(id: u32, data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let root = effect_root(&file, EFFECT_ROOT)
            .ok_or_else(|| Error::Format("effect file: no root #e62de7fa".into()))?;
        Ok(Self::from_root(id, root))
    }

    pub fn from_root(id: u32, root: &Node) -> Self {
        let mut settings = EffectSettings::default();
        if let Some(s) = root.child("settings") {
            settings.priority = s.value_hash(0x4367_6e89).and_then(int_of).unwrap_or(0) as i32;
            settings.max_instances = uint(s, KEY_MAX_INSTANCES).unwrap_or(0) & 0x07ff_ffff;
            settings.tail_time = s.value_hash(KEY_TAIL).and_then(float_of).unwrap_or(0.0);
            settings.looping = s.value("looping").and_then(int_of).unwrap_or(0) != 0;
            let mode = uint(s, KEY_MODE).filter(|m| (1..=3).contains(m));
            let f1 = s.value_hash(KEY_MODE_FLOAT_1).and_then(float_of);
            let f23 = s.value_hash(KEY_MODE_FLOAT_23).and_then(float_of);
            match mode {
                Some(m) => {
                    settings.throttle_mode = m as u8;
                    settings.throttle_float = f23.unwrap_or(0.0);
                }
                None if f1.is_some_and(|f| f > 0.0) => {
                    settings.throttle_mode = 1;
                    settings.throttle_float = f1.unwrap_or(0.0);
                }
                None => {}
            }
        }
        let mut components = BTreeMap::new();
        if let Some((_, list)) = root.children.iter().find(|(k, _)| k.hash() == KEY_COMPONENTS) {
            for (role, c) in &list.children {
                let (Some(uid), Some(kind)) = (uint(c, KEY_UID), c.int("type")) else {
                    continue;
                };
                let mut variations = BTreeMap::new();
                if let Some((_, vs)) = c.children.iter().find(|(k, _)| k.hash() == KEY_VARIATIONS) {
                    for (_, v) in &vs.children {
                        if let Some(vuid) = uint(v, KEY_UID) {
                            variations.insert(
                                vuid,
                                Variation {
                                    uid: vuid,
                                    node: v.clone(),
                                },
                            );
                        }
                    }
                }
                components.insert(
                    uid,
                    ComponentDesc {
                        uid,
                        role: role.hash(),
                        kind: kind.max(0) as u32,
                        variations,
                    },
                );
            }
        }
        let matrix = root
            .children
            .iter()
            .find(|(k, _)| k.hash() == KEY_MATRIX)
            .map(|(_, n)| InteractionMatrix::parse(n))
            .unwrap_or_default();
        let mut inputs = BTreeMap::new();
        if let Some(n) = root.child("inputs") {
            for (k, v) in &n.values {
                if let Some(f) = float_of(v) {
                    inputs.insert(k.hash(), f);
                }
            }
        }
        Self {
            id,
            settings,
            components,
            matrix,
            inputs,
        }
    }

    /// The names of every particle system the file references (type 1 variations' `name`).
    pub fn particle_names(&self) -> Vec<String> {
        self.components
            .values()
            .filter(|c| c.kind == kind::PARTICLE)
            .flat_map(|c| c.variations.values())
            .filter_map(|v| v.node.str("name").map(str::to_string))
            .collect()
    }

    /// The `(component uid, variation)` list an interaction list instantiates, in component uid order.
    pub fn instances(&self, interactions: &[Interaction]) -> Vec<(&ComponentDesc, &Variation)> {
        self.matrix
            .resolve(interactions)
            .into_iter()
            .filter_map(|(c, v)| {
                let comp = self.components.get(&c)?;
                Some((comp, comp.variations.get(&v)?))
            })
            .collect()
    }
}

/// Convert a [`Key`] role hash to a printable label (diagnostics).
pub fn role_label(role: u32) -> String {
    for n in ["Sound", "Decal", "SoundPostEffect"] {
        if hash_str(n) == role {
            return n.to_string();
        }
    }
    format!("#{role:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Key;

    fn node(values: Vec<(u32, Variant)>, children: Vec<(u32, Node)>) -> Node {
        Node {
            tag: None,
            children: children.into_iter().map(|(k, n)| (Key::Hash(k), n)).collect(),
            values: values.into_iter().map(|(k, v)| (Key::Hash(k), v)).collect(),
        }
    }

    #[test]
    fn hashes_of_the_named_keys() {
        assert_eq!(hash_str("effects"), KEY_EFFECT_LIST);
    }

    #[test]
    fn resolve_follows_the_documented_rules() {
        let mut m = InteractionMatrix::default();
        m.default.insert(1, Some((0, 1)));
        m.default.insert(2, Some((1, 1)));
        m.default.insert(3, Some((7, 1)));
        m.flagged = Some(PairMap::from([(2, Some((1, 3)))]));
        m.materials.insert(14, PairMap::from([(2, None), (1, Some((0, 2)))]));
        m.materials.insert(8, PairMap::from([(2, Some((1, 2)))]));
        let r = |i: &[Interaction]| m.resolve(i);
        assert_eq!(r(&[]), BTreeMap::from([(0, 1), (1, 1), (7, 1)]));
        // water flag: the flagged map overrides, a later surface is not applied (stops)
        assert_eq!(
            r(&[Interaction::WATER, Interaction::surface(8)]),
            BTreeMap::from([(0, 1), (1, 3), (7, 1)])
        );
        // flesh: particle role removed, sound variation 2
        assert_eq!(r(&[Interaction::surface(14)]), BTreeMap::from([(0, 2), (7, 1)]));
        // metal: particle variation 2
        assert_eq!(
            r(&[Interaction::surface(8)]),
            BTreeMap::from([(0, 1), (1, 2), (7, 1)])
        );
        // kind 0 uid 0 and unknown surfaces change nothing
        assert_eq!(r(&[Interaction { kind: 0, uid: 0 }, Interaction::surface(99)]), r(&[]));
    }

    #[test]
    fn parses_a_synthetic_file() {
        let var = |uid: i32, name: &str| {
            node(
                vec![(KEY_UID, Variant::Int(uid)), (hash_str("name"), Variant::Str(name.into()))],
                vec![],
            )
        };
        let comp = node(
            vec![(KEY_UID, Variant::Int(0)), (hash_str("type"), Variant::Int(1))],
            vec![(KEY_VARIATIONS, node(vec![], vec![(1, var(1, "a")), (2, var(2, "b"))]))],
        );
        let root = node(
            vec![],
            vec![
                (
                    hash_str("settings"),
                    node(
                        vec![
                            (KEY_MAX_INSTANCES, Variant::Int(10)),
                            (KEY_TAIL, Variant::Float(0.5)),
                            (hash_str("looping"), Variant::Int(1)),
                            (KEY_MODE_FLOAT_1, Variant::Float(2.0)),
                        ],
                        vec![],
                    ),
                ),
                (KEY_COMPONENTS, node(vec![], vec![(77, comp)])),
                (
                    KEY_MATRIX,
                    node(
                        vec![],
                        vec![(
                            KEY_MATRIX_DEFAULT,
                            node(
                                vec![(77, Variant::Str("0:2".into())), (78, Variant::Str("-1".into()))],
                                vec![],
                            ),
                        )],
                    ),
                ),
                (hash_str("inputs"), node(vec![(hash_str("impulse"), Variant::Float(0.0))], vec![])),
            ],
        );
        let d = EffectDescription::from_root(5, &root);
        assert!(d.settings.looping);
        assert_eq!((d.settings.max_instances, d.settings.tail_time), (10, 0.5));
        assert_eq!((d.settings.throttle_mode, d.settings.throttle_float), (1, 2.0));
        assert_eq!(d.components[&0].variations.len(), 2);
        assert_eq!(d.particle_names(), vec!["a".to_string(), "b".to_string()]);
        let inst = d.instances(&[]);
        assert_eq!(inst.len(), 1);
        assert_eq!(inst[0].1.uid, 2);
        assert_eq!(d.inputs.len(), 1);
    }

    fn game() -> Option<GameFs> {
        GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()
    }

    /// Retail files (needs JC2_TEST_DIR): the claims of the research probe (56 checks).
    #[test]
    fn retail_registry_and_files() {
        let Some(fs) = game() else { return };
        let reg = EffectRegistry::load(&fs).unwrap();
        assert_eq!(reg.live_cap, 255);
        assert_eq!(reg.entries.len(), 468);
        assert!(reg.entries.iter().enumerate().all(|(i, e)| e.id as usize == i));
        assert_eq!(reg.slot_count(), 468);
        for (id, name) in [
            (425, "vehicle_collision_light"),
            (449, "vehicle_collision_medium"),
            (450, "vehicle_collision_heavy"),
            (448, "fx_char_parachute_leafs_01"),
            (370, "char_water_splash"),
        ] {
            assert_eq!(reg.get(id).unwrap().name.to_lowercase(), name);
        }
        assert_eq!(reg.entries.iter().filter(|e| e.is_xml()).count(), 8);
        let mut parsed = 0;
        let mut kinds = BTreeMap::<u32, usize>::new();
        let mut max_inst = BTreeMap::<u32, usize>::new();
        let (mut looping, mut pairs, mut bad, mut flagged) = (0, 0, Vec::new(), 0);
        let mut surfaces = std::collections::BTreeSet::new();
        for e in &reg.entries {
            let d = reg.load_description(&fs, e.id);
            if e.is_xml() {
                assert!(d.is_none(), "{}", e.name);
                continue;
            }
            let d = d.unwrap_or_else(|| panic!("{} did not parse", e.filename));
            parsed += 1;
            looping += usize::from(d.settings.looping);
            *max_inst.entry(d.settings.max_instances).or_default() += 1;
            flagged += usize::from(d.matrix.flagged.is_some());
            for c in d.components.values() {
                *kinds.entry(c.kind).or_default() += 1;
            }
            surfaces.extend(d.matrix.materials.keys().copied());
            let maps = std::iter::once(&d.matrix.default)
                .chain(d.matrix.flagged.iter())
                .chain(d.matrix.materials.values());
            for m in maps {
                for p in m.values() {
                    pairs += 1;
                    if let Some((c, v)) = p
                        && !d.components.get(c).is_some_and(|c| c.variations.contains_key(v))
                    {
                        bad.push(e.name.clone());
                    }
                }
            }
        }
        assert_eq!(parsed, 460);
        assert_eq!(looping, 89);
        assert_eq!(flagged, 35);
        assert_eq!(pairs, 2169);
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].eq_ignore_ascii_case("hook_detach"));
        assert_eq!(
            kinds,
            BTreeMap::from([(0, 339), (1, 412), (3, 20), (4, 37), (5, 10), (7, 129), (8, 11), (9, 138)])
        );
        assert_eq!(
            max_inst,
            BTreeMap::from([(1, 19), (2, 9), (3, 3), (4, 11), (5, 3), (8, 1), (10, 381), (16, 30), (20, 2), (50, 1)])
        );
        assert_eq!(surfaces.into_iter().collect::<Vec<_>>(), (0..35).collect::<Vec<_>>());
    }
}
