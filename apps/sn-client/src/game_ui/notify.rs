//! The inventory's notifications in the client (M9u8b5a); the rules live
//! in `sn_ui::notifications`.
//!
//! - `Inventory.Pickup`: `Add(Inventory, item, 4)` for each item that
//!   comes into the grid or a slot from none of our stores (only a
//!   pick-up does that in what we run: crafting is not ported); the
//!   first stores seen are the baseline. `OnRemoveItem` (it left the
//!   grid) and `OnUnequip` (it left its slot) `Remove` it.
//! - The targets: every item icon and filled slot of the tab
//!   (`RegisterItemTarget` / `UnregisterTarget` as icons come and go),
//!   visible while its icon and foreground are active and enabled with an
//!   inherited alpha above 0.
//! - `Progress(p)`: the icon's `NotificationLabel` ("+", alpha sin(p·π/2),
//!   upper right at (−10, −10); a slot's at its `iconRect`'s size ×
//!   −0.1464466), gone at 0. The glow (`NOTIFICATION`) is M9u8b5b.
//! - `uGUI_PDA.OnLateUpdate`: the toolbar's inventory icon shows
//!   `GetCount(Inventory)` while above 0.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use sn_assets::NotificationLabelNodes;
use sn_ui::layout::V2;
use sn_ui::notifications::{
    Group, ICON_LABEL_ANCHOR, ICON_LABEL_OFFSET, LabelNodes, Notifications, PICKUP_DURATION,
    SYMBOL, anchor_point, label_alpha, label_state, slot_label_offset,
};

/// A notification target: an item's icon (a store's or a slot's) and its
/// label's offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Target {
    icon: IconNodes,
    offset: V2,
}

/// Our stores' ids last frame.
#[derive(Default)]
struct Seen {
    grid: BTreeSet<u64>,
    slots: BTreeSet<(String, u64)>,
    all: BTreeSet<u64>,
}

pub(super) struct Notices {
    manager: Notifications<Target>,
    template: sn_assets::UiScene,
    nodes: NotificationLabelNodes,
    /// Labels grafted, by their parent (an icon's or a toolbar icon's
    /// root); hidden ones are kept for the next.
    labels: HashMap<usize, LabelNodes>,
    /// The labels shown, by parent.
    shown: BTreeSet<usize>,
    seen: Option<Seen>,
    /// The toolbar's anchor and offset (`uGUI_Toolbar`).
    toolbar_anchor: V2,
    toolbar_offset: V2,
    count: usize,
    /// Each target's last progress, for the logs (quarter steps).
    logged: HashMap<usize, i32>,
    /// The labels' state, applied after the tab's.
    pub state: UiState,
}

/// `isActiveAndEnabled` up the tree with the state's changes.
fn active_in(scene: &sn_assets::UiScene, state: &UiState, mut node: usize) -> bool {
    for _ in 0..scene.tree.nodes.len() {
        let Some(n) = scene.tree.nodes.get(node) else {
            return false;
        };
        if !state.active.get(&node).copied().unwrap_or(n.active_self) {
            return false;
        }
        match n.parent {
            Some(p) => node = p,
            None => return true,
        }
    }
    false
}

impl Notices {
    pub(super) fn new(
        template: sn_assets::UiScene,
        nodes: NotificationLabelNodes,
        toolbar_anchor: i32,
        toolbar_offset: V2,
    ) -> Notices {
        Notices {
            manager: Notifications::new(),
            template,
            nodes,
            labels: HashMap::new(),
            shown: BTreeSet::new(),
            seen: None,
            toolbar_anchor: anchor_point(toolbar_anchor),
            toolbar_offset,
            count: 0,
            logged: HashMap::new(),
            state: UiState::default(),
        }
    }

    /// `INotificationTarget.IsVisible` for an icon in `state`.
    fn visible(scene: &sn_assets::UiScene, state: &UiState, t: Target) -> bool {
        let fg = t.icon.foreground;
        active_in(scene, state, t.icon.root)
            && active_in(scene, state, fg)
            && state.enabled.get(&fg).copied().unwrap_or(true)
            && scene.inherited_alpha(fg, state) > 0.0
    }

