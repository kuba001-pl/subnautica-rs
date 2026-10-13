//! `ui --fonts [--atlas]` (M9u3, `docs/DESIGN.md` "M9u plan",
//! `docs/formats/ui.md` § Fonts): TextMesh Pro's font assets made from the
//! game's font files, checked against what the assets store.
//! - Every asset's face metrics from its font file, at its sampling size,
//!   against its stored `m_FaceInfo` (within 0.01).
//! - `Aller_Rg SDF`'s 2 stored glyphs remade: metrics, glyph, used and
//!   free rectangles, and the distance texels (mean error target ≤ 3/255,
//!   written in the plan before the first run).
//! - The characters of the English language file and of the in-scope
//!   screens' texts, each looked up as `TMP_Text.GetTextElement` does
//!   (confirmed from its IL): the text's font and its fallbacks, the
//!   text's sprite asset, `TMP Settings`' fallbacks, its default font,
//!   its default sprite asset. 0 missing.
//! - The cost of making the glyphs.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, FontAssets, ObjectRef, ScriptAssemblies, UiTree};
use sn_install::GameData;
use sn_ui::font::{Added, DynamicAtlas, FontFile, GlyphRect};
use sn_unity::Value;

use crate::Result;

/// The plan's target for the stored glyphs' texels.
const MEAN_TEXEL_TARGET: f64 = 3.0;
/// Face metrics agree within this (pixels).
const FACE_TOLERANCE: f32 = 0.01;
/// Fields the game's font engine takes from hinted glyphs (`H` and `x`),
/// and the one derived from them. Ours are unhinted, so these are
/// reported, not counted (the game uses its stored values).
const HINTED: [&str; 3] = ["m_CapLine", "m_MeanLine", "m_StrikethroughOffset"];

/// Texts whose stored string is never shown: the game sets the text
/// before it shows the object (confirmed from the IL). Their characters
/// are listed, not counted.
const OVERWRITTEN: [(&str, &str); 6] = [
    (
        "HandReticle/HandCanvas/Hand",
        "HandReticle.UpdateText sets it, shown only if not empty",
    ),
    (
        "HandReticle/HandCanvas/HandSubscript",
        "HandReticle.UpdateText",
    ),
    ("HandReticle/UseCanvas/Use", "HandReticle.UpdateText"),
    (
        "HandReticle/UseCanvas/UseSubscript",
        "HandReticle.UpdateText",
    ),
    (
        "Toolbar/CycleLeft_BtnTxt",
        "uGUI_Toolbar.UpdateCycleBtnText (OnEnable): hidden without a gamepad, else GameInput.FormatButton",
    ),
    (
        "Toolbar/CycleRight_BtnTxt",
        "uGUI_Toolbar.UpdateCycleBtnText",
    ),
];

struct Checks {
    failed: usize,
}

impl Checks {
    fn eq(&mut self, what: &str, got: usize, want: usize) {
        let ok = got == want;
        if !ok {
            self.failed += 1;
        }
        println!(
            "  check {what}: {got} (want {want}) {}",
            if ok { "ok" } else { "DIFFERS" }
        );
    }

    fn le(&mut self, what: &str, got: f64, max: f64) {
        let ok = got <= max;
        if !ok {
            self.failed += 1;
        }
        println!(
            "  check {what}: {got:.3} (want ≤ {max}) {}",
            if ok { "ok" } else { "DIFFERS" }
        );
    }
}

/// Characters TMP lays out without a glyph (controls, line and paragraph
/// separators, zero-width ones).
fn needs_glyph(c: char) -> bool {
    !(c.is_control()
        || matches!(
            c,
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2028}' | '\u{2029}' | '\u{FEFF}'
        ))
}

/// The asset and its fallbacks, depth first, each once (TMP's search
/// order through `m_FallbackFontAssetTable`).
fn chain(fonts: &FontAssets, start: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![start];
    while let Some(i) = stack.pop() {
        if out.contains(&i) {
            continue;
        }
        out.push(i);
        for f in fonts.assets[i].fallbacks.iter().rev() {
            if let Some(j) = fonts.assets.iter().position(|a| a.object.key() == f.key()) {
                stack.push(j);
            }
        }
    }
    out
}

