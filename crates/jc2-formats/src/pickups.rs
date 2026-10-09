//! Pickups: the exported entities (`.ee` → `.epe` blackboard) that hand Rico a weapon
//! or ammunition. `exported\pickups\pickup.<weapon>.ee` is a bare `CWeapon`
//! (`weaponname`, `pickup_count`) with its pick-up smart object (`CWeaponPickupAction`);
//! `exported\weaponstashes\mod.stash.<weapon>.ee` is a crate (`wea00-b.lod` and its lid)
//! with the weapon lying in it — the location files place the stashes with a
//! `CSpawnSet` + `CExportedEntityComponent` (`placement::exported_spawns`). Grenades and
//! charges are a `CItem` with a `modelname` instead. See `research/formats/spawning.md`.

use crate::placement::Transform3;
use crate::property::{Node, PropertyFile, Variant};

/// What a pickup gives.
#[derive(Clone, Debug, PartialEq)]
pub enum PickupKind {
    /// A gun, by its `weapons.bin` name (`weaponname`).
    Weapon(String),
    /// A thrown item (`CItem`: the grenade, the triggered charge), by its model `.lod`.
    Item(String),
    /// A resource crate's contents (`CResourceItem`): parts or cash.
    Resource(ResourceKind),
}

/// The pickup inside an exported entity.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupItem {
    pub kind: PickupKind,
    /// `pickup_count` (`#dc0f0e6c`): 1 … 50, -1 on the minigun — read as magazines
    /// (assumed; the game caps it at what can be carried).
    pub count: i32,
    /// Where it sits in the entity (relative to the entity's origin).
    pub local: Transform3,
}

/// An exported entity holding a pickup: its models (the stash crate) and the item.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupEntity {
    /// `.lod`s drawn with it (`weapons\wea00\wea00-b.lod`, the crate) and where.
    pub models: Vec<(String, Transform3)>,
    pub item: PickupItem,
}

fn world(n: &Node) -> Option<Transform3> {
    match n.value("world")? {
        Variant::Mat4x4(m) => Some(Transform3::from_mat4x4(*m)),
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        _ => None,
    }
}

impl PickupEntity {
    /// From the `.epe` of the container (or any property file with the same objects).
    pub fn parse(data: &[u8]) -> crate::Result<Option<Self>> {
        let file = PropertyFile::parse_any(data)?;
        Ok(Self::from_property(&file))
    }

    pub fn from_property(file: &PropertyFile) -> Option<Self> {
        fn walk(
            n: &Node,
            parent: &Transform3,
            models: &mut Vec<(String, Transform3)>,
            item: &mut Option<PickupItem>,
        ) {
            let here = world(n).map(|w| w.then(parent)).unwrap_or(*parent);
            let class = n.str("_class").unwrap_or("");
            let count = n.int("pickup_count").unwrap_or(1);
            match class {
                "CWeapon" if item.is_none() => {
                    if let Some(w) = n.str("weaponname").filter(|w| !w.is_empty()) {
                        *item = Some(PickupItem {
                            kind: PickupKind::Weapon(w.to_string()),
                            count,
                            local: here,
                        });
                    }
                    return;
                }
                "CItem" if item.is_none() => {
                    if let Some(m) = n.str("modelname").filter(|m| !m.is_empty()) {
                        *item = Some(PickupItem {
                            kind: PickupKind::Item(m.to_string()),
                            count,
                            local: here,
                        });
                    }
                    return;
                }
                "CSimpleRigidObject" => {
                    if let Some(f) = n.str("filename").filter(|f| !f.is_empty()) {
                        models.push((f.to_string(), here));
                    }
                }
                _ => {}
            }
            for (_, c) in &n.children {
                walk(c, &here, models, item);
            }
        }
        let mut models = Vec::new();
        let mut item = None;
        for root in &file.nodes {
            walk(root, &Transform3::IDENTITY, &mut models, &mut item);
        }
        item.map(|item| Self { models, item })
    }
}

/// The models an exported entity draws (`CSimpleRigidObject` `filename`s, placed), for
/// entities without an item (`pickup.boost.weapon.ee`: the `pu04-a.lod` box).
pub fn entity_models(data: &[u8]) -> crate::Result<Vec<(String, Transform3)>> {
    fn walk(n: &Node, parent: &Transform3, out: &mut Vec<(String, Transform3)>) {
        let here = world(n).map(|w| w.then(parent)).unwrap_or(*parent);
        if n.str("_class") == Some("CSimpleRigidObject")
            && let Some(f) = n.str("filename").filter(|f| !f.is_empty())
        {
            out.push((f.to_string(), here));
        }
        for (_, c) in &n.children {
            walk(c, &here, out);
        }
    }
    let file = PropertyFile::parse_any(data)?;
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    Ok(out)
}

/// What a resource crate (`CResourceItem`, `type`) holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// 0: an armour part (five raise Rico's health).
    Armor,
    /// 1: a weapon part.
    Weapon,
    /// 2: a vehicle part.
    Vehicle,
    /// 3: a cash stash.
    Cash,
}

impl ResourceKind {
    fn from_type(t: i32) -> Option<Self> {
        Some(match t {
            0 => Self::Armor,
            1 => Self::Weapon,
            2 => Self::Vehicle,
            3 => Self::Cash,
            _ => return None,
        })
    }

