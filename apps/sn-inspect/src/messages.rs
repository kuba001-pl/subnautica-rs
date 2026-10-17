//! `ui --messages` (M9u7b, `docs/formats/ui.md` § Messages): `ErrorMessage`
//! read from the game (its fields, the times in its constructor,
//! `OnLateUpdate`'s constants, `prefabMessage`), then its rules
//! (`sn_ui::messages`) through a scripted timeline of the game's own
//! strings, checked every frame against a replay written here apart from
//! the rules, in doubles:
//! - one message; the same text three more times (two in one frame:
//!   `(x2)`…`(x4)`, its end renewed without the flight); four at once (the
//!   pool of four used up) and a fifth (four more copies made); expiry
//!   while others stay, the ones below sliding up (`offsetY`); a message
//!   after all have gone (an entry reused from the pool);
//! - each frame: the messages (entry, text, count, end), the pool, every
//!   shown entry's position (the flight from the centre, `EaseOutSine`,
//!   the column) and alpha (the fade line, `EaseOutSine`).
//!
//! The frame time is 0.0173 s so that no message's end falls within a
//! float's error of a frame's time (an exact tie would be decided by
//! rounding, in the game too).
//!
//! Every 10th frame the scene is drawn with the copies grafted: each shown
//! entry draws its text's visible characters as quads, at the alpha the
//! rules set; a hidden one draws nothing; the stored nodes draw as before.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_ui::messages::{MessageInputs, Messages};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 0.0173;
const FRAMES: usize = 900;

/// The script: (frame, made before the time step, language key).
const SCRIPT: &[(usize, bool, &str)] = &[
    (10, false, "YouDied"),
    (120, true, "InventoryFull"),
    (150, true, "InventoryFull"),
    (160, true, "InventoryFull"),
    (160, true, "InventoryFull"),
    (200, false, "YouDiedLostStuff"),
    (200, false, "InventoryCantDropHere"),
    (230, false, "Loading"),
    (700, false, "InventoryFull"),
];

/// The replay's message.
struct Msg {
    /// Its entry, by order made.
    entry: usize,
    text: String,
    num: i32,
    end: f64,
}

/// `ErrorMessage` again, in doubles, entries by the order they are made.
struct Replay {
    time: f64,
    msgs: Vec<Msg>,
    pool: Vec<usize>,
    made: usize,
    offset_y: f64,
    shown: HashMap<usize, String>,
    /// Each shown entry's local position and alpha this frame.
    out: HashMap<usize, ([f64; 2], f64)>,
}

