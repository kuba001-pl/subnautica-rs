//! The PDA's open and close (M9u8a3, `sn_sim::pda`) and the clicks on its
//! screen (M9u8b3a; M9d5b's Bevy panel, which stood in for the inventory
//! tab, is gone: `crate::game_ui`'s worker draws the game's tab and
//! hit-tests the pointer on it). Tab opens the PDA
//! (`GUIHand`'s `Button.PDA`) and closes it once focused
//! (`PDA.ManagedUpdate`); the hand on the lifepod's locker opens it with
//! the locker beside the grid, as `EscapePod.StorageOpen` does (the pod's
//! `storage_open`, `PDA.Open` 0.25 s later, the target `storagePivot`).
//! The screen's content shows from `Open` to `Close` and takes clicks once
//! the PDA is focused (`Activated`); the cursor is free from `Activated` to
//! `Close`. `crate::player` reads [`Panel::pda`] for the
//! look, the hand and the camera's locked mode, `crate::body` for the
//! model. A left click does the item's action by the game's rule
//! (`sn_sim::inventory::item_action`), a right click drops it where the
//! game would ([`drop_request`]). Each mouse click while focused goes to
//! the UI worker ([`ScreenClick`]), which finds the toolbar icon, item icon
//! or slot under it and sends an item's click back ([`item_clicks`]); the
//! left button's press and release go as [`DragInput`]s (M9u8b4: a press on
//! an item clicks on the release, or drags it, and the drop comes back as
//! the `Move` a click would send).
//! `--panel` does the same from a script, clicking at the icons' centres,
//! for checking from the logs.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use sn_assets::ItemText;
use sn_protocol::{ItemId, MoveTarget, Request, Store};
use sn_session::drops::{Draws, drop_at};
use sn_sim::V3;
use sn_sim::inventory::{self as inv, Button as Click, DropPlace, Place};
use sn_sim::pda::{CloseReason, Event as PdaEvent, Pda, State as PdaState};

use crate::body::BodyDrive;
use crate::items::Text as ItemTextRes;
use crate::player::PlayerSim;
use crate::session::Session;

/// `EscapePod.StorageOpen` / `StorageOnClosePDA`: the storage sequence.
const STORAGE_SEQUENCE: f64 = 0.25;
/// The pod animator's bool.
const STORAGE_OPEN: &str = "storage_open";

/// A `--panel` step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scripted {
    /// Tab (opens without the locker).
    Open,
    Close,
    /// A left click on every item in the locker.
    TakeAll,
    /// A left click on the first grid item that equips.
    Equip,
    /// A right click on the first grid item.
    Drop,
    /// A left click on the first item in the slots (M9u6c2).
    Unequip,
    /// A left click at the centre of the PDA toolbar's icon N (M9u8a4b),
    /// `tabN`.
    Tab(u8),
    /// `hoverN` (M9u8b3b3): the pointer rests on the grid's item N (in
    /// layout order) at its icon's centre, until the next hover step or
    /// the close.
    Hover(u8),
    /// `hover-empty`: the pointer rests on the grid's last cell.
    HoverEmpty,
    /// `dragN` (M9u8b4): the left button pressed on the grid's item N and
    /// dragged at once (the threshold passed); `drag-lockerN` the
    /// locker's item N, `drag-slotS` the item in slot S.
    Drag(u8),
    DragLocker(u8),
    DragSlot(&'static str),
    /// `over-grid`, `over-locker`, `over-slotS`: the pointer moves to the
    /// grid's last cell, the locker's, or slot S's centre.
    OverGrid,
    OverLocker,
    OverSlot(&'static str),
    /// `press`: the left button pressed where the pointer is (a
    /// `release` without moving clicks, as the mouse does).
    Press,
    /// `release`: the left button released where the pointer is.
    Release,
    /// `cancel`: the drag stopped without a drop.
    Cancel,
}

/// Where `--panel`'s scripted pointer rests (M9u8b3b3); the UI worker
/// projects it to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptedHover {
    /// Item `id`'s icon (or slot) centre.
    Item(u64),
    /// The centre of the player's grid's last cell.
    LastCell,
    /// The centre of the locker's last cell (M9u8b4).
    LockerCell,
    /// A slot's centre.
    Slot(&'static str),
}

/// The left button on the PDA's screen, for the UI worker's drag
/// (M9u8b4): it finds what is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragInput {
    /// Pressed.
    Down,
    /// Released.
    Up,
    /// `--panel`: pressed on the item and dragged past the threshold.
    Start(u64),
    /// The drag stopped without a drop.
    Cancel,
}

