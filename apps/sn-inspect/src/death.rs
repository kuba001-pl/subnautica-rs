//! `ui --death` (M9u6e1, `docs/formats/ui.md` § Death, respawn and
//! overlays): the three screens' config as read, then their rules
//! (`sn_ui::death`) through a scripted drowning run by our own vitals
//! (`sn_sim::vitals`: out of oxygen, suffocation, death, the respawn
//! screen shown and hidden), with the player animator's `DeathFade` at a
//! scripted time, checked every frame against a replay written here
//! apart from the rules:
//! - the overlay's alpha is its curve at `1 − t` (1 on the death's frame),
//!   drawn iff above 0;
//! - the vignette: nothing on the event's frame, `BeginDeathFade` the
//!   next frame (after its `Update`), then `a += (1 − a) · min(1, 5 dt)`
//!   each frame until `a ≥ 0.98` (then 1); from the first frame at 6 s
//!   after the event, `a ← a (1 − min(1, 2 dt))` from the next frame,
//!   reset to 0 and disabled on the frame that starts below 0.02;
//! - the fader: 1 from the frame after `Show`; from the frame after
//!   `Hide`, `0.5 (1 − cos π t)` with `t` falling by `dt / fadeInTime`.
//!
//! Then a `CutToBlack` (black at once) and a death by damage with oxygen
//! left (the overlay at 1 for that frame only). A few frames are drawn
//! as the client does and their draws counted.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_sim::vitals::{Situation, Vitals, VitalsEvent};
use sn_ui::death::{DeathInputs, DeathScreens, DeathType};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 1.0 / 60.0;
const FRAMES: usize = 1800;
/// The drowning's oxygen at the start (the run is short).
const START_OXYGEN: f64 = 3.0;
/// When the animator's `DeathFade` fires, after the death (scripted: the
/// clips' event times are 0 to 1.814 s; `drown_death`'s is 1.675 s).
const EVENT_AFTER_DEATH: f32 = 1.675;

/// The replay's vignette.
#[derive(Default)]
struct Vignette {
    /// The event's frame.
    trigger: Option<usize>,
    alpha: f64,
    active: bool,
    fade_in: bool,
    enabled: Option<bool>,
    /// The frame `BeginFadeOut` ran in.
    out_at: Option<usize>,
}

