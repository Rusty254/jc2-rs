//! Civilians and road traffic: who the world simulation spawns
//! (`settings\worldsim.bin`), where people roam (the `CCivilianRoamArea`s of the location
//! files) and what kind of ground a place is (`terrain\gamezone.bin` names the game-zone
//! bytes of the terrain tiles). See `research/formats/civilians.md`.
//!
//! `worldsim.bin` (keys named by hashing, ✓ in civilians.md):
//!
//! ```text
//! gamezone_file = "Terrain\GameZone.bin", "worldsim.aiteam", …
//! character_types { <hash> { type = "PD_Generic_Male_2.cdoll",
//!                            spawnzones = "city:5 village:5 religious:5 default:5",
//!                            gamezones = "jungle:5 sea:5 city:5 grass:5 default:5" } … }
//! lave { max_count = 15, #15edfea4 { min_inner_spawn_range = 100, … },
//!        spawners { … type = "road" … }, vehicle_types { <hash> { type = "lave.v001_sedan.ee",
//!        spawnzones, gamezones } … } }
//! seve { … boats }, arve { … planes }, traffic_density { … density 0/3/4/8/16 },
//! night_time { start = "01:00", end = "05:00", factor = 0.34 }
//! ```

use crate::placement::Transform3;
use crate::property::{Node, PropertyFile, Variant};

/// A `CCivilianRoamArea`: a box people wander in.
#[derive(Clone, Debug, PartialEq)]
pub struct RoamArea {
    pub name: String,
    /// The box's centre and orientation (engine axes).
    pub world: Transform3,
    /// `box_size` ✓: half extents along the area's own x, y, z (m).
    pub half_extents: [f32; 3],
    /// `capacity` ✓: how many people it holds.
    pub capacity: u32,
    /// Its `SCivilianRoamAreaSpawnPoint`s, absolute.
    pub spawn_points: Vec<[f32; 3]>,
}

impl RoamArea {
    /// The point at `(u, v)` in −1..1 across the box's x and z, absolute.
    pub fn point(&self, u: f32, v: f32) -> [f32; 3] {
        self.world.transform_point([
            u.clamp(-1.0, 1.0) * self.half_extents[0],
            0.0,
            v.clamp(-1.0, 1.0) * self.half_extents[2],
        ])
    }

    /// Whether an absolute point lies inside the box in x and z, give or take `margin`.
    pub fn contains(&self, p: [f32; 3], margin: f32) -> bool {
        let t = self.world.translation();
        let d = [p[0] - t[0], p[1] - t[1], p[2] - t[2]];
        let (x, z) = (self.world.row(0), self.world.row(2));
        let lx = (d[0] * x[0] + d[1] * x[1] + d[2] * x[2]) / norm2(x).max(1e-9);
        let lz = (d[0] * z[0] + d[1] * z[1] + d[2] * z[2]) / norm2(z).max(1e-9);
        lx.abs() <= self.half_extents[0] + margin && lz.abs() <= self.half_extents[2] + margin
    }
}

fn norm2(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

fn world_of(n: &Node) -> Option<Transform3> {
    match n.value("world")? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        Variant::Mat4x4(m) => Some(Transform3::from_mat4x4(*m)),
        _ => None,
    }
}

