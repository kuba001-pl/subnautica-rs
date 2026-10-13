//! Anti-aliased coverage of an outline: for every pixel, the area inside
//! the outline (non-zero winding, clamped to 1). Our own area
//! accumulation: each edge, split at rows and columns, adds its signed
//! height to the cell it crosses, weighted by how much of the cell lies
//! to its right; a running sum along each row gives the coverage. This is
//! the same quantity FreeType's smooth rasterizer computes, so a glyph's
//! coverage is exact up to the curves' flattening.

/// Coverage in `[0, 1]`, `width × height`, bottom row first.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl Coverage {
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.values[y * self.width + x]
    }
}

/// Rasterizes closed polylines given in pixels, where pixel `(x, y)` is
/// the square `[x, x+1) × [y, y+1)`. Parts outside the box are clipped
/// (left of it counts as fully left).
pub fn rasterize(contours: &[Vec<[f64; 2]>], width: usize, height: usize) -> Coverage {
    // One extra column: an edge in the last column carries into it.
    let stride = width + 1;
    let mut acc = vec![0f64; stride * height];
    for c in contours {
        let n = c.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let p0 = c[i];
            let p1 = c[(i + 1) % n];
            edge(&mut acc, stride, width, height, p0, p1);
        }
    }
    let mut values = vec![0f32; width * height];
    for y in 0..height {
        let mut sum = 0.0;
        for x in 0..width {
            sum += acc[y * stride + x];
            values[y * width + x] = sum.abs().min(1.0) as f32;
        }
    }
    Coverage {
        width,
        height,
        values,
    }
}

fn edge(acc: &mut [f64], stride: usize, width: usize, height: usize, p0: [f64; 2], p1: [f64; 2]) {
    if p0[1] == p1[1] || !(p0[0].is_finite() && p0[1].is_finite()) {
        return;
    }
    if !(p1[0].is_finite() && p1[1].is_finite()) {
        return;
    }
    // Upward edges add, downward subtract.
    let (dir, a, b) = if p0[1] < p1[1] {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (b[0] - a[0]) / (b[1] - a[1]);
    let y_from = a[1].max(0.0);
    let y_to = b[1].min(height as f64);
    if y_from >= y_to {
        return;
    }
    let mut row = y_from.floor() as usize;
    while (row as f64) < y_to && row < height {
        let ya = y_from.max(row as f64);
        let yb = y_to.min(row as f64 + 1.0);
        if yb > ya {
            let xa = a[0] + (ya - a[1]) * dxdy;
            let xb = a[0] + (yb - a[1]) * dxdy;
            span(
                &mut acc[row * stride..(row + 1) * stride],
                width,
                dir,
                ya,
                yb,
                xa,
                xb,
            );
        }
        row += 1;
    }
}

/// One edge piece inside a row, from `(xa, ya)` to `(xb, yb)`: split at
/// column boundaries, each part adds `dy·(1 − f)` to its column and `dy·f`
/// to the next, `f` the part's mean position in its column.
fn span(row: &mut [f64], width: usize, dir: f64, ya: f64, yb: f64, xa: f64, xb: f64) {
    let dy = yb - ya;
    let (x0, x1) = if xa <= xb { (xa, xb) } else { (xb, xa) };
    let mut put = |xl: f64, xr: f64, h: f64| {
        let mid = (xl + xr) * 0.5;
        if mid < 0.0 {
            row[0] += dir * h;
            return;
        }
        let col = mid.floor();
        if col >= width as f64 {
            return;
        }
        let ci = col as usize;
        let f = mid - col;
        row[ci] += dir * h * (1.0 - f);
        row[ci + 1] += dir * h * f;
    };
    let c0 = x0.floor();
    let c1 = x1.floor();
    if c0 == c1 || x1 - x0 < 1e-12 {
        put(x0, x1, dy);
        return;
    }
    // Split at each column boundary; the height of each part is
    // proportional to its width.
    let per_x = dy / (x1 - x0);
    let mut left = x0;
    let mut boundary = c0 + 1.0;
    while left < x1 {
        let right = boundary.min(x1);
        put(left, right, (right - left) * per_x);
        left = right;
        boundary += 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn a_pixel_aligned_square_covers_its_pixels() {
        let c = rasterize(&[square(1.0, 1.0, 3.0, 2.0)], 4, 3);
        let want = [0., 0., 0., 0., 0., 1., 1., 0., 0., 0., 0., 0.];
        assert_eq!(c.values, want.map(|v: f64| v as f32));
    }

    #[test]
    fn half_pixels_and_both_windings() {
        // Counter-clockwise and clockwise give the same coverage.
        let ccw = square(0.5, 0.0, 1.5, 1.0);
        let mut cw = ccw.clone();
        cw.reverse();
        for c in [ccw, cw] {
            let r = rasterize(&[c], 2, 1);
            assert_eq!(r.values, [0.5, 0.5]);
        }
    }

    #[test]
    fn a_diagonal_halves_a_pixel_and_area_is_kept() {
        let tri = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let r = rasterize(&[tri], 1, 1);
        assert!((r.values[0] - 0.5).abs() < 1e-6);
        // A sloped quad over many cells: the sum is its area.
        let quad = vec![[0.3, 0.2], [5.7, 1.1], [4.9, 3.8], [0.8, 2.6]];
        let r = rasterize(std::slice::from_ref(&quad), 7, 5);
        let area: f64 = r.values.iter().map(|&v| f64::from(v)).sum();
        let shoelace = 0.5
            * (0..4)
                .map(|i| {
                    let (a, b) = (quad[i], quad[(i + 1) % 4]);
                    a[0] * b[1] - b[0] * a[1]
                })
                .sum::<f64>()
                .abs();
        assert!((area - shoelace).abs() < 1e-4, "{area} {shoelace}");
    }

    #[test]
    fn overlaps_clamp_and_holes_cut() {
        // Two overlapping squares of the same winding: covered once.
        let a = square(0.0, 0.0, 2.0, 1.0);
        let r = rasterize(&[a.clone(), a.clone()], 2, 1);
        assert_eq!(r.values, [1.0, 1.0]);
        // A hole (opposite winding) empties its pixel.
        let mut hole = square(1.0, 0.0, 2.0, 1.0);
        hole.reverse();
        let r = rasterize(&[a, hole], 2, 1);
        assert_eq!(r.values, [1.0, 0.0]);
    }

    #[test]
    fn outside_the_box_is_clipped_without_panicking() {
        let r = rasterize(&[square(-3.0, -2.0, 1.5, 9.0)], 3, 2);
        assert_eq!(r.values, [1.0, 0.5, 0.0, 1.0, 0.5, 0.0]);
        let r = rasterize(&[square(5.0, 0.0, 9.0, 1.0)], 3, 1);
        assert_eq!(r.values, [0.0; 3]);
        let r = rasterize(&[vec![[f64::NAN, 0.0], [1.0, 1.0], [0.0, 1.0]]], 2, 2);
        assert_eq!(r.values.len(), 4);
    }
}
