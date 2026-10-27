//! Dragging items on the PDA's inventory tab (M9u8b4); the rules live in
//! `sn_ui::drag`. A drop sends the same `Request::Move` a click sends (a
//! store, an item to swap with, or a slot), found by running the game's
//! `AddOrSwap` (`sn_sim::inventory::add_or_swap`) on a copy of our
//! stores: no new message, the server checks it again.
//!
//! - The press on an item's icon or a filled slot, the drag past
//!   `pixelDragThreshold`, the release: a click on the pressed item, or the
//!   drop where the pointer is (`uGUI_ItemsContainer.OnDrop`: onto an item
//!   of another store `AddOrSwap(itemA, itemB)` then `(itemA, store)`; onto
//!   a store's empty cells `(itemA, store)`; `uGUI_Equipment.OnDrop`: a
//!   compatible slot, empty or holding an item `CanSwap` allows).
//! - `ItemDragManager`: the source icon locked (alpha 0.3), the dragged
//!   icon (`DraggedIcon`) at the pointer at the item's grid size and
//!   radius 33, snapped to a slot or a swap target (`OnDragHoverStay`),
//!   the swap target's foreground at 0.3 and the `ItemActions/Swap` hint.
//! - The slots: tinted by `UpdateFeedbackState` while dragging; the hover
//!   (`OnPointerEnter` / `Exit`) and the tooltip wait for the drag's end.

use super::*;
use crate::inventory::DragInput;
use sn_protocol::{ItemId, MoveTarget, Request, Store};
use sn_sim::inventory as inv;
use sn_ui::drag::{
    DRAG_THRESHOLD, HINT_SCALE, LOCK_ALPHA, PointerDrag, Release, Snap, slot_tint, snap_step,
    swap_hint_offset,
};
use sn_ui::draw::{apply, invert, to_ancestor};
use sn_ui::inventory_tab::TabHit;
use sn_ui::item_icons::{ICON_RADIUS, icon_size, set_item};
use sn_ui::layout::{Tree, V2};

/// Our player's stores, as the main thread saw them this frame.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stores {
    pub inventory: inv::Inventory,
    pub locker: inv::Container,
    pub techs: inv::Techs,
}

impl Stores {
    /// Where `id` is.
    fn store_of(&self, id: u64) -> Option<Store> {
        Some(
            match inv::place_of(&self.inventory, Some(&self.locker), inv::ItemId(id))? {
                inv::Place::Grid => Store::Grid,
                inv::Place::Slots => Store::Slots,
                inv::Place::Storage => Store::PodLocker,
            },
        )
    }

    /// The move of `id` to `to` / `target` if `AddOrSwap` takes it, on a
    /// copy (as the server will run it).
    fn try_move(&self, id: u64, to: Store, target: MoveTarget) -> Option<Request> {
        let index = |s: Store| match s {
            Store::Grid => 0,
            Store::Slots => 1,
            Store::PodLocker => 2,
        };
        let from = index(self.store_of(id)?);
        let mut inventory = self.inventory.clone();
        let mut locker = self.locker.clone();
        let mut stores = [
            inv::Store::Grid(&mut inventory.container),
            inv::Store::Slots(&mut inventory.equipment),
            inv::Store::Grid(&mut locker),
        ];
        let t = match &target {
            MoveTarget::Store => inv::Target::Store,
            MoveTarget::Item(b) => inv::Target::Item(inv::ItemId(u64::from(b.0))),
            MoveTarget::Slot(s) => inv::Target::Slot(s.clone()),
        };
        let item = ItemId(u32::try_from(id).ok()?);
        inv::add_or_swap(
            &self.techs,
            &mut stores,
            from,
            inv::ItemId(id),
            index(to),
            &t,
        )
        .then_some(Request::Move { item, to, target })
    }
}