/// A click on the PDA's screen (M9u8a4b, M9u8b3a), hit-tested by
/// `crate::game_ui`'s worker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenClick {
    /// The mouse: physical pixels from the bottom left (Unity's screen
    /// space) and `PointerEventData`'s button (0 left, 1 right, 2 middle).
    At { pixel: [f32; 2], button: u8 },
    /// `--panel tabN`: the left button at toolbar icon N's centre,
    /// projected.
    Icon(usize),
    /// `--panel`'s item steps: `button` at item `id`'s icon (or slot)
    /// centre, projected.
    Item { id: u64, button: u8 },
}

/// The player's slot names a `--panel` step may name.
fn slot_name(s: &str) -> Option<&'static str> {
    ["Head", "Body", "Gloves", "Foots", "Chip1", "Chip2", "Tank"]
        .into_iter()
        .find(|&name| name == s)
}

impl Scripted {
    pub fn parse(s: &str) -> Option<Scripted> {
        Some(match s {
            "open" => Scripted::Open,
            "close" => Scripted::Close,
            "take-all" => Scripted::TakeAll,
            "equip" => Scripted::Equip,
            "drop" => Scripted::Drop,
            "unequip" => Scripted::Unequip,
            "hover-empty" => Scripted::HoverEmpty,
            "over-grid" => Scripted::OverGrid,
            "over-locker" => Scripted::OverLocker,
            "press" => Scripted::Press,
            "release" => Scripted::Release,
            "cancel" => Scripted::Cancel,
            _ if s.starts_with("drag-locker") => {
                Scripted::DragLocker(s.strip_prefix("drag-locker")?.parse().ok()?)
            }
            _ if s.starts_with("drag-slot") => {
                Scripted::DragSlot(slot_name(s.strip_prefix("drag-slot")?)?)
            }
            _ if s.starts_with("over-slot") => {
                Scripted::OverSlot(slot_name(s.strip_prefix("over-slot")?)?)
            }
            _ if s.starts_with("drag") => Scripted::Drag(s.strip_prefix("drag")?.parse().ok()?),
            _ => match s.strip_prefix("hover") {
                Some(n) => Scripted::Hover(n.parse().ok()?),
                None => Scripted::Tab(s.strip_prefix("tab")?.parse().ok()?),
            },
        })
    }
}

#[derive(Resource)]
pub struct Panel {
    /// The player's PDA (M9u8a3).
    pub pda: Pda,
    /// Opened by the locker: the locker is the used storage.
    with_locker: bool,
    /// `EscapePod.storageSequence`: when it ends (play seconds) and
    /// whether it then opens the panel.
    storage: Option<(f64, bool)>,
    /// `PlayerSim::locker_opened` last seen.
    locker_seen: u32,
    /// `PDA.target`'s position (`storagePivot`) while the locker's PDA is
    /// open.
    target: Option<V3>,
    /// `PlayerSim::controls` last frame (death's edges, M9u8a3).
    controls_seen: bool,
    /// The drops' draws (the game's `Random` is unseeded).
    draws: Draws,
    /// `--panel`, latest first.
    script: Vec<(f64, Scripted)>,
    /// Test script barrier: wait for these items in our authoritative grid.
    pub script_transfers: Vec<ItemId>,
    script_retry_at: f64,
    /// Clicks on the PDA's screen since the last frame sent to the UI
    /// worker.
    pub screen_clicks: Vec<ScreenClick>,
    /// The pointer (physical pixels from the bottom left) while the
    /// screen takes clicks, for the worker's hover.
    pub pointer: Option<[f32; 2]>,
    /// `--panel`'s scripted pointer (M9u8b3b3), while the screen takes
    /// clicks.
    pub hover: Option<ScriptedHover>,
    /// The left button's presses and releases (M9u8b4), for the worker.
    pub drag_inputs: Vec<DragInput>,
}

