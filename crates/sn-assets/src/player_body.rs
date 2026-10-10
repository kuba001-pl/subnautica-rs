//! The player's body (M9g1, `docs/DESIGN.md` § 4.3 "M9g plan"): the
//! `Player` hierarchy of the main scene, the suit models the equipment rule
//! shows, the head, the camera's nodes, the animator and the numbers of
//! `ArmsController`. Facts in `docs/formats/gameplay.md` § The player's body.

use std::collections::HashMap;
use std::sync::Arc;

use sn_anim::{Animator, Program};
use sn_unity::{ArmsController, PPtr, PlayerFields, SkinnedMeshRenderer};

use crate::anim::PosedNode;
use crate::{Assets, ObjectRef, Prefab, Result};

/// The player's animator as the rules need it (M9g5d/e): the compiled
/// controller, the pose before animation, and the two nodes the camera
/// follows, posed by the animator: `Player.camAnchor` (a cinematic moves
/// `camRoot` there) and `CameraToPlayerManager.headCameraBone` (the death
/// camera). Their chains start below the player's root and pass through
/// the view model (checked to be stored at the root's origin).
#[derive(Clone)]
pub struct PlayerAnimation {
    pub program: Arc<Program>,
    pub defaults: Vec<f32>,
    pub cam_anchor: PosedNode,
    pub head_camera: PosedNode,
    /// `MainCameraControl.minimumY` / `maximumY`.
    pub look_limits: (f64, f64),
}

impl PlayerAnimation {
    pub fn animator(&self) -> Animator {
        Animator::new(self.program.clone(), self.defaults.clone())
    }
}

/// `TechType.None`: what `Equipment.GetTechTypeInSlot` returns for an
/// empty slot.
pub const TECH_TYPE_NONE: i32 = 0;

/// Parameters of the player's animator set every frame by the rules M9g
/// ports (`ArmsController.Update`, `SetPlayerSpeedParameters`,
/// `UpdateDiving`, `InstallAnimationRules`; `Player.Start` for `vr_active`,
/// `Player.OnKill` for the death triggers), with values that change in play.
pub const RULE_PARAMETERS: &[&str] = &[
    "move_speed",
    "move_speed_x",
    "move_speed_y",
    "move_speed_z",
    "view_pitch",
    "view_turn",
    "is_underwater",
    "on_surface",
    "diving",
    "diving_land",
    "jump",
    "verticalOffset",
    "cinematics_enabled",
    "vr_active",
    "player_death",
    "player_death_fire",
    "player_death_explosion",
];

/// Parameters the same scripts set every frame to a value that is fixed
/// while the player has no tool, PDA, vehicle, creature or base: false
/// (`grab`, `bash`: 0.4 s after a creature grabs or bashes; the others by
/// tool, PDA, builder, vehicle, piloting or a Bleeder). Ported as that
/// value; each becomes a rule with its item.
pub const FIXED_PARAMETERS: &[&str] = &[
    "grab",
    "bash",
    "using_tool",
    "using_tool_alt",
    "holding_tool",
    "holding_welder",
    "in_seamoth",
    "in_exosuit",
    "cyclops_steering",
    "bleeder",
    "using_pda",
    "using_builder",
];

/// One `Player.equipmentModels` entry with its GameObjects as nodes of
/// [`PlayerBody::prefab`] (`None`: a null reference or an object outside
/// the player).
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentSlot {
    pub slot: String,
    pub default_node: Option<usize>,
    /// (tech type, model node).
    pub models: Vec<(i32, Option<usize>)>,
}

