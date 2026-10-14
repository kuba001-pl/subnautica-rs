//! `ui --draw` (M9u4d, M9u5a; `docs/formats/ui.md` § Draw order, § UI
//! shaders and materials): the draw list of every in-scope tree (images,
//! raw images and texts on their laid-out rects, in uGUI's order, in
//! canvas space, with alpha, clipping, materials and the texts' glyph
//! UVs), built twice from scratch: the two hashes must be equal. Also the
//! draws per shader and keyword set, the shaders M9u5c–d will not port,
//! every text quad's UVs against its font atlas, and the glyphs made.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases, UiDraw, UiTree, font_texture_key};
use sn_install::GameData;

use crate::Result;

/// The shaders the client ports (M9u5c–d; `Hidden/UI/CircularBar`
/// M9u6b1; `docs/DESIGN.md`).
const PORTED: [&str; 7] = [
    "Hidden/UI/CircularBar",
    "UI/Default",
    "UI/Simple",
    "TextMeshPro/Distance Field",
    "UI/Bar",
    "uGUI/WorldCursor",
    "uGUI/Compass",
];

/// Target written before the first run (`docs/DESIGN.md` M9u5a): glyphs
/// for every stored text of the HUD and the PDA screen in at most 1 s.
const GLYPH_SECONDS: f64 = 1.0;

/// The properties the TMP programs read that `GetPadding` and the
/// shader need saved.
const TMP_PROPERTIES: [&str; 10] = [
    "_GradientScale",
    "_WeightNormal",
    "_WeightBold",
    "_ScaleRatioA",
    "_TextureWidth",
    "_TextureHeight",
    "_ScaleX",
    "_ScaleY",
    "_Sharpness",
    "_PerspectiveFilter",
];

pub(crate) fn trees(assets: &Assets) -> Result<Vec<UiTree>> {
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let mut out = vec![assets.ui_tree("uGUI.prefab", &root)?];
    for o in crate::ui::other_roots(assets, &root)? {
        let mut t = assets.ui_tree("other root", &o)?;
        t.key = format!(
            "uGUI.prefab root {}",
            t.nodes.first().map_or("", |n| n.name.as_str())
        );
        out.push(t);
    }
    let main = assets.scene("main")?;
    let (screen, _) = assets.pda_screen_refs(&main)?;
    out.push(assets.ui_tree("PDA screen", &screen)?);
    Ok(out)
}

/// The numbers a run collects over every tree.
#[derive(Default)]
struct Totals {
    /// (shader, keywords) → draws and material names.
    census: BTreeMap<(String, String), (usize, BTreeSet<String>)>,
    unported: BTreeMap<String, usize>,
    items: usize,
    with_material: usize,
    quads: usize,
    uv_outside: usize,
    centre_off: usize,
    reach_past: usize,
    failed: usize,
}

fn lo(a: &[f32]) -> f32 {
    a.iter().copied().fold(f32::INFINITY, f32::min)
}

fn hi(a: &[f32]) -> f32 {
    a.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}

