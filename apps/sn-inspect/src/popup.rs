//! `ui --popup` (M9u7d1, `docs/formats/ui.md` § The unlock pop-up):
//! `uGUI_PopupNotification` and its skins read from the game, then its
//! rules (`sn_ui::popup`) through a scripted queue of `OnAnalyze` entries
//! (`PDAData.analysisTech`'s message, picture and sound, the tech's name
//! through the language file), checked every frame against a replay
//! written here apart from the rules, in doubles:
//! - two entries in one frame, a third later (queued behind them), a
//!   fourth by `ShowNext` while the second is shown (which hides it from
//!   where it is and goes first);
//! - each frame: the phase, the transition value, the group's alpha, the
//!   `Unlock` skin's anchored position (`GetRectPositions` at `MiddleLeft`,
//!   `EaseOutSine`), which entry is current.
//!
//! Every 5th frame the scene is drawn: while shown, the skin's background,
//! picture and two texts are drawn, moved with the anchored position; the
//! `Journal` skin and the empty `controls` draw nothing; while hidden the
//! pop-up draws nothing; the other stored nodes draw as before.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_ui::popup::{Entry, Phase, Popup, TIME_IN, TIME_OUT, UNLOCK};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 0.0173;
const FRAMES: usize = 1200;

/// The script: (frame, tech type, by `ShowNext`).
const SCRIPT: &[(usize, &str, bool)] = &[
    (0, "PowerCell", false),
    (0, "CyclopsDecoyModule", false),
    (10, "DoubleTank", false),
    (450, "AdvancedWiringKit", true),
];

/// `uGUI_PopupNotification` again, in doubles; entries by script index.
struct Replay {
    /// `PDA.time`, a float in the game (summed as it sums it).
    time: f32,
    time_in: f64,
    duration: f64,
    time_out: f64,
    start: f64,
    phase: Phase,
    value: f64,
    queue: Vec<usize>,
    current: Option<usize>,
    /// The skin's anchored position once set.
    at: Option<[f64; 2]>,
    alpha: Option<f64>,
    shown: Vec<(usize, f64)>,
}

impl Replay {
    fn hide(&mut self) {
        self.duration = self.duration.max(0.0);
        self.start = f64::from(self.time)
            - (self.time_in + self.duration + self.time_out * (1.0 - self.value));
    }

    fn frame(&mut self, adds: &[(usize, bool)], duration: f64, from: [f64; 2], to: [f64; 2]) {
        for &(k, next) in adds {
            if next {
                self.hide();
                self.queue.insert(0, k);
            } else {
                self.queue.push(k);
            }
        }
        let num = f64::from(self.time) - self.start;
        let phase = if self.phase == Phase::None {
            Phase::None
        } else if num < 0.0 {
            Phase::Zero
        } else if num < self.time_in {
            Phase::In
        } else if num < self.time_in + self.duration {
            Phase::One
        } else if num < self.time_in + self.duration + self.time_out {
            Phase::Out
        } else {
            Phase::Done
        };
        if phase != self.phase {
            self.phase = phase;
            self.alpha = Some(if (Phase::In..=Phase::Out).contains(&phase) {
                1.0
            } else {
                0.0
            });
            if phase == Phase::Done {
                self.current = None;
            }
        }
        let total = self.time_in + self.duration + self.time_out;
        let up = if self.time_in > 0.0 {
            num / self.time_in
        } else {
            0.0
        };
        let down = if self.time_out > 0.0 {
            (total - num) / self.time_out
        } else {
            0.0
        };
        let b = up.min(down).clamp(0.0, 1.0);
        let moved = b != self.value;
        self.value = b;
        if matches!(self.phase, Phase::None | Phase::Done) && !self.queue.is_empty() {
            if self.phase == Phase::None {
                self.phase = Phase::Zero;
            }
            let k = self.queue.remove(0);
            self.time_in = f64::from(TIME_IN);
            self.duration = duration;
            self.time_out = f64::from(TIME_OUT);
            self.start = f64::from(self.time) - self.time_in * self.value;
            self.shown.push((k, f64::from(self.time)));
            self.current = Some(k);
        }
        self.time += DT;
        if moved {
            let t = (self.value * std::f64::consts::FRAC_PI_2)
                .sin()
                .clamp(0.0, 1.0);
            self.at = Some([
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ]);
        }
    }
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

