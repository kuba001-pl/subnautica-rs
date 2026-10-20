//! The PDA's open and close (M9u8a3, `sn_sim::pda`) and the inventory
//! panel (M9d5b, `docs/DESIGN.md` "M9d5b plan"): our own Bevy UI standing
//! in for the PDA's inventory tab until M9u8b. Tab opens the PDA
//! (`GUIHand`'s `Button.PDA`) and closes it once focused
//! (`PDA.ManagedUpdate`); the hand on the lifepod's locker opens it with
//! the locker beside the grid, as `EscapePod.StorageOpen` does (the pod's
//! `storage_open`, `PDA.Open` 0.25 s later, the target `storagePivot`).
//! The panel shows from `Open` to `Close` (the screen's content) and takes
//! clicks once the PDA is focused (`Activated`); the cursor is free from
//! `Activated` to `Close`. `crate::player` reads [`Panel::pda`] for the
//! look, the hand and the camera's locked mode, `crate::body` for the
//! model. A left click does the item's action by the game's rule
//! (`sn_sim::inventory::item_action`), a right click drops it where the
//! game would ([`drop_request`]). A click elsewhere goes to the PDA
//! screen's toolbar (M9u8a4b, [`ToolbarClick`]; hit tested by the UI
//! worker). `--panel` does the same from a script, for checking from the
//! logs.

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
/// A grid cell on screen (px; our layout).
const CELL: f32 = 46.0;

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
}

/// A click for the PDA screen's toolbar (M9u8a4b), taken by
/// `crate::game_ui`'s worker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolbarClick {
    /// The mouse: physical pixels from the bottom left (Unity's screen
    /// space) and `PointerEventData`'s button (0 left, 1 right, 2 middle).
    At { pixel: [f32; 2], button: u8 },
    /// `--panel tabN`: the left button at icon N's centre, projected.
    Icon(usize),
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
            _ => Scripted::Tab(s.strip_prefix("tab")?.parse().ok()?),
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
    /// What the UI shows (rebuilt when it changes).
    drawn: Option<String>,
    /// Clicks for the PDA's toolbar since the last frame sent to the UI
    /// worker.
    pub toolbar_clicks: Vec<ToolbarClick>,
}

impl Panel {
    /// The panel (the inventory tab's content) is shown: from `Open` to
    /// `Close` (`content.SetVisible`).
    pub fn shown(&self) -> bool {
        self.pda.using_pda()
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
            drawn: None,
            toolbar_clicks: Vec::new(),
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
    boxes: Query<(&Interaction, &ItemBox)>,
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

    // Clicks on an item (the content is interactable once focused).
    if panel.shown() && panel.pda.focused() {
        let pressed = [
            (MouseButton::Left, Click::Left),
            (MouseButton::Right, Click::Right),
            (MouseButton::Middle, Click::Middle),
        ]
        .into_iter()
        .find(|(b, _)| buttons.just_pressed(*b));
        if let Some((b, click_kind)) = pressed {
            buttons.clear_just_pressed(b);
            let over = boxes
                .iter()
                .find(|(i, _)| **i != Interaction::None)
                .map(|(_, b)| b.0);
            if let Some(item) = over {
                click(&mut session, &sim, panel, text, item, click_kind);
            } else if let Some((window, _)) = windows.iter().next()
                && let Some(c) = window.physical_cursor_position()
            {
                // Not on our panel: the PDA screen's (only its toolbar
                // takes clicks so far).
                let button = match b {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    _ => 2,
                };
                let pixel = [c.x, window.physical_height() as f32 - c.y];
                panel
                    .toolbar_clicks
                    .push(ToolbarClick::At { pixel, button });
            }
        }
    }

    // `--panel`.
    while panel.script.last().is_some_and(|&(t, step)| {
        // A click waits while the PDA opens (not focused yet).
        let waits = !matches!(step, Scripted::Open | Scripted::Close)
            && panel.pda.state() == PdaState::Opening;
        now >= t && sim.controls() && !waits
    }) {
        let Some((t, step)) = panel.script.pop() else {
            break;
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
                for item in locker {
                    click(&mut session, &sim, panel, text, item, Click::Left);
                }
            }
            Scripted::TakeAll => info!("inventory: the locker is not open"),
            Scripted::Equip => match equips.first() {
                Some(&item) => click(&mut session, &sim, panel, text, item, Click::Left),
                None => info!("inventory: nothing in the grid equips"),
            },
            Scripted::Drop => match grid.first() {
                Some(&item) => click(&mut session, &sim, panel, text, item, Click::Right),
                None => info!("inventory: nothing in the grid to drop"),
            },
            Scripted::Unequip => match slots.first() {
                Some(&item) => click(&mut session, &sim, panel, text, item, Click::Left),
                None => info!("inventory: nothing in the slots"),
            },
            Scripted::Tab(i) if panel.shown() && panel.pda.focused() => {
                panel
                    .toolbar_clicks
                    .push(ToolbarClick::Icon(usize::from(i)));
            }
            Scripted::Tab(_) => info!("inventory: the PDA is not open"),
        }
    }
}

#[derive(Component)]
pub struct PanelRoot;

/// An item's box on the panel.
#[derive(Component)]
pub struct ItemBox(pub ItemId);

pub fn setup(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        GlobalZIndex(3),
        Visibility::Hidden,
        PanelRoot,
    ));
}

/// An item as drawn: its place in a grid and its name.
#[derive(Debug)]
struct Boxed {
    id: ItemId,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    name: String,
}

