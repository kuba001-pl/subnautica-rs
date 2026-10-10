//! Blend tree weights (`docs/formats/animation.md` § Blend trees): which
//! leaves a state plays and how much of each.

use sn_unity::{BlendNode, BlendType};

/// A leaf of a state's blend tree with its share of the state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaf {
    /// The controller's clip index (`u32::MAX`: no clip).
    pub clip: u32,
    pub weight: f32,
    /// The node's duration factor: the clip plays over its length times
    /// this (negative: backwards). **Hypothesis**: 1 in 2,378 of 2,430
    /// leaves; the others are 0.5, 2, −1, ⅓, ⅔, 5, ¼ and 0.1.
    pub duration: f32,
    pub cycle_offset: f32,
}

/// 1D: between the two neighbouring thresholds, linearly; clamped to the
/// first and last child. Thresholds are taken in the stored order, which
/// is ascending in the game's data.
pub fn weights_1d(thresholds: &[f32], x: f32) -> Vec<f32> {
    let n = thresholds.len();
    let mut w = vec![0.0; n];
    if n == 0 {
        return w;
    }
    if x <= thresholds[0] || n == 1 {
        w[0] = 1.0;
        return w;
    }
    if x >= thresholds[n - 1] {
        w[n - 1] = 1.0;
        return w;
    }
    for i in 0..n - 1 {
        let (a, b) = (thresholds[i], thresholds[i + 1]);
        if x >= a && x <= b {
            let t = if b > a { (x - a) / (b - a) } else { 0.0 };
            w[i] = 1.0 - t;
            w[i + 1] = t;
            break;
        }
    }
    w
}

fn angle_of(p: [f32; 2]) -> f32 {
    p[1].atan2(p[0])
}

/// 2D simple directional (**hypothesis**, Unity's algorithm is not
/// published): the input lies in the triangle of the centre and the two
/// children whose directions bracket it; weights are its barycentric
/// coordinates, the centre's share going to the two when there is no
/// centre child; beyond the outer edge the two share by the edge. At the
/// centre: the centre child, or all children equally without one.
pub fn weights_2d_simple(positions: &[[f32; 2]], p: [f32; 2]) -> Vec<f32> {
    let n = positions.len();
    let mut w = vec![0.0; n];
    if n == 0 {
        return w;
    }
    let centre = positions
        .iter()
        .position(|q| q[0].abs() < 1e-5 && q[1].abs() < 1e-5);
    let len = (p[0] * p[0] + p[1] * p[1]).sqrt();
    let around: Vec<usize> = (0..n).filter(|&i| Some(i) != centre).collect();
    if len < 1e-5 || around.is_empty() {
        match centre {
            Some(c) => w[c] = 1.0,
            None => w.iter_mut().for_each(|x| *x = 1.0 / n as f32),
        }
        return w;
    }
    if around.len() == 1 {
        let a = around[0];
        let pa = positions[a];
        let la = (pa[0] * pa[0] + pa[1] * pa[1]).sqrt();
        let t = (len / la.max(1e-5)).min(1.0);
        w[a] = t;
        match centre {
            Some(c) => w[c] = 1.0 - t,
            None => w[a] = 1.0,
        }
        return w;
    }
    // Neighbours by angle: the first child counter-clockwise from p (b)
    // and the first clockwise (a).
    let ap = angle_of(p);
    let tau = std::f32::consts::TAU;
    let ccw = |i: usize| (angle_of(positions[i]) - ap).rem_euclid(tau);
    let b = *around
        .iter()
        .min_by(|&&i, &&j| ccw(i).total_cmp(&ccw(j)))
        .unwrap_or(&around[0]);
    let a = *around
        .iter()
        .max_by(|&&i, &&j| ccw(i).total_cmp(&ccw(j)))
        .unwrap_or(&around[0]);
    if ccw(b) < 1e-5 {
        // Exactly in a child's direction: it and the centre.
        let pb = positions[b];
        let lb = (pb[0] * pb[0] + pb[1] * pb[1]).sqrt();
        let t = (len / lb.max(1e-5)).min(1.0);
        match centre {
            Some(c) => {
                w[b] = t;
                w[c] = 1.0 - t;
            }
            None => w[b] = 1.0,
        }
        return w;
    }
    // p = s·pa + t·pb.
    let (pa, pb) = (positions[a], positions[b]);
    let det = pa[0] * pb[1] - pa[1] * pb[0];
    let (mut s, mut t) = if det.abs() > 1e-8 {
        (
            (p[0] * pb[1] - p[1] * pb[0]) / det,
            (pa[0] * p[1] - pa[1] * p[0]) / det,
        )
    } else {
        // The two are opposite (a gap of 180° or more): split by angle.
        let span = (ccw(a) - ccw(b)).abs().max(1e-5);
        let t = 1.0 - ccw(b) / span;
        (1.0 - t, t)
    };
    s = s.max(0.0);
    t = t.max(0.0);
    let sum = s + t;
    if sum > 1.0 || centre.is_none() {
        let sum = sum.max(1e-8);
        w[a] += s / sum;
        w[b] += t / sum;
    } else {
        w[a] += s;
        w[b] += t;
        if let Some(c) = centre {
            w[c] = 1.0 - sum;
        }
    }
    w
}