/// Every civilian roam area of a placement file.
pub fn roam_areas(file: &PropertyFile) -> Vec<RoamArea> {
    fn walk(n: &Node, parent: &Transform3, out: &mut Vec<RoamArea>) {
        let here = world_of(n).map(|w| w.then(parent)).unwrap_or(*parent);
        if n.str("_class") == Some("CCivilianRoamArea") {
            let half_extents = match n.value("box_size") {
                Some(Variant::Vec3(v)) => *v,
                _ => [5.0, 5.0, 5.0],
            };
            let spawn_points = n
                .children
                .iter()
                .filter(|(_, c)| c.str("_class") == Some("SCivilianRoamAreaSpawnPoint"))
                .filter_map(|(_, c)| world_of(c).map(|w| w.then(&here).translation()))
                .collect();
            out.push(RoamArea {
                name: n.str("name").unwrap_or("").to_string(),
                world: here,
                half_extents,
                capacity: n.int("capacity").unwrap_or(4).max(0) as u32,
                spawn_points,
            });
            return;
        }
        for (_, c) in &n.children {
            walk(c, &here, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

/// One kind of thing the world simulation spawns, with its weights per zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnType {
    /// `type`: a `.cdoll` (`PD_Generic_Male_2.cdoll`) or a vehicle (`lave.v001_sedan.ee`).
    pub name: String,
    /// `spawnzones` ✓: weight per settlement kind (`city`, `village`, `industry`,
    /// `tourist`, `religious`, `default`).
    pub spawn_zones: Vec<(String, f32)>,
    /// `gamezones` ✓: weight per game zone of the ground (`arctic`, `desert`, `jungle`,
    /// `sea`, `city`, `grass`, `default`).
    pub game_zones: Vec<(String, f32)>,
}

impl SpawnType {
    /// The weight in a game zone (lower case); 0 when it is not listed.
    pub fn weight_in(&self, zone: &str) -> f32 {
        self.game_zones
            .iter()
            .find(|(z, _)| z == zone)
            .map_or(0.0, |(_, w)| *w)
    }

    /// The container of a `.cdoll` type: `PD_Generic_Male_2.cdoll` → `pd_generic_male_2.ee`.
    pub fn doll_container(&self) -> String {
        let lower = self.name.to_ascii_lowercase();
        format!("{}.ee", lower.strip_suffix(".cdoll").unwrap_or(&lower))
    }
}

/// `"city:5 village:5"` → `[("city", 5), ("village", 5)]`.
pub fn parse_weights(s: &str) -> Vec<(String, f32)> {
    s.split_whitespace()
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((k.to_ascii_lowercase(), v.parse().ok()?))
        })
        .collect()
}

/// The node of spawn distances in each class (`lave`, `seve`, `arve`, the people); its
/// name did not hash from the candidates.
const RANGES: u32 = 0x15ed_fea4;

/// Spawn distances of a class of things (`#15edfea4`, metres; the names inside ✓).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpawnRanges {
    pub min_inner: f32,
    pub max_inner: f32,
    pub min_outer: f32,
    pub max_outer: f32,
    pub front_despawn: f32,
    pub back_despawn: f32,
}

/// `settings\worldsim.bin`, the parts jc2-rs uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldSim {
    /// `character_types` ✓: the pedestrians.
    pub characters: Vec<SpawnType>,
    /// `lave.vehicle_types` ✓: the cars on the roads.
    pub cars: Vec<SpawnType>,
    /// `lave.max_count` ✓ (15).
    pub max_cars: u32,
    /// `lave.#15edfea4`: the cars' spawn and despawn distances.
    pub car_ranges: SpawnRanges,
}

fn types_of(n: &Node) -> Vec<SpawnType> {
    n.children
        .iter()
        .filter_map(|(_, t)| {
            let name = t.str("type")?.to_string();
            Some(SpawnType {
                name,
                spawn_zones: parse_weights(t.str("spawnzones").unwrap_or("")),
                game_zones: parse_weights(t.str("gamezones").unwrap_or("")),
            })
        })
        .collect()
}

impl WorldSim {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            if let Some(c) = root.child("character_types") {
                out.characters = types_of(c);
            }
            if let Some(l) = root.child("lave") {
                if let Some(v) = l.child("vehicle_types") {
                    out.cars = types_of(v);
                }
                out.max_cars = l.float("max_count").unwrap_or(0.0).max(0.0) as u32;
                if let Some(r) = l
                    .children
                    .iter()
                    .find(|(k, _)| k.hash() == RANGES)
                    .map(|(_, n)| n)
                {
                    let f = |k: &str| r.float(k).unwrap_or(0.0);
                    out.car_ranges = SpawnRanges {
                        min_inner: f("min_inner_spawn_range"),
                        max_inner: f("max_inner_spawn_range"),
                        min_outer: f("min_outer_spawn_range"),
                        max_outer: f("max_outer_spawn_range"),
                        front_despawn: f("front_despawn_range"),
                        back_despawn: f("back_despawn_range"),
                    };
                }
            }
        }
        Ok(out)
    }
}

/// Pick one of `types` by its weight in `zone` (falling back to `default`), with `r` in
/// 0..1. `None` when nothing has weight there.
pub fn pick<'a>(types: &'a [SpawnType], zone: &str, r: f32) -> Option<&'a SpawnType> {
    let weigh = |z: &str| -> f32 { types.iter().map(|t| t.weight_in(z)).sum() };
    let zone = if weigh(zone) > 0.0 { zone } else { "default" };
    let total = weigh(zone);
    if total <= 0.0 {
        return None;
    }
    let mut x = r.clamp(0.0, 0.999_999) * total;
    for t in types {
        let w = t.weight_in(zone);
        if x < w {
            return Some(t);
        }
        x -= w;
    }
    types.iter().rev().find(|t| t.weight_in(zone) > 0.0)
}

/// `terrain\gamezone.bin`: which game zone (`arctic`, `desert`, `jungle`, `sea`, `city`,
/// `grass`, `default`) each terrain material belongs to. Every zone node has a
/// `description` and, two levels down, `materials` ✓: the gameplay material bytes of
/// the terrain tiles (`TerrainTile::material_local`) that make that zone.
#[derive(Clone, Debug, PartialEq)]
pub struct GameZones {
    by_material: Vec<Option<String>>,
}

