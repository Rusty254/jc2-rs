//! `settings\weapons.bin`: every weapon in the game (`weapon_N` nodes). A handful of
//! keys are named (`ammo_velocity`, `ammo_max_range`, `weapon_accuracy`,
//! `max_bullet_spread`, by hashing candidates); the others are read by hash with their
//! meaning taken from how they vary across the arsenal — see
//! `research/formats/weapons.md`.
//!
//! Next to it: `settings\weapon_upgrades.bin` (per weapon and upgrade level: fire
//! interval, damage, magazine size — [`Upgrades`]) and `settings\ammunition.bin` (how
//! much of each ammunition type can be carried — [`ammunition`]).

use crate::hash::hash_str;
use crate::property::{Node, PropertyFile, Variant};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Weapon {
    pub id: i32,
    pub name: String,
    /// `#94142e13`: the short name for the HUD (`Beretta`, `Scorpion SMG`).
    pub short_name: String,
    /// The model (`Weapons\WEA02\WEA02_LOD1-WEA02.rbm`) or its `.lod`.
    pub model: String,
    /// `#be0ec6e0`: 13 (shotgun pellet) … 19 (Beretta) … 300 (sniper) … 1000 (rocket).
    pub damage: f32,
    /// `ammo_max_range` (m).
    pub range: f32,
    /// `ammo_velocity` (m/s): 750 for bullets, 2000 sniper, 60–100 grenades and rockets.
    pub velocity: f32,
    /// `#58b8bb06`: 10 on the pump shotgun, 1 elsewhere — pellets per shot.
    pub pellets: u32,
    /// `#84ecc4af`: 0.01 … 400 (sniper) — the push on what it hits (assumed).
    pub force: f32,
    /// `weapon_accuracy`.
    pub accuracy: f32,
    /// Native settings+2ec; aim_type1 bypasses weapon_accuracy for the player bound.
    pub aim_type: i32,
    /// `max_bullet_spread`, degrees in the file; independent of character accuracy.
    pub max_bullet_spread_degrees: f32,
    /// `#e56e25f4`: 10 for bullets, 4 rockets, 61 grenades, 246 vehicle rockets — the
    /// projectile kind (assumed).
    pub projectile: i32,
    /// `reloadtime` (s): 0.8 on the hand guns, 1.1 rifles, 1.2 the pump shotgun.
    pub reload_time: f32,
    /// `#a0d58b16`: the ammunition type, an index into `ammunition.bin`'s `max_ammo_N`
    /// (1 Beretta, 2 SMG, 7 rockets — shared by both rocket launchers; assumed from
    /// the values).
    pub ammo_type: i32,
    /// `unlimited_ammo`: 1 on the minigun.
    pub unlimited_ammo: bool,
    /// `#01dbd94a`: 1 on the SMG, the rifles, the machine guns and the miniguns, 0 on
    /// the pistols, shotguns and launchers — read as "fires while held" (assumed).
    pub automatic: bool,
    /// `#9acc62e9`: how the AI uses it (absent on the grappling hook).
    pub ai: Option<AiUse>,
    /// `#62d2f184`: the hand modifier set (`WEAPON_MODIFIER_DUALWIELD_PISTOL`,
    /// `WEAPON_MODIFIER_DUALWIELD_SMG`, `WEAPON_MODIFIER_1H_SHOTGUN`,
    /// `DUAL_WEAPON_MODIFIER_GRENADE_LAUNCHER`; empty on the two-handed guns): one-handed
    /// weapons can be wielded in either hand.
    pub hand: String,
}

/// The AI block of a weapon (`#9acc62e9`, tagged `AI_MODIFIER_PISTOL` and the like):
/// the keys found by hashing candidates.
#[derive(Clone, Debug, PartialEq)]
pub struct AiUse {
    /// `min_range`, `max_range` (m): the distances it engages at.
    pub min_range: f32,
    pub max_range: f32,
    /// `min_burst_delay`, `max_burst_delay` (s): the pause between bursts.
    pub min_burst_delay: f32,
    pub max_burst_delay: f32,
    /// `magazine_size`, on the hand guns (8 for the Beretta).
    pub magazine_size: Option<u32>,
    /// `min_burst_length`, `max_burst_length` (s): how long a burst lasts (0–0.1 on
    /// the pistols — a shot or two —, 1–2 s on the SMG and the rifle, 2–4 s the MG).
    pub min_burst_length: f32,
    pub max_burst_length: f32,
    /// `firerate` (s between shots), on the hand guns only (0.05).
    pub fire_interval: Option<f32>,
    /// `#4fdb09b7`: the AI's weapon class (`AI_MODIFIER_PISTOL`, `AI_MODIFIER_SMG`,
    /// `AI_MODIFIER_SHOTGUN`, `AI_MODIFIER_2H`, `AI_MODIFIER_SNIPER`,
    /// `AI_MODIFIER_GRENADELAUNCHER`, `AI_MODIFIER_RPG`; `WEAPON_MODIFIER_RPG` on the
    /// Panay launcher).
    pub modifier: String,
}

