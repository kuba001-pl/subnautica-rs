//! `uGUI_Compass` (M9u6c2; `docs/formats/ui.md` § The depth and compass),
//! the game's own `Graphic` that draws the HUD's compass ring, ported from
//! the decompiled script (read only; the code is ours):
//! - [`Compass::mesh`]: `OnPopulateMesh`'s quad, put in a small 3D scene
//!   (`TRS(position, rotation, scale)`, `Matrix4x4.Perspective(fov, 1,
//!   near, far)`) and flattened by `scale2D × x / z`; its UVs divided by z
//!   with 1/z in `uv1.x`, so `uGUI/Compass` interpolates them in
//!   perspective;
//! - [`Compass::angle_values`]: `UpdateAngle`'s material values;
//! - [`Compass::label`]: `UpdateLabels`' place and alpha of each label.
//!
//! Unity's math is reproduced in `f32`: `Quaternion.Euler` turns about z,
//! then x, then y; matrices are row-major and act on column vectors;
//! `MultiplyPoint3x4` drops the last row (no divide).

use crate::mesh::{MeshData, Vertex, color32};
use crate::state::MaterialValues;

/// A 4×4 matrix, rows first.
pub type Mat4 = [[f32; 4]; 4];

/// `uGUI_Compass.Label`: the text's node, the name and the angle.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub node: usize,
    pub name: String,
    pub angle: f32,
}

/// The script's serialized fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Compass {
    /// `Graphic.color`.
    pub color: [f32; 4],
    pub size: f32,
    pub position: [f32; 3],
    /// Euler angles, degrees.
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub scale_2d: f32,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub radius: f32,
    pub alpha_from: f32,
    pub alpha_to: f32,
    pub labels: Vec<Label>,
}

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [[0.0; 4]; 4];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

/// `Matrix4x4.MultiplyPoint3x4`.
pub fn multiply_point_3x4(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    let row = |r: usize| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
    [row(0), row(1), row(2)]
}

/// `Quaternion.Euler(x, y, z)` (degrees) as (x, y, z, w): y × x × z.
pub fn euler(e: [f32; 3]) -> [f32; 4] {
    let half = |d: f32| (d.to_radians() / 2.0).sin_cos();
    let (sx, cx) = half(e[0]);
    let (sy, cy) = half(e[1]);
    let (sz, cz) = half(e[2]);
    [
        cy * sx * cz + sy * cx * sz,
        sy * cx * cz - cy * sx * sz,
        cy * cx * sz - sy * sx * cz,
        cy * cx * cz + sy * sx * sz,
    ]
}

/// `Matrix4x4.TRS`.
pub fn trs(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> Mat4 {
    let [x, y, z, w] = q;
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let mut m = [
        [0.0, 0.0, 0.0, 0.0],
        [0.0; 4],
        [0.0; 4],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = r[i][j] * s[j];
        }
        m[i][3] = t[i];
    }
    m
}

/// `Matrix4x4.Perspective(fov, aspect, near, far)` (OpenGL's).
pub fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let cot = 1.0 / (fov.to_radians() / 2.0).tan();
    let dz = near - far;
    [
        [cot / aspect, 0.0, 0.0, 0.0],
        [0.0, cot, 0.0, 0.0],
        [0.0, 0.0, (far + near) / dz, 2.0 * near * far / dz],
        [0.0, 0.0, -1.0, 0.0],
    ]
}

/// The `direction` setter's wrap: `value − floor(value)`.
pub fn wrap(direction: f32) -> f32 {
    direction - direction.floor()
}

impl Compass {
    fn projection(&self) -> Mat4 {
        perspective(self.fov, 1.0, self.near, self.far)
    }

    /// `projection × TRS(position, Euler(rotation.x, rotation.y,
    /// rotation.z − 360 × direction), scale)`: `_matrixLocal` after
    /// `UpdateAngle` (direction 0: the mesh's matrix).
    pub fn matrix(&self, direction: f32) -> Mat4 {
        let [x, y, z] = self.rotation;
        let model = trs(
            self.position,
            euler([x, y, z - direction * 360.0]),
            self.scale,
        );
        mul(&self.projection(), &model)
    }

    /// A point of the compass's plane on the canvas: through `m`, then
    /// `scale2D × x / z`; also z.
    fn flatten(&self, m: &Mat4, p: [f32; 3]) -> ([f32; 2], f32) {
        let v = multiply_point_3x4(m, p);
        let z = v[2];
        ([self.scale_2d * v[0] / z, self.scale_2d * v[1] / z], z)
    }

    /// `OnPopulateMesh` while visible: one quad, in the graphic's local
    /// space (the direction turns the texture in the shader, not the
    /// mesh).
    pub fn mesh(&self) -> MeshData {
        let s = self.size;
        let corners = [[-s, -s], [-s, s], [s, s], [s, -s]];
        let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let m = self.matrix(0.0);
        let color = color32(self.color);
        let mut out = MeshData::default();
        for (c, uv) in corners.iter().zip(uvs) {
            let (pos, z) = self.flatten(&m, [c[0], c[1], 0.0]);
            out.vertices.push(Vertex {
                pos,
                color,
                uv: [uv[0] / z, uv[1] / z],
                uv1: [1.0 / z, 0.0],
            });
        }
        out.indices = vec![0, 1, 2, 2, 3, 0];
        out
    }

