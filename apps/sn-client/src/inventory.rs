//! The inventory panel (M9d5b, `docs/DESIGN.md` "M9d5b plan"): our own
//! Bevy UI standing in for the PDA's inventory tab until M9u. Tab opens
//! it (the grid and the slots); the hand on the lifepod's locker opens it
//! with the locker beside them, as `EscapePod.StorageOpen` does (the pod's
//! `storage_open`, the panel 0.25 s later). While it is open the look and
//! the hand are off and the cursor is free (`crate::player` asks
//! [`Panel::open`]). A left click does the item's action by the game's
//! rule (`sn_sim::inventory::item_action`), a right click drops it where
//! the game would ([`drop_request`]). `--panel` does the same from a
//! script, for checking from the logs.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use sn_assets::ItemText;
use sn_protocol::{ItemId, MoveTarget, Request, Store};
use sn_session::drops::{Draws, drop_at};
use sn_sim::V3;
use sn_sim::inventory::{self as inv, Button as Click, DropPlace, Place};

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
            _ => return None,
        })
    }
}

#[derive(Resource)]
pub struct Panel {
    /// The PDA is open (`PDA.isInUse`).
    pub open: bool,
    /// Opened by the locker: the locker is the used storage.
    with_locker: bool,
    /// `EscapePod.storageSequence`: when it ends (play seconds) and
    /// whether it then opens the panel.
    storage: Option<(f64, bool)>,
    /// `PlayerSim::locker_opened` last seen.
    locker_seen: u32,
    /// `PDA.target` and `activeSqrDistance`.
    target: Option<(V3, f64)>,
    /// The drops' draws (the game's `Random` is unseeded).
    draws: Draws,
    /// `--panel`, latest first.
    script: Vec<(f64, Scripted)>,
    /// What the UI shows (rebuilt when it changes).
    drawn: Option<String>,
}

impl Panel {
    pub fn new(mut script: Vec<(f64, Scripted)>, seed: u64) -> Panel {
        script.sort_by(|a, b| b.0.total_cmp(&a.0));
        Panel {
            open: false,
            with_locker: false,
            storage: None,
            locker_seen: 0,
            target: None,
            draws: Draws(seed),
            script,
            drawn: None,
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

/// `PDA.Open`: refused while open, dead or in a cinematic.
fn open(
    panel: &mut Panel,
    sim: &PlayerSim,
    windows: &mut Query<(&Window, &mut CursorOptions)>,
    with_locker: bool,
) -> bool {
    if panel.open || !sim.controls() {
        info!(
            "inventory: the panel cannot open now (open {}, controls {})",
            panel.open,
            sim.controls()
        );
        return false;
    }
    panel.open = true;
    panel.with_locker = with_locker;
    panel.target = if with_locker {
        sim.locker_centre()
            .zip(sim.position())
            .map(|(t, p)| (t, (t - p).dot(t - p) + 1.0))
    } else {
        None
    };
    set_cursor(windows, true);
    info!(
        "inventory: panel open{}",
        match panel.target {
            Some((_, d)) => format!(
                " with the locker; closes beyond {:.2} m (activeSqrDistance {d:.2})",
                d.sqrt()
            ),
            None => String::new(),
        }
    );
    true
}

/// `PDA.Close`, then its callback: for the locker,
/// `EscapePod.StorageOnClosePDA`.
fn close(
    panel: &mut Panel,
    sim: &mut PlayerSim,
    drive: &mut BodyDrive,
    windows: &mut Query<(&Window, &mut CursorOptions)>,
    why: &str,
) {
    if !panel.open {
        return;
    }
    panel.open = false;
    panel.target = None;
    set_cursor(windows, false);
    info!("inventory: panel closed ({why})");
    if std::mem::take(&mut panel.with_locker) {
        storage_closed(panel, sim, drive);
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
        if opens && !open(panel, &sim, &mut windows, true) {
            storage_closed(panel, &mut sim, &mut drive);
        }
    }

    // Tab (`GameInput.Button.PDA`) toggles; Escape closes (ours).
    if keys.clear_just_pressed(KeyCode::Tab) {
        if panel.open {
            close(panel, &mut sim, &mut drive, &mut windows, "Tab");
        } else {
            open(panel, &sim, &mut windows, false);
        }
    }
    if panel.open && keys.clear_just_pressed(KeyCode::Escape) {
        close(panel, &mut sim, &mut drive, &mut windows, "Escape");
    }
    if panel.open && !sim.controls() {
        close(panel, &mut sim, &mut drive, &mut windows, "no controls");
    }
    // `PDA.ManagedUpdate`: too far from the target.
    if let (true, Some((at, limit)), Some(p)) = (panel.open, panel.target, sim.position()) {
        let d = (at - p).dot(at - p);
        if d >= limit {
            let why = format!(
                "{:.2} m from the locker, limit {:.2} m",
                d.sqrt(),
                limit.sqrt()
            );
            close(panel, &mut sim, &mut drive, &mut windows, &why);
        }
    }

    // Clicks on an item.
    if panel.open {
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
            }
        }
    }

    // `--panel`.
    while panel
        .script
        .last()
        .is_some_and(|&(t, _)| now >= t && sim.controls())
    {
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
                open(panel, &sim, &mut windows, false);
            }
            Scripted::Close => close(panel, &mut sim, &mut drive, &mut windows, "--panel"),
            Scripted::TakeAll if panel.open && with_locker => {
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
    let want = if panel.open {
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
    let view = if panel.open {
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
        assert_eq!(Scripted::parse("nope"), None);
    }
}
