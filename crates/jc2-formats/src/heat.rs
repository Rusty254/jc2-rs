//! Heat: `settings\heatsystem.bin` (`heatsystem { max_heat = 100, … }`) and
//! `settings\heatspawn.bin`, the reinforcements each faction sends as the heat climbs.
//! Per faction (`spawn_event = "__heatspawn_government"`) a band per heat range
//! (`heat_000`, `heat_00` … `heat_04`: `min_heat_level` … `max_heat_level`, 0–20, 20–40 …
//! on a 0–100 scale), each with `waves { wave0, wave1 }` (or the units directly) giving
//! `initial_delay`, `min_spawn_delay`, `max_spawn_delay`, `respawn_delay` and the units
//! (`unit_00 { unit_type, max_units }`); `unit_types` says what a unit is (`type` land,
//! air or sea, `entity_file` the vehicle, `num_seats` the soldiers in it). Keys named by
//! hashing; see `research/formats/spawning.md`.

use crate::property::{Node, PropertyFile};

/// A kind of reinforcement unit (`unit_types`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitType {
    pub name: String,
    /// `type`: `land`, `air` or `sea`.
    pub kind: String,
    /// `entity_file`: the vehicle (`exported\vehicles\lave\lave.v007_military.ee`).
    pub entity_file: String,
    /// `num_seats`: soldiers it brings.
    pub num_seats: u32,
}

/// One wave of a band: its timing and units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatWave {
    pub initial_delay: f32,
    pub min_spawn_delay: f32,
    pub max_spawn_delay: f32,
    pub respawn_delay: f32,
    /// `unit_type`, `max_units`.
    pub units: Vec<(String, u32)>,
}

/// What a faction sends within a range of heat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatBand {
    pub min_heat_level: f32,
    pub max_heat_level: f32,
    pub waves: Vec<HeatWave>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatSpawn {
    /// By `spawn_event` (`__heatspawn_government`), bands ordered by `min_heat_level`.
    pub factions: Vec<(String, Vec<HeatBand>)>,
    pub unit_types: Vec<UnitType>,
}

fn wave(n: &Node) -> HeatWave {
    HeatWave {
        initial_delay: n.float("initial_delay").unwrap_or(0.0),
        min_spawn_delay: n.float("min_spawn_delay").unwrap_or(0.0),
        max_spawn_delay: n.float("max_spawn_delay").unwrap_or(0.0),
        respawn_delay: n.float("respawn_delay").unwrap_or(0.0),
        units: n
            .children
            .iter()
            .filter_map(|(_, u)| {
                Some((
                    u.str("unit_type")?.to_string(),
                    u.float("max_units").unwrap_or(1.0).max(0.0) as u32,
                ))
            })
            .collect(),
    }
}

impl HeatSpawn {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            for (_, f) in &root.children {
                if let Some(ev) = f.str("spawn_event") {
                    let mut bands = Vec::new();
                    for (_, b) in &f.children {
                        let Some(min) = b.float("min_heat_level") else {
                            continue;
                        };
                        let waves = match b.child("waves") {
                            Some(w) => w.children.iter().map(|(_, w)| wave(w)).collect(),
                            None => vec![wave(b)],
                        };
                        bands.push(HeatBand {
                            min_heat_level: min,
                            max_heat_level: b.float("max_heat_level").unwrap_or(min),
                            waves,
                        });
                    }
                    bands.sort_by(|a, b| a.min_heat_level.total_cmp(&b.min_heat_level));
                    out.factions.push((ev.to_string(), bands));
                }
            }
            if let Some(t) = root.child("unit_types") {
                for (_, u) in &t.children {
                    out.unit_types.push(UnitType {
                        name: u.str("name").unwrap_or("").to_string(),
                        kind: u.str("type").unwrap_or("").to_string(),
                        entity_file: u.str("entity_file").unwrap_or("").to_string(),
                        num_seats: u.float("num_seats").unwrap_or(1.0).max(0.0) as u32,
                    });
                }
            }
        }
        Ok(out)
    }

    pub fn faction(&self, spawn_event: &str) -> Option<&[HeatBand]> {
        self.factions
            .iter()
            .find(|f| f.0 == spawn_event)
            .map(|f| f.1.as_slice())
    }

    pub fn unit(&self, name: &str) -> Option<&UnitType> {
        self.unit_types
            .iter()
            .find(|u| u.name.eq_ignore_ascii_case(name))
    }

    /// The band of a faction for a heat value (`min < heat ≤ max`).
    pub fn band(&self, spawn_event: &str, heat: f32) -> Option<&HeatBand> {
        self.faction(spawn_event)?
            .iter()
            .find(|b| heat > b.min_heat_level && heat <= b.max_heat_level)
    }

    /// Soldiers a band brings by land: the seats of its land units, all waves.
    pub fn land_soldiers(&self, band: &HeatBand) -> u32 {
        band.waves
            .iter()
            .flat_map(|w| &w.units)
            .filter_map(|(t, n)| {
                let u = self.unit(t)?;
                (u.kind == "land").then_some(n * u.num_seats)
            })
            .sum()
    }
}

