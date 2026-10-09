//! The game's bitmap fonts: `gui\fonts\fonts.blz`, a SARC of seven `.font` files
//! (`jc2_smallfont_8 / 11 / 12 / 14 / 16 / 18` and `jc2_subtitle_12`), what the Anark HUD
//! draws its words and numbers with (`research/formats/hud.md`).
//!
//! A `.font` is, little-endian:
//!
//! ```text
//! u32   dds_size
//! [u8]  a DDS file (DXT5, one mip) — the glyph atlas
//! f32   line_gap      added to line_height between lines (-1 in the 12 point face)
//! f32   spacing       added (rounded: (int)(spacing + 0.5)) to every glyph's advance
//! u16   line_height   pixels from one baseline to the next
//! u16   first         the code point of lookup[0]
//! u16   glyph_count
//! u16   lookup_len
//! glyph_count × 20 bytes:   f32 u0, v0, u1, v1   the atlas rectangle, normalised
//!                           u8  width, height    the same rectangle in pixels
//!                           u8  y_offset         from the top of the line to the glyph's top
//!                           u8  0                (always 0 in the game's fonts)
//! lookup_len × i16:         glyph index for code point `first + i`, -1 for none
//! ```
//!
//! Read off the engine's loader (`FUN_0095a470`), its text measurement (`FUN_00553720`: a
//! string is the sum of `width + (int)(spacing + 0.5)`) and its quad builder (`FUN_00939810`: the quad is
//! `width × height` at `x_pen`, `line_top + y_offset`; the pen then advances by
//! `width + spacing + extra letter spacing`; the next line is `line_height + line_gap +
//! extra line spacing` lower). There is no kerning, no bearing and no separate advance: the
//! glyph rectangles are tight, the atlas keeps 2–4 px between them. A code point that is
//! not in the font draws the engine's default glyph (all zeros: nothing, no advance).
//!
//! The atlas is DXT5. For most fonts the glyph is **in the alpha channel** and the colour
//! is white (so a text colour multiplies in); the 14 point face (the CHAOS numerals) is a
//! chrome gradient with a dark outline in RGB; the 12 point face also carries the pad
//! button icons in colour; the 8 point face is a pill: its glyph cells carry the dark
//! gradient strip behind the letters; the subtitle face is white letters inside a dark
//! outline that lives in alpha.

use crate::dds::DdsImage;
use crate::error::{Error, Result};
use crate::reader::Reader;
use crate::sarc::SmallArchive;
use crate::vfs::GameFs;

/// The container's name for [`GameFs::read_inflated`].
pub const FONTS_CONTAINER: &str = "fonts.blz";

/// In `jc2_smallfont_11` (the round counter's digits) the infinity sign sits on the code
/// point of `E`.
pub const INFINITY_CHAR: char = 'E';
/// In `jc2_smallfont_14` (the CHAOS numerals) the scorpion badge is U+0464 and the `$`
/// is a plain `$`; in `jc2_smallfont_12` U+0464..U+048D are the pad button icons.
pub const SCORPION_CHAR: char = '\u{464}';

/// The seven fonts' member names.
pub const FONT_NAMES: [&str; 7] = [
    "jc2_smallfont_8.font",
    "jc2_smallfont_11.font",
    "jc2_smallfont_12.font",
    "jc2_smallfont_14.font",
    "jc2_smallfont_16.font",
    "jc2_smallfont_18.font",
    "jc2_subtitle_12.font",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Atlas rectangle, normalised: `[u0, v0, u1, v1]` (v down).
    pub uv: [f32; 4],
    /// The same rectangle in pixels (also the glyph's advance before spacing).
    pub width: u8,
    pub height: u8,
    /// Pixels from the top of the line to the top of the glyph.
    pub y_offset: u8,
}

#[derive(Clone, Debug)]
pub struct Font {
    /// The member's name (`jc2_smallfont_18.font`).
    pub name: String,
    pub atlas: DdsImage,
    pub line_gap: f32,
    pub spacing: f32,
    pub line_height: u16,
    pub first: u16,
    pub glyphs: Vec<Glyph>,
    pub lookup: Vec<i16>,
}

