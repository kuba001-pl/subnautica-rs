//! `ui --pulse` (M9u6b2, `docs/formats/ui.md` § The bars' animations):
//! the HUD's four bars driven through a scripted timeline (oxygen held at
//! three low levels, health low, a meal, a heal, the PDA opened and
//! closed), drawn each frame as the client does, against a replay written
//! here from the decompiled scripts (`CoroutineTween`, `OnPulse`, the
//! bars' `LateUpdate`) with the curves read straight from `sn-unity`:
//! - each frame, each bar's halo alpha in the draw list against the
//!   replay's (the clip's `m_Alpha` curve at the replayed time), N/N;
//! - each pulse's period, measured from the drawn halo, against the
//!   curves' `pulseDelay + pulseTime`;
//! - during the punches, the icon's drawn width against its rest width ×
//!   (the clip's scale + the punch), N/N;
//! - the coin turn while the PDA is open, logged (its random kick is not
//!   replayed).

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState};
use sn_install::GameData;
use sn_ui::hud::{BarKind, Hud, HudInputs, hud_mode};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_ui::tween::oscillation;
use sn_unity::{AnimationCurve, EditorCurveKind, Fields, Keyframe, Value};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 1.0 / 60.0;
const SECONDS: f32 = 56.0;

/// The timeline's inputs at time `t` (s): (oxygen, health, ate, healed,
/// PDA open).
fn script(t: f32, frame: usize) -> (f32, f32, Option<f32>, Option<f32>, bool) {
    let at = |s: f32| frame == (s / DT).round() as usize;
    let oxygen = match t {
        t if t < 3.0 => 45.0,
        t if t < 15.0 => 6.0,
        t if t < 27.0 => 3.0,
        t if t < 39.0 => 1.0,
        _ => 45.0,
    };
    let health = if (27.0..39.0).contains(&t) {
        12.0
    } else {
        100.0
    };
    let ate = at(40.0).then_some(40.0);
    let healed = at(42.0).then_some(30.0);
    let pda = (45.0..50.0).contains(&t);
    (oxygen, health, ate, healed, pda)
}

/// An `AnimationCurve` script field as `sn-unity`'s curve.
fn curve(f: &Fields, name: &str) -> Result<AnimationCurve> {
    let c = f
        .get(name)
        .and_then(Value::as_struct)
        .ok_or_else(|| format!("no {name}"))?;
    let n = |k: &Fields, x: &str| k.get(x).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let keys = c
        .get("m_Curve")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(Value::as_struct)
        .map(|k| Keyframe {
            time: n(k, "time"),
            value: n(k, "value"),
            in_slope: n(k, "inSlope"),
            out_slope: n(k, "outSlope"),
            weighted_mode: 0,
            in_weight: 0.0,
            out_weight: 0.0,
        })
        .collect();
    Ok(AnimationCurve {
        keys,
        pre_infinity: 2,
        post_infinity: 2,
    })
}

/// One bar's replay.
struct Replay {
    kind: BarKind,
    capacity: f32,
    delay_curve: AnimationCurve,
    time_curve: AnimationCurve,
    /// The clip's halo `m_Alpha`, and (health, food, water) the icon's x
    /// scale.
    halo: AnimationCurve,
    icon_x: Option<AnimationCurve>,
    length: f32,
    halo_node: usize,
    /// The icon's image (the child of `icon`) and its rest width.
    icon_image: Option<usize>,
    rest_width: f32,
    // `CoroutineTween` and `OnPulse`.
    running: bool,
    time: f32,
    duration: f32,
    delay: f32,
    pulse: f32,
    normalized: f32,
    // The period, from the drawn halo: the frames it rises past 0.5.
    rises: Vec<f32>,
    last_alpha: f32,
}

impl Replay {
    fn set_duration(&mut self, v: f32) {
        let v = v.max(0.0);
        if self.duration != v {
            let f = if self.duration > 0.0 {
                self.time / self.duration
            } else {
                0.0
            };
            self.duration = v;
            self.time = f * v;
        }
    }