impl Panel {
    /// The panel (the inventory tab's content) is shown: from `Open` to
    /// `Close` (`content.SetVisible`).
    pub fn script_ready(&self) -> bool {
        self.script.is_empty()
            && self.script_transfers.is_empty()
            && self.pda.state() == PdaState::Closed
    }

    pub fn shown(&self) -> bool {
        self.pda.using_pda()
    }

    /// Opened with the locker as the used storage (M9u8b1).
    pub fn with_locker(&self) -> bool {
        self.with_locker
    }

    pub fn new(mut script: Vec<(f64, Scripted)>, seed: u64) -> Panel {
        script.sort_by(|a, b| b.0.total_cmp(&a.0));
        Panel {
            pda: Pda::new(),
            controls_seen: false,
            with_locker: false,
            storage: None,
            locker_seen: 0,
            target: None,
            draws: Draws(seed),
            script,
            script_transfers: Vec::new(),
            script_retry_at: 0.0,
            screen_clicks: Vec::new(),
            pointer: None,
            hover: None,
            drag_inputs: Vec::new(),
        }
    }
}

/// `Inventory.InternalDropItem` for `item` (`sn_session::drops::drop_at`):
/// logged next to the camera and the ray's point. `None` before the
/// player exists.
pub fn drop_request(
    sim: &PlayerSim,
    draws: &mut Draws,
    item: ItemId,
    randomize: bool,
) -> Option<Request> {
    let (camera, forward) = sim.camera()?;
    let d = drop_at(sim.world(), camera, forward, randomize, draws);
    let (ray, point) = (d.ray, V3::from_f32(d.at.point));
    info!(
        "inventory: drop of item {}: camera ({:.3}, {:.3}, {:.3}) forward ({:.3}, {:.3}, {:.3}); draws {:.3}, ({:.3}, {:.3}); ray to ({:.3}, {:.3}, {:.3}), {:.2} m; lands at ({:.3}, {:.3}, {:.3}), {:.2} m from the camera; rotation {:?}{}",
        item.0,
        camera.x,
        camera.y,
        camera.z,
        forward.x,
        forward.y,
        forward.z,
        d.value,
        d.circle.0,
        d.circle.1,
        ray.x,
        ray.y,
        ray.z,
        (ray - camera).length(),
        point.x,
        point.y,
        point.z,
        (point - camera).length(),
        d.at.rotation,
        if randomize { " (random)" } else { "" }
    );
    Some(Request::Drop { item, at: d.at })
}

fn sim_id(id: ItemId) -> inv::ItemId {
    inv::ItemId(u64::from(id.0))
}

