//! Text drawn with the game's own bitmap fonts (`gui\fonts\fonts.blz`, decoded by
//! `jc2_formats::font`), for Bevy UI, which cannot draw them itself.
//!
//! A [`BitmapText`] is a component on a UI node. A system lays the string out (the pure
//! [`layout_text`]: advances are `width + spacing` per glyph, as the engine does it, no
//! kerning; alignment and greedy wrapping on spaces) and keeps one child `ImageNode` per
//! glyph, each showing its rectangle of the font's atlas (`ImageNode::rect`) at an
//! absolute position. The atlas is DXT5 with the glyph in alpha and white colour, so
//! `ImageNode::color` tints it; the node's `width`/`height` are set to the text's size so a
//! flex parent (a pill) wraps it like it wrapped a `Text`. An optional drop shadow is a
//! second, darker copy of every glyph behind it. Characters the font lacks are drawn with
//! Bevy's default font (a `Text` child per character, advanced by a monospace estimate); so is
//! everything when the fonts could not be read.
//!
//! Sizes are in the HUD's reference pixels (the 2000 × 1116 frame as `vmin`:
//! `hud::r`). `size` is the height of a capital letter in them (of the digit `0` for the
//! numeral faces): the glyphs are scaled so the font's own cap height becomes `size`.

use crate::Game;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use jc2_formats::font::{Font, FontSet};

/// The seven faces, by the job the HUD gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudFont {
    /// `jc2_smallfont_8`: every glyph cell carries a dark gradient strip (a pill with the
    /// letter on it); also vehicle icons and a trophy medallion.
    Pill,
    /// `jc2_smallfont_11`: `+`, the digits and the infinity sign (`E`), small and bold.
    Ammo,
    /// `jc2_smallfont_12`: the regular-weight humanist sans (and the pad button icons).
    Light,
    /// `jc2_smallfont_14`: the CHAOS popup's chrome italic numerals, `$` and the scorpion.
    Chaos,
    /// `jc2_smallfont_16`: bold digits with `/` and `:`.
    Digits,
    /// `jc2_smallfont_18`: the bold humanist sans: Latin, Latin-1, Latin Extended-A, Cyrillic.
    Bold,
    /// `jc2_subtitle_12`: white letters in a dark outline.
    Subtitle,
}

impl HudFont {
    pub const ALL: [HudFont; 7] = [
        HudFont::Pill,
        HudFont::Ammo,
        HudFont::Light,
        HudFont::Chaos,
        HudFont::Digits,
        HudFont::Bold,
        HudFont::Subtitle,
    ];

    /// The member of `fonts.blz`.
    pub fn file(self) -> &'static str {
        match self {
            HudFont::Pill => "jc2_smallfont_8.font",
            HudFont::Ammo => "jc2_smallfont_11.font",
            HudFont::Light => "jc2_smallfont_12.font",
            HudFont::Chaos => "jc2_smallfont_14.font",
            HudFont::Digits => "jc2_smallfont_16.font",
            HudFont::Bold => "jc2_smallfont_18.font",
            HudFont::Subtitle => "jc2_subtitle_12.font",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The height of a capital (of `0` for the numeral faces) in the font's own pixels,
    /// as the HUD reference frame sees it: the original draws its fonts 1:1 on frames
    /// 2561 pixels wide, 1.28 times the 2000-pixel reference frame, so a `size` of
    /// `native()` is the original's own size.
    pub fn native(self) -> f32 {
        let cap = match self {
            HudFont::Pill => 33.0,
            HudFont::Ammo => 13.0,
            HudFont::Light => 15.0,
            HudFont::Chaos => 38.0,
            HudFont::Digits => 20.0,
            HudFont::Bold => 17.0,
            HudFont::Subtitle => 18.0,
        };
        cap / 1.28
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    // Available for multiline labels; current HUD labels use left or centre alignment.
    #[allow(dead_code)]
    Right,
}

/// Text in one of the game's bitmap fonts. Needs the [`BitmapTextPlugin`].
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Node = text_node(), Rendered)]
pub struct BitmapText {
    pub font: HudFont,
    pub text: String,
    /// Height of a capital letter in reference pixels.
    pub size: f32,
    pub colour: Color,
    pub align: TextAlign,
    /// A drop shadow: offset in reference pixels and colour (its alpha is multiplied by the
    /// text's, so it fades with it).
    pub shadow: Option<(Vec2, Color)>,
    /// Wrap lines wider than this many reference pixels (at spaces).
    pub max_width: Option<f32>,
}

fn text_node() -> Node {
    Node {
        flex_shrink: 0.0,
        ..default()
    }
}

