//! `sn-client`: the desktop client. For now it streams the terrain around the
//! camera from the player's install (with levels of detail) and lets you fly
//! around it.

mod terrain;
mod terrain_look;

use std::path::PathBuf;
use std::time::Duration;

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::diagnostic::{
    DiagnosticsStore, FrameTimeDiagnosticsPlugin, SystemInformationDiagnosticsPlugin,
};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::PresentMode;
use sn_install::GameData;

use crate::terrain::{LodRanges, TerrainStreamer};
use crate::terrain_look::{PendingTerrainLook, TerrainLookPlugin};

const USAGE: &str = "\
Usage: sn-client [--game-dir <PATH>] [--start <X> <Y> <Z>] [--look <X> <Y> <Z>]
                 [--view <METRES>] [--debug-colours]
                 [--benchmark <FRAMES> | --flythrough <X> <Y> <Z> [--speed <M/S>]]

  --game-dir     folder containing Subnautica.exe (or set SUBNAUTICA_DIR)
  --start        camera start, Unity world coordinates (default 0 -10 0, the
                 lifepod start in the Safe Shallows)
  --look         point the camera looks at, Unity world coordinates
  --debug-colours  false colours per terrain type instead of the game's
                 terrain materials
  --view         view distance in metres (default 1200); level-of-detail
                 ranges scale with it
  --benchmark    once the start area is loaded, render FRAMES frames without
                 vsync, log frame times, save out/client-benchmark.png, exit
  --flythrough   once loaded, fly in a straight line to X Y Z (Unity world
                 coordinates) at --speed (default 40 m/s) without vsync, then
                 log frame times, memory and streaming latency, save
                 out/client-flythrough.png and exit

Controls: right mouse (hold) or M (toggle) to look around, WASD to move,
Q/E down/up, Shift to go fast, mouse wheel to change speed.
";

struct Args {
    game_dir: Option<PathBuf>,
    start: Vec3,
    look: Option<Vec3>,
    view: f32,
    benchmark: Option<usize>,
    flythrough: Option<Vec3>,
    speed: f32,
    debug_colours: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        game_dir: None,
        start: Vec3::new(0.0, -10.0, 0.0),
        look: None,
        view: 1200.0,
        benchmark: None,
        flythrough: None,
        speed: 40.0,
        debug_colours: false,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut it = raw.iter();
    let number = |s: Option<&String>, what: &str| -> Result<f32, String> {
        s.ok_or(format!("{what} needs a value"))?
            .parse()
            .map_err(|_| format!("{what} needs a number"))
    };
    let vec3 = |it: &mut std::slice::Iter<String>, what: &str| -> Result<Vec3, String> {
        Ok(Vec3::new(
            number(it.next(), what)?,
            number(it.next(), what)?,
            number(it.next(), what)?,
        ))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game-dir" => {
                args.game_dir = Some(it.next().ok_or("--game-dir needs a path")?.into());
            }
            "--start" => args.start = vec3(&mut it, "--start")?,
            "--look" => args.look = Some(vec3(&mut it, "--look")?),
            "--view" => args.view = number(it.next(), "--view")?,
            "--benchmark" => args.benchmark = Some(number(it.next(), "--benchmark")? as usize),
            "--flythrough" => args.flythrough = Some(vec3(&mut it, "--flythrough")?),
            "--speed" => args.speed = number(it.next(), "--speed")?,
            "--debug-colours" => args.debug_colours = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    if args.view < 100.0 {
        return Err("--view must be at least 100 metres".into());
    }
    Ok(args)
}

/// Reads the game's terrain materials and textures (about a second).
fn load_terrain_look(game_dir: Option<PathBuf>) -> Result<sn_assets::TerrainMaterials, String> {
    let start = std::time::Instant::now();
    let game = GameData::locate(game_dir).map_err(String::from)?;
    let assets = sn_assets::Assets::index(&game)?;
    let materials = sn_assets::terrain_materials(&assets)?;
    for w in &materials.warnings {
        eprintln!("warning: {w}");
    }
    eprintln!(
        "terrain materials: {} types, {} textures read in {:.2} s",
        materials.types.iter().flatten().count(),
        materials.texture_count,
        start.elapsed().as_secs_f64()
    );
    Ok(materials)
}

/// Unity world coordinates → Bevy (flip z; see terrain.rs).
fn unity_to_bevy(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, -p.z)
}

/// Scales the default level-of-detail ranges to the view distance.
fn lod_ranges(view: f32) -> LodRanges {
    let default = LodRanges::default().0;
    let scale = view / default[default.len() - 1];
    LodRanges(default.map(|r| r * scale))
}

