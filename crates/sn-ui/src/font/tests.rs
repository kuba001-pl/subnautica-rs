//! Font tests on a TrueType file written here (no fixture files): upem
//! 1000, glyphs `.notdef`, `H` (a 500 × 700 box), `x` (400 × 500), space,
//! `o` (a ring of quadratic curves) and `X`, a composite of two `x`: one
//! moved 246 units right with `ROUND_XY_TO_GRID`, one scaled by 0.5.

use super::*;

fn be16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_be_bytes());
}

fn be32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}

/// A simple glyph: contours of (x, y, on-curve) points, absolute.
fn glyph(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
    let pts: Vec<(i16, i16, bool)> = contours.iter().flat_map(|c| c.iter().copied()).collect();
    let mut b = Vec::new();
    be16(&mut b, contours.len() as u16);
    let xs = pts.iter().map(|p| p.0);
    let ys = pts.iter().map(|p| p.1);
    for v in [
        xs.clone().min().unwrap_or(0),
        ys.clone().min().unwrap_or(0),
        xs.max().unwrap_or(0),
        ys.max().unwrap_or(0),
    ] {
        be16(&mut b, v as u16);
    }
    let mut end = 0u16;
    for c in contours {
        end += c.len() as u16;
        be16(&mut b, end - 1);
    }
    be16(&mut b, 0); // no instructions
    for p in &pts {
        b.push(u8::from(p.2)); // 16-bit deltas for x and y
    }
    let (mut px, mut py) = (0i16, 0i16);
    for p in &pts {
        be16(&mut b, (p.0 - px) as u16);
        px = p.0;
    }
    for p in &pts {
        be16(&mut b, (p.1 - py) as u16);
        py = p.1;
    }
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

/// A composite glyph: `x` at (246, 0) rounded to the grid, and `x`
/// scaled by 0.5 at the origin.
fn composite_glyph() -> Vec<u8> {
    let mut b = Vec::new();
    be16(&mut b, (-1i16) as u16);
    for v in [25i16, 0, 696, 500] {
        be16(&mut b, v as u16);
    }
    // words | xy values | round to grid | more components
    be16(&mut b, 0x0027);
    be16(&mut b, 2);
    be16(&mut b, 246);
    be16(&mut b, 0);
    // words | xy values | scale
    be16(&mut b, 0x000B);
    be16(&mut b, 2);
    be16(&mut b, 0);
    be16(&mut b, 0);
    be16(&mut b, 0x2000);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

fn name_table(names: &[(u16, &str)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut b = Vec::new();
    be16(&mut b, 0);
    be16(&mut b, names.len() as u16);
    be16(&mut b, (6 + 12 * names.len()) as u16);
    for (id, s) in names {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, 0x409, *id, utf16.len() as u16, strings.len() as u16] {
            be16(&mut b, v);
        }
        strings.extend(utf16);
    }
    b.extend(strings);
    b
}

/// The test font's bytes.
pub(crate) fn test_font() -> Vec<u8> {
    let ring: [&[(i16, i16, bool)]; 2] = [
        &[
            (250, 0, true),
            (450, 0, false),
            (450, 250, true),
            (450, 500, false),
            (250, 500, true),
            (50, 500, false),
            (50, 250, true),
            (50, 0, false),
        ],
        &[
            (250, 100, true),
            (150, 100, false),
            (150, 250, true),
            (150, 400, false),
            (250, 400, true),
            (350, 400, false),
            (350, 250, true),
            (350, 100, false),
        ],
    ];
    let glyphs = [
        Vec::new(),
        glyph(&[&[
            (100, 0, true),
            (100, 700, true),
            (600, 700, true),
            (600, 0, true),
        ]]),
        glyph(&[&[
            (50, 0, true),
            (50, 500, true),
            (450, 500, true),
            (450, 0, true),
        ]]),
        Vec::new(),
        glyph(&ring),
        composite_glyph(),
    ];
    let advances: [u16; 6] = [500, 700, 500, 250, 500, 750];
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        be16(&mut loca, (glyf.len() / 2) as u16);
        glyf.extend_from_slice(g);
    }
    be16(&mut loca, (glyf.len() / 2) as u16);

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, 1000);
    head.extend_from_slice(&[0; 16]);
    for v in [0i16, 0, 600, 700] {
        be16(&mut head, v as u16);
    }
    be16(&mut head, 0);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 0); // short loca
    be16(&mut head, 0);

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for v in [800i16, -200, 90, 700, 0, 0, 600, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut hhea, v as u16);
    }
    be16(&mut hhea, advances.len() as u16);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, glyphs.len() as u16);

    let mut hmtx = Vec::new();
    for a in advances {
        be16(&mut hmtx, a);
        be16(&mut hmtx, 0);
    }

    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 10);
    be32(&mut cmap, 12);
    let groups: [(u32, u32); 5] = [
        (' ' as u32, 3),
        ('H' as u32, 1),
        ('X' as u32, 5),
        ('o' as u32, 4),
        ('x' as u32, 2),
    ];
    be16(&mut cmap, 12);
    be16(&mut cmap, 0);
    be32(&mut cmap, 16 + 12 * groups.len() as u32);
    be32(&mut cmap, 0);
    be32(&mut cmap, groups.len() as u32);
    for (c, g) in groups {
        be32(&mut cmap, c);
        be32(&mut cmap, c);
        be32(&mut cmap, g);
    }

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, (-100i16) as u16);
    be16(&mut post, 50);
    post.extend_from_slice(&[0; 20]);

    let name = name_table(&[(1, "Test Sans"), (2, "Regular")]);

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    tables.sort_by_key(|t| *t.0);
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as u16);
    out.extend_from_slice(&[0; 6]);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, t) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset as u32);
        be32(&mut out, t.len() as u32);
        body.extend_from_slice(t);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() + body.len();
    }
    out.extend(body);
    out
}

