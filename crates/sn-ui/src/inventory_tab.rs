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
//!
//! The icons (M9u8b2a, [`crate::item_icons`]): [`InventoryTab::sync_items`]
//! each frame with the containers' contents; `Uninit` destroys the
//! storage's icons and clears the slots'.
//!
//! The pointer (M9u8b3a): [`InventoryTab::hit`] finds the icon or slot
//! under a canvas point (`GraphicRaycaster` on the icons' and the slots'
//! rects); [`InventoryTab::set_hovered`] is `OnPointerEnter` (`hoveredItem`,
//! `HighlightSlots`: the slots compatible with the item's
//! `EquipmentType` marked, tinted `tintHighlighted`) and `OnPointerExit`
//! (`ExtinguishSlots`). Nothing is dragged yet (M9u8b4), so a slot's state
//! is default or highlighted.

use std::collections::BTreeMap;

use crate::draw::{apply, invert, to_ancestor};
use crate::item_icons::{ContainerIcons, IconNodes, SlotIcon, TabItem, slot_clear, slot_set_item};
use crate::layout::{Tree, V2};
use crate::state::UiState;

/// `uGUI_ItemsContainer.CellWidth` / `CellHeight`.
pub const CELL: f32 = 71.0;

/// `uGUI_EquipmentSlot.tintDefault` (255, 255, 255, 127).
pub const TINT_DEFAULT: [f32; 4] = [1.0, 1.0, 1.0, 127.0 / 255.0];
/// `tintHighlighted` (255, 255, 255, 255).
pub const TINT_HIGHLIGHTED: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// What a pointer is over on the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabHit {
    /// An item's icon in the inventory or the used storage.
    Item(u64),
    /// A slot (its item, if any).
    Slot { slot: String, item: Option<u64> },
}

impl TabHit {
    /// The item `OnPointerEnter` / `OnPointerClick` acts on.
    pub fn item(&self) -> Option<u64> {
        match self {
            TabHit::Item(id) => Some(*id),
            TabHit::Slot { item, .. } => *item,
        }
    }
}

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
    /// `iconRect`'s node and laid-out size ([`InventoryTabConfig::lay_out`]).
    pub icon_rect: usize,
    pub icon_size: V2,
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
        for s in &mut self.slots {
            s.icon_size = tree.rect(s.icon_rect).size();
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
    /// The containers' icons and the slots' (by slot name).
    pub inventory_icons: ContainerIcons,
    pub storage_icons: ContainerIcons,
    pub slot_icons: BTreeMap<String, SlotIcon>,
    /// `ItemDragManager.hoveredItem`.
    pub hovered: Option<u64>,
    /// The slots marked compatible.
    pub highlighted: Vec<String>,
    pub state: UiState,
}

/// Which icon `make` is asked for: an item's under a container's node, or
/// a slot's under its `iconRect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MakeIcon {
    Item(usize),
    Slot(usize),
}

