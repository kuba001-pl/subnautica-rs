//! Hand-computed cases of `uGUI_CanvasScaler`'s Static mode.

use super::*;

fn hud(scale_mode: i32) -> ScalerSettings {
    ScalerSettings {
        reference_resolution: [1920.0, 1080.0],
        mode: MODE_STATIC,
        distance: 1.0,
        scale_mode,
    }
}

fn close(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps
}

#[test]
fn at_the_reference_resolution() {
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    let p = place(&hud(1), &cam, 60.0, [1920, 1080], 1.0).unwrap();
    assert_eq!(p.position, [0.0, 0.0, 1.0]);
    assert_eq!(p.size, [1920.0, 1080.0]);
    assert_eq!(p.scale_factor, 1.0);
    // The frustum at 1 m: 2 tan 30° = 1.154 70 high, × 16/9 wide; per
    // pixel 1.154 70 / 1080.
    let per_pixel = 2.0 * 30f32.to_radians().tan() / 1080.0;
    assert!(close(p.scale[0], per_pixel, 1e-8));
    assert!(close(p.scale[1], per_pixel, 1e-8));
    assert_eq!(p.scale[2], p.scale[0]);
}

#[test]
fn larger_and_wider_screens() {
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    // 2560 × 1440: factor 4/3, the same 1920 × 1080 canvas.
    let p = place(&hud(1), &cam, 60.0, [2560, 1440], 1.0).unwrap();
    assert!(close(p.scale_factor, 4.0 / 3.0, 1e-6));
    assert!(close(p.size[0], 1920.0, 1e-3) && close(p.size[1], 1080.0, 1e-3));
    // 1920 × 1200: factor min(1, 1.11) = 1, a taller canvas.
    let p = place(&hud(1), &cam, 60.0, [1920, 1200], 1.0).unwrap();
    assert_eq!(p.scale_factor, 1.0);
    assert_eq!(p.size, [1920.0, 1200.0]);
    // 3440 × 1440: factor 1440 / 1080, 2580 wide.
    let p = place(&hud(1), &cam, 60.0, [3440, 1440], 1.0).unwrap();
    assert!(close(p.size[0], 2580.0, 1e-2) && close(p.size[1], 1080.0, 1e-3));
}

#[test]
fn ui_scale_unless_dont_scale() {
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    let p = place(&hud(1), &cam, 60.0, [1920, 1080], 1.25).unwrap();
    assert_eq!(p.scale_factor, 1.25);
    assert!(close(p.size[0], 1536.0, 1e-3) && close(p.size[1], 864.0, 1e-3));
    let p = place(&hud(SCALE_DONT), &cam, 60.0, [1920, 1080], 1.25).unwrap();
    assert_eq!(p.scale_factor, 1.0);
}

#[test]
fn the_canvas_fills_the_screen() {
    for (screen, fov) in [
        ([1920, 1080], 60.0),
        ([2560, 1440], 75.0),
        ([1280, 1024], 60.0),
    ] {
        // A turned camera somewhere else: the canvas still fills the view.
        let cam = UiCamera {
            position: [3.0, -2.0, 5.0],
            rotation: [0.0, 0.382_683_4, 0.0, 0.923_879_5],
            field_of_view: fov,
            near: 0.03,
            far: 1700.0,
        };
        let mut s = hud(1);
        s.distance = 2.5;
        let p = place(&s, &cam, fov, screen, 1.0).unwrap();
        let want = [
            [0.0, 0.0],
            [0.0, screen[1] as f32],
            [screen[0] as f32, screen[1] as f32],
            [screen[0] as f32, 0.0],
        ];
        for (c, w) in p.corners([0.5, 0.5]).iter().zip(want) {
            let px = cam.project(*c, screen).unwrap();
            assert!(
                close(px[0], w[0], 0.05) && close(px[1], w[1], 0.05),
                "{px:?} {w:?}"
            );
        }
    }
}

