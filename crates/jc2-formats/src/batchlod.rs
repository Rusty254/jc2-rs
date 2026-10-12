//! `.batchlod`: a model batch of a far (`.flz`) metropolis container (native reader
//! `0xa52980`, JC2-146, `research/formats/cgd-model-batch-next.md` section 4). 193 retail
//! files in 28 far containers, all version 2.
//!
//! ```text
//! u32 version (1 or 2)
//! batch object:  64-byte matrix (row-major 4x4, translation in the last row), u32 length + name
//!                (an `.rbm` path under `ModelBatch\`; only the file name is a member of the container)
//! u32 N entries, each:
//!     entry object:  matrix, name (the part `.rbm`), u32 K
//!     K records, each: matrix, name (a `.lod`),
//!         version 2: u32 flag; if non-zero: f32 scale, f32[3] anchor (same override as the .cgd)
//! u32 M, M names (the parts' models; consumer not identified, JC2-146 Gap 3)
//! 5 bytes the native reader does not consume (JC2-146 Gap 3)
//! ```

use crate::error::{Error, Result};
use crate::placement::Transform3;
use crate::reader::Reader;

/// The scale and anchor a version-2 record may carry (the same override as a `.cgd` instance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordOverride {
    pub scale: f32,
    pub anchor: [f32; 3],
}

