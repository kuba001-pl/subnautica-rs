//! A capsule moving through the world (M9a, `docs/DESIGN.md` § 4.3 "M9a
//! plan"). The game uses PhysX (a `Rigidbody` with a `CapsuleCollider`
//! under water); this is our own kinematic version, not a port.
//!
//! - The world is a set of **bodies** (a terrain batch, one placed object),
//!   each holding triangles and primitive shapes in world space, with a
//!   uniform grid for the broad phase.
//! - [`World::sweep`] moves the capsule along a straight line and returns
//!   the first contact. It uses conservative advancement on exact distances
//!   between the capsule's segment and each shape: the capsule never moves
//!   further than the current gap, so it cannot pass through a thin wall
//!   however fast it moves. Every shape is convex, so the distance to one
//!   shape is a convex function of the time along the move: once it stops
//!   shrinking, it never shrinks again.
//! - [`World::move_and_slide`] sweeps, then slides the rest of the move
//!   along what it hit (up to [`MAX_SLIDES`] times), keeping a gap of at
//!   least [`SKIN`] / 2.
//!
//! Triangles collide on both sides (the game's are one-sided or not:
//! not checked).

use std::collections::{BTreeMap, HashMap};

use crate::V3;

/// Gap the capsule keeps from what it touches. A sweep stops between
/// `SKIN / 2` and `SKIN` before the contact.
pub const SKIN: f64 = 0.01;

/// Slides per [`World::move_and_slide`] call.
pub const MAX_SLIDES: usize = 4;

/// Conservative advancement steps per shape before giving up and stopping
/// where it is (still at least `SKIN / 2` away).
const MAX_ADVANCE_STEPS: usize = 64;

/// Broad-phase grid cell, metres.
const CELL: f64 = 4.0;

/// Shapes covering more cells than this are checked on every query.
const MAX_CELLS_PER_SHAPE: i64 = 4096;

/// Moving along a contact's plane counts as moving away if its approach
/// speed is below this share of the move (rounding when sliding).
const TANGENT_TOLERANCE: f64 = 1e-6;

/// Pushes the slid move slightly off the plane it hit, so the next sweep
/// sees it moving away.
const OVERBOUNCE: f64 = 1.001;

/// A capsule: the segment `a`–`b` (relative to the mover's position) and
/// the radius around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: V3,
    pub b: V3,
    pub radius: f64,
}

impl Capsule {
    /// Unity's `CapsuleCollider`: `height` along `direction` (0 x, 1 y,
    /// 2 z), including the two caps, around `center`. A height below twice
    /// the radius makes a sphere.
    pub fn unity(radius: f64, height: f64, direction: usize, center: V3) -> Capsule {
        let half = (height * 0.5 - radius).max(0.0);
        let axis = match direction {
            0 => V3::new(1.0, 0.0, 0.0),
            1 => V3::Y,
            _ => V3::new(0.0, 0.0, 1.0),
        };
        Capsule {
            a: center - axis * half,
            b: center + axis * half,
            radius,
        }
    }
}

/// A primitive shape in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Triangle([V3; 3]),
    Sphere {
        center: V3,
        radius: f64,
    },
    Capsule {
        a: V3,
        b: V3,
        radius: f64,
    },
    /// Oriented box: `axes` are unit and perpendicular, `half` the half
    /// extents along them.
    Box {
        center: V3,
        axes: [V3; 3],
        half: [f64; 3],
    },
}

impl Shape {
    fn bounds(&self) -> (V3, V3) {
        match *self {
            Shape::Triangle([a, b, c]) => (a.min(b).min(c), a.max(b).max(c)),
            Shape::Sphere { center, radius } => {
                let r = V3::new(radius, radius, radius);
                (center - r, center + r)
            }
            Shape::Capsule { a, b, radius } => {
                let r = V3::new(radius, radius, radius);
                (a.min(b) - r, a.max(b) + r)
            }
            Shape::Box { center, axes, half } => {
                let e = |k: usize| {
                    (0..3)
                        .map(|i| (axes[i].get(k) * half[i]).abs())
                        .sum::<f64>()
                };
                let e = V3::new(e(0), e(1), e(2));
                (center - e, center + e)
            }
        }
    }

