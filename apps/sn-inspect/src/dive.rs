//! `dive` (M9c): oxygen, suffocation, death and respawn with the game's
//! numbers, headless, through the player's rules and collision. Out of
//! the lifepod, up to the surface, down to the seabed and held there
//! until the player suffocates; the respawn in the pod; out again, a dive
//! of 20 s cut short by surfacing. Logs oxygen each second next to what
//! the values read predict, and the times between the events. See
//! `docs/DESIGN.md` § 4.3 "M9c plan".

use std::process::ExitCode;
use std::time::Instant;

use sn_install::GameData;
use sn_sim::V3;
use sn_sim::player::Input;
use sn_sim::vitals::{
    DEATH_TO_RESPAWN, DepthClass, RESPAWN_TO_RESTORE, RESTORE_TO_CONTROLS, Situation, Vitals,
    VitalsEvent, VitalsParams, breath_period, depth_class, depth_of, oxygen_per_breath,
};

use crate::Result;
use crate::collision::Streamed;
use crate::walk::{Expected, Run, board, check_body, check_hatches, leave_pod, report, start};

/// The plan's "M9g3 expected states" and "M9g5d expected values" for the
/// dive's phases.
const DIVE_EXPECTED: [Expected; 11] = [
    Expected {
        phase: "hatch bot_out_trigger_first",
        layer: "Cinematics",
        has: &["escapepod_first_botout_cine"],
        only: &["New State", "escapepod_first_botout_cine"],
    },
    Expected {
        phase: "hatch bot_out_trigger",
        layer: "Cinematics",
        has: &["escapepod_botout"],
        only: &["New State", "escapepod_botout"],
    },
    Expected {
        phase: "hatch bot_in_trigger",
        layer: "Cinematics",
        has: &["escapepod_botin"],
        only: &["New State", "escapepod_botin"],
    },
    Expected {
        phase: "clear of the pod",
        layer: "Cinematics",
        has: &["New State"],
        only: &["New State", "escapepod_first_botout_cine"],
    },
    Expected {
        phase: "clear of the pod again",
        layer: "Cinematics",
        has: &["New State"],
        only: &["New State", "escapepod_botout"],
    },
    Expected {
        phase: "hatch bot_out_trigger",
        layer: "Base Modes",
        has: &[],
        only: &["Walking", "Swim", "surface swim"],
    },
    Expected {
        phase: "in the pod",
        layer: "Base Modes",
        has: &["Walking"],
        only: &["Walking"],
    },
    Expected {
        phase: "at the surface",
        layer: "Base Modes",
        has: &["surface swim"],
        only: &["Swim", "surface swim"],
    },
    Expected {
        phase: "down to the seabed",
        layer: "Base Modes",
        has: &["Swim"],
        only: &["Swim", "surface swim", "Walking"],
    },
    Expected {
        phase: "down to the seabed",
        layer: "Death",
        has: &["New State", "player_death"],
        only: &["New State", "player_death"],
    },
    Expected {
        phase: "in the pod",
        layer: "Death",
        has: &["New State"],
        only: &["New State"],
    },
];

fn input(dir: V3) -> Input {
    Input {
        move_dir: dir,
        ..Input::default()
    }
}

/// Our own running prediction of the oxygen from the values read: a
/// steady drain at the depth class's rate while the player cannot
/// breathe, the refill while the source is near the surface. The game's
/// breaths are steps of one period's worth, so the two may differ by one
/// breath.
struct Predicted {
    oxygen: f64,
    /// The largest breath seen in the prediction's span.
    breath: f64,
}

impl Predicted {
    fn step(&mut self, vp: &VitalsParams, dt: f64, s: &Situation, frozen: bool) {
        let class = depth_class(depth_of(vp, s.y));
        if Vitals::underwater(vp, s) && !frozen {
            let period = breath_period(class);
            if class != DepthClass::Surface {
                self.breath = self.breath.max(oxygen_per_breath(period, class));
            }
            self.oxygen = (self.oxygen - oxygen_per_breath(period, class) / period * dt).max(0.0);
        }
        if s.y + vp.oxygen_above_player > vp.ocean_level - 1.0 || !Vitals::underwater(vp, s) {
            self.oxygen = (self.oxygen + vp.refill_per_second * dt).min(vp.oxygen_capacity);
        }
    }
}

