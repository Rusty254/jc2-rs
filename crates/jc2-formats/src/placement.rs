//! Object placements: the `.blo` property files in the location containers
//! (`locations\wc\XX\XXxZZ.nlz` → `XXxZZ.nl.blo`) list every object of a tile as a tree
//! of property nodes. A node with a `_class` is an object; its `world` value is its
//! transform (a 4×3 matrix: three basis rows then the translation, row-vector
//! convention), absolute for root objects and relative to the parent for nested ones.
//! Objects that draw a model carry `filename` (a `.lod` descriptor or `.rbm` path) and
//! usually `collision` (a `.pfx` Havok shape); `visible = 0` marks parts that only
//! appear later (the `_dst` destroyed-state pieces of damageable buildings).
//!
//! Axes are the engine's, right-handed like Bevy's: x east (a model's right), y up, z south
//! (a model faces −z; `research/formats/coordinates.md`). Positions range over
//! the whole world (±16 384 m), not the tile.

use crate::property::{Node, PropertyFile, Variant};

/// An affine transform stored the engine's way: rows 0..3 are the x, y and z basis
/// vectors, row 3 is the translation; points transform as `p' = p·M + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform3 {
    pub m: [f32; 12],
}

impl Transform3 {
    pub const IDENTITY: Self = Self {
        m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
    };

    pub fn from_mat4x3(m: [f32; 12]) -> Self {
        Self { m }
    }

    /// From a row-major 4×4 (the blackboard layout stores these); the last column is dropped.
    pub fn from_mat4x4(m: [f32; 16]) -> Self {
        let mut out = [0.0; 12];
        for r in 0..4 {
            out[r * 3..r * 3 + 3].copy_from_slice(&m[r * 4..r * 4 + 3]);
        }
        Self { m: out }
    }

    pub fn row(&self, r: usize) -> [f32; 3] {
        [self.m[r * 3], self.m[r * 3 + 1], self.m[r * 3 + 2]]
    }

    pub fn translation(&self) -> [f32; 3] {
        self.row(3)
    }

    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let (x, y, z) = (self.row(0), self.row(1), self.row(2));
        let t = self.row(3);
        [
            p[0] * x[0] + p[1] * y[0] + p[2] * z[0] + t[0],
            p[0] * x[1] + p[1] * y[1] + p[2] * z[1] + t[1],
            p[0] * x[2] + p[1] * y[2] + p[2] * z[2] + t[2],
        ]
    }

    pub fn transform_vector(&self, p: [f32; 3]) -> [f32; 3] {
        let (x, y, z) = (self.row(0), self.row(1), self.row(2));
        [
            p[0] * x[0] + p[1] * y[0] + p[2] * z[0],
            p[0] * x[1] + p[1] * y[1] + p[2] * z[1],
            p[0] * x[2] + p[1] * y[2] + p[2] * z[2],
        ]
    }

    /// This transform (local, inside `parent`) expressed in the parent's space:
    /// `p·(self·parent)`.
    pub fn then(&self, parent: &Transform3) -> Transform3 {
        let mut m = [0.0; 12];
        for r in 0..3 {
            let v = parent.transform_vector(self.row(r));
            m[r * 3..r * 3 + 3].copy_from_slice(&v);
        }
        let t = parent.transform_point(self.translation());
        m[9..12].copy_from_slice(&t);
        Transform3 { m }
    }
}

/// One placed object.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub class: String,
    pub name: String,
    /// Absolute transform in engine axes.
    pub world: Transform3,
    /// `filename`: the `.lod` descriptor (or model) this object draws, if any.
    pub model: Option<String>,
    /// `collision`: the `.pfx` shape, if any.
    pub collision: Option<String>,
    /// `visible` (default on): destroyed-state parts and other initially hidden
    /// objects carry `visible = 0`.
    pub visible: bool,
    /// `grappling_weight_category` (0, 1 or 2 in the shipped data) when the object
    /// authors it. `None` = the key is absent (characters never carry it).
    pub grapple_weight: Option<u8>,
    /// Nesting depth; root objects are 0.
    pub depth: usize,
    /// Index of the root node in the file this object came from.
    pub root: usize,
}

impl Placement {
    pub fn position(&self) -> [f32; 3] {
        self.world.translation()
    }
}

fn node_transform(node: &Node) -> Option<Transform3> {
    match node.value("world")? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        Variant::Mat4x4(m) => Some(Transform3::from_mat4x4(*m)),
        _ => None,
    }
}

