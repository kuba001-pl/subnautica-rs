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
//! Oxygen, health, suffocation and respawn (`sn_sim::vitals`, M9c) run
//! with the player at the same step and fill the HUD (`crate::hud`).
//!
//! The body (M9g4): the game's empty-hand rules (`sn_sim::body`) run every
//! frame; they give the player's animator its parameters and place the
//! view model and the camera (`crate::body` applies them). The camera is
//! at the game's eye (`CameraPose::eye`), or behind it with
//! `--third-person` (a debug orbit camera that shows the head).
//!
//! The hatches (M9g5e) play their cinematics as `sn-inspect walk` does
//! (M9g5d): the pod's and the player's animators run here as well (the
//! drawn rigs get the same parameters through `crate::body`), a hatch
//! moves the player along the pod's animated `cin_target` until the clip's
//! end event, and the camera goes to `Player.camAnchor` meanwhile. After a
//! death the camera follows the head camera bone until the respawn.
//!
//! Controls: click to capture the mouse (Esc releases it), mouse to look
//! (the game's `MainCameraControl` rule, sensitivity and pitch limits,
//! `sn_sim::look`), WASD to move, Space up / jump, C down, E or left click
//! to use (the lifepod's hatches). The key layout is our own.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Instant;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use sn_anim::Animator;
use sn_assets::{
    BatchBodies, CollisionLoader, HATCH_BODY, LayerRules, PlayerAnimation, PodCinematics,
    PosedNode, SCENE_BODY,
};
use sn_install::GameData;
use sn_sim::body::{AnimValue, Body as PlayerBody, BodyFrame, BodyParams};
use sn_sim::cinematic::{CinematicFrame, HatchRun, Signal, apply, ease_tilt};
use sn_sim::collide::{Body, MOVE, World};
use sn_sim::look::{Look, LookParams};
use sn_sim::player::{Event, HatchTrigger, Hatches, Input, Player, PlayerParams, hand_target};
use sn_sim::vitals::{Situation, Vitals, VitalsEvent, VitalsParams, depth_of};
use sn_sim::{Pose, Q, V3};

use crate::body::BodyDrive;
use crate::hud::Hud;

/// The worker gets the player's position again after it moved this far.
const RESEND_DISTANCE: f64 = 4.0;

/// Physics steps run at most per frame (a long frame drops the rest).
const MAX_STEPS_PER_FRAME: usize = 10;

/// `--third-person`: the camera this far behind the eye (m), our own.
const THIRD_PERSON_DISTANCE: f32 = 2.5;

/// A lifepod hatch trigger as the main thread needs it.
struct TriggerInfo {
    name: String,
    hand_text: String,
    /// Its object's world position (where to look to use it).
    at: V3,
    bodies: Vec<Body>,
}

/// What the worker found at start.
struct Init {
    params: PlayerParams,
    vitals: VitalsParams,
    look: LookParams,
    spawn: V3,
    spawn_yaw: f64,
    in_pod: bool,
    pod: Option<V3>,
    pod_bodies: Vec<Body>,
    triggers: Vec<TriggerInfo>,
    hatches: Hatches,
    /// The body's numbers (`None`: no body; the camera at the player's
    /// transform, as before M9g4).
    body: Option<BodyParams>,
    /// The player's animator and its camera nodes (M9g5e).
    anim: Option<PlayerAnimation>,
    /// The pod's animator for the hatch cinematics (M9g5e; `None`: the
    /// hatches teleport, as M9b).
    pod_cinematics: Option<PodCinematics>,
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
    look_params: LookParams,
    look: Look,
    vitals_params: VitalsParams,
    vitals: Vitals,
    /// Where a respawn puts the player, and whether that is in the pod.
    respawn: (V3, bool),
    triggers: Vec<TriggerInfo>,
    hatches: Hatches,
    body: Option<(BodyParams, PlayerBody)>,
    /// The player's animator, run here too (the camera's nodes).
    anim: Option<(PlayerAnimation, Animator)>,
    pod_anim: Option<(PodCinematics, Animator)>,
    /// The hatch cinematic playing.
    hatch: Option<HatchPlay>,
}

