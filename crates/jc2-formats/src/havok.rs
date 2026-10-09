//! Havok 5.5 binary packfiles (`0x57E0E057 0x10C0C010`): the container for the
//! collision shapes (`.pfx`, tile `.pfs`), animations and behaviours.
//!
//! The game ships them with the `__types__` section empty, so the class layouts are not
//! self-describing; this module only reads the container — header, sections, class
//! names, fixups — and gives typed access to the raw objects. `collision.rs` knows the
//! shape layouts (worked out from the data, 32-bit little-endian, see
//! `research/formats/havok.md`).
//!
//! Layout: 64-byte header (magic ×2, user tag, file version 5, layout rules
//! `04 01 00 01` = 4-byte pointers, little-endian, reuse padding, empty-base-class
//! optimisation, section count 3, contents section 2, class-name section 0, version
//! string `Havok-5.5.0-r1`), then 48-byte section headers (19-char tag, absolute data
//! start, then offsets *within the section* of the local, global and virtual fixup
//! tables, exports, imports, end). Fixups: local `(from, to)` — the pointer at `from`
//! points at `to` in the same section; global `(from, section, to)`; virtual
//! `(from, section, class-name offset)` — the object at `from` is an instance of that
//! class. Class names: `u32 signature, u8 0x09, NUL-terminated name`, padded with 0xFF.
//! Arrays (`hkArray`) are `{ptr, size, capacity | flags}` (12 bytes); vectors are 16-byte
//! aligned `hkVector4`.

use crate::error::{Error, Result};
use std::collections::HashMap;

pub const MAGIC: [u8; 8] = [0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10];

#[derive(Clone, Debug)]
pub struct Section {
    pub tag: String,
    pub data_start: u32,
    pub local_fixups: u32,
    pub global_fixups: u32,
    pub virtual_fixups: u32,
    pub exports: u32,
    pub imports: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HkObject {
    /// Offset in the data section.
    pub offset: u32,
    pub class: String,
    /// Bytes to the next object (or the end of the data).
    pub size: u32,
}

#[derive(Clone, Debug)]
pub struct Packfile<'a> {
    /// The Havok bytes (after any `PFX\0`/`PFS\0` wrapper).
    pub bytes: &'a [u8],
    pub version: String,
    pub sections: Vec<Section>,
    /// Class-name offset → (name, signature).
    pub classnames: HashMap<u32, (String, u32)>,
    data: usize,
    data_len: usize,
    contents_section: u32,
    local: HashMap<u32, u32>,
    global: HashMap<u32, (u32, u32)>,
    pub objects: Vec<HkObject>,
}

