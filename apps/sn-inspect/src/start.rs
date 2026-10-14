//! `ui --start` (M9u6s, `docs/formats/ui.md` § Start-up state): what
//! `uGUI` draws in its start state, by screen. Each screen (a child of a
//! root canvas, one level down for `ScreenCanvas`) is listed with its
//! draws (and how many are visible), quads, the area its visible draws
//! cover as a fraction of its canvas,
//! and the scripts on its nodes. Each visible draw is named with the rule
//! that shows it ([`SHOWN_BY`]); one not named fails the run.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
/// A draw covering more than this fraction of its canvas is "large".
const LARGE: f32 = 0.05;

/// Why a draw shows at start: (path prefix, end of path or "", the rule).
/// Every visible draw must match one.
const SHOWN_BY: &[(&str, &str, &str)] = &[
    (
        "uGUI/DepthClear/",
        "",
        "uGUI/DepthClear: depth only, nothing drawn in our pass (M9u5c)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/",
        "/PulseHalo",
        "the bar's pulse clip, not ported: stored state (M9u6b2)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/",
        "/PulseWave",
        "the bar's pulse clip, not ported: stored state (M9u6b2)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/BackgroundQuad",
        "",
        "uGUI_SceneHUD: Survival, the quad background (M9u6b1)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/",
        "",
        "uGUI_SceneHUD: Survival, the four bars (M9u6b1)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/DepthCompass/PlayerDepth/",
        "",
        "uGUI_DepthCompass: DepthMode.Player (its values: M9u6c)",
    ),
    (
        "uGUI/ScreenCanvas/HUD/Content/HandReticle/IconCanvas/Default",
        "",
        "HandReticle: the Default icon (its rules: M9u6d)",
    ),
];

/// The rule that shows a draw of `path`.
fn shown_by(path: &str) -> Option<&'static str> {
    SHOWN_BY
        .iter()
        .find(|(prefix, end, _)| path.starts_with(prefix) && path.ends_with(end))
        .map(|(_, _, why)| *why)
}

#[derive(Default)]
struct Screen {
    draws: usize,
    /// Draws with some alpha (the item's and a vertex's above 0).
    visible: usize,
    quads: usize,
    /// The union of the draws' bounds, in the root canvas's space.
    min: [f32; 2],
    max: [f32; 2],
    largest: f32,
    canvas: usize,
    scripts: BTreeSet<String>,
}

/// The screen a node belongs to: its path cut after the root canvas's
/// child (`uGUI/ScreenCanvas/HUD/...` → `uGUI/ScreenCanvas/HUD`).
fn screen_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let keep = if parts.get(1) == Some(&"ScreenCanvas") {
        3
    } else {
        2
    };
    parts[..parts.len().min(keep)].join("/")
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
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let start = assets.ui_start_state(&layouts, &tree)?;
    for n in &start.notes {
        println!("start rule: {n}");
    }
    let state = UiState::from(&start);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let drawn = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
    let mut screens: BTreeMap<String, Screen> = BTreeMap::new();
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unnamed = 0;
    let mut large = Vec::new();
    for it in &drawn.list.items {
        let s = screens
            .entry(screen_of(&tree.nodes[it.node].path))
            .or_insert_with(|| Screen {
                min: [f32::MAX; 2],
                max: [f32::MIN; 2],
                ..Default::default()
            });
        s.draws += 1;
        // `Hidden/UI/CircularBar` draws nothing at `_Value` ≤ 0.
        let empty_bar = drawn
            .materials
            .get(it.material)
            .and_then(|m| m.floats.get("_Value"))
            .is_some_and(|v| *v <= 0.0);
        let seen = it.alpha > 0.0 && it.vertices.iter().any(|v| v.color[3] > 0) && !empty_bar;
        if seen {
            s.visible += 1;
        }
        // The scripts on the node and its parents, nearest first.
        let mut chain = Vec::new();
        let mut at = Some(it.node);
        while let Some(n) = at {
            let node = &tree.nodes[n];
            for c in &node.components {
                if let Some(name) = c
                    .script
                    .as_deref()
                    .filter(|n| n.starts_with("uGUI_") || !n.contains("Layout"))
                {
                    chain.push(format!("{}@{}", name, node.name));
                }
            }
            at = node.parent;
        }
        println!(
            "draw {} {}: {} quads, alpha {:.3}{}; scripts up: {}",
            it.kind,
            tree.nodes[it.node].path,
            it.vertices.len() / 4,
            it.alpha,
            if seen { "" } else { " (invisible)" },
            chain.join(" < ")
        );
        if !seen {
            continue;
        }
        let path = &tree.nodes[it.node].path;
        match shown_by(path) {
            Some(why) => {
                *named.entry(why).or_insert(0) += 1;
            }
            None => {
                unnamed += 1;
                println!("UNNAMED visible draw: {} {path}", it.kind);
            }
        }
        s.quads += it.vertices.len() / 4;
        s.canvas = it.root;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &it.vertices {
            for k in 0..2 {
                lo[k] = lo[k].min(v.pos[k]);
                hi[k] = hi[k].max(v.pos[k]);
            }
        }
        for k in 0..2 {
            s.min[k] = s.min[k].min(lo[k]);
            s.max[k] = s.max[k].max(hi[k]);
        }
        let area = (hi[0] - lo[0]).max(0.0) * (hi[1] - lo[1]).max(0.0);
        s.largest = s.largest.max(area);
        let canvas = placements
            .get(&it.root)
            .map(|p| p.size[0] * p.size[1])
            .unwrap_or(0.0);
        if canvas > 0.0 && area / canvas > LARGE {
            large.push(format!("{} ({:.3})", path, area / canvas));
        }
    }
    for n in &tree.nodes {
        if let Some(s) = screens.get_mut(&screen_of(&n.path)) {
            for c in &n.components {
                if let Some(name) = &c.script {
                    s.scripts.insert(name.clone());
                }
            }
        }
    }
    println!(
        "start state at {}×{}: {} draws in {} screens",
        SCREEN[0],
        SCREEN[1],
        drawn.list.items.len(),
        screens.len()
    );
    for (name, s) in &screens {
        let canvas = placements
            .get(&s.canvas)
            .map(|p| p.size[0] * p.size[1])
            .unwrap_or(0.0);
        let frac = |a: f32| if canvas > 0.0 { a / canvas } else { 0.0 };
        let bounds = (s.max[0] - s.min[0]).max(0.0) * (s.max[1] - s.min[1]).max(0.0);
        println!(
            "screen {name}: {} draws ({} visible), {} quads, bounds {:.3} of its canvas, largest draw {:.3}; scripts: {}",
            s.draws,
            s.visible,
            s.quads,
            frac(bounds),
            frac(s.largest),
            s.scripts.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    for (why, n) in &named {
        println!("shown by {why}: {n} draws");
    }
    let visible: usize = screens.values().map(|s| s.visible).sum();
    println!(
        "{} of {visible} visible draws named with the rule that shows them",
        visible - unnamed
    );
    println!(
        "{} large draws (over {LARGE} of the canvas): {}",
        large.len(),
        large.join(", ")
    );
    Ok(if unnamed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
