//! Mesh tests: expected vertices worked out by hand from the rules in
//! `mesh.rs`.

use super::*;

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// A 128 × 128 sprite trimmed to 90 × 100 at (10, 20) of a 256² page,
/// 5 px from its left and 8 from its bottom.
fn trimmed() -> SpriteInfo {
    SpriteInfo::new(
        [128.0, 128.0],
        [0.0; 4],
        100.0,
        [10.0, 20.0, 90.0, 100.0],
        [5.0, 8.0],
        [256.0, 256.0],
        true,
        false,
    )
}

/// A 30 × 30 sprite, borders 10, alone in a 30² texture.
fn bordered() -> SpriteInfo {
    SpriteInfo::new(
        [30.0, 30.0],
        [10.0; 4],
        100.0,
        [0.0, 0.0, 30.0, 30.0],
        [0.0, 0.0],
        [30.0, 30.0],
        false,
        false,
    )
}

fn image(sprite: Option<SpriteInfo>, kind: i32) -> ImageParams {
    ImageParams {
        sprite,
        kind,
        preserve_aspect: false,
        fill_center: true,
        fill_method: 4,
        fill_amount: 1.0,
        fill_clockwise: true,
        fill_origin: 0,
        use_sprite_mesh: false,
        pixels_per_unit_multiplier: 1.0,
        color: WHITE,
    }
}

fn positions(m: &MeshData) -> Vec<V2> {
    m.vertices.iter().map(|v| v.pos).collect()
}

/// The area the triangles cover.
fn area(m: &MeshData) -> f32 {
    m.indices
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| m.vertices[t[k] as usize].pos);
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
        })
        .sum()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn without_a_sprite_it_is_the_rect() {
    let (kind, m) = image_mesh(
        rect(-50.0, -25.0, 100.0, 50.0),
        [0.5, 0.5],
        &image(None, 0),
        100.0,
    );
    assert_eq!(kind, MeshKind::Plain);
    assert_eq!(
        positions(&m),
        [[-50.0, -25.0], [-50.0, 25.0], [50.0, 25.0], [50.0, -25.0]]
    );
    assert_eq!(m.vertices[2].uv, [1.0, 1.0]);
    assert_eq!(m.indices, [0, 1, 2, 2, 3, 0]);
    assert_eq!(m.vertices[0].color, [255; 4]);
}

#[test]
fn a_trimmed_sprite_keeps_its_margin() {
    let s = trimmed();
    assert_eq!(s.padding, [5.0, 8.0, 33.0, 20.0]);
    let (_, m) = image_mesh(
        rect(-32.0, -32.0, 64.0, 64.0),
        [0.5, 0.5],
        &image(Some(s), 0),
        100.0,
    );
    // 64 / 128 = half a pixel per sprite pixel.
    assert_eq!(
        positions(&m),
        [[-29.5, -28.0], [-29.5, 22.0], [15.5, 22.0], [15.5, -28.0]]
    );
    assert_eq!(m.vertices[0].uv, [10.0 / 256.0, 20.0 / 256.0]);
    assert_eq!(m.vertices[2].uv, [100.0 / 256.0, 120.0 / 256.0]);
}

#[test]
fn preserve_aspect_shrinks_around_the_pivot() {
    let s = SpriteInfo::new(
        [200.0, 100.0],
        [0.0; 4],
        100.0,
        [0.0, 0.0, 200.0, 100.0],
        [0.0, 0.0],
        [200.0, 100.0],
        false,
        false,
    );
    let mut img = image(Some(s), 0);
    img.preserve_aspect = true;
    let (_, m) = image_mesh(rect(-50.0, -50.0, 100.0, 100.0), [0.5, 0.5], &img, 100.0);
    assert_eq!(m.vertices[0].pos, [-50.0, -25.0]);
    assert_eq!(m.vertices[2].pos, [50.0, 25.0]);
    // Pivot at the bottom: the shrunk rect stays at the bottom.
    let (_, m) = image_mesh(rect(-50.0, 0.0, 100.0, 100.0), [0.5, 0.0], &img, 100.0);
    assert_eq!(m.vertices[0].pos, [-50.0, 0.0]);
}

#[test]
fn sliced_has_nine_parts_and_squeezes_borders() {
    let (kind, m) = image_mesh(
        rect(0.0, 0.0, 100.0, 50.0),
        [0.0, 0.0],
        &image(Some(bordered()), 1),
        100.0,
    );
    assert_eq!(kind, MeshKind::Sliced);
    assert_eq!(m.vertices.len(), 36);
    let xs: Vec<f32> = m.vertices.iter().map(|v| v.pos[0]).collect();
    for x in [0.0, 10.0, 90.0, 100.0] {
        assert!(xs.contains(&x));
    }
    // The centre part: inner UVs 1/3..2/3.
    let centre = &m.vertices[16..20];
    assert_eq!(centre[0].pos, [10.0, 10.0]);
    assert_eq!(centre[2].pos, [90.0, 40.0]);
    assert!(close(centre[0].uv[0], 1.0 / 3.0) && close(centre[2].uv[1], 2.0 / 3.0));
    let mut img = image(Some(bordered()), 1);
    img.fill_center = false;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 50.0), [0.0, 0.0], &img, 100.0);
    assert_eq!(m.vertices.len(), 32);
    // 15 px wide with 10 + 10 borders: 7.5 each.
    let (_, m) = image_mesh(
        rect(0.0, 0.0, 15.0, 50.0),
        [0.0, 0.0],
        &image(Some(bordered()), 1),
        100.0,
    );
    assert!(m.vertices.iter().any(|v| v.pos[0] == 7.5));
    // Pixels per unit 200: borders half as wide.
    let mut img = image(Some(bordered()), 1);
    img.pixels_per_unit_multiplier = 2.0;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 50.0), [0.0, 0.0], &img, 100.0);
    assert!(m.vertices.iter().any(|v| v.pos[0] == 5.0));
    // No borders: drawn simple.
    let (kind, _) = image_mesh(
        rect(0.0, 0.0, 10.0, 10.0),
        [0.0, 0.0],
        &image(Some(trimmed()), 1),
        100.0,
    );
    assert_eq!(kind, MeshKind::SlicedAsSimple);
}

