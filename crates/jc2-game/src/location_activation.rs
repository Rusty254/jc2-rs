//! Native camera-driven activation of world locations (JC2-144), a pure model of the
//! location manager's per-frame pass (research/formats/location-activation-next.md,
//! JC2-119; retail `JustCause2.exe` SHA-256 `df99a479...b74f80`).
//!
//! Every registry location has two records, registered far first and then near
//! (`0x60e7d0`). Each pass, in this order (`0x5df830`):
//!
//! 1. poll the in-flight record (`0x520070`);
//! 2. unload (`0x7f7440`): the first in-flight record, else the last-loaded loaded record,
//!    whose raw box no longer strictly contains the camera in XZ; **one per pass**, and while
//!    it is being released no load starts;
//! 3. load (`0x584d50` -> `0x7ed240`): only when nothing is in flight, the records are
//!    scanned from the last to the first and the camera must be strictly inside a record's box
//!    shrunk by [`LOAD_MARGIN`] (XZ only). Near beats far; otherwise the record whose nearest
//!    box face (3-D, `0xa30ab0`) is farthest from the camera wins, ties to the earlier
//!    record (`0x620510`). **One start per pass.**
//!
//! The model owns no game state: the host (`locations.rs`) feeds back when a started record
//! has finished loading ([`ActivationState::finish_load`]), failed
//! ([`ActivationState::fail`]) or finished releasing ([`ActivationState::finish_unload`]).
//! The native work budget, asynchronous IO and CLocation load states 0..3 are not decoded
//! (brief Gap 2), so the host decides when "loaded" is reached.
use jc2_formats::location::{
    LocationRegistry, RecordKind, keep_contains_xz, load_contains_xz, nearest_face_distance,
};
use std::collections::HashSet;

