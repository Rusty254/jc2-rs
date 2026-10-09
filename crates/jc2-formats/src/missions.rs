//! Missions, strongholds and colonels (`research/formats/missions.md`).
//!
//! The mission system is data with a thin layer of code in the exe:
//!
//! * `missions\missions.bmf` — the mission table: 441 records of 60 bytes (`id`, the
//!   mission or stronghold it `requires`, `hashlittle(id)`, flags, the number of
//!   objectives, the kind, the chaos that unlocks it, the faction). [`parse_bmf`].
//! * `missions\regions.csv` — each stronghold's areas of influence and the chaos that
//!   opens them (`f1s07_r2;60000`). [`parse_regions`].
//! * `missions\missiondata.bin` — what a completed mission pays by kind (`rewards ✓ {
//!   key_missions ✓ { money, chaos, weapon_parts ✓, vehicle_parts ✓, armor_parts ✓ } … }`)
//!   and the banner texts. [`MissionData`].
//! * `rules\missionrules.brf` — a rule system (its own binary tree format, [`parse_brf`]):
//!   `f2t10Obj01.COMPLETED AND f2t10Obj01.COMPLETED ⇒ f2t10Obj02.ACTIVE`; the objectives of
//!   a mission come out in order with [`objective_chain`].
//! * `missions\<kind>\<id>\<id>.blz` → `<id>.bl.bl` — the mission's script: world objects
//!   (`CEventTrigger`, `CTimeTrigger`, `CCountTrigger`, `CShapeTrigger`, `CMessageObject`,
//!   `CTargetIndicator`, `CLocalGuiMapIcon`, `CDialogue`, `CSpawnSet` …) wired together by
//!   event strings, talking to the mission system with `msy.<objective>.complete` and
//!   hearing `msy.<objective>.activated`. [`script_objects`].
//! * Strongholds are settlements of kind 32 whose `icons_global.bin` entry names the
//!   takeover mission, the event their targets fire and how many there are
//!   ([`strongholds`]).
//! * Colonels are `CSpawnSet`s with an agent whose role is `general` in the location
//!   files ([`colonels`]).

use crate::hash::hash_str;
use crate::placement::Transform3;
use crate::property::{Key, Node, PropertyFile, Variant};
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------------
// missions.bmf
// ---------------------------------------------------------------------------------

/// Bytes per `missions.bmf` record.
pub const BMF_RECORD: usize = 60;

/// What a mission is: `missions.bmf`'s kind field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionKind {
    /// 3: Agency ("key") missions, `km01` … `km07`.
    Agency,
    /// 4: faction missions, `f1m03`.
    FactionMission,
    /// 5: faction tasks, `f2t10`.
    FactionTask,
    /// 6: stronghold takeovers, `f1s07`.
    Stronghold,
    /// 8: skulls (`csk`), 9: black boxes (`cbb`), 10: drug drops (`cdd`).
    Skulls,
    BlackBoxes,
    DrugDrops,
    /// 18: race challenges (`cch`).
    Challenge,
    Other(u32),
}

impl MissionKind {
    pub fn from_code(c: u32) -> Self {
        match c {
            3 => Self::Agency,
            4 => Self::FactionMission,
            5 => Self::FactionTask,
            6 => Self::Stronghold,
            8 => Self::Skulls,
            9 => Self::BlackBoxes,
            10 => Self::DrugDrops,
            18 => Self::Challenge,
            o => Self::Other(o),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Agency => "agency mission",
            Self::FactionMission => "faction mission",
            Self::FactionTask => "faction task",
            Self::Stronghold => "stronghold takeover",
            Self::Skulls => "skull",
            Self::BlackBoxes => "black box",
            Self::DrugDrops => "drug drop",
            Self::Challenge => "race challenge",
            Self::Other(_) => "other",
        }
    }

    /// The node of `missiondata.bin`'s `rewards` that pays for it.
    pub fn reward_key(&self) -> Option<&'static str> {
        Some(match self {
            Self::Agency => "key_missions",
            Self::FactionMission => "faction_missions",
            Self::FactionTask => "faction_tasks",
            Self::Stronghold => "stronghold_takeovers",
            Self::Skulls => "collect_skulls",
            Self::BlackBoxes => "collect_black_boxes",
            Self::DrugDrops => "collect_drug_drops",
            _ => return None,
        })
    }

    /// The banner's first line when it starts (text database keys: "Agency Mission
    /// Started", "Faction Mission Started", "Stronghold Takeover Started", "Challenge
    /// Started"). These keys are what `missiondata.bin`'s message list holds; its own
    /// key names are hashes nobody has named.
    pub fn started_text(&self) -> (&'static str, &'static str) {
        match self {
            Self::Agency => ("mission_msg_14", "Agency Mission Started"),
            Self::FactionMission | Self::FactionTask => {
                ("mission_msg_15", "Faction Mission Started")
            }
            Self::Stronghold => ("mission_msg_16", "Stronghold Takeover Started"),
            Self::Challenge => ("mission_msg_17", "Challenge Started"),
            _ => ("mission_msg_15", "Mission Started"),
        }
    }

    /// The banner's first line when it is done.
    pub fn completed_text(&self) -> (&'static str, &'static str) {
        match self {
            Self::Agency => ("mission_msg_08", "Agency Mission Completed"),
            Self::FactionMission | Self::FactionTask => {
                ("mission_msg_09", "Faction Mission Completed")
            }
            Self::Stronghold => ("mission_msg_11", "Stronghold Takeover Completed"),
            _ => ("mission_msg_01", "Mission Completed"),
        }
    }
}

/// Who gives a mission: `missions.bmf`'s faction field (0 … 3; the `f1`/`f2`/`f3` of the
/// ids). The text database says who they are (`f1s07_intfile_desc_txt`: the Roaches).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Faction {
    Roaches,
    UlarBoys,
    Reapers,
    Agency,
}