impl BitmapText {
    pub fn new(font: HudFont, text: impl Into<String>, size: f32) -> Self {
        Self {
            font,
            text: text.into(),
            size,
            colour: Color::WHITE,
            align: TextAlign::Left,
            shadow: None,
            max_width: None,
        }
    }
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }
    pub fn with_colour(mut self, colour: Color) -> Self {
        self.colour = colour;
        self
    }
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
    pub fn with_shadow(mut self, offset: f32, colour: Color) -> Self {
        self.shadow = Some((Vec2::splat(offset), colour));
        self
    }
    pub fn with_wrap(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }
}

/// What a [`BitmapText`] has drawn so far: its glyph nodes, to recolour or replace.
#[derive(Component, Default, Debug)]
pub struct Rendered {
    last: Option<BitmapText>,
    /// `(entity, is_shadow)` for the font glyphs, shadows first.
    glyphs: Vec<(Entity, bool)>,
    /// The default-font stand-ins for characters the font lacks.
    fallbacks: Vec<Entity>,
    /// The size of the text in reference pixels, as laid out.
    pub size: Vec2,
}

#[cfg(test)]
impl Rendered {
    /// The font glyphs drawn (without their shadows) plus the fallback characters: one per
    /// visible character.
    pub fn drawn(&self) -> usize {
        self.glyphs.iter().filter(|(_, s)| !s).count() + self.fallbacks.len()
    }
    /// Characters drawn with the default font because the font lacks them.
    pub fn fallback_count(&self) -> usize {
        self.fallbacks.len()
    }
    /// The glyph nodes (shadows excluded).
    pub fn glyph_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.glyphs.iter().filter(|(_, s)| !s).map(|(e, _)| *e)
    }
}

// ---------------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------------

/// One placed character, in reference pixels from the text's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub ch: char,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The glyph's rectangle in the atlas `[x, y, w, h]`; `None` for a fallback character.
    pub rect: Option<[u32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub glyphs: Vec<PlacedGlyph>,
    /// The widest line, in reference pixels.
    pub width: f32,
    /// From the capital's top of the first line to the last line's bottom.
    pub height: f32,
    pub lines: usize,
    /// Each line's width.
    pub line_widths: Vec<f32>,
}

/// A monospace default-font character is this many times `size` wide (the default font's
/// advance is 0.6 em and its cap height about 0.7 em).
const FALLBACK_ADVANCE: f32 = 0.86;
/// …and its em is this many times `size`.
const FALLBACK_EM: f32 = 1.45;

/// The pixel height of the font's reference capital: `A`, else `0`, else the first glyph.
fn cap_h(font: &Font) -> f32 {
    (font.cap_box().1 as f32).max(1.0)
}

/// Lay `text` out in `font` (or in the default font alone when `None`), `size` reference
/// pixels per capital height. Pure: no ECS.
pub fn layout_text(
    font: Option<&Font>,
    text: &str,
    size: f32,
    align: TextAlign,
    max_width: Option<f32>,
) -> TextBlock {
    let scale = font.map(|f| size / cap_h(f)).unwrap_or(0.0);
    // advance of one character in reference pixels, and whether the font has it
    let advance = |c: char| -> (f32, bool) {
        match font.and_then(|f| f.advance(c).map(|a| (f, a))) {
            Some((_, a)) => (a as f32 * scale, true),
            None => (size * FALLBACK_ADVANCE, false),
        }
    };
    let space = advance(' ').0;

    // split into lines: hard breaks, then greedy wrapping at spaces
    let mut lines: Vec<Vec<char>> = Vec::new();
    for hard in text.split('\n') {
        let hard = hard.trim_end_matches('\r');
        let mut cur: Vec<char> = Vec::new();
        let mut cur_w = 0.0f32;
        for (wi, word) in hard.split(' ').enumerate() {
            let w: f32 = word.chars().map(|c| advance(c).0).sum();
            let join = if wi > 0 { space } else { 0.0 };
            let wrap =
                max_width.is_some_and(|max| !cur.is_empty() && w > 0.0 && cur_w + join + w > max);
            if wrap {
                // the space the line broke at is dropped
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            } else if wi > 0 {
                cur.push(' ');
                cur_w += space;
            }
            cur.extend(word.chars());
            cur_w += w;
        }
        lines.push(cur);
    }

    let line_widths: Vec<f32> = lines
        .iter()
        .map(|l| l.iter().map(|c| advance(*c).0).sum())
        .collect();
    let width = line_widths.iter().cloned().fold(0.0, f32::max);

    let (cap_top, line_step, box_bottom) = match font {
        Some(f) => {
            let (top, h) = f.cap_box();
            let lh = f.line_advance() * scale;
            let ch = h as f32;
            // room under the baseline for descenders: what the line has, up to 35 % of the cap
            let below = (f.line_height as f32 - top as f32 - ch).clamp(0.0, ch * 0.35);
            (top as f32, lh, (ch + below) * scale)
        }
        None => (0.0, size * 2.0, size * 1.45),
    };

    let mut glyphs = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut x = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (width - line_widths[li]) * 0.5,
            TextAlign::Right => width - line_widths[li],
        };
        let y0 = li as f32 * line_step;
        for &c in line {
            let (adv, has) = advance(c);
            if c != ' ' {
                if let (true, Some(f)) = (has, font)
                    && let Some(g) = f.glyph(c)
                {
                    glyphs.push(PlacedGlyph {
                        ch: c,
                        x,
                        y: y0 + (g.y_offset as f32 - cap_top) * scale,
                        w: g.width as f32 * scale,
                        h: g.height as f32 * scale,
                        rect: Some(f.rect_px(g)),
                    });
                } else if !c.is_control() {
                    glyphs.push(PlacedGlyph {
                        ch: c,
                        x,
                        y: y0,
                        w: adv,
                        h: size * FALLBACK_EM,
                        rect: None,
                    });
                }
            }
            x += adv;
        }
    }
    let n = lines.len();
    TextBlock {
        glyphs,
        width,
        height: (n.saturating_sub(1)) as f32 * line_step + box_bottom,
        lines: n,
        line_widths,
    }
}