/// What a click on `item` does: the game's action for that button, then
/// the request it sends (if any), logged.
fn click(
    session: &mut Session,
    sim: &PlayerSim,
    panel: &mut Panel,
    text: Option<&ItemText>,
    item: ItemId,
    button: Click,
) {
    let me = session.player();
    let place = DropPlace {
        in_escape_pod: sim.in_pod().unwrap_or(false),
        ..DropPlace::default()
    };
    let with_locker = panel.with_locker;
    let found = session
        .with_items(|items, _| {
            let p = items.players.get(&me?)?;
            let storage = with_locker.then_some(&items.locker);
            let id = sim_id(item);
            let from = inv::place_of(&p.inventory, storage, id)?;
            let tech = match from {
                Place::Grid => p.inventory.container.get(id).map(|i| i.tech),
                Place::Slots => p
                    .inventory
                    .equipment
                    .items()
                    .find(|i| i.id == id)
                    .map(|i| i.tech),
                Place::Storage => storage.and_then(|s| s.get(id)).map(|i| i.tech),
            }?;
            let info = items
                .classes
                .get(&item)
                .and_then(|c| items.rules().pickups.get(c));
            let eatable = info.is_some_and(|i| i.eatable);
            let randomize = info.is_some_and(|i| i.randomize_rotation);
            let actions = inv::item_actions(
                &items.rules().techs,
                &p.inventory,
                storage,
                id,
                eatable,
                place,
            );
            Some((from, tech, actions, randomize))
        })
        .flatten();
    let Some((from, tech, actions, randomize)) = found else {
        info!("inventory: {button:?} click on item {}: not ours", item.0);
        return;
    };
    let action = inv::item_action(actions, button);
    let all: Vec<&str> = [
        inv::USE,
        inv::EAT,
        inv::EQUIP,
        inv::UNEQUIP,
        inv::ASSIGN,
        inv::SWITCH,
        inv::SWAP,
        inv::DROP,
    ]
    .into_iter()
    .filter(|a| actions & a != 0)
    .map(inv::action_name)
    .collect();
    let name = text.map_or_else(|| tech.to_string(), |t| t.tech(tech));
    info!(
        "inventory: {button:?} click on {name} #{} ({from:?}): actions [{}] → {}",
        item.0,
        all.join(", "),
        inv::action_name(action)
    );
    let to = |p: Place| match p {
        Place::Grid => Store::Grid,
        Place::Slots => Store::Slots,
        Place::Storage => Store::PodLocker,
    };
    let request = match action {
        inv::EQUIP | inv::SWAP | inv::UNEQUIP | inv::SWITCH => Some(Request::Move {
            item,
            to: to(inv::opposite(from, with_locker)),
            target: MoveTarget::Store,
        }),
        inv::DROP => drop_request(sim, &mut panel.draws, item, randomize),
        inv::USE | inv::EAT => {
            info!("inventory: {} is not ported yet", inv::action_name(action));
            None
        }
        _ => {
            if button == Click::Right && place.in_escape_pod {
                info!("inventory: no drop in the lifepod (CanDropItemHere)");
            }
            None
        }
    };
    if let Some(r) = request {
        session.request(r);
    }
}

