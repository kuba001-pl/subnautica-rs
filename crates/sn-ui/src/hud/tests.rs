use super::*;
use crate::clip::{Channel, Key, Track};

fn key(time: f32, value: f32) -> Key {
    Key {
        time,
        value,
        in_slope: 0.0,
        out_slope: 0.0,
    }
}

fn step(time: f32, value: f32) -> Key {
    Key {
        time,
        value,
        in_slope: f32::INFINITY,
        out_slope: f32::INFINITY,
    }
}

/// A synthetic pulse: no pulse until 60 % is lost, then a 1 s delay and a
/// 1 s pulse; the clip (1 s) raises the halo's alpha to 1 at 0.5 s and
/// scales the icon to 2 at 0.5 s.
fn pulse(n: usize, icon: bool) -> Pulse {
    let tri = |top: f32, base: f32| Curve {
        keys: vec![key(0.0, base), key(0.5, top), key(1.0, base)],
    };
    let mut tracks = vec![Track {
        node: n + 5,
        channel: Channel::GroupAlpha(tri(1.0, 0.0)),
    }];
    if icon {
        tracks.push(Track {
            node: n + 6,
            channel: Channel::Scale([tri(2.0, 1.0), tri(2.0, 1.0), tri(1.0, 1.0)]),
        });
    }
    Pulse {
        reference_capacity: 100.0,
        delay: Curve {
            keys: vec![step(0.0, -1.0), step(0.6, 1.0)],
        },
        time: Curve {
            keys: vec![step(0.0, -1.0), step(0.6, 1.0)],
        },
        clip: Clip {
            name: "pulse".into(),
            length: 1.0,
            tracks,
        },
    }
}

fn circular() -> CircularBar {
    CircularBar {
        color: [1.0; 4],
        border: true,
        border_color: [1.0; 4],
        edge_width: 0.15,
        border_width: 0.4,
        round_corners: true,
        size: 52.0,
        width: 20.0,
        max_value: 1.0,
        overlay1_st: [2.0, 1.0, 0.0, 0.0],
        overlay2_st: [1.0, 3.0, 0.0, 0.0],
        overlay_shift: [0.3, 0.07],
        overlay_alpha: [0.5, 0.25],
    }
}

fn config() -> HudConfig {
    let bar = |kind, n: usize| BarConfig {
        kind,
        root: n,
        bar: n + 1,
        circular: circular(),
        text: n + 2,
        damp_speed: 0.1,
        overlays: (kind == BarKind::Oxygen).then_some(Overlays {
            speed: [0.25, 0.7],
            alpha: [0.75, 0.5],
        }),
        pulse: pulse(n, kind != BarKind::Oxygen),
        icon: (kind != BarKind::Oxygen).then_some(IconMotion {
            icon: n + 6,
            scale: [1.0; 3],
            spring_coef: 100.0,
            velocity_damp: 0.9,
            velocity_max: -1.0,
            random_velocity: 1000.0,
        }),
    };
    HudConfig {
        background_double: 1,
        background_quad: 2,
        bars: vec![
            bar(BarKind::Oxygen, 10),
            bar(BarKind::Health, 20),
            bar(BarKind::Food, 30),
            bar(BarKind::Water, 40),
        ],
    }
}

fn inputs() -> HudInputs {
    HudInputs {
        active: true,
        mode: 2,
        oxygen: 44.5,
        oxygen_capacity: 45.0,
        health: 80.0,
        max_health: 100.0,
        food: 50.5,
        water: 90.5,
        delta_time: 0.02,
        time: 4.0,
        pda_open: false,
        healed: None,
        ate: None,
        drank: None,
        depth: 0.0,
        compass: false,
    }
}

#[test]
fn the_mode_follows_the_game_modes_options() {
    assert_eq!(hud_mode(false, false, false), 2);
    assert_eq!(hud_mode(true, false, false), 1);
    assert_eq!(hud_mode(true, true, false), 1);
    assert_eq!(hud_mode(true, true, true), 0);
}

