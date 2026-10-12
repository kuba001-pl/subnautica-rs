//! Items on this machine (M9d4b): follows the session's shared item state
//! and makes the world follow it. A gone key's drawn entities and
//! collision bodies go; the objects the server made are drawn and get
//! bodies. Two debug flags send requests without the hand
//! (`--pick-up`, `--drop`), for checking this from the logs.
//!
//! M9d5: the hand on items ([`aim`]): what it shows over a pickup, an
//! outcrop or the lifepod's locker, and what a click asks the server; the
//! own inventory logged after each change (M9d5b: and the lifepod's
//! locker).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use sn_assets::{ItemText, TargetBody, TargetKind};
use sn_protocol::{EntityKey, HitKind, ItemId, Request};
use sn_sim::V3;
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
    /// The own inventory's last logged line, and the blueprints known.
    inventory_line: String,
    known: Vec<i32>,
    /// The lifepod locker's last logged line (M9d5b).
    locker_line: String,
    /// `--drop`'s draws.
    random: u64,
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
            random: sn_net::random_seed(),
            ..Default::default()
        }
    }
}

/// The item text (names and the hand's keys) of the language read.
#[derive(Resource, Clone)]
pub struct Text(pub Arc<ItemText>);

/// Language keys the game's code passes as literals (M9d5): the pick-up
/// text (`LanguageCache.GetPickupText`), the full inventory
/// (`Pickupable.OnHandHover`), the lifepod's locker
/// (`EscapePod.StorageHover`).
const PICK_UP_FORMAT: &str = "PickUpFormat";
const INVENTORY_FULL: &str = "InventoryFull";
const POD_STORAGE: &str = "EscapePodStorageOpen";

/// What the hand is on, as the game's `OnHandHover` / `OnHandClick` of
/// that object see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Aim {
    /// `HandReticle`'s hand text and its subscript.
    pub text: String,
    pub subscript: String,
    pub action: AimAction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AimAction {
    /// A click asks for this, and plays the grab (`Player.PlayGrab`).
    Request(Request),
    /// A click opens the lifepod's locker (`EscapePod.StorageOpen`).
    OpenLocker,
    /// A click does nothing (no room, not pickupable).
    None,
}

