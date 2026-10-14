use super::*;

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
    assert_eq!(hud.bars[2], food);
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
