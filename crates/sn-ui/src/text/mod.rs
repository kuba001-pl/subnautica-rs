//! TextMesh Pro's text layout (M9u4c, `docs/formats/ui.md` § Text layout).
//!
//! What `TextMeshProUGUI` does to place a text, ported from the game's
//! `Unity.TextMeshPro.dll` (read decompiled, the code is ours):
//! - [`input`]: the text to TMP's processing array (escapes for texts
//!   typed in the inspector, `<br>`, `<nbsp>`, `<zwsp>`), and rich-text
//!   tags (`ValidateHtmlTag`: `<b>`, `<i>`, `<size>`, `<color>`);
//! - [`FontSet::lookup`]: which font asset draws a character
//!   (`GetTextElement`: the font's weight table, its fallbacks, the
//!   settings' fallbacks and default font);
//! - [`generate`]: `GenerateTextMesh`'s layout (line breaking with saved
//!   and restored states, overflow, auto-size, alignment);
//! - [`preferred`]: `CalculatePreferredValues` (a text's preferred size
//!   as a layout element).
//!
//! Not ported (logged by the callers where met): sprites, monospace,
//! right to left, pages, linked texts, ellipsis, the CJK line-breaking
//! rules, underline and strikethrough meshes, the other rich-text tags.

pub mod generate;
pub mod input;
pub mod mesh;
pub mod preferred;
pub mod sprite;

use std::cell::RefCell;
use std::collections::HashMap;

use crate::font::{FaceInfo, FontFile, GlyphMetrics};

pub use generate::{CharInfo, LineInfo, TextLayout, layout_text, layout_text_with};
pub use input::{Source, SpriteAssetValue, SpriteTag, Tag, parse_tag, processing_array};
pub use mesh::{AtlasGlyph, MeshOptions, TextMesh, text_meshes, text_meshes_with};
pub use preferred::preferred_values;

/// `FontStyles` bits.
pub mod style {
    pub const BOLD: u32 = 1;
    pub const ITALIC: u32 = 2;
    pub const UNDERLINE: u32 = 4;
    pub const LOWER_CASE: u32 = 8;
    pub const UPPER_CASE: u32 = 16;
    pub const SMALL_CAPS: u32 = 32;
    pub const STRIKETHROUGH: u32 = 64;
}

/// A TMP font asset as the layout reads it.
pub struct TmpFont<'a> {
    pub name: String,
    /// The stored `m_FaceInfo` (TMP uses it, not the font file's).
    pub face: FaceInfo,
    pub file: FontFile<'a>,
    /// `m_CreationSettings.pointSize`: glyph metrics are at this size.
    pub sampling_point_size: i32,
    pub bold_spacing: f32,
    pub bold_style: f32,
    pub normal_style: f32,
    pub normal_spacing_offset: f32,
    pub italic_style: f32,
    pub tab_size: f32,
    /// `m_FallbackFontAssetTable`, as indices into the set.
    pub fallbacks: Vec<usize>,
    /// `m_FontWeightTable`: per weight / 100, (regular, italic) faces.
    pub weights: [[Option<usize>; 2]; 10],
    /// Glyph pair adjustments (first glyph, second glyph) → x advance of
    /// the first, in pixels at the sampling size (`m_FontFeatureTable`;
    /// filled at runtime in the game when fonts features are on).
    pub kerning: HashMap<(u32, u32), f32>,
}

/// Every font asset and `TMP Settings`' fallbacks.
pub struct FontSet<'a> {
    pub fonts: Vec<TmpFont<'a>>,
    pub settings_fallbacks: Vec<usize>,
    pub default_font: Option<usize>,
    /// `TMP_Settings.missingGlyphCharacter` (0: U+25A1).
    pub missing_glyph: u32,
    /// The sprite assets (M9u6d1) and the default one (`TMP_Settings.
    /// defaultSpriteAsset` as the game sets it at run time).
    pub sprites: Vec<sprite::SpriteAsset>,
    pub default_sprite: Option<usize>,
    metrics: RefCell<HashMap<(usize, u32), GlyphMetrics>>,
}

/// One font's padding inputs for a text (M9u5a): the padding of the
/// material its characters draw with (`GetPaddingForMaterial`; the
/// text's own for its font, a fallback material's for a fallback font)
/// and that material's `_GradientScale` and `_ScaleRatioA` for the style
/// padding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontPad {
    pub padding: f32,
    pub gradient_scale: Option<f32>,
    pub scale_ratio_a: Option<f32>,
}

/// Each font's [`FontPad`]; fonts not listed use `default`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pads {
    pub default: FontPad,
    pub fonts: HashMap<usize, FontPad>,
}

impl Pads {
    /// The same padding for every font and no style padding.
    pub fn uniform(padding: f32) -> Pads {
        Pads {
            default: FontPad {
                padding,
                ..FontPad::default()
            },
            fonts: HashMap::new(),
        }
    }

