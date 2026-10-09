//! `.tab` archive tables: the index of a `.arc` blob.
//!
//! Layout (PC, little-endian; console versions are big-endian and detected by
//! the alignment field reading as `0x0800` byte-swapped):
//!
//! ```text
//! u32 alignment            always 0x800
//! repeat until EOF:
//!   u32 name_hash          hashlittle(file name)   (see crate::hash)
//!   u32 offset             byte offset into the .arc, 0x800-aligned
//!   u32 size               byte size
//! ```

use crate::error::{Error, Result};
use crate::reader::{Endian, Reader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabEntry {
    pub name_hash: u32,
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Debug)]
pub struct ArchiveTable {
    pub alignment: u32,
    pub endian: Endian,
    pub entries: Vec<TabEntry>,
}

impl ArchiveTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let raw = r.u32()?;
        let endian = if raw == 0x800 {
            Endian::Little
        } else if raw.swap_bytes() == 0x800 {
            Endian::Big
        } else {
            return Err(Error::format(format!(
                "not a .tab file (alignment field {raw:#x})"
            )));
        };
        r.endian = endian;
        let mut entries = Vec::with_capacity(r.remaining() / 12);
        while r.remaining() >= 12 {
            entries.push(TabEntry {
                name_hash: r.u32()?,
                offset: r.u32()?,
                size: r.u32()?,
            });
        }
        Ok(Self {
            alignment: 0x800,
            endian,
            entries,
        })
    }

    pub fn get(&self, name_hash: u32) -> Option<&TabEntry> {
        self.entries.iter().find(|e| e.name_hash == name_hash)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialise back to the on-disk layout (for repacking).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.entries.len() * 12);
        let w = |out: &mut Vec<u8>, v: u32| match self.endian {
            Endian::Little => out.extend_from_slice(&v.to_le_bytes()),
            Endian::Big => out.extend_from_slice(&v.to_be_bytes()),
        };
        w(&mut out, self.alignment);
        for e in &self.entries {
            w(&mut out, e.name_hash);
            w(&mut out, e.offset);
            w(&mut out, e.size);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = ArchiveTable {
            alignment: 0x800,
            endian: Endian::Little,
            entries: vec![
                TabEntry {
                    name_hash: 1,
                    offset: 0x800,
                    size: 10,
                },
                TabEntry {
                    name_hash: 2,
                    offset: 0x1000,
                    size: 20,
                },
            ],
        };
        let p = ArchiveTable::parse(&t.to_bytes()).unwrap();
        assert_eq!(p.entries, t.entries);
    }

    #[test]
    fn rejects_garbage() {
        assert!(ArchiveTable::parse(&[1, 2, 3, 4]).is_err());
    }
}
