//! `settings\enemytypespawnsettings.bin`: who the enemy soldiers are. For each stage of
//! the game's progress (a `min_chaos` … `max_chaos` band of chaos points) two classes,
//! `grunt` and `elite`, each with its `health` and the weapons it may carry
//! (`weapon_N { weapon_name, probability }`), plus the chance a soldier is an elite per
//! difficulty (`elite_probability_easy/normal/hard/extreme`, in percent); and for each
//! faction the dolls the two classes wear (`factions { … { grunt_cdoll, elite_cdoll } }`).
//! Every key was named by hashing candidates (`research/formats/spawning.md`).

use crate::property::{Node, PropertyFile, Variant};

/// One class of soldier at one stage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnemyClass {
    /// `health`: 100 for a grunt (120 at the last stage), 250 for an elite.
    pub health: f32,
    /// `weapon_N`: `weapon_name` (a `weapons.bin` name) and its `probability` (weights
    /// that add up to 100).
    pub weapons: Vec<(String, f32)>,
}

impl EnemyClass {
    fn from_node(n: &Node) -> Self {
        // `weapon_0` first: the files list them out of order
        let mut weapons: Vec<(usize, String, f32)> = n
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, (k, w))| {
                let name = w.str("weapon_name")?;
                let idx = (0..16)
                    .find(|j| crate::hash::hash_str(&format!("weapon_{j}")) == k.hash())
                    .unwrap_or(16 + i);
                Some((idx, name.to_string(), w.float("probability").unwrap_or(0.0)))
            })
            .collect();
        weapons.sort_by_key(|w| w.0);
        Self {
            health: n.float("health").unwrap_or(100.0),
            weapons: weapons.into_iter().map(|(_, n, p)| (n, p)).collect(),
        }
    }

    /// The weapon for a roll `r` in `0..1`, by the weights.
    pub fn pick_weapon(&self, r: f32) -> Option<&str> {
        let total: f32 = self.weapons.iter().map(|w| w.1.max(0.0)).sum();
        if total <= 0.0 {
            return self.weapons.first().map(|w| w.0.as_str());
        }
        let mut at = r.clamp(0.0, 0.9999) * total;
        for (name, p) in &self.weapons {
            if at < p.max(0.0) {
                return Some(name);
            }
            at -= p.max(0.0);
        }
        self.weapons.last().map(|w| w.0.as_str())
    }
}

/// The game's difficulty settings, for `elite_probability_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Normal,
    Hard,
    Extreme,
}

/// One stage of the game (a band of chaos points).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnemyStage {
    pub min_chaos: f32,
    pub max_chaos: f32,
    /// Percent of soldiers that are elites: easy, normal, hard, extreme.
    pub elite_probability: [f32; 4],
    pub grunt: EnemyClass,
    pub elite: EnemyClass,
}

impl EnemyStage {
    /// The chance (0..1) that a soldier is an elite.
    pub fn elite_chance(&self, d: Difficulty) -> f32 {
        self.elite_probability[d as usize] / 100.0
    }
}

/// The dolls a faction's soldiers wear (`exported\cdoll\<name>.ee`, given here as the
/// `.cdoll` inside, `pd_gov_base01.cdoll`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactionDolls {
    /// The faction's key: a hash (the government's, `#4fad2d93`, is not named yet).
    pub key: u32,
    pub grunt_cdoll: String,
    pub elite_cdoll: String,
}

impl FactionDolls {
    /// The containers the dolls live in: `pd_gov_base01.ee`, lower-case.
    pub fn grunt_container(&self) -> String {
        container_of(&self.grunt_cdoll)
    }
    pub fn elite_container(&self) -> String {
        container_of(&self.elite_cdoll)
    }
}

fn container_of(cdoll: &str) -> String {
    let bare = crate::placement::bare_name(cdoll).to_ascii_lowercase();
    match bare.strip_suffix(".cdoll") {
        Some(b) => format!("{b}.ee"),
        None => bare,
    }
}

