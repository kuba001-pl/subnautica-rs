use super::*;
use crate::clip::Key;

const DT: f32 = 1.0 / 60.0;

fn times() -> VignetteTimes {
    VignetteTimes {
        begin_fade_delay: 0.0,
        fade_out_delay: 6.0,
        rate_in: 5.0,
        rate_out: 2.0,
        full: 0.98,
        clear: 0.02,
    }
}

fn config() -> DeathConfig {
    DeathConfig {
        black_overlay: 1,
        vignette: times(),
        loading_text: 2,
        loading_background: FaderConfig {
            group: 3,
            fade_in_time: 1.0,
            fade_out_time: 0.5,
            use_unscaled_time: true,
            initial_state: false,
        },
        loading: "Loading...".into(),
        overlays: vec![OverlayConfig {
            graphic: 4,
            // A straight line from (0, 0) to (1, 1).
            curve: Curve {
                keys: vec![
                    Key {
                        time: 0.0,
                        value: 0.0,
                        in_slope: 1.0,
                        out_slope: 1.0,
                    },
                    Key {
                        time: 1.0,
                        value: 1.0,
                        in_slope: 1.0,
                        out_slope: 1.0,
                    },
                ],
            },
        }],
    }
}

/// Frame `n`'s inputs at 60 fps.
fn frame(n: u32) -> DeathInputs {
    DeathInputs {
        delta_time: DT,
        unscaled_delta_time: DT,
        time: n as f32 * DT,
        ..DeathInputs::default()
    }
}

#[test]
fn the_fader_starts_hidden_shows_at_once_and_fades_out_over_fade_in_time() {
    let mut s = DeathScreens::new(config());
    let st = s.update(&frame(1));
    // `Awake`'s `SetState(false)`, then the first `ApplyState`.
    assert_eq!(st.group_alpha[&3], 0.0);
    assert!(!s.fader.sequence.active());
    // `Show`: drawn from the next frame's `Update` on.
    let st = s.update(&DeathInputs {
        respawn_show: true,
        ..frame(2)
    });
    assert_eq!(st.group_alpha[&3], 0.0);
    assert_eq!(st.text[&2], "Loading...");
    assert_eq!(s.update(&frame(3)).group_alpha[&3], 1.0);
    // `Hide` → `FadeOut(fadeInTime)`: 1 s, not `fadeOutTime`'s 0.5 s.
    s.update(&DeathInputs {
        respawn_hide: true,
        ..frame(4)
    });
    let mut n = 5;
    let mut frames = 0;
    while s.fader.sequence.t() > 0.0 {
        let st = s.update(&frame(n));
        let a = st.group_alpha[&3];
        assert!((a - fader_alpha(s.fader.sequence.t())).abs() < 1e-6);
        n += 1;
        frames += 1;
        assert!(frames < 100);
    }
    // 60 steps of 1/60 s, or one more when the f32 sum stops just above 0.
    assert!((60..=61).contains(&frames), "{frames}");
    assert_eq!(s.update(&frame(n)).group_alpha[&3], 0.0);
}

#[test]
fn fade_to_black_waits_a_frame_lerps_in_holds_then_fades_out_and_resets() {
    let mut s = DeathScreens::new(config());
    let st = s.update(&frame(0));
    assert_eq!(st.color[&1], [0.0; 4]);
    assert!(!st.enabled.contains_key(&1));
    // The event in frame 10.
    s.update(&DeathInputs {
        vignette: Some(DeathType::FadeToBlack),
        ..frame(10)
    });
    // Frame 11: `Update` still inactive, then `BeginDeathFade`.
    let st = s.update(&frame(11));
    assert_eq!(st.color[&1][3], 0.0);
    assert!(st.enabled[&1]);
    // Frame 12: the first lerp.
    let st = s.update(&frame(12));
    assert!((st.color[&1][3] - DT * 5.0).abs() < 1e-6);
    // Fully black once it passed 0.98; still black at 5.9 s.
    let mut n = 13;
    while s.death.alpha < 1.0 {
        s.update(&frame(n));
        n += 1;
    }
    let at = n - 1;
    let mut a = 0.0f32;
    for _ in 12..at {
        a += (1.0 - a) * DT * 5.0;
    }
    assert!(a >= 0.98, "{a}");
    while (n as f32 * DT) < 10.0 * DT + 5.9 {
        assert_eq!(s.update(&frame(n)).color[&1][3], 1.0);
        n += 1;
    }
    // `BeginFadeOut` 6 s after the trigger; then `Lerp` by dt × 2 to 0.
    let due = 10.0 * DT + 6.0;
    while (n as f32 * DT) < due {
        assert_eq!(s.update(&frame(n)).color[&1][3], 1.0);
        n += 1;
    }
    s.update(&frame(n));
    assert!(!s.death.fade_in);
    let st = s.update(&frame(n + 1));
    assert!((st.color[&1][3] - (1.0 - DT * 2.0)).abs() < 1e-6);
    let mut m = n + 2;
    while s.death.active {
        s.update(&frame(m));
        m += 1;
        assert!(m < n + 1000);
    }
    let st = s.state();
    assert!(!st.enabled[&1]);
    assert_eq!(st.color[&1][3], 0.0);
}

#[test]
fn cut_to_black_is_black_at_once_and_a_second_trigger_is_ignored() {
    let mut s = DeathScreens::new(config());
    let st = s.update(&DeathInputs {
        vignette: Some(DeathType::CutToBlack),
        ..frame(1)
    });
    assert_eq!(st.color[&1][3], 1.0);
    assert!(st.enabled[&1]);
    let queued = s.death.invokes.len();
    s.update(&DeathInputs {
        vignette: Some(DeathType::FadeToBlack),
        ..frame(2)
    });
    assert_eq!(s.death.invokes.len(), queued);
}

#[test]
fn the_overlay_follows_its_curve_and_is_disabled_at_zero() {
    let mut s = DeathScreens::new(config());
    let st = s.update(&frame(0));
    assert!(!st.enabled[&4]);
    let st = s.update(&DeathInputs {
        suffocation: Some(0.25),
        ..frame(1)
    });
    assert!(st.enabled[&4]);
    assert!((st.renderer_alpha[&4] - 0.25).abs() < 1e-6);
    // `OnKill`'s `Set(0, 1)` after the suffocation's.
    let st = s.update(&DeathInputs {
        suffocation: Some(0.0),
        killed: true,
        ..frame(2)
    });
    assert_eq!(st.renderer_alpha[&4], 1.0);
    assert_eq!(s.overlay_alpha(0), 1.0);
    // Back to 0: disabled, the last alpha kept but not drawn.
    let st = s.update(&DeathInputs {
        suffocation: Some(0.0),
        ..frame(3)
    });
    assert!(!st.enabled[&4]);
    assert_eq!(s.overlay_alpha(0), 0.0);
    // An id the script does not have is ignored, as `Set` does.
    s.set_overlay(7, 1.0);
}

#[test]
fn death_events_map_to_the_types() {
    assert_eq!(
        DeathType::from_event("DeathFade"),
        Some(DeathType::FadeToBlack)
    );
    assert_eq!(
        DeathType::from_event("DeathCut"),
        Some(DeathType::CutToBlack)
    );
    // `cine_attack_kill`'s lower-case `deathCut` has no receiver method.
    assert_eq!(DeathType::from_event("deathCut"), None);
}