impl AiUse {
    fn from_node(n: &Node) -> Self {
        Self {
            min_range: n.float("min_range").unwrap_or(0.0),
            max_range: n.float("max_range").unwrap_or(50.0),
            min_burst_delay: n.float("min_burst_delay").unwrap_or(0.5),
            max_burst_delay: n.float("max_burst_delay").unwrap_or(1.5),
            magazine_size: n.float("magazine_size").map(|m| m as u32),
            min_burst_length: n.float("min_burst_length").unwrap_or(0.0),
            max_burst_length: n.float("max_burst_length").unwrap_or(0.0),
            fire_interval: n.float("firerate"),
            modifier: match n.value_hash(AI_MODIFIER) {
                Some(Variant::Str(s)) => s.clone(),
                _ => String::new(),
            },
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

const AI_BLOCK: u32 = 0x9acc_62e9;
const SHORT_NAME: u32 = 0x9414_2e13;
const AMMO_TYPE: u32 = 0xa0d5_8b16;
const AUTOMATIC: u32 = 0x01db_d94a;
const HAND_MODIFIER: u32 = 0x62d2_f184;
const AI_MODIFIER: u32 = 0x4fdb_09b7;

impl Weapon {
    /// The name for the HUD: the short name, else the table name.
    pub fn label(&self) -> &str {
        if self.short_name.is_empty() {
            &self.name
        } else {
            &self.short_name
        }
    }
}

impl Weapon {
    fn from_node(n: &Node) -> Option<Self> {
        Some(Self {
            id: n.int("id")?,
            name: n.str("name")?.to_string(),
            short_name: match n.value_hash(SHORT_NAME) {
                Some(Variant::Str(s)) => s.clone(),
                _ => String::new(),
            },
            model: n.str("model").unwrap_or("").to_string(),
            damage: num(n, 0xbe0e_c6e0).unwrap_or(10.0),
            range: n.float("ammo_max_range").unwrap_or(100.0),
            velocity: n.float("ammo_velocity").unwrap_or(750.0),
            pellets: num(n, 0x58b8_bb06).unwrap_or(1.0).max(1.0) as u32,
            force: num(n, 0x84ec_c4af).unwrap_or(1.0),
            accuracy: n.float("weapon_accuracy").unwrap_or(1.0),
            aim_type: n.int("aim_type").unwrap_or(0),
            max_bullet_spread_degrees: n.float("max_bullet_spread").unwrap_or(0.0),
            projectile: num(n, 0xe56e_25f4).unwrap_or(10.0) as i32,
            reload_time: n.float("reloadtime").unwrap_or(1.0),
            ammo_type: num(n, AMMO_TYPE).unwrap_or(0.0) as i32,
            unlimited_ammo: n.float("unlimited_ammo").unwrap_or(0.0) != 0.0,
            automatic: num(n, AUTOMATIC).unwrap_or(0.0) != 0.0,
            ai: n
                .children
                .iter()
                .find(|(k, _)| k.hash() == AI_BLOCK)
                .map(|(_, c)| AiUse::from_node(c)),
            hand: match n.value_hash(HAND_MODIFIER) {
                Some(Variant::Str(s)) => s.clone(),
                _ => String::new(),
            },
        })
    }
}

/// Every weapon of the file.
pub fn parse(data: &[u8]) -> crate::Result<Vec<Weapon>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if n.value("id").is_some()
                && n.value("name").is_some()
                && let Some(w) = Weapon::from_node(n)
            {
                out.push(w);
            }
        });
    }
    Ok(out)
}

/// A thrown or placed explosive: the `#a17fc6ed` list (`CHandGrenade`,
/// `CTriggeredExplosive`, `CFlare`), keyed by string ids (`WEA33`). The explosion values
/// sit in per-upgrade sub-nodes (`level_0` … `level_5`); these are level 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Throwable {
    pub id: String,
    pub class: String,
    /// `model_name`.
    pub model: String,
    /// `damage` at the centre.
    pub damage: f32,
    /// `impulse`: the push of the blast.
    pub impulse: f32,
    /// `range` (m): the blast radius.
    pub range: f32,
    /// `detonation_time` (s): the fuse (1 on the triggered charge, which waits for
    /// its trigger).
    pub detonation_time: f32,
    /// `speed` (m/s): how hard it is thrown; `max_speed` caps it.
    pub speed: f32,
    pub max_speed: f32,
    /// `#7b655f19`, per level (10 … 20 on the grenade): read as how many can be
    /// carried (assumed).
    pub max_count: u32,
}