/// The government faction's key in `factions` (its dolls are `pd_gov_base01` and
/// `pd_gov_elite`); its name does not hash from the candidates tried.
pub const GOVERNMENT: u32 = 0x4fad_2d93;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnemyTypes {
    /// Ordered by `min_chaos`.
    pub stages: Vec<EnemyStage>,
    pub factions: Vec<FactionDolls>,
    /// The AI teams the root names (`bofcx1.aiteam`, `pursuitandsearch.aiteam`,
    /// `heatboat.aiteam`).
    pub aiteams: Vec<String>,
}

impl EnemyTypes {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            for (_, v) in &root.values {
                if let Variant::Str(s) = v
                    && s.ends_with(".aiteam")
                {
                    out.aiteams.push(s.clone());
                }
            }
            if let Some(types) = root.child("enemy_types") {
                for (_, st) in &types.children {
                    let prob = |k: &str| st.float(k).unwrap_or(0.0);
                    out.stages.push(EnemyStage {
                        min_chaos: prob("min_chaos"),
                        max_chaos: prob("max_chaos"),
                        elite_probability: [
                            prob("elite_probability_easy"),
                            prob("elite_probability_normal"),
                            prob("elite_probability_hard"),
                            prob("elite_probability_extreme"),
                        ],
                        grunt: st
                            .child("grunt")
                            .map(EnemyClass::from_node)
                            .unwrap_or_default(),
                        elite: st
                            .child("elite")
                            .map(EnemyClass::from_node)
                            .unwrap_or_default(),
                    });
                }
            }
            if let Some(f) = root.child("factions") {
                for (k, n) in &f.children {
                    out.factions.push(FactionDolls {
                        key: k.hash(),
                        grunt_cdoll: n.str("grunt_cdoll").unwrap_or("").to_string(),
                        elite_cdoll: n.str("elite_cdoll").unwrap_or("").to_string(),
                    });
                }
            }
        }
        out.stages
            .sort_by(|a, b| a.min_chaos.total_cmp(&b.min_chaos));
        Ok(out)
    }

    /// The stage for a chaos score (the first stage below 0, the last beyond the top).
    pub fn stage(&self, chaos: f32) -> Option<&EnemyStage> {
        self.stages
            .iter()
            .find(|s| chaos >= s.min_chaos && chaos < s.max_chaos)
            .or_else(|| {
                if chaos < self.stages.first()?.min_chaos {
                    self.stages.first()
                } else {
                    self.stages.last()
                }
            })
    }

    pub fn faction(&self, key: u32) -> Option<&FactionDolls> {
        self.factions.iter().find(|f| f.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `enemytypespawnsettings.bin`.
    #[test]
    fn soldiers_by_stage() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("enemytypespawnsettings.bin") else {
            return;
        };
        let e = EnemyTypes::parse(&data).unwrap();
        assert_eq!(e.stages.len(), 5);
        assert!(e.aiteams.iter().any(|t| t == "bofcx1.aiteam"));
        let s = e.stage(0.0).unwrap();
        assert_eq!((s.min_chaos, s.max_chaos), (0.0, 100_000.0));
        assert_eq!(s.elite_probability, [0.0, 10.0, 20.0, 30.0]);
        assert_eq!(s.elite_chance(Difficulty::Normal), 0.1);
        assert_eq!((s.grunt.health, s.elite.health), (100.0, 250.0));
        assert_eq!(s.grunt.weapons[0], ("Beretta Handgun".to_string(), 60.0));
        assert_eq!(s.grunt.weapons.len(), 4);
        assert_eq!(s.elite.weapons[0], ("Assault".to_string(), 70.0));
        assert_eq!(s.grunt.pick_weapon(0.1), Some("Beretta Handgun"));
        assert_eq!(s.elite.pick_weapon(0.75), Some("Shotgun (pump action)"));
        assert_eq!(s.elite.pick_weapon(0.95), Some("MachineGun"));
        let top = e.stage(1e9).unwrap();
        assert_eq!((top.grunt.health, top.elite.health), (120.0, 265.0));
        let gov = e.faction(GOVERNMENT).unwrap();
        assert_eq!(gov.grunt_container(), "pd_gov_base01.ee");
        assert_eq!(gov.elite_container(), "pd_gov_elite.ee");
        assert_eq!(e.factions.len(), 8);
    }
}
