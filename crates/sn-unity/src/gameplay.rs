//! The game's scripts and ScriptableObjects that hold gameplay numbers
//! (Phase E, `docs/DESIGN.md` § 4.3): the player's components, `PDAData`,
//! `EntTechData`, `BreakableResource`. Fields in declaration order, Unity
//! 2019.4 serialization (no type trees): `bool` takes 4 bytes (aligned),
//! enums are `i32`, strings and arrays are aligned, object references are
//! PPtrs. Where a parser reads every field it requires the data to end with
//! the last one, which checks the layout on every object read. See
//! `docs/formats/gameplay.md`.

use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;
use crate::water_surface::AnimationCurve;
use crate::{ErrorKind, Result};

fn fields<'a>(data: &'a [u8], big_endian: bool) -> Result<Reader<'a>> {
    let header = MonoBehaviourHeader::parse(data, big_endian)?;
    let mut r = Reader::new(data, big_endian);
    r.seek(header.fields_offset)?;
    Ok(r)
}

fn at_end(r: &Reader, data: &[u8]) -> Result<()> {
    if r.pos() != data.len() {
        return Err(r.error(ErrorKind::Invalid(format!(
            "{} bytes after the last field",
            data.len() - r.pos()
        ))));
    }
    Ok(())
}

impl Reader<'_> {
    /// A serialized `bool`: one byte, then aligned to 4.
    fn bool4(&mut self) -> Result<bool> {
        let b = self.u8()? != 0;
        self.align(4)?;
        Ok(b)
    }

    fn i32_list(&mut self) -> Result<Vec<i32>> {
        let n = self.count(4)?;
        (0..n).map(|_| self.i32()).collect()
    }

    fn vector3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

/// One `EntTechData.Entry`: a prefab name (lower case) and its tech type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntTechEntry {
    pub prefab_name: String,
    pub tech_type: i32,
}

/// `EntTechData.entTechMap` (the resource `EntTechData`).
pub fn parse_ent_tech_data(data: &[u8], big_endian: bool) -> Result<Vec<EntTechEntry>> {
    let mut r = fields(data, big_endian)?;
    let n = r.count(8)?;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        entries.push(EntTechEntry {
            prefab_name: r.aligned_string()?,
            tech_type: r.i32()?,
        });
    }
    at_end(&r, data)?;
    Ok(entries)
}

/// `Story.StoryGoal`.
#[derive(Clone, Debug, PartialEq)]
pub struct StoryGoal {
    pub delay: f32,
    pub key: String,
    /// `Story.GoalType` value.
    pub goal_type: i32,
}

/// `KnownTech.AnalysisTech`: owning (or scanning) `tech_type` unlocks
/// `unlock_tech_types`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisTech {
    pub tech_type: i32,
    /// A language key.
    pub unlock_message: String,
    pub unlock_sound: PPtr,
    pub unlock_popup: PPtr,
    pub unlock_tech_types: Vec<i32>,
    pub story_goals: Vec<StoryGoal>,
}

/// `KnownTech.CompoundTech`: known once every dependency is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompoundTech {
    pub tech_type: i32,
    pub dependencies: Vec<i32>,
}

/// `PDAScanner.EntryData`: what scanning `key` does.
#[derive(Clone, Debug, PartialEq)]
pub struct ScannerEntry {
    pub key: i32,
    pub locked: bool,
    pub total_fragments: i32,
    pub destroy_after_scan: bool,
    /// The databank entry it unlocks (a key; empty for none).
    pub encyclopedia: String,
    pub blueprint: i32,
    pub scan_time: f32,
    pub is_fragment: bool,
}

/// `PDALog.EntryData`.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub key: String,
    /// `PDALog.EntryType` value.
    pub kind: i32,
    pub icon: PPtr,
    pub sound: PPtr,
    pub do_not_auto_play: bool,
}

/// `PDAEncyclopedia.EntryData` (the databank).
#[derive(Clone, Debug, PartialEq)]
pub struct EncyclopediaEntry {
    pub key: String,
    /// Tree path in the databank, `/`-separated.
    pub path: String,
    /// `PDAEncyclopedia.EntryData.Kind` value.
    pub kind: i32,
    pub unlocked: bool,
    pub popup: PPtr,
    pub image: PPtr,
    pub sound: PPtr,
    pub audio: PPtr,
}

/// The `PDAData` ScriptableObject (`Player.pdaData`).
#[derive(Clone, Debug, PartialEq)]
pub struct PdaData {
    pub default_log_icon: PPtr,
    pub log: Vec<LogEntry>,
    pub encyclopedia: Vec<EncyclopediaEntry>,
    pub scanner: Vec<ScannerEntry>,
    /// Blueprints known at the start of a new game.
    pub default_tech: Vec<i32>,
    pub analysis_tech: Vec<AnalysisTech>,
    pub compound_tech: Vec<CompoundTech>,
}

impl PdaData {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PdaData> {
        let mut r = fields(data, big_endian)?;
        let default_log_icon = PPtr::read(&mut r)?;

        let n = r.count(32)?;
        let mut log = Vec::with_capacity(n);
        for _ in 0..n {
            log.push(LogEntry {
                key: r.aligned_string()?,
                kind: r.i32()?,
                icon: PPtr::read(&mut r)?,
                sound: PPtr::read(&mut r)?,
                do_not_auto_play: r.bool4()?,
            });
        }

        let n = r.count(64)?;
        let mut encyclopedia = Vec::with_capacity(n);
        for _ in 0..n {
            encyclopedia.push(EncyclopediaEntry {
                key: r.aligned_string()?,
                path: r.aligned_string()?,
                kind: r.i32()?,
                unlocked: r.bool4()?,
                popup: PPtr::read(&mut r)?,
                image: PPtr::read(&mut r)?,
                sound: PPtr::read(&mut r)?,
                audio: PPtr::read(&mut r)?,
            });
        }

        let n = r.count(32)?;
        let mut scanner = Vec::with_capacity(n);
        for _ in 0..n {
            scanner.push(ScannerEntry {
                key: r.i32()?,
                locked: r.bool4()?,
                total_fragments: r.i32()?,
                destroy_after_scan: r.bool4()?,
                encyclopedia: r.aligned_string()?,
                blueprint: r.i32()?,
                scan_time: r.f32()?,
                is_fragment: r.bool4()?,
            });
        }

        let default_tech = r.i32_list()?;

        let n = r.count(40)?;
        let mut analysis_tech = Vec::with_capacity(n);
        for _ in 0..n {
            let tech_type = r.i32()?;
            let unlock_message = r.aligned_string()?;
            let unlock_sound = PPtr::read(&mut r)?;
            let unlock_popup = PPtr::read(&mut r)?;
            let unlock_tech_types = r.i32_list()?;
            let goals = r.count(12)?;
            let mut story_goals = Vec::with_capacity(goals);
            for _ in 0..goals {
                story_goals.push(StoryGoal {
                    delay: r.f32()?,
                    key: r.aligned_string()?,
                    goal_type: r.i32()?,
                });
            }
            analysis_tech.push(AnalysisTech {
                tech_type,
                unlock_message,
                unlock_sound,
                unlock_popup,
                unlock_tech_types,
                story_goals,
            });
        }

        let n = r.count(8)?;
        let mut compound_tech = Vec::with_capacity(n);
        for _ in 0..n {
            compound_tech.push(CompoundTech {
                tech_type: r.i32()?,
                dependencies: r.i32_list()?,
            });
        }
        at_end(&r, data)?;
        Ok(PdaData {
            default_log_icon,
            log,
            encyclopedia,
            scanner,
            default_tech,
            analysis_tech,
            compound_tech,
        })
    }
}