impl<'a> Packfile<'a> {
    /// True when the bytes hold a packfile, possibly behind a short wrapper.
    pub fn is_packfile(bytes: &[u8]) -> bool {
        find_magic(bytes).is_some()
    }

    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let start = find_magic(bytes).ok_or_else(|| Error::format("not a Havok packfile"))?;
        let b = &bytes[start..];
        let u32_at = |o: usize| -> Result<u32> {
            b.get(o..o + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
                .ok_or(Error::Truncated {
                    offset: o,
                    needed: 4,
                    len: b.len(),
                })
        };
        if b.len() < 64 {
            return Err(Error::format("packfile header truncated"));
        }
        let file_version = u32_at(12)?;
        let layout = &b[16..20];
        if layout[0] != 4 || layout[1] != 1 {
            return Err(Error::format(format!(
                "unsupported packfile layout {layout:?} (want 32-bit little-endian)"
            )));
        }
        let num_sections = u32_at(20)? as usize;
        let contents_section = u32_at(24)? as usize;
        let classname_section = u32_at(32)? as usize;
        let version =
            String::from_utf8_lossy(b[40..56].split(|&c| c == 0).next().unwrap_or(&[])).to_string();
        if file_version != 5 && !version.starts_with("Havok-5") {
            return Err(Error::format(format!(
                "packfile version {file_version} ({version}) not supported"
            )));
        }
        let mut sections = Vec::with_capacity(num_sections);
        let mut off = 64;
        for _ in 0..num_sections {
            if b.len() < off + 48 {
                return Err(Error::format("packfile section header truncated"));
            }
            let tag = String::from_utf8_lossy(&b[off..off + 19])
                .trim_end_matches('\0')
                .to_string();
            let v: Vec<u32> = (0..7)
                .map(|i| u32_at(off + 20 + i * 4))
                .collect::<Result<_>>()?;
            sections.push(Section {
                tag,
                data_start: v[0],
                local_fixups: v[1],
                global_fixups: v[2],
                virtual_fixups: v[3],
                exports: v[4],
                imports: v[5],
                end: v[6],
            });
            off += 48;
        }
        let cn = sections
            .get(classname_section)
            .ok_or_else(|| Error::format("bad class-name section index"))?;
        let mut classnames = HashMap::new();
        {
            let start = cn.data_start as usize;
            let end = start + cn.local_fixups as usize;
            let mut p = start;
            while p + 5 < end.min(b.len()) {
                let sig = u32_at(p)?;
                if b[p + 4] != 0x09 {
                    break;
                }
                let name_start = p + 5;
                let name_end = b[name_start..end]
                    .iter()
                    .position(|&c| c == 0)
                    .map(|i| name_start + i)
                    .unwrap_or(end);
                let name = String::from_utf8_lossy(&b[name_start..name_end]).to_string();
                classnames.insert((name_start - start) as u32, (name, sig));
                p = name_end + 1;
                while p < end && b[p] == 0xFF {
                    p += 1;
                }
            }
        }
        let ds = sections
            .get(contents_section)
            .ok_or_else(|| Error::format("bad contents section index"))?
            .clone();
        let data = ds.data_start as usize;
        let data_len = ds.local_fixups as usize;
        if b.len() < data + ds.end as usize {
            return Err(Error::format("packfile data section truncated"));
        }
        let table = |from: u32, to: u32, stride: usize| -> Vec<Vec<u32>> {
            let mut out = Vec::new();
            let (mut p, end) = (data + from as usize, data + to as usize);
            while p + stride <= end {
                let vals: Vec<u32> = (0..stride / 4)
                    .map(|i| u32_at(p + i * 4).unwrap_or(u32::MAX))
                    .collect();
                if vals[0] != u32::MAX {
                    out.push(vals);
                }
                p += stride;
            }
            out
        };
        let local: HashMap<u32, u32> = table(ds.local_fixups, ds.global_fixups, 8)
            .into_iter()
            .map(|v| (v[0], v[1]))
            .collect();
        let global: HashMap<u32, (u32, u32)> = table(ds.global_fixups, ds.virtual_fixups, 12)
            .into_iter()
            .map(|v| (v[0], (v[1], v[2])))
            .collect();
        let mut objects: Vec<HkObject> = table(ds.virtual_fixups, ds.exports, 12)
            .into_iter()
            .map(|v| HkObject {
                offset: v[0],
                class: classnames
                    .get(&v[2])
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| format!("?{:x}", v[2])),
                size: 0,
            })
            .collect();
        objects.sort_by_key(|o| o.offset);
        for i in 0..objects.len() {
            let next = objects
                .get(i + 1)
                .map(|o| o.offset)
                .unwrap_or(data_len as u32);
            objects[i].size = next.saturating_sub(objects[i].offset);
        }
        Ok(Self {
            bytes: b,
            version,
            sections,
            classnames,
            data,
            data_len,
            contents_section: contents_section as u32,
            local,
            global,
            objects,
        })
    }

    /// The data section's bytes.
    pub fn data(&self) -> &'a [u8] {
        &self.bytes[self.data..self.data + self.data_len]
    }

    pub fn u8(&self, off: u32) -> u8 {
        self.data().get(off as usize).copied().unwrap_or(0)
    }

    pub fn u16(&self, off: u32) -> u16 {
        let d = self.data();
        let o = off as usize;
        match d.get(o..o + 2) {
            Some(s) => u16::from_le_bytes([s[0], s[1]]),
            None => 0,
        }
    }

    pub fn u32(&self, off: u32) -> u32 {
        let d = self.data();
        let o = off as usize;
        match d.get(o..o + 4) {
            Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            None => 0,
        }
    }

    pub fn f32(&self, off: u32) -> f32 {
        f32::from_bits(self.u32(off))
    }

    pub fn vec4(&self, off: u32) -> [f32; 4] {
        [
            self.f32(off),
            self.f32(off + 4),
            self.f32(off + 8),
            self.f32(off + 12),
        ]
    }

    pub fn vec3(&self, off: u32) -> [f32; 3] {
        [self.f32(off), self.f32(off + 4), self.f32(off + 8)]
    }

    /// Follow the pointer stored at `off` (a local or global fixup into the data
    /// section). `None` for a null pointer or a pointer into another section.
    pub fn pointer(&self, off: u32) -> Option<u32> {
        if let Some(&to) = self.local.get(&off) {
            return Some(to);
        }
        if let Some(&(section, to)) = self.global.get(&off)
            && section == self.contents_section
        {
            return Some(to);
        }
        None
    }

    /// An `hkArray` at `off`: `(data offset, element count)`, or `None` when empty.
    pub fn array(&self, off: u32) -> Option<(u32, u32)> {
        let size = self.u32(off + 4);
        let ptr = self.pointer(off)?;
        (size > 0).then_some((ptr, size))
    }

    /// Class of the object starting exactly at `off`.
    pub fn class_at(&self, off: u32) -> Option<&str> {
        self.objects
            .iter()
            .find(|o| o.offset == off)
            .map(|o| o.class.as_str())
    }

    /// Objects of a class, in file order.
    pub fn objects_of(&self, class: &str) -> impl Iterator<Item = &HkObject> {
        self.objects.iter().filter(move |o| o.class == class)
    }

    /// The root `hkRootLevelContainer`'s named variants: `(name, class, object offset)`.
    pub fn root_variants(&self) -> Vec<(String, String, u32)> {
        let mut out = Vec::new();
        let Some(root) = self.objects_of("hkRootLevelContainer").next() else {
            return out;
        };
        let Some((items, n)) = self.array(root.offset) else {
            return out;
        };
        for i in 0..n {
            let e = items + i * 12;
            let name = self.pointer(e).map(|p| self.cstr(p)).unwrap_or_default();
            let class = self
                .pointer(e + 4)
                .map(|p| self.cstr(p))
                .unwrap_or_default();
            if let Some(obj) = self.pointer(e + 8) {
                out.push((name, class, obj));
            }
        }
        out
    }

    /// NUL-terminated string at a data offset.
    pub fn cstr(&self, off: u32) -> String {
        let d = self.data();
        let o = off as usize;
        let end = d[o.min(d.len())..]
            .iter()
            .position(|&c| c == 0)
            .map(|i| o + i)
            .unwrap_or(d.len());
        String::from_utf8_lossy(&d[o.min(d.len())..end]).to_string()
    }
}

fn find_magic(bytes: &[u8]) -> Option<usize> {
    // the wrappers are short; the magic sits within the first few hundred bytes
    bytes.windows(8).take(1024).position(|w| w == MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `31x14.nlz` reachable: every `.pfx` in it parses and has
    /// a rigid body.
    #[test]
    fn parses_tile_collision_files() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("31x14.nlz") else {
            return;
        };
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let mut n = 0;
        for e in &sarc.entries {
            if !e.name.to_ascii_lowercase().ends_with(".pfx") {
                continue;
            }
            let bytes = sarc.data(&data, e).unwrap();
            let pf = Packfile::parse(bytes).unwrap_or_else(|err| panic!("{}: {err}", e.name));
            assert_eq!(pf.version, "Havok-5.5.0-r1");
            assert_eq!(pf.objects_of("hkpRigidBody").count(), 1, "{}", e.name);
            let roots = pf.root_variants();
            assert_eq!(roots.len(), 1, "{}: {roots:?}", e.name);
            assert_eq!(roots[0].1, "hkpPhysicsData");
            n += 1;
        }
        assert!(n > 50, "{n} collision files");
    }
}
