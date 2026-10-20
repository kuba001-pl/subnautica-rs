//! The PDA's inventory tab without its icons (M9u8b1, `docs/formats/ui.md`
//! § The inventory tab): which of its containers and equipment slots
//! show, their sizes and labels. Our own code after reading the game's
//! `uGUI_InventoryTab`, `uGUI_ItemsContainer`, `uGUI_Equipment` and
//! `uGUI_EquipmentSlot`.
//!
//! - `uGUI_ItemsContainer.Awake`: its object inactive (the inventory, the
//!   storage and the four torpedo containers).
//! - `uGUI_Equipment.Awake`: every `uGUI_EquipmentSlot` with a slot name
//!   `SetActive(false)`, then its own object inactive. A slot's `Awake`
//!   sets its background's colour by its state: default, white at alpha
//!   127.
//! - `uGUI_InventoryTab.Start`: `inventory.Init(Inventory.main.container)`:
//!   `OnResize(w, h)` (the rect 71·w × 71·h by
//!   `SetSizeWithCurrentAnchors`, `grid.uvRect` (0, 0, w, h)), active.
//! - `OnOpenPDA`: with one used storage (the locker) `storage.Init` with
//!   it; with none `equipment.Init(Inventory.main.equipment)`: each slot
//!   the player's `Equipment` has active (`ExtinguishSlots`: no change),
//!   the doll active. The storage label is the used container's `label`
//!   (`EquipmentLabel` for the equipment), else `StorageLabel`; texts by
//!   `Language.Get` (the key itself when missing). Several used storages
//!   (the torpedo containers) are not reached by our game.
//! - `OnClosePDA`: `storage.Uninit` (and the torpedo containers'),
//!   `equipment.Uninit` (only when it was `Init`: every slot inactive,
//!   the doll inactive).
//! - `OnLanguageChanged` (from `uGUI_PDA.Start`): `inventoryLabel` =
//!   `InventoryLabel`.

use std::collections::BTreeMap;

use crate::layout::{Tree, V2};
use crate::state::UiState;

/// `uGUI_ItemsContainer.CellWidth` / `CellHeight`.
pub const CELL: f32 = 71.0;

/// `uGUI_EquipmentSlot.tintDefault` (255, 255, 255, 127).
pub const TINT_DEFAULT: [f32; 4] = [1.0, 1.0, 1.0, 127.0 / 255.0];

/// A `uGUI_ItemsContainer`'s nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerNodes {
    /// Its own node (its object and its `rectTransform`).
    pub node: usize,
    /// `grid` (a `RawImage`), if set.
    pub grid: Option<usize>,
    /// The rect's stored `sizeDelta` and its parent's size × its anchors'
    /// span, per axis ([`InventoryTabConfig::lay_out`]).
    pub size_delta: V2,
    pub parent_span: V2,
}

impl ContainerNodes {
    pub fn new(node: usize, grid: Option<usize>) -> ContainerNodes {
        ContainerNodes {
            node,
            grid,
            size_delta: [0.0, 0.0],
            parent_span: [0.0, 0.0],
        }
    }
}

/// A `uGUI_EquipmentSlot` with a slot name.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotNodes {
    pub slot: String,
    pub node: usize,
    /// `background` (an `Image`) and `hint`.
    pub background: usize,
    pub hint: Option<usize>,
}

/// `uGUI_InventoryTab`'s nodes and the texts it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryTabConfig {
    pub inventory: ContainerNodes,
    pub storage: ContainerNodes,
    pub torpedo: Vec<ContainerNodes>,
    /// `equipment`'s node and its slots, in hierarchy order.
    pub equipment: usize,
    pub slots: Vec<SlotNodes>,
    pub inventory_label: usize,
    pub storage_label: usize,
    /// `Language.Get` for the keys the tab uses.
    pub language: BTreeMap<String, String>,
}

impl InventoryTabConfig {
    /// The containers' stored `sizeDelta`s and parents' spans from the
    /// laid-out `tree` (before `OnResize`).
    pub fn lay_out(&mut self, tree: &Tree) {
        let each = |c: &mut ContainerNodes| {
            if let Some(n) = tree.nodes.get(c.node) {
                let r = &n.rect;
                let p = tree.parent_size(c.node);
                c.size_delta = r.size_delta;
                c.parent_span = [0, 1].map(|a| p[a] * (r.anchor_max[a] - r.anchor_min[a]));
            }
        };
        each(&mut self.inventory);
        each(&mut self.storage);
        for t in &mut self.torpedo {
            each(t);
        }
    }

