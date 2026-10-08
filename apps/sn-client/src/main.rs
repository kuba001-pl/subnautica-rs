//! `sn-client`: the desktop client. For now it streams the terrain around the
//! camera from the player's install (with levels of detail) and lets you fly
//! around it.

mod game_light;
mod object_look;
mod objects;
mod sky;
mod sky_dome;
mod stars;
mod sun_shafts;
mod terrain;
mod terrain_look;
mod textures;
mod water;
mod water_fft;
mod water_surface;

use std::path::PathBuf;
use std::time::Duration;

use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::diagnostic::{
    DiagnosticsStore, FrameTimeDiagnosticsPlugin, SystemInformationDiagnosticsPlugin,
};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::PresentMode;
use sn_install::GameData;

use crate::game_light::{GameLightPlugin, PendingCaustics};
use crate::object_look::ObjectLookPlugin;
use crate::objects::ObjectStreamer;
use crate::sky_dome::{PendingSky, PendingStars, SkyDomePlugin, SkyWorld};
use crate::terrain::{BlockSettings, LodRanges, TerrainStreamer};
use crate::terrain_look::{PendingTerrainLook, SUN_ILLUMINANCE, TerrainLookPlugin};
use crate::water::{WaterData, WaterFog, WaterFogOff, WaterFogPlugin, WaterWorld};
use crate::water_surface::{
    PendingWaterSurface, WaterQuality, WaterSurfacePlugin, WaterSurfaceUniform,
};

const USAGE: &str = "\
Usage: sn-client [--game-dir <PATH>] [--start <X> <Y> <Z>] [--look <X> <Y> <Z>]
                 [--view <METRES>] [--debug-colours]
                 [--benchmark <FRAMES> | --flythrough <X> <Y> <Z> [--speed <M/S>]]

  --game-dir     folder containing Subnautica.exe (or set SUBNAUTICA_DIR)
  --start        camera start, Unity world coordinates (default 0 -10 0, the
                 lifepod start in the Safe Shallows)
  --look         point the camera looks at, Unity world coordinates
  --debug-colours  false colours per terrain type instead of the game's
                 terrain materials (also turns world objects off)
  --no-objects   terrain only, no world objects (coral, rocks, …)
  --fog-unit     scale on the game's light values (calibration; default 1:
                 one game light unit = 1.0 in the image, as in Unity)
  --color-grading  off | neutral | aces: the game's option of that name
                 (default off, the game's default; neutral and aces use
                 Bevy's nearest tonemappers for now)
  --no-water-fog HDR camera without the water fog (for comparisons)
  --no-water-surface  no water surface (for comparisons)
  --water-quality  medium | high: the game's option of that name (default
                 high: simulated waves; medium: the 64 baked frames)
  --gpu-timings  with --benchmark/--flythrough: log GPU time per render pass
  --time         game clock in hours for the sun and sky (default 9.6, i.e.
                 09:36, when a new game starts)
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
    no_objects: bool,
    fog_unit: f32,
    color_grading: ColorGrading,
    no_water_fog: bool,
    no_water_surface: bool,
    water_quality: WaterQuality,
    gpu_timings: bool,
    /// Game clock, hours.
    time: f32,
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
        no_objects: false,
        fog_unit: 1.0,
        color_grading: ColorGrading::Off,
        no_water_fog: false,
        no_water_surface: false,
        water_quality: WaterQuality::High,
        gpu_timings: false,
        time: sky::NEW_GAME_HOURS,
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
            "--no-objects" => args.no_objects = true,
            "--no-water-fog" => args.no_water_fog = true,
            "--no-water-surface" => args.no_water_surface = true,
            "--water-quality" => {
                args.water_quality = match it.next().map(String::as_str) {
                    Some("medium") => WaterQuality::Medium,
                    Some("high") => WaterQuality::High,
                    _ => return Err("--water-quality takes medium or high".into()),
                }
            }
            "--gpu-timings" => args.gpu_timings = true,
            "--time" => args.time = number(it.next(), "--time")?,
            "--fog-unit" => args.fog_unit = number(it.next(), "--fog-unit")?,
            "--color-grading" => {
                args.color_grading = match it.next().map(String::as_str) {
                    Some("off") => ColorGrading::Off,
                    Some("neutral") => ColorGrading::Neutral,
                    Some("aces") => ColorGrading::Aces,
                    _ => return Err("--color-grading takes off, neutral or aces".into()),
                }
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    if args.view < 100.0 {
        return Err("--view must be at least 100 metres".into());
    }
    Ok(args)
}

