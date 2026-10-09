//! Collision shapes out of the Havok packfiles (`.pfx`): the rigid bodies of the
//! `hkpPhysicsData` inside, each with its transform and a tree of shapes.
//!
//! Class layouts (Havok 5.5, 32-bit; offsets from the object start, see
//! `research/formats/havok.md`):
//!
//! | Class | Fields |
//! |---|---|
//! | `hkpShape` | 0 vtable, 4 refcount, 8 userData, 12 type |
//! | `hkpConvexShape` | 16 radius |
//! | `hkpBoxShape` | 32 halfExtents |
//! | `hkpSphereShape` | (radius only) |
//! | `hkpCapsuleShape` | 32 vertexA, 48 vertexB (w = radius) |
//! | `hkpCylinderShape` | 20 cylRadius, 24 base radius factor, 32 vertexA, 48 vertexB, 64/80 perpendiculars |
//! | `hkpConvexVerticesShape` | 32 aabbHalfExtents, 48 aabbCenter, 64 rotatedVertices (array of 48-byte transposed 4-point groups: xxxx yyyy zzzz), 76 numVertices, 80 planeEquations |
//! | `hkpConvexTranslateShape` | 24 child, 32 translation |
//! | `hkpConvexTransformShape` | 24 child, 32 transform (rotation columns, translation) |
//! | `hkpTransformShape` | 20 child, 32 rotation (quaternion), 48 transform |
//! | `hkpListShape` | 24 childInfo array, 16-byte entries with the child at +0 |
//! | `hkpMoppBvTreeShape` | 16 MOPP code, 52 child |
//! | `hkp*ExtendedMeshShape` | 32 scaling, 48 aabbHalfExtents, 64 aabbCenter, 80 triangle subparts, 176 radius; pointers to `*MeshSubpartStorage` objects (8 vertices, 20 indices8, 32 indices16, 44 indices32) and to extra shapes |
//! | `hkpRigidBody` | 16 shape, 224 transform (rotation columns 224/240/256, translation 272) |
//!
//! Also here: the tiles' `XXxZZ.pfi` lists — which `.pfx` sits where in a tile.

use crate::havok::Packfile;
use crate::placement::Transform3;

#[derive(Clone, Debug, PartialEq)]
pub enum CollisionShape {
    /// `half_extents` already include the convex radius (Havok's collision margin).
    Box {
        half_extents: [f32; 3],
        radius: f32,
    },
    Sphere {
        radius: f32,
    },
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
    },
    Cylinder {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
    },
    ConvexHull {
        points: Vec<[f32; 3]>,
        radius: f32,
    },
    TriMesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
    Transformed {
        transform: Transform3,
        child: Box<CollisionShape>,
    },
    Compound {
        children: Vec<CollisionShape>,
    },
    Unsupported {
        class: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollisionBody {
    pub transform: Transform3,
    pub shape: CollisionShape,
}

impl CollisionShape {
    /// Leaves with their transform relative to the body: compounds and transformed
    /// shapes unfolded, `Unsupported` dropped.
    pub fn flatten(&self) -> Vec<(Transform3, CollisionShape)> {
        let mut out = Vec::new();
        self.flatten_into(&Transform3::IDENTITY, &mut out);
        out
    }

    fn flatten_into(&self, parent: &Transform3, out: &mut Vec<(Transform3, CollisionShape)>) {
        match self {
            CollisionShape::Transformed { transform, child } => {
                child.flatten_into(&transform.then(parent), out)
            }
            CollisionShape::Compound { children } => {
                for c in children {
                    c.flatten_into(parent, out);
                }
            }
            CollisionShape::Unsupported { .. } => {}
            leaf => out.push((*parent, leaf.clone())),
        }
    }

    pub fn leaf_count(&self) -> usize {
        self.flatten().len()
    }

    pub fn kind(&self) -> &'static str {
        match self {
            CollisionShape::Box { .. } => "box",
            CollisionShape::Sphere { .. } => "sphere",
            CollisionShape::Capsule { .. } => "capsule",
            CollisionShape::Cylinder { .. } => "cylinder",
            CollisionShape::ConvexHull { .. } => "hull",
            CollisionShape::TriMesh { .. } => "mesh",
            CollisionShape::Transformed { .. } => "transformed",
            CollisionShape::Compound { .. } => "compound",
            CollisionShape::Unsupported { .. } => "unsupported",
        }
    }
}

/// An `hkTransform` (three rotation columns and a translation, 64 bytes) as a
/// row-vector transform.
fn hk_transform(pf: &Packfile, off: u32) -> Transform3 {
    let c0 = pf.vec3(off);
    let c1 = pf.vec3(off + 16);
    let c2 = pf.vec3(off + 32);
    let t = pf.vec3(off + 48);
    // column-vector M with columns c0 c1 c2 ⇔ row-vector form with rows c0 c1 c2
    Transform3::from_mat4x3([
        c0[0], c0[1], c0[2], c1[0], c1[1], c1[2], c2[0], c2[1], c2[2], t[0], t[1], t[2],
    ])
}