    fn is_valid(&self) -> bool {
        match *self {
            Shape::Triangle([a, b, c]) => {
                a.is_finite()
                    && b.is_finite()
                    && c.is_finite()
                    && (b - a).cross(c - a).length() > 1e-10
            }
            Shape::Sphere { center, radius } => {
                center.is_finite() && radius.is_finite() && radius >= 0.0
            }
            Shape::Capsule { a, b, radius } => {
                a.is_finite() && b.is_finite() && radius.is_finite() && radius >= 0.0
            }
            Shape::Box { center, axes, half } => {
                center.is_finite()
                    && axes.iter().all(|a| a.is_finite())
                    && half.iter().all(|h| h.is_finite() && *h >= 0.0)
            }
        }
    }
}

/// Closest points between the point `p` and the triangle `abc` (Ericson,
/// *Real-Time Collision Detection*, 5.1.5; our own code).
fn closest_point_triangle(p: V3, a: V3, b: V3, c: V3) -> V3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Closest points between the segments `p1 q1` and `p2 q2` (Ericson 5.1.9).
fn closest_segments(p1: V3, q1: V3, p2: V3, q2: V3) -> (V3, V3) {
    const EPS: f64 = 1e-12;
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t) = if a <= EPS && e <= EPS {
        (0.0, 0.0)
    } else if a <= EPS {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s = if denom > EPS {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t = (b * s + f) / e;
            if t < 0.0 {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else if t > 1.0 {
                (((b - c) / a).clamp(0.0, 1.0), 1.0)
            } else {
                (s, t)
            }
        }
    };
    (p1 + d1 * s, p2 + d2 * t)
}

/// Where the segment `pq` crosses the triangle `abc`, if it does.
fn segment_crosses_triangle(p: V3, q: V3, a: V3, b: V3, c: V3) -> Option<V3> {
    let n = (b - a).cross(c - a);
    let dp = (p - a).dot(n);
    let dq = (q - a).dot(n);
    if (dp > 0.0 && dq > 0.0) || (dp < 0.0 && dq < 0.0) || dp == dq {
        return None;
    }
    let x = p + (q - p) * (dp / (dp - dq));
    let inside = (b - a).cross(x - a).dot(n) >= 0.0
        && (c - b).cross(x - b).dot(n) >= 0.0
        && (a - c).cross(x - c).dot(n) >= 0.0;
    inside.then_some(x)
}

/// Closest points between the segment `pq` and the triangle `abc`: either
/// they cross, or an end of the segment or an edge of the triangle is part
/// of the closest pair.
fn closest_segment_triangle(p: V3, q: V3, [a, b, c]: [V3; 3]) -> (V3, V3) {
    if let Some(x) = segment_crosses_triangle(p, q, a, b, c) {
        return (x, x);
    }
    let mut best = (p, closest_point_triangle(p, a, b, c));
    let mut best_d = (best.0 - best.1).dot(best.0 - best.1);
    let mut consider = |pair: (V3, V3)| {
        let d = (pair.0 - pair.1).dot(pair.0 - pair.1);
        if d < best_d {
            best = pair;
            best_d = d;
        }
    };
    consider((q, closest_point_triangle(q, a, b, c)));
    consider(closest_segments(p, q, a, b));
    consider(closest_segments(p, q, b, c));
    consider(closest_segments(p, q, c, a));
    best
}

/// The 12 triangles of a box's faces.
fn box_triangles(center: V3, axes: [V3; 3], half: [f64; 3]) -> [[V3; 3]; 12] {
    let mut out = [[V3::ZERO; 3]; 12];
    let mut n = 0;
    for i in 0..3 {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        let u = axes[j] * half[j];
        let v = axes[k] * half[k];
        for sign in [-1.0, 1.0] {
            let f = center + axes[i] * (half[i] * sign);
            let quad = [f - u - v, f + u - v, f + u + v, f - u + v];
            out[n] = [quad[0], quad[1], quad[2]];
            out[n + 1] = [quad[0], quad[2], quad[3]];
            n += 2;
        }
    }
    out
}

fn inside_box(p: V3, center: V3, axes: [V3; 3], half: [f64; 3]) -> bool {
    let d = p - center;
    (0..3).all(|i| d.dot(axes[i]).abs() <= half[i])
}