impl GameZones {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut by_material = vec![None; 256];
        for root in &file.nodes {
            root.walk(&mut |n| {
                let Some(name) = n.str("description") else {
                    return;
                };
                n.walk(&mut |m| {
                    if let Some(Variant::Ints(list)) = m.value("materials") {
                        for &i in list {
                            if let Some(slot) = by_material.get_mut(i as usize) {
                                *slot = Some(name.to_ascii_lowercase());
                            }
                        }
                    }
                });
            });
        }
        Ok(Self { by_material })
    }

    /// The zone of a terrain material, `default` for one no zone lists.
    pub fn of_material(&self, material: u8) -> &str {
        self.by_material[material as usize]
            .as_deref()
            .unwrap_or("default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_and_picks() {
        let w = parse_weights("city:30 sea:30");
        assert_eq!(w, [("city".into(), 30.0), ("sea".into(), 30.0)]);
        let types = vec![
            SpawnType {
                name: "a".into(),
                game_zones: parse_weights("city:30 default:10"),
                ..Default::default()
            },
            SpawnType {
                name: "b".into(),
                game_zones: parse_weights("city:10 jungle:5"),
                ..Default::default()
            },
        ];
        assert_eq!(pick(&types, "city", 0.0).unwrap().name, "a");
        assert_eq!(pick(&types, "city", 0.8).unwrap().name, "b");
        assert_eq!(pick(&types, "jungle", 0.1).unwrap().name, "b");
        // no weight in the arctic: the defaults
        assert_eq!(pick(&types, "arctic", 0.9).unwrap().name, "a");
        let doll = SpawnType {
            name: "PD_Generic_Male_2.cdoll".into(),
            ..Default::default()
        };
        assert_eq!(doll.doll_container(), "pd_generic_male_2.ee");
    }

    /// `worldsim.bin`, `gamezone.bin` and the roam areas of the village 26,41. Needs
    /// JC2_TEST_DIR.
    #[test]
    fn worldsim_people_cars_and_roam_areas() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("worldsim.bin") else {
            return;
        };
        let ws = WorldSim::parse(&data).unwrap();
        assert_eq!(ws.characters.len(), 31, "{:?}", ws.characters);
        assert_eq!(ws.cars.len(), 35);
        assert_eq!(ws.max_cars, 15);
        assert_eq!(ws.car_ranges.max_outer, 160.0);
        let sedan = ws
            .cars
            .iter()
            .find(|c| c.name == "lave.v001_sedan.ee")
            .unwrap();
        assert_eq!(sedan.weight_in("city"), 30.0);
        // every person's doll is in the archives and dresses on the civilian skeleton
        for c in &ws.characters {
            assert!(fs.contains(&c.doll_container()), "{}", c.doll_container());
        }
        // their models are skinned to the 40 bones of the civilian skeleton
        let data = fs.read_inflated("pd_generic_female_5.ee").unwrap();
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let skel =
            crate::animation::Skeleton::parse(&fs.read_inflated("civilian_biped.bsk").unwrap())
                .unwrap();
        for e in &sarc.entries {
            if e.name.contains("_lod1-") && e.name.ends_with(".rbm") {
                let m = crate::rbm::RbmModel::parse(sarc.data(&data, e).unwrap()).unwrap();
                let max = m
                    .blocks
                    .iter()
                    .flat_map(|b| b.mesh.joints.iter().flatten())
                    .max()
                    .copied()
                    .unwrap_or(0);
                eprintln!("{}: joints up to {max} of {}", e.name, skel.bones.len());
                assert!((max as usize) < skel.bones.len(), "{}: joint {max}", e.name);
            }
        }
        let jungle = pick(&ws.characters, "jungle", 0.5).unwrap();
        assert!(jungle.weight_in("jungle") > 0.0);

        let zones = GameZones::parse(&fs.read_inflated("gamezone.bin").unwrap()).unwrap();
        assert_eq!(zones.of_material(33), "city");
        assert_eq!(zones.of_material(0x41), "arctic");
        assert_eq!(zones.of_material(16), "jungle");
        assert_eq!(zones.of_material(255), "default");

        let file = crate::roads::container_placements(&fs, "26x41.nlz").unwrap();
        let areas = roam_areas(&file);
        assert_eq!(areas.len(), 2);
        let a = areas.iter().find(|a| a.capacity == 8).unwrap();
        assert_eq!(a.half_extents, [15.0, 5.0, 65.0]);
        assert_eq!(a.spawn_points.len(), 4);
        assert!(a.spawn_points.iter().all(|p| a.contains(*p, 0.5)));
        assert!(a.contains(a.point(0.9, -0.9), 0.01));
        assert!(!a.contains(a.point(1.0, 0.0), -1.0));
    }
}