/// `settings\heatsystem.bin`: `heatsystem { max_heat }` (100). The rest of the block
/// (`combat_time` 1, `combat_delay` 2, `undetected_time` 2, and unnamed 10/20/40/60/80
/// steps and 100000…300000 chaos thresholds) is not used yet.
pub fn max_heat(data: &[u8]) -> crate::Result<Option<f32>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = None;
    for root in &file.nodes {
        root.walk(&mut |n| {
            if out.is_none() {
                out = n.float("max_heat");
            }
        });
    }
    Ok(out)
}

pub const GOVERNMENT_HEAT: &str = "__heatspawn_government";

/// `settings\heatsystem.bin`'s heat levels: `heat_level_max_heat_N` (20, 40, 60, 80, 100:
/// the top of level N) and `heat_level_chaos_N` (0, 0, 100 000, 200 000, 300 000: the
/// chaos a level needs before the heat can reach it). Keys named from the exe's
/// `heat_level_chaos_%d` / `heat_level_max_heat_%d`.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatLevels {
    pub max_heat: f32,
    /// (`heat_level_chaos_N`, `heat_level_max_heat_N`) for N = 1 … 5.
    pub levels: Vec<(f32, f32)>,
}

impl Default for HeatLevels {
    fn default() -> Self {
        Self {
            max_heat: 100.0,
            levels: (1..=5).map(|n| (0.0, n as f32 * 20.0)).collect(),
        }
    }
}

impl HeatLevels {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(m) = n.float("max_heat") {
                    out.max_heat = m;
                    let levels: Vec<(f32, f32)> = (1..=5)
                        .map_while(|i| {
                            Some((
                                n.float(&format!("heat_level_chaos_{i}"))?,
                                n.float(&format!("heat_level_max_heat_{i}"))?,
                            ))
                        })
                        .collect();
                    if !levels.is_empty() {
                        out.levels = levels;
                    }
                }
            });
        }
        Ok(out)
    }

    /// The most heat there can be at a chaos total: the top of the highest level whose
    /// chaos is reached.
    pub fn cap(&self, chaos: f32) -> f32 {
        self.levels
            .iter()
            .filter(|(c, _)| chaos >= *c)
            .map(|(_, m)| *m)
            .fold(0.0, f32::max)
            .min(self.max_heat)
    }

    /// The width of a heat level (the first level's top: 20).
    pub fn level_width(&self) -> f32 {
        self.levels.first().map_or(20.0, |l| l.1).max(1.0)
    }
}

/// One event of `ai\aiheatparams.bin` (`Points`, `CivilianPoints`, `Range`, `Delay`,
/// `UseWitness`, `HeatSoundEvent`, `CivilianSoundEvent`: named by hashing). The event
/// names themselves (the keys) did not hash from any candidate; they are identified by
/// their civilian reaction (`CivilianSoundEvent`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatEvent {
    pub key: u32,
    /// `Points`: what the event is worth to the soldiers (100 for nearly all).
    pub points: f32,
    /// `CivilianPoints`: what it is worth to civilians.
    pub civilian_points: f32,
    /// `Range` (m): how near it must be to be noticed; 0 = no limit.
    pub range: f32,
    /// `Delay` (s) before it counts.
    pub delay: f32,
    /// `UseWitness`: someone has to see (or hear) it.
    pub use_witness: bool,
    /// `CivilianSoundEvent`: the civilians' reaction (`CIVILIAN_NPC_KILLED_BY_PLAYER`).
    pub civilian_reaction: Option<String>,
}

/// The two event tables of `aiheatparams.bin`: `#7f245b02` (18 events, nearly all worth
/// 100 points) and `#8c4886f1` (16, nearly all 0 — only the killing of an NPC and
/// `#96a83491` count). Which game state picks the second is not known; jc2-rs uses the
/// first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeatEvents {
    pub main: Vec<HeatEvent>,
    pub other: Vec<HeatEvent>,
}

const HEAT_TABLE_MAIN: u32 = 0x7f24_5b02;
const HEAT_TABLE_OTHER: u32 = 0x8c48_86f1;
const CIVILIAN_POINTS: &str = "CivilianPoints";

