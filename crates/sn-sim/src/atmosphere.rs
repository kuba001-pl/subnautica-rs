//! Point queries for streamed atmosphere colliders. Unity world coordinates.
use crate::{V3, collide::Shape};

#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub shape: Shape,
    pub priority: i32,
    /// Local lifetime identity, not Unity's runtime instance ID.
    pub tie: u64,
    pub biome: String,
}

/// Inclusive primitive boundaries, as a zero-radius overlap query requires.
pub fn contains(shape: &Shape, p: V3) -> bool {
    if !p.is_finite() {
        return false;
    }
    match *shape {
        Shape::Box { center, axes, half } => {
            let delta = p - center;
            center.is_finite()
                && (0..3).all(|i| {
                    half[i].is_finite()
                        && half[i] >= 0.0
                        && axes[i].is_finite()
                        && delta.dot(axes[i]).abs() <= half[i]
                })
        }
        Shape::Sphere { center, radius } => {
            center.is_finite()
                && radius.is_finite()
                && radius >= 0.0
                && (p - center).dot(p - center) <= radius * radius
        }
        Shape::Capsule { a, b, radius } => {
            if !a.is_finite() || !b.is_finite() || !radius.is_finite() || radius < 0.0 {
                return false;
            }
            let ab = b - a;
            let length2 = ab.dot(ab);
            let t = if length2 > 0.0 {
                ((p - a).dot(ab) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let delta = p - (a + ab * t);
            delta.dot(delta) <= radius * radius
        }
        Shape::Triangle(_) => false,
    }
}

/// Highest priority then local tie key. Empty biome overrides are retained:
/// the native point query does not fall through to the map in that case.
pub fn enclosing<'a>(volumes: impl IntoIterator<Item = &'a Volume>, p: V3) -> Option<&'a Volume> {
    volumes
        .into_iter()
        .filter(|v| contains(&v.shape, p))
        .max_by_key(|v| (v.priority as u32, v.tie))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(priority: i32, tie: u64, biome: &str) -> Volume {
        Volume {
            shape: Shape::Sphere {
                center: V3::ZERO,
                radius: 2.0,
            },
            priority,
            tie,
            biome: biome.into(),
        }
    }

    #[test]
    fn primitive_boundaries_and_invalid_queries() {
        let box_shape = Shape::Box {
            center: V3::new(3.0, 0.0, 0.0),
            axes: [V3::Y, V3::new(-1.0, 0.0, 0.0), V3::new(0.0, 0.0, 1.0)],
            half: [1.0, 2.0, 3.0],
        };
        assert!(contains(&box_shape, V3::new(5.0, 1.0, 3.0)));
        assert!(!contains(&box_shape, V3::new(5.001, 1.0, 3.0)));
        let s = sphere(0, 0, "a");
        assert!(contains(&s.shape, V3::new(2.0, 0.0, 0.0)));
        assert!(!contains(&s.shape, V3::new(2.001, 0.0, 0.0)));
        assert!(!contains(&s.shape, V3::new(f64::NAN, 0.0, 0.0)));
        let capsule = Shape::Capsule {
            a: V3::ZERO,
            b: V3::Y * 4.0,
            radius: 1.0,
        };
        assert!(contains(&capsule, V3::new(1.0, 2.0, 0.0)));
        assert!(contains(&capsule, V3::Y * 5.0));
        assert!(!contains(&capsule, V3::Y * 5.001));
        assert!(contains(
            &Shape::Capsule {
                a: V3::ZERO,
                b: V3::ZERO,
                radius: 1.0
            },
            V3::Y
        ));
    }

    #[test]
    fn priority_ties_empty_override_and_unload() {
        let a = sphere(5, 99, "outer");
        let b = sphere(10, 1, "inner");
        let c = sphere(10, 2, "");
        assert_eq!(enclosing([&a, &b, &c], V3::ZERO), Some(&c));
        assert_eq!(enclosing([&c, &a, &b], V3::ZERO), Some(&c));
        assert_eq!(enclosing([&a, &b], V3::ZERO), Some(&b));
        assert_eq!(enclosing([&a], V3::ZERO), Some(&a));
        assert!(enclosing([&a, &b], V3::Y * 3.0).is_none());
        assert!(enclosing([], V3::ZERO).is_none());
    }
}
