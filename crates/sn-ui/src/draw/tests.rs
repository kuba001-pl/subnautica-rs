//! Draw-list tests on trees built here.

use super::*;
use crate::layout::{Node, RectTransform};

fn node(name: &str, parent: Option<usize>, size: V2, pos: V2) -> Node {
    Node {
        name: name.into(),
        parent,
        children: Vec::new(),
        active_self: true,
        rect: RectTransform {
            size_delta: size,
            anchored_position: pos,
            ..RectTransform::default()
        },
        components: Vec::new(),
    }
}

fn tree(mut nodes: Vec<Node>) -> Tree {
    for i in 0..nodes.len() {
        if let Some(p) = nodes[i].parent {
            nodes[p].children.push(i);
        }
    }
    Tree::new(nodes)
}

/// A quad over the node's rect.
fn quad(t: &Tree, i: usize, texture: u64) -> Graphic {
    let r = t.rect(i);
    let (kind, mesh) = crate::mesh::image_mesh(
        r,
        t.nodes[i].rect.pivot,
        &crate::mesh::ImageParams {
            sprite: None,
            kind: 0,
            preserve_aspect: false,
            fill_center: true,
            fill_method: 0,
            fill_amount: 1.0,
            fill_clockwise: true,
            fill_origin: 0,
            use_sprite_mesh: false,
            pixels_per_unit_multiplier: 1.0,
            color: [1.0; 4],
        },
        100.0,
    );
    Graphic {
        node: i,
        mesh,
        texture,
        maskable: true,
        cull_transparent: false,
        kind: format!("{kind:?}"),
    }
}

fn canvas(override_sorting: bool, order: i32) -> CanvasInfo {
    CanvasInfo {
        enabled: true,
        override_sorting,
        sorting_layer: 0,
        sorting_order: order,
    }
}

/// canvas (400 × 300) ─ a (100², at (-50, 0)) ─ b (20², at (5, 5))
///                     └ c (60 × 40, at (100, 50))
fn scene() -> Tree {
    tree(vec![
        node("canvas", None, [400.0, 300.0], [0.0, 0.0]),
        node("a", Some(0), [100.0, 100.0], [-50.0, 0.0]),
        node("b", Some(1), [20.0, 20.0], [5.0, 5.0]),
        node("c", Some(0), [60.0, 40.0], [100.0, 50.0]),
    ])
}

fn input(t: &Tree) -> DrawInput {
    DrawInput {
        canvases: [(0, canvas(false, 0))].into_iter().collect(),
        graphics: (1..4).rev().map(|i| quad(t, i, i as u64)).collect(),
        ..DrawInput::default()
    }
}

#[test]
fn hierarchy_order_and_canvas_space() {
    let t = scene();
    let d = draw_list(&t, &input(&t));
    let nodes: Vec<usize> = d.items.iter().map(|i| i.node).collect();
    assert_eq!(nodes, [1, 2, 3]);
    // b: its rect −10..10 around (−50 + 5, 5).
    let b = &d.items[1];
    assert_eq!(b.vertices[0].pos, [-55.0, -5.0]);
    assert_eq!(b.vertices[2].pos, [-35.0, 15.0]);
    assert!(b.clip.is_none());
}

