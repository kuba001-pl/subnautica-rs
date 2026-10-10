//! `walk` (M9b): a scripted run with the player's own rules
//! (`sn_sim::player`), headless: spawn in the lifepod, walk to its exit,
//! use it, swim away for 10 s and back, board. Logs positions, motor
//! changes, the hatch triggers used and speeds next to the values read
//! from the game. See `docs/DESIGN.md` § 4.3 "M9b plan". The pieces
//! ([`start`], [`leave_pod`], [`Run`]) are shared with `dive` (M9c). Both
//! also run the player's body and animator (M9g3, [`BodyRun`]); the hand's
//! ray starts at the game's eye (M9g2), not at the player's transform.

use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Lifepod, SCENE_BODY};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::player::{
    Event, HatchTrigger, Hatches, Input, Motor, Player, PlayerParams, hand_target,
};
use sn_sim::vitals::{Situation, Vitals, VitalsEvent, VitalsParams};

use crate::Result;
use crate::body_run::BodyRun;
use crate::collision::{HATCH, Streamed, kind};
use crate::swim::{PENETRATION, percentile};

/// Yaw and pitch (Unity's, radians) to look from `eye` at `at`.
fn look_at(eye: V3, at: V3) -> (f64, f64) {
    let d = at - eye;
    let yaw = d.x.atan2(d.z);
    let flat = (d.x * d.x + d.z * d.z).sqrt();
    let pitch = (-d.y).atan2(flat);
    (yaw, pitch)
}

pub(crate) struct Run<'a, 'g> {
    pub(crate) s: &'a mut Streamed<'g>,
    pub(crate) params: PlayerParams,
    pub(crate) player: Player,
    pub(crate) pod: Lifepod,
    pub(crate) pod_point: V3,
    hatches: Hatches,
    /// Oxygen and health (`dive`, M9c): stepped with the player; while
    /// the controls are off (dead) the player does not move.
    pub(crate) vitals: Option<(VitalsParams, Vitals)>,
    /// The vitals' events with the time they happened.
    pub(crate) vitals_events: Vec<(f64, VitalsEvent)>,
    /// The player's body and animator (M9g3).
    pub(crate) body: BodyRun,
    pub(crate) steps: usize,
    penetrations: usize,
    /// Surfaces the capsule's centre passed through in a step (the hatch's
    /// moves excepted).
    crossings: usize,
    min_gap: f64,
    step_us: Vec<f64>,
    walk_speeds: Vec<f64>,
    swim_speeds: Vec<f64>,
}

impl Run<'_, '_> {
    pub(crate) fn t(&self) -> f64 {
        self.steps as f64 * self.params.fixed_dt
    }

    /// The capsule's centre in the world.
    fn centre(&self) -> V3 {
        let c = self.player.capsule(&self.params);
        self.player.position + (c.a + c.b) * 0.5
    }

    pub(crate) fn log(&self, what: &str) {
        let p = &self.player;
        println!(
            "  t {:>5.2} s: {what}; at ({:.2}, {:.2}, {:.2}), {:?}, in pod {}, grounded {}, speed {:.2} m/s",
            self.t(),
            p.position.x,
            p.position.y,
            p.position.z,
            p.motor,
            p.in_pod,
            p.grounded,
            p.velocity.length()
        );
    }

    /// Puts trigger `i`'s bodies in the world or takes them out.
    fn show_trigger(&mut self, i: usize, on: bool) {
        for (j, b) in self.pod.triggers[i].bodies.iter().enumerate() {
            let id = HATCH | ((i as u64) << 8) | j as u64;
            if on {
                self.s.world.insert(id, b.clone());
            } else {
                self.s.world.remove(id);
            }
        }
    }

    fn at(&self, i: usize) -> V3 {
        V3::from_f32(self.pod.triggers[i].at)
    }

