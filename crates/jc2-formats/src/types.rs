//! `settings\vehicletypes.bin` and `settings\charactertypes.bin`: one node per vehicle
//! (`entity` = its container, `vehicles\lave\lave.v014_jeep.ee` without the folder) or
//! character type (`name`), with the streamer's `mem_cost` and `perf_cost` budgets. The
//! vehicles carry one more value, `#c097e6ef`, that grows from 310 on a Vespa through
//! ~2000 on cars to 6000 on planes; jc2-rs takes it as the vehicle's health (assumed).

use crate::property::{PropertyFile, Variant};

#[derive(Clone, Debug, PartialEq)]
pub struct VehicleType {
    /// `entity`: `lave\lave.v014_jeep.ee`.
    pub entity: String,
    pub mem_cost: f32,
    pub perf_cost: f32,
    /// `#c097e6ef` (health, assumed).
    pub health: f32,
}

impl VehicleType {
    /// The container's file name (`lave.v014_jeep.ee`), how the fleet names vehicles.
    pub fn container(&self) -> &str {
        crate::placement::bare_name(&self.entity)
    }
}

const HEALTH: u32 = 0xc097_e6ef;

pub fn vehicle_types(data: &[u8]) -> crate::Result<Vec<VehicleType>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if let Some(entity) = n.str("entity") {
                let health = match n.value_hash(HEALTH) {
                    Some(Variant::Float(f)) => *f,
                    Some(Variant::Int(i)) => *i as f32,
                    _ => 1000.0,
                };
                out.push(VehicleType {
                    entity: entity.to_string(),
                    mem_cost: n.float("mem_cost").unwrap_or(0.0),
                    perf_cost: n.float("perf_cost").unwrap_or(0.0),
                    health,
                });
            }
        });
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharacterType {
    /// `pd_gov_base01`: the character's container without `.ee`.
    pub name: String,
    pub mem_cost: f32,
    pub perf_cost: f32,
}

pub fn character_types(data: &[u8]) -> crate::Result<Vec<CharacterType>> {
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if let Some(name) = n.str("name")
                && n.value("mem_cost").is_some()
            {
                out.push(CharacterType {
                    name: name.to_string(),
                    mem_cost: n.float("mem_cost").unwrap_or(0.0),
                    perf_cost: n.float("perf_cost").unwrap_or(0.0),
                });
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// Needs JC2_TEST_DIR with `vehicletypes.bin` and `charactertypes.bin`.
    #[test]
    fn types() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(v), Ok(c)) = (
            fs.read_inflated("vehicletypes.bin"),
            fs.read_inflated("charactertypes.bin"),
        ) else {
            return;
        };
        let v = super::vehicle_types(&v).unwrap();
        assert_eq!(v.len(), 91);
        let jeep = v
            .iter()
            .find(|t| t.container() == "lave.v014_jeep.ee")
            .unwrap();
        assert_eq!((jeep.health, jeep.mem_cost), (1980.0, 1084.0));
        let c = super::character_types(&c).unwrap();
        assert!(c.len() > 100, "{}", c.len());
        let g = c.iter().find(|t| t.name == "pd_gov_base01").unwrap();
        assert_eq!(g.mem_cost, 100.0);
    }
}
