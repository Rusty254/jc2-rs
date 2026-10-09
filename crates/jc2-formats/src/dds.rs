//! A small DDS (DirectDraw Surface) container reader with CPU decoders for the handful
//! of pixel formats the terrain uses: DXT1/DXT5 (BC1/BC3), `A4R4G4B4`, `A8L8` and
//! `A8R8G8B8`. The game keeps most textures as plain DDS files (the models' textures
//! go straight to the GPU through Bevy's own loader); this module is for the ones the
//! engine reads back on the CPU — the per-tile terrain layers embedded in the terrain
//! tile files, the global landscape maps and the terrain texture atlases — so jc2tool can
//! picture them and the renderer can average, slice and repack them.

use crate::error::{Error, Result};
use crate::reader::Reader;

const HEADER_SIZE: usize = 128;

/// Pixel formats we can describe. Anything else parses but does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdsFormat {
    /// BC1, 8 bytes per 4×4 block, 1-bit alpha.
    Dxt1,
    /// BC2, 16 bytes per block, explicit 4-bit alpha.
    Dxt3,
    /// BC3, 16 bytes per block, interpolated alpha.
    Dxt5,
    /// 16 bits per pixel, four 4-bit channels: `a << 12 | r << 8 | g << 4 | b`.
    Argb4444,
    /// 16 bits per pixel: luminance in the low byte, alpha in the high byte.
    A8L8,
    /// 32 bits per pixel, `a << 24 | r << 16 | g << 8 | b`.
    Argb8888,
    /// 8-bit luminance.
    L8,
    /// Some other pixel format: `(fourcc, bit count, masks)`.
    Other([u8; 4], u32, [u32; 4]),
}

impl DdsFormat {
    /// Bytes of one mip level with the given dimensions.
    pub fn level_bytes(&self, width: u32, height: u32) -> Option<usize> {
        let blocks = |bytes: usize| {
            Some(bytes * width.div_ceil(4).max(1) as usize * height.div_ceil(4).max(1) as usize)
        };
        let px = |bytes: usize| Some(bytes * width as usize * height as usize);
        match self {
            DdsFormat::Dxt1 => blocks(8),
            DdsFormat::Dxt3 | DdsFormat::Dxt5 => blocks(16),
            DdsFormat::Argb4444 | DdsFormat::A8L8 => px(2),
            DdsFormat::Argb8888 => px(4),
            DdsFormat::L8 => px(1),
            DdsFormat::Other(_, bits, _) if *bits > 0 => px((*bits as usize).div_ceil(8)),
            DdsFormat::Other(..) => None,
        }
    }

    pub fn is_block_compressed(&self) -> bool {
        matches!(self, DdsFormat::Dxt1 | DdsFormat::Dxt3 | DdsFormat::Dxt5)
    }
}

/// A parsed DDS file: header fields plus the pixel data of every mip level, in file
/// order (level 0 first).
#[derive(Clone, Debug)]
pub struct DdsImage {
    pub width: u32,
    pub height: u32,
    /// Mip levels present (at least 1; a header mip count of 0 means 1).
    pub mips: u32,
    pub format: DdsFormat,
    /// Everything after the 128-byte header.
    pub data: Vec<u8>,
}

