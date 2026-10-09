//! SARC "small archives": the `.ee / .bl / .nl / .fl` containers (and their
//! zlib-compressed `.eez / .blz / .nlz / .flz` twins once inflated).
//!
//! ```text
//! u32 4                    header size; reads as 0x04000000 on big-endian data
//! "SARC"
//! u32 version              1 or 2
//! u32 index_size           bytes of index that follow
//! index: repeat while >= 16 bytes of index remain:
//!   u32 name_len
//!   u8  name[name_len]     NUL-padded ASCII, e.g. "02x45.pfs"
//!   u32 offset             from start of the SARC
//!   u32 size
//! file data...
//! ```

use crate::error::{Error, Result};
use crate::reader::{Endian, Reader};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SarcEntry {
    pub name: String,
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Debug)]
pub struct SmallArchive {
    pub version: u32,
    pub endian: Endian,
    pub entries: Vec<SarcEntry>,
}

pub const MAGIC: &[u8; 4] = b"SARC";

/// Cheap check: does this blob start with a SARC header?
pub fn is_sarc(data: &[u8]) -> bool {
    data.len() >= 8
        && &data[4..8] == MAGIC
        && (data[0..4] == [4, 0, 0, 0] || data[0..4] == [0, 0, 0, 4])
}

impl SmallArchive {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let hs = r.u32()?;
        let endian = if hs == 4 {
            Endian::Little
        } else if hs.swap_bytes() == 4 {
            Endian::Big
        } else {
            return Err(Error::format("not a SARC (bad header size)"));
        };
        if r.bytes(4)? != MAGIC {
            return Err(Error::format("not a SARC (bad magic)"));
        }
        r.endian = endian;
        let version = r.u32()?;
        if version != 1 && version != 2 {
            return Err(Error::format(format!("unsupported SARC version {version}")));
        }
        let index_size = r.u32()? as usize;
        let index_start = r.pos();
        let index = Reader::with_endian(r.bytes(index_size)?, endian);
        let mut ir = index;
        let mut entries = Vec::new();
        while ir.remaining() >= 16 {
            let name_len = ir.u32()? as usize;
            if name_len > 1024 {
                return Err(Error::format(format!(
                    "SARC entry name too long ({name_len}) at index offset {}",
                    index_start + ir.pos()
                )));
            }
            let name = ir.str(name_len)?;
            let offset = ir.u32()?;
            let size = ir.u32()?;
            entries.push(SarcEntry { name, offset, size });
        }
        Ok(Self {
            version,
            endian,
            entries,
        })
    }

    pub fn get(&self, name: &str) -> Option<&SarcEntry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Slice the entry's bytes out of the archive blob it was parsed from.
    pub fn data<'a>(&self, blob: &'a [u8], entry: &SarcEntry) -> Result<&'a [u8]> {
        let (o, s) = (entry.offset as usize, entry.size as usize);
        blob.get(o..o + s).ok_or(Error::Truncated {
            offset: o,
            needed: s,
            len: blob.len(),
        })
    }

    /// Build a SARC blob from (name, bytes) pairs. Names are not padded; the
    /// index is padded to a 16-byte multiple and each file starts 4-aligned,
    /// matching what the original tools produce.
    pub fn build(entries: &[(&str, &[u8])], version: u32) -> Vec<u8> {
        let raw_index_len: usize = entries.iter().map(|(name, _)| 4 + name.len() + 8).sum();
        let index_len = raw_index_len.div_ceil(16) * 16;
        let mut offsets = Vec::with_capacity(entries.len());
        let mut data_off = 16 + index_len;
        for (_, bytes) in entries {
            data_off = data_off.div_ceil(4) * 4;
            offsets.push(data_off as u32);
            data_off += bytes.len();
        }
        let mut index = Vec::with_capacity(index_len);
        for ((name, bytes), off) in entries.iter().zip(&offsets) {
            index.extend_from_slice(&(name.len() as u32).to_le_bytes());
            index.extend_from_slice(name.as_bytes());
            index.extend_from_slice(&off.to_le_bytes());
            index.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        }
        index.resize(index_len, 0);
        let mut out = Vec::with_capacity(data_off);
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(index_len as u32).to_le_bytes());
        out.extend_from_slice(&index);
        for ((_, bytes), off) in entries.iter().zip(&offsets) {
            out.resize(*off as usize, 0);
            out.extend_from_slice(bytes);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_parse() {
        let blob = SmallArchive::build(&[("a.bin", b"hello"), ("longer_name.pfs", b"world!!")], 2);
        assert!(is_sarc(&blob));
        let s = SmallArchive::parse(&blob).unwrap();
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.data(&blob, &s.entries[0]).unwrap(), b"hello");
        assert_eq!(
            s.data(&blob, s.get("LONGER_NAME.PFS").unwrap()).unwrap(),
            b"world!!"
        );
    }
}