struct Dive<'r, 'a, 'g> {
    r: &'r mut Run<'a, 'g>,
    predicted: Predicted,
    /// Largest |oxygen − predicted| seen, the breath it is checked
    /// against, and the time.
    worst: (f64, f64, f64),
}

impl Dive<'_, '_, '_> {
    fn situation(&self) -> Situation {
        Situation {
            y: self.r.player.position.y,
            in_pod: self.r.player.in_pod,
            landed: None,
            world_settled: true,
            cinematic: false,
        }
    }

    fn vitals(&self) -> (&VitalsParams, &Vitals) {
        let (vp, v) = self.r.vitals.as_ref().expect("vitals run in the dive");
        (vp, v)
    }

    /// One step; logs the oxygen once per simulated second.
    fn step(&mut self, dir: V3) -> Result<()> {
        self.r.step(&input(dir))?;
        let s = self.situation();
        let dt = self.r.params.fixed_dt;
        let (vp, v) = self.vitals();
        let (vp, oxygen, frozen) = (vp.clone(), v.oxygen, !v.controls_enabled());
        // Frozen from the death until the controls are back; the restore
        // sets the oxygen, so the prediction follows it there.
        if frozen {
            self.predicted.oxygen = oxygen;
        } else {
            self.predicted.step(&vp, dt, &s, frozen);
        }
        let diff = oxygen - self.predicted.oxygen;
        if diff.abs() > self.worst.0 {
            self.worst = (diff.abs(), self.predicted.breath, self.r.t());
        }
        if self.r.steps % (1.0 / dt).round() as usize == 0 {
            let depth = depth_of(&vp, s.y);
            let (_, v) = self.vitals();
            self.r.log(&format!(
                "depth {depth:.2} m ({:?}), oxygen {oxygen:.2} (predicted {:.2}), health {:.1}, overlay {:.2}",
                depth_class(depth),
                self.predicted.oxygen,
                v.health,
                v.overlay()
            ));
        }
        Ok(())
    }

    fn underwater(&self) -> bool {
        Vitals::underwater(self.vitals().0, &self.situation())
    }

    /// The time of the first event `e` from index `from` on.
    fn when(&self, e: VitalsEvent, from: usize) -> Option<f64> {
        self.r.vitals_events[from..]
            .iter()
            .find(|(_, x)| *x == e)
            .map(|(t, _)| *t)
    }

    fn time(&self) -> f64 {
        self.vitals().1.time
    }

    /// Swims 8 m away from the pod at 2 m below the surface (the exits
    /// end under the pod, where there is no way straight up).
    fn clear_of_the_pod(&mut self) -> Result<()> {
        let out = self.r.player.position - self.r.pod_point;
        let away = V3::new(out.x, 0.0, out.z)
            .normalized()
            .unwrap_or(V3::new(1.0, 0.0, 0.0));
        let mut target = self.r.pod_point + away * 8.0;
        target.y = self.r.params.ocean_level - 2.0;
        self.r.go_to(target, 10.0, None, 0.5)?;
        self.r.log("clear of the pod");
        self.resync();
        Ok(())
    }

    /// Restarts the prediction from the actual oxygen, after steps taken
    /// by the shared walking code (which does not predict).
    fn resync(&mut self) {
        self.predicted.oxygen = self.vitals().1.oxygen;
    }
}

