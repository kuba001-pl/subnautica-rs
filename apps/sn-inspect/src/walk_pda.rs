//! `walk --pda` (M9u8a2): the PDA opened and closed by script, in the
//! lifepod (walking) and under water (swimming), with the player's body
//! and animator as `walk` runs them. Checks the open and close times
//! against `sn_sim::pda`'s sequence, each arm layer's states against the
//! ones written in `docs/DESIGN.md` (M9u8a2) before the run, and logs the
//! screen anchor's pose relative to the eye once the arms hold the PDA.

use std::process::ExitCode;
use std::time::Instant;

use sn_install::GameData;
use sn_sim::pda::{Event as PdaEvent, TIME_DRAW, TIME_HOLSTER};
use sn_sim::player::Input;
use sn_sim::{Pose, V3};

use crate::Result;
use crate::collision::Streamed;
use crate::walk::{Expected, NEVER, Run, check_body, leave_pod, report, start};

/// How long the PDA stays open, and how long the script waits after it
/// closed.
const HOLD: f64 = 2.0;
const AFTER: f64 = 1.0;

/// One open–hold–close of the PDA.
struct Cycle {
    label: &'static str,
    open_at: f64,
    activated: Option<f64>,
    close_at: f64,
    deactivated: Option<f64>,
    /// The model active from the open to the `Deactivated`, on every step.
    model_ok: bool,
    /// The anchor in the eye's space at the end of the hold: position
    /// (m) and the angle between its +z and the eye's (degrees).
    anchor_in_eye: (V3, f64),
}

/// The eye's frame: `to` in `eye`'s space.
fn in_space(eye: Pose, to: Pose) -> Pose {
    let inv = eye.rotation.inverse();
    Pose::new(inv.rotate(to.position - eye.position), inv * to.rotation)
}

fn steps_until(r: &mut Run, input: &Input, event: PdaEvent, limit: f64) -> Result<Option<f64>> {
    let end = r.t() + limit;
    while r.t() < end {
        let from = r.pda_events.len();
        r.step(input)?;
        if let Some((t, _)) = r.pda_events[from..].iter().find(|(_, e)| *e == event) {
            return Ok(Some(*t));
        }
    }
    Ok(None)
}

/// Opens the PDA, holds it [`HOLD`] s, closes it and waits [`AFTER`] s,
/// standing still; the phases are `{label}: opening`, `open`, `closing`
/// and `after`.
fn cycle(r: &mut Run, label: &'static str) -> Result<Cycle> {
    // Stand still facing where the player looks now.
    let input = Input {
        yaw: r.look.yaw(),
        pitch: r.look.pitch(),
        ..Input::default()
    };
    r.body.set_phase(&format!("{label}: opening"));
    let open_at = r.t();
    let opened = r.pda.open(None, r.player.position, r.player.cinematic);
    r.log(&format!("{label}: PDA.Open → {opened}"));
    let mut model_ok = r.pda.active();
    let activated = steps_until(r, &input, PdaEvent::Activated, 2.0)?;
    model_ok &= r.pda.active();
    r.body.set_phase(&format!("{label}: open"));
    let end = r.t() + HOLD;
    while r.t() < end {
        r.step(&input)?;
        model_ok &= r.pda.active();
    }
    let player = Pose::new(r.player.position, r.player.rotation);
    let eye = r.body.camera(player, None);
    let (_, anchor) = r.body.pda_poses(player);
    let a = in_space(eye, anchor);
    let z = V3::new(0.0, 0.0, 1.0);
    let angle = a
        .rotation
        .rotate(z)
        .dot(z)
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    let cam = r.body.camera_pose();
    r.log(&format!(
        "{label}: held {HOLD} s; camera pitch {:.3}° (root {:.3}°, up {:.3}°), offset z {:.4} m; the screen anchor in the eye's space at ({:.4}, {:.4}, {:.4}) m, its +z {angle:.2}° from the eye's",
        cam.camera_pitch(),
        cam.root_pitch,
        cam.up_pitch,
        cam.offset_z,
        a.position.x,
        a.position.y,
        a.position.z
    ));
    r.body.set_phase(&format!("{label}: closing"));
    let close_at = r.t();
    let closed = r.pda.close();
    r.log(&format!("{label}: PDA.Close → {closed}"));
    let mut deactivated = None;
    let end = r.t() + 2.0;
    while r.t() < end && deactivated.is_none() {
        model_ok &= r.pda.active();
        let from = r.pda_events.len();
        r.step(&input)?;
        deactivated = r.pda_events[from..]
            .iter()
            .find(|(_, e)| *e == PdaEvent::Deactivated)
            .map(|(t, _)| *t);
    }
    r.body.set_phase(&format!("{label}: after"));
    let end = r.t() + AFTER;
    while r.t() < end {
        r.step(&input)?;
        model_ok &= !r.pda.active();
    }
    Ok(Cycle {
        label,
        open_at,
        activated,
        close_at,
        deactivated,
        model_ok,
        anchor_in_eye: (a.position, angle),
    })
}

