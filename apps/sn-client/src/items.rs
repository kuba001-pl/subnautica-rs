//! Items on this machine (M9d4b): follows the session's shared item state
//! and makes the world follow it. A gone key's drawn entities and
//! collision bodies go; the objects the server made are drawn and get
//! bodies. Two debug flags send requests without the hand
//! (`--pick-up`, `--drop`), for checking this from the logs.
//!
//! M9d5: the hand on items (`sn_assets::hand::aim`, M9d6a moved it there
//! for `sn-inspect`): what it shows over a pickup, an outcrop or the
//! lifepod's locker, and what a click asks the server; the
//! own inventory logged after each change (M9d5b: and the lifepod's
//! locker).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use sn_assets::ItemText;
use sn_protocol::{EntityKey, ItemId, Request};
use sn_world::Transform as Placement;

use crate::objects::ObjectStreamer;
use crate::player::PlayerSim;
use crate::session::Session;

/// `--pick-up`: requests for the pickups within this distance (m).
const PICK_UP_REACH: f64 = 3.0;
/// `--pick-up`: at most this many.
const PICK_UP_MAX: usize = 3;

/// The item state this machine has made the world follow.
#[derive(Resource, Default)]
pub struct ItemsView {
    pub craft_at: Vec<(f64, String)>,
    craft_line: String,
    /// World events applied when last looked at.
    events: Option<u32>,
    /// The own inventory's last logged line, and the blueprints known.
    inventory_line: String,
    known: Vec<i32>,
    /// The lifepod locker's last logged line (M9d5b).
    locker_line: String,
    /// `--drop`'s draws.
    draws: sn_session::drops::Draws,
    gone: HashSet<EntityKey>,
    objects: BTreeMap<ItemId, (String, [f32; 3], [i16; 4])>,
    /// `--pick-up`, `--drop`: seconds of play.
    pick_up_at: Option<f64>,
    drop_at: Option<f64>,
}

impl ItemsView {
    /// With the debug flags' times (seconds of play).
    pub fn new(
        pick_up_at: Option<f64>,
        drop_at: Option<f64>,
        craft_at: Vec<(f64, String)>,
    ) -> ItemsView {
        ItemsView {
            craft_at,
            pick_up_at,
            drop_at,
            draws: sn_session::drops::Draws(sn_net::random_seed()),
            ..Default::default()
        }
    }
}

/// The item text (names and the hand's keys) of the language read.
#[derive(Resource, Clone)]
pub struct Text(pub Arc<ItemText>);

/// The own inventory as one log line: the grid's items with their cells,
/// the slots in use, and the blueprints known.
fn describe(text: &ItemText, inv: &sn_sim::inventory::Inventory) -> String {
    let slots: Vec<String> = inv
        .equipment
        .slots()
        .filter_map(|(slot, item)| Some(format!("{slot}: {}", text.tech(item?.tech))))
        .collect();
    format!(
        "{}; slots [{}]; known {}",
        describe_grid(text, &inv.container),
        slots.join(", "),
        inv.known.known().count()
    )
}

/// A grid and its items with their cells.
fn describe_grid(text: &ItemText, c: &sn_sim::inventory::Container) -> String {
    let (placed, _) = c.layout();
    let tech_of = |id| {
        c.get(id)
            .map_or_else(|| "?".to_string(), |i| text.tech(i.tech))
    };
    let grid: Vec<String> = placed
        .iter()
        .map(|p| {
            format!(
                "{} #{} at ({}, {}) {}x{}",
                tech_of(p.id),
                p.id.0,
                p.x,
                p.y,
                p.width,
                p.height
            )
        })
        .collect();
    format!("grid {}x{} [{}]", c.size().0, c.size().1, grid.join(", "))
}

/// The wire's rotation as a placement's.
fn rotation(q: [i16; 4]) -> [f32; 4] {
    q.map(|c| f32::from(c) / 32_767.0)
}

