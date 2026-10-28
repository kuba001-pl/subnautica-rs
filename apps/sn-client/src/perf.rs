//! Opt-in wall-clock CPU evidence. Set SN_CLIENT_PERF=1; reports go to stdout.
//! Timers include OS descheduling and nested spans must not be added together.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use bevy::prelude::*;

static ENABLED: AtomicBool = AtomicBool::new(false);
static CAPTURING: AtomicBool = AtomicBool::new(false);
static SAMPLING: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<BTreeMap<(&str, bool), Vec<f32>>> = Mutex::new(BTreeMap::new());
static RENDER_CPU: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static TRACE: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static FRAMES: Mutex<Vec<Frame>> = Mutex::new(Vec::new());
static PHASES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Event {
    name: &'static str,
    start: f64,
    end: f64,
    count: usize,
}

struct Frame {
    start: f64,
    end: f64,
    ms: f32,
    play: f64,
    position: Option<[f32; 3]>,
}

fn now() -> f64 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Call before requesting readback: capture and exit are not gameplay.
pub fn capture_started() {
    CAPTURING.store(true, Ordering::Relaxed);
    SAMPLING.store(false, Ordering::Relaxed);
}

/// Timed schedule boundaries are opt-in, including rendering and driver work
/// outside our own systems. Their ranges overlap inner spans.
pub fn install(app: &mut App) {
    use bevy::render::{Render, RenderApp, RenderSystems};
    // With profiling enabled, keep upload accounting on even for the unlimited
    // baseline. A configured production budget takes precedence.
    if !app
        .world()
        .contains_resource::<bevy::render::render_asset::RenderAssetBytesPerFrame>()
    {
        app.insert_resource(bevy::render::render_asset::RenderAssetBytesPerFrame::new(
            usize::MAX,
        ));
    }
    app.add_systems(First, || stage_begin("main::schedules"))
        .add_systems(Last, || stage_end("main::schedules"));
    app.init_resource::<Interval>()
        .add_systems(PreUpdate, frame_start);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        use bevy::render::mesh::{RenderMesh, allocator::allocate_and_free_meshes};
        use bevy::render::render_asset::prepare_assets;
        use bevy::render::texture::GpuImage;
        render.add_systems(
            Render,
            (
                (|| stage_begin("render::mesh_alloc"))
                    .in_set(RenderSystems::PrepareAssets)
                    .before(allocate_and_free_meshes),
                (|| stage_end("render::mesh_alloc"))
                    .in_set(RenderSystems::PrepareAssets)
                    .after(allocate_and_free_meshes)
                    .before(prepare_assets::<RenderMesh>),
                (|| stage_begin("render::images"))
                    .in_set(RenderSystems::PrepareAssets)
                    .before(prepare_assets::<GpuImage>),
                (|| stage_end("render::images"))
                    .in_set(RenderSystems::PrepareAssets)
                    .after(prepare_assets::<GpuImage>)
                    .before(prepare_assets::<RenderMesh>),
            ),
        );
        render.add_systems(
            Render,
            (
                (|| stage_begin("render::assets"))
                    .after(RenderSystems::ExtractCommands)
                    .before(RenderSystems::PrepareAssets),
                (|| {
                    stage_end("render::assets");
                    stage_begin("render::meshes");
                })
                .after(RenderSystems::PrepareAssets)
                .before(RenderSystems::PrepareMeshes),
                (|| {
                    stage_end("render::meshes");
                    stage_begin("render::specialize");
                })
                .after(RenderSystems::PrepareMeshes)
                .before(RenderSystems::CreateViews),
                (|| {
                    stage_end("render::specialize");
                    stage_begin("render::queue");
                })
                .after(RenderSystems::PrepareViews)
                .before(RenderSystems::Queue),
                (|| {
                    stage_end("render::queue");
                    stage_begin("render::prepare");
                })
                .after(RenderSystems::Queue)
                .before(RenderSystems::Prepare),
                (|| {
                    stage_end("render::prepare");
                    stage_begin("render::render");
                })
                .after(RenderSystems::Prepare)
                .before(RenderSystems::Render),
                (|| stage_end("render::render"))
                    .after(RenderSystems::Render)
                    .before(RenderSystems::Cleanup),
            ),
        );
        render.add_systems(
            Render,
            phase_census
                .after(RenderSystems::Prepare)
                .before(RenderSystems::Render),
        );
        // Nested in the coarse stages above: views/specialization, batching
        // and buffer writes, bind groups, and the render graph (encode,
        // submit, present).
        render.add_systems(
            Render,
            (
                (|| stage_begin("render::views"))
                    .after(RenderSystems::PrepareMeshes)
                    .before(RenderSystems::CreateViews),
                (|| stage_end("render::views"))
                    .after(RenderSystems::PrepareViews)
                    .before(RenderSystems::Queue),
                (|| stage_begin("render::batch"))
                    .after(RenderSystems::PhaseSort)
                    .before(RenderSystems::Prepare),
                (|| {
                    stage_end("render::batch");
                    stage_begin("render::bind_groups");
                })
                .after(RenderSystems::PrepareResourcesFlush)
                .before(RenderSystems::PrepareBindGroups),
                (|| {
                    stage_end("render::bind_groups");
                    stage_begin("render::graph");
                })
                .after(RenderSystems::PrepareBindGroups)
                .before(RenderSystems::Render),
                (|| stage_end("render::graph"))
                    .after(RenderSystems::Render)
                    .before(RenderSystems::Cleanup),
            ),
        );
    }
}

