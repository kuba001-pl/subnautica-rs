//! `TMP_Text.CalculatePreferredValues` and its callers
//! `GetPreferredWidth` / `GetPreferredHeight`, ported from the decompiled
//! game code (our code, the same control flow): a text's preferred size
//! as a layout element. Unlike `GenerateTextMesh` it has no vertical
//! overflow and no truncation; it breaks lines only when wrapping.

use super::generate::{Styling, case_transform, is_white_space, prepare};
use super::input::{parse_tag, processing_array};
use super::{FontSet, SYNTHESIZED, TextParams, style};

const LARGE_POSITIVE: f32 = 32767.0;
const LARGE_NEGATIVE: f32 = -32767.0;
const MAX_ITERATIONS: u32 = 100;

#[derive(Clone, Copy, Default)]
struct Ch {
    unicode: u32,
    adjusted_ascender: f32,
}

#[derive(Clone)]
struct Saved {
    previous_word_break: isize,
    total_count: isize,
    first_of_line: usize,
    styling: Styling,
    x_advance: f32,
    max_text_ascender: f32,
    element_descender: f32,
    start_of_line_ascender: f32,
    max_line_ascender: f32,
    max_line_descender: f32,
    line_number: usize,
    line_offset: f32,
    driven_line_spacing: bool,
    preferred_width: f32,
    preferred_height: f32,
}

struct Calc<'f, 'p> {
    fonts: &'f FontSet<'f>,
    p: &'p TextParams,
    chars: &'p [u32],
    prepared: &'p [super::generate::Prepared],
    margin_x: f32,
    wrap: bool,
    auto: bool,
    font_size: f32,
    min_font_size: f32,
    max_font_size: f32,
    line_spacing_delta: f32,
    char_width_adj: f32,
    iteration: u32,
    info: Vec<Ch>,
    st: Styling,
    count: usize,
    first_of_line: usize,
    x_advance: f32,
    max_text_ascender: f32,
    element_descender: f32,
    start_of_line_ascender: f32,
    max_line_ascender: f32,
    max_line_descender: f32,
    line_number: usize,
    line_offset: f32,
    driven_line_spacing: bool,
    preferred_width: f32,
    preferred_height: f32,
}

impl Calc<'_, '_> {
    fn save(&self, i: isize, count: isize) -> Saved {
        Saved {
            previous_word_break: i,
            total_count: count,
            first_of_line: self.first_of_line,
            styling: self.st.clone(),
            x_advance: self.x_advance,
            max_text_ascender: self.max_text_ascender,
            element_descender: self.element_descender,
            start_of_line_ascender: self.start_of_line_ascender,
            max_line_ascender: self.max_line_ascender,
            max_line_descender: self.max_line_descender,
            line_number: self.line_number,
            line_offset: self.line_offset,
            driven_line_spacing: self.driven_line_spacing,
            preferred_width: self.preferred_width,
            preferred_height: self.preferred_height,
        }
    }

    fn restore(&mut self, s: &Saved) -> isize {
        self.count = (s.total_count + 1).max(0) as usize;
        self.first_of_line = s.first_of_line;
        self.st = s.styling.clone();
        self.x_advance = s.x_advance;
        self.max_text_ascender = s.max_text_ascender;
        self.element_descender = s.element_descender;
        self.start_of_line_ascender = s.start_of_line_ascender;
        self.max_line_ascender = s.max_line_ascender;
        self.max_line_descender = s.max_line_descender;
        self.line_number = s.line_number;
        self.line_offset = s.line_offset;
        self.driven_line_spacing = s.driven_line_spacing;
        self.preferred_width = s.preferred_width;
        self.preferred_height = s.preferred_height;
        s.previous_word_break
    }

    fn get(&self, k: usize) -> Ch {
        self.info.get(k).copied().unwrap_or_default()
    }

    fn set(&mut self, k: usize, c: Ch) {
        while self.info.len() <= k {
            self.info.push(Ch::default());
        }
        self.info[k] = c;
    }

