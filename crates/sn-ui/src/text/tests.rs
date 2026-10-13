//! Text layout tests on the font written in `font::tests` (upem 1000,
//! ascender 800, descender −200, line gap 90; `H` 700 wide, `x` 500,
//! space 250), sampled at 90 px. At point size 9 the scale is 0.1: `H`
//! advances 6.3, `x` 4.5, a space 2.25; the ascent line is 7.2, the
//! descent −1.8, a line 9.81 (with the gap's 0.81).

use std::collections::HashMap;

use super::*;
use crate::font::FontFile;
use crate::layout::Rect;

fn set(bytes: &[u8]) -> FontSet<'_> {
    let file = FontFile::parse(bytes).unwrap();
    let face = file.face_info(90);
    FontSet::new(
        vec![TmpFont {
            name: "Test".into(),
            face,
            file,
            sampling_point_size: 90,
            bold_spacing: 7.0,
            bold_style: 0.75,
            normal_style: 0.0,
            normal_spacing_offset: 0.0,
            italic_style: 35.0,
            tab_size: 10.0,
            fallbacks: Vec::new(),
            weights: [[None; 2]; 10],
            kerning: HashMap::new(),
        }],
        Vec::new(),
        None,
        63,
    )
}

fn rect(w: f32, h: f32) -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn visible(l: &TextLayout) -> Vec<&CharInfo> {
    l.characters.iter().filter(|c| c.visible).collect()
}

#[test]
fn one_line_at_the_top_left() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let l = layout_text(
        &fonts,
        &TextParams::new("Hx", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    assert_eq!(l.lines.len(), 1);
    let v = visible(&l);
    assert_eq!(v.len(), 2);
    // Top: the baseline is the ascent line (7.2) below the top.
    assert!(close(v[0].baseline, 100.0 - 7.2), "{}", v[0].baseline);
    // H: bearing 9 px at 90 → 0.9; its top at bearing y 63 → 6.3.
    assert!(close(v[0].bottom_left[0], 0.9));
    assert!(close(v[0].top_left[1], 92.8 + 6.3));
    assert!(close(v[1].origin, 6.3));
    assert!(close(l.lines[0].max_advance, 10.8));
    assert_eq!(l.font_size, 9.0);
}

#[test]
fn alignment_moves_lines() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let mut p = TextParams::new("Hx", 0, 9.0);
    p.h_align = 2;
    p.v_align = 1024;
    let l = layout_text(&fonts, &p, rect(200.0, 100.0), 0.0);
    // Centre: the line's 10.8 centred in 200.
    assert!(close(l.characters[0].origin, 100.0 - 5.4));
    // Bottom: the descent line (−1.8) on the bottom.
    assert!(close(l.characters[0].baseline, 1.8));
    p.h_align = 4;
    p.v_align = 512;
    let l = layout_text(&fonts, &p, rect(200.0, 100.0), 0.0);
    assert!(close(l.characters[1].x_advance, 200.0));
    // Middle: halfway between ascender 7.2 and descender −1.8.
    assert!(close(l.characters[0].baseline, 50.0 - (7.2 - 1.8) / 2.0));
}

#[test]
fn words_wrap_at_spaces() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let l = layout_text(
        &fonts,
        &TextParams::new("xx xx xx", 0, 9.0),
        rect(20.0, 100.0),
        0.0,
    );
    // "xx xx" needs 20.25: each word on its own line.
    assert_eq!(l.lines.len(), 3);
    let lines: Vec<usize> = visible(&l).iter().map(|c| c.line).collect();
    assert_eq!(lines, [0, 0, 1, 1, 2, 2]);
    // Lines 9.81 apart; each starts at the left.
    let v = visible(&l);
    assert!(
        close(v[0].baseline - v[2].baseline, 9.81),
        "{}",
        v[0].baseline - v[2].baseline
    );
    assert!(close(v[2].origin, 0.0));
    // Without wrapping it overflows on one line.
    let mut p = TextParams::new("xx xx xx", 0, 9.0);
    p.word_wrap = false;
    let l = layout_text(&fonts, &p, rect(20.0, 100.0), 0.0);
    assert_eq!(l.lines.len(), 1);
    assert!(close(l.lines[0].max_advance, 31.5));
    // A new line breaks too.
    let l = layout_text(
        &fonts,
        &TextParams::new("x\nx", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    assert_eq!(l.lines.len(), 2);
}

#[test]
fn truncate_hides_what_does_not_fit() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let mut p = TextParams::new("x\nx\nx", 0, 9.0);
    p.overflow = 3;
    // Room for two lines (9 + 9.81 < 20), not three.
    let l = layout_text(&fonts, &p, rect(200.0, 20.0), 0.0);
    assert!(l.truncated);
    assert_eq!(visible(&l).len(), 2);
}

