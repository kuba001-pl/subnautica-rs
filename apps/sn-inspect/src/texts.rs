//! `ui --texts` (M9u4c, `docs/formats/ui.md` § Text layout): a census of
//! the in-scope screens' TMP texts, so the text layout ports what they
//! use: each setting's values and how many texts use them, and the
//! rich-text tags in the stored texts and in the English language file.

use std::collections::BTreeMap;
use std::process::ExitCode;

use sn_assets::{Assets, ScriptAssemblies, UiTree};
use sn_install::GameData;
use sn_unity::{Fields, Value};

use crate::Result;

/// Each rich-text tag's name (`<b>`, `</color>`, `<size=…>` → `b`,
/// `/color`, `size`), counted.
pub fn tags(text: &str, out: &mut BTreeMap<String, usize>) {
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('>') else {
            break;
        };
        let inner = &after[..j];
        // Another '<' first: this one is text; look again from there.
        if let Some(k) = inner.find('<') {
            rest = &after[k..];
            continue;
        }
        // Not a tag if it is long free text.
        if inner.len() <= 64 && !inner.is_empty() {
            let name: String = inner
                .chars()
                .take_while(|c| !matches!(c, '=' | ' ' | '"'))
                .collect::<String>()
                .to_ascii_lowercase();
            *out.entry(name).or_default() += 1;
        }
        rest = &after[j + 1..];
    }
}

fn value_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::Float(f)) => format!("{f}"),
        Some(Value::Int(i)) => format!("{i}"),
        Some(Value::UInt(u)) => format!("{u}"),
        Some(Value::Bool(b)) => format!("{b}"),
        Some(Value::Struct(s)) => {
            s.0.iter()
                .map(|(_, v)| value_text(Some(v)))
                .collect::<Vec<_>>()
                .join(",")
        }
        Some(other) => format!("{other:?}"),
        None => "-".into(),
    }
}

const SETTINGS: [&str; 22] = [
    "m_fontSize",
    "m_enableAutoSizing",
    "m_fontSizeMin",
    "m_fontSizeMax",
    "m_fontStyle",
    "m_fontWeight",
    "m_HorizontalAlignment",
    "m_VerticalAlignment",
    "m_characterSpacing",
    "m_wordSpacing",
    "m_lineSpacing",
    "m_lineSpacingMax",
    "m_paragraphSpacing",
    "m_charWidthMaxAdj",
    "m_enableWordWrapping",
    "m_wordWrappingRatios",
    "m_overflowMode",
    "m_enableKerning",
    "m_enableExtraPadding",
    "m_isRichText",
    "m_parseCtrlCharacters",
    "m_margin",
];

