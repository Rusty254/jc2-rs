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
#[derive(Clone, Debug)]
pub struct Location {
    pub identity: String,
    pub lower: [f32; 3],
    pub upper: [f32; 3],
    pub raw: Node,
}
impl Location {
    pub fn near_container(&self) -> String {
        format!("locations/{}.nlz", self.identity)
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
        Self::parse(&fs.read_inflated("locations/world.bin")?)
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
