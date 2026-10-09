//! Binary property containers — the engine's generic "object with named
//! values" format behind `.bin`, `.asb`, `.epe`, `.pfs`, `.blo`, … files.
//!
//! A file is a sequence of nodes. Each node:
//!
//! ```text
//! u8  section_count
//! repeat section_count times:
//!   u16 section_type        1 node-by-name, 2 variant-by-name, 3 tag,
//!                           4 node-by-hash, 5 variant-by-hash
//!   u16 element_count       (for tag: the tag string length)
//!   elements...
//!     by-name:  u32 name_len, name, then the child node / variant
//!     by-hash:  u32 name_hash, then the child node / variant
//! ```
//!
//! Variants: `u8 type` then payload — 1 i32, 2 f32, 3 string (u16 len),
//! 4 vec2, 5 vec3, 6 vec4, 7 mat3x3, 8 mat4x3, 9 i32 array (i32 count),
//! 10 f32 array (i32 count).
//!
//! Shipped files use the by-hash sections almost exclusively; the names are
//! recovered with [`crate::names::NameTable`].

use crate::error::{Error, Result};
use crate::hash::hash_str;
use crate::reader::{Endian, Reader};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Key {
    Name(String),
    Hash(u32),
}

impl Key {
    pub fn hash(&self) -> u32 {
        match self {
            Key::Name(n) => hash_str(n),
            Key::Hash(h) => *h,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Variant {
    Int(i32),
    Float(f32),
    Str(String),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Mat3x3([f32; 9]),
    Mat4x3([f32; 12]),
    /// Only produced by the blackboard (PCBB) layout.
    Mat4x4([f32; 16]),
    Ints(Vec<i32>),
    Floats(Vec<f32>),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Node {
    pub tag: Option<String>,
    pub children: Vec<(Key, Node)>,
    pub values: Vec<(Key, Variant)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PropertyFile {
    pub nodes: Vec<Node>,
}

/// Heuristic from the original tools: a property file starts with a section
/// count of 1 or 2 followed by a section type of 1, 4 or 5 (little-endian).
pub fn looks_like_property(data: &[u8]) -> bool {
    data.len() >= 3
        && (data[0] == 1 || data[0] == 2)
        && matches!(data[1], 1 | 4 | 5)
        && data[2] == 0
}

impl PropertyFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::parse_endian(data, Endian::Little)
    }

    pub fn parse_endian(data: &[u8], endian: Endian) -> Result<Self> {
        let mut r = Reader::with_endian(data, endian);
        let mut nodes = Vec::new();
        while !r.at_end() {
            nodes.push(parse_node(&mut r, 0)?);
        }
        Ok(Self { nodes })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for n in &self.nodes {
            write_node(&mut out, n);
        }
        out
    }
}

fn parse_node(r: &mut Reader, depth: usize) -> Result<Node> {
    if depth > 64 {
        return Err(Error::format("property nesting too deep"));
    }
    let mut node = Node::default();
    let section_count = r.u8()?;
    let mut seen = [false; 6];
    for _ in 0..section_count {
        let ty = r.u16()?;
        let count = r.u16()? as usize;
        if ty == 0 || ty > 5 || seen[ty as usize] {
            return Err(Error::format(format!(
                "bad property section type {ty} at {}",
                r.pos()
            )));
        }
        seen[ty as usize] = true;
        match ty {
            1 => {
                for _ in 0..count {
                    let name = read_name(r)?;
                    node.children
                        .push((Key::Name(name), parse_node(r, depth + 1)?));
                }
            }
            2 => {
                for _ in 0..count {
                    let name = read_name(r)?;
                    node.values.push((Key::Name(name), parse_variant(r)?));
                }
            }
            3 => node.tag = Some(r.str(count)?),
            4 => {
                for _ in 0..count {
                    let h = r.u32()?;
                    node.children
                        .push((Key::Hash(h), parse_node(r, depth + 1)?));
                }
            }
            5 => {
                for _ in 0..count {
                    let h = r.u32()?;
                    node.values.push((Key::Hash(h), parse_variant(r)?));
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(node)
}

fn read_name(r: &mut Reader) -> Result<String> {
    let len = r.u32()? as usize;
    if len >= 0x7fff {
        return Err(Error::format("property name too long"));
    }
    r.str(len)
}

fn parse_variant(r: &mut Reader) -> Result<Variant> {
    let ty = r.u8()?;
    Ok(match ty {
        1 => Variant::Int(r.i32()?),
        2 => Variant::Float(r.f32()?),
        3 => {
            let len = r.u16()? as usize;
            Variant::Str(r.str(len)?)
        }
        4 => Variant::Vec2([r.f32()?, r.f32()?]),
        5 => Variant::Vec3([r.f32()?, r.f32()?, r.f32()?]),
        6 => Variant::Vec4([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        7 => {
            let mut m = [0f32; 9];
            for v in &mut m {
                *v = r.f32()?;
            }
            Variant::Mat3x3(m)
        }
        8 => {
            let mut m = [0f32; 12];
            for v in &mut m {
                *v = r.f32()?;
            }
            Variant::Mat4x3(m)
        }
        9 => {
            let n = r.i32()?.max(0) as usize;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(r.i32()?);
            }
            Variant::Ints(v)
        }
        10 => {
            let n = r.i32()?.max(0) as usize;
            let mut v = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                v.push(r.f32()?);
            }
            Variant::Floats(v)
        }
        _ => {
            return Err(Error::format(format!(
                "unknown variant type {ty} at {}",
                r.pos() - 1
            )));
        }
    })
}

fn write_node(out: &mut Vec<u8>, node: &Node) {
    let by_name_nodes: Vec<_> = node
        .children
        .iter()
        .filter(|(k, _)| matches!(k, Key::Name(_)))
        .collect();
    let by_hash_nodes: Vec<_> = node
        .children
        .iter()
        .filter(|(k, _)| matches!(k, Key::Hash(_)))
        .collect();
    let by_name_vals: Vec<_> = node
        .values
        .iter()
        .filter(|(k, _)| matches!(k, Key::Name(_)))
        .collect();
    let by_hash_vals: Vec<_> = node
        .values
        .iter()
        .filter(|(k, _)| matches!(k, Key::Hash(_)))
        .collect();
    let count = [
        !by_name_nodes.is_empty(),
        !by_name_vals.is_empty(),
        node.tag.is_some(),
        !by_hash_nodes.is_empty(),
        !by_hash_vals.is_empty(),
    ]
    .iter()
    .filter(|&&b| b)
    .count() as u8;
    out.push(count);
    let name_of = |k: &Key| match k {
        Key::Name(n) => n.clone(),
        Key::Hash(_) => unreachable!(),
    };
    if !by_name_nodes.is_empty() {
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(by_name_nodes.len() as u16).to_le_bytes());
        for (k, n) in by_name_nodes {
            let name = name_of(k);
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            write_node(out, n);
        }
    }
    if !by_name_vals.is_empty() {
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&(by_name_vals.len() as u16).to_le_bytes());
        for (k, v) in by_name_vals {
            let name = name_of(k);
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            write_variant(out, v);
        }
    }
    if let Some(tag) = &node.tag {
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        out.extend_from_slice(tag.as_bytes());
    }
    if !by_hash_nodes.is_empty() {
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&(by_hash_nodes.len() as u16).to_le_bytes());
        for (k, n) in by_hash_nodes {
            out.extend_from_slice(&k.hash().to_le_bytes());
            write_node(out, n);
        }
    }
    if !by_hash_vals.is_empty() {
        out.extend_from_slice(&5u16.to_le_bytes());
        out.extend_from_slice(&(by_hash_vals.len() as u16).to_le_bytes());
        for (k, v) in by_hash_vals {
            out.extend_from_slice(&k.hash().to_le_bytes());
            write_variant(out, v);
        }
    }
}

fn write_variant(out: &mut Vec<u8>, v: &Variant) {
    let f = |out: &mut Vec<u8>, x: &f32| out.extend_from_slice(&x.to_le_bytes());
    match v {
        Variant::Int(i) => {
            out.push(1);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Variant::Float(x) => {
            out.push(2);
            f(out, x);
        }
        Variant::Str(s) => {
            out.push(3);
            out.extend_from_slice(&(s.len() as u16).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Variant::Vec2(a) => {
            out.push(4);
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Vec3(a) => {
            out.push(5);
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Vec4(a) => {
            out.push(6);
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Mat3x3(a) => {
            out.push(7);
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Mat4x3(a) => {
            out.push(8);
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Mat4x4(a) => {
            // the raw layout has no 4x4 type; store as a float array
            out.push(10);
            out.extend_from_slice(&(a.len() as i32).to_le_bytes());
            a.iter().for_each(|x| f(out, x));
        }
        Variant::Ints(a) => {
            out.push(9);
            out.extend_from_slice(&(a.len() as i32).to_le_bytes());
            a.iter()
                .for_each(|i| out.extend_from_slice(&i.to_le_bytes()));
        }
        Variant::Floats(a) => {
            out.push(10);
            out.extend_from_slice(&(a.len() as i32).to_le_bytes());
            a.iter().for_each(|x| f(out, x));
        }
    }
}

/// "PCBB" blackboard layout, used by exported entities (`.epe`) and other
/// editor-saved property files. A file is a sequence of objects:
///
/// ```text
/// "PCBB"  u32 length  <length bytes: a memory image>
/// ```
///
/// Inside the image, a node is a linked list of 16-byte entries
/// `{ u32 name_hash, u32 kind (1 node / 2 variant), i32 data, i32 next }` with
/// offsets relative to the image. A node entry's `data` points at an `i32`
/// holding the offset of the child's first entry (−1 = empty). A variant
/// entry's `data` points at `u32 type` followed by the value inline (int,
/// float) or an `i32` offset to it (string = NUL-terminated; vectors and
/// matrices = floats; arrays = `i32 byte_size` then values).
pub const BLACKBOARD_MAGIC: &[u8; 4] = b"PCBB";

pub fn is_blackboard(data: &[u8]) -> bool {
    data.len() >= 8 && &data[0..4] == BLACKBOARD_MAGIC
}

impl PropertyFile {
    pub fn parse_blackboard(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let mut nodes = Vec::new();
        while !r.at_end() {
            if r.bytes(4)? != BLACKBOARD_MAGIC {
                return Err(Error::format(format!("expected PCBB at {}", r.pos() - 4)));
            }
            let len = r.u32()? as usize;
            let image = r.bytes(len)?;
            if image.len() < 16 {
                continue;
            }
            nodes.push(bb_node(image, 0, 0)?);
        }
        Ok(Self { nodes })
    }

    /// Parse whichever layout the bytes are in.
    pub fn parse_any(data: &[u8]) -> Result<Self> {
        if is_blackboard(data) {
            Self::parse_blackboard(data)
        } else {
            Self::parse(data)
        }
    }
}

fn bb_i32(img: &[u8], off: usize) -> Result<i32> {
    img.get(off..off + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(Error::Truncated {
            offset: off,
            needed: 4,
            len: img.len(),
        })
}

fn bb_u32(img: &[u8], off: usize) -> Result<u32> {
    Ok(bb_i32(img, off)? as u32)
}

fn bb_f32(img: &[u8], off: usize) -> Result<f32> {
    Ok(f32::from_bits(bb_u32(img, off)?))
}

fn bb_floats<const N: usize>(img: &[u8], off: usize) -> Result<[f32; N]> {
    let mut out = [0f32; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = bb_f32(img, off + 4 * i)?;
    }
    Ok(out)
}

fn bb_node(img: &[u8], mut entry: usize, depth: usize) -> Result<Node> {
    if depth > 64 {
        return Err(Error::format("blackboard nesting too deep"));
    }
    let mut node = Node::default();
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 100_000 {
            return Err(Error::format("blackboard entry loop"));
        }
        let id = bb_u32(img, entry)?;
        let kind = bb_u32(img, entry + 4)?;
        let data = bb_i32(img, entry + 8)?;
        let next = bb_i32(img, entry + 12)?;
        match kind {
            1 => {
                let child = if data == -1 {
                    -1
                } else {
                    bb_i32(img, data as usize)?
                };
                let n = if child == -1 {
                    Node::default()
                } else {
                    bb_node(img, child as usize, depth + 1)?
                };
                node.children.push((Key::Hash(id), n));
            }
            2 => {
                if data != -1 {
                    node.values
                        .push((Key::Hash(id), bb_variant(img, data as usize)?));
                }
            }
            k => {
                return Err(Error::format(format!(
                    "blackboard entry kind {k} at {entry}"
                )));
            }
        }
        if next == -1 {
            break;
        }
        entry = next as usize;
    }
    Ok(node)
}

fn bb_variant(img: &[u8], off: usize) -> Result<Variant> {
    let ty = bb_u32(img, off)?;
    let inline = off + 4;
    let indirect = |img: &[u8]| -> Result<Option<usize>> {
        let o = bb_i32(img, inline)?;
        Ok(if o == -1 { None } else { Some(o as usize) })
    };
    Ok(match ty {
        1 => Variant::Int(bb_i32(img, inline)?),
        2 => Variant::Float(bb_f32(img, inline)?),
        3 => match indirect(img)? {
            None => Variant::Str(String::new()),
            Some(o) => {
                let end = img[o..]
                    .iter()
                    .position(|&c| c == 0)
                    .map(|p| o + p)
                    .unwrap_or(img.len());
                Variant::Str(String::from_utf8_lossy(&img[o..end]).into_owned())
            }
        },
        4 => match indirect(img)? {
            None => Variant::Vec2([0.0; 2]),
            Some(o) => Variant::Vec2(bb_floats(img, o)?),
        },
        5 => match indirect(img)? {
            None => Variant::Vec3([0.0; 3]),
            Some(o) => Variant::Vec3(bb_floats(img, o)?),
        },
        6 => match indirect(img)? {
            None => Variant::Vec4([0.0; 4]),
            Some(o) => Variant::Vec4(bb_floats(img, o)?),
        },
        7 => match indirect(img)? {
            None => Variant::Mat3x3([0.0; 9]),
            Some(o) => Variant::Mat3x3(bb_floats(img, o)?),
        },
        8 => match indirect(img)? {
            None => Variant::Mat4x4([0.0; 16]),
            Some(o) => Variant::Mat4x4(bb_floats(img, o)?),
        },
        9 | 10 => match indirect(img)? {
            None => {
                if ty == 9 {
                    Variant::Ints(Vec::new())
                } else {
                    Variant::Floats(Vec::new())
                }
            }
            Some(o) => {
                let size = bb_i32(img, o)?.max(0) as usize;
                let n = size / 4;
                if ty == 9 {
                    Variant::Ints(
                        (0..n)
                            .map(|i| bb_i32(img, o + 4 + 4 * i))
                            .collect::<Result<_>>()?,
                    )
                } else {
                    Variant::Floats(
                        (0..n)
                            .map(|i| bb_f32(img, o + 4 + 4 * i))
                            .collect::<Result<_>>()?,
                    )
                }
            }
        },
        t => {
            return Err(Error::format(format!(
                "blackboard variant type {t} at {off}"
            )));
        }
    })
}

impl Node {
    /// Look up a direct child value by name (hashes it) or by raw hash key.
    pub fn value(&self, name: &str) -> Option<&Variant> {
        let h = hash_str(name);
        self.values
            .iter()
            .find(|(k, _)| k.hash() == h)
            .map(|(_, v)| v)
    }
    /// Look up a direct child value by its raw key hash (for keys whose name is unknown).
    pub fn value_hash(&self, hash: u32) -> Option<&Variant> {
        self.values
            .iter()
            .find(|(k, _)| k.hash() == hash)
            .map(|(_, v)| v)
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        let h = hash_str(name);
        self.children
            .iter()
            .find(|(k, _)| k.hash() == h)
            .map(|(_, n)| n)
    }
    pub fn float(&self, name: &str) -> Option<f32> {
        match self.value(name)? {
            Variant::Float(f) => Some(*f),
            Variant::Int(i) => Some(*i as f32),
            _ => None,
        }
    }
    pub fn int(&self, name: &str) -> Option<i32> {
        match self.value(name)? {
            Variant::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.value(name)? {
            Variant::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Walk every node depth-first.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node)) {
        f(self);
        for (_, c) in &self.children {
            c.walk(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        // Serialisation groups by-name entries before by-hash ones, so build
        // the fixture in that order to compare structurally.
        let mut inner = Node::default();
        inner
            .values
            .push((Key::Name("action".into()), Variant::Int(7)));
        inner
            .values
            .push((Key::Hash(0x1234), Variant::Str("MOUSE2".into())));
        inner.tag = Some("mapping".into());
        let mut named = Node::default();
        named
            .children
            .push((Key::Name("named".into()), inner.clone()));
        let mut root = Node::default();
        root.children.push((Key::Hash(0xdead_beef), inner));
        root.children.push((Key::Hash(0x1111_2222), named));
        root.values
            .push((Key::Hash(1), Variant::Vec3([1.0, 2.0, 3.0])));
        root.values
            .push((Key::Hash(2), Variant::Floats(vec![0.5, 0.25])));
        let pf = PropertyFile { nodes: vec![root] };
        let bytes = pf.to_bytes();
        // shipped files start with a 1- or 2-section root; ours has two (by-hash nodes + values)
        assert!(looks_like_property(&bytes), "{:02x?}", &bytes[..4]);
        let back = PropertyFile::parse(&bytes).unwrap();
        assert_eq!(back, pf);
    }

    #[test]
    fn real_header_shape() {
        // First bytes of a shipped keymap: 1 section, type 4 (node by hash), 1 node.
        let head = [0x01, 0x04, 0x00, 0x01, 0x00];
        assert!(looks_like_property(&head));
    }
}
