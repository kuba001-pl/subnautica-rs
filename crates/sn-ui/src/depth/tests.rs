use super::*;
use crate::compass::Label;

fn ring() -> Compass {
    Compass {
        color: [1.0; 4],
        size: 3.9,
        position: [0.0, -0.54, -12.6],
        rotation: [-70.8, 0.0, 0.0],
        scale: [1.0; 3],
        scale_2d: 342.0,
        fov: 90.0,
        near: 0.0,
        far: 10.0,
        radius: 4.5,
        alpha_from: 0.375,
        alpha_to: 0.625,
        labels: vec![
            Label {
                node: 10,
                name: "N".into(),
                angle: 270.0,
            },
            Label {
                node: 11,
                name: "NE".into(),
                angle: 315.0,
            },
        ],
    }
}

fn config() -> DepthConfig {
    DepthConfig {
        shadow: 1,
        half_moon: 2,
        depth_text: 3,
        suffix_text: 4,
        submersible: [5, 6, 7, 8],
        compass: 9,
        labels: vec![(10, "N".into()), (11, "NE".into())],
        shadow_normal: SpriteId(100),
        half_moon_normal: SpriteId(101),
        half_moon_compass_normal: SpriteId(102),
        text_color_normal: [227, 243, 246, 255],
        meter_suffix: "m".into(),
        ring: ring(),
    }
}

fn inputs(depth: f32) -> HudInputs {
    HudInputs {
        active: true,
        mode: 2,
        oxygen: 45.0,
        oxygen_capacity: 45.0,
        health: 100.0,
        max_health: 100.0,
        food: 50.5,
        water: 90.5,
        delta_time: 0.02,
        time: 1.0,
        pda_open: false,
        healed: None,
        ate: None,
        drank: None,
        depth,
        compass: false,
        yaw: 0.0,
    }
}

#[test]
fn the_first_frame_initializes_and_shows_the_player_depth() {
    let mut d = DepthCompass::new(config());
    let s = d.update(&inputs(12.7)).clone();
    assert_eq!(d.mode, DepthMode::Player);
    assert_eq!(s.text.get(&3).map(String::as_str), Some("12"));
    assert_eq!(s.text.get(&4).map(String::as_str), Some("m"));
    // The submersible suffix and the labels' texts from the language.
    assert_eq!(s.text.get(&7).map(String::as_str), Some("m"));
    assert_eq!(s.text.get(&11).map(String::as_str), Some("NE"));
    for n in [1, 2, 3, 4] {
        assert_eq!(s.enabled.get(&n), Some(&true), "node {n}");
    }
    for n in [5, 6, 7, 8, 9, 10, 11] {
        assert_eq!(s.enabled.get(&n), Some(&false), "node {n}");
    }
    assert_eq!(s.sprite.get(&1), Some(&Some(SpriteId(100))));
    assert_eq!(s.sprite.get(&2), Some(&Some(SpriteId(101))));
    let m = &s.material[&1];
    assert_eq!(m.floats["_SrcFactor"], 2.0);
    assert_eq!(m.floats["_DstFactor"], 10.0);
    assert!(m.keywords["ALPHA_PREMULTIPLY"]);
    assert_eq!(
        s.color[&3],
        [227.0 / 255.0, 243.0 / 255.0, 246.0 / 255.0, 1.0]
    );
    assert_eq!(s.color[&4], s.color[&3]);
}

#[test]
fn the_text_is_the_floor_and_changes_only_with_the_integer() {
    let mut d = DepthCompass::new(config());
    for (depth, text) in [
        (0.0, "0"),
        (0.99, "0"),
        (1.0, "1"),
        (199.5, "199"),
        (2500.2, "2500"),
    ] {
        let s = d.update(&inputs(depth));
        assert_eq!(s.text[&3], text, "depth {depth}");
    }
    assert_eq!(d.player_depth, 2500);
}

#[test]
fn the_pda_hides_the_depth_and_keeps_its_text() {
    let mut d = DepthCompass::new(config());
    d.update(&inputs(5.5));
    let open = HudInputs {
        pda_open: true,
        ..inputs(9.0)
    };
    let s = d.update(&open).clone();
    assert_eq!(d.mode, DepthMode::None);
    for n in [1, 2, 3, 4] {
        assert_eq!(s.enabled.get(&n), Some(&false), "node {n}");
    }
    // `UpdateDepth` sets no text in `None`.
    assert_eq!(s.text[&3], "5");
    let s = d.update(&inputs(9.0));
    assert_eq!(s.enabled.get(&3), Some(&true));
    assert_eq!(s.text[&3], "9");
    // An inactive HUD (the player not running) hides it too.
    let off = HudInputs {
        active: false,
        ..inputs(9.0)
    };
    assert_eq!(d.update(&off).enabled.get(&2), Some(&false));
}

#[test]
fn a_compass_shows_the_compass_and_its_half_moon() {
    let mut d = DepthCompass::new(config());
    d.update(&inputs(3.0));
    let with = HudInputs {
        compass: true,
        yaw: 0.0,
        ..inputs(3.0)
    };
    let s = d.update(&with).clone();
    assert_eq!(s.sprite[&2], Some(SpriteId(102)));
    for n in [9, 10, 11] {
        assert_eq!(s.enabled.get(&n), Some(&true), "node {n}");
    }
    // The PDA hides the compass too; the half moon keeps its sprite while
    // hidden and gets the normal one back when it shows.
    let open = HudInputs {
        pda_open: true,
        ..with
    };
    let s = d.update(&open).clone();
    assert_eq!(s.enabled.get(&9), Some(&false));
    assert_eq!(s.sprite[&2], Some(SpriteId(102)));
    let s = d.update(&inputs(3.0));
    assert_eq!(s.sprite[&2], Some(SpriteId(101)));
    assert_eq!(s.enabled.get(&10), Some(&false));
}

#[test]
fn the_direction_turns_the_ring_and_moves_the_labels_only_on_a_change() {
    let mut d = DepthCompass::new(config());
    let with = |yaw: f32| HudInputs {
        compass: true,
        yaw,
        ..inputs(3.0)
    };
    let s = d.update(&with(90.0)).clone();
    assert_eq!(d.direction, 0.25);
    let angle = s.material[&9].floats["_Angle"];
    assert!((angle + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    let (pos, alpha) = ring().label(270.0, 0.25);
    assert_eq!(s.transform[&10].anchored_position, Some(pos));
    assert_eq!(s.color[&10], [1.0, 1.0, 1.0, alpha]);
    // 450° wraps to the same direction: nothing is set again.
    d.state.color.remove(&10);
    let s = d.update(&with(450.0));
    assert!(!s.color.contains_key(&10));
    // Without the compass the direction is not set.
    let s = d.update(&inputs(3.0)).clone();
    assert_eq!(d.direction, 0.25);
    assert!(!s.color.contains_key(&10));
}