#[test]
fn face_info_follows_the_rule() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let i = f.face_info(90);
    assert_eq!(i.family_name, "Test Sans");
    assert_eq!(i.style_name, "Regular");
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(close(i.ascent_line, 72.0), "{}", i.ascent_line);
    assert!(close(i.descent_line, -18.0));
    assert!(close(i.line_height, 98.1));
    // post: −100 − 50/2.
    assert!(close(i.underline_offset, -11.25));
    assert!(close(i.underline_thickness, 4.5));
    assert!(close(i.cap_line, 63.0));
    assert!(close(i.mean_line, 45.0));
    assert!(close(i.strikethrough_offset, 18.0));
    // 250 × 0.09 = 22.5, rounded away from zero.
    assert_eq!(i.tab_width, 23.0);
    assert_eq!(
        (i.superscript_offset, i.subscript_offset),
        (i.ascent_line, i.descent_line)
    );
}

#[test]
fn glyph_metrics_are_fixed_point() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let h = f.glyph(f.glyph_index('H').unwrap(), 90).unwrap();
    assert_eq!(h.metrics.bearing_x, 9.0);
    assert_eq!(h.metrics.width, 45.0);
    assert_eq!(h.metrics.height, 63.0);
    assert_eq!(h.metrics.bearing_y, 63.0);
    // 700 × 377487 / 65536 = 4031.98 → 4032 / 64.
    assert_eq!(h.metrics.advance, 63.0);
    assert_eq!((h.left, h.bottom, h.width, h.height), (9, 0, 45, 63));
    // At 45 px the box is fractional: floor and ceil.
    let x = f.glyph(f.glyph_index('x').unwrap(), 45).unwrap();
    assert_eq!(x.metrics.bearing_x * 64.0, 144.0);
    assert_eq!((x.left, x.width), (2, 19));
    let space = f.glyph(f.glyph_index(' ').unwrap(), 90).unwrap();
    assert_eq!((space.width, space.height), (0, 0));
    assert!(space.contours.is_empty());
    assert!(f.glyph(99, 90).is_err());
    // An absurd size is refused, not allocated.
    assert!(matches!(f.glyph(1, 1_000_000), Err(FontError::TooLarge(1))));
    assert!(f.glyph(1, i32::MAX).is_err());
    assert!(f.glyph(1 << 20, 90).is_err());
    assert_eq!(f.glyph_index('Q'), None);
}

