//! `global/all_lights.light_info`: the pre-placed glow sprites of the distant light sources
//! (research `research/formats/glow-sprites.md`, JC2-136; loader `0x00930000`).
//!
//! Layout (native): `u32` version = 1, `u32` count, then 16-byte records: `x, y, z` as `f32`
//! (world metres) and one `u32` whose low three bytes are R, G, B and whose top byte is the type
//! id. The type is the sprite size class: the quad is `1 + type / 2` metres across (the vertex
//! shader of the `lightsource` pair). The file is sorted by 4096 m tile (8 x 8 tiles over
//! +-16384 m, index `0x0096d7b0`).

use crate::error::{Error, Result};

/// The file inside the archives.
pub const LIGHT_INFO_FILE: &str = "global/all_lights.light_info";
/// Tiles per axis and tile side in metres (`CLightManager`, `0x0096d7b0`).
pub const TILES_PER_AXIS: usize = 8;
pub const TILE_SIZE: f32 = 4096.0;
/// Half the world extent (the tile grid origin is at -16384 m).
pub const WORLD_HALF: f32 = 16384.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowSprite {
    /// World position in the engine's metres (`x`, height, `z`).
    pub pos: [f32; 3],
    pub rgb: [u8; 3],
    /// The size class.
    pub kind: u8,
}

impl GlowSprite {
    /// The quad size in metres: `1 + type / 2` (vertex shader `S`).
    pub fn size(&self) -> f32 {
        1.0 + f32::from(self.kind) / 2.0
    }

    /// The tile slot `floor((z + 16384) / 4096) * 8 + floor((x + 16384) / 4096)` of `0x0096d7b0`
    /// (a truncating float to int, equal to floor for the positive operand). `None` outside the
    /// grid.
    pub fn tile(&self) -> Option<usize> {
        let t = |v: f32| ((v + WORLD_HALF) / TILE_SIZE).floor();
        let (tx, tz) = (t(self.pos[0]), t(self.pos[2]));
        let n = TILES_PER_AXIS as f32;
        ((0.0..n).contains(&tx) && (0.0..n).contains(&tz))
            .then(|| tz as usize * TILES_PER_AXIS + tx as usize)
    }
}

/// Parse the file. Rejects a wrong version or a count that does not fit the data.
pub fn parse(data: &[u8]) -> Result<Vec<GlowSprite>> {
    let word = |at: usize| -> Option<[u8; 4]> { data.get(at..at + 4)?.try_into().ok() };
    let version = word(0).map(u32::from_le_bytes);
    if version != Some(1) {
        return Err(Error::format(format!("light_info version {version:?}")));
    }
    let count = word(4).map(u32::from_le_bytes).unwrap_or(0) as usize;
    let body = &data[8..];
    if body.len() < count.saturating_mul(16) {
        return Err(Error::format("light_info is shorter than its count"));
    }
    Ok(body[..count * 16]
        .as_chunks::<16>()
        .0
        .iter()
        .map(|r| {
            let f = |i: usize| f32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
            GlowSprite {
                pos: [f(0), f(4), f(8)],
                rgb: [r[12], r[13], r[14]],
                kind: r[15],
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(x: f32, y: f32, z: f32, rgb: [u8; 3], kind: u8) -> Vec<u8> {
        let mut v = Vec::new();
        for f in [x, y, z] {
            v.extend_from_slice(&f.to_le_bytes());
        }
        v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], kind]);
        v
    }

    #[test]
    fn records_carry_position_colour_and_size_class() {
        let mut d = vec![1, 0, 0, 0, 2, 0, 0, 0];
        d.extend(record(10.5, 200.0, -30.0, [0xa5, 0xb2, 0xff], 2));
        d.extend(record(-16384.0, 153.0, 16383.0, [1, 2, 3], 39));
        let s = parse(&d).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].pos, [10.5, 200.0, -30.0]);
        assert_eq!(s[0].rgb, [0xa5, 0xb2, 0xff]);
        assert_eq!(s[0].size(), 2.0);
        assert_eq!(s[1].size(), 20.5);
        // (x 10.5, z -30) is tile (4, 3) = 3 * 8 + 4; the far corner is tile 7 * 8 + 0
        assert_eq!(s[0].tile(), Some(28));
        assert_eq!(s[1].tile(), Some(56));
    }

    #[test]
    fn bad_headers_are_rejected() {
        assert!(parse(&[2, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(parse(&[1, 0, 0, 0, 1, 0, 0, 0, 0]).is_err());
        assert!(parse(&[]).is_err());
    }

    /// The retail file: 32012 sprites, sorted by tile, 18 type ids (probe-checked numbers).
    /// Needs JC2_TEST_DIR.
    #[test]
    fn retail_file_matches_the_probe() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let bytes = fs.read_any(LIGHT_INFO_FILE).expect("all_lights.light_info");
        let s = parse(&bytes).unwrap();
        assert_eq!(s.len(), 32012);
        let tiles: Vec<usize> = s.iter().map(|g| g.tile().unwrap()).collect();
        assert!(tiles.windows(2).all(|w| w[0] <= w[1]), "sorted by tile");
        let mut kinds: Vec<u8> = s.iter().map(|g| g.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), 18);
        assert_eq!(s.iter().filter(|g| g.kind == 14).count(), 18308);
    }
}