    pub(crate) fn step(&mut self, input: &Input) -> Result<()> {
        if self.steps % 25 == 0 {
            self.s.stream(self.player.position)?;
        }
        let before = self.centre();
        let moves = self
            .vitals
            .as_ref()
            .is_none_or(|(_, v)| v.controls_enabled());
        let t = Instant::now();
        // Dead: `playerController.SetEnabled(false)`, the body stays.
        let events = if moves {
            self.player.step(&self.params, &self.s.world, input)
        } else {
            Vec::new()
        };
        self.step_us.push(t.elapsed().as_secs_f64() * 1e6);
        self.steps += 1;
        let mut landed = None;
        for e in events {
            match e {
                Event::MotorChanged(m) => self.log(&format!("motor → {m:?}")),
                Event::Landed { impact_y } => {
                    landed = Some(impact_y);
                    self.body.landed(impact_y);
                }
                Event::Jumped => self.body.jumped(self.t()),
                other => self.log(&format!("{other:?}")),
            }
        }
        let respawned = self.step_vitals(landed)?;
        self.step_body(input);
        if respawned {
            return Ok(());
        }
        let after = self.centre();
        // Longer than any step's move: the hatch put the player elsewhere.
        if (after - before).length() < 1.0 {
            let n = self.s.world.crossings(before, after);
            if n > 0 && self.crossings < 5 {
                self.log(&format!("PASSED THROUGH {n} surface(s)"));
            }
            self.crossings += n;
        }
        let capsule = self.player.capsule(&self.params);
        if let Some(c) = self.s.world.clearance(&capsule, self.player.position, 0.5) {
            self.min_gap = self.min_gap.min(c.gap);
            if c.gap < PENETRATION {
                self.penetrations += 1;
                if self.penetrations <= 5 {
                    self.log(&format!("PENETRATION gap {:.4} ({})", c.gap, kind(c.body)));
                }
            }
        }
        let v = self.player.velocity;
        match self.player.motor {
            Motor::Walk if self.player.grounded => {
                self.walk_speeds.push(V3::new(v.x, 0.0, v.z).length());
            }
            Motor::Swim => self.swim_speeds.push(v.length()),
            Motor::Walk => {}
        }
        Ok(())
    }

    /// One step of the body and its animator (M9g3), after the player
    /// and the vitals.
    fn step_body(&mut self, input: &Input) {
        let p = &self.player;
        let controls = self
            .vitals
            .as_ref()
            .is_none_or(|(_, v)| v.controls_enabled());
        let frame = sn_sim::body::BodyFrame {
            dt: self.params.fixed_dt,
            time: self.t(),
            position: p.position,
            velocity: p.velocity,
            grounded: p.walk_grounded,
            // `Player.IsUnderwater`.
            underwater: !p.in_pod && p.position.y < self.params.ocean_level,
            swimming: p.swimming,
            inside: p.in_pod,
            look: self.body.look(input.yaw, input.pitch),
            strafe: if controls { input.move_dir.x } else { 0.0 },
            controls,
            bobbing: true,
        };
        let world = &self.s.world;
        let mut ray =
            |o: V3, d: V3, l: f64| world.cast(sn_sim::collide::MOVE, o, d, 0.0, l).is_some();
        self.body.step(&frame, &mut ray);
    }

    /// One step of the vitals, if they run: logs their events (not the
    /// breaths) and moves the player to the respawn point when they say
    /// so. True if it did.
    fn step_vitals(&mut self, landed: Option<f64>) -> Result<bool> {
        let Some((vp, v)) = self.vitals.as_mut() else {
            return Ok(false);
        };
        let situation = Situation {
            y: self.player.position.y,
            in_pod: self.player.in_pod,
            landed,
            world_settled: true,
        };
        let events = v.step(vp, self.params.fixed_dt, &situation);
        let (time, oxygen, health) = (v.time, v.oxygen, v.health);
        let mut respawn = false;
        for e in events {
            self.vitals_events.push((time, e));
            respawn |= e == VitalsEvent::MoveToRespawn;
            if e == VitalsEvent::Died {
                self.body.died();
            }
            if !matches!(e, VitalsEvent::Breath(_)) {
                self.log(&format!("{e:?} (oxygen {oxygen:.2}, health {health:.1})"));
            }
        }
        if respawn {
            // `EscapePod.RespawnPlayer`: the pod's player spawn, inside.
            let spawn = V3::from_f32(self.pod.spawn.position);
            self.player.teleport(&self.params, spawn, Some(true));
            self.s.stream(self.player.position)?;
            self.log("moved to the respawn point");
        }
        Ok(respawn)
    }