/// The hand on a target (a pickup or an outcrop) or on the lifepod's
/// locker. `room`: whether the own inventory has room for a tech type
/// (`Player.HasInventoryRoom`).
pub fn aim(
    text: &ItemText,
    rules: &sn_sim::inventory::ItemRules,
    target: Option<&TargetBody>,
    locker: bool,
    room: impl Fn(i32) -> bool,
) -> Option<Aim> {
    if locker {
        return Some(Aim {
            text: text.get(POD_STORAGE).to_string(),
            subscript: String::new(),
            action: AimAction::OpenLocker,
        });
    }
    let t = target?;
    // A server-made object's class is the server's to say.
    let class = match t.key {
        EntityKey::Spawned(_) => String::new(),
        _ => t.class.clone(),
    };
    match t.kind {
        TargetKind::Pickup => {
            let info = rules.pickups.get(&t.class)?;
            let name = text.tech(info.tech);
            // `AllowedToPickUp`: pickupable and room (the 1 s after a drop
            // always passes, see `sn_sim::inventory::pickup`).
            Some(if info.pickupable && room(info.tech) {
                Aim {
                    text: text.format(PICK_UP_FORMAT, &name),
                    subscript: String::new(),
                    action: AimAction::Request(Request::PickUp {
                        key: t.key.clone(),
                        class,
                    }),
                }
            } else if info.pickupable {
                Aim {
                    text: name,
                    subscript: text.get(INVENTORY_FULL).to_string(),
                    action: AimAction::None,
                }
            } else {
                Aim {
                    text: name,
                    subscript: String::new(),
                    action: AimAction::None,
                }
            })
        }
        TargetKind::Outcrop => {
            let info = rules.outcrops.get(&t.class)?;
            // `BreakIntoResources` spawns at `position + up ×
            // verticalSpawnOffset`.
            let up = sn_sim::Q::from_f32(t.at.rotation).rotate(V3::new(0.0, 1.0, 0.0));
            let spawn = sn_sim::inventory::Outcrop::spawn_point(
                V3::from_f32(t.at.position),
                up,
                f64::from(info.vertical_offset),
            );
            Some(Aim {
                text: text.get(&info.break_text).to_string(),
                subscript: String::new(),
                action: AimAction::Request(Request::Hit {
                    key: t.key.clone(),
                    class,
                    // No tool held: `PlayBash` is false, so the hand hits.
                    kind: HitKind::Hand,
                    spawn: spawn.to_f32(),
                }),
            })
        }
    }
}

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
    let view = &mut *view;
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
            crate::inventory::drop_request(&sim, &mut view.random, item, randomize)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sn_sim::inventory::{ItemRules, OutcropInfo, PickupInfo};

    use super::*;

    fn text() -> ItemText {
        ItemText {
            tech_names: HashMap::from([(5, "Titanium".to_string()), (7, "Limestone".to_string())]),
            strings: HashMap::from([
                ("Titanium".to_string(), "Ti".to_string()),
                ("PickUpFormat".to_string(), "Pick up {0}".to_string()),
                ("InventoryFull".to_string(), "Full".to_string()),
                ("BreakLime".to_string(), "Break it".to_string()),
                ("EscapePodStorageOpen".to_string(), "Open".to_string()),
            ]),
        }
    }

    fn rules() -> ItemRules {
        let mut r = ItemRules::default();
        let pickup = |tech, pickupable| PickupInfo {
            tech,
            pickupable,
            destroy_on_death: true,
            eatable: false,
            randomize_rotation: true,
        };
        r.pickups.insert("ti".into(), pickup(5, true));
        r.pickups.insert("stuck".into(), pickup(5, false));
        r.outcrops.insert(
            "lime".into(),
            OutcropInfo {
                hits_to_break: 3,
                vertical_offset: 0.5,
                num_chances: 1,
                list: Vec::new(),
                default_class: "ti".into(),
                break_text: "BreakLime".into(),
            },
        );
        r
    }

    fn target(class: &str, kind: TargetKind, key: EntityKey) -> TargetBody {
        TargetBody {
            key,
            class: class.into(),
            kind,
            at: Placement {
                position: [1.0, 2.0, 3.0],
                // 90° about z: up becomes −x.
                rotation: [
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                ],
                scale: [1.0; 3],
            },
        }
    }

    #[test]
    fn the_hand_on_items_says_and_asks_what_the_game_does() {
        let (t, r) = (text(), rules());
        let placed = EntityKey::Placed("a".into());
        let ti = target("ti", TargetKind::Pickup, placed.clone());

        let a = aim(&t, &r, Some(&ti), false, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Pick up Ti", ""));
        assert_eq!(
            a.action,
            AimAction::Request(Request::PickUp {
                key: placed.clone(),
                class: "ti".into()
            })
        );

        // No room: the name and "inventory full", no request.
        let a = aim(&t, &r, Some(&ti), false, |_| false).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Ti", "Full"));
        assert_eq!(a.action, AimAction::None);

        // Not pickupable: the name only.
        let stuck = target("stuck", TargetKind::Pickup, placed.clone());
        let a = aim(&t, &r, Some(&stuck), false, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Ti", ""));
        assert_eq!(a.action, AimAction::None);

        // A server-made object: the server knows its class.
        let spawned = target("ti", TargetKind::Pickup, EntityKey::Spawned(ItemId(9)));
        let a = aim(&t, &r, Some(&spawned), false, |_| true).unwrap();
        assert_eq!(
            a.action,
            AimAction::Request(Request::PickUp {
                key: EntityKey::Spawned(ItemId(9)),
                class: String::new()
            })
        );

        // An outcrop: its break text; a hand hit, resources at
        // position + up × verticalSpawnOffset.
        let lime = target("lime", TargetKind::Outcrop, placed.clone());
        let a = aim(&t, &r, Some(&lime), false, |_| true).unwrap();
        assert_eq!(a.text, "Break it");
        let AimAction::Request(Request::Hit { kind, spawn, .. }) = a.action else {
            panic!("{:?}", a.action);
        };
        assert_eq!(kind, HitKind::Hand);
        for (s, e) in spawn.iter().zip([0.5, 2.0, 3.0]) {
            assert!((s - e).abs() < 1e-5, "{spawn:?}");
        }

        // The locker; nothing; a class the rules do not know.
        let a = aim(&t, &r, None, true, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.action), ("Open", AimAction::OpenLocker));
        assert!(aim(&t, &r, None, false, |_| true).is_none());
        let unknown = target("x", TargetKind::Pickup, placed);
        assert!(aim(&t, &r, Some(&unknown), false, |_| true).is_none());
    }
}
