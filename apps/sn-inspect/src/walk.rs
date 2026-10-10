//! `walk` (M9b): a scripted run with the player's own rules
//! (`sn_sim::player`), headless: spawn in the lifepod, walk to its exit,
//! use it, swim away for 10 s and back, board. Logs positions, motor
//! changes, the hatch triggers used and speeds next to the values read
//! from the game. See `docs/DESIGN.md` § 4.3 "M9b plan".

use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Lifepod, SCENE_BODY};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::player::{
    Event, HatchTrigger, Hatches, Input, Motor, Player, PlayerParams, hand_target,
};

use crate::Result;
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

struct Run<'a, 'g> {
    s: &'a mut Streamed<'g>,
    params: PlayerParams,
    player: Player,
    pod: Lifepod,
    hatches: Hatches,
    steps: usize,
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
    fn t(&self) -> f64 {
        self.steps as f64 * self.params.fixed_dt
    }

    /// The capsule's centre in the world.
    fn centre(&self) -> V3 {
        let c = self.player.capsule(&self.params);
        self.player.position + (c.a + c.b) * 0.5
    }

    fn log(&self, what: &str) {
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

    fn step(&mut self, input: &Input) -> Result<()> {
        if self.steps % 25 == 0 {
            self.s.stream(self.player.position)?;
        }
        let before = self.centre();
        let t = Instant::now();
        let events = self.player.step(&self.params, &self.s.world, input);
        self.step_us.push(t.elapsed().as_secs_f64() * 1e6);
        self.steps += 1;
        for e in events {
            match e {
                Event::MotorChanged(m) => self.log(&format!("motor → {m:?}")),
                Event::Landed | Event::Jumped => {}
                other => self.log(&format!("{other:?}")),
            }
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

    /// Looks at trigger `i` and uses it if the hand points at it.
    fn try_use(&mut self, i: usize) -> Option<f64> {
        let eye = self.player.position;
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
    fn go_to(
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

pub fn run(game: &GameData, seed: u64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
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

    // The lifepod as in a new game: no hatch used yet.
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
        s: &mut s,
        params,
        player,
        pod,
        hatches,
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

    // 1. Stand for a second.
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

    // 2. To the exit that ends in the water, and through it.
    let level = r.params.ocean_level;
    let exit = (0..r.hatches.triggers.len()).find(|&i| {
        let t = &r.hatches.triggers[i];
        t.active && t.exits && t.end.is_some_and(|e| e.y < level)
    });
    let mut ok = false;
    if let Some(i) = exit {
        ok = r.go_to(r.at(i), 10.0, Some(i), 0.0)?;
        if !ok {
            r.log(&format!(
                "could not use {:?}",
                r.pod.triggers[i].trigger.name
            ));
        }
    } else {
        println!("  no active exit ends in the water");
    }

    // 3. Swim away for 10 s, back, and board through the nearest entry.
    if ok {
        let pod = V3::from_f32(point);
        let out = r.player.position - pod;
        let away = V3::new(out.x, 0.0, out.z)
            .normalized()
            .unwrap_or(V3::new(1.0, 0.0, 0.0));
        let far = r.player.position + away * 100.0 + V3::new(0.0, -3.0, 0.0);
        r.go_to(far, 10.0, None, 0.5)?;
        r.log(&format!(
            "swam away, {:.1} m from the pod",
            (r.player.position - pod).length()
        ));
        let here = r.player.position;
        let entry = (0..r.hatches.triggers.len())
            .filter(|&i| r.hatches.triggers[i].active && r.hatches.triggers[i].enters)
            .min_by(|&a, &b| {
                (r.at(a) - here)
                    .length()
                    .total_cmp(&(r.at(b) - here).length())
            });
        ok = match entry {
            Some(i) => {
                // Under it first (it sits under the floor), then up to it.
                let at = r.at(i);
                let used = r.go_to(at - V3::Y * 1.5, 30.0, Some(i), 0.5)?
                    || r.go_to(at, 20.0, Some(i), 0.0)?;
                if !used {
                    r.log(&format!(
                        "could not use {:?}",
                        r.pod.triggers[i].trigger.name
                    ));
                }
                used
            }
            None => {
                println!("  no active entry");
                false
            }
        };
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
    let penetrations = r.penetrations + r.crossings;
    crate::swim::print_load_stats(&s);
    let ok = ok && penetrations == 0;
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