/// Skips a `GUIStyle` (a built-in struct): name, 8 `GUIStyleState`s
/// (background PPtr, text colour), 4 `RectOffset`s, font PPtr, font size,
/// font style, alignment, word wrap and rich text (two bytes, aligned),
/// text clipping, image position, content offset, fixed width and height,
/// stretch width and height (two bytes, aligned). Checked on the player:
/// the fields after it land on the values the code's initialisers give.
fn skip_gui_style(r: &mut Reader) -> Result<()> {
    r.aligned_string()?;
    for _ in 0..8 {
        PPtr::read(r)?;
        r.bytes(16)?;
    }
    r.bytes(4 * 16)?;
    PPtr::read(r)?;
    r.bytes(12)?;
    r.bytes(2)?;
    r.align(4)?;
    r.bytes(4 * 6)?;
    r.bytes(2)?;
    r.align(4)
}

/// `Player.EquipmentModel`: the model shown while `tech_type` is in the
/// slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipmentModel {
    pub tech_type: i32,
    /// A GameObject.
    pub model: PPtr,
}

/// `Player.EquipmentType`: an equipment slot's models
/// (`Player.EquipmentChanged`: the model whose tech type is in the slot is
/// active, the others not; `default_model` is active when none matched).
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentType {
    pub slot: String,
    /// A GameObject, or null.
    pub default_model: PPtr,
    pub equipment: Vec<EquipmentModel>,
}

/// The `Player` script's numbers and the parts of the body it points at.
/// Every serialized field is read (M9g1); its `Event<T>` and
/// `MonitoredValue<T>` fields are generic, which Unity 2019.4 does not
/// serialize.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerFields {
    /// `equipmentModels`, in order.
    pub equipment_models: Vec<EquipmentType>,
    pub movement_speed: f32,
    pub depth_level: f32,
    /// `head`: a SkinnedMeshRenderer, drawn only in shadows in first
    /// person (`SetHeadVisible`).
    pub head: PPtr,
    pub player_sphere_radius: f32,
    /// `camRoot` (an `SNCameraRoot`).
    pub cam_root: PPtr,
    /// `armsController` (an `ArmsController`).
    pub arms_controller: PPtr,
    /// `playerAnimator` (an `Animator`).
    pub player_animator: PPtr,
    pub crush_depth: f32,
    pub pda_data: PPtr,
    /// Seconds before passing out with no oxygen.
    pub suffocation_time: f32,
    /// Seconds to come back after suffocating.
    pub suffocation_recovery_time: f32,
    pub live_mixin: PPtr,
    pub ground_motor: PPtr,
    pub rigid_body: PPtr,
    /// `diveGoal`.
    pub dive_goal: StoryGoal,
    /// `leftHandBone` (a Transform).
    pub left_hand_bone: PPtr,
}

impl PlayerFields {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PlayerFields> {
        let mut r = fields(data, big_endian)?;
        PPtr::read(&mut r)?; // currentMountedVehicle
        PPtr::read(&mut r)?; // jumpSound
        let n = r.count(20)?;
        let mut equipment_models = Vec::with_capacity(n);
        for _ in 0..n {
            let slot = r.aligned_string()?;
            let default_model = PPtr::read(&mut r)?;
            let models = r.count(16)?;
            let mut equipment = Vec::with_capacity(models);
            for _ in 0..models {
                equipment.push(EquipmentModel {
                    tech_type: r.i32()?,
                    model: PPtr::read(&mut r)?,
                });
            }
            equipment_models.push(EquipmentType {
                slot,
                default_model,
                equipment,
            });
        }
        let movement_speed = r.f32()?;
        let depth_level = r.f32()?;
        let head = PPtr::read(&mut r)?;
        let player_sphere_radius = r.f32()?;
        let cam_root = PPtr::read(&mut r)?;
        // camAnchor, surfaceFXSpawn, temperatureDamage, scubaMaskModelSpawn,
        // fpParticleEmissionPoint, pda
        for _ in 0..6 {
            PPtr::read(&mut r)?;
        }
        let arms_controller = PPtr::read(&mut r)?;
        let player_animator = PPtr::read(&mut r)?;
        PPtr::read(&mut r)?; // playerArrowTransform
        PPtr::read(&mut r)?; // oxygenMgr
        skip_gui_style(&mut r)?; // textStyle
        let crush_depth = r.f32()?;
        let pda_data = PPtr::read(&mut r)?;
        for _ in 0..3 {
            PPtr::read(&mut r)?; // rightHandSlot, radiateSound, deathMusic
        }
        let suffocation_time = r.f32()?;
        let suffocation_recovery_time = r.f32()?;
        let live_mixin = PPtr::read(&mut r)?;
        PPtr::read(&mut r)?; // footStepSounds
        PPtr::read(&mut r)?; // acidLoopingSound
        let ground_motor = PPtr::read(&mut r)?;
        let rigid_body = PPtr::read(&mut r)?;
        PPtr::read(&mut r)?; // teleportingLoopSound
        PPtr::read(&mut r)?; // bottom
        let dive_goal = StoryGoal {
            delay: r.f32()?,
            key: r.aligned_string()?,
            goal_type: r.i32()?,
        };
        PPtr::read(&mut r)?; // guiHand
        PPtr::read(&mut r)?; // infectedMixin
        AnimationCurve::read(&mut r)?; // infectionRevealCurve
        AnimationCurve::read(&mut r)?; // infectionCureCurve
        PPtr::read(&mut r)?; // infectionRevealSound
        let left_hand_bone = PPtr::read(&mut r)?;
        at_end(&r, data)?;
        Ok(PlayerFields {
            equipment_models,
            movement_speed,
            depth_level,
            head,
            player_sphere_radius,
            cam_root,
            arms_controller,
            player_animator,
            crush_depth,
            pda_data,
            suffocation_time,
            suffocation_recovery_time,
            live_mixin,
            ground_motor,
            rigid_body,
            dive_goal,
            left_hand_bone,
        })
    }
}

/// `ArmsController` on the player's `player_view` (M9g1): the numbers of
/// its animation rules. Every serialized field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmsController {
    /// `smoothSpeedUnderWater` / `smoothSpeedAboveWater`: the rate of the
    /// `Vector3.Slerp` that smooths the velocity given to the animator
    /// (× `Time.deltaTime`).
    pub smooth_speed_under_water: f32,
    pub smooth_speed_above_water: f32,
    /// `turnAnimationDampTime`: the damp time of `view_turn`.
    pub turn_animation_damp_time: f32,
    pub left_aim_ik_transform: PPtr,
    pub right_aim_ik_transform: PPtr,
    pub look_target_transform: PPtr,
    pub attached_left_hand_target: PPtr,
    pub left_hand_elbow: PPtr,
    pub bleeder_attack_target: PPtr,
    /// `ikToggleTime`: seconds for the hands' IK weights to reach 0 or 1.
    pub ik_toggle_time: f32,
    pub left_hand_attach: PPtr,
    pub right_hand_attach: PPtr,
    pub left_hand: PPtr,
    pub right_hand: PPtr,
}

impl ArmsController {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<ArmsController> {
        let mut r = fields(data, big_endian)?;
        let a = ArmsController {
            smooth_speed_under_water: r.f32()?,
            smooth_speed_above_water: r.f32()?,
            turn_animation_damp_time: r.f32()?,
            left_aim_ik_transform: PPtr::read(&mut r)?,
            right_aim_ik_transform: PPtr::read(&mut r)?,
            look_target_transform: PPtr::read(&mut r)?,
            attached_left_hand_target: PPtr::read(&mut r)?,
            left_hand_elbow: PPtr::read(&mut r)?,
            bleeder_attack_target: PPtr::read(&mut r)?,
            ik_toggle_time: r.f32()?,
            left_hand_attach: PPtr::read(&mut r)?,
            right_hand_attach: PPtr::read(&mut r)?,
            left_hand: PPtr::read(&mut r)?,
            right_hand: PPtr::read(&mut r)?,
        };
        at_end(&r, data)?;
        Ok(a)
    }
}

