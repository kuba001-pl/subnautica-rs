use super::*;

const DT: f32 = 0.02;

fn config() -> MessagesConfig {
    MessagesConfig {
        canvas: 0,
        offset: [140.0, 140.0],
        y_spacing: 10.0,
        times: MessageTimes {
            fly_in: 0.3,
            delay: 5.0,
            fade_out: 0.6,
            invisible: 0.1,
            slide_rate: 7.0,
            snap: 1.0,
        },
        font_color: [1.0, 0.5, 0.25, 1.0],
    }
}

fn layout() -> MessageLayout {
    MessageLayout {
        canvas_rect: Rect {
            x: -960.0,
            y: -540.0,
            width: 1920.0,
            height: 1080.0,
        },
        entry_width: 1000.0,
        anchor_base: [0.0, 0.0],
        inverse_scale: 1.0,
    }
}

/// 10 units a character wide, 30 high.
fn measure(t: &str) -> [f32; 2] {
    [10.0 * t.chars().count() as f32, 30.0]
}

struct Run {
    m: Messages,
    next: usize,
}

impl Run {
    fn new() -> Run {
        Run {
            m: Messages::new(config()),
            next: 100,
        }
    }

    fn frame(&mut self, late: &[&str]) {
        let next = &mut self.next;
        let mut make = || {
            *next += 1;
            *next
        };
        let inputs = MessageInputs {
            delta_time: DT,
            early: Vec::new(),
            late: late.iter().map(|s| s.to_string()).collect(),
        };
        self.m.update(&inputs, &layout(), &measure, &mut make);
    }

    fn pos(&self, entry: usize) -> [f32; 2] {
        self.m.state.transform[&entry].anchored_position.unwrap()
    }

    fn alpha(&self, entry: usize) -> f32 {
        self.m.state.color[&entry][3]
    }
}

#[test]
fn a_message_flies_in_stays_fades_and_goes_back_to_the_pool() {
    let mut r = Run::new();
    r.frame(&["Hello"]);
    assert_eq!(r.m.entries.len(), 4, "GetEntry makes four");
    assert_eq!(r.m.pool.len(), 3);
    let e = r.m.messages[0].entry;
    assert_eq!(e, *r.m.entries.last().unwrap(), "the pool's last");
    assert!(r.m.state.active[&e]);
    assert_eq!(r.m.state.text[&e], "Hello");
    // Added at the frame's time: no flight yet, at the centre.
    assert_eq!(r.pos(e), [-25.0, 15.0]);
    assert_eq!(r.alpha(e), 1.0);
    assert_eq!(r.m.state.color[&e][..3], [1.0, 0.5, 0.25]);
    // At rest: the canvas's left + 140, its top − 140.
    for _ in 0..20 {
        r.frame(&[]);
    }
    let rest = [-960.0 + 140.0, 540.0 - 140.0];
    let p = r.pos(e);
    assert!(
        (p[0] - rest[0]).abs() < 1e-3 && (p[1] - rest[1]).abs() < 1e-3,
        "{p:?}"
    );
    // Fading over 0.6 s after 5.3 s; gone after 6 s.
    let mut frames = 21;
    while r.m.messages.len() == 1 {
        r.frame(&[]);
        frames += 1;
    }
    // Added at 0.02 s, so it ends at 6.02 s: removed on the first frame
    // past it (f32 sums).
    assert!((301..=302).contains(&frames), "{frames}");
    assert!(!r.m.state.active[&e]);
    assert_eq!(r.m.pool.len(), 4);
    // `offsetY` took the height and slides back to 0.
    assert!(r.m.offset_y > 0.0 && r.m.offset_y < 30.0);
    for _ in 0..100 {
        r.frame(&[]);
    }
    assert_eq!(r.m.offset_y, 0.0);
}

#[test]
fn the_alpha_follows_the_fade_line() {
    let mut r = Run::new();
    r.frame(&["A"]);
    let e = r.m.messages[0].entry;
    let end = r.m.messages[0].time_end;
    while r.m.time < end - 0.1 - 0.3 {
        r.frame(&[]);
    }
    let t = r.m.time;
    let v = ((end - 0.1 - t) / 0.6).clamp(0.0, 1.0);
    let want = ease_out_sine(v);
    assert!((r.alpha(e) - want).abs() < 1e-4, "{} vs {want}", r.alpha(e));
    assert!(r.alpha(e) > 0.0 && r.alpha(e) < 1.0);
}

#[test]
fn an_equal_text_is_counted_and_renewed() {
    let mut r = Run::new();
    r.frame(&["Full"]);
    for _ in 0..50 {
        r.frame(&[]);
    }
    let first_end = r.m.messages[0].time_end;
    r.frame(&["Full"]);
    assert_eq!(r.m.messages.len(), 1);
    let m = &r.m.messages[0];
    assert_eq!((m.num, r.m.state.text[&m.entry].as_str()), (2, "Full (x2)"));
    // Renewed without the flight.
    assert!((m.time_end - (r.m.time + 5.7)).abs() < 1e-5);
    assert!(m.time_end > first_end);
    r.frame(&["Full", "Full"]);
    let m = &r.m.messages[0];
    assert_eq!(r.m.state.text[&m.entry], "Full (x4)");
}

#[test]
fn messages_stack_and_the_pool_grows_by_four() {
    let mut r = Run::new();
    r.frame(&["a", "b", "c", "d", "e"]);
    assert_eq!(r.m.messages.len(), 5);
    assert_eq!(r.m.entries.len(), 8);
    assert_eq!(r.m.pool.len(), 3);
    for _ in 0..30 {
        r.frame(&[]);
    }
    // Each 30 high + 10 apart.
    let ys: Vec<f32> = r.m.messages.iter().map(|m| r.pos(m.entry)[1]).collect();
    for (k, y) in ys.iter().enumerate() {
        let want = 540.0 - 140.0 - 40.0 * k as f32;
        assert!((y - want).abs() < 1e-3, "{k}: {y} vs {want}");
    }
}

#[test]
fn the_message_after_a_removed_one_waits_a_frame() {
    let mut r = Run::new();
    r.frame(&["a", "b"]);
    // Both end at the same time: the game's loop removes "a", then skips
    // "b" (now at the index it just left).
    while r.m.messages.len() == 2 {
        r.frame(&[]);
    }
    assert_eq!(r.m.messages.len(), 1);
    assert_eq!(r.m.messages[0].text, "b");
    r.frame(&[]);
    assert!(r.m.messages.is_empty());
}

#[test]
fn an_early_call_uses_the_time_before_the_step() {
    let mut m = Messages::new(config());
    let mut n = 0;
    let mut make = || {
        n += 1;
        n
    };
    let inputs = MessageInputs {
        delta_time: 0.5,
        early: vec!["x".into()],
        late: vec!["y".into()],
    };
    m.update(&inputs, &layout(), &measure, &mut make);
    assert_eq!(m.messages[0].time_end, 6.0);
    assert_eq!(m.messages[1].time_end, 6.5);
    // An empty text adds nothing.
    m.add("", &mut make);
    assert_eq!(m.messages.len(), 2);
}
