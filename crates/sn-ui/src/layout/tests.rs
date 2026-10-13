//! Layout tests on trees built here; expected values computed by hand
//! from the rules in `rebuild.rs`.

use super::*;

fn node(
    name: &str,
    parent: Option<usize>,
    rect: RectTransform,
    components: Vec<Component>,
) -> Node {
    Node {
        name: name.into(),
        parent,
        children: Vec::new(),
        active_self: true,
        rect,
        components: components
            .into_iter()
            .map(|kind| Comp {
                kind,
                enabled: true,
            })
            .collect(),
    }
}

/// Links children to parents in node order.
fn tree(mut nodes: Vec<Node>) -> Tree {
    for i in 0..nodes.len() {
        if let Some(p) = nodes[i].parent {
            nodes[p].children.push(i);
        }
    }
    Tree::new(nodes)
}

fn sized(w: f32, h: f32) -> RectTransform {
    RectTransform {
        size_delta: [w, h],
        ..RectTransform::default()
    }
}

fn element(pref: V2) -> Component {
    Component::LayoutElement {
        ignore: false,
        min: [-1.0, -1.0],
        preferred: pref,
        flexible: [-1.0, -1.0],
        priority: 1,
    }
}

fn hgroup() -> HvGroup {
    HvGroup {
        vertical: false,
        padding: Padding {
            left: 2,
            right: 3,
            top: 4,
            bottom: 6,
        },
        spacing: 5.0,
        alignment: 0,
        control: [true, true],
        force_expand: [false, false],
        use_scale: [false, false],
    }
}