/// What is under the dragged item.
#[derive(Clone, Debug, PartialEq)]
enum Over {
    /// Another item's icon in a store.
    Item(u64, Store),
    Slot(String, Option<u64>),
    /// A store's empty cells.
    Cells(Store),
    Nothing,
}

/// The action hint (`ItemActionHint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hint {
    None,
    Swap,
}

/// What the dragged item does over `Over` this frame.
#[derive(Clone, Debug, PartialEq)]
struct Feedback {
    over: Over,
    /// `DragSnap` this frame: the target's snap and the icon it dims.
    snap: Option<(Snap, Option<usize>)>,
    hint: Hint,
    /// The request a release here sends.
    drop: Option<Request>,
}

/// One frame's pointer on the tab.
pub(super) struct DragFrame<'a> {
    pub l: &'a Tree,
    pub root: usize,
    pub placed: &'a Placement,
    pub camera: &'a UiCamera,
    pub screen: [i32; 2],
    /// The pointer on the canvas.
    pub canvas: Option<V2>,
    pub inputs: &'a [DragInput],
    pub dt: f32,
    pub time: f32,
}

/// What the frame's drag gives back.
#[derive(Default)]
pub(super) struct DragOut {
    pub moves: Vec<Request>,
    /// (item, button) clicks, as `screen_click` finds them.
    pub item_clicks: Vec<(u64, u8)>,
    /// Presses not on an item (the toolbar, empty slots).
    pub screen_clicks: Vec<ScreenClick>,
}

/// The `DraggedIcon` and the hint's nodes, grafted on first use.
#[derive(Default)]
pub(super) struct Dragging {
    pointer: PointerDrag,
    icon: Option<IconNodes>,
    hint: Option<IconNodes>,
    /// The dragged item and its locked icon, from `DragStart`.
    source: Option<(u64, IconNodes)>,
    /// The dimmed swap target's foreground.
    dimmed: Option<usize>,
    snap_t: f32,
    snap: Option<Snap>,
    /// Where the icon was put last frame (canvas), for the check.
    placed_at: Option<V2>,
    /// The largest distance between that and its laid-out centre.
    pub placed_error: f32,
    /// The frames checked.
    placed_checks: u32,
    logged: Option<String>,
    /// What the pointer was over at the last drop (an item's icon or a
    /// slot; `None`: neither), until it is over something else.
    held: Option<Option<Held>>,
    /// The dragged icon's and the hint's state, applied after the tab's.
    pub state: UiState,
}

impl Dragging {
    pub(super) fn active(&self) -> bool {
        self.pointer.dragged.is_some()
    }

    /// True while the pointer rests on what it was over at the drop: no
    /// `OnPointerEnter` comes until it enters something else, so the hover
    /// (and the slots it lights) waits.
    pub(super) fn hover_held(&self) -> bool {
        self.held.is_some()
    }

    /// `DragStop` (or no press at all): the source unlocked, the dragged
    /// icon hidden, the slots extinguished, nothing hovered.
    pub(super) fn stop(&mut self, tab: &mut InventoryTab, time: f32) {
        self.pointer.cancel();
        let was = self.source.take().map(|(id, source)| {
            tab.state.renderer_alpha.insert(source.background, 1.0);
            tab.state.renderer_alpha.insert(source.foreground, 1.0);
            id
        });
        if let Some(node) = self.dimmed.take() {
            tab.state.renderer_alpha.insert(node, 1.0);
        }
        self.state = UiState::default();
        for icon in [self.icon, self.hint].into_iter().flatten() {
            self.state.active.insert(icon.root, false);
        }
        if let Some(id) = was {
            for s in &tab.config.slots {
                tab.state
                    .color
                    .insert(s.background, sn_ui::inventory_tab::TINT_DEFAULT);
            }
            tab.highlighted.clear();
            tab.hovered = None;
            info!(
                "game UI drag at {time:.3} s: stop #{id}: source alpha 1, slots default, dragged icon hidden; laid-out icon within {:.3} of its rule's place in {} frames",
                self.placed_error, self.placed_checks
            );
        }
        self.snap = None;
        self.placed_at = None;
        self.logged = None;
    }