fn is_shape_class(class: &str) -> bool {
    class.starts_with("hkp") && class.ends_with("Shape")
}

/// Pointers stored inside `[off, off + size)` that resolve to objects: `(target, class)`.
fn object_pointers(pf: &Packfile, off: u32, size: u32) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut p = off;
    while p + 4 <= off + size {
        if let Some(to) = pf.pointer(p)
            && let Some(class) = pf.class_at(to)
        {
            out.push((to, class.to_string()));
        }
        p += 4;
    }
    out
}

fn convert(pf: &Packfile, off: u32, depth: usize) -> CollisionShape {
    let class = pf.class_at(off).unwrap_or("?").to_string();
    if depth > 16 {
        return CollisionShape::Unsupported { class };
    }
    let child = |ptr_off: u32| -> CollisionShape {
        match pf.pointer(ptr_off) {
            Some(c) => convert(pf, c, depth + 1),
            None => CollisionShape::Unsupported {
                class: format!("{class} (null child)"),
            },
        }
    };
    match class.as_str() {
        // The convex radius is a collision margin around the core shape: a box's real
        // extent is halfExtents + radius, a cylinder's radius is cylRadius + radius.
        "hkpBoxShape" => {
            let r = pf.f32(off + 16);
            let h = pf.vec3(off + 32);
            CollisionShape::Box {
                half_extents: [h[0] + r, h[1] + r, h[2] + r],
                radius: r,
            }
        }
        "hkpSphereShape" => CollisionShape::Sphere {
            radius: pf.f32(off + 16),
        },
        "hkpCapsuleShape" => CollisionShape::Capsule {
            a: pf.vec3(off + 32),
            b: pf.vec3(off + 48),
            radius: pf.f32(off + 16),
        },
        "hkpCylinderShape" => CollisionShape::Cylinder {
            a: pf.vec3(off + 32),
            b: pf.vec3(off + 48),
            radius: pf.f32(off + 20) + pf.f32(off + 16),
        },
        "hkpConvexVerticesShape" => {
            let n = pf.u32(off + 76) as usize;
            let mut points = Vec::with_capacity(n);
            if let Some((groups, count)) = pf.array(off + 64) {
                for g in 0..count {
                    let base = groups + g * 48;
                    let xs = pf.vec4(base);
                    let ys = pf.vec4(base + 16);
                    let zs = pf.vec4(base + 32);
                    for k in 0..4 {
                        if points.len() < n {
                            points.push([xs[k], ys[k], zs[k]]);
                        }
                    }
                }
            }
            CollisionShape::ConvexHull {
                points,
                radius: pf.f32(off + 16),
            }
        }
        "hkpConvexTranslateShape" => {
            let t = pf.vec3(off + 32);
            CollisionShape::Transformed {
                transform: Transform3::from_mat4x3([
                    1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, t[0], t[1], t[2],
                ]),
                child: Box::new(child(off + 24)),
            }
        }
        "hkpConvexTransformShape" => CollisionShape::Transformed {
            transform: hk_transform(pf, off + 32),
            child: Box::new(child(off + 24)),
        },
        "hkpTransformShape" => CollisionShape::Transformed {
            transform: hk_transform(pf, off + 48),
            child: Box::new(child(off + 20)),
        },
        "hkpListShape" => {
            let mut children = Vec::new();
            if let Some((entries, n)) = pf.array(off + 24) {
                for i in 0..n {
                    if let Some(c) = pf.pointer(entries + i * 16) {
                        children.push(convert(pf, c, depth + 1));
                    }
                }
            }
            CollisionShape::Compound { children }
        }
        "hkpMoppBvTreeShape" => child(off + 52),
        c if c.contains("ExtendedMeshShape")
            || c == "hkpMeshShape"
            || c == "hkpSimpleMeshShape" =>
        {
            let size = pf
                .objects
                .iter()
                .find(|o| o.offset == off)
                .map(|o| o.size)
                .unwrap_or(0);
            let mut children = Vec::new();
            let mut vertices: Vec<[f32; 3]> = Vec::new();
            let mut triangles: Vec<[u32; 3]> = Vec::new();
            for (target, tclass) in object_pointers(pf, off, size) {
                if tclass.ends_with("MeshSubpartStorage") {
                    let base = vertices.len() as u32;
                    if let Some((v, n)) = pf.array(target + 8) {
                        for i in 0..n {
                            vertices.push(pf.vec3(v + i * 16));
                        }
                    }
                    let mut idx: Vec<u32> = Vec::new();
                    if let Some((p, n)) = pf.array(target + 20) {
                        idx.extend((0..n).map(|i| pf.u8(p + i) as u32));
                    }
                    if let Some((p, n)) = pf.array(target + 32) {
                        idx.extend((0..n).map(|i| pf.u16(p + i * 2) as u32));
                    }
                    if let Some((p, n)) = pf.array(target + 44) {
                        idx.extend((0..n).map(|i| pf.u32(p + i * 4)));
                    }
                    for t in idx.as_chunks::<3>().0 {
                        triangles.push([t[0] + base, t[1] + base, t[2] + base]);
                    }
                } else if is_shape_class(&tclass) && target != off {
                    children.push(convert(pf, target, depth + 1));
                }
            }
            if !triangles.is_empty() {
                children.insert(
                    0,
                    CollisionShape::TriMesh {
                        vertices,
                        triangles,
                    },
                );
            }
            match children.len() {
                0 => CollisionShape::Unsupported { class },
                1 => children.pop().unwrap(),
                _ => CollisionShape::Compound { children },
            }
        }
        _ => CollisionShape::Unsupported { class },
    }
}

