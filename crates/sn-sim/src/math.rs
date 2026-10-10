//! The small amount of vector maths the rules need.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub const ZERO: V3 = V3::new(0.0, 0.0, 0.0);
    pub const Y: V3 = V3::new(0.0, 1.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> V3 {
        V3 { x, y, z }
    }

    pub fn from_f32(p: [f32; 3]) -> V3 {
        V3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }

    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// `None` for (nearly) zero vectors.
    pub fn normalized(self) -> Option<V3> {
        let l = self.length();
        (l > 1e-12).then(|| self * (1.0 / l))
    }

    pub fn min(self, o: V3) -> V3 {
        V3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: V3) -> V3 {
        V3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn get(self, axis: usize) -> f64 {
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl SubAssign for V3 {
    fn sub_assign(&mut self, o: V3) {
        *self = *self - o;
    }
}

impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}

impl V3 {
    /// `Vector3.Lerp` (`t` clamped to 0…1).
    pub fn lerp(a: V3, b: V3, t: f64) -> V3 {
        let t = t.clamp(0.0, 1.0);
        a + (b - a) * t
    }
}

/// `Mathf.LerpAngle` (degrees, `t` clamped to 0…1): the short way round.
pub fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    let mut d = (b - a).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    a + d * t.clamp(0.0, 1.0)
}

/// A rotation: Unity's quaternion (x, y, z, w; left-handed, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Q {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Default for Q {
    fn default() -> Q {
        Q::IDENTITY
    }
}

impl Q {
    pub const IDENTITY: Q = Q {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn from_f32(q: [f32; 4]) -> Q {
        Q {
            x: f64::from(q[0]),
            y: f64::from(q[1]),
            z: f64::from(q[2]),
            w: f64::from(q[3]),
        }
    }

    pub fn to_f32(self) -> [f32; 4] {
        [self.x as f32, self.y as f32, self.z as f32, self.w as f32]
    }

    fn axis(axis: V3, deg: f64) -> Q {
        let (s, c) = (deg.to_radians() * 0.5).sin_cos();
        Q {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: c,
        }
    }

    /// `Quaternion.Euler(x, y, z)`, degrees: z first, then x, then y.
    pub fn euler(x: f64, y: f64, z: f64) -> Q {
        Q::axis(V3::new(0.0, 1.0, 0.0), y)
            * Q::axis(V3::new(1.0, 0.0, 0.0), x)
            * Q::axis(V3::new(0.0, 0.0, 1.0), z)
    }

    /// `Quaternion.eulerAngles`: the (x, y, z) of [`Q::euler`], each in
    /// 0…360.
    pub fn to_euler(self) -> V3 {
        let Q { x, y, z, w } = self.normalized();
        let sx = (2.0 * (w * x - y * z)).clamp(-1.0, 1.0);
        let ex = sx.asin();
        let (ey, ez) = if sx.abs() > 0.999_999 {
            // Looking straight up or down: all the turn on y.
            (
                (2.0 * (w * y - x * z)).atan2(1.0 - 2.0 * (y * y + z * z)),
                0.0,
            )
        } else {
            (
                (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (x * x + y * y)),
                (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (x * x + z * z)),
            )
        };
        let deg = |r: f64| r.to_degrees().rem_euclid(360.0);
        V3::new(deg(ex), deg(ey), deg(ez))
    }

    fn product(self, o: Q) -> Q {
        Q {
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        }
    }

    pub fn inverse(self) -> Q {
        let n = self.dot(self).max(1e-300);
        Q {
            x: -self.x / n,
            y: -self.y / n,
            z: -self.z / n,
            w: self.w / n,
        }
    }

    pub fn dot(self, o: Q) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    pub fn normalized(self) -> Q {
        let n = self.dot(self).sqrt();
        if n < 1e-12 {
            return Q::IDENTITY;
        }
        Q {
            x: self.x / n,
            y: self.y / n,
            z: self.z / n,
            w: self.w / n,
        }
    }

    /// `v` rotated.
    pub fn rotate(self, v: V3) -> V3 {
        let u = V3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// `Quaternion.Slerp` (`t` clamped to 0…1; the short way round,
    /// **hypothesis** for Unity's native code).
    pub fn slerp(a: Q, b: Q, t: f64) -> Q {
        let t = t.clamp(0.0, 1.0);
        let mut b = b;
        let mut d = a.dot(b);
        if d < 0.0 {
            b = Q {
                x: -b.x,
                y: -b.y,
                z: -b.z,
                w: -b.w,
            };
            d = -d;
        }
        let (ka, kb) = if d > 0.9995 {
            (1.0 - t, t)
        } else {
            let theta = d.clamp(-1.0, 1.0).acos();
            let s = theta.sin();
            (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s)
        };
        Q {
            x: a.x * ka + b.x * kb,
            y: a.y * ka + b.y * kb,
            z: a.z * ka + b.z * kb,
            w: a.w * ka + b.w * kb,
        }
        .normalized()
    }
}

/// `a * b`: `b` first, then `a`.
impl Mul for Q {
    type Output = Q;
    fn mul(self, o: Q) -> Q {
        self.product(o)
    }
}

/// A placement without scale: where and how turned.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub position: V3,
    pub rotation: Q,
}

impl Pose {
    pub fn new(position: V3, rotation: Q) -> Pose {
        Pose { position, rotation }
    }