/// `Oxygen`: an oxygen store (the player's own lungs, or a tank).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oxygen {
    pub oxygen_capacity: f32,
    pub is_player: bool,
}

impl Oxygen {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Oxygen> {
        let mut r = fields(data, big_endian)?;
        let oxygen = Oxygen {
            oxygen_capacity: r.f32()?,
            is_player: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(oxygen)
    }
}

/// `LiveMixin`: health. Every serialized field (its `Event<float>`
/// fields are not serialized: the data ends after `player`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveMixin {
    pub data: PPtr,
    pub health: f32,
    /// `startHealthPercent`: health on respawn is `maxHealth` × this
    /// (`LiveMixin.OnRespawn`).
    pub start_health_percent: f32,
    pub player: PPtr,
}

impl LiveMixin {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LiveMixin> {
        let mut r = fields(data, big_endian)?;
        let data_ptr = PPtr::read(&mut r)?;
        let health = r.f32()?;
        let start_health_percent = r.f32()?;
        PPtr::read(&mut r)?; // damageClip
        PPtr::read(&mut r)?; // deathClip
        let player = PPtr::read(&mut r)?;
        at_end(&r, data)?;
        Ok(LiveMixin {
            data: data_ptr,
            health,
            start_health_percent,
            player,
        })
    }
}

/// `MainCameraControl`: the first-person mouse look (M9c). Every field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainCameraControl {
    pub cam_pda_z_offset: f32,
    pub step_amount: f32,
    /// `minimumX` / `maximumX`: declared but not used by the code.
    pub minimum_x: f32,
    pub maximum_x: f32,
    /// `minimumY` / `maximumY`: the pitch limits, degrees (up positive).
    pub minimum_y: f32,
    pub maximum_y: f32,
    pub mouse_look_enabled: bool,
    /// `skin`: the camera sits this far below its parent.
    pub skin: f32,
    /// `viewModel`: the Transform turned by the camera's yaw and moved by
    /// its bob (M9g).
    pub view_model: PPtr,
    pub max_view_model_rotation: f32,
    pub max_view_model_movement: f32,
    pub camera_tilt_mod: f32,
    pub camera_offset_transform: PPtr,
    pub camera_up_transform: PPtr,
}

impl MainCameraControl {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MainCameraControl> {
        let mut r = fields(data, big_endian)?;
        let cam_pda_z_offset = r.f32()?;
        r.f32()?; // camPDAZStart (set in Awake)
        let step_amount = r.f32()?;
        let minimum_x = r.f32()?;
        let maximum_x = r.f32()?;
        let minimum_y = r.f32()?;
        let maximum_y = r.f32()?;
        let mouse_look_enabled = r.bool4()?;
        for _ in 0..4 {
            r.f32()?; // rotationY, rotationX, camRotationX, camRotationY
        }
        let skin = r.f32()?;
        let view_model = PPtr::read(&mut r)?;
        r.vector3()?; // cameraAngleMotion
        let max_view_model_rotation = r.f32()?;
        let max_view_model_movement = r.f32()?;
        let camera_tilt_mod = r.f32()?;
        let camera_offset_transform = PPtr::read(&mut r)?;
        let camera_up_transform = PPtr::read(&mut r)?;
        r.bool4()?; // _cinematicMode
        r.bool4()?; // lookAroundMode
        at_end(&r, data)?;
        Ok(MainCameraControl {
            cam_pda_z_offset,
            step_amount,
            minimum_x,
            maximum_x,
            minimum_y,
            maximum_y,
            mouse_look_enabled,
            skin,
            view_model,
            max_view_model_rotation,
            max_view_model_movement,
            camera_tilt_mod,
            camera_offset_transform,
            camera_up_transform,
        })
    }
}

/// `LiveMixinData` (a ScriptableObject shared by every `LiveMixin` of a
/// kind).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveMixinData {
    pub max_health: f32,
    pub min_damage_for_sound: f32,
    pub loop_effect_below_percent: f32,
    pub destroy_on_death: bool,
    pub weldable: bool,
    pub knifeable: bool,
    pub can_resurrect: bool,
    pub pass_damage_data_on_death: bool,
    pub broadcast_kill_on_death: bool,
    pub invincible_in_creative: bool,
}

impl LiveMixinData {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LiveMixinData> {
        let mut r = fields(data, big_endian)?;
        let max_health = r.f32()?;
        let min_damage_for_sound = r.f32()?;
        let loop_effect_below_percent = r.f32()?;
        for _ in 0..4 {
            PPtr::read(&mut r)?; // damage, death, electrical, looping effects
        }
        let d = LiveMixinData {
            max_health,
            min_damage_for_sound,
            loop_effect_below_percent,
            destroy_on_death: r.bool4()?,
            weldable: r.bool4()?,
            knifeable: r.bool4()?,
            can_resurrect: r.bool4()?,
            pass_damage_data_on_death: r.bool4()?,
            broadcast_kill_on_death: r.bool4()?,
            invincible_in_creative: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(d)
    }
}

/// The fields of `PlayerMotor`, the base of `UnderwaterMotor` and
/// `GroundMotor`. `PlayerController` overwrites the speeds when the motor
/// mode changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerMotor {
    pub using_gravity: bool,
    pub can_control: bool,
    pub forward_max_speed: f32,
    pub backward_max_speed: f32,
    pub strafe_max_speed: f32,
    pub vertical_max_speed: f32,
    pub climb_speed: f32,
    pub gravity: f32,
    pub underwater_gravity: f32,
    pub can_swim: bool,
    pub forward_sprint_modifier: f32,
    pub strafe_sprint_modifier: f32,
    pub swim_drag: f32,
    pub ground_drag: f32,
    pub air_drag: f32,
    pub ladder_drag: f32,
    pub ladder_acceleration: f32,
    pub water_acceleration: f32,
    pub ground_acceleration: f32,
    pub air_acceleration: f32,
    pub can_jump: bool,
    pub jump_height: f32,
}

impl PlayerMotor {
    fn read(r: &mut Reader) -> Result<PlayerMotor> {
        PPtr::read(r)?; // rb
        PPtr::read(r)?; // playerController
        Ok(PlayerMotor {
            using_gravity: r.bool4()?,
            can_control: r.bool4()?,
            forward_max_speed: r.f32()?,
            backward_max_speed: r.f32()?,
            strafe_max_speed: r.f32()?,
            vertical_max_speed: r.f32()?,
            climb_speed: r.f32()?,
            gravity: r.f32()?,
            underwater_gravity: r.f32()?,
            can_swim: r.bool4()?,
            forward_sprint_modifier: r.f32()?,
            strafe_sprint_modifier: r.f32()?,
            swim_drag: r.f32()?,
            ground_drag: r.f32()?,
            air_drag: r.f32()?,
            ladder_drag: r.f32()?,
            ladder_acceleration: r.f32()?,
            water_acceleration: r.f32()?,
            ground_acceleration: r.f32()?,
            air_acceleration: r.f32()?,
            can_jump: r.bool4()?,
            jump_height: r.f32()?,
        })
    }

    /// The base fields of any `PlayerMotor` (e.g. `GroundMotor`, whose own
    /// fields are not read).
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PlayerMotor> {
        PlayerMotor::read(&mut fields(data, big_endian)?)
    }
}

