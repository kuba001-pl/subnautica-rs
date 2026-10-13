//! The game's TMP texts as `sn-ui`'s text layout reads them (M9u4c): the
//! font assets as a [`FontSet`] and each `TextMeshProUGUI`'s fields as
//! [`TextParams`].

use std::collections::HashMap;

use sn_ui::font::FontFile;
use sn_ui::text::{FontSet, Source, TextParams, TmpFont};
use sn_unity::{Fields, Value};

use crate::{Assets, FontAssets, Layouts, ObjectRef, Result, UiTree};

fn num(f: &Fields, name: &str) -> Result<f64> {
    f.get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("no number {name}"))
}

fn flag(f: &Fields, name: &str) -> Result<bool> {
    f.get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("no bool {name}"))
}

/// A text of a UI tree with its settings.
pub struct UiText {
    pub node: usize,
    pub enabled: bool,
    pub params: TextParams,
    /// `m_fontSize` as stored: with auto-size, the size TMP last chose.
    pub stored_font_size: f32,
}

impl Assets<'_> {
    /// Every font asset as the text layout reads it, with `TMP Settings`'
    /// fallbacks. Kerning tables are empty: the game fills them at run
    /// time from the font files (not ported).
    pub fn tmp_font_set<'f>(&self, fonts: &'f FontAssets) -> Result<FontSet<'f>> {
        let index_of = |o: &ObjectRef| fonts.assets.iter().position(|a| a.object.key() == o.key());
        let mut out = Vec::with_capacity(fonts.assets.len());
        for a in &fonts.assets {
            let s = a.stored()?;
            let font = a
                .source
                .as_ref()
                .and_then(|o| fonts.font(o))
                .ok_or_else(|| format!("{}: no source font", a.name))?;
            let file = FontFile::parse(&font.font.data).map_err(|e| format!("{}: {e}", a.name))?;
            let mut weights = [[None; 2]; 10];
            let table = a
                .fields
                .get("m_FontWeightTable")
                .and_then(Value::as_array)
                .unwrap_or(&[]);
            for (k, pair) in table.iter().enumerate().take(10) {
                let Some(p) = pair.as_struct() else {
                    continue;
                };
                for (j, face) in ["regularTypeface", "italicTypeface"]
                    .into_iter()
                    .enumerate()
                {
                    if let Some(ptr) = p
                        .get(face)
                        .and_then(Value::as_pptr)
                        .filter(|p| !p.is_null())
                    {
                        if let Some(o) = self.resolve(&a.object.file, ptr)? {
                            weights[k][j] = index_of(&o);
                        }
                    }
                }
            }
            let f = &a.fields;
            out.push(TmpFont {
                name: a.name.clone(),
                face: s.face.clone(),
                file,
                sampling_point_size: s.point_size,
                bold_spacing: num(f, "boldSpacing")? as f32,
                bold_style: num(f, "boldStyle")? as f32,
                normal_style: num(f, "normalStyle")? as f32,
                normal_spacing_offset: num(f, "normalSpacingOffset")? as f32,
                italic_style: num(f, "italicStyle")? as f32,
                tab_size: num(f, "tabSize")? as f32,
                fallbacks: a.fallbacks.iter().filter_map(index_of).collect(),
                weights,
                kerning: HashMap::new(),
            });
        }
        let (fallbacks, default_font, missing) = match &fonts.settings {
            Some(s) => (
                s.fallbacks.iter().filter_map(index_of).collect(),
                s.default_font.as_ref().and_then(index_of),
                u32::try_from(s.missing_glyph).unwrap_or(0),
            ),
            None => (Vec::new(), None, 0),
        };
        Ok(FontSet::new(out, fallbacks, default_font, missing))
    }

    /// Every `TextMeshProUGUI` of a tree whose font asset is one of the
    /// set's, with its settings.
    pub fn ui_texts(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        fonts: &FontAssets,
    ) -> Result<Vec<UiText>> {
        let mut out = Vec::new();
        for (node, c) in tree.components() {
            if c.script.as_deref() != Some("TextMeshProUGUI") {
                continue;
            }
            let script = self
                .mono_script(&c.object)
                .ok_or("TextMeshProUGUI without its script")?;
            let layout = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
            let (_, d) = c.object.data()?;
            let (h, f) = sn_unity::read_script(d, c.object.file.file().big_endian, &layout)
                .map_err(|e| format!("{}: {e}", tree.nodes[node].path))?;
            let Some(font) = f
                .get("m_fontAsset")
                .and_then(Value::as_pptr)
                .map(|p| self.resolve(&c.object.file, p))
                .transpose()?
                .flatten()
                .and_then(|o| fonts.assets.iter().position(|a| a.object.key() == o.key()))
            else {
                continue;
            };
            let margin = f
                .get("m_margin")
                .and_then(Value::as_struct)
                .ok_or("no m_margin")?;
            let color = f
                .get("m_fontColor32")
                .and_then(Value::as_struct)
                .ok_or("no m_fontColor32")?;
            let c8 = |n: &str| num(color, n).map(|v| v as u8);
            let size = num(&f, "m_fontSize")? as f32;
            out.push(UiText {
                node,
                enabled: h.enabled,
                stored_font_size: size,
                params: TextParams {
                    text: f
                        .get("m_text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    source: Source::Stored,
                    font,
                    font_size: size,
                    font_size_base: num(&f, "m_fontSizeBase")? as f32,
                    auto_size: flag(&f, "m_enableAutoSizing")?,
                    font_size_min: num(&f, "m_fontSizeMin")? as f32,
                    font_size_max: num(&f, "m_fontSizeMax")? as f32,
                    style: num(&f, "m_fontStyle")? as u32,
                    weight: num(&f, "m_fontWeight")? as u32,
                    h_align: num(&f, "m_HorizontalAlignment")? as u32,
                    v_align: num(&f, "m_VerticalAlignment")? as u32,
                    character_spacing: num(&f, "m_characterSpacing")? as f32,
                    word_spacing: num(&f, "m_wordSpacing")? as f32,
                    line_spacing: num(&f, "m_lineSpacing")? as f32,
                    line_spacing_max: num(&f, "m_lineSpacingMax")? as f32,
                    paragraph_spacing: num(&f, "m_paragraphSpacing")? as f32,
                    char_width_max_adj: num(&f, "m_charWidthMaxAdj")? as f32,
                    word_wrap: flag(&f, "m_enableWordWrapping")?,
                    wrapping_ratios: num(&f, "m_wordWrappingRatios")? as f32,
                    overflow: num(&f, "m_overflowMode")? as i32,
                    kerning: flag(&f, "m_enableKerning")?,
                    rich_text: flag(&f, "m_isRichText")?,
                    parse_ctrl_characters: flag(&f, "m_parseCtrlCharacters")?,
                    margin: [
                        num(margin, "x")? as f32,
                        num(margin, "y")? as f32,
                        num(margin, "z")? as f32,
                        num(margin, "w")? as f32,
                    ],
                    color: [c8("r")?, c8("g")?, c8("b")?, c8("a")?],
                },
            });
        }
        Ok(out)
    }
}