/// A text of a screen: its path, its text, its font asset and sprite
/// asset.
struct ScreenText {
    path: String,
    text: String,
    font: Option<ObjectRef>,
    sprites: Option<ObjectRef>,
}

/// Where a character was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Found {
    Font(usize),
    Sprite(usize),
}

/// `TMP_Text.GetTextElement`'s order for a text with font `font` and
/// sprite asset `sprites`: the font's chain, the text's sprite asset, the
/// settings' fallbacks (each with its chain), the default font's chain,
/// the default sprite asset.
fn lookup_order(fonts: &FontAssets, font: Option<usize>, sprites: Option<usize>) -> Vec<Found> {
    let font_index = |o: &ObjectRef| fonts.assets.iter().position(|a| a.object.key() == o.key());
    let sprite_index = |o: &ObjectRef| {
        fonts
            .sprite_assets
            .iter()
            .position(|a| a.object.key() == o.key())
    };
    let mut out: Vec<Found> = Vec::new();
    let add_chain = |out: &mut Vec<Found>, i: usize| {
        for j in chain(fonts, i) {
            if !out.contains(&Found::Font(j)) {
                out.push(Found::Font(j));
            }
        }
    };
    if let Some(i) = font {
        add_chain(&mut out, i);
    }
    if let Some(i) = sprites {
        out.push(Found::Sprite(i));
    }
    if let Some(s) = &fonts.settings {
        for f in s.fallbacks.iter().filter_map(font_index) {
            add_chain(&mut out, f);
        }
        if let Some(f) = s.default_font.as_ref().and_then(font_index) {
            add_chain(&mut out, f);
        }
        if let Some(i) = s.default_sprite_asset.as_ref().and_then(sprite_index) {
            if !out.contains(&Found::Sprite(i)) {
                out.push(Found::Sprite(i));
            }
        }
    }
    out
}

/// Every `TextMeshProUGUI`'s text, font asset and sprite asset in a UI
/// tree.
fn texts(assets: &Assets, layouts: &sn_assets::Layouts, tree: &UiTree) -> Result<Vec<ScreenText>> {
    let mut out = Vec::new();
    for (node, c) in tree.components() {
        if c.script.as_deref() != Some("TextMeshProUGUI") {
            continue;
        }
        let script = assets
            .mono_script(&c.object)
            .ok_or("TextMeshProUGUI without its script")?;
        let layout = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
        let (_, d) = c.object.data()?;
        let (_, f) = sn_unity::read_script(d, c.object.file.file().big_endian, &layout)
            .map_err(|e| format!("{}: {e}", tree.nodes[node].path))?;
        let text = f
            .get("m_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let field = |name: &str| -> Result<Option<ObjectRef>> {
            match f.get(name).and_then(Value::as_pptr) {
                Some(p) => assets.resolve(&c.object.file, p),
                None => Ok(None),
            }
        };
        out.push(ScreenText {
            path: tree.nodes[node].path.clone(),
            text,
            font: field("m_fontAsset")?,
            sprites: field("m_spriteAsset")?,
        });
    }
    Ok(out)
}