    fn move_next(&mut self) {
        if !self.running || self.duration <= 0.0 {
            return;
        }
        self.time = (self.time + DT) % self.duration;
        let scalar = self.time / self.duration;
        let n = (self.duration * scalar - self.delay) / self.pulse;
        self.normalized = if n.is_finite() {
            n.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    fn clip_time(&self) -> f32 {
        (self.normalized * self.length).rem_euclid(self.length)
    }
}

/// The width of a draw item's vertices.
fn width(d: &sn_assets::UiDraw, node: usize) -> Option<f32> {
    let it = d.list.items.iter().find(|i| i.node == node)?;
    let (lo, hi) = it
        .vertices
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            (lo.min(v.pos[0]), hi.max(v.pos[0]))
        });
    Some(hi - lo)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = std::time::Instant::now();
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
    let (food, water) = sn_assets::survival_start(&sn_assets::read_assembly(game)?)?;
    // The replay's data, read from the scripts and clips again.
    let mut replays = Vec::new();
    for (b, script) in config.bars.iter().zip([
        "uGUI_OxygenBar",
        "uGUI_HealthBar",
        "uGUI_FoodBar",
        "uGUI_WaterBar",
    ]) {
        let (_, c) = tree
            .scripts(script)
            .find(|(n, _)| *n == b.root)
            .ok_or_else(|| format!("{script} not found"))?;
        let (_, f) = assets.script_fields(&layouts, c)?;
        let field = if f.get("animation").is_some() {
            "animation"
        } else {
            "pulseAnimation"
        };
        let p = f
            .get(field)
            .and_then(Value::as_pptr)
            .ok_or("no animation")?;
        let anim = assets
            .resolve(&c.object.file, p)?
            .ok_or("animation not found")?;
        let a = assets.legacy_animation(&anim)?;
        let clip_ref = assets
            .resolve(&anim.file, a.clips[0])?
            .ok_or("clip not found")?;
        let clip = assets.animation_clip(&clip_ref)?;
        let find = |kind: EditorCurveKind, path: &str| {
            clip.editor
                .iter()
                .find(|e| e.kind == kind && e.path == path)
                .map(|e| e.curves[0].clone())
        };
        let halo = find(EditorCurveKind::Float, "PulseHalo").ok_or("no halo curve")?;
        let icon_x = find(EditorCurveKind::Scale, "Icon");
        let root_path = &tree.nodes[b.root].path;
        let halo_node = tree
            .nodes
            .iter()
            .position(|n| n.path == format!("{root_path}/PulseHalo"))
            .ok_or("no PulseHalo")?;
        let icon_image = b.icon.and_then(|m| {
            tree.nodes
                .iter()
                .position(|n| n.parent == Some(m.icon) && n.name == "Icon")
        });
        println!(
            "{script}: {} ({} s), halo {} keys, icon scale {}, pulseReferenceCapacity {}",
            clip.name,
            clip.stop_time - clip.start_time,
            halo.keys.len(),
            if icon_x.is_some() {
                "animated"
            } else {
                "not animated"
            },
            b.pulse.reference_capacity
        );
        replays.push(Replay {
            kind: b.kind,
            capacity: f
                .get("pulseReferenceCapacity")
                .and_then(Value::as_f64)
                .ok_or("no pulseReferenceCapacity")? as f32,
            delay_curve: curve(&f, "pulseDelayCurve")?,
            time_curve: curve(&f, "pulseTimeCurve")?,
            halo,
            icon_x,
            length: clip.stop_time - clip.start_time,
            halo_node,
            icon_image,
            rest_width: 0.0,
            running: false,
            time: 0.0,
            duration: 0.0,
            delay: -1.0,
            pulse: -1.0,
            normalized: 0.0,
            rises: Vec::new(),
            last_alpha: 0.0,
        });
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
    let frames = (SECONDS / DT).round() as usize;
    let (mut halo_checks, mut halo_differ) = (0usize, 0usize);
    let (mut punch_checks, mut punch_differ) = (0usize, 0usize);
    // The phases whose period is measured: (name, from, to, bar).
    let phases = [
        ("oxygen 6 of 45", 3.0, 15.0, 0),
        ("oxygen 3 of 45", 15.0, 27.0, 0),
        ("oxygen 1 of 45", 27.0, 39.0, 0),
        ("health 12 of 100", 27.0, 39.0, 1),
    ];
    let mut phase_rises: Vec<Vec<f32>> = vec![Vec::new(); phases.len()];
    let mut turn_log = Vec::new();
    for frame in 0..frames {
        let t = (frame + 1) as f32 * DT;
        let (oxygen, health, ate, healed, pda) = script(t, frame);
        let inputs = HudInputs {
            active: true,
            mode: hud_mode(false, false, false),
            oxygen,
            oxygen_capacity: 45.0,
            health,
            max_health: 100.0,
            food,
            water,
            delta_time: DT,
            time: t,
            pda_open: pda,
            healed,
            ate,
            drank: None,
            depth: 0.0,
            compass: false,
        };
        let mut state = start_state.clone();
        state.apply(&hud.update(&inputs));
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        for (i, r) in replays.iter_mut().enumerate() {
            // The replay's frame: `OnEnable` on the first (its first
            // `MoveNext` at once, duration 0), else the coroutine; the
            // clip; `LateUpdate`.
            if frame == 0 {
                r.running = true;
                r.time = 0.0;
            }
            r.move_next();
            let want = r.halo.evaluate(r.clip_time());
            let has = match r.kind {
                BarKind::Oxygen => oxygen,
                BarKind::Health => health,
                BarKind::Food => food,
                BarKind::Water => water,
            };
            let x = 1.0 - (has / r.capacity).clamp(0.0, 1.0);
            r.delay = r.delay_curve.evaluate(x).max(0.0);
            r.pulse = r.time_curve.evaluate(x).max(0.0);
            let duration = r.delay + r.pulse;
            if r.duration > 0.0 && duration <= 0.0 {
                r.normalized = 0.0;
            }
            r.set_duration(duration);
            // The drawn halo (a fully transparent one may be culled).
            let drawn = d
                .list
                .items
                .iter()
                .find(|it| it.node == r.halo_node)
                .map_or(0.0, |it| it.alpha);
            halo_checks += 1;
            if (drawn - want).abs() > 1e-5 {
                halo_differ += 1;
                if halo_differ <= 10 {
                    println!(
                        "  t {t:.3} s {:?}: halo drawn {drawn:.5}, replay {want:.5}",
                        r.kind
                    );
                }
            }
            if r.last_alpha < 0.5 && drawn >= 0.5 {
                for (k, (_, from, to, bar)) in phases.iter().enumerate() {
                    if *bar == i && t > *from && t <= *to {
                        phase_rises[k].push(t);
                    }
                }
                r.rises.push(t);
            }
            r.last_alpha = drawn;
            // The icon: its rest width, then the punches.
            if let (Some(img), Some(ix)) = (r.icon_image, &r.icon_x) {
                let w = width(&d, img).unwrap_or(0.0);
                if frame == 0 {
                    r.rest_width = w;
                }
                let bar = &hud.bars[i];
                if bar.punch_tween.running() && r.rest_width > 0.0 {
                    let (o, _) = oscillation(
                        100.0,
                        5.0,
                        bar.punch_seed,
                        bar.punch_tween.time() / bar.punch_tween.duration(),
                    );
                    let want = ix.evaluate(r.clip_time()) + o * bar.punch_max_scale;
                    let got = w / r.rest_width;
                    punch_checks += 1;
                    if (got - want).abs() > 1e-3 {
                        punch_differ += 1;
                        if punch_differ <= 10 {
                            println!(
                                "  t {t:.3} s {:?}: icon width ×{got:.4}, replay ×{want:.4}",
                                r.kind
                            );
                        }
                    }
                }
                if r.kind == BarKind::Health && (44.9..52.0).contains(&t) && frame % 15 == 0 {
                    turn_log.push(format!(
                        "{t:.2} s {}: {:.1}° (width ×{:.3})",
                        if pda { "open" } else { "closed" },
                        hud.bars[i].rotation.unwrap_or(0.0),
                        w / r.rest_width
                    ));
                }
            }
        }
    }
    let mut ok = halo_differ == 0 && punch_differ == 0 && punch_checks > 0;
    println!("{halo_checks} halo alpha checks, {halo_differ} differ");
    println!("{punch_checks} icon punch checks, {punch_differ} differ");
    for (k, (name, from, to, bar)) in phases.iter().enumerate() {
        let r = &replays[*bar];
        let has: f32 = name
            .split(' ')
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let x = 1.0 - (has / r.capacity).clamp(0.0, 1.0);
        let want = r.delay_curve.evaluate(x).max(0.0) + r.time_curve.evaluate(x).max(0.0);
        let rises = &phase_rises[k];
        // The first rise follows the level change; periods from then on.
        let periods: Vec<f32> = rises.windows(2).map(|w| w[1] - w[0]).collect();
        let mean = periods.iter().sum::<f32>() / periods.len().max(1) as f32;
        let good = !periods.is_empty() && periods.iter().all(|p| (p - want).abs() <= DT * 1.01);
        ok &= good;
        println!(
            "{name} ({from}–{to} s): curves' delay + time {want:.4} s; {} pulses, measured period mean {mean:.4} s (min {:.4}, max {:.4}){}",
            rises.len(),
            periods.iter().copied().fold(f32::MAX, f32::min),
            periods.iter().copied().fold(f32::MIN, f32::max),
            if good {
                ""
            } else {
                "  DIFFERS (beyond one frame)"
            }
        );
    }
    for r in &replays {
        println!("{:?}: halo rose past 0.5 {} times", r.kind, r.rises.len());
    }
    println!("health icon turn: {}", turn_log.join("; "));
    println!("{frames} frames in {:.1} s", start.elapsed().as_secs_f64());
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
