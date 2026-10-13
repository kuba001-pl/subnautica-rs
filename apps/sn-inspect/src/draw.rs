//! `ui --draw` (M9u4d, `docs/formats/ui.md` § Draw order): the draw list
//! of every in-scope tree (images, raw images and texts on their laid-out
//! rects, in uGUI's order, in canvas space, with alpha and clipping),
//! built twice from scratch: the two hashes must be equal.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, FontAssets, Layouts, UiGraphicKind, UiTree};
use sn_install::GameData;
use sn_ui::draw::{DrawInput, DrawList, Graphic, draw_list};
use sn_ui::layout::layout_with;
use sn_ui::mesh::{MeshData, Vertex, image_mesh, raw_image_mesh};
use sn_ui::text::{FontSet, Texts, layout_text};

use crate::Result;

fn key_hash<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// One tree's draw list.
fn build(
    assets: &Assets,
    layouts: &Layouts,
    ui: &UiTree,
    fonts: &FontAssets,
    set: &FontSet,
    sorting_layers: &[(String, u32)],
) -> Result<(DrawList, Vec<i32>)> {
    let parts = assets.ui_draw_parts(layouts, ui, sorting_layers)?;
    let mut tree = assets.ui_layout(layouts, ui)?.tree;
    let texts = assets.ui_texts(layouts, ui, fonts)?;
    let sizer = Texts {
        fonts: set,
        params: texts.iter().map(|t| (t.node, t.params.clone())).collect(),
    };
    layout_with(&mut tree, &sizer);
    let mut graphics = Vec::new();
    let finite = |r: sn_ui::layout::Rect| r.width.is_finite() && r.height.is_finite();
    for g in assets.ui_graphics(layouts, ui)? {
        if !g.enabled {
            continue;
        }
        let rect = tree.rect(g.node);
        if !finite(rect) {
            continue;
        }
        let (mesh, texture, kind) = match &g.kind {
            UiGraphicKind::Image { params, texture } => {
                let (kind, m) = image_mesh(rect, tree.nodes[g.node].rect.pivot, params, 100.0);
                (
                    m,
                    texture.as_ref().map_or(0, |t| key_hash(&t.key())),
                    format!("Image {kind:?}"),
                )
            }
            UiGraphicKind::RawImage {
                color,
                uv_rect,
                texture,
            } => (
                raw_image_mesh(rect, *color, *uv_rect, true),
                texture.as_ref().map_or(0, |t| key_hash(&t.key())),
                "RawImage".to_string(),
            ),
            _ => continue,
        };
        graphics.push(Graphic {
            node: g.node,
            mesh,
            texture,
            maskable: parts.maskable.get(&g.node).copied().unwrap_or(true),
            cull_transparent: parts
                .cull_transparent
                .get(&g.node)
                .copied()
                .unwrap_or(false),
            kind,
        });
    }
    for t in texts.iter().filter(|t| t.enabled) {
        let rect = tree.rect(t.node);
        if !finite(rect) {
            continue;
        }
        let l = layout_text(set, &t.params, rect, 0.0);
        // Positions only: the text's UVs come with its mesh (M9u5).
        let mut mesh = MeshData::default();
        for c in l.characters.iter().filter(|c| c.visible) {
            let s = mesh.vertices.len() as u32;
            for p in [c.bottom_left, c.top_left, c.top_right, c.bottom_right] {
                mesh.vertices.push(Vertex {
                    pos: p,
                    color: c.color,
                    uv: [0.0, 0.0],
                });
            }
            mesh.indices
                .extend_from_slice(&[s, s + 1, s + 2, s + 2, s + 3, s]);
        }
        graphics.push(Graphic {
            node: t.node,
            mesh,
            texture: 1 << 40 | t.params.font as u64,
            maskable: parts.maskable.get(&t.node).copied().unwrap_or(true),
            cull_transparent: parts
                .cull_transparent
                .get(&t.node)
                .copied()
                .unwrap_or(false),
            kind: "Text".into(),
        });
    }
    let input = DrawInput {
        canvases: parts.canvases,
        groups: parts.groups,
        masks: parts.masks,
        graphics,
    };
    Ok((draw_list(&tree, &input), parts.unknown_layers))
}

fn trees(assets: &Assets) -> Result<Vec<UiTree>> {
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

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let tags = sn_assets::physics_settings(&Assets::index(game)?)?.tags;
    println!(
        "sorting layers (TagManager order): {:?}",
        tags.sorting_layers
    );
    let mut hashes: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    let mut failed = 0;
    for (run, h) in hashes.iter_mut().enumerate() {
        // Everything from scratch each run.
        let assets = Assets::index(game)?;
        let assemblies = sn_assets::ScriptAssemblies::load(game)?;
        let layouts = assemblies.layouts()?;
        let fonts = assets.font_assets(&assemblies)?;
        let set = assets.tmp_font_set(&fonts)?;
        for ui in trees(&assets)? {
            let (d, unknown_layers) =
                build(&assets, &layouts, &ui, &fonts, &set, &tags.sorting_layers)?;
            h.push(d.hash());
            if run > 0 {
                continue;
            }
            if !unknown_layers.is_empty() {
                println!(
                    "  {}: sorting layer ids not in the TagManager: {unknown_layers:?}",
                    ui.key
                );
            }
            let vertices: usize = d.items.iter().map(|i| i.vertices.len()).sum();
            let mut by_canvas: BTreeMap<String, usize> = BTreeMap::new();
            let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
            let mut textures = BTreeSet::new();
            for it in &d.items {
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
                    failed += 1;
                    println!("  NOT FINITE: {}", ui.nodes[it.node].path);
                }
            }
            let clipped = d.items.iter().filter(|i| i.clip.is_some()).count();
            let faded = d.items.iter().filter(|i| i.alpha < 1.0).count();
            println!(
                "{}: {} draws, {vertices} vertices, {} textures; clipped {clipped}, culled {}, transparent {}, alpha < 1: {faded}, without a canvas {}; hash {:016x}",
                ui.key,
                d.items.len(),
                textures.len(),
                d.culled,
                d.transparent,
                d.without_canvas,
                d.hash()
            );
            if !d.items.is_empty() {
                println!("  by draw group: {by_canvas:?}");
                println!("  by kind: {by_kind:?}");
            }
            let order: Vec<&str> = {
                let mut seen = Vec::new();
                for it in &d.items {
                    let p = ui.nodes[it.canvas].path.as_str();
                    if seen.last() != Some(&p) {
                        seen.push(p);
                    }
                }
                seen
            };
            if order.len() > 1 {
                println!("  draw groups in order: {order:?}");
            }
        }
    }
    let same = hashes[0] == hashes[1];
    println!(
        "two runs from scratch: {} of {} draw-list hashes equal; non-finite {failed} ({:.1} s)",
        hashes[0]
            .iter()
            .zip(&hashes[1])
            .filter(|(a, b)| a == b)
            .count(),
        hashes[0].len(),
        start.elapsed().as_secs_f64()
    );
    Ok(if same && failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