impl Faction {
    pub fn from_code(c: u32) -> Option<Self> {
        Some(match c {
            0 => Self::Roaches,
            1 => Self::UlarBoys,
            2 => Self::Reapers,
            3 => Self::Agency,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Roaches => "Roaches",
            Self::UlarBoys => "Ular Boys",
            Self::Reapers => "Reapers",
            Self::Agency => "Agency",
        }
    }

    /// The faction number `icon_types.bin`, `icons_global.bin` (`#7a5ce7b9`) and the
    /// `CTeamComponent`s use: 1 Reapers, 2 Roaches, 3 Ular Boys (0 the government).
    pub fn icon_faction(&self) -> i32 {
        match self {
            Self::Reapers => 1,
            Self::Roaches => 2,
            Self::UlarBoys => 3,
            Self::Agency => 0,
        }
    }

    pub fn from_icon_faction(f: i32) -> Option<Self> {
        Some(match f {
            1 => Self::Reapers,
            2 => Self::Roaches,
            3 => Self::UlarBoys,
            _ => return None,
        })
    }

    /// The faction's node in `enemytypespawnsettings.bin`'s `factions` (its dolls).
    pub fn enemies_key(&self) -> Option<&'static str> {
        match self {
            Self::Roaches => Some("roaches"),
            Self::UlarBoys => Some("ularboys"),
            Self::Reapers => Some("reapers"),
            Self::Agency => None,
        }
    }

    /// The faction's stronghold icon, plain and taken (`hud_icon_sh_roaches_dif.dds`,
    /// `…_c_dif.dds`).
    pub fn stronghold_icon(&self, taken: bool) -> Option<String> {
        let f = match self {
            Self::Roaches => "roaches",
            Self::UlarBoys => "ular",
            Self::Reapers => "reapers",
            Self::Agency => return None,
        };
        Some(format!(
            "hud_icon_sh_{f}{}_dif.dds",
            if taken { "_c" } else { "" }
        ))
    }
}

/// One record of `missions.bmf`.
///
/// ```text
/// 0x00 char[16]  id            "f2t10\0" + stack garbage
/// 0x10 char[16]  requires      the mission/stronghold done first ("f2s01"; km01 names itself)
/// 0x20 u32       hashlittle(id)
/// 0x24 u8[8]     flags         (unnamed: 1 on the collectables' first, km01 …)
/// 0x2c u32       objectives    how many objectives the rules chain (f2t10: 2)
/// 0x30 u32       kind          3 agency, 4 faction mission, 5 task, 6 stronghold, 8/9/10, 18
/// 0x34 u32       chaos         unlocked at this much chaos (the region's in regions.csv)
/// 0x38 u32       faction       0 Roaches, 1 Ular Boys, 2 Reapers, 3 Agency
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MissionRecord {
    pub id: String,
    pub requires: String,
    pub hash: u32,
    pub flags: [u8; 8],
    pub objectives: u32,
    pub kind_code: u32,
    pub kind: MissionKind,
    pub chaos: u32,
    pub faction_code: u32,
}

impl MissionRecord {
    pub fn faction(&self) -> Option<Faction> {
        Faction::from_code(self.faction_code)
    }