    let config = assets.popup_config(&layouts, &tree, &mut scene)?;
    println!(
        "uGUI_PopupNotification: {} (group at {}), defaultDuration {} s, {} skins",
        tree.nodes[config.root].path,
        tree.nodes[config.group].path,
        config.default_duration,
        config.skins.len()
    );
    for s in &config.skins {
        println!(
            "  skin {}: ox {}, oy {}, title {}, text {}, controls {}, image {:?}, defaultSprite {}, sounds {:?} / {:?} / {:?}",
            tree.nodes[s.root].path,
            s.ox,
            s.oy,
            tree.nodes[s.title].name,
            tree.nodes[s.text].name,
            tree.nodes[s.controls].name,
            s.image.map(|n| &tree.nodes[n].name),
            s.default_sprite.is_some(),
            s.sound_slide_in,
            s.sound_slide_out,
            s.default_sound
        );
    }
    check(
        config.skins.len() == 2
            && tree.nodes[config.skins[UNLOCK].root].name == "Unlock"
            && config.default_duration > 0.0,
        "two skins, Unlock first, a default duration",
    );
    let layout = sn_assets::popup_layout(&assets, &mut scene, &set, &placements, &start, &config)?;
    println!(
        "  rects: pop-up {:?}, skins {:?}",
        layout.parent, layout.skins
    );
    let player = sn_assets::player_data(&assets)?;
    let analysis = assets.analysis_popups(&layouts, &player)?;
    let dll = sn_assets::read_assembly(game)?;
    let text = sn_assets::item_text(game, &dll, "English")?;
    let by_name: HashMap<&str, i32> = text
        .tech_names
        .iter()
        .map(|(k, v)| (v.as_str(), *k))
        .collect();
    let (mut messages, mut pictures) = (HashMap::new(), 0);
    for a in analysis.values() {
        *messages.entry(a.unlock_message.as_str()).or_insert(0) += 1;
        pictures += usize::from(a.unlock_popup.is_some());
    }
    println!(
        "PDAData.analysisTech: {} entries, messages {messages:?}, {pictures} with a picture",
        analysis.len()
    );
    let mut entries: Vec<Entry> = Vec::new();
    for (_, tech, _) in SCRIPT {
        let t = by_name
            .get(tech)
            .ok_or_else(|| format!("{tech}: no tech type"))?;
        let a = analysis
            .get(t)
            .ok_or_else(|| format!("{tech}: no analysis entry"))?;
        let e = sn_assets::analysis_entry(&config, a, tech, &text.strings, &mut scene);
        println!(
            "  OnAnalyze({tech}): title {:?}, text {:?}, picture {:?}, sound {:?}",
            e.title, e.text, a.unlock_popup, e.sound
        );
        entries.push(e);
    }
    check(
        entries.iter().all(|e| {
            !e.title.is_empty() && e.title != "NotificationBlueprintUnlocked" && !e.text.is_empty()
        }),
        "every title and text found in the language file",
    );
    check(
        entries[1].sprite.is_some() && entries[0].sprite.is_none(),
        "CyclopsDecoyModule has its own picture, PowerCell none (defaultSprite)",
    );