/// One spatial record: the location's index in the registry and which of its two records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActivationKey {
    pub location: usize,
    pub kind: RecordKind,
}
impl ActivationKey {
    fn slot(self) -> usize {
        self.location * 2
            + match self.kind {
                RecordKind::Far => 0,
                RecordKind::Near => 1,
            }
    }
    fn from_slot(slot: usize) -> Self {
        Self {
            location: slot / 2,
            kind: if slot.is_multiple_of(2) {
                RecordKind::Far
            } else {
                RecordKind::Near
            },
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Rec {
    lo: [f32; 3],
    hi: [f32; 3],
    /// The host serves this record (`false` for tile containers, the disabled locations...).
    eligible: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// Begin loading this record (one per pass).
    Load(ActivationKey),
    /// Release this record (one per pass); no load starts until
    /// [`ActivationState::finish_unload`].
    Unload(ActivationKey),
}
#[derive(Clone, Debug, Default)]
pub struct ActivationState {
    /// Two records per location, in registration order (far, near).
    recs: Vec<Rec>,
    in_flight: Option<usize>,
    /// Finished records in load order.
    loaded: Vec<usize>,
    unloading: Option<usize>,
    /// Native: a record whose open failed gets flags bit 0 and is never a candidate again.
    failed: HashSet<usize>,
}
impl ActivationState {
    /// Builds the record list of `registry`; `eligible` selects the locations the host
    /// serves (their records can load; the others never are candidates).
    pub fn new(registry: &LocationRegistry, eligible: impl Fn(usize) -> bool) -> Self {
        let mut recs = Vec::with_capacity(registry.0.len() * 2);
        for (i, loc) in registry.0.iter().enumerate() {
            let ok = eligible(i) && loc.enabled_at_start;
            for kind in [RecordKind::Far, RecordKind::Near] {
                let (lo, hi) = loc.bounds(kind);
                recs.push(Rec {
                    lo,
                    hi,
                    eligible: ok,
                });
            }
        }
        Self {
            recs,
            ..Default::default()
        }
    }
    pub fn location_count(&self) -> usize {
        self.recs.len() / 2
    }
    pub fn in_flight(&self) -> Option<ActivationKey> {
        self.in_flight.map(ActivationKey::from_slot)
    }
    pub fn unloading(&self) -> Option<ActivationKey> {
        self.unloading.map(ActivationKey::from_slot)
    }
    #[cfg(test)]
    pub fn loaded(&self) -> impl Iterator<Item = ActivationKey> + '_ {
        self.loaded.iter().copied().map(ActivationKey::from_slot)
    }
    fn active(&self, slot: usize) -> bool {
        self.in_flight == Some(slot) || self.loaded.contains(&slot)
    }
    fn bounds(&self, slot: usize) -> ([f32; 3], [f32; 3]) {
        (self.recs[slot].lo, self.recs[slot].hi)
    }
    /// `0x7ed240` + `0x620510`: the record to start with the camera at `camera`, if any.
    pub fn pick(&self, camera: [f32; 3]) -> Option<ActivationKey> {
        let mut best: Option<usize> = None;
        for slot in (0..self.recs.len()).rev() {
            let r = &self.recs[slot];
            if !r.eligible
                || self.active(slot)
                || self.unloading == Some(slot)
                || self.failed.contains(&slot)
                || !load_contains_xz((r.lo, r.hi), camera[0], camera[2])
            {
                continue;
            }
            best = Some(match best {
                None => slot,
                Some(b) => self.prefer(b, slot, camera),
            });
        }
        best.map(ActivationKey::from_slot)
    }
    /// `0x620510(best, candidate)`: near beats far; otherwise the larger nearest-face
    /// distance wins and ties go to the candidate.
    fn prefer(&self, best: usize, candidate: usize, camera: [f32; 3]) -> usize {
        let near = |s: usize| s % 2 == 1;
        match (near(best), near(candidate)) {
            (true, false) => best,
            (false, true) => candidate,
            _ => {
                let a = nearest_face_distance(self.bounds(best), camera);
                let b = nearest_face_distance(self.bounds(candidate), camera);
                if a > b { best } else { candidate }
            }
        }
    }
    /// `0x7f7440`: the record to release, if any. The in-flight record is tested first, then
    /// the loaded ones from the last loaded back; the first that no longer strictly
    /// contains the camera in XZ (raw box, no margin).
    pub fn pick_unload(&self, camera: [f32; 3]) -> Option<ActivationKey> {
        self.in_flight
            .into_iter()
            .chain(self.loaded.iter().rev().copied())
            .find(|&s| {
                let (lo, hi) = self.bounds(s);
                !keep_contains_xz((lo, hi), camera[0], camera[2])
            })
            .map(ActivationKey::from_slot)
    }
    /// One pass: unload first (and block loads while one is pending), else start at most
    /// one load, and only when nothing is in flight.
    pub fn step(&mut self, camera: [f32; 3]) -> Step {
        if self.unloading.is_some() {
            return Step::Idle;
        }
        if let Some(key) = self.pick_unload(camera) {
            let slot = key.slot();
            if self.in_flight == Some(slot) {
                self.in_flight = None;
            }
            self.loaded.retain(|&s| s != slot);
            self.unloading = Some(slot);
            return Step::Unload(key);
        }
        if self.in_flight.is_some() {
            return Step::Idle;
        }
        match self.pick(camera) {
            Some(key) => {
                self.in_flight = Some(key.slot());
                Step::Load(key)
            }
            None => Step::Idle,
        }
    }
    /// The in-flight record reached state "loaded" (`0x520070`).
    pub fn finish_load(&mut self, key: ActivationKey) {
        if self.in_flight == Some(key.slot()) {
            self.in_flight = None;
            self.loaded.push(key.slot());
        }
    }
    /// The in-flight record could not be opened: dropped and never a candidate again.
    pub fn fail(&mut self, key: ActivationKey) {
        if self.in_flight == Some(key.slot()) {
            self.in_flight = None;
        }
        self.loaded.retain(|&s| s != key.slot());
        self.failed.insert(key.slot());
    }
    /// The pending release completed (`0x520070` after the CLocation left state 5).
    pub fn finish_unload(&mut self) {
        self.unloading = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::location::Location;
    use jc2_formats::property::Node;

    fn loc(name: &str, lo: [f32; 3], hi: [f32; 3]) -> Location {
        Location {
            identity: name.into(),
            lower: lo,
            upper: hi,
            raw: Node::default(),
            enabled_at_start: true,
        }
    }
    fn keys(order: &[ActivationKey]) -> Vec<(usize, RecordKind)> {
        order.iter().map(|k| (k.location, k.kind)).collect()
    }
    /// The probe's `activation_order`: every record the model starts with the camera held
    /// at `p`, each load finishing before the next.
    pub(crate) fn order(state: &mut ActivationState, p: [f32; 3]) -> Vec<ActivationKey> {
        let mut out = Vec::new();
        while let Step::Load(k) = state.step(p) {
            state.finish_load(k);
            out.push(k);
        }
        out
    }

    #[test]
    fn near_before_far_then_farthest_face_then_earlier_record() {
        // a: wide, camera near its middle; b: same footprint shifted so its faces are closer
        let reg = LocationRegistry(vec![
            loc("a", [0.0; 3], [4000.0; 3]),
            loc("b", [100.0; 3], [3900.0; 3]),
            loc("c", [0.0; 3], [4000.0; 3]),
        ]);
        let mut s = ActivationState::new(&reg, |_| true);
        let order = keys(&order(&mut s, [2000.0, 2000.0, 2000.0]));
        use RecordKind::{Far, Near};
        // near records first; a and c tie on the face distance (earlier record = a); b is
        // closer to its faces (1900 against 2000 of the near boxes at 1000/900 inset)
        assert_eq!(
            order,
            [
                (0, Near),
                (2, Near),
                (1, Near),
                (0, Far),
                (2, Far),
                (1, Far)
            ]
        );
    }
    #[test]
    fn y_is_ignored_by_the_gate_but_counts_in_the_tie_break() {
        let reg = LocationRegistry(vec![
            loc("a", [0.0, 0.0, 0.0], [4000.0, 4000.0, 4000.0]),
            loc("b", [0.0, 0.0, 0.0], [4000.0, 6000.0, 4000.0]),
        ]);
        let mut s = ActivationState::new(&reg, |_| true);
        // Y far outside both boxes still loads (XZ only)
        assert!(s.pick([2000.0, 90000.0, 2000.0]).is_some());
        // y = 2400: a's top face is 1600 away (near box tops at 3000: 600 away), b's near
        // box is y 1000..5000 so its nearest face is 1400 away: b wins
        let k = s.pick([2000.0, 2400.0, 2000.0]).unwrap();
        assert_eq!((k.location, k.kind), (1, RecordKind::Near));
        let _ = &mut s;
    }
    #[test]
    fn one_start_serialised_and_unloads_block_loads() {
        let reg = LocationRegistry(vec![
            loc("a", [0.0; 3], [4000.0; 3]),
            loc("b", [0.0; 3], [4000.0; 3]),
        ]);
        let mut s = ActivationState::new(&reg, |_| true);
        let p = [2000.0, 0.0, 2000.0];
        let Step::Load(first) = s.step(p) else {
            panic!()
        };
        // nothing else starts while the first is in flight
        assert_eq!(s.step(p), Step::Idle);
        assert_eq!(s.step(p), Step::Idle);
        s.finish_load(first);
        let Step::Load(second) = s.step(p) else {
            panic!()
        };
        s.finish_load(second);
        // move out of the first location's box only (a is [0,4000]; camera at 4005 keeps
        // nothing: both leave together, so the last loaded unloads first, one per pass)
        let out = [4005.0, 0.0, 2000.0];
        assert_eq!(s.step(out), Step::Unload(second));
        // a pending unload blocks loads and further unloads
        assert_eq!(s.step(out), Step::Idle);
        assert_eq!(s.step([2000.0, 0.0, 2000.0]), Step::Idle);
        s.finish_unload();
        assert_eq!(s.step(out), Step::Unload(first));
        s.finish_unload();
        assert_eq!(s.step(out), Step::Idle);
        assert_eq!(s.loaded().count(), 0);
    }
    #[test]
    fn hysteresis_is_the_ten_metre_band() {
        let reg = LocationRegistry(vec![loc("a", [0.0; 3], [4000.0; 3])]);
        let mut s = ActivationState::new(&reg, |_| true);
        // far box x: loads strictly inside (10, 3990), keeps inside (0, 4000)
        assert_eq!(s.pick([10.0, 0.0, 2000.0]), None);
        let k = s.pick([10.5, 0.0, 2000.0]).unwrap();
        assert_eq!(k.kind, RecordKind::Far);
        assert_eq!(s.step([10.5, 0.0, 2000.0]), Step::Load(k));
        s.finish_load(k);
        // inside the band: kept, but a second start there is refused (already active)
        assert_eq!(s.step([5.0, 0.0, 2000.0]), Step::Idle);
        assert_eq!(s.step([0.5, 0.0, 2000.0]), Step::Idle);
        assert_eq!(s.step([0.0, 0.0, 2000.0]), Step::Unload(k));
    }
    #[test]
    fn failed_records_never_retry_and_ineligible_ones_never_start() {
        let reg = LocationRegistry(vec![
            loc("a", [0.0; 3], [4000.0; 3]),
            loc("b", [0.0; 3], [4000.0; 3]),
        ]);
        let mut s = ActivationState::new(&reg, |i| i == 0);
        let p = [2000.0, 0.0, 2000.0];
        let Step::Load(k) = s.step(p) else { panic!() };
        assert_eq!((k.location, k.kind), (0, RecordKind::Near));
        s.fail(k);
        let Step::Load(k2) = s.step(p) else { panic!() };
        assert_eq!((k2.location, k2.kind), (0, RecordKind::Far));
        s.finish_load(k2);
        assert_eq!(s.step(p), Step::Idle);
    }
    /// An in-flight record that stops containing the camera is released first.
    #[test]
    fn an_in_flight_record_is_unloaded_before_loaded_ones() {
        use RecordKind::{Far, Near};
        let reg = LocationRegistry(vec![
            loc("a", [0.0; 3], [6000.0; 3]),
            loc("b", [4000.0, 0.0, 0.0], [10000.0, 6000.0, 6000.0]),
        ]);
        let mut s = ActivationState::new(&reg, |_| true);
        let both = [4500.0, 3000.0, 3000.0];
        let Step::Load(first) = s.step(both) else {
            panic!()
        };
        assert_eq!((first.location, first.kind), (0, Near));
        s.finish_load(first);
        // a's far box is the farthest-from-its-faces candidate (1500 against b's 500)
        let Step::Load(second) = s.step(both) else {
            panic!()
        };
        assert_eq!((second.location, second.kind), (0, Far));
        // `second` is in flight; the camera leaves every box of `a`
        let out = [6005.0, 3000.0, 3000.0];
        assert_eq!(s.step(out), Step::Unload(second));
        s.finish_unload();
        assert_eq!(s.step(out), Step::Unload(first));
    }

    /// The probe's three km01.base cameras and the tile 31,14 centre against the real
    /// registry (all 1238 locations eligible, as in the probe's model).
    #[test]
    fn retail_km01_ring_and_near_box_orders() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let registry = LocationRegistry::load(&GameFs::open(dir).unwrap()).unwrap();
        assert_eq!(registry.0.len(), 1238);
        let idx = |n: &str| registry.0.iter().position(|l| l.identity == n).unwrap();
        let km = idx("keymissions/km01/km01.base");
        let tile = idx("wc/31/31x14");
        let run = |p: [f32; 3]| {
            let mut s = ActivationState::new(&registry, |_| true);
            order(&mut s, p)
        };
        let of = |order: &[ActivationKey], i: usize| -> Vec<RecordKind> {
            order
                .iter()
                .filter(|k| k.location == i)
                .map(|k| k.kind)
                .collect()
        };
        use RecordKind::{Far, Near};
        // inside km01's near box: near then far; every near record precedes every far one
        let inside = run([-1245.0, 1500.0, -7050.0]);
        assert_eq!(of(&inside, km), [Near, Far]);
        let first_far = inside.iter().position(|k| k.kind == Far).unwrap();
        assert!(inside[first_far..].iter().all(|k| k.kind == Far));
        // the brief's model counts for these cameras
        assert_eq!(inside.len(), 30);
        // the 1000 m ring (x -2600 is between the raw box -3170.6 and the near box -2170.6)
        let ring = run([-2600.0, 1500.0, -7050.0]);
        assert_eq!(of(&ring, km), [Far]);
        // outside the raw box: nothing of km01
        let outside = run([-3200.0, 1500.0, -7050.0]);
        assert!(of(&outside, km).is_empty());
        // terrain tile 31,14 centre: both records of the tile's own location, near first
        let t = run([-256.0, 100.0, -8960.0]);
        assert_eq!(of(&t, tile), [Near, Far]);
        assert_eq!(t.len(), 36);
        // the ring is exactly the far-only band: -3170.6 + 10 < x <= -2170.6 + 10
        let x_only_far = |x: f32| of(&run([x, 1500.0, -7050.0]), km);
        assert_eq!(x_only_far(-3155.0), [Far]);
        assert!(
            x_only_far(-3165.0).is_empty(),
            "inside the 10 m band: kept, not started"
        );
        assert_eq!(x_only_far(-3175.0), Vec::<RecordKind>::new());
        assert_eq!(x_only_far(-2165.0), [Far], "inside the near box edge band");
        assert_eq!(x_only_far(-2155.0), [Near, Far]);
        let _ = keys(&[]);
    }
    /// JC2-150 spawn-load bound: how many `wc` near records (each a gameplay-enabled tile
    /// container) the camera holds at once, over the whole world on a 256 m grid (measured: at most 9, mean 2.6). The legacy
    /// tile set was the (2r+1)^2 = 9 tiles of the default radius 1. Prints the census.
    #[test]
    fn retail_wc_near_window_size_over_the_world() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let registry = LocationRegistry::load(&fs).unwrap();
        let wc: Vec<_> = registry
            .0
            .iter()
            .filter(|l| l.identity.starts_with("wc/"))
            .collect();
        assert_eq!(wc.len(), 817);
        let (mut max, mut sum, mut n, mut max_bytes, mut sum_bytes) =
            (0usize, 0usize, 0usize, 0u64, 0u64);
        let mut z = -16384.0f32;
        while z < 16384.0 {
            let mut x = -16384.0f32;
            while x < 16384.0 {
                let inside: Vec<_> = wc
                    .iter()
                    .filter(|l| l.contains_load(RecordKind::Near, x, z))
                    .collect();
                if !inside.is_empty() {
                    let bytes: u64 = inside
                        .iter()
                        .filter_map(|l| fs.locate(&l.container(RecordKind::Near)))
                        .map(|loc| loc.size)
                        .sum();
                    max = max.max(inside.len());
                    sum += inside.len();
                    n += 1;
                    max_bytes = max_bytes.max(bytes);
                    sum_bytes += bytes;
                }
                x += 256.0;
            }
            z += 256.0;
        }
        eprintln!(
            "wc near window: {n} grid points with a record, max {max} records, mean {:.1},              max {} KiB of near containers, mean {} KiB",
            sum as f64 / n as f64,
            max_bytes / 1024,
            sum_bytes / n as u64 / 1024
        );
        assert!(n > 0 && max == 9, "{max}");
    }
}
