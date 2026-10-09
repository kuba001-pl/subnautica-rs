//! The colliders of a prefab, in world space when placed (M9a). Read with
//! `sn_unity::Collider`; see `docs/formats/gameplay.md` § Colliders.
//!
//! Kept: enabled, non-trigger colliders on active nodes (active in the
//! hierarchy). Unity's scaling rules (*hypothesis*, from Unity's
//! documentation, not checked in the game): a box scales per axis; a
//! sphere's radius by the largest |scale|; a capsule's radius by the larger
//! |scale| of its two cross axes and its height by its own axis; a mesh
//! collider's vertices by the full transform. Scale is taken per axis
//! (`Transform::then`), so a rotated child under a non-uniformly scaled
//! parent gets no skew, as everywhere else in our code.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use sn_unity::{Collider, ColliderShape};
use sn_world::Transform;

use crate::{Assets, ObjectRef, Prefab, Result};

/// Mesh colliders' triangles in mesh space (three positions per triangle),
/// by mesh object, shared between prefabs.
pub type ColliderMeshes = HashMap<(PathBuf, String, i64), Arc<Vec<[f32; 3]>>>;

/// One kept collider: its node's placement in the prefab and its shape.
#[derive(Clone, Debug)]
pub struct PrefabCollider {
    pub node: usize,
    /// The node in the prefab (root transform left out).
    pub in_prefab: Transform,
    /// The node's physics layer.
    pub layer: u32,
    pub shape: ColliderShape,
    /// For mesh colliders: the mesh's triangles (empty when the mesh could
    /// not be read; counted in [`ColliderCounts::mesh_errors`]).
    pub mesh: Arc<Vec<[f32; 3]>>,
}

/// What [`Assets::prefab_colliders`] found and left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColliderCounts {
    pub kept: usize,
    pub triggers: usize,
    pub disabled: usize,
    pub inactive: usize,
    pub null_mesh: usize,
    pub mesh_errors: usize,
    pub convex: usize,
    pub layout_errors: usize,
    /// Kept colliders per physics layer.
    pub layers: BTreeMap<u32, usize>,
}

impl ColliderCounts {
    pub fn add(&mut self, o: &ColliderCounts) {
        self.kept += o.kept;
        self.triggers += o.triggers;
        self.disabled += o.disabled;
        self.inactive += o.inactive;
        self.null_mesh += o.null_mesh;
        self.mesh_errors += o.mesh_errors;
        self.convex += o.convex;
        self.layout_errors += o.layout_errors;
        for (l, n) in &o.layers {
            *self.layers.entry(*l).or_default() += n;
        }
    }
}

/// A collider placed in the world (Unity coordinates).
#[derive(Clone, Debug, PartialEq)]
pub enum WorldCollider {
    /// `axes`: unit, perpendicular; `half`: half extents along them.
    Box {
        center: [f32; 3],
        axes: [[f32; 3]; 3],
        half: [f32; 3],
    },
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    /// The segment `a`–`b` and the radius around it.
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
    },
    Triangles(Vec<[[f32; 3]; 3]>),
}

const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

impl PrefabCollider {
    /// In the world, with the prefab's root at `placement`.
    pub fn world(&self, placement: &Transform) -> WorldCollider {
        let t = placement.then(&self.in_prefab);
        let s = t.scale.map(f32::abs);
        match self.shape {
            ColliderShape::Box { size, center } => WorldCollider::Box {
                center: t.transform_point(center),
                axes: AXES.map(|a| t.rotate_vector(a)),
                half: [0, 1, 2].map(|a| size[a].abs() * s[a] * 0.5),
            },
            ColliderShape::Sphere { radius, center } => WorldCollider::Sphere {
                center: t.transform_point(center),
                radius: radius.abs() * s[0].max(s[1]).max(s[2]),
            },
            ColliderShape::Capsule {
                radius,
                height,
                direction,
                center,
            } => {
                let d = direction.clamp(0, 2) as usize;
                let (j, k) = ((d + 1) % 3, (d + 2) % 3);
                let radius = radius.abs() * s[j].max(s[k]);
                let half = (height.abs() * s[d] * 0.5 - radius).max(0.0);
                let c = t.transform_point(center);
                let axis = t.rotate_vector(AXES[d]);
                WorldCollider::Capsule {
                    a: [0, 1, 2].map(|i| c[i] - axis[i] * half),
                    b: [0, 1, 2].map(|i| c[i] + axis[i] * half),
                    radius,
                }
            }
            ColliderShape::Mesh { .. } => WorldCollider::Triangles(
                self.mesh
                    .chunks_exact(3)
                    .map(|p| [0, 1, 2].map(|i| t.transform_point(p[i])))
                    .collect(),
            ),
        }
    }
}