#[test]
fn tiles_are_quads_or_a_repeating_uv() {
    let tile = SpriteInfo::new(
        [20.0, 20.0],
        [0.0; 4],
        100.0,
        [0.0, 0.0, 20.0, 20.0],
        [0.0, 0.0],
        [20.0, 20.0],
        false,
        true,
    );
    let (kind, m) = image_mesh(
        rect(0.0, 0.0, 50.0, 30.0),
        [0.0, 0.0],
        &image(Some(tile), 2),
        100.0,
    );
    assert_eq!(kind, MeshKind::TiledRepeat);
    assert_eq!(m.vertices[2].uv, [2.5, 1.5]);
    // Packed: 3 × 2 tiles, the last column and row cut.
    let mut packed = tile;
    packed.packed = true;
    let (kind, m) = image_mesh(
        rect(0.0, 0.0, 50.0, 30.0),
        [0.0, 0.0],
        &image(Some(packed), 2),
        100.0,
    );
    assert_eq!(kind, MeshKind::TiledQuads);
    assert_eq!(m.vertices.len(), 24);
    let last = &m.vertices[20..24];
    assert_eq!(last[2].pos, [50.0, 30.0]);
    assert_eq!(last[2].uv, [0.5, 0.5]);
    assert!(close(area(&m), 1500.0));
    // Borders: 2 × 1 tiles of 10 px (30 − 20), 2 side quads per row, 2
    // per column, 4 corners.
    let (kind, m) = image_mesh(
        rect(0.0, 0.0, 40.0, 30.0),
        [0.0, 0.0],
        &image(Some(bordered()), 2),
        100.0,
    );
    assert_eq!(kind, MeshKind::TiledQuads);
    let (columns, rows) = (2, 1);
    assert_eq!(
        m.vertices.len(),
        4 * (columns * rows + 2 * rows + 2 * columns + 4)
    );
    assert!(close(area(&m), 1200.0));
}

#[test]
fn horizontal_and_vertical_fills_cut_the_quad() {
    let s = bordered();
    let mut img = image(Some(s), 3);
    img.fill_method = 0;
    img.fill_amount = 0.25;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 50.0), [0.0, 0.0], &img, 100.0);
    assert_eq!(m.vertices[2].pos, [25.0, 50.0]);
    assert!(close(m.vertices[2].uv[0], 0.25));
    img.fill_origin = 1;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 50.0), [0.0, 0.0], &img, 100.0);
    assert_eq!(m.vertices[0].pos, [75.0, 0.0]);
    img.fill_method = 1;
    img.fill_origin = 0;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 40.0), [0.0, 0.0], &img, 100.0);
    assert_eq!(m.vertices[1].pos, [0.0, 10.0]);
    img.fill_amount = 0.0005;
    let (_, m) = image_mesh(rect(0.0, 0.0, 100.0, 40.0), [0.0, 0.0], &img, 100.0);
    assert!(m.vertices.is_empty());
}

#[test]
fn radial_fills_cover_their_share_at_the_quarters() {
    for (method, quads) in [(2, 1), (3, 2), (4, 4)] {
        for origin in 0..4 {
            for clockwise in [true, false] {
                for (fill, share) in [(0.5, 0.5), (1.0, 1.0)] {
                    let mut img = image(Some(bordered()), 3);
                    img.fill_method = method;
                    img.fill_origin = origin;
                    img.fill_clockwise = clockwise;
                    img.fill_amount = fill;
                    let (_, m) = image_mesh(rect(0.0, 0.0, 40.0, 40.0), [0.0, 0.0], &img, 100.0);
                    let a = area(&m);
                    assert!(
                        close(a, 1600.0 * share),
                        "method {method} origin {origin} cw {clockwise} fill {fill}: {a}"
                    );
                    assert!(m.vertices.len() <= quads * 4);
                    for v in &m.vertices {
                        assert!(
                            (0.0..=40.0).contains(&v.pos[0]) && (0.0..=40.0).contains(&v.pos[1])
                        );
                    }
                }
            }
        }
    }
    // Radial 360 at a quarter and three quarters.
    for (fill, share) in [(0.25, 0.25), (0.75, 0.75)] {
        let mut img = image(Some(bordered()), 3);
        img.fill_amount = fill;
        let (_, m) = image_mesh(rect(0.0, 0.0, 40.0, 40.0), [0.0, 0.0], &img, 100.0);
        assert!(close(area(&m), 1600.0 * share), "{fill}: {}", area(&m));
    }
}

#[test]
fn raw_images_and_colours() {
    let m = raw_image_mesh(
        rect(0.0, 0.0, 10.0, 10.0),
        [1.0, 0.5, 0.0, 2.0],
        [0.25, 0.5, 0.5, 0.25],
        true,
    );
    assert_eq!(m.vertices[0].uv, [0.25, 0.5]);
    assert_eq!(m.vertices[2].uv, [0.75, 0.75]);
    assert_eq!(m.vertices[0].color, [255, 128, 0, 255]);
    assert!(
        raw_image_mesh(rect(0.0, 0.0, 1.0, 1.0), WHITE, [0.0, 0.0, 1.0, 1.0], false)
            .vertices
            .is_empty()
    );
    assert_eq!(color32([-1.0, 0.2, 0.998, 1.0]), [0, 51, 254, 255]);
}
