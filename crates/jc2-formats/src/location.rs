//! World location registry. Bounds names retain raw evidence: retail activation
//! margins and the additional mode/priority fields are not decoded here.
use crate::property::{Node, PropertyFile, Variant};
use crate::{Error, GameFs, Result};

pub fn canonical(name: &str) -> String {
    name.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

/// Resolve bare aliases only when the archive index identifies one full path.
/// Distinct archives sharing a basename must never share location ownership.
pub fn container_identity(fs: &GameFs, name: &str) -> Result<String> {
    let normalized = canonical(name);
    if normalized.contains('/') {
        return Ok(normalized);
    }
    let candidates: std::collections::HashSet<_> = fs
        .entries()
        .iter()
        .filter_map(|e| fs.names.file(e.name_hash))
        .map(canonical)
        .filter(|p| p.rsplit('/').next() == Some(normalized.as_str()))
        .collect();
    if candidates.len() > 1 {
        return Err(Error::format(format!("ambiguous container alias {name}")));
    }
    Ok(candidates.into_iter().next().unwrap_or(normalized))
}
/// Inset of the near (`.nl`) record's box inside the registry bounds, metres per side
/// on all three axes: `FUN_008bd7e0`, called by the location registrar `FUN_0060e7d0`
/// for the record whose flags carry `0xc` (the one `FUN_00830560` opens as the near
/// `CLocation`, suffix `.nl` by `FUN_005f4780`). Skipped when the box would be empty.
pub const NEAR_INSET: f32 = 1000.0;
/// Hysteresis band of the per-frame activation test, metres: the location manager
/// constructor `0x5e21f0` stores `10.0` (`0xe251e8`) at its +0x68 / +0x6c / +0x70. A record
/// starts loading only when the camera is strictly inside its box shrunk by this amount in
/// X and Z (`0x7ed240`); it is kept while strictly inside the raw box (`0x7f7440`).
/// research/formats/location-activation-next.md (JC2-119).
pub const LOAD_MARGIN: f32 = 10.0;
/// `world.bin` key of the optional far-box margin read by `0x7c4840` (`#1070a35f`, default
/// 0). Its sign and int/float conversion are not proven (activation brief, Gap 9); four
/// locations carry one (-75, -40, 100, 100), none a tile or km01.base.
pub const FAR_MARGIN_KEY: u32 = 0x1070a35f;

/// Which of a location's two registry records (`0x60e7d0`, registered far first, then
/// near): the far one (flags 8, `.fl`/`.flz`) or the near one (flags 0xc, `.nl`/`.nlz`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecordKind {
    Far,
    Near,
}
type Bounds = ([f32; 3], [f32; 3]);
/// Strictly inside the box shrunk by [`LOAD_MARGIN`] on both X and Z; Y is never read
/// (`0x7ed240`).
pub fn load_contains_xz(b: Bounds, x: f32, z: f32) -> bool {
    b.0[0] + LOAD_MARGIN < x
        && x < b.1[0] - LOAD_MARGIN
        && b.0[2] + LOAD_MARGIN < z
        && z < b.1[2] - LOAD_MARGIN
}
/// Strictly inside the raw box on X and Z, no margin (`0x7f7440` unloads when
/// `x <= min.x || z <= min.z || max.x <= x || max.z <= z`).
pub fn keep_contains_xz(b: Bounds, x: f32, z: f32) -> bool {
    b.0[0] < x && x < b.1[0] && b.0[2] < z && z < b.1[2]
}
/// `0xa30ab0`: the distance to the nearest of the box's six face planes, minimum over x, y
/// and z and both faces of `|p - face|` (the camera's Y does enter this tie-break).
pub fn nearest_face_distance(b: Bounds, p: [f32; 3]) -> f32 {
    (0..3)
        .map(|i| (p[i] - b.0[i]).abs().min((p[i] - b.1[i]).abs()))
        .fold(f32::INFINITY, f32::min)
}

