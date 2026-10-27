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
    fx: Vec<Entity>,
    fx_active: bool,
    animation_line: Option<(bool, bool)>,
    pub fabricator_open: bool,
    opened_seen: u32,
    /// The fabricator's ghost: tech, class and placement (drawn as the
    /// local-only object [`GHOST_ID`]).
    preview: Option<(i32, String, Placement)>,
    /// The output tech the ghost was last made for.
    ghost_tech: Option<i32>,
    /// The end tick of the craft whose completion was handled.
    pickup_tried: Option<u32>,
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
    mut sim: Option<ResMut<PlayerSim>>,
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
            view.objects = objects;
            if let Some(s) = streamer.as_mut() {
                s.set_spawned(&mut commands, &placed(&view), events);
            }
            if let Some(sim) = sim.as_ref() {
                sim.send_objects(list);
            }
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
    let Some(mut sim) = sim.take() else { return };
    let view = &mut *view;
    let play = sim.play_seconds();
    let fabricator = session.with_items(|items, _| items.fabricator.clone());
    if let Some(f) = &sim.fabricator {
        let distance = sim.position().map_or(f64::INFINITY, |p| {
            (p - sn_sim::V3::from_f32(f.at.position)).length()
        });
        let has_output = fabricator.as_ref().is_some_and(|f| f.job.is_some());
        let was_open = view.fabricator_open;
        view.fabricator_open = sn_sim::craft::opened(
            was_open,
            sim.fabricator_opened != view.opened_seen,
            has_output,
            distance < f64::from(f.close_distance),
            fabricator.as_ref().map_or(0.0, |f| f.power),
        );
        view.opened_seen = sim.fabricator_opened;
        if was_open != view.fabricator_open {
            info!(
                "fabricator: open={} distance={distance:.4} closeDistance={}",
                view.fabricator_open, f.close_distance
            );
        }
        // GhostCrafter.OnCraftingEnd, once per craft: pickup in range, else
        // the CraftingEnd message.
        if let Some(job) = fabricator.as_ref().and_then(|f| f.job.as_ref()) {
            if job.done && view.pickup_tried != Some(job.end) {
                view.pickup_tried = Some(job.end);
                if Some(job.owner) != me {
                    info!("fabricator: craft done (another player's)");
                } else if distance < f64::from(f.close_distance) {
                    info!(
                        "fabricator: craft done, pickup at distance {distance:.4} < closeDistance {}",
                        f.close_distance
                    );
                    session.request(Request::TakeCrafted { crafter: 0 });
                } else {
                    info!(
                        "fabricator: craft done at distance {distance:.4} >= closeDistance {}: CraftingEnd (formatted message not shown: not ported)",
                        f.close_distance
                    );
                }
            }
        }
        // CrafterGhostModel.UpdateModel(currentTechType): the output's ghost
        // at itemSpawnPoint while the machine holds it (crafting or done).
        let tech = fabricator
            .as_ref()
            .and_then(|f| f.job.as_ref())
            .and_then(|j| j.output.current());
        if tech != view.ghost_tech {
            view.ghost_tech = tech;
            let class = tech.and_then(|tech| {
                session
                    .with_items(|items, _| items.rules().tech_class.get(&tech).cloned())
                    .flatten()
            });
            let ghost = class.as_ref().map(|c| f.ghosts.get(c));
            let at = match ghost {
                Some(Some(Some(g))) => {
                    info!(
                        "fabricator: ghost of tech {tech:?} ({:?}, node {:?}) at itemSpawnPoint {:?}, root {:?}; drawn plainly: FX_BUILDING (_Built progress) not ported",
                        class, g.node, f.item_spawn.position, g.root.position
                    );
                    Some(f.item_spawn.then(&g.root))
                }
                Some(Some(None)) => {
                    info!(
                        "fabricator: tech {tech:?} ({class:?}) has no ghost model in the game; none drawn"
                    );
                    None
                }
                Some(g) => {
                    info!(
                        "fabricator: tech {tech:?} ({class:?}): {}; whole prefab drawn plainly at itemSpawnPoint {:?} (not the game's)",
                        if g.is_some() {
                            "ghost node inactive in the prefab"
                        } else {
                            "ghost not read"
                        },
                        f.item_spawn.position
                    );
                    Some(f.item_spawn)
                }
                None => {
                    info!("fabricator: ghost removed (the machine is empty)");
                    None
                }
            };
            if let Some(streamer) = streamer.as_mut() {
                let events = view.events.unwrap_or(0);
                if view.preview.take().is_some() {
                    streamer.set_spawned(&mut commands, &placed(view), events);
                }
                view.preview = tech.zip(class).zip(at).map(|((t, c), a)| (t, c, a));
                streamer.set_spawned(&mut commands, &placed(view), events);
            }
        }
    }
    while view.craft_at.first().is_some_and(|(at, _)| play >= *at) {
        let (_, name) = view.craft_at.remove(0);
        if name == "open" {
            sim.use_fabricator = true;
            continue;
        }
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

/// The fabricator ghost's object id: item ids never reach it (allocation
/// checks `next + 1`), and it is never simulated or sent.
const GHOST_ID: u32 = u32::MAX;

#[derive(Component)]
pub struct FabricatorGhostAttached;

/// The preview follows the animator's exposed spawn point. Its initial world
/// transform was authored relative to the stored spawn point, so rebase once.
pub fn attach_fabricator_preview(
    mut commands: Commands,
    sim: Option<Res<PlayerSim>>,
    streamer: Option<Res<ObjectStreamer>>,
    anchors: Query<&crate::objects::NamedNodes>,
    mut parts: Query<&mut Transform, Without<FabricatorGhostAttached>>,
) {
    let Some(f) = sim.as_ref().and_then(|s| s.fabricator.as_ref()) else {
        return;
    };
    let Some(anchor) = anchors
        .iter()
        .flat_map(|n| &n.0)
        .find(|(name, _)| name == "FabricatorSpawnPoint")
        .map(|(_, e)| *e)
    else {
        return;
    };
    let Some(streamer) = streamer else {
        return;
    };
    let t = f.item_spawn;
    let [x, y, z, w] = t.rotation;
    let inverse = Mat4::from_scale_rotation_translation(
        Vec3::from_array(t.scale),
        Quat::from_xyzw(-x, -y, z, w),
        Vec3::new(t.position[0], t.position[1], -t.position[2]),
    )
    .inverse();
    for &entity in streamer.fabricator_preview() {
        if let Ok(mut transform) = parts.get_mut(entity) {
            let world = Mat4::from_scale_rotation_translation(
                transform.scale,
                transform.rotation,
                transform.translation,
            );
            *transform = Transform::from_matrix(inverse * world);
            commands
                .entity(entity)
                .insert((ChildOf(anchor), FabricatorGhostAttached));
            info!("fabricator: preview entity attached to animated itemSpawnPoint");
        }
    }
}

/// The server-made objects to draw, and the fabricator's ghost.
fn placed(view: &ItemsView) -> Vec<(u32, String, Placement)> {
    view.objects
        .iter()
        .map(|(id, (class, point, rot))| {
            let at = Placement {
                position: *point,
                rotation: rotation(*rot),
                scale: [1.0; 3],
            };
            (id.0, class.clone(), at)
        })
        .chain(
            view.preview
                .iter()
                .map(|(_, class, at)| (GHOST_ID, class.clone(), *at)),
        )
        .collect()
}

/// Fabricator.OnOpenedChanged / OnStateChanged; unrelated rigs simply have
/// neither parameter. The installed controller supplies the motion.
pub fn fabricator_animation(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    sim: Option<Res<PlayerSim>>,
    mut view: ResMut<ItemsView>,
    session: Res<Session>,
    mut rigs: Query<&mut crate::animation::AnimatedRig>,
) {
    let tick = session.server_tick();
    let job = session
        .with_items(|items, _| items.fabricator.job.clone())
        .flatten();
    let crafting = job.as_ref().is_some_and(|j| !j.done);
    let mut bound = 0;
    for mut rig in &mut rigs {
        bound += usize::from(
            rig.animator
                .set_bool(sn_unity::name_hash("open_fabricator"), view.fabricator_open),
        );
        rig.animator
            .set_bool(sn_unity::name_hash("fabricating"), crafting);
    }
    if bound > 0 && view.animation_line != Some((view.fabricator_open, crafting)) {
        info!(
            "fabricator: {bound} animator bound, open={} fabricating={crafting}",
            view.fabricator_open
        );
        view.animation_line = Some((view.fabricator_open, crafting));
    }
    let fx = job
        .as_ref()
        .is_some_and(|j| !j.done && (tick - f64::from(j.start)) * session.tick_seconds() > 1.0);
    if fx == view.fx_active {
        return;
    }
    view.fx_active = fx;
    for entity in view.fx.drain(..) {
        commands.entity(entity).despawn();
    }
    if !fx {
        info!("fabricator: beams/sparks plain fallback removed");
        return;
    }
    let Some(f) = sim.as_ref().and_then(|s| s.fabricator.as_ref()) else {
        return;
    };
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let beam = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.65, 1.0),
        unlit: true,
        ..default()
    });
    let spark = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.75, 0.3),
        unlit: true,
        ..default()
    });
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], -p[2]);
    for b in f.beams {
        let mut point = f.item_spawn.position;
        point[1] = point[1].min(b.position[1]);
        let Some(end) = sn_sim::craft::beam_end(
            sn_sim::V3::from_f32(b.position),
            sn_sim::V3::from_f32(b.rotate_vector([0.0, 0.0, 1.0])),
            sn_sim::V3::from_f32(point),
            sn_sim::V3::from_f32(f.item_spawn.rotate_vector([0.0, 1.0, 0.0])),
        ) else {
            continue;
        };
        let end = [end.x as f32, end.y as f32, end.z as f32];
        let delta = v(end) - v(b.position);
        let transform = Transform {
            translation: (v(end) + v(b.position)) * 0.5,
            rotation: Quat::from_rotation_arc(Vec3::Y, delta.normalize_or_zero()),
            scale: Vec3::new(0.006, delta.length(), 0.006),
        };
        view.fx.push(
            commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(beam.clone()),
                    transform,
                ))
                .id(),
        );
        for i in 0..6 {
            let angle = i as f32 * std::f32::consts::TAU / 6.0;
            view.fx.push(
                commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(spark.clone()),
                        Transform::from_translation(
                            v(end) + Vec3::new(angle.cos() * 0.025, 0.01, angle.sin() * 0.025),
                        )
                        .with_scale(Vec3::splat(0.008)),
                    ))
                    .id(),
            );
        }
        info!(
            "fabricator: beam {:?} -> {end:?}, six sparks; plain geometry fallback (particle simulation and fabrication light not ported)",
            b.position
        );
    }
}
