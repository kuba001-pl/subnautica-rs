//! Animated objects (M7f4c, `docs/formats/animation.md`): the game's
//! `Animator`s run every frame (`sn-anim`) and move the Transforms below
//! them; skinned meshes bend with their bones on the GPU (Bevy's skinning,
//! the bones as joints).
//!
//! The worker describes each animator of a prefab as a [`RigDesc`]; every
//! spawned instance gets the rig's Transforms as a small entity hierarchy
//! with an [`AnimatedRig`] on its base.

use std::sync::Arc;
use std::time::Instant;

use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use sn_anim::{Animator, Program, SlotKind};
use sn_world::Transform as Placement;

/// One animator of a prefab, as the main thread spawns it.
#[derive(Clone)]
pub struct RigDesc {
    /// The animator's parent, relative to the prefab root (where the rig
    /// hangs; the root itself is the instance).
    pub base: Placement,
    /// The animator's GameObject (node 0) and every node below it: parent
    /// (index in this list) and stored local placement.
    pub nodes: Vec<(Option<u16>, Placement)>,
    pub program: Arc<Program>,
    /// Per program slot: the node it moves (`None`: not in the hierarchy,
    /// or not a Transform).
    pub slot_nodes: Vec<Option<u16>>,
    /// Every slot's value before animation.
    pub defaults: Vec<f32>,
    /// `Animator.cullingMode`: 0 always animate, 1 cull update transforms,
    /// 2 cull completely.
    pub culling: i32,
    pub skins: Vec<SkinDesc>,
    /// For logs: the prefab and the controller.
    pub name: String,
}

/// A skinned mesh bent by a rig's nodes.
#[derive(Clone)]
pub struct SkinDesc {
    /// Rig node of each bone (the mesh's joint index → node).
    pub joints: Vec<u16>,
    /// One per bone, in Bevy's coordinates.
    pub inverse_bindposes: Vec<Mat4>,
}

/// An animator running on a spawned instance (on the rig's base entity).
#[derive(Component)]
pub struct AnimatedRig {
    pub animator: Animator,
    /// Per program slot: the entity it moves.
    pub slots: Vec<Option<Entity>>,
    pub culling: i32,
    /// The rig's drawn parts: visible when any is.
    pub parts: Vec<Entity>,
}

/// Counts for the log (`--no-animation` turns animation off).
#[derive(Resource, Default)]
pub struct AnimationStats {
    pub rigs: usize,
    /// Rigs updated / whose Transforms were written in the last frame.
    pub updated: usize,
    pub applied: usize,
    /// CPU time of the last frame's animation, µs.
    pub micros: f32,
    /// Largest CPU time over the run, µs.
    pub worst_micros: f32,
}

/// Unity (left-handed) slot values to Bevy's transform parts: z mirrored.
fn position(v: &[f32]) -> Vec3 {
    Vec3::new(v[0], v[1], -v[2])
}

fn rotation(v: &[f32]) -> Quat {
    Quat::from_xyzw(-v[0], -v[1], v[2], v[3]).normalize()
}

/// Every frame: run each animator and write its Transforms. Culling as
/// the game's: mode 0 always; mode 1 runs the state machine but writes
/// nothing while no part is visible; mode 2 stops while no part is visible
/// (visibility from the last frame).
pub fn animate(
    time: Res<Time>,
    mut stats: ResMut<AnimationStats>,
    mut rigs: Query<&mut AnimatedRig>,
    visibility: Query<&ViewVisibility>,
    mut transforms: Query<&mut Transform, Without<AnimatedRig>>,
) {
    let start = Instant::now();
    let dt = time.delta_secs();
    let (mut count, mut updated, mut applied) = (0, 0, 0);
    for mut rig in &mut rigs {
        count += 1;
        let visible = rig
            .parts
            .iter()
            .any(|&e| visibility.get(e).is_ok_and(|v| v.get()));
        if rig.culling == 2 && !visible {
            continue;
        }
        rig.animator.update(dt);
        updated += 1;
        if rig.culling == 1 && !visible {
            continue;
        }
        applied += 1;
        let rig = &*rig;
        let program = rig.animator.program();
        let pose = rig.animator.pose();
        for (slot, entity) in program.slots.iter().zip(&rig.slots) {
            let Some(entity) = entity else { continue };
            let Ok(mut t) = transforms.get_mut(*entity) else {
                continue;
            };
            let v = &pose[slot.offset..slot.offset + slot.kind.width()];
            match slot.kind {
                SlotKind::Position => t.translation = position(v),
                SlotKind::Rotation => t.rotation = rotation(v),
                SlotKind::Scale => t.scale = Vec3::new(v[0], v[1], v[2]),
                SlotKind::Float { .. } => {}
            }
        }
    }
    let micros = start.elapsed().as_secs_f32() * 1e6;
    stats.rigs = count;
    stats.updated = updated;
    stats.applied = applied;
    stats.micros = micros;
    stats.worst_micros = stats.worst_micros.max(micros);
}

/// A Unity bind pose (column-major 4×4) as Bevy's: mirrored in z on both
/// sides (`S·M·S`, S = diag(1, 1, −1, 1)).
pub fn bindpose_to_bevy(m: &[f32; 16]) -> Mat4 {
    let mut out = *m;
    for col in 0..4 {
        for row in 0..4 {
            if (row == 2) != (col == 2) {
                out[col * 4 + row] = -out[col * 4 + row];
            }
        }
    }
    Mat4::from_cols_array(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_bind_poses_match_mirrored_points() {
        // A Unity transform: rotate 30° about y, move (1, 2, 3).
        let q = Quat::from_rotation_y(30f32.to_radians());
        let unity = Mat4::from_rotation_translation(q, Vec3::new(1.0, 2.0, 3.0));
        let p = Vec3::new(0.5, -1.0, 2.0);
        let mirror = |v: Vec3| Vec3::new(v.x, v.y, -v.z);
        let bevy = bindpose_to_bevy(&unity.to_cols_array());
        let a = mirror(unity.transform_point3(p));
        let b = bevy.transform_point3(mirror(p));
        assert!((a - b).length() < 1e-5, "{a} vs {b}");
    }

    #[test]
    fn slot_rotations_match_the_placement_conversion() {
        // The same conversion as objects' `to_bevy`.
        let v = [0.1f32, 0.2, 0.3, 0.927];
        let r = rotation(&v);
        let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
        assert!((r.x + v[0] / n).abs() < 1e-6 && (r.z - v[2] / n).abs() < 1e-6);
        assert_eq!(position(&[1.0, 2.0, 3.0]), Vec3::new(1.0, 2.0, -3.0));
    }
}
