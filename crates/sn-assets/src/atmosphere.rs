//! Atmosphere metadata read in place. No game geometry is retained in source.
use sn_sim::{V3, atmosphere::Volume, collide::Shape};
use sn_unity::{Collider, ColliderShape, GameObject, StructLayout, read_script};
use sn_world::Transform;

use crate::{
    Assets, Mat4, Prefab, Result,
    skin::{mul, trs},
    terrain::script_class,
};

#[derive(Clone, Debug)]
pub struct AtmosphereTemplate {
    pub priority: i32,
    pub biome: String,
    pub shape: ColliderShape,
    /// Full affine product, relative to the replaced prefab root.
    pub local: Mat4,
}

impl AtmosphereTemplate {
    pub fn under(&self, parent: &Transform) -> Self {
        let mut out = self.clone();
        out.local = mul(&trs(parent), &self.local);
        out
    }
    pub fn placed(&self, placement: &Transform, tie: u64) -> Result<Volume> {
        let matrix = mul(&trs(placement), &self.local);
        if !matrix.iter().all(|x| x.is_finite()) {
            return Err("atmosphere: non-finite transform".into());
        }
        let columns =
            [0, 1, 2].map(|i| V3::from_f32([matrix[i * 4], matrix[i * 4 + 1], matrix[i * 4 + 2]]));
        let scale = columns.map(V3::length);
        if scale.iter().any(|s| *s < 1e-8) {
            return Err("atmosphere: collapsed collider transform".into());
        }
        let axes = [0, 1, 2].map(|i| columns[i] * (1.0 / scale[i]));
        let point = |p: [f32; 3]| {
            V3::from_f32([0, 1, 2].map(|r| {
                matrix[r] * p[0] + matrix[4 + r] * p[1] + matrix[8 + r] * p[2] + matrix[12 + r]
            }))
        };
        let shape = match self.shape {
            ColliderShape::Box { center, size } => {
                if (0..3).any(|i| axes[i].dot(axes[(i + 1) % 3]).abs() > 1e-5) {
                    return Err("atmosphere: sheared box requires verified PhysX dimensions".into());
                }
                if !size.iter().chain(&center).all(|x| x.is_finite()) {
                    return Err("atmosphere: non-finite box".into());
                }
                Shape::Box {
                    center: point(center),
                    axes,
                    half: [0, 1, 2].map(|i| f64::from(size[i].abs()) * scale[i] * 0.5),
                }
            }
            ColliderShape::Sphere { center, radius } => {
                if !radius.is_finite() || !center.iter().all(|x| x.is_finite()) {
                    return Err("atmosphere: non-finite sphere".into());
                }
                Shape::Sphere {
                    center: point(center),
                    radius: f64::from(radius.abs()) * scale.into_iter().fold(0.0, f64::max),
                }
            }
            ColliderShape::Capsule {
                center,
                radius,
                height,
                direction,
            } => {
                let axis = usize::try_from(direction)
                    .ok()
                    .filter(|i| *i < 3)
                    .ok_or("atmosphere: invalid capsule direction")?;
                if !radius.is_finite()
                    || !height.is_finite()
                    || !center.iter().all(|x| x.is_finite())
                {
                    return Err("atmosphere: non-finite capsule".into());
                }
                let radius =
                    f64::from(radius.abs()) * scale[(axis + 1) % 3].max(scale[(axis + 2) % 3]);
                let half = (f64::from(height.abs()) * scale[axis] * 0.5 - radius).max(0.0);
                let c = point(center);
                Shape::Capsule {
                    a: c - axes[axis] * half,
                    b: c + axes[axis] * half,
                    radius,
                }
            }
            ColliderShape::Mesh { .. } => {
                return Err("atmosphere: unsupported mesh collider".into());
            }
        };
        Ok(Volume {
            shape,
            priority: self.priority,
            tie,
            biome: self.biome.clone(),
        })
    }
}