impl HeatEvents {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        let table = |n: &Node| -> Vec<HeatEvent> {
            n.children
                .iter()
                .map(|(k, e)| HeatEvent {
                    key: k.hash(),
                    points: e.float("Points").unwrap_or(0.0),
                    civilian_points: e.float(CIVILIAN_POINTS).unwrap_or(0.0),
                    range: e.float("Range").unwrap_or(0.0),
                    delay: e.float("Delay").unwrap_or(0.0),
                    use_witness: e.float("UseWitness").unwrap_or(0.0) != 0.0,
                    civilian_reaction: e
                        .str("CivilianSoundEvent")
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                })
                .collect()
        };
        for root in &file.nodes {
            for (k, n) in &root.children {
                match k.hash() {
                    HEAT_TABLE_MAIN => out.main = table(n),
                    HEAT_TABLE_OTHER => out.other = table(n),
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    /// The event of the main table whose civilians react with `reaction`.
    pub fn by_reaction(&self, reaction: &str) -> Option<&HeatEvent> {
        self.main
            .iter()
            .find(|e| e.civilian_reaction.as_deref() == Some(reaction))
    }

    /// A soldier (or anyone) killed by the player.
    pub fn kill(&self) -> Option<&HeatEvent> {
        self.by_reaction("CIVILIAN_NPC_KILLED_BY_PLAYER")
    }

    /// An explosion the player set off.
    pub fn explosion(&self) -> Option<&HeatEvent> {
        self.by_reaction("CIVILIAN_EXPLOSION_TRIGGERED_BY_PLAYER")
    }

    /// Shots fired.
    pub fn shots(&self) -> Option<&HeatEvent> {
        self.by_reaction("CIVILIAN_SHOTS_FIRED")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `heatspawn.bin` and `heatsystem.bin`.
    #[test]
    fn government_reinforcements() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(h), Ok(s)) = (
            fs.read_inflated("heatspawn.bin"),
            fs.read_inflated("heatsystem.bin"),
        ) else {
            return;
        };
        assert_eq!(max_heat(&s).unwrap(), Some(100.0));
        let h = HeatSpawn::parse(&h).unwrap();
        let gov = h.faction(GOVERNMENT_HEAT).unwrap();
        // -1…0, then five bands of 20
        assert_eq!(gov.len(), 6);
        assert_eq!((gov[1].min_heat_level, gov[1].max_heat_level), (0.0, 20.0));
        assert_eq!(
            (gov[5].min_heat_level, gov[5].max_heat_level),
            (80.0, 100.0)
        );
        let b1 = h.band(GOVERNMENT_HEAT, 10.0).unwrap();
        assert_eq!(b1.waves[0].respawn_delay, 6.0);
        assert!(b1.waves[0].units.contains(&("ut_mc".to_string(), 2)));
        let mc = h.unit("ut_mc").unwrap();
        assert_eq!((mc.kind.as_str(), mc.num_seats), ("land", 1));
        assert!(mc.entity_file.ends_with("lave.v007_military.ee"));
        // two motorcycles and a small jeep: four soldiers
        assert_eq!(h.land_soldiers(b1), 4);
        assert!(h.faction("__heatspawn_roaches").is_some());
    }

    /// Needs JC2_TEST_DIR with `heatsystem.bin` and `aiheatparams.bin`.
    #[test]
    fn heat_levels_and_events() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(s), Ok(p)) = (
            fs.read_inflated("heatsystem.bin"),
            fs.read_inflated("aiheatparams.bin"),
        ) else {
            return;
        };
        let l = HeatLevels::parse(&s).unwrap();
        assert_eq!(l.max_heat, 100.0);
        assert_eq!(
            l.levels,
            vec![
                (0.0, 20.0),
                (0.0, 40.0),
                (100_000.0, 60.0),
                (200_000.0, 80.0),
                (300_000.0, 100.0)
            ]
        );
        // at the start the heat cannot pass level 2
        assert_eq!(l.cap(0.0), 40.0);
        assert_eq!(l.cap(150_000.0), 60.0);
        assert_eq!(l.cap(1e9), 100.0);
        assert_eq!(l.level_width(), 20.0);
        let e = HeatEvents::parse(&p).unwrap();
        assert_eq!((e.main.len(), e.other.len()), (18, 16));
        let kill = e.kill().unwrap();
        assert_eq!(kill.key, 0x0d03_8313);
        assert_eq!((kill.points, kill.civilian_points), (100.0, 50.0));
        assert_eq!((kill.range, kill.delay), (40.0, 0.5));
        let blast = e.explosion().unwrap();
        assert_eq!(
            (blast.points, blast.range, blast.use_witness),
            (100.0, 50.0, true)
        );
        let shots = e.shots().unwrap();
        assert_eq!((shots.range, shots.civilian_points), (70.0, 30.0));
    }
}
