//! Area sets: which shared model container a tile draws on.
//!
//! Besides its own location container every tile may reference models from one of the
//! `global\areasets\areasetNN.blz` containers (regional building kits: the jammer and
//! bunker props, settlement houses…), on top of `general.blz`, which is always loaded.
//! The assignment is a picture: `areasets.bmp` is a 64 × 64 8-bit BMP with one pixel per
//! tile (column = tile x, row from the top of the image = tile z) and `areasets.bin`
//! lists the sets with the colour that stands for each (`folder` is the container's name
//! without `.blz`, keys `#66dd312b/#745eb2e4/#e91487dc` are its red, green and blue).
//!
//! Checked against the data: tiles whose location file references models missing from
//! both their own container and `general.blz` find them in the set the picture names
//! (areaset08 for 22,1 / 17,2 / 58,5; areaset04 for 25,6). A few references still fall
//! outside (45,9 wants two `gb017` models that only areaset03/07 carry), so a renderer
//! should fall back to the other sets rather than fail.

use crate::error::{Error, Result};
use crate::property::{PropertyFile, Variant};
use crate::terrain::TILES_PER_SIDE;

#[derive(Clone, Debug, PartialEq)]
pub struct AreaSet {
    /// Container name without extension, lower case (`areaset05`, `general`).
    pub folder: String,
    /// Display name (`AreaSet05`).
    pub name: String,
    pub colour: [u8; 3],
}

impl AreaSet {
    /// Archive path of this set's container.
    pub fn container(&self) -> String {
        format!("global\\areasets\\{}.blz", self.folder)
    }
}

#[derive(Clone, Debug)]
pub struct AreaSets {
    pub sets: Vec<AreaSet>,
    /// Per tile (row-major, row = z): index into `sets`, or `None` for a colour the
    /// list does not name.
    pub tiles: Vec<Option<u16>>,
}

/// Minimal 8-bit palettised BMP reader: returns (width, height, RGB per pixel, top-down).
fn parse_bmp8(data: &[u8]) -> Result<(usize, usize, Vec<[u8; 3]>)> {
    if data.len() < 54 || &data[0..2] != b"BM" {
        return Err(Error::format("not a BMP"));
    }
    let u32_at = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let pixels_at = u32_at(10) as usize;
    let header = u32_at(14) as usize;
    let width = u32_at(18) as usize;
    let height_raw = u32_at(22) as i32;
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    let compression = u32_at(30);
    if bpp != 8 || compression != 0 {
        return Err(Error::format(format!(
            "unsupported BMP: {bpp} bpp, compression {compression}"
        )));
    }
    let top_down = height_raw < 0;
    let height = height_raw.unsigned_abs() as usize;
    let colours = match u32_at(46) {
        0 => 256,
        n => n as usize,
    };
    let palette_at = 14 + header;
    if data.len() < palette_at + colours * 4 {
        return Err(Error::format("BMP palette truncated"));
    }
    let palette: Vec<[u8; 3]> = (0..colours)
        .map(|i| {
            let e = &data[palette_at + i * 4..palette_at + i * 4 + 4];
            [e[2], e[1], e[0]]
        })
        .collect();
    let stride = width.div_ceil(4) * 4;
    if data.len() < pixels_at + stride * height {
        return Err(Error::format("BMP pixels truncated"));
    }
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let stored_row = if top_down { y } else { height - 1 - y };
        let row = &data[pixels_at + stored_row * stride..];
        for &i in &row[..width] {
            out.push(*palette.get(i as usize).unwrap_or(&[0, 0, 0]));
        }
    }
    Ok((width, height, out))
}

impl AreaSets {
    pub fn parse(bin: &[u8], bmp: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(bin)?;
        let mut sets = Vec::new();
        let int = |n: &crate::property::Node, key: u32| match n.value_hash(key) {
            Some(Variant::Int(i)) => Some(*i),
            _ => None,
        };
        for root in &file.nodes {
            for (_, group) in &root.children {
                for (_, n) in &group.children {
                    let Some(folder) = n.str("folder") else {
                        continue;
                    };
                    let (Some(r), Some(g), Some(b)) =
                        (int(n, 0x66dd312b), int(n, 0x745eb2e4), int(n, 0xe91487dc))
                    else {
                        continue;
                    };
                    sets.push(AreaSet {
                        folder: folder.to_ascii_lowercase(),
                        name: n.str("name").unwrap_or(folder).to_string(),
                        colour: [r as u8, g as u8, b as u8],
                    });
                }
            }
        }
        if sets.is_empty() {
            return Err(Error::format("areasets.bin lists no sets"));
        }
        let (w, h, px) = parse_bmp8(bmp)?;
        let n = TILES_PER_SIDE as usize;
        if w != n || h != n {
            return Err(Error::format(format!(
                "areasets.bmp is {w}×{h}, expected {n}×{n}"
            )));
        }
        let tiles = px
            .iter()
            .map(|c| sets.iter().position(|s| &s.colour == c).map(|i| i as u16))
            .collect();
        Ok(Self { sets, tiles })
    }

    /// The set tile `(x, z)` draws on, if the picture names one.
    pub fn set_for_tile(&self, x: u32, z: u32) -> Option<&AreaSet> {
        let n = TILES_PER_SIDE;
        if x >= n || z >= n {
            return None;
        }
        let i = self.tiles[(z * n + x) as usize]?;
        self.sets.get(i as usize)
    }

    /// Every set container except `general.blz`, in list order.
    pub fn regional_containers(&self) -> Vec<String> {
        self.sets
            .iter()
            .filter(|s| s.folder != "general")
            .map(AreaSet::container)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `areasets.bin` and `areasets.bmp` reachable.
    #[test]
    fn real_map() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(bin), Ok(bmp)) = (
            fs.read_inflated("areasets.bin"),
            fs.read_inflated("areasets.bmp"),
        ) else {
            return;
        };
        let a = AreaSets::parse(&bin, &bmp).unwrap();
        assert_eq!(a.sets.len(), 16);
        assert!(a.sets.iter().any(|s| s.folder == "general"));
        assert_eq!(
            a.set_for_tile(22, 1).map(|s| s.folder.as_str()),
            Some("areaset08")
        );
        assert_eq!(
            a.set_for_tile(25, 6).map(|s| s.folder.as_str()),
            Some("areaset04")
        );
        assert_eq!(
            a.set_for_tile(31, 14).map(|s| s.folder.as_str()),
            Some("areaset05")
        );
        assert_eq!(a.regional_containers().len(), 15);
        assert!(
            a.regional_containers()
                .contains(&"global\\areasets\\areaset05.blz".to_string())
        );
    }

    #[test]
    fn bmp_rows_are_bottom_up() {
        // 4×2, 8 bpp, two palette entries; stored bottom row first
        let mut d = Vec::new();
        d.extend_from_slice(b"BM");
        d.extend_from_slice(&0u32.to_le_bytes()); // size (unused)
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&(14u32 + 40 + 8).to_le_bytes()); // pixel offset
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&4u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&8u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 16]); // compression, image size, ppm x/y
        d.extend_from_slice(&2u32.to_le_bytes()); // colours used
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&[0, 0, 255, 0]); // palette 0: red (BGR)
        d.extend_from_slice(&[255, 0, 0, 0]); // palette 1: blue
        d.extend_from_slice(&[1, 1, 1, 1]); // stored first = bottom row: blue
        d.extend_from_slice(&[0, 0, 0, 0]); // top row: red
        let (w, h, px) = parse_bmp8(&d).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(px[0], [255, 0, 0]);
        assert_eq!(px[4], [0, 0, 255]);
    }
}