fn level0(n: &Node, name: &str) -> Option<f32> {
    n.child(name)?.float("level_0")
}

impl Throwable {
    fn from_node(n: &Node) -> Option<Self> {
        Some(Self {
            id: n.str("id")?.to_string(),
            class: n.str("_class")?.to_string(),
            model: n.str("model_name").unwrap_or("").to_string(),
            damage: level0(n, "damage").unwrap_or(0.0),
            impulse: level0(n, "impulse").unwrap_or(0.0),
            range: level0(n, "range")
                .or_else(|| n.float("radius"))
                .unwrap_or(0.0),
            detonation_time: n.float("detonation_time").unwrap_or(0.0),
            speed: n.float("speed").unwrap_or(0.0),
            max_speed: n.float("max_speed").unwrap_or(0.0),
            max_count: n
                .children
                .iter()
                .find(|(k, _)| k.hash() == MAX_COUNT)
                .and_then(|(_, c)| c.float("level_0"))
                .unwrap_or(0.0) as u32,
        })
    }
}

const MAX_COUNT: u32 = 0x7b65_5f19;

/// The grenades, charges and flares of the file.
pub fn throwables(data: &[u8]) -> crate::Result<Vec<Throwable>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if matches!(n.value("id"), Some(Variant::Str(_)))
                && let Some(t) = Throwable::from_node(n)
            {
                out.push(t);
            }
        });
    }
    Ok(out)
}

/// Native CWeaponSettings inputs retained under their evidenced raw hashes.
pub const AIM_SPREAD: u32 = 0x6a1f_b54c;
pub const ACCURACY_LOSS: u32 = 0x52aa_95ac;
pub const PLAYER_RECOVERY: u32 = 0x4f5a_b59d;
pub const PLAYER_SETTINGS: u32 = 0xa0f5_dc0e;

fn hashed_float(n: &Node, hash: u32) -> Option<f32> {
    n.values
        .iter()
        .find(|(k, _)| k.hash() == hash)
        .and_then(|(_, v)| match v {
            Variant::Float(f) => Some(*f),
            _ => None,
        })
}

fn hashed_level(n: &Node, hash: u32, level: usize) -> Option<f32> {
    n.children
        .iter()
        .find(|(k, _)| k.hash() == hash)
        .and_then(|(_, n)| n.float(&format!("level_{level}")))
}

/// CPlayer configuration consumed by 00788450, within the exact player root.
/// Default0.5 is native fallback; retail player.bin overrides it to5.0.
pub fn player_accuracy_recovery(data: &[u8]) -> crate::Result<f32> {
    let file = PropertyFile::parse_any(data)?;
    let node = file
        .nodes
        .iter()
        .flat_map(|n| &n.children)
        .find(|(k, _)| k.hash() == PLAYER_SETTINGS)
        .map(|(_, n)| n);
    Ok(node
        .and_then(|n| hashed_float(n, PLAYER_RECOVERY))
        .unwrap_or(0.5))
}

/// One upgrade level of a weapon in `settings\weapon_upgrades.bin` (every key named by
/// hashing: `firerate`, `damage`, `damage_max`, `impulse`, `magazinesize`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Upgrade {
    /// `firerate`: seconds between shots (Beretta 0.12, SMG 0.0667, pump 0.325).
    pub fire_interval: f32,
    pub damage: f32,
    pub damage_max: f32,
    pub impulse: f32,
    /// `magazinesize`: rounds in a magazine (Beretta 12, SMG 30, Assault 20, rockets 3;
    /// 1000 on vehicle guns, 0 on some launchers).
    pub magazine: u32,
    /// Raw #6a1fb54c; native settings loader converts these degrees to radians.
    pub aim_spread_degrees: f32,
    /// Raw #52aa95ac; dimensionless accuracy loss once per fired round.
    pub accuracy_loss: f32,
}

/// `settings\weapon_upgrades.bin`: under the root, one node per weapon keyed by the hash
/// of its `weapons.bin` name (`Beretta Handgun`), each value a `level_0` … `level_5`
/// node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Upgrades {
    by_name: HashMap<u32, Vec<Upgrade>>,
}

