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
