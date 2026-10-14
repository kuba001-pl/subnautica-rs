//! The item rules read from the install, as data: what every machine
//! replays the item events with (M9d3, `docs/DESIGN.md` "M9d3 plan").

use std::collections::BTreeMap;

use super::{AnalysisEntry, SlotTable, TechType, Techs};

/// What the rules need to know of a pickupable prefab.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupInfo {
    /// By the game's rule (`CraftData.GetTechType`).
    pub tech: TechType,
    /// `Pickupable.isPickupable`.
    pub pickupable: bool,
    pub destroy_on_death: bool,
    /// Has an `Eatable`.
    pub eatable: bool,
    /// `Pickupable.randomizeRotationWhenDropped` (M9d5b).
    pub randomize_rotation: bool,
    /// `Pickupable.usePackUpIcon` (M9u6d2: the hand's icon and text).
    pub pack_up_icon: bool,
}

/// One entry of an outcrop's `prefabList`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutcropDrop {
    pub tech: TechType,
    pub chance: f32,
    pub class: String,
}

/// An outcrop prefab's `BreakableResource`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutcropInfo {
    pub hits_to_break: i32,
    /// `verticalSpawnOffset`: resources appear this far along the
    /// outcrop's up (the client computes the point).
    pub vertical_offset: f32,
    pub num_chances: i32,
    pub list: Vec<OutcropDrop>,
    pub default_class: String,
    /// `breakText`: the hand text's language key (M9d5).
    pub break_text: String,
}

/// The item rules read from the install (`sn-assets::item_rules`, M9d3d;
/// tests make their own). Two machines with the same rules fingerprint hold the
/// same.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRules {
    pub techs: Techs,
    pub slots: SlotTable,
    /// `Inventory.UnlockDefaultEquipmentSlots`.
    pub player_slots: Vec<String>,
    /// `Inventory.Awake`'s grid.
    pub inventory_size: (i32, i32),
    pub analysis: Vec<AnalysisEntry>,
    /// `PDAData.defaultTech`.
    pub default_tech: Vec<TechType>,
    /// By prefab class id.
    pub pickups: BTreeMap<String, PickupInfo>,
    pub outcrops: BTreeMap<String, OutcropInfo>,
    pub locker_size: (i32, i32),
    /// `LootSpawner.escapePodTechTypes`.
    pub supplies: Vec<TechType>,
    /// `PlayerEntropy.randomizers` at the start.
    pub luck: Vec<(TechType, f32)>,
    /// The prefab the server spawns for a tech type (the supplies).
    pub tech_class: BTreeMap<TechType, String>,
}