fn main() -> AppExit {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return AppExit::error();
        }
    };
    let ranges = lod_ranges(args.view);
    let (look, layers) = if args.debug_colours {
        (None, [0i32; 256])
    } else {
        match load_terrain_look(args.game_dir.clone()) {
            Ok(mats) => {
                let mut layers = [0i32; 256];
                for m in mats.types.iter().flatten() {
                    layers[m.type_id] = m.layer;
                }
                (Some(mats), layers)
            }
            Err(message) => {
                eprintln!("error: {message}");
                return AppExit::error();
            }
        }
    };
    let streamer = match GameData::locate(args.game_dir.clone())
        .map_err(String::from)
        .and_then(|game| TerrainStreamer::start(game, ranges, layers))
    {
        Ok(streamer) => streamer,
        Err(message) => {
            eprintln!("error: {message}");
            return AppExit::error();
        }
    };

    let measuring = args.benchmark.is_some() || args.flythrough.is_some();
    let present_mode = if measuring {
        PresentMode::AutoNoVsync
    } else {
        PresentMode::AutoVsync
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "subnautica-rs".into(),
            resolution: (1600, 900).into(),
            present_mode,
            ..default()
        }),
        ..default()
    }))
    .add_plugins((
        FrameTimeDiagnosticsPlugin::default(),
        SystemInformationDiagnosticsPlugin,
        FreeCameraPlugin,
        TerrainLookPlugin,
    ))
    .insert_resource(PendingTerrainLook(look))
    .insert_resource(ClearColor(WATER_COLOUR))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        ..default()
    })
    .insert_resource(streamer)
    .insert_resource(Setup {
        start: unity_to_bevy(args.start),
        look: unity_to_bevy(
            args.look
                .unwrap_or(args.start + Vec3::new(60.0, -30.0, 60.0)),
        ),
        fog_end: args.view,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (terrain::stream, log_stats));
    if let Some(frames) = args.benchmark {
        app.insert_resource(Measurement::new(Mode::Benchmark { frames }))
            .add_systems(Update, measure);
    } else if let Some(to) = args.flythrough {
        let from = unity_to_bevy(args.start);
        app.insert_resource(Measurement::new(Mode::Flythrough {
            from,
            to: unity_to_bevy(to),
            speed: args.speed,
        }))
        .add_systems(Update, measure);
    }
    app.run()
}

const WATER_COLOUR: Color = Color::srgb(0.05, 0.25, 0.35);

#[derive(Resource)]
struct Setup {
    start: Vec3,
    look: Vec3,
    fog_end: f32,
}

fn setup(mut commands: Commands, settings: Res<Setup>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(settings.start).looking_at(settings.look, Vec3::Y),
        FreeCamera {
            walk_speed: 15.0,
            run_speed: 80.0,
            ..default()
        },
        DistanceFog {
            color: WATER_COLOUR,
            falloff: FogFalloff::Linear {
                start: settings.fog_end * 0.1,
                end: settings.fog_end,
            },
            ..default()
        },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 100.0, 0.0).looking_at(Vec3::new(30.0, 0.0, -50.0), Vec3::Y),
    ));
}

fn process_memory_gib(diagnostics: &DiagnosticsStore) -> f64 {
    diagnostics
        .get(&SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE)
        .and_then(|d| d.value())
        .unwrap_or(0.0)
}

/// Logs frame rate, streaming state and memory every 2 seconds.
fn log_stats(
    time: Res<Time>,
    mut last: Local<Duration>,
    diagnostics: Res<DiagnosticsStore>,
    streamer: Res<TerrainStreamer>,
    camera: Query<&Transform, With<Camera3d>>,
) {
    if time.elapsed() - *last < Duration::from_secs(2) {
        return;
    }
    *last = time.elapsed();
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let s = streamer.stats();
    let position = camera.single().map(|t| t.translation).unwrap_or_default();
    info!(
        "stats: {fps:.0} fps | batches shown {} (lod0..3 {:?}), {} triangles | queued {}, meshing {}, cached {} | process {:.2} GiB | camera (unity) {:.0} {:.0} {:.0}",
        s.shown_batches,
        s.per_lod,
        s.shown_triangles,
        s.queued,
        s.in_flight,
        s.cached_batches,
        process_memory_gib(&diagnostics),
        position.x,
        position.y,
        -position.z,
    );
}

enum Mode {
    Benchmark { frames: usize },
    Flythrough { from: Vec3, to: Vec3, speed: f32 },
}

#[derive(PartialEq)]
enum Phase {
    /// Waiting for the start area to finish loading.
    Loading,
    Running,
    /// Flythrough: arrived, waiting for the destination to finish loading.
    Arriving,
    Done,
}