    /// What has to be done first (`None` for the first mission, which names itself).
    pub fn prerequisite(&self) -> Option<&str> {
        (!self.requires.is_empty() && !self.requires.eq_ignore_ascii_case(&self.id))
            .then_some(self.requires.as_str())
    }
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// `missions.bmf`: every record, in file order.
pub fn parse_bmf(data: &[u8]) -> crate::Result<Vec<MissionRecord>> {
    if !data.len().is_multiple_of(BMF_RECORD) {
        return Err(crate::Error::format(format!(
            "missions.bmf: {} bytes is not a whole number of {BMF_RECORD}-byte records",
            data.len()
        )));
    }
    Ok(data
        .as_chunks::<BMF_RECORD>()
        .0
        .iter()
        .map(|r| {
            let mut flags = [0u8; 8];
            flags.copy_from_slice(&r[0x24..0x2c]);
            let kind_code = u32_at(r, 0x30);
            MissionRecord {
                id: cstr(&r[..16]),
                requires: cstr(&r[16..32]),
                hash: u32_at(r, 0x20),
                flags,
                objectives: u32_at(r, 0x2c),
                kind_code,
                kind: MissionKind::from_code(kind_code),
                chaos: u32_at(r, 0x34),
                faction_code: u32_at(r, 0x38),
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------------
// regions.csv
// ---------------------------------------------------------------------------------

/// A stronghold's area of influence: `f1s07_r2;60000`.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub stronghold: String,
    /// 1, 2, 3.
    pub index: u32,
    /// The chaos that opens it.
    pub chaos: u32,
}

pub fn parse_regions(data: &[u8]) -> Vec<Region> {
    String::from_utf8_lossy(data)
        .lines()
        .filter_map(|l| {
            let (name, chaos) = l.trim().split_once(';')?;
            let (stronghold, r) = name.rsplit_once("_r")?;
            Some(Region {
                stronghold: stronghold.to_string(),
                index: r.parse().ok()?,
                chaos: chaos.trim().parse().ok()?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// missiondata.bin
// ---------------------------------------------------------------------------------

/// What finishing something pays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reward {
    pub money: u32,
    pub chaos: u32,
    pub weapon_parts: u32,
    pub vehicle_parts: u32,
    pub armor_parts: u32,
}

/// `missiondata.bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MissionData {
    /// `rewards.max_chaos` (809 750: the chaos of the whole game's missions?).
    pub max_chaos: u32,
    /// By name (`faction_tasks`), unnamed keys as `#hash`; nested ones as
    /// `settlement_chaos.light`.
    pub rewards: BTreeMap<String, Reward>,
    /// The banner texts the file lists (`mission_msg_14`, `interest_msg_08` …).
    pub messages: Vec<String>,
}

/// Key names found by hashing candidates (`missions.md`).
const DATA_NAMES: [&str; 13] = [
    "rewards",
    "key_missions",
    "faction_missions",
    "faction_tasks",
    "stronghold_takeovers",
    "tutorial_missions",
    "collect_skulls",
    "collect_black_boxes",
    "collect_drug_drops",
    "settlement_chaos",
    "light",
    "medium",
    "heavy",
];

fn key_name(k: &Key) -> String {
    match k {
        Key::Name(n) => n.clone(),
        Key::Hash(h) => DATA_NAMES
            .iter()
            .find(|n| hash_str(n) == *h)
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("#{h:08x}")),
    }
}

fn num(n: &Node, key: &str) -> u32 {
    match n.value(key) {
        Some(Variant::Int(i)) => (*i).max(0) as u32,
        Some(Variant::Float(f)) => f.max(0.0) as u32,
        _ => 0,
    }
}

fn reward_of(n: &Node) -> Reward {
    Reward {
        money: num(n, "money"),
        chaos: num(n, "chaos"),
        weapon_parts: num(n, "weapon_parts"),
        vehicle_parts: num(n, "vehicle_parts"),
        armor_parts: num(n, "armor_parts"),
    }
}

impl MissionData {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(r) = n.child("rewards") {
                    out.max_chaos = num(r, "max_chaos");
                    for (k, c) in &r.children {
                        let name = key_name(k);
                        if c.value("money").is_some() {
                            out.rewards.insert(name, reward_of(c));
                        } else {
                            for (k2, c2) in &c.children {
                                out.rewards
                                    .insert(format!("{name}.{}", key_name(k2)), reward_of(c2));
                            }
                        }
                    }
                }
                // the message list: a node of strings only
                if !n.values.is_empty()
                    && n.children.is_empty()
                    && n.values.iter().all(|(_, v)| {
                        matches!(v, Variant::Str(s) if s.contains("_msg_") || s.starts_with("gui_txt_"))
                    })
                {
                    for (_, v) in &n.values {
                        if let Variant::Str(s) = v {
                            out.messages.push(s.clone());
                        }
                    }
                }
            });
        }
        out.messages.sort();
        Ok(out)
    }

    pub fn reward(&self, kind: MissionKind) -> Option<&Reward> {
        self.rewards.get(kind.reward_key()?)
    }
}

// ---------------------------------------------------------------------------------
// missionrules.brf
// ---------------------------------------------------------------------------------

/// A node of the `.brf` tree.
#[derive(Clone, Debug, PartialEq)]
pub enum BrfValue {
    Node(Vec<(String, BrfValue)>),
    Int(i32),
    Float(f32),
    Str(String),
}

impl BrfValue {
    pub fn get(&self, key: &str) -> Option<&BrfValue> {
        match self {
            Self::Node(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        match self.get(key)? {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, BrfValue)] {
        match self {
            Self::Node(v) => v,
            _ => &[],
        }
    }
}

struct BrfReader<'a> {
    d: &'a [u8],
    p: usize,
}

impl BrfReader<'_> {
    fn take(&mut self, n: usize) -> crate::Result<&[u8]> {
        if self.p + n > self.d.len() {
            return Err(crate::Error::Truncated {
                offset: self.p,
                needed: n,
                len: self.d.len(),
            });
        }
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> crate::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> crate::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> crate::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn node(&mut self, depth: usize) -> crate::Result<BrfValue> {
        if depth > 64 {
            return Err(crate::Error::format("brf: nested too deep"));
        }
        let groups = self.u8()?;
        let mut out = Vec::new();
        for _ in 0..groups {
            let ty = self.u16()?;
            let count = self.u16()?;
            for _ in 0..count {
                let n = self.u32()? as usize;
                let key = String::from_utf8_lossy(self.take(n)?).into_owned();
                let v = match ty {
                    1 => self.node(depth + 1)?,
                    2 => match self.u8()? {
                        1 => BrfValue::Int(self.u32()? as i32),
                        2 => BrfValue::Float(f32::from_bits(self.u32()?)),
                        3 => {
                            let n = self.u16()? as usize;
                            BrfValue::Str(String::from_utf8_lossy(self.take(n)?).into_owned())
                        }
                        t => {
                            return Err(crate::Error::format(format!(
                                "brf: value tag {t} at {}",
                                self.p - 1
                            )));
                        }
                    },
                    t => {
                        return Err(crate::Error::format(format!(
                            "brf: group type {t} at {}",
                            self.p
                        )));
                    }
                };
                out.push((key, v));
            }
        }
        Ok(BrfValue::Node(out))
    }
}

/// The `.brf` tree: `u8 groups; per group u16 type (1 nodes, 2 values), u16 count, then
/// per entry u32 length + key and a node (type 1) or a tagged value (type 2: u8 1 = i32,
/// 2 = f32, 3 = u16 length + string)`.
pub fn parse_brf(data: &[u8]) -> crate::Result<BrfValue> {
    let mut r = BrfReader { d: data, p: 0 };
    let root = r.node(0)?;
    if r.p != data.len() {
        return Err(crate::Error::format(format!(
            "brf: {} bytes left over",
            data.len() - r.p
        )));
    }
    Ok(root)
}

/// A rule's condition.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// `{ type = "input", var = "f2t10Obj01.COMPLETED" }`
    Input(String),
    /// `{ type = "operation", opcode = "AND", left, right }` (every rule in the file is an
    /// AND).
    Op {
        opcode: String,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unknown,
}

impl Expr {
    fn from_brf(v: &BrfValue) -> Self {
        match v.str("type") {
            Some("input") => Self::Input(v.str("var").unwrap_or("").to_string()),
            Some("operation") => match (v.get("left"), v.get("right")) {
                (Some(l), Some(r)) => Self::Op {
                    opcode: v.str("opcode").unwrap_or("").to_string(),
                    left: Box::new(Self::from_brf(l)),
                    right: Box::new(Self::from_brf(r)),
                },
                _ => Self::Unknown,
            },
            _ => Self::Unknown,
        }
    }

    /// Whether it holds, given the variables that are set.
    pub fn eval(&self, set: &dyn Fn(&str) -> bool) -> bool {
        match self {
            Self::Input(v) => set(v),
            Self::Op {
                opcode,
                left,
                right,
            } => match opcode.as_str() {
                "AND" => left.eval(set) && right.eval(set),
                "OR" => left.eval(set) || right.eval(set),
                _ => false,
            },
            Self::Unknown => false,
        }
    }

    /// The variables it reads.
    pub fn inputs(&self) -> Vec<&str> {
        match self {
            Self::Input(v) => vec![v.as_str()],
            Self::Op { left, right, .. } => {
                let mut v = left.inputs();
                v.extend(right.inputs());
                v
            }
            Self::Unknown => Vec::new(),
        }
    }
}

/// One rule of `missionrules.brf`: when `condition` holds, every output's `var` is set
/// (opcode `=`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub prio: i32,
    pub condition: Expr,
    pub outputs: Vec<String>,
}

pub fn parse_rules(data: &[u8]) -> crate::Result<Vec<Rule>> {
    let root = parse_brf(data)?;
    let rules = root
        .get("rules")
        .ok_or_else(|| crate::Error::format("brf: no rules"))?;
    Ok(rules
        .entries()
        .iter()
        .map(|(_, r)| Rule {
            name: r.str("name").unwrap_or("").to_string(),
            prio: r.int("prio").unwrap_or(0),
            condition: r
                .get("operation")
                .map(Expr::from_brf)
                .unwrap_or(Expr::Unknown),
            outputs: r
                .get("outputs")
                .map(|o| {
                    o.entries()
                        .iter()
                        .filter_map(|(_, x)| x.str("var").map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

/// The objectives of `mission`, in the order its rules activate them: from
/// `<id>.ACTIVE AND <id>.RESOURCE_LOADED` along `ObjN.COMPLETED ⇒ ObjM.ACTIVE` to
/// `<id>.COMPLETED`. `f2t10` → `f2t10Obj01`, `f2t10Obj02`.
pub fn objective_chain(rules: &[Rule], mission: &str) -> Vec<String> {
    let m = mission.to_ascii_lowercase();
    let ours: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.name.to_ascii_lowercase().starts_with(&m))
        .collect();
    let mut set: HashSet<String> = [format!("{m}.active"), format!("{m}.resource_loaded")]
        .into_iter()
        .collect();
    let mut out: Vec<String> = Vec::new();
    for _ in 0..ours.len() + 1 {
        let mut progressed = false;
        for r in &ours {
            if !r
                .condition
                .eval(&|v: &str| set.contains(&v.to_ascii_lowercase()))
            {
                continue;
            }
            for o in &r.outputs {
                let lo = o.to_ascii_lowercase();
                if let Some(obj) = o
                    .strip_suffix(".ACTIVE")
                    .or_else(|| o.strip_suffix(".active"))
                    && !out.iter().any(|x| x.eq_ignore_ascii_case(obj))
                {
                    out.push(obj.to_string());
                    set.insert(format!("{}.completed", obj.to_ascii_lowercase()));
                    progressed = true;
                } else if set.insert(lo) {
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------------
// mission scripts: the world objects of `<id>.bl.bl` and of exported entities
// ---------------------------------------------------------------------------------

/// The shape of a `CShapeTrigger` (its `CShapeTriggerComponent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// `type = 0`: a box, `size` its full extents (assumed: full, not half).
    Box { half: [f32; 3] },
    /// `type = 1`: a sphere, `size[0]` its radius.
    Sphere { radius: f32 },
}

impl Shape {
    /// Whether `p` (relative to the shape's centre, in its frame) is inside.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        match self {
            Shape::Box { half } => (0..3).all(|i| p[i].abs() <= half[i]),
            Shape::Sphere { radius } => p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= radius * radius,
        }
    }
}

/// What a script object does. Event strings hold several events separated by spaces;
/// they are kept as written (see [`events`]).
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptKind {
    /// `CEventTrigger`: hears `#63f8bec3` (or `#09cf8b80`), fires `#bd811f1d`.
    EventTrigger {
        on: String,
        fire: String,
    },
    /// `CTimeTrigger`: started by `#9d5ffbb3` (or `#26db8770`), stopped by `#6be89c08`,
    /// fires `#e349054e` `time` seconds later (again and again with `loop`).
    TimeTrigger {
        start_on: String,
        stop_on: String,
        time: f32,
        looped: bool,
        fire: String,
    },
    /// `CCountTrigger`: `#5aeeb914` counts up, `#885f02a4` down; at `max_count` it fires
    /// `#43f1b36e`. `text` is the HUD counter's label.
    CountTrigger {
        up_on: String,
        down_on: String,
        max: i32,
        fire: String,
        text: Option<String>,
    },
    /// `CShapeTrigger`: fires `enter_event` / `exit_event` when Rico (`player`) or a vehicle
    /// enters or leaves its shape.
    ShapeTrigger {
        shape: Shape,
        enter: String,
        exit: String,
        player: bool,
    },
    /// `CMessageObject`: shows the objective line `#382b5a0c` on `#f09abd78`.
    Message {
        show_on: String,
        text: String,
    },
    /// `CTargetIndicator`: a world marker with `text` while enabled.
    TargetIndicator {
        text: String,
    },
    /// `CLocalGuiMapIcon`: a map icon of kind `type` (15: objective) while enabled;
    /// `#c3116fcb` the objective it belongs to.
    MapIcon {
        icon: i32,
        objective: String,
    },
    /// `CDialogue`: plays line `cue_name` on `#bd8853ca`, fires `#e472e3c3` when done.
    Dialogue {
        play_on: String,
        cue: String,
        done: String,
    },
    /// `CSpawnSet` with a `CExportedEntityComponent`: puts `entity` (`#f25a8a76`) down and
    /// fires `#f57c8a0e`; with a `CTeamComponent` it places soldiers (`team`).
    Spawn {
        entity: Option<String>,
        spawned: String,
        distance: f32,
        team: bool,
    },
    /// `CRestartPoint`: where the mission restarts (`#3dad288a` the mission).
    RestartPoint {
        mission: String,
    },
    Other,
}

/// One world object of a script.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptObject {
    pub class: String,
    pub name: String,
    /// Absolute in its file (exported entities: relative to where they are put).
    pub world: Transform3,
    /// `enabled` (true when the object has none).
    pub enabled: bool,
    /// `enable_event` / `enable` / `enableonevent` (and a target indicator's `#f09abd78`).
    pub enable_on: String,
    /// `disable_event` / `disable` (and a target indicator's `#d0bd211a`).
    pub disable_on: String,
    pub kind: ScriptKind,
}

/// The events of an event string, lower case (the data writes `msy.f1s07Obj04.completed`
/// and `msy.f1s07obj04.completed` for the same thing).
pub fn events(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split_whitespace().map(|t| t.to_ascii_lowercase())
}

fn sh(n: &Node, h: u32) -> &str {
    match n.value_hash(h) {
        Some(Variant::Str(s)) => s,
        _ => "",
    }
}

fn s<'a>(n: &'a Node, name: &str) -> &'a str {
    n.str(name).unwrap_or("")
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

fn node_world(n: &Node) -> Option<Transform3> {
    match n.value("world")? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        Variant::Mat4x4(m) => Some(Transform3::from_mat4x4(*m)),
        _ => None,
    }
}

fn child_of_class<'a>(n: &'a Node, class: &str) -> Option<&'a Node> {
    n.children
        .iter()
        .map(|(_, c)| c)
        .find(|c| c.str("_class") == Some(class))
}

fn decode(n: &Node, class: &str) -> ScriptKind {
    match class {
        "CEventTrigger" => ScriptKind::EventTrigger {
            on: join(sh(n, 0x63f8_bec3), sh(n, 0x09cf_8b80)),
            fire: sh(n, 0xbd81_1f1d).to_string(),
        },
        "CTimeTrigger" => ScriptKind::TimeTrigger {
            start_on: join(sh(n, 0x9d5f_fbb3), sh(n, 0x26db_8770)),
            stop_on: sh(n, 0x6be8_9c08).to_string(),
            time: n.float("time").unwrap_or(0.0),
            looped: n.int("loop").is_some_and(|l| l != 0),
            fire: sh(n, 0xe349_054e).to_string(),
        },
        "CCountTrigger" => ScriptKind::CountTrigger {
            up_on: sh(n, 0x5aee_b914).to_string(),
            down_on: sh(n, 0x885f_02a4).to_string(),
            max: n.int("max_count").unwrap_or(1),
            fire: sh(n, 0x43f1_b36e).to_string(),
            text: n.str("text").filter(|t| !t.is_empty()).map(str::to_string),
        },
        "CShapeTrigger" => {
            let comp = child_of_class(n, "CShapeTriggerComponent");
            let size = comp
                .and_then(|c| match c.value("size") {
                    Some(Variant::Vec3(v)) => Some(*v),
                    _ => None,
                })
                .unwrap_or([1.0, 1.0, 1.0]);
            let shape = match comp.and_then(|c| c.int("type")).unwrap_or(0) {
                1 => Shape::Sphere { radius: size[0] },
                _ => Shape::Box {
                    half: [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0],
                },
            };
            ScriptKind::ShapeTrigger {
                shape,
                enter: s(n, "enter_event").to_string(),
                exit: s(n, "exit_event").to_string(),
                player: n.int("player").is_none_or(|p| p != 0),
            }
        }
        "CMessageObject" => ScriptKind::Message {
            show_on: sh(n, 0xf09a_bd78).to_string(),
            text: sh(n, 0x382b_5a0c).to_string(),
        },
        "CTargetIndicator" => ScriptKind::TargetIndicator {
            text: s(n, "text").to_string(),
        },
        "CLocalGuiMapIcon" => ScriptKind::MapIcon {
            icon: n.int("type").unwrap_or(15),
            objective: sh(n, 0xc311_6fcb).to_string(),
        },
        "CDialogue" => ScriptKind::Dialogue {
            play_on: sh(n, 0xbd88_53ca).to_string(),
            cue: s(n, "cue_name").to_string(),
            done: sh(n, 0xe472_e3c3).to_string(),
        },
        "CSpawnSet" => {
            let eec = child_of_class(n, "CExportedEntityComponent");
            ScriptKind::Spawn {
                entity: eec
                    .map(|c| sh(c, 0xf25a_8a76).to_string())
                    .filter(|e| !e.is_empty()),
                spawned: eec
                    .map(|c| sh(c, 0xf57c_8a0e).to_string())
                    .unwrap_or_default(),
                distance: n.float("spawn_distance").unwrap_or(300.0),
                team: child_of_class(n, "CTeamComponent").is_some(),
            }
        }
        "CRestartPoint" => ScriptKind::RestartPoint {
            mission: sh(n, 0x3dad_288a).to_string(),
        },
        _ => ScriptKind::Other,
    }
}

/// Every object with a `_class` in a script file (a mission's `.bl.bl`, an exported
/// entity's `.epe`), depth-first, with absolute transforms. Components that belong to
/// their parent (`CShapeTriggerComponent`, `CExportedEntityComponent`,
/// `CPlacementComponent`, `STeamAgentInfo`, …) are folded into it, not listed.
pub fn script_objects(file: &PropertyFile) -> Vec<ScriptObject> {
    fn walk(n: &Node, parent: &Transform3, out: &mut Vec<ScriptObject>) {
        let mut here = *parent;
        if let Some(class) = n.str("_class") {
            if let Some(w) = node_world(n) {
                here = w.then(parent);
            }
            let component =
                class.ends_with("Component") || class.starts_with('S') || class == "CTeamComponent";
            if !component {
                let target = class == "CTargetIndicator";
                out.push(ScriptObject {
                    class: class.to_string(),
                    name: s(n, "name").to_string(),
                    world: here,
                    enabled: n.int("enabled").is_none_or(|e| e != 0),
                    enable_on: join(
                        &join(s(n, "enable_event"), s(n, "enable")),
                        &join(
                            s(n, "enableonevent"),
                            if target { sh(n, 0xf09a_bd78) } else { "" },
                        ),
                    ),
                    disable_on: join(
                        &join(s(n, "disable_event"), s(n, "disable")),
                        if target { sh(n, 0xd0bd_211a) } else { "" },
                    ),
                    kind: decode(n, class),
                });
            }
            // a spawn set's agents carry their own triggers and markers (a colonel's);
            // those belong to the soldiers, not to the script
            if class == "CSpawnSet" {
                return;
            }
        }
        for (_, c) in &n.children {
            walk(c, &here, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

/// A mission's script: `<id>.blz` → its `.bl` member.
pub fn mission_script(fs: &crate::GameFs, id: &str) -> crate::Result<Vec<ScriptObject>> {
    Ok(script_objects(&mission_file(fs, id)?))
}

/// A mission's script file, parsed.
pub fn mission_file(fs: &crate::GameFs, id: &str) -> crate::Result<PropertyFile> {
    let name = format!("{}.blz", id.to_ascii_lowercase());
    let data = fs.read_inflated(&name)?;
    first_property_member(&data, &[".bl"])
        .ok_or_else(|| crate::Error::format(format!("{name}: no script member")))
}

/// The first member of a SARC container whose name ends in one of `ends`, parsed as a
/// property file (zlib removed).
pub fn first_property_member(data: &[u8], ends: &[&str]) -> Option<PropertyFile> {
    let sarc = crate::sarc::SmallArchive::parse(data).ok()?;
    let e = sarc.entries.iter().find(|e| {
        let n = e.name.to_ascii_lowercase();
        ends.iter().any(|x| n.ends_with(x))
    })?;
    let bytes = crate::compress::maybe_inflate(sarc.data(data, e).ok()?).ok()?;
    PropertyFile::parse_any(&bytes).ok()
}

/// Where a mission starts: its first restart point, else its first target indicator,
/// else its first object with a position (engine axes).
pub fn start_point(objects: &[ScriptObject]) -> Option<[f32; 3]> {
    let not_origin = |o: &&ScriptObject| {
        let t = o.world.translation();
        t[0].abs() + t[2].abs() > 1.0
    };
    objects
        .iter()
        .filter(not_origin)
        .find(|o| matches!(o.kind, ScriptKind::RestartPoint { .. }))
        .or_else(|| {
            objects
                .iter()
                .filter(not_origin)
                .find(|o| matches!(o.kind, ScriptKind::TargetIndicator { .. }))
        })
        .or_else(|| objects.iter().find(not_origin))
        .map(|o| o.world.translation())
}

// ---------------------------------------------------------------------------------
// strongholds
// ---------------------------------------------------------------------------------

/// A faction stronghold: a settlement of kind 32 whose `icons_global.bin` entry names its
/// takeover mission.
#[derive(Clone, Debug, PartialEq)]
pub struct Stronghold {
    /// The settlement's id (`03x26`).
    pub id: String,
    /// `#0fba8173`: the takeover mission (`f1s07`).
    pub mission: String,
    /// `#01dfc541`.
    pub position: [f32; 3],
    /// `#12252e33`: what its targets fire (`f1s07.CountUp`, `f1s02.incree`,
    /// `f1s03.count.up`).
    pub count_event: String,
    /// `#483e0726`: how many there are (29 for `f1s07`).
    pub total: u32,
    /// `#7a5ce7b9`: the faction that takes it (1 Reapers, 2 Roaches, 3 Ular Boys).
    pub faction: i32,
    /// `#80bdb73e`: fired when it is complete (`f1s07.complete`).
    pub complete_event: String,
    /// `#0d9a95f0`: fired on entering (`location::settlement.enter f1s07.enter`).
    pub enter_event: String,
}

impl Stronghold {
    /// The container with the base itself (`locations\strongholds\f1s07\f1s07.base.nlz`).
    pub fn base_container(&self) -> String {
        format!("{}.base.nlz", self.mission.to_ascii_lowercase())
    }

    /// The counter key [`crate::chaos::counter_in`] gives its targets' events.
    pub fn counter(&self) -> Option<&str> {
        crate::chaos::counter_in(&self.count_event)
    }

    pub fn faction(&self) -> Option<Faction> {
        Faction::from_icon_faction(self.faction)
    }
}

/// The strongholds of `icons_global.bin`.
pub fn strongholds(icons_global: &[u8]) -> crate::Result<Vec<Stronghold>> {
    let file = PropertyFile::parse_any(icons_global)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            let mission = sh(n, 0x0fba_8173);
            if mission.is_empty() {
                return;
            }
            let Some(Variant::Vec3(p)) = n.value_hash(0x01df_c541) else {
                return;
            };
            let int = |h: u32| match n.value_hash(h) {
                Some(Variant::Int(i)) => *i,
                _ => 0,
            };
            out.push(Stronghold {
                id: s(n, "id").to_string(),
                mission: mission.to_string(),
                position: *p,
                count_event: sh(n, 0x1225_2e33).to_string(),
                total: int(0x483e_0726).max(0) as u32,
                faction: int(0x7a5c_e7b9),
                complete_event: sh(n, 0x80bd_b73e).to_string(),
                enter_event: sh(n, 0x0d9a_95f0).to_string(),
            });
        });
    }
    Ok(out)
}

/// A mission giver's map icon (`f2t10Emp`: kind 1 faction mission, 2 Agency, 4
/// stronghold) as `icons_global.bin` places it.
pub fn giver_position(icons_global: &[u8], mission: &str) -> Option<[f32; 3]> {
    let file = PropertyFile::parse_any(icons_global).ok()?;
    let want = format!("{}emp", mission.to_ascii_lowercase());
    let mut found = None;
    for root in &file.nodes {
        root.walk(&mut |n| {
            if found.is_none()
                && n.str("id").is_some_and(|i| i.eq_ignore_ascii_case(&want))
                && let Some(Variant::Vec3(p)) = n.value_hash(0x01df_c541)
            {
                found = Some(*p);
            }
        });
    }
    found
}

// ---------------------------------------------------------------------------------
// colonels
// ---------------------------------------------------------------------------------

/// One soldier of a colonel's team.
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    /// `#78efe1bc`: `general` for the colonel.
    pub role: String,
    /// `#1df63231`.
    pub health: f32,
    /// `weapon_name`: an index (11 on the colonels; the table it indexes is not known).
    pub weapon: i32,
    /// `#e47c51fc`: the doll (`missionspecific\pd_ms_gov_shielder.cdoll`), or empty.
    pub doll: String,
}

impl Agent {
    /// The doll's container (`pd_ms_gov_shielder.ee`), if the agent names one.
    pub fn doll_container(&self) -> Option<String> {
        let bare = crate::placement::bare_name(&self.doll);
        let stem = bare.rsplit_once('.').map(|(s, _)| s).unwrap_or(bare);
        (!stem.is_empty()).then(|| format!("{}.ee", stem.to_ascii_lowercase()))
    }
}

/// A colonel ("general" in the data): an elite officer with guards, placed by a location.
#[derive(Clone, Debug, PartialEq)]
pub struct Colonel {
    /// `age40` (from the marker's text key `age40_tin_txt_01`).
    pub id: String,
    /// The spawn set's position (engine axes).
    pub position: [f32; 3],
    pub colonel: Agent,
    pub guards: Vec<Agent>,
    /// `#71e44fd6`: the event his death fires.
    pub killed_event: String,
    /// What the event triggers listening to it fire: `gsy.activity.type08` (the activity:
    /// "Colonel Assassinated", +1000 chaos, $5000)…
    pub activity: Option<u32>,
    /// …the settlement it counts towards (`51x10.count.up`)…
    pub counter: Option<String>,
    /// …and `msy.age40.complete`.
    pub complete_event: Option<String>,
}

impl Colonel {
    /// His name in the text database ("Colonel Rakyat").
    pub fn name_key(&self) -> String {
        format!("{}_tin_txt_01", self.id)
    }

    /// "Colonel Rakyat Assassinated".
    pub fn message_key(&self) -> String {
        format!("{}_msg_01", self.id)
    }
}

fn agent_of(a: &Node) -> Agent {
    Agent {
        role: sh(a, 0x78ef_e1bc).to_string(),
        health: match a.value_hash(0x1df6_3231) {
            Some(Variant::Int(i)) => *i as f32,
            Some(Variant::Float(f)) => *f,
            _ => 100.0,
        },
        weapon: a.int("weapon_name").unwrap_or(0),
        doll: sh(a, 0xe47c_51fc).to_string(),
    }
}

/// The colonels a location file places.
pub fn colonels(file: &PropertyFile) -> Vec<Colonel> {
    // every event trigger: (what it hears, what it fires)
    let mut triggers: Vec<(String, String)> = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if n.str("_class") == Some("CEventTrigger") {
                triggers.push((
                    join(sh(n, 0x63f8_bec3), sh(n, 0x09cf_8b80)),
                    sh(n, 0xbd81_1f1d).to_string(),
                ));
            }
        });
    }
    let mut out = Vec::new();
    fn walk(n: &Node, parent: &Transform3, triggers: &[(String, String)], out: &mut Vec<Colonel>) {
        let here = node_world(n).map(|w| w.then(parent)).unwrap_or(*parent);
        if n.str("_class") == Some("CSpawnSet") {
            if let Some(team) = child_of_class(n, "CTeamComponent") {
                let agents: Vec<&Node> = team
                    .children
                    .iter()
                    .map(|(_, c)| c)
                    .filter(|c| c.str("_class") == Some("STeamAgentInfo"))
                    .collect();
                if let Some(general) = agents
                    .iter()
                    .find(|a| sh(a, 0x78ef_e1bc).eq_ignore_ascii_case("general"))
                {
                    // the marker's text names him
                    let mut text = String::new();
                    general.walk(&mut |c| {
                        if text.is_empty()
                            && c.str("_class") == Some("CTargetIndicator")
                            && let Some(t) = c.str("text")
                        {
                            text = t.to_string();
                        }
                    });
                    let id = text
                        .split_once("_tin_txt")
                        .map(|(a, _)| a.to_string())
                        .unwrap_or_default();
                    let killed = sh(general, 0x71e4_4fd6).to_string();
                    let heard: Vec<String> = events(&killed).collect();
                    let fired: Vec<&str> = triggers
                        .iter()
                        .filter(|(on, _)| events(on).any(|e| heard.contains(&e)))
                        .map(|(_, f)| f.as_str())
                        .collect();
                    let activity = fired.iter().find_map(|f| crate::chaos::activity_in(f));
                    let counter = fired
                        .iter()
                        .find_map(|f| crate::chaos::counter_in(f))
                        .map(str::to_string);
                    let complete_event = fired.iter().find_map(|f| {
                        f.split_whitespace()
                            .find(|t| t.starts_with("msy.") && t.ends_with(".complete"))
                            .map(str::to_string)
                    });
                    let id = if id.is_empty() {
                        complete_event
                            .as_deref()
                            .and_then(|c| c.strip_prefix("msy."))
                            .and_then(|c| c.strip_suffix(".complete"))
                            .unwrap_or("")
                            .to_string()
                    } else {
                        id
                    };
                    out.push(Colonel {
                        id,
                        position: here.translation(),
                        colonel: agent_of(general),
                        guards: agents
                            .iter()
                            .filter(|a| !std::ptr::eq(**a, *general))
                            .map(|a| agent_of(a))
                            .collect(),
                        killed_event: killed,
                        activity,
                        counter,
                        complete_event,
                    });
                }
            }
            return;
        }
        for (_, c) in &n.children {
            walk(c, &here, triggers, out);
        }
    }
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &triggers, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> Option<crate::GameFs> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        crate::GameFs::open(dir).ok()
    }

    #[test]
    fn regions_and_shapes() {
        let r = parse_regions(b"f1s07_r1;0\r\nf1s07_r2;60000\nbad line\nf3s06_r2;390000");
        assert_eq!(r.len(), 3);
        assert_eq!(
            r[1],
            Region {
                stronghold: "f1s07".into(),
                index: 2,
                chaos: 60000
            }
        );
        let b = Shape::Box {
            half: [1.0, 2.0, 3.0],
        };
        assert!(b.contains([0.9, -1.9, 2.9]) && !b.contains([1.1, 0.0, 0.0]));
        assert!(Shape::Sphere { radius: 5.0 }.contains([3.0, 0.0, 3.9]));
        assert_eq!(
            events("msy.f1s07Obj04.completed  f1s07.X").collect::<Vec<_>>(),
            ["msy.f1s07obj04.completed", "f1s07.x"]
        );
        assert_eq!(Faction::Roaches.icon_faction(), 2);
        assert_eq!(
            Faction::UlarBoys.stronghold_icon(true).as_deref(),
            Some("hud_icon_sh_ular_c_dif.dds")
        );
    }

    #[test]
    fn brf_round_trip_by_hand() {
        // { rules { r1 { name = "m.x", prio = 0 } } }
        let mut d = vec![1u8, 1, 0, 1, 0];
        d.extend_from_slice(&5u32.to_le_bytes());
        d.extend_from_slice(b"rules");
        d.extend_from_slice(&[1, 1, 0, 1, 0]);
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(b"r1");
        d.extend_from_slice(&[1, 2, 0, 2, 0]);
        d.extend_from_slice(&4u32.to_le_bytes());
        d.extend_from_slice(b"name");
        d.extend_from_slice(&[3, 3, 0]);
        d.extend_from_slice(b"m.x");
        d.extend_from_slice(&4u32.to_le_bytes());
        d.extend_from_slice(b"prio");
        d.push(1);
        d.extend_from_slice(&7i32.to_le_bytes());
        let root = parse_brf(&d).unwrap();
        let r1 = root.get("rules").unwrap().get("r1").unwrap();
        assert_eq!((r1.str("name"), r1.int("prio")), (Some("m.x"), Some(7)));
    }

    /// Needs JC2_TEST_DIR with `missions.bmf`, `regions.csv`, `missiondata.bin`.
    #[test]
    fn the_mission_table() {
        let Some(fs) = fs() else { return };
        let (Ok(b), Ok(r), Ok(m)) = (
            fs.read_inflated("missions.bmf"),
            fs.read_inflated("regions.csv"),
            fs.read_inflated("missiondata.bin"),
        ) else {
            return;
        };
        let t = parse_bmf(&b).unwrap();
        assert_eq!(t.len(), 441);
        let count = |k: MissionKind| t.iter().filter(|m| m.kind == k).count();
        assert_eq!(count(MissionKind::Agency), 7);
        assert_eq!(count(MissionKind::FactionMission), 11);
        assert_eq!(count(MissionKind::FactionTask), 38);
        assert_eq!(count(MissionKind::Stronghold), 9);
        assert_eq!(count(MissionKind::Challenge), 76);
        assert_eq!(count(MissionKind::Skulls), 100);
        // every record's hash is its id's
        assert!(t.iter().all(|m| m.hash == hash_str(&m.id)));
        let f2t10 = t.iter().find(|m| m.id == "f2t10").unwrap();
        assert_eq!(
            (
                f2t10.requires.as_str(),
                f2t10.objectives,
                f2t10.kind,
                f2t10.chaos,
                f2t10.faction()
            ),
            (
                "f1s03",
                2,
                MissionKind::FactionTask,
                390_000,
                Some(Faction::Roaches)
            )
        );
        // the chain: km01 → km02 → the three first strongholds → the next ones
        let get = |id: &str| t.iter().find(|m| m.id == id).unwrap();
        assert_eq!(get("km01").prerequisite(), None);
        assert_eq!(get("km02").prerequisite(), Some("km01"));
        assert_eq!(get("f1s07").prerequisite(), Some("km02"));
        assert_eq!(get("f1s02").prerequisite(), Some("f1s07"));
        assert_eq!((get("f1s02").chaos, get("km03").chaos), (110_000, 75_000));
        assert_eq!(get("f3s01").faction(), Some(Faction::Reapers));
        // regions: the chaos values of the records are the regions'
        let regions = parse_regions(&r);
        assert_eq!(regions.len(), 21);
        assert!(
            regions
                .iter()
                .any(|g| g.stronghold == "f1s02" && g.index == 2 && g.chaos == 180_000)
        );
        let d = MissionData::parse(&m).unwrap();
        assert_eq!(d.max_chaos, 809_750);
        let task = d.reward(MissionKind::FactionTask).unwrap();
        assert_eq!(
            (
                task.money,
                task.chaos,
                task.weapon_parts,
                task.vehicle_parts
            ),
            (10_000, 5_000, 1, 1)
        );
        let sh = d.reward(MissionKind::Stronghold).unwrap();
        assert_eq!((sh.money, sh.chaos), (17_500, 7_500));
        let km = d.reward(MissionKind::Agency).unwrap();
        assert_eq!(
            (km.money, km.vehicle_parts, km.weapon_parts),
            (20_000, 3, 3)
        );
        assert_eq!(d.rewards["settlement_chaos.heavy"].chaos, 3_000);
        assert!(d.messages.iter().any(|m| m == "mission_msg_14"));
    }

    /// Needs JC2_TEST_DIR with `missionrules.brf`.
    #[test]
    fn the_rules_chain_the_objectives() {
        let Some(fs) = fs() else { return };
        let Ok(d) = fs.read_inflated("missionrules.brf") else {
            return;
        };
        let rules = parse_rules(&d).unwrap();
        assert_eq!(rules.len(), 325);
        assert_eq!(
            objective_chain(&rules, "f2t10"),
            ["f2t10Obj01", "f2t10Obj02"]
        );
        assert_eq!(
            objective_chain(&rules, "f1m03"),
            ["f1m03Obj01", "f1m03Obj02", "f1m03Obj03", "f1m03Obj04"]
        );
        // the stronghold starts with its fifth: "Talk to the technician"
        assert_eq!(
            objective_chain(&rules, "f1s07"),
            [
                "f1s07Obj05",
                "f1s07Obj01",
                "f1s07Obj02",
                "f1s07Obj03",
                "f1s07Obj04"
            ]
        );
        assert_eq!(objective_chain(&rules, "km02").len(), 9);
    }

    /// The proof mission's script. Needs JC2_TEST_DIR with `f2t10.blz`.
    #[test]
    fn the_script_of_f2t10() {
        let Some(fs) = fs() else { return };
        let Ok(objs) = mission_script(&fs, "f2t10") else {
            return;
        };
        let spawns: Vec<&ScriptObject> = objs
            .iter()
            .filter(|o| {
                matches!(&o.kind, ScriptKind::Spawn { entity: Some(e), .. } if e.contains("gb017b"))
            })
            .collect();
        assert_eq!(spawns.len(), 5, "the five silos");
        let msg = objs
            .iter()
            .find_map(|o| match &o.kind {
                ScriptKind::Message { text, show_on } if text == "f2t10_obj_txt_01" => {
                    Some(show_on.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(msg, "f2t10.Obj01.tinMicMob.show");
        let count = objs
            .iter()
            .find_map(|o| match &o.kind {
                ScriptKind::CountTrigger { max, fire, .. }
                    if fire == "f2t10.all.destructables.complete" =>
                {
                    Some(*max)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(count, 5);
        // the base's trigger: a 250 m shape round the objective marker
        let base = objs
            .iter()
            .find(|o| o.name == "sht.f2t10.base.entered")
            .unwrap();
        let p = base.world.translation();
        assert!(
            (p[0] - 5848.0).abs() < 2.0 && (p[2] - 10392.1).abs() < 2.0,
            "{p:?}"
        );
        let start = start_point(&objs).unwrap();
        assert!((start[0] - 9134.6).abs() < 1.0, "{start:?}");
    }

    /// Needs JC2_TEST_DIR with `icons_global.bin`.
    #[test]
    fn nine_strongholds() {
        let Some(fs) = fs() else { return };
        let Ok(d) = fs.read_inflated("icons_global.bin") else {
            return;
        };
        let s = strongholds(&d).unwrap();
        assert_eq!(s.len(), 9);
        let h = s.iter().find(|s| s.mission == "f1s07").unwrap();
        assert_eq!(
            (
                h.id.as_str(),
                h.count_event.as_str(),
                h.total,
                h.faction(),
                h.complete_event.as_str()
            ),
            (
                "03x26",
                "f1s07.CountUp",
                29,
                Some(Faction::Roaches),
                "f1s07.complete"
            )
        );
        assert_eq!(h.base_container(), "f1s07.base.nlz");
        assert_eq!(h.counter(), Some("f1s07.CountUp"));
        let f1s03 = s.iter().find(|s| s.mission == "f1s03").unwrap();
        assert_eq!(f1s03.counter(), Some("f1s03"));
        assert!(giver_position(&d, "f2t10").is_some());
    }

    /// The colonel of tile 51,10. Needs JC2_TEST_DIR with `51x10.nlz`.
    #[test]
    fn a_colonel_and_his_guard() {
        let Some(fs) = fs() else { return };
        let Ok(data) = fs.read_inflated("51x10.nlz") else {
            return;
        };
        let file = first_property_member(&data, &[".blo"]).unwrap();
        let c = colonels(&file);
        assert_eq!(c.len(), 1, "{c:#?}");
        let c = &c[0];
        assert_eq!(c.id, "age40");
        assert_eq!(c.colonel.health, 500.0);
        assert_eq!(
            c.colonel.doll_container().as_deref(),
            Some("pd_ms_gov_shielder.ee")
        );
        assert_eq!(c.activity, Some(8));
        assert_eq!(c.counter.as_deref(), Some("51x10"));
        assert_eq!(c.complete_event.as_deref(), Some("msy.age40.complete"));
        assert!(!c.guards.is_empty());
        assert!((c.position[0] - 10081.6).abs() < 1.0);
    }
}
