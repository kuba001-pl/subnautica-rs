//! Items on this machine (M9d4b): follows the session's shared item state
//! and makes the world follow it. A gone key's drawn entities and
//! collision bodies go; the objects the server made are drawn and get
//! bodies. Two debug flags send requests without the hand
//! (`--pick-up`, `--drop`), for checking this from the logs.

use std::collections::{BTreeMap, HashSet};

use bevy::prelude::*;
use sn_protocol::{DropAt, EntityKey, ItemId, Request};
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
    /// World events applied when last looked at.
    events: Option<u32>,
    gone: HashSet<EntityKey>,
    objects: BTreeMap<ItemId, (String, [f32; 3], [i16; 4])>,
    /// `--pick-up`, `--drop`: seconds of play.
    pick_up_at: Option<f64>,
    drop_at: Option<f64>,
}

impl ItemsView {
    /// With the debug flags' times (seconds of play).
    pub fn new(pick_up_at: Option<f64>, drop_at: Option<f64>) -> ItemsView {
        ItemsView {
            pick_up_at,
            drop_at,
            ..Default::default()
        }
    }
}

/// The wire's rotation as a placement's.
fn rotation(q: [i16; 4]) -> [f32; 4] {
    q.map(|c| f32::from(c) / 32_767.0)
}

pub fn sync(
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
            let held = me.and_then(|p| items.players.get(&p)).map_or(0, |p| {
                p.inventory.container.len() + p.inventory.equipment.items().count()
            });
            Some((events, new_gone, objects, held))
        })
        .flatten();
    if let Some((events, new_gone, objects, held)) = changed {
        view.events = Some(events);
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
                s.set_spawned(&mut commands, &placed);
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
    let play = sim.play_seconds();
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
                    items
                        .players
                        .get(&p)?
                        .inventory
                        .container
                        .items()
                        .next()
                        .map(|i| ItemId(i.id.0 as u32))
                })
                .flatten()
        });
        match (first, sim.eye()) {
            (Some(item), Some((at, forward))) => {
                // Our stand-in for the drop ray (M9d5 ports it): a metre
                // ahead of the player.
                let point = (at + forward).to_f32();
                info!("items: --drop at {play:.1} s: item {} at {point:?}", item.0);
                session.request(Request::Drop {
                    item,
                    at: DropAt {
                        point,
                        rotation: [0, 0, 0, 32_767],
                    },
                });
            }
            _ => info!("items: --drop at {play:.1} s: nothing held"),
        }
    }
}