impl DdsImage {
    /// Parse a DDS file. Trailing bytes beyond the mip chain are kept (some files pad).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_SIZE || &bytes[..4] != b"DDS " {
            return Err(Error::format("not a DDS file"));
        }
        let mut r = Reader::new(bytes);
        r.seek(4)?;
        let size = r.u32()?;
        if size != 124 {
            return Err(Error::format(format!("DDS header size {size}")));
        }
        let _flags = r.u32()?;
        let height = r.u32()?;
        let width = r.u32()?;
        let _pitch = r.u32()?;
        let _depth = r.u32()?;
        let mips = r.u32()?.max(1);
        r.seek(76)?;
        let _pf_size = r.u32()?;
        let pf_flags = r.u32()?;
        let fourcc: [u8; 4] = r.bytes(4)?.try_into().unwrap();
        let bits = r.u32()?;
        let masks = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let format = if pf_flags & 0x4 != 0 {
            match &fourcc {
                b"DXT1" => DdsFormat::Dxt1,
                b"DXT3" => DdsFormat::Dxt3,
                b"DXT5" => DdsFormat::Dxt5,
                _ => DdsFormat::Other(fourcc, bits, masks),
            }
        } else {
            match (bits, masks) {
                (16, [0xf00, 0xf0, 0xf, 0xf000]) => DdsFormat::Argb4444,
                (16, [0xff, 0, 0, 0xff00]) => DdsFormat::A8L8,
                (32, [0xff0000, 0xff00, 0xff, _]) => DdsFormat::Argb8888,
                (8, [0xff, 0, 0, 0]) => DdsFormat::L8,
                _ => DdsFormat::Other(fourcc, bits, masks),
            }
        };
        let img = Self {
            width,
            height,
            mips,
            format,
            data: bytes[HEADER_SIZE..].to_vec(),
        };
        // the whole chain must be there (a truncated file is worse than an error)
        if let Some(need) = img.chain_bytes()
            && img.data.len() < need
        {
            return Err(Error::format(format!(
                "DDS {width}×{height} {format:?} with {mips} mips needs {need} bytes, has {}",
                img.data.len()
            )));
        }
        Ok(img)
    }

    /// Size of mip level `level`.
    pub fn level_size(&self, level: u32) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    /// Bytes of mip level `level` (`None` for formats we cannot size).
    pub fn level_bytes(&self, level: u32) -> Option<usize> {
        let (w, h) = self.level_size(level);
        self.format.level_bytes(w, h)
    }

    /// Byte offset of mip level `level` within `data`.
    pub fn level_offset(&self, level: u32) -> Option<usize> {
        (0..level).map(|l| self.level_bytes(l)).sum()
    }

    /// Bytes of the whole mip chain.
    pub fn chain_bytes(&self) -> Option<usize> {
        self.level_offset(self.mips)
    }

    /// The raw bytes of one mip level.
    pub fn level(&self, level: u32) -> Option<&[u8]> {
        let off = self.level_offset(level)?;
        let n = self.level_bytes(level)?;
        self.data.get(off..off + n)
    }

    /// Decode mip level `level` to tightly packed RGBA8 (sRGB bytes as stored).
    /// `A8L8` becomes `(l, l, l, a)`.
    pub fn decode_rgba(&self, level: u32) -> Option<Vec<u8>> {
        let (w, h) = self.level_size(level);
        let src = self.level(level)?;
        let n = (w * h) as usize;
        let mut out = vec![0u8; n * 4];
        match self.format {
            DdsFormat::Dxt1 => decode_bc1(src, w, h, &mut out),
            DdsFormat::Dxt5 => decode_bc3(src, w, h, &mut out),
            DdsFormat::Dxt3 => decode_bc2(src, w, h, &mut out),
            DdsFormat::Argb4444 => {
                for i in 0..n {
                    let v = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                    let x4 = |s: u16| (((v >> s) & 0xf) * 17) as u8;
                    out[i * 4..i * 4 + 4].copy_from_slice(&[x4(8), x4(4), x4(0), x4(12)]);
                }
            }
            DdsFormat::A8L8 => {
                for i in 0..n {
                    let (l, a) = (src[i * 2], src[i * 2 + 1]);
                    out[i * 4..i * 4 + 4].copy_from_slice(&[l, l, l, a]);
                }
            }
            DdsFormat::Argb8888 => {
                for i in 0..n {
                    let p = &src[i * 4..i * 4 + 4];
                    out[i * 4..i * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            DdsFormat::L8 => {
                for i in 0..n {
                    out[i * 4..i * 4 + 4].copy_from_slice(&[src[i], src[i], src[i], 255]);
                }
            }
            DdsFormat::Other(..) => return None,
        }
        Some(out)
    }

    /// Raw 16-bit texels of a 16-bit-per-pixel level (`Argb4444`, `A8L8`), row-major.
    pub fn texels_u16(&self, level: u32) -> Option<Vec<u16>> {
        if !matches!(self.format, DdsFormat::Argb4444 | DdsFormat::A8L8) {
            return None;
        }
        let src = self.level(level)?;
        Some(
            src.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        )
    }

    /// Mean colour (RGB, 0..255) of a rectangle of mip level `level`, by decoding it.
    pub fn mean_rgb(&self, level: u32, x: u32, y: u32, w: u32, h: u32) -> Option<[f32; 3]> {
        let rgba = self.decode_rgba(level)?;
        let (lw, lh) = self.level_size(level);
        let mut sum = [0.0f64; 3];
        let mut n = 0.0;
        for yy in y..(y + h).min(lh) {
            for xx in x..(x + w).min(lw) {
                let p = &rgba[((yy * lw + xx) * 4) as usize..];
                sum[0] += p[0] as f64;
                sum[1] += p[1] as f64;
                sum[2] += p[2] as f64;
                n += 1.0;
            }
        }
        if n == 0.0 {
            return None;
        }
        Some([
            (sum[0] / n) as f32,
            (sum[1] / n) as f32,
            (sum[2] / n) as f32,
        ])
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

/// Decode a BC1 colour block (8 bytes) into 16 RGBA pixels. With `opaque`, the
/// `c0 <= c1` mode's fourth entry is black with alpha 0 as the format specifies.
fn bc1_block(block: &[u8], px: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let mix = |a: [u8; 3], b: [u8; 3], wa: u32, wb: u32, div: u32| {
        [
            ((a[0] as u32 * wa + b[0] as u32 * wb) / div) as u8,
            ((a[1] as u32 * wa + b[1] as u32 * wb) / div) as u8,
            ((a[2] as u32 * wa + b[2] as u32 * wb) / div) as u8,
        ]
    };
    let (p2, p3, a3) = if c0 > c1 {
        (mix(p0, p1, 2, 1, 3), mix(p0, p1, 1, 2, 3), 255)
    } else {
        (mix(p0, p1, 1, 1, 2), [0, 0, 0], 0)
    };
    let pal = [
        [p0[0], p0[1], p0[2], 255],
        [p1[0], p1[1], p1[2], 255],
        [p2[0], p2[1], p2[2], 255],
        [p3[0], p3[1], p3[2], a3],
    ];
    for (i, p) in px.iter_mut().enumerate() {
        *p = pal[((bits >> (2 * i)) & 3) as usize];
    }
}

fn put_block(out: &mut [u8], w: u32, h: u32, bx: u32, by: u32, px: &[[u8; 4]; 16]) {
    for (i, p) in px.iter().enumerate() {
        let x = bx * 4 + (i as u32 % 4);
        let y = by * 4 + (i as u32 / 4);
        if x < w && y < h {
            let o = ((y * w + x) * 4) as usize;
            out[o..o + 4].copy_from_slice(p);
        }
    }
}

fn decode_bc1(src: &[u8], w: u32, h: u32, out: &mut [u8]) {
    let bw = w.div_ceil(4).max(1);
    let bh = h.div_ceil(4).max(1);
    let mut px = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let o = ((by * bw + bx) * 8) as usize;
            bc1_block(&src[o..o + 8], &mut px);
            put_block(out, w, h, bx, by, &px);
        }
    }
}

fn decode_bc3(src: &[u8], w: u32, h: u32, out: &mut [u8]) {
    let bw = w.div_ceil(4).max(1);
    let bh = h.div_ceil(4).max(1);
    let mut px = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let o = ((by * bw + bx) * 16) as usize;
            let b = &src[o..o + 16];
            bc1_block(&b[8..16], &mut px);
            let a0 = b[0] as u32;
            let a1 = b[1] as u32;
            let mut abits = 0u64;
            for (i, v) in b[2..8].iter().enumerate() {
                abits |= (*v as u64) << (8 * i);
            }
            let apal: [u8; 8] = if a0 > a1 {
                [
                    a0 as u8,
                    a1 as u8,
                    ((6 * a0 + a1) / 7) as u8,
                    ((5 * a0 + 2 * a1) / 7) as u8,
                    ((4 * a0 + 3 * a1) / 7) as u8,
                    ((3 * a0 + 4 * a1) / 7) as u8,
                    ((2 * a0 + 5 * a1) / 7) as u8,
                    ((a0 + 6 * a1) / 7) as u8,
                ]
            } else {
                [
                    a0 as u8,
                    a1 as u8,
                    ((4 * a0 + a1) / 5) as u8,
                    ((3 * a0 + 2 * a1) / 5) as u8,
                    ((2 * a0 + 3 * a1) / 5) as u8,
                    ((a0 + 4 * a1) / 5) as u8,
                    0,
                    255,
                ]
            };
            for (i, p) in px.iter_mut().enumerate() {
                p[3] = apal[((abits >> (3 * i)) & 7) as usize];
            }
            put_block(out, w, h, bx, by, &px);
        }
    }
}