#[test]
fn inactive_hides_everything_and_runs_no_bar() {
    let mut hud = Hud::new(config());
    let s = hud.update(&HudInputs {
        active: false,
        ..inputs()
    });
    for n in [1, 2, 10, 20, 30, 40] {
        assert_eq!(s.active.get(&n), Some(&false), "node {n}");
    }
    assert!(s.text.is_empty() && s.material.is_empty());
    assert!(hud.bars.iter().all(|b| b.value.is_none()));
}

#[test]
fn survival_shows_four_bars_over_the_quad_background() {
    let mut hud = Hud::new(config());
    let s = hud.update(&inputs());
    assert!(!s.active[&1]);
    assert!(s.active[&2]);
    for n in [10, 20, 30, 40] {
        assert!(s.active[&n]);
    }
    // One SmoothDamp step from 0 towards each target.
    let step = |target: f32| {
        let mut v = 0.0;
        smooth_damp(0.0, target, &mut v, 0.1, f32::INFINITY, 0.02)
    };
    assert_eq!(s.material[&11].floats["_Value"], step(44.5 / 45.0));
    assert_eq!(s.material[&21].floats["_Value"], step(0.8));
    assert_eq!(s.material[&31].floats["_Value"], step(0.505));
    // Oxygen: RoundToInt(44.5) = 44 (half to even), whatever the bar shows.
    assert_eq!(s.text[&12], "44");
    // Health and food: CeilToInt(curr × capacity) of the smoothed value.
    assert_eq!(s.text[&22], (step(0.8) * 100.0).ceil().to_string());
    assert_eq!(s.text[&32], (step(0.505) * 100.0).ceil().to_string());
}

#[test]
fn the_bars_settle_on_their_values() {
    let mut hud = Hud::new(config());
    let mut s = UiState::default();
    for _ in 0..200 {
        s = hud.update(&inputs());
    }
    assert!((s.material[&11].floats["_Value"] - 44.5 / 45.0).abs() < 1e-4);
    assert_eq!(s.text[&22], "80");
    assert_eq!(s.text[&32], "51");
    assert_eq!(s.text[&42], "91");
}

#[test]
fn a_hidden_bar_keeps_its_value() {
    let mut hud = Hud::new(config());
    hud.update(&inputs());
    let food = hud.bars[2].clone();
    // Mode 1: oxygen and health only; food and water do not run.
    let s = hud.update(&HudInputs {
        mode: 1,
        food: 10.0,
        ..inputs()
    });
    assert!(!s.active[&30]);
    assert!(s.active[&1] && !s.active[&2]);
    let b = &hud.bars[2];
    assert_eq!(
        (b.curr, b.vel, &b.text, b.value),
        (food.curr, food.vel, &food.text, food.value)
    );
    // `OnDisable`: its tweens stopped.
    assert!(!b.enabled && !b.pulse_tween.running() && !b.subscribed);
    // What it set before stays set.
    assert_eq!(s.text[&32], food.text.unwrap());
}

#[test]
fn the_oxygen_overlays_scroll_with_time_and_fade_with_oxygen() {
    let mut hud = Hud::new(config());
    let s = hud.update(&inputs());
    let m = &s.material[&11];
    // xyz from the bar's fields, w = −time × speed.
    assert_eq!(m.vectors["_Overlay1_ST"], [2.0, 1.0, 0.0, -4.0 * 0.25]);
    assert_eq!(m.vectors["_Overlay2_ST"], [1.0, 3.0, 0.0, -4.0 * 0.7]);
    // 44.5/45 → the line through (0.5, 1) and (1, 0): 2 × (1 − 0.98889).
    let k = evaluate_line(0.5, 1.0, 1.0, 0.0, 44.5 / 45.0);
    let a = m.vectors["_OverlayAlpha"];
    assert!((a[0] - k * 0.75).abs() < 1e-6 && (a[1] - k * 0.5).abs() < 1e-6);
    // Half the oxygen or less: full alpha.
    let s = hud.update(&HudInputs {
        oxygen: 10.0,
        ..inputs()
    });
    assert_eq!(
        s.material[&11].vectors["_OverlayAlpha"],
        [0.75, 0.5, 0.0, 0.0]
    );
    // The other bars have no overlays.
    assert!(!s.material[&21].vectors.contains_key("_Overlay1_ST"));
}

