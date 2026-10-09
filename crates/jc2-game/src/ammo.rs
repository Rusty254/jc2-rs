//! Ammunition: a gun's magazine and reserve, firing and reloading. The sizes are the
//! game's: `magazinesize` from `weapon_upgrades.bin` (at the gun's upgrade level), the reserve from
//! `ammunition.bin`'s `max_ammo_N` for the gun's ammunition type (counted in magazines),
//! the reload time from `weapons.bin`'s `reloadtime` (`research/formats/weapons.md`).

use jc2_formats::weapons::{Upgrades, Weapon};

/// One gun's rounds.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponState {
    /// Rounds in the magazine.
    pub magazine: u32,
    /// Rounds carried besides it.
    pub reserve: u32,
    pub magazine_size: u32,
    pub max_reserve: u32,
    /// Seconds a reload takes.
    pub reload_time: f32,
    /// Seconds left of the reload under way.
    pub reloading: Option<f32>,
    /// Never runs out (the minigun's `unlimited_ammo`).
    pub unlimited: bool,
}

/// A magazine size for guns whose upgrade table says nothing useful (0, or the vehicle
/// guns' 1000).
const FALLBACK_MAGAZINE: u32 = 10;

impl WeaponState {
    pub fn new(magazine_size: u32, max_reserve: u32, reload_time: f32) -> Self {
        let magazine_size = magazine_size.max(1);
        Self {
            magazine: magazine_size,
            reserve: max_reserve,
            magazine_size,
            max_reserve,
            reload_time: reload_time.max(0.0),
            reloading: None,
            unlimited: false,
        }
    }

    /// A full gun, sized from the data: the magazine from the upgrade table (else the AI
    /// block's `magazine_size`), the reserve `max_ammo[ammo_type]` magazines, at an upgrade
    /// level (0 … 5).
    pub fn for_weapon_at(w: &Weapon, upgrades: &Upgrades, max_ammo: &[u32], level: usize) -> Self {
        let mag = upgrades
            .get(&w.name, level)
            .map(|u| u.magazine)
            .filter(|m| (1..1000).contains(m))
            .or_else(|| w.ai.as_ref().and_then(|a| a.magazine_size))
            .unwrap_or(FALLBACK_MAGAZINE);
        let mags = usize::try_from(w.ammo_type)
            .ok()
            .and_then(|t| max_ammo.get(t).copied())
            .unwrap_or(4);
        let mut s = Self::new(mag, mags * mag.max(1), w.reload_time);
        s.unlimited = w.unlimited_ammo;
        s
    }

    pub fn can_fire(&self) -> bool {
        self.reloading.is_none() && self.magazine > 0
    }

    /// Spend a round. False (and nothing spent) when empty or reloading.
    pub fn fire(&mut self) -> bool {
        if !self.can_fire() {
            return false;
        }
        if !self.unlimited {
            self.magazine -= 1;
        }
        true
    }

    /// Start reloading, when there is room in the magazine and rounds to put in it.
    pub fn start_reload(&mut self) -> bool {
        if self.reloading.is_some()
            || self.magazine >= self.magazine_size
            || (self.reserve == 0 && !self.unlimited)
        {
            return false;
        }
        self.reloading = Some(self.reload_time);
        true
    }

    /// Time passes: a reload under way finishes, an empty magazine starts one.
    pub fn tick(&mut self, dt: f32) {
        if let Some(t) = self.reloading.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.reloading = None;
                let want = self.magazine_size - self.magazine;
                let take = if self.unlimited {
                    want
                } else {
                    want.min(self.reserve)
                };
                self.magazine += take;
                if !self.unlimited {
                    self.reserve -= take;
                }
            }
        } else if self.magazine == 0 {
            self.start_reload();
        }
    }

    /// The gun is upgraded to a new magazine size (`max_reserve` carried rounds): the
    /// magazine keeps its rounds (topped up by what the bigger one holds more, from the
    /// reserve), the reserve is re-sized.
    pub fn resize(&mut self, magazine_size: u32, max_reserve: u32) {
        let magazine_size = magazine_size.max(1);
        let grow = magazine_size.saturating_sub(self.magazine_size);
        self.magazine_size = magazine_size;
        self.max_reserve = max_reserve;
        let take = grow.min(self.reserve);
        self.reserve -= take;
        self.magazine = (self.magazine + take).min(magazine_size);
        self.reserve = self.reserve.min(max_reserve);
    }

    /// Ammunition picked up: into the reserve, up to what can be carried. The rounds
    /// that fit.
    pub fn add(&mut self, rounds: u32) -> u32 {
        let room = self.max_reserve.saturating_sub(self.reserve);
        let take = rounds.min(room);
        self.reserve += take;
        take
    }

    /// Out of rounds altogether.
    pub fn is_empty(&self) -> bool {
        !self.unlimited && self.magazine == 0 && self.reserve == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Beretta's numbers: 12 rounds, 15 magazines in reserve, 0.8 s to reload.
    #[test]
    fn fires_a_magazine_then_reloads() {
        let mut w = WeaponState::new(12, 15 * 12, 0.8);
        assert_eq!((w.magazine, w.reserve), (12, 180));
        for _ in 0..12 {
            assert!(w.fire());
            w.tick(0.01);
        }
        // empty: the next tick starts the reload, and nothing fires during it
        assert_eq!(w.magazine, 0);
        assert!(w.reloading.is_some());
        assert!(!w.fire());
        w.tick(0.5);
        assert!(!w.fire(), "still reloading");
        assert_eq!(w.reserve, 180);
        w.tick(0.31);
        assert_eq!(w.reloading, None);
        assert_eq!((w.magazine, w.reserve), (12, 168));
        assert!(w.fire());
        // `R` with a part-empty magazine tops it up
        w.fire();
        assert!(w.start_reload());
        assert!(!w.start_reload(), "already reloading");
        w.tick(0.8);
        assert_eq!((w.magazine, w.reserve), (12, 166));
        // a full magazine does not reload
        assert!(!w.start_reload());
    }

    #[test]
    fn runs_dry_and_picks_up() {
        let mut w = WeaponState::new(3, 4, 0.5);
        let mut shots = 0;
        for _ in 0..200 {
            if w.fire() {
                shots += 1;
            }
            w.tick(0.1);
        }
        assert_eq!(shots, 7);
        assert!(w.is_empty());
        assert!(!w.start_reload());
        // a pickup fills the reserve, never past what can be carried
        assert_eq!(w.add(100), 4);
        assert_eq!(w.reserve, 4);
        w.tick(0.1);
        w.tick(0.5);
        assert_eq!((w.magazine, w.reserve), (3, 1));
    }

    #[test]
    fn unlimited_never_runs_out() {
        let mut w = WeaponState::new(5, 0, 1.0);
        w.unlimited = true;
        for _ in 0..50 {
            assert!(w.fire());
            w.tick(0.01);
        }
        assert_eq!(w.magazine, 5);
    }
}