/// What the client reads from the game at start-up.
struct GameLook {
    materials: sn_assets::TerrainMaterials,
    water: WaterData,
    sky: (sn_unity::SkyManager, sn_unity::SkyLight),
    surface: sn_assets::WaterSurfaceData,
    sky_textures: sn_assets::SkyTextures,
    caustics: Vec<sn_assets::TerrainTexture>,
    stars: Vec<stars::GpuStar>,
}

/// Reads the game's terrain materials and textures, water and sky (about a
/// second).
fn load_terrain_look(game_dir: Option<PathBuf>) -> Result<GameLook, String> {
    let start = std::time::Instant::now();
    let game = GameData::locate(game_dir).map_err(String::from)?;
    let assets = sn_assets::Assets::index(&game)?;
    let materials = sn_assets::terrain_materials(&assets)?;
    let water = load_water(&game, &assets)?;
    let sky = sn_assets::sky(&assets)?;
    let surface = sn_assets::water_surface(&assets)?;
    let sky_textures = sn_assets::sky_textures(&assets, &sky.0)?;
    let stars = stars::parse(&sn_assets::resource_bytes(&assets, "starsdata")?)?;
    let caustics =
        sn_assets::water_caustics(&assets, surface.surface.num_caustics_frames.max(0) as usize)?;
    for w in &materials.warnings {
        eprintln!("warning: {w}");
    }
    eprintln!(
        "terrain materials: {} types, {} textures read in {:.2} s",
        materials.types.iter().flatten().count(),
        materials.texture_count,
        start.elapsed().as_secs_f64()
    );
    Ok(GameLook {
        materials,
        water,
        sky,
        surface,
        sky_textures,
        caustics,
        stars,
    })
}