    /// One pass; `None` when auto-size wants another.
    #[allow(clippy::too_many_lines)]
    fn pass(&mut self) -> Option<[f32; 2]> {
        let p = self.p;
        let fonts = self.fonts;
        let primary = &fonts.fonts[p.font];
        let italic_style = primary.italic_style as i32;
        let total = self.prepared.len();
        let base_scale = self.font_size / primary.face.point_size as f32 * primary.face.scale;
        let em_scale = self.font_size * 0.01;
        let line_gap =
            primary.face.line_height - (primary.face.ascent_line - primary.face.descent_line);
        self.st = Styling::new(p, self.font_size, italic_style);
        self.info.clear();
        self.count = 0;
        self.first_of_line = 0;
        self.x_advance = 0.0;
        self.max_text_ascender = 0.0;
        self.element_descender = 0.0;
        self.start_of_line_ascender = 0.0;
        self.max_line_ascender = LARGE_NEGATIVE;
        self.max_line_descender = LARGE_POSITIVE;
        self.line_number = 0;
        self.line_offset = 0.0;
        self.driven_line_spacing = false;
        self.preferred_width = 0.0;
        self.preferred_height = 0.0;
        let width = self.margin_x + 0.0001;
        // num9 (this line's width), a (the widest line), num10 (height),
        // num11 (the last needed width).
        let (mut line_width, mut widest, mut height, mut needed) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut first_word_of_line = true; // flag2
        let mut cjk_run = false; // flag3
        let mut soft_hyphen = false; // flag4
        let mut subst: (isize, u32) = (-1, 0);
        let mut word_wrap = self.save(-1, -1);
        let mut soft_break = self.save(-1, -1);
        self.iteration += 1;
        let len = self.chars.len() as isize;
        let mut i: isize = 0;
        while i < len && self.chars[i as usize] != 0 {
            let mut unicode = self.chars[i as usize];
            if p.rich_text && unicode == u32::from('<') {
                if let Some((tag, end)) = parse_tag(self.chars, i as usize + 1) {
                    self.st.apply(&tag, self.font_size, p.style, italic_style);
                    i = end as isize + 1;
                    continue;
                }
            }
            let Some(prep) = self.prepared.get(self.count).copied() else {
                break;
            };
            let mut element = prep.element;
            let mut substituted = false;
            if subst.0 == self.count as isize {
                unicode = subst.1;
                substituted = true;
                if unicode == 3 {
                    element.glyph = SYNTHESIZED;
                    element.metrics = Default::default();
                }
            } else {
                unicode = prep.unicode;
            }
            let (_, small_caps) = case_transform(unicode, self.st.style);
            let font = &fonts.fonts[element.font.min(fonts.fonts.len() - 1)];
            // TMP takes a substituted line feed's size from the previous
            // character of the last mesh pass; without that pass, the
            // current size (the same for every in-scope text).
            let size = self.st.size;
            let mut scale = size * small_caps / font.face.point_size as f32 * font.face.scale;
            let (asc_line, desc_line) = if substituted && unicode == 8230 {
                (0.0, 0.0)
            } else {
                (font.face.ascent_line, font.face.descent_line)
            };
            let unscaled = scale;
            if unicode == 173 || unicode == 3 {
                scale = 0.0;
            }
            let cc = self.count;
            let metrics = element.metrics;
            let white = is_white_space(unicode);
            let mut adv_adjust = 0.0;
            if p.kerning && cc + 1 < total {
                if let Some(k) = font
                    .kerning
                    .get(&(element.glyph, self.prepared[cc + 1].element.glyph))
                {
                    adv_adjust = *k;
                }
            }
            let bold_spacing = if !element.alternate && self.st.style & style::BOLD != 0 {
                font.bold_spacing
            } else {
                0.0
            };
            let ascender = asc_line * scale / small_caps;
            let descender = desc_line * scale / small_caps;
            let first_of_line = cc == self.first_of_line;
            let mut ch = Ch {
                unicode,
                adjusted_ascender: 0.0,
            };
            if first_of_line || !white {
                self.max_line_ascender = self.max_line_ascender.max(ascender);
                self.max_line_descender = self.max_line_descender.min(descender);
                ch.adjusted_ascender = ascender;
                self.element_descender = descender - self.line_offset;
            } else {
                ch.adjusted_ascender = self.max_line_ascender;
                self.element_descender = self.max_line_descender - self.line_offset;
            }
            self.set(cc, ch);
            if self.line_number == 0 && (first_of_line || !white) {
                self.max_text_ascender = self.max_line_ascender;
            }
            let justified = p.h_align & (8 | 16) != 0;
            let cw = 1.0 - self.char_width_adj;
            if unicode == 9
                || (!white && unicode != 8203 && unicode != 173 && unicode != 3)
                || (unicode == 173 && !soft_hyphen)
            {
                needed = self.x_advance.abs()
                    + metrics.advance * cw * if unicode == 173 { unscaled } else { scale };
                if needed > width * if justified { 1.05 } else { 1.0 }
                    && self.wrap
                    && cc != self.first_of_line
                {
                    i = self.restore(&word_wrap);
                    if self.count > 0
                        && self.get(self.count - 1).unicode == 173
                        && !soft_hyphen
                        && p.overflow == 0
                    {
                        subst = (self.count as isize - 1, 45);
                        i -= 1;
                        self.count -= 1;
                        i += 1;
                        continue;
                    }
                    soft_hyphen = false;
                    if self.get(self.count).unicode == 173 {
                        soft_hyphen = true;
                        i += 1;
                        continue;
                    }
                    if self.auto && first_word_of_line {
                        if self.char_width_adj < p.char_width_max_adj / 100.0
                            && self.iteration < MAX_ITERATIONS
                        {
                            let mut t = needed;
                            if self.char_width_adj > 0.0 {
                                t /= 1.0 - self.char_width_adj;
                            }
                            let over =
                                needed - (width - 0.0001) * if justified { 1.05 } else { 1.0 };
                            self.char_width_adj =
                                (self.char_width_adj + over / t).min(p.char_width_max_adj / 100.0);
                            return None;
                        }
                        if self.font_size > p.font_size_min && self.iteration < MAX_ITERATIONS {
                            self.max_font_size = self.font_size;
                            let step = ((self.font_size - self.min_font_size) / 2.0).max(0.05);
                            self.font_size -= step;
                            self.font_size = (((self.font_size * 20.0 + 0.5) as i32) as f32 / 20.0)
                                .max(p.font_size_min);
                            return None;
                        }
                    }
                    let delta = self.max_line_ascender - self.start_of_line_ascender;
                    if self.line_offset > 0.0 && delta.abs() > 0.01 && !self.driven_line_spacing {
                        self.element_descender -= delta;
                        self.line_offset += delta;
                    }
                    let asc = self.max_line_ascender - self.line_offset;
                    let desc = self.max_line_descender - self.line_offset;
                    self.element_descender = self.element_descender.min(desc);
                    self.first_of_line = self.count;
                    line_width += self.x_advance;
                    height = if self.wrap {
                        self.max_text_ascender - self.element_descender
                    } else {
                        height.max(asc - desc)
                    };
                    self.line_number += 1;
                    let adjusted = self.get(self.count).adjusted_ascender;
                    self.line_offset += -self.max_line_descender
                        + adjusted
                        + (line_gap + self.line_spacing_delta) * base_scale
                        + p.line_spacing * em_scale;
                    self.driven_line_spacing = false;
                    self.max_line_ascender = LARGE_NEGATIVE;
                    self.max_line_descender = LARGE_POSITIVE;
                    self.start_of_line_ascender = adjusted;
                    self.x_advance = 0.0;
                    first_word_of_line = true;
                    i += 1;
                    continue;
                }
            }
            if unicode == 9 {
                let tab = font.face.tab_width * font.tab_size * scale;
                let stop = (self.x_advance / tab).ceil() * tab;
                self.x_advance = if stop > self.x_advance {
                    stop
                } else {
                    self.x_advance + tab
                };
            } else {
                self.x_advance += ((metrics.advance + adv_adjust) * scale
                    + (font.normal_spacing_offset + p.character_spacing + bold_spacing) * em_scale)
                    * cw;
                if white || unicode == 8203 {
                    self.x_advance += p.word_spacing * em_scale;
                }
            }
            if unicode == 13 {
                widest = widest.max(line_width + self.x_advance);
                line_width = 0.0;
                self.x_advance = 0.0;
            }
            if matches!(unicode, 10 | 11 | 3 | 8232 | 8233) || cc + 1 == total {
                let delta = self.max_line_ascender - self.start_of_line_ascender;
                if self.line_offset > 0.0 && delta.abs() > 0.01 && !self.driven_line_spacing {
                    self.element_descender -= delta;
                    self.line_offset += delta;
                }
                let desc = self.max_line_descender - self.line_offset;
                self.element_descender = self.element_descender.min(desc);
                if cc + 1 == total {
                    line_width = widest.max(line_width + needed);
                } else {
                    widest = widest.max(line_width + needed);
                    line_width = 0.0;
                }
                height = self.max_text_ascender - self.element_descender;
                if matches!(unicode, 10 | 11 | 45 | 8232 | 8233) {
                    word_wrap = self.save(i, cc as isize);
                    self.line_number += 1;
                    self.first_of_line = cc + 1;
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
                    self.count += 1;
                    i += 1;
                    continue;
                }
                if unicode == 3 {
                    i = len;
                }
            }
            if self.wrap || matches!(p.overflow, 1 | 3) {
                let breakable = (white || matches!(unicode, 8203 | 45 | 173))
                    && !matches!(unicode, 160 | 8199 | 8209 | 8239 | 8288);
                let cjk = (unicode > 4352 && unicode < 4607)
                    || (unicode > 43360 && unicode < 43391)
                    || (unicode > 44032 && unicode < 55295)
                    || (unicode > 11904 && unicode < 40959)
                    || (unicode > 63744 && unicode < 64255)
                    || (unicode > 65072 && unicode < 65103)
                    || (unicode > 65280 && unicode < 65519);
                if breakable {
                    word_wrap = self.save(i, cc as isize);
                    first_word_of_line = false;
                    cjk_run = false;
                    soft_break.previous_word_break = -1;
                } else if cjk {
                    // Line-breaking rules not ported: every CJK character
                    // may break.
                    word_wrap = self.save(i, cc as isize);
                    first_word_of_line = false;
                    cjk_run = true;
                } else if cjk_run {
                    word_wrap = self.save(i, cc as isize);
                    cjk_run = false;
                } else if first_word_of_line {
                    if white || (unicode == 173 && !soft_hyphen) {
                        soft_break = self.save(i, cc as isize);
                    }
                    word_wrap = self.save(i, cc as isize);
                    cjk_run = false;
                }
            }
            self.count += 1;
            i += 1;
        }
        let _ = &soft_break;
        let range = self.max_font_size - self.min_font_size;
        if self.auto
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
            return None;
        }
        let m = p.margin;
        line_width += m[0].max(0.0) + m[2].max(0.0);
        height += m[1].max(0.0) + m[3].max(0.0);
        // TMP rounds up to the hundredth.
        line_width = ((line_width * 100.0 + 1.0) as i32) as f32 / 100.0;
        height = ((height * 100.0 + 1.0) as i32) as f32 / 100.0;
        Some([line_width, height])
    }
}

