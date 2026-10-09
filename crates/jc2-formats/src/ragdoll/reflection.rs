//! Checked cross-section access to the reflection retained in original BRDs.
use crate::{
    error::{Error, Result},
    havok::MAGIC,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct Member {
    pub offset: usize,
    pub kind: u8,
    pub subtype: u8,
    pub count: u16,
    pub class: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Class {
    pub size: usize,
    pub parent: Option<String>,
    pub members: BTreeMap<String, Member>,
}
pub struct Reader<'a> {
    pub bytes: &'a [u8],
    pub objects: BTreeMap<usize, String>,
    pub classes: BTreeMap<String, Class>,
    pub data_start: usize,
    pointers: HashMap<usize, usize>,
    regions: Vec<(usize, usize)>,
}
impl<'a> Reader<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let start = bytes
            .windows(8)
            .take(1024)
            .position(|x| x == MAGIC)
            .ok_or_else(|| Error::format("ragdoll: no Havok magic"))?;
        let mut r = Self {
            bytes: &bytes[start..],
            objects: BTreeMap::new(),
            classes: BTreeMap::new(),
            data_start: 0,
            pointers: HashMap::new(),
            regions: Vec::new(),
        };
        r.read(0, 64)?;
        if r.bytes[16..20] != [4, 1, 0, 1] || r.u32(12)? != 5 {
            return Err(Error::format(
                "ragdoll: expected Havok 5 32-bit little endian",
            ));
        }
        let n = r.u32(20)? as usize;
        if !(1..=16).contains(&n) {
            return Err(Error::format("ragdoll: section count"));
        }
        let mut sections = Vec::new();
        for i in 0..n {
            let mut f = [0usize; 7];
            for (j, v) in f.iter_mut().enumerate() {
                *v = r.u32(84 + i * 48 + j * 4)? as usize;
            }
            if !f[1..].windows(2).all(|w| w[0] <= w[1]) {
                return Err(Error::format("ragdoll: reversed section tables"));
            }
            r.read(f[0], f[6])?;
            r.regions.push((f[0], f[0] + f[1]));
            sections.push(f);
        }
        r.data_start = sections
            .get(r.u32(24)? as usize)
            .ok_or_else(|| Error::format("ragdoll: contents section"))?[0];
        for f in &sections {
            for o in (f[0] + f[1]..f[0] + f[2]).step_by(8) {
                r.read(o, 8)?;
                let a = r.u32(o)? as usize;
                if a == u32::MAX as usize {
                    continue;
                }
                let b = r.u32(o + 4)? as usize;
                if a + 4 > f[1] || b >= f[1] {
                    return Err(Error::format("ragdoll: local fixup range"));
                }
                r.pointers.insert(f[0] + a, f[0] + b);
            }
            for o in (f[0] + f[2]..f[0] + f[3]).step_by(12) {
                r.read(o, 12)?;
                let a = r.u32(o)? as usize;
                if a == u32::MAX as usize {
                    continue;
                }
                let t = r.u32(o + 4)? as usize;
                let b = r.u32(o + 8)? as usize;
                let target = sections
                    .get(t)
                    .ok_or_else(|| Error::format("ragdoll: global fixup section"))?;
                if a + 4 > f[1] || b >= target[1] {
                    return Err(Error::format("ragdoll: global fixup range"));
                }
                r.pointers.insert(f[0] + a, target[0] + b);
            }
            for o in (f[0] + f[3]..f[0] + f[4]).step_by(12) {
                r.read(o, 12)?;
                let a = r.u32(o)? as usize;
                if a == u32::MAX as usize {
                    continue;
                }
                let t = r.u32(o + 4)? as usize;
                let b = r.u32(o + 8)? as usize;
                let target = sections
                    .get(t)
                    .ok_or_else(|| Error::format("ragdoll: virtual fixup section"))?;
                if a >= f[1] || b >= target[1] {
                    return Err(Error::format("ragdoll: virtual fixup range"));
                }
                r.objects.insert(f[0] + a, r.string(target[0] + b)?);
            }
        }
        // hkClass/hkClassMember ABI validated against their own original reflection.
        let class_offsets: Vec<_> = r
            .objects
            .iter()
            .filter(|(_, c)| c.as_str() == "hkClass")
            .map(|(&o, _)| o)
            .collect();
        for o in class_offsets {
            let name = r.string(r.required_pointer(o)?)?;
            let parent = r
                .pointer(o + 4)
                .map(|p| r.required_pointer(p).and_then(|p| r.string(p)))
                .transpose()?;
            let size = r.u32(o + 8)? as usize;
            let count = r.u32(o + 28)? as usize;
            if count > 256 || size > 65536 {
                return Err(Error::format("ragdoll: reflection size"));
            }
            let mut members = BTreeMap::new();
            if count > 0 {
                let p = r.required_pointer(o + 24)?;
                r.payload(p, count * 24)?;
                for i in 0..count {
                    let m = p + i * 24;
                    let member_name = r.string(r.required_pointer(m)?)?;
                    let offset = r.u16(m + 18)? as usize;
                    if offset >= size {
                        return Err(Error::format("ragdoll: reflected member outside class"));
                    }
                    let class = r
                        .pointer(m + 4)
                        .map(|p| r.required_pointer(p).and_then(|p| r.string(p)))
                        .transpose()?;
                    members.insert(
                        member_name,
                        Member {
                            offset,
                            kind: r.read(m + 12, 1)?[0],
                            subtype: r.read(m + 13, 1)?[0],
                            count: r.u16(m + 14)?,
                            class,
                        },
                    );
                }
            }
            r.classes.insert(
                name,
                Class {
                    size,
                    parent,
                    members,
                },
            );
        }
        if r.classes.is_empty() {
            return Err(Error::format("ragdoll: reflection required"));
        }
        Ok(r)
    }
    pub fn read(&self, o: usize, n: usize) -> Result<&'a [u8]> {
        self.bytes
            .get(
                o..o.checked_add(n)
                    .ok_or_else(|| Error::format("ragdoll: offset overflow"))?,
            )
            .ok_or(Error::Truncated {
                offset: o,
                needed: n,
                len: self.bytes.len(),
            })
    }
    pub fn payload(&self, o: usize, n: usize) -> Result<&'a [u8]> {
        let end = o
            .checked_add(n)
            .ok_or_else(|| Error::format("ragdoll: payload overflow"))?;
        if !self.regions.iter().any(|&(a, b)| o >= a && end <= b) {
            return Err(Error::format("ragdoll: access outside section payload"));
        }
        self.read(o, n)
    }
    pub fn u32(&self, o: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read(o, 4)?.try_into().unwrap()))
    }
    pub fn u16(&self, o: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read(o, 2)?.try_into().unwrap()))
    }
    pub fn float(&self, o: usize) -> Result<f32> {
        let f = f32::from_bits(self.u32(o)?);
        if !f.is_finite() {
            return Err(Error::format("ragdoll: nonfinite float"));
        }
        Ok(f)
    }
    pub fn floats<const N: usize>(&self, o: usize) -> Result<[f32; N]> {
        self.payload(o, N * 4)?;
        let mut v = [0.; N];
        for (i, x) in v.iter_mut().enumerate() {
            *x = self.float(o + i * 4)?;
        }
        Ok(v)
    }
    pub fn pointer(&self, o: usize) -> Option<usize> {
        self.pointers.get(&o).copied()
    }
    pub fn required_pointer(&self, o: usize) -> Result<usize> {
        self.pointer(o)
            .ok_or_else(|| Error::format(format!("ragdoll: missing fixup @{o:x}")))
    }
    pub fn string(&self, o: usize) -> Result<String> {
        let end = self
            .regions
            .iter()
            .find(|&&(a, b)| o >= a && o < b)
            .map(|&(_, b)| b)
            .ok_or_else(|| Error::format("ragdoll: string range"))?;
        let s = self.read(o, end - o)?;
        let n = s
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::format("ragdoll: unterminated string"))?;
        String::from_utf8(s[..n].to_vec()).map_err(|_| Error::format("ragdoll: invalid string"))
    }
    pub fn member(&self, class: &str, name: &str) -> Result<&Member> {
        let mut c = class;
        for _ in 0..64 {
            let x = self
                .classes
                .get(c)
                .ok_or_else(|| Error::format(format!("ragdoll: missing class {c}")))?;
            if let Some(m) = x.members.get(name) {
                return Ok(m);
            }
            match x.parent.as_deref() {
                Some(p) => c = p,
                None => break,
            }
        }
        Err(Error::format(format!(
            "ragdoll: missing member {class}.{name}"
        )))
    }
    pub fn field(&self, o: usize, class: &str, name: &str) -> Result<usize> {
        Ok(o + self.member(class, name)?.offset)
    }
    pub fn nested(&self, o: usize, class: &str, name: &str) -> Result<(usize, String)> {
        let m = self.member(class, name)?;
        if m.kind != 25 {
            return Err(Error::format("ragdoll: expected embedded struct"));
        }
        Ok((
            o + m.offset,
            m.class
                .clone()
                .ok_or_else(|| Error::format("ragdoll: struct class missing"))?,
        ))
    }
    pub fn array(
        &self,
        o: usize,
        class: &str,
        name: &str,
        stride: usize,
    ) -> Result<(usize, usize)> {
        let m = self.member(class, name)?;
        if !matches!(m.kind, 22 | 26) {
            return Err(Error::format("ragdoll: expected array"));
        }
        let f = o + m.offset;
        let n = self.u32(f + 4)? as usize;
        if n > 65536 {
            return Err(Error::format("ragdoll: array count"));
        }
        if n == 0 {
            return Ok((0, 0));
        }
        let p = self.required_pointer(f)?;
        self.payload(
            p,
            n.checked_mul(stride)
                .ok_or_else(|| Error::format("ragdoll: array overflow"))?,
        )?;
        Ok((p, n))
    }
    pub fn object(&self, o: usize, expected: &str) -> Result<()> {
        if self.objects.get(&o).map(String::as_str) != Some(expected) {
            return Err(Error::format(format!(
                "ragdoll: expected {expected} @{o:x}"
            )));
        }
        self.payload(
            o,
            self.classes
                .get(expected)
                .ok_or_else(|| Error::format("ragdoll: object reflection missing"))?
                .size,
        )?;
        Ok(())
    }
}
