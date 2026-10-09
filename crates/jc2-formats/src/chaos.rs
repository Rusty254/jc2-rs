//! Chaos: what sabotage is worth and how the game's progress is staged.
//!
//! * `settings\achievements.bin` → `statistics { activities { N { type, money, chaos,
//!   title, sabotage, text } } }`: one entry per **activity type**, the number the
//!   location files fire as `gsy.activity.typeNN` when something is destroyed (a gas
//!   pump is 11: +100 chaos, $500, "Gas Pump"). `title` is a text-database key
//!   (`act_type_txt_11`) or, for a few unlocalised entries, the English itself
//!   ("Military Vehicle Destroyed"); `text` the message some of them show ("Military
//!   communications has been disrupted."); `sabotage` is 1 for the government
//!   installations.
//! * `settings\aidifficulty.bin` → `chaos_levels { chaos_00 … chaos_04 { name, max_chaos } }`:
//!   the five stages of the game by chaos points (100 000, 200 000, 300 000, 400 000, then
//!   99 999 999); the same `chaos_NN` keys name the stages of
//!   `enemytypespawnsettings.bin`.
//!
//! Every key was named by hashing candidates (`research/formats/chaos.md`).

use crate::property::{Node, PropertyFile, Variant};
use std::collections::BTreeMap;

/// One activity type: what destroying (or doing) it gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// `type`: the number in `gsy.activity.typeNN`.
    pub kind: u32,
    pub money: u32,
    pub chaos: u32,
    /// `title`: a text key (`act_type_txt_11`) or plain English.
    pub title: String,
    /// `text`: a message key (`act_message_txt_02`), shown for the big ones.
    pub message: Option<String>,
    /// `sabotage`: a government installation (counts as sabotage).
    pub sabotage: bool,
}

/// `achievements.bin`'s activity table, by type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activities(pub BTreeMap<u32, Activity>);

fn num(n: &Node, key: &str) -> Option<f32> {
    match n.value(key)? {
        Variant::Int(i) => Some(*i as f32),
        Variant::Float(f) => Some(*f),
        _ => None,
    }
}

impl Activities {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = BTreeMap::new();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(acts) = n.child("activities") {
                    for (_, a) in &acts.children {
                        let Some(kind) = num(a, "type") else { continue };
                        let kind = kind as u32;
                        out.insert(
                            kind,
                            Activity {
                                kind,
                                money: num(a, "money").unwrap_or(0.0).max(0.0) as u32,
                                chaos: num(a, "chaos").unwrap_or(0.0).max(0.0) as u32,
                                title: a.str("title").unwrap_or("").to_string(),
                                message: a.str("text").map(str::to_string),
                                sabotage: num(a, "sabotage").unwrap_or(0.0) != 0.0,
                            },
                        );
                    }
                }
            });
        }
        Ok(Self(out))
    }

    pub fn get(&self, kind: u32) -> Option<&Activity> {
        self.0.get(&kind)
    }
}

/// The game's stages by chaos (`aidifficulty.bin`'s `chaos_levels`): each stage runs up
/// to its `max_chaos`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChaosLevels {
    /// (`name`, `max_chaos`), in order.
    pub levels: Vec<(String, u32)>,
}

impl ChaosLevels {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut levels = Vec::new();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(c) = n.child("chaos_levels") {
                    for (_, l) in &c.children {
                        if let Some(m) = num(l, "max_chaos") {
                            levels.push((l.str("name").unwrap_or("").to_string(), m as u32));
                        }
                    }
                }
            });
        }
        levels.sort_by_key(|l| l.1);
        Ok(Self { levels })
    }

    /// The stage (0 …) a chaos total is in.
    pub fn stage(&self, chaos: u32) -> usize {
        self.levels
            .iter()
            .position(|l| chaos < l.1)
            .unwrap_or(self.levels.len().saturating_sub(1))
    }

    /// Where the stage that `chaos` is in starts and ends.
    pub fn bounds(&self, chaos: u32) -> (u32, u32) {
        let s = self.stage(chaos);
        let lo = if s == 0 { 0 } else { self.levels[s - 1].1 };
        (lo, self.levels.get(s).map_or(u32::MAX, |l| l.1))
    }
}

/// The activity type in an event string (`gsy.activity.type11`, `gsy.activity.type04`).
pub fn activity_in(events: &str) -> Option<u32> {
    events
        .split_whitespace()
        .find_map(|t| t.strip_prefix("gsy.activity.type")?.parse().ok())
}

/// The settlement counter an event string bumps (`31x14.count.up` → `31x14`). The
/// strongholds' targets fire their own counters instead (`f1s07.CountUp`,
/// `f1s02.incree`: `icons_global.bin`'s `#12252e33` of the stronghold, `missions.md`):
/// those come back whole.
pub fn counter_in(events: &str) -> Option<&str> {
    events.split_whitespace().find_map(|t| {
        if let Some(s) = t.strip_suffix(".count.up") {
            return (!s.is_empty()).then_some(s);
        }
        let l = t.to_ascii_lowercase();
        (l.ends_with(".countup") || l.ends_with(".incree")).then_some(t)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_tokens() {
        let e = "e{go098b.10x10({85CD})}::destroyed gsy.activity.type14 31x14.count.up";
        assert_eq!(activity_in(e), Some(14));
        assert_eq!(counter_in(e), Some("31x14"));
        assert_eq!(activity_in("gsy.activity.type04 32x14.count.up"), Some(4));
        assert_eq!(activity_in("gsy.exit.settlement"), None);
        assert_eq!(counter_in("t{x}::sht.pickup.enter"), None);
        assert_eq!(
            counter_in("gsy.activity.type22 f1s07.CountUp"),
            Some("f1s07.CountUp")
        );
        assert_eq!(counter_in("f2s04.incree"), Some("f2s04.incree"));
    }

    /// Needs JC2_TEST_DIR with `achievements.bin` and `aidifficulty.bin`.
    #[test]
    fn activities_and_stages() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(a), Ok(d)) = (
            fs.read_inflated("achievements.bin"),
            fs.read_inflated("aidifficulty.bin"),
        ) else {
            return;
        };
        let a = Activities::parse(&a).unwrap();
        assert_eq!(a.0.len(), 31);
        let pump = a.get(11).unwrap();
        assert_eq!((pump.chaos, pump.money), (100, 500));
        assert_eq!(pump.title, "act_type_txt_11");
        assert!(pump.sabotage);
        let generator = a.get(17).unwrap();
        assert_eq!((generator.chaos, generator.money), (250, 1250));
        let radar = a.get(15).unwrap();
        assert_eq!((radar.chaos, radar.money), (250, 1250));
        let mast = a.get(20).unwrap();
        assert_eq!((mast.chaos, mast.money), (1000, 5000));
        assert_eq!(mast.message.as_deref(), Some("act_message_txt_06"));
        let small = a.get(13).unwrap();
        assert_eq!((small.chaos, small.money, small.sabotage), (10, 50, false));
        assert_eq!(a.get(23).unwrap().title, "Military Vehicle Destroyed");
        let l = ChaosLevels::parse(&d).unwrap();
        assert_eq!(l.levels.len(), 5);
        assert_eq!(l.levels[0], ("chaos_00".to_string(), 100_000));
        assert_eq!(l.stage(0), 0);
        assert_eq!(l.stage(99_999), 0);
        assert_eq!(l.stage(100_000), 1);
        assert_eq!(l.stage(450_000), 4);
        assert_eq!(l.bounds(150_000), (100_000, 200_000));
    }
}