impl InventoryTab {
    /// The `Awake`s (containers, equipment, slots) and `OnLanguageChanged`.
    pub fn new(config: InventoryTabConfig) -> InventoryTab {
        let mut t = InventoryTab {
            config,
            inventory_size: None,
            used: None,
            equipment_init: false,
            inventory_icons: ContainerIcons::default(),
            storage_icons: ContainerIcons::default(),
            slot_icons: BTreeMap::new(),
            hovered: None,
            highlighted: Vec::new(),
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

    /// The icons for the containers' contents now: the inventory's (once
    /// `Start` ran), the used storage's, the equipment's (`(slot, item)`)
    /// while it is `Init`. `make` makes an icon's nodes.
    pub fn sync_items(
        &mut self,
        inventory: &[TabItem],
        storage: &[TabItem],
        equipment: &[(String, TabItem)],
        make: &mut dyn FnMut(MakeIcon) -> IconNodes,
    ) {
        if self.inventory_size.is_some() {
            let parent = self.config.inventory.node;
            self.inventory_icons
                .sync(inventory, &mut self.state, &mut || {
                    make(MakeIcon::Item(parent))
                });
        }
        if matches!(self.used, Some(UsedStorage::Container { .. })) {
            let parent = self.config.storage.node;
            self.storage_icons.sync(storage, &mut self.state, &mut || {
                make(MakeIcon::Item(parent))
            });
        }
        if !self.equipment_init {
            return;
        }
        for slot in self.config.slots.clone() {
            let want = equipment
                .iter()
                .find(|(s, _)| *s == slot.slot)
                .map(|(_, i)| i);
            let have = self.slot_icons.get(&slot.slot).and_then(|s| s.item);
            match want {
                // `OnEquip`: `SetItem`.
                Some(item) if have != Some(item.id) => {
                    let icon = match self.slot_icons.get(&slot.slot) {
                        Some(s) => s.icon,
                        None => make(MakeIcon::Slot(slot.icon_rect)),
                    };
                    slot_set_item(&mut self.state, icon, slot.hint, item, slot.icon_size);
                    self.slot_icons.insert(
                        slot.slot.clone(),
                        SlotIcon {
                            icon,
                            item: Some(item.id),
                        },
                    );
                }
                // `OnUnequip`: `ClearIcon`.
                None if have.is_some() => self.clear_slot(&slot.slot),
                _ => {}
            }
        }
    }

    /// The icon or slot under `point` (in the canvas root `root`'s space)
    /// in the laid-out tree `l`: the shown item icons (the used storage's,
    /// then the inventory's), then the shown slots.
    pub fn hit(&self, l: &Tree, root: usize, point: V2) -> Option<TabHit> {
        let inside = |node: usize| -> bool {
            if !l.active(node) {
                return false;
            }
            let Some(m) = invert(&to_ancestor(l, node, root)) else {
                return false;
            };
            let p = apply(&m, [point[0], point[1], 0.0]);
            let r = l.rect(node);
            p[0] >= r.x && p[0] <= r.x + r.width && p[1] >= r.y && p[1] <= r.y + r.height
        };
        for icons in [&self.storage_icons, &self.inventory_icons] {
            for (id, (icon, _)) in &icons.icons {
                if inside(icon.root) {
                    return Some(TabHit::Item(*id));
                }
            }
        }
        self.config
            .slots
            .iter()
            .find(|s| inside(s.node))
            .map(|s| TabHit::Slot {
                slot: s.slot.clone(),
                item: self.slot_icons.get(&s.slot).and_then(|i| i.item),
            })
    }

    /// `uGUI_Equipment.HighlightSlots`: `slots` marked compatible.
    pub fn highlight_slots(&mut self, slots: &[String]) {
        for s in self.config.slots.clone() {
            if slots.contains(&s.slot) && !self.highlighted.contains(&s.slot) {
                self.state.color.insert(s.background, TINT_HIGHLIGHTED);
                self.highlighted.push(s.slot);
            }
        }
    }

    /// `ExtinguishSlots`: every slot back to the default tint.
    pub fn extinguish_slots(&mut self) {
        for s in self.config.slots.clone() {
            if self.highlighted.contains(&s.slot) {
                self.state.color.insert(s.background, TINT_DEFAULT);
            }
        }
        self.highlighted.clear();
    }

    /// The pointer over `item` (with the slots compatible with it), or over
    /// none: `OnPointerExit` then `OnPointerEnter` when it changes. True
    /// when it changed.
    pub fn set_hovered(&mut self, item: Option<(u64, &[String])>) -> bool {
        let id = item.map(|(i, _)| i);
        if id == self.hovered {
            return false;
        }
        if self.hovered.is_some() {
            self.hovered = None;
            self.extinguish_slots();
        }
        if let Some((i, slots)) = item {
            self.hovered = Some(i);
            if self.equipment_init {
                self.highlight_slots(slots);
            }
        }
        true
    }

    /// After `OnPointerClick`'s action: `ExtinguishSlots`.
    pub fn on_pointer_click(&mut self) {
        self.extinguish_slots();
    }

    fn clear_slot(&mut self, slot: &str) {
        let hint = self
            .config
            .slots
            .iter()
            .find(|s| s.slot == slot)
            .and_then(|s| s.hint);
        if let Some(s) = self.slot_icons.get_mut(slot) {
            slot_clear(&mut self.state, Some(s.icon), hint);
            s.item = None;
        }
    }

    /// `OnClosePDA`: the storage and torpedo containers' `Uninit`, the
    /// equipment's when it was `Init`.
    pub fn on_close_pda(&mut self) {
        // `ItemDragManager.hoveredItem = null`; `Uninit`'s `DeselectItem`.
        self.hovered = None;
        self.extinguish_slots();
        self.storage_icons.clear(&mut self.state);
        let filled: Vec<String> = self
            .slot_icons
            .iter()
            .filter(|(_, s)| s.item.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for k in filled {
            self.clear_slot(&k);
        }
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