// ---------------------------------------------------------------------------------
// the plugin
// ---------------------------------------------------------------------------------

/// The seven fonts with their atlas textures.
#[derive(Resource)]
pub struct BitmapFonts {
    fonts: Vec<(Font, Handle<Image>)>,
}

impl BitmapFonts {
    /// Decode every atlas into an RGBA texture. `None` if a face of [`HudFont`] is missing.
    pub fn from_set(set: &FontSet, images: &mut Assets<Image>) -> Option<Self> {
        let mut fonts = Vec::new();
        for f in HudFont::ALL {
            let font = set.get(f.file())?.clone();
            let rgba = font.atlas_rgba()?;
            let image = Image::new(
                Extent3d {
                    width: font.atlas.width,
                    height: font.atlas.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            fonts.push((font, images.add(image)));
        }
        Some(Self { fonts })
    }

    pub fn font(&self, f: HudFont) -> &Font {
        &self.fonts[f.index()].0
    }

    fn image(&self, f: HudFont) -> &Handle<Image> {
        &self.fonts[f.index()].1
    }
}

pub struct BitmapTextPlugin;

impl Plugin for BitmapTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, load_fonts).add_systems(
            PostUpdate,
            render_bitmap_text.before(bevy::ui::UiSystems::Prepare),
        );
    }
}

fn load_fonts(
    mut commands: Commands,
    game: Option<Res<Game>>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let (Some(game), Some(mut images)) = (game, images) else {
        return;
    };
    match FontSet::load(&game.fs) {
        Ok(set) => match BitmapFonts::from_set(&set, &mut images) {
            Some(fonts) => commands.insert_resource(fonts),
            None => warn!("fonts.blz lacks a face the HUD uses; the default font stands in"),
        },
        Err(e) => warn!("fonts.blz not readable ({e}); the default font stands in"),
    }
}

fn with_alpha(c: Color, a: f32) -> Color {
    let s = c.to_srgba();
    Color::srgba(s.red, s.green, s.blue, s.alpha * a)
}

