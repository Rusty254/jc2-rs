//! Character dolls (`exported\cdoll\*.ee` → `<name>.cdoll`) and the AI spawn data of
//! the location files (teams, patrol routes, guard areas). See
//! `research/formats/character.md`.

use crate::placement::Transform3;
use crate::property::{Node, PropertyFile, Variant};

/// A `.cdoll`: the base model, the skeleton, and the groups of interchangeable parts
/// (heads, hats, jackets…) of which one each is worn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterDoll {
    /// `#797211b8`: `Models\characters\factions\pd_gov_base\pd_gov_base-base.lod`.
    pub base: String,
    /// `#f53a7638`: `Animations\Skeletons\syncreadthis\biped.bsk`.
    pub skeleton: String,
    /// Under `#68ddf8de`: each group's `name` and its variants' `.lod`s (`#a5ae7a6c`).
    pub groups: Vec<(String, Vec<String>)>,
}

const BASE: u32 = 0x7972_11b8;
const SKELETON: u32 = 0xf53a_7638;
const PART_GROUPS: u32 = 0x68dd_f8de;
const PART_LOD: u32 = 0xa5ae_7a6c;

fn str_hash(n: &Node, h: u32) -> Option<String> {
    match n.value_hash(h)? {
        Variant::Str(s) => Some(s.clone()),
        _ => None,
    }
}

impl CharacterDoll {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        Ok(Self::from_property(&file))
    }

    pub fn from_property(file: &PropertyFile) -> Self {
        let mut d = Self::default();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if d.base.is_empty()
                    && let Some(b) = str_hash(n, BASE)
                {
                    d.base = b;
                    d.skeleton = str_hash(n, SKELETON).unwrap_or_default();
                    for (k, groups) in &n.children {
                        if k.hash() != PART_GROUPS {
                            continue;
                        }
                        for (_, g) in &groups.children {
                            let variants: Vec<String> = g
                                .children
                                .iter()
                                .filter_map(|(_, v)| str_hash(v, PART_LOD))
                                .collect();
                            if !variants.is_empty() {
                                d.groups
                                    .push((g.str("name").unwrap_or("").to_string(), variants));
                            }
                        }
                    }
                }
            });
        }
        d
    }

    /// The `.lod` files of one outfit: the base and one variant per group, picked by
    /// `seed`.
    pub fn outfit(&self, seed: usize) -> Vec<String> {
        let mut out = vec![self.base.clone()];
        for (i, (_, v)) in self.groups.iter().enumerate() {
            out.push(v[(seed + i * 7) % v.len()].clone());
        }
        out
    }
}

/// A team spawn set (`CSpawnSet` with a `CTeamComponent`): soldiers that guard an area or
/// walk a patrol route.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamSpawn {
    pub name: String,
    pub world: Transform3,
    /// `#f5afca6d`: the AI team definition (`bofcx1.aiteam`).
    pub aiteam: String,
    /// One entry per `STeamAgentInfo`: its role (`#78efe1bc`: `Guard`, `Patrol`).
    pub agents: Vec<String>,
    /// `#dfbe4772`: the `CPatrolRouteGameObject` it walks.
    pub route: Option<String>,
    /// `#a870db99`: the `CAOOBoxGameObject` (area of operation) it guards.
    pub area: Option<String>,
    /// The `CTeamComponent`'s `faction`: 0 the government, 1 Reapers, 2 Roaches, 3 Ular
    /// Boys (the faction soldiers a taken stronghold gets, `missions.md`).
    pub faction: i32,
    /// The spawn set's `enabled` (absent: on).
    pub enabled: bool,
    /// `enable_event` / `disable_event`: what switches it on and off
    /// (`f1s07.before.enabled`, `msy.f1s07obj04.completed`).
    pub enable_event: String,
    pub disable_event: String,
}

/// A `CPatrolRouteGameObject`: `route` = 4 floats per point (x, y, z relative to the
/// object, and a 0.5 that may be a pause or a speed), points absolute here.
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolRoute {
    pub name: String,
    pub points: Vec<[f32; 3]>,
}

/// A `CAOOBoxGameObject` with its `SGuardPoint`s (absolute).
#[derive(Clone, Debug, PartialEq)]
pub struct GuardArea {
    pub name: String,
    pub centre: [f32; 3],
    pub points: Vec<[f32; 3]>,
}

const AITEAM: u32 = 0xf5af_ca6d;
const ROLE: u32 = 0x78ef_e1bc;
const ROUTE: u32 = 0xdfbe_4772;
const AREA: u32 = 0xa870_db99;

