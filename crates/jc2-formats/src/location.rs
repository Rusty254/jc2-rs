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