fn collect(node: &Node, parent: &Transform3, depth: usize, root: usize, out: &mut Vec<Placement>) {
    let mut here = *parent;
    if let Some(class) = node.str("_class") {
        if let Some(local) = node_transform(node) {
            here = local.then(parent);
        }
        out.push(Placement {
            class: class.to_string(),
            name: node.str("name").unwrap_or("").to_string(),
            world: here,
            // Cutscene filename identifies a sequence, not render geometry. The raw
            // property and script consumers retain it; this is no cutscene receipt.
            model: node
                .str("filename")
                .filter(|_| class != "CCutsceneObject")
                .map(str::to_string),
            collision: node.str("collision").map(str::to_string),
            visible: node.int("visible").is_none_or(|v| v != 0),
            grapple_weight: node
                .int("grappling_weight_category")
                .and_then(|v| u8::try_from(v).ok())
                .filter(|v| *v < 0x80),
            depth,
            root,
        });
        for (_, child) in &node.children {
            collect(child, &here, depth + 1, root, out);
        }
    } else {
        for (_, child) in &node.children {
            collect(child, &here, depth, root, out);
        }
    }
}

/// A vehicle a location spawns: the `CVehicleComponent` (`#fa726895` = the container,
/// `vehicles\lave\lave.v044truck.ee`) of a `CSpawnSet`, at the set's transform.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSpawn {
    pub name: String,
    pub container: String,
    pub world: Transform3,
    /// `spawn_distance`: how close the player comes before it appears (m).
    pub spawn_distance: f32,
}

const VEHICLE_CONTAINER: u32 = 0xfa72_6895;