#[derive(Resource)]
struct Measurement {
    mode: Mode,
    phase: Phase,
    frame_ms: Vec<f32>,
    travelled: f32,
    max_triangles: usize,
    max_cached: usize,
    max_memory_gib: f64,
    phase_started: Duration,
}

impl Measurement {
    fn new(mode: Mode) -> Self {
        Measurement {
            mode,
            phase: Phase::Loading,
            frame_ms: Vec::new(),
            travelled: 0.0,
            max_triangles: 0,
            max_cached: 0,
            max_memory_gib: 0.0,
            phase_started: Duration::ZERO,
        }
    }
}

fn percentiles(values: &[f32]) -> (f32, f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
    let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
    (mean, p95, *sorted.last().unwrap())
}

fn measure(
    mut commands: Commands,
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    mut streamer: ResMut<TerrainStreamer>,
    mut m: ResMut<Measurement>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    let Ok(mut camera) = camera.single_mut() else {
        return;
    };
    let stats = streamer.stats();
    m.max_triangles = m.max_triangles.max(stats.shown_triangles);
    m.max_cached = m.max_cached.max(stats.cached_batches);
    m.max_memory_gib = m.max_memory_gib.max(process_memory_gib(&diagnostics));
    let now = time.elapsed();

    match m.phase {
        Phase::Loading => {
            if streamer.settled() {
                info!(
                    "measure: start area loaded after {:.2} s: {} batches, {} triangles",
                    now.as_secs_f32(),
                    stats.shown_batches,
                    stats.shown_triangles
                );
                // Only latencies from here on describe streaming while moving.
                streamer.latencies.clear();
                m.phase = Phase::Running;
                m.phase_started = now;
            }
        }
        Phase::Running => {
            m.frame_ms.push(time.delta_secs() * 1000.0);
            match m.mode {
                Mode::Benchmark { frames } => {
                    if m.frame_ms.len() >= frames {
                        m.phase = Phase::Done;
                    }
                }
                Mode::Flythrough { from, to, speed } => {
                    m.travelled += speed * time.delta_secs();
                    let length = from.distance(to);
                    let t = (m.travelled / length).min(1.0);
                    let position = from.lerp(to, t);
                    *camera = Transform::from_translation(position).looking_at(
                        position + (to - from).normalize() + Vec3::new(0.0, -0.3, 0.0),
                        Vec3::Y,
                    );
                    if t >= 1.0 {
                        info!(
                            "measure: arrived after {:.1} s",
                            (now - m.phase_started).as_secs_f32()
                        );
                        m.phase = Phase::Arriving;
                        m.phase_started = now;
                    }
                }
            }
        }
        Phase::Arriving => {
            m.frame_ms.push(time.delta_secs() * 1000.0);
            let waited = (now - m.phase_started).as_secs_f32();
            if streamer.settled() || waited > 30.0 {
                info!(
                    "measure: destination loaded {waited:.2} s after arriving (settled: {})",
                    streamer.settled()
                );
                m.phase = Phase::Done;
            }
        }
        Phase::Done => return,
    }
    if m.phase != Phase::Done {
        return;
    }

    let (mean, p95, worst) = percentiles(&m.frame_ms);
    info!(
        "measure: {} frames; mean {mean:.2} ms ({:.0} fps), 95th percentile {p95:.2} ms, worst {worst:.2} ms",
        m.frame_ms.len(),
        1000.0 / mean.max(0.001),
    );
    info!(
        "measure: peak {} triangles shown, peak {} batches cached, peak process memory {:.2} GiB; now {} batches shown",
        m.max_triangles, m.max_cached, m.max_memory_gib, stats.shown_batches,
    );
    for lod in 0..=sn_terrain::MAX_LOD {
        let latency: Vec<f32> = streamer
            .latencies
            .iter()
            .filter(|l| l.0 == lod)
            .map(|l| l.1)
            .collect();
        let meshing: Vec<f32> = streamer
            .latencies
            .iter()
            .filter(|l| l.0 == lod)
            .map(|l| l.2)
            .collect();
        let (lm, lp, lw) = percentiles(&latency);
        let (mm, mp, _) = percentiles(&meshing);
        info!(
            "measure: lod {lod}: {} batches streamed; request→screen mean {lm:.0} ms, p95 {lp:.0} ms, worst {lw:.0} ms; meshing mean {mm:.0} ms, p95 {mp:.0} ms",
            latency.len(),
        );
    }
    let name = match m.mode {
        Mode::Benchmark { .. } => "out/client-benchmark.png",
        Mode::Flythrough { .. } => "out/client-flythrough.png",
    };
    if let Err(e) = std::fs::create_dir_all("out") {
        error!("out/: {e}");
    }
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(name))
        .observe(
            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::Success);
            },
        );
}