#[test]
fn auto_size_finds_the_largest_size_that_fits() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let mut p = TextParams::new("HHHH", 0, 72.0);
    p.auto_size = true;
    p.font_size_min = 2.0;
    p.font_size_max = 72.0;
    p.word_wrap = false;
    // 4 × 0.7 × size ≤ 10.0001: at most 3.5714.
    let l = layout_text(&fonts, &p, rect(10.0, 100.0), 0.0);
    assert!(
        l.font_size <= 3.5714 && l.font_size > 3.45,
        "{}",
        l.font_size
    );
    assert!(l.iterations > 3 && l.iterations < 100);
    // Sizes are steps of 0.05.
    assert!(close((l.font_size * 20.0).round(), l.font_size * 20.0));
    // Room enough: the maximum.
    let l = layout_text(&fonts, &p, rect(1000.0, 1000.0), 0.0);
    assert_eq!(l.font_size, 72.0);
}

#[test]
fn tags_change_size_style_and_colour() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let l = layout_text(
        &fonts,
        &TextParams::new("x<size=50%>x</size>x", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    let v = visible(&l);
    assert!(close(v[1].x_advance - v[1].origin, 2.25));
    assert!(close(v[2].x_advance - v[2].origin, 4.5));
    // Bold without a bold face: TMP's bold spacing, 7 × size / 100.
    let l = layout_text(
        &fonts,
        &TextParams::new("<b>x</b>x", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    let v = visible(&l);
    assert!(close(v[0].x_advance, 4.5 + 0.63));
    assert_eq!(v[0].style & style::BOLD, style::BOLD);
    assert_eq!(v[1].style & style::BOLD, 0);
    let l = layout_text(
        &fonts,
        &TextParams::new("<color=#FF000080>x</color>x", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    assert_eq!(l.characters[0].color, [255, 0, 0, 128]);
    assert_eq!(l.characters[1].color, [255; 4]);
    // An unknown tag is text; a missing character becomes '?' or a
    // space: '?' is not in the test font, the space is.
    let l = layout_text(
        &fonts,
        &TextParams::new("<q>", 0, 9.0),
        rect(200.0, 100.0),
        0.0,
    );
    assert_eq!(l.characters.len(), 3);
    assert!(l.characters.iter().all(|c| c.unicode == 32));
}

#[test]
fn kerning_and_spacing() {
    let bytes = crate::font::tests::test_font();
    let mut fonts = set(&bytes);
    let x = fonts.fonts[0].file.glyph_index('x').unwrap();
    fonts.fonts[0].kerning.insert((x, x), -10.0);
    let mut p = TextParams::new("xx", 0, 9.0);
    let l = layout_text(&fonts, &p, rect(200.0, 100.0), 0.0);
    assert!(close(l.characters[0].x_advance, 4.5 - 1.0));
    p.kerning = false;
    p.character_spacing = 10.0;
    p.word_spacing = 0.0;
    let l = layout_text(&fonts, &p, rect(200.0, 100.0), 0.0);
    assert!(close(l.characters[0].x_advance, 4.5 + 0.9));
}

#[test]
fn preferred_sizes() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let p = TextParams::new("xx xx", 0, 9.0);
    let [w, h] = preferred_values(&fonts, &p, 0.0);
    // 20.25 wide, rounded up to the hundredth; one line, 7.2 + 1.8.
    assert!(close(w, 20.26), "{w}");
    assert!(close(h, 9.01), "{h}");
    // Wrapped at 20: two lines, 9.81 apart.
    let [_, h] = preferred_values(&fonts, &p, 20.0);
    assert!(close(h, 18.82), "{h}");
    assert_eq!(
        preferred_values(&fonts, &TextParams::new("", 0, 9.0), 0.0),
        [0.0, 0.0]
    );
}

#[test]
fn a_fitter_takes_the_text_s_preferred_size() {
    use crate::layout::{Comp, Component, Node, RectTransform, Tree, layout_with};
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let node = |name: &str, parent, kinds: Vec<Component>| Node {
        name: name.into(),
        parent,
        children: Vec::new(),
        active_self: true,
        rect: RectTransform {
            size_delta: [100.0, 50.0],
            ..RectTransform::default()
        },
        components: kinds
            .into_iter()
            .map(|kind| Comp {
                kind,
                enabled: true,
            })
            .collect(),
    };
    let mut nodes = vec![
        node("canvas", None, vec![]),
        node(
            "label",
            Some(0),
            vec![Component::Text, Component::SizeFitter { fit: [2, 2] }],
        ),
    ];
    nodes[0].children.push(1);
    let mut tree = Tree::new(nodes);
    let texts = Texts {
        fonts: &fonts,
        params: [(1, TextParams::new("xx xx", 0, 9.0))]
            .into_iter()
            .collect(),
    };
    layout_with(&mut tree, &texts);
    // Width first (20.26), then the height wrapped at that width: one line.
    assert_eq!(tree.nodes[1].rect.size_delta, [20.26, 9.01]);
}

/// Font 0 with a material padding of `pad`, gradient scale `g` and ratio
/// A 0.9 (M9u5a).
fn pads(pad: f32, g: f32) -> Pads {
    let mut p = Pads::default();
    p.fonts.insert(
        0,
        FontPad {
            padding: pad,
            gradient_scale: Some(g),
            scale_ratio_a: Some(0.9),
        },
    );
    p
}

#[test]
fn material_padding_grows_the_quads() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let l = layout_text_with(
        &fonts,
        &TextParams::new("Hx", 0, 9.0),
        rect(200.0, 100.0),
        &pads(2.0, 10.0),
    );
    let v = visible(&l);
    // Normal style 0: no style padding. H: (9 − 2) · 0.1 from the left,
    // its top (63 + 2) · 0.1 above the baseline 92.8.
    assert!(close(v[0].bottom_left[0], 0.7));
    assert!(close(v[0].top_left[1], 92.8 + 6.5));
    assert!(close(v[0].padding, 2.0) && v[0].style_padding == 0.0);
    // The advance does not change.
    assert!(close(v[1].origin, 6.3));
}

#[test]
fn bold_style_padding_and_its_clamp() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let mut p = TextParams::new("H", 0, 9.0);
    p.style = style::BOLD;
    let l = layout_text_with(&fonts, &p, rect(200.0, 100.0), &pads(2.0, 10.0));
    let c = visible(&l)[0];
    // Bold style 0.75 / 4 · 10 · 0.9 = 1.6875; padding stays 2.
    assert!(close(c.style_padding, 1.6875));
    assert!(close(c.padding, 2.0));
    assert!(close(c.bottom_left[0], (9.0 - 2.0 - 1.6875) * 0.1));
    // Gradient 3: style 0.50625, and 2.8 + 0.50625 > 3 shrinks the
    // padding to 3 − 0.50625.
    let l = layout_text_with(&fonts, &p, rect(200.0, 100.0), &pads(2.8, 3.0));
    let c = visible(&l)[0];
    assert!(close(c.style_padding, 0.50625));
    assert!(close(c.padding, 2.49375));
}

fn atlas_glyph(_font: usize, _glyph: u32) -> Option<AtlasGlyph> {
    Some(AtlasGlyph {
        rect: crate::font::GlyphRect::new(10, 20, 7, 8),
        atlas_width: 100,
        atlas_height: 200,
    })
}

#[test]
fn text_vertices_uvs_and_scale() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let p = TextParams::new("H", 0, 9.0);
    let l = layout_text_with(&fonts, &p, rect(200.0, 100.0), &pads(2.0, 10.0));
    let options = MeshOptions {
        horizontal_mapping: 0,
        vertical_mapping: 0,
        sdf_shader: true,
        lossy_scale_y: -2.0,
    };
    let (meshes, unported) = text_meshes(&l, &p, &options, atlas_glyph);
    assert!(unported.is_empty());
    assert_eq!(meshes.len(), 1);
    let v = &meshes[0].mesh.vertices;
    assert_eq!(v.len(), 4);
    assert_eq!(meshes[0].mesh.indices, vec![0, 1, 2, 2, 3, 0]);
    // Rect (10, 20, 7, 8) in 100 × 200, grown by the padding 2.
    let uv = |i: usize| v[i].uv;
    assert!(close(uv(0)[0], 0.08) && close(uv(0)[1], 0.09));
    assert!(close(uv(1)[0], 0.08) && close(uv(1)[1], 0.15));
    assert!(close(uv(2)[0], 0.19) && close(uv(2)[1], 0.15));
    assert!(close(uv(3)[0], 0.19) && close(uv(3)[1], 0.09));
    // Scale 0.1 × |−2|; packed corners (0, 0), (0, 1), (1, 1), (1, 0).
    assert!(v.iter().all(|x| close(x.uv1[1], 0.2)));
    assert_eq!(v[0].uv1[0], 0.0);
    assert_eq!(v[1].uv1[0], 511.0);
    assert_eq!(v[2].uv1[0], 511.0 * 4096.0 + 511.0);
    assert_eq!(v[3].uv1[0], 511.0 * 4096.0);
    // Positions are the layout's quad.
    assert_eq!(v[0].pos, l.characters[0].bottom_left);
}

#[test]
fn bold_text_vertices_and_non_sdf_uvs() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let mut p = TextParams::new("H", 0, 9.0);
    p.style = style::BOLD;
    p.color = [255, 255, 255, 100];
    let l = layout_text_with(&fonts, &p, rect(200.0, 100.0), &pads(2.0, 10.0));
    let mut options = MeshOptions {
        horizontal_mapping: 0,
        vertical_mapping: 0,
        sdf_shader: true,
        lossy_scale_y: 1.0,
    };
    let (m, _) = text_meshes(&l, &p, &options, atlas_glyph);
    let v = &m[0].mesh.vertices;
    // Bold: the scale is negative; the UVs reach the style padding too.
    assert!(close(v[0].uv1[1], -0.1));
    assert!(close(v[0].uv[0], (10.0 - 3.6875) / 100.0));
    // The text colour's alpha caps the vertices'.
    assert_eq!(v[0].color[3], 100);
    options.sdf_shader = false;
    let (m, _) = text_meshes(&l, &p, &options, atlas_glyph);
    assert!(close(m[0].mesh.vertices[0].uv[0], 0.08));
}

#[test]
fn characters_without_glyphs_and_other_mappings_are_counted() {
    let bytes = crate::font::tests::test_font();
    let fonts = set(&bytes);
    let p = TextParams::new("Hx", 0, 9.0);
    let l = layout_text(&fonts, &p, rect(200.0, 100.0), 0.0);
    let options = MeshOptions {
        horizontal_mapping: 1,
        vertical_mapping: 0,
        sdf_shader: true,
        lossy_scale_y: 1.0,
    };
    let x = l.characters[1].glyph;
    let (m, unported) = text_meshes(&l, &p, &options, |f, g| {
        if g == x { None } else { atlas_glyph(f, g) }
    });
    assert_eq!(m[0].mesh.vertices.len(), 4);
    assert_eq!(unported.len(), 2, "{unported:?}");
}