/// Materials, the census and the text UV checks of one tree.
fn check(ui: &UiTree, d: &UiDraw, atlases: &GlyphAtlases, t: &mut Totals) {
    for u in &d.unported {
        // Counted by what, not by text.
        let what = u.rsplit(": ").next().unwrap_or(u).to_string();
        *t.unported.entry(what).or_default() += 1;
    }
    for it in &d.list.items {
        t.items += 1;
        let Some(m) = d.materials.get(it.material) else {
            t.failed += 1;
            println!("  NO MATERIAL: {}", ui.nodes[it.node].path);
            continue;
        };
        t.with_material += 1;
        let e = t
            .census
            .entry((
                m.shader.clone(),
                m.draw_keywords(it.clip.is_some()).join(" "),
            ))
            .or_default();
        e.0 += 1;
        e.1.insert(m.name.clone());
        if !it.kind.starts_with("Text") {
            continue;
        }
        let font = (it.texture & 0xFFFF_FFFF) as usize;
        let Some(atlas) = atlases
            .atlases
            .get(font)
            .filter(|_| it.texture == font_texture_key(font))
        else {
            t.failed += 1;
            println!("  TEXT WITHOUT A FONT ATLAS: {}", ui.nodes[it.node].path);
            continue;
        };
        let (w, h) = (atlas.width as f32, atlas.height as f32);
        for q in it.vertices.chunks(4) {
            t.quads += 1;
            let us: Vec<f32> = q.iter().map(|v| v.uv[0]).collect();
            let vs: Vec<f32> = q.iter().map(|v| v.uv[1]).collect();
            if lo(&us) < 0.0 || lo(&vs) < 0.0 || hi(&us) > 1.0 || hi(&vs) > 1.0 {
                t.uv_outside += 1;
            }
            // The quad's centre must fall in one packed glyph; its reach
            // past that glyph's packed rect is logged (the material
            // padding can exceed the atlas padding).
            let (cx, cy) = ((lo(&us) + hi(&us)) / 2.0 * w, (lo(&vs) + hi(&vs)) / 2.0 * h);
            let used = atlas.packer.used.iter().find(|r| {
                cx >= r.x as f32
                    && cx <= (r.x + r.width) as f32
                    && cy >= r.y as f32
                    && cy <= (r.y + r.height) as f32
            });
            match used {
                None => t.centre_off += 1,
                Some(r) => {
                    if lo(&us) * w < r.x as f32 - 0.01
                        || lo(&vs) * h < r.y as f32 - 0.01
                        || hi(&us) * w > (r.x + r.width) as f32 + 0.01
                        || hi(&vs) * h > (r.y + r.height) as f32 + 0.01
                    {
                        t.reach_past += 1;
                    }
                }
            }
        }
    }
}