impl Assets<'_> {
    /// Read all active atmosphere components, including invisible trigger nodes.
    /// Script layout is derived once from the installed assembly by the caller.
    pub fn prefab_atmospheres(
        &self,
        prefab: &Prefab,
        layout: &StructLayout,
    ) -> Result<Vec<AtmosphereTemplate>> {
        let mut matrices = Vec::with_capacity(prefab.nodes.len());
        let mut out = Vec::new();
        for (i, node) in prefab.nodes.iter().enumerate() {
            let local = if i == 0 {
                trs(&Transform::default())
            } else {
                trs(&node.local)
            };
            let matrix = match node.parent {
                Some(p) => mul(
                    matrices
                        .get(p)
                        .ok_or("atmosphere: parent not before child")?,
                    &local,
                ),
                None => local,
            };
            matrices.push(matrix);
            // Optimized animation rigs reconstruct hidden bones with identities
            // made by prefab::expand_terminal_avatar, not serialized GameObjects.
            // They still participate in the transform chain above.
            if !node.active || node.object.1.contains("#avatar:") {
                continue;
            }
            let file = self.file(&node.object.0, &node.object.1)?;
            let (_, bytes) = file
                .object(node.object.2)
                .ok_or("atmosphere: missing GameObject")?;
            let go = GameObject::parse(bytes, file.file().big_endian).map_err(|e| e.to_string())?;
            let mut atmosphere = None;
            let mut colliders = Vec::new();
            for reference in &go.components {
                let Some(object) = self.resolve(&file, *reference)? else {
                    continue;
                };
                let (info, bytes) = object.data()?;
                if info.class_id == 114
                    && script_class(self, &object).as_deref() == Some("AtmosphereVolume")
                {
                    let (_, fields) = read_script(bytes, object.file.file().big_endian, layout)
                        .map_err(|e| format!("{} atmosphere: {e}", prefab.key))?;
                    let priority = fields
                        .get("priority")
                        .and_then(|v| v.as_i64())
                        .and_then(|v| i32::try_from(v).ok())
                        .ok_or("atmosphere: missing priority")?;
                    let affects = fields
                        .get("affectsVisuals")
                        .and_then(|v| v.as_bool())
                        .ok_or("atmosphere: missing affectsVisuals")?;
                    let biome = fields
                        .get("overrideBiome")
                        .and_then(|v| v.as_str())
                        .ok_or("atmosphere: missing overrideBiome")?;
                    if affects {
                        atmosphere = Some((priority, biome.to_owned()));
                    }
                } else if Collider::is_collider(info.class_id) {
                    colliders.push(
                        Collider::parse(info.class_id, bytes, object.file.file().big_endian)
                            .map_err(|e| format!("{} atmosphere collider: {e}", prefab.key))?,
                    );
                }
            }
            if let Some((priority, biome)) = atmosphere {
                let [collider] = colliders.as_slice() else {
                    return Err(format!(
                        "{} atmosphere: expected exactly one collider, got {}",
                        prefab.key,
                        colliders.len()
                    ));
                };
                if collider.enabled {
                    out.push(AtmosphereTemplate {
                        priority,
                        biome,
                        shape: collider.shape,
                        local: matrix,
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_sim::atmosphere::contains;

    #[test]
    fn scaled_rotated_box_and_reflection() {
        let t = AtmosphereTemplate {
            priority: 7,
            biome: "synthetic".into(),
            shape: ColliderShape::Box {
                center: [1.0, 0.0, 0.0],
                size: [2.0, 4.0, 6.0],
            },
            local: trs(&Transform::default()),
        };
        let p = Transform {
            position: [10.0, 20.0, 30.0],
            rotation: [
                0.0,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            scale: [-2.0, 3.0, 4.0],
        };
        let volume = t.placed(&p, 1).unwrap();
        assert!(contains(
            &volume.shape,
            V3::from_f32(p.transform_point([1.0, 0.0, 0.0]))
        ));
        assert!(!contains(
            &volume.shape,
            V3::from_f32(p.transform_point([2.01, 0.0, 0.0]))
        ));
    }

    #[test]
    fn sphere_capsule_scaling_and_rejected_transforms() {
        let mut t = AtmosphereTemplate {
            priority: 0,
            biome: String::new(),
            shape: ColliderShape::Sphere {
                center: [0.0; 3],
                radius: 2.0,
            },
            local: trs(&Transform::default()),
        };
        let p = Transform {
            scale: [2.0, 3.0, -4.0],
            ..Default::default()
        };
        assert!(contains(&t.placed(&p, 1).unwrap().shape, V3::Y * 8.0));
        t.shape = ColliderShape::Capsule {
            center: [0.0; 3],
            radius: 1.0,
            height: 8.0,
            direction: 1,
        };
        let Shape::Capsule { a, b, radius } = t.placed(&p, 1).unwrap().shape else {
            panic!()
        };
        assert_eq!(radius, 4.0);
        assert_eq!((b - a).length(), 16.0);
        assert!(
            t.placed(
                &Transform {
                    scale: [0.0, 1.0, 1.0],
                    ..Default::default()
                },
                1
            )
            .is_err()
        );
        t.shape = ColliderShape::Box {
            center: [0.0; 3],
            size: [1.0; 3],
        };
        t.local[4] = 0.5;
        assert!(t.placed(&p, 1).is_err());
    }
}
