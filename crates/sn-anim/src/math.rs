//! Quaternions as Unity stores them: `[x, y, z, w]`.

pub type Quat = [f32; 4];

pub const IDENTITY: Quat = [0.0, 0.0, 0.0, 1.0];

pub fn dot(a: Quat, b: Quat) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// Unit length; the identity for a zero (or non-finite) quaternion.
pub fn normalize(q: Quat) -> Quat {
    let len = dot(q, q).sqrt();
    if len > 1e-12 && len.is_finite() {
        q.map(|c| c / len)
    } else {
        IDENTITY
    }
}

/// `a · b`: rotate by `b`, then by `a`.
pub fn mul(a: Quat, b: Quat) -> Quat {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

pub fn conjugate(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Normalised linear blend from `a` to `b` on the shorter arc.
pub fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let s = if dot(a, b) < 0.0 { -1.0 } else { 1.0 };
    normalize([0, 1, 2, 3].map(|i| a[i] + (s * b[i] - a[i]) * t))
}

/// Unity's `Quaternion.Euler(x, y, z)` (degrees): rotate about z, then x,
/// then y.
pub fn euler(deg: [f32; 3]) -> Quat {
    let half = |d: f32| (d.to_radians() * 0.5).sin_cos();
    let (sx, cx) = half(deg[0]);
    let (sy, cy) = half(deg[1]);
    let (sz, cz) = half(deg[2]);
    let qx = [sx, 0.0, 0.0, cx];
    let qy = [0.0, sy, 0.0, cy];
    let qz = [0.0, 0.0, sz, cz];
    mul(qy, mul(qx, qz))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rotate(q: Quat, v: [f32; 3]) -> [f32; 3] {
        let p = mul(mul(q, [v[0], v[1], v[2], 0.0]), conjugate(q));
        [p[0], p[1], p[2]]
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn euler_is_z_then_x_then_y() {
        // 90° about y takes +z to +x (Unity, left-handed).
        assert!(close(
            rotate(euler([0.0, 90.0, 0.0]), [0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0]
        ));
        // z first: (90 about z) takes +x to +y; then 90 about x takes +y to +z.
        assert!(close(
            rotate(euler([90.0, 0.0, 90.0]), [1.0, 0.0, 0.0]),
            [0.0, 0.0, 1.0]
        ));
    }

    #[test]
    fn nlerp_takes_the_short_way() {
        let a = euler([0.0, 10.0, 0.0]);
        let b = euler([0.0, 30.0, 0.0]).map(|c| -c);
        let m = nlerp(a, b, 0.5);
        assert!(close(
            rotate(m, [0.0, 0.0, 1.0]),
            rotate(euler([0.0, 20.0, 0.0]), [0.0, 0.0, 1.0])
        ));
    }
}
