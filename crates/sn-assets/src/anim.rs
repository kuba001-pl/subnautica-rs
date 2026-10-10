//! Animation data across files: an animator's controller with its clips,
//! and the paths its clips' bindings name (M7f4,
//! `docs/formats/animation.md`).

use std::collections::HashMap;
use std::sync::Arc;

use sn_anim::{Program, SlotKind};
use sn_unity::{
    ANIMATION_CLIP, ANIMATOR_CONTROLLER, AVATAR, AnimationClip, AnimatorController, Avatar,
    PLAYER_SETTINGS, clamps_blend_shape_weights, name_hash,
};

use crate::prefab::Prefab;
use crate::{Assets, ObjectRef, Result};

/// A controller and the clips it plays (`clips[i]` is the controller's
/// clip `i`; `None` if the reference is null or the clip failed to load).
pub struct AnimationSet {
    pub controller: AnimatorController,
    pub clips: Vec<Option<Arc<AnimationClip>>>,
    /// Errors met loading clips (the set is still usable without them).
    pub errors: Vec<String>,
}

fn data_of(object: &ObjectRef, class: i32) -> Result<&[u8]> {
    let (info, data) = object.data()?;
    if info.class_id != class {
        return Err(format!(
            "{} object {}: class {} where {class} was expected",
            object.file.name, object.path_id, info.class_id
        ));
    }
    Ok(data)
}

impl Assets<'_> {
    pub fn animator_controller(&self, object: &ObjectRef) -> Result<AnimatorController> {
        AnimatorController::parse(
            data_of(object, ANIMATOR_CONTROLLER)?,
            object.file.file().big_endian,
        )
        .map_err(|e| format!("AnimatorController {}: {e}", object.path_id))
    }

    pub fn animation_clip(&self, object: &ObjectRef) -> Result<AnimationClip> {
        AnimationClip::parse(
            data_of(object, ANIMATION_CLIP)?,
            object.file.file().big_endian,
        )
        .map_err(|e| format!("AnimationClip {}: {e}", object.path_id))
    }

    pub fn avatar(&self, object: &ObjectRef) -> Result<Avatar> {
        Avatar::parse(data_of(object, AVATAR)?, object.file.file().big_endian)
            .map_err(|e| format!("Avatar {}: {e}", object.path_id))
    }

    /// A controller with its clips. `cache` shares clips between
    /// controllers (keyed by the clip object).
    pub fn animation_set(
        &self,
        controller: &ObjectRef,
        cache: &mut HashMap<(std::path::PathBuf, String, i64), Arc<AnimationClip>>,
    ) -> Result<AnimationSet> {
        let parsed = self.animator_controller(controller)?;
        let mut clips = Vec::with_capacity(parsed.clips.len());
        let mut errors = Vec::new();
        for (i, pptr) in parsed.clips.iter().enumerate() {
            let clip = match self.resolve(&controller.file, *pptr) {
                Ok(Some(object)) => {
                    let key = object.key();
                    if let Some(c) = cache.get(&key) {
                        Some(c.clone())
                    } else {
                        match self.animation_clip(&object) {
                            Ok(c) => {
                                let c = Arc::new(c);
                                cache.insert(key, c.clone());
                                Some(c)
                            }
                            Err(e) => {
                                errors.push(format!("clip {i}: {e}"));
                                None
                            }
                        }
                    }
                }
                Ok(None) => None,
                Err(e) => {
                    errors.push(format!("clip {i}: {e}"));
                    None
                }
            };
            clips.push(clip);
        }
        Ok(AnimationSet {
            controller: parsed,
            clips,
            errors,
        })
    }
}

/// The project's quality levels (`QualitySettings`, `globalgamemanagers`;
/// M7f4f).
pub fn quality_settings(assets: &Assets) -> Result<sn_unity::QualitySettings> {
    let ggm = assets.standalone("globalgamemanagers")?;
    let info = ggm
        .objects()
        .iter()
        .find(|o| o.class_id == QUALITY_SETTINGS)
        .ok_or("globalgamemanagers: no QualitySettings")?;
    let (_, data) = ggm
        .object(info.path_id)
        .ok_or("globalgamemanagers: QualitySettings unreadable")?;
    sn_unity::QualitySettings::parse(data, ggm.file().big_endian)
        .map_err(|e| format!("QualitySettings: {e}"))
}

/// `QualitySettings`' class id.
const QUALITY_SETTINGS: i32 = 47;

