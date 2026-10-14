//! `ui --depth` (M9u6c1, `docs/formats/ui.md` § The depth and compass): the
//! depth readout driven by its rules (`sn_ui::depth`) through a scripted
//! timeline (a dive to 139 m and back up, the PDA open, the HUD inactive,
//! a compass flag), drawn every frame as the client does. Every frame is
//! checked against what the decompiled `uGUI_DepthCompass` gives, written
//! here apart from the rules:
//! - the player graphics are drawn iff the HUD is active and the PDA is
//!   closed;
//! - the depth text is the last shown `floor(depth)`'s string, drawn with
//!   one quad a digit, in `textColorNormal`; the suffix is the language
//!   file's `MeterSuffix`;
//! - the half moon draws as its compass sprite iff the compass flag was
//!   last on while shown (compared with a draw of that sprite);
//! - the shadow's material has the multiplicative blend, premultiplied;
//! - the compass's labels are drawn iff the compass is visible (shown and
//!   the flag on; their places and alphas are M9u6c2), the compass
//!   graphic never (its drawing is M9u6c2), the submersible graphics
//!   never.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases, UiDraw, UiState};
use sn_install::GameData;
use sn_ui::depth::DepthCompass;
use sn_ui::hud::{HudInputs, hud_mode};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 1.0 / 60.0;
const SECONDS: usize = 70;

/// The player's height: 1 m above the water until 2 s, down at 4 m/s to
/// −139 m at 37 s, then up at 3 m/s.
fn height(t: f32) -> f32 {
    if t < 2.0 {
        1.0
    } else if t < 37.0 {
        1.0 - 4.0 * (t - 2.0)
    } else {
        -139.0 + 3.0 * (t - 37.0)
    }
}

/// The PDA is open 10–14 s and 52–52.5 s; the HUD inactive 20–22 s; the
/// compass flag on 26–31 s and 50–55 s.
fn timeline(t: f32) -> (bool, bool, bool) {
    let within = |a: f32, b: f32| (a..b).contains(&t);
    let pda = within(10.0, 14.0) || within(52.0, 52.5);
    let active = !within(20.0, 22.0);
    let compass = within(26.0, 31.0) || within(50.0, 55.0);
    (pda, active, compass)
}

fn items(d: &UiDraw, node: usize) -> Vec<&sn_ui::draw::DrawItem> {
    d.list.items.iter().filter(|i| i.node == node).collect()
}