#[test]
fn composite_offsets_are_scaled_then_rounded_to_the_grid() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    let parts = composite::components(&face, 5).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].glyph, parts[0].args), (2, [246, 0]));
    assert!(!parts[0].has_transform());
    assert_eq!(parts[1].matrix, [0x8000, 0, 0, 0x8000]);
    assert!(composite::components(&face, 1).is_none());
    assert!(composite::components(&face, 0).is_none());
    let g = f.glyph(f.glyph_index('X').unwrap(), 90).unwrap();
    // Left: x's 50 units scaled (288/64 px), then halved: 144/64.
    assert_eq!(g.metrics.bearing_x * 64.0, 144.0);
    // Right: x's 450 units (2592/64) + 246 units (1417/64) rounded to
    // the pixel, 1408/64: 4000/64. Unrounded it would be 4009/64.
    assert_eq!((g.metrics.bearing_x + g.metrics.width) * 64.0, 4000.0);
    assert_eq!(g.metrics.height * 64.0, 2880.0);
    assert_eq!(g.contours.len(), 2);
}

#[test]
fn curves_are_flattened_onto_the_outline() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let o = f.glyph(f.glyph_index('o').unwrap(), 90).unwrap();
    assert_eq!(o.contours.len(), 2);
    // Every flattened point of the outer contour lies within the
    // tolerance of the quadratic ellipse it approximates (centre 22.5,
    // radii 18 × 22.5 px); the curve bulges no further than its control box.
    for p in &o.contours[0] {
        assert!(p[0] >= 4.49 && p[0] <= 40.51 && p[1] >= -0.01 && p[1] <= 45.01);
    }
    assert!(o.contours[0].len() > 8);
    // The hole is empty and the ring is not.
    let field = f.render(&o, 9);
    let at = |x: f64, y: f64| {
        let i = (x - f64::from(o.left) + 9.0) as usize;
        let j = (y - f64::from(o.bottom) + 9.0) as usize;
        field.texels[j * field.width + i]
    };
    assert!(at(22.5, 22.5) < 128, "{}", at(22.5, 22.5));
    assert!(at(8.0, 22.5) > 128, "{}", at(8.0, 22.5));
    assert!(at(-4.0, 22.5) < 128);
}

#[test]
fn a_box_glyph_field_rises_by_the_gradient() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let h = f.glyph(f.glyph_index('H').unwrap(), 90).unwrap();
    let field = f.render(&h, 9);
    assert_eq!((field.width, field.height), (63, 81));
    // Row through the middle: from the field's first texel (centre 0.5,
    // 8.5 px outside the box's edge at 9) into the box, 12.75 per texel.
    let row = 40 * field.width;
    assert_eq!(field.texels[row], sdf::encode(8.5, 10.0));
    assert_eq!(field.texels[row + 8], sdf::encode(0.5, 10.0));
    assert_eq!(field.texels[row + 9], sdf::encode(-0.5, 10.0));
    assert_eq!(field.texels[row + 20], sdf::encode(-11.5, 10.0));
    // Coverage of the same box: whole pixels.
    let c = raster::rasterize(&h.contours, 63, 70);
    assert_eq!(c.get(9, 10), 1.0);
    assert_eq!(c.get(8, 10), 0.0);
}

#[test]
fn the_atlas_adds_caches_and_runs_out() {
    let bytes = test_font();
    let f = FontFile::parse(&bytes).unwrap();
    let mut a = DynamicAtlas::new(256, 256, 9, 90);
    let Added::New(h) = a.add_character(&f, 'H') else {
        panic!("H not added");
    };
    assert_eq!(h.rect, GlyphRect::new(10, 10, 45, 63));
    assert_eq!(a.packer.used, [GlyphRect::new(0, 0, 64, 82)]);
    // The field sits one texel in; the box's inside is above the edge.
    assert_eq!(a.texels[0], 0);
    assert!(a.texels[40 * 256 + 30] > 200);
    assert_eq!(a.add_character(&f, 'H'), Added::Cached(h));
    assert_eq!(a.add_character(&f, 'Q'), Added::Missing);
    let Added::New(space) = a.add_character(&f, ' ') else {
        panic!("space not added");
    };
    assert_eq!(space.rect, GlyphRect::default());
    assert_eq!(space.metrics.advance, 22.5);
    assert_eq!(a.packer.used.len(), 1);
    let mut small = DynamicAtlas::new(40, 40, 9, 90);
    assert_eq!(small.add_character(&f, 'H'), Added::Full);
    // Bad input does not panic.
    assert!(FontFile::parse(&bytes[..100]).is_err());
    assert!(FontFile::parse(&[]).is_err());
}