/// The AI spawn data of a placement file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPlacements {
    pub teams: Vec<TeamSpawn>,
    pub routes: Vec<PatrolRoute>,
    pub areas: Vec<GuardArea>,
}

fn world_of(n: &Node) -> Option<Transform3> {
    match n.value("world")? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        _ => None,
    }
}

impl AiPlacements {
    pub fn from_property(file: &PropertyFile) -> Self {
        let mut out = Self::default();
        fn walk(n: &Node, parent: &Transform3, out: &mut AiPlacements) {
            let here = world_of(n).map(|w| w.then(parent)).unwrap_or(*parent);
            match n.str("_class").unwrap_or("") {
                "CSpawnSet" => {
                    let mut team = None;
                    for (_, c) in &n.children {
                        if c.str("_class") == Some("CTeamComponent") {
                            team = Some(c);
                        }
                    }
                    if let Some(t) = team {
                        let agents = t
                            .children
                            .iter()
                            .filter(|(_, a)| a.str("_class") == Some("STeamAgentInfo"))
                            .map(|(_, a)| str_hash(a, ROLE).unwrap_or_default())
                            .collect();
                        out.teams.push(TeamSpawn {
                            name: n.str("name").unwrap_or("").to_string(),
                            world: here,
                            aiteam: str_hash(t, AITEAM).unwrap_or_default(),
                            agents,
                            route: str_hash(t, ROUTE).filter(|s| !s.is_empty()),
                            area: str_hash(t, AREA).filter(|s| !s.is_empty()),
                            faction: t.int("faction").unwrap_or(0),
                            enabled: n.int("enabled").is_none_or(|e| e != 0),
                            enable_event: n.str("enable_event").unwrap_or("").to_string(),
                            disable_event: n.str("disable_event").unwrap_or("").to_string(),
                        });
                    }
                    return;
                }
                "CPatrolRouteGameObject" => {
                    if let Some(Variant::Floats(f)) = n.value("route") {
                        let points = f
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|p| here.transform_point([p[0], p[1], p[2]]))
                            .collect();
                        out.routes.push(PatrolRoute {
                            name: n.str("name").unwrap_or("").to_string(),
                            points,
                        });
                    }
                    return;
                }
                "CAOOBoxGameObject" => {
                    let points = n
                        .children
                        .iter()
                        .filter(|(_, c)| c.str("_class") == Some("SGuardPoint"))
                        .filter_map(|(_, c)| world_of(c).map(|w| w.then(&here).translation()))
                        .collect();
                    out.areas.push(GuardArea {
                        name: n.str("name").unwrap_or("").to_string(),
                        centre: here.translation(),
                        points,
                    });
                    return;
                }
                _ => {}
            }
            for (_, c) in &n.children {
                walk(c, &here, out);
            }
        }
        for root in &file.nodes {
            walk(root, &Transform3::IDENTITY, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `pd_gov_base01.ee` and `31x14.nlz`.
    #[test]
    fn soldier_doll_and_village_guards() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        if let Ok(data) = fs.read_inflated("pd_gov_base01.ee") {
            let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
            let e = sarc.get("pd_gov_base01.cdoll").unwrap();
            let d = CharacterDoll::parse(sarc.data(&data, e).unwrap()).unwrap();
            assert!(d.base.ends_with("pd_gov_base-base.lod"), "{d:?}");
            assert!(d.skeleton.to_ascii_lowercase().ends_with("biped.bsk"));
            let names: Vec<&str> = d.groups.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, ["Heads", "Hats"]);
            assert_eq!(d.groups[1].1.len(), 4);
            assert_eq!(d.outfit(0).len(), 3);
        }
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
        let ai = AiPlacements::from_property(&PropertyFile::parse_any(&bytes).unwrap());
        assert_eq!(ai.teams.len(), 6, "{:#?}", ai.teams);
        assert!(ai.teams.iter().all(|t| t.aiteam == "bofcx1.aiteam"));
        let patrol = ai
            .teams
            .iter()
            .find(|t| t.route.as_deref() == Some("par.Route.31x14.01"))
            .expect("the village patrol");
        assert_eq!(patrol.agents, ["Patrol"]);
        let route = ai
            .routes
            .iter()
            .find(|r| r.name == "par.Route.31x14.01")
            .unwrap();
        assert_eq!(route.points.len(), 7);
        // the first point is (-27.9, 0.1, -12.4) from the route's origin
        assert!(
            (route.points[0][0] - (-509.55 - 27.9)).abs() < 0.1,
            "{:?}",
            route.points[0]
        );
        assert_eq!(ai.areas.len(), 5);
        assert!(ai.areas.iter().any(|a| a.points.len() >= 2));
    }
}