pub fn run(game: &GameData, seed: u64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
    let mut r = start(&mut s, seed)?;
    let assets = r.s.loader.assets();
    let data = sn_assets::player_data(assets)?;
    let code = sn_assets::player_code(&sn_assets::read_assembly(game)?)?;
    let vp = sn_assets::vitals_params(&data, &code);
    println!(
        "vitals read: oxygen capacity {}, refill {}/s at the surface (source {} m above the player), suffocation {} s, recovery {} s, max health {} × {} on respawn, ocean level {}",
        vp.oxygen_capacity,
        vp.refill_per_second,
        vp.oxygen_above_player,
        vp.suffocation_time,
        vp.suffocation_recovery_time,
        vp.max_health,
        vp.start_health_percent,
        vp.ocean_level
    );
    println!(
        "ported from the code: breath periods safe {} / unsafe {} / crush {} s, costs {} / {} / {} per breath; death → respawn {DEATH_TO_RESPAWN} s → restore {RESPAWN_TO_RESTORE} s → controls {RESTORE_TO_CONTROLS} s",
        breath_period(DepthClass::Safe),
        breath_period(DepthClass::Unsafe),
        breath_period(DepthClass::Crush),
        oxygen_per_breath(3.0, DepthClass::Safe),
        oxygen_per_breath(2.25, DepthClass::Unsafe),
        oxygen_per_breath(1.5, DepthClass::Crush),
    );
    r.vitals = Some((vp.clone(), Vitals::new(&vp)));
    let mut d = Dive {
        r: &mut r,
        predicted: Predicted {
            oxygen: vp.oxygen_capacity,
            breath: 0.0,
        },
        worst: (0.0, 0.0, 0.0),
    };
    let mut ok = true;
    let mut check = |what: &str, good: bool| {
        println!("check: {what}: {}", if good { "ok" } else { "FAILED" });
        ok &= good;
    };
    let dt = d.r.params.fixed_dt;
    let up = V3::new(0.0, 1.0, 0.0);
    let down = V3::new(0.0, -1.0, 0.0);

    // 1. Out of the pod and up to the surface.
    let out = leave_pod(d.r)?;
    check("left the pod", out);
    d.r.body.set_phase("clear of the pod");
    d.clear_of_the_pod()?;
    d.r.body.set_phase("up");
    let mut above = 0;
    for _ in 0..(15.0 / dt) as usize {
        d.step(up)?;
        above = if d.underwater() { 0 } else { above + 1 };
        if above == 1 {
            d.r.body.set_phase("at the surface");
        }
        if above as f64 * dt >= 2.0 {
            break;
        }
    }
    d.r.log("at the surface");
    check(
        "breathing at the surface, oxygen full",
        !d.underwater() && d.vitals().1.oxygen == vp.oxygen_capacity,
    );

    // 2. Down to the seabed and held there until the death and respawn.
    d.r.body.set_phase("down to the seabed");
    let from = d.r.vitals_events.len();
    let mut went_under = None;
    let mut y_at_empty = None;
    for _ in 0..(150.0 / dt) as usize {
        d.step(down)?;
        if went_under.is_none() && d.underwater() {
            went_under = Some(d.time());
            d.r.log("under water");
        }
        if y_at_empty.is_none() && d.when(VitalsEvent::SuffocationStarted, from).is_some() {
            y_at_empty = Some(d.r.player.position.y);
        }
        if d.when(VitalsEvent::ControlsBack, from).is_some() {
            break;
        }
    }
    let ev = |e| d.when(e, from);
    let (empty, died, moved, restored, back) = (
        ev(VitalsEvent::SuffocationStarted),
        ev(VitalsEvent::Died),
        ev(VitalsEvent::MoveToRespawn),
        ev(VitalsEvent::Restored),
        ev(VitalsEvent::ControlsBack),
    );
    let span = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => b - a,
        _ => f64::NAN,
    };
    let to_empty = span(went_under, empty);
    let to_death = span(empty, died);
    println!(
        "times: under water → oxygen 0 {to_empty:.2} s (read: {} units at the depth's rate, ± one breath); → death {to_death:.2} s (read: suffocationTime {}); → respawn {:.2} s; → restored {:.2} s; → controls {:.2} s",
        vp.oxygen_capacity,
        vp.suffocation_time,
        span(died, moved),
        span(moved, restored),
        span(restored, back)
    );
    let one = dt * 1.5;
    let seabed_class = depth_class(depth_of(&vp, y_at_empty.unwrap_or(0.0)));
    println!(
        "depth class where the oxygen ran out: {seabed_class:?} ({:.2} m)",
        depth_of(&vp, y_at_empty.unwrap_or(0.0))
    );
    let period = breath_period(seabed_class);
    let rate = oxygen_per_breath(period, seabed_class) / period;
    check(
        "time to empty within one breath period of capacity / rate",
        (to_empty - vp.oxygen_capacity / rate).abs() <= period + one,
    );
    check(
        "death after suffocationTime (± one step)",
        (to_death - vp.suffocation_time - dt).abs() <= one,
    );
    check(
        "respawn, restore and controls at the game's delays",
        (span(died, moved) - DEATH_TO_RESPAWN).abs() <= one
            && (span(moved, restored) - RESPAWN_TO_RESTORE).abs() <= one
            && (span(restored, back) - RESTORE_TO_CONTROLS).abs() <= one,
    );
    {
        let (_, v) = d.vitals();
        check(
            "respawned in the pod with full oxygen and health",
            d.r.player.in_pod && v.oxygen == vp.oxygen_capacity && v.health == vp.max_health,
        );
    }

    // 3. Out again, 20 s down, then up until it breathes: the refill.
    let out = leave_pod(d.r)?;
    check("left the pod again", out);
    d.r.body.set_phase("clear of the pod again");
    d.clear_of_the_pod()?;
    d.r.body.set_phase("second dive");
    for _ in 0..(20.0 / dt) as usize {
        d.step(down)?;
    }
    // The seabed may have led back under the pod.
    d.clear_of_the_pod()?;
    d.r.body.set_phase("up again");
    let mut surfaced = None;
    let mut full = None;
    for _ in 0..(30.0 / dt) as usize {
        d.step(up)?;
        let o = d.vitals().1.oxygen;
        if surfaced.is_none() && !d.underwater() {
            surfaced = Some((d.time(), o));
            d.r.log(&format!("surfaced with oxygen {o:.2}"));
        }
        if surfaced.is_some() && o == vp.oxygen_capacity {
            full = Some(d.time());
            break;
        }
    }
    // The refill starts 1 m below the surface (the source test), so it is
    // timed from when the oxygen last fell.
    let refill_start =
        d.r.vitals_events
            .iter()
            .rev()
            .find(|(_, e)| matches!(e, VitalsEvent::Breath(_)))
            .map(|(t, _)| *t);
    match (surfaced, full) {
        (Some((ts, o)), Some(tf)) => {
            println!(
                "refill: surfaced with {o:.2}, full {:.2} s later (read: {}/s, from 1 m below the surface; the last breath was {:.2} s before surfacing)",
                tf - ts,
                vp.refill_per_second,
                refill_start.map_or(f64::NAN, |b| ts - b)
            );
            check(
                "full within (capacity − oxygen) / refill rate of surfacing",
                tf - ts <= (vp.oxygen_capacity - o) / vp.refill_per_second + one,
            );
        }
        _ => check("surfaced and refilled", false),
    }
    // Back under (not onto the pod's roof) before swimming to the entry.
    for _ in 0..(1.0 / dt) as usize {
        d.step(down)?;
    }
    let boarded = board(d.r)?;
    check("boarded", boarded);
    println!(
        "oxygen against the prediction: largest difference {:.3} at t {:.2} s (one breath: {:.3})",
        d.worst.0, d.worst.2, d.worst.1
    );
    check(
        "oxygen within one breath of the prediction",
        d.worst.0 <= d.worst.1 + 1e-9,
    );
    let deaths =
        d.r.vitals_events
            .iter()
            .filter(|(_, e)| *e == VitalsEvent::Died)
            .count();
    check("exactly one death", deaths == 1);

    let penetrations = report(d.r);
    check(
        "no penetrations or surfaces passed through",
        penetrations == 0,
    );
    let body_ok = d.r.body.report();
    check(
        "body: no NaN, unit quaternions, every parameter known",
        body_ok,
    );
    let states_ok = check_body(&d.r.body, &DIVE_EXPECTED);
    check("body: states as expected", states_ok);
    let names: Vec<&str> = d.r.hatch_uses.iter().map(|h| h.name.as_str()).collect();
    check(
        &format!("hatch cinematics: first-use exit, exit, entry ({names:?})"),
        names == ["bot_out_trigger_first", "bot_out_trigger", "bot_in_trigger"],
    );
    let hatches_ok = check_hatches(d.r);
    check(
        "hatch cinematics: durations, end places, oxygen",
        hatches_ok,
    );
    crate::swim::print_load_stats(&s);
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