/// Every rigid body in the packfile with its shape tree.
pub fn bodies(pf: &Packfile) -> Vec<CollisionBody> {
    pf.objects_of("hkpRigidBody")
        .map(|o| {
            let shape = match pf.pointer(o.offset + 16) {
                Some(s) => convert(pf, s, 0),
                None => CollisionShape::Unsupported {
                    class: "hkpRigidBody (no shape)".into(),
                },
            };
            CollisionBody {
                transform: hk_transform(pf, o.offset + 224),
                shape,
            }
        })
        .collect()
}

/// One placed collision shape of a tile.
#[derive(Clone, Debug, PartialEq)]
pub struct CollisionInstance {
    /// Full engine path of the `.pfx`.
    pub pfx: String,
    pub name_hash: u32,
    /// Transform relative to the list's base.
    pub local: Transform3,
    /// By value: the same for every instance of a shape (its size in memory?).
    pub size: u32,
}

/// `XXxZZ.pfi`: `"PFI\0"`, u32 1, u32 2, u32 count, a 4×4 base matrix (row-major, the
/// translation in the last row), 48 bytes of in-memory leftovers, then per instance
/// `u32 hash, u32 length, path, 4×4 matrix, u32 size`. Instance transforms are relative
/// to the base: the first instance of `31x14.pfi` lands on the village health locker.
#[derive(Clone, Debug, PartialEq)]
pub struct CollisionInstances {
    pub base: Transform3,
    pub instances: Vec<CollisionInstance>,
}

fn mat4_to_transform(m: &[f32]) -> Transform3 {
    Transform3::from_mat4x3([
        m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10], m[12], m[13], m[14],
    ])
}

impl CollisionInstances {
    pub const MAGIC: &'static [u8; 4] = b"PFI\0";

    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        use crate::reader::Reader;
        let mut r = Reader::new(data);
        if r.bytes(4)? != Self::MAGIC {
            return Err(crate::Error::format("not a PFI collision instance list"));
        }
        let _one = r.u32()?;
        let _two = r.u32()?;
        let count = r.u32()? as usize;
        let mut m = [0.0f32; 16];
        for v in m.iter_mut() {
            *v = r.f32()?;
        }
        let base = mat4_to_transform(&m);
        r.bytes(48)?;
        let mut instances = Vec::with_capacity(count);
        for _ in 0..count {
            let name_hash = r.u32()?;
            let len = r.u32()? as usize;
            let pfx = r.str(len)?;
            for v in m.iter_mut() {
                *v = r.f32()?;
            }
            let size = r.u32()?;
            instances.push(CollisionInstance {
                pfx,
                name_hash,
                local: mat4_to_transform(&m),
                size,
            });
        }
        Ok(Self { base, instances })
    }

    /// World transform of instance `i`.
    pub fn world(&self, i: usize) -> Transform3 {
        self.instances[i].local.then(&self.base)
    }
}

/// Shortcut: parse a `.pfx` and return its bodies.
pub fn parse_pfx(bytes: &[u8]) -> crate::Result<Vec<CollisionBody>> {
    let pf = Packfile::parse(bytes)?;
    Ok(bodies(&pf))
}