/// A drawn image's texture and UVs.
fn look(d: &UiDraw, node: usize) -> Option<(u64, Vec<[f32; 2]>)> {
    let it = items(d, node).into_iter().next()?;
    Some((it.texture, it.vertices.iter().map(|v| v.uv).collect()))
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
    let language = sn_assets::language(game, "English")?;
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
    let config = assets.depth_config(&layouts, &tree, &mut scene, &language)?;
    let suffix = language.get("MeterSuffix").cloned();
    println!(
        "uGUI_DepthCompass: shadow {}, half moon {}, texts {} / {}; MeterSuffix {:?}; textColorNormal {:?}; compass {} with {} labels",
        tree.nodes[config.shadow].path,
        tree.nodes[config.half_moon].name,
        tree.nodes[config.depth_text].name,
        tree.nodes[config.suffix_text].name,
        suffix,
        config.text_color_normal,
        tree.nodes[config.compass].name,
        config.labels.len()
    );
    for (n, text) in &config.labels {
        println!("  label {}: {text:?}", tree.nodes[*n].name);
    }
    // Each half-moon sprite as drawn on its own (the reference looks).
    let mut reference = |sprite| -> Result<Option<(u64, Vec<[f32; 2]>)>> {
        let mut s = start_state.clone();
        s.enabled.insert(config.half_moon, true);
        s.sprite.insert(config.half_moon, Some(sprite));
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &s)?;
        Ok(look(&d, config.half_moon))
    };
    let normal = reference(config.half_moon_normal)?;
    let with_compass = reference(config.half_moon_compass_normal)?;
    if normal.is_none() || normal == with_compass {
        return Err("the two half-moon sprites don't draw apart".into());
    }
    let mut depth = DepthCompass::new(config.clone());
    let player = [
        config.shadow,
        config.half_moon,
        config.depth_text,
        config.suffix_text,
    ];
    let never: Vec<usize> = config
        .submersible
        .iter()
        .copied()
        .chain([config.compass])
        .collect();
    let color = config.text_color_normal;
    // The replay's state: the last shown integer and the compass flag of
    // the last frame shown (the half moon is re-picked on showing and on
    // a change of the flag while shown, so it ends each shown frame on the
    // flag).
    let mut shown_depth: Option<i32> = None;
    let mut moon_compass = false;
    let (mut checked, mut differ) = (0usize, 0usize);
    let mut times = Vec::new();
    let mut per_kind: HashMap<&str, usize> = HashMap::new();
    for frame in 0..SECONDS * 60 {
        let t = (frame + 1) as f32 * DT;
        let (pda, active, compass) = timeline(t);
        let y = height(t);
        let inputs = HudInputs {
            active,
            mode: hud_mode(false, false, false),
            oxygen: 45.0,
            oxygen_capacity: 45.0,
            health: 100.0,
            max_health: 100.0,
            food: 50.5,
            water: 90.5,
            delta_time: DT,
            time: t,
            pda_open: pda,
            healed: None,
            ate: None,
            drank: None,
            depth: (0.0 - y).max(0.0),
            compass,
        };
        let mut state = start_state.clone();
        state.apply(depth.update(&inputs));
        let begin = Instant::now();
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        times.push(begin.elapsed().as_secs_f64() * 1000.0);
        // The replay.
        let shown = active && !pda;
        let flag = shown && compass;
        if shown {
            moon_compass = flag;
            shown_depth = Some(((0.0 - y).max(0.0)).floor() as i32);
        }
        let want_text = shown_depth.map(|v| v.to_string()).unwrap_or_default();
        let mut fails = Vec::new();
        let mut check = |kind: &'static str, ok: bool, what: String| {
            checked += 1;
            *per_kind.entry(kind).or_default() += 1;
            if !ok {
                fails.push(what);
            }
        };
        for n in player {
            let drawn = !items(&d, n).is_empty();
            check(
                "shown",
                drawn == shown,
                format!("{} drawn {drawn}", tree.nodes[n].name),
            );
        }
        // `SetVisible` enables the labels' texts with the compass (their
        // places and alphas are `UpdateLabels`', M9u6c2).
        for (n, _) in &config.labels {
            let drawn = !items(&d, *n).is_empty();
            check(
                "labels",
                drawn == flag,
                format!("label {} drawn {drawn}", tree.nodes[*n].name),
            );
        }
        for &n in &never {
            let drawn = !items(&d, n).is_empty();
            check("hidden", !drawn, format!("{} drawn", tree.nodes[n].name));
        }
        let text = state
            .text
            .get(&config.depth_text)
            .cloned()
            .unwrap_or_default();
        check(
            "text",
            text == want_text,
            format!("text {text:?}, want {want_text:?}"),
        );
        let suffix_text = state.text.get(&config.suffix_text).cloned();
        check(
            "suffix",
            suffix_text == suffix,
            format!("suffix {suffix_text:?}"),
        );
        if shown {
            let digits = items(&d, config.depth_text);
            let quads: usize = digits.iter().map(|i| i.vertices.len() / 4).sum();
            let colors_ok = digits
                .iter()
                .flat_map(|i| &i.vertices)
                .all(|v| v.color == color);
            check(
                "digits",
                quads == want_text.chars().count() && colors_ok,
                format!("{quads} quads, colours {colors_ok}"),
            );
            let want = if moon_compass { &with_compass } else { &normal };
            check(
                "half moon",
                look(&d, config.half_moon) == *want,
                format!(
                    "half moon not the {} sprite",
                    if moon_compass { "compass" } else { "normal" }
                ),
            );
            let m = items(&d, config.shadow)
                .first()
                .and_then(|i| d.materials.get(i.material));
            let blend_ok = m.is_some_and(|m| {
                m.floats.get("_SrcFactor") == Some(&2.0)
                    && m.floats.get("_DstFactor") == Some(&10.0)
                    && m.keywords.iter().any(|k| k == "ALPHA_PREMULTIPLY")
            });
            check("shadow", blend_ok, "shadow's blend".into());
        }
        if !fails.is_empty() {
            differ += 1;
            println!("t {t:.3} s (y {y:.3}): DIFFERS: {}", fails.join("; "));
        }
        if (frame + 1) % 60 == 0 {
            println!(
                "t {:>2} s: y {y:>8.3}, pda {pda}, active {active}, compass {compass}: mode {:?}, text \"{text}\" {:?}, half moon {}",
                (frame + 1) / 60,
                depth.mode,
                suffix_text.unwrap_or_default(),
                if moon_compass { "compass" } else { "normal" }
            );
        }
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let mut kinds: Vec<_> = per_kind.into_iter().collect();
    kinds.sort();
    println!(
        "{checked} depth checks ({kinds:?}) over {} frames, {differ} frames differ; draw mean {mean:.3} ms; glyphs made {}; ({:.1} s)",
        times.len(),
        atlases.made,
        start.elapsed().as_secs_f64()
    );
    Ok(if differ == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
