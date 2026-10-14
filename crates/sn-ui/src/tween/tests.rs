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