/// A placed model: matrix and the model's file name (path stripped).
#[derive(Clone, Debug, PartialEq)]
pub struct BatchObject {
    pub world: Transform3,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchRecord {
    pub object: BatchObject,
    pub over: Option<RecordOverride>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchEntry {
    pub object: BatchObject,
    pub records: Vec<BatchRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchLod {
    pub version: u32,
    /// The merged model of the whole batch.
    pub batch: BatchObject,
    pub entries: Vec<BatchEntry>,
    pub parts: Vec<String>,
    /// Bytes after the part names (5 in every retail file).
    pub tail: usize,
}

/// Axis-aligned box (engine axes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    /// The box of `bbox` (model space) placed by `world`: the eight corners transformed
    /// (inferred reading of native `0xa424b0`, "AABB from the model bounds and matrix").
    pub fn of_model(world: &Transform3, bbox_min: [f32; 3], bbox_max: [f32; 3]) -> Self {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for i in 0..8 {
            let c = [
                if i & 1 == 0 { bbox_min[0] } else { bbox_max[0] },
                if i & 2 == 0 { bbox_min[1] } else { bbox_max[1] },
                if i & 4 == 0 { bbox_min[2] } else { bbox_max[2] },
            ];
            let p = world.transform_point(c);
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        Self { min, max }
    }
    /// Squared distance from a point to the box, 0 inside (native `0x4ebc90`).
    pub fn distance_sq(&self, p: [f32; 3]) -> f32 {
        (0..3)
            .map(|k| {
                let d = (self.min[k] - p[k]).max(p[k] - self.max[k]).max(0.0);
                d * d
            })
            .sum()
    }
}

/// Which of the three native batch tiers draws for a camera (`0xa4c6b0`, `0xa4c5a0`,
/// `0xa4bc90`; the entry-box reading of the 120 m test is inferred, JC2-146 section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchTier {
    /// The batch's merged model: camera at least 350 m from the batch box.
    Merged,
    /// The entry's own model: inside 350 m of the batch, at least 120 m from the entry's box.
    Entry,
    /// The entry's individual records: inside 350 m of the batch and closer than 120 m to
    /// the entry's box.
    Records,
}

pub const MERGED_DISTANCE: f32 = 350.0;
pub const RECORD_DISTANCE: f32 = 120.0;

impl BatchTier {
    /// The tier for squared camera distances to the batch box and to one entry's box.
    pub fn select(batch_d2: f32, entry_d2: f32) -> Self {
        if batch_d2 >= MERGED_DISTANCE * MERGED_DISTANCE {
            Self::Merged
        } else if entry_d2 >= RECORD_DISTANCE * RECORD_DISTANCE {
            Self::Entry
        } else {
            Self::Records
        }
    }
}

fn name(r: &mut Reader) -> Result<String> {
    let n = r.u32()? as usize;
    if n > 4096 {
        return Err(Error::format(format!("batchlod name length {n}")));
    }
    let full = r.str(n)?;
    Ok(full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string())
}

fn object(r: &mut Reader) -> Result<BatchObject> {
    let mut m = [0.0f32; 16];
    for v in m.iter_mut() {
        *v = r.f32()?;
    }
    Ok(BatchObject {
        world: Transform3::from_mat4x3([
            m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10], m[12], m[13], m[14],
        ]),
        name: name(r)?,
    })
}

impl BatchLod {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.u32()?;
        if !(1..=2).contains(&version) {
            return Err(Error::format(format!("batchlod version {version}")));
        }
        let batch = object(&mut r)?;
        let count = r.u32()? as usize;
        if count > 100_000 {
            return Err(Error::format(format!("batchlod entry count {count}")));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let entry = object(&mut r)?;
            let k = r.u32()? as usize;
            if k > 100_000 {
                return Err(Error::format(format!("batchlod record count {k}")));
            }
            let mut records = Vec::with_capacity(k);
            for _ in 0..k {
                let object = object(&mut r)?;
                let over = if version == 2 && r.u32()? != 0 {
                    let scale = r.f32()?;
                    Some(RecordOverride {
                        scale,
                        anchor: [r.f32()?, r.f32()?, r.f32()?],
                    })
                } else {
                    None
                };
                records.push(BatchRecord { object, over });
            }
            entries.push(BatchEntry {
                object: entry,
                records,
            });
        }
        let m = r.u32()? as usize;
        if m > 100_000 {
            return Err(Error::format(format!("batchlod part count {m}")));
        }
        let parts = (0..m).map(|_| name(&mut r)).collect::<Result<Vec<_>>>()?;
        Ok(Self {
            version,
            batch,
            entries,
            parts,
            tail: r.remaining(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameFs;
    use crate::sarc::SmallArchive;

    fn put_name(d: &mut Vec<u8>, n: &str) {
        d.extend_from_slice(&(n.len() as u32).to_le_bytes());
        d.extend_from_slice(n.as_bytes());
    }
    fn put_matrix(d: &mut Vec<u8>, t: [f32; 3]) {
        let m = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, t[0], t[1], t[2], 1.0,
        ];
        for v in m {
            d.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn synthetic() -> Vec<u8> {
        let mut d = 2u32.to_le_bytes().to_vec();
        put_matrix(&mut d, [100.0, 0.0, 0.0]);
        put_name(&mut d, "C:\\depot\\ModelBatch\\b.rbm");
        d.extend_from_slice(&1u32.to_le_bytes());
        put_matrix(&mut d, [1.0, 2.0, 3.0]);
        put_name(&mut d, "p.rbm");
        d.extend_from_slice(&2u32.to_le_bytes());
        put_matrix(&mut d, [4.0, 5.0, 6.0]);
        put_name(&mut d, "a.lod");
        d.extend_from_slice(&0u32.to_le_bytes());
        put_matrix(&mut d, [7.0, 8.0, 9.0]);
        put_name(&mut d, "b.lod");
        d.extend_from_slice(&1u32.to_le_bytes());
        for v in [4.0f32, 1.0, 2.0, 3.0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u32.to_le_bytes());
        put_name(&mut d, "m.rbm");
        d.extend_from_slice(&[0u8; 5]);
        d
    }

    #[test]
    fn parse_synthetic() {
        let b = BatchLod::parse(&synthetic()).unwrap();
        assert_eq!(b.version, 2);
        assert_eq!(b.batch.name, "b.rbm");
        assert_eq!(b.batch.world.translation(), [100.0, 0.0, 0.0]);
        assert_eq!(b.entries.len(), 1);
        let e = &b.entries[0];
        assert_eq!(e.object.name, "p.rbm");
        assert_eq!(e.records.len(), 2);
        assert_eq!(e.records[0].over, None);
        assert_eq!(
            e.records[1].over,
            Some(RecordOverride {
                scale: 4.0,
                anchor: [1.0, 2.0, 3.0]
            })
        );
        assert_eq!(b.parts, vec!["m.rbm"]);
        assert_eq!(b.tail, 5);
        assert!(BatchLod::parse(&synthetic()[..60]).is_err());
    }

    #[test]
    fn box_distance_and_tiers() {
        let w = Transform3::from_mat4x3([1., 0., 0., 0., 1., 0., 0., 0., 1., 10., 0., 0.]);
        let b = Aabb::of_model(&w, [-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]);
        assert_eq!(b.min, [9.0, 0.0, -1.0]);
        assert_eq!(b.distance_sq([10.0, 1.0, 0.0]), 0.0);
        assert_eq!(b.distance_sq([13.0, 1.0, 0.0]), 4.0);
        assert_eq!(BatchTier::select(350.0 * 350.0, 0.0), BatchTier::Merged);
        assert_eq!(
            BatchTier::select(349.0 * 349.0, 120.0 * 120.0),
            BatchTier::Entry
        );
        assert_eq!(BatchTier::select(0.0, 119.0 * 119.0), BatchTier::Records);
    }

    /// Every retail `.batchlod` parses to its five trailing bytes (JC2-146 census: 193
    /// files, 707 entries, 5 074 records), and records lie near their entry's model box:
    /// that confirms the matrix convention the tiers rely on.
    #[test]
    fn retail_census_and_matrix_convention() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let (mut files, mut entries, mut records, mut near, mut with_box) = (0, 0, 0, 0, 0);
        for l in fs.entries() {
            let Some(name) = fs.names.file(l.name_hash) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.contains("metropolis") || !lower.ends_with(".flz") {
                continue;
            }
            let Ok(data) = fs.read_inflated(name) else {
                continue;
            };
            let Ok(sarc) = SmallArchive::parse(&data) else {
                continue;
            };
            for e in &sarc.entries {
                if !e.name.to_ascii_lowercase().ends_with(".batchlod") {
                    continue;
                }
                let bytes = crate::compress::maybe_inflate(sarc.data(&data, e).unwrap()).unwrap();
                let b = BatchLod::parse(&bytes).unwrap();
                assert_eq!(b.tail, 5, "{name}/{}", e.name);
                files += 1;
                entries += b.entries.len();
                for en in &b.entries {
                    records += en.records.len();
                    let Some(m) = sarc.get(&en.object.name) else {
                        continue;
                    };
                    let rbm = crate::rbm::RbmModel::parse(
                        &crate::compress::maybe_inflate(sarc.data(&data, m).unwrap()).unwrap(),
                    )
                    .unwrap();
                    let bx = Aabb::of_model(&en.object.world, rbm.bbox_min, rbm.bbox_max);
                    with_box += 1;
                    // a record's origin lies within 60 m of its entry's box
                    if en
                        .records
                        .iter()
                        .all(|r| bx.distance_sq(r.object.world.translation()) < 3600.0)
                    {
                        near += 1;
                    }
                }
            }
        }
        assert_eq!((files, entries, records), (193, 707, 5074));
        assert!(
            with_box > 0 && near * 10 >= with_box * 9,
            "{near}/{with_box}"
        );
    }
}
