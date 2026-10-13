//! `TextMeshProUGUI.GenerateTextMesh`'s layout and the auto-size loop of
//! `OnPreRenderCanvas`, ported from the decompiled game code (our code,
//! the same control flow: the character loop with its saved and restored
//! word-wrap states, the overflow and auto-size branches, the line
//! finishing and the alignment pass).
//!
//! Units are TMP's: the rect's own space (pivot at the origin), pixels of
//! the canvas. Positions are each character's quad before the mesh step
//! adds the material padding ([`Pads`]: none for layout; the materials'
//! when the text mesh is built, M9u5a).

use super::input::{Tag, Unit, parse_tag, processing_array};
use super::{Element, FontSet, Pads, SYNTHESIZED, TextParams, style};
use crate::font::GlyphMetrics;
use crate::layout::{Rect, V2};

const LARGE_POSITIVE: f32 = 32767.0;
const LARGE_NEGATIVE: f32 = -32767.0;
/// `m_AutoSizeMaxIterationCount`.
const MAX_ITERATIONS: u32 = 100;

/// C#'s `char.IsSeparator` (categories Zs, Zl, Zp).
pub fn is_separator(c: u32) -> bool {
    matches!(
        c,
        0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000
    )
}

/// C#'s `char.IsWhiteSpace` for a UTF-16 unit.
pub fn is_white_space(c: u32) -> bool {
    c <= 0xFFFF && char::from_u32(c).is_some_and(char::is_whitespace)
}

/// A character after SetArraySizes: what it is and which glyph draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prepared {
    pub unicode: u32,
    pub element: Element,
}

/// One laid-out character (`TMP_CharacterInfo`, the fields kept).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharInfo {
    pub unicode: u32,
    pub font: usize,
    pub glyph: u32,
    pub alternate: bool,
    pub point_size: f32,
    pub scale: f32,
    pub style: u32,
    pub color: [u8; 4],
    pub bottom_left: V2,
    pub top_left: V2,
    pub top_right: V2,
    pub bottom_right: V2,
    pub origin: f32,
    pub x_advance: f32,
    pub ascender: f32,
    pub descender: f32,
    pub adjusted_ascender: f32,
    pub adjusted_descender: f32,
    pub baseline: f32,
    pub visible: bool,
    pub line: usize,
    /// The material padding and style padding its quad was made with
    /// (`num6`, `num7` in `GenerateTextMesh`; the UVs reach as far).
    pub padding: f32,
    pub style_padding: f32,
}

/// One line (`TMP_LineInfo`, the fields kept).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineInfo {
    pub first: usize,
    pub last: usize,
    pub first_visible: usize,
    pub last_visible: usize,
    pub character_count: usize,
    pub visible_count: usize,
    pub space_count: usize,
    pub control_count: usize,
    pub max_advance: f32,
    pub width: f32,
    pub ascender: f32,
    pub descender: f32,
    pub baseline: f32,
    pub line_height: f32,
    pub alignment: u32,
    pub margin_left: f32,
    pub margin_right: f32,
}

impl Default for LineInfo {
    fn default() -> LineInfo {
        LineInfo {
            first: 0,
            last: 0,
            first_visible: 0,
            last_visible: 0,
            character_count: 0,
            visible_count: 0,
            space_count: 0,
            control_count: 0,
            max_advance: 0.0,
            width: 0.0,
            ascender: LARGE_NEGATIVE,
            descender: LARGE_POSITIVE,
            baseline: 0.0,
            line_height: 0.0,
            alignment: 0,
            margin_left: 0.0,
            margin_right: 0.0,
        }
    }
}

/// The result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    /// The point size used (auto-size's result).
    pub font_size: f32,
    pub iterations: u32,
    pub characters: Vec<CharInfo>,
    pub lines: Vec<LineInfo>,
    pub truncated: bool,
    pub first_overflow: Option<usize>,
    /// `m_charWidthAdjDelta` of the last pass.
    pub char_width_adj: f32,
    /// Rich-text tags and options met that are not ported.
    pub unported: Vec<String>,
}

/// TMP's processing stacks (`TMP_TextProcessingStack`): a default and
/// what was added.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stack<T: Copy> {
    items: Vec<T>,
}

impl<T: Copy> Stack<T> {
    pub(crate) fn new(default: T) -> Stack<T> {
        Stack {
            items: vec![default],
        }
    }

    pub(crate) fn add(&mut self, v: T) {
        self.items.push(v);
    }

    /// Removes the last and gives the new current (the default when
    /// empty).
    pub(crate) fn remove(&mut self) -> T {
        if self.items.len() > 1 {
            self.items.pop();
        }
        *self.items.last().unwrap_or(&self.items[0])
    }

    pub(crate) fn peek(&self) -> T {
        *self.items.last().unwrap_or(&self.items[0])
    }
}

/// The styling state tags change (and the word-wrap states save).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Styling {
    pub style: u32,
    pub weight: u32,
    pub bold_count: u8,
    pub italic_count: u8,
    pub italic_angle: i32,
    pub italic_stack: Stack<i32>,
    pub size: f32,
    pub size_stack: Stack<f32>,
    pub weight_stack: Stack<u32>,
    pub color: [u8; 4],
    pub color_stack: Stack<[u8; 4]>,
}

impl Styling {
    pub(crate) fn new(p: &TextParams, font_size: f32, italic_style: i32) -> Styling {
        let weight = if p.style & style::BOLD != 0 {
            700
        } else {
            p.weight
        };
        Styling {
            style: p.style,
            weight,
            bold_count: 0,
            italic_count: 0,
            italic_angle: italic_style,
            italic_stack: Stack::new(italic_style),
            size: font_size,
            size_stack: Stack::new(font_size),
            weight_stack: Stack::new(weight),
            color: p.color,
            color_stack: Stack::new(p.color),
        }
    }