/// `UnderwaterMotor`: swimming.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderwaterMotor {
    pub motor: PlayerMotor,
    pub fast_swim_mode: bool,
    pub capsule_collider: PPtr,
    pub player_speed_modifier: f32,
}

impl UnderwaterMotor {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<UnderwaterMotor> {
        let mut r = fields(data, big_endian)?;
        let motor = PlayerMotor::read(&mut r)?;
        r.vector3()?; // vel
        let m = UnderwaterMotor {
            motor,
            fast_swim_mode: r.bool4()?,
            capsule_collider: PPtr::read(&mut r)?,
            player_speed_modifier: r.f32()?,
        };
        at_end(&r, data)?;
        Ok(m)
    }
}

/// `GroundMotor`: walking (Unity's `CharacterMotor` script, adapted). The
/// `PlayerMotor` fields, then its own serialized classes in declaration
/// order (`[NonSerialized]` and private fields are not stored).
#[derive(Clone, Debug, PartialEq)]
pub struct GroundMotor {
    pub motor: PlayerMotor,
    /// `movement`.
    pub max_forward_speed: f32,
    pub max_sideways_speed: f32,
    pub max_backwards_speed: f32,
    /// Speed factor by slope angle in degrees (negative: downhill).
    pub slope_speed_multiplier: AnimationCurve,
    pub max_fall_speed: f32,
    /// `jumping`.
    pub jump_enabled: bool,
    pub jump_base_height: f32,
    pub jump_extra_height: f32,
    pub jump_perp_amount: f32,
    pub jump_steep_perp_amount: f32,
    /// `movingPlatform`.
    pub moving_platform_enabled: bool,
    pub movement_transfer: i32,
    /// `sliding`.
    pub sliding_enabled: bool,
    pub sliding_speed: f32,
    pub sliding_sideways_control: f32,
    pub sliding_speed_control: f32,
    /// `controllerSetup`.
    pub step_offset: f32,
    pub slope_limit: f32,
    /// `floatingModeSetup` (a debug mode).
    pub floating_gravity: f32,
    pub floating_air_acceleration: f32,
    pub floating_max_fall_speed: f32,
    pub floating_jump_height: f32,
    pub min_wind_speed_to_affect_movement: f32,
    pub percent_wind_dampening_on_ground: f32,
    pub percent_wind_dampening_in_air: f32,
    pub fly_cheat_enabled: bool,
}

impl GroundMotor {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<GroundMotor> {
        let mut r = fields(data, big_endian)?;
        let motor = PlayerMotor::read(&mut r)?;
        PPtr::read(&mut r)?; // controller
        let m = GroundMotor {
            motor,
            max_forward_speed: r.f32()?,
            max_sideways_speed: r.f32()?,
            max_backwards_speed: r.f32()?,
            slope_speed_multiplier: AnimationCurve::read(&mut r)?,
            max_fall_speed: r.f32()?,
            jump_enabled: r.bool4()?,
            jump_base_height: r.f32()?,
            jump_extra_height: r.f32()?,
            jump_perp_amount: r.f32()?,
            jump_steep_perp_amount: r.f32()?,
            moving_platform_enabled: r.bool4()?,
            movement_transfer: r.i32()?,
            sliding_enabled: r.bool4()?,
            sliding_speed: r.f32()?,
            sliding_sideways_control: r.f32()?,
            sliding_speed_control: r.f32()?,
            step_offset: r.f32()?,
            slope_limit: r.f32()?,
            floating_gravity: r.f32()?,
            floating_air_acceleration: r.f32()?,
            floating_max_fall_speed: r.f32()?,
            floating_jump_height: r.f32()?,
            min_wind_speed_to_affect_movement: r.f32()?,
            percent_wind_dampening_on_ground: r.f32()?,
            percent_wind_dampening_in_air: r.f32()?,
            fly_cheat_enabled: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(m)
    }
}

/// `UseableDiveHatch` (a `HandTarget`, which stores no fields): where the
/// hatch puts the player when leaving (`outsideExit`) and entering
/// (`insideSpawn`).
#[derive(Clone, Debug, PartialEq)]
pub struct UseableDiveHatch {
    pub outside_exit: PPtr,
    pub inside_spawn: PPtr,
    pub ignore_object: PPtr,
    pub exit_search_distance: f32,
    pub enter_custom_text: String,
    pub exit_custom_text: String,
    pub enter_custom_goal_text: String,
    pub custom_goal_with_loot_only: bool,
    pub enter_only: bool,
    pub is_for_escape_pod: bool,
    pub is_for_water_park: bool,
    pub secure_inventory: bool,
    pub enter_cinematic_controller: PPtr,
    pub exit_cinematic_controller: PPtr,
}

impl UseableDiveHatch {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<UseableDiveHatch> {
        let mut r = fields(data, big_endian)?;
        let h = UseableDiveHatch {
            outside_exit: PPtr::read(&mut r)?,
            inside_spawn: PPtr::read(&mut r)?,
            ignore_object: PPtr::read(&mut r)?,
            exit_search_distance: r.f32()?,
            enter_custom_text: r.aligned_string()?,
            exit_custom_text: r.aligned_string()?,
            enter_custom_goal_text: r.aligned_string()?,
            custom_goal_with_loot_only: r.bool4()?,
            enter_only: r.bool4()?,
            is_for_escape_pod: r.bool4()?,
            is_for_water_park: r.bool4()?,
            secure_inventory: r.bool4()?,
            enter_cinematic_controller: PPtr::read(&mut r)?,
            exit_cinematic_controller: PPtr::read(&mut r)?,
        };
        at_end(&r, data)?;
        Ok(h)
    }
}

/// One persistent listener of a serialized `UnityEvent` (2019.4 layout:
/// target, method name, mode, arguments, call state).
#[derive(Clone, Debug, PartialEq)]
pub struct PersistentCall {
    pub target: PPtr,
    pub method_name: String,
    pub mode: i32,
    pub object_argument: PPtr,
    pub object_argument_type: String,
    pub int_argument: i32,
    pub float_argument: f32,
    pub string_argument: String,
    pub bool_argument: bool,
    pub call_state: i32,
}

/// A serialized `UnityEvent`'s persistent calls.
fn unity_event(r: &mut Reader) -> Result<Vec<PersistentCall>> {
    let n = r.count(40)?;
    let mut calls = Vec::with_capacity(n);
    for _ in 0..n {
        calls.push(PersistentCall {
            target: PPtr::read(r)?,
            method_name: r.aligned_string()?,
            mode: r.i32()?,
            object_argument: PPtr::read(r)?,
            object_argument_type: r.aligned_string()?,
            int_argument: r.i32()?,
            float_argument: r.f32()?,
            string_argument: r.aligned_string()?,
            bool_argument: r.bool4()?,
            call_state: r.i32()?,
        });
    }
    Ok(calls)
}

/// `CinematicModeTrigger` (`CinematicModeTriggerBase` fields, then
/// `handText`): starts a cinematic when used by hand (`trigger_type` 0) or
/// when the player enters or leaves its trigger volume (1).
#[derive(Clone, Debug, PartialEq)]
pub struct CinematicModeTrigger {
    pub trigger_type: i32,
    /// 0 on enter, 1 on exit.
    pub volume_trigger_type: i32,
    pub show_icon_on_hand_hover: bool,
    pub cinematic_controller: PPtr,
    pub secure_inventory: bool,
    pub restore_active_quick_slot: bool,
    pub on_cinematic_start: Vec<PersistentCall>,
    pub on_cinematic_end: Vec<PersistentCall>,
    pub debug: bool,
    pub hand_text: String,
}

impl CinematicModeTrigger {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<CinematicModeTrigger> {
        let mut r = fields(data, big_endian)?;
        let t = CinematicModeTrigger {
            trigger_type: r.i32()?,
            volume_trigger_type: r.i32()?,
            show_icon_on_hand_hover: r.bool4()?,
            cinematic_controller: PPtr::read(&mut r)?,
            secure_inventory: r.bool4()?,
            restore_active_quick_slot: r.bool4()?,
            on_cinematic_start: unity_event(&mut r)?,
            on_cinematic_end: unity_event(&mut r)?,
            debug: r.bool4()?,
            hand_text: r.aligned_string()?,
        };
        at_end(&r, data)?;
        Ok(t)
    }
}

/// `PlayerCinematicController`: moves the player along an animated
/// transform; at the end, to `endTransform` if set (else the player stays
/// where the animation left it).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerCinematicController {
    pub animated_transform: PPtr,
    pub inform_game_object: PPtr,
    pub end_transform: PPtr,
    pub only_use_end_transform_in_vr: bool,
    pub play_in_vr: bool,
    pub interrupt_auto_move: bool,
    pub player_view_animation_name: String,
    pub player_view_interpolate_anim_param: String,
    pub anim_param: String,
    pub interpolate_anim_param: String,
    pub interpolation_time: f32,
    pub interpolation_time_out: f32,
    pub receivers_anim_param: String,
    pub anim_param_receivers: Vec<PPtr>,
    pub interpolate_during_animation: bool,
    pub debug: bool,
    pub animator: PPtr,
    pub sound: PPtr,
    pub enforce_cinematic_mode_end: bool,
}

