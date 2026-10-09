//! Tiny bounds-checked cursor over a byte slice, little- or big-endian.

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    pub endian: Endian,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            endian: Endian::Little,
        }
    }
    pub fn with_endian(data: &'a [u8], endian: Endian) -> Self {
        Self {
            data,
            pos: 0,
            endian,
        }
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }
    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(Error::Truncated {
                offset: pos,
                needed: 0,
                len: self.data.len(),
            });
        }
        self.pos = pos;
        Ok(())
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::Truncated {
                offset: self.pos,
                needed: n,
                len: self.data.len(),
            });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn peek(&self, n: usize) -> &'a [u8] {
        let end = (self.pos + n).min(self.data.len());
        &self.data[self.pos..end]
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        let v = [b[0], b[1]];
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(v),
            Endian::Big => u16::from_be_bytes(v),
        })
    }
    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        let v = [b[0], b[1], b[2], b[3]];
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(v),
            Endian::Big => u32::from_be_bytes(v),
        })
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    /// `n` consecutive u16 values.
    pub fn u16_vec(&mut self, n: usize) -> Result<Vec<u16>> {
        let b = self.bytes(n * 2)?;
        Ok(b.as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                let v = [c[0], c[1]];
                match self.endian {
                    Endian::Little => u16::from_le_bytes(v),
                    Endian::Big => u16::from_be_bytes(v),
                }
            })
            .collect())
    }
    /// ASCII string of exactly `n` bytes, truncated at the first NUL.
    pub fn str(&mut self, n: usize) -> Result<String> {
        let b = self.bytes(n)?;
        let b = b.split(|&c| c == 0).next().unwrap_or(b);
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}