#[test]
fn no_pulse_above_the_curves_start_the_halo_stays_at_the_clips_start() {
    let mut hud = Hud::new(config());
    let mut s = UiState::default();
    for _ in 0..100 {
        s = hud.update(&inputs());
    }
    // Health at 80 of 100: 20 % lost, below the 60 % where the curves
    // start: duration 0, the clip at 0.
    assert_eq!(hud.bars[1].pulse_tween.duration(), 0.0);
    assert_eq!(s.group_alpha.get(&25), Some(&0.0));
    assert_eq!(s.transform[&26].scale, Some([1.0, 1.0, 1.0]));
}

#[test]
fn a_low_bar_pulses_with_its_delay_and_time() {
    let mut hud = Hud::new(config());
    let low = HudInputs {
        health: 30.0,
        delta_time: 0.05,
        ..inputs()
    };
    // Frame 1 sets the duration (2 s: 1 s delay + 1 s pulse); then the
    // tween's time is 0.05 per frame.
    let mut alphas = Vec::new();
    for _ in 0..81 {
        let s = hud.update(&low);
        alphas.push(s.group_alpha[&25]);
    }
    assert_eq!(hud.bars[1].pulse_tween.duration(), 2.0);
    // Frame k (from 0) samples the tween's time after k updates of
    // 0.05 s, modulo 2: in the delay (time < 1) the alpha is 0; at 1.5 s
    // (normalized 0.5) the peak.
    let at = |k: usize| alphas[k];
    assert_eq!(at(10), 0.0);
    assert!((at(30) - 1.0).abs() < 1e-4, "{}", at(30));
    // One period (40 frames) later, the same.
    assert!((at(70) - at(30)).abs() < 1e-4);
    assert!((at(50) - 0.0).abs() < 1e-6);
}

#[test]
fn eating_punches_the_icon_then_it_comes_back() {
    let mut hud = Hud::new(config());
    hud.update(&inputs());
    let s = hud.update(&HudInputs {
        ate: Some(50.0),
        ..inputs()
    });
    let seed = hud.bars[2].punch_seed;
    // Punch(2.5, 1.5); its first MoveNext at t = 0.02 / 2.5.
    let (o, o1) = oscillation(100.0, 5.0, seed, 0.02 / 2.5);
    let sc = s.transform[&36].scale.unwrap();
    assert!((sc[0] - (1.0 + 1.5 * o)).abs() < 1e-5, "{sc:?}");
    assert!((sc[1] - (1.0 + 1.5 * o1)).abs() < 1e-5);
    assert_eq!(sc[2], 1.0);
    // 2.5 s later the punch has stopped: the clip's scale again.
    let mut s = s;
    for _ in 0..130 {
        s = hud.update(&inputs());
    }
    assert!(!hud.bars[2].punch_tween.running());
    assert_eq!(s.transform[&36].scale, Some([1.0, 1.0, 1.0]));
}

#[test]
fn opening_the_pda_turns_the_icons_after_a_whole_second() {
    let mut hud = Hud::new(config());
    let open = HudInputs {
        pda_open: true,
        ..inputs()
    };
    let mut turned = Vec::new();
    for _ in 0..600 {
        hud.update(&open);
        turned.push(hud.bars[1].rotation);
    }
    // 0.02 s frames: no step until PDA.time − the enable time ≥ 1 s.
    // (`PDA.time` is a float sum: the whole second is frame 50 or 51.)
    let first = turned.iter().position(Option::is_some);
    assert!(matches!(first, Some(50 | 51)), "{first:?}");
    // It settles on 180°.
    assert_eq!(hud.bars[1].rotation, Some(180.0));
    let s = hud.update(&open);
    let q = s.transform[&26].rotation.unwrap();
    assert!((q[1] - 1.0).abs() < 1e-6 && q[3].abs() < 1e-6, "{q:?}");
}