#[allow(clippy::too_many_arguments)]
pub fn sync(
    text: Option<Res<Text>>,
    mut commands: Commands,
    mut view: ResMut<ItemsView>,
    mut session: ResMut<Session>,
    mut streamer: Option<ResMut<ObjectStreamer>>,
    sim: Option<Res<PlayerSim>>,
) {
    let me = session.player();
    let seen = view.events;
    let changed = session
        .with_items(|items, events| {
            if seen == Some(events) {
                return None;
            }
            let new_gone: Vec<EntityKey> = items
                .gone
                .keys()
                .filter(|k| !view.gone.contains(*k))
                .cloned()
                .collect();
            let objects: BTreeMap<ItemId, (String, [f32; 3], [i16; 4])> = items
                .objects
                .iter()
                .map(|(id, o)| (*id, (o.class.clone(), o.point, o.rotation)))
                .collect();
            let own = me.and_then(|p| items.players.get(&p));
            let held = own.map_or(0, |p| {
                p.inventory.container.len() + p.inventory.equipment.items().count()
            });
            let inventory = own.zip(text.as_ref()).map(|(p, t)| {
                (
                    describe(&t.0, &p.inventory),
                    p.inventory.known.known().collect::<Vec<i32>>(),
                )
            });
            let locker = text.as_ref().map(|t| describe_grid(&t.0, &items.locker));
            Some((events, new_gone, objects, held, inventory, locker))
        })
        .flatten();
    if let Some((events, new_gone, objects, held, inventory, locker)) = changed {
        view.events = Some(events);
        if let Some(line) = locker
            && line != view.locker_line
        {
            info!("items: lifepod locker after {events} world events: {line}");
            view.locker_line = line;
        }
        if let Some((line, known)) = inventory
            && line != view.inventory_line
        {
            info!("items: inventory after {events} world events: {line}");
            if let Some(t) = text.as_ref() {
                let new: Vec<String> = known
                    .iter()
                    .filter(|k| !view.known.contains(k))
                    .map(|&k| t.0.tech(k))
                    .collect();
                if !new.is_empty() && !view.known.is_empty() {
                    info!("items: blueprints unlocked: {}", new.join(", "));
                }
            }
            view.inventory_line = line;
            view.known = known;
        }
        if !new_gone.is_empty() {
            view.gone.extend(new_gone.iter().cloned());
            if let Some(s) = streamer.as_mut() {
                s.hide_keys(&mut commands, &new_gone);
            }
            if let Some(sim) = sim.as_ref() {
                sim.send_gone(new_gone.clone());
            }
        }
        let objects_changed = objects != view.objects;
        if objects_changed {
            let list: Vec<(ItemId, String, [f32; 3], [i16; 4])> = objects
                .iter()
                .map(|(id, (c, p, r))| (*id, c.clone(), *p, *r))
                .collect();
            if let Some(s) = streamer.as_mut() {
                let placed: Vec<(u32, String, Placement)> = list
                    .iter()
                    .map(|(id, c, p, r)| {
                        let at = Placement {
                            position: *p,
                            rotation: rotation(*r),
                            scale: [1.0; 3],
                        };
                        (id.0, c.clone(), at)
                    })
                    .collect();
                s.set_spawned(&mut commands, &placed, events);
            }
            if let Some(sim) = sim.as_ref() {
                sim.send_objects(list);
            }
            view.objects = objects;
        }
        if !new_gone.is_empty() || objects_changed {
            info!(
                "items: after {events} world events: {} keys gone (+{}), {} server-made objects, {held} items held; drawing {:?}",
                view.gone.len(),
                new_gone.len(),
                view.objects.len(),
                streamer.as_ref().map(|s| s.item_stats)
            );
        }
    }

    // Debug requests.
    let Some(sim) = sim else { return };
    let view = &mut *view;
    let play = sim.play_seconds();
    while view.craft_at.first().is_some_and(|(at, _)| play >= *at) {
        let (_, name) = view.craft_at.remove(0);
        let request = if name == "take" {
            Some(Request::TakeCrafted { crafter: 0 })
        } else {
            text.as_ref().and_then(|t| {
                t.0.tech_names
                    .iter()
                    .find(|(_, n)| *n == &name)
                    .map(|(&tech, _)| Request::Craft { crafter: 0, tech })
            })
        };
        if let Some(q) = request {
            info!("craft: --craft at {play:.3}s {q:?}");
            session.request(q);
        } else {
            info!("craft: unknown tech {name:?}");
        }
    }
    if let Some(line) = session.with_items(|items, events| {
        format!(
            "events={events} power={} job={:?}",
            items.fabricator.power, items.fabricator.job
        )
    }) && line != view.craft_line
    {
        info!("craft: {line}");
        view.craft_line = line;
    }
    if view.pick_up_at.is_some_and(|t| play >= t) {
        view.pick_up_at = None;
        let near: Vec<_> = sim
            .targets_near(PICK_UP_REACH)
            .into_iter()
            .filter(|t| t.kind == sn_assets::TargetKind::Pickup)
            .take(PICK_UP_MAX)
            .collect();
        info!(
            "items: --pick-up at {play:.1} s: {} pickups within {PICK_UP_REACH} m",
            near.len()
        );
        for t in near {
            let class = match t.key {
                EntityKey::Spawned(_) => String::new(),
                _ => t.class.clone(),
            };
            session.request(Request::PickUp { key: t.key, class });
        }
    }
    if view.drop_at.is_some_and(|t| play >= t) {
        view.drop_at = None;
        let first = me.and_then(|p| {
            session
                .with_items(|items, _| {
                    let i = items.players.get(&p)?.inventory.container.items().next()?;
                    let id = ItemId(i.id.0 as u32);
                    let randomize = items
                        .classes
                        .get(&id)
                        .and_then(|c| items.rules().pickups.get(c))
                        .is_some_and(|i| i.randomize_rotation);
                    Some((id, randomize))
                })
                .flatten()
        });
        // Where the game drops it (M9d5b: `InternalDropItem`).
        let request = first.and_then(|(item, randomize)| {
            crate::inventory::drop_request(&sim, &mut view.draws, item, randomize)
        });
        match request {
            Some(r) => {
                info!("items: --drop at {play:.1} s");
                session.request(r);
            }
            None => info!("items: --drop at {play:.1} s: nothing held"),
        }
    }
}
