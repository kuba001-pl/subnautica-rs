//! `ui --icon-mesh` (M9u7c1, `docs/formats/ui.md` § Picked-up icons):
//! nodes made by code and `uGUI_Icon` on the real `uGUI`. A
//! `NotifierIcon` (`new GameObject` + `uGUI_ItemIcon`, its foreground's
//! `uGUI_Icon`) is grafted under `uGUI_IconNotifier.canvas` for each of a
//! list of tech types, given its sprite (`SpriteManager.Get(techType)`)
//! and sized as `SetSize(iconSize)` does, and drawn:
//! - one item per icon, of the foreground, with `iconMaterial` (`UI/Icon`,
//!   no keyword, `_SrcFactor`/`_DstFactor` 5/10) and the sprite's page;
//! - its vertices are the sprite's `vertices` × (64 ÷ 128 × 100) about one
//!   centre, which moves with the icon's anchored position; its indices
//!   the sprite's `triangles`;
//! - every UV inside the sprite's packed rect in its page (to a pixel: the
//!   game's sprite vertices are stored to 0.01 units);
//! - `SetAlpha(0.5)` halves the vertex alpha;
//! - the stored nodes draw exactly as before.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, LocalTransform, UiScene, UiState};
use sn_install::GameData;
use sn_ui::icon::fit;
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// Tech types whose icons are drawn (the last has none: `Unknown`).
const TECH: [&str; 21] = [
    "Titanium",
    "Copper",
    "Quartz",
    "Lead",
    "Silver",
    "Gold",
    "Diamond",
    "Lithium",
    "Salt",
    "Magnetite",
    "Nickel",
    "Kyanite",
    "AluminumOxide",
    "UraniniteCrystal",
    "Bladderfish",
    "Peeper",
    "FirstAidKit",
    "Knife",
    "Scanner",
    "Flashlight",
    "NotATechType",
];
const SPACING: f32 = 70.0;

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let sprites = assets.sprites(&catalog)?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let start = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };
    let before = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    let stored = scene.node_count();

    let config = assets.notifier_config(&layouts, &tree)?;
    let canvas = config.canvas;
    println!(
        "uGUI_IconNotifier: canvas {}, interval {} s, iconSize {}, {} animators",
        tree.nodes[canvas].path,
        config.interval,
        config.icon_size,
        config.animators.len()
    );
    for (k, a) in config.animators.iter().enumerate() {
        println!(
            "  animator {k}: duration {} s, keys x {} y {} scale {} alpha {}, oscillate {} (offset {}, reduction {}, frequency {}, scale {})",
            a.duration,
            a.x.keys.len(),
            a.y.keys.len(),
            a.scale.keys.len(),
            a.alpha.keys.len(),
            a.oscillate,
            a.osc_offset,
            a.osc_reduction,
            a.osc_frequency,
            a.osc_scale
        );
    }
    let material = assets.icon_material(&catalog, &layouts)?;
    println!(
        "iconMaterial: shader {}, keywords {:?}, _SrcFactor {}, _DstFactor {}, {} floats, {} vectors, {} textures",
        material.shader,
        material.keywords,
        material.float("_SrcFactor"),
        material.float("_DstFactor"),
        material.floats.len(),
        material.colors.len(),
        material.textures.len()
    );
    check(
        material.shader == sn_ui::icon::SHADER
            && material.keywords.is_empty()
            && material.float("_SrcFactor") == 5.0
            && material.float("_DstFactor") == 10.0,
        "iconMaterial is UI/Icon with its defaults, no keyword",
    );
    let layer = tree.nodes[canvas].layer;
    let template = UiScene::from_code(
        "NotifierIcon",
        sn_assets::notifier_icon_nodes(layer, &material),
    )?;

    let mut state = start.clone();
    // (tech, icon root, foreground, sprite's mesh, page, packed rect).
    let mut icons = Vec::new();
    for (k, tech) in TECH.iter().enumerate() {
        let s = sprites
            .item_icon(tech)
            .ok_or_else(|| format!("{tech}: no icon and no Unknown"))?;
        let r = scene.graft(&template, canvas)?;
        let fg = r + 1;
        let (mesh, page) = assets.icon_sprite(&s.object)?;
        let id = scene.sprite_id(&s.object);
        state.active.insert(r, true);
        state.sprite.insert(fg, Some(id));
        state.enabled.insert(fg, true);
        state.transform.insert(
            r,
            LocalTransform {
                anchored_position: Some([100.0 + SPACING * k as f32, 300.0]),
                size_delta: Some([config.icon_size; 2]),
                ..LocalTransform::default()
            },
        );
        state.transform.insert(
            fg,
            LocalTransform {
                size_delta: Some(fit([config.icon_size; 2], mesh.size, true)),
                ..LocalTransform::default()
            },
        );
        icons.push((
            tech,
            r,
            fg,
            mesh,
            page,
            s.packed.texture_rect,
            s.sprite.name.clone(),
        ));
    }
    check(
        scene.node_count() == stored + 2 * TECH.len(),
        "two nodes appended per icon",
    );
    // The last icon at half alpha.
    let half = icons.last().map(|i| i.2).unwrap_or(0);
    state.renderer_alpha.insert(half, 0.5);
    let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
    let (mut one_item, mut material_ok, mut mesh_ok, mut page_ok) = (0, 0, 0, 0);
    let (mut worst_uv_px, mut worst_reach, mut worst_spread) = (0.0f32, 0.0f32, 0.0f32);
    let mut centres = Vec::new();
    for (tech, r, fg, mesh, page, packed, name) in &icons {
        let items: Vec<_> = d.list.items.iter().filter(|i| i.node == *fg).collect();
        let none_of_root = !d.list.items.iter().any(|i| i.node == *r);
        if items.len() == 1 && none_of_root {
            one_item += 1;
        }
        let Some(item) = items.first() else {
            println!("  {tech}: not drawn");
            continue;
        };
        let m = &d.materials[item.material];
        if m.shader == sn_ui::icon::SHADER && m.keywords.is_empty() {
            material_ok += 1;
        }
        let page_key = page.as_ref().map_or(0, sn_assets::texture_key);
        if item.texture == page_key && page_key != 0 {
            page_ok += 1;
        }
        // Each drawn vertex is the sprite's × 50 about one centre.
        let k = config.icon_size / mesh.size[0] * mesh.pixels_per_unit;
        let mut c = Vec::new();
        for (v, s) in item.vertices.iter().zip(&mesh.vertices) {
            c.push([v.pos[0] - s[0] * k, v.pos[1] - s[1] * k]);
            worst_reach = worst_reach.max((s[0] * k).abs()).max((s[1] * k).abs());
        }
        let c0 = c.first().copied().unwrap_or([f32::NAN; 2]);
        for p in &c {
            worst_spread = worst_spread
                .max((p[0] - c0[0]).abs())
                .max((p[1] - c0[1]).abs());
        }
        centres.push(c0);
        let indices: Vec<u32> = mesh.triangles.iter().map(|&i| u32::from(i)).collect();
        if item.vertices.len() == mesh.vertices.len() && item.indices == indices {
            mesh_ok += 1;
        }
        // UVs against the packed rect, in pixels of the page.
        let size = match page {
            Some(p) => {
                let (_, td) = p.data()?;
                let t = sn_unity::Texture2D::parse(td, p.file.file().big_endian)
                    .map_err(|e| format!("Texture2D: {e}"))?;
                [t.width as f32, t.height as f32]
            }
            None => [1.0, 1.0],
        };
        let mut out = 0.0f32;
        for v in &item.vertices {
            let px = [v.uv[0] * size[0], v.uv[1] * size[1]];
            let (x0, y0) = (packed[0], packed[1]);
            let (x1, y1) = (x0 + packed[2], y0 + packed[3]);
            out = out
                .max(x0 - px[0])
                .max(px[0] - x1)
                .max(y0 - px[1])
                .max(px[1] - y1);
        }
        worst_uv_px = worst_uv_px.max(out);
        println!(
            "  {tech:<16} sprite {name:<18} {:>3} vertices {:>3} indices, page {}×{}, UVs at most {:.3} px outside the packed rect",
            item.vertices.len(),
            item.indices.len(),
            size[0],
            size[1],
            out.max(0.0)
        );
    }
    let n = icons.len();
    println!(
        "{one_item} of {n} icons draw one item (the foreground's); material {material_ok} of {n}; mesh {mesh_ok} of {n}; page {page_ok} of {n}"
    );
    check(one_item == n, "each icon draws one item, its foreground's");
    check(material_ok == n, "each draws with UI/Icon, no keyword");
    check(
        mesh_ok == n,
        "each draws the sprite's vertices and triangles",
    );
    check(page_ok == n, "each samples its sprite's page");
    println!(
        "vertices about one centre: spread {worst_spread:.5} canvas units; reach {worst_reach:.3} (≤ {} of the 64 fit); UVs outside the packed rect at most {worst_uv_px:.3} px",
        config.icon_size / 2.0
    );
    check(
        worst_spread < 1e-3,
        "each icon's vertices are the sprite's × 50 about one centre",
    );
    check(
        worst_reach <= config.icon_size / 2.0 + 1e-3,
        "every vertex inside the 64 × 64 fit",
    );
    check(
        worst_uv_px <= 1.0,
        "every UV inside its packed rect (to a pixel)",
    );
    // The centres move as the anchored positions (canvas units: the canvas
    // is at scale 1 in its root, `ui --canvas`).
    let mut moved = 0.0f32;
    for (k, c) in centres.iter().enumerate() {
        let want = [centres[0][0] + SPACING * k as f32, centres[0][1]];
        moved = moved
            .max((c[0] - want[0]).abs())
            .max((c[1] - want[1]).abs());
    }
    println!("centres follow the anchored positions to {moved:.5}");
    check(moved < 1e-3, "the centres follow the anchored positions");
    let alpha = d
        .list
        .items
        .iter()
        .find(|i| i.node == half)
        .and_then(|i| i.vertices.first())
        .map(|v| v.color[3]);
    println!("SetAlpha(0.5): vertex alpha {alpha:?}");
    check(
        alpha == Some(128),
        "SetAlpha(0.5) halves the vertex alpha (255 → 128)",
    );
    // Hidden again, the stored nodes draw as before.
    let after = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    let old: Vec<_> = after
        .list
        .items
        .iter()
        .filter(|i| i.node < stored)
        .collect();
    check(
        old.len() == before.list.items.len()
            && old.iter().zip(&before.list.items).all(|(x, y)| **x == *y)
            && after.list.items.len() == old.len(),
        "the stored nodes draw exactly as before; hidden icons draw nothing",
    );
    println!(
        "{} stored items before, {} after, {} in all; checks {ok} of {all}",
        before.list.items.len(),
        old.len(),
        after.list.items.len()
    );
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
