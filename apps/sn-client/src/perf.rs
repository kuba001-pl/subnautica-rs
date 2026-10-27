//! Opt-in wall-clock CPU evidence. Set SN_CLIENT_PERF=1; reports go to stdout.
//! Timers include OS descheduling and nested spans must not be added together.
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bevy::prelude::*;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SAMPLING: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<BTreeMap<(&str, bool), Vec<f32>>> = Mutex::new(BTreeMap::new());
static RENDER_CPU: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());
const MAX_SAMPLES: usize = 120_000;

pub fn init() -> bool {
    let enabled = std::env::var("SN_CLIENT_PERF").as_deref() == Ok("1");
    ENABLED.store(enabled, Ordering::Relaxed);
    enabled
}

pub struct Span(Option<(&'static str, Instant, bool)>);

pub fn span(name: &'static str) -> Span {
    Span(
        ENABLED
            .load(Ordering::Relaxed)
            .then(|| (name, Instant::now(), SAMPLING.load(Ordering::Relaxed))),
    )
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, start, sampling)) = self.0 {
            record(name, start.elapsed().as_secs_f32() * 1000.0, sampling);
        }
    }
}

fn record(name: &'static str, ms: f32, sampling: bool) {
    if let Ok(mut samples) = SAMPLES.lock() {
        let values = samples.entry((name, sampling)).or_default();
        if values.len() < MAX_SAMPLES {
            values.push(ms);
        }
    }
}

/// Run before `measure`: select the same settled benchmark interval, or player
/// frames after one second of play. The selection also applies to worker spans.
pub fn frame(
    time: Res<Time<Real>>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    measurement: Option<Res<crate::Measurement>>,
    player: Option<Res<crate::player::PlayerSim>>,
    mut frames: Local<usize>,
) {
    let sampling = measurement.as_ref().map_or_else(
        || player.as_ref().is_some_and(|p| p.play_seconds() >= 1.0),
        |m| matches!(m.phase, crate::Phase::Running | crate::Phase::Arriving),
    );
    record("frame", time.delta_secs() * 1000.0, sampling);
    SAMPLING.store(sampling, Ordering::Relaxed);
    *frames += 1;
    let snapshot_due = *frames >= 120;
    if snapshot_due {
        *frames = 0;
    }
    if sampling && snapshot_due {
        let mut costs: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.path().as_str().ends_with("elapsed_cpu"))
            .filter_map(|d| Some((d.path().as_str().to_string(), d.average()?)))
            .collect();
        costs.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Ok(mut latest) = RENDER_CPU.lock() {
            *latest = costs;
        }
    }
}

pub fn report() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Ok(samples) = SAMPLES.lock() else { return };
    println!(
        "perf: wall milliseconds; sampled=benchmark interval or player play>=1s; other=loading/warmup; cap={MAX_SAMPLES} per series; overlapping spans are not additive"
    );
    if let Ok(costs) = RENDER_CPU.lock() {
        for (name, ms) in costs.iter() {
            println!(
                "perf: render recent diagnostic mean {name}={ms:.3} ms (last snapshot, taken every 120 frames)"
            );
        }
    }
    for ((name, sampling), values) in samples.iter() {
        if let Some(s) = crate::baseline::frame_summary(values) {
            println!(
                "perf: {} {name} n={} mean={:.3} p50={:.3} p95={:.3} p99={:.3} max={:.3}",
                if *sampling { "sampled" } else { "other" },
                values.len(),
                s.mean,
                s.median,
                s.p95,
                s.p99,
                s.worst
            );
            if *name == "frame" {
                let mut bins = [0usize; 6];
                for &v in values {
                    let bin = [8.333, 16.667, 33.333, 50.0, 100.0]
                        .iter()
                        .position(|&edge| v < edge)
                        .unwrap_or(5);
                    bins[bin] += 1;
                }
                println!("perf: frame histogram <8.333/<16.667/<33.333/<50/<100/>=100ms: {bins:?}");
            }
        }
    }
}