    /// Looks at trigger `i` and uses it if the hand points at it.
    fn try_use(&mut self, i: usize) -> Option<f64> {
        if self
            .vitals
            .as_ref()
            .is_some_and(|(_, v)| !v.controls_enabled())
        {
            return None;
        }
        let eye = self.body.eye(self.player.position);
        let (yaw, pitch) = look_at(eye, self.at(i));
        let (d, hit) = hand_target(&self.s.world, eye, yaw, pitch)?;
        if hit.body & HATCH == 0 || ((hit.body >> 8) & 0xff) as usize != i {
            return None;
        }
        let t = &self.pod.triggers[i].trigger;
        let what = format!(
            "used {:?} ({:?}, animation {:?}) from {d:.2} m",
            t.name, t.trigger.hand_text, t.animation
        );
        let switched = self
            .hatches
            .use_trigger(i, &mut self.player, &self.params)?;
        self.log(&what);
        for (k, on) in switched {
            self.show_trigger(k, on);
            self.log(&format!(
                "first use: {:?} {}",
                self.pod.triggers[k].trigger.name,
                if on { "on" } else { "off" }
            ));
        }
        Some(d)
    }

    /// Walks or swims towards `target` for at most `seconds`; stops when
    /// trigger `use_trigger` was used (true) or when within `near` metres.
    pub(crate) fn go_to(
        &mut self,
        target: V3,
        seconds: f64,
        use_trigger: Option<usize>,
        near: f64,
    ) -> Result<bool> {
        let end = self.steps + (seconds / self.params.fixed_dt) as usize;
        // Blocked for half a second: step aside for half a second (as a
        // player walks round the ladder), switching sides each time.
        let (mut blocked, mut aside, mut side) = (0usize, 0usize, 1.0);
        let half = (0.5 / self.params.fixed_dt) as usize;
        while self.steps < end {
            if let Some(i) = use_trigger
                && self.try_use(i).is_some()
            {
                return Ok(true);
            }
            if (target - self.player.position).length() < near {
                return Ok(false);
            }
            let (mut yaw, pitch) = look_at(self.player.position, target);
            if aside > 0 {
                aside -= 1;
                yaw += side * std::f64::consts::FRAC_PI_2;
            }
            let input = Input {
                move_dir: V3::new(0.0, 0.0, 1.0),
                yaw,
                pitch,
                jump: false,
            };
            let before = self.player.position;
            self.step(&input)?;
            let moved = (self.player.position - before).length();
            if moved < 0.3 * self.params.walk_speed * self.params.fixed_dt {
                blocked += 1;
                if blocked >= half && aside == 0 {
                    blocked = 0;
                    aside = half;
                    side = -side;
                    self.log("blocked: stepping aside");
                }
            } else {
                blocked = 0;
            }
        }
        Ok(false)
    }
}

/// The lifepod for world seed `seed` as in a new game (no hatch used yet)
/// and the player standing at its spawn.
pub(crate) fn start<'a, 'g>(s: &'a mut Streamed<'g>, seed: u64) -> Result<Run<'a, 'g>> {
    let assets = s.loader.assets();
    let data = sn_assets::player_data(assets)?;
    let settings = sn_assets::physics_settings(assets)?;
    let params = sn_assets::player_params(&data, &settings);
    println!(
        "values read: swim {:.2} m/s forward (drag {}, acceleration {}), walk {} m/s, gravity {}, step offset {:.2}, slope limit {}°, ocean level {}, physics step {:.3} s",
        params.swim_forward,
        params.swim_drag,
        params.water_acceleration,
        params.walk_speed,
        params.gravity,
        params.step_offset,
        params.slope_limit_degrees,
        params.ocean_level,
        params.fixed_dt
    );

    let body = BodyRun::new(assets, data.ocean_level)?;
    let (point, _) = assets.start_map()?.random_start(seed);
    let rules = s.rules.clone();
    let pod = s.loader.lifepod(&rules, point)?;
    for (i, b) in pod.bodies.iter().enumerate() {
        s.world.insert(SCENE_BODY | i as u64, b.clone());
    }
    println!(
        "seed {seed}: lifepod at {point:?}, player spawn {:?}; {} colliders in the pod and its modules",
        pod.spawn.position, pod.colliders
    );
    for t in &pod.triggers {
        let tr = &t.trigger;
        println!(
            "  trigger {:<24} {:<16} active {:<5} at ({:.2}, {:.2}, {:.2}), end {}, {}{}; {} bodies",
            format!("{:?}", tr.name),
            format!("{:?}", tr.trigger.hand_text),
            t.active,
            t.at[0],
            t.at[1],
            t.at[2],
            tr.end.map_or("none".into(), |e| format!(
                "({:.2}, {:.2}, {:.2}){}",
                e.position[0],
                e.position[1],
                e.position[2],
                if tr.end_only_in_vr {
                    " (the game's in VR only)"
                } else {
                    ""
                }
            )),
            if tr.enters { "enters" } else { "" },
            if tr.exits { "exits" } else { "" },
            t.bodies.len()
        );
    }
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

    let mut player = Player::new(&params, V3::from_f32(pod.spawn.position), true);
    player.pod_position = Some(V3::from_f32(point));
    let mut r = Run {
        s,
        params,
        player,
        pod,
        pod_point: V3::from_f32(point),
        hatches,
        vitals: None,
        vitals_events: Vec::new(),
        body,
        steps: 0,
        penetrations: 0,
        crossings: 0,
        min_gap: f64::INFINITY,
        step_us: Vec::new(),
        walk_speeds: Vec::new(),
        swim_speeds: Vec::new(),
    };
    for i in 0..r.pod.triggers.len() {
        if r.pod.triggers[i].active {
            r.show_trigger(i, true);
        }
    }
    r.s.stream(r.player.position)?;
    r.log("spawned");
    Ok(r)
}