    /// `ValidateHtmlTag`'s effect for the tags ported. `base_size` is the
    /// pass's `m_fontSize`, `fixed_style` the object's own style.
    pub(crate) fn apply(&mut self, tag: &Tag, base_size: f32, fixed_style: u32, italic_style: i32) {
        match tag {
            Tag::Bold => {
                self.style |= style::BOLD;
                self.bold_count = self.bold_count.saturating_add(1);
                self.weight = 700;
            }
            Tag::EndBold => {
                if fixed_style & style::BOLD == 0 {
                    self.bold_count = self.bold_count.saturating_sub(1);
                    if self.bold_count == 0 {
                        self.style &= !style::BOLD;
                        self.weight = self.weight_stack.peek();
                    }
                }
            }
            Tag::Italic(angle) => {
                self.style |= style::ITALIC;
                self.italic_count = self.italic_count.saturating_add(1);
                self.italic_angle = angle.unwrap_or(italic_style);
                self.italic_stack.add(self.italic_angle);
            }
            Tag::EndItalic => {
                if fixed_style & style::ITALIC == 0 {
                    self.italic_angle = self.italic_stack.remove();
                    self.italic_count = self.italic_count.saturating_sub(1);
                    if self.italic_count == 0 {
                        self.style &= !style::ITALIC;
                    }
                }
            }
            Tag::Size(v, unit, relative) => {
                self.size = match unit {
                    Unit::Pixels if *relative => base_size + v,
                    Unit::Pixels => *v,
                    Unit::FontUnits => base_size * v,
                    Unit::Percentage => base_size * v / 100.0,
                };
                self.size_stack.add(self.size);
            }
            Tag::EndSize => self.size = self.size_stack.remove(),
            Tag::Color(c) => {
                self.color = *c;
                self.color_stack.add(*c);
            }
            Tag::EndColor => self.color = self.color_stack.remove(),
            Tag::Unported(_) => {}
        }
    }
}

/// `SetArraySizes`: every character of the processing array (tags taken
/// out) with its element, looked up with the style its tags give.
pub fn prepare(
    fonts: &FontSet,
    p: &TextParams,
    chars: &[u32],
    unported: &mut Vec<String>,
) -> Vec<Prepared> {
    let font = &fonts.fonts[p.font];
    let italic_style = font.italic_style as i32;
    let mut st = Styling::new(p, p.font_size, italic_style);
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() && chars[i] != 0 {
        let mut c = chars[i];
        if p.rich_text && c == u32::from('<') {
            if let Some((tag, end)) = parse_tag(chars, i + 1) {
                if let Tag::Unported(n) = &tag {
                    unported.push(format!("<{n}>"));
                }
                st.apply(&tag, p.font_size, p.style, italic_style);
                i = end + 1;
                continue;
            }
        }
        c = case_transform(c, st.style).0;
        let element = fonts
            .lookup(c, p.font, p.font, st.style, st.weight)
            .or_else(|| {
                c = if fonts.missing_glyph == 0 {
                    9633
                } else {
                    fonts.missing_glyph
                };
                fonts.lookup(c, p.font, p.font, st.style, st.weight)
            })
            .or_else(|| {
                c = 32;
                fonts.lookup(c, p.font, p.font, st.style, st.weight)
            })
            .unwrap_or_else(|| {
                c = 3;
                Element {
                    font: p.font,
                    glyph: SYNTHESIZED,
                    metrics: GlyphMetrics::default(),
                    alternate: false,
                }
            });
        out.push(Prepared {
            unicode: c,
            element,
        });
        i += 1;
    }
    out
}

/// Upper case, lower case, small caps: the character and its size factor.
pub(crate) fn case_transform(c: u32, st: u32) -> (u32, f32) {
    let Some(ch) = char::from_u32(c) else {
        return (c, 1.0);
    };
    if st & style::UPPER_CASE != 0 {
        if ch.is_lowercase() {
            return (u32::from(ch.to_uppercase().next().unwrap_or(ch)), 1.0);
        }
    } else if st & style::LOWER_CASE != 0 {
        if ch.is_uppercase() {
            return (u32::from(ch.to_lowercase().next().unwrap_or(ch)), 1.0);
        }
    } else if st & style::SMALL_CAPS != 0 && ch.is_lowercase() {
        return (u32::from(ch.to_uppercase().next().unwrap_or(ch)), 0.8);
    }
    (c, 1.0)
}

/// A word-wrap state (`WordWrapState`): what `SaveWordWrappingState`
/// keeps.
#[derive(Clone, Debug, PartialEq)]
struct Saved {
    previous_word_break: isize,
    total_count: isize,
    line_visible_count: usize,
    first_of_line: usize,
    first_visible_of_line: usize,
    last_visible_of_line: usize,
    styling: Styling,
    x_advance: f32,
    max_cap_height: f32,
    max_text_ascender: f32,
    element_descender: f32,
    start_of_line_ascender: f32,
    max_line_ascender: f32,
    max_line_descender: f32,
    page_ascender: f32,
    extents: [f32; 4],
    line_number: usize,
    line_offset: f32,
    driven_line_spacing: bool,
    glyph_adjustment: f32,
    margin_left: f32,
    margin_right: f32,
    non_breaking_space: bool,
    line_info: Option<LineInfo>,
}

/// The pass state of `GenerateTextMesh`.
struct Gen<'f, 'p> {
    fonts: &'f FontSet<'f>,
    p: &'p TextParams,
    chars: &'p [u32],
    prepared: &'p [Prepared],
    rect: Rect,
    pads: &'p Pads,
    // Across passes (auto-size).
    font_size: f32,
    min_font_size: f32,
    max_font_size: f32,
    line_spacing_delta: f32,
    char_width_adj: f32,
    iteration: u32,
    // Within a pass.
    info: Vec<CharInfo>,
    lines: Vec<LineInfo>,
    st: Styling,
    count: usize,
    x_advance: f32,
    line_offset: f32,
    first_of_line: usize,
    first_visible_of_line: usize,
    last_visible_of_line: usize,
    last_of_line: usize,
    max_line_ascender: f32,
    max_line_descender: f32,
    line_number: usize,
    start_of_line_ascender: f32,
    line_visible_count: usize,
    driven_line_spacing: bool,
    first_overflow: Option<usize>,
    margin_left: f32,
    margin_right: f32,
    max_cap_height: f32,
    max_text_ascender: f32,
    element_descender: f32,
    page_ascender: f32,
    extents: [f32; 4],
    glyph_adjustment: f32,
    non_breaking_space: bool,
    truncated: bool,
}

