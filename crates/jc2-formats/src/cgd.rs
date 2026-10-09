//! `XXxZZ.nl.cgd`: the compiled static geometry of a location — the props that have
//! no object of their own in the placement file (`CGeometryObject` bodies, fences,
//! poles, crates…) and are drawn straight from this list.
//!
//! Layout (read off `31x14.nl.cgd`, 11 124 bytes, and checked against the tile's
//! `.pfi` collision list, which carries the same `gp070`/`gp071`/`go173` shapes at
//! the same places):
//!
//! ```text
//! u32   0x83030314           magic
//! u32   7                    version
//! u32   count                instances
//! u32   models               distinct `.lod` names
//! u32   strings              size of the name table at the end
//! u8    [40]                 zero (in-memory pointers)
//! f32   [count × 16]         world matrix per instance, row-major 4×4, translation in
//!                            the last row (row-vector convention like the `.blo`)
//! u16   [count]              model index into the name table
//! u8    [count × 12]         zero
//! f32   [count]              −1.0 (unused: a fade distance?)
//! u16   [count]              3    (LOD/flags, the same for every instance seen)
//! f32   [count × 6]          world-space AABB min, max
//! char  [strings]            NUL-separated `.lod` file names (bare, no path)
//! ```
//!
//! The instances are absolute (the first of `31x14` is `gp001-a.lod` at
//! −462.36, 837.71, −8845.61: the health-locker body whose door and medicine kit sit
//! in the `.blo` as `CSimpleRigidObject`s at the same spot).

use crate::error::{Error, Result};
use crate::placement::Transform3;
use crate::reader::Reader;

#[derive(Clone, Debug, PartialEq)]
pub struct StaticInstance {
    /// Index into [`CompiledGeometry::models`].
    pub model: usize,
    /// Absolute transform in engine axes.
    pub world: Transform3,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub flags: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompiledGeometry {
    pub version: u32,
    /// Bare `.lod` names.
    pub models: Vec<String>,
    pub instances: Vec<StaticInstance>,
}

impl CompiledGeometry {
    pub const MAGIC: u32 = 0x8303_0314;

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let magic = r.u32()?;
        if magic != Self::MAGIC {
            return Err(Error::format(format!(
                "not a compiled geometry list (magic {magic:08x})"
            )));
        }
        let version = r.u32()?;
        let count = r.u32()? as usize;
        let model_count = r.u32()? as usize;
        let strings = r.u32()? as usize;
        r.bytes(40)?;
        let mut worlds = Vec::with_capacity(count);
        for _ in 0..count {
            let mut m = [0.0f32; 16];
            for v in m.iter_mut() {
                *v = r.f32()?;
            }
            worlds.push(Transform3::from_mat4x3([
                m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10], m[12], m[13], m[14],
            ]));
        }
        let indices = r.u16_vec(count)?;
        r.bytes(count * 12)?;
        for _ in 0..count {
            r.f32()?;
        }
        let flags = r.u16_vec(count)?;
        let mut boxes = Vec::with_capacity(count);
        for _ in 0..count {
            let mut b = [0.0f32; 6];
            for v in b.iter_mut() {
                *v = r.f32()?;
            }
            boxes.push(b);
        }
        let table = r.bytes(strings)?;
        let models: Vec<String> = table
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        if models.len() != model_count {
            return Err(Error::format(format!(
                "cgd names {} but header says {model_count}",
                models.len()
            )));
        }
        let mut instances = Vec::with_capacity(count);
        for i in 0..count {
            let model = indices[i] as usize;
            if model >= models.len() {
                return Err(Error::format(format!("cgd instance {i} model {model}")));
            }
            instances.push(StaticInstance {
                model,
                world: worlds[i],
                aabb_min: [boxes[i][0], boxes[i][1], boxes[i][2]],
                aabb_max: [boxes[i][3], boxes[i][4], boxes[i][5]],
                flags: flags[i],
            });
        }
        Ok(Self {
            version,
            models,
            instances,
        })
    }

    /// Model name of an instance.
    pub fn model_of(&self, inst: &StaticInstance) -> &str {
        &self.models[inst.model]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameFs;
    use crate::sarc::SmallArchive;

    fn synthetic() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&CompiledGeometry::MAGIC.to_le_bytes());
        d.extend_from_slice(&7u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        let names = b"a.lod\0";
        d.extend_from_slice(&(names.len() as u32).to_le_bytes());
        d.extend(std::iter::repeat_n(0u8, 40));
        for i in 0..2 {
            let m: [f32; 16] = [
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                10.0 * i as f32,
                5.0,
                -3.0,
                1.0,
            ];
            for v in m {
                d.extend_from_slice(&v.to_le_bytes());
            }
        }
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend(std::iter::repeat_n(0u8, 24));
        d.extend_from_slice(&(-1.0f32).to_le_bytes());
        d.extend_from_slice(&(-1.0f32).to_le_bytes());
        d.extend_from_slice(&3u16.to_le_bytes());
        d.extend_from_slice(&3u16.to_le_bytes());
        for _ in 0..2 {
            for v in [-1.0f32, 4.0, -4.0, 1.0, 6.0, -2.0] {
                d.extend_from_slice(&v.to_le_bytes());
            }
        }
        d.extend_from_slice(names);
        d
    }

    #[test]
    fn parse_synthetic() {
        let g = CompiledGeometry::parse(&synthetic()).unwrap();
        assert_eq!(g.models, vec!["a.lod"]);
        assert_eq!(g.instances.len(), 2);
        assert_eq!(g.instances[1].world.translation(), [10.0, 5.0, -3.0]);
        assert_eq!(g.instances[1].aabb_max, [1.0, 6.0, -2.0]);
        assert_eq!(g.model_of(&g.instances[0]), "a.lod");
        assert!(CompiledGeometry::parse(&synthetic()[4..]).is_err());
    }

    /// `31x14.nl.cgd` in the fixture's `31x14.nlz` (JC2_TEST_DIR): 100 instances of 22
    /// models, the first one the health-locker body next to its `.blo` door.
    #[test]
    fn fixture_31x14() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("31x14.nlz") else {
            return;
        };
        let sarc = SmallArchive::parse(&data).unwrap();
        let e = sarc.get("31x14.nl.cgd").expect("cgd member");
        let g = CompiledGeometry::parse(sarc.data(&data, e).unwrap()).unwrap();
        assert_eq!(g.version, 7);
        assert_eq!(g.instances.len(), 100);
        assert_eq!(g.models.len(), 22);
        let first = &g.instances[0];
        assert_eq!(g.model_of(first), "gp001-a.lod");
        let p = first.world.translation();
        assert!(
            (p[0] + 462.36).abs() < 0.01 && (p[1] - 837.71).abs() < 0.01,
            "{p:?}"
        );
        // every instance's box contains its own origin's footprint, roughly
        for i in &g.instances {
            let p = i.world.translation();
            assert!(
                i.aabb_min[0] <= p[0] + 2.0 && i.aabb_max[0] >= p[0] - 2.0,
                "{p:?}"
            );
            assert_eq!(i.flags, 3);
        }
    }
}
