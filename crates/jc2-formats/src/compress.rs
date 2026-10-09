//! zlib handling. Files whose extension ends in `z` inside the archives
//! (`.eez .blz .nlz .flz`) are raw zlib streams (no size prefix) wrapping a
//! SARC. The same files dropped into `dropzone\` are usually stored raw, so
//! callers should detect rather than trust the extension.

use crate::error::{Error, Result};
use std::borrow::Cow;
use std::io::Read;

/// zlib stream header: CMF 0x78 (deflate, 32K window) + a valid FLG byte.
pub fn is_zlib(data: &[u8]) -> bool {
    data.len() >= 2
        && data[0] == 0x78
        && ((data[0] as u16) << 8 | data[1] as u16).is_multiple_of(31)
}

pub fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4);
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| Error::format(format!("zlib inflate failed: {e}")))?;
    Ok(out)
}

pub fn deflate(data: &[u8], level: u32) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
    enc.write_all(data).expect("in-memory write");
    enc.finish().expect("in-memory finish")
}

/// Inflate if the data is a zlib stream, otherwise hand it back untouched.
pub fn maybe_inflate(data: &[u8]) -> Result<Cow<'_, [u8]>> {
    if is_zlib(data) {
        Ok(Cow::Owned(inflate(data)?))
    } else {
        Ok(Cow::Borrowed(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = b"the quick brown fox ".repeat(50);
        let z = deflate(&src, 6);
        assert!(is_zlib(&z));
        assert_eq!(inflate(&z).unwrap(), src);
        assert_eq!(maybe_inflate(b"DDS ").unwrap().as_ref(), b"DDS ");
    }
}