/// What a pass ended with.
enum Pass {
    /// Auto-size changed something: run again.
    Again,
    Done,
}

impl Gen<'_, '_> {
    fn save(&self, index: isize, count: isize) -> Saved {
        Saved {
            previous_word_break: index,
            total_count: count,
            line_visible_count: self.line_visible_count,
            first_of_line: self.first_of_line,
            first_visible_of_line: self.first_visible_of_line,
            last_visible_of_line: self.last_visible_of_line,
            styling: self.st.clone(),
            x_advance: self.x_advance,
            max_cap_height: self.max_cap_height,
            max_text_ascender: self.max_text_ascender,
            element_descender: self.element_descender,
            start_of_line_ascender: self.start_of_line_ascender,
            max_line_ascender: self.max_line_ascender,
            max_line_descender: self.max_line_descender,
            page_ascender: self.page_ascender,
            extents: self.extents,
            line_number: self.line_number,
            line_offset: self.line_offset,
            driven_line_spacing: self.driven_line_spacing,
            glyph_adjustment: self.glyph_adjustment,
            margin_left: self.margin_left,
            margin_right: self.margin_right,
            non_breaking_space: self.non_breaking_space,
            line_info: self.lines.get(self.line_number).copied(),
        }
    }

    /// Returns the index to continue after.
    fn restore(&mut self, s: &Saved) -> isize {
        self.count = (s.total_count + 1).max(0) as usize;
        self.line_visible_count = s.line_visible_count;
        self.first_of_line = s.first_of_line;
        self.first_visible_of_line = s.first_visible_of_line;
        self.last_visible_of_line = s.last_visible_of_line;
        self.st = s.styling.clone();
        self.x_advance = s.x_advance;
        self.max_cap_height = s.max_cap_height;
        self.max_text_ascender = s.max_text_ascender;
        self.element_descender = s.element_descender;
        self.start_of_line_ascender = s.start_of_line_ascender;
        self.max_line_ascender = s.max_line_ascender;
        self.max_line_descender = s.max_line_descender;
        self.page_ascender = s.page_ascender;
        self.extents = s.extents;
        self.line_number = s.line_number;
        self.line_offset = s.line_offset;
        self.driven_line_spacing = s.driven_line_spacing;
        self.glyph_adjustment = s.glyph_adjustment;
        self.margin_left = s.margin_left;
        self.margin_right = s.margin_right;
        self.non_breaking_space = s.non_breaking_space;
        if let Some(li) = s.line_info {
            self.line(s.line_number);
            self.lines[s.line_number] = li;
        }
        s.previous_word_break
    }

    fn line(&mut self, n: usize) -> &mut LineInfo {
        while self.lines.len() <= n {
            self.lines.push(LineInfo::default());
        }
        &mut self.lines[n]
    }

    fn ch(&mut self, n: usize) -> &mut CharInfo {
        while self.info.len() <= n {
            self.info.push(CharInfo::default());
        }
        &mut self.info[n]
    }

    fn get(&self, n: usize) -> CharInfo {
        self.info.get(n).copied().unwrap_or_default()
    }

    fn adjust_line_offset(&mut self, from: usize, to: usize, offset: f32) {
        for k in from..=to.min(self.info.len().saturating_sub(1)) {
            let c = &mut self.info[k];
            for v in [
                &mut c.bottom_left,
                &mut c.top_left,
                &mut c.top_right,
                &mut c.bottom_right,
            ] {
                v[1] -= offset;
            }
            c.ascender -= offset;
            c.baseline -= offset;
            c.descender -= offset;
        }
    }

    /// Auto-size's step down: the next point size between the minimum
    /// tried and this one.
    fn shrink(&mut self) {
        self.max_font_size = self.font_size;
        let step = ((self.font_size - self.min_font_size) / 2.0).max(0.05);
        self.font_size -= step;
        self.font_size =
            (((self.font_size * 20.0 + 0.5) as i32) as f32 / 20.0).max(self.p.font_size_min);
    }