/// The update after `t` reaches its end, at the 50 Hz step: the
/// sequence's time plus one or two steps.
fn sequence_ok(took: f64, time: f32, dt: f64) -> bool {
    let time = f64::from(time);
    took >= time + dt - 1e-6 && took <= time + 2.0 * dt + 1e-6
}

/// The upperbody layer's transitions in `[from, to)`: (time, from, to).
fn transitions(r: &Run, layer: &str, from: f64, to: f64) -> Vec<(f64, String, String)> {
    r.body
        .transitions
        .iter()
        .filter(|(t, l, _, _)| l == layer && *t >= from - 1e-9 && *t < to - 1e-9)
        .map(|(t, _, a, b)| (*t, a.clone(), b.clone()))
        .collect()
}

/// The checks of one cycle; prints each.
fn check_cycle(r: &Run, c: &Cycle) -> bool {
    let dt = r.params.fixed_dt;
    let mut ok = true;
    let mut line = |what: String, good: bool| {
        println!(
            "check: {}: {what}: {}",
            c.label,
            if good { "ok" } else { "FAILED" }
        );
        ok &= good;
    };
    let opened = c.activated.map(|t| t - c.open_at);
    line(
        format!(
            "Activated {} after the open (expected {TIME_DRAW} s + 1–2 steps of {dt} s)",
            opened.map_or("never".into(), |t| format!("{t:.3} s"))
        ),
        opened.is_some_and(|t| sequence_ok(t, TIME_DRAW, dt)),
    );
    let closed = c.deactivated.map(|t| t - c.close_at);
    line(
        format!(
            "Deactivated {} after the close (expected {TIME_HOLSTER} s + 1–2 steps)",
            closed.map_or("never".into(), |t| format!("{t:.3} s"))
        ),
        closed.is_some_and(|t| sequence_ok(t, TIME_HOLSTER, dt)),
    );
    line(
        "the model active from the open to Deactivated, then inactive".into(),
        c.model_ok,
    );
    // The arms (written before the run): on the open New State →
    // first_check → open → idle, idle starting about 0.1 + 0.833 s after;
    // on the close idle → close → New State, the last about 0.375 s after.
    let end = c.close_at + HOLD + AFTER + 1.0;
    let opening = transitions(r, "upperbody", c.open_at, c.close_at);
    let names = |v: &[(f64, String, String)]| -> Vec<String> {
        v.iter().map(|(_, a, b)| format!("{a} → {b}")).collect()
    };
    let expect_open = [
        "New State → first_check",
        "first_check → open",
        "open → idle",
    ];
    let idle_after = opening
        .iter()
        .find(|(_, a, b)| a == "open" && b == "idle")
        .map(|(t, _, _)| t - c.open_at);
    line(
        format!(
            "upperbody on the open {:?} (expected {expect_open:?}), idle started {} after (expected about 0.93 s)",
            names(&opening),
            idle_after.map_or("never".into(), |t| format!("{t:.3} s"))
        ),
        names(&opening) == expect_open && idle_after.is_some_and(|t| (t - 0.933).abs() < 0.06),
    );
    let closing = transitions(r, "upperbody", c.close_at, end);
    let expect_close = ["idle → close", "close → New State"];
    let back = closing
        .iter()
        .find(|(_, a, b)| a == "close" && b == "New State")
        .map(|(t, _, _)| t - c.close_at);
    line(
        format!(
            "upperbody on the close {:?} (expected {expect_close:?}), New State started {} after (expected about 0.375 s)",
            names(&closing),
            back.map_or("never".into(), |t| format!("{t:.3} s"))
        ),
        names(&closing) == expect_close && back.is_some_and(|t| (t - 0.375).abs() < 0.06),
    );
    let (at, angle) = c.anchor_in_eye;
    line(
        format!(
            "the screen anchor in front of the eye ({:.3} m ahead) and facing it ({angle:.1}° < 30°)",
            at.z
        ),
        at.z > 0.1 && angle < 30.0,
    );
    ok
}