/// The per-tree summary (as M9u4d's, with materials and texts).
fn report(ui: &UiTree, d: &UiDraw, t: &mut Totals) {
    if !d.unknown_layers.is_empty() {
        println!(
            "  {}: sorting layer ids not in the TagManager: {:?}",
            ui.key, d.unknown_layers
        );
    }
    let items = &d.list.items;
    let vertices: usize = items.iter().map(|i| i.vertices.len()).sum();
    let mut by_canvas: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut textures = BTreeSet::new();
    for it in items {
        *by_canvas
            .entry(ui.nodes[it.canvas].path.clone())
            .or_default() += 1;
        *by_kind
            .entry(it.kind.split(' ').next().unwrap_or("").to_string())
            .or_default() += 1;
        textures.insert(it.texture);
        if it
            .vertices
            .iter()
            .any(|v| !(v.pos[0].is_finite() && v.pos[1].is_finite()))
        {
            t.failed += 1;
            println!("  NOT FINITE: {}", ui.nodes[it.node].path);
        }
    }
    let clipped = items.iter().filter(|i| i.clip.is_some()).count();
    let faded = items.iter().filter(|i| i.alpha < 1.0).count();
    println!(
        "{}: {} draws, {vertices} vertices, {} textures, {} materials; texts {} (sub-meshes {}); clipped {clipped}, culled {}, transparent {}, alpha < 1: {faded}, without a canvas {}; hash {:016x}",
        ui.key,
        items.len(),
        textures.len(),
        d.materials.len(),
        d.texts,
        d.sub_meshes,
        d.list.culled,
        d.list.transparent,
        d.list.without_canvas,
        d.list.hash()
    );
    if !items.is_empty() {
        println!("  by draw group: {by_canvas:?}");
        println!("  by kind: {by_kind:?}");
    }
    let mut order: Vec<&str> = Vec::new();
    for it in items {
        let p = ui.nodes[it.canvas].path.as_str();
        if order.last() != Some(&p) {
            order.push(p);
        }
    }
    if order.len() > 1 {
        println!("  draw groups in order: {order:?}");
    }
    for m in d
        .materials
        .iter()
        .filter(|m| m.shader == "TextMeshPro/Distance Field")
    {
        let missing: Vec<&str> = TMP_PROPERTIES.into_iter().filter(|k| !m.has(k)).collect();
        println!(
            "  TMP material {:?} [{}]: gradient {}, ratio A {:.4}, C {:.4}; not saved: {missing:?}",
            m.name,
            m.keywords.join(" "),
            m.float("_GradientScale"),
            m.float("_ScaleRatioA"),
            m.float("_ScaleRatioC"),
        );
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let tags = sn_assets::physics_settings(&Assets::index(game)?)?.tags;
    println!(
        "sorting layers (TagManager order): {:?}",
        tags.sorting_layers
    );
    let mut hashes: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    let mut totals = Totals::default();
    let mut glyph_seconds = 0.0;
    for (run, h) in hashes.iter_mut().enumerate() {
        // Everything from scratch each run.
        let assets = Assets::index(game)?;
        let assemblies = sn_assets::ScriptAssemblies::load(game)?;
        let layouts = assemblies.layouts()?;
        let fonts = assets.font_assets(&assemblies)?;
        let set = assets.tmp_font_set(&fonts)?;
        let mut atlases = GlyphAtlases::new(&fonts)?;
        for ui in trees(&assets)? {
            let d = assets.ui_draw(
                &layouts,
                &ui,
                &fonts,
                &set,
                &mut atlases,
                &tags.sorting_layers,
            )?;
            h.push(d.list.hash());
            if run == 0 {
                check(&ui, &d, &atlases, &mut totals);
                report(&ui, &d, &mut totals);
            }
        }
        if run > 0 {
            continue;
        }
        glyph_seconds = atlases.seconds;
        let t = &mut totals;
        println!("draws per shader and keyword set (with the clip keyword):");
        let mut not_ported = 0;
        for ((shader, keys), (n, names)) in &t.census {
            let mark = if PORTED.contains(&shader.as_str()) {
                ""
            } else {
                not_ported += n;
                "  NOT PORTED YET"
            };
            println!("  {shader} [{keys}]: {n} draws; materials {names:?}{mark}");
        }
        println!(
            "materials: {} of {} draw items have one; {not_ported} draws use a shader not ported yet",
            t.with_material, t.items
        );
        println!(
            "text quads: {}; UVs outside the atlas {}; centre in no packed glyph {}; reaching past their packed rect (material padding > atlas padding) {}",
            t.quads, t.uv_outside, t.centre_off, t.reach_past
        );
        if t.uv_outside + t.centre_off > 0 {
            t.failed += 1;
        }
        let used: Vec<String> = atlases
            .atlases
            .iter()
            .enumerate()
            .filter(|(_, a)| !a.glyphs.is_empty())
            .map(|(i, a)| format!("{} {} glyphs", fonts.assets[i].name, a.glyphs.len()))
            .collect();
        println!(
            "glyphs made: {} in {:.3} s (target <= {GLYPH_SECONDS} s), {:.2} ms each; atlas full {}; {used:?}",
            atlases.made,
            atlases.seconds,
            if atlases.made > 0 {
                atlases.seconds * 1000.0 / atlases.made as f64
            } else {
                0.0
            },
            atlases.full
        );
        if !t.unported.is_empty() {
            println!("met but not ported (texts): {:?}", t.unported);
        }
    }
    let same = hashes[0] == hashes[1];
    println!(
        "two runs from scratch: {} of {} draw-list hashes equal; failures {} ({:.1} s)",
        hashes[0]
            .iter()
            .zip(&hashes[1])
            .filter(|(a, b)| a == b)
            .count(),
        hashes[0].len(),
        totals.failed,
        start.elapsed().as_secs_f64()
    );
    Ok(
        if same && totals.failed == 0 && glyph_seconds <= GLYPH_SECONDS {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        },
    )
}