    /// The character width adjustment (`m_charWidthMaxAdj`), as TMP
    /// applies it before shrinking.
    fn adjust_width(&mut self, needed: f32, available: f32, justified: bool) {
        let mut total = needed;
        if self.char_width_adj > 0.0 {
            total /= 1.0 - self.char_width_adj;
        }
        let over = needed - (available - 0.0001) * if justified { 1.05 } else { 1.0 };
        self.char_width_adj += over / total;
        self.char_width_adj = self.char_width_adj.min(self.p.char_width_max_adj / 100.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_new_line(
        &mut self,
        i: isize,
        base_scale: f32,
        element_scale: f32,
        em_scale: f32,
        glyph_adjustment: f32,
        bold_spacing: f32,
        char_spacing: f32,
        width: f32,
        line_gap: f32,
        max_visible_descender: &mut (bool, f32),
    ) {
        let delta = self.max_line_ascender - self.start_of_line_ascender;
        if self.line_offset > 0.0 && delta.abs() > 0.01 && !self.driven_line_spacing {
            self.adjust_line_offset(self.first_of_line, self.count, delta);
            self.element_descender -= delta;
            self.line_offset += delta;
        }
        let asc = self.max_line_ascender - self.line_offset;
        let desc = self.max_line_descender - self.line_offset;
        self.element_descender = self.element_descender.min(desc);
        if !max_visible_descender.0 {
            max_visible_descender.1 = self.element_descender;
        }
        let last = self.count.saturating_sub(1);
        self.first_visible_of_line = self.first_visible_of_line.max(self.first_of_line);
        self.last_of_line = last;
        self.last_visible_of_line = self.last_visible_of_line.max(self.first_visible_of_line);
        let font = &self.fonts.fonts[self.get(self.count).font.min(self.fonts.fonts.len() - 1)];
        let trailing = (glyph_adjustment * element_scale
            + (font.normal_spacing_offset + char_spacing + bold_spacing) * em_scale)
            * (1.0 - self.char_width_adj);
        let max_advance = self.get(self.last_visible_of_line).x_advance - trailing;
        let (first, fv, lv, lvc, n, offset) = (
            self.first_of_line,
            self.first_visible_of_line,
            self.last_visible_of_line,
            self.line_visible_count,
            self.line_number,
            self.line_offset,
        );
        let li = self.line(n);
        li.first = first;
        li.first_visible = fv;
        li.last = last;
        li.last_visible = lv;
        li.character_count = last + 1 - first.min(last + 1);
        li.visible_count = lvc;
        li.width = width;
        li.max_advance = max_advance;
        li.baseline = -offset;
        li.ascender = asc;
        li.descender = desc;
        li.line_height = asc - desc + line_gap * base_scale;
        self.ch(last).x_advance = max_advance;
        self.first_of_line = self.count;
        self.line_visible_count = 0;
        self.line_number += 1;
        self.line(self.line_number);
        let adjusted = self.get(self.count).adjusted_ascender;
        self.line_offset += -self.max_line_descender
            + adjusted
            + (line_gap + self.line_spacing_delta) * base_scale
            + self.p.line_spacing * em_scale;
        self.start_of_line_ascender = adjusted;
        self.max_line_ascender = LARGE_NEGATIVE;
        self.max_line_descender = LARGE_POSITIVE;
        self.x_advance = 0.0;
        let _ = i;
    }

    fn reset(&mut self) {
        let p = self.p;
        let font = &self.fonts.fonts[p.font];
        self.info.clear();
        self.lines.clear();
        self.lines.push(LineInfo::default());
        self.st = Styling::new(p, self.font_size, font.italic_style as i32);
        self.count = 0;
        self.x_advance = 0.0;
        self.line_offset = 0.0;
        self.first_of_line = 0;
        self.first_visible_of_line = 0;
        self.last_visible_of_line = 0;
        self.last_of_line = 0;
        self.max_line_ascender = LARGE_NEGATIVE;
        self.max_line_descender = LARGE_POSITIVE;
        self.line_number = 0;
        self.start_of_line_ascender = 0.0;
        self.line_visible_count = 0;
        self.driven_line_spacing = false;
        self.first_overflow = None;
        self.margin_left = 0.0;
        self.margin_right = 0.0;
        self.max_cap_height = 0.0;
        self.max_text_ascender = 0.0;
        self.element_descender = 0.0;
        self.page_ascender = 0.0;
        self.extents = [
            LARGE_POSITIVE,
            LARGE_POSITIVE,
            LARGE_NEGATIVE,
            LARGE_NEGATIVE,
        ];
        self.glyph_adjustment = 0.0;
        self.non_breaking_space = false;
        self.truncated = false;
    }

    #[allow(clippy::too_many_lines)]
    fn pass(&mut self) -> Pass {
        self.reset();
        let p = self.p;
        let fonts = self.fonts;
        let primary = &fonts.fonts[p.font];
        let total = self.prepared.len();
        let italic_style = primary.italic_style as i32;
        let base_scale = self.font_size / primary.face.point_size as f32 * primary.face.scale;
        let mut element_scale = base_scale;
        let em_scale = self.font_size * 0.01;
        let line_gap =
            primary.face.line_height - (primary.face.ascent_line - primary.face.descent_line);
        let margin = p.margin;
        let margin_width = (self.rect.width - margin[0] - margin[2]).max(0.0);
        let margin_height = (self.rect.height - margin[1] - margin[3]).max(0.0);
        let mut width = margin_width + 0.0001 - self.margin_left - self.margin_right;
        let mut max_visible_descender = (false, 0.0f32);
        let mut first_word_of_line = true; // flag5
        let mut first_visible_pending = true; // flag4
        let mut soft_hyphen = false; // flag7
        let mut last_soft_break: isize = 0; // num14
        let mut subst: (isize, u32) = (-1, 0);
        let mut word_wrap = self.save(-1, -1);
        let mut last_valid = self.save(-1, -1);
        let mut soft_break = self.save(-1, -1);
        let mut char_spacing;
        let mut bold_spacing;
        let mut unicode: u32 = 0;
        let mut i: isize = 0;
        let len = self.chars.len() as isize;
        macro_rules! next {
            () => {{
                i += 1;
                continue;
            }};
        }
        while i < len && self.chars[i as usize] != 0 {
            unicode = self.chars[i as usize];
            if p.rich_text && unicode == u32::from('<') {
                if let Some((tag, end)) = parse_tag(self.chars, i as usize + 1) {
                    self.st.apply(&tag, self.font_size, p.style, italic_style);
                    i = end as isize;
                    next!();
                }
            }
            let Some(prep) = self.prepared.get(self.count).copied() else {
                break;
            };
            let mut font_i = prep.element.font;
            let mut element = prep.element;
            let mut substituted = false;
            if subst.0 == self.count as isize {
                unicode = subst.1;
                substituted = true;
                if unicode == 3 {
                    element = Element {
                        font: font_i,
                        glyph: SYNTHESIZED,
                        metrics: GlyphMetrics::default(),
                        alternate: false,
                    };
                    self.truncated = true;
                }
            }
            let (u, small_caps) = case_transform(unicode, self.st.style);
            unicode = u;
            if !substituted {
                unicode = prep.unicode;
            }
            font_i = font_i.min(fonts.fonts.len() - 1);
            let font = &fonts.fonts[font_i];
            let size_for_scale = if substituted
                && self.chars[i as usize] == 10
                && self.count != self.first_of_line
            {
                self.get(self.count - 1).point_size
            } else {
                self.st.size
            };
            let char_scale =
                size_for_scale * small_caps / font.face.point_size as f32 * font.face.scale;
            let (asc_line, desc_line) = if substituted && unicode == 8230 {
                (0.0, 0.0)
            } else {
                (font.face.ascent_line, font.face.descent_line)
            };
            element_scale = char_scale;
            let baseline_y = font.face.baseline * char_scale * font.face.scale;
            let unscaled = element_scale;
            if unicode == 173 || unicode == 3 {
                element_scale = 0.0;
            }
            let cc = self.count;
            let point_size = self.st.size;
            let (st_style, st_color) = (self.st.style, self.st.color);
            {
                let c = self.ch(cc);
                c.unicode = unicode;
                c.font = font_i;
                c.glyph = element.glyph;
                c.alternate = element.alternate;
                c.scale = element_scale;
                c.point_size = point_size;
                c.color = st_color;
                c.style = st_style;
            }
            let metrics = element.metrics;
            let white = is_white_space(unicode);
            // Kerning (the pair with the next character; the second
            // adjustments of a 'kern'-table pair are 0).
            let mut adv_adjust = 0.0;
            char_spacing = p.character_spacing;
            self.glyph_adjustment = 0.0;
            if p.kerning && cc + 1 < total {
                let next = self.prepared[cc + 1].element.glyph;
                if let Some(k) = font.kerning.get(&(element.glyph, next)) {
                    adv_adjust = *k;
                }
                self.glyph_adjustment = adv_adjust;
            }
            let bold = !element.alternate && self.st.style & style::BOLD != 0;
            let (pad, style_padding) = self.pads.of(font_i, font, bold);
            bold_spacing = if bold { font.bold_spacing } else { 0.0 };
            let s = element_scale;
            let cw = 1.0 - self.char_width_adj;
            let mut tl = [
                self.x_advance + (metrics.bearing_x - pad - style_padding) * s * cw,
                baseline_y + (metrics.bearing_y + pad) * s - self.line_offset,
            ];
            let mut bl = [tl[0], tl[1] - (metrics.height + pad * 2.0) * s];
            let mut tr = [
                bl[0] + (metrics.width + pad * 2.0 + style_padding * 2.0) * s * cw,
                tl[1],
            ];
            let mut br = [tr[0], bl[1]];
            if !element.alternate && self.st.style & style::ITALIC != 0 {
                let shear = self.st.italic_angle as f32 * 0.01;
                let top = shear * ((metrics.bearing_y + pad + style_padding) * s);
                let bottom =
                    shear * ((metrics.bearing_y - metrics.height - pad - style_padding) * s);
                let mid = (top - bottom) / 2.0;
                tl[0] += top - mid;
                bl[0] += bottom - mid;
                tr[0] += top - mid;
                br[0] += bottom - mid;
            }
            let ascender = asc_line * element_scale / small_caps;
            let descender = desc_line * element_scale / small_caps;
            let first_of_line = cc == self.first_of_line;
            {
                let x_advance = self.x_advance;
                let line_offset = self.line_offset;
                let c = self.ch(cc);
                c.bottom_left = bl;
                c.top_left = tl;
                c.top_right = tr;
                c.bottom_right = br;
                c.origin = x_advance;
                c.baseline = baseline_y - line_offset;
                c.padding = pad;
                c.style_padding = style_padding;
            }
            if first_of_line || !white {
                self.max_line_ascender = self.max_line_ascender.max(ascender);
                self.max_line_descender = self.max_line_descender.min(descender);
                let lo = self.line_offset;
                let c = self.ch(cc);
                c.adjusted_ascender = ascender;
                c.adjusted_descender = descender;
                c.ascender = ascender - lo;
                c.descender = descender - lo;
                self.element_descender = descender - lo;
            } else {
                let (ma, md, lo) = (
                    self.max_line_ascender,
                    self.max_line_descender,
                    self.line_offset,
                );
                let c = self.ch(cc);
                c.adjusted_ascender = ma;
                c.adjusted_descender = md;
                c.ascender = ma - lo;
                c.descender = md - lo;
                self.element_descender = md - lo;
            }
            if self.line_number == 0 && (first_of_line || !white) {
                self.max_text_ascender = self.max_line_ascender;
                self.max_cap_height = self
                    .max_cap_height
                    .max(font.face.cap_line * element_scale / small_caps);
            }
            if self.line_offset == 0.0 && (first_of_line || !white) {
                self.page_ascender = self.page_ascender.max(ascender);
            }
            self.ch(cc).visible = false;
            let justified = p.h_align & (8 | 16) != 0;
            let mut margin_left = self.margin_left;
            let mut margin_right = self.margin_right;
            if unicode == 9
                || (!white && unicode != 8203 && unicode != 173 && unicode != 3)
                || (unicode == 173 && !soft_hyphen)
            {
                self.ch(cc).visible = true;
                if substituted {
                    let li = self.line(self.line_number);
                    margin_left = li.margin_left;
                    margin_right = li.margin_right;
                }
                width = margin_width + 0.0001 - margin_left - margin_right;
                let needed = self.x_advance.abs()
                    + metrics.advance
                        * cw
                        * if unicode == 173 {
                            unscaled
                        } else {
                            element_scale
                        };
                let height = self.max_text_ascender - (self.max_line_descender - self.line_offset)
                    + if self.line_offset > 0.0 && !self.driven_line_spacing {
                        self.max_line_ascender - self.start_of_line_ascender
                    } else {
                        0.0
                    };
                let count_here = cc;
                if height > margin_height + 0.0001 {
                    self.first_overflow.get_or_insert(cc);
                    if p.auto_size {
                        if self.line_spacing_delta > p.line_spacing_max
                            && self.line_offset > 0.0
                            && self.iteration < MAX_ITERATIONS
                        {
                            let adjust = (margin_height - height) / self.line_number as f32;
                            self.line_spacing_delta = (self.line_spacing_delta
                                + adjust / base_scale)
                                .max(p.line_spacing_max);
                            return Pass::Again;
                        }
                        if self.font_size > p.font_size_min && self.iteration < MAX_ITERATIONS {
                            self.shrink();
                            return Pass::Again;
                        }
                    }
                    if p.overflow == 3 {
                        i = self.restore(&last_valid);
                        subst = (count_here as isize, 3);
                        next!();
                    }
                }
                if needed > width * if justified { 1.05 } else { 1.0 } {
                    if p.word_wrap && cc != self.first_of_line {
                        i = self.restore(&word_wrap);
                        let adjusted = self.get(self.count).adjusted_ascender;
                        let line_height = if self.line_offset > 0.0 && !self.driven_line_spacing {
                            self.max_line_ascender - self.start_of_line_ascender
                        } else {
                            0.0
                        } - self.max_line_descender
                            + adjusted
                            + (line_gap + self.line_spacing_delta) * base_scale
                            + p.line_spacing * em_scale;
                        let next_height = self.max_text_ascender + line_height + self.line_offset
                            - self.get(self.count).adjusted_descender;
                        if self.count > 0
                            && self.get(self.count - 1).unicode == 173
                            && !soft_hyphen
                            && (p.overflow == 0 || next_height < margin_height + 0.0001)
                        {
                            subst = (self.count as isize - 1, 45);
                            i -= 1;
                            self.count -= 1;
                            next!();
                        }
                        soft_hyphen = false;
                        if self.get(self.count).unicode == 173 {
                            soft_hyphen = true;
                            next!();
                        }
                        if p.auto_size && first_word_of_line {
                            if self.char_width_adj < p.char_width_max_adj / 100.0
                                && self.iteration < MAX_ITERATIONS
                            {
                                self.adjust_width(needed, width, justified);
                                return Pass::Again;
                            }
                            if self.font_size > p.font_size_min && self.iteration < MAX_ITERATIONS {
                                self.shrink();
                                return Pass::Again;
                            }
                        }
                        let previous = soft_break.previous_word_break;
                        if first_word_of_line && previous != -1 && previous != last_soft_break {
                            i = self.restore(&soft_break.clone());
                            last_soft_break = previous;
                            if self.count > 0 && self.get(self.count - 1).unicode == 173 {
                                subst = (self.count as isize - 1, 45);
                                i -= 1;
                                self.count -= 1;
                                next!();
                            }
                        }
                        if next_height <= margin_height + 0.0001 {
                            self.insert_new_line(
                                i,
                                base_scale,
                                element_scale,
                                em_scale,
                                self.glyph_adjustment,
                                bold_spacing,
                                char_spacing,
                                width,
                                line_gap,
                                &mut max_visible_descender,
                            );
                            first_visible_pending = true;
                            first_word_of_line = true;
                            next!();
                        }
                        self.first_overflow.get_or_insert(self.count);
                        if p.auto_size {
                            if self.line_spacing_delta > p.line_spacing_max
                                && self.iteration < MAX_ITERATIONS
                            {
                                let adjust =
                                    (margin_height - next_height) / (self.line_number + 1) as f32;
                                self.line_spacing_delta = (self.line_spacing_delta
                                    + adjust / base_scale)
                                    .max(p.line_spacing_max);
                                return Pass::Again;
                            }
                            if self.char_width_adj < p.char_width_max_adj / 100.0
                                && self.iteration < MAX_ITERATIONS
                            {
                                self.adjust_width(needed, width, justified);
                                return Pass::Again;
                            }
                            if self.font_size > p.font_size_min && self.iteration < MAX_ITERATIONS {
                                self.shrink();
                                return Pass::Again;
                            }
                        }
                        match p.overflow {
                            3 => {
                                i = self.restore(&last_valid);
                                subst = (count_here as isize, 3);
                                next!();
                            }
                            // Overflow, masking, scroll rect (and the
                            // modes not ported: ellipsis, page, linked).
                            _ => {
                                self.insert_new_line(
                                    i,
                                    base_scale,
                                    element_scale,
                                    em_scale,
                                    self.glyph_adjustment,
                                    bold_spacing,
                                    char_spacing,
                                    width,
                                    line_gap,
                                    &mut max_visible_descender,
                                );
                                first_visible_pending = true;
                                first_word_of_line = true;
                                next!();
                            }
                        }
                    } else {
                        if p.auto_size && self.iteration < MAX_ITERATIONS {
                            if self.char_width_adj < p.char_width_max_adj / 100.0 {
                                self.adjust_width(needed, width, justified);
                                return Pass::Again;
                            }
                            if self.font_size > p.font_size_min {
                                self.shrink();
                                return Pass::Again;
                            }
                        }
                        if p.overflow == 3 {
                            i = self.restore(&word_wrap);
                            subst = (count_here as isize, 3);
                            next!();
                        }
                    }
                }
                match unicode {
                    9 => {
                        self.ch(cc).visible = false;
                        self.last_visible_of_line = cc;
                        self.line(self.line_number).space_count += 1;
                    }
                    173 => self.ch(cc).visible = false,
                    _ => {
                        if first_visible_pending {
                            first_visible_pending = false;
                            self.first_visible_of_line = cc;
                        }
                        self.line_visible_count += 1;
                        self.last_visible_of_line = cc;
                        let li = self.line(self.line_number);
                        li.margin_left = margin_left;
                        li.margin_right = margin_right;
                    }
                }
            } else if (matches!(unicode, 10 | 11 | 160 | 8199 | 8232 | 8233)
                || is_separator(unicode))
                && !matches!(unicode, 173 | 8203 | 8288)
            {
                self.line(self.line_number).space_count += 1;
                if unicode == 160 {
                    self.line(self.line_number).control_count += 1;
                }
            }
            let ln = self.line_number;
            self.ch(cc).line = ln;
            {
                let only_char = self.line(ln).character_count == 1;
                let justification = p.h_align;
                if (!matches!(unicode, 10 | 11 | 13) && !substituted) || only_char {
                    self.line(ln).alignment = justification;
                }
            }
            if unicode == 9 {
                let tab = font.face.tab_width * font.tab_size * element_scale;
                let next_stop = (self.x_advance / tab).ceil() * tab;
                self.x_advance = if next_stop > self.x_advance {
                    next_stop
                } else {
                    self.x_advance + tab
                };
            } else {
                self.x_advance += ((metrics.advance + adv_adjust) * element_scale
                    + (font.normal_spacing_offset + char_spacing + bold_spacing) * em_scale)
                    * cw;
                if white || unicode == 8203 {
                    self.x_advance += p.word_spacing * em_scale;
                }
            }
            let xa = self.x_advance;
            self.ch(cc).x_advance = xa;
            if unicode == 13 {
                self.x_advance = 0.0;
            }
            if matches!(unicode, 10 | 11 | 3 | 8232 | 8233)
                || (unicode == 45 && substituted)
                || cc + 1 == total
            {
                let delta = self.max_line_ascender - self.start_of_line_ascender;
                if self.line_offset > 0.0 && delta.abs() > 0.01 && !self.driven_line_spacing {
                    self.adjust_line_offset(self.first_of_line, cc, delta);
                    self.element_descender -= delta;
                    self.line_offset += delta;
                }
                let asc = self.max_line_ascender - self.line_offset;
                let desc = self.max_line_descender - self.line_offset;
                self.element_descender = self.element_descender.min(desc);
                if !max_visible_descender.0 {
                    max_visible_descender.1 = self.element_descender;
                }
                self.first_visible_of_line = self.first_visible_of_line.max(self.first_of_line);
                self.last_of_line = cc;
                self.last_visible_of_line =
                    self.last_visible_of_line.max(self.first_visible_of_line);
                let trailing =
                    ((font.normal_spacing_offset + char_spacing + bold_spacing) * em_scale) * cw;
                let lv = self.last_visible_of_line;
                let max_advance = if self.get(lv).visible {
                    self.get(lv).x_advance - trailing
                } else {
                    self.get(cc).x_advance - trailing
                };
                let (first, fv, lvc, offset) = (
                    self.first_of_line,
                    self.first_visible_of_line,
                    self.line_visible_count,
                    self.line_offset,
                );
                let li = self.line(ln);
                li.first = first;
                li.first_visible = fv;
                li.last = cc;
                li.last_visible = lv;
                li.character_count = cc + 1 - first.min(cc + 1);
                li.visible_count = lvc;
                li.width = width;
                if li.character_count == 1 {
                    li.alignment = p.h_align;
                }
                li.max_advance = max_advance;
                li.baseline = -offset;
                li.ascender = asc;
                li.descender = desc;
                li.line_height = asc - desc + line_gap * base_scale;
                if matches!(unicode, 10 | 11 | 45 | 8232 | 8233) {
                    self.line_number += 1;
                    first_visible_pending = true;
                    first_word_of_line = true;
                    self.first_of_line = cc + 1;
                    self.line_visible_count = 0;
                    self.line(self.line_number);
                    let adjusted = self.get(cc).adjusted_ascender;
                    let paragraph = if unicode == 10 || unicode == 8233 {
                        p.paragraph_spacing
                    } else {
                        0.0
                    };
                    self.line_offset += -self.max_line_descender
                        + adjusted
                        + (line_gap + self.line_spacing_delta) * base_scale
                        + (p.line_spacing + paragraph) * em_scale;
                    self.driven_line_spacing = false;
                    self.max_line_ascender = LARGE_NEGATIVE;
                    self.max_line_descender = LARGE_POSITIVE;
                    self.start_of_line_ascender = adjusted;
                    self.x_advance = 0.0;
                    word_wrap = self.save(i, cc as isize);
                    last_valid = self.save(i, cc as isize);
                    self.count += 1;
                    next!();
                }
                if unicode == 3 {
                    i = len;
                }
            }
            if self.get(cc).visible {
                let c = self.get(cc);
                self.extents[0] = self.extents[0].min(c.bottom_left[0]);
                self.extents[1] = self.extents[1].min(c.bottom_left[1]);
                self.extents[2] = self.extents[2].max(c.top_right[0]);
                self.extents[3] = self.extents[3].max(c.top_right[1]);
            }
            if p.word_wrap || matches!(p.overflow, 1 | 3 | 6) {
                let breakable = (white || matches!(unicode, 8203 | 45 | 173))
                    && !self.non_breaking_space
                    && !matches!(unicode, 160 | 8199 | 8209 | 8239 | 8288);
                let cjk = !self.non_breaking_space
                    && ((unicode > 4352 && unicode < 4607)
                        || (unicode > 43360 && unicode < 43391)
                        || (unicode > 44032 && unicode < 55295)
                        || (unicode > 11904 && unicode < 40959)
                        || (unicode > 63744 && unicode < 64255)
                        || (unicode > 65072 && unicode < 65103)
                        || (unicode > 65280 && unicode < 65519));
                if breakable {
                    word_wrap = self.save(i, cc as isize);
                    first_word_of_line = false;
                    soft_break.previous_word_break = -1;
                } else if cjk {
                    // The leading and following character rules are not
                    // ported: every CJK character may break.
                    word_wrap = self.save(i, cc as isize);
                    first_word_of_line = false;
                } else if first_word_of_line {
                    if white || (unicode == 173 && !soft_hyphen) {
                        soft_break = self.save(i, cc as isize);
                    }
                    word_wrap = self.save(i, cc as isize);
                }
            }
            last_valid = self.save(i, cc as isize);
            self.count += 1;
            i += 1;
        }
        let range = self.max_font_size - self.min_font_size;
        if p.auto_size
            && range > 0.051
            && self.font_size < p.font_size_max
            && self.iteration < MAX_ITERATIONS
        {
            if self.char_width_adj < p.char_width_max_adj / 100.0 {
                self.char_width_adj = 0.0;
            }
            self.min_font_size = self.font_size;
            let step = ((self.max_font_size - self.font_size) / 2.0).max(0.05);
            self.font_size += step;
            self.font_size =
                (((self.font_size * 20.0 + 0.5) as i32) as f32 / 20.0).min(p.font_size_max);
            return Pass::Again;
        }
        let _ = (unicode, element_scale);
        self.info.truncate(self.count);
        if self.count == 0 || (self.count == 1 && unicode == 3) {
            self.info.clear();
            return Pass::Done;
        }
        self.align(margin, max_visible_descender.1);
        Pass::Done
    }

    /// The alignment pass: each line moved by the vertical anchor and its
    /// horizontal alignment.
    fn align(&mut self, margin: [f32; 4], max_visible_descender: f32) {
        let r = self.rect;
        let bottom_left = [r.x, r.y];
        let top_left = [r.x, r.y + r.height];
        let mid_y = (bottom_left[1] + top_left[1]) / 2.0;
        let anchor: V2 = match self.p.v_align {
            256 => [
                top_left[0] + margin[0],
                top_left[1] - self.max_text_ascender - margin[1],
            ],
            512 => [
                bottom_left[0] + margin[0],
                mid_y
                    - (self.max_text_ascender + margin[1] + max_visible_descender - margin[3])
                        / 2.0,
            ],
            1024 => [
                bottom_left[0] + margin[0],
                bottom_left[1] - max_visible_descender + margin[3],
            ],
            2048 => [bottom_left[0] + margin[0], mid_y],
            4096 => [
                bottom_left[0] + margin[0],
                mid_y - (self.extents[3] + margin[1] + self.extents[1] - margin[3]) / 2.0,
            ],
            8192 => [
                bottom_left[0] + margin[0],
                mid_y - (self.max_cap_height - margin[1] - margin[3]) / 2.0,
            ],
            _ => [
                top_left[0] + margin[0],
                top_left[1] - self.max_text_ascender - margin[1],
            ],
        };
        for j in 0..self.info.len() {
            let line = self.info[j].line;
            let li = self.lines.get(line).copied().unwrap_or_default();
            let dx = match li.alignment {
                2 => li.margin_left + li.width / 2.0 - li.max_advance / 2.0,
                4 => li.margin_left + li.width - li.max_advance,
                32 => {
                    li.margin_left + li.width / 2.0 - {
                        let a = self
                            .info
                            .get(li.first_visible)
                            .map_or(0.0, |c| c.bottom_left[0]);
                        let b = self
                            .info
                            .get(li.last_visible)
                            .map_or(0.0, |c| c.top_right[0]);
                        (a + b) / 2.0
                    }
                }
                // Left (justified and flush are not ported: left).
                _ => li.margin_left,
            };
            let offset = [anchor[0] + dx, anchor[1]];
            let c = &mut self.info[j];
            for v in [
                &mut c.bottom_left,
                &mut c.top_left,
                &mut c.top_right,
                &mut c.bottom_right,
            ] {
                v[0] += offset[0];
                v[1] += offset[1];
            }
            c.origin += offset[0];
            c.x_advance += offset[0];
            c.ascender += offset[1];
            c.descender += offset[1];
            c.baseline += offset[1];
        }
    }
}

/// Lays out `p` in `rect` as `OnPreRenderCanvas` does: auto-size from
/// `fontSizeBase` clamped to the range, `GenerateTextMesh` until the size
/// settles. `padding` moves the quads (0 for layout), the same for every
/// font and without style padding; [`layout_text_with`] takes the
/// materials' paddings.
pub fn layout_text(fonts: &FontSet, p: &TextParams, rect: Rect, padding: f32) -> TextLayout {
    layout_text_with(fonts, p, rect, &Pads::uniform(padding))
}

/// [`layout_text`] with each font's material padding and style padding.
pub fn layout_text_with(fonts: &FontSet, p: &TextParams, rect: Rect, pads: &Pads) -> TextLayout {
    let mut unported = Vec::new();
    if p.font >= fonts.fonts.len() {
        return TextLayout::default();
    }
    for (what, on) in [
        ("ellipsis overflow", p.overflow == 1),
        ("page overflow", p.overflow == 5),
        ("linked overflow", p.overflow == 6),
        ("justified or flush alignment", p.h_align & (8 | 16) != 0),
        ("upper, lower or small caps", p.style & (8 | 16 | 32) != 0),
        ("underline or strikethrough", p.style & (4 | 64) != 0),
    ] {
        if on {
            unported.push(what.to_string());
        }
    }
    let chars = processing_array(&p.text, p.source, p.parse_ctrl_characters, p.rich_text);
    let prepared = prepare(fonts, p, &chars, &mut unported);
    let font_size = if p.auto_size {
        p.font_size_base.clamp(p.font_size_min, p.font_size_max)
    } else {
        p.font_size
    };
    let mut g = Gen {
        fonts,
        p,
        chars: &chars,
        prepared: &prepared,
        rect,
        pads,
        font_size,
        min_font_size: p.font_size_min,
        max_font_size: p.font_size_max,
        line_spacing_delta: 0.0,
        char_width_adj: 0.0,
        iteration: 0,
        info: Vec::new(),
        lines: Vec::new(),
        st: Styling::new(p, font_size, 0),
        count: 0,
        x_advance: 0.0,
        line_offset: 0.0,
        first_of_line: 0,
        first_visible_of_line: 0,
        last_visible_of_line: 0,
        last_of_line: 0,
        max_line_ascender: 0.0,
        max_line_descender: 0.0,
        line_number: 0,
        start_of_line_ascender: 0.0,
        line_visible_count: 0,
        driven_line_spacing: false,
        first_overflow: None,
        margin_left: 0.0,
        margin_right: 0.0,
        max_cap_height: 0.0,
        max_text_ascender: 0.0,
        element_descender: 0.0,
        page_ascender: 0.0,
        extents: [0.0; 4],
        glyph_adjustment: 0.0,
        non_breaking_space: false,
        truncated: false,
    };
    if prepared.is_empty() {
        return TextLayout {
            font_size,
            unported,
            ..TextLayout::default()
        };
    }
    loop {
        let r = g.pass();
        g.iteration += 1;
        // A guard beyond TMP's own iteration limit.
        if matches!(r, Pass::Done) || g.iteration > MAX_ITERATIONS + 10 {
            break;
        }
    }
    let lines = g.lines.iter().take(g.line_number + 1).copied().collect();
    TextLayout {
        font_size: g.font_size,
        iterations: g.iteration,
        characters: g.info,
        lines,
        truncated: g.truncated,
        first_overflow: g.first_overflow,
        char_width_adj: g.char_width_adj,
        unported,
    }
}