/// Draw-structure census of the latest sampled frame: per view, how many
/// multi-draw batch sets (one indirect draw and one bin-unpacking dispatch
/// each), batchable and unbatchable bins, and sorted items each phase has.
fn phase_census(
    opaque: Res<
        bevy::render::render_phase::ViewBinnedRenderPhases<bevy::core_pipeline::core_3d::Opaque3d>,
    >,
    mask: Res<
        bevy::render::render_phase::ViewBinnedRenderPhases<
            bevy::core_pipeline::core_3d::AlphaMask3d,
        >,
    >,
    shadow: Res<bevy::render::render_phase::ViewBinnedRenderPhases<bevy::pbr::Shadow>>,
    transparent: Res<
        bevy::render::render_phase::ViewSortedRenderPhases<
            bevy::core_pipeline::core_3d::Transparent3d,
        >,
    >,
    indirect: Res<bevy::render::batching::gpu_preprocessing::IndirectParametersBuffers>,
) {
    if !SAMPLING.load(Ordering::Relaxed) {
        return;
    }
    fn binned<B: bevy::render::render_phase::BinnedPhaseItem>(
        out: &mut Vec<String>,
        name: &str,
        phases: &bevy::render::render_phase::ViewBinnedRenderPhases<B>,
        indirect: &bevy::render::batching::gpu_preprocessing::IndirectParametersBuffers,
    ) {
        let mut rows: Vec<_> = phases
            .values()
            .map(|p| {
                [
                    p.multidrawable_meshes.len(),
                    p.batchable_meshes.len(),
                    p.unbatchable_meshes.len(),
                ]
            })
            .collect();
        rows.sort_unstable();
        let indirect_draws = indirect
            .buffers
            .get(&std::any::TypeId::of::<B>())
            .map_or(0, |b| b.indexed.batch_count() + b.non_indexed.batch_count());
        let direct_draws: usize = phases
            .values()
            .map(|p| {
                p.batchable_meshes.len()
                    + p.unbatchable_meshes
                        .values()
                        .map(|b| b.entities.len())
                        .sum::<usize>()
            })
            .sum();
        let instances: usize = phases
            .values()
            .flat_map(|p| p.batchable_meshes.values())
            .map(|b| b.entities().len())
            .sum();
        out.push(format!(
            "{name}: {} views, [multidraw batch sets, batchable bins, unbatchable bins] per view {rows:?}; effective mesh draws={}; direct batch instances={instances}",
            rows.len(), indirect_draws + direct_draws
        ));
    }
    let mut out = Vec::new();
    binned(&mut out, "opaque", &opaque, &indirect);
    binned(&mut out, "alpha_mask", &mask, &indirect);
    binned(&mut out, "shadow", &shadow, &indirect);
    let mut sorted: Vec<_> = transparent.values().map(|p| p.items.len()).collect();
    sorted.sort_unstable();
    out.push(format!("transparent: sorted items per view {sorted:?}"));
    if let Ok(mut phases) = PHASES.lock() {
        *phases = out;
    }
}

static STAGES: Mutex<BTreeMap<&str, Span>> = Mutex::new(BTreeMap::new());
fn stage_begin(name: &'static str) {
    if let Ok(mut stages) = STAGES.lock() {
        stages.insert(name, span(name));
    }
}
fn stage_end(name: &'static str) {
    // Release this lock before dropping Span (it records in other locks).
    let timer = STAGES.lock().ok().and_then(|mut s| s.remove(name));
    drop(timer);
}

#[derive(Default)]
struct Window {
    previous: f64,
    start: f64,
    end: f64,
}

fn frame_start(mut window: Local<Window>, mut interval: ResMut<Interval>) {
    let end = now();
    window.start = window.previous;
    window.end = end;
    window.previous = end;
    interval.0 = [window.start, window.end];
}

#[derive(Resource, Default)]
pub(crate) struct Interval([f64; 2]);

const MAX_SAMPLES: usize = 120_000;

pub fn init() -> bool {
    let enabled = std::env::var("SN_CLIENT_PERF").as_deref() == Ok("1");
    ENABLED.store(enabled, Ordering::Relaxed);
    enabled
}