/// The vehicle spawn sets of a placement file.
pub fn vehicle_spawns(file: &PropertyFile) -> Vec<VehicleSpawn> {
    fn find_vehicle(node: &Node) -> Option<String> {
        if node.str("_class") == Some("CVehicleComponent")
            && let Some(crate::property::Variant::Str(c)) = node.value_hash(VEHICLE_CONTAINER)
            && !c.is_empty()
        {
            return Some(c.clone());
        }
        node.children.iter().find_map(|(_, c)| find_vehicle(c))
    }
    fn walk(node: &Node, parent: &Transform3, out: &mut Vec<VehicleSpawn>) {
        let mut here = *parent;
        if let Some(local) = node_transform(node) {
            here = local.then(parent);
        }
        if node.str("_class") == Some("CSpawnSet") {
            if let Some(container) = find_vehicle(node) {
                out.push(VehicleSpawn {
                    name: node.str("name").unwrap_or("").to_string(),
                    container,
                    world: here,
                    spawn_distance: node.float("spawn_distance").unwrap_or(100.0),
                });
            }
            return;
        }
        for (_, c) in &node.children {
            walk(c, &here, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

/// An exported entity a location spawns: the `CExportedEntityComponent` of a
/// `CSpawnSet` (`#f25a8a76` = the container, `weaponstashes\mod.stash.smg.ee`), at the
/// set's transform. Weapon stashes and other `.ee` objects.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedSpawn {
    pub name: String,
    pub entity: String,
    pub world: Transform3,
}

const EXPORTED_ENTITY: u32 = 0xf25a_8a76;

/// The exported-entity spawn sets of a placement file.
pub fn exported_spawns(file: &PropertyFile) -> Vec<ExportedSpawn> {
    fn walk(node: &Node, parent: &Transform3, out: &mut Vec<ExportedSpawn>) {
        let here = node_transform(node)
            .map(|l| l.then(parent))
            .unwrap_or(*parent);
        if node.str("_class") == Some("CSpawnSet") {
            for (_, c) in &node.children {
                if c.str("_class") == Some("CExportedEntityComponent")
                    && let Some(crate::property::Variant::Str(e)) = c.value_hash(EXPORTED_ENTITY)
                    && !e.is_empty()
                {
                    out.push(ExportedSpawn {
                        name: node.str("name").unwrap_or("").to_string(),
                        entity: e.clone(),
                        world: here,
                    });
                }
            }
            return;
        }
        for (_, c) in &node.children {
            walk(c, &here, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

/// Every object in a placement file, depth-first, with absolute transforms.
pub fn placements(file: &PropertyFile) -> Vec<Placement> {
    let mut out = Vec::new();
    for (i, root) in file.nodes.iter().enumerate() {
        collect(root, &Transform3::IDENTITY, 0, i, &mut out);
    }
    out
}

/// The file-name part of an engine path (`a\b\c.lod` → `c.lod`), which is how
/// container members and archive entries are keyed.
pub fn bare_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Key;

    fn obj(class: &str, world: Option<[f32; 12]>, model: Option<&str>) -> Node {
        let mut n = Node::default();
        n.values
            .push((Key::Name("_class".into()), Variant::Str(class.into())));
        if let Some(w) = world {
            n.values
                .push((Key::Name("world".into()), Variant::Mat4x3(w)));
        }
        if let Some(m) = model {
            n.values
                .push((Key::Name("filename".into()), Variant::Str(m.into())));
        }
        n
    }

    #[test]
    fn compose() {
        // parent: rotate 90° about y (x → -z in row-vector form) and move to (10, 0, 0)
        let parent =
            Transform3::from_mat4x3([0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
        let local =
            Transform3::from_mat4x3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0]);
        let w = local.then(&parent);
        let t = w.translation();
        // (1,2,3)·R = (3, 2, -1), plus (10,0,0)
        assert!(
            (t[0] - 13.0).abs() < 1e-6 && (t[1] - 2.0).abs() < 1e-6 && (t[2] + 1.0).abs() < 1e-6,
            "{t:?}"
        );
        assert_eq!(w.row(0), [0.0, 0.0, -1.0]);
        assert_eq!(Transform3::IDENTITY.then(&parent), parent);
    }

    #[test]
    fn tree() {
        let mut root = Node::default();
        let mut building = obj(
            "CGeometryObject",
            Some([
                1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 100.0, 5.0, -200.0,
            ]),
            None,
        );
        let part = obj(
            "CSimpleRigidObject",
            Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0]),
            Some("structures\\x\\door.lod"),
        );
        building.children.push((Key::Hash(1), part));
        root.children.push((Key::Hash(2), building));
        let file = PropertyFile { nodes: vec![root] };
        let p = placements(&file);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].class, "CGeometryObject");
        assert_eq!(p[0].depth, 0);
        assert_eq!(p[1].position(), [100.0, 8.0, -200.0]);
        assert_eq!(p[1].model.as_deref(), Some("structures\\x\\door.lod"));
        assert_eq!(bare_name(p[1].model.as_deref().unwrap()), "door.lod");
        assert_eq!(p[1].depth, 1);
    }

    #[test]
    fn grapple_weight_is_read_when_authored() {
        let mut part = obj("CSimpleRigidObject", None, None);
        part.values.push((
            Key::Name("grappling_weight_category".into()),
            Variant::Int(2),
        ));
        let mut root = Node::default();
        root.children.push((Key::Hash(1), part));
        root.children
            .push((Key::Hash(2), obj("CSimpleRigidObject", None, None)));
        let p = placements(&PropertyFile { nodes: vec![root] });
        assert_eq!(p[0].grapple_weight, Some(2));
        assert_eq!(p[1].grapple_weight, None);
    }

    /// Needs JC2_TEST_DIR with `31x14.nlz`.
    #[test]
    fn village_vehicle_spawns() {
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
        let v = vehicle_spawns(&file);
        let names: Vec<&str> = v.iter().map(|s| bare_name(&s.container)).collect();
        assert!(names.contains(&"arve.v060_attackheli.ee"), "{names:?}");
        assert!(names.contains(&"lave.v044truck.ee"), "{names:?}");
        let heli = v
            .iter()
            .find(|s| s.container.ends_with("v060_attackheli.ee"))
            .unwrap();
        let t = heli.world.translation();
        assert!(
            (t[0] + 401.1).abs() < 0.1 && (t[2] + 8879.7).abs() < 0.1,
            "{t:?}"
        );
        assert_eq!(heli.spawn_distance, 200.0);
        // three weapon stashes: the SMG, grenades, the Beretta
        let x = exported_spawns(&file);
        let stashes: Vec<&str> = x.iter().map(|s| bare_name(&s.entity)).collect();
        assert_eq!(
            stashes,
            [
                "mod.stash.smg.ee",
                "mod.stash.grenade.ee",
                "mod.stash.beretta.ee"
            ]
        );
        let t = x[0].world.translation();
        assert!(
            (t[0] + 458.79).abs() < 0.1 && (t[2] + 8857.33).abs() < 0.1,
            "{t:?}"
        );
    }
}
