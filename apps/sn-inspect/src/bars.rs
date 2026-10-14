//! `ui --bars` (M9u6b1, `docs/formats/ui.md` § The HUD's bars): the HUD's
//! four bars driven by their rules (`sn_ui::hud`) through a scripted dive,
//! with oxygen and health from our vitals rules (`sn_sim::vitals`), drawn
//! each frame as the client does. Each second, each bar's `_Value` in the
//! draw list and its text are compared with a replay written here from
//! the decompiled scripts (`SetValue`): equal, N/N. The oxygen overlays'
//! ST and alpha are compared with their formula. Draw time per frame
//! logged.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_sim::vitals::{Situation, Vitals, VitalsParams};
use sn_ui::draw::DrawList;
use sn_ui::hud::{BarKind, Hud, HudInputs, hud_mode};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_ui::tween::smooth_damp;

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// 60 frames a second, 75 seconds.
const DT: f32 = 1.0 / 60.0;
const SECONDS: usize = 75;

/// The dive: 5 s at the surface, down to 20 m until 60 s (a 20-point hit at
/// 30 s), then up.
fn height(params: &VitalsParams, t: f32) -> f64 {
    let surface = params.ocean_level + 1.0;
    if !(5.0..60.0).contains(&t) {
        surface
    } else {
        params.ocean_level - 20.0
    }
}

/// The replay: `SetValue` of each bar, as the decompiled scripts read.
#[derive(Default, Clone, Copy)]
struct Replay {
    curr: f32,
    vel: f32,
}

/// A material value of the bar's draw item.
fn bar_value(d: &sn_assets::UiDraw, node: usize, name: &str) -> Option<[f32; 4]> {
    let it = d.list.items.iter().find(|i| i.node == node)?;
    let m = d.materials.get(it.material)?;
    if let Some(f) = m.floats.get(name) {
        return Some([*f, 0.0, 0.0, 0.0]);
    }
    m.colors.get(name).copied()
}