/// A 100 × 50 horizontal group with children preferring 10, 20 and 30 px
/// wide and 20 px high.
fn row(g: HvGroup) -> Tree {
    tree(vec![
        node("group", None, sized(100.0, 50.0), vec![Component::Group(g)]),
        node("a", Some(0), sized(1.0, 1.0), vec![element([10.0, 20.0])]),
        node("b", Some(0), sized(1.0, 1.0), vec![element([20.0, 20.0])]),
        node("c", Some(0), sized(1.0, 1.0), vec![element([30.0, 20.0])]),
    ])
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn rects_follow_anchors_and_pivot() {
    let mut child = RectTransform {
        anchor_min: [0.0, 0.0],
        anchor_max: [1.0, 1.0],
        size_delta: [-20.0, -10.0],
        anchored_position: [3.0, 4.0],
        ..RectTransform::default()
    };
    let t = tree(vec![
        node("root", None, sized(200.0, 100.0), vec![]),
        node("child", Some(0), child, vec![]),
    ]);
    assert_eq!(
        t.rect(1),
        Rect {
            x: -90.0,
            y: -45.0,
            width: 180.0,
            height: 90.0
        }
    );
    // Root rect −100..100: anchor point at the parent's centre + (3, 4).
    assert_eq!(t.local_position(1), [3.0, 4.0]);
    child.anchor_max = [0.0, 0.0];
    child.pivot = [0.0, 1.0];
    let t = tree(vec![
        node("root", None, sized(200.0, 100.0), vec![]),
        node("child", Some(0), child, vec![]),
    ]);
    assert_eq!(t.local_position(1), [-97.0, -46.0]);
    assert_eq!(t.rect(1).y, 10.0);
}

#[test]
fn a_row_places_its_children_at_their_preferred_sizes() {
    let mut t = row(hgroup());
    let report = layout(&mut t);
    assert_eq!(report.roots, [0]);
    let sizes: Vec<f32> = (1..4).map(|i| t.nodes[i].rect.size_delta[0]).collect();
    assert_eq!(sizes, [10.0, 20.0, 30.0]);
    // Upper left: from the left padding, 5 apart; x is the pivot (0.5).
    let xs: Vec<f32> = (1..4)
        .map(|i| t.nodes[i].rect.anchored_position[0])
        .collect();
    assert_eq!(xs, [2.0 + 5.0, 17.0 + 10.0, 42.0 + 15.0]);
    for i in 1..4 {
        let r = &t.nodes[i].rect;
        assert_eq!((r.anchor_min, r.anchor_max), ([0.0, 1.0], [0.0, 1.0]));
        // The other axis: clamp(inner 40, 0, preferred 20) = 20, from the
        // top padding: y = −4 − 20 × 0.5.
        assert_eq!(r.size_delta[1], 20.0);
        assert_eq!(r.anchored_position[1], -14.0);
        assert_eq!(
            report.driven[i],
            Driven {
                anchors: true,
                position: [true, true],
                size: [true, true]
            }
        );
    }
    assert!(!report.driven[0].any());
}

#[test]
fn alignment_moves_the_surplus() {
    let mut g = hgroup();
    g.alignment = 4; // middle centre
    let mut t = row(g);
    layout(&mut t);
    // Surplus 100 − (60 + 10 + 5) = 25, half of it before the first.
    assert_eq!(t.nodes[1].rect.anchored_position[0], 2.0 + 12.5 + 5.0);
    // Vertically: inner 40, child 20: surplus 50 − 30 = 20, half: top 4 + 10.
    assert_eq!(t.nodes[1].rect.anchored_position[1], -14.0 - 10.0);
}

#[test]
fn force_expand_shares_the_surplus_and_shrinking_lerps() {
    let mut g = hgroup();
    g.force_expand = [true, false];
    let mut t = row(g);
    layout(&mut t);
    // Flexible 1 each: 25 / 3 more each.
    let w = t.nodes[1].rect.size_delta[0];
    assert!(close(w, 10.0 + 25.0 / 3.0), "{w}");
    // Too small: between min (0) and preferred by (40 − 15) / (75 − 15).
    let mut t = row(hgroup());
    t.nodes[0].rect.size_delta[0] = 40.0;
    layout(&mut t);
    let k = 25.0 / 60.0;
    assert!(close(t.nodes[3].rect.size_delta[0], 30.0 * k));
}

#[test]
fn uncontrolled_children_keep_their_size_and_scale_counts() {
    let mut g = hgroup();
    g.control = [false, false];
    g.alignment = 2; // upper right
    let mut t = row(g);
    layout(&mut t);
    // Size deltas (1 × 1) are the sizes: total 3 + 10 + 5 = 18.
    assert_eq!(t.nodes[1].rect.size_delta, [1.0, 1.0]);
    assert_eq!(t.nodes[1].rect.anchored_position[0], 2.0 + 82.0 + 0.5);
    let mut g = hgroup();
    g.use_scale = [true, false];
    let mut t = row(g);
    t.nodes[1].rect.scale = [2.0, 1.0, 1.0];
    layout(&mut t);
    // The first child counts 20 wide: the second starts at 2 + 20 + 5.
    assert_eq!(t.nodes[2].rect.anchored_position[0], 27.0 + 10.0);
}

#[test]
fn ignored_and_inactive_children_are_left_out() {
    let mut t = row(hgroup());
    // Its only ignorer ignores; disabled, it still counts (Unity's
    // `GetComponents` keeps disabled ones here).
    t.nodes[1].components = vec![Comp {
        kind: Component::LayoutElement {
            ignore: true,
            min: [-1.0; 2],
            preferred: [-1.0; 2],
            flexible: [-1.0; 2],
            priority: 1,
        },
        enabled: false,
    }];
    t.nodes[2].active_self = false;
    let report = layout(&mut t);
    // Only c, from the left padding.
    assert_eq!(t.nodes[3].rect.anchored_position[0], 2.0 + 15.0);
    assert!(!report.driven[1].any() && !report.driven[2].any());
}

#[test]
fn a_fitter_takes_its_group_s_preferred_size() {
    let mut g = hgroup();
    g.vertical = true;
    let mut t = row(g);
    t.nodes[0].components.push(Comp {
        kind: Component::SizeFitter { fit: [2, 2] },
        enabled: true,
    });
    let report = layout(&mut t);
    // Height: 20 × 3 + 5 × 2 + 4 + 6; width: the widest + 2 + 3.
    assert_eq!(t.nodes[0].rect.size_delta, [35.0, 80.0]);
    assert_eq!(report.driven[0].size, [true, true]);
    // Fitted to their sizes, the children have no surplus.
    assert_eq!(t.nodes[3].rect.anchored_position[1], -(4.0 + 50.0) - 10.0);
}

#[test]
fn priority_and_the_largest_value_decide() {
    let image = Component::Image {
        sprite: Some(SpriteSize {
            size: [64.0, 32.0],
            border: [4.0, 5.0, 6.0, 7.0],
            pixels_per_unit: 100.0,
        }),
        kind: 0,
    };
    let fitter = Component::SizeFitter { fit: [2, 2] };
    let mut t = tree(vec![
        node("root", None, sized(500.0, 500.0), vec![]),
        node(
            "img",
            Some(0),
            sized(1.0, 1.0),
            vec![image.clone(), fitter.clone()],
        ),
    ]);
    layout(&mut t);
    assert_eq!(t.nodes[1].rect.size_delta, [64.0, 32.0]);
    // Sliced: the borders (left + right, bottom + top); 200 ppu halves.
    let sliced = Component::Image {
        sprite: Some(SpriteSize {
            size: [64.0, 32.0],
            border: [4.0, 5.0, 6.0, 7.0],
            pixels_per_unit: 50.0,
        }),
        kind: 1,
    };
    t.nodes[1].components = vec![sliced, fitter.clone()]
        .into_iter()
        .map(|kind| Comp {
            kind,
            enabled: true,
        })
        .collect();
    layout(&mut t);
    assert_eq!(t.nodes[1].rect.size_delta, [20.0, 24.0]);
    // A layout element (priority 1) wins over the image, even if smaller;
    // its −1 height falls back to the image's.
    t.nodes[1].components.push(Comp {
        kind: element([8.0, -1.0]),
        enabled: true,
    });
    layout(&mut t);
    assert_eq!(t.nodes[1].rect.size_delta, [8.0, 24.0]);
    // Disabled, it counts for nothing.
    t.nodes[1].components[2].enabled = false;
    layout(&mut t);
    assert_eq!(t.nodes[1].rect.size_delta, [20.0, 24.0]);
}

#[test]
fn unknown_text_sizes_stay_unknown() {
    let mut t = tree(vec![
        node("root", None, sized(500.0, 500.0), vec![]),
        node(
            "label",
            Some(0),
            sized(1.0, 1.0),
            vec![Component::Text, Component::SizeFitter { fit: [0, 2] }],
        ),
        node(
            "fixed",
            Some(0),
            sized(1.0, 1.0),
            vec![
                Component::Text,
                element([30.0, 12.0]),
                Component::SizeFitter { fit: [2, 2] },
            ],
        ),
    ]);
    let report = layout(&mut t);
    assert!(t.nodes[1].rect.size_delta[1].is_nan());
    assert_eq!(t.nodes[1].rect.size_delta[0], 1.0);
    assert!(!report.driven[1].size[0] && report.driven[1].size[1]);
    // A layout element with a higher priority decides alone.
    assert_eq!(t.nodes[2].rect.size_delta, [30.0, 12.0]);
}

#[test]
fn roots_are_the_highest_group_ancestors() {
    let mut inner = hgroup();
    inner.vertical = true;
    let mut t = tree(vec![
        node("canvas", None, sized(500.0, 500.0), vec![]),
        node(
            "outer",
            Some(0),
            sized(100.0, 50.0),
            vec![Component::Group(hgroup())],
        ),
        node(
            "inner",
            Some(1),
            sized(1.0, 1.0),
            vec![Component::Group(inner)],
        ),
        node(
            "leaf",
            Some(2),
            sized(1.0, 1.0),
            vec![element([10.0, 10.0])],
        ),
        node(
            "fitted",
            Some(0),
            sized(1.0, 1.0),
            vec![Component::CircularBar { size: 52.0 }],
        ),
        node(
            "off",
            Some(0),
            sized(1.0, 1.0),
            vec![Component::Group(hgroup())],
        ),
    ]);
    t.nodes[5].components[0].enabled = false;
    let report = layout(&mut t);
    assert_eq!(report.roots, [1, 4]);
    // The inner group fits around its leaf: 10 + padding.
    assert_eq!(t.nodes[2].rect.size_delta, [15.0, 20.0]);
    assert_eq!(t.nodes[4].rect.size_delta, [52.0, 52.0]);
}

#[test]
fn aspect_fitters_drive_size_or_envelope() {
    let mut t = tree(vec![
        node("root", None, sized(160.0, 90.0), vec![]),
        node(
            "w",
            Some(0),
            sized(40.0, 1.0),
            vec![Component::AspectFitter {
                mode: 1,
                ratio: 2.0,
            }],
        ),
        node(
            "env",
            Some(0),
            sized(1.0, 1.0),
            vec![Component::AspectFitter {
                mode: 4,
                ratio: 1.6,
            }],
        ),
        node(
            "fit",
            Some(0),
            sized(1.0, 1.0),
            vec![Component::AspectFitter {
                mode: 3,
                ratio: 1.6,
            }],
        ),
    ]);
    layout(&mut t);
    assert_eq!(t.nodes[1].rect.size_delta, [40.0, 20.0]);
    // Envelope 160 × 90 at 1.6: 160 × 100, so 10 taller than the parent.
    assert!(close(t.nodes[2].rect.size_delta[1], 10.0));
    assert_eq!(t.nodes[2].rect.size_delta[0], 0.0);
    assert_eq!(t.nodes[2].rect.anchor_max, [1.0, 1.0]);
    // Fit in parent: 144 × 90, 16 narrower.
    assert!(close(t.nodes[3].rect.size_delta[0], -16.0));
}

#[test]
fn unported_components_are_counted_and_unknown() {
    let mut t = tree(vec![
        node("root", None, sized(500.0, 500.0), vec![]),
        node(
            "grid",
            Some(0),
            sized(1.0, 1.0),
            vec![
                Component::Unported {
                    name: "FlexibleGridLayout".into(),
                    element: true,
                    controller: true,
                    group: true,
                    ignorer: false,
                },
                Component::SizeFitter { fit: [2, 0] },
            ],
        ),
    ]);
    let report = layout(&mut t);
    assert!(t.nodes[1].rect.size_delta[0].is_nan());
    assert_eq!(report.unported.get("FlexibleGridLayout"), Some(&1));
}
