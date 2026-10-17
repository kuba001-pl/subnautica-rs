use super::*;
use crate::clip::Key;

const DT: f32 = 0.02;

fn line(points: &[(f32, f32)]) -> Curve {
    let keys = points
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0))
        .collect::<Vec<_>>();
    Curve {
        keys: points
            .iter()
            .enumerate()
            .map(|(i, &(time, value))| Key {
                time,
                value,
                in_slope: if i == 0 { keys[0] } else { keys[i - 1] },
                out_slope: *keys.get(i).unwrap_or(&keys[keys.len() - 1]),
            })
            .collect(),
    }
}

fn config(oscillate: bool) -> NotifierConfig {
    let a = IconAnimator {
        duration: 0.7,
        x: line(&[(0.0, 0.5), (1.0, 0.5)]),
        y: line(&[(0.0, 0.5), (1.0, 0.0)]),
        scale: line(&[(0.0, 0.5), (1.0, 1.0)]),
        alpha: line(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]),
        oscillate,
        osc_offset: 0.0,
        osc_reduction: 130.0,
        osc_frequency: 5.0,
        osc_scale: 4.0,
    };
    NotifierConfig {
        canvas: 0,
        interval: 0.3,
        icon_size: 64.0,
        animators: vec![a.clone(), a],
    }
}

fn rect() -> Rect {
    Rect {
        x: -960.0,
        y: -540.0,
        width: 1920.0,
        height: 1080.0,
    }
}

struct Run {
    n: Notifier,
    next: usize,
}

impl Run {
    fn new(oscillate: bool) -> Run {
        Run {
            n: Notifier::new(config(oscillate), 7),
            next: 100,
        }
    }

    fn frame(&mut self, plays: usize) {
        let next = &mut self.next;
        let mut make = || {
            *next += 2;
            Icon {
                root: *next,
                foreground: *next + 1,
            }
        };
        let p: Vec<(SpriteId, V2, usize)> = (0..plays)
            .map(|k| (SpriteId(k as u64), [128.0, 128.0], FROM))
            .collect();
        self.n.update(&p, DT, rect(), &mut make);
    }
}

#[test]
fn the_first_goes_at_once_and_the_rest_wait_the_interval() {
    let mut r = Run::new(false);
    r.frame(3);
    assert_eq!(r.n.icons.len(), 4, "GetInstance makes four");
    assert_eq!(r.n.events.shown.len(), 1);
    assert_eq!(r.n.queue.len(), 2);
    let mut shown = vec![r.n.pda_time];
    for _ in 0..100 {
        r.frame(0);
        for &(_, t) in &r.n.events.shown {
            shown.push(t);
        }
    }
    assert_eq!(shown.len(), 3);
    for w in shown.windows(2) {
        let gap = w[1] - w[0];
        assert!((0.3 - 1e-6..0.3 + DT + 1e-6).contains(&gap), "{gap}");
    }
}

#[test]
fn an_icon_moves_down_from_the_centre_and_goes_back_to_the_pool() {
    let mut r = Run::new(false);
    r.frame(1);
    let icon = r.n.active[0].icon.unwrap();
    let s = &r.n.state;
    assert!(s.active[&icon.root] && s.enabled[&icon.foreground]);
    assert_eq!(s.sprite[&icon.foreground], Some(SpriteId(0)));
    assert_eq!(s.transform[&icon.root].size_delta, Some([64.0, 64.0]));
    assert_eq!(s.transform[&icon.foreground].size_delta, Some([64.0, 64.0]));
    assert_eq!(r.n.events.last_sibling, [icon.root]);
    // t = 0: at the centre of the rect's size, half size, invisible.
    assert_eq!(
        s.transform[&icon.root].anchored_position,
        Some([960.0, 540.0])
    );
    assert_eq!(s.transform[&icon.root].scale, Some([0.5, 0.5, 1.0]));
    assert_eq!(s.renderer_alpha[&icon.foreground], 0.0);
    let mut frames = 1;
    while r.n.busy() {
        r.frame(0);
        frames += 1;
    }
    // t reaches 1 after 0.7 ÷ 0.02 = 35 steps; released on the frame after.
    assert!((36..=37).contains(&frames), "{frames}");
    assert_eq!(r.n.events.released, [icon]);
    assert!(!r.n.state.active[&icon.root]);
    assert_eq!(r.n.pool.len(), 4);
    let p = r.n.state.transform[&icon.root].anchored_position.unwrap();
    assert!((p[1] - 0.0).abs() < 1e-3, "{p:?}");
}

#[test]
fn the_oscillation_is_added_to_the_scale() {
    let mut r = Run::new(true);
    r.frame(1);
    r.frame(0);
    let req = &r.n.active[0];
    let icon = req.icon.unwrap();
    // The second frame animates t = dt ÷ duration (the first one's step);
    // the oscillation's time is t × duration (offset 0).
    let t = DT / 0.7;
    let (o, o1) = oscillation(130.0, 5.0, req.random, t * 0.7);
    let want = config(true).animators[0].scale.evaluate(t);
    let s = r.n.state.transform[&icon.root].scale.unwrap();
    assert!((s[0] - (want + o * 4.0)).abs() < 1e-4, "{s:?}");
    assert!((s[1] - (want + o1 * 4.0)).abs() < 1e-4, "{s:?}");
    assert!((s[0] - want).abs() > 0.1, "the oscillation shows: {s:?}");
}

#[test]
fn the_pool_grows_by_four_and_released_icons_are_reused() {
    let mut r = Run::new(false);
    r.frame(6);
    for _ in 0..200 {
        r.frame(0);
    }
    // At most three icons out at once (0.7 s ÷ 0.3 s), so four suffice.
    assert_eq!(r.n.icons.len(), 4);
    assert_eq!(r.n.pool.len(), 4);
    // Five at once by hand: the pool is emptied and four more are made.
    let mut make_count = 0;
    let mut next = 1000;
    for k in 0..5 {
        let mut make = || {
            make_count += 1;
            next += 2;
            Icon {
                root: next,
                foreground: next + 1,
            }
        };
        r.n.time = -1.0;
        r.n.update(&[(SpriteId(k), [128.0, 128.0], TO)], DT, rect(), &mut make);
    }
    assert_eq!(make_count, 4);
    assert_eq!(r.n.icons.len(), 8);
    assert_eq!(r.n.active.len(), 5);
}

#[test]
fn a_wide_sprite_keeps_its_aspect() {
    let mut r = Run::new(false);
    let mut next = 0;
    let mut make = || {
        next += 2;
        Icon {
            root: next,
            foreground: next + 1,
        }
    };
    r.n.update(&[(SpriteId(9), [128.0, 64.0], FROM)], DT, rect(), &mut make);
    let icon = r.n.active[0].icon.unwrap();
    assert_eq!(
        r.n.state.transform[&icon.foreground].size_delta,
        Some([64.0, 32.0])
    );
    assert_eq!(
        r.n.state.material[&icon.foreground].vectors["_FillRect"],
        [-32.0, -16.0, 64.0, 32.0]
    );
}
