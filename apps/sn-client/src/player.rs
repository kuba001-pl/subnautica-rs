//! The player (M9b, `docs/DESIGN.md` § 4.3 "M9b plan"): first-person
//! camera, swimming and walking by the game's rules (`sn_sim::player`)
//! against the game's collision (`sn_assets::CollisionLoader`), the
//! lifepod's hatches. `--free-cam` keeps the fly camera instead.
//!
//! A worker thread loads the collision bodies of the batches within the
//! game's collision range of the player and sends them over; the main
//! thread keeps the `sn-sim` world, reads input and runs the player at the
//! game's physics step.
//!
//! Controls: click to capture the mouse (Esc releases it), mouse to look,
//! WASD to move, Space up / jump, C down, E or left click to use (the
//! lifepod's hatches). Our own key layout and mouse sensitivity, not read
//! from the game.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Instant;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use sn_assets::{BatchBodies, CollisionLoader, HATCH_BODY, LayerRules, SCENE_BODY};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::collide::{Body, World};
use sn_sim::player::{Event, HatchTrigger, Hatches, Input, Player, PlayerParams, hand_target};

/// Radians per pixel of mouse movement (ours).
const SENSITIVITY: f64 = 0.0025;

/// Look limit up and down (ours, until `MainCameraControl` is read).
const MAX_PITCH: f64 = 89.0 * std::f64::consts::PI / 180.0;

/// The worker gets the player's position again after it moved this far.
const RESEND_DISTANCE: f64 = 4.0;

/// Physics steps run at most per frame (a long frame drops the rest).
const MAX_STEPS_PER_FRAME: usize = 10;

/// A lifepod hatch trigger as the main thread needs it.
struct TriggerInfo {
    name: String,
    hand_text: String,
    bodies: Vec<Body>,
}

/// What the worker found at start.
struct Init {
    params: PlayerParams,
    spawn: V3,
    spawn_yaw: f64,
    in_pod: bool,
    pod: Option<V3>,
    pod_bodies: Vec<Body>,
    triggers: Vec<TriggerInfo>,
    hatches: Hatches,
    summary: String,
}

enum FromWorker {
    Init(Box<Init>),
    Bodies {
        add: Vec<(u64, Body)>,
        remove: Vec<u64>,
        load_ms: Vec<f64>,
        batches: usize,
    },
    Error(String),
}

/// The running player and its world.
struct State {
    params: PlayerParams,
    player: Player,
    yaw: f64,
    pitch: f64,
    triggers: Vec<TriggerInfo>,
    hatches: Hatches,
}

#[derive(Resource)]
pub struct PlayerSim {
    to_worker: Sender<[f32; 3]>,
    from_worker: Mutex<Receiver<FromWorker>>,
    world: World,
    state: Option<State>,
    /// The bodies around the player arrived; the player moves from then.
    ready: bool,
    started: Instant,
    accumulator: f64,
    last_sent: Option<V3>,
    /// Physics steps run, for the status log.
    steps: u64,
}

/// Lifepod 5 where the player starts (the client's own start choice).
pub struct Start {
    /// The pod's point (`None`: no lifepod; the player starts at
    /// `position`, swimming or walking by the water level).
    pub lifepod: Option<[f32; 3]>,
    pub position: [f32; 3],
    pub slot_seed: Option<u64>,
}

impl PlayerSim {
    pub fn start(game: GameData, start: Start) -> PlayerSim {
        let (to_worker, rx) = channel();
        let (tx, from_worker) = channel();
        std::thread::Builder::new()
            .name("collision".into())
            .spawn(move || {
                if let Err(e) = worker(&game, start, &rx, &tx) {
                    let _ = tx.send(FromWorker::Error(e));
                }
            })
            .expect("spawn the collision thread");
        PlayerSim {
            to_worker,
            from_worker: Mutex::new(from_worker),
            world: World::new(),
            state: None,
            ready: false,
            started: Instant::now(),
            accumulator: 0.0,
            last_sent: None,
            steps: 0,
        }
    }
}