fn decode_bc2(src: &[u8], w: u32, h: u32, out: &mut [u8]) {
    let bw = w.div_ceil(4).max(1);
    let bh = h.div_ceil(4).max(1);
    let mut px = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let o = ((by * bw + bx) * 16) as usize;
            let b = &src[o..o + 16];
            bc1_block(&b[8..16], &mut px);
            for (i, p) in px.iter_mut().enumerate() {
                let nib = (b[i / 2] >> (4 * (i % 2))) & 0xf;
                p[3] = nib * 17;
            }
            put_block(out, w, h, bx, by, &px);
        }
    }
}

/// Build a DDS file in memory (for tests and for repacking slices of an atlas).
pub fn write_dds(width: u32, height: u32, mips: u32, format: DdsFormat, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_SIZE + data.len());
    out.extend_from_slice(b"DDS ");
    let mut h = [0u32; 31];
    h[0] = 124;
    h[1] = 0x1 | 0x2 | 0x4 | 0x1000 | if mips > 1 { 0x20000 } else { 0 };
    h[2] = height;
    h[3] = width;
    h[4] = format.level_bytes(width, height).unwrap_or(0) as u32;
    h[6] = mips;
    h[18] = 32;
    match format {
        DdsFormat::Dxt1 | DdsFormat::Dxt3 | DdsFormat::Dxt5 => {
            h[19] = 0x4;
            let cc = match format {
                DdsFormat::Dxt1 => b"DXT1",
                DdsFormat::Dxt3 => b"DXT3",
                _ => b"DXT5",
            };
            h[20] = u32::from_le_bytes(*cc);
        }
        DdsFormat::Argb4444 => {
            h[19] = 0x41;
            h[21] = 16;
            h[22..26].copy_from_slice(&[0xf00, 0xf0, 0xf, 0xf000]);
        }
        DdsFormat::A8L8 => {
            h[19] = 0x20001;
            h[21] = 16;
            h[22..26].copy_from_slice(&[0xff, 0, 0, 0xff00]);
        }
        DdsFormat::Argb8888 => {
            h[19] = 0x41;
            h[21] = 32;
            h[22..26].copy_from_slice(&[0xff0000, 0xff00, 0xff, 0xff000000]);
        }
        DdsFormat::L8 => {
            h[19] = 0x20000;
            h[21] = 8;
            h[22] = 0xff;
        }
        DdsFormat::Other(cc, bits, masks) => {
            h[19] = if bits == 0 { 0x4 } else { 0x40 };
            h[20] = u32::from_le_bytes(cc);
            h[21] = bits;
            h[22..26].copy_from_slice(&masks);
        }
    }
    h[26] = 0x1000 | if mips > 1 { 0x400008 } else { 0 };
    for v in h {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb4444_round_trip() {
        let texels: Vec<u16> = vec![0xb00b, 0xf000, 0x0f00, 0x00f0];
        let mut data = Vec::new();
        for t in &texels {
            data.extend_from_slice(&t.to_le_bytes());
        }
        let file = write_dds(2, 2, 1, DdsFormat::Argb4444, &data);
        let img = DdsImage::parse(&file).unwrap();
        assert_eq!(img.format, DdsFormat::Argb4444);
        assert_eq!(img.texels_u16(0).unwrap(), texels);
        let rgba = img.decode_rgba(0).unwrap();
        // 0xb00b: a=11 r=0 g=0 b=11 → (0, 0, 187, 187)
        assert_eq!(&rgba[0..4], &[0, 0, 187, 187]);
        assert_eq!(&rgba[8..12], &[255, 0, 0, 0]);
    }

    #[test]
    fn bc1_solid_block_and_mips() {
        // one 4×4 block: c0 = pure red (0xf800), c1 = pure blue (0x001f), all indices 0
        let mut data = Vec::new();
        data.extend_from_slice(&0xf800u16.to_le_bytes());
        data.extend_from_slice(&0x001fu16.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        // mip 1 (2×2) and mip 2 (1×1): one block each, all index 1
        for _ in 0..2 {
            data.extend_from_slice(&0xf800u16.to_le_bytes());
            data.extend_from_slice(&0x001fu16.to_le_bytes());
            data.extend_from_slice(&0x5555_5555u32.to_le_bytes());
        }
        let file = write_dds(4, 4, 3, DdsFormat::Dxt1, &data);
        let img = DdsImage::parse(&file).unwrap();
        assert_eq!(img.chain_bytes(), Some(24));
        assert_eq!(img.level_offset(2), Some(16));
        let m0 = img.decode_rgba(0).unwrap();
        assert_eq!(&m0[0..4], &[255, 0, 0, 255]);
        let m2 = img.decode_rgba(2).unwrap();
        assert_eq!(m2, vec![0, 0, 255, 255]);
        assert_eq!(img.mean_rgb(0, 0, 0, 4, 4), Some([255.0, 0.0, 0.0]));
        assert!(DdsImage::parse(&file[..file.len() - 4]).is_err());
    }

    #[test]
    fn a8l8_and_argb8888() {
        let file = write_dds(1, 1, 1, DdsFormat::A8L8, &[10, 200]);
        let img = DdsImage::parse(&file).unwrap();
        assert_eq!(img.format, DdsFormat::A8L8);
        assert_eq!(img.decode_rgba(0).unwrap(), vec![10, 10, 10, 200]);
        let file = write_dds(1, 1, 1, DdsFormat::Argb8888, &[1, 2, 3, 4]);
        let img = DdsImage::parse(&file).unwrap();
        assert_eq!(img.format, DdsFormat::Argb8888);
        assert_eq!(img.decode_rgba(0).unwrap(), vec![3, 2, 1, 4]);
    }
}