/// Closest points between the segment `pq` and the shape's core (its
/// surface for triangles and boxes, its centre point or segment otherwise),
/// and the shape's own radius.
fn closest(shape: &Shape, p: V3, q: V3) -> (V3, V3, f64) {
    match *shape {
        Shape::Triangle(t) => {
            let (s, c) = closest_segment_triangle(p, q, t);
            (s, c, 0.0)
        }
        Shape::Sphere { center, radius } => {
            let (s, c) = closest_segments(p, q, center, center);
            (s, c, radius)
        }
        Shape::Capsule { a, b, radius } => {
            let (s, c) = closest_segments(p, q, a, b);
            (s, c, radius)
        }
        Shape::Box { center, axes, half } => {
            for end in [p, q] {
                if inside_box(end, center, axes, half) {
                    return (end, end, 0.0);
                }
            }
            let mut best = (p, p);
            let mut best_d = f64::INFINITY;
            for t in box_triangles(center, axes, half) {
                let (s, c) = closest_segment_triangle(p, q, t);
                let d = (s - c).dot(s - c);
                if d < best_d {
                    best = (s, c);
                    best_d = d;
                }
            }
            (best.0, best.1, 0.0)
        }
    }
}

/// Gap between the capsule at `at` and the shape (negative: overlapping),
/// the contact normal (from the shape towards the capsule) and the
/// contact point on the shape. `fallback` is used as the normal when the
/// cores touch.
fn gap(shape: &Shape, capsule: &Capsule, at: V3, fallback: V3) -> (f64, V3, V3) {
    let (s, c, r) = closest(shape, at + capsule.a, at + capsule.b);
    let sep = s - c;
    let normal = sep.normalized().unwrap_or(fallback);
    (sep.length() - capsule.radius - r, normal, c)
}

/// Sweeps the capsule from `from` by `delta` against one shape, up to the
/// share `max_t` of the move. Returns the share of the move at the contact,
/// the normal and the contact point.
fn sweep_shape(
    shape: &Shape,
    capsule: &Capsule,
    from: V3,
    delta: V3,
    max_t: f64,
) -> Option<(f64, V3, V3)> {
    let len = delta.length();
    let dir = delta.normalized()?;
    let mut t = 0.0;
    for _ in 0..MAX_ADVANCE_STEPS {
        let (g, normal, point) = gap(shape, capsule, from + delta * t, -dir);
        if g <= SKIN {
            // Convex distance: not shrinking now means never shrinking.
            if dir.dot(normal) >= -TANGENT_TOLERANCE {
                return None;
            }
            return Some((t, normal, point));
        }
        t += (g - SKIN * 0.5) / len;
        if t >= max_t {
            return None;
        }
    }
    // Still at least SKIN / 2 away: stopping here is safe.
    let (_, normal, point) = gap(shape, capsule, from + delta * t, -dir);
    Some((t, normal, point))
}

type Cell = [i32; 3];

