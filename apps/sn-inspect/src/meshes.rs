//! `ui --meshes` (M9u4b, `docs/formats/ui.md` § Graphic meshes): the
//! meshes of every active, enabled `Image` and `RawImage` of the in-scope
//! trees, built on the rects M9u4a's layout gives. Logged per kind:
//! graphics, vertices, triangles; checked: every vertex inside its rect,
//! every index valid, UVs inside the sprite's packed rectangle (but for
//! texture repeat).

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, ScriptAssemblies, UiGraphicKind, UiTree};
use sn_install::GameData;
use sn_ui::layout::layout;
use sn_ui::mesh::{MeshKind, image_mesh, raw_image_mesh};

use crate::Result;

const EPS: f32 = 1e-3;

#[derive(Default)]
struct Totals {
    /// Kind → (graphics, vertices, triangles).
    by_kind: BTreeMap<String, (usize, usize, usize)>,
    unknown_rect: usize,
    other: BTreeMap<String, usize>,
    text: usize,
    failures: Vec<String>,
    textures: BTreeSet<(std::path::PathBuf, String, i64)>,
    atlas_sprites: usize,
}

fn tree_meshes(
    assets: &Assets,
    layouts: &sn_assets::Layouts,
    ui: &UiTree,
    t: &mut Totals,
) -> Result<()> {
    let mut tree = assets.ui_layout(layouts, ui)?.tree;
    layout(&mut tree);
    let graphics = assets.ui_graphics(layouts, ui)?;
    let (mut built, mut verts) = (0, 0);
    for g in &graphics {
        if !g.enabled || !tree.active(g.node) {
            continue;
        }
        let path = &ui.nodes[g.node].path;
        let rect = tree.rect(g.node);
        let pivot = tree.nodes[g.node].rect.pivot;
        let (kind, mesh, outer) = match &g.kind {
            UiGraphicKind::Text => {
                t.text += 1;
                continue;
            }
            UiGraphicKind::Other(name) => {
                *t.other.entry(name.clone()).or_default() += 1;
                continue;
            }
            UiGraphicKind::CircularBar { .. } => {
                *t.other.entry("uGUI_CircularBar".into()).or_default() += 1;
                continue;
            }
            UiGraphicKind::Compass { .. } => {
                *t.other.entry("uGUI_Compass".into()).or_default() += 1;
                continue;
            }
            _ if !(rect.width.is_finite() && rect.height.is_finite()) => {
                t.unknown_rect += 1;
                continue;
            }
            UiGraphicKind::Image { params, texture } => {
                if let Some(tex) = texture {
                    t.textures.insert(tex.key());
                }
                if params.sprite.is_some_and(|s| s.packed) {
                    t.atlas_sprites += 1;
                }
                let (kind, mesh) = image_mesh(rect, pivot, params, tree.reference_pixels_per_unit);
                let outer = match kind {
                    MeshKind::Plain | MeshKind::TiledRepeat => None,
                    _ => params.sprite.map(|s| s.outer_uv),
                };
                (format!("Image {kind:?}"), mesh, outer)
            }
            UiGraphicKind::RawImage {
                color,
                uv_rect,
                texture,
            } => {
                if let Some(tex) = texture {
                    t.textures.insert(tex.key());
                }
                // Without a texture `mainTexture` is the white texture.
                (
                    "RawImage".to_string(),
                    raw_image_mesh(rect, *color, *uv_rect, true),
                    None,
                )
            }
        };
        for v in &mesh.vertices {
            let inside = v.pos[0] >= rect.x - EPS
                && v.pos[0] <= rect.x + rect.width + EPS
                && v.pos[1] >= rect.y - EPS
                && v.pos[1] <= rect.y + rect.height + EPS;
            if !inside {
                t.failures.push(format!(
                    "{path} ({kind}): vertex {:?} outside {rect:?}",
                    v.pos
                ));
                break;
            }
            if let Some(o) = outer {
                let (u0, u1) = (o[0].min(o[2]), o[0].max(o[2]));
                let (w0, w1) = (o[1].min(o[3]), o[1].max(o[3]));
                if v.uv[0] < u0 - EPS
                    || v.uv[0] > u1 + EPS
                    || v.uv[1] < w0 - EPS
                    || v.uv[1] > w1 + EPS
                {
                    t.failures
                        .push(format!("{path} ({kind}): uv {:?} outside {o:?}", v.uv));
                    break;
                }
            }
        }
        if mesh.indices.len() % 3 != 0
            || mesh
                .indices
                .iter()
                .any(|&i| i as usize >= mesh.vertices.len())
        {
            t.failures.push(format!("{path} ({kind}): bad indices"));
        }
        let e = t.by_kind.entry(kind).or_default();
        e.0 += 1;
        e.1 += mesh.vertices.len();
        e.2 += mesh.indices.len() / 3;
        built += 1;
        verts += mesh.vertices.len();
    }
    println!(
        "{}: {} graphics, {built} meshes built, {verts} vertices",
        ui.key,
        graphics.len()
    );
    Ok(())
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let mut trees = vec![assets.ui_tree("uGUI.prefab", &root)?];
    for o in crate::ui::other_roots(&assets, &root)? {
        let mut t = assets.ui_tree("other root", &o)?;
        t.key = format!(
            "uGUI.prefab root {}",
            t.nodes.first().map_or("", |n| n.name.as_str())
        );
        trees.push(t);
    }
    let main = assets.scene("main")?;
    let (screen, _) = assets.pda_screen_refs(&main)?;
    trees.push(assets.ui_tree("PDA screen", &screen)?);
    let mut t = Totals::default();
    for ui in &trees {
        tree_meshes(&assets, &layouts, ui, &mut t)?;
    }
    for (kind, (n, v, tri)) in &t.by_kind {
        println!("  {kind}: {n} graphics, {v} vertices, {tri} triangles");
    }
    println!(
        "  not built: {} with a rect that needs text layout (M9u4c), {} TMP texts (M9u4c), the game's own graphics {:?}",
        t.unknown_rect, t.text, t.other
    );
    println!(
        "  {} textures drawn from ({} images with an atlas-packed sprite)",
        t.textures.len(),
        t.atlas_sprites
    );
    for f in &t.failures {
        println!("  FAILS: {f}");
    }
    println!(
        "{} meshes fail a check ({:.1} s)",
        t.failures.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(if t.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