impl Assets<'_> {
    /// The colliders the player can hit on a prefab, and counts of what was
    /// left out. `meshes` caches mesh colliders' triangles between calls.
    pub fn prefab_colliders(
        &self,
        prefab: &Prefab,
        meshes: &mut ColliderMeshes,
    ) -> Result<(Vec<PrefabCollider>, ColliderCounts)> {
        let mut out = Vec::new();
        let mut counts = ColliderCounts::default();
        for (i, node) in prefab.nodes.iter().enumerate() {
            for c in self.node_components(node)? {
                let (info, data) = c.data()?;
                if !Collider::is_collider(info.class_id) {
                    continue;
                }
                let col = match Collider::parse(info.class_id, data, c.file.file().big_endian) {
                    Ok(col) => col,
                    Err(_) => {
                        counts.layout_errors += 1;
                        continue;
                    }
                };
                if col.is_trigger {
                    counts.triggers += 1;
                    continue;
                }
                if !col.enabled {
                    counts.disabled += 1;
                    continue;
                }
                if !node.active {
                    counts.inactive += 1;
                    continue;
                }
                let mut mesh = Arc::new(Vec::new());
                if let ColliderShape::Mesh {
                    convex, mesh: ptr, ..
                } = col.shape
                {
                    counts.convex += usize::from(convex);
                    match self.resolve(&c.file, ptr)? {
                        None => {
                            counts.null_mesh += 1;
                            continue;
                        }
                        Some(object) => match self.collider_mesh(&object, meshes) {
                            Ok(m) => mesh = m,
                            Err(_) => {
                                counts.mesh_errors += 1;
                                continue;
                            }
                        },
                    }
                }
                counts.kept += 1;
                *counts.layers.entry(node.layer).or_default() += 1;
                out.push(PrefabCollider {
                    node: i,
                    in_prefab: node.in_prefab,
                    layer: node.layer,
                    shape: col.shape,
                    mesh,
                });
            }
        }
        Ok((out, counts))
    }

    fn collider_mesh(
        &self,
        object: &ObjectRef,
        meshes: &mut ColliderMeshes,
    ) -> Result<Arc<Vec<[f32; 3]>>> {
        if let Some(m) = meshes.get(&object.key()) {
            return Ok(m.clone());
        }
        let (_, g) = self.mesh(object)?;
        let mut tris = Vec::new();
        for sub in &g.sub_meshes {
            for t in sub.chunks_exact(3) {
                let p = |i: u32| g.positions.get(i as usize).copied();
                if let (Some(a), Some(b), Some(c)) = (p(t[0]), p(t[1]), p(t[2])) {
                    tris.extend([a, b, c]);
                }
            }
        }
        let m = Arc::new(tris);
        meshes.insert(object.key(), m.clone());
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(shape: ColliderShape, in_prefab: Transform) -> PrefabCollider {
        PrefabCollider {
            node: 0,
            in_prefab,
            layer: 0,
            shape,
            mesh: Arc::new(vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn scaling_rules() {
        let at = Transform {
            position: [10.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [2.0, 3.0, -4.0],
        };
        let sphere = placed(
            ColliderShape::Sphere {
                radius: 0.5,
                center: [1.0, 0.0, 0.0],
            },
            Transform::default(),
        );
        assert_eq!(
            sphere.world(&at),
            WorldCollider::Sphere {
                center: [12.0, 0.0, 0.0],
                radius: 2.0
            }
        );
        let capsule = placed(
            ColliderShape::Capsule {
                radius: 0.5,
                height: 4.0,
                direction: 1,
                center: [0.0; 3],
            },
            Transform::default(),
        );
        // Radius × max(|2|, |-4|) = 2; height 4 × 3 = 12, segment ±(6 − 2).
        let WorldCollider::Capsule { a, b, radius } = capsule.world(&at) else {
            panic!()
        };
        assert_eq!(radius, 2.0);
        assert!(close(a, [10.0, -4.0, 0.0]) && close(b, [10.0, 4.0, 0.0]));
        let boxed = placed(
            ColliderShape::Box {
                size: [1.0, 2.0, 3.0],
                center: [0.0, 1.0, 0.0],
            },
            Transform::default(),
        );
        let WorldCollider::Box { center, half, .. } = boxed.world(&at) else {
            panic!()
        };
        assert!(close(center, [10.0, 3.0, 0.0]));
        assert!(close(half, [1.0, 3.0, 6.0]));
        let mesh = placed(
            ColliderShape::Mesh {
                convex: false,
                cooking_options: 0,
                mesh: sn_unity::PPtr::default(),
            },
            Transform::default(),
        );
        let WorldCollider::Triangles(t) = mesh.world(&at) else {
            panic!()
        };
        assert_eq!(
            t,
            vec![[[10.0, 0.0, 0.0], [12.0, 0.0, 0.0], [10.0, 3.0, 0.0]]]
        );
    }
}
