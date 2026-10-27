//! The inventory tab's item icons (M9u8b2a, `docs/formats/ui.md` § The
//! inventory tab's icons): `uGUI_ItemsContainer`'s icons per item and
//! `uGUI_EquipmentSlot`'s icon. Our own code after reading the game's
//! `uGUI_ItemsContainer`, `uGUI_EquipmentSlot` and `uGUI_ItemIcon`.
//!
//! - `OnAddItem`: `new GameObject("Item Icon")` + `uGUI_ItemIcon`,
//!   `Init(anchor (0, 1), pivot (0, 1))`; `SetBackgroundSprite(
//!   SpriteManager.GetBackground(techType))` (a Resources sprite, so
//!   `SLICE_9_GRID` on; sized to the sprite; `_Size` the desired size,
//!   still (−1, −1)); `SetBackgroundRadius(33)`; `SetForegroundSprite`
//!   (the item's, sized to it); `SetSize(66·w + 5·(w − 1), …)` (the icon,
//!   the background without keeping the aspect, the foreground keeping
//!   it); `SetPosition(71·x + 1, −(71·y + 1))`; inactive while x or y < 0.
//! - `OnRemoveItem`: the icon destroyed. `OnChangeItemPosition`: moved
//!   (shown once placed).
//! - `uGUI_EquipmentSlot.SetItem`: the slot's icon (made on first use:
//!   `Equipment Slot Icon` under `iconRect`, anchor and pivot at the
//!   centre, at (0, 0), last sibling) with the item's sprites, radius
//!   min(size) ÷ 2, foreground alpha 1, `SetSize(iconRect's size)`, shown;
//!   the hint hidden. `ClearIcon`: the icon hidden, the hint shown.
//!
//! Destroyed icons are hidden and their nodes kept for the next icon
//! (the game makes a new one, last among its siblings; icons never
//! overlap, so the draw order between them shows nothing). Bars
//! (`SetBarValue`) and chroma are not ported (DESIGN M9u8b2).

use std::collections::BTreeMap;

use crate::icon::{fill_rect, fit};
use crate::layout::V2;
use crate::state::{SpriteId, UiState};

/// `uGUI_ItemsContainer.IconRadius`.
pub const ICON_RADIUS: f32 = 33.0;

/// A sprite: the scene's id and `sprite.rect`'s size.
pub type Sprite = (SpriteId, V2);

/// `GetIconSize(w, h)`: 66 per cell and 5 between cells.
pub fn icon_size((w, h): (i32, i32)) -> V2 {
    [(w * 66 + (w - 1) * 5) as f32, (h * 66 + (h - 1) * 5) as f32]
}

/// `GetIconPosition(x, y)` as `SetPosition` uses it: (71·x + 1,
/// −(71·y + 1)) from the container's top left.
pub fn icon_position((x, y): (i32, i32)) -> V2 {
    [(x * 71 + 1) as f32, -((y * 71 + 1) as f32)]
}

/// An icon's nodes: the `uGUI_ItemIcon` and its `Background` and
/// `Foreground` (`uGUI_Icon`s, the background first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconNodes {
    pub root: usize,
    pub background: usize,
    pub foreground: usize,
}

/// One item as an icon shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabItem {
    /// Our item id (the game's `InventoryItem`).
    pub id: u64,
    /// `TechData.GetItemSize`, cells.
    pub size: (i32, i32),
    /// The container's place, cells (`InventoryItem.x`, `y`).
    pub at: (i32, i32),
    /// `SpriteManager.GetBackground(techType)` and `Get(techType)`.
    pub background: Sprite,
    pub foreground: Sprite,
}

/// `SetBackgroundSprite` (on a Resources sprite) and `SetForegroundSprite`
/// then `SetSize(size)` and the radius: the icon's graphics.
pub fn set_item(state: &mut UiState, icon: IconNodes, item: &TabItem, size: V2, radius: f32) {
    state
        .sprite
        .insert(icon.background, Some(item.background.0));
    state.enabled.insert(icon.background, true);
    state
        .sprite
        .insert(icon.foreground, Some(item.foreground.0));
    state.enabled.insert(icon.foreground, true);
    let t = |s: &mut UiState, n: usize, v: V2| {
        s.transform.entry(n).or_default().size_delta = Some(v);
    };
    // `SetSize`: the icon, the background (aspect not kept), the
    // foreground (kept).
    t(state, icon.root, size);
    t(state, icon.background, fit(size, item.background.1, false));
    t(state, icon.foreground, fit(size, item.foreground.1, true));
    let fill = fill_rect(size, size);
    let bg = state.material.entry(icon.background).or_default();
    bg.keywords.insert("SLICE_9_GRID".into(), true);
    bg.vectors
        .insert("_Size".into(), [size[0], size[1], 0.0, 0.0]);
    bg.floats.insert("_Radius".into(), radius);
    bg.vectors.insert("_FillRect".into(), fill);
    state
        .material
        .entry(icon.foreground)
        .or_default()
        .vectors
        .insert("_FillRect".into(), fill);
}