fn cell_of(p: V3) -> Cell {
    [p.x, p.y, p.z].map(|v| (v / CELL).floor().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}

fn cell_count(lo: Cell, hi: Cell) -> i64 {
    (0..3)
        .map(|a| i64::from(hi[a]) - i64::from(lo[a]) + 1)
        .product()
}

fn overlaps(a: (V3, V3), b: (V3, V3)) -> bool {
    a.0.x <= b.1.x
        && b.0.x <= a.1.x
        && a.0.y <= b.1.y
        && b.0.y <= a.1.y
        && a.0.z <= b.1.z
        && b.0.z <= a.1.z
}

/// Triangles and shapes of one thing in the world (a terrain batch, a
/// placed object), in world space.
#[derive(Clone, Debug, Default)]
pub struct Body {
    /// Kept in `f32` (terrain is large); converted when used.
    triangles: Vec<[[f32; 3]; 3]>,
    shapes: Vec<Shape>,
    bounds: Option<(V3, V3)>,
    /// Ids: triangles first, then shapes.
    cells: HashMap<Cell, Vec<u32>>,
    large: Vec<u32>,
    skipped: usize,
}

impl Body {
    /// Degenerate (zero-area) and non-finite triangles and shapes are
    /// skipped and counted ([`Body::skipped`]).
    pub fn new(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>, shapes: Vec<Shape>) -> Body {
        let mut body = Body::default();
        for t in triangles {
            if Shape::Triangle(t.map(V3::from_f32)).is_valid() {
                body.triangles.push(t);
            } else {
                body.skipped += 1;
            }
        }
        for s in shapes {
            if s.is_valid() {
                body.shapes.push(s);
            } else {
                body.skipped += 1;
            }
        }
        let count = body.triangles.len() + body.shapes.len();
        for id in 0..count as u32 {
            let b = body.shape(id).bounds();
            body.bounds = Some(match body.bounds {
                Some((lo, hi)) => (lo.min(b.0), hi.max(b.1)),
                None => b,
            });
            let (lo, hi) = (cell_of(b.0), cell_of(b.1));
            if cell_count(lo, hi) > MAX_CELLS_PER_SHAPE {
                body.large.push(id);
                continue;
            }
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        body.cells.entry([x, y, z]).or_default().push(id);
                    }
                }
            }
        }
        body
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// Triangles and shapes left out as degenerate or non-finite.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    pub fn bounds(&self) -> Option<(V3, V3)> {
        self.bounds
    }

    fn shape(&self, id: u32) -> Shape {
        let id = id as usize;
        match self.triangles.get(id) {
            Some(t) => Shape::Triangle(t.map(V3::from_f32)),
            None => self.shapes[id - self.triangles.len()],
        }
    }

    /// Ids of the shapes whose cells overlap the box, sorted, no repeats.
    fn candidates(&self, lo: V3, hi: V3, out: &mut Vec<u32>) {
        out.clear();
        out.extend_from_slice(&self.large);
        let (lo, hi) = (cell_of(lo), cell_of(hi));
        if cell_count(lo, hi) > self.cells.len() as i64 {
            for (cell, ids) in &self.cells {
                if (0..3).all(|a| lo[a] <= cell[a] && cell[a] <= hi[a]) {
                    out.extend_from_slice(ids);
                }
            }
        } else {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        if let Some(ids) = self.cells.get(&[x, y, z]) {
                            out.extend_from_slice(ids);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// A contact found by a sweep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Share of the move done before the contact (0..1).
    pub t: f64,
    /// From the shape towards the capsule, unit length.
    pub normal: V3,
    /// On the shape's surface (or core, for spheres and capsules).
    pub point: V3,
    pub body: u64,
}

/// The nearest shape around a capsule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearance {
    /// Negative: overlapping.
    pub gap: f64,
    pub normal: V3,
    pub point: V3,
    pub body: u64,
}

/// Result of [`World::move_and_slide`].
#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub position: V3,
    pub contacts: Vec<Hit>,
}

/// Everything the capsule can hit, as bodies keyed by the caller's ids.
#[derive(Clone, Debug, Default)]
pub struct World {
    /// Ordered, so that results do not depend on hashing.
    bodies: BTreeMap<u64, Body>,
}

impl World {
    pub fn new() -> World {
        World::default()
    }

    /// Adds or replaces the body `id`.
    pub fn insert(&mut self, id: u64, body: Body) -> Option<Body> {
        self.bodies.insert(id, body)
    }