/// Quads drawn for a text node (all its sub-meshes).
fn quads(list: &DrawList, node: usize) -> usize {
    list.items
        .iter()
        .filter(|i| i.node == node)
        .map(|i| i.vertices.len() / 4)
        .sum()
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let config = assets.hud_config(&layouts, &tree)?;
    let assembly = sn_assets::read_assembly(game)?;
    let (food, water) = sn_assets::survival_start(&assembly)?;
    let data = sn_assets::player_data(&assets)?;
    let code = sn_assets::player_code(&assembly)?;
    let vp = sn_assets::vitals_params(&data, &code);
    println!(
        "Survival.ResetStats: food {food}, water {water}; oxygen capacity {}, max health {}",
        vp.oxygen_capacity, vp.max_health
    );
    for b in &config.bars {
        println!(
            "  {:?}: {} (bar {}, text {}); dampSpeed {}; size {}, width {}, maxValue {}, border {}, round corners {}, overlays {:?}",
            b.kind,
            tree.nodes[b.root].path,
            tree.nodes[b.bar].name,
            tree.nodes[b.text].name,
            b.damp_speed,
            b.circular.size,
            b.circular.width,
            b.circular.max_value,
            b.circular.border,
            b.circular.round_corners,
            b.overlays
        );
    }
    // Which bars have an overlay texture (the `OVERLAY` keyword).
    for g in assets.ui_graphics(&layouts, &tree)? {
        if let sn_assets::UiGraphicKind::CircularBar {
            texture, overlay, ..
        } = &g.kind
        {
            println!(
                "  uGUI_CircularBar {}: _texture {}, _overlay {}",
                tree.nodes[g.node].path,
                if texture.is_some() {
                    "set"
                } else {
                    "none (white)"
                },
                if overlay.is_some() { "set" } else { "none" }
            );
        }
    }
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let start_state = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let mut hud = Hud::new(config.clone());
    let mut vitals = Vitals::new(&vp);
    let mut replay = [Replay::default(); 4];
    let (mut checked, mut differ) = (0, 0);
    let mut times = Vec::new();
    let mode = hud_mode(false, false, false);
    let frames = SECONDS * 60;
    for frame in 0..frames {
        let t = (frame + 1) as f32 * DT;
        let s = Situation {
            y: height(&vp, t),
            world_settled: true,
            ..Situation::default()
        };
        vitals.step(&vp, f64::from(DT), &s);
        if frame == 30 * 60 {
            vitals.take_damage(20.0, &mut Vec::new());
        }
        let inputs = HudInputs {
            active: true,
            mode,
            oxygen: vitals.oxygen as f32,
            oxygen_capacity: vp.oxygen_capacity as f32,
            health: vitals.health as f32,
            max_health: vp.max_health as f32,
            food,
            water,
            delta_time: DT,
            time: t,
            pda_open: false,
            healed: None,
            ate: None,
            drank: None,
            depth: 0.0,
            compass: false,
        };
        let mut state = start_state.clone();
        state.apply(&hud.update(&inputs));
        let begin = Instant::now();
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        times.push(begin.elapsed().as_secs_f64() * 1000.0);
        // The replay, every frame.
        for (b, r) in config.bars.iter().zip(replay.iter_mut()) {
            let target = match b.kind {
                BarKind::Oxygen => inputs.oxygen / inputs.oxygen_capacity,
                BarKind::Health => (inputs.health / inputs.max_health).clamp(0.0, 1.0),
                BarKind::Food => food / 100.0,
                BarKind::Water => water / 100.0,
            };
            r.curr = smooth_damp(r.curr, target, &mut r.vel, b.damp_speed, f32::INFINITY, DT);
        }
        if (frame + 1) % 60 != 0 {
            continue;
        }
        let mut line = format!(
            "t {:>2} s: oxygen {:.2}, health {:.0}",
            (frame + 1) / 60,
            vitals.oxygen,
            vitals.health
        );
        for (b, r) in config.bars.iter().zip(&replay) {
            let want_value = (r.curr * b.circular.max_value).clamp(0.0, 1.0);
            let want_text = match b.kind {
                BarKind::Oxygen => inputs.oxygen.round_ties_even() as i32,
                BarKind::Health => (r.curr * inputs.max_health).ceil() as i32,
                BarKind::Food | BarKind::Water => (r.curr * 100.0).ceil() as i32,
            }
            .to_string();
            let drawn = bar_value(&d, b.bar, "_Value").map(|v| v[0]);
            let text = state.text.get(&b.text).cloned().unwrap_or_default();
            let text_quads = quads(&d.list, b.text);
            let mut ok = drawn == Some(want_value)
                && text == want_text
                && text_quads == want_text.chars().count();
            if let Some(o) = b.overlays {
                let st1 = bar_value(&d, b.bar, "_Overlay1_ST");
                let alpha = bar_value(&d, b.bar, "_OverlayAlpha");
                let k = (2.0 * (1.0 - inputs.oxygen / inputs.oxygen_capacity)).clamp(0.0, 1.0);
                let want_st = {
                    let mut v = b.circular.overlay1_st;
                    v[3] = -t * o.speed[0];
                    v
                };
                let near = |a: f32, b: f32| (a - b).abs() <= 1e-6;
                ok &= st1 == Some(want_st)
                    && alpha.is_some_and(|a| {
                        near(a[0], (k * o.alpha[0]).clamp(0.0, 1.0))
                            && near(a[1], (k * o.alpha[1]).clamp(0.0, 1.0))
                    });
                line += &format!(
                    "; overlay ST.w {:.3}, alpha {:.3}",
                    st1.map_or(f32::NAN, |v| v[3]),
                    alpha.map_or(f32::NAN, |v| v[0])
                );
            }
            checked += 1;
            if !ok {
                differ += 1;
            }
            line += &format!(
                "; {:?} {:.4} \"{text}\" ({text_quads} quads){}",
                b.kind,
                drawn.unwrap_or(f32::NAN),
                if ok {
                    String::new()
                } else {
                    format!("  DIFFERS (want {want_value:.4} \"{want_text}\")")
                }
            );
        }
        println!("{line}");
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "{checked} bar checks, {differ} differ; draw mean {mean:.3} ms, p95 {:.3} ms, max {:.3} ms over {frames} frames; glyphs made {}; ({:.1} s)",
        sorted[sorted.len() * 95 / 100],
        sorted[sorted.len() - 1],
        atlases.made,
        start.elapsed().as_secs_f64()
    );
    Ok(if differ == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
