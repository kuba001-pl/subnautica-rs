use super::*;

#[test]
fn smooth_damp_matches_hand_computed_steps() {
    // smoothTime 0.1, dt 0.02: ω = 20, x = 0.4,
    // exp = 1 / (1 + 0.4 + 0.0768 + 0.01504) = 0.670313…
    // From 0 to 1, velocity 0: change = −1, temp = −0.4,
    // velocity = (0 + 20 × 0.4) × exp = 5.362505…,
    // value = 1 + (−1 − 0.4) × exp = 0.061561…
    let mut v = 0.0;
    let x = smooth_damp(0.0, 1.0, &mut v, 0.1, f32::INFINITY, 0.02);
    let exp = 1.0 / (1.0 + 0.4 + 0.48 * 0.16 + 0.235 * 0.064);
    assert!((x - (1.0 - 1.4 * exp)).abs() < 1e-6, "{x}");
    assert!((v - 8.0 * exp).abs() < 1e-5, "{v}");
    // It approaches and never passes the target.
    let mut c = x;
    for _ in 0..200 {
        c = smooth_damp(c, 1.0, &mut v, 0.1, f32::INFINITY, 0.02);
        assert!(c <= 1.0);
    }
    assert!((c - 1.0).abs() < 1e-4, "{c}");
}

#[test]
fn smooth_damp_clamps_the_change_by_max_speed() {
    // maxSpeed 1 × smoothTime 0.5: the change seen is at most 0.5, so the
    // value moves towards 9.5, not 0.
    let mut v = 0.0;
    let x = smooth_damp(10.0, 0.0, &mut v, 0.5, 1.0, 0.1);
    let (omega, dt) = (4.0f32, 0.1f32);
    let k = omega * dt;
    let exp = 1.0 / (1.0 + k + 0.48 * k * k + 0.235 * k * k * k);
    let temp = omega * 0.5 * dt;
    assert!((x - (9.5 + (0.5 + temp) * exp)).abs() < 1e-5, "{x}");
}

#[test]
fn smooth_damp_snaps_on_overshoot() {
    // A large velocity towards the target past it in one step: the value
    // is the target and the velocity 0.
    let mut v = -1000.0;
    let x = smooth_damp(1.0, 0.0, &mut v, 0.1, f32::INFINITY, 0.1);
    assert_eq!(x, 0.0);
    assert_eq!(v, 0.0);
}

#[test]
fn a_sequence_moves_over_its_time_then_ends_one_update_later() {
    let mut s = Sequence::default();
    assert!(!s.active());
    s.set_from(0.1, false, true);
    let mut ended = None;
    for i in 1..=10 {
        if s.update(0.025) {
            ended = Some(i);
            break;
        }
    }
    // 0.025, 0.05, … 0.1: t = 1 after 4 updates; the fifth ends it.
    assert_eq!(ended, Some(5));
    assert_eq!(s.t(), 1.0);
    assert!(!s.active());
    // Back down from where it is.
    s.set(0.5, false);
    s.update(0.25);
    assert!((s.t() - 0.5).abs() < 1e-6);
}

#[test]
fn a_zero_time_sequence_jumps_and_force_state_ends_next_update() {
    let mut s = Sequence::default();
    s.set_from(0.0, false, true);
    assert!(!s.update(0.016));
    assert_eq!(s.t(), 1.0);
    assert!(s.update(0.016));
    s.force_state(false);
    assert_eq!(s.t(), 0.0);
    assert!(s.active());
    assert!(s.update(0.016));
    assert!(!s.active());
}

#[test]
fn a_loop_tween_wraps_and_keeps_its_fraction_when_the_duration_changes() {
    let mut t = CoroutineTween::new(TweenMode::Loop, 0.0);
    // Duration 0: started, but no update.
    let (stopped, calls) = t.start(0.1);
    assert!(!stopped && calls.update.is_none() && t.running());
    t.set_duration(2.0);
    assert_eq!(t.move_next(0.5).update, Some(0.25));
    // 0.5 of 2 s is a quarter; at 4 s the time keeps the quarter: 1 s.
    t.set_duration(4.0);
    assert_eq!(t.time(), 1.0);
    // 1 + 3.5 = 4.5 → 0.5 of 4.
    assert_eq!(t.move_next(3.5).update, Some(0.125));
}

#[test]
fn a_once_tween_ends_at_one_and_a_restart_stops_the_old_run_first() {
    let mut t = CoroutineTween::new(TweenMode::Once, 1.0);
    let (stopped, calls) = t.start(0.25);
    assert!(!stopped);
    assert_eq!(calls.update, Some(0.25));
    let (stopped, calls) = t.start(0.5);
    assert!(stopped);
    assert_eq!(calls.update, Some(0.5));
    let calls = t.move_next(0.75);
    assert_eq!(
        calls,
        TweenCalls {
            update: Some(1.0),
            stopped: true
        }
    );
    assert!(!t.running());
    assert_eq!(t.move_next(0.1), TweenCalls::default());
}

#[test]
fn approximately_is_relative_with_a_tiny_floor() {
    assert!(approximately(1.0, 1.0 + 5e-7));
    assert!(!approximately(1.0, 1.0 + 2e-6));
    assert!(approximately(0.0, 0.0));
    assert!(!approximately(0.0, 1e-30));
}

#[test]
fn coin_rotation_waits_for_a_whole_second_then_steps_once_a_frame() {
    let (mut current, mut prev, mut vel) = (0.0f32, 0.0f32, 0.0f32);
    // 0.5 s in: no whole second, nothing moves.
    assert!(!coin_rotation(
        &mut current,
        180.0,
        &mut prev,
        0.5,
        &mut vel,
        100.0,
        0.9,
        -1.0
    ));
    assert_eq!((current, prev), (0.0, 0.0));
    // 1.0 s: one 0.02 s step. velocity = (0 + 100 × 180 × 0.02) × 0.9 =
    // 324, current = 324 × 0.02 = 6.48.
    assert!(coin_rotation(
        &mut current,
        180.0,
        &mut prev,
        1.0,
        &mut vel,
        100.0,
        0.9,
        -1.0
    ));
    assert!((vel - 324.0).abs() < 1e-3, "{vel}");
    assert!((current - 6.48).abs() < 1e-4, "{current}");
    assert!((prev - 0.02).abs() < 1e-7);
    // More than 20 s behind: one step of the whole time.
    let (mut c2, mut p2, mut v2) = (0.0f32, 0.0f32, 0.0f32);
    coin_rotation(&mut c2, 1.0, &mut p2, 30.0, &mut v2, 1.0, 1.0, 0.5);
    // velocity = (0 + 1 × 1 × 30) × 1 = 30, clamped to 0.5; c = 15.
    assert_eq!((v2, c2, p2), (0.5, 15.0, 30.0));
}

#[test]
fn oscillation_starts_from_its_seed_and_decays() {
    let (o, o1) = oscillation(100.0, 5.0, 0.25, 0.0);
    // amplitude 0.5, sin(0.25 × 2π) = 1: (1, 0).
    assert!((o - 1.0).abs() < 1e-6 && o1.abs() < 1e-6);
    let (o, o1) = oscillation(100.0, 5.0, 0.0, 1.0);
    // amplitude 0.5 / 100, sin(5 × 2π) ≈ 0.
    assert!((o - 0.005).abs() < 1e-5 && (o1 - 0.005).abs() < 1e-5);
}