impl Upgrades {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut by_name = HashMap::new();
        let lv = |n: &Node, key: &str, i: usize| {
            n.child(key)
                .and_then(|c| c.float(&format!("level_{i}")))
                .unwrap_or(0.0)
        };
        for root in &file.nodes {
            root.walk(&mut |n| {
                for (k, w) in &n.children {
                    if w.child("magazinesize").is_none() || w.child("firerate").is_none() {
                        continue;
                    }
                    let levels = (0..6)
                        .map(|i| Upgrade {
                            fire_interval: lv(w, "firerate", i),
                            damage: lv(w, "damage", i),
                            damage_max: lv(w, "damage_max", i),
                            impulse: lv(w, "impulse", i),
                            magazine: lv(w, "magazinesize", i).max(0.0) as u32,
                            aim_spread_degrees: hashed_level(w, AIM_SPREAD, i).unwrap_or(0.0),
                            accuracy_loss: hashed_level(w, ACCURACY_LOSS, i).unwrap_or(0.0),
                        })
                        .collect();
                    by_name.insert(k.hash(), levels);
                }
            });
        }
        Ok(Self { by_name })
    }

    /// A weapon's values at an upgrade level (0 … 5), by its `weapons.bin` name.
    pub fn get(&self, name: &str, level: usize) -> Option<Upgrade> {
        let l = self.by_name.get(&hash_str(name))?;
        l.get(level.min(l.len().saturating_sub(1))).copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// `settings\ammunition.bin`: `ammunition { max_ammo_0 … max_ammo_19 }`, the most of each
/// ammunition type ([`Weapon::ammo_type`]) Rico can carry — counted in magazines, as the
/// values (15 for the Beretta, 4 rockets, 9 for the rifle) only make sense that way
/// (assumed). Index `i` is `max_ammo_i`.
pub fn ammunition(data: &[u8]) -> crate::Result<Vec<u32>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if out.is_empty() && n.value("max_ammo_0").is_some() {
                while let Some(v) = n.float(&format!("max_ammo_{}", out.len())) {
                    out.push(v.max(0.0) as u32);
                }
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// Needs JC2_TEST_DIR with `weapons.bin` in the dropzone.
    #[test]
    fn arsenal() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("weapons.bin") else {
            return;
        };
        let w = super::parse(&data).unwrap();
        assert!(w.len() > 40, "{}", w.len());
        let b = w.iter().find(|w| w.name == "Beretta Handgun").unwrap();
        assert_eq!(b.id, 2);
        assert_eq!(b.damage, 19.0);
        assert_eq!(b.range, 100.0);
        assert_eq!(b.velocity, 750.0);
        assert!(b.model.ends_with("WEA02_LOD1-WEA02.rbm"));
        let s = w
            .iter()
            .find(|w| w.name == "Shotgun (pump action)")
            .unwrap();
        assert_eq!(s.pellets, 10);
        let ai = b.ai.as_ref().unwrap();
        assert_eq!((ai.max_range, ai.magazine_size), (50.0, Some(8)));
        assert_eq!((ai.min_burst_delay, ai.max_burst_delay), (0.15, 0.3));
        let rifle = w.iter().find(|w| w.name == "Assault").unwrap();
        assert_eq!(rifle.damage, 32.0);
        assert_eq!(rifle.ai.as_ref().unwrap().max_burst_delay, 1.5);
        let t = super::throwables(&data).unwrap();
        let g = t.iter().find(|t| t.id == "WEA33").unwrap();
        assert_eq!(g.class, "CHandGrenade");
        assert_eq!((g.damage, g.impulse, g.range), (500.0, 1000.0, 5.0));
        assert_eq!((g.detonation_time, g.speed), (3.5, 40.0));
        assert!(g.model.ends_with("WEA33_LOD1-WEA33.rbm"));
        let c4 = t.iter().find(|t| t.id == "WEA35").unwrap();
        assert_eq!((c4.damage, c4.range), (750.0, 6.25));
        assert_eq!(g.max_count, 10);
        // reloads, ammunition types, which fire while held
        assert_eq!((b.reload_time, b.ammo_type, b.automatic), (0.8, 1, false));
        assert_eq!(b.label(), "Beretta");
        let smg = w.iter().find(|w| w.name == "Scorpion_SMG").unwrap();
        assert_eq!(
            (smg.reload_time, smg.ammo_type, smg.automatic),
            (1.1, 2, true)
        );
        assert_eq!((rifle.ammo_type, rifle.automatic), (8, true));
        let rl = w.iter().find(|w| w.name == "RocketLauncher").unwrap();
        let prl = w.iter().find(|w| w.name == "PanayRocketLauncher").unwrap();
        assert_eq!((rl.ammo_type, prl.ammo_type), (7, 7));
        assert!(
            w.iter()
                .find(|w| w.name == "Minigun")
                .unwrap()
                .unlimited_ammo
        );
        let sai = smg.ai.as_ref().unwrap();
        assert_eq!((sai.min_burst_length, sai.max_burst_length), (1.0, 2.0));
        assert_eq!(ai.fire_interval, Some(0.05));
        // the hand and AI modifiers: which pose set each gun is wielded with
        assert_eq!(b.hand, "WEAPON_MODIFIER_DUALWIELD_PISTOL");
        assert_eq!(ai.modifier, "AI_MODIFIER_PISTOL");
        assert_eq!(smg.hand, "WEAPON_MODIFIER_DUALWIELD_SMG");
        assert_eq!((s.hand.as_str(), rifle.hand.as_str()), ("", ""));
        assert_eq!(rifle.ai.as_ref().unwrap().modifier, "AI_MODIFIER_2H");
        assert_eq!(rl.ai.as_ref().unwrap().modifier, "AI_MODIFIER_RPG");
    }

    /// Needs JC2_TEST_DIR with `weapon_upgrades.bin` and `ammunition.bin`.
    #[test]
    fn upgrades_and_ammunition() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(u), Ok(a)) = (
            fs.read_inflated("weapon_upgrades.bin"),
            fs.read_inflated("ammunition.bin"),
        ) else {
            return;
        };
        let u = super::Upgrades::parse(&u).unwrap();
        assert!(u.len() > 30, "{}", u.len());
        let b = u.get("Beretta Handgun", 0).unwrap();
        assert_eq!((b.magazine, b.fire_interval, b.damage), (12, 0.12, 16.0));
        assert_eq!(u.get("Beretta Handgun", 5).unwrap().magazine, 22);
        assert_eq!(u.get("Scorpion_SMG", 0).unwrap().magazine, 30);
        assert_eq!(u.get("Assault", 0).unwrap().magazine, 20);
        assert_eq!(u.get("Shotgun (pump action)", 0).unwrap().magazine, 6);
        assert_eq!(u.get("RocketLauncher", 0).unwrap().magazine, 3);
        let a = super::ammunition(&a).unwrap();
        assert_eq!(a.len(), 20);
        assert_eq!((a[0], a[1], a[2], a[7], a[8]), (0, 15, 7, 4, 9));
    }
    #[test]
    fn authored_accuracy_retail_units_and_player_scope() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let upgrades =
            super::Upgrades::parse(&fs.read_inflated("weapon_upgrades.bin").unwrap()).unwrap();
        for (name, angles, loss) in [
            ("Beretta Handgun", [2.2, 1.96, 1.72, 1.48, 1.24, 1.0], 0.07),
            ("Assault", [2.5, 2.3, 2.1, 1.9, 1.7, 1.5], 0.07),
            ("RocketLauncher", [0.0; 6], 0.0),
        ] {
            for (level, angle) in angles.into_iter().enumerate() {
                let up = upgrades.get(name, level).unwrap();
                assert!(
                    (up.aim_spread_degrees - angle).abs() < 1e-6,
                    "{name}/{level}"
                );
                assert!((up.accuracy_loss - loss).abs() < 1e-6);
            }
        }
        let weapons = super::parse(&fs.read_inflated("weapons.bin").unwrap()).unwrap();
        let shotgun = weapons
            .iter()
            .find(|w| w.name == "Shotgun (pump action)")
            .unwrap();
        assert_eq!(shotgun.max_bullet_spread_degrees, 1.0);
        let launcher = weapons.iter().find(|w| w.name == "RocketLauncher").unwrap();
        assert_eq!(
            (launcher.aim_type, launcher.max_bullet_spread_degrees),
            (1, 0.0)
        );
        assert_eq!(
            super::player_accuracy_recovery(&fs.read_inflated("player.bin").unwrap()).unwrap(),
            5.0
        );
        // A recovery-like property outside the native player root cannot override it.
        let bogus = crate::property::PropertyFile {
            nodes: vec![crate::property::Node {
                values: vec![(
                    crate::property::Key::Hash(super::PLAYER_RECOVERY),
                    crate::property::Variant::Float(999.0),
                )],
                ..Default::default()
            }],
        };
        assert_eq!(
            super::player_accuracy_recovery(&bogus.to_bytes()).unwrap(),
            0.5
        );
    }
}