fn ease(v: f64) -> f64 {
    (v * std::f64::consts::PI * 0.5).sin()
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    y1 + (y2 - y1) * (x - x1) / (x2 - x1)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let layers = &tags.sorting_layers;
    let code = sn_assets::read_assembly(game)?;
    let language = sn_assets::language(game, "English")?;
    let start = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, layers)?;
    let (config, template) = assets.messages_config(&layouts, &tree, &code, &fonts, layers)?;
    let t = config.times;
    println!(
        "ErrorMessage: messageCanvas {}; offset {:?}, ySpacing {}; times (constructor) fly-in {} s, delay {} s, fade-out {} s, invisible {} s; OnLateUpdate offsetY × {}, snap below {}; prefabMessage {} ({} nodes), m_fontColor {:?}",
        tree.nodes[config.canvas].path,
        config.offset,
        config.y_spacing,
        t.fly_in,
        t.delay,
        t.fade_out,
        t.invisible,
        t.slide_rate,
        t.snap,
        template.tree.nodes[0].name,
        template.node_count(),
        config.font_color
    );
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    let mut scale_modes = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        scale_modes.insert(node, s.scale_mode);
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let layout = sn_assets::message_layout(
        &assets,
        &mut scene,
        &set,
        &placements,
        &start,
        &config,
        &template,
        &scale_modes,
        DEFAULT_UI_SCALE,
    )?;
    println!(
        "layout at {}×{}: messageCanvas rect {:?}; entry width {}, anchor point {:?}; inverse scale {}",
        SCREEN[0],
        SCREEN[1],
        layout.canvas_rect,
        layout.entry_width,
        layout.anchor_base,
        layout.inverse_scale
    );
    let r = template.stored_rect(0).ok_or("no rect")?;
    let measure = |text: &str| {
        template
            .preferred_size(&set, 0, text, layout.entry_width)
            .unwrap_or([0.0, 0.0])
    };
    let text_of = |key: &str| language.get(key).cloned().unwrap_or_else(|| key.into());
    println!(
        "prefabMessage rect: anchors {:?}–{:?}, pivot {:?}, size {:?}; preferred size of {:?}: {:?}",
        r.anchor_min,
        r.anchor_max,
        r.pivot,
        r.size_delta,
        text_of("InventoryFull"),
        measure(&text_of("InventoryFull"))
    );

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
    check(
        t.fly_in > 0.0 && t.delay > 0.0 && t.fade_out > 0.0 && t.invisible > 0.0,
        "the four times are read",
    );

    let mut messages = Messages::new(config.clone());
    let mut replay = Replay {
        time: 0.0,
        msgs: Vec::new(),
        pool: Vec::new(),
        made: 0,
        offset_y: 0.0,
        shown: HashMap::new(),
        out: HashMap::new(),
    };
    let (dt, tt) = (f64::from(DT), t);
    let (fly, delay, fade, invisible) = (
        f64::from(tt.fly_in),
        f64::from(tt.delay),
        f64::from(tt.fade_out),
        f64::from(tt.invisible),
    );
    let height = |text: &str| f64::from(measure(text)[1]);
    let add = |r: &mut Replay, text: String| {
        if let Some(m) = r.msgs.iter_mut().find(|m| m.text == text) {
            m.end = r.time + delay + fade + invisible;
            m.num += 1;
            r.shown.insert(m.entry, format!("{text} (x{})", m.num));
            return;
        }
        if r.pool.is_empty() {
            for _ in 0..4 {
                r.pool.push(r.made);
                r.made += 1;
            }
        }
        let entry = r.pool.pop().unwrap_or_default();
        r.shown.insert(entry, text.clone());
        r.msgs.push(Msg {
            entry,
            text,
            num: 1,
            end: r.time + fly + delay + fade + invisible,
        });
    };
    let (mut frame_checks, mut frame_bad) = (0usize, 0usize);
    let (mut draws, mut draw_bad) = (0usize, 0usize);
    let mut worst_pos = 0.0f64;
    let mut worst_alpha = 0.0f64;
    let mut most = 0usize;
    let mut events: Vec<String> = Vec::new();
    let mut was: Vec<String> = Vec::new();
    for frame in 0..FRAMES {
        let mut inputs = MessageInputs {
            delta_time: DT,
            ..MessageInputs::default()
        };
        for &(_, early, key) in SCRIPT.iter().filter(|s| s.0 == frame) {
            if early {
                inputs.early.push(text_of(key));
            } else {
                inputs.late.push(text_of(key));
            }
        }
        // The rules, grafting copies as `GetEntry` asks.
        {
            let scene = &mut scene;
            let template = &template;
            let mut make = || scene.graft(template, config.canvas).unwrap_or(usize::MAX);
            messages.update(&inputs, &layout, &measure, &mut make);
        }
        // The replay.
        for m in &inputs.early {
            add(&mut replay, m.clone());
        }
        replay.time += dt;
        for m in &inputs.late {
            add(&mut replay, m.clone());
        }
        let time = replay.time;
        let mut i = 0;
        while i < replay.msgs.len() {
            if time > replay.msgs[i].end {
                let m = replay.msgs.remove(i);
                replay.offset_y += height(&replay.shown[&m.entry]);
                replay.pool.push(m.entry);
            }
            i += 1;
        }
        replay.offset_y -= replay.offset_y * f64::from(tt.slide_rate) * dt;
        if replay.offset_y < f64::from(tt.snap) {
            replay.offset_y = 0.0;
        }
        replay.out.clear();
        let rect = layout.canvas_rect;
        let inv = f64::from(layout.inverse_scale);
        let mut above = 0.0;
        for (j, m) in replay.msgs.iter().enumerate() {
            let s = measure(&replay.shown[&m.entry]);
            let (w, h) = (f64::from(s[0]), f64::from(s[1]));
            let a = [-0.5 * f64::from(layout.entry_width).min(w), 0.5 * h];
            let y = -(f64::from(config.offset[1]) * inv
                + replay.offset_y
                + above
                + j as f64 * f64::from(config.y_spacing));
            above += h;
            let b = [
                f64::from(rect.x) + f64::from(config.offset[0]) * inv,
                -f64::from(rect.y) + y,
            ];
            let k = ease(
                line(
                    m.end - invisible - fade - delay - fly,
                    0.0,
                    m.end - invisible - fade - delay,
                    1.0,
                    time,
                )
                .clamp(0.0, 1.0),
            );
            let alpha = ease(
                line(m.end - invisible - fade, 1.0, m.end - invisible, 0.0, time).clamp(0.0, 1.0),
            );
            replay.out.insert(
                m.entry,
                ([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k], alpha),
            );
        }

        // Compare.
        frame_checks += 1;
        let node_of = |k: usize| messages.entries.get(k).copied();
        let mut good = messages.messages.len() == replay.msgs.len()
            && messages.pool.len() == replay.pool.len()
            && messages.entries.len() == replay.made
            && (f64::from(messages.offset_y) - replay.offset_y).abs() < 1e-2;
        for (m, r) in messages.messages.iter().zip(&replay.msgs) {
            good &= Some(m.entry) == node_of(r.entry)
                && m.text == r.text
                && m.num == r.num
                && (f64::from(m.time_end) - r.end).abs() < 1e-3
                && messages.state.text.get(&m.entry) == replay.shown.get(&r.entry)
                && messages.state.active.get(&m.entry) == Some(&true);
            let (p, alpha) = replay.out[&r.entry];
            let Some(ap) = messages
                .state
                .transform
                .get(&m.entry)
                .and_then(|t| t.anchored_position)
            else {
                good = false;
                continue;
            };
            let local = [
                f64::from(ap[0] + layout.anchor_base[0]),
                f64::from(ap[1] + layout.anchor_base[1]),
            ];
            let e = (local[0] - p[0]).abs().max((local[1] - p[1]).abs());
            let ea = (f64::from(messages.state.color[&m.entry][3]) - alpha).abs();
            worst_pos = worst_pos.max(e);
            worst_alpha = worst_alpha.max(ea);
            good &= e < 1e-2 && ea < 1e-3;
        }
        for &e in &messages.pool {
            good &= messages.state.active.get(&e) == Some(&false);
        }
        if !good {
            frame_bad += 1;
            if frame_bad <= 5 {
                println!(
                    "  frame {frame}: rules {} messages, pool {}, offsetY {:.3}; replay {}, pool {}, offsetY {:.3}",
                    messages.messages.len(),
                    messages.pool.len(),
                    messages.offset_y,
                    replay.msgs.len(),
                    replay.pool.len(),
                    replay.offset_y
                );
            }
        }
        most = most.max(messages.messages.len());
        let now: Vec<String> = messages
            .messages
            .iter()
            .map(|m| messages.state.text[&m.entry].clone())
            .collect();
        if now != was {
            events.push(format!(
                "{:.3} s (frame {frame}): {now:?}, pool {} of {}",
                messages.time,
                messages.pool.len(),
                messages.entries.len()
            ));
            was = now;
        }

        // A draw every 10th frame.
        if frame % 10 == 0 {
            draws += 1;
            let mut state = start.clone();
            state.apply(&messages.state);
            let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
            let mut good = true;
            for &e in &messages.entries {
                let items: Vec<_> = d.list.items.iter().filter(|i| i.node == e).collect();
                let shown = messages.messages.iter().any(|m| m.entry == e);
                if !shown {
                    good &= items.is_empty();
                    continue;
                }
                let text = &messages.state.text[&e];
                let visible = text.chars().filter(|c| !c.is_whitespace()).count();
                let quads: usize = items.iter().map(|i| i.vertices.len() / 4).sum();
                let alpha = messages.state.color[&e][3];
                let want = sn_assets::color32([1.0, 1.0, 1.0, config.font_color[3] * alpha])[3];
                let top = items
                    .iter()
                    .flat_map(|i| i.vertices.iter().map(|v| v.color[3]))
                    .max()
                    .unwrap_or(0);
                // A fully clear text may be left out of the list.
                if alpha > 0.0 {
                    good &= quads == visible && top.abs_diff(want) <= 1;
                }
            }
            let old: Vec<_> = d.list.items.iter().filter(|i| i.node < stored).collect();
            good &= old.len() == before.list.items.len()
                && old.iter().zip(&before.list.items).all(|(x, y)| **x == *y);
            if !good {
                draw_bad += 1;
                if draw_bad <= 5 {
                    println!("  draw at frame {frame} differs");
                }
            }
        }
    }
    for e in &events {
        println!("  {e}");
    }
    println!(
        "frames {frame_checks}, differing {frame_bad}; largest position error {worst_pos:.5} canvas units, alpha {worst_alpha:.6}; at most {most} messages; {} entries made; draws {draws}, differing {draw_bad}",
        messages.entries.len()
    );
    check(frame_bad == 0, "every frame equals the replay");
    check(
        draw_bad == 0,
        "every draw shows the shown entries' texts at their alpha, nothing for the hidden ones, the stored nodes as before",
    );
    check(
        messages.entries.len() == 8,
        "the pool grew to eight (two chunks of four)",
    );
    check(most == 5, "five messages at most");
    check(
        events.iter().any(|e| e.contains("(x4)")),
        "an equal text counted to (x4)",
    );
    check(messages.messages.len() == 1, "one message at the end");
    println!("checks {ok} of {all}");
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