    /// `child` (in this pose's space) in the parent's space.
    pub fn then(&self, child: &Pose) -> Pose {
        Pose {
            position: self.position + self.rotation.rotate(child.position),
            rotation: self.rotation * child.rotation,
        }
    }

    /// `Transform.InverseTransformPoint` (no scale).
    pub fn inverse_transform_point(&self, p: V3) -> V3 {
        self.rotation.inverse().rotate(p - self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn euler_applies_z_then_x_then_y() {
        let q = Q::euler(30.0, 40.0, 50.0);
        let v = V3::new(0.3, -0.2, 1.0);
        let step = Q::euler(0.0, 40.0, 0.0)
            .rotate(Q::euler(30.0, 0.0, 0.0).rotate(Q::euler(0.0, 0.0, 50.0).rotate(v)));
        assert!(close(q.rotate(v), step));
        // y: +z turns to +x (left-handed, y up): the yaw.
        assert!(close(
            Q::euler(0.0, 90.0, 0.0).rotate(V3::new(0.0, 0.0, 1.0)),
            V3::new(1.0, 0.0, 0.0)
        ));
        // x: +z turns down (a positive pitch looks down).
        assert!(Q::euler(30.0, 0.0, 0.0).rotate(V3::new(0.0, 0.0, 1.0)).y < 0.0);
    }

    #[test]
    fn euler_angles_come_back() {
        for (x, y, z) in [
            (10.0, 20.0, 30.0),
            (-40.0, 200.0, 5.0),
            (0.0, 359.0, 0.0),
            (80.0, -100.0, -60.0),
        ] {
            let e = Q::euler(x, y, z).to_euler();
            let back = Q::euler(e.x, e.y, e.z);
            let q = Q::euler(x, y, z);
            assert!(q.dot(back).abs() > 1.0 - 1e-9, "{x} {y} {z} gave {e:?}");
            assert!((0.0..360.0).contains(&e.y));
        }
        let e = Q::euler(-40.0, 200.0, 5.0).to_euler();
        assert!((e.x - 320.0).abs() < 1e-9 && (e.y - 200.0).abs() < 1e-9);
    }

    #[test]
    fn slerp_halfway_and_short_way() {
        let a = Q::IDENTITY;
        let b = Q::euler(0.0, 90.0, 0.0);
        let h = Q::slerp(a, b, 0.5);
        assert!((h.to_euler().y - 45.0).abs() < 1e-9);
        // The same rotation written with the opposite sign: still 45 degrees.
        let neg = Q {
            x: -b.x,
            y: -b.y,
            z: -b.z,
            w: -b.w,
        };
        assert!((Q::slerp(a, neg, 0.5).to_euler().y - 45.0).abs() < 1e-9);
        assert!(Q::slerp(a, b, 2.0).dot(b) > 1.0 - 1e-12);
    }

    #[test]
    fn poses_compose_and_invert() {
        let parent = Pose::new(V3::new(1.0, 2.0, 3.0), Q::euler(0.0, 90.0, 0.0));
        let child = Pose::new(V3::new(0.0, 0.0, 2.0), Q::IDENTITY);
        let w = parent.then(&child);
        assert!(close(w.position, V3::new(3.0, 2.0, 3.0)));
        assert!(close(
            parent.inverse_transform_point(w.position),
            child.position
        ));
        assert!((lerp_angle(350.0, 10.0, 0.5) - 360.0).abs() < 1e-9);
        assert!((lerp_angle(10.0, 350.0, 2.0) + 10.0).abs() < 1e-9);
    }
}