#[derive(Debug)]
struct View {
    grid: ((i32, i32), Vec<Boxed>),
    slots: Vec<(String, Option<(ItemId, String)>)>,
    locker: Option<((i32, i32), Vec<Boxed>)>,
}

fn boxes(c: &inv::Container, text: &ItemText) -> ((i32, i32), Vec<Boxed>) {
    let list = c
        .layout()
        .0
        .iter()
        .map(|p| Boxed {
            id: ItemId(p.id.0 as u32),
            x: p.x,
            y: p.y,
            width: p.width,
            height: p.height,
            name: c.get(p.id).map_or_else(String::new, |i| text.tech(i.tech)),
        })
        .collect();
    (c.size(), list)
}

const BOX: Color = Color::srgba(0.18, 0.42, 0.55, 0.95);
const BOX_HOVER: Color = Color::srgba(0.3, 0.62, 0.78, 0.95);

fn grid_node(p: &mut ChildSpawnerCommands, title: &str, (size, list): &((i32, i32), Vec<Boxed>)) {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|col| {
        col.spawn((
            Text::new(title),
            TextFont::from_font_size(16.0),
            TextColor(Color::WHITE),
        ));
        col.spawn((
            Node {
                width: Val::Px(size.0 as f32 * CELL),
                height: Val::Px(size.1 as f32 * CELL),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.1, 0.14, 0.9)),
        ))
        .with_children(|g| {
            for y in 0..size.1 {
                for x in 0..size.0 {
                    g.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x as f32 * CELL + 1.0),
                            top: Val::Px(y as f32 * CELL + 1.0),
                            width: Val::Px(CELL - 2.0),
                            height: Val::Px(CELL - 2.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                    ));
                }
            }
            for b in list {
                g.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(b.x as f32 * CELL + 2.0),
                        top: Val::Px(b.y as f32 * CELL + 2.0),
                        width: Val::Px(b.width as f32 * CELL - 4.0),
                        height: Val::Px(b.height as f32 * CELL - 4.0),
                        padding: UiRect::all(Val::Px(2.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(BOX),
                    Interaction::None,
                    ItemBox(b.id),
                ))
                .with_children(|i| {
                    i.spawn((
                        Text::new(b.name.clone()),
                        TextFont::from_font_size(10.0),
                        TextColor(Color::WHITE),
                    ));
                });
            }
        });
    });
}

/// After the items: the panel shows the own grid, the slots and (when
/// open by the locker) the locker, rebuilt when any of them changes.
pub fn draw(
    mut commands: Commands,
    mut panel: ResMut<Panel>,
    session: Res<Session>,
    text: Option<Res<ItemTextRes>>,
    mut root: Query<(Entity, &mut Visibility), With<PanelRoot>>,
) {
    let Ok((root, mut visibility)) = root.single_mut() else {
        return;
    };
    let want = if panel.shown() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != want {
        *visibility = want;
    }
    let Some(text) = text else { return };
    let me = session.player();
    let with_locker = panel.with_locker;
    let view = if panel.shown() {
        session
            .with_items(|items, _| {
                let p = items.players.get(&me?)?;
                Some(View {
                    grid: boxes(&p.inventory.container, &text.0),
                    slots: p
                        .inventory
                        .equipment
                        .slots()
                        .map(|(s, i)| {
                            (
                                s.to_string(),
                                i.map(|i| (ItemId(i.id.0 as u32), text.0.tech(i.tech))),
                            )
                        })
                        .collect(),
                    locker: with_locker.then(|| boxes(&items.locker, &text.0)),
                })
            })
            .flatten()
    } else {
        None
    };
    let key = format!("{view:?}");
    if panel.drawn.as_ref() == Some(&key) {
        return;
    }
    panel.drawn = Some(key);
    commands.entity(root).despawn_related::<Children>();
    let Some(view) = view else { return };
    commands.entity(root).with_children(|p| {
        p.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(28.0),
                padding: UiRect::all(Val::Px(18.0)),
                align_items: AlignItems::FlexStart,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        ))
        .with_children(|row| {
            if let Some(locker) = &view.locker {
                grid_node(row, "Locker", locker);
            }
            grid_node(row, "Inventory", &view.grid);
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    Text::new("Equipment"),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::WHITE),
                ));
                for (slot, item) in &view.slots {
                    let mut e = col.spawn((
                        Node {
                            width: Val::Px(3.0 * CELL),
                            height: Val::Px(CELL * 0.6),
                            padding: UiRect::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(if item.is_some() {
                            BOX
                        } else {
                            Color::srgba(1.0, 1.0, 1.0, 0.06)
                        }),
                    ));
                    if let Some((id, _)) = item {
                        e.insert((Interaction::None, ItemBox(*id)));
                    }
                    let label = match item {
                        Some((_, name)) => format!("{slot}: {name}"),
                        None => slot.clone(),
                    };
                    e.with_children(|t| {
                        t.spawn((
                            Text::new(label),
                            TextFont::from_font_size(11.0),
                            TextColor(Color::WHITE),
                        ));
                    });
                }
            });
        });
    });
}

/// The hovered item's box lights up.
#[allow(clippy::type_complexity)]
pub fn hover(
    mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<ItemBox>)>,
) {
    for (i, mut c) in &mut q {
        c.0 = if *i == Interaction::None {
            BOX
        } else {
            BOX_HOVER
        };
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
        assert_eq!(Scripted::parse("nope"), None);
    }
}