impl Font {
    pub fn parse(name: &str, data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let dds_size = r.u32()? as usize;
        let dds = r.bytes(dds_size)?;
        let atlas = DdsImage::parse(dds)?;
        let line_gap = r.f32()?;
        let spacing = r.f32()?;
        let line_height = r.u16()?;
        let first = r.u16()?;
        let glyph_count = r.u16()? as usize;
        let lookup_len = r.u16()? as usize;
        let mut glyphs = Vec::with_capacity(glyph_count);
        for _ in 0..glyph_count {
            let uv = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
            let width = r.u8()?;
            let height = r.u8()?;
            let y_offset = r.u8()?;
            let _zero = r.u8()?;
            glyphs.push(Glyph {
                uv,
                width,
                height,
                y_offset,
            });
        }
        let mut lookup = Vec::with_capacity(lookup_len);
        for _ in 0..lookup_len {
            lookup.push(r.u16()? as i16);
        }
        if !r.at_end() {
            return Err(Error::format(format!(
                "{name}: {} bytes after the lookup table",
                r.remaining()
            )));
        }
        Ok(Self {
            name: name.to_string(),
            atlas,
            line_gap,
            spacing,
            line_height,
            first,
            glyphs,
            lookup,
        })
    }

    /// The glyph for `c`, if the font has one.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        let i = (c as u32).checked_sub(self.first as u32)? as usize;
        let g = *self.lookup.get(i)?;
        if g < 0 {
            return None;
        }
        self.glyphs.get(g as usize)
    }

    /// Every code point the font has, with its glyph.
    pub fn chars(&self) -> impl Iterator<Item = (char, &Glyph)> + '_ {
        self.lookup.iter().enumerate().filter_map(|(i, &g)| {
            let c = char::from_u32(self.first as u32 + i as u32)?;
            let g = self.glyphs.get(usize::try_from(g).ok()?)?;
            Some((c, g))
        })
    }

    /// The whole pixel spacing the engine adds to every advance: `(int)(spacing + 0.5)`.
    pub fn spacing_px(&self) -> i32 {
        (self.spacing + 0.5) as i32
    }

    /// Pen advance after `c`, in atlas pixels. `None` when the font lacks the character.
    pub fn advance(&self, c: char) -> Option<i32> {
        self.glyph(c).map(|g| g.width as i32 + self.spacing_px())
    }

    /// Width of a single line in atlas pixels (characters the font lacks add nothing;
    /// `\n` and `\r` are not handled here: see [`crate::font::Font::line_width`] callers).
    pub fn line_width(&self, text: &str) -> i32 {
        text.chars().filter_map(|c| self.advance(c)).sum()
    }

    /// Baseline-to-baseline distance for consecutive lines, in atlas pixels.
    pub fn line_advance(&self) -> f32 {
        self.line_height as f32 + self.line_gap
    }

    /// The top of a capital 'A' (or digit '0' for the numeral faces) below the line's top,
    /// and its height: where the visible text starts and how tall it is.
    pub fn cap_box(&self) -> (u8, u8) {
        let g = self.glyph('A').or_else(|| self.glyph('0'));
        match g {
            Some(g) => (g.y_offset, g.height),
            None => self
                .glyphs
                .first()
                .map(|g| (g.y_offset, g.height))
                .unwrap_or((0, 0)),
        }
    }

    /// The glyph's rectangle in atlas pixels `[x, y, w, h]` (the normalised rectangle
    /// rounded; the size is the glyph's own).
    pub fn rect_px(&self, g: &Glyph) -> [u32; 4] {
        let w = self.atlas.width as f32;
        let h = self.atlas.height as f32;
        [
            (g.uv[0] * w).round() as u32,
            (g.uv[1] * h).round() as u32,
            g.width as u32,
            g.height as u32,
        ]
    }

    /// The decoded atlas as RGBA8.
    pub fn atlas_rgba(&self) -> Option<Vec<u8>> {
        self.atlas.decode_rgba(0)
    }

    /// Render one line into an RGBA8 image: the glyphs composited over `bg` as the
    /// engine's blend does (source over, straight alpha), with the colour channels
    /// multiplied by `tint` (white shows the atlas as it is). Returns `(w, h, pixels)`
    /// with `pad` pixels round the text.
    pub fn render_line(
        &self,
        text: &str,
        tint: [u8; 4],
        bg: [u8; 4],
        pad: u32,
    ) -> (u32, u32, Vec<u8>) {
        let width = self.line_width(text).max(1) as u32 + pad * 2;
        let height = self.line_height as u32 + pad * 2;
        let mut out = vec![0u8; (width * height * 4) as usize];
        for px in out.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&bg);
        }
        let Some(atlas) = self.atlas_rgba() else {
            return (width, height, out);
        };
        let mut pen = pad as i32;
        for c in text.chars() {
            let Some(g) = self.glyph(c) else { continue };
            let [sx, sy, gw, gh] = self.rect_px(g);
            for y in 0..gh {
                for x in 0..gw {
                    let (ax, ay) = (sx + x, sy + y);
                    if ax >= self.atlas.width || ay >= self.atlas.height {
                        continue;
                    }
                    let s = &atlas[((ay * self.atlas.width + ax) * 4) as usize..][..4];
                    let dx = pen + x as i32;
                    let dy = pad as i32 + g.y_offset as i32 + y as i32;
                    if dx < 0 || dy < 0 || dx >= width as i32 || dy >= height as i32 {
                        continue;
                    }
                    let d = &mut out[((dy as u32 * width + dx as u32) * 4) as usize..][..4];
                    let sa = s[3] as u32 * tint[3] as u32 / 255;
                    for k in 0..3 {
                        let sc = s[k] as u32 * tint[k] as u32 / 255;
                        d[k] = ((sc * sa + d[k] as u32 * (255 - sa)) / 255) as u8;
                    }
                    d[3] = (sa + d[3] as u32 * (255 - sa) / 255) as u8;
                }
            }
            pen += g.width as i32 + self.spacing_px();
        }
        (width, height, out)
    }
}