    /// What one crate gives: a part, or $2500 (`0x9c4`) of cash — read off the exe's
    /// handler (`FUN_00854510`: `+1` part, `+0x9c4` money), not a data file.
    pub fn amount(self) -> u32 {
        match self {
            Self::Cash => 2500,
            _ => 1,
        }
    }
}

/// A resource crate a location places: `CResourceItem { type, #f25a8a76 =
/// "exported\pickups\pickup.boost.weapon.ee", #ec22ee85 = "t{…}::sht.pickup.enter" }`; its
/// trigger's `enter_event` also fires the settlement's `<id>.count.up`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceItem {
    pub kind: ResourceKind,
    pub name: String,
    /// The pickup entity (`exported\pickups\pickup.boost.weapon.ee`: the `pu04-a.lod` box).
    pub entity: String,
    pub world: Transform3,
    /// The settlement whose completion it counts towards.
    pub counter: Option<String>,
}

const RESOURCE_ENTITY: u32 = 0xf25a_8a76;
const RESOURCE_EVENT: u32 = 0xec22_ee85;

/// The resource crates of a placement file (eight in `31x14`: three weapon parts, two
/// vehicle parts, two cash stashes, an armour part).
pub fn resource_items(file: &PropertyFile) -> Vec<ResourceItem> {
    let mut strings: Vec<&str> = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            for (_, v) in &n.values {
                if let Variant::Str(s) = v
                    && crate::chaos::counter_in(s).is_some()
                {
                    strings.push(s);
                }
            }
        });
    }
    fn walk(n: &Node, parent: &Transform3, strings: &[&str], out: &mut Vec<ResourceItem>) {
        let here = world(n).map(|w| w.then(parent)).unwrap_or(*parent);
        if n.str("_class") == Some("CResourceItem")
            && let Some(kind) = n.int("type").and_then(ResourceKind::from_type)
        {
            let s = |h: u32| match n.value_hash(h) {
                Some(Variant::Str(s)) => s.as_str(),
                _ => "",
            };
            let event = s(RESOURCE_EVENT);
            let counter = event
                .split_whitespace()
                .next()
                .and_then(|tok| {
                    strings
                        .iter()
                        .find(|v| v.split_whitespace().any(|t| t == tok))
                })
                .and_then(|v| crate::chaos::counter_in(v))
                .map(str::to_string);
            out.push(ResourceItem {
                kind,
                name: n.str("name").unwrap_or("").to_string(),
                entity: s(RESOURCE_ENTITY).to_string(),
                world: here,
                counter,
            });
        }
        for (_, c) in &n.children {
            walk(c, &here, strings, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &strings, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `31x14.nlz`.
    #[test]
    fn resource_crates() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("31x14.nlz") else {
            return;
        };
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.ends_with(".blo"))
            .unwrap();
        let bytes = crate::compress::maybe_inflate(sarc.data(&data, e).unwrap()).unwrap();
        let file = PropertyFile::parse_any(&bytes).unwrap();
        let r = resource_items(&file);
        assert_eq!(r.len(), 8);
        let n = |k| r.iter().filter(|i| i.kind == k).count();
        assert_eq!(
            (
                n(ResourceKind::Weapon),
                n(ResourceKind::Vehicle),
                n(ResourceKind::Cash),
                n(ResourceKind::Armor)
            ),
            (3, 2, 2, 1)
        );
        assert!(r.iter().all(|i| i.counter.as_deref() == Some("31x14")));
        let w = r.iter().find(|i| i.kind == ResourceKind::Weapon).unwrap();
        assert_eq!(w.entity, "exported\\pickups\\pickup.boost.weapon.ee");
        let t = w.world.translation();
        assert!(
            (t[0] + 409.0).abs() < 1.0 && (t[2] + 8916.7).abs() < 1.0,
            "{t:?}"
        );
        assert_eq!(ResourceKind::Cash.amount(), 2500);
    }

    fn epe(fs: &crate::GameFs, container: &str) -> Option<PickupEntity> {
        let data = fs.read_inflated(container).ok()?;
        let sarc = crate::sarc::SmallArchive::parse(&data).ok()?;
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".epe"))?;
        PickupEntity::parse(sarc.data(&data, e).ok()?).ok()?
    }

    /// Needs JC2_TEST_DIR with the pickups and stashes.
    #[test]
    fn pickups_and_stashes() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        if !fs.contains("pickup.scorpionsmg.ee") || !fs.contains("mod.stash.smg.ee") {
            return;
        }
        let p = epe(&fs, "pickup.scorpionsmg.ee").unwrap();
        assert_eq!(p.item.kind, PickupKind::Weapon("Scorpion_SMG".into()));
        assert_eq!(p.item.count, 4);
        assert!(p.models.is_empty());
        let s = epe(&fs, "mod.stash.smg.ee").unwrap();
        assert_eq!(s.item.kind, PickupKind::Weapon("Scorpion_SMG".into()));
        assert_eq!(s.item.count, 5);
        let lods: Vec<&str> = s.models.iter().map(|m| m.0.as_str()).collect();
        assert!(lods.contains(&"weapons\\wea00\\wea00-b.lod"), "{lods:?}");
        // the gun lies in the crate, a few decimetres from its origin
        let t = s.item.local.translation();
        assert!(t.iter().all(|v| v.abs() < 1.0), "{t:?}");
        let g = epe(&fs, "mod.stash.grenade.ee").unwrap();
        assert_eq!(
            g.item.kind,
            PickupKind::Item("weapons\\wea33\\wea33-wea33.lod".into())
        );
    }
}