impl PlayerCinematicController {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PlayerCinematicController> {
        let mut r = fields(data, big_endian)?;
        let animated_transform = PPtr::read(&mut r)?;
        let inform_game_object = PPtr::read(&mut r)?;
        let end_transform = PPtr::read(&mut r)?;
        let only_use_end_transform_in_vr = r.bool4()?;
        let play_in_vr = r.bool4()?;
        let interrupt_auto_move = r.bool4()?;
        let player_view_animation_name = r.aligned_string()?;
        let player_view_interpolate_anim_param = r.aligned_string()?;
        let anim_param = r.aligned_string()?;
        let interpolate_anim_param = r.aligned_string()?;
        let interpolation_time = r.f32()?;
        let interpolation_time_out = r.f32()?;
        let receivers_anim_param = r.aligned_string()?;
        let n = r.count(12)?;
        let mut anim_param_receivers = Vec::with_capacity(n);
        for _ in 0..n {
            anim_param_receivers.push(PPtr::read(&mut r)?);
        }
        let c = PlayerCinematicController {
            animated_transform,
            inform_game_object,
            end_transform,
            only_use_end_transform_in_vr,
            play_in_vr,
            interrupt_auto_move,
            player_view_animation_name,
            player_view_interpolate_anim_param,
            anim_param,
            interpolate_anim_param,
            interpolation_time,
            interpolation_time_out,
            receivers_anim_param,
            anim_param_receivers,
            interpolate_during_animation: r.bool4()?,
            debug: r.bool4()?,
            animator: PPtr::read(&mut r)?,
            sound: PPtr::read(&mut r)?,
            enforce_cinematic_mode_end: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(c)
    }
}

/// `AutoParent`: on `Start` its object is put under `parent_transform`,
/// with an identity local placement if `make_locals_identity` (M9g2: how
/// the main camera follows the player). Every field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoParent {
    pub parent_transform: PPtr,
    pub make_locals_identity: bool,
}

impl AutoParent {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<AutoParent> {
        let mut r = fields(data, big_endian)?;
        let a = AutoParent {
            parent_transform: PPtr::read(&mut r)?,
            make_locals_identity: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(a)
    }
}

/// `EscapePodFirstUseCinematicsController`: which lifepod hatch triggers
/// are the first-use ones (13 object references).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EscapePodFirstUse {
    pub escape_pod: PPtr,
    pub escape_pod_rigidbody: PPtr,
    pub bottom: PPtr,
    pub bottom_first_use: PPtr,
    pub top: PPtr,
    pub top_first_use: PPtr,
    /// The creatures its first-use cinematics spawn and where.
    pub creatures: [PPtr; 7],
}

impl EscapePodFirstUse {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<EscapePodFirstUse> {
        let mut r = fields(data, big_endian)?;
        let mut p = || PPtr::read(&mut r);
        let e = EscapePodFirstUse {
            escape_pod: p()?,
            escape_pod_rigidbody: p()?,
            bottom: p()?,
            bottom_first_use: p()?,
            top: p()?,
            top_first_use: p()?,
            creatures: [p()?, p()?, p()?, p()?, p()?, p()?, p()?],
        };
        at_end(&r, data)?;
        Ok(e)
    }
}

/// `PlayerController`: switches motors and sets their speeds per motor
/// mode (swim, Seaglide, walk/run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerController {
    pub stand_height: f32,
    pub swim_height: f32,
    pub camera_offset: f32,
    pub underwater_controller: PPtr,
    pub ground_controller: PPtr,
    pub controller_radius: f32,
    pub default_swim_drag: f32,
    pub swim_forward_max_speed: f32,
    pub swim_backward_max_speed: f32,
    pub swim_strafe_max_speed: f32,
    pub swim_vertical_max_speed: f32,
    pub swim_water_acceleration: f32,
    pub seaglide_forward_max_speed: f32,
    pub seaglide_backward_max_speed: f32,
    pub seaglide_strafe_max_speed: f32,
    pub seaglide_vertical_max_speed: f32,
    pub seaglide_water_acceleration: f32,
    pub seaglide_swim_drag: f32,
    pub walk_run_forward_max_speed: f32,
    pub walk_run_backward_max_speed: f32,
    pub walk_run_strafe_max_speed: f32,
    pub default_camera_minimum_y: f32,
    pub walk_run_camera_minimum_y: f32,
}

impl PlayerController {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PlayerController> {
        let mut r = fields(data, big_endian)?;
        for _ in 0..3 {
            PPtr::read(&mut r)?; // player, useRigidbody, cameraControl
        }
        let stand_height = r.f32()?;
        let swim_height = r.f32()?;
        let camera_offset = r.f32()?;
        r.vector3()?; // velocity
        let underwater_controller = PPtr::read(&mut r)?;
        let ground_controller = PPtr::read(&mut r)?;
        PPtr::read(&mut r)?; // activeController
        let controller_radius = r.f32()?;
        let default_swim_drag = r.f32()?;
        r.bool4()?; // inputEnabled
        let c = PlayerController {
            stand_height,
            swim_height,
            camera_offset,
            underwater_controller,
            ground_controller,
            controller_radius,
            default_swim_drag,
            swim_forward_max_speed: r.f32()?,
            swim_backward_max_speed: r.f32()?,
            swim_strafe_max_speed: r.f32()?,
            swim_vertical_max_speed: r.f32()?,
            swim_water_acceleration: r.f32()?,
            seaglide_forward_max_speed: r.f32()?,
            seaglide_backward_max_speed: r.f32()?,
            seaglide_strafe_max_speed: r.f32()?,
            seaglide_vertical_max_speed: r.f32()?,
            seaglide_water_acceleration: r.f32()?,
            seaglide_swim_drag: r.f32()?,
            walk_run_forward_max_speed: r.f32()?,
            walk_run_backward_max_speed: r.f32()?,
            walk_run_strafe_max_speed: r.f32()?,
            default_camera_minimum_y: r.f32()?,
            walk_run_camera_minimum_y: r.f32()?,
        };
        at_end(&r, data)?;
        Ok(c)
    }
}

