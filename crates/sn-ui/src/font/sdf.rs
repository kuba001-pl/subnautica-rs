//! A glyph's signed distance field, as TMP's atlases hold it (Alpha8).
//!
//! Encoding (*confirmed* against the game's stored `Aller_Rg` glyphs and
//! materials, `docs/formats/ui.md` § Fonts): the texel is
//! `0.5 − d / (2·s)` scaled to 0–255, where `d` is the distance from the
//! texel's centre to the outline in pixels (positive outside, negative
//! inside) and `s` the material's `_GradientScale`, which TMP sets to the
//! atlas padding + 1. The field covers the glyph's bitmap box with
//! `padding` texels on each side.
//!
//! Distance: the exact distance from each texel's centre to the
//! flattened outline, inside by the non-zero winding rule. The game's
//! render mode (SDFAA) derives it from an anti-aliased bitmap instead;
//! the difference is measured on the stored glyphs (`sn-inspect ui
//! --fonts`), not assumed.

use super::ScaledGlyph;

/// A distance field, bottom row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<u8>,
}

/// The texel value for a signed distance (pixels, positive outside) with
/// gradient scale `spread`.
pub fn encode(distance: f64, spread: f64) -> u8 {
    let v = 0.5 - distance / (2.0 * spread);
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// Signed distance from `p` to closed polylines: negative inside
/// (non-zero winding).
pub fn signed_distance(contours: &[Vec<[f64; 2]>], p: [f64; 2]) -> f64 {
    let mut best = f64::INFINITY;
    let mut winding = 0i32;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let a = c[i];
            let b = c[(i + 1) % n];
            best = best.min(segment_distance(p, a, b));
            // A ray to +x: count upward and downward crossings.
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                if x > p[0] {
                    winding += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
    }
    if winding != 0 { -best } else { best }
}

/// The glyph's field: `(width + 2·padding) × (height + 2·padding)`, with
/// gradient scale `padding + 1`.
pub fn glyph_field(glyph: &ScaledGlyph, padding: u32) -> Field {
    let width = glyph.width as usize + 2 * padding as usize;
    let height = glyph.height as usize + 2 * padding as usize;
    let spread = f64::from(padding) + 1.0;
    let x0 = f64::from(glyph.left) - f64::from(padding);
    let y0 = f64::from(glyph.bottom) - f64::from(padding);
    let mut texels = vec![0u8; width * height];
    if glyph.contours.is_empty() {
        return Field {
            width,
            height,
            texels,
        };
    }
    for j in 0..height {
        for i in 0..width {
            let p = [x0 + i as f64 + 0.5, y0 + j as f64 + 0.5];
            texels[j * width + i] = encode(signed_distance(&glyph.contours, p), spread);
        }
    }
    Field {
        width,
        height,
        texels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_is_half_and_the_gradient_is_the_scale() {
        assert_eq!(encode(0.0, 10.0), 128);
        // One texel outside: 0.5 − 1/20.
        assert_eq!(encode(1.0, 10.0), 115);
        assert_eq!(encode(-1.0, 10.0), 140);
        assert_eq!(encode(10.0, 10.0), 0);
        assert_eq!(encode(-50.0, 10.0), 255);
    }

    #[test]
    fn distances_to_a_square_with_a_hole() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        let c = [outer, hole];
        assert!((signed_distance(&c, [-2.0, 5.0]) - 2.0).abs() < 1e-9);
        assert!((signed_distance(&c, [2.0, 5.0]) + 2.0).abs() < 1e-9);
        // In the hole: outside.
        assert!((signed_distance(&c, [5.0, 5.0]) - 1.0).abs() < 1e-9);
        // Diagonal from a corner.
        assert!((signed_distance(&c, [13.0, 14.0]) - 5.0).abs() < 1e-9);
    }
}