    /// The frame: the stores' changes, the targets, `LateUpdate` with `dt`, the labels. `state`
    /// builds the scene's state this frame for the visibility (only when
    /// a notification has a target).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update(
        &mut self,
        scene: &mut sn_assets::UiScene,
        tab: &InventoryTab,
        toolbar: Option<&sn_ui::toolbar::Toolbar>,
        contents: &TabContents,
        dt: f32,
        time: f32,
        state: &dyn Fn(&UiState) -> UiState,
    ) -> Result<(), String> {
        let mut progress = Vec::new();
        // The stores against last frame.
        let grid: BTreeSet<u64> = contents.inventory.iter().map(|e| e.id).collect();
        let slots: BTreeSet<(String, u64)> = contents
            .equipment
            .iter()
            .map(|(s, e)| (s.clone(), e.id))
            .collect();
        let all: BTreeSet<u64> = grid
            .iter()
            .copied()
            .chain(slots.iter().map(|(_, i)| *i))
            .chain(contents.locker.iter().map(|e| e.id))
            .collect();
        if contents.stores.is_none() {
            self.seen = None;
        } else if let Some(seen) = &self.seen {
            let new: Vec<u64> = grid
                .iter()
                .copied()
                .chain(slots.iter().map(|(_, i)| *i))
                .filter(|i| !seen.all.contains(i))
                .collect();
            for id in &new {
                progress.extend(self.manager.add((Group::Inventory, *id), PICKUP_DURATION));
            }
            if !new.is_empty() {
                info!(
                    "game UI notifications at {time:.3} s: picked up {new:?}: Add(Inventory, id, {PICKUP_DURATION}); count {}",
                    self.manager.count(Group::Inventory)
                );
            }
            let gone: Vec<u64> = seen
                .grid
                .difference(&grid)
                .copied()
                .chain(seen.slots.difference(&slots).map(|(_, i)| *i))
                .collect();
            for id in gone {
                let p = self.manager.remove((Group::Inventory, id));
                if self.manager.count(Group::Inventory) != self.count || !p.is_empty() {
                    info!(
                        "game UI notifications at {time:.3} s: #{id} left the grid or its slot: Remove; count {}",
                        self.manager.count(Group::Inventory)
                    );
                }
                progress.extend(p);
            }
        }
        if contents.stores.is_some() {
            self.seen = Some(Seen { grid, slots, all });
        }

        // The targets: the tab's icons now.
        let mut want: BTreeMap<u64, Target> = BTreeMap::new();
        for icons in [&tab.inventory_icons, &tab.storage_icons] {
            for (id, (icon, _)) in &icons.icons {
                want.insert(
                    *id,
                    Target {
                        icon: *icon,
                        offset: ICON_LABEL_OFFSET,
                    },
                );
            }
        }
        for s in &tab.config.slots {
            if let Some(si) = tab.slot_icons.get(&s.slot)
                && let Some(id) = si.item
            {
                want.insert(
                    id,
                    Target {
                        icon: si.icon,
                        offset: slot_label_offset(s.icon_size),
                    },
                );
            }
        }
        let have: Vec<((Group, u64), Target)> =
            self.manager.targets.iter().map(|(k, t)| (*k, *t)).collect();
        let needs_state = self
            .manager
            .notifications
            .keys()
            .any(|k| self.manager.targets.contains_key(k) || want.contains_key(&k.1));
        let st = needs_state.then(|| state(&self.state));
        let visible = |t: Target| st.as_ref().is_some_and(|s| Self::visible(scene, s, t));
        for (key, t) in have {
            if want.get(&key.1) != Some(&t) {
                progress.extend(self.manager.unregister(t));
            }
        }
        for (id, t) in &want {
            let key = (Group::Inventory, *id);
            if self.manager.targets.get(&key) != Some(t) {
                progress.extend(self.manager.register(key, *t, &visible));
            }
        }
        if self
            .manager
            .notifications
            .keys()
            .any(|k| self.manager.targets.contains_key(k))
        {
            progress.extend(self.manager.late_update(dt, &visible));
        }

        // `Progress(p)` on the icons.
        for (t, p) in progress {
            let alpha = label_alpha(p);
            let step = (p * 4.0).ceil() as i32;
            if self.logged.get(&t.icon.root) != Some(&step) {
                self.logged.insert(t.icon.root, step);
                info!(
                    "game UI notifications at {time:.3} s: icon {} progress {p:.3}: '+' alpha {alpha:.3}",
                    t.icon.root
                );
            }
            self.show(
                scene,
                t.icon.root,
                SYMBOL,
                alpha,
                t.offset,
                anchor_point(ICON_LABEL_ANCHOR),
            )?;
        }
        // `SetNotificationsAmount(i, GetCount)`: the inventory's icon.
        let count = self.manager.count(Group::Inventory);
        let index = REGULAR_TABS
            .iter()
            .position(|t| *t == sn_ui::pda::Tab::Inventory);
        if let (Some(tb), Some(i)) = (toolbar, index)
            && let Some(icon) = tb.icons.get(i).filter(|_| i < tb.count)
            && (count != self.count || (count > 0) != self.shown.contains(&icon.root))
        {
            let (anchor, offset) = (self.toolbar_anchor, self.toolbar_offset);
            let alpha = if count > 0 { 1.0 } else { 0.0 };
            self.show(scene, icon.root, &count.to_string(), alpha, offset, anchor)?;
            info!(
                "game UI notifications at {time:.3} s: the inventory tab's count {} → {count} (label {})",
                self.count,
                if count > 0 { "shown" } else { "gone" }
            );
        }
        self.count = count;
        Ok(())
    }

    /// `SetNotificationAlpha(alpha)` then the text and offset: the label
    /// under `parent` made (grafted, anchored) when first shown, hidden at
    /// alpha 0.
    fn show(
        &mut self,
        scene: &mut sn_assets::UiScene,
        parent: usize,
        text: &str,
        alpha: f32,
        offset: V2,
        anchor: V2,
    ) -> Result<(), String> {
        if alpha <= 0.0 {
            if self.shown.remove(&parent)
                && let Some(l) = self.labels.get(&parent)
            {
                self.state.active.insert(l.root, false);
            }
            return Ok(());
        }
        let made = !self.shown.contains(&parent);
        let label = match self.labels.get(&parent) {
            Some(l) => *l,
            None => {
                let root = scene.graft(&self.template, parent)?;
                scene.set_anchors(root, anchor);
                let l = LabelNodes {
                    root,
                    background: root + self.nodes.background,
                    text: root + self.nodes.text,
                };
                self.labels.insert(parent, l);
                l
            }
        };
        self.shown.insert(parent);
        label_state(
            &mut self.state,
            label,
            text,
            alpha,
            offset,
            self.nodes.text_color,
            made,
        );
        Ok(())
    }
}