    pub fn remove(&mut self, id: u64) -> Option<Body> {
        self.bodies.remove(&id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.bodies.contains_key(&id)
    }

    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.bodies.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Calls `f` for every shape whose cells overlap the box.
    fn each_candidate(&self, lo: V3, hi: V3, mut f: impl FnMut(u64, Shape)) {
        let mut ids = Vec::new();
        for (&id, body) in &self.bodies {
            if !body.bounds.is_some_and(|b| overlaps(b, (lo, hi))) {
                continue;
            }
            body.candidates(lo, hi, &mut ids);
            for &s in &ids {
                f(id, body.shape(s));
            }
        }
    }

    /// The first contact when the capsule moves from `from` by `delta`.
    pub fn sweep(&self, capsule: &Capsule, from: V3, delta: V3) -> Option<Hit> {
        if delta.length() < 1e-12 || !from.is_finite() || !delta.is_finite() {
            return None;
        }
        let pad = V3::new(1.0, 1.0, 1.0) * (capsule.radius + SKIN);
        let (a, b) = (capsule.a.min(capsule.b), capsule.a.max(capsule.b));
        let lo = (from + a).min(from + delta + a) - pad;
        let hi = (from + b).max(from + delta + b) + pad;
        let mut best: Option<Hit> = None;
        self.each_candidate(lo, hi, |body, shape| {
            let max_t = best.map_or(1.0, |h| h.t);
            if let Some((t, normal, point)) = sweep_shape(&shape, capsule, from, delta, max_t) {
                if best.is_none_or(|h| t < h.t) {
                    best = Some(Hit {
                        t,
                        normal,
                        point,
                        body,
                    });
                }
            }
        });
        best
    }

    /// The nearest shape within `range` of the capsule's surface at `at`.
    pub fn clearance(&self, capsule: &Capsule, at: V3, range: f64) -> Option<Clearance> {
        let pad = V3::new(1.0, 1.0, 1.0) * (capsule.radius + range);
        let lo = (at + capsule.a).min(at + capsule.b) - pad;
        let hi = (at + capsule.a).max(at + capsule.b) + pad;
        let mut best: Option<Clearance> = None;
        self.each_candidate(lo, hi, |body, shape| {
            let (g, normal, point) = gap(&shape, capsule, at, V3::Y);
            if g <= range && best.is_none_or(|b| g < b.gap) {
                best = Some(Clearance {
                    gap: g,
                    normal,
                    point,
                    body,
                });
            }
        });
        best
    }

    /// Pushes the capsule out of anything closer than `SKIN / 2` (a spawn
    /// point inside a rock, rounding). Returns the new position and the
    /// smallest gap found before pushing.
    pub fn push_out(&self, capsule: &Capsule, at: V3) -> (V3, Option<f64>) {
        let mut pos = at;
        let mut first = None;
        for _ in 0..MAX_SLIDES {
            let Some(c) = self.clearance(capsule, pos, SKIN) else {
                break;
            };
            first.get_or_insert(c.gap);
            if c.gap >= SKIN * 0.5 {
                break;
            }
            pos += c.normal * (SKIN - c.gap);
        }
        (pos, first)
    }

    /// Moves the capsule from `from` by `delta`, sliding along what it
    /// hits (Quake-style velocity clipping, with creases between two planes
    /// and a stop at three or when the slide would turn back).
    pub fn move_and_slide(&self, capsule: &Capsule, from: V3, delta: V3) -> Slide {
        let mut pos = from;
        let mut remaining = delta;
        let mut contacts = Vec::new();
        let mut planes: Vec<V3> = Vec::new();
        for _ in 0..MAX_SLIDES {
            if remaining.length() < 1e-9 {
                break;
            }
            let Some(hit) = self.sweep(capsule, pos, remaining) else {
                pos += remaining;
                break;
            };
            pos += remaining * hit.t;
            let left = remaining * (1.0 - hit.t);
            contacts.push(hit);
            planes.push(hit.normal);
            remaining = clip_to_planes(left, &planes, delta);
        }
        Slide {
            position: pos,
            contacts,
        }
    }
}

fn clip(v: V3, n: V3) -> V3 {
    let d = v.dot(n);
    if d < 0.0 { v - n * (d * OVERBOUNCE) } else { v }
}

/// The part of `v` that slides along the planes hit so far (the last one
/// first).
fn clip_to_planes(v: V3, planes: &[V3], original: V3) -> V3 {
    let Some((&last, earlier)) = planes.split_last() else {
        return v;
    };
    let mut out = clip(v, last);
    for &p in earlier {
        if out.dot(p) >= 0.0 {
            continue;
        }
        if planes.len() >= 3 {
            return V3::ZERO;
        }
        // Into an earlier plane: slide along the crease of the two.
        let Some(crease) = last.cross(p).normalized() else {
            return V3::ZERO;
        };
        out = crease * crease.dot(v);
        break;
    }
    if out.dot(original) <= 0.0 {
        return V3::ZERO;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The underwater player (radius 0.3, height 0.75, its centre 0.125 m
    /// under the origin; `docs/DESIGN.md` § 4.3 "M9a plan").
    fn player() -> Capsule {
        Capsule::unity(0.3, 0.75, 1, V3::new(0.0, -0.125, 0.0))
    }

    fn v(x: f64, y: f64, z: f64) -> V3 {
        V3::new(x, y, z)
    }

    fn f(p: V3) -> [f32; 3] {
        p.to_f32()
    }

    /// A square of two triangles: corner `o`, edges `u` and `w`.
    fn quad(o: V3, u: V3, w: V3) -> [[[f32; 3]; 3]; 2] {
        [
            [f(o), f(o + u), f(o + u + w)],
            [f(o), f(o + u + w), f(o + w)],
        ]
    }

    fn world_of(tris: Vec<[[f32; 3]; 3]>, shapes: Vec<Shape>) -> World {
        let mut w = World::new();
        w.insert(1, Body::new(tris, shapes));
        w
    }

    fn floor() -> World {
        world_of(
            quad(v(-50.0, 0.0, -50.0), v(100.0, 0.0, 0.0), v(0.0, 0.0, 100.0)).to_vec(),
            vec![],
        )
    }

    fn bottom(c: &Capsule, pos: V3) -> f64 {
        pos.y + c.a.y.min(c.b.y) - c.radius
    }

    /// Asserts the capsule keeps at least `SKIN / 2` (minus rounding) from
    /// everything.
    fn assert_clear(w: &World, c: &Capsule, pos: V3) {
        if let Some(cl) = w.clearance(c, pos, 1.0) {
            assert!(cl.gap >= SKIN * 0.5 - 1e-9, "gap {} at {pos:?}", cl.gap);
        }
    }

    #[test]
    fn unity_capsule_matches_the_player() {
        let c = player();
        assert!((c.a.y - -0.2).abs() < 1e-12 && (c.b.y - -0.05).abs() < 1e-12);
        let ball = Capsule::unity(0.5, 0.2, 2, V3::ZERO);
        assert_eq!(ball.a, ball.b);
    }

    #[test]
    fn slides_along_a_floor() {
        let w = floor();
        let c = player();
        let s = w.move_and_slide(&c, v(0.0, 1.0, 0.0), v(5.0, -3.0, 0.0));
        assert!(!s.contacts.is_empty());
        assert!(s.contacts[0].normal.y > 0.999);
        let b = bottom(&c, s.position);
        // The overbounce lifts it a little above SKIN.
        assert!((SKIN * 0.5..=2.0 * SKIN).contains(&b), "bottom {b}");
        assert!((s.position.x - 5.0).abs() < 1e-3, "x {}", s.position.x);
        assert_clear(&w, &c, s.position);
    }

    #[test]
    fn moving_away_from_a_touched_surface_is_free() {
        let w = floor();
        let c = player();
        let rest = w.move_and_slide(&c, v(0.0, 1.0, 0.0), v(0.0, -5.0, 0.0));
        let up = w.move_and_slide(&c, rest.position, v(0.0, 2.0, 0.0));
        assert!(up.contacts.is_empty());
        assert!((up.position.y - rest.position.y - 2.0).abs() < 1e-9);
        let along = w.move_and_slide(&c, rest.position, v(3.0, 0.0, 4.0));
        assert!(along.contacts.is_empty());
        assert!((along.position - rest.position - v(3.0, 0.0, 4.0)).length() < 1e-9);
    }

    #[test]
    fn slides_along_a_wall() {
        let w = world_of(
            quad(v(2.0, -50.0, -50.0), v(0.0, 100.0, 0.0), v(0.0, 0.0, 100.0)).to_vec(),
            vec![],
        );
        let c = player();
        let s = w.move_and_slide(&c, V3::ZERO, v(5.0, 0.0, 3.0));
        assert!(s.position.x <= 2.0 - c.radius - SKIN * 0.5 + 1e-9);
        assert!(s.position.x >= 2.0 - c.radius - 2.0 * SKIN);
        assert!((s.position.z - 3.0).abs() < 1e-3, "z {}", s.position.z);
        assert_clear(&w, &c, s.position);
    }

    #[test]
    fn stops_in_an_inside_corner() {
        let mut tris = quad(v(2.0, -50.0, -50.0), v(0.0, 100.0, 0.0), v(0.0, 0.0, 100.0)).to_vec();
        tris.extend(quad(
            v(-50.0, -50.0, 2.0),
            v(100.0, 0.0, 0.0),
            v(0.0, 100.0, 0.0),
        ));
        tris.extend(quad(
            v(-50.0, -3.0, -50.0),
            v(100.0, 0.0, 0.0),
            v(0.0, 0.0, 100.0),
        ));
        let w = world_of(tris, vec![]);
        let c = player();
        let mut pos = V3::ZERO;
        for _ in 0..50 {
            pos = w.move_and_slide(&c, pos, v(0.4, -0.3, 0.25)).position;
            assert_clear(&w, &c, pos);
        }
        let corner = 2.0 - c.radius;
        assert!(pos.x <= corner && pos.x > corner - SKIN - 1e-9, "{pos:?}");
        assert!(pos.z <= corner && pos.z > corner - SKIN - 1e-9, "{pos:?}");
    }

    #[test]
    fn a_thin_wall_stops_a_fast_move_from_both_sides() {
        let w = world_of(
            quad(v(0.0, -5.0, -5.0), v(0.0, 10.0, 0.0), v(0.0, 0.0, 10.0)).to_vec(),
            vec![],
        );
        let c = player();
        // 100 m/s at 60 steps per second is 1.7 m a step; one 100 m move too.
        for (from, delta) in [
            (v(-1.0, 0.0, 0.0), v(100.0, 0.0, 0.0)),
            (v(1.0, 0.0, 0.0), v(-100.0, 0.0, 0.0)),
        ] {
            let mut pos = from;
            for _ in 0..60 {
                pos = w.move_and_slide(&c, pos, delta * (1.0 / 60.0)).position;
            }
            assert!(pos.x.signum() == from.x.signum(), "{pos:?}");
            let one = w.move_and_slide(&c, from, delta);
            assert!(one.position.x.signum() == from.x.signum());
            assert_clear(&w, &c, one.position);
        }
    }

    #[test]
    fn primitives_stop_the_capsule_at_their_surface() {
        let c = Capsule::unity(0.3, 1.0, 1, V3::ZERO);
        let rot = std::f64::consts::FRAC_1_SQRT_2;
        // Expected stopping x of the capsule's axis for a move along +x.
        let cases = [
            (
                Shape::Sphere {
                    center: v(5.0, 0.0, 0.0),
                    radius: 1.0,
                },
                5.0 - 1.0 - 0.3,
            ),
            (
                Shape::Capsule {
                    a: v(5.0, 0.0, -3.0),
                    b: v(5.0, 0.0, 3.0),
                    radius: 0.5,
                },
                5.0 - 0.5 - 0.3,
            ),
            (
                Shape::Box {
                    center: v(5.0, 0.0, 0.0),
                    axes: [v(1.0, 0.0, 0.0), V3::Y, v(0.0, 0.0, 1.0)],
                    half: [1.0, 1.0, 1.0],
                },
                5.0 - 1.0 - 0.3,
            ),
            // Turned 45° about y: its edge points at the capsule.
            (
                Shape::Box {
                    center: v(5.0, 0.0, 0.0),
                    axes: [v(rot, 0.0, rot), V3::Y, v(-rot, 0.0, rot)],
                    half: [1.0, 1.0, 1.0],
                },
                5.0 - std::f64::consts::SQRT_2 - 0.3,
            ),
        ];
        for (shape, stop) in cases {
            let w = world_of(vec![], vec![shape]);
            let s = w.move_and_slide(&c, V3::ZERO, v(10.0, 0.0, 0.0));
            assert!(
                s.position.x <= stop - SKIN * 0.5 + 1e-9 && s.position.x >= stop - SKIN - 1e-9,
                "{shape:?}: stopped at {}",
                s.position.x
            );
        }
    }

    #[test]
    fn inside_a_box_is_a_penetration_and_push_out_leaves() {
        let w = world_of(
            vec![],
            vec![Shape::Box {
                center: V3::ZERO,
                axes: [v(1.0, 0.0, 0.0), V3::Y, v(0.0, 0.0, 1.0)],
                half: [1.0, 1.0, 1.0],
            }],
        );
        let c = Capsule::unity(0.3, 0.6, 1, V3::ZERO);
        assert!(w.clearance(&c, V3::ZERO, 1.0).unwrap().gap < 0.0);
        // Slightly into the top face: pushed out upwards.
        let (pos, first) = w.push_out(&c, v(0.0, 1.25, 0.0));
        assert!(first.unwrap() < 0.0);
        assert!(pos.y > 1.3 && pos.y < 1.3 + SKIN + 1e-9, "{pos:?}");
    }

    #[test]
    fn slides_over_the_seams_of_a_bumpy_floor() {
        // A 40 × 40 m height field, 1 m cells, gentle waves: terrain-like.
        let h = |x: f64, z: f64| 0.4 * (x * 0.7).sin() + 0.3 * (z * 0.9).cos();
        let mut tris = Vec::new();
        for i in -20..20 {
            for k in -20..20 {
                let (x, z) = (f64::from(i), f64::from(k));
                let p = |x: f64, z: f64| v(x, h(x, z), z);
                let (a, b, cc, d) = (p(x, z), p(x + 1.0, z), p(x + 1.0, z + 1.0), p(x, z + 1.0));
                tris.push([f(a), f(b), f(cc)]);
                tris.push([f(a), f(cc), f(d)]);
            }
        }
        let w = world_of(tris, vec![]);
        let c = player();
        let mut pos = v(-15.0, 2.0, 0.3);
        let step = v(7.6 / 60.0, -0.05, 0.0);
        let steps = 200;
        let mut contacts = 0;
        for _ in 0..steps {
            let s = w.move_and_slide(&c, pos, step);
            contacts += s.contacts.len();
            pos = s.position;
            assert_clear(&w, &c, pos);
        }
        let asked = step.x * f64::from(steps);
        let travelled = pos.x + 15.0;
        assert!(contacts > 0);
        assert!(travelled > 0.9 * asked, "{travelled} of {asked}");
    }

    #[test]
    fn a_random_walk_in_a_closed_room_never_penetrates() {
        // A 10 m cube seen from inside, with obstacles.
        let s = 10.0;
        let mut tris = Vec::new();
        let (x, y, z) = (v(s, 0.0, 0.0), v(0.0, s, 0.0), v(0.0, 0.0, s));
        let o = v(-5.0, -5.0, -5.0);
        tris.extend(quad(o, x, z));
        tris.extend(quad(o + y, x, z));
        tris.extend(quad(o, y, z));
        tris.extend(quad(o + x, y, z));
        tris.extend(quad(o, x, y));
        tris.extend(quad(o + z, x, y));
        // A tilted ramp.
        tris.push([[-4.0, -5.0, -4.0], [4.0, -5.0, -4.0], [0.0, 0.0, 4.0]]);
        let rot = std::f64::consts::FRAC_1_SQRT_2;
        let shapes = vec![
            Shape::Sphere {
                center: v(2.0, 1.0, -2.0),
                radius: 1.2,
            },
            Shape::Capsule {
                a: v(-3.0, -4.0, 3.0),
                b: v(-1.0, 2.0, 2.0),
                radius: 0.6,
            },
            Shape::Box {
                center: v(-2.0, 2.0, -2.0),
                axes: [v(rot, rot, 0.0), v(-rot, rot, 0.0), v(0.0, 0.0, 1.0)],
                half: [1.0, 0.5, 1.5],
            },
        ];
        let w = world_of(tris, shapes);
        let c = player();
        let mut seed: u64 = 12345;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        let (mut pos, _) = w.push_out(&c, v(3.0, -3.0, 3.0));
        let mut contacts = 0;
        for _ in 0..3000 {
            let d = v(rnd(), rnd(), rnd()) * 3.0;
            let s = w.move_and_slide(&c, pos, d);
            contacts += s.contacts.len();
            pos = s.position;
            assert_clear(&w, &c, pos);
            assert!(
                pos.x.abs() < 5.0 && pos.y.abs() < 5.5 && pos.z.abs() < 5.0,
                "{pos:?}"
            );
        }
        assert!(contacts > 1000, "{contacts}");
    }

    #[test]
    fn bad_shapes_are_skipped() {
        let body = Body::new(
            vec![
                [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
                [[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ],
            vec![Shape::Sphere {
                center: V3::ZERO,
                radius: f64::NAN,
            }],
        );
        assert_eq!(
            (body.triangle_count(), body.shape_count(), body.skipped()),
            (1, 0, 3)
        );
        // An empty world: nothing to hit.
        let w = World::new();
        assert!(w.sweep(&player(), V3::ZERO, v(1.0, 0.0, 0.0)).is_none());
        assert!(w.clearance(&player(), V3::ZERO, 10.0).is_none());
    }

    #[test]
    fn bodies_come_and_go() {
        let mut w = floor();
        let c = player();
        assert!(w.sweep(&c, v(0.0, 1.0, 0.0), v(0.0, -5.0, 0.0)).is_some());
        assert!(w.remove(1).is_some());
        assert!(w.sweep(&c, v(0.0, 1.0, 0.0), v(0.0, -5.0, 0.0)).is_none());
    }
}