/// Axis-aligned bounds of all leaves (in body space), for sanity checks.
pub fn bounds(body: &CollisionBody) -> Option<([f32; 3], [f32; 3])> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;
    let mut add = |p: [f32; 3]| {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
        any = true;
    };
    for (tf, leaf) in body.shape.flatten() {
        let tf = tf.then(&body.transform);
        match leaf {
            CollisionShape::Box {
                half_extents: h, ..
            } => {
                for k in 0..8 {
                    let c = [
                        if k & 1 == 0 { -h[0] } else { h[0] },
                        if k & 2 == 0 { -h[1] } else { h[1] },
                        if k & 4 == 0 { -h[2] } else { h[2] },
                    ];
                    add(tf.transform_point(c));
                }
            }
            CollisionShape::Sphere { radius } => {
                let c = tf.translation();
                add([c[0] - radius, c[1] - radius, c[2] - radius]);
                add([c[0] + radius, c[1] + radius, c[2] + radius]);
            }
            CollisionShape::Capsule { a, b, radius }
            | CollisionShape::Cylinder { a, b, radius } => {
                for p in [a, b] {
                    let w = tf.transform_point(p);
                    add([w[0] - radius, w[1] - radius, w[2] - radius]);
                    add([w[0] + radius, w[1] + radius, w[2] + radius]);
                }
            }
            CollisionShape::ConvexHull { points, .. } => {
                for p in points {
                    add(tf.transform_point(p));
                }
            }
            CollisionShape::TriMesh { vertices, .. } => {
                for p in vertices {
                    add(tf.transform_point(p));
                }
            }
            _ => {}
        }
    }
    any.then_some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sarc::SmallArchive;

    /// Needs JC2_TEST_DIR with `31x14.nlz` and `lave.v046_buscoach.ee` reachable.
    #[test]
    fn shapes_from_test_data() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
        let mut unsupported = Vec::new();
        for container in ["31x14.nlz", "lave.v046_buscoach.ee"] {
            let Ok(data) = fs.read_inflated(container) else {
                continue;
            };
            let sarc = SmallArchive::parse(&data).unwrap();
            for e in &sarc.entries {
                if !e.name.to_ascii_lowercase().ends_with(".pfx") {
                    continue;
                }
                let bodies = parse_pfx(sarc.data(&data, e).unwrap()).unwrap();
                assert_eq!(bodies.len(), 1, "{}", e.name);
                let b = &bodies[0];
                for (_, leaf) in b.shape.flatten() {
                    *kinds.entry(leaf.kind()).or_default() += 1;
                }
                if b.shape.flatten().is_empty() {
                    unsupported.push(format!("{}: {:?}", e.name, b.shape));
                }
                let (lo, hi) = bounds(b).unwrap_or_else(|| panic!("{}: no bounds", e.name));
                for i in 0..3 {
                    assert!(
                        hi[i] - lo[i] < 60.0 && hi[i] >= lo[i],
                        "{}: {lo:?} {hi:?}",
                        e.name
                    );
                }
                if e.name == "v046_lod1-body_col.pfx" {
                    // the bus is about 13 m long and 4 m tall
                    assert!(hi[2] - lo[2] > 11.0 && hi[1] - lo[1] > 3.0, "{lo:?} {hi:?}");
                    assert!(
                        b.shape
                            .flatten()
                            .iter()
                            .any(|(_, l)| matches!(l, CollisionShape::TriMesh { .. }))
                    );
                }
                if e.name == "gp001_lod1-b_col.pfx" {
                    let leaves = b.shape.flatten();
                    assert_eq!(leaves.len(), 1);
                    assert!(matches!(leaves[0].1, CollisionShape::Box { .. }));
                    let t = leaves[0].0.translation();
                    assert!(
                        (t[0] + 0.345).abs() < 0.01 && (t[1] - 0.434).abs() < 0.01,
                        "{t:?}"
                    );
                }
            }
        }
        eprintln!("leaf kinds: {kinds:?}");
        assert!(unsupported.is_empty(), "{unsupported:?}");
        assert!(
            kinds.contains_key("box") && kinds.contains_key("hull") && kinds.contains_key("mesh")
        );
    }

    #[test]
    fn instance_list() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("31x14.nlz") else {
            return;
        };
        let sarc = SmallArchive::parse(&data).unwrap();
        let e = sarc.get("31x14.pfi").unwrap();
        let list = CollisionInstances::parse(sarc.data(&data, e).unwrap()).unwrap();
        assert_eq!(list.instances.len(), 167);
        assert!(list.instances[0].pfx.ends_with("gp001_lod1-a_col.pfx"));
        // the health locker of the village
        let t = list.world(0).translation();
        assert!(
            (t[0] + 462.4).abs() < 0.5 && (t[1] - 837.7).abs() < 0.5 && (t[2] + 8845.6).abs() < 0.5,
            "{t:?}"
        );
        for i in 0..list.instances.len() {
            let t = list.world(i).translation();
            assert!(
                t[0] > -1100.0 && t[0] < 100.0 && t[2] > -9300.0 && t[2] < -8300.0,
                "{i}: {t:?}"
            );
            assert!(list.instances[i].pfx.to_ascii_lowercase().ends_with(".pfx"));
        }
    }
}