/// `GetPreferredWidth()` (no wrapping, no auto-size, at the maximum size
/// if auto-sized) and `GetPreferredHeight()` (wrapping at
/// `margin_width`, the rect's width less margins, or unbounded if 0).
pub fn preferred_values(fonts: &FontSet, p: &TextParams, margin_width: f32) -> [f32; 2] {
    if p.font >= fonts.fonts.len() {
        return [0.0, 0.0];
    }
    let chars = processing_array(&p.text, p.source, p.parse_ctrl_characters, p.rich_text);
    let mut unported = Vec::new();
    let prepared = prepare(fonts, p, &chars, &mut unported);
    if prepared.is_empty() {
        return [0.0, 0.0];
    }
    let start = if p.auto_size {
        p.font_size_max
    } else {
        p.font_size
    };
    let run = |margin_x: f32, wrap: bool, auto: bool| -> [f32; 2] {
        let mut c = Calc {
            fonts,
            p,
            chars: &chars,
            prepared: &prepared,
            margin_x,
            wrap,
            auto,
            font_size: start,
            min_font_size: p.font_size_min,
            max_font_size: p.font_size_max,
            line_spacing_delta: 0.0,
            char_width_adj: 0.0,
            iteration: 0,
            info: Vec::new(),
            st: Styling::new(p, start, 0),
            count: 0,
            first_of_line: 0,
            x_advance: 0.0,
            max_text_ascender: 0.0,
            element_descender: 0.0,
            start_of_line_ascender: 0.0,
            max_line_ascender: 0.0,
            max_line_descender: 0.0,
            line_number: 0,
            line_offset: 0.0,
            driven_line_spacing: false,
            preferred_width: 0.0,
            preferred_height: 0.0,
        };
        loop {
            if let Some(v) = c.pass() {
                return v;
            }
            if c.iteration > MAX_ITERATIONS + 10 {
                return [0.0, 0.0];
            }
        }
    };
    let w = run(LARGE_POSITIVE, false, false)[0];
    let margin_x = if margin_width != 0.0 {
        margin_width
    } else {
        LARGE_POSITIVE
    };
    let h = run(margin_x, p.word_wrap, p.auto_size)[1];
    [w, h]
}