/// `BreakableResource.RandomPrefab`: one roll of what an outcrop drops.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomPrefab {
    /// The prefab's asset GUID (an Addressables key).
    pub prefab: String,
    pub tech_type: i32,
    pub chance: f32,
}

/// `BreakableResource`: an outcrop that breaks into resources.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakableResource {
    pub prefab_list: Vec<RandomPrefab>,
    pub default_prefab: String,
    pub default_tech_type: i32,
    pub vertical_spawn_offset: f32,
    pub num_chances: i32,
    pub hits_to_break: i32,
    /// A language key.
    pub break_text: String,
    pub custom_goal_text: String,
}

/// An `AssetReference`: the asset GUID, then the sub-object name and type
/// (both empty on every reference seen).
fn asset_reference(r: &mut Reader) -> Result<String> {
    let guid = r.aligned_string()?;
    r.aligned_string()?;
    r.aligned_string()?;
    Ok(guid)
}

impl BreakableResource {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<BreakableResource> {
        let mut r = fields(data, big_endian)?;
        let n = r.count(20)?;
        let mut prefab_list = Vec::with_capacity(n);
        for _ in 0..n {
            prefab_list.push(RandomPrefab {
                prefab: asset_reference(&mut r)?,
                tech_type: r.i32()?,
                chance: r.f32()?,
            });
        }
        let default_prefab = asset_reference(&mut r)?;
        let default_tech_type = r.i32()?;
        let vertical_spawn_offset = r.f32()?;
        let num_chances = r.i32()?;
        let hits_to_break = r.i32()?;
        for _ in 0..4 {
            PPtr::read(&mut r)?; // hitSound, breakSound, hitFX, breakFX
        }
        let b = BreakableResource {
            prefab_list,
            default_prefab,
            default_tech_type,
            vertical_spawn_offset,
            num_chances,
            hits_to_break,
            break_text: r.aligned_string()?,
            custom_goal_text: r.aligned_string()?,
        };
        at_end(&r, data)?;
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian writer for synthetic objects.
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        /// A MonoBehaviour header with an empty name.
        fn behaviour() -> W {
            let mut w = W::default();
            w.pptr(0, 1).u8(1).align().pptr(1, 2).string("");
            w
        }
        fn u8(&mut self, v: u8) -> &mut W {
            self.0.push(v);
            self
        }
        fn align(&mut self) -> &mut W {
            self.0.resize(self.0.len().next_multiple_of(4), 0);
            self
        }
        fn bool4(&mut self, v: bool) -> &mut W {
            self.u8(u8::from(v)).align()
        }
        fn i32(&mut self, v: i32) -> &mut W {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f32(&mut self, v: f32) -> &mut W {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn pptr(&mut self, file: i32, path: i64) -> &mut W {
            self.i32(file);
            self.0.extend_from_slice(&path.to_le_bytes());
            self
        }
        fn string(&mut self, s: &str) -> &mut W {
            self.i32(s.len() as i32);
            self.0.extend_from_slice(s.as_bytes());
            self.align()
        }
        fn ints(&mut self, v: &[i32]) -> &mut W {
            self.i32(v.len() as i32);
            for &x in v {
                self.i32(x);
            }
            self
        }
    }

    /// Every prefix is an error and no byte change panics.
    fn robust<T>(data: &[u8], parse: impl Fn(&[u8]) -> Result<T>) {
        for i in 0..data.len() {
            assert!(parse(&data[..i]).is_err(), "prefix {i} parsed");
            let mut bad = data.to_vec();
            bad[i] = 0xff;
            let _ = parse(&bad);
        }
    }

    #[test]
    fn ent_tech_data() {
        let mut w = W::behaviour();
        w.i32(2).string("abc").i32(7).string("longer-name").i32(-1);
        let e = parse_ent_tech_data(&w.0, false).unwrap();
        assert_eq!(
            e,
            vec![
                EntTechEntry {
                    prefab_name: "abc".into(),
                    tech_type: 7
                },
                EntTechEntry {
                    prefab_name: "longer-name".into(),
                    tech_type: -1
                },
            ]
        );
        w.u8(0);
        assert!(parse_ent_tech_data(&w.0, false).is_err(), "trailing byte");
        robust(&w.0[..w.0.len() - 1], |d| parse_ent_tech_data(d, false));
    }

    fn pda_sample() -> Vec<u8> {
        let mut w = W::behaviour();
        w.pptr(0, 5);
        // log
        w.i32(1)
            .string("Log1")
            .i32(2)
            .pptr(0, 6)
            .pptr(0, 7)
            .bool4(true);
        // encyclopedia
        w.i32(1)
            .string("Ency")
            .string("Tech/Basic")
            .i32(1)
            .bool4(false)
            .pptr(0, 0)
            .pptr(0, 8)
            .pptr(0, 0)
            .pptr(0, 0);
        // scanner
        w.i32(1)
            .i32(100)
            .bool4(true)
            .i32(3)
            .bool4(false)
            .string("Ency")
            .i32(101)
            .f32(2.5)
            .bool4(true);
        w.ints(&[1, 2, 3]);
        // analysis tech
        w.i32(1)
            .i32(40)
            .string("NotificationBlueprintUnlocked")
            .pptr(0, 9)
            .pptr(0, 10)
            .ints(&[41, 42]);
        w.i32(1).f32(1.5).string("Goal").i32(3);
        // compound tech
        w.i32(1).i32(50).ints(&[51, 52]);
        w.0
    }

    #[test]
    fn pda_data() {
        let data = pda_sample();
        let p = PdaData::parse(&data, false).unwrap();
        assert_eq!(p.default_log_icon.path_id, 5);
        assert_eq!(p.log.len(), 1);
        assert_eq!(p.log[0].kind, 2);
        assert!(p.log[0].do_not_auto_play);
        assert_eq!(p.encyclopedia[0].path, "Tech/Basic");
        assert_eq!(p.encyclopedia[0].image.path_id, 8);
        let s = &p.scanner[0];
        assert_eq!((s.key, s.total_fragments, s.blueprint), (100, 3, 101));
        assert!(s.locked && !s.destroy_after_scan && s.is_fragment);
        assert_eq!(s.scan_time, 2.5);
        assert_eq!(p.default_tech, vec![1, 2, 3]);
        let a = &p.analysis_tech[0];
        assert_eq!(a.tech_type, 40);
        assert_eq!(a.unlock_tech_types, vec![41, 42]);
        assert_eq!(
            a.story_goals,
            vec![StoryGoal {
                delay: 1.5,
                key: "Goal".into(),
                goal_type: 3
            }]
        );
        assert_eq!(
            p.compound_tech,
            vec![CompoundTech {
                tech_type: 50,
                dependencies: vec![51, 52]
            }]
        );
        robust(&data, |d| PdaData::parse(d, false));
    }

    fn player_sample() -> Vec<u8> {
        let mut w = W::behaviour();
        w.pptr(0, 0).pptr(1, 3);
        w.i32(2);
        w.string("Body").pptr(0, 4).i32(1).i32(9).pptr(0, 5);
        w.string("Head").pptr(0, 0).i32(0);
        w.f32(1.0).f32(2.0).pptr(0, 6).f32(0.5);
        for i in 0..11 {
            w.pptr(0, 10 + i);
        }
        // GUIStyle: name, states, offsets, font, ints, bools, …
        w.string("style");
        for _ in 0..8 {
            w.pptr(0, 0).f32(1.0).f32(1.0).f32(1.0).f32(1.0);
        }
        for _ in 0..16 {
            w.i32(0);
        }
        w.pptr(2, 77).i32(18).i32(0).i32(4).u8(0).u8(1).align();
        w.i32(0).i32(0).f32(0.0).f32(0.0).f32(0.0).f32(0.0);
        w.u8(1).u8(0).align();
        w.f32(-1.0).pptr(1, 0xde0);
        w.pptr(0, 30).pptr(0, 31).pptr(0, 32);
        w.f32(8.0).f32(4.0);
        w.pptr(0, 40)
            .pptr(0, 41)
            .pptr(0, 42)
            .pptr(0, 43)
            .pptr(0, 44);
        w.pptr(0, 45).pptr(0, 46);
        w.f32(0.0).string("Dive").i32(3);
        w.pptr(0, 47);
        // infectedMixin, two AnimationCurves (one key, none), the reveal
        // sound, leftHandBone
        w.pptr(0, 48);
        w.i32(1);
        for v in [0.0, 1.0, 0.0, 0.0] {
            w.f32(v);
        }
        w.i32(0).f32(0.33).f32(0.33).i32(2).i32(2).i32(4);
        w.i32(0).i32(2).i32(2).i32(4);
        w.pptr(0, 49).pptr(0, 50);
        w.0
    }

    #[test]
    fn player_fields() {
        let data = player_sample();
        let p = PlayerFields::parse(&data, false).unwrap();
        assert_eq!(
            p.equipment_models,
            vec![
                EquipmentType {
                    slot: "Body".into(),
                    default_model: PPtr {
                        file_id: 0,
                        path_id: 4
                    },
                    equipment: vec![EquipmentModel {
                        tech_type: 9,
                        model: PPtr {
                            file_id: 0,
                            path_id: 5
                        }
                    }],
                },
                EquipmentType {
                    slot: "Head".into(),
                    default_model: PPtr {
                        file_id: 0,
                        path_id: 0
                    },
                    equipment: vec![],
                },
            ]
        );
        assert_eq!(p.head.path_id, 6);
        assert_eq!(p.cam_root.path_id, 10);
        assert_eq!(p.arms_controller.path_id, 17);
        assert_eq!(p.player_animator.path_id, 18);
        assert_eq!(p.left_hand_bone.path_id, 50);
        assert_eq!((p.movement_speed, p.depth_level), (1.0, 2.0));
        assert_eq!(p.player_sphere_radius, 0.5);
        assert_eq!(p.crush_depth, -1.0);
        assert_eq!(
            p.pda_data,
            PPtr {
                file_id: 1,
                path_id: 0xde0
            }
        );
        assert_eq!(
            (p.suffocation_time, p.suffocation_recovery_time),
            (8.0, 4.0)
        );
        assert_eq!(p.live_mixin.path_id, 40);
        assert_eq!(p.ground_motor.path_id, 43);
        assert_eq!(p.rigid_body.path_id, 44);
        assert_eq!(p.dive_goal.key, "Dive");
        robust(&data, |d| PlayerFields::parse(d, false));
        // A byte more is an error: the layout is checked to the end.
        let mut longer = data.clone();
        longer.extend_from_slice(&[0; 4]);
        assert!(PlayerFields::parse(&longer, false).is_err());
    }

    #[test]
    fn arms_controller() {
        let mut w = W::behaviour();
        w.f32(4.0).f32(8.0).f32(0.1);
        for i in 0..6 {
            w.pptr(0, 20 + i);
        }
        w.f32(0.5);
        for i in 0..4 {
            w.pptr(0, 30 + i);
        }
        let a = ArmsController::parse(&w.0, false).unwrap();
        assert_eq!(
            (
                a.smooth_speed_under_water,
                a.smooth_speed_above_water,
                a.turn_animation_damp_time,
                a.ik_toggle_time
            ),
            (4.0, 8.0, 0.1, 0.5)
        );
        assert_eq!(a.look_target_transform.path_id, 22);
        assert_eq!(a.bleeder_attack_target.path_id, 25);
        assert_eq!(a.right_hand.path_id, 33);
        robust(&w.0, |d| ArmsController::parse(d, false));
        w.u8(0);
        assert!(ArmsController::parse(&w.0, false).is_err());
    }

    #[test]
    fn main_camera_control() {
        let mut w = W::behaviour();
        w.f32(0.18).f32(0.0).f32(0.0);
        w.f32(-360.0).f32(360.0).f32(-87.0).f32(87.0).bool4(true);
        w.f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0);
        w.pptr(0, 5).f32(0.0).f32(0.0).f32(0.0);
        w.f32(10.0).f32(5.0).f32(0.1).pptr(0, 6).pptr(0, 7);
        w.bool4(false).bool4(false);
        let c = MainCameraControl::parse(&w.0, false).unwrap();
        assert_eq!((c.minimum_y, c.maximum_y, c.skin), (-87.0, 87.0, 0.0));
        assert!(c.mouse_look_enabled);
        assert_eq!(c.camera_up_transform.path_id, 7);
        assert_eq!(c.view_model.path_id, 5);
        robust(&w.0, |d| MainCameraControl::parse(d, false));
        // A byte more is an error: the layout is checked to the end.
        w.u8(0);
        assert!(MainCameraControl::parse(&w.0, false).is_err());
    }