/// Stands for a second, then walks to the exit that ends in the water and
/// uses it. False if it could not.
pub(crate) fn leave_pod(r: &mut Run) -> Result<bool> {
    r.body.set_phase("in the pod");
    for _ in 0..50 {
        r.step(&Input::default())?;
    }
    let floor =
        r.s.world
            .cast(sn_sim::collide::MOVE, r.player.position, -V3::Y, 0.0, 5.0)
            .map(|(d, _)| d);
    r.log(&format!(
        "standing; floor {} below the camera",
        floor.map_or("not found".into(), |d| format!("{d:.3} m"))
    ));
    let level = r.params.ocean_level;
    let exit = (0..r.hatches.triggers.len()).find(|&i| {
        let t = &r.hatches.triggers[i];
        t.active && t.exits && t.end.is_some_and(|e| e.y < level)
    });
    let Some(i) = exit else {
        println!("  no active exit ends in the water");
        return Ok(false);
    };
    r.body.set_phase("walking to the hatch");
    let ok = r.go_to(r.at(i), 10.0, Some(i), 0.0)?;
    if !ok {
        r.log(&format!(
            "could not use {:?}",
            r.pod.triggers[i].trigger.name
        ));
    }
    Ok(ok)
}

/// Prints the collision checks and the cost per step. Returns the
/// penetrations plus the surfaces passed through (0 for a good run).
pub(crate) fn report(r: &Run) -> usize {
    let mut us = r.step_us.clone();
    us.sort_by(f64::total_cmp);
    println!(
        "{} steps ({:.1} s simulated); penetrations (gap < {PENETRATION} m): {}; surfaces passed through: {}; smallest gap {:.4} m; cost per step mean {:.1} µs, p99 {:.1} µs",
        r.steps,
        r.t(),
        r.penetrations,
        r.crossings,
        r.min_gap,
        us.iter().sum::<f64>() / us.len().max(1) as f64,
        percentile(&us, 0.99)
    );
    r.penetrations + r.crossings
}