pub fn run(game: &GameData, write_atlas: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let assemblies = ScriptAssemblies::load(game)?;
    let fonts = assets.font_assets(&assemblies)?;
    let mut checks = Checks { failed: 0 };
    println!(
        "{} fonts, {} TMP font assets ({:.2} s)",
        fonts.fonts.len(),
        fonts.assets.len(),
        start.elapsed().as_secs_f64()
    );
    checks.eq("fonts", fonts.fonts.len(), 9);
    checks.eq("font assets", fonts.assets.len(), 8);

    // Face metrics.
    let mut stored = Vec::new();
    let (mut compared, mut differ, mut hinted_differ) = (0, 0, 0);
    let mut dynamic = 0;
    for a in &fonts.assets {
        let s = a.stored()?;
        let font = a
            .source
            .as_ref()
            .and_then(|o| fonts.font(o))
            .ok_or_else(|| format!("{}: no source font", a.name))?;
        let file = FontFile::parse(&font.font.data).map_err(|e| format!("{}: {e}", a.name))?;
        let ours = file.face_info(s.point_size);
        if s.population_mode == 1 && s.render_mode == 4165 {
            dynamic += 1;
        }
        println!(
            "{:?} ← font {:?} ({} glyphs, upem {}): point size {}, padding {}, atlas {}×{}, \
             population {}, render mode {}, multi-atlas {}, {} stored glyphs, {} fallbacks",
            a.name,
            font.font.name,
            file.glyph_count(),
            file.units_per_em(),
            s.point_size,
            s.padding,
            s.atlas_width,
            s.atlas_height,
            s.population_mode,
            s.render_mode,
            s.multi_atlas,
            s.glyphs.len(),
            a.fallbacks.len()
        );
        for (name, want, got) in [
            ("m_FamilyName", &s.face.family_name, &ours.family_name),
            ("m_StyleName", &s.face.style_name, &ours.style_name),
        ] {
            compared += 1;
            if want != got {
                differ += 1;
                println!("    {name}: ours {got:?}, stored {want:?} DIFFERS");
            }
        }
        for ((name, want), (_, got)) in s.face.numbers().iter().zip(ours.numbers()) {
            let same = (want - got).abs() <= FACE_TOLERANCE;
            if HINTED.contains(name) {
                if !same {
                    hinted_differ += 1;
                }
                println!(
                    "    {name}: ours {got}, stored {want} (hinted in the game){}",
                    if same { "" } else { " differs" }
                );
                continue;
            }
            compared += 1;
            if !same {
                differ += 1;
                println!("    {name}: ours {got}, stored {want} DIFFERS");
            }
        }
        stored.push((s, file));
    }
    println!(
        "face metrics: {compared} fields compared, {differ} differ; hinted fields: {hinted_differ} of {} differ",
        fonts.assets.len() * HINTED.len()
    );
    checks.eq("dynamic SDFAA assets", dynamic, 8);
    let multi: Vec<&str> = stored
        .iter()
        .zip(&fonts.assets)
        .filter(|((s, _), _)| s.multi_atlas)
        .map(|(_, a)| a.name.as_str())
        .collect();
    println!("  multi-atlas assets (a new page when the atlas is full): {multi:?}");
    checks.eq("face fields that differ (not hinted)", differ, 0);

    // The stored glyphs of Aller_Rg SDF, remade in a fresh atlas in the
    // order of its character table.
    let ai = fonts
        .assets
        .iter()
        .position(|a| a.name == "Aller_Rg SDF")
        .ok_or("no Aller_Rg SDF")?;
    let (s, file) = &stored[ai];
    let page = fonts.assets[ai]
        .atlas_textures
        .first()
        .ok_or("Aller_Rg SDF: no atlas texture")?;
    let page = assets.texture(page)?;
    let (pw, ph) = (page.texture.width, page.texture.height);
    checks.eq(
        "stored atlas is Alpha8 at the asset's size",
        usize::from(page.texture.format == 1 && pw == s.atlas_width && ph == s.atlas_height),
        1,
    );
    let mut atlas = DynamicAtlas::new(s.atlas_width, s.atlas_height, s.padding, s.point_size);
    let (mut glyph_same, mut rect_same) = (0, 0);
    for &(unicode, index) in &s.characters {
        let c = char::from_u32(unicode).ok_or("bad unicode")?;
        let want = s
            .glyphs
            .iter()
            .find(|g| g.index == index)
            .ok_or("character without its glyph")?;
        let Added::New(got) = atlas.add_character(file, c) else {
            return Err(format!("U+{unicode:04X} not added"));
        };
        println!(
            "  U+{unicode:04X} glyph {index}: ours {:?} {:?}, stored {:?} {:?}",
            got.metrics, got.rect, want.metrics, want.rect
        );
        glyph_same += usize::from(got.index == want.index && got.metrics == want.metrics);
        rect_same += usize::from(got.rect == want.rect);
    }
    checks.eq(
        "stored glyphs: same index and metrics",
        glyph_same,
        s.glyphs.len(),
    );
    checks.eq(
        "stored glyphs: same glyph rectangle",
        rect_same,
        s.glyphs.len(),
    );
    checks.eq(
        "used rectangles equal (in order)",
        usize::from(atlas.packer.used == s.used_rects),
        1,
    );
    let ours_free: BTreeSet<GlyphRect> = atlas.packer.free.iter().copied().collect();
    let want_free: BTreeSet<GlyphRect> = s.free_rects.iter().copied().collect();
    println!("  free rectangles: ours {ours_free:?}, stored {want_free:?}");
    checks.eq(
        "free rectangles equal (as sets)",
        usize::from(ours_free == want_free),
        1,
    );

    // Texels: over each used rectangle, ours against the stored page.
    let w = s.atlas_width as usize;
    let stored_texel = |x: i32, y: i32| -> f64 {
        if x < 0 || y < 0 || x >= pw || y >= ph {
            return 0.0;
        }
        f64::from(page.data[y as usize * w + x as usize])
    };
    let (mut sum, mut n, mut max) = (0.0, 0usize, 0.0f64);
    let mut hist: BTreeMap<u32, usize> = BTreeMap::new();
    for u in &s.used_rects {
        for y in u.y..u.y + u.height {
            for x in u.x..u.x + u.width {
                let e = (f64::from(atlas.texels[y as usize * w + x as usize]) - stored_texel(x, y))
                    .abs();
                sum += e;
                n += 1;
                max = max.max(e);
                *hist.entry((e as u32).min(16)).or_default() += 1;
            }
        }
    }
    let mean = sum / n.max(1) as f64;
    println!(
        "  texels compared {n}: mean |error| {mean:.3}/255, max {max}/255; histogram {hist:?}"
    );
    // The same with our texels shifted, to see the alignment.
    let mut shifts = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (mut s2, mut n2) = (0.0, 0usize);
            for u in &s.used_rects {
                for y in u.y..u.y + u.height {
                    for x in u.x..u.x + u.width {
                        let ours = f64::from(atlas.texels[y as usize * w + x as usize]);
                        s2 += (ours - stored_texel(x + dx, y + dy)).abs();
                        n2 += 1;
                    }
                }
            }
            shifts.push(((dx, dy), s2 / n2.max(1) as f64));
        }
    }
    shifts.sort_by(|a, b| a.1.total_cmp(&b.1));
    println!(
        "  mean error by shift (best first): {}",
        shifts
            .iter()
            .map(|((dx, dy), e)| format!("({dx},{dy}) {e:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    checks.le(
        "stored glyphs' mean texel error (/255)",
        mean,
        MEAN_TEXEL_TARGET,
    );
    checks.eq(
        "best alignment is no shift",
        usize::from(shifts.first().map(|s| s.0) == Some((0, 0))),
        1,
    );
    let other_texels = page.data.iter().filter(|&&v| v != 0).count()
        - s.used_rects
            .iter()
            .map(|u| {
                (u.y..u.y + u.height)
                    .flat_map(|y| (u.x..u.x + u.width).map(move |x| (x, y)))
                    .filter(|&(x, y)| stored_texel(x, y) != 0.0)
                    .count()
            })
            .sum::<usize>();
    checks.eq("stored texels outside the used rectangles", other_texels, 0);

    // Text: which characters each text needs, in its font's chain.
    let english = sn_assets::language(game, "English")?;
    let mut english_chars: BTreeMap<char, usize> = BTreeMap::new();
    for v in english.values() {
        for c in v.chars().filter(|&c| needs_glyph(c)) {
            *english_chars.entry(c).or_default() += 1;
        }
    }
    let layouts = assemblies.layouts()?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let mut trees = vec![assets.ui_tree("uGUI.prefab", &root)?];
    for o in crate::ui::other_roots(&assets, &root)? {
        trees.push(assets.ui_tree("uGUI.prefab (other root)", &o)?);
    }
    let main = assets.scene("main")?;
    let (screen, _) = assets.pda_screen_refs(&main)?;
    trees.push(assets.ui_tree("PDA screen", &screen)?);
    if let Some(st) = &fonts.settings {
        let font_name = |o: &ObjectRef| {
            fonts
                .assets
                .iter()
                .find(|a| a.object.key() == o.key())
                .map_or("(not a bundle asset)".to_string(), |a| a.name.clone())
        };
        println!(
            "TMP Settings ({} in {}): default font {:?}, fallbacks {:?}, default sprite asset {:?}, missing glyph character {}",
            st.object.path_id,
            st.object.file.name,
            st.default_font.as_ref().map(font_name),
            st.fallbacks.iter().map(font_name).collect::<Vec<_>>(),
            st.default_sprite_asset
                .as_ref()
                .and_then(|o| fonts.sprite_asset(o))
                .map(|a| a.name.as_str()),
            st.missing_glyph
        );
    }
    checks.eq(
        "TMP Settings found",
        usize::from(fonts.settings.is_some()),
        1,
    );
    for a in &fonts.sprite_assets {
        let pua = a
            .unicodes
            .iter()
            .filter(|&&u| (0xE000..=0xF8FF).contains(&u))
            .count();
        println!(
            "sprite asset {:?}: {} sprite characters, {pua} in the private use area",
            a.name,
            a.unicodes.len()
        );
    }
    let font_of = |o: &Option<ObjectRef>| {
        o.as_ref()
            .and_then(|f| fonts.assets.iter().position(|a| a.object.key() == f.key()))
    };
    let sprites_of = |o: &Option<ObjectRef>| {
        o.as_ref().and_then(|f| {
            fonts
                .sprite_assets
                .iter()
                .position(|a| a.object.key() == f.key())
        })
    };
    let find = |order: &[Found], c: char| -> Option<Found> {
        order.iter().copied().find(|f| match *f {
            Found::Font(i) => stored[i].1.glyph_index(c).is_some(),
            Found::Sprite(i) => fonts.sprite_assets[i].unicodes.contains(&u32::from(c)),
        })
    };
    let mut by_font: BTreeMap<String, usize> = BTreeMap::new();
    let mut screen_texts = Vec::new();
    for t in &trees {
        screen_texts.extend(texts(&assets, &layouts, t)?);
    }
    for t in &screen_texts {
        let name = font_of(&t.font).map_or("(none / other)".to_string(), |i| {
            fonts.assets[i].name.clone()
        });
        *by_font.entry(name).or_default() += 1;
    }
    println!(
        "texts of the in-scope screens: {} ({by_font:?}), {} with their own sprite asset",
        screen_texts.len(),
        screen_texts.iter().filter(|t| t.sprites.is_some()).count()
    );
    let mut screen_missing: BTreeMap<char, usize> = BTreeMap::new();
    let mut by_sprite: BTreeMap<String, usize> = BTreeMap::new();
    let mut overwritten_missing = 0;
    for t in &screen_texts {
        if t.text.is_empty() {
            continue;
        }
        if let Some((_, why)) = OVERWRITTEN.iter().find(|(p, _)| t.path.ends_with(p)) {
            let order = lookup_order(&fonts, font_of(&t.font), sprites_of(&t.sprites));
            let missing: Vec<String> = t
                .text
                .chars()
                .filter(|&c| needs_glyph(c) && find(&order, c).is_none())
                .map(|c| format!("U+{:04X}", u32::from(c)))
                .collect();
            overwritten_missing += missing.len();
            println!(
                "  set before shown, not checked: {} ({why}); stored text's missing {missing:?}",
                t.path
            );
            continue;
        }
        if font_of(&t.font).is_none() {
            println!("  {}: no font asset of the bundle", t.path);
        }
        let order = lookup_order(&fonts, font_of(&t.font), sprites_of(&t.sprites));
        for c in t.text.chars().filter(|&c| needs_glyph(c)) {
            match find(&order, c) {
                None => {
                    *screen_missing.entry(c).or_default() += 1;
                    println!("  missing U+{:04X} in {}", u32::from(c), t.path);
                }
                Some(Found::Sprite(i)) => {
                    *by_sprite
                        .entry(format!(
                            "U+{:04X} in {:?} ({})",
                            u32::from(c),
                            fonts.sprite_assets[i].name,
                            t.path
                        ))
                        .or_default() += 1;
                }
                Some(Found::Font(_)) => {}
            }
        }
    }
    for (k, n) in &by_sprite {
        println!("  sprite character {k}: {n}");
    }
    println!(
        "  screen text characters missing: {screen_missing:?} ({overwritten_missing} in stored texts the game replaces)"
    );
    checks.eq("screen text characters missing", screen_missing.len(), 0);
    let mut used_fonts: BTreeSet<usize> = screen_texts
        .iter()
        .filter_map(|t| font_of(&t.font))
        .collect();
    used_fonts.insert(ai);
    for &fi in &used_fonts {
        let order = lookup_order(&fonts, Some(fi), None);
        let names: Vec<String> = order
            .iter()
            .map(|f| match *f {
                Found::Font(i) => fonts.assets[i].name.clone(),
                Found::Sprite(i) => format!("sprites {}", fonts.sprite_assets[i].name),
            })
            .collect();
        let mut first: BTreeMap<String, usize> = BTreeMap::new();
        let mut missing = Vec::new();
        for &c in english_chars.keys() {
            match find(&order, c) {
                Some(Found::Font(i)) => {
                    *first.entry(fonts.assets[i].name.clone()).or_default() += 1
                }
                Some(Found::Sprite(i)) => {
                    *first
                        .entry(format!("sprites {}", fonts.sprite_assets[i].name))
                        .or_default() += 1
                }
                None => missing.push(format!("U+{:04X}", u32::from(c))),
            }
        }
        println!(
            "  English: {} distinct characters; order {names:?}: found in {first:?}, {} missing {missing:?}",
            english_chars.len(),
            missing.len()
        );
        checks.eq(
            &format!("English characters missing for {}", fonts.assets[fi].name),
            missing.len(),
            0,
        );
    }

    // Cost: every English character made on demand, through Aller_Rg
    // SDF's chain, each asset with its own atlas.
    let ch = chain(&fonts, ai);
    let mut atlases: HashMap<usize, DynamicAtlas> = HashMap::new();
    let (mut made, mut full, mut missing) = (0usize, 0usize, 0usize);
    let t0 = Instant::now();
    for &c in english_chars.keys() {
        let mut done = false;
        for &i in &ch {
            let (s, file) = &stored[i];
            if file.glyph_index(c).is_none() {
                continue;
            }
            let a = atlases.entry(i).or_insert_with(|| {
                DynamicAtlas::new(s.atlas_width, s.atlas_height, s.padding, s.point_size)
            });
            match a.add_character(file, c) {
                Added::New(_) => made += 1,
                Added::Cached(_) => {}
                Added::Full => full += 1,
                Added::Missing => continue,
            }
            done = true;
            break;
        }
        if !done {
            missing += 1;
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "glyph cost: {made} glyphs made in {:.1} ms ({:.0} µs each); {full} did not fit, {missing} in no font",
        secs * 1000.0,
        secs * 1e6 / made.max(1) as f64
    );
    let mut order: Vec<&usize> = atlases.keys().collect();
    order.sort();
    for i in order {
        let a = &atlases[i];
        let area: i64 = a
            .packer
            .used
            .iter()
            .map(|r| i64::from(r.width) * i64::from(r.height))
            .sum();
        println!(
            "  atlas {:?}: {} glyphs, {:.1}% of {}×{} used",
            fonts.assets[*i].name,
            a.glyphs.len(),
            100.0 * area as f64 / (f64::from(a.width) * f64::from(a.height)),
            a.width,
            a.height
        );
    }
    checks.eq("English glyphs that did not fit their atlas", full, 0);

    if write_atlas {
        let dir = std::path::Path::new("out/ui/fonts");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (i, a) in &atlases {
            let (w, h) = (a.width as usize, a.height as usize);
            let mut rgba = Vec::with_capacity(w * h * 4);
            // Top row first for the PNG.
            for y in (0..h).rev() {
                for &v in &a.texels[y * w..(y + 1) * w] {
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            let path = dir.join(format!("{}.png", fonts.assets[*i].name));
            std::fs::write(&path, crate::png::encode_rgba(w, h, &rgba))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            println!("  wrote {}", path.display());
        }
    }

    println!(
        "{} checks differ ({:.1} s)",
        checks.failed,
        start.elapsed().as_secs_f64()
    );
    Ok(if checks.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
