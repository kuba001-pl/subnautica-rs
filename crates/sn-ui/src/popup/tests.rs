use super::*;

const DT: f32 = 0.02;

fn config() -> PopupConfig {
    let skin = |b: usize| PopupSkin {
        root: b,
        group: b,
        ox: 40.0,
        oy: 40.0,
        title: b + 1,
        text: b + 2,
        controls: b + 3,
        image: Some(b + 4),
        default_sprite: Some(SpriteId(9)),
        sound_slide_in: None,
        sound_slide_out: None,
        default_sound: None,
    };
    PopupConfig {
        root: 0,
        group: 0,
        skins: vec![skin(10), skin(20)],
        default_duration: 5.0,
    }
}

fn layout() -> PopupLayout {
    let skin = Rect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 132.0,
    };
    PopupLayout {
        parent: Rect {
            x: -960.0,
            y: -540.0,
            width: 1920.0,
            height: 1080.0,
        },
        skins: vec![skin, skin],
    }
}

fn entry(title: &str) -> Entry {
    Entry {
        duration: 5.0,
        skin: UNLOCK,
        title: title.into(),
        text: "Power Cell".into(),
        sprite: None,
        sound: Some("unlock".into()),
        ..Entry::default()
    }
}

#[test]
fn trapezoid_step_and_approximately() {
    // Up over [0, 0.25], 1 for 5 s, down over [5.25, 5.5].
    assert_eq!(trapezoid(0.0, 0.25, 5.0, 0.25, 0.125), 0.5);
    assert_eq!(trapezoid(0.0, 0.25, 5.0, 0.25, 3.0), 1.0);
    assert_eq!(trapezoid(0.0, 0.25, 5.0, 0.25, 5.375), 0.5);
    assert_eq!(trapezoid(0.0, 0.25, 5.0, 0.25, 9.0), 0.0);
    assert_eq!(trapezoid(0.0, 0.25, 5.0, 0.25, -1.0), 0.0);
    // No ramps: 0 (the game's guard), never a division by zero.
    assert_eq!(trapezoid(0.0, 0.0, 0.0, 0.0, 1.0), 0.0);
    assert_eq!(step(0.0, 0.25, 0.1), 0.4);
    assert_eq!(step(0.0, 0.0, 0.1), 0.0);
    assert!(approximately(1.0, 1.0 + 5e-7));
    assert!(!approximately(1.0, 1.0 + 2e-6));
    assert!(approximately(0.0, 0.0));
    assert!(!approximately(0.0, 1e-30));
}

#[test]
fn an_entry_slides_in_stays_and_slides_out() {
    let mut p = Popup::new(config());
    let l = layout();
    // Frame 0: queued and shown at once (phase Zero, value 0, hidden).
    p.update(&[entry("A")], DT, &l);
    assert_eq!(p.events.shown.len(), 1);
    assert_eq!(p.phase, Phase::Zero);
    assert_eq!(p.state.group_alpha.get(&0), None);
    assert_eq!(p.state.group_alpha[&10], 1.0);
    assert_eq!(p.state.group_alpha[&20], 0.0);
    assert_eq!(p.state.text[&11], "A");
    assert_eq!(p.state.sprite[&14], Some(SpriteId(9)));
    assert_eq!(p.events.sounds, vec!["start unlock".to_string()]);
    // Frame 1: In, shown; the skin moves from beyond the left edge.
    p.update(&[], DT, &l);
    assert_eq!(p.phase, Phase::In);
    assert_eq!(p.state.group_alpha[&0], 1.0);
    let x = p.state.transform[&10].anchored_position.unwrap();
    let t = ease_out_sine(DT / TIME_IN);
    assert!((x[0] - (-400.0 + 440.0 * t)).abs() < 1e-3, "{x:?}");
    assert_eq!(x[1], 0.5 * (1080.0 - 132.0));
    let mut out_at = None;
    let mut done_at = None;
    for f in 2..400 {
        p.update(&[], DT, &l);
        for &(ph, at) in &p.events.phases {
            match ph {
                Phase::Out => out_at = Some((f, at)),
                Phase::Done => done_at = Some((f, at)),
                _ => {}
            }
        }
    }
    // Out at num ≥ 5.25 s, done at ≥ 5.5 s (started at frame 0's time).
    let (_, out) = out_at.unwrap();
    let (_, done) = done_at.unwrap();
    assert!((5.25 - 1e-4..5.25 + DT).contains(&out), "{out}");
    assert!((5.5 - 1e-4..5.5 + DT).contains(&done), "{done}");
    assert_eq!(p.state.group_alpha[&0], 0.0);
    assert!(p.current.is_none());
    assert!(p.idle());
    let x = p.state.transform[&10].anchored_position.unwrap();
    assert_eq!(x[0], -400.0);
}

#[test]
fn queued_entries_follow_one_another_and_the_sound_plays_once() {
    let mut p = Popup::new(config());
    let l = layout();
    p.update(&[entry("A"), entry("B"), entry("C")], DT, &l);
    // B's and C's sounds are dropped: A, with the same one, was still in
    // the queue when they were added.
    assert_eq!(p.queue.len(), 2);
    assert_eq!(p.queue[0].sound, None);
    assert_eq!(p.queue[1].sound, None);
    let mut shown = vec![(p.events.shown[0].0.title.clone(), p.events.shown[0].1)];
    for _ in 0..1000 {
        p.update(&[], DT, &l);
        for (e, at) in &p.events.shown {
            shown.push((e.title.clone(), *at));
        }
    }
    let titles: Vec<&str> = shown.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(titles, ["A", "B", "C"]);
    // Each next one is shown on the frame its predecessor is done.
    for w in shown.windows(2) {
        let gap = w[1].1 - w[0].1;
        assert!((5.5 - 1e-3..5.5 + 2.0 * DT).contains(&gap), "{gap}");
    }
}

#[test]
fn show_next_hides_the_current_one_from_where_it_is() {
    let mut p = Popup::new(config());
    let l = layout();
    p.update(&[entry("A")], DT, &l);
    for _ in 0..50 {
        p.update(&[], DT, &l);
    }
    assert_eq!(p.phase, Phase::One);
    let now = p.time;
    p.show_next(entry("B"));
    // Hide: start moved so that num is the start of the slide out.
    assert!((now - p.start - (TIME_IN + 5.0)).abs() < 1e-3);
    let mut seen = Vec::new();
    for _ in 0..40 {
        p.update(&[], DT, &l);
        seen.extend(p.events.phases.iter().map(|(ph, _)| *ph));
        if !p.events.shown.is_empty() {
            break;
        }
    }
    assert_eq!(seen, [Phase::Out, Phase::Done]);
    assert_eq!(p.current.as_ref().unwrap().title, "B");
}

#[test]
fn not_idle_while_an_entry_waits_to_slide_in() {
    // A caller may skip `update` while idle: the entry shown on the frame
    // its predecessor is done must still slide in.
    let mut p = Popup::new(config());
    let l = layout();
    assert!(p.idle());
    p.update(&[entry("A"), entry("B")], DT, &l);
    let mut b_in = false;
    for _ in 0..800 {
        if p.idle() {
            p.time += DT;
            continue;
        }
        p.update(&[], DT, &l);
        b_in |= p.current.as_ref().is_some_and(|c| c.title == "B") && p.phase == Phase::In;
    }
    assert!(b_in);
    assert!(p.idle());
    assert!(p.events.shown.is_empty());
}