/// The player's body as the main scene holds it.
#[derive(Clone)]
pub struct PlayerBody {
    /// The `Player` top-level object of the main scene (node 0 at the
    /// player's stored world placement), with the equipment rule applied
    /// for a new game (nothing equipped, [`PlayerBody::equip`]).
    pub prefab: Prefab,
    pub player: PlayerFields,
    pub arms: ArmsController,
    /// The node with the player's `Animator` (`Player.playerAnimator`).
    pub animator_node: usize,
    /// Its `AnimatorController`.
    pub controller: ObjectRef,
    /// The head renderer's node (`Player.head`).
    pub head_node: usize,
    /// `MainCameraControl.viewModel`.
    pub view_model_node: usize,
    /// The node `MainCameraControl` sits on, its `cameraUPTransform` and
    /// its `cameraOffsetTransform`.
    pub camera_node: usize,
    pub camera_up_node: usize,
    pub camera_offset_node: usize,
    /// `MainCameraControl` itself (look limits, `skin`, tilt).
    pub camera: sn_unity::MainCameraControl,
    /// Where the main camera hangs (M9g2): the scene's one `AutoParent`
    /// puts the top-level object holding the `MainCamera`-tagged camera
    /// under this node, with identity locals, when the game starts.
    pub main_camera_parent: usize,
    /// Where that camera sits in its own top-level object (its local
    /// placement chain below the object's root; identity is expected).
    pub main_camera_in_object: sn_world::Transform,
    pub slots: Vec<EquipmentSlot>,
    /// `Player.camAnchor` (M9g5): where a cinematic brings the camera.
    pub cam_anchor_node: usize,
    /// `CameraToPlayerManager.headCameraBone` (M9g5): the camera copies
    /// it after death.
    pub head_camera_node: usize,
}

impl PlayerBody {
    /// `Player.EquipmentChanged`: per slot, each model is active when its
    /// tech type is the one in the slot (`in_slot`, [`TECH_TYPE_NONE`] when
    /// empty), the others not; the default model is active when none
    /// matched. Null models are skipped, as the game's `if (model)`.
    pub fn equip(&mut self, in_slot: impl Fn(&str) -> i32) {
        for (node, active) in equipment_changes(&self.slots, in_slot) {
            self.prefab.set_active(node, active);
        }
    }
}

impl PlayerBody {
    /// The numbers `sn_sim::body` needs (M9g2). `ocean_level`:
    /// `Ocean.GetOceanLevel` ([`crate::PlayerData::ocean_level`]).
    pub fn body_params(&self, ocean_level: f32) -> sn_sim::body::BodyParams {
        use sn_sim::V3;
        let nodes = &self.prefab.nodes;
        let a = &self.arms;
        let c = &self.camera;
        // `cameraAngleMotion` is the view model's stored local Euler
        // angles; its y (Unity's Z-X-Y decomposition) times the tilt.
        let [x, y, z, w] = nodes[self.view_model_node].local.rotation.map(f64::from);
        let yaw = (2.0 * (x * z + w * y))
            .atan2(1.0 - 2.0 * (x * x + y * y))
            .to_degrees();
        sn_sim::body::BodyParams {
            smooth_speed_under_water: f64::from(a.smooth_speed_under_water),
            smooth_speed_above_water: f64::from(a.smooth_speed_above_water),
            turn_animation_damp_time: f64::from(a.turn_animation_damp_time),
            skin: f64::from(c.skin),
            step_amount: f64::from(c.step_amount),
            view_model_roll: yaw * f64::from(c.camera_tilt_mod),
            camera_up_position: V3::from_f32(nodes[self.camera_up_node].local.position),
            camera_offset_position: V3::from_f32(nodes[self.camera_offset_node].local.position),
            ocean_level: f64::from(ocean_level),
        }
    }
}

/// The `SetActive` calls of `Player.EquipmentChanged`, in the game's order
/// (see [`PlayerBody::equip`]).
pub fn equipment_changes(
    slots: &[EquipmentSlot],
    in_slot: impl Fn(&str) -> i32,
) -> Vec<(usize, bool)> {
    let mut changes = Vec::new();
    for slot in slots {
        let tech = in_slot(&slot.slot);
        let mut matched = false;
        for &(model_tech, node) in &slot.models {
            let on = model_tech == tech;
            matched |= on;
            if let Some(n) = node {
                changes.push((n, on));
            }
        }
        if let Some(d) = slot.default_node {
            changes.push((d, !matched));
        }
    }
    changes
}