pub fn run(game: &GameData, seed: u64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
    let mut r = start(&mut s, seed)?;
    let point = r.pod_point;

    // 1–2. Stand, walk to the exit that ends in the water, use it.
    let mut ok = leave_pod(&mut r)?;

    // 3. Swim away for 10 s, back, and board through the nearest entry.
    if ok {
        let out = r.player.position - point;
        let away = V3::new(out.x, 0.0, out.z)
            .normalized()
            .unwrap_or(V3::new(1.0, 0.0, 0.0));
        let far = r.player.position + away * 100.0 + V3::new(0.0, -3.0, 0.0);
        r.body.set_phase("swimming away");
        r.go_to(far, 10.0, None, 0.5)?;
        r.log(&format!(
            "swam away, {:.1} m from the pod",
            (r.player.position - point).length()
        ));
        ok = board(&mut r)?;
        r.body.set_phase("after boarding");
        for _ in 0..50 {
            r.step(&Input::default())?;
        }
        r.log("after boarding");
        ok &= r.player.in_pod && r.player.motor == Motor::Walk && r.player.grounded;
    }

    let stats = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        (percentile(&s, 0.5), s.last().copied().unwrap_or(0.0))
    };
    let (walk_p50, walk_max) = stats(&r.walk_speeds);
    let (swim_p50, swim_max) = stats(&r.swim_speeds);
    println!(
        "speeds: walking (grounded, horizontal) median {walk_p50:.2} max {walk_max:.2} m/s (read: {}); swimming median {swim_p50:.2} max {swim_max:.2} m/s (read: {:.2}, after the drag {:.2})",
        r.params.walk_speed,
        r.params.swim_forward,
        r.params.swim_forward * (1.0 - r.params.swim_drag * r.params.fixed_dt)
    );
    let penetrations = report(&r);
    let body_ok = r.body.report() & check_body(&r.body, &WALK_EXPECTED);
    crate::swim::print_load_stats(&s);
    let ok = ok && penetrations == 0 && body_ok;
    println!(
        "{} (total {:.1} s)",
        if ok { "RUN OK" } else { "RUN FAILED" },
        started.elapsed().as_secs_f64()
    );
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What a phase's base layer must have been in (`all`: some of these,
/// nothing else; `has`: at least these), from the plan's "M9g3 expected
/// states", written before the run.
pub(crate) struct Expected {
    pub(crate) phase: &'static str,
    pub(crate) layer: &'static str,
    pub(crate) has: &'static [&'static str],
    pub(crate) only: &'static [&'static str],
}

/// States no phase of the scripts may reach (`docs/DESIGN.md`).
const NEVER: &[&str] = &[
    "Dive",
    "Dive_loops",
    "player_view_jump_loop",
    "player_view_jump_land",
    "pda",
    "cyclops_steering",
];

const WALK_EXPECTED: [Expected; 4] = [
    Expected {
        phase: "in the pod",
        layer: "Base Modes",
        has: &["Walking"],
        only: &["Walking"],
    },
    Expected {
        phase: "walking to the hatch",
        layer: "Base Modes",
        has: &["Walking"],
        only: &["Walking"],
    },
    Expected {
        phase: "swimming away",
        layer: "Base Modes",
        has: &["Swim"],
        only: &["Walking", "Swim", "surface swim"],
    },
    Expected {
        phase: "after boarding",
        layer: "Base Modes",
        has: &["Walking"],
        only: &["Walking", "Swim", "surface swim"],
    },
];

/// Checks the states seen against `expected` and [`NEVER`]; prints each.
pub(crate) fn check_body(body: &BodyRun, expected: &[Expected]) -> bool {
    let mut ok = true;
    for e in expected {
        let seen = body.states(e.phase, e.layer);
        let good = e.has.iter().all(|s| seen.contains(*s))
            && seen.iter().all(|s| e.only.contains(&s.as_str()));
        println!(
            "check: {} {:?} in {:?}, expected {:?} (only {:?}): {}",
            e.phase,
            e.layer,
            seen,
            e.has,
            e.only,
            if good { "ok" } else { "FAILED" }
        );
        ok &= good;
    }
    let all = body.states("", "Base Modes");
    let bad: Vec<&String> = all.iter().filter(|s| NEVER.contains(&s.as_str())).collect();
    println!(
        "check: never in {NEVER:?}: {}",
        if bad.is_empty() {
            "ok".to_string()
        } else {
            format!("FAILED {bad:?}")
        }
    );
    ok && bad.is_empty()
}

/// Swims to the nearest active entry and uses it. False if it could not.
pub(crate) fn board(r: &mut Run) -> Result<bool> {
    r.body.set_phase("boarding");
    let here = r.player.position;
    let entry = (0..r.hatches.triggers.len())
        .filter(|&i| r.hatches.triggers[i].active && r.hatches.triggers[i].enters)
        .min_by(|&a, &b| {
            (r.at(a) - here)
                .length()
                .total_cmp(&(r.at(b) - here).length())
        });
    let Some(i) = entry else {
        println!("  no active entry");
        return Ok(false);
    };
    // Under it first (it sits under the floor), then up to it.
    let at = r.at(i);
    let used = r.go_to(at - V3::Y * 1.5, 30.0, Some(i), 0.5)? || r.go_to(at, 20.0, Some(i), 0.0)?;
    if !used {
        r.log(&format!(
            "could not use {:?}",
            r.pod.triggers[i].trigger.name
        ));
    }
    Ok(used)
}