    #[test]
    fn oxygen_and_health() {
        let mut w = W::behaviour();
        w.f32(45.0).bool4(true);
        let o = Oxygen::parse(&w.0, false).unwrap();
        assert_eq!(o.oxygen_capacity, 45.0);
        assert!(o.is_player);
        robust(&w.0, |d| Oxygen::parse(d, false));

        let mut w = W::behaviour();
        w.pptr(1, 9)
            .f32(100.0)
            .f32(0.5)
            .pptr(0, 0)
            .pptr(0, 0)
            .pptr(0, 40);
        let l = LiveMixin::parse(&w.0, false).unwrap();
        assert_eq!((l.data.path_id, l.health), (9, 100.0));
        assert_eq!((l.start_health_percent, l.player.path_id), (0.5, 40));
        robust(&w.0, |d| LiveMixin::parse(d, false));

        let mut w = W::behaviour();
        w.f32(100.0).f32(5.0).f32(0.2);
        for _ in 0..4 {
            w.pptr(0, 0);
        }
        for b in [false, true, false, true, false, false, true] {
            w.bool4(b);
        }
        let d = LiveMixinData::parse(&w.0, false).unwrap();
        assert_eq!(d.max_health, 100.0);
        assert!(d.weldable && d.can_resurrect && d.invincible_in_creative);
        assert!(!d.destroy_on_death && !d.knifeable);
        robust(&w.0, |d| LiveMixinData::parse(d, false));
    }

    fn motor(w: &mut W) {
        w.pptr(0, 1).pptr(0, 2).bool4(true).bool4(true);
        for v in 1..=7 {
            w.f32(v as f32);
        }
        w.bool4(true);
        for v in 8..=17 {
            w.f32(v as f32);
        }
        w.bool4(false).f32(18.0);
    }