/// A hatch cinematic playing.
struct HatchPlay {
    run: HatchRun,
    started: f64,
    /// `camRoot` where the cinematic put it in the last step.
    camera_root: Option<Pose>,
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
    third_person: bool,
    look_down: f64,
    /// `--use-hatch` / `--kill`: seconds of play (taken when done).
    use_hatch_at: Vec<f64>,
    hatch_name: Option<String>,
    hold_forward_from: Option<f64>,
    kill_at: Option<f64>,
    /// After a cinematic ends: trace every physics step until this time
    /// (only with the debug flags `--use-hatch` / `--hold-forward`).
    trace_until: f64,
    trace_hatches: bool,
    /// `--use-hatch`: the trigger being walked to and the time given up.
    auto_target: Option<(usize, f64)>,
    /// Its walk: the last place it moved from and when; strafing until.
    auto_moved: (V3, f64),
    auto_strafe_until: f64,
    /// Play time of the last death-camera log line.
    death_logged: Option<f64>,
}

/// Lifepod 5 where the player starts (the client's own start choice).
pub struct Start {
    /// The pod's point (`None`: no lifepod; the player starts at
    /// `position`, swimming or walking by the water level).
    pub lifepod: Option<[f32; 3]>,
    pub position: [f32; 3],
    pub slot_seed: Option<u64>,
    /// `--third-person`.
    pub third_person: bool,
    /// `--look-down`: the starting pitch, degrees down.
    pub look_down: f64,
    /// `--use-hatch`: use the nearest hatch at each of these times.
    pub use_hatch: Vec<f64>,
    /// `--hatch-name`: only hatches whose trigger name contains this.
    pub hatch_name: Option<String>,
    /// `--hold-forward`: hold W from this much play on.
    pub hold_forward: Option<f64>,
    /// `--kill`: the player dies after this much play.
    pub kill: Option<f64>,
}

impl PlayerSim {
    /// `Player.escapePod`: the player is in the lifepod (`None` until the
    /// player exists).
    pub fn in_pod(&self) -> Option<bool> {
        self.state.as_ref().map(|s| s.player.in_pod)
    }

    /// Seconds of play (physics steps run).
    pub fn play_seconds(&self) -> f64 {
        self.state
            .as_ref()
            .map_or(0.0, |s| self.steps as f64 * s.params.fixed_dt)
    }