/// Whether the game clamps blend shape weights to 0–100
/// (`PlayerSettings.legacyClampBlendShapeWeights`, `globalgamemanagers`).
pub fn blend_shape_clamp(assets: &Assets) -> Result<bool> {
    let ggm = assets.standalone("globalgamemanagers")?;
    let info = ggm
        .objects()
        .iter()
        .find(|o| o.class_id == PLAYER_SETTINGS)
        .ok_or("globalgamemanagers: no PlayerSettings")?;
    let (_, data) = ggm
        .object(info.path_id)
        .ok_or("globalgamemanagers: PlayerSettings unreadable")?;
    clamps_blend_shape_weights(data).map_err(|e| format!("PlayerSettings: {e}"))
}

impl Assets<'_> {
    /// The blend shape channel names of every skinned renderer in
    /// `prefab` whose mesh has some, by node (for
    /// [`Prefab::bind_animator`]). Meshes that fail to load are left out.
    pub fn blend_shape_names(&self, prefab: &Prefab) -> HashMap<usize, Vec<String>> {
        let mut out = HashMap::new();
        for (i, n) in prefab.nodes.iter().enumerate() {
            let Some(object) = n.mesh.as_ref().filter(|_| n.skinned) else {
                continue;
            };
            if let Ok(mesh) = self.mesh_info(object)
                && !mesh.blend_shapes.is_empty()
            {
                let names = mesh.blend_shapes.channels.into_iter().map(|c| c.name);
                out.insert(i, names.collect());
            }
        }
        out
    }
}

impl Prefab {
    /// The nodes at and below `node` by the hash of their path relative
    /// to it (`""` for `node` itself, `"a/b"` below), as bindings name
    /// them. With two siblings of one name, the first keeps the hash.
    pub fn binding_paths(&self, node: usize) -> HashMap<u32, usize> {
        let mut paths: HashMap<usize, String> = HashMap::new();
        let mut out = HashMap::new();
        if node >= self.nodes.len() {
            return out;
        }
        paths.insert(node, String::new());
        out.insert(name_hash(""), node);
        // Nodes are stored parents first.
        for i in node + 1..self.nodes.len() {
            let Some(parent) = self.nodes[i].parent else {
                continue;
            };
            let Some(base) = paths.get(&parent) else {
                continue;
            };
            let path = if base.is_empty() {
                self.nodes[i].name.clone()
            } else {
                format!("{base}/{}", self.nodes[i].name)
            };
            out.entry(name_hash(&path)).or_insert(i);
            paths.insert(i, path);
        }
        out
    }
}

/// What an animator drives, matched to a prefab: each slot's node (`None`:
/// the path is not in the hierarchy) and every value before animation.
pub struct AnimatorBinding {
    pub nodes: Vec<Option<usize>>,
    pub defaults: Vec<f32>,
    /// Slots whose path is not in the hierarchy.
    pub missing: usize,
}

impl Prefab {
    /// Binds `program`'s slots to the nodes below the animator on `node`.
    /// Defaults: Transforms as the hierarchy stores them; blend shape
    /// weights from the renderer where `blend_shape_names` gives the slot's
    /// shape; any other property 0 (not read yet: logged by the callers).
    pub fn bind_animator(
        &self,
        node: usize,
        program: &Program,
        blend_shape_names: &dyn Fn(usize) -> Vec<String>,
    ) -> AnimatorBinding {
        let paths = self.binding_paths(node);
        let mut defaults = vec![0.0; program.width];
        let mut nodes = Vec::with_capacity(program.slots.len());
        let mut missing = 0;
        for slot in &program.slots {
            let at = slot.offset;
            let found = paths.get(&slot.path).copied();
            if found.is_none() {
                missing += 1;
            }
            nodes.push(found);
            let Some(n) = found.and_then(|i| self.nodes.get(i).map(|n| (i, n))) else {
                if slot.kind == SlotKind::Rotation {
                    defaults[at + 3] = 1.0;
                } else if slot.kind == SlotKind::Scale {
                    defaults[at..at + 3].copy_from_slice(&[1.0; 3]);
                }
                continue;
            };
            let (index, n) = n;
            match slot.kind {
                SlotKind::Position => defaults[at..at + 3].copy_from_slice(&n.local.position),
                SlotKind::Rotation => defaults[at..at + 4].copy_from_slice(&n.local.rotation),
                SlotKind::Scale => defaults[at..at + 3].copy_from_slice(&n.local.scale),
                // A blend shape weight: the attribute is the CRC-32 of the
                // channel's name (no `blendShape.` prefix; M7f4d).
                SlotKind::Float {
                    type_id: 137,
                    attribute,
                    custom_type: 20,
                } => {
                    let names = blend_shape_names(index);
                    if let Some(i) = names.iter().position(|name| name_hash(name) == attribute) {
                        defaults[at] = n.blend_shape_weights.get(i).copied().unwrap_or(0.0);
                    }
                }
                SlotKind::Float { .. } => {}
            }
        }
        AnimatorBinding {
            nodes,
            defaults,
            missing,
        }
    }
}

