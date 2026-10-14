use super::*;

/// Icons `Default` (node 10, graphic 20), `Hand` (11, 21), `PackUp` (12,
/// 22), `Progress` (13, 23); texts 1–4; canvas 5; progress 6 and 7.
fn config() -> ReticleConfig {
    let icon = |kind, node| ReticleIcon {
        kind,
        node,
        graphics: vec![node + 10],
    };
    ReticleConfig {
        icon_canvas: 5,
        icon_canvas_scale: [1.0, 1.0, 1.0],
        icons: vec![
            icon(IconType::Default, 10),
            icon(IconType::Hand, 11),
            icon(IconType::PackUp, 12),
            icon(IconType::Progress, 13),
        ],
        icon_scale_speed: 10.0,
        texts: [1, 2, 3, 4],
        progress_image: 6,
        progress_text: 7,
        add_button_format: "{0} ({1})".into(),
        progress_format: "{0:P0}".into(),
    }
}

fn frame(icon: IconType, hand: &str) -> ReticleInputs {
    let mut i = ReticleInputs {
        delta_time: 0.02,
        icon,
        ..ReticleInputs::default()
    };
    i.texts[HAND] = hand.into();
    i
}

fn alpha(r: &HandReticle, graphic: usize) -> Option<f32> {
    r.state.renderer_alpha.get(&graphic).copied()
}

#[test]
fn it_starts_settled_on_the_default_icon() {
    let mut r = HandReticle::new(config());
    assert_eq!(r.icon_type, IconType::Default);
    let s = r.update(&ReticleInputs {
        delta_time: 0.02,
        ..ReticleInputs::default()
    });
    // Nothing fades; the texts are empty and inactive.
    assert!(s.renderer_alpha.is_empty());
    assert_eq!(s.active.get(&1), Some(&false));
    assert_eq!(s.text.get(&1).map(String::as_str), Some(""));
    // The progress is set once (0, "0%").
    assert_eq!(s.fill_amount.get(&6), Some(&0.0));
    assert_eq!(s.text.get(&7).map(String::as_str), Some("0%"));
}

#[test]
fn a_new_icon_fades_in_over_a_tenth_of_a_second() {
    let mut r = HandReticle::new(config());
    let mut hand = Vec::new();
    let mut default = Vec::new();
    for _ in 0..7 {
        r.update(&frame(IconType::Hand, "Pick up X"));
        hand.push(alpha(&r, 21).unwrap());
        default.push(alpha(&r, 20).unwrap());
    }
    // 0.02 s steps of 0.1 s: 0.2 a frame (to f32 precision).
    let want = [0.2, 0.4, 0.6, 0.8, 1.0, 1.0, 1.0];
    for (a, b) in hand.iter().zip(want) {
        assert!((a - b).abs() < 1e-5, "{hand:?}");
    }
    for (a, b) in default.iter().zip(want) {
        assert!((a - (1.0 - b)).abs() < 1e-5, "{default:?}");
    }
    assert_eq!(r.state.active.get(&11), Some(&true));
    assert_eq!(r.state.active.get(&10), Some(&false));
    // The text is shown.
    assert_eq!(r.state.active.get(&1), Some(&true));
    assert_eq!(r.state.text.get(&1).map(String::as_str), Some("Pick up X"));
    // Back to nothing: the text hides, `Default` comes back.
    r.update(&frame(IconType::Default, ""));
    assert_eq!(r.state.active.get(&1), Some(&false));
    assert!((alpha(&r, 20).unwrap() - 0.2).abs() < 1e-5);
}

#[test]
fn hidden_drops_every_icon_at_once() {
    let mut r = HandReticle::new(config());
    let mut i = frame(IconType::Hand, "");
    i.hidden = true;
    r.update(&i);
    assert_eq!(r.icon_type, IconType::None);
    assert_eq!(alpha(&r, 20), Some(0.0));
    assert_eq!(r.state.active.get(&10), Some(&false));
    assert_eq!(alpha(&r, 21), None);
    // Shown again: `Default` fades in from 0.
    r.update(&frame(IconType::Default, ""));
    assert!((alpha(&r, 20).unwrap() - 0.2).abs() < 1e-5);
}

#[test]
fn the_icon_scale_lerps_to_its_target() {
    let mut r = HandReticle::new(config());
    let mut i = frame(IconType::Hand, "");
    i.icon_scale = 2.0;
    r.update(&i);
    // 1 + (2 − 1) × 0.02 × 10.
    assert!((r.scale - 1.2).abs() < 1e-6);
    i.icon_scale = 5.0; // clamped to 2
    r.update(&i);
    assert!((r.scale - 1.36).abs() < 1e-6);
    let t = r.state.transform.get(&5).unwrap().scale.unwrap();
    assert_eq!(t, [r.scale, r.scale, 1.0]);
}

#[test]
fn the_progress_is_set_on_a_change() {
    let mut r = HandReticle::new(config());
    let mut i = frame(IconType::Progress, "");
    i.progress = 0.456;
    r.update(&i);
    assert_eq!(r.state.fill_amount.get(&6), Some(&0.456));
    assert_eq!(r.state.text.get(&7).map(String::as_str), Some("46%"));
    i.progress = 3.0;
    r.update(&i);
    assert_eq!(r.state.fill_amount.get(&6), Some(&1.0));
    assert_eq!(r.state.text.get(&7).map(String::as_str), Some("100%"));
}

#[test]
fn texts_get_the_button() {
    let c = config();
    let b = "<sprite name=\"MouseButtonLeft\" color=#ADF8FFFF>";
    assert_eq!(
        get_text(&c, "Pick up Titanium", Some(b)),
        format!("Pick up Titanium ({b})")
    );
    assert_eq!(get_text(&c, "Titanium", None), "Titanium");
    assert_eq!(get_text(&c, "", None), "");
    assert_eq!(
        composite("{{{0}}} {1}", &["a", "b"]).as_deref(),
        Some("{a} b")
    );
    assert_eq!(composite("{0:N}", &["a"]), None);
    assert_eq!(composite("{2}", &["a"]), None);
    assert_eq!(IconType::from_i32(8), Some(IconType::PackUp));
    assert_eq!(IconType::from_i32(11), None);
}