    /// The icon of `id`: a store's or a slot's.
    fn icon_of(tab: &InventoryTab, id: u64) -> Option<IconNodes> {
        tab.inventory_icons
            .icons
            .get(&id)
            .or_else(|| tab.storage_icons.icons.get(&id))
            .map(|(i, _)| *i)
            .or_else(|| {
                tab.slot_icons
                    .values()
                    .find(|i| i.item == Some(id))
                    .map(|i| i.icon)
            })
    }

    /// The frame: the inputs, the drag's feedback, the drop.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update(
        &mut self,
        f: &DragFrame,
        tab: &mut InventoryTab,
        contents: &TabContents,
        items: &BTreeMap<u64, TabItem>,
        swap_sprite: Option<(sn_ui::state::SpriteId, V2)>,
        scene: &mut sn_assets::UiScene,
        template: &sn_assets::UiScene,
    ) -> Result<DragOut, String> {
        let mut out = DragOut::default();
        let (l, root) = (f.l, f.root);
        let hit = f.canvas.and_then(|p| tab.hit(l, root, p));
        let over_item = hit.as_ref().and_then(TabHit::item);
        if self.held.as_ref().is_some_and(|h| *h != held(hit.as_ref())) {
            self.held = None;
        }
        let pixel = f.canvas.and_then(|p| {
            f.camera.project(
                apply(&f.placed.world_from_canvas(), [p[0], p[1], 0.0]),
                f.screen,
            )
        });
        let mut released = false;
        for event in f.inputs {
            match (*event, pixel) {
                (DragInput::Cancel, _) => self.stop(tab, f.time),
                (DragInput::Down, Some(p)) => match over_item {
                    Some(id) => self.pointer.press(id, p),
                    None => out.screen_clicks.push(ScreenClick::At {
                        pixel: p,
                        button: 0,
                    }),
                },
                (DragInput::Start(id), Some(p)) if over_item == Some(id) => {
                    self.pointer.start(id, p)
                }
                (DragInput::Start(id), _) => {
                    info!(
                        "game UI drag at {:.3} s: #{id} is not under the pointer",
                        f.time
                    )
                }
                (DragInput::Up, _) => released = true,
                _ => {}
            }
        }
        if let Some(p) = pixel {
            self.pointer.motion(p, DRAG_THRESHOLD);
        }
        let Some(id) = self.pointer.dragged else {
            if released && let Some(Release::Click(id)) = self.pointer.release(over_item) {
                info!(
                    "game UI PDA inventory tab at {:.3} s: release on #{id}: click, button 0",
                    f.time
                );
                tab.on_pointer_click();
                out.item_clicks.push((id, 0));
            }
            return Ok(out);
        };
        let entry = contents
            .inventory
            .iter()
            .chain(&contents.locker)
            .chain(contents.equipment.iter().map(|(_, e)| e))
            .find(|e| e.id == id);
        let (Some(entry), Some(source), Some(stores), Some(item), Some(point)) = (
            entry,
            Self::icon_of(tab, id),
            contents.stores.as_ref(),
            items.get(&id),
            f.canvas,
        ) else {
            info!(
                "game UI drag at {:.3} s: #{id} or the pointer is gone",
                f.time
            );
            self.stop(tab, f.time);
            return Ok(out);
        };
        let centre = |node: usize| -> V2 {
            let r = l.rect(node);
            let c = apply(
                &to_ancestor(l, node, root),
                [r.x + r.width * 0.5, r.y + r.height * 0.5, 0.0],
            );
            [c[0], c[1]]
        };
        let dragged_size = icon_size(entry.size);
        let from_slot = tab.slot_icons.values().any(|s| s.item == Some(id));
        // `DragStart`: snapped to the source; the icon locked.
        if self.source.is_none() {
            let slot_rect = tab
                .config
                .slots
                .iter()
                .find(|s| {
                    tab.slot_icons
                        .get(&s.slot)
                        .is_some_and(|i| i.item == Some(id))
                })
                .map(|s| s.icon_rect);
            self.snap = Some(match slot_rect {
                Some(r) => Snap::slot(centre(r), l.rect(r).size()),
                None => Snap {
                    at: centre(source.root),
                    size: dragged_size,
                    radius: ICON_RADIUS,
                },
            });
            self.snap_t = 1.0;
            self.source = Some((id, source));
            self.placed_error = 0.0;
            self.placed_checks = 0;
            info!(
                "game UI drag at {:.3} s: start #{id} (tech {}, {}): source alpha {LOCK_ALPHA}, size {dragged_size:?}, slots lit {:?}",
                f.time,
                entry.tech,
                if from_slot { "a slot" } else { "a store" },
                entry.compatible
            );
        }
        tab.state
            .renderer_alpha
            .insert(source.background, LOCK_ALPHA);
        tab.state
            .renderer_alpha
            .insert(source.foreground, LOCK_ALPHA);
        if let Some(node) = self.dimmed.take() {
            tab.state.renderer_alpha.insert(node, 1.0);
        }
        // Last frame's icon against its laid-out centre.
        if let (Some(at), Some(icon)) = (self.placed_at, self.icon)
            && l.active(icon.root)
        {
            let c = centre(icon.root);
            let e = ((c[0] - at[0]).powi(2) + (c[1] - at[1]).powi(2)).sqrt();
            self.placed_error = self.placed_error.max(e);
            self.placed_checks += 1;
        }

        let feedback = self.feedback(
            tab,
            stores,
            l,
            root,
            id,
            entry,
            hit.as_ref(),
            point,
            dragged_size,
            &centre,
        );
        // The slots' `UpdateFeedbackState` while dragging.
        let hovered_slot = match &feedback.over {
            Over::Slot(s, _) => Some(s.as_str()),
            _ => None,
        };
        let mut tints = Vec::new();
        for s in tab.config.slots.iter().filter(|s| l.active(s.node)) {
            let tint = slot_tint(
                entry.compatible.contains(&s.slot),
                hovered_slot == Some(s.slot.as_str()),
                true,
            );
            tab.state.color.insert(s.background, tint);
            tints.push(format!("{} {}", s.slot, tint_name(tint)));
        }
        let line = format!(
            "over {:?}, snap {}, hint {:?}, drop {:?}; slots [{}]",
            feedback.over,
            feedback.snap.is_some(),
            feedback.hint,
            feedback.drop,
            tints.join(", ")
        );
        if self.logged.as_ref() != Some(&line) {
            info!("game UI drag at {:.3} s: #{id}: {line}", f.time);
            self.logged = Some(line);
        }

        if released {
            match self.pointer.release(over_item) {
                Some(Release::Drop(_)) => match feedback.drop {
                    Some(request) => {
                        info!("game UI drag at {:.3} s: drop #{id}: {request:?}", f.time);
                        out.moves.push(request);
                    }
                    None => info!(
                        "game UI drag at {:.3} s: drop #{id} on {:?}: nothing to do",
                        f.time, feedback.over
                    ),
                },
                r => info!(
                    "game UI drag at {:.3} s: release {r:?}: the click is ignored while dragging",
                    f.time
                ),
            }
            self.stop(tab, f.time);
            self.held = Some(held(hit.as_ref()));
            return Ok(out);
        }

        // `OnDragHoverStay` then `OnLateUpdate`.
        if let Some((snap, dim)) = feedback.snap {
            self.snap = Some(snap);
            if let Some(node) = dim {
                tab.state.renderer_alpha.insert(node, LOCK_ALPHA);
                self.dimmed = Some(node);
            }
        }
        self.snap_t = snap_step(self.snap_t, feedback.snap.is_some(), f.dt);
        let shown = match self.snap {
            Some(s) => s.blend(point, dragged_size, self.snap_t),
            None => Snap {
                at: point,
                size: dragged_size,
                radius: ICON_RADIUS,
            },
        };
        let icon = match self.icon {
            Some(i) => i,
            None => {
                let i = graft_icon(scene, template, root)?;
                self.icon = Some(i);
                i
            }
        };
        // The root's anchors are at its rect's centre.
        let r = l.rect(root);
        let anchor = [r.x + r.width * 0.5, r.y + r.height * 0.5];
        let mut state = UiState::default();
        set_item(&mut state, icon, item, shown.size, shown.radius);
        state.active.insert(icon.root, true);
        state
            .transform
            .entry(icon.root)
            .or_default()
            .anchored_position = Some([shown.at[0] - anchor[0], shown.at[1] - anchor[1]]);
        self.placed_at = Some(shown.at);
        // The hint: native size, scaled by the icon's depth.
        if feedback.hint == Hint::Swap
            && let Some((sprite, native)) = swap_sprite
        {
            let hint = match self.hint {
                Some(i) => i,
                None => {
                    let i = graft_icon(scene, template, root)?;
                    self.hint = Some(i);
                    i
                }
            };
            let world = apply(
                &f.placed.world_from_canvas(),
                [shown.at[0], shown.at[1], 0.0],
            );
            let depth: f32 = world
                .iter()
                .zip(f.camera.position)
                .zip(f.camera.forward())
                .map(|((&v, o), d)| (v - o) * d)
                .sum();
            let k = depth * HINT_SCALE / f.placed.scale[0];
            let size = [native[0] * k, native[1] * k];
            let offset = swap_hint_offset(dragged_size);
            state.active.insert(hint.root, true);
            state.enabled.insert(hint.background, false);
            state.sprite.insert(hint.foreground, Some(sprite));
            state.enabled.insert(hint.foreground, true);
            for n in [hint.root, hint.foreground] {
                state.transform.entry(n).or_default().size_delta = Some(size);
            }
            state
                .material
                .entry(hint.foreground)
                .or_default()
                .vectors
                .insert("_FillRect".into(), sn_ui::icon::fill_rect(size, size));
            state
                .transform
                .entry(hint.root)
                .or_default()
                .anchored_position = Some([
                shown.at[0] + offset[0] - anchor[0],
                shown.at[1] + offset[1] - anchor[1],
            ]);
        } else if let Some(hint) = self.hint {
            state.active.insert(hint.root, false);
        }
        self.state = state;
        Ok(out)
    }

    /// `OnDragHoverStay` and `OnDrop`'s rules for what is under `point`.
    #[allow(clippy::too_many_arguments)]
    fn feedback(
        &self,
        tab: &InventoryTab,
        stores: &Stores,
        l: &Tree,
        root: usize,
        id: u64,
        entry: &TabEntry,
        hit: Option<&TabHit>,
        point: V2,
        dragged: V2,
        centre: &dyn Fn(usize) -> V2,
    ) -> Feedback {
        let inside = |node: usize| {
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
        let over = match hit {
            Some(TabHit::Item(b)) => match stores.store_of(*b) {
                Some(s) => Over::Item(*b, s),
                None => Over::Nothing,
            },
            Some(TabHit::Slot { slot, item }) => Over::Slot(slot.clone(), *item),
            None if inside(tab.config.inventory.node) => Over::Cells(Store::Grid),
            None if inside(tab.config.storage.node) => Over::Cells(Store::PodLocker),
            None => Over::Nothing,
        };
        let from = stores.store_of(id);
        let item_id = |b: u64| u32::try_from(b).ok().map(ItemId);
        let none = |over| Feedback {
            over,
            snap: None,
            hint: Hint::None,
            drop: None,
        };
        match over.clone() {
            Over::Item(b, _) if b == id => none(over),
            Over::Item(b, store) => {
                let Some(b_id) = item_id(b) else {
                    return none(over);
                };
                // `CanSwap(draggedItem, item)`.
                let swap = stores.try_move(id, store, MoveTarget::Item(b_id));
                let snap = swap.as_ref().and_then(|_| {
                    let icon = Self::icon_of(tab, b)?;
                    Some((
                        Snap::swap(centre(icon.root), dragged),
                        Some(icon.foreground),
                    ))
                });
                let drop = if Some(store) == from {
                    None
                } else {
                    swap.or_else(|| stores.try_move(id, store, MoveTarget::Store))
                };
                Feedback {
                    hint: if snap.is_some() {
                        Hint::Swap
                    } else {
                        Hint::None
                    },
                    snap,
                    drop,
                    over,
                }
            }
            Over::Slot(slot, held) => {
                // `CanSwitchOrSwap(slot)`.
                let compatible = entry.compatible.contains(&slot);
                let rect = tab.config.slots.iter().find(|s| s.slot == slot);
                let target = MoveTarget::Slot(slot.clone());
                match (compatible, held, rect) {
                    (true, None, Some(r)) => Feedback {
                        snap: Some((
                            Snap::slot(centre(r.icon_rect), l.rect(r.icon_rect).size()),
                            None,
                        )),
                        hint: Hint::None,
                        drop: stores.try_move(id, Store::Slots, target),
                        over,
                    },
                    (true, Some(b), Some(r)) if b != id => {
                        let swap = stores.try_move(id, Store::Slots, target);
                        let fg = tab.slot_icons.get(&slot).map(|i| i.icon.foreground);
                        Feedback {
                            snap: swap
                                .as_ref()
                                .map(|_| (Snap::swap(centre(r.icon_rect), dragged), fg)),
                            hint: if swap.is_some() {
                                Hint::Swap
                            } else {
                                Hint::None
                            },
                            drop: swap,
                            over,
                        }
                    }
                    _ => none(over),
                }
            }
            Over::Cells(store) if Some(store) != from => Feedback {
                drop: stores.try_move(id, store, MoveTarget::Store),
                snap: None,
                hint: Hint::None,
                over,
            },
            _ => none(over),
        }
    }

    /// The dragged icon and the hint drawn over the rest of the PDA (the
    /// game's own canvas, sorting layer `ItemDragManager`).
    pub(super) fn draw_last(&self, draw: &mut sn_assets::UiDraw) {
        let nodes = |i: Option<IconNodes>| i.map(|i| [i.background, i.foreground]);
        let (icon, hint) = (nodes(self.icon), nodes(self.hint));
        if icon.is_none() && hint.is_none() {
            return;
        }
        draw.list.items.sort_by_key(|d| {
            if hint.is_some_and(|h| h.contains(&d.node)) {
                2
            } else if icon.is_some_and(|i| i.contains(&d.node)) {
                1
            } else {
                0
            }
        });
    }
}

