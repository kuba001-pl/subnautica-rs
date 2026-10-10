//! The player's body (M9g1, `docs/DESIGN.md` § 4.3 "M9g plan"): the
//! `Player` hierarchy of the main scene, the suit models the equipment rule
//! shows, the head, the camera's nodes, the animator and the numbers of
//! `ArmsController`. Facts in `docs/formats/gameplay.md` § The player's body.

use sn_unity::{ArmsController, PPtr, PlayerFields, SkinnedMeshRenderer};

use crate::{Assets, ObjectRef, Prefab, Result};

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
    /// The node `MainCameraControl` sits on, and its `cameraUPTransform`.
    pub camera_node: usize,
    pub camera_up_node: usize,
    pub slots: Vec<EquipmentSlot>,
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
            slots,
        };
        body.equip(|_| TECH_TYPE_NONE);
        Ok(body)
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

    #[test]
    fn parameter_lists_do_not_overlap() {
        for p in RULE_PARAMETERS {
            assert!(!FIXED_PARAMETERS.contains(p), "{p}");
        }
    }
}