    /// `GenerateTextMesh`'s `num6` (padding) and `num7` (style padding)
    /// for a character of font `index`: a bold character (not from a
    /// bold typeface) takes `boldStyle / 4 · gradient · ratio A` with a
    /// gradient scale, any other `normalStyle / 4 · gradient · ratio A`
    /// with both; the padding shrinks so the two stay within the
    /// gradient scale.
    pub fn of(&self, index: usize, font: &TmpFont, bold: bool) -> (f32, f32) {
        let p = self.fonts.get(&index).copied().unwrap_or(self.default);
        let mut pad = p.padding;
        let style = match (p.gradient_scale, p.scale_ratio_a) {
            (Some(g), a) if bold => Some((g, font.bold_style / 4.0 * g * a.unwrap_or(0.0))),
            (Some(g), Some(a)) => Some((g, font.normal_style / 4.0 * g * a)),
            _ => None,
        };
        match style {
            Some((g, s)) => {
                if s + pad > g {
                    pad = g - s;
                }
                (pad, s)
            }
            None => (pad, 0.0),
        }
    }
}

/// The glyph index of a synthesized (empty) character.
pub const SYNTHESIZED: u32 = u32::MAX;

/// The characters every font asset has, empty if its font lacks them
/// (`TMP_FontAsset.AddSynthesizedCharactersAndFaceMetrics`).
pub const SYNTHESIZED_CHARACTERS: [u32; 12] =
    [3, 9, 10, 11, 13, 1564, 8203, 8206, 8207, 8232, 8233, 8288];

/// A character's text element: which font and glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Element {
    pub font: usize,
    pub glyph: u32,
    pub metrics: GlyphMetrics,
    /// From a weight table's face (bold or italic typeface).
    pub alternate: bool,
    /// A sprite (M9u6d1): its sprite asset and character index; `glyph`
    /// and `metrics` are then its sprite glyph's.
    pub sprite: Option<(usize, usize)>,
}

impl<'a> FontSet<'a> {
    pub fn new(
        fonts: Vec<TmpFont<'a>>,
        settings_fallbacks: Vec<usize>,
        default_font: Option<usize>,
        missing_glyph: u32,
    ) -> FontSet<'a> {
        FontSet {
            fonts,
            settings_fallbacks,
            default_font,
            missing_glyph,
            sprites: Vec::new(),
            default_sprite: None,
            metrics: RefCell::new(HashMap::new()),
        }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.fonts.iter().position(|f| f.name == name)
    }

    /// The glyph's metrics at its font's sampling size (what TMP stores
    /// when it adds the glyph).
    pub fn metrics(&self, font: usize, glyph: u32) -> GlyphMetrics {
        if glyph == SYNTHESIZED {
            return GlyphMetrics::default();
        }
        if let Some(m) = self.metrics.borrow().get(&(font, glyph)) {
            return *m;
        }
        let f = &self.fonts[font];
        let m = f
            .file
            .glyph(glyph, f.sampling_point_size)
            .map(|g| g.metrics)
            .unwrap_or_default();
        self.metrics.borrow_mut().insert((font, glyph), m);
        m
    }

    /// In this font only (with its weight table): `GetCharacterFromFontAsset_Internal`
    /// without fallbacks. A dynamic asset has every character its font
    /// file has.
    fn in_font(&self, font: usize, c: u32, italic: bool, weight: u32) -> Option<Element> {
        let f = self.fonts.get(font)?;
        if italic || weight != 400 {
            let k = match weight {
                100 => 1,
                200 => 2,
                300 => 3,
                500 => 5,
                600 => 6,
                700 => 7,
                800 => 8,
                900 => 9,
                _ => 4,
            };
            if let Some(alt) = f.weights[k][usize::from(italic)] {
                if let Some(g) = self.glyph_of(alt, c) {
                    return Some(Element {
                        font: alt,
                        glyph: g,
                        metrics: self.metrics(alt, g),
                        alternate: true,
                        sprite: None,
                    });
                }
            }
        }
        let g = self.glyph_of(font, c)?;
        Some(Element {
            font,
            glyph: g,
            metrics: self.metrics(font, g),
            alternate: false,
            sprite: None,
        })
    }

    fn glyph_of(&self, font: usize, c: u32) -> Option<u32> {
        let file = &self.fonts.get(font)?.file;
        let real = char::from_u32(c)
            .and_then(|ch| file.glyph_index(ch))
            .filter(|&g| g != 0);
        // `AddSynthesizedCharactersAndFaceMetrics`: control characters a
        // font has no glyph for get an empty one.
        real.or(SYNTHESIZED_CHARACTERS.contains(&c).then_some(SYNTHESIZED))
    }

    /// With fallbacks, depth first, each asset searched once.
    fn with_fallbacks(
        &self,
        font: usize,
        c: u32,
        italic: bool,
        weight: u32,
        seen: &mut Vec<usize>,
    ) -> Option<Element> {
        if let Some(e) = self.in_font(font, c, italic, weight) {
            return Some(e);
        }
        for &fb in &self.fonts.get(font)?.fallbacks {
            if seen.contains(&fb) {
                continue;
            }
            seen.push(fb);
            if let Some(e) = self.with_fallbacks(fb, c, italic, weight, seen) {
                return Some(e);
            }
        }
        None
    }

    /// `TMP_Text.GetTextElement` for a text whose font asset is `primary`
    /// and current font `current` (sprite assets are not searched: none
    /// of the game's sprite characters are in the texts' ranges).
    pub fn lookup(
        &self,
        c: u32,
        current: usize,
        primary: usize,
        style: u32,
        weight: u32,
    ) -> Option<Element> {
        let italic = style & style::ITALIC != 0;
        if let Some(e) = self.in_font(current, c, italic, weight) {
            return Some(e);
        }
        let mut seen = vec![current];
        for &fb in &self.fonts.get(current)?.fallbacks {
            if !seen.contains(&fb) {
                seen.push(fb);
                if let Some(e) = self.with_fallbacks(fb, c, italic, weight, &mut seen) {
                    return Some(e);
                }
            }
        }
        if current != primary {
            if let Some(e) = self.in_font(primary, c, italic, weight) {
                return Some(e);
            }
            for &fb in &self.fonts.get(primary)?.fallbacks {
                if !seen.contains(&fb) {
                    seen.push(fb);
                    if let Some(e) = self.with_fallbacks(fb, c, italic, weight, &mut seen) {
                        return Some(e);
                    }
                }
            }
        }
        for &fb in &self.settings_fallbacks {
            if !seen.contains(&fb) {
                seen.push(fb);
                if let Some(e) = self.with_fallbacks(fb, c, italic, weight, &mut seen) {
                    return Some(e);
                }
            }
        }
        if let Some(d) = self.default_font {
            let mut seen = Vec::new();
            return self.with_fallbacks(d, c, italic, weight, &mut seen);
        }
        None
    }
}