fn census(
    assets: &Assets,
    layouts: &sn_assets::Layouts,
    ui: &UiTree,
    settings: &mut BTreeMap<String, BTreeMap<String, usize>>,
    stored_tags: &mut BTreeMap<String, usize>,
) -> Result<usize> {
    let mut n = 0;
    for (node, c) in ui.components() {
        if c.script.as_deref() != Some("TextMeshProUGUI") {
            continue;
        }
        let script = assets.mono_script(&c.object).ok_or("no script")?;
        let layout = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
        let (_, d) = c.object.data()?;
        let (h, f): (_, Fields) =
            sn_unity::read_script(d, c.object.file.file().big_endian, &layout)
                .map_err(|e| format!("{}: {e}", ui.nodes[node].path))?;
        n += 1;
        let active = if ui.nodes[node].active && h.enabled {
            "active"
        } else {
            "inactive"
        };
        *settings
            .entry("(active and enabled)".into())
            .or_default()
            .entry(active.into())
            .or_default() += 1;
        for s in SETTINGS {
            *settings
                .entry(s.into())
                .or_default()
                .entry(value_text(f.get(s)))
                .or_default() += 1;
        }
        if let Some(t) = f.get("m_text").and_then(Value::as_str) {
            tags(t, stored_tags);
        }
    }
    Ok(n)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let mut trees = vec![assets.ui_tree("uGUI.prefab", &root)?];
    for o in crate::ui::other_roots(&assets, &root)? {
        trees.push(assets.ui_tree("other root", &o)?);
    }
    let main = assets.scene("main")?;
    let (screen, _) = assets.pda_screen_refs(&main)?;
    trees.push(assets.ui_tree("PDA screen", &screen)?);
    let mut settings = BTreeMap::new();
    let mut stored_tags = BTreeMap::new();
    let mut n = 0;
    for ui in &trees {
        n += census(&assets, &layouts, ui, &mut settings, &mut stored_tags)?;
    }
    println!("{n} TMP texts in the in-scope trees");
    for (s, values) in &settings {
        println!("  {s}: {values:?}");
    }
    println!("tags in the stored texts: {stored_tags:?}");
    let english = sn_assets::language(game, "English")?;
    let mut lang_tags = BTreeMap::new();
    let mut with_tags = 0;
    for v in english.values() {
        let before: usize = lang_tags.values().sum();
        tags(v, &mut lang_tags);
        if lang_tags.values().sum::<usize>() > before {
            with_tags += 1;
        }
    }
    println!(
        "tags in the English language file ({} strings, {with_tags} with tags): {lang_tags:?}",
        english.len()
    );

    // Our layout of every text; the auto-sized ones against the size the
    // editor stored.
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let t0 = std::time::Instant::now();
    let (mut laid, mut glyphs) = (0usize, 0usize);
    let (mut same, mut differ, mut unknown) = (0usize, 0usize, 0usize);
    let mut unported: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = 0;
    for ui in &trees {
        let mut tree = assets.ui_layout(&layouts, ui)?.tree;
        let ui_texts = assets.ui_texts(&layouts, ui, &fonts)?;
        let sizer = sn_ui::text::Texts {
            fonts: &set,
            params: ui_texts
                .iter()
                .map(|t| (t.node, t.params.clone()))
                .collect(),
        };
        sn_ui::layout::layout_with(&mut tree, &sizer);
        for t in ui_texts {
            let rect = tree.rect(t.node);
            let path = &ui.nodes[t.node].path;
            if !(rect.width.is_finite() && rect.height.is_finite()) {
                if t.params.auto_size {
                    unknown += 1;
                    println!("  auto-size {path}: rect waits for text layout");
                }
                continue;
            }
            let l = sn_ui::text::layout_text(&set, &t.params, rect, 0.0);
            laid += 1;
            glyphs += l.characters.iter().filter(|c| c.visible).count();
            for u in &l.unported {
                *unported.entry(u.clone()).or_default() += 1;
            }
            if l.characters
                .iter()
                .any(|c| !(c.bottom_left[0].is_finite() && c.top_right[1].is_finite()))
            {
                failed += 1;
                println!("  NOT FINITE: {path}");
            }
            if t.params.auto_size {
                let ok = (l.font_size - t.stored_font_size).abs() < 0.001;
                if ok {
                    same += 1;
                } else {
                    differ += 1;
                }
                println!(
                    "  auto-size {path}: rect {:.1}×{:.1}, base {} in [{}, {}], stored {}, ours {} ({} passes, {} lines, wrap {}, {} characters){}",
                    rect.width,
                    rect.height,
                    t.params.font_size_base,
                    t.params.font_size_min,
                    t.params.font_size_max,
                    t.stored_font_size,
                    l.font_size,
                    l.iterations,
                    l.lines.len(),
                    t.params.word_wrap,
                    t.params.text.chars().count(),
                    if ok { "" } else { " DIFFERS" }
                );
            }
        }
    }
    println!(
        "laid out {laid} texts, {glyphs} visible characters, in {:.1} ms; non-finite {failed}; unported met {unported:?}",
        t0.elapsed().as_secs_f64() * 1000.0
    );
    println!(
        "auto-size against the stored size: {same} equal, {differ} differ, {unknown} wait for a text-sized rect"
    );
    Ok(if failed == 0 && differ == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_named_and_counted() {
        let mut t = BTreeMap::new();
        tags(
            "<color=#FF0000>Red</color> a < b <size=120%><B>x</b>",
            &mut t,
        );
        assert_eq!(t.get("color"), Some(&1));
        assert_eq!(t.get("/color"), Some(&1));
        assert_eq!(t.get("size"), Some(&1));
        assert_eq!(t.get("b"), Some(&1));
        assert_eq!(t.get("/b"), Some(&1));
        assert_eq!(t.len(), 5);
    }
}