#[allow(clippy::type_complexity)]
fn render_bitmap_text(
    mut commands: Commands,
    fonts: Option<Res<BitmapFonts>>,
    mut texts: Query<(Entity, &BitmapText, &mut Rendered, &mut Node), Changed<BitmapText>>,
    mut images: Query<&mut ImageNode>,
    mut colours: Query<&mut TextColor>,
) {
    for (entity, bt, mut rendered, mut node) in &mut texts {
        let previous = rendered.last.take();
        // Only the colours changed: recolour what is there (fades run every frame).
        if let Some(prev) = &previous
            && prev.font == bt.font
            && prev.text == bt.text
            && prev.size == bt.size
            && prev.align == bt.align
            && prev.max_width == bt.max_width
            && prev.shadow.map(|s| s.0) == bt.shadow.map(|s| s.0)
        {
            for (e, is_shadow) in &rendered.glyphs {
                if let Ok(mut img) = images.get_mut(*e) {
                    img.color = if *is_shadow {
                        bt.shadow
                            .map(|(_, c)| with_alpha(c, bt.colour.alpha()))
                            .unwrap_or(Color::NONE)
                    } else {
                        bt.colour
                    };
                }
            }
            for e in &rendered.fallbacks {
                if let Ok(mut c) = colours.get_mut(*e) {
                    c.0 = bt.colour;
                }
            }
            rendered.last = Some(bt.clone());
            continue;
        }

        let loaded = fonts.as_ref().map(|f| (f.font(bt.font), f.image(bt.font)));
        let layout = layout_text(
            loaded.map(|l| l.0),
            &bt.text,
            bt.size,
            bt.align,
            bt.max_width,
        );
        for (e, _) in rendered.glyphs.drain(..) {
            commands.entity(e).despawn();
        }
        for e in rendered.fallbacks.drain(..) {
            commands.entity(e).despawn();
        }

        let w = crate::hud::r(layout.width);
        let h = crate::hud::r(layout.height);
        if node.width != w {
            node.width = w;
        }
        if node.height != h {
            node.height = h;
        }
        rendered.size = Vec2::new(layout.width, layout.height);

        let mut glyphs: Vec<(Entity, bool)> = Vec::new();
        let mut fallbacks: Vec<Entity> = Vec::new();
        commands.entity(entity).with_children(|p| {
            let at = |g: &PlacedGlyph, dx: f32, dy: f32| Node {
                position_type: PositionType::Absolute,
                left: crate::hud::r(g.x + dx),
                top: crate::hud::r(g.y + dy),
                width: crate::hud::r(g.w),
                height: crate::hud::r(g.h),
                ..default()
            };
            let glyph_node = |g: &PlacedGlyph, dx: f32, dy: f32, c: Color| {
                let (_, image) = loaded.unwrap();
                let [x, y, w, h] = g.rect.unwrap();
                (
                    at(g, dx, dy),
                    ImageNode {
                        image: image.clone(),
                        rect: Some(Rect::new(
                            x as f32,
                            y as f32,
                            (x + w) as f32,
                            (y + h) as f32,
                        )),
                        color: c,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                )
            };
            if let Some((off, sc)) = bt.shadow {
                let c = with_alpha(sc, bt.colour.alpha());
                for g in layout.glyphs.iter().filter(|g| g.rect.is_some()) {
                    glyphs.push((p.spawn(glyph_node(g, off.x, off.y, c)).id(), true));
                }
            }
            for g in &layout.glyphs {
                if g.rect.is_some() {
                    glyphs.push((p.spawn(glyph_node(g, 0.0, 0.0, bt.colour)).id(), false));
                } else {
                    // the default font's stand-in: a Text child, its top a little above the cap
                    let em = g.h;
                    let mut n = at(g, 0.0, -0.22 * em);
                    n.height = Val::Auto;
                    fallbacks.push(
                        p.spawn((
                            n,
                            Text::new(g.ch.to_string()),
                            TextFont {
                                font_size: crate::hud::rf(em),
                                ..default()
                            },
                            TextColor(bt.colour),
                            bevy::text::TextLayout::no_wrap(),
                        ))
                        .id(),
                    );
                }
            }
        });
        rendered.glyphs = glyphs;
        rendered.fallbacks = fallbacks;
        rendered.last = Some(bt.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::dds::{DdsFormat, DdsImage};
    use jc2_formats::font::Glyph;

    /// A font with 'A' 10×20 at line offset 4, 'B' 6×20, ' ' 4×2; spacing 1, line height 30.
    fn font() -> Font {
        let g = |w: u8, h: u8, y: u8, u0: f32| Glyph {
            uv: [u0, 0.0, u0 + w as f32 / 64.0, h as f32 / 32.0],
            width: w,
            height: h,
            y_offset: y,
        };
        let mut lookup = vec![-1i16; 34];
        lookup[0] = 0; // ' ' (code 32)
        lookup[33] = 1; // 'A' (65)
        let mut f = Font {
            name: "t.font".into(),
            atlas: DdsImage {
                width: 64,
                height: 32,
                mips: 1,
                format: DdsFormat::Dxt5,
                data: vec![0; 64 * 32],
            },
            line_gap: 0.0,
            spacing: 1.0,
            line_height: 30,
            first: 32,
            glyphs: vec![g(4, 2, 20, 0.5), g(10, 20, 4, 0.0), g(6, 20, 4, 0.2)],
            lookup,
        };
        f.lookup.push(2); // 'B' (66)
        f
    }

    #[test]
    fn width_is_the_sum_of_advances() {
        let f = font();
        // cap height is 20 (A); size 20 -> scale 1: A = 10 + 1, B = 6 + 1, space = 4 + 1
        let l = layout_text(Some(&f), "AB A", 20.0, TextAlign::Left, None);
        assert_eq!(l.lines, 1);
        assert_eq!(l.width, 11.0 + 7.0 + 5.0 + 11.0);
        // spaces are advanced but not drawn
        assert_eq!(l.glyphs.len(), 3);
        assert_eq!(l.glyphs[1].x, 11.0);
        assert_eq!(l.glyphs[2].x, 11.0 + 7.0 + 5.0);
        // scaled with the size: twice as big, twice as wide
        let l2 = layout_text(Some(&f), "AB A", 40.0, TextAlign::Left, None);
        assert_eq!(l2.width, l.width * 2.0);
        assert_eq!(l2.glyphs[0].w, 20.0);
        // matches the font's own measure
        assert_eq!(l.width, f.line_width("AB A") as f32);
    }

    #[test]
    fn glyphs_sit_at_their_offset_below_the_caps() {
        let f = font();
        // the cap top is A's y offset (4), so A sits at the text's top
        let l = layout_text(Some(&f), "A", 20.0, TextAlign::Left, None);
        assert_eq!(l.glyphs[0].y, 0.0);
        assert_eq!(l.glyphs[0].rect, Some(f.rect_px(f.glyph('A').unwrap())));
        // second line: one line height (30) lower
        let l = layout_text(Some(&f), "A\nA", 20.0, TextAlign::Left, None);
        assert_eq!(l.lines, 2);
        assert_eq!(l.glyphs[1].y, 30.0);
    }

    #[test]
    fn alignment_inside_the_widest_line() {
        let f = font();
        let wide = 11.0 + 5.0 + 11.0; // "A A"
        let left = layout_text(Some(&f), "A A\nB", 20.0, TextAlign::Left, None);
        let centre = layout_text(Some(&f), "A A\nB", 20.0, TextAlign::Center, None);
        let right = layout_text(Some(&f), "A A\nB", 20.0, TextAlign::Right, None);
        assert_eq!(left.width, wide);
        let b = |l: &TextBlock| l.glyphs.iter().find(|g| g.ch == 'B').unwrap().x;
        assert_eq!(b(&left), 0.0);
        assert_eq!(b(&centre), (wide - 7.0) / 2.0);
        assert_eq!(b(&right), wide - 7.0);
        assert_eq!(right.line_widths, vec![wide, 7.0]);
    }

    #[test]
    fn wrapping_breaks_at_spaces() {
        let f = font();
        // each "AB" is 18 wide, a space 5: two words need 41
        let one = layout_text(Some(&f), "AB AB", 20.0, TextAlign::Left, Some(41.0));
        assert_eq!(one.lines, 1);
        let two = layout_text(Some(&f), "AB AB", 20.0, TextAlign::Left, Some(40.0));
        assert_eq!(two.lines, 2);
        assert_eq!(two.width, 18.0);
        // three words, room for two per line
        let three = layout_text(Some(&f), "AB AB AB", 20.0, TextAlign::Left, Some(41.0));
        assert_eq!(three.lines, 2);
        assert_eq!(three.line_widths, vec![41.0, 18.0]);
        // a word wider than the limit stays whole on its own line
        let long = layout_text(Some(&f), "ABABAB", 20.0, TextAlign::Left, Some(10.0));
        assert_eq!(long.lines, 1);
        assert_eq!(long.glyphs.len(), 6);
    }

    #[test]
    fn missing_characters_fall_back_to_the_default_font() {
        let f = font();
        let l = layout_text(Some(&f), "AzA", 20.0, TextAlign::Left, None);
        assert_eq!(l.glyphs.len(), 3);
        assert!(l.glyphs[0].rect.is_some());
        assert!(l.glyphs[1].rect.is_none());
        assert!(l.glyphs[2].rect.is_some());
        // the stand-in has a width and the next glyph is after it
        assert!(l.glyphs[1].w > 0.0);
        assert_eq!(l.glyphs[2].x, 11.0 + l.glyphs[1].w);
        // without any font everything falls back
        let none = layout_text(None, "Hi", 20.0, TextAlign::Left, None);
        assert_eq!(none.glyphs.len(), 2);
        assert!(none.glyphs.iter().all(|g| g.rect.is_none()));
    }

    #[test]
    fn empty_text_is_empty() {
        let f = font();
        let l = layout_text(Some(&f), "", 20.0, TextAlign::Center, None);
        assert_eq!((l.width, l.glyphs.len(), l.lines), (0.0, 0, 1));
    }
}