/// A text object's settings (`TextMeshProUGUI`'s serialized fields that
/// the layout reads).
#[derive(Clone, Debug, PartialEq)]
pub struct TextParams {
    pub text: String,
    pub source: Source,
    /// `m_fontAsset`, index into the set.
    pub font: usize,
    /// `m_fontSize` (without auto-size), `m_fontSizeBase` (with).
    pub font_size: f32,
    pub font_size_base: f32,
    pub auto_size: bool,
    pub font_size_min: f32,
    pub font_size_max: f32,
    pub style: u32,
    pub weight: u32,
    /// `HorizontalAlignmentOptions`: 1 left, 2 centre, 4 right, 8
    /// justified, 16 flush, 32 geometry.
    pub h_align: u32,
    /// `VerticalAlignmentOptions`: 256 top, 512 middle, 1024 bottom, 2048
    /// baseline, 4096 geometry, 8192 capline.
    pub v_align: u32,
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub line_spacing_max: f32,
    pub paragraph_spacing: f32,
    pub char_width_max_adj: f32,
    pub word_wrap: bool,
    pub wrapping_ratios: f32,
    /// 0 overflow, 1 ellipsis, 2 masking, 3 truncate, 4 scroll rect, 5
    /// page, 6 linked.
    pub overflow: i32,
    pub kerning: bool,
    pub rich_text: bool,
    pub parse_ctrl_characters: bool,
    /// Left, top, right, bottom.
    pub margin: [f32; 4],
    pub color: [u8; 4],
}

impl TextParams {
    /// A plain text with TMP's defaults.
    pub fn new(text: &str, font: usize, font_size: f32) -> TextParams {
        TextParams {
            text: text.into(),
            source: Source::Code,
            font,
            font_size,
            font_size_base: font_size,
            auto_size: false,
            font_size_min: 18.0,
            font_size_max: 72.0,
            style: 0,
            weight: 400,
            h_align: 1,
            v_align: 256,
            character_spacing: 0.0,
            word_spacing: 0.0,
            line_spacing: 0.0,
            line_spacing_max: 0.0,
            paragraph_spacing: 0.0,
            char_width_max_adj: 0.0,
            word_wrap: true,
            wrapping_ratios: 0.4,
            overflow: 0,
            kerning: true,
            rich_text: true,
            parse_ctrl_characters: true,
            margin: [0.0; 4],
            color: [255; 4],
        }
    }
}

/// The texts of a UI tree as the layout system sizes them: node → its
/// settings (`TMP_Text`'s `preferredWidth` and `preferredHeight`).
pub struct Texts<'s, 'f> {
    pub fonts: &'s FontSet<'f>,
    pub params: HashMap<usize, TextParams>,
}

impl crate::layout::TextSizer for Texts<'_, '_> {
    fn preferred(&self, node: usize, axis: usize, rect_width: f32) -> f32 {
        let Some(p) = self.params.get(&node) else {
            return f32::NAN;
        };
        // `ComputeMarginSize`: the rect's width less the margins.
        let margin_width = rect_width - p.margin[0] - p.margin[2];
        preferred_values(self.fonts, p, margin_width)[axis.min(1)]
    }
}

#[cfg(test)]
mod tests;