    pub fn start(game: GameData, start: Start) -> PlayerSim {
        let third_person = start.third_person;
        let look_down = start.look_down;
        let kill_at = start.kill;
        let mut use_hatch_at = start.use_hatch.clone();
        use_hatch_at.sort_by(|a, b| b.total_cmp(a));
        let (hatch_name, hold_forward_from) = (start.hatch_name.clone(), start.hold_forward);
        let trace_hatches = !start.use_hatch.is_empty() || start.hold_forward.is_some();
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
            third_person,
            look_down,
            use_hatch_at,
            hatch_name,
            hold_forward_from,
            kill_at,
            trace_until: 0.0,
            trace_hatches,
            auto_target: None,
            auto_moved: (V3::ZERO, 0.0),
            auto_strafe_until: 0.0,
            death_logged: None,
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
    let code = sn_assets::player_code(&sn_assets::read_assembly(game)?)?;
    let vitals = sn_assets::vitals_params(&data, &code);
    let look = sn_assets::look_params(&data, &code);
    let rules = LayerRules::new(&settings, data.player_layer);
    let (body, anim, body_note) = match loader.assets().player_body() {
        Ok(b) => {
            let p = b.body_params(data.ocean_level);
            let eye = p.camera_up_position + p.camera_offset_position;
            let (anim, anim_note) = match loader.assets().player_animation(&b) {
                Ok(a) => (Some(a), String::new()),
                Err(e) => (None, format!("; no hatch cinematics ({e})")),
            };
            let note = format!(
                "body: eye at rest ({:.3}, {:.3}, {:.3}) from the player's transform, smoothing {} / {}{anim_note}",
                eye.x, eye.y, eye.z, p.smooth_speed_under_water, p.smooth_speed_above_water
            );
            (Some(p), anim, note)
        }
        Err(e) => (
            None,
            None,
            format!("body: not read ({e}); the camera stays at the player's transform"),
        ),
    };
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
                        cinematic: t.trigger.cinematic_params(),
                        enters: t.trigger.enters,
                        exits: t.trigger.exits,
                        active: t.active,
                    })
                    .collect(),
                first_use: pod.first_use.clone(),
            };
            Init {
                params: params.clone(),
                vitals: vitals.clone(),
                look,
                spawn: V3::from_f32(pod.spawn.position),
                spawn_yaw: f64::from(forward[0]).atan2(f64::from(forward[2])),
                in_pod: true,
                pod: Some(V3::from_f32(point)),
                pod_bodies: pod.bodies,
                triggers: pod
                    .triggers
                    .into_iter()
                    .map(|t| TriggerInfo {
                        at: V3::from_f32(t.at),
                        name: t.trigger.name,
                        hand_text: t.trigger.trigger.hand_text,
                        bodies: t.bodies,
                    })
                    .collect(),
                hatches,
                body,
                anim: anim.clone(),
                pod_cinematics: pod.cinematics,
                summary: format!("{summary}; {body_note}"),
            }
        }
        None => Init {
            params: params.clone(),
            vitals: vitals.clone(),
            look,
            spawn: V3::from_f32(start.position),
            spawn_yaw: 0.0,
            in_pod: false,
            pod: None,
            pod_bodies: Vec::new(),
            triggers: Vec::new(),
            hatches: Hatches::default(),
            body,
            anim,
            pod_cinematics: None,
            summary: format!("no lifepod; {body_note}"),
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

/// `Player.IsUnderwater` (no subs or bases yet: out of the pod and below
/// the ocean level).
fn underwater(params: &PlayerParams, p: &Player) -> bool {
    !p.in_pod && p.position.y < params.ocean_level
}

/// A Unity placement as Bevy's: z mirrored (the x and y of the rotation
/// change sign).
fn pose_to_bevy(p: &Pose) -> Transform {
    let q = p.rotation;
    Transform {
        translation: unity_to_bevy(p.position),
        rotation: Quat::from_xyzw(-q.x as f32, -q.y as f32, q.z as f32, q.w as f32).normalize(),
        scale: Vec3::ONE,
    }
}

fn player_pose(p: &Player) -> Pose {
    Pose::new(p.position, p.rotation)
}

/// A node of the player's hierarchy in the world for the player's
/// animator's pose: the view model's place, then the node's chain.
fn node_world(body: &PlayerBody, node: &PosedNode, anim: &Animator, player: Pose) -> Pose {
    let t = node.in_prefab(anim.pose());
    body.view_model(player).then(&Pose::new(
        V3::from_f32(t.position),
        Q::from_f32(t.rotation).normalized(),
    ))
}

/// `Player.camAnchor` in the world now (the player's transform without a
/// body).
fn cam_anchor(state: &State) -> Pose {
    let pose = player_pose(&state.player);
    match (&state.body, &state.anim) {
        (Some((_, b)), Some((pa, a))) => node_world(b, &pa.cam_anchor, a, pose),
        _ => pose,
    }
}

/// Puts a cinematic frame on the player and keeps its camera.
fn put(state: &mut State, f: &CinematicFrame, h: &mut HatchPlay) {
    let (lo, hi) = (state.look_params.minimum_y, state.look_params.maximum_y);
    apply(f, &mut state.player, &mut state.look, &state.params, lo, hi);
    h.camera_root = f.camera_root;
}

/// What a cinematic's signals do: the pod's and the player's animator
/// parameters (here and, through `drive`, on the drawn rigs), the
/// trigger's end calls.
fn signals(state: &mut State, world: &mut World, drive: &mut BodyDrive, i: usize, list: &[Signal]) {
    for s in list {
        let Some((pc, pod)) = state.pod_anim.as_mut() else {
            return;
        };
        let Some(names) = pc.names.get(i).cloned() else {
            return;
        };
        match *s {
            Signal::Prepare(on) => {
                if let Some(p) = names.prepare {
                    pod.set_bool(p, on);
                    drive.pod_values.push((p, on));
                }
            }
            Signal::Play(on) => {
                pod.set_bool(names.play, on);
                drive.pod_values.push((names.play, on));
                if let Some(p) = names.player {
                    if let Some((_, a)) = state.anim.as_mut()
                        && !a.set_bool(p, on)
                    {
                        warn!(
                            "player: the player's controller lacks {:?}",
                            names.player_name
                        );
                    }
                    drive.player_values.push((p, on));
                }
            }
            Signal::TriggerEnd => {
                for (k, on) in state.hatches.finish(i, &mut state.player) {
                    show_trigger(world, &state.triggers, k, on);
                    info!(
                        "player: first use: {:?} {}",
                        state.triggers[k].name,
                        if on { "on" } else { "off" }
                    );
                }
            }
            Signal::Ended => {}
        }
    }
}

/// One physics step of the pod's animator and of the hatch cinematic
/// playing (as `sn-inspect walk`): the pod's end events, then the
/// controller's late update.
fn step_cinematic(state: &mut State, world: &mut World, drive: &mut BodyDrive, dt: f64, now: f64) {
    let Some((pc, pod)) = state.pod_anim.as_mut() else {
        return;
    };
    pod.update(dt as f32);
    let animated = pc.animated_pose(pod);
    let ends = pod
        .events
        .iter()
        .filter(|e| e.function == "OnPlayerCinematicModeEnd")
        .count();
    let Some(mut h) = state.hatch.take() else {
        return;
    };
    let forwarded = pc.names.get(h.run.trigger).is_some_and(|n| n.forwarded);
    let mut frames: Vec<CinematicFrame> = Vec::new();
    for _ in 0..ends {
        if forwarded {
            let anchor = cam_anchor(state);
            if let Some(f) = h.run.cinematic.end_event(now, animated, anchor) {
                put(state, &f, &mut h);
                frames.push(f);
            }
        }
    }
    let anchor = cam_anchor(state);
    let f = h
        .run
        .cinematic
        .late_update(now, player_pose(&state.player), animated, anchor);
    put(state, &f, &mut h);
    frames.push(f);
    for f in &frames {
        signals(state, world, drive, h.run.trigger, &f.signals);
    }
    if h.run.cinematic.active {
        state.hatch = Some(h);
    } else {
        let p = state.player.position;
        info!(
            "player: cinematic of {:?} ended after {:.2} s at ({:.2}, {:.2}, {:.2}), in pod {}",
            state.triggers[h.run.trigger].name,
            now - h.started,
            p.x,
            p.y,
            p.z,
            state.player.in_pod
        );
    }
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
    mut hud: ResMut<Hud>,
    mut drive: ResMut<BodyDrive>,
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
                info!(
                    "player: oxygen {} (refill {}/s at the surface), suffocation {} s, recovery {} s, health {} (x {} on respawn); look {:.4} degrees per mouse count (sensitivity {}), pitch {} to {} degrees",
                    init.vitals.oxygen_capacity,
                    init.vitals.refill_per_second,
                    init.vitals.suffocation_time,
                    init.vitals.suffocation_recovery_time,
                    init.vitals.max_health,
                    init.vitals.start_health_percent,
                    init.look.degrees_per_count(),
                    init.look.mouse_sensitivity,
                    init.look.minimum_y,
                    init.look.maximum_y
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
                    look_params: init.look,
                    look: Look {
                        rotation_y: (-sim.look_down)
                            .clamp(init.look.minimum_y, init.look.maximum_y),
                        ..Look::facing(init.spawn_yaw)
                    },
                    vitals: Vitals::new(&init.vitals),
                    vitals_params: init.vitals,
                    respawn: (init.spawn, init.in_pod),
                    triggers: init.triggers,
                    hatches: init.hatches,
                    body: init.body.map(|p| (p, PlayerBody::new(&p))),
                    anim: init.anim.map(|a| {
                        let animator = a.animator();
                        (a, animator)
                    }),
                    pod_anim: init.pod_cinematics.map(|c| {
                        drive.pod_controller = c.program.controller.name.clone();
                        info!(
                            "player: hatch cinematics: pod animator {:?}, {} triggers forwarded of {}",
                            c.program.controller.name,
                            c.names.iter().filter(|n| n.forwarded).count(),
                            c.names.len()
                        );
                        let animator = c.animator();
                        (c, animator)
                    }),
                    hatch: None,
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
    // Dead or respawning: no look, movement or use (`Player.OnKill`); in
    // a cinematic neither (`PlayerController` off).
    let controls = state.vitals.controls_enabled() && !state.player.cinematic;
    if captured && controls {
        // Bevy's mouse delta is y down, Unity's y up.
        let d = motion.delta;
        state
            .look
            .apply(&state.look_params, f64::from(d.x), -f64::from(d.y));
    }
    // `--use-hatch`: aim at the nearest usable hatch, then use it below.
    let play = sim.steps as f64 * state.params.fixed_dt;
    let mut auto_use = false;
    if sim.ready && controls && sim.use_hatch_at.last().is_some_and(|&t| play >= t) {
        sim.use_hatch_at.pop();
        let pose = player_pose(&state.player);
        let eye = state.body.as_ref().map_or(state.player.position, |(p, b)| {
            b.camera(p, pose, None).position
        });
        let nearest = (0..state.triggers.len())
            .filter(|&i| state.hatches.triggers.get(i).is_some_and(|t| t.active))
            .filter(|&i| {
                sim.hatch_name
                    .as_ref()
                    .is_none_or(|n| state.triggers[i].name.contains(n.as_str()))
            })
            .min_by(|&a, &b| {
                (state.triggers[a].at - eye)
                    .length()
                    .total_cmp(&(state.triggers[b].at - eye).length())
            });
        match nearest {
            Some(i) => {
                info!(
                    "player: --use-hatch: going to {:?} ({:.2} m)",
                    state.triggers[i].name,
                    (state.triggers[i].at - eye).length()
                );
                sim.auto_target = Some((i, play + 15.0));
            }
            None => info!("player: --use-hatch: no usable hatch"),
        }
    }
    // Aim at it; use it when the hand points at it, else walk towards it.
    let mut auto_walk = false;
    let mut forced_use: Option<usize> = None;
    if let Some((i, give_up)) = sim.auto_target
        && controls
    {
        let pose = player_pose(&state.player);
        let eye = state.body.as_ref().map_or(state.player.position, |(p, b)| {
            b.camera(p, pose, None).position
        });
        let d = state.triggers[i].at - eye;
        let yaw = d.x.atan2(d.z);
        let pitch = (-d.y).atan2((d.x * d.x + d.z * d.z).sqrt());
        state.look = Look {
            rotation_x: yaw.to_degrees(),
            rotation_y: (-pitch.to_degrees())
                .clamp(state.look_params.minimum_y, state.look_params.maximum_y),
        };
        let target = hand_target(&sim.world, eye, state.look.yaw(), state.look.pitch());
        let hits = target
            .is_some_and(|(_, h)| h.body & HATCH_BODY != 0 && ((h.body >> 8) & 0xff) as usize == i);
        if (play * 50.0).round() as u64 % 50 == 0 {
            info!(
                "player: --use-hatch: at ({:.2}, {:.2}, {:.2}), {:.2} m from {:?}, look ({:.1}, {:.1}), hand on {}",
                state.player.position.x,
                state.player.position.y,
                state.player.position.z,
                d.length(),
                state.triggers[i].name,
                state.look.rotation_x,
                state.look.rotation_y,
                target.map_or("nothing".to_string(), |(t, h)| format!(
                    "{} {:#x} at {t:.2} m",
                    sn_assets::body_kind(h.body),
                    h.body
                ))
            );
        }
        if hits {
            auto_use = true;
            sim.auto_target = None;
        } else if play > give_up - 13.0 && target.is_some_and(|(t, _)| t > 1.0) {
            // 2 s without the hand's ray reaching it and something in the
            // way within reach (the pod's hull above the top entry when
            // standing on it): use it directly (debug only).
            info!(
                "player: --use-hatch: the hand's ray does not reach {:?}; using it directly",
                state.triggers[i].name
            );
            forced_use = Some(i);
            sim.auto_target = None;
        } else if play > give_up {
            info!(
                "player: --use-hatch: gave up on {:?}",
                state.triggers[i].name
            );
            sim.auto_target = None;
        } else {
            auto_walk = true;
            // Blocked for half a second: step aside for half a second (as
            // `sn-inspect walk` does round the ladder).
            if (state.player.position - sim.auto_moved.0).length() > 0.1 {
                sim.auto_moved = (state.player.position, play);
            } else if play - sim.auto_moved.1 > 0.5 && play > sim.auto_strafe_until {
                sim.auto_strafe_until = play + 0.5;
                sim.auto_moved = (state.player.position, play + 0.5);
            }
        }
    }
    let auto_strafe = auto_walk && play < sim.auto_strafe_until;
    let (yaw, pitch) = (state.look.yaw(), state.look.pitch());

    // Use (the lifepod's hatches).
    let wants_use = auto_use
        || keys.just_pressed(KeyCode::KeyE)
        || (captured && buttons.just_pressed(MouseButton::Left));
    if (wants_use || forced_use.is_some()) && sim.ready && controls {
        let pose = player_pose(&state.player);
        let eye = state.body.as_ref().map_or(state.player.position, |(p, b)| {
            b.camera(p, pose, None).position
        });
        let aimed = match forced_use {
            Some(i) => Some((
                0.0,
                sn_sim::collide::Hit {
                    t: 0.0,
                    normal: V3::ZERO,
                    point: state.triggers[i].at,
                    body: HATCH_BODY | ((i as u64) << 8),
                },
            )),
            None => hand_target(&sim.world, eye, yaw, pitch),
        };
        match aimed {
            Some((d, hit)) if hit.body & HATCH_BODY != 0 => {
                let i = ((hit.body >> 8) & 0xff) as usize;
                let (name, text) = (
                    state.triggers[i].name.clone(),
                    state.triggers[i].hand_text.clone(),
                );
                let cinematic = state.pod_anim.is_some() && state.anim.is_some();
                let now = sim.steps as f64 * state.params.fixed_dt;
                if cinematic {
                    let root = state
                        .body
                        .as_ref()
                        .map_or(pose, |(_, b)| b.camera_root(pose));
                    let anchor = cam_anchor(state);
                    match state.hatches.begin(
                        i,
                        now,
                        &mut state.player,
                        &mut state.look,
                        &state.params,
                        root,
                        anchor,
                    ) {
                        Some((run, first)) => {
                            info!("player: used {name:?} ({text:?}) from {d:.2} m: cinematic");
                            let h = HatchPlay {
                                run,
                                started: now,
                                camera_root: None,
                            };
                            signals(state, &mut sim.world, &mut drive, i, &first);
                            state.hatch = Some(h);
                        }
                        None => info!("player: {name:?} cannot be used now"),
                    }
                } else {
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
            }
            Some((d, hit)) => info!(
                "player: nothing to use ({} at {d:.2} m)",
                sn_assets::body_kind(hit.body)
            ),
            None => info!("player: nothing to use within reach"),
        }
    }

    // Movement input.
    let held_w = sim.hold_forward_from.is_some_and(|t| play >= t);
    let key = |k: KeyCode| {
        if keys.pressed(k)
            || ((held_w || auto_walk) && k == KeyCode::KeyW)
            || (auto_strafe && k == KeyCode::KeyD)
        {
            1.0
        } else {
            0.0
        }
    };
    let input = Input {
        move_dir: V3::new(
            key(KeyCode::KeyD) - key(KeyCode::KeyA),
            key(KeyCode::Space) - key(KeyCode::KeyC),
            key(KeyCode::KeyW) - key(KeyCode::KeyS),
        ),
        yaw,
        pitch,
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
            let events = if state.vitals.controls_enabled() {
                state.player.step(&state.params, &sim.world, &input)
            } else {
                Vec::new()
            };
            let now = sim.steps as f64 * dt;
            let mut landed = None;
            for e in events {
                match e {
                    Event::Landed { impact_y } => {
                        landed = Some(impact_y);
                        if let Some((_, b)) = state.body.as_mut() {
                            b.landed(impact_y);
                        }
                    }
                    Event::Jumped => {
                        if let Some((_, b)) = state.body.as_mut() {
                            b.jumped(now);
                        }
                    }
                    other => info!(
                        "player: {other:?} at ({:.2}, {:.2}, {:.2})",
                        state.player.position.x, state.player.position.y, state.player.position.z
                    ),
                }
            }
            let situation = Situation {
                y: state.player.position.y,
                in_pod: state.player.in_pod,
                landed,
                world_settled: sim.ready,
                cinematic: state.player.cinematic,
            };
            let mut vitals_events = Vec::new();
            if sim.kill_at.is_some_and(|t| now >= t) {
                sim.kill_at = None;
                info!("player: --kill");
                let all = state.vitals_params.max_health;
                state.vitals.take_damage(all, &mut vitals_events);
            }
            vitals_events.extend(state.vitals.step(&state.vitals_params, dt, &situation));
            for e in vitals_events {
                if matches!(e, VitalsEvent::Breath(_)) {
                    continue;
                }
                info!(
                    "player: {e:?} (oxygen {:.1}, health {:.1}) at ({:.2}, {:.2}, {:.2})",
                    state.vitals.oxygen,
                    state.vitals.health,
                    state.player.position.x,
                    state.player.position.y,
                    state.player.position.z
                );
                if e == VitalsEvent::Died
                    && let Some((_, b)) = state.body.as_mut()
                {
                    b.died();
                }
                if e == VitalsEvent::MoveToRespawn {
                    // `EscapePod.RespawnPlayer` (or the start point).
                    let (at, in_pod) = state.respawn;
                    state.player.teleport(&state.params, at, Some(in_pod));
                    // `ResetPlayerOnDeath`: `DisableHeadCameraController`.
                    if let Some((_, b)) = state.body.as_mut() {
                        b.respawned();
                    }
                }
            }
            // `Player.FixedUpdate`'s falling clock, `UnderWaterTracker`.
            if let Some((_, b)) = state.body.as_mut() {
                let p = &state.player;
                b.fixed_step(
                    now,
                    underwater(&state.params, p),
                    p.walk_grounded,
                    p.cinematic,
                );
            }
            let was_cinematic = state.player.cinematic;
            step_cinematic(state, &mut sim.world, &mut drive, dt, now);
            if !state.player.cinematic {
                state.player.rotation = ease_tilt(state.player.rotation, dt);
            }
            // After a cinematic: trace the player for 3 s (debugging the
            // hatches).
            if sim.trace_hatches && was_cinematic && !state.player.cinematic {
                sim.trace_until = now + 3.0;
            }
            if now <= sim.trace_until {
                let p = &state.player;
                let e = p.rotation.to_euler();
                let gap = sim
                    .world
                    .clearance(&p.capsule(&state.params), p.position, 1.0)
                    .map_or(f64::NAN, |c| c.gap);
                info!(
                    "player: trace {now:.2} s: gap {gap:.3} m, at ({:.3}, {:.3}, {:.3}), {:?}, swimming {}, grounded {} (walk {}), velocity ({:.2}, {:.2}, {:.2}), rotation euler ({:.1}, {:.1}, {:.1}), look ({:.1}, {:.1}), input ({:.0}, {:.0}, {:.0}), height {:.2}",
                    p.position.x,
                    p.position.y,
                    p.position.z,
                    p.motor,
                    p.swimming,
                    p.grounded,
                    p.walk_grounded,
                    p.velocity.x,
                    p.velocity.y,
                    p.velocity.z,
                    e.x,
                    e.y,
                    e.z,
                    state.look.rotation_x,
                    state.look.rotation_y,
                    input.move_dir.x,
                    input.move_dir.y,
                    input.move_dir.z,
                    p.height
                );
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

    // The body (M9g4): `ArmsController.Update` and
    // `MainCameraControl.OnUpdate` once per frame, as the game's.
    let pose = match state.body.as_mut() {
        Some((params, body)) => {
            let started = Instant::now();
            let pl = &state.player;
            let frame = BodyFrame {
                dt: f64::from(time.delta_secs()),
                time: sim.steps as f64 * state.params.fixed_dt + sim.accumulator,
                position: pl.position,
                velocity: pl.velocity,
                grounded: pl.walk_grounded,
                underwater: underwater(&state.params, pl),
                swimming: pl.swimming,
                inside: pl.in_pod,
                look: state.look,
                strafe: if controls { input.move_dir.x } else { 0.0 },
                controls,
                bobbing: true,
            };
            let world = &sim.world;
            let mut ray = |o: V3, d: V3, l: f64| world.cast(MOVE, o, d, 0.0, l).is_some();
            let values = body.update(params, &frame, &mut ray);
            // The same values for our own copy of the player's animator.
            if let Some((_, a)) = state.anim.as_mut() {
                for &(name, v) in &values {
                    let id = sn_unity::name_hash(name);
                    match v {
                        AnimValue::Float(x) => a.set_float(id, x as f32),
                        AnimValue::Bool(b) => a.set_bool(id, b),
                        AnimValue::Trigger => a.set_trigger(id),
                    };
                }
                a.update(frame.dt as f32);
            }
            drive.values = values;
            drive.rules_micros = started.elapsed().as_secs_f32() * 1e6;
            Some((*params, body.pose))
        }
        None => None,
    };
    drive.shown = pose.is_some();
    let ppose = player_pose(&state.player);

    // The camera: at the game's eye (the main camera hangs on
    // `cameraOffsetTransform`, `AutoParent`), `camRoot` moved by a
    // cinematic or on the head camera bone after a death; without a body
    // at the player's transform.
    let (eye, rotation) = match (&pose, state.body.as_ref()) {
        (Some((params, _)), Some((_, body))) => {
            drive.view_model = pose_to_bevy(&body.view_model(ppose));
            let root = match (&state.anim, body.head_camera) {
                (Some((pa, a)), true) => {
                    let head = node_world(body, &pa.head_camera, a, ppose);
                    // Once a second: where the death camera is.
                    if sim.death_logged.is_none_or(|t| play >= t + 1.0) {
                        sim.death_logged = Some(play);
                        let d = head.position - ppose.position;
                        info!(
                            "player: death camera on the head camera bone, {:.2} m above the player's transform ({:.2} m sideways)",
                            d.y,
                            (d.x * d.x + d.z * d.z).sqrt()
                        );
                    }
                    Some(head)
                }
                _ => state.hatch.as_ref().and_then(|h| h.camera_root),
            };
            let cam = pose_to_bevy(&body.camera(params, ppose, root));
            (cam.translation, cam.rotation)
        }
        _ => (
            unity_to_bevy(p),
            camera_rotation(state.look.yaw(), state.look.pitch()),
        ),
    };
    if let Ok(mut t) = camera.single_mut() {
        t.rotation = rotation;
        t.translation = if sim.third_person {
            // Behind the eye along the view, looking at it.
            eye - rotation * Vec3::NEG_Z * THIRD_PERSON_DISTANCE
        } else {
            eye
        };
    }

    let v = &state.vitals;
    *hud = Hud {
        shown: true,
        oxygen: v.oxygen,
        capacity: state.vitals_params.oxygen_capacity,
        seconds_left: v.seconds_left(),
        health: v.health,
        max_health: state.vitals_params.max_health,
        depth: depth_of(&state.vitals_params, p.y),
        overlay: v.overlay(),
        dead: !v.controls_enabled(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_sim::body::CameraPose;
    use sn_sim::player::look_rotate;

    #[test]
    fn poses_convert_to_bevy() {
        // Any Unity rotation turns vectors as its Bevy conversion does.
        let pose = Pose::new(V3::new(1.0, 2.0, 3.0), Q::euler(20.0, 135.0, 4.0));
        let t = pose_to_bevy(&pose);
        assert!((t.translation - Vec3::new(1.0, 2.0, -3.0)).length() < 1e-5);
        for v in [
            V3::new(0.0, 0.0, 1.0),
            V3::new(1.0, 0.0, 0.0),
            V3::new(0.0, 1.0, 0.0),
        ] {
            let expect = unity_to_bevy(pose.rotation.rotate(v));
            let got = t.rotation * unity_to_bevy(v);
            assert!((got - expect).length() < 1e-5, "{v:?}: {got} vs {expect}");
        }
        // The camera of a level player looks along the rig's pose.
        let cam = CameraPose {
            root_y: -0.1,
            root_pitch: 20.0,
            yaw: 135.0,
            roll: 4.0,
            up_pitch: -10.0,
        };
        let ahead = cam.rotate(V3::new(0.0, 0.0, 1.0));
        let q = Q::euler(cam.root_pitch, cam.yaw, cam.roll) * Q::euler(cam.up_pitch, 0.0, 0.0);
        assert!((q.rotate(V3::new(0.0, 0.0, 1.0)) - ahead).length() < 1e-9);
    }

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