fn signed_angle(a: [f32; 2], b: [f32; 2]) -> f32 {
    let la = a[0] * a[0] + a[1] * a[1];
    let lb = b[0] * b[0] + b[1] * b[1];
    if la < 1e-10 || lb < 1e-10 {
        return 0.0;
    }
    (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1])
}

/// 2D freeform directional: gradient band interpolation in polar space.
/// Each pair (i, j) has the vector `(−angle(pᵢ→pⱼ), (|pᵢ| − |pⱼ|) · kᵢⱼ)`
/// with `kᵢⱼ = 2 / (|pᵢ| + |pⱼ|)`: confirmed, those are exactly the
/// stored `m_ChildPairVectorArray` and `m_ChildPairAvgMagInvArray` of the
/// game's 5 such nodes. Child i's weight is the smallest over j of
/// `1 − (uᵢ·vᵢⱼ)/|vᵢⱼ|²` (clamped to 0..1) with `uᵢ` the same vector from
/// pᵢ to the input, then all are normalised (**hypothesis** for the use
/// of the vectors, after Johansen's gradient bands).
pub fn weights_2d_freeform(positions: &[[f32; 2]], p: [f32; 2]) -> Vec<f32> {
    let n = positions.len();
    let mag = |q: [f32; 2]| (q[0] * q[0] + q[1] * q[1]).sqrt();
    let lp = mag(p);
    let mut w = vec![0.0; n];
    for (i, &pi) in positions.iter().enumerate() {
        let li = mag(pi);
        let mut h = 1.0f32;
        for (j, &pj) in positions.iter().enumerate() {
            if i == j {
                continue;
            }
            let lj = mag(pj);
            let k = if li + lj > 1e-8 { 2.0 / (li + lj) } else { 0.0 };
            let v = [-signed_angle(pi, pj), (li - lj) * k];
            let u = [-signed_angle(pi, p), (li - lp) * k];
            let vv = v[0] * v[0] + v[1] * v[1];
            if vv < 1e-12 {
                continue;
            }
            h = h.min(1.0 - (u[0] * v[0] + u[1] * v[1]) / vv);
        }
        w[i] = h.clamp(0.0, 1.0);
    }
    let sum: f32 = w.iter().sum();
    if sum > 1e-8 {
        w.iter_mut().for_each(|x| *x /= sum);
    } else if n > 0 {
        w.iter_mut().for_each(|x| *x = 1.0 / n as f32);
    }
    w
}

/// The leaves a blend tree plays with their weights (summing to 1 when
/// any leaf has a clip). `param` reads a float parameter by name hash.
/// Unsupported node types play their children equally (none in the game;
/// counted by the caller from `BlendType`).
pub fn leaves(nodes: &[BlendNode], param: &dyn Fn(u32) -> f32) -> Vec<Leaf> {
    let mut out = Vec::new();
    if !nodes.is_empty() {
        walk(nodes, 0, 1.0, param, &mut out, 0);
    }
    out
}