    /// `UpdateAngle`'s material values: `_Angle` (radians), `_AlphaFrom`,
    /// `_AlphaTo`.
    pub fn angle_values(&self, direction: f32) -> MaterialValues {
        let mut m = MaterialValues::default();
        let angle = (self.rotation[2] - direction * 360.0) * (std::f32::consts::PI / 180.0);
        m.floats.insert("_Angle".into(), angle);
        m.floats.insert("_AlphaFrom".into(), self.alpha_from);
        m.floats.insert("_AlphaTo".into(), self.alpha_to);
        m
    }

    /// `UpdateLabels` for one label: its `anchoredPosition` (the labels are
    /// children of the compass, so the script's trip through world space
    /// is the identity) and its colour's alpha (the colour is white).
    pub fn label(&self, angle: f32, direction: f32) -> ([f32; 2], f32) {
        let f = (std::f32::consts::PI / 180.0) * (self.rotation[2] + angle);
        let p = [self.radius * f.cos(), self.radius * f.sin(), 0.0];
        let (pos, _) = self.flatten(&self.matrix(direction), p);
        let n = (self.rotation[2] + angle - 90.0 - direction * 360.0) / 360.0;
        let t =
            ((n - n.floor() - self.alpha_from) / (self.alpha_to - self.alpha_from)).clamp(0.0, 1.0);
        let t = 1.0 - (2.0 * t - 1.0).abs();
        (pos, t.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compass() -> Compass {
        Compass {
            color: [1.0; 4],
            size: 3.9,
            position: [0.0, -0.54, -12.6],
            rotation: [-70.8, 0.0, 0.0],
            scale: [1.0; 3],
            scale_2d: 342.0,
            fov: 90.0,
            near: 0.0,
            far: 10.0,
            radius: 4.5,
            alpha_from: 0.375,
            alpha_to: 0.625,
            labels: Vec::new(),
        }
    }

    #[test]
    fn euler_turns_about_z_then_x_then_y() {
        // 90° about y takes +z to +x; 90° about x takes +y to +z.
        let m = trs([0.0; 3], euler([0.0, 90.0, 0.0]), [1.0; 3]);
        let p = multiply_point_3x4(&m, [0.0, 0.0, 1.0]);
        assert!((p[0] - 1.0).abs() < 1e-6 && p[2].abs() < 1e-6, "{p:?}");
        let m = trs([0.0; 3], euler([90.0, 0.0, 0.0]), [1.0; 3]);
        let p = multiply_point_3x4(&m, [0.0, 1.0, 0.0]);
        assert!((p[2] - 1.0).abs() < 1e-6, "{p:?}");
        // Both: z first (x → y), then x (y → z), then y (z → x).
        let m = trs([0.0; 3], euler([90.0, 90.0, 90.0]), [1.0; 3]);
        let p = multiply_point_3x4(&m, [1.0, 0.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn the_projection_with_near_zero_gives_the_distance_as_z() {
        let m = perspective(90.0, 1.0, 0.0, 10.0);
        let p = multiply_point_3x4(&m, [2.0, 1.0, -8.0]);
        assert!((p[0] - 2.0).abs() < 1e-6 && (p[1] - 1.0).abs() < 1e-6);
        assert!((p[2] - 8.0).abs() < 1e-6);
    }

    #[test]
    fn the_quad_is_a_trapezoid_wider_at_the_bottom() {
        // Tilted back by 70.8°: the near (bottom) edge is wider.
        let m = compass().mesh();
        assert_eq!(m.vertices.len(), 4);
        let width = |a: usize, b: usize| (m.vertices[a].pos[0] - m.vertices[b].pos[0]).abs();
        assert!(width(0, 3) > width(1, 2));
        // Symmetric about x = 0; UV ÷ z, 1/z kept.
        assert!((m.vertices[0].pos[0] + m.vertices[3].pos[0]).abs() < 1e-3);
        for v in &m.vertices {
            assert!(v.uv1[0] > 0.0);
            let uv = [v.uv[0] / v.uv1[0], v.uv[1] / v.uv1[0]];
            assert!(
                uv.iter()
                    .all(|c| (c - 0.0).abs() < 1e-5 || (c - 1.0).abs() < 1e-5)
            );
        }
    }

    #[test]
    fn a_label_is_brightest_at_the_front() {
        // At direction 0, the label at angle 270 (N) is at the near middle
        // (−90° of the ring, the bottom), t = frac((270 − 90) / 360) = 0.5:
        // alpha sin(1).
        let c = compass();
        let (pos, alpha) = c.label(270.0, 0.0);
        assert!(pos[0].abs() < 1e-3);
        assert!((alpha - 1f32.sin()).abs() < 1e-6);
        // Behind (angle 90): t clamps to 0, alpha 0.
        let (_, alpha) = c.label(90.0, 0.0);
        assert_eq!(alpha, 0.0);
        // A quarter turn moves N a quarter of the ring.
        let (pos, alpha) = c.label(270.0, 0.25);
        assert!(pos[0].abs() > 10.0);
        assert_eq!(alpha, 0.0);
    }

    #[test]
    fn the_angle_values_follow_the_direction() {
        let m = compass().angle_values(0.25);
        assert!((m.floats["_Angle"] + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(m.floats["_AlphaFrom"], 0.375);
        assert_eq!(wrap(1.25), 0.25);
        assert_eq!(wrap(-0.25), 0.75);
    }
}
