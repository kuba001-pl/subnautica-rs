use super::*;

const DT: f32 = 1.0 / 60.0;
const PLAYER: V3 = V3::new(0.0, 0.0, 0.0);

/// Updates until `event`; returns how many updates it took.
fn until(pda: &mut Pda, event: Event, target: Option<V3>) -> usize {
    for n in 1..1000 {
        if pda.update(DT, false, target, PLAYER).contains(&event) {
            return n;
        }
    }
    panic!("no {event:?}");
}

/// The update count of a `time`-second sequence at 60 Hz: `t` reaches
/// its end after `time / dt` updates (one more if the float sum falls
/// short), and the callback runs on the update after.
fn sequence_updates(time: f32) -> std::ops::RangeInclusive<usize> {
    let n = (time / DT).round() as usize;
    n + 1..=n + 2
}

#[test]
fn starts_closed() {
    let mut pda = Pda::new();
    assert_eq!(pda.state(), State::Closed);
    assert!(!pda.in_use() && !pda.active() && !pda.using_pda());
    assert!(pda.update(DT, true, None, PLAYER).is_empty());
    assert_eq!(pda.time, DT);
    assert_eq!(pda.delta_time, DT);
}

#[test]
fn opens_in_half_a_second_and_closes_in_0_3() {
    let mut pda = Pda::new();
    assert!(pda.open(None, PLAYER, false));
    assert_eq!(pda.state(), State::Opening);
    assert!(pda.in_use() && pda.active() && pda.using_pda() && !pda.focused());
    let n = until(&mut pda, Event::Activated, None);
    assert!(sequence_updates(TIME_DRAW).contains(&n), "{n}");
    assert_eq!(pda.state(), State::Opened);
    assert!(pda.focused());

    assert!(pda.close());
    assert_eq!(pda.state(), State::Closing);
    // `using_pda` clears at once; the model stays until `Deactivated`.
    assert!(pda.in_use() && pda.active() && !pda.using_pda() && !pda.focused());
    let n = until(&mut pda, Event::Deactivated, None);
    assert!(sequence_updates(TIME_HOLSTER).contains(&n), "{n}");
    assert_eq!(pda.state(), State::Closed);
    assert!(!pda.in_use() && !pda.active());
}

#[test]
fn refusals() {
    let mut pda = Pda::new();
    assert!(!pda.open(None, PLAYER, true), "in a cinematic");
    pda.set_ignore_input(true);
    assert!(!pda.open(None, PLAYER, false), "input ignored");
    pda.set_ignore_input(false);
    assert!(pda.open(None, PLAYER, false));
    assert!(!pda.open(None, PLAYER, false), "already in use");
    // Closing: still in use until `Deactivated`.
    until(&mut pda, Event::Activated, None);
    assert!(pda.close());
    // `Close` does not look at the state: while closing it runs again
    // (the sequence goes on from its `t`), as the game's.
    assert!(pda.close(), "Close while closing");
    assert!(!pda.open(None, PLAYER, false), "in use while closing");
    until(&mut pda, Event::Deactivated, None);
    assert!(pda.open(None, PLAYER, false));
}

#[test]
fn the_button_closes_only_when_focused() {
    let mut pda = Pda::new();
    pda.open(None, PLAYER, false);
    // Opening: not focused yet, so the button does nothing.
    let e = pda.update(DT, true, None, PLAYER);
    assert!(e.is_empty(), "{e:?}");
    until(&mut pda, Event::Activated, None);
    let e = pda.update(DT, true, None, PLAYER);
    assert_eq!(e, vec![Event::Close(CloseReason::Button)]);
}

#[test]
fn closes_past_the_target_distance() {
    let mut pda = Pda::new();
    let target = V3::new(2.0, 0.0, 0.0);
    pda.open(Some(target), PLAYER, false);
    // activeSqrDistance = 2² + 1 = 5: the player walking away stays
    // open at √5 − ε from the target and closes it at √5.
    let near = V3::new(2.0 - 5f64.sqrt() + 1e-6, 0.0, 0.0);
    assert!(pda.update(DT, false, Some(target), near).is_empty());
    let far = V3::new(2.0 - 5f64.sqrt() - 1e-9, 0.0, 0.0);
    assert_eq!(
        pda.update(DT, false, Some(target), far),
        vec![Event::Close(CloseReason::Distance)]
    );
    // A target that is gone closes too.
    let mut pda = Pda::new();
    pda.open(Some(target), PLAYER, false);
    assert_eq!(
        pda.update(DT, false, None, PLAYER),
        vec![Event::Close(CloseReason::Distance)]
    );
    // Without a target, distance never closes it.
    let mut pda = Pda::new();
    pda.open(None, PLAYER, false);
    assert!(
        pda.update(DT, false, Some(V3::new(1e6, 0.0, 0.0)), PLAYER)
            .is_empty()
    );
}