/// The biome map, batch override biomes and the scene's water settings.
fn load_water(game: &GameData, assets: &sn_assets::Assets) -> Result<WaterData, String> {
    let index = game.read_index()?;
    let batch_size = sn_terrain::batch_voxels(&index);
    let (map, names) = game.read_biome_map()?;
    let manager = sn_assets::water_biomes(assets)?;
    let volume = sn_assets::water_volume(assets)?;
    let mut overrides = std::collections::HashMap::new();
    let (batches, _) = game.object_batches()?;
    for coord in batches {
        let Some(tree) = game.read_batch_objects(coord)? else {
            continue;
        };
        for o in tree.objects.iter().filter(|o| o.parent.is_none()) {
            for c in o
                .components
                .iter()
                .filter(|c| c.type_name == "LargeWorldBatchRoot")
            {
                let s = sn_world::BatchRootSettings::parse(&c.data)
                    .map_err(|e| format!("batch {coord}: {e}"))?;
                if let Some(b) = s.override_biome {
                    overrides.insert(coord, b);
                }
            }
        }
    }
    Ok(WaterData {
        cell: 2.0 * manager.region_bounds / manager.texture_size.max(1) as f32,
        biomes: manager
            .biomes
            .into_iter()
            .map(|b| (b.name, b.settings))
            .collect(),
        land_size: (index.batches()[0] as i32 * batch_size[0]) as usize,
        batch_size,
        map,
        names,
        overrides,
        volume,
    })
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
    let (look, blocks, water, sky_data, surface, sky_textures, caustics, stars) =
        if args.debug_colours {
            (None, None, None, None, None, None, None, None)
        } else {
            match load_terrain_look(args.game_dir.clone()) {
                Ok(GameLook {
                    materials,
                    water,
                    sky: sky_data,
                    surface,
                    sky_textures,
                    caustics,
                    stars,
                }) => {
                    let mut blocks = BlockSettings {
                        layer: [0; 256],
                        gloss: [0.0; 256],
                    };
                    for m in materials.types.iter().flatten() {
                        blocks.layer[m.type_id] = m.layer;
                        blocks.gloss[m.type_id] = m.blend.gloss;
                    }
                    (
                        Some(materials),
                        Some(blocks),
                        Some(water),
                        Some(sky_data),
                        Some(surface),
                        Some(sky_textures),
                        Some(caustics),
                        Some(stars),
                    )
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    return AppExit::error();
                }
            }
        };
    let streamer = match GameData::locate(args.game_dir.clone())
        .map_err(String::from)
        .and_then(|game| TerrainStreamer::start(game, ranges, blocks))
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
        ObjectLookPlugin,
        WaterFogPlugin,
        WaterSurfacePlugin,
        SkyDomePlugin,
        GameLightPlugin,
        sun_shafts::SunShaftsPlugin,
    ))
    .insert_resource(PendingTerrainLook(look))
    .insert_resource(PendingCaustics(caustics))
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
    .add_systems(
        Update,
        (
            terrain::stream,
            objects::stream_objects
                .after(terrain::stream)
                .run_if(resource_exists::<ObjectStreamer>),
            log_stats,
        ),
    );
    if args.gpu_timings {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    if let Some((manager, light)) = &sky_data {
        let state = sky::state(manager, light, sky::timeline_from_clock(args.time));
        info!(
            "sky at {:.2} h (sky timeline {:.2} h): sun {:?} from {:?}, top ambient {:?}",
            args.time, state.timeline, state.sun, state.to_sun, state.top_ambient
        );
        app.insert_resource(state);
        if let Some(textures) = sky_textures {
            // A new game's first day: the clock's fraction of it (the
            // planet's orbit depends on the day).
            app.insert_resource(PendingSky(Some(textures)))
                .insert_resource(PendingStars(stars))
                .insert_resource(SkyWorld {
                    manager: manager.clone(),
                    day: f64::from(args.time) / 24.0,
                });
        }
    }
    let underwater = water.is_some();
    if let Some(water) = water {
        // Our image holds the game's values: one game light unit is 1.0,
        // as in Unity's HDR buffer.
        app.insert_resource(WaterWorld::new(water, args.fog_unit));
    }
    let surface = surface.filter(|_| !args.no_water_surface);
    app.insert_resource(args.color_grading);
    app.insert_resource(Underwater {
        on: underwater,
        surface: surface.is_some(),
    });
    if args.no_water_fog {
        app.insert_resource(WaterFogOff);
    }
    app.insert_resource(PendingWaterSurface(surface));
    app.insert_resource(args.water_quality);
    if !args.debug_colours && !args.no_objects {
        match GameData::locate(args.game_dir.clone()) {
            Ok(game) => {
                app.insert_resource(ObjectStreamer::start(game));
            }
            Err(e) => {
                eprintln!("error: {e}");
                return AppExit::error();
            }
        }
    }
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

/// The game's "Color grading" option (Unity's Post Processing Stack v1,
/// `UwePostProcessingManager`): off (the default) writes the HDR values
/// clamped to 0…1; neutral and ACES tonemap (here: Bevy's nearest
/// tonemappers, **not** the game's exact curves yet).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
enum ColorGrading {
    Off,
    Neutral,
    Aces,
}

impl ColorGrading {
    fn tonemapping(self) -> bevy::core_pipeline::tonemapping::Tonemapping {
        use bevy::core_pipeline::tonemapping::Tonemapping;
        match self {
            ColorGrading::Off => Tonemapping::None,
            ColorGrading::Neutral => Tonemapping::Reinhard,
            ColorGrading::Aces => Tonemapping::AcesFitted,
        }
    }
}

const WATER_COLOUR: Color = Color::srgb(0.05, 0.25, 0.35);

#[derive(Resource)]
struct Setup {
    start: Vec3,
    look: Vec3,
    fog_end: f32,
}

/// Whether the game's water fog replaces the plain distance fog, and
/// whether there is a water surface.
#[derive(Resource)]
struct Underwater {
    on: bool,
    surface: bool,
}

fn setup(
    mut commands: Commands,
    settings: Res<Setup>,
    underwater: Res<Underwater>,
    sky: Option<Res<sky::SkyState>>,
    measuring: Option<Res<Measurement>>,
    grading: Res<ColorGrading>,
) {
    let mut camera = commands.spawn((
        Camera3d::default(),
        Transform::from_translation(settings.start).looking_at(settings.look, Vec3::Y),
    ));
    // Measurements keep the camera where they put it (mouse or keyboard
    // input in the window would otherwise move it).
    if measuring.is_none() {
        camera.insert(FreeCamera {
            walk_speed: 15.0,
            run_speed: 80.0,
            ..default()
        });
    }
    if underwater.on {
        // The fog pass reads depth and works on linear HDR colour.
        camera.insert((
            Camera3d {
                depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING)
                    .into(),
                ..default()
            },
            bevy::camera::Hdr,
            WaterFog::default(),
            // The game's soft shadows.
            bevy::light::ShadowFilteringMethod::Gaussian,
            grading.tonemapping(),
        ));
        if underwater.surface {
            // The surface pass starts from a copy of the fogged image.
            camera.insert((
                WaterSurfaceUniform::default(),
                bevy::camera::CameraMainTextureUsages::default()
                    .with(TextureUsages::COPY_SRC | TextureUsages::COPY_DST),
            ));
        }
    } else {
        camera.insert(DistanceFog {
            color: WATER_COLOUR,
            falloff: FogFalloff::Linear {
                start: settings.fog_end * 0.1,
                end: settings.fog_end,
            },
            ..default()
        });
    }
    // The game's sun when we have its sky (one game light unit is
    // SUN_ILLUMINANCE), else a fixed white one.
    let (direction, colour, illuminance) = match sky.as_deref() {
        Some(s) => {
            let peak = s.sun.max_element().max(1e-6);
            let c = s.sun / peak;
            (
                -s.to_sun,
                Color::linear_rgb(c.x, c.y, c.z),
                SUN_ILLUMINANCE * peak,
            )
        }
        None => (
            Vec3::new(30.0, -100.0, -50.0).normalize(),
            Color::WHITE,
            SUN_ILLUMINANCE,
        ),
    };
    commands.spawn((
        DirectionalLight {
            illuminance,
            color: colour,
            // The game's sun casts shadows ("Detail" High: soft shadows).
            shadow_maps_enabled: true,
            ..default()
        },
        sun_cascades(),
        Transform::default().looking_to(direction, Vec3::Y),
    ));
}