/// The object under the pointer as `FPSInputModule`'s enter and exit see
/// it: an item's icon or a slot (a slot's icon takes no raycasts).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Held {
    Item(u64),
    Slot(String),
}

fn held(hit: Option<&TabHit>) -> Option<Held> {
    match hit? {
        TabHit::Item(id) => Some(Held::Item(*id)),
        TabHit::Slot { slot, .. } => Some(Held::Slot(slot.clone())),
    }
}

/// A new `uGUI_ItemIcon` under the PDA's root (anchor and pivot centred).
fn graft_icon(
    scene: &mut sn_assets::UiScene,
    template: &sn_assets::UiScene,
    root: usize,
) -> Result<IconNodes, String> {
    let n = scene.graft(template, root)?;
    Ok(IconNodes {
        root: n,
        background: n + 1,
        foreground: n + 2,
    })
}

fn tint_name(t: [f32; 4]) -> &'static str {
    use sn_ui::drag::{TINT_ALLOWED, TINT_DENIED};
    use sn_ui::inventory_tab::{TINT_DEFAULT, TINT_HIGHLIGHTED};
    match t {
        TINT_ALLOWED => "allowed",
        TINT_DENIED => "denied",
        TINT_HIGHLIGHTED => "highlighted",
        TINT_DEFAULT => "default",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4×4 grid with a slot-3 item (1) and a 2×2 item (3), the locker
    /// with a slot-3 item (2); slots S and T take type 3.
    fn stores() -> Stores {
        let table = inv::SlotTable {
            slots: vec![("S".into(), 3), ("T".into(), 3)],
            vehicle_module: -1,
            seamoth_module: -2,
            exosuit_module: -3,
        };
        let mut techs = inv::Techs::default();
        techs.equipment.insert(10, 3);
        techs.sizes.insert(11, (2, 2));
        let mut inventory = inv::Inventory::new((4, 4), &table, &["S", "T"], Default::default());
        inventory
            .container
            .add(&techs, inv::Item::new(1, 10))
            .unwrap();
        inventory
            .container
            .add(&techs, inv::Item::new(3, 11))
            .unwrap();
        let mut locker = inv::Container::new(4, 4);
        locker.add(&techs, inv::Item::new(2, 10)).unwrap();
        Stores {
            inventory,
            locker,
            techs,
        }
    }

    fn mv(id: u32, to: Store, target: MoveTarget) -> Option<Request> {
        Some(Request::Move {
            item: ItemId(id),
            to,
            target,
        })
    }

    #[test]
    fn drops_are_the_moves_a_click_sends_and_leave_the_stores_alone() {
        let s = stores();
        let before = s.clone();
        let slot = |n: &str| MoveTarget::Slot(n.into());
        // Grid → locker, locker → grid, grid → slot, a swap with the locker.
        for (id, to, target) in [
            (1, Store::PodLocker, MoveTarget::Store),
            (2, Store::Grid, MoveTarget::Store),
            (1, Store::Slots, slot("S")),
            (1, Store::PodLocker, MoveTarget::Item(ItemId(2))),
        ] {
            assert_eq!(
                s.try_move(id, to, target.clone()),
                mv(id as u32, to, target)
            );
        }
        // The same store, a missing slot, a 2×2 item into a slot.
        assert_eq!(s.try_move(1, Store::Grid, MoveTarget::Store), None);
        assert_eq!(s.try_move(1, Store::Slots, slot("absent")), None);
        assert_eq!(s.try_move(3, Store::Slots, slot("S")), None);
        assert_eq!(s, before);
    }

    #[test]
    fn slot_items_drag_back_and_between_slots() {
        let mut s = stores();
        let Some(Request::Move { .. }) = s.try_move(1, Store::Slots, MoveTarget::Slot("S".into()))
        else {
            panic!("no equip");
        };
        let mut stores = [
            inv::Store::Grid(&mut s.inventory.container),
            inv::Store::Slots(&mut s.inventory.equipment),
        ];
        assert!(inv::add_or_swap(
            &s.techs,
            &mut stores,
            0,
            inv::ItemId(1),
            1,
            &inv::Target::Slot("S".into())
        ));
        assert_eq!(s.store_of(1), Some(Store::Slots));
        let t = MoveTarget::Slot("T".into());
        assert_eq!(
            s.try_move(1, Store::Slots, t.clone()),
            mv(1, Store::Slots, t)
        );
        assert_eq!(
            s.try_move(1, Store::Grid, MoveTarget::Store),
            mv(1, Store::Grid, MoveTarget::Store)
        );
        // The locker's item swaps with the slot's.
        let t = MoveTarget::Slot("S".into());
        assert_eq!(
            s.try_move(2, Store::Slots, t.clone()),
            mv(2, Store::Slots, t)
        );
    }
}