#[test]
fn a_different_ui_field_of_view_misses_the_corners() {
    // The frustum follows the main camera's field of view; a UI camera
    // with another one would not see the canvas fill the screen.
    let cam = UiCamera::game(70.0, 0.03, 1700.0);
    let p = place(&hud(1), &cam, 60.0, [1920, 1080], 1.0).unwrap();
    let px = cam.project(p.corners([0.5, 0.5])[2], [1920, 1080]).unwrap();
    assert!(px[0] < 1900.0, "{px:?}");
}

#[test]
fn anchored_modes_are_not_ported() {
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    for mode in [MODE_INVERSED, MODE_PARENTED, MODE_WORLD] {
        let mut s = hud(1);
        s.mode = mode;
        assert!(place(&s, &cam, 60.0, [1920, 1080], 1.0).is_err());
    }
    assert!(place(&hud(1), &cam, 60.0, [0, 1080], 1.0).is_err());
}

#[test]
fn the_canvas_rect_takes_the_size() {
    use crate::layout::{Node, RectTransform};
    let node = |parent: Option<usize>, rect: RectTransform| Node {
        name: String::new(),
        parent,
        children: Vec::new(),
        active_self: true,
        rect,
        components: Vec::new(),
    };
    let mut parent = node(None, RectTransform::default());
    parent.rect.size_delta = [100.0, 50.0];
    parent.children.push(1);
    // Anchors 0.2–0.6 across, equal vertically.
    let mut canvas = node(Some(0), RectTransform::default());
    canvas.rect.anchor_min = [0.2, 0.5];
    canvas.rect.anchor_max = [0.6, 0.5];
    let mut t = Tree::new(vec![parent, canvas]);
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    let p = place(&hud(1), &cam, 60.0, [1920, 1200], 1.0).unwrap();
    apply_to_tree(&mut t, 1, &p);
    // 1920 − 100 × 0.4 across; 1200 up.
    assert!(close(t.nodes[1].rect.size_delta[0], 1880.0, 1e-3));
    assert!(close(t.nodes[1].rect.size_delta[1], 1200.0, 1e-3));
    let r = t.rect(1);
    assert!(close(r.width, 1920.0, 1e-3) && close(r.height, 1200.0, 1e-3));
    assert_eq!(t.nodes[1].rect.scale, p.scale);
}

#[test]
fn a_screen_point_finds_its_place_on_a_tilted_canvas() {
    // The PDA's canvas: 0.19 m ahead, a little off the axis, turned 0.9°
    // about y and 10° about x, at 0.000212 m per unit.
    let cam = UiCamera::game(60.0, 0.03, 1700.0);
    let (h, v) = (0.45f32.to_radians(), 5f32.to_radians());
    let q = [
        v.sin() * h.cos(),
        h.sin() * v.cos(),
        -v.sin() * h.sin(),
        v.cos() * h.cos(),
    ];
    let p = Placement {
        position: [0.0217, -0.0012, 0.1874],
        rotation: q,
        size: [1195.0, 970.0],
        scale: [0.000212, 0.000212, 0.000212],
        scale_factor: 1.0,
    };
    let m = p.world_from_canvas();
    for point in [[0.0, 0.0], [-300.0, 420.0], [500.0, -400.0]] {
        let world = apply(&m, [point[0], point[1], 0.0]);
        let pixel = cam.project(world, [1920, 1080]).unwrap();
        let back = p
            .ray_point(cam.position, cam.screen_ray(pixel, [1920, 1080]))
            .unwrap();
        assert!(close(back[0], point[0], 0.05) && close(back[1], point[1], 0.05));
    }
    // Pixel (960, 540) is the camera's axis: dir (0, 0, 1).
    let d = cam.screen_ray([960.0, 540.0], [1920, 1080]);
    assert!(close(d[0], 0.0, 1e-6) && close(d[1], 0.0, 1e-6) && d[2] > 0.0);
    // Behind: a ray away from the canvas misses.
    assert_eq!(p.ray_point(cam.position, [0.0, 0.0, -1.0]), None);
}