/// What a sync did, for the logs and the checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconEvents {
    pub added: Vec<u64>,
    pub removed: Vec<u64>,
    pub moved: Vec<u64>,
    /// Icons made by code this time.
    pub made: usize,
}

/// A `uGUI_ItemsContainer`'s icons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerIcons {
    /// Item → (its icon, its place).
    pub icons: BTreeMap<u64, (IconNodes, (i32, i32))>,
    /// Hidden icons, kept for reuse.
    pub pool: Vec<IconNodes>,
    pub events: IconEvents,
}

impl ContainerIcons {
    /// The icons for `items` (the container's contents now): `OnRemoveItem`
    /// for the gone, `OnChangeItemPosition` for the moved, `OnAddItem` for
    /// the new (`make`: a new icon's nodes under the container).
    pub fn sync(
        &mut self,
        items: &[TabItem],
        state: &mut UiState,
        make: &mut dyn FnMut() -> IconNodes,
    ) {
        self.events = IconEvents::default();
        let gone: Vec<u64> = self
            .icons
            .keys()
            .copied()
            .filter(|id| !items.iter().any(|i| i.id == *id))
            .collect();
        for id in gone {
            if let Some((icon, _)) = self.icons.remove(&id) {
                state.active.insert(icon.root, false);
                self.pool.push(icon);
                self.events.removed.push(id);
            }
        }
        for item in items {
            match self.icons.get_mut(&item.id) {
                Some((icon, at)) if *at != item.at => {
                    *at = item.at;
                    let icon = *icon;
                    if item.at.0 >= 0 && item.at.1 >= 0 {
                        state.active.insert(icon.root, true);
                    }
                    state
                        .transform
                        .entry(icon.root)
                        .or_default()
                        .anchored_position = Some(icon_position(item.at));
                    self.events.moved.push(item.id);
                }
                Some(_) => {}
                None => {
                    let icon = match self.pool.pop() {
                        Some(i) => i,
                        None => {
                            self.events.made += 1;
                            make()
                        }
                    };
                    set_item(state, icon, item, icon_size(item.size), ICON_RADIUS);
                    state
                        .transform
                        .entry(icon.root)
                        .or_default()
                        .anchored_position = Some(icon_position(item.at));
                    let placed = item.at.0 >= 0 && item.at.1 >= 0;
                    state.active.insert(icon.root, placed);
                    self.icons.insert(item.id, (icon, item.at));
                    self.events.added.push(item.id);
                }
            }
        }
    }

    /// `Uninit`: every icon destroyed.
    pub fn clear(&mut self, state: &mut UiState) {
        self.sync(&[], state, &mut || unreachable_icon());
    }
}

/// Never called: a sync with no items makes no icon.
fn unreachable_icon() -> IconNodes {
    IconNodes {
        root: usize::MAX,
        background: usize::MAX,
        foreground: usize::MAX,
    }
}

/// A `uGUI_EquipmentSlot`'s icon and what it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotIcon {
    pub icon: IconNodes,
    /// The item shown (`None`: `ClearIcon`ed).
    pub item: Option<u64>,
}

/// `SetItem(item)` on a slot whose `iconRect` is `size`, its icon `icon`,
/// its hint `hint`.
pub fn slot_set_item(
    state: &mut UiState,
    icon: IconNodes,
    hint: Option<usize>,
    item: &TabItem,
    size: V2,
) {
    set_item(state, icon, item, size, size[0].min(size[1]) * 0.5);
    state
        .transform
        .entry(icon.root)
        .or_default()
        .anchored_position = Some([0.0, 0.0]);
    state.active.insert(icon.root, true);
    if let Some(h) = hint {
        state.active.insert(h, false);
    }
}

/// `ClearIcon`.
pub fn slot_clear(state: &mut UiState, icon: Option<IconNodes>, hint: Option<usize>) {
    if let Some(i) = icon {
        state.active.insert(i.root, false);
    }
    if let Some(h) = hint {
        state.active.insert(h, true);
    }
}

#[cfg(test)]
mod tests;