    let skin = &config.skins[UNLOCK];
    let rect = layout.skins[UNLOCK];
    let y = 0.5 * f64::from(layout.parent.height - rect.height);
    let from = [-f64::from(rect.width), y];
    let to = [f64::from(skin.ox), y];
    let mut popup = Popup::new(config.clone());
    let mut replay = Replay {
        time: 0.0f32,
        time_in: 0.0,
        duration: 0.0,
        time_out: 0.0,
        start: 0.0,
        phase: Phase::None,
        value: 0.0,
        queue: Vec::new(),
        current: None,
        at: None,
        alpha: None,
        shown: Vec::new(),
    };
    // The pop-up's nodes and the Unlock skin's.
    let nodes = skin_nodes(&tree, config.root);
    let skin_set = skin_nodes(&tree, skin.root);
    // `Enqueue` may drop an entry's sound: entries are told apart by what
    // they show.
    let index = |e: &Entry| {
        entries
            .iter()
            .position(|x| x.title == e.title && x.text == e.text && x.sprite == e.sprite)
    };
    let (mut frame_bad, mut worst_value, mut worst_at) = (0usize, 0.0f64, 0.0f64);
    let (mut draws, mut draw_bad, mut shown_draws) = (0usize, 0usize, 0usize);
    let mut shown = Vec::new();
    let mut phases = Vec::new();
    let mut sounds = Vec::new();
    // A drawn item's offset from the skin's anchored position, by node and
    // entry (a text's bounds depend on its glyphs).
    let mut offsets: HashMap<(usize, Option<usize>), [f32; 2]> = HashMap::new();
    let mut worst_draw = 0.0f32;
    // The other nodes' stored draws.
    let stored_others: Vec<_> = before
        .list
        .items
        .iter()
        .filter(|i| !nodes.contains(&i.node))
        .collect();
    check(
        before
            .list
            .items
            .iter()
            .all(|i| !nodes.contains(&i.node) || i.alpha == 0.0),
        "at start the pop-up draws nothing (its group's alpha is 0, Awake)",
    );
    for f in 0..FRAMES {
        let mut adds = Vec::new();
        let mut enqueue = Vec::new();
        for (k, (at, _, next)) in SCRIPT.iter().enumerate() {
            if *at != f {
                continue;
            }
            adds.push((k, *next));
            if *next {
                popup.show_next(entries[k].clone());
            } else {
                enqueue.push(entries[k].clone());
            }
        }
        popup.update(&enqueue, DT, &layout);
        replay.frame(&adds, f64::from(config.default_duration), from, to);
        for (e, at) in &popup.events.shown {
            let k = index(e).unwrap_or(usize::MAX);
            shown.push((k, *at));
        }
        phases.extend(popup.events.phases.iter().map(|(p, t)| (*p, *t)));
        sounds.extend(popup.events.sounds.iter().cloned());
        let s = &popup.state;
        let at = s
            .transform
            .get(&skin.root)
            .and_then(|t| t.anchored_position);
        let ea = match (at, replay.at) {
            (Some(a), Some(b)) => (f64::from(a[0]) - b[0])
                .abs()
                .max((f64::from(a[1]) - b[1]).abs()),
            (None, None) => 0.0,
            _ => f64::INFINITY,
        };
        let ev = (f64::from(popup.value) - replay.value).abs();
        worst_at = worst_at.max(ea);
        worst_value = worst_value.max(ev);
        let current = popup.current.as_ref().and_then(index);
        let bad = popup.phase != replay.phase
            || current != replay.current
            || s.group_alpha.get(&config.group).map(|a| f64::from(*a)) != replay.alpha
            || ev > 1e-5
            || ea > 0.01;
        if bad {
            frame_bad += 1;
            if frame_bad <= 5 {
                println!(
                    "  frame {f}: differs (phase {:?} / {:?}, current {current:?} / {:?}, value {} / {}, at {at:?} / {:?})",
                    popup.phase, replay.phase, replay.current, popup.value, replay.value, replay.at
                );
            }
        }
        if f % 5 != 0 {
            continue;
        }
        let mut state = start.clone();
        state.apply(&popup.state);
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        draws += 1;
        // Drawn: in the list with an alpha above 0 (a transparent group's
        // graphics stay in the list at alpha 0) and vertices (an empty text
        // has none).
        let ours: Vec<_> = d
            .list
            .items
            .iter()
            .filter(|i| nodes.contains(&i.node) && i.alpha > 0.0 && !i.vertices.is_empty())
            .collect();
        let others: Vec<_> = d
            .list
            .items
            .iter()
            .filter(|i| !nodes.contains(&i.node))
            .collect();
        let mut good = others.len() == stored_others.len()
            && others.iter().zip(&stored_others).all(|(x, y)| x == y);
        if popup.is_showing() {
            shown_draws += 1;
            let at = at.unwrap_or([0.0; 2]);
            // The background, the picture, the title and the text.
            good &= ours.len() == 4 && ours.iter().all(|i| skin_set.contains(&i.node));
            good &= ours
                .iter()
                .any(|i| Some(i.node) == skin.image && i.texture != 0);
            for i in &ours {
                let x = i
                    .vertices
                    .iter()
                    .map(|v| v.pos[0])
                    .fold(f32::INFINITY, f32::min);
                let y = i
                    .vertices
                    .iter()
                    .map(|v| v.pos[1])
                    .fold(f32::INFINITY, f32::min);
                let o = [x - at[0], y - at[1]];
                match offsets.get(&(i.node, current)) {
                    Some(p) => {
                        worst_draw = worst_draw.max((p[0] - o[0]).abs()).max((p[1] - o[1]).abs())
                    }
                    None => {
                        offsets.insert((i.node, current), o);
                    }
                }
            }
        } else {
            good &= ours.is_empty();
        }
        if !good {
            draw_bad += 1;
            if draw_bad <= 5 {
                println!(
                    "  draw at frame {f}: differs ({} of ours: {:?}, {} others)",
                    ours.len(),
                    ours.iter()
                        .map(|i| &tree.nodes[i.node].name)
                        .collect::<Vec<_>>(),
                    others.len()
                );
            }
        }
    }
    println!(
        "{FRAMES} frames, {frame_bad} differing from the replay (value ≤ {worst_value:.7}, position ≤ {worst_at:.5})"
    );
    println!(
        "shown: {:?}",
        shown
            .iter()
            .map(|(k, t)| format!("{} at {t:.4}", SCRIPT.get(*k).map_or("?", |s| s.1)))
            .collect::<Vec<_>>()
    );
    let marks: Vec<String> = phases
        .iter()
        .map(|(p, t)| format!("{p:?} {t:.4}"))
        .collect();
    println!("phases: {marks:?}");
    println!("sounds (not ported): {sounds:?}");
    println!(
        "{draws} draws ({shown_draws} shown), {draw_bad} differing; drawn items move with the skin (≤ {worst_draw:.4} canvas units)"
    );
    check(frame_bad == 0, "every frame equals the replay");
    let order: Vec<usize> = shown.iter().map(|(k, _)| *k).collect();
    check(
        order == [0, 1, 3, 2],
        "shown in order: two queued, ShowNext first, the later one last",
    );
    check(
        replay.shown.len() == shown.len()
            && replay
                .shown
                .iter()
                .zip(&shown)
                .all(|(a, b)| a.0 == b.0 && (a.1 - f64::from(b.1)).abs() < 1e-3),
        "shown at the replay's times",
    );
    let gap = shown[1].1 - shown[0].1;
    let total = TIME_IN + config.default_duration + TIME_OUT;
    check(
        gap >= total - 1e-3 && gap < total + 2.0 * DT,
        "the next one follows when the first is done (0.25 + 5 + 0.25 s)",
    );
    let hidden_after = shown[2].1 - SCRIPT[3].0 as f32 * DT;
    check(
        (TIME_OUT - 1e-3..TIME_OUT + 2.0 * DT).contains(&hidden_after),
        "ShowNext: the shown one slides out at once (0.25 s), then the new one",
    );
    check(
        draw_bad == 0,
        "every draw: the skin's four items while shown, none while hidden",
    );
    check(
        worst_draw < 0.01,
        "the drawn items move with the skin's anchored position",
    );
    check(popup.idle(), "nothing left at the end");
    let mut end = start.clone();
    end.apply(&popup.state);
    let after = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &end)?;
    let after_others: Vec<_> = after
        .list
        .items
        .iter()
        .filter(|i| !nodes.contains(&i.node))
        .collect();
    check(
        after_others.len() == stored_others.len()
            && after_others.iter().zip(&stored_others).all(|(x, y)| x == y)
            && after
                .list
                .items
                .iter()
                .all(|i| !nodes.contains(&i.node) || i.alpha == 0.0),
        "all hidden again: the pop-up draws nothing, the other nodes as before",
    );
    println!("checks {ok} of {all}");
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// A node and every node under it.
fn skin_nodes(tree: &sn_assets::UiTree, root: usize) -> Vec<usize> {
    (0..tree.nodes.len())
        .filter(|&n| {
            let mut at = Some(n);
            while let Some(k) = at {
                if k == root {
                    return true;
                }
                at = tree.nodes[k].parent;
            }
            false
        })
        .collect()
}