#[test]
fn transforms_rotate_and_scale() {
    let mut t = scene();
    // a rotated 90° about z and scaled ×2: b's offset (5, 5) becomes
    // (−10, 10).
    t.nodes[1].rect.rotation = [
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    t.nodes[1].rect.scale = [2.0, 2.0, 1.0];
    let d = draw_list(&t, &input(&t));
    let b = &d.items[1];
    // b's corner (−10, −10) → ×2, rotated: (20, −20); then + (5,5)×2
    // rotated (−10, 10); then + a's place (−50, 0).
    let p = b.vertices[0].pos;
    assert!(
        (p[0] - (-50.0 - 10.0 + 20.0)).abs() < 1e-3 && (p[1] - (10.0 - 20.0)).abs() < 1e-3,
        "{p:?}"
    );
}

#[test]
fn override_sorting_draws_by_its_order() {
    let t = scene();
    let mut inp = input(&t);
    // a has its own canvas, sorted before the root (order −1).
    inp.canvases.insert(1, canvas(true, -1));
    let d = draw_list(&t, &inp);
    let nodes: Vec<usize> = d.items.iter().map(|i| i.node).collect();
    assert_eq!(nodes, [1, 2, 3]);
    assert_eq!(d.items[0].canvas, 1);
    inp.canvases.insert(1, canvas(true, 5));
    let d = draw_list(&t, &inp);
    let nodes: Vec<usize> = d.items.iter().map(|i| i.node).collect();
    assert_eq!(nodes, [3, 1, 2]);
    // Without override sorting the nested canvas keeps its place.
    inp.canvases.insert(1, canvas(false, 5));
    let d = draw_list(&t, &inp);
    assert_eq!(d.items[0].node, 1);
    assert_eq!(d.items[0].canvas, 0);
}

#[test]
fn canvas_groups_multiply_alpha() {
    let t = scene();
    let mut inp = input(&t);
    let group = |alpha, ignore| GroupInfo {
        alpha,
        ignore_parent_groups: ignore,
        enabled: true,
    };
    inp.groups.insert(0, group(0.5, false));
    inp.groups.insert(1, group(0.5, false));
    let d = draw_list(&t, &inp);
    assert_eq!(d.items[1].alpha, 0.25);
    assert_eq!(d.items[1].vertices[0].color[3], 64);
    assert_eq!(d.items[2].alpha, 0.5);
    inp.groups.insert(1, group(0.5, true));
    let d = draw_list(&t, &inp);
    assert_eq!(d.items[1].alpha, 0.5);
    // A transparent graphic with cullTransparentMesh is not drawn.
    inp.groups.insert(1, group(0.0, false));
    inp.graphics
        .iter_mut()
        .for_each(|g| g.cull_transparent = true);
    let d = draw_list(&t, &inp);
    assert_eq!(d.transparent, 2);
    assert_eq!(d.items.len(), 1);
}

#[test]
fn rect_masks_clip_and_cull() {
    let mut t = scene();
    let mut inp = input(&t);
    let mask = |padding| MaskInfo {
        enabled: true,
        padding,
        softness: [0, 0],
    };
    // a masks its children: a's rect −100..0 × −50..50, inset 10 left.
    inp.masks.insert(1, mask([10.0, 0.0, 0.0, 0.0]));
    let d = draw_list(&t, &inp);
    let clip = d.items[1].clip.unwrap();
    assert_eq!(
        (clip.x, clip.y, clip.width, clip.height),
        (-90.0, -50.0, 90.0, 100.0)
    );
    // c is not under a: not clipped.
    assert!(d.items[2].clip.is_none());
    // b moved out of a's rect: culled.
    t.nodes[2].rect.anchored_position = [500.0, 0.0];
    let inp2 = DrawInput {
        graphics: (1..4).rev().map(|i| quad(&t, i, i as u64)).collect(),
        ..inp
    };
    let d = draw_list(&t, &inp2);
    assert_eq!(d.culled, 1);
    assert!(d.items.iter().all(|i| i.node != 2));
    // Not maskable: drawn, unclipped.
    let mut inp3 = inp2;
    inp3.graphics.iter_mut().for_each(|g| g.maskable = false);
    let d = draw_list(&t, &inp3);
    assert_eq!(d.culled, 0);
    // Two masks intersect; an override canvas between stops the outer.
    let t = scene();
    let mut inp = input(&t);
    inp.masks.insert(0, mask([0.0, 0.0, 0.0, 0.0]));
    // a's rect inset 60 from the bottom: y 10..50 (b, y −5..15, overlaps).
    inp.masks.insert(1, mask([0.0, 60.0, 0.0, 0.0]));
    let d = draw_list(&t, &inp);
    assert_eq!(d.items[1].node, 2);
    let c = d.items[1].clip.unwrap();
    assert_eq!((c.x, c.y, c.width, c.height), (-100.0, 10.0, 100.0, 40.0));
    inp.canvases.insert(1, canvas(true, 0));
    inp.masks.remove(&1);
    let d = draw_list(&t, &inp);
    assert!(d.items.iter().find(|i| i.node == 2).unwrap().clip.is_none());
}

#[test]
fn the_hash_is_stable_and_sensitive() {
    let t = scene();
    let a = draw_list(&t, &input(&t)).hash();
    let b = draw_list(&t, &input(&t)).hash();
    assert_eq!(a, b);
    let mut t2 = scene();
    t2.nodes[3].rect.anchored_position[0] += 0.5;
    assert_ne!(a, draw_list(&t2, &input(&t2)).hash());
}
