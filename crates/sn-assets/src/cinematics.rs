//! The lifepod's hatch cinematics as the rules need them (M9g5d,
//! `docs/DESIGN.md` § 4.3 "M9g5 plan"): the pod's animator (its program
//! and stored values), the node it moves the player along, and each
//! trigger's animator parameters. Facts in `docs/formats/gameplay.md`
//! § The player's body.

use std::collections::HashMap;
use std::sync::Arc;

use sn_anim::{Animator, Program};
use sn_unity::name_hash;
use sn_world::Transform;

use crate::anim::PosedNode;
use crate::scene::{CinematicTrigger, Scene};
use crate::{Assets, Result};

/// A trigger's animator parameters (name hashes; the names for logs).
#[derive(Clone, Debug, PartialEq)]
pub struct CinematicNames {
    /// The pod's `animParam`.
    pub play: u32,
    /// The pod's `interpolateAnimParam` (`prepare_…`).
    pub prepare: Option<u32>,
    /// The player's `playerViewAnimationName`.
    pub player: Option<u32>,
    pub play_name: String,
    pub player_name: String,
    /// The pod's `OnPlayerCinematicModeEndForward` passes the end event to
    /// this trigger's controller.
    pub forwarded: bool,
}

/// The pod's animator and the node the hatch cinematics move the player
/// along (all 8 use the same animator and node, checked at load).
#[derive(Clone)]
pub struct PodCinematics {
    pub program: Arc<Program>,
    /// Every pose value before animation (the hierarchy's stored places).
    pub defaults: Vec<f32>,
    /// The pod scene's top-level object in the world (the node's chain is
    /// relative to it).
    pub root: Transform,
    /// `cin_target`.
    pub animated: PosedNode,
    /// Per trigger, in the order given to [`PodCinematics::load`].
    pub names: Vec<CinematicNames>,
}

fn hash(name: &str) -> Option<u32> {
    (!name.is_empty()).then(|| name_hash(name))
}

impl PodCinematics {
    /// From the placed pod scene and its hand triggers; `None` without
    /// triggers.
    pub fn load(
        assets: &Assets,
        scene: &Scene,
        triggers: &[&CinematicTrigger],
    ) -> Result<Option<PodCinematics>> {
        let Some(first) = triggers.first() else {
            return Ok(None);
        };
        let (Some((r, animator)), Some((ar, animated))) =
            (first.animator_node, first.animated_node)
        else {
            return Err("hatch cinematic: animator or animated node not in the scene".into());
        };
        if triggers.iter().any(|t| {
            t.animator_node != first.animator_node || t.animated_node != first.animated_node
        }) {
            return Err("hatch cinematics: not all on one animator and node".into());
        }
        if ar != r {
            return Err("hatch cinematic: the animated node is under another object".into());
        }
        let prefab = &scene.roots[r];
        let controller = prefab.nodes[animator]
            .animator
            .as_ref()
            .and_then(|a| a.controller.clone())
            .ok_or("hatch cinematic: the pod's animator has no controller")?;
        let set = assets.animation_set(&controller, &mut HashMap::new())?;
        let program = Arc::new(Program::new(Arc::new(set.controller.clone()), &set.clips));
        let binding = prefab.bind_animator(animator, &program, &|_| Vec::new());
        let posed = prefab
            .posed_node(animated, &program, &binding)
            .ok_or("hatch cinematic: animated node missing")?;
        let forwards = scene.cinematic_forwards(assets)?;
        let names = triggers
            .iter()
            .map(|t| {
                let c = &t.cinematic;
                CinematicNames {
                    play: name_hash(&c.anim_param),
                    prepare: hash(&c.interpolate_anim_param),
                    player: hash(&c.player_view_animation_name),
                    play_name: c.anim_param.clone(),
                    player_name: c.player_view_animation_name.clone(),
                    forwarded: forwards
                        .iter()
                        .any(|f| f.forward.contains(&t.cinematic_key)),
                }
            })
            .collect();
        Ok(Some(PodCinematics {
            program,
            defaults: binding.defaults,
            root: prefab.nodes[0].local,
            animated: posed,
            names,
        }))
    }

    /// The pod's animator in its default states.
    pub fn animator(&self) -> Animator {
        Animator::new(self.program.clone(), self.defaults.clone())
    }

    /// `cin_target` in the world for the animator's current pose.
    pub fn animated_pose(&self, animator: &Animator) -> sn_sim::Pose {
        let t = self.root.then(&self.animated.in_prefab(animator.pose()));
        sn_sim::Pose::new(
            sn_sim::V3::from_f32(t.position),
            sn_sim::Q::from_f32(t.rotation).normalized(),
        )
    }
}
