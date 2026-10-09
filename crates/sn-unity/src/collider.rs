//! Physics colliders (built-in classes, Unity 2019.4 layouts): the shapes
//! the player collides with (M9a). Every collider starts with
//! `m_GameObject`, `m_Material`, `m_IsTrigger` and `m_Enabled` (two bytes,
//! aligned); the shape's fields follow. The parser requires the data to end
//! with the last field. See `docs/formats/gameplay.md` § Colliders.

use crate::objects::PPtr;
use crate::reader::Reader;
use crate::{ErrorKind, Result};

pub const BOX_COLLIDER: i32 = 65;
pub const SPHERE_COLLIDER: i32 = 135;
pub const CAPSULE_COLLIDER: i32 = 136;
pub const MESH_COLLIDER: i32 = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderShape {
    Box {
        size: [f32; 3],
        center: [f32; 3],
    },
    Sphere {
        radius: f32,
        center: [f32; 3],
    },
    /// `direction`: the axis of the height (0 x, 1 y, 2 z).
    Capsule {
        radius: f32,
        height: f32,
        direction: i32,
        center: [f32; 3],
    },
    Mesh {
        convex: bool,
        cooking_options: i32,
        mesh: PPtr,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collider {
    pub game_object: PPtr,
    pub material: PPtr,
    pub is_trigger: bool,
    pub enabled: bool,
    pub shape: ColliderShape,
}

impl Collider {
    /// Whether `class_id` is one of the colliders read here.
    pub fn is_collider(class_id: i32) -> bool {
        matches!(
            class_id,
            BOX_COLLIDER | SPHERE_COLLIDER | CAPSULE_COLLIDER | MESH_COLLIDER
        )
    }

    pub fn parse(class_id: i32, data: &[u8], big_endian: bool) -> Result<Collider> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let material = PPtr::read(&mut r)?;
        let is_trigger = r.u8()? != 0;
        let enabled = r.u8()? != 0;
        r.align(4)?;
        let v3 = |r: &mut Reader| -> Result<[f32; 3]> { Ok([r.f32()?, r.f32()?, r.f32()?]) };
        let shape = match class_id {
            BOX_COLLIDER => ColliderShape::Box {
                size: v3(&mut r)?,
                center: v3(&mut r)?,
            },
            SPHERE_COLLIDER => ColliderShape::Sphere {
                radius: r.f32()?,
                center: v3(&mut r)?,
            },
            CAPSULE_COLLIDER => ColliderShape::Capsule {
                radius: r.f32()?,
                height: r.f32()?,
                direction: r.i32()?,
                center: v3(&mut r)?,
            },
            MESH_COLLIDER => {
                let convex = r.u8()? != 0;
                r.align(4)?;
                ColliderShape::Mesh {
                    convex,
                    cooking_options: r.i32()?,
                    mesh: PPtr::read(&mut r)?,
                }
            }
            other => {
                return Err(r.error(ErrorKind::Unsupported(format!(
                    "class {other} is not a collider"
                ))));
            }
        };
        if r.pos() != data.len() {
            return Err(r.error(ErrorKind::Invalid(format!(
                "{} bytes after the last field",
                data.len() - r.pos()
            ))));
        }
        Ok(Collider {
            game_object,
            material,
            is_trigger,
            enabled,
            shape,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(trigger: bool, enabled: bool) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&7i64.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&9i64.to_le_bytes());
        b.extend_from_slice(&[u8::from(trigger), u8::from(enabled), 0, 0]);
        b
    }

    fn floats(b: &mut Vec<u8>, v: &[f32]) {
        for x in v {
            b.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn samples() -> Vec<(i32, Vec<u8>, ColliderShape)> {
        let mut boxed = head(false, true);
        floats(&mut boxed, &[1.0, 2.0, 3.0, 0.5, 0.0, -0.5]);
        let mut sphere = head(true, true);
        floats(&mut sphere, &[0.75, 0.0, 1.0, 0.0]);
        let mut capsule = head(false, false);
        floats(&mut capsule, &[0.3, 1.8]);
        capsule.extend_from_slice(&1i32.to_le_bytes());
        floats(&mut capsule, &[0.0, 0.9, 0.0]);
        let mut mesh = head(false, true);
        mesh.extend_from_slice(&[1, 0, 0, 0]);
        mesh.extend_from_slice(&14i32.to_le_bytes());
        mesh.extend_from_slice(&2i32.to_le_bytes());
        mesh.extend_from_slice(&33i64.to_le_bytes());
        vec![
            (
                BOX_COLLIDER,
                boxed,
                ColliderShape::Box {
                    size: [1.0, 2.0, 3.0],
                    center: [0.5, 0.0, -0.5],
                },
            ),
            (
                SPHERE_COLLIDER,
                sphere,
                ColliderShape::Sphere {
                    radius: 0.75,
                    center: [0.0, 1.0, 0.0],
                },
            ),
            (
                CAPSULE_COLLIDER,
                capsule,
                ColliderShape::Capsule {
                    radius: 0.3,
                    height: 1.8,
                    direction: 1,
                    center: [0.0, 0.9, 0.0],
                },
            ),
            (
                MESH_COLLIDER,
                mesh,
                ColliderShape::Mesh {
                    convex: true,
                    cooking_options: 14,
                    mesh: PPtr {
                        file_id: 2,
                        path_id: 33,
                    },
                },
            ),
        ]
    }

    #[test]
    fn reads_every_shape() {
        for (class, data, shape) in samples() {
            let c = Collider::parse(class, &data, false).unwrap();
            assert_eq!(c.shape, shape);
            assert_eq!(c.game_object.path_id, 7);
            assert_eq!(c.material.path_id, 9);
        }
        let (_, sphere, _) = &samples()[1];
        assert!(
            Collider::parse(SPHERE_COLLIDER, sphere, false)
                .unwrap()
                .is_trigger
        );
        let (_, capsule, _) = &samples()[2];
        assert!(
            !Collider::parse(CAPSULE_COLLIDER, capsule, false)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn wrong_class_or_size_is_an_error() {
        let (_, boxed, _) = &samples()[0];
        assert!(Collider::parse(1, boxed, false).is_err());
        // A box read as a sphere leaves bytes over.
        assert!(Collider::parse(SPHERE_COLLIDER, boxed, false).is_err());
        for (class, data, _) in samples() {
            for i in 0..data.len() {
                assert!(Collider::parse(class, &data[..i], false).is_err());
            }
        }
    }
}
