//! Skinned meshes in a still pose: each vertex moved by its bones as Unity
//! does on the GPU, `Σ wᵢ · boneᵢ · bindPoseᵢ · v`, with the bones where the
//! hierarchy stores them (an `Animator` would move them; not done here).

use sn_unity::MeshGeometry;
use sn_world::Transform;

/// A 4×4 matrix, column by column (`m[col * 4 + row]`), as Unity stores
/// `Matrix4x4`.
pub type Mat4 = [f32; 16];

/// `T · R · S` of a transform.
pub fn trs(t: &Transform) -> Mat4 {
    let [x, y, z, w] = t.rotation;
    let [sx, sy, sz] = t.scale;
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let s = [sx, sy, sz];
    let mut m = [0.0; 16];
    for col in 0..3 {
        for row in 0..3 {
            m[col * 4 + row] = r[col][row] * s[col];
        }
    }
    m[12] = t.position[0];
    m[13] = t.position[1];
    m[14] = t.position[2];
    m[15] = 1.0;
    m
}

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    m
}

fn point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

fn vector(m: &Mat4, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2])
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 { v.map(|c| c / len) } else { v }
}

/// Moves a mesh's vertices by its skin. `bones[i]` is bone `i`'s placement
/// (`None`: missing) in the space the result should be in; `bind_poses`
/// from the mesh. Weights of missing or out-of-range bones are left out
/// and the rest renormalised; a vertex with none keeps its position.
/// Returns `None` if the mesh has no skin.
pub fn skin(
    geometry: &MeshGeometry,
    bind_poses: &[Mat4],
    bones: &[Option<Transform>],
) -> Option<MeshGeometry> {
    let n = geometry.positions.len();
    if geometry.bone_weights.len() != n || geometry.bone_indices.len() != n || bones.is_empty() {
        return None;
    }
    let matrices: Vec<Option<Mat4>> = bones
        .iter()
        .enumerate()
        .map(|(i, b)| Some(mul(&trs(b.as_ref()?), bind_poses.get(i)?)))
        .collect();
    let mut out = geometry.clone();
    for v in 0..n {
        let mut m = [0.0; 16];
        let mut total = 0.0;
        for (&bone, &w) in geometry.bone_indices[v]
            .iter()
            .zip(&geometry.bone_weights[v])
        {
            if w <= 0.0 {
                continue;
            }
            let Some(Some(b)) = matrices.get(bone as usize) else {
                continue;
            };
            for (a, x) in m.iter_mut().zip(b) {
                *a += w * x;
            }
            total += w;
        }
        if total <= 0.0 {
            continue;
        }
        let m = m.map(|x| x / total);
        out.positions[v] = point(&m, geometry.positions[v]);
        if let Some(nrm) = geometry.normals.get(v) {
            out.normals[v] = normalize(vector(&m, *nrm));
        }
        if let Some(t) = geometry.tangents.get(v) {
            let [x, y, z] = normalize(vector(&m, [t[0], t[1], t[2]]));
            out.tangents[v] = [x, y, z, t[3]];
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    fn inverse_rigid(t: &Transform) -> Mat4 {
        // For rotation + translation (scale 1): Rᵀ, −Rᵀ·p.
        let m = trs(t);
        let mut inv = [0.0; 16];
        for c in 0..3 {
            for r in 0..3 {
                inv[c * 4 + r] = m[r * 4 + c];
            }
        }
        let p = vector(&inv, t.position);
        inv[12] = -p[0];
        inv[13] = -p[1];
        inv[14] = -p[2];
        inv[15] = 1.0;
        inv
    }

    fn geometry(points: &[[f32; 3]], weights: &[[f32; 4]], indices: &[[u32; 4]]) -> MeshGeometry {
        MeshGeometry {
            positions: points.to_vec(),
            normals: vec![[0.0, 1.0, 0.0]; points.len()],
            bone_weights: weights.to_vec(),
            bone_indices: indices.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn trs_matches_transform_then() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let t = Transform {
            position: [1.0, 2.0, 3.0],
            rotation: [0.0, s, 0.0, s],
            scale: [2.0, 1.0, 0.5],
        };
        let p = [0.3, -0.7, 1.1];
        let expected = t
            .then(&Transform {
                position: p,
                ..Default::default()
            })
            .position;
        assert!(close(point(&trs(&t), p), expected));
    }

    #[test]
    fn bones_at_their_bind_pose_leave_the_mesh_alone() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let bone = Transform {
            position: [0.0, 1.0, 0.0],
            rotation: [s, 0.0, 0.0, s],
            scale: [1.0; 3],
        };
        let g = geometry(
            &[[1.0, 2.0, 3.0], [-1.0, 0.5, 0.0]],
            &[[1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
            &[[0, 0, 0, 0], [0, 1, 0, 0]],
        );
        let binds = [inverse_rigid(&bone), inverse_rigid(&Transform::default())];
        let out = skin(&g, &binds, &[Some(bone), Some(Transform::default())]).unwrap();
        for (a, b) in out.positions.iter().zip(&g.positions) {
            assert!(close(*a, *b), "{a:?} vs {b:?}");
        }
        assert!(close(out.normals[0], [0.0, 1.0, 0.0]));
    }

    #[test]
    fn moved_bones_carry_their_vertices() {
        // Two bones bound at the origin; bone 1 then moves up by 2.
        let identity = trs(&Transform::default());
        let up = Transform {
            position: [0.0, 2.0, 0.0],
            ..Default::default()
        };
        let g = geometry(
            &[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            &[
                [1.0, 0.0, 0.0, 0.0],
                [0.5, 0.5, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
            ],
            &[[0, 0, 0, 0], [0, 1, 0, 0], [0, 1, 0, 0]],
        );
        let out = skin(
            &g,
            &[identity, identity],
            &[Some(Transform::default()), Some(up)],
        )
        .unwrap();
        assert!(close(out.positions[0], [1.0, 0.0, 0.0]));
        assert!(close(out.positions[1], [2.0, 1.0, 0.0]));
        assert!(close(out.positions[2], [3.0, 2.0, 0.0]));
    }

    #[test]
    fn missing_bones_are_left_out() {
        let identity = trs(&Transform::default());
        let up = Transform {
            position: [0.0, 2.0, 0.0],
            ..Default::default()
        };
        let g = geometry(
            &[[1.0, 0.0, 0.0], [5.0, 0.0, 0.0]],
            &[[0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
            // Bone 1 missing; bone 7 out of range.
            &[[0, 1, 0, 0], [7, 0, 0, 0]],
        );
        let out = skin(&g, &[identity, identity], &[Some(up), None]).unwrap();
        assert!(close(out.positions[0], [1.0, 2.0, 0.0]));
        assert!(close(out.positions[1], [5.0, 0.0, 0.0]));
        // No skin: nothing to do.
        let plain = MeshGeometry {
            positions: vec![[0.0; 3]],
            ..Default::default()
        };
        assert!(skin(&plain, &[identity], &[Some(up)]).is_none());
    }
}