    #[test]
    fn ground_motor() {
        let mut w = W::behaviour();
        motor(&mut w);
        w.pptr(0, 9); // controller
        w.f32(10.0).f32(10.0).f32(10.0);
        // slopeSpeedMultiplier: two keys, then pre/post infinity and rotation order.
        w.i32(2);
        for (t, v) in [(-90.0, 1.0), (90.0, 0.0)] {
            w.f32(t).f32(v).f32(0.0).f32(0.0).i32(0).f32(0.33).f32(0.33);
        }
        w.i32(2).i32(2).i32(4);
        w.f32(50.0); // maxFallSpeed
        w.bool4(true).f32(1.0).f32(4.1).f32(0.0).f32(0.5);
        w.bool4(true).i32(2);
        w.bool4(true).f32(7.0).f32(1.0).f32(0.2);
        w.f32(0.4).f32(60.0);
        w.f32(8.0).f32(10.0).f32(3.0).f32(2.0);
        w.f32(15.0).f32(0.25).f32(0.5).bool4(false);
        let g = GroundMotor::parse(&w.0, false).unwrap();
        assert_eq!(g.motor.jump_height, 18.0);
        assert_eq!(g.slope_speed_multiplier.keys.len(), 2);
        assert_eq!((g.max_fall_speed, g.sliding_speed), (50.0, 7.0));
        assert_eq!((g.step_offset, g.slope_limit), (0.4, 60.0));
        assert_eq!(g.movement_transfer, 2);
        assert!(g.jump_enabled && !g.fly_cheat_enabled);
        robust(&w.0, |d| GroundMotor::parse(d, false));
    }

    #[test]
    fn auto_parent() {
        let mut w = W::behaviour();
        w.pptr(0, 183).bool4(true);
        let a = AutoParent::parse(&w.0, false).unwrap();
        assert_eq!(a.parent_transform.path_id, 183);
        assert!(a.make_locals_identity);
        robust(&w.0, |d| AutoParent::parse(d, false));
        w.u8(0);
        assert!(AutoParent::parse(&w.0, false).is_err());
    }

    #[test]
    fn hatch_and_cinematic_scripts() {
        let mut w = W::behaviour();
        w.pptr(0, 1).pptr(0, 2).pptr(0, 0).f32(2.0);
        w.string("Enter").string("Exit").string("");
        w.bool4(false)
            .bool4(false)
            .bool4(true)
            .bool4(false)
            .bool4(true);
        w.pptr(0, 3).pptr(0, 4);
        let h = UseableDiveHatch::parse(&w.0, false).unwrap();
        assert_eq!((h.outside_exit.path_id, h.inside_spawn.path_id), (1, 2));
        assert!(h.is_for_escape_pod && h.secure_inventory && !h.enter_only);
        assert_eq!(h.exit_custom_text, "Exit");
        robust(&w.0, |d| UseableDiveHatch::parse(d, false));

        let call = |w: &mut W, method: &str| {
            w.pptr(0, 7).string(method).i32(1);
            w.pptr(0, 0)
                .string("")
                .i32(0)
                .f32(0.0)
                .string("")
                .bool4(false)
                .i32(2);
        };
        let mut w = W::behaviour();
        w.i32(0)
            .i32(0)
            .bool4(true)
            .pptr(0, 5)
            .bool4(false)
            .bool4(true);
        w.i32(1);
        call(&mut w, "CinematicEnter");
        w.i32(2);
        call(&mut w, "CinematicExit");
        call(&mut w, "Other");
        w.bool4(false).string("ExitEscapePod");
        let t = CinematicModeTrigger::parse(&w.0, false).unwrap();
        assert_eq!(t.cinematic_controller.path_id, 5);
        assert_eq!(t.on_cinematic_start[0].method_name, "CinematicEnter");
        assert_eq!(t.on_cinematic_end.len(), 2);
        assert_eq!(t.on_cinematic_end[0].call_state, 2);
        assert_eq!(t.hand_text, "ExitEscapePod");
        robust(&w.0, |d| CinematicModeTrigger::parse(d, false));

        let mut w = W::behaviour();
        w.pptr(0, 1).pptr(0, 0).pptr(0, 6);
        w.bool4(true).bool4(false).bool4(true);
        for s in ["view", "", "cinematicMode", ""] {
            w.string(s);
        }
        w.f32(0.25).f32(0.5).string("");
        w.i32(1).pptr(0, 8);
        w.bool4(false)
            .bool4(false)
            .pptr(0, 9)
            .pptr(0, 0)
            .bool4(true);
        let c = PlayerCinematicController::parse(&w.0, false).unwrap();
        assert_eq!(c.end_transform.path_id, 6);
        assert!(c.only_use_end_transform_in_vr && c.enforce_cinematic_mode_end);
        assert_eq!(c.anim_param, "cinematicMode");
        assert_eq!(c.anim_param_receivers.len(), 1);
        robust(&w.0, |d| PlayerCinematicController::parse(d, false));

        let mut w = W::behaviour();
        for i in 1..=13 {
            w.pptr(0, i);
        }
        let f = EscapePodFirstUse::parse(&w.0, false).unwrap();
        assert_eq!((f.bottom.path_id, f.top_first_use.path_id), (3, 6));
        assert_eq!(f.creatures[6].path_id, 13);
        robust(&w.0, |d| EscapePodFirstUse::parse(d, false));
    }

    #[test]
    fn motors_and_controller() {
        let mut w = W::behaviour();
        motor(&mut w);
        w.f32(0.0)
            .f32(0.0)
            .f32(0.0)
            .bool4(true)
            .pptr(0, 3)
            .f32(1.25);
        let m = UnderwaterMotor::parse(&w.0, false).unwrap();
        assert_eq!(m.motor.forward_max_speed, 1.0);
        assert_eq!(m.motor.underwater_gravity, 7.0);
        assert!(m.motor.can_swim && !m.motor.can_jump);
        assert_eq!(m.motor.forward_sprint_modifier, 8.0);
        assert_eq!(m.motor.air_acceleration, 17.0);
        assert_eq!(m.motor.jump_height, 18.0);
        assert!(m.fast_swim_mode);
        assert_eq!(m.player_speed_modifier, 1.25);
        assert_eq!(PlayerMotor::parse(&w.0, false).unwrap(), m.motor);
        robust(&w.0, |d| UnderwaterMotor::parse(d, false));

        let mut w = W::behaviour();
        w.pptr(0, 1).pptr(0, 2).pptr(0, 3);
        w.f32(1.8).f32(0.9).f32(-0.3);
        w.f32(0.0).f32(0.0).f32(0.0);
        w.pptr(0, 4).pptr(0, 5).pptr(0, 4);
        w.f32(0.3).f32(0.1).bool4(true);
        for v in 1..=16 {
            w.f32(v as f32);
        }
        let c = PlayerController::parse(&w.0, false).unwrap();
        assert_eq!((c.stand_height, c.swim_height), (1.8, 0.9));
        assert_eq!(c.ground_controller.path_id, 5);
        assert_eq!(c.swim_forward_max_speed, 1.0);
        assert_eq!(c.seaglide_swim_drag, 11.0);
        assert_eq!(c.walk_run_strafe_max_speed, 14.0);
        assert_eq!(c.walk_run_camera_minimum_y, 16.0);
        robust(&w.0, |d| PlayerController::parse(d, false));
    }

    #[test]
    fn breakable_resource() {
        let mut w = W::behaviour();
        w.i32(1)
            .string("guid-a")
            .string("")
            .string("")
            .i32(5)
            .f32(0.25);
        w.string("guid-b").string("").string("").i32(6);
        w.f32(0.1).i32(1).i32(3);
        for _ in 0..4 {
            w.pptr(0, 0);
        }
        w.string("BreakText").string("");
        let b = BreakableResource::parse(&w.0, false).unwrap();
        assert_eq!(
            b.prefab_list,
            vec![RandomPrefab {
                prefab: "guid-a".into(),
                tech_type: 5,
                chance: 0.25
            }]
        );
        assert_eq!(
            (b.default_prefab.as_str(), b.default_tech_type),
            ("guid-b", 6)
        );
        assert_eq!((b.num_chances, b.hits_to_break), (1, 3));
        assert_eq!(b.break_text, "BreakText");
        robust(&w.0, |d| BreakableResource::parse(d, false));
    }
}