impl Assets<'_> {
    /// The player's body from the main scene, as at the start of a new
    /// game (nothing equipped).
    pub fn player_body(&self) -> Result<PlayerBody> {
        let scene = self.scene("main")?;
        let big_endian = scene.file.file().big_endian;
        let mut players = scene.behaviours(self, "Player");
        if players.len() != 1 {
            return Err(format!(
                "main scene: {} Player behaviours, expected 1",
                players.len()
            ));
        }
        let player_ref = players.remove(0);
        let player = PlayerFields::parse(player_ref.data()?.1, big_endian)
            .map_err(|e| format!("Player: {e}"))?;
        let (root, _) = scene
            .behaviour_node(self, &player_ref)?
            .ok_or("Player: object not in the main scene")?;
        let file = &player_ref.file;
        // A GameObject reference → its node in the player's hierarchy.
        let node_of_object = |pptr: PPtr| -> Result<Option<usize>> {
            Ok(self
                .resolve(file, pptr)?
                .and_then(|go| match scene.locate(&go) {
                    Some((r, n)) if r == root => Some(n),
                    _ => None,
                }))
        };
        // A component reference → the node of its GameObject.
        let node_of_component = |pptr: PPtr, what: &str| -> Result<usize> {
            let c = self
                .resolve(file, pptr)?
                .ok_or_else(|| format!("{what} is null"))?;
            let (_, data) = c.data()?;
            // Every component starts with its `m_GameObject`.
            let go = PPtr::component_game_object(data, c.file.file().big_endian)
                .map_err(|e| format!("{what}: {e}"))?;
            node_of_object(go)?.ok_or_else(|| format!("{what}: not on the player"))
        };
        let behaviour_node = |b: &ObjectRef, what: &str| -> Result<usize> {
            match scene.behaviour_node(self, b)? {
                Some((r, n)) if r == root => Ok(n),
                _ => Err(format!("{what}: not on the player")),
            }
        };

        let mut slots = Vec::with_capacity(player.equipment_models.len());
        for e in &player.equipment_models {
            let mut models = Vec::with_capacity(e.equipment.len());
            for m in &e.equipment {
                models.push((m.tech_type, node_of_object(m.model)?));
            }
            slots.push(EquipmentSlot {
                slot: e.slot.clone(),
                default_node: node_of_object(e.default_model)?,
                models,
            });
        }

        let head_node = node_of_component(player.head, "Player.head")?;
        // The head must be a skinned renderer (`SetHeadVisible` sets its
        // shadow mode).
        let head = self
            .resolve(file, player.head)?
            .ok_or("Player.head is null")?;
        SkinnedMeshRenderer::parse(head.data()?.1, big_endian)
            .map_err(|e| format!("Player.head: {e}"))?;
        let animator_node = node_of_component(player.player_animator, "Player.playerAnimator")?;
        let prefab = scene.roots[root].clone();
        let controller = prefab.nodes[animator_node]
            .animator
            .as_ref()
            .and_then(|a| a.controller.clone())
            .ok_or("Player.playerAnimator: no controller")?;

        let arms_ref = self
            .resolve(file, player.arms_controller)?
            .ok_or("Player.armsController is null")?;
        if self.script_class(&arms_ref).as_deref() != Some("ArmsController") {
            return Err("Player.armsController is not an ArmsController".into());
        }
        let arms = ArmsController::parse(arms_ref.data()?.1, big_endian)
            .map_err(|e| format!("ArmsController: {e}"))?;
        if behaviour_node(&arms_ref, "ArmsController")? != animator_node {
            return Err("ArmsController is not on the animator's node".into());
        }

        let mut cameras = scene.behaviours(self, "MainCameraControl");
        if cameras.len() != 1 {
            return Err(format!(
                "main scene: {} MainCameraControl behaviours, expected 1",
                cameras.len()
            ));
        }
        let camera_ref = cameras.remove(0);
        let camera = sn_unity::MainCameraControl::parse(camera_ref.data()?.1, big_endian)
            .map_err(|e| format!("MainCameraControl: {e}"))?;
        let camera_node = behaviour_node(&camera_ref, "MainCameraControl")?;
        let view_model_node = node_of_component(camera.view_model, "MainCameraControl.viewModel")?;
        let camera_up_node = node_of_component(
            camera.camera_up_transform,
            "MainCameraControl.cameraUPTransform",
        )?;
        let camera_offset_node = node_of_component(
            camera.camera_offset_transform,
            "MainCameraControl.cameraOffsetTransform",
        )?;

        // The main camera: the scene's `AutoParent` moves its top-level
        // object under a player node.
        let mut parents = scene.behaviours(self, "AutoParent");
        if parents.len() != 1 {
            return Err(format!(
                "main scene: {} AutoParent behaviours, expected 1",
                parents.len()
            ));
        }
        let auto_ref = parents.remove(0);
        let auto = sn_unity::AutoParent::parse(auto_ref.data()?.1, big_endian)
            .map_err(|e| format!("AutoParent: {e}"))?;
        if !auto.make_locals_identity {
            return Err("AutoParent: makeLocalsIdentity is off".into());
        }
        let main_camera_parent =
            node_of_component(auto.parent_transform, "AutoParent.parentTransform")?;
        let (camera_root, camera_root_node) = scene
            .behaviour_node(self, &auto_ref)?
            .ok_or("AutoParent: object not in the main scene")?;
        if camera_root_node != 0 {
            return Err("AutoParent is not on a top-level object".into());
        }
        let main_camera_in_object = self
            .main_camera_node(&scene, camera_root)?
            .map(|n| scene.roots[camera_root].nodes[n].in_prefab)
            .ok_or("AutoParent's object holds no MainCamera-tagged camera")?;

        let cam_anchor_node = node_of_component(player.cam_anchor, "Player.camAnchor")?;
        let mut managers = scene.behaviours(self, "CameraToPlayerManager");
        if managers.len() != 1 {
            return Err(format!(
                "main scene: {} CameraToPlayerManager behaviours, expected 1",
                managers.len()
            ));
        }
        let manager_ref = managers.remove(0);
        behaviour_node(&manager_ref, "CameraToPlayerManager")?;
        let manager = sn_unity::CameraToPlayerManager::parse(manager_ref.data()?.1, big_endian)
            .map_err(|e| format!("CameraToPlayerManager: {e}"))?;
        let head_camera_node = node_of_component(
            manager.head_camera_bone,
            "CameraToPlayerManager.headCameraBone",
        )?;

        let mut body = PlayerBody {
            prefab,
            player,
            arms,
            animator_node,
            controller,
            head_node,
            view_model_node,
            camera_node,
            camera_up_node,
            camera_offset_node,
            camera,
            main_camera_parent,
            main_camera_in_object,
            slots,
            cam_anchor_node,
            head_camera_node,
        };
        body.equip(|_| TECH_TYPE_NONE);
        Ok(body)
    }

    /// The player's animator and its camera nodes ([`PlayerAnimation`]).
    pub fn player_animation(&self, body: &PlayerBody) -> Result<PlayerAnimation> {
        let mut cache = HashMap::new();
        let set = self.animation_set(&body.controller, &mut cache)?;
        if !set.errors.is_empty() {
            return Err(format!("player animation: {:?}", set.errors));
        }
        let program = Arc::new(Program::new(Arc::new(set.controller.clone()), &set.clips));
        let binding = body
            .prefab
            .bind_animator(body.animator_node, &program, &|_| Vec::new());
        if binding.missing > 0 {
            return Err(format!(
                "player animator: {} slots not in the hierarchy",
                binding.missing
            ));
        }
        let posed = |node: usize, what: &str| {
            body.prefab
                .posed_node(node, &program, &binding)
                .ok_or_else(|| format!("player: {what} not in the hierarchy"))
        };
        let cam_anchor = posed(body.cam_anchor_node, "camAnchor")?;
        let head_camera = posed(body.head_camera_node, "headCameraBone")?;
        let vm = &body.prefab.nodes[body.view_model_node];
        if vm.parent != Some(0)
            || vm.local.position != [0.0; 3]
            || vm.local.rotation != [0.0, 0.0, 0.0, 1.0]
        {
            return Err("player: the view model is not at the player's origin".into());
        }
        Ok(PlayerAnimation {
            program,
            defaults: binding.defaults,
            cam_anchor,
            head_camera,
            look_limits: (
                f64::from(body.camera.minimum_y),
                f64::from(body.camera.maximum_y),
            ),
        })
    }

    /// A material's shader name (`m_ParsedForm.m_Name`).
    pub fn shader_name(&self, material: &ObjectRef) -> Result<String> {
        let (_, data) = material.data()?;
        let m = sn_unity::Material::parse(data, material.file.file().big_endian)
            .map_err(|e| format!("Material: {e}"))?;
        let shader = self
            .resolve(&material.file, m.shader)?
            .ok_or("material without a shader")?;
        let (_, data) = shader.data()?;
        Ok(sn_unity::Shader::parse(data, shader.file.file().big_endian)
            .map_err(|e| format!("Shader: {e}"))?
            .name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots() -> Vec<EquipmentSlot> {
        vec![
            EquipmentSlot {
                slot: "Body".into(),
                default_node: Some(1),
                models: vec![(10, Some(2)), (11, Some(3)), (12, None)],
            },
            EquipmentSlot {
                slot: "Foots".into(),
                default_node: None,
                models: vec![(20, Some(4))],
            },
        ]
    }

    #[test]
    fn nothing_equipped_shows_the_defaults() {
        let c = equipment_changes(&slots(), |_| TECH_TYPE_NONE);
        assert_eq!(c, vec![(2, false), (3, false), (1, true), (4, false)]);
    }

    #[test]
    fn an_equipped_model_replaces_the_default() {
        let c = equipment_changes(&slots(), |s| if s == "Body" { 11 } else { 20 });
        assert_eq!(c, vec![(2, false), (3, true), (1, false), (4, true)]);
        // A matching model with a null reference still hides the default.
        let c = equipment_changes(&slots(), |s| if s == "Body" { 12 } else { 0 });
        assert_eq!(c, vec![(2, false), (3, false), (1, false), (4, false)]);
    }

    /// `sn_sim::body` sets exactly the listed parameters (the fire and
    /// explosion death triggers have no cause yet).
    #[test]
    fn sim_sets_the_listed_parameters() {
        use sn_sim::V3;
        use sn_sim::body::{Body, BodyFrame, BodyParams};
        let params = BodyParams {
            smooth_speed_under_water: 10.0,
            smooth_speed_above_water: 15.0,
            turn_animation_damp_time: 0.0,
            skin: 0.0,
            step_amount: 0.0,
            view_model_roll: 0.0,
            camera_up_position: V3::ZERO,
            camera_offset_position: V3::ZERO,
            ocean_level: 0.0,
        };
        let mut body = Body::new(&params);
        body.died();
        let frame = BodyFrame {
            dt: 0.02,
            time: 0.0,
            position: V3::ZERO,
            velocity: V3::ZERO,
            grounded: true,
            underwater: false,
            swimming: false,
            inside: false,
            look: Default::default(),
            strafe: 0.0,
            controls: true,
            bobbing: true,
        };
        let mut set: Vec<&str> = body
            .update(&params, &frame, &mut |_, _, _| false)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        set.sort_unstable();
        let mut listed: Vec<&str> = RULE_PARAMETERS
            .iter()
            .chain(FIXED_PARAMETERS)
            .copied()
            .filter(|n| !matches!(*n, "player_death_fire" | "player_death_explosion"))
            .collect();
        listed.sort_unstable();
        assert_eq!(set, listed);
    }

    #[test]
    fn parameter_lists_do_not_overlap() {
        for p in RULE_PARAMETERS {
            assert!(!FIXED_PARAMETERS.contains(p), "{p}");
        }
    }
}