fn walk(
    nodes: &[BlendNode],
    i: usize,
    weight: f32,
    param: &dyn Fn(u32) -> f32,
    out: &mut Vec<Leaf>,
    depth: usize,
) {
    let Some(n) = nodes.get(i) else {
        return;
    };
    if depth > 32 || weight <= 0.0 {
        return;
    }
    if n.is_leaf() {
        out.push(Leaf {
            clip: n.clip,
            weight,
            duration: n.duration,
            cycle_offset: n.cycle_offset,
        });
        return;
    }
    let ws = match n.kind {
        BlendType::Simple1D => weights_1d(&n.thresholds, param(n.param)),
        BlendType::SimpleDirectional2D => {
            weights_2d_simple(&n.positions, [param(n.param), param(n.param_y)])
        }
        BlendType::FreeformDirectional2D => {
            weights_2d_freeform(&n.positions, [param(n.param), param(n.param_y)])
        }
        _ => vec![1.0 / n.children.len() as f32; n.children.len()],
    };
    for (&child, &w) in n.children.iter().zip(&ws) {
        walk(nodes, child as usize, weight * w, param, out, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(w: &[f32]) -> f32 {
        w.iter().sum()
    }

    #[test]
    fn one_d_between_neighbours_and_clamped() {
        let t = [0.0, 1.0, 3.0];
        assert_eq!(weights_1d(&t, -1.0), [1.0, 0.0, 0.0]);
        assert_eq!(weights_1d(&t, 0.25), [0.75, 0.25, 0.0]);
        assert_eq!(weights_1d(&t, 2.0), [0.0, 0.5, 0.5]);
        assert_eq!(weights_1d(&t, 9.0), [0.0, 0.0, 1.0]);
    }

    const CROSS: [[f32; 2]; 5] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, -1.0], [-1.0, 0.0]];

    #[test]
    fn two_d_simple_on_samples_between_and_beyond() {
        // On a child: that child only.
        let w = weights_2d_simple(&CROSS, [0.0, 1.0]);
        assert_eq!(w, [0.0, 1.0, 0.0, 0.0, 0.0]);
        // Halfway out towards a child: half it, half the centre.
        let w = weights_2d_simple(&CROSS, [0.5, 0.0]);
        assert!((w[2] - 0.5).abs() < 1e-6 && (w[0] - 0.5).abs() < 1e-6);
        // Between two children: barycentric with the centre.
        let w = weights_2d_simple(&CROSS, [0.25, 0.25]);
        assert!((w[1] - 0.25).abs() < 1e-6 && (w[2] - 0.25).abs() < 1e-6);
        assert!((w[0] - 0.5).abs() < 1e-6);
        // Beyond the edge: the two only.
        let w = weights_2d_simple(&CROSS, [2.0, 2.0]);
        assert!((w[1] - 0.5).abs() < 1e-6 && (w[2] - 0.5).abs() < 1e-6 && w[0] == 0.0);
        // At the centre.
        assert_eq!(weights_2d_simple(&CROSS, [0.0, 0.0])[0], 1.0);
        for p in [[0.3, -0.8], [-2.0, 0.1], [0.01, 0.0]] {
            assert!((sum(&weights_2d_simple(&CROSS, p)) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn two_d_freeform_is_exact_on_children() {
        // The game's 9-child layout (`cave_crawler_controller`).
        let pos = [
            [0.0, 1.0],
            [0.0, 0.5],
            [0.0, -0.5],
            [0.0, -1.0],
            [0.0, 0.0],
            [0.5, 0.0],
            [1.0, 0.0],
            [-0.5, 0.0],
            [-1.0, 0.0],
        ];
        for (i, &p) in pos.iter().enumerate() {
            let w = weights_2d_freeform(&pos, p);
            assert!((w[i] - 1.0).abs() < 1e-5, "{i}: {w:?}");
        }
        for p in [[0.2, 0.7], [-0.9, -0.3], [3.0, 0.0]] {
            let w = weights_2d_freeform(&pos, p);
            assert!((sum(&w) - 1.0).abs() < 1e-5);
            assert!(w.iter().all(|x| *x >= 0.0));
        }
    }

    #[test]
    fn nested_trees_multiply_weights() {
        use sn_unity::name_hash;
        let leaf = |clip: u32| BlendNode {
            kind: BlendType::Simple1D,
            param: 0,
            param_y: 0,
            children: vec![],
            thresholds: vec![],
            positions: vec![],
            magnitudes: vec![],
            pair_vectors: vec![],
            pair_avg_mag_inv: vec![],
            neighbors: vec![],
            direct_params: vec![],
            normalized_blend_values: false,
            clip,
            duration: 1.0,
            cycle_offset: 0.0,
            mirror: false,
        };
        let mut root = leaf(u32::MAX);
        root.children = vec![1, 2];
        root.thresholds = vec![0.0, 1.0];
        root.param = name_hash("a");
        let mut inner = leaf(u32::MAX);
        inner.children = vec![3, 4];
        inner.thresholds = vec![0.0, 1.0];
        inner.param = name_hash("b");
        let nodes = vec![root, leaf(0), inner, leaf(1), leaf(2)];
        let p = |h: u32| if h == name_hash("a") { 0.5 } else { 0.25 };
        let l = leaves(&nodes, &p);
        let w: Vec<(u32, f32)> = l.iter().map(|l| (l.clip, l.weight)).collect();
        assert_eq!(w, [(0, 0.5), (1, 0.375), (2, 0.125)]);
    }
}