    /// `Language.Get(key)`.
    pub fn text(&self, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        self.language
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}

/// What the tab shows on the right when the PDA opens.
#[derive(Clone, Debug, PartialEq)]
pub enum UsedStorage {
    /// No used storage: the player's equipment, its slot names.
    Equipment(Vec<String>),
    /// One used container (the locker): its size and its `label` key.
    Container { size: (i32, i32), label: String },
}

/// The tab's state.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryTab {
    pub config: InventoryTabConfig,
    /// The inventory container's size (`Start`'s `Init`).
    pub inventory_size: Option<(i32, i32)>,
    /// What `OnOpenPDA` showed, until `OnClosePDA`.
    pub used: Option<UsedStorage>,
    /// `uGUI_Equipment.equipment != null` (`Init` until `Uninit`).
    equipment_init: bool,
    pub state: UiState,
}

impl InventoryTab {
    /// The `Awake`s (containers, equipment, slots) and `OnLanguageChanged`.
    pub fn new(config: InventoryTabConfig) -> InventoryTab {
        let mut t = InventoryTab {
            config,
            inventory_size: None,
            used: None,
            equipment_init: false,
            state: UiState::default(),
        };
        let c = t.config.clone();
        for n in std::iter::once(&c.inventory)
            .chain(std::iter::once(&c.storage))
            .chain(c.torpedo.iter())
        {
            t.state.active.insert(n.node, false);
        }
        for s in &c.slots {
            t.state.color.insert(s.background, TINT_DEFAULT);
            t.state.active.insert(s.node, false);
        }
        t.state.active.insert(c.equipment, false);
        t.state
            .text
            .insert(c.inventory_label, c.text("InventoryLabel"));
        // `storageLabelKey` starts as `StorageLabel`.
        t.state.text.insert(c.storage_label, c.text("StorageLabel"));
        t
    }

    /// `uGUI_InventoryTab.Start`: `inventory.Init` with the player's
    /// container of `size`.
    pub fn start(&mut self, size: (i32, i32)) {
        let c = self.config.inventory.clone();
        self.init_container(&c, size);
        self.inventory_size = Some(size);
    }

    /// `uGUI_ItemsContainer.Init`: `OnResize` and active.
    fn init_container(&mut self, c: &ContainerNodes, (w, h): (i32, i32)) {
        let size = [w as f32 * CELL, h as f32 * CELL];
        let t = self.state.transform.entry(c.node).or_default();
        t.size_delta = Some([size[0] - c.parent_span[0], size[1] - c.parent_span[1]]);
        if let Some(g) = c.grid {
            self.state.uv_rect.insert(g, [0.0, 0.0, w as f32, h as f32]);
        }
        self.state.active.insert(c.node, true);
    }

    /// The rect size `OnResize` gives a container of `size`.
    pub fn container_size((w, h): (i32, i32)) -> V2 {
        [w as f32 * CELL, h as f32 * CELL]
    }

    /// `OnOpenPDA(tab, explicitly)`.
    pub fn on_open_pda(&mut self, used: UsedStorage) {
        let key = match &used {
            UsedStorage::Container { size, label } => {
                let c = self.config.storage.clone();
                self.init_container(&c, *size);
                label.clone()
            }
            UsedStorage::Equipment(slots) => {
                for s in self.config.slots.clone() {
                    if slots.contains(&s.slot) {
                        self.state.active.insert(s.node, true);
                    }
                }
                self.state.active.insert(self.config.equipment, true);
                self.equipment_init = true;
                "EquipmentLabel".to_string()
            }
        };
        let key = if key.is_empty() { "StorageLabel" } else { &key };
        let text = self.config.text(key);
        self.state.text.insert(self.config.storage_label, text);
        self.used = Some(used);
    }

    /// `OnClosePDA`: the storage and torpedo containers' `Uninit`, the
    /// equipment's when it was `Init`.
    pub fn on_close_pda(&mut self) {
        self.state.active.insert(self.config.storage.node, false);
        for t in &self.config.torpedo {
            self.state.active.insert(t.node, false);
        }
        if self.equipment_init {
            for s in &self.config.slots {
                self.state.active.insert(s.node, false);
            }
            self.state.active.insert(self.config.equipment, false);
            self.equipment_init = false;
        }
        self.used = None;
    }
}

#[cfg(test)]
mod tests;