/// The game's shadow cascades at "Detail" High (`QualitySettings`, read
/// once with UnityPy on the dev machine; `docs/formats/lighting.md`
/// § Shadows): 4 cascades to 50 m, split at 6.7 %, 20 % and 46.7 %.
fn sun_cascades() -> bevy::light::CascadeShadowConfig {
    let distance = 50.0;
    let mut config = bevy::light::CascadeShadowConfigBuilder {
        num_cascades: 4,
        minimum_distance: 0.1,
        maximum_distance: distance,
        first_cascade_far_bound: distance * 0.066_666_67,
        overlap_proportion: 0.2,
    }
    .build();
    config.bounds = vec![
        distance * 0.066_666_67,
        distance * 0.2,
        distance * 0.466_666_7,
        distance,
    ];
    config
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
    objects: Option<Res<ObjectStreamer>>,
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
    if let Some(objects) = objects {
        let o = objects.stats();
        info!(
            "objects: {} entities (cell levels 0..3 {:?}) in {} batches, {} queued | {} prefabs, {} meshes, {} materials, {} textures | {} warnings",
            o.entities,
            o.per_level,
            o.batches,
            o.queued,
            o.prefabs,
            o.meshes,
            o.materials,
            o.textures,
            o.warnings
        );
        info!(
            "objects: {} MarmosetUBER materials ({} with specular maps, {} with glow maps); made per sky: {:?}",
            o.uber[0], o.uber[1], o.uber[2], o.per_sky
        );
    }
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
    objects: Option<Res<ObjectStreamer>>,
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
    let settled = streamer.settled() && objects.as_ref().is_none_or(|o| o.settled(&streamer));

    match m.phase {
        Phase::Loading => {
            if settled {
                info!(
                    "measure: start area loaded after {:.2} s: {} batches, {} meshes, {} triangles",
                    now.as_secs_f32(),
                    stats.shown_batches,
                    stats.shown_meshes,
                    stats.shown_triangles
                );
                if let Some(o) = &objects {
                    let s = o.stats();
                    info!(
                        "measure: objects: {} entities (cell levels 0..3 {:?}), {} prefabs, {} meshes, {} materials, {} textures, {} warnings",
                        s.entities,
                        s.per_level,
                        s.prefabs,
                        s.meshes,
                        s.materials,
                        s.textures,
                        s.warnings
                    );
                }
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
            if settled || waited > 30.0 {
                info!(
                    "measure: destination loaded {waited:.2} s after arriving (settled: {settled})"
                );
                m.phase = Phase::Done;
            }
        }
        Phase::Done => return,
    }
    if m.phase != Phase::Done {
        return;
    }

    // With --gpu-timings: the render diagnostics' GPU times.
    let mut gpu: Vec<(String, f64)> = diagnostics
        .iter()
        .filter(|d| d.path().as_str().ends_with("elapsed_gpu"))
        .filter_map(|d| Some((d.path().as_str().to_string(), d.average()?)))
        .collect();
    gpu.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (path, ms) in gpu.iter().take(12) {
        info!("measure: gpu {path}: {ms:.3} ms");
    }
    let (mean, p95, worst) = percentiles(&m.frame_ms);
    info!(
        "measure: {} frames; mean {mean:.2} ms ({:.0} fps), 95th percentile {p95:.2} ms, worst {worst:.2} ms",
        m.frame_ms.len(),
        1000.0 / mean.max(0.001),
    );
    let (p, f) = (camera.translation, camera.forward());
    info!(
        "measure: camera (unity) at {:.1} {:.1} {:.1}, looking {:.2} {:.2} {:.2}",
        p.x, p.y, -p.z, f.x, f.y, -f.z
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