/// A node's placement as an animator's pose moves it (M9g5b): the chain
/// from below the prefab's root down to the node, each link a node's
/// stored local placement with its animated position, rotation and scale
/// slots (pose offsets) put in its place.
#[derive(Clone, Debug, PartialEq)]
pub struct PosedNode {
    /// Root side first; the root's own placement left out, as
    /// [`crate::PrefabNode::in_prefab`].
    links: Vec<PosedLink>,
}

/// One node of a [`PosedNode`] chain: its stored local placement and the
/// pose offsets of its animated position, rotation and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosedLink {
    pub local: sn_world::Transform,
    pub position: Option<usize>,
    pub rotation: Option<usize>,
    pub scale: Option<usize>,
}

impl PosedNode {
    pub fn new(links: Vec<PosedLink>) -> PosedNode {
        PosedNode { links }
    }

    /// The node relative to the prefab's root for `pose` (an
    /// `Animator::pose`; offsets past its end keep the stored values).
    pub fn in_prefab(&self, pose: &[f32]) -> sn_world::Transform {
        let mut t = sn_world::Transform::default();
        for l in &self.links {
            let mut local = l.local;
            let get = |at: Option<usize>, n: usize| at.and_then(|a| pose.get(a..a + n));
            if let Some(v) = get(l.position, 3) {
                local.position = [v[0], v[1], v[2]];
            }
            if let Some(v) = get(l.rotation, 4) {
                local.rotation = [v[0], v[1], v[2], v[3]];
            }
            if let Some(v) = get(l.scale, 3) {
                local.scale = [v[0], v[1], v[2]];
            }
            t = t.then(&local);
        }
        t
    }
}

impl Prefab {
    /// `node`'s [`PosedNode`] for an animator's `program` bound by
    /// `binding` ([`Prefab::bind_animator`]); `None` if `node` is not in
    /// the prefab.
    pub fn posed_node(
        &self,
        node: usize,
        program: &Program,
        binding: &AnimatorBinding,
    ) -> Option<PosedNode> {
        self.nodes.get(node)?;
        let mut chain = Vec::new();
        let mut at = Some(node);
        while let Some(i) = at {
            let n = &self.nodes[i];
            // The root's own placement is the instance's.
            if n.parent.is_none() {
                break;
            }
            chain.push(i);
            at = n.parent;
        }
        chain.reverse();
        let links = chain
            .into_iter()
            .map(|i| {
                let mut link = PosedLink {
                    local: self.nodes[i].local,
                    position: None,
                    rotation: None,
                    scale: None,
                };
                for (slot, bound) in program.slots.iter().zip(&binding.nodes) {
                    if *bound != Some(i) {
                        continue;
                    }
                    match slot.kind {
                        SlotKind::Position => link.position = Some(slot.offset),
                        SlotKind::Rotation => link.rotation = Some(slot.offset),
                        SlotKind::Scale => link.scale = Some(slot.offset),
                        SlotKind::Float { .. } => {}
                    }
                }
                link
            })
            .collect();
        Some(PosedNode { links })
    }
}

#[cfg(test)]
mod posed_tests {
    use super::*;
    use sn_world::Transform;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn a_posed_chain_takes_the_animated_values() {
        // Parent 1 m up; child 2 m along the parent's z. The pose turns the
        // parent 90° about y and moves the child to 3 m along z.
        let parent = Transform {
            position: [0.0, 1.0, 0.0],
            ..Transform::default()
        };
        let child = Transform {
            position: [0.0, 0.0, 2.0],
            ..Transform::default()
        };
        let chain = PosedNode::new(vec![
            PosedLink {
                local: parent,
                position: None,
                rotation: Some(0),
                scale: None,
            },
            PosedLink {
                local: child,
                position: Some(4),
                rotation: None,
                scale: None,
            },
        ]);
        // Stored: rotation slot holds identity, position the stored 2 m.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let stored = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0];
        assert!(close(chain.in_prefab(&stored).position, [0.0, 1.0, 2.0]));
        // Posed: +z turned 90° about y is +x (Unity, left-handed).
        let posed = [0.0, h, 0.0, h, 0.0, 0.0, 3.0];
        let t = chain.in_prefab(&posed);
        assert!(close(t.position, [3.0, 1.0, 0.0]), "{:?}", t.position);
        // A pose too short keeps the stored values.
        assert!(close(chain.in_prefab(&[]).position, [0.0, 1.0, 2.0]));
    }
}