#[derive(Clone, Debug)]
pub struct Location {
    pub identity: String,
    pub lower: [f32; 3],
    pub upper: [f32; 3],
    pub raw: Node,
    /// False when `locations/conditions.bin` lists the location with `#6a0d2ca3 = 0`:
    /// `FUN_005ddcc0` then clears the enabled bit of its near and far records until its
    /// `enable_event` fires. Every other registry location starts enabled
    /// (`FUN_0060e7d0` sets the bit on both records).
    pub enabled_at_start: bool,
}
impl Location {
    pub fn near_container(&self) -> String {
        format!("locations/{}.nlz", self.identity)
    }
    /// The near record's box: the registry bounds inset by [`NEAR_INSET`] on every axis,
    /// or the raw bounds when the inset box would be empty (same rule as the exe).
    pub fn near_bounds(&self) -> ([f32; 3], [f32; 3]) {
        let (mut lo, mut hi) = (self.lower, self.upper);
        if (0..3).all(|i| lo[i] + NEAR_INSET < hi[i] - NEAR_INSET) {
            for i in 0..3 {
                lo[i] += NEAR_INSET;
                hi[i] -= NEAR_INSET;
            }
        }
        (lo, hi)
    }
    /// The optional far-box margin (`#1070a35f`, 0 when absent). The int/float
    /// conversion is inferred: all four retail values are whole numbers.
    pub fn far_margin(&self) -> f32 {
        match self.raw.value_hash(FAR_MARGIN_KEY) {
            Some(Variant::Float(f)) => *f,
            Some(Variant::Int(i)) => *i as f32,
            _ => 0.0,
        }
    }
    /// The far record's box: the registry bounds grown by the margin on every axis
    /// (`0x8d92a0` -> `0x8b87d0(min, max, -margin)`), only when the result stays non-empty
    /// (probe model of `0x7c4840`); the raw bounds otherwise. The margin's sign is [I].
    pub fn far_bounds(&self) -> ([f32; 3], [f32; 3]) {
        let m = self.far_margin();
        let (mut lo, mut hi) = (self.lower, self.upper);
        if (0..3).all(|i| lo[i] - m < hi[i] + m) {
            for i in 0..3 {
                lo[i] -= m;
                hi[i] += m;
            }
        }
        (lo, hi)
    }
    pub fn bounds(&self, kind: RecordKind) -> ([f32; 3], [f32; 3]) {
        match kind {
            RecordKind::Far => self.far_bounds(),
            RecordKind::Near => self.near_bounds(),
        }
    }
    /// The container the record opens: `.nlz` (near) or `.flz` (far) under `locations/`.
    pub fn container(&self, kind: RecordKind) -> String {
        match kind {
            RecordKind::Far => format!("locations/{}.flz", self.identity),
            RecordKind::Near => self.near_container(),
        }
    }
    /// May the record start loading with the camera at `(x, z)`? Strict, shrunk by
    /// [`LOAD_MARGIN`], Y-free (`0x7ed240`).
    pub fn contains_load(&self, kind: RecordKind, x: f32, z: f32) -> bool {
        load_contains_xz(self.bounds(kind), x, z)
    }
    /// Is a loaded record kept with the camera at `(x, z)`? Strictly inside the raw box
    /// (`0x7f7440`).
    pub fn contains_keep(&self, kind: RecordKind, x: f32, z: f32) -> bool {
        keep_contains_xz(self.bounds(kind), x, z)
    }
    /// Project policy: inclusive XZ intersection of the near record's box with the
    /// supplied region (the exe's per-frame activation test is not decoded).
    pub fn near_intersects_xz(&self, lower: [f32; 2], upper: [f32; 2]) -> bool {
        let (lo, hi) = self.near_bounds();
        lo[0] <= upper[0] && hi[0] >= lower[0] && lo[2] <= upper[1] && hi[2] >= lower[1]
    }
    /// Project policy: inclusive XZ intersection with the supplied streaming region.
    /// The raw vertical bounds are retained but not used as mission gates.
    pub fn intersects_xz(&self, lower: [f32; 2], upper: [f32; 2]) -> bool {
        self.lower[0] <= upper[0]
            && self.upper[0] >= lower[0]
            && self.lower[2] <= upper[1]
            && self.upper[2] >= lower[1]
    }
}
#[derive(Clone, Debug, Default)]
pub struct LocationRegistry(pub Vec<Location>);
impl LocationRegistry {
    pub fn load(fs: &GameFs) -> Result<Self> {
        let mut registry = Self::parse(&fs.read_inflated("locations/world.bin")?)?;
        if let Ok(bytes) = fs.read_inflated("locations/conditions.bin") {
            registry.apply_conditions(&bytes)?;
        }
        Ok(registry)
    }
    /// `locations/conditions.bin` (`FUN_005ddcc0`): nodes with `name`, `enable_event`,
    /// `disable_event` and `#6a0d2ca3` (read with default 1). A 0 starts the named
    /// location disabled. Returns the names it disabled.
    pub fn apply_conditions(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        let file = PropertyFile::parse_any(bytes)?;
        let mut disabled = Vec::new();
        for root in &file.nodes {
            root.walk(&mut |n| {
                let Some(name) = n.str("name") else { return };
                let enabled = match n.value_hash(0x6a0d2ca3) {
                    Some(Variant::Int(v)) => *v != 0,
                    _ => true,
                };
                if !enabled {
                    disabled.push(canonical(name));
                }
            });
        }
        for loc in &mut self.0 {
            if disabled.contains(&loc.identity) {
                loc.enabled_at_start = false;
            }
        }
        Ok(disabled)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(bytes)?;
        let mut out = Vec::new();
        let mut error = None;
        for root in &file.nodes {
            root.walk(&mut |n| {
                let Some(name) = n.str("file") else { return };
                let (Some(Variant::Vec3(upper)), Some(Variant::Vec3(lower))) =
                    (n.value_hash(0x2b5ef194), n.value_hash(0x7df7f859))
                else {
                    error = Some(Error::format(format!("location {name}: missing bounds")));
                    return;
                };
                if (0..3)
                    .any(|i| !lower[i].is_finite() || !upper[i].is_finite() || lower[i] > upper[i])
                {
                    error = Some(Error::format(format!("location {name}: invalid bounds")));
                    return;
                }
                out.push(Location {
                    identity: canonical(name),
                    lower: *lower,
                    upper: *upper,
                    raw: n.clone(),
                    enabled_at_start: true,
                });
            });
        }
        if let Some(e) = error {
            return Err(e);
        }
        Ok(Self(out))
    }
    pub fn resolve(&self, identity: &str) -> Option<&Location> {
        let name = canonical(identity);
        self.0.iter().find(|l| l.identity == name)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retail_registry_retains_bounds_and_city_container() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let registry = LocationRegistry::load(&fs).unwrap();
        let loc = registry
            .resolve("Metropolis\\CITY.DISTRICT.B1.HOUSES")
            .unwrap();
        assert_eq!(loc.lower, [-15211.349, -1393.2965, -3219.276]);
        assert_eq!(loc.upper, [-10848.1045, 2146.8567, 816.7408]);
        assert!(fs.contains(&loc.near_container()));
        assert!(loc.intersects_xz([-14000., -2000.], [-13000., -1000.]));
        assert!(!loc.intersects_xz([0., 0.], [512., 512.]));
        assert_eq!(
            loc.raw.str("file"),
            Some("metropolis\\city.district.b1.houses")
        );
    }
    #[test]
    fn retail_start_base_is_an_unconditional_registry_location() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let registry = LocationRegistry::load(&fs).unwrap();
        assert_eq!(registry.0.len(), 1238);
        // Kem Gunung Dataran Tinggi (settlement `km01.base`), not a `wc` tile.
        let base = registry.resolve("keymissions\\km01\\km01.base").unwrap();
        assert!(base.enabled_at_start);
        assert!(fs.contains(&base.near_container()));
        assert_eq!(base.lower, [-3170.6323, -254.15771, -8916.654]);
        let (lo, hi) = base.near_bounds();
        assert_eq!(
            [lo[0], lo[2], hi[0], hi[2]],
            [
                -3170.6323 + NEAR_INSET,
                -8916.654 + NEAR_INSET,
                680.9554 - NEAR_INSET,
                -5184.7334 - NEAR_INSET
            ]
        );
        // tile 29,18 (the base's settlement icon) is inside the near box, 31,14 is not
        let tile = |x: u32, z: u32| {
            let (ox, oz) = crate::terrain::tile_origin(x, z);
            (
                [ox, oz],
                [
                    ox + crate::terrain::TILE_SIZE,
                    oz + crate::terrain::TILE_SIZE,
                ],
            )
        };
        let (a, b) = tile(29, 18);
        assert!(base.near_intersects_xz(a, b));
        let (a, b) = tile(31, 14);
        assert!(!base.near_intersects_xz(a, b));
        assert!(
            base.intersects_xz(a, b),
            "the raw (far) box does reach 31,14"
        );
        // the five conditional locations start disabled
        let disabled: Vec<_> = registry
            .0
            .iter()
            .filter(|l| !l.enabled_at_start)
            .map(|l| l.identity.as_str())
            .collect();
        assert_eq!(disabled.len(), 5, "{disabled:?}");
        assert!(disabled.contains(&"keymissions/km07/km07.submarine"));
    }
    /// The brief's fixtures (location-activation-next.md section 2), all from `world.bin`.
    #[test]
    fn retail_activation_windows_match_the_brief() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let registry = LocationRegistry::load(&GameFs::open(dir).unwrap()).unwrap();
        let r1 = |v: f32| (v * 10.0).round() / 10.0;
        // (raw x, raw z, near-load x, near-load z, far-load x, far-load z)
        type Win = ([f32; 2], [f32; 2], [f32; 2], [f32; 2], [f32; 2], [f32; 2]);
        let fixtures: [(&str, Win); 2] = [
            (
                "keymissions\\km01\\km01.base",
                (
                    [-3170.632, 680.955],
                    [-8916.654, -5184.733],
                    [-2160.6, -329.0],
                    [-7906.7, -6194.7],
                    [-3160.6, 671.0],
                    [-8906.7, -5194.7],
                ),
            ),
            (
                "wc\\31\\31x14",
                (
                    [-2452.979, 1173.927],
                    [-10466.992, -6944.185],
                    [-1443.0, 163.9],
                    [-9457.0, -7954.2],
                    [-2443.0, 1163.9],
                    [-10457.0, -6954.2],
                ),
            ),
        ];
        for (name, (rx, rz, nlx, nlz, flx, flz)) in fixtures {
            let loc = registry.resolve(name).unwrap();
            assert!(loc.enabled_at_start);
            assert_eq!(loc.far_margin(), 0.0);
            let (lo, hi) = loc.far_bounds();
            assert_eq!(
                [r1(lo[0]), r1(hi[0]), r1(lo[2]), r1(hi[2])],
                [r1(rx[0]), r1(rx[1]), r1(rz[0]), r1(rz[1])],
                "{name} far box = raw"
            );
            let (lo, hi) = loc.near_bounds();
            // near box = raw inset by exactly 1000 m on every axis
            assert_eq!(lo[0], loc.lower[0] + NEAR_INSET);
            assert_eq!(hi[2], loc.upper[2] - NEAR_INSET);
            let win = |kind| {
                let (lo, hi) = loc.bounds(kind);
                [
                    r1(lo[0] + LOAD_MARGIN),
                    r1(hi[0] - LOAD_MARGIN),
                    r1(lo[2] + LOAD_MARGIN),
                    r1(hi[2] - LOAD_MARGIN),
                ]
            };
            assert_eq!(win(RecordKind::Near), [nlx[0], nlx[1], nlz[0], nlz[1]]);
            assert_eq!(win(RecordKind::Far), [flx[0], flx[1], flz[0], flz[1]]);
        }
        // the margin key: exactly four locations, values -75, -40, 100, 100
        let mut margins: Vec<f32> = registry
            .0
            .iter()
            .map(Location::far_margin)
            .filter(|m| *m != 0.0)
            .collect();
        margins.sort_by(f32::total_cmp);
        assert_eq!(margins, [-75.0, -40.0, 100.0, 100.0]);
    }
    #[test]
    fn activation_tests_are_strict_xz_only_with_a_ten_metre_band() {
        let b = ([0.0, -50.0, 0.0], [100.0, 50.0, 200.0]);
        // load: strictly inside, 10 m in from each side
        assert!(!load_contains_xz(b, 10.0, 100.0));
        assert!(load_contains_xz(b, 10.01, 100.0));
        assert!(!load_contains_xz(b, 90.0, 100.0));
        assert!(!load_contains_xz(b, 50.0, 10.0));
        assert!(load_contains_xz(b, 50.0, 189.99));
        // keep: strictly inside the raw box, the band is the whole hysteresis
        assert!(keep_contains_xz(b, 0.01, 100.0));
        assert!(!keep_contains_xz(b, 0.0, 100.0));
        assert!(!keep_contains_xz(b, 100.0, 100.0));
        assert!(!keep_contains_xz(b, 50.0, 200.0));
        // Y never enters the containment tests, but does enter the tie-break distance
        assert_eq!(nearest_face_distance(b, [50.0, 0.0, 100.0]), 50.0);
        assert_eq!(nearest_face_distance(b, [50.0, 45.0, 100.0]), 5.0);
        assert_eq!(nearest_face_distance(b, [50.0, 9000.0, 100.0]), 50.0);
    }
    #[test]
    fn far_bounds_apply_the_margin_only_when_the_box_stays_non_empty() {
        use crate::property::Key;
        let loc = |m: f32, lo: f32, hi: f32| Location {
            identity: "t".into(),
            lower: [lo; 3],
            upper: [hi; 3],
            raw: Node {
                values: vec![(Key::Hash(FAR_MARGIN_KEY), Variant::Float(m))],
                ..Default::default()
            },
            enabled_at_start: true,
        };
        assert_eq!(
            loc(100.0, 0.0, 10.0).far_bounds(),
            ([-100.0; 3], [110.0; 3])
        );
        assert_eq!(loc(-4.0, 0.0, 10.0).far_bounds(), ([4.0; 3], [6.0; 3]));
        // a shrink past empty keeps the raw box
        assert_eq!(loc(-5.0, 0.0, 10.0).far_bounds(), ([0.0; 3], [10.0; 3]));
        assert_eq!(loc(0.0, 0.0, 10.0).near_bounds(), ([0.0; 3], [10.0; 3]));
    }
    #[test]
    fn malformed_registry_never_normalizes_invalid_bounds() {
        use crate::property::Key;
        let node = Node {
            values: vec![
                (Key::Name("file".into()), Variant::Str("broken".into())),
                (Key::Hash(0x2b5ef194), Variant::Vec3([1.; 3])),
                (Key::Hash(0x7df7f859), Variant::Vec3([2.; 3])),
            ],
            ..Default::default()
        };
        assert!(LocationRegistry::parse(&PropertyFile { nodes: vec![node] }.to_bytes()).is_err());
    }
}
