//! `sn-client`: the desktop client. For now (M3) it streams the terrain
//! around the lifepod start from the player's install and lets you fly
//! around it.

mod terrain;

use std::path::PathBuf;
use std::time::Duration;

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::PresentMode;
use sn_install::GameData;
use sn_world::BatchCoord;

use crate::terrain::TerrainLoader;

const USAGE: &str = "\
Usage: sn-client [--game-dir <PATH>] [--batch <X> <Y> <Z>] [--radius <R>]
                 [--benchmark <FRAMES>]

  --game-dir     folder containing Subnautica.exe (or set SUBNAUTICA_DIR)
  --batch        terrain batch to centre on (default 12 18 12, the lifepod start)
  --radius       batches to load around it (default 1 = 3x3x3 batches)
  --benchmark    after loading, render FRAMES frames without vsync, log frame
                 times, save a screenshot to out/client-benchmark.png and exit

Controls: right mouse (hold) or M (toggle) to look around, WASD to move,
Q/E down/up, Shift to go fast, mouse wheel to change speed.
";

struct Args {
    game_dir: Option<PathBuf>,
    batch: BatchCoord,
    radius: i32,
    benchmark: Option<usize>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        game_dir: None,
        batch: BatchCoord::new(12, 18, 12),
        radius: 1,
        benchmark: None,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut it = raw.iter();
    let number = |s: Option<&String>, what: &str| -> Result<i64, String> {
        s.ok_or(format!("{what} needs a value"))?
            .parse()
            .map_err(|_| format!("{what} needs a whole number"))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game-dir" => {
                args.game_dir = Some(it.next().ok_or("--game-dir needs a path")?.into());
            }
            "--batch" => {
                let x = number(it.next(), "--batch")? as i32;
                let y = number(it.next(), "--batch")? as i32;
                let z = number(it.next(), "--batch")? as i32;
                args.batch = BatchCoord::new(x, y, z);
            }
            "--radius" => args.radius = number(it.next(), "--radius")? as i32,
            "--benchmark" => args.benchmark = Some(number(it.next(), "--benchmark")? as usize),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

/// Benchmark state: frame times recorded after the terrain finished loading.
#[derive(Resource)]
struct Benchmark {
    frames: usize,
    times: Vec<f32>,
    screenshot_requested: bool,
}

fn main() -> AppExit {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return AppExit::error();
        }
    };
    let loader = match GameData::locate(args.game_dir.clone())
        .map_err(String::from)
        .and_then(|game| terrain::start(game, args.batch, args.radius))
    {
        Ok(loader) => loader,
        Err(message) => {
            eprintln!("error: {message}");
            return AppExit::error();
        }
    };

    let present_mode = if args.benchmark.is_some() {
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
    .add_plugins((FrameTimeDiagnosticsPlugin::default(), FreeCameraPlugin))
    .insert_resource(ClearColor(WATER_COLOUR))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        ..default()
    })
    .insert_resource(loader)
    .add_systems(Startup, setup)
    .add_systems(Update, (terrain::receive, log_stats));
    if let Some(frames) = args.benchmark {
        app.insert_resource(Benchmark {
            frames,
            times: Vec::with_capacity(frames),
            screenshot_requested: false,
        })
        .add_systems(Update, run_benchmark);
    }
    app.run()
}

const WATER_COLOUR: Color = Color::srgb(0.05, 0.25, 0.35);

fn setup(mut commands: Commands) {
    // Just above the water at the lifepod start (Unity world (0, 0, 0); see
    // sn_world::voxel_to_world), looking down towards the seabed.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-40.0, 5.0, 40.0).looking_at(Vec3::new(20.0, -25.0, -20.0), Vec3::Y),
        FreeCamera {
            walk_speed: 15.0,
            run_speed: 80.0,
            ..default()
        },
        DistanceFog {
            color: WATER_COLOUR,
            falloff: FogFalloff::Linear {
                start: 60.0,
                end: 450.0,
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

/// Logs frame rate, frame time and terrain progress every 2 seconds.
fn log_stats(
    time: Res<Time>,
    mut last: Local<Duration>,
    diagnostics: Res<DiagnosticsStore>,
    loader: Res<TerrainLoader>,
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
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let position = camera.single().map(|t| t.translation).unwrap_or_default();
    info!(
        "stats: {fps:.0} fps, {frame_ms:.2} ms/frame, terrain {}/{} batches, {} triangles, camera {position:.0}",
        loader.loaded,
        loader.total.map_or("?".into(), |n| n.to_string()),
        loader.triangles,
    );
}

fn run_benchmark(
    mut commands: Commands,
    time: Res<Time>,
    loader: Res<TerrainLoader>,
    mut bench: ResMut<Benchmark>,
) {
    if !loader.done() || bench.screenshot_requested {
        return;
    }
    if bench.times.len() < bench.frames {
        bench.times.push(time.delta_secs() * 1000.0);
        return;
    }
    let mut sorted = bench.times.clone();
    sorted.sort_by(f32::total_cmp);
    let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
    let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
    let max = sorted.last().copied().unwrap_or(0.0);
    info!(
        "benchmark: {} frames after loading; mean {mean:.2} ms ({:.0} fps), 95th percentile {p95:.2} ms, worst {max:.2} ms; {} triangles; load time {:.2} s",
        sorted.len(),
        1000.0 / mean,
        loader.triangles,
        loader.finished_after.unwrap_or(0.0),
    );
    bench.screenshot_requested = true;
    if let Err(e) = std::fs::create_dir_all("out") {
        error!("out/: {e}");
    }
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk("out/client-benchmark.png"))
        .observe(
            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::Success);
            },
        );
}