fn set_cursor(windows: &mut Query<(&Window, &mut CursorOptions)>, free: bool) {
    for (window, mut cursor) in windows.iter_mut() {
        if free {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        } else if window.focused {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
    }
}

/// `PDA.Open`: refused while in use, dead or in a cinematic (our
/// `controls`). With the locker, the target is `storagePivot`.
fn open(panel: &mut Panel, sim: &PlayerSim, with_locker: bool) -> bool {
    let target = if with_locker {
        sim.storage_pivot()
    } else {
        None
    };
    let Some(player) = sim.position() else {
        return false;
    };
    if !panel.pda.open(target, player, !sim.controls()) {
        info!(
            "inventory: PDA.Open refused (in use {}, controls {})",
            panel.pda.in_use(),
            sim.controls()
        );
        return false;
    }
    panel.with_locker = with_locker;
    panel.target = target;
    info!(
        "inventory: PDA.Open at {:.3} s (PDA.time {:.3}){}; the model in the hand, using_pda on, the panel shown",
        sim.play_seconds(),
        panel.pda.time,
        match target {
            Some(t) => {
                let d = (t - player).dot(t - player);
                format!(
                    " with the locker; closes beyond {:.2} m of storagePivot (activeSqrDistance {:.2})",
                    (d + 1.0).sqrt(),
                    d + 1.0
                )
            }
            None => String::new(),
        }
    );
    true
}

/// After `PDA.Close` went through: the cursor locked again
/// (`InterceptInput(false)`), then its callback: for the locker,
/// `EscapePod.StorageOnClosePDA`.
fn closed(
    panel: &mut Panel,
    sim: &mut PlayerSim,
    drive: &mut BodyDrive,
    windows: &mut Query<(&Window, &mut CursorOptions)>,
    why: &str,
) {
    panel.target = None;
    set_cursor(windows, false);
    info!(
        "inventory: PDA.Close at {:.3} s ({why}); using_pda off, the panel hidden, the model until Deactivated",
        sim.play_seconds()
    );
    if std::mem::take(&mut panel.with_locker) {
        storage_closed(panel, sim, drive);
    }
}

/// `PDA.Close` called from outside `ManagedUpdate` (Escape, death).
fn close(
    panel: &mut Panel,
    sim: &mut PlayerSim,
    drive: &mut BodyDrive,
    windows: &mut Query<(&Window, &mut CursorOptions)>,
    why: &str,
) {
    if panel.pda.close() {
        closed(panel, sim, drive, windows, why);
    }
}

/// `EscapePod.StorageOnClosePDA`.
fn storage_closed(panel: &mut Panel, sim: &mut PlayerSim, drive: &mut BodyDrive) {
    let now = sim.play_seconds();
    panel.storage = Some((now + STORAGE_SEQUENCE, false));
    let known = sim.set_pod_bool(STORAGE_OPEN, false, drive);
    info!("inventory: locker closing: {STORAGE_OPEN} off (in the pod's controller: {known})");
}

/// Before the player: the locker's sequence, Tab and Escape, the PDA's
/// distance rule, clicks on the panel and the `--panel` script. The keys
/// and buttons it uses are taken, so the player does not see them.
#[allow(clippy::too_many_arguments)]
pub fn input(
    time: Res<Time>,
    mut panel: ResMut<Panel>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut windows: Query<(&Window, &mut CursorOptions)>,
    mut sim: ResMut<PlayerSim>,
    mut session: ResMut<Session>,
    mut drive: ResMut<BodyDrive>,
    text: Option<Res<ItemTextRes>>,
    mut frame: Option<ResMut<crate::game_ui::GameUiFrame>>,
) {
    let panel = &mut *panel;
    let now = sim.play_seconds();
    let text = text.as_ref().map(|t| &*t.0);

    // `EscapePod.StorageOpen`: the hand clicked the locker.
    if sim.locker_opened != panel.locker_seen {
        panel.locker_seen = sim.locker_opened;
        if panel.storage.is_some() {
            info!("inventory: locker busy (its sequence runs), click ignored");
        } else {
            let known = sim.set_pod_bool(STORAGE_OPEN, true, &mut drive);
            panel.storage = Some((now + STORAGE_SEQUENCE, true));
            info!(
                "inventory: locker opening at {now:.2} s: {STORAGE_OPEN} on (in the pod's controller: {known}); panel in {STORAGE_SEQUENCE} s"
            );
        }
    }
    if let Some((end, opens)) = panel.storage
        && now >= end
    {
        panel.storage = None;
        // `StorageOnFlapOpened`: the locker as used storage, `PDA.Open`;
        // refused, `StorageOnClosePDA`.
        if opens && !open(panel, &sim, true) {
            storage_closed(panel, &mut sim, &mut drive);
        }
    }

    // `GUIHand.OnUpdate`: Tab (`Button.PDA`) opens the PDA while the
    // avatar's input is on (not focused); `PDA.ManagedUpdate` below closes
    // it with the same press once focused.
    let tab = keys.clear_just_pressed(KeyCode::Tab);
    if tab && !panel.pda.focused() && !panel.pda.in_use() {
        open(panel, &sim, false);
    }
    // Escape closes (ours: the game's Escape opens its menu); death too.
    if panel.pda.in_use() && keys.clear_just_pressed(KeyCode::Escape) {
        close(panel, &mut sim, &mut drive, &mut windows, "Escape");
    }
    // `Player.OnKill`: an opening or open PDA closes, then its input is
    // ignored until `ResetPlayerOnDeath` (ours: while the controls are
    // off, which also covers the cinematics, where `Open` is refused).
    let controls = sim.controls();
    if controls != panel.controls_seen {
        panel.controls_seen = controls;
        if !controls && matches!(panel.pda.state(), PdaState::Opening | PdaState::Opened) {
            close(
                panel,
                &mut sim,
                &mut drive,
                &mut windows,
                "the controls went off",
            );
        }
        panel.pda.set_ignore_input(!controls);
    }
    // `PDA.PerformUpdate`: the sequence, the button and the distance.
    if let Some(player) = sim.position() {
        let state = panel.pda.state();
        let events = panel
            .pda
            .update(time.delta_secs(), tab, panel.target, player);
        for e in events {
            match e {
                PdaEvent::Activated => {
                    // `Activated`: `UWE.Utils.lockCursor = false`.
                    set_cursor(&mut windows, true);
                    info!(
                        "inventory: PDA Activated at {:.3} s (PDA.time {:.3}): focused, the cursor free, clicks taken",
                        sim.play_seconds(),
                        panel.pda.time
                    );
                }
                PdaEvent::Deactivated => info!(
                    "inventory: PDA Deactivated at {:.3} s (PDA.time {:.3}): the model hidden, not in use",
                    sim.play_seconds(),
                    panel.pda.time
                ),
                PdaEvent::Close(why) => {
                    let why = match (why, panel.target) {
                        (CloseReason::Distance, Some(t)) => {
                            format!("{:.2} m from storagePivot", (t - player).length())
                        }
                        (CloseReason::Distance, None) => "the target is gone".into(),
                        (CloseReason::Button, _) => "Tab".into(),
                        (CloseReason::Call, _) => "a call".into(),
                    };
                    closed(panel, &mut sim, &mut drive, &mut windows, &why);
                }
                PdaEvent::Open => {}
            }
        }
        if panel.pda.state() != state {
            info!(
                "inventory: PDA {:?} → {:?} at {:.3} s",
                state,
                panel.pda.state(),
                sim.play_seconds()
            );
        }
    }

    // The screen takes clicks once focused: each mouse click to the
    // worker; the item clicks it found come back (`item_clicks`).
    panel.pointer = None;
    if !(panel.shown() && panel.pda.focused()) {
        panel.hover = None;
    }
    if panel.shown() && panel.pda.focused() {
        let window = windows.iter().next().map(|(w, _)| w);
        if let Some(w) = window
            && let Some(c) = w.physical_cursor_position()
        {
            let pixel = [c.x, w.physical_height() as f32 - c.y];
            panel.pointer = Some(pixel);
            let pressed = [
                (MouseButton::Left, 0u8),
                (MouseButton::Right, 1),
                (MouseButton::Middle, 2),
            ]
            .into_iter()
            .find(|(b, _)| buttons.just_pressed(*b));
            if let Some((b, button)) = pressed {
                buttons.clear_just_pressed(b);
                // The left button goes to the drag (M9u8b4): an item's
                // click comes on the release, as `FPSInputModule` sends it.
                if button == 0 {
                    panel.drag_inputs.push(DragInput::Down);
                } else {
                    panel.screen_clicks.push(ScreenClick::At { pixel, button });
                }
            }
        }
        if buttons.just_released(MouseButton::Left) {
            panel.drag_inputs.push(DragInput::Up);
        }
        // The drops the worker found (M9u8b4): the same `Move` a click
        // sends.
        let moves = frame
            .as_mut()
            .map(|f| std::mem::take(&mut f.item_moves))
            .unwrap_or_default();
        for request in moves {
            info!("inventory: drag → {request:?}");
            session.request(request);
        }
        let clicks = frame
            .as_mut()
            .map(|f| std::mem::take(&mut f.item_clicks))
            .unwrap_or_default();
        for (id, button) in clicks {
            let kind = match button {
                0 => Click::Left,
                1 => Click::Right,
                _ => Click::Middle,
            };
            click(&mut session, &sim, panel, text, ItemId(id as u32), kind);
        }
    } else if let Some(f) = frame.as_mut() {
        f.item_clicks.clear();
        f.item_moves.clear();
    }

    // Test scripts advance only after the server confirms TakeAll. Keep
    // normal input and game rules unchanged while the UI worker catches up.
    if !panel.script_transfers.is_empty() {
        let me = session.player();
        let confirmed = session
            .with_items(|items, _| {
                me.and_then(|me| items.players.get(&me)).is_some_and(|p| {
                    let ids: Vec<_> = p
                        .inventory
                        .container
                        .layout()
                        .0
                        .iter()
                        .map(|q| ItemId(q.id.0 as u32))
                        .collect();
                    panel.script_transfers.iter().all(|id| ids.contains(id))
                })
            })
            .unwrap_or(false);
        if confirmed {
            info!(
                "inventory: scripted transfer confirmed: {} items at {now:.3}s",
                panel.script_transfers.len()
            );
            panel.script_transfers.clear();
        } else if now >= panel.script_retry_at && panel.pda.focused() {
            // A scripted click can precede the animated screen's first
            // usable pose. Retry only items still in the server's locker;
            // never advance the script on the strength of queued clicks.
            let remaining = session
                .with_items(|items, _| {
                    items
                        .locker
                        .layout()
                        .0
                        .iter()
                        .map(|q| ItemId(q.id.0 as u32))
                        .filter(|id| panel.script_transfers.contains(id))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for item in remaining {
                panel.screen_clicks.push(ScreenClick::Item {
                    id: u64::from(item.0),
                    button: 0,
                });
            }
            panel.script_retry_at = now + 1.0;
        }
    }
    // `--panel`.
    if panel.script.last().is_some_and(|&(t, step)| {
        // A click waits while the PDA opens (not focused yet).
        let waits = !matches!(step, Scripted::Open | Scripted::Close)
            && panel.pda.state() == PdaState::Opening;
        now >= t && sim.controls() && !waits && panel.script_transfers.is_empty()
    }) {
        let Some((t, step)) = panel.script.pop() else {
            return;
        };
        info!("inventory: --panel at {t:.1} s: {step:?}");
        let me = session.player();
        let with_locker = panel.with_locker;
        // The grid's and the locker's items in their layout order.
        let lists = session
            .with_items(|items, _| {
                let p = items.players.get(&me?)?;
                let order = |c: &inv::Container| {
                    c.layout()
                        .0
                        .iter()
                        .map(|q| ItemId(q.id.0 as u32))
                        .collect::<Vec<_>>()
                };
                let equips: Vec<ItemId> = order(&p.inventory.container)
                    .into_iter()
                    .filter(|&i| {
                        let a = inv::item_actions(
                            &items.rules().techs,
                            &p.inventory,
                            None,
                            sim_id(i),
                            false,
                            DropPlace::default(),
                        );
                        a & (inv::EQUIP | inv::SWAP) != 0
                    })
                    .collect();
                let slots: Vec<ItemId> = p
                    .inventory
                    .equipment
                    .items()
                    .map(|i| ItemId(i.id.0 as u32))
                    .collect();
                Some((
                    order(&p.inventory.container),
                    order(&items.locker),
                    equips,
                    slots,
                ))
            })
            .flatten()
            .unwrap_or_default();
        let (grid, locker, equips, slots) = lists;
        match step {
            Scripted::Open => {
                open(panel, &sim, false);
            }
            Scripted::Close => close(panel, &mut sim, &mut drive, &mut windows, "--panel"),
            Scripted::TakeAll if panel.shown() && with_locker => {
                info!("inventory: taking {} items out of the locker", locker.len());
                panel.script_transfers.clone_from(&locker);
                panel.script_retry_at = now + 1.0;
                for item in locker {
                    panel.screen_clicks.push(ScreenClick::Item {
                        id: u64::from(item.0),
                        button: 0,
                    });
                }
            }
            Scripted::TakeAll => info!("inventory: the locker is not open"),
            Scripted::Equip => match equips.first() {
                Some(&item) => panel.screen_clicks.push(ScreenClick::Item {
                    id: u64::from(item.0),
                    button: 0,
                }),
                None => info!("inventory: nothing in the grid equips"),
            },
            Scripted::Drop => match grid.first() {
                Some(&item) => panel.screen_clicks.push(ScreenClick::Item {
                    id: u64::from(item.0),
                    button: 1,
                }),
                None => info!("inventory: nothing in the grid to drop"),
            },
            Scripted::Unequip => match slots.first() {
                Some(&item) => panel.screen_clicks.push(ScreenClick::Item {
                    id: u64::from(item.0),
                    button: 0,
                }),
                None => info!("inventory: nothing in the slots"),
            },
            Scripted::Tab(i) if panel.shown() && panel.pda.focused() => {
                panel.screen_clicks.push(ScreenClick::Icon(usize::from(i)));
            }
            Scripted::Tab(_) => info!("inventory: the PDA is not open"),
            Scripted::Hover(n) => match grid.get(usize::from(n)) {
                Some(&item) => panel.hover = Some(ScriptedHover::Item(u64::from(item.0))),
                None => info!("inventory: no grid item {n} to hover"),
            },
            Scripted::HoverEmpty | Scripted::OverGrid => {
                panel.hover = Some(ScriptedHover::LastCell)
            }
            Scripted::OverLocker => panel.hover = Some(ScriptedHover::LockerCell),
            Scripted::OverSlot(slot) => panel.hover = Some(ScriptedHover::Slot(slot)),
            Scripted::Drag(n) | Scripted::DragLocker(n) => {
                let list = if matches!(step, Scripted::DragLocker(_)) {
                    &locker
                } else {
                    &grid
                };
                if let Some(item) = list.get(usize::from(n)) {
                    let id = u64::from(item.0);
                    panel.hover = Some(ScriptedHover::Item(id));
                    panel.drag_inputs.push(DragInput::Start(id));
                } else {
                    info!("inventory: drag source {step:?} absent");
                }
            }
            Scripted::DragSlot(slot) => {
                panel.hover = Some(ScriptedHover::Slot(slot));
                if let Some(id) = session
                    .with_items(|items, _| {
                        Some(
                            items
                                .players
                                .get(&me?)?
                                .inventory
                                .equipment
                                .item_in(slot)?
                                .id
                                .0,
                        )
                    })
                    .flatten()
                {
                    panel.drag_inputs.push(DragInput::Start(id));
                }
            }
            Scripted::Press => panel.drag_inputs.push(DragInput::Down),
            Scripted::Release => panel.drag_inputs.push(DragInput::Up),
            Scripted::Cancel => panel.drag_inputs.push(DragInput::Cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_steps_parse() {
        assert_eq!(Scripted::parse("take-all"), Some(Scripted::TakeAll));
        assert_eq!(Scripted::parse("drop"), Some(Scripted::Drop));
        assert_eq!(Scripted::parse("unequip"), Some(Scripted::Unequip));
        assert_eq!(Scripted::parse("tab3"), Some(Scripted::Tab(3)));
        assert_eq!(Scripted::parse("tab"), None);
        assert_eq!(Scripted::parse("hover2"), Some(Scripted::Hover(2)));
        assert_eq!(Scripted::parse("hover-empty"), Some(Scripted::HoverEmpty));
        assert_eq!(Scripted::parse("hover"), None);
        assert_eq!(Scripted::parse("drag2"), Some(Scripted::Drag(2)));
        assert_eq!(
            Scripted::parse("drag-locker1"),
            Some(Scripted::DragLocker(1))
        );
        assert_eq!(
            Scripted::parse("drag-slotTank"),
            Some(Scripted::DragSlot("Tank"))
        );
        assert_eq!(Scripted::parse("drag-slotNope"), None);
        assert_eq!(
            Scripted::parse("over-slotChip1"),
            Some(Scripted::OverSlot("Chip1"))
        );
        assert_eq!(Scripted::parse("over-locker"), Some(Scripted::OverLocker));
        assert_eq!(Scripted::parse("release"), Some(Scripted::Release));
        assert_eq!(Scripted::parse("press"), Some(Scripted::Press));
        assert_eq!(Scripted::parse("nope"), None);
    }
}