impl Vignette {
    fn step(&mut self, frame: usize, time: f64, dt: f64) {
        // `Update`.
        if self.active {
            if self.fade_in {
                if self.alpha < 0.98 {
                    self.enabled = Some(true);
                    self.alpha += (1.0 - self.alpha) * (5.0 * dt).min(1.0);
                } else {
                    self.alpha = 1.0;
                }
            } else {
                let before = self.alpha;
                self.alpha *= 1.0 - (2.0 * dt).min(1.0);
                if before < 0.02 {
                    // `ResetOverlay`.
                    self.alpha = 0.0;
                    self.active = false;
                    self.fade_in = false;
                    self.enabled = Some(false);
                    return;
                }
            }
        }
        // The delayed calls.
        if let Some(t) = self.trigger {
            let at = t as f64 * dt;
            if frame == t + 1 {
                self.active = true;
                self.fade_in = true;
                self.enabled = Some(true);
            }
            if self.out_at.is_none() && frame > t && time + 1e-6 >= at + 6.0 {
                self.fade_in = false;
                self.out_at = Some(frame);
            }
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
    let language = sn_assets::language(game, "English")?;
    let code = sn_assets::read_assembly(game)?;
    let config = assets.death_config(&layouts, &tree, &code, &language)?;
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };

    let v = config.vignette;
    let f = config.loading_background;
    println!(
        "uGUI_PlayerDeath: blackOverlay {}; Invoke BeginDeathFade {} s, BeginFadeOut {} s; rates in {} out {}; alpha limits {} / {}",
        tree.nodes[config.black_overlay].path,
        v.begin_fade_delay,
        v.fade_out_delay,
        v.rate_in,
        v.rate_out,
        v.full,
        v.clear
    );
    println!(
        "uGUI_SceneRespawning: loadingText {}, text {:?}; fader group {}: fadeInTime {}, fadeOutTime {}, unscaled {}, initialState {}",
        tree.nodes[config.loading_text].path,
        config.loading,
        tree.nodes[f.group].path,
        f.fade_in_time,
        f.fade_out_time,
        f.use_unscaled_time,
        f.initial_state
    );
    for (k, o) in config.overlays.iter().enumerate() {
        let keys: Vec<String> = o
            .curve
            .keys
            .iter()
            .map(|k| {
                format!(
                    "({}, {}; in {}, out {})",
                    k.time, k.value, k.in_slope, k.out_slope
                )
            })
            .collect();
        println!(
            "uGUI_Overlays[{k}]: {}; curve {}; at 0, 0.25, 0.5, 0.75, 1: {:?}",
            tree.nodes[o.graphic].path,
            keys.join(" "),
            [0.0, 0.25, 0.5, 0.75, 1.0].map(|x| o.curve.evaluate(x))
        );
    }
    // The values the replay below assumes are the ones read.
    check(
        (
            v.begin_fade_delay,
            v.fade_out_delay,
            v.rate_in,
            v.rate_out,
            v.full,
            v.clear,
        ) == (0.0, 6.0, 5.0, 2.0, 0.98, 0.02),
        "the vignette's numbers are the decompiled code's",
    );
    check(
        !f.initial_state && f.use_unscaled_time,
        "the respawn fader starts off, in unscaled time",
    );

    // The scene, to draw a few frames.
    let start_state = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let under = |n: usize, root: usize| tree.nodes[n].path.starts_with(&tree.nodes[root].path);

    // The drowning.
    let data = sn_assets::player_data(&assets)?;
    let pc = sn_assets::player_code(&code)?;
    let vp = sn_assets::vitals_params(&data, &pc);
    let mut vitals = Vitals::new(&vp);
    vitals.oxygen = START_OXYGEN;
    let mut situation = Situation {
        y: vp.ocean_level - 10.0,
        world_settled: true,
        ..Situation::default()
    };
    let mut screens = DeathScreens::new(config.clone());
    let mut vig = Vignette::default();
    let (mut show_at, mut hide_at) = (None::<usize>, None::<usize>);
    let mut died_at = None::<usize>;
    let mut fader_alpha = 0.0f64;
    let (mut frame_checks, mut frame_fails) = (0usize, 0usize);
    let mut marks: Vec<(usize, &str)> = Vec::new();
    let (mut black_full, mut black_reset, mut fader_clear) = (None, None, None);
    let (mut drawn, mut dying_drawn) = (0, false);
    for frame in 1..=FRAMES {
        let time = frame as f32 * DT;
        let events = vitals.step(&vp, f64::from(DT), &situation);
        let mut inputs = DeathInputs {
            delta_time: DT,
            unscaled_delta_time: DT,
            time,
            suffocation: Some(vitals.overlay() as f32),
            ..DeathInputs::default()
        };
        for e in &events {
            match e {
                VitalsEvent::Died => {
                    inputs.killed = true;
                    died_at = Some(frame);
                    marks.push((frame, "died"));
                }
                VitalsEvent::MoveToRespawn => {
                    inputs.respawn_show = true;
                    situation.in_pod = true;
                    show_at = Some(frame);
                    marks.push((frame, "respawn shown"));
                }
                VitalsEvent::Restored => {
                    inputs.respawn_hide = true;
                    hide_at = Some(frame);
                    marks.push((frame, "respawn hidden"));
                }
                VitalsEvent::ControlsBack => marks.push((frame, "controls back")),
                VitalsEvent::SuffocationStarted => marks.push((frame, "suffocating")),
                _ => {}
            }
        }
        if let Some(d) = died_at
            && vig.trigger.is_none()
            && time >= d as f32 * DT + EVENT_AFTER_DEATH
        {
            inputs.vignette = Some(DeathType::FadeToBlack);
            vig.trigger = Some(frame);
            marks.push((frame, "DeathFade"));
        }
        let state = screens.update(&inputs);

        // The replay.
        vig.step(frame, f64::from(time), f64::from(DT));
        if let Some(s) = show_at
            && frame == s + 1
        {
            fader_alpha = 1.0;
        }
        if let Some(h) = hide_at
            && frame > h
        {
            let t = (1.0 - (frame - h) as f64 * f64::from(DT) / f64::from(f.fade_in_time)).max(0.0);
            fader_alpha = 0.5 * (1.0 - (std::f64::consts::PI * t).cos());
            if t <= 0.0 && fader_clear.is_none() {
                fader_clear = Some(frame);
            }
        }
        let value = if inputs.killed {
            1.0
        } else {
            vitals.overlay() as f32
        };
        let overlay = config.overlays[0].curve.evaluate(value).clamp(0.0, 1.0);
        if vig.alpha >= 1.0 && black_full.is_none() {
            black_full = Some(frame);
        }
        if vig.enabled == Some(false) && vig.out_at.is_some() && black_reset.is_none() {
            black_reset = Some(frame);
        }

        let mut fail = |good: bool, what: String| {
            frame_checks += 1;
            if !good {
                frame_fails += 1;
                if frame_fails <= 10 {
                    println!("  frame {frame} ({time:.3} s): {what}");
                }
            }
        };
        let got = state
            .color
            .get(&config.black_overlay)
            .map_or(-1.0, |c| c[3]);
        fail(
            (f64::from(got) - vig.alpha).abs() < 1e-4,
            format!("black alpha {got} ≠ {:.5}", vig.alpha),
        );
        let got = state.enabled.get(&config.black_overlay).copied();
        fail(
            got == vig.enabled,
            format!("black enabled {got:?} ≠ {:?}", vig.enabled),
        );
        let got = state.group_alpha.get(&f.group).copied().unwrap_or(-1.0);
        fail(
            (f64::from(got) - fader_alpha).abs() < 1e-4,
            format!("fader alpha {got} ≠ {fader_alpha:.5}"),
        );
        let got = state.text.get(&config.loading_text);
        let want = show_at.map(|_| &config.loading);
        fail(got == want, format!("loading text {got:?} ≠ {want:?}"));
        let g = config.overlays[0].graphic;
        let got = state.enabled.get(&g).copied();
        fail(
            got == Some(overlay > 0.0),
            format!("overlay enabled {got:?}, curve {overlay}"),
        );
        if overlay > 0.0 {
            let got = state.renderer_alpha.get(&g).copied().unwrap_or(-1.0);
            fail(
                (got - overlay).abs() < 1e-6,
                format!("overlay alpha {got} ≠ {overlay}"),
            );
        }

        // Draw a few frames: dying, black, the respawn screen, after.
        let dying = died_at.is_none() && !dying_drawn && vitals.overlay() >= 0.5;
        dying_drawn |= dying;
        let draw_now = dying
            || match (died_at, show_at, hide_at) {
                (Some(d), None, _) if frame == d + 60 => true,
                (_, Some(s), None) if frame == s + 2 => true,
                (_, _, Some(h)) if frame == h + 30 || frame == h + 300 => true,
                _ => false,
            };
        if draw_now {
            let mut s = start_state.clone();
            s.apply(&state);
            let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &s)?;
            drawn += 1;
            let visible = |root: usize| {
                d.list
                    .items
                    .iter()
                    .filter(|i| {
                        under(i.node, root)
                            && i.alpha > 0.0
                            && i.vertices.iter().any(|v| v.color[3] > 0)
                    })
                    .count()
            };
            let (black, respawn, over) =
                (visible(config.black_overlay), visible(f.group), visible(g));
            // Where each screen is in the draw order (later draws on top).
            let at = |root: usize| {
                d.list
                    .items
                    .iter()
                    .position(|i| under(i.node, root))
                    .map_or("-".to_string(), |k| format!("{k}"))
            };
            println!(
                "  draw order of {} items: respawn screen at {}, black overlay at {}, out-of-oxygen at {}",
                d.list.items.len(),
                at(f.group),
                at(config.black_overlay),
                at(g)
            );
            println!(
                "frame {frame} ({time:.3} s): overlay {:.3}, black {:.3}, fader {:.3}; visible draws: black overlay {black}, respawn screen {respawn}, out-of-oxygen {over}",
                screens.overlay_alpha(0),
                screens.death.alpha,
                screens.fader.alpha.unwrap_or(0.0),
            );
            check(
                (black > 0) == (screens.death.alpha > 0.0 && screens.death.enabled == Some(true)),
                &format!("frame {frame}: black overlay drawn iff shown"),
            );
            check(
                (respawn > 0) == (screens.fader.alpha.unwrap_or(0.0) > 0.0),
                &format!("frame {frame}: respawn screen drawn iff its alpha > 0"),
            );
            check(
                (over > 0) == (screens.overlay_alpha(0) > 0.0),
                &format!("frame {frame}: out-of-oxygen overlay drawn iff shown"),
            );
        }
    }
    for (frame, what) in &marks {
        println!("  {:>8.3} s (frame {frame}): {what}", *frame as f32 * DT);
    }
    let secs = |a: Option<usize>, b: Option<usize>| match (a, b) {
        (Some(a), Some(b)) => Some((b as f64 - a as f64) * f64::from(DT)),
        _ => None,
    };
    let trigger = marks_frame(&marks, "DeathFade");
    let full = secs(trigger, black_full);
    let out = secs(trigger, vig.out_at);
    let fade = secs(hide_at, fader_clear);
    println!(
        "vignette: black {full:?} s after the event, fade out from {out:?} s (rule {} s), reset at frame {black_reset:?}; fader clear {fade:?} s after Hide (rule {} s)",
        v.fade_out_delay, f.fade_in_time
    );
    check(
        died_at.is_some() && show_at.is_some() && hide_at.is_some() && trigger.is_some(),
        "the run died, showed and hid the respawn screen, and fired the event",
    );
    check(
        out.is_some_and(|o| o + 1e-6 >= 6.0 && o < 6.0 + f64::from(DT) + 1e-6),
        "the fade out starts within a frame of 6 s after the event",
    );
    check(
        fade.is_some_and(|s| (s - f64::from(f.fade_in_time)).abs() <= f64::from(DT) + 1e-6),
        "the fader clears within a frame of fadeInTime after Hide",
    );
    check(
        black_reset.is_some(),
        "the vignette resets after fading out",
    );