pub struct Span(Option<(&'static str, Instant, bool, f64)>);

pub fn span(name: &'static str) -> Span {
    Span(ENABLED.load(Ordering::Relaxed).then(|| {
        (
            name,
            Instant::now(),
            SAMPLING.load(Ordering::Relaxed),
            now(),
        )
    }))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, start, sampling, at)) = self.0 {
            let ms = start.elapsed().as_secs_f32() * 1000.0;
            record(name, ms, sampling);
            if ms >= 0.1
                && let Ok(mut events) = TRACE.lock()
                && events.len() < MAX_SAMPLES
            {
                events.push(Event {
                    name,
                    start: at,
                    end: at + f64::from(ms),
                    count: 0,
                });
            }
        }
    }
}

pub fn count(name: &'static str, count: usize) {
    if count == 0 || !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let at = now();
    if let Ok(mut events) = TRACE.lock()
        && events.len() < MAX_SAMPLES
    {
        events.push(Event {
            name,
            start: at,
            end: at + 0.0001,
            count,
        });
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
    interval: Res<Interval>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    measurement: Option<Res<crate::Measurement>>,
    player: Option<Res<crate::player::PlayerSim>>,
    mut frames: Local<usize>,
) {
    let sampling = !CAPTURING.load(Ordering::Relaxed)
        && measurement.as_ref().map_or_else(
            || player.as_ref().is_some_and(|p| p.play_seconds() >= 1.0),
            |m| matches!(m.phase, crate::Phase::Running | crate::Phase::Arriving),
        );
    record("frame", time.delta_secs() * 1000.0, sampling);
    if sampling
        && let Ok(mut frames) = FRAMES.lock()
        && frames.len() < MAX_SAMPLES
    {
        frames.push(Frame {
            start: interval.0[0],
            end: interval.0[1],
            ms: time.delta_secs() * 1000.0,
            play: player.as_ref().map_or(0.0, |p| p.play_seconds()),
            position: player
                .as_ref()
                .and_then(|p| p.position())
                .map(|p| [p.x as f32, p.y as f32, p.z as f32]),
        });
    }
    SAMPLING.store(sampling, Ordering::Relaxed);
    *frames += 1;
    let snapshot_due = *frames >= 120;
    if snapshot_due {
        *frames = 0;
    }
    if sampling && snapshot_due {
        let mut costs: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                let path = d.path().as_str();
                path.ends_with("elapsed_cpu") || path.ends_with("elapsed_gpu")
            })
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
    report_trace();
    let Ok(samples) = SAMPLES.lock() else { return };
    println!(
        "perf: wall milliseconds; sampled=benchmark interval or player play>=1s; other=loading/warmup; cap={MAX_SAMPLES} per series; overlapping spans are not additive"
    );
    if let Ok(phases) = PHASES.lock() {
        for line in phases.iter() {
            println!("perf: phases {line}");
        }
    }
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

/// Write numeric evidence only under ignored out/. Spans use a shared monotonic
/// clock, so render/worker work can be correlated without assuming thread order.
fn report_trace() {
    use std::io::Write;
    let (Ok(frames), Ok(events)) = (FRAMES.lock(), TRACE.lock()) else {
        return;
    };
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all("out/p1")?;
        let mut file = std::io::BufWriter::new(std::fs::File::create("out/p1/frames.csv")?);
        writeln!(file, "start_ms,end_ms,frame_ms,play_seconds,x,y,z")?;
        for f in frames.iter() {
            let p = f.position.unwrap_or([0.0; 3]);
            writeln!(
                file,
                "{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
                f.start, f.end, f.ms, f.play, p[0], p[1], p[2]
            )?;
        }
        let mut file = std::io::BufWriter::new(std::fs::File::create("out/p1/spans.csv")?);
        writeln!(file, "name,start_ms,end_ms,count")?;
        for e in events.iter() {
            writeln!(file, "{},{:.3},{:.3},{}", e.name, e.start, e.end, e.count)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("perf: trace write failed: {e}");
    }
    let mut slow: Vec<_> = frames.iter().enumerate().collect();
    slow.sort_by(|a, b| b.1.ms.total_cmp(&a.1.ms));
    for (idx, f) in slow.into_iter().take(12) {
        println!(
            "perf: slow frame {:.3} ms play={:.3} interval={:.3}..{:.3} position={:?}",
            f.ms, f.play, f.start, f.end, f.position
        );
        let count_start = idx
            .checked_sub(1)
            .map_or(f.start - 25.0, |prev| frames[prev].start);
        let mut costs: BTreeMap<&str, f64> = BTreeMap::new();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for e in events
            .iter()
            .filter(|e| e.start < f.end && e.end > count_start)
        {
            if e.count > 0 {
                *counts.entry(e.name).or_default() += e.count;
            }
            if e.end > f.start {
                *costs.entry(e.name).or_default() +=
                    (e.end.min(f.end) - e.start.max(f.start)).max(0.0);
            }
        }
        let mut costs: Vec<_> = costs.into_iter().collect();
        costs.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, count) in counts {
            println!("perf: work {name}={count}");
        }
        for (name, ms) in costs.into_iter().take(8) {
            println!("perf: overlapping {name}={ms:.3} ms");
        }
    }
}