/// The fonts of `fonts.blz`.
#[derive(Clone, Debug)]
pub struct FontSet {
    pub fonts: Vec<Font>,
}

impl FontSet {
    pub fn parse(blob: &[u8]) -> Result<Self> {
        let sarc = SmallArchive::parse(blob)?;
        let mut fonts = Vec::new();
        for e in &sarc.entries {
            if e.name.ends_with(".font") {
                fonts.push(Font::parse(&e.name, sarc.data(blob, e)?)?);
            }
        }
        Ok(Self { fonts })
    }

    pub fn load(fs: &GameFs) -> Result<Self> {
        Self::parse(&fs.read_inflated(FONTS_CONTAINER)?)
    }

    /// A font by name, with or without `.font` (`jc2_smallfont_18`).
    pub fn get(&self, name: &str) -> Option<&Font> {
        self.fonts
            .iter()
            .find(|f| f.name == name || f.name.strip_suffix(".font") == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny synthetic font: a 4×4 DXT5 atlas and three glyphs.
    fn synthetic() -> Vec<u8> {
        let dds = crate::dds::write_dds(4, 4, 1, crate::dds::DdsFormat::Dxt5, &[0u8; 16]);
        let mut v = Vec::new();
        v.extend((dds.len() as u32).to_le_bytes());
        v.extend(&dds);
        v.extend(0.0f32.to_le_bytes());
        v.extend(1.0f32.to_le_bytes());
        v.extend(10u16.to_le_bytes()); // line height
        v.extend(65u16.to_le_bytes()); // first 'A'
        v.extend(2u16.to_le_bytes()); // two glyphs
        v.extend(3u16.to_le_bytes()); // lookup A, B, C
        for (u0, w) in [(0.0f32, 2u8), (0.5, 3)] {
            for f in [u0, 0.0, u0 + w as f32 / 4.0, 0.5] {
                v.extend(f.to_le_bytes());
            }
            v.extend([w, 2, 3, 0]);
        }
        for g in [0i16, -1, 1] {
            v.extend(g.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_a_synthetic_font() {
        let f = Font::parse("t.font", &synthetic()).unwrap();
        assert_eq!(f.glyphs.len(), 2);
        assert_eq!(f.glyph('A').unwrap().width, 2);
        assert!(f.glyph('B').is_none());
        assert_eq!(f.glyph('C').unwrap().width, 3);
        assert!(f.glyph('D').is_none());
        // advance = width + (int)(1.0 + 0.5)
        assert_eq!(f.advance('A'), Some(3));
        assert_eq!(f.line_width("ACB"), 3 + 4);
        assert_eq!(f.line_advance(), 10.0);
        assert_eq!(f.chars().count(), 2);
    }

    #[test]
    fn trailing_bytes_are_an_error() {
        let mut v = synthetic();
        v.push(0);
        assert!(Font::parse("t.font", &v).is_err());
    }

    fn game() -> Option<FontSet> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        FontSet::load(&fs).ok()
    }

    /// Needs JC2_TEST_DIR. Every font: sane metrics, every glyph rectangle inside its atlas
    /// and as large as the table says, every lookup entry valid, ASCII covered by the text
    /// faces.
    #[test]
    fn the_game_fonts() {
        let Some(set) = game() else { return };
        assert_eq!(set.fonts.len(), 7);
        for name in FONT_NAMES {
            assert!(set.get(name).is_some(), "{name} missing");
        }
        for f in &set.fonts {
            assert_eq!(f.atlas.format, crate::dds::DdsFormat::Dxt5, "{}", f.name);
            assert!(f.line_height >= 11 && f.line_height <= 64, "{} lh", f.name);
            assert!(f.spacing >= 0.0 && f.spacing <= 4.0, "{} spacing", f.name);
            assert!(f.line_gap.abs() <= 4.0, "{} gap", f.name);
            assert!(f.glyphs.len() >= 12, "{}", f.name);
            let (aw, ah) = (f.atlas.width as f32, f.atlas.height as f32);
            for (i, g) in f.glyphs.iter().enumerate() {
                let [u0, v0, u1, v1] = g.uv;
                assert!(
                    u0 >= 0.0 && v0 >= 0.0 && u1 <= 1.0 && v1 <= 1.0,
                    "{} glyph {i}",
                    f.name
                );
                assert!(u1 >= u0 && v1 >= v0, "{} glyph {i} inverted", f.name);
                // the pixel size is the rectangle's size
                assert!(
                    ((u1 - u0) * aw - g.width as f32).abs() < 1.01,
                    "{} glyph {i} w",
                    f.name
                );
                assert!(
                    ((v1 - v0) * ah - g.height as f32).abs() < 1.01,
                    "{} glyph {i} h",
                    f.name
                );
                let [x, y, w, h] = f.rect_px(g);
                assert!(
                    x + w <= f.atlas.width && y + h <= f.atlas.height,
                    "{} glyph {i}",
                    f.name
                );
                assert!(
                    g.y_offset as u32 + g.height as u32 <= f.line_height as u32 + 4,
                    "{} glyph {i} sits below its line",
                    f.name
                );
            }
            for &l in &f.lookup {
                assert!(l < 0 || (l as usize) < f.glyphs.len(), "{} lookup", f.name);
            }
            // every glyph is reachable from some code point except at most one
            let reachable = f.chars().count();
            assert!(f.glyphs.len() - reachable <= 1, "{}: {reachable}", f.name);
        }
        // the text faces cover printable ASCII; the numeral faces do not
        for name in [
            "jc2_smallfont_8",
            "jc2_smallfont_12",
            "jc2_smallfont_18",
            "jc2_subtitle_12",
        ] {
            let f = set.get(name).unwrap();
            for c in ' '..='~' {
                assert!(f.glyph(c).is_some(), "{name} lacks {c:?}");
            }
            // and Latin-1 and Cyrillic
            for c in ['é', 'ß', 'Я', 'я'] {
                assert!(f.glyph(c).is_some(), "{name} lacks {c:?}");
            }
        }
        let digits = |n: &str, extra: &[char]| {
            let f = set.get(n).unwrap();
            for c in '0'..='9' {
                assert!(f.glyph(c).is_some(), "{n} lacks {c}");
            }
            for &c in extra {
                assert!(f.glyph(c).is_some(), "{n} lacks {c:?}");
            }
        };
        digits("jc2_smallfont_11", &['+']);
        digits("jc2_smallfont_14", &['+', '$']);
        digits("jc2_smallfont_16", &['/', ':']);
    }

    /// Spot checks against values read straight off the tables.
    #[test]
    fn known_glyph_metrics() {
        let Some(set) = game() else { return };
        let f = set.get("jc2_smallfont_18").unwrap();
        assert_eq!((f.line_height, f.first, f.spacing_px()), (51, 32, 2));
        let a = f.glyph('A').unwrap();
        assert_eq!((a.width, a.height, a.y_offset), (15, 17, 28));
        assert_eq!(f.advance('A'), Some(17));
        // the space is a 6 px wide box
        assert_eq!(f.glyph(' ').unwrap().width, 6);
        let f16 = set.get("jc2_smallfont_16").unwrap();
        assert_eq!((f16.line_height, f16.first), (21, 47));
        assert_eq!(f16.glyph('0').unwrap().width, 15);
        let f14 = set.get("jc2_smallfont_14").unwrap();
        // the scorpion badge lives past the Cyrillic block
        assert!(f14.glyph(SCORPION_CHAR).is_some());
        assert!(
            set.get("jc2_smallfont_11")
                .unwrap()
                .glyph(INFINITY_CHAR)
                .is_some()
        );
    }
}