    // `CutToBlack`: black at once.
    let mut s2 = DeathScreens::new(config.clone());
    s2.update(&DeathInputs {
        delta_time: DT,
        unscaled_delta_time: DT,
        time: DT,
        ..DeathInputs::default()
    });
    let st = s2.update(&DeathInputs {
        delta_time: DT,
        unscaled_delta_time: DT,
        time: 2.0 * DT,
        vignette: Some(DeathType::CutToBlack),
        ..DeathInputs::default()
    });
    check(
        st.color.get(&config.black_overlay).map(|c| c[3]) == Some(1.0)
            && st.enabled.get(&config.black_overlay) == Some(&true),
        "CutToBlack is black on its frame",
    );

    // A death by damage with oxygen left: the overlay at 1 for one frame.
    let mut vitals = Vitals::new(&vp);
    let mut s3 = DeathScreens::new(config.clone());
    let situation = Situation {
        y: vp.ocean_level + 1.0,
        world_settled: true,
        ..Situation::default()
    };
    let mut seen = Vec::new();
    for frame in 1..=3 {
        let events = vitals.step(&vp, f64::from(DT), &situation);
        let mut killed = events.contains(&VitalsEvent::Died);
        if frame == 2 {
            let mut e = Vec::new();
            vitals.take_damage(vp.max_health, &mut e);
            killed |= e.contains(&VitalsEvent::Died);
        }
        s3.update(&DeathInputs {
            delta_time: DT,
            unscaled_delta_time: DT,
            time: frame as f32 * DT,
            suffocation: Some(vitals.overlay() as f32),
            killed,
            ..DeathInputs::default()
        });
        seen.push(s3.overlay_alpha(0));
    }
    println!("death by damage: out-of-oxygen overlay per frame {seen:?}");
    let one = config.overlays[0].curve.evaluate(1.0).clamp(0.0, 1.0);
    check(
        seen == vec![0.0, one, 0.0],
        "a death by damage shows the overlay on its frame only",
    );

    check(frame_fails == 0, "every frame equals the replay");
    println!(
        "{frame_checks} frame checks over {FRAMES} frames, {frame_fails} differ; {drawn} frames drawn; checks {ok} of {all}"
    );
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn marks_frame(marks: &[(usize, &str)], what: &str) -> Option<usize> {
    marks.iter().find(|(_, w)| *w == what).map(|(f, _)| *f)
}