/// Each phase's states (written before the run, `docs/DESIGN.md`
/// M9u8a2): the base layer `Walking` → `pda` → `Walking` in the pod and
/// only `Swim` under water; `right arm`, `left` and `both` in their
/// defaults.
const PDA_EXPECTED: [Expected; 10] = [
    Expected {
        phase: "in the pod: opening",
        layer: "Base Modes",
        has: &["Walking", "pda"],
        only: &["Walking", "pda"],
    },
    Expected {
        phase: "in the pod: open",
        layer: "Base Modes",
        has: &["pda"],
        only: &["pda"],
    },
    Expected {
        phase: "in the pod: closing",
        layer: "Base Modes",
        has: &["pda", "Walking"],
        only: &["pda", "Walking"],
    },
    Expected {
        phase: "in the pod: after",
        layer: "Base Modes",
        has: &["Walking"],
        only: &["Walking"],
    },
    Expected {
        phase: "under water: opening",
        layer: "Base Modes",
        has: &["Swim"],
        only: &["Swim"],
    },
    Expected {
        phase: "under water: open",
        layer: "Base Modes",
        has: &["Swim"],
        only: &["Swim"],
    },
    Expected {
        phase: "under water: closing",
        layer: "Base Modes",
        has: &["Swim"],
        only: &["Swim"],
    },
    Expected {
        phase: "in the pod: open",
        layer: "right arm",
        has: &["Nothing"],
        only: &["Nothing"],
    },
    Expected {
        phase: "in the pod: open",
        layer: "left",
        has: &["none"],
        only: &["none"],
    },
    Expected {
        phase: "in the pod: open",
        layer: "both",
        has: &["nothing"],
        only: &["nothing"],
    },
];

pub fn run(game: &GameData, seed: u64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
    let mut r = start(&mut s, seed)?;
    // Stand in the pod for a second, then the PDA.
    r.body.set_phase("standing");
    for _ in 0..50 {
        r.step(&Input::default())?;
    }
    let mut cycles = vec![cycle(&mut r, "in the pod")?];
    // Out through the hatch, swim 5 s away and down, the PDA again.
    let mut ok = leave_pod(&mut r)?;
    if ok {
        let point = r.pod_point;
        let out = r.player.position - point;
        let away = V3::new(out.x, 0.0, out.z)
            .normalized()
            .unwrap_or(V3::new(1.0, 0.0, 0.0));
        let far = r.player.position + away * 100.0 + V3::new(0.0, -3.0, 0.0);
        r.body.set_phase("swimming away");
        r.go_to(far, 5.0, None, 0.5)?;
        r.body.set_phase("drifting");
        for _ in 0..50 {
            r.step(&Input::default())?;
        }
        r.log("under water, still");
        cycles.push(cycle(&mut r, "under water")?);
    }
    for (t, e) in &r.pda_events {
        println!("  t {t:>6.2} s: PDA {e:?}");
    }
    let mut cycles_ok = true;
    for c in &cycles {
        cycles_ok &= check_cycle(&r, c);
    }
    // `pda` is a state of these runs; the others stay unreachable.
    let never: Vec<&str> = NEVER.iter().copied().filter(|s| *s != "pda").collect();
    let body_ok = r.body.report() & check_body(&r.body, &PDA_EXPECTED, &never);
    let penetrations = report(&r);
    crate::swim::print_load_stats(&s);
    ok &= cycles.len() == 2 && cycles_ok && body_ok && penetrations == 0;
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
