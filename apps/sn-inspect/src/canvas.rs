//! `ui --canvas` (M9u5b, `docs/formats/ui.md` § Canvas placement): where
//! each `uGUI_CanvasScaler` puts its canvas on several screens, with the
//! game's UI camera (origin, no rotation, the main camera's field of view),
//! and where the canvas's corners land on the screen through that camera.
//! Then each tree is laid out and drawn at those canvas sizes.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases};
use sn_install::GameData;
use sn_ui::scaler::{
    DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, MODE_STATIC, Placement, UiCamera, place,
};

use crate::Result;

/// The screens tried: the plan's two, a 16:10 one and an ultrawide one.
const SCREENS: [[i32; 2]; 4] = [[1920, 1080], [2560, 1440], [1920, 1200], [3440, 1440]];

/// Expected before the first run (`docs/DESIGN.md` M9u5b): each Static
/// canvas's corners land on the screen's corners within this many pixels.
const CORNER_PIXELS: f32 = 0.5;

/// The main camera's planes in the main scene (`scene main --tree`); the
/// UI camera takes them (`SNCameraRoot.Awake`).
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;

fn mode_name(mode: i32) -> &'static str {
    match mode {
        0 => "Static",
        1 => "Inversed",
        2 => "Parented",
        3 => "World",
        _ => "?",
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    println!(
        "UI camera: at {:?}, rotation {:?}, field of view {} (the main camera's), near {}, far {}; UI scale {DEFAULT_UI_SCALE}",
        camera.position, camera.rotation, camera.field_of_view, camera.near, camera.far
    );
    let mut failed = 0;
    let mut worst: f32 = 0.0;
    let mut placed_count = 0;
    for ui in crate::draw::trees(&assets)? {
        let scalers = assets.ui_scalers(&ui)?;
        if scalers.is_empty() {
            continue;
        }
        let layout = assets.ui_layout(&layouts, &ui)?.tree;
        println!("{}:", ui.key);
        let mut by_screen: Vec<HashMap<usize, Placement>> = vec![HashMap::new(); SCREENS.len()];
        for (node, s) in &scalers {
            let n = &layout.nodes[*node];
            let stored = layout.rect(*node);
            println!(
                "  {}: {} mode, distance {}, reference {:?}, scale mode {}; stored rect {}×{}, pivot {:?}, scale {:?}",
                ui.nodes[*node].path,
                mode_name(s.mode),
                s.distance,
                s.reference_resolution,
                s.scale_mode,
                stored.width,
                stored.height,
                n.rect.pivot,
                n.rect.scale
            );
            if s.mode != MODE_STATIC {
                println!(
                    "    not placed: {} follows an anchor (M9u8a)",
                    mode_name(s.mode)
                );
                continue;
            }
            for (k, screen) in SCREENS.iter().enumerate() {
                let p = match place(s, &camera, DEFAULT_FIELD_OF_VIEW, *screen, DEFAULT_UI_SCALE) {
                    Ok(p) => p,
                    Err(e) => {
                        failed += 1;
                        println!("    {screen:?}: FAILED {e}");
                        continue;
                    }
                };
                let want = [
                    [0.0, 0.0],
                    [0.0, screen[1] as f32],
                    [screen[0] as f32, screen[1] as f32],
                    [screen[0] as f32, 0.0],
                ];
                let mut err: f32 = 0.0;
                let mut landed = Vec::new();
                for (c, w) in p.corners(n.rect.pivot).iter().zip(want) {
                    match camera.project(*c, *screen) {
                        Some(px) => {
                            err = err.max((px[0] - w[0]).abs()).max((px[1] - w[1]).abs());
                            landed.push([px[0], px[1]]);
                        }
                        None => err = f32::INFINITY,
                    }
                }
                worst = worst.max(err);
                if err > CORNER_PIXELS {
                    failed += 1;
                }
                placed_count += 1;
                println!(
                    "    {}×{}: at {:?}, size {:.2}×{:.2}, scale {:.7}, factor {:.4}; corners on screen {:?}, off by {err:.4} px{}",
                    screen[0],
                    screen[1],
                    p.position,
                    p.size[0],
                    p.size[1],
                    p.scale[0],
                    p.scale_factor,
                    landed
                        .iter()
                        .map(|q| [
                            (q[0] * 100.0).round() / 100.0,
                            (q[1] * 100.0).round() / 100.0
                        ])
                        .collect::<Vec<_>>(),
                    if err > CORNER_PIXELS { "  FAILED" } else { "" }
                );
                by_screen[k].insert(*node, p);
            }
        }
        // The tree laid out and drawn at each screen's canvas sizes.
        for (k, screen) in SCREENS.iter().enumerate() {
            if by_screen[k].is_empty() {
                continue;
            }
            let d = assets.ui_draw_placed(
                &layouts,
                &ui,
                &fonts,
                &set,
                &mut atlases,
                &tags.sorting_layers,
                &by_screen[k],
                &sn_assets::UiStartState::default(),
            )?;
            let vertices: usize = d.list.items.iter().map(|i| i.vertices.len()).sum();
            let not_finite = d
                .list
                .items
                .iter()
                .filter(|i| {
                    i.vertices
                        .iter()
                        .any(|v| !(v.pos[0].is_finite() && v.pos[1].is_finite()))
                })
                .count();
            failed += not_finite;
            // Draws reaching past their draw group's canvas rect (logged:
            // panels can sit off screen on purpose).
            let mut outside = 0;
            for it in &d.list.items {
                let Some(p) = by_screen[k].get(&it.canvas) else {
                    continue;
                };
                let pivot = layout.nodes[it.canvas].rect.pivot;
                let (x0, y0) = (-pivot[0] * p.size[0], -pivot[1] * p.size[1]);
                let (x1, y1) = (x0 + p.size[0], y0 + p.size[1]);
                if it.vertices.iter().any(|v| {
                    v.pos[0] < x0 - 0.5
                        || v.pos[0] > x1 + 0.5
                        || v.pos[1] < y0 - 0.5
                        || v.pos[1] > y1 + 0.5
                }) {
                    outside += 1;
                    if k == 0 {
                        println!(
                            "    reaches past its canvas: {} ({})",
                            ui.nodes[it.node].path, it.kind
                        );
                    }
                }
            }
            println!(
                "  drawn at {}×{}: {} draws, {vertices} vertices, non-finite {not_finite}, reaching past their canvas {outside}; hash {:016x}",
                screen[0],
                screen[1],
                d.list.items.len(),
                d.list.hash()
            );
        }
    }
    println!(
        "placed {placed_count} canvas×screen pairs; worst corner {worst:.4} px (expected <= {CORNER_PIXELS}); failures {failed} ({:.1} s)",
        start.elapsed().as_secs_f64()
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