fn worker(
    game: &GameData,
    start: Start,
    rx: &Receiver<[f32; 3]>,
    tx: &Sender<FromWorker>,
) -> Result<(), String> {
    let mut loader = CollisionLoader::new(game, start.slot_seed)?;
    let data = sn_assets::player_data(loader.assets())?;
    let settings = sn_assets::physics_settings(loader.assets())?;
    let params = sn_assets::player_params(&data, &settings);
    let rules = LayerRules::new(&settings, data.player_layer);
    let init = match start.lifepod {
        Some(point) => {
            let pod = loader.lifepod(&rules, point)?;
            let forward = pod.spawn.rotate_vector([0.0, 0.0, 1.0]);
            let summary = format!(
                "lifepod at {point:?}: {} colliders, {} hatch triggers ({} active); player spawn {:?}",
                pod.colliders,
                pod.triggers.len(),
                pod.triggers.iter().filter(|t| t.active).count(),
                pod.spawn.position
            );
            let hatches = Hatches {
                triggers: pod
                    .triggers
                    .iter()
                    .map(|t| HatchTrigger {
                        end: t.trigger.end.map(|e| V3::from_f32(e.position)),
                        enters: t.trigger.enters,
                        exits: t.trigger.exits,
                        active: t.active,
                    })
                    .collect(),
                first_use: pod.first_use.clone(),
            };
            Init {
                params: params.clone(),
                spawn: V3::from_f32(pod.spawn.position),
                spawn_yaw: f64::from(forward[0]).atan2(f64::from(forward[2])),
                in_pod: true,
                pod: Some(V3::from_f32(point)),
                pod_bodies: pod.bodies,
                triggers: pod
                    .triggers
                    .into_iter()
                    .map(|t| TriggerInfo {
                        name: t.trigger.name,
                        hand_text: t.trigger.trigger.hand_text,
                        bodies: t.bodies,
                    })
                    .collect(),
                hatches,
                summary,
            }
        }
        None => Init {
            params: params.clone(),
            spawn: V3::from_f32(start.position),
            spawn_yaw: 0.0,
            in_pod: false,
            pod: None,
            pod_bodies: Vec::new(),
            triggers: Vec::new(),
            hatches: Hatches::default(),
            summary: "no lifepod".into(),
        },
    };
    let mut pos = init.spawn.to_f32();
    tx.send(FromWorker::Init(Box::new(init)))
        .map_err(|_| "client gone")?;
    let mut batches = BatchBodies::new();
    loop {
        let c = batches.update(&mut loader, &rules, pos)?;
        if !c.add.is_empty() || !c.remove.is_empty() {
            tx.send(FromWorker::Bodies {
                add: c.add,
                remove: c.remove,
                load_ms: c.load_ms,
                batches: batches.loaded(),
            })
            .map_err(|_| "client gone")?;
        }
        // Wait for the next position; keep only the latest.
        pos = rx.recv().map_err(|_| "client gone")?;
        loop {
            match rx.try_recv() {
                Ok(p) => pos = p,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
}

fn unity_to_bevy(p: V3) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, -p.z as f32)
}

/// Unity's yaw (about +y) and pitch (positive looks down) as a Bevy
/// rotation (right-handed, the camera looks along −z).
fn camera_rotation(yaw: f64, pitch: f64) -> Quat {
    Quat::from_rotation_y(-yaw as f32) * Quat::from_rotation_x(-pitch as f32)
}

fn trigger_body(i: usize, j: usize) -> u64 {
    HATCH_BODY | ((i as u64) << 8) | j as u64
}

fn show_trigger(world: &mut World, triggers: &[TriggerInfo], i: usize, on: bool) {
    for (j, b) in triggers[i].bodies.iter().enumerate() {
        if on {
            world.insert(trigger_body(i, j), b.clone());
        } else {
            world.remove(trigger_body(i, j));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut windows: Query<(&Window, &mut CursorOptions)>,
    mut sim: ResMut<PlayerSim>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    let sim = &mut *sim;
    // Messages from the worker.
    let messages: Vec<FromWorker> = match sim.from_worker.lock() {
        Ok(rx) => rx.try_iter().collect(),
        Err(_) => Vec::new(),
    };
    for message in messages {
        match message {
            FromWorker::Init(init) => {
                info!("player: {}", init.summary);
                info!(
                    "player: swim {:.2} m/s (drag {}, after it {:.2}), walk {} m/s, step offset {:.2} m, slope limit {}°, ocean level {} m, physics step {:.3} s",
                    init.params.swim_forward,
                    init.params.swim_drag,
                    init.params.swim_forward * (1.0 - init.params.swim_drag * init.params.fixed_dt),
                    init.params.walk_speed,
                    init.params.step_offset,
                    init.params.slope_limit_degrees,
                    init.params.ocean_level,
                    init.params.fixed_dt
                );
                for (i, b) in init.pod_bodies.into_iter().enumerate() {
                    sim.world.insert(SCENE_BODY | i as u64, b);
                }
                for (i, t) in init.hatches.triggers.iter().enumerate() {
                    if t.active {
                        show_trigger(&mut sim.world, &init.triggers, i, true);
                    }
                }
                let mut player = Player::new(&init.params, init.spawn, init.in_pod);
                player.pod_position = init.pod;
                sim.state = Some(State {
                    params: init.params,
                    player,
                    yaw: init.spawn_yaw,
                    pitch: 0.0,
                    triggers: init.triggers,
                    hatches: init.hatches,
                });
            }
            FromWorker::Bodies {
                add,
                remove,
                load_ms,
                batches,
            } => {
                for id in remove {
                    sim.world.remove(id);
                }
                let n = add.len();
                for (id, b) in add {
                    sim.world.insert(id, b);
                }
                if !load_ms.is_empty() {
                    info!(
                        "player: collision for {} more batches ({n} bodies) in {:.0} ms; {batches} batches around the player, {} bodies",
                        load_ms.len(),
                        load_ms.iter().sum::<f64>(),
                        sim.world.len()
                    );
                }
                if !sim.ready {
                    sim.ready = true;
                    info!(
                        "player: ready {:.1} s after start",
                        sim.started.elapsed().as_secs_f32()
                    );
                }
            }
            FromWorker::Error(e) => error!("player: collision: {e}"),
        }
    }
    let Some(state) = sim.state.as_mut() else {
        return;
    };

    // Mouse capture and look.
    let mut captured = false;
    for (window, mut cursor) in &mut windows {
        if buttons.just_pressed(MouseButton::Left) && window.focused {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        if keys.just_pressed(KeyCode::Escape) {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        captured |= cursor.grab_mode != CursorGrabMode::None;
    }
    if captured {
        let d = motion.delta;
        state.yaw += f64::from(d.x) * SENSITIVITY;
        state.pitch = (state.pitch + f64::from(d.y) * SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }

    // Use (the lifepod's hatches).
    let wants_use =
        keys.just_pressed(KeyCode::KeyE) || (captured && buttons.just_pressed(MouseButton::Left));
    if wants_use && sim.ready {
        let eye = state.player.position;
        match hand_target(&sim.world, eye, state.yaw, state.pitch) {
            Some((d, hit)) if hit.body & HATCH_BODY != 0 => {
                let i = ((hit.body >> 8) & 0xff) as usize;
                let (name, text) = (
                    state.triggers[i].name.clone(),
                    state.triggers[i].hand_text.clone(),
                );
                match state
                    .hatches
                    .use_trigger(i, &mut state.player, &state.params)
                {
                    Some(switched) => {
                        info!(
                            "player: used {name:?} ({text:?}) from {d:.2} m → ({:.2}, {:.2}, {:.2}), in pod {}",
                            state.player.position.x,
                            state.player.position.y,
                            state.player.position.z,
                            state.player.in_pod
                        );
                        for (k, on) in switched {
                            show_trigger(&mut sim.world, &state.triggers, k, on);
                        }
                    }
                    None => info!("player: {name:?} cannot be used (no end point)"),
                }
            }
            Some((d, hit)) => info!(
                "player: nothing to use ({} at {d:.2} m)",
                sn_assets::body_kind(hit.body)
            ),
            None => info!("player: nothing to use within reach"),
        }
    }

    // Movement input.
    let key = |k: KeyCode| if keys.pressed(k) { 1.0 } else { 0.0 };
    let input = Input {
        move_dir: V3::new(
            key(KeyCode::KeyD) - key(KeyCode::KeyA),
            key(KeyCode::Space) - key(KeyCode::KeyC),
            key(KeyCode::KeyW) - key(KeyCode::KeyS),
        ),
        yaw: state.yaw,
        pitch: state.pitch,
        jump: keys.pressed(KeyCode::Space),
    };

    // Fixed physics steps, once the bodies around the player are in.
    if sim.ready {
        let dt = state.params.fixed_dt;
        sim.accumulator += f64::from(time.delta_secs());
        let mut steps = 0;
        while sim.accumulator >= dt && steps < MAX_STEPS_PER_FRAME {
            sim.accumulator -= dt;
            steps += 1;
            sim.steps += 1;
            // A status line every 10 s of play.
            if sim.steps % ((10.0 / dt) as u64).max(1) == 0 {
                let p = &state.player;
                info!(
                    "player: {:.0} s: at ({:.2}, {:.2}, {:.2}), {:?}, in pod {}, grounded {}, speed {:.2} m/s",
                    sim.steps as f64 * dt,
                    p.position.x,
                    p.position.y,
                    p.position.z,
                    p.motor,
                    p.in_pod,
                    p.grounded,
                    p.velocity.length()
                );
            }
            for e in state.player.step(&state.params, &sim.world, &input) {
                match e {
                    Event::Landed | Event::Jumped => {}
                    other => info!(
                        "player: {other:?} at ({:.2}, {:.2}, {:.2})",
                        state.player.position.x, state.player.position.y, state.player.position.z
                    ),
                }
            }
        }
        if steps == MAX_STEPS_PER_FRAME {
            sim.accumulator = 0.0;
        }
    }

    // Tell the worker where the player is.
    let p = state.player.position;
    if sim
        .last_sent
        .is_none_or(|s| (s - p).length() > RESEND_DISTANCE)
    {
        sim.last_sent = Some(p);
        let _ = sim.to_worker.send(p.to_f32());
    }

    // The camera sits at the player's transform.
    if let Ok(mut t) = camera.single_mut() {
        t.translation = unity_to_bevy(p);
        t.rotation = camera_rotation(state.yaw, state.pitch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_sim::player::look_rotate;

    #[test]
    fn camera_looks_where_the_rules_move() {
        // The rules' forward (Unity) and the camera's forward (Bevy) agree.
        for (yaw, pitch) in [(0.0, 0.0), (1.0, 0.3), (-2.5, -0.7)] {
            let unity = look_rotate(yaw, pitch, V3::new(0.0, 0.0, 1.0));
            let bevy = camera_rotation(yaw, pitch) * Vec3::NEG_Z;
            let expect = unity_to_bevy(unity);
            assert!(
                (bevy - expect).length() < 1e-5,
                "{yaw} {pitch}: {bevy} vs {expect}"
            );
        }
    }
}
