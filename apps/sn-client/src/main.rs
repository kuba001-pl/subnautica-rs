//! `sn-client`: the desktop client. It streams the world around the camera
//! from the player's install (terrain with levels of detail, objects, the
//! scenes) and puts you in it as the player (M9b), or lets you fly around
//! with `--free-cam`.

mod animation;
mod aurora;
mod body;
mod effects;
mod game_light;
mod grass_look;
mod hud;
mod lifepod_light;
mod object_look;
mod objects;
mod player;
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
use std::sync::Arc;
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

use crate::game_light::{GameLightPlugin, LightTextures, PendingLightTextures};
use crate::object_look::ObjectLookPlugin;
use crate::objects::{BodyOptions, ObjectStreamer, SceneOptions};
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
  --start        camera start, Unity world coordinates (default: where the
                 player starts, in Lifepod 5; 0 -10 0 with --no-scenes)
  --look         point the camera looks at, Unity world coordinates
  --debug-colours  false colours per terrain type instead of the game's
                 terrain materials (also turns world objects off)
  --no-objects   terrain only, no world objects (coral, rocks, …)
  --no-local-lights  the objects' point and spot lights off (for comparisons)
  --slot-seed    world seed for filling the spawn slots (default 1; the game
                 picks anew in every save)
  --no-slots     leave the spawn slots empty (for comparisons)
  --no-placeholders  don't spawn what the objects' placeholders hold (the
                 cache doors, key terminals, ion crystals; for comparisons)
  --no-grass     no terrain grass (for comparisons)
  --no-animation the objects' animators off: everything in its stored pose
                 (for comparisons)
  --no-lod       objects always at their most detailed level, never culled
                 by distance (for comparisons)
  --fov          vertical field of view in degrees: the game's option of
                 that name (default: its default, 60, read from the game)
  --no-scenes    without the scenes the game spawns at start (the Aurora,
                 the Precursor bases it holds)
  --aurora       intact | exploded: hold the Aurora before or after its
                 explosion (default: a new game's, on the game clock)
  --aurora-countdown  <S>: the Aurora's countdown S game seconds after the
                 start (default: Random.Range(2.3, 4) days, drawn from
                 --lifepod-seed); the ship is swapped 27 s after it
  --lifepod-seed seed of a new game's random draws: Lifepod 5's start point
                 and the Aurora's countdown (default 1; the game draws anew
                 in every new game)
  --lifepod      <X> <Z>: put Lifepod 5 there instead (Unity world metres)
  --lifepod-state  operational | danger | damaged: the pod's lighting state
                 (default damaged: a new game's after the intro, played or
                 skipped, until the pod is repaired)
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
                 09:36, when a new game starts); the clock runs from there
                 (the sun and sky stay at the start time for now)
  --time-scale   game seconds per real second (default 1, as the game)
  --view         view distance in metres (default 1200); level-of-detail
                 ranges scale with it
  --benchmark    once the start area is loaded, render FRAMES frames without
                 vsync, log frame times, save out/client-benchmark.png, exit
  --free-cam     a fly camera instead of the player (also with --benchmark
                 and --flythrough)
  --third-person a debug camera 2.5 m behind the player's eye that shows
                 the head (the player's body is drawn in either view)
  --look-down    the player's starting pitch, degrees (negative looks up;
                 clamped to the game's limits)
  --shot         --shot <SECONDS> <NAME>: after SECONDS of play, save
                 out/NAME.png and exit (a check for the human)
  --use-hatch    --use-hatch <SECONDS[,SECONDS…]>: at each time (seconds of
                 play), aim at the nearest usable lifepod hatch and use it
                 (a debug check of the hatch cinematics, M9g5e)
  --hatch-name   --hatch-name <TEXT>: --use-hatch picks only hatches whose
                 trigger name contains TEXT (e.g. bot_out)
  --hold-forward --hold-forward <SECONDS>: from then on, hold W (a debug
                 check that the player moves)
  --kill         --kill <SECONDS>: after SECONDS of play the player takes
                 its full health as damage (a debug check of the death)
  --flythrough   once loaded, fly in a straight line to X Y Z (Unity world
                 coordinates) at --speed (default 40 m/s) without vsync, then
                 log frame times, memory and streaming latency, save
                 out/client-flythrough.png and exit

Controls (player): click to capture the mouse, Esc to release it; mouse to
look, WASD to move, Space up / jump, C down, E or left click to use (the
lifepod's hatches).
Controls (--free-cam): right mouse (hold) or M (toggle) to look around, WASD
to move, Q/E down/up, Shift to go fast, mouse wheel to change speed.
";

struct Args {
    game_dir: Option<PathBuf>,
    start: Vec3,
    /// `--start` given (else the player's spawn in the lifepod).
    start_given: bool,
    lifepod_seed: u64,
    /// `--lifepod X Z`.
    lifepod: Option<[f32; 2]>,
    look: Option<Vec3>,
    view: f32,
    benchmark: Option<usize>,
    flythrough: Option<Vec3>,
    speed: f32,
    debug_colours: bool,
    no_objects: bool,
    no_local_lights: bool,
    /// `None`: spawn slots stay empty.
    slot_seed: Option<u64>,
    no_placeholders: bool,
    no_grass: bool,
    no_animation: bool,
    no_lod: bool,
    /// `--fov`: else the game's default.
    fov: Option<f32>,
    /// `None`: no scenes.
    scenes: Option<SceneOptions>,
    fog_unit: f32,
    color_grading: ColorGrading,
    no_water_fog: bool,
    no_water_surface: bool,
    water_quality: WaterQuality,
    gpu_timings: bool,
    /// Game clock, hours.
    time: f32,
    /// `--time-scale`: the clock's speed.
    time_scale: f32,
    /// `--aurora`: held intact (false) or exploded (true).
    aurora_held: Option<bool>,
    /// `--aurora-countdown`, seconds after the start.
    aurora_countdown: Option<f32>,
    /// `--free-cam`: the fly camera instead of the player.
    free_cam: bool,
    /// `--third-person`: the camera behind the player's eye (M9g4).
    third_person: bool,
    /// `--look-down`: the starting pitch, degrees down.
    look_down: f32,
    /// `--use-hatch`: seconds of play at which to use the nearest hatch.
    use_hatch: Vec<f64>,
    /// `--hatch-name`: only hatches whose name contains this.
    hatch_name: Option<String>,
    /// `--hold-forward`: hold W from this much play on.
    hold_forward: Option<f32>,
    /// `--kill`: seconds of play before the player dies.
    kill: Option<f32>,
    /// `--shot`: seconds of play, then `out/<name>.png`.
    shot: Option<(f32, String)>,
    /// `--lifepod-state`: the pod's `LightingController` state.
    lifepod_state: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        game_dir: None,
        start: Vec3::new(0.0, -10.0, 0.0),
        start_given: false,
        lifepod_seed: 1,
        lifepod: None,
        look: None,
        view: 1200.0,
        benchmark: None,
        flythrough: None,
        speed: 40.0,
        debug_colours: false,
        no_objects: false,
        no_local_lights: false,
        slot_seed: Some(1),
        no_placeholders: false,
        no_grass: false,
        no_animation: false,
        no_lod: false,
        fov: None,
        scenes: Some(SceneOptions {
            lifepod: None,
            lifepod_state: sn_sim::lighting::DAMAGED,
        }),
        fog_unit: 1.0,
        color_grading: ColorGrading::Off,
        no_water_fog: false,
        no_water_surface: false,
        water_quality: WaterQuality::High,
        gpu_timings: false,
        time: sky::NEW_GAME_HOURS,
        time_scale: 1.0,
        aurora_held: None,
        aurora_countdown: None,
        free_cam: false,
        third_person: false,
        look_down: 0.0,
        use_hatch: Vec::new(),
        hatch_name: None,
        hold_forward: None,
        kill: None,
        shot: None,
        lifepod_state: sn_sim::lighting::DAMAGED,
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
            "--start" => {
                args.start = vec3(&mut it, "--start")?;
                args.start_given = true;
            }
            "--lifepod-seed" => {
                let seed = it.next().and_then(|v| v.parse().ok());
                args.lifepod_seed = seed.ok_or("--lifepod-seed needs a whole number")?;
            }
            "--lifepod" => {
                let x = number(it.next(), "--lifepod")?;
                let z = number(it.next(), "--lifepod")?;
                args.lifepod = Some([x, z]);
            }
            "--look" => args.look = Some(vec3(&mut it, "--look")?),
            "--view" => args.view = number(it.next(), "--view")?,
            "--benchmark" => args.benchmark = Some(number(it.next(), "--benchmark")? as usize),
            "--flythrough" => args.flythrough = Some(vec3(&mut it, "--flythrough")?),
            "--speed" => args.speed = number(it.next(), "--speed")?,
            "--debug-colours" => args.debug_colours = true,
            "--no-objects" => args.no_objects = true,
            "--no-local-lights" => args.no_local_lights = true,
            "--slot-seed" => {
                let seed = it.next().and_then(|v| v.parse().ok());
                args.slot_seed = Some(seed.ok_or("--slot-seed needs a whole number")?);
            }
            "--no-slots" => args.slot_seed = None,
            "--no-placeholders" => args.no_placeholders = true,
            "--no-grass" => args.no_grass = true,
            "--no-animation" => args.no_animation = true,
            "--no-lod" => args.no_lod = true,
            "--fov" => args.fov = Some(number(it.next(), "--fov")?),
            "--no-scenes" => args.scenes = None,
            "--aurora" => {
                args.aurora_held = Some(match it.next().map(String::as_str) {
                    Some("intact") => false,
                    Some("exploded") => true,
                    _ => return Err("--aurora takes intact or exploded".into()),
                });
            }
            "--aurora-countdown" => {
                args.aurora_countdown = Some(number(it.next(), "--aurora-countdown")?);
            }
            "--time-scale" => args.time_scale = number(it.next(), "--time-scale")?,
            "--no-water-fog" => args.no_water_fog = true,
            "--no-water-surface" => args.no_water_surface = true,
            "--water-quality" => {
                args.water_quality = match it.next().map(String::as_str) {
                    Some("medium") => WaterQuality::Medium,
                    Some("high") => WaterQuality::High,
                    _ => return Err("--water-quality takes medium or high".into()),
                }
            }
            "--lifepod-state" => {
                args.lifepod_state = match it.next().map(String::as_str) {
                    Some("operational") => sn_sim::lighting::OPERATIONAL,
                    Some("danger") => sn_sim::lighting::DANGER,
                    Some("damaged") => sn_sim::lighting::DAMAGED,
                    _ => return Err("--lifepod-state takes operational, danger or damaged".into()),
                }
            }
            "--gpu-timings" => args.gpu_timings = true,
            "--free-cam" => args.free_cam = true,
            "--third-person" => args.third_person = true,
            "--look-down" => args.look_down = number(it.next(), "--look-down")?,
            "--use-hatch" => {
                let list = it.next().ok_or("--use-hatch takes seconds")?;
                args.use_hatch = list
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse::<f64>()
                            .map_err(|_| format!("--use-hatch: {t:?} is not a number"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--hatch-name" => {
                args.hatch_name = Some(it.next().ok_or("--hatch-name takes a name")?.clone());
            }
            "--hold-forward" => {
                args.hold_forward = Some(number(it.next(), "--hold-forward")?);
            }
            "--kill" => args.kill = Some(number(it.next(), "--kill")?),
            "--shot" => {
                let seconds = number(it.next(), "--shot")?;
                let name = it.next().ok_or("--shot takes <SECONDS> <NAME>")?;
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err("--shot: NAME takes letters, digits, - and _".into());
                }
                args.shot = Some((seconds, name.clone()));
            }
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
    if args.fov.is_some_and(|f| !(f > 1.0 && f < 179.0)) {
        return Err("--fov must be between 1 and 179 degrees".into());
    }
    if args.view < 100.0 {
        return Err("--view must be at least 100 metres".into());
    }
    if !(args.time_scale >= 0.0 && args.time_scale.is_finite()) {
        return Err("--time-scale must be 0 or more".into());
    }
    if args
        .aurora_countdown
        .is_some_and(|s| !(s >= 0.0 && s.is_finite()))
    {
        return Err("--aurora-countdown must be 0 or more seconds".into());
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
    caustics: LightTextures,
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
    let caustics = LightTextures {
        caustics: sn_assets::water_caustics(
            &assets,
            surface.surface.num_caustics_frames.max(0) as usize,
        )?,
        // Unity's default spot-light cookie (`docs/formats/lighting.md`).
        spot_cookie: sn_assets::builtin_texture(&assets, "Soft")?,
    };
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

/// Lifepod 5's start point (`RandomStart` with our seeded draw, or the
/// chosen x, z) and where the player starts in it.
fn lifepod_start(
    game_dir: Option<PathBuf>,
    seed: u64,
    chosen: Option<[f32; 2]>,
) -> Result<([f32; 3], sn_world::Transform), String> {
    let game = GameData::locate(game_dir).map_err(|e| e.to_string())?;
    let assets = sn_assets::Assets::index(&game)?;
    let point = match chosen {
        Some([x, z]) => [x, 0.0, z],
        None => {
            let map = assets.start_map()?;
            let (p, tries) = map.random_start(seed);
            println!(
                "lifepod: seed {seed}: start {p:?} after {tries} draws ({:.2} % of the start map valid)",
                map.valid_share() * 100.0
            );
            p
        }
    };
    let mut scene = assets.scene("escapepod")?;
    scene.spawn_lightmapped_prefab();
    let spawn = scene.place_escape_pod(&assets, point)?;
    Ok((point, spawn))
}

fn main() -> AppExit {
    let mut args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return AppExit::error();
        }
    };
    if !args.debug_colours
        && !args.no_objects
        && let Some(options) = args.scenes.as_mut()
    {
        match lifepod_start(args.game_dir.clone(), args.lifepod_seed, args.lifepod) {
            Ok((point, spawn)) => {
                options.lifepod = Some(point);
                options.lifepod_state = args.lifepod_state;
                if !args.start_given {
                    args.start = Vec3::from(spawn.position);
                    if args.look.is_none() {
                        // Unity's forward is the rotation's +z.
                        let ahead = spawn
                            .then(&sn_world::Transform {
                                position: [0.0, 0.0, 10.0],
                                ..Default::default()
                            })
                            .position;
                        args.look = Some(Vec3::from(ahead));
                    }
                }
            }
            Err(e) => eprintln!("warning: no lifepod: {e}"),
        }
    }
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
                        grass: Arc::new(if args.no_grass {
                            Vec::new()
                        } else {
                            materials.grass_types()
                        }),
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
    let play = !measuring && !args.free_cam;
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
        grass_look::GrassLookPlugin,
        WaterFogPlugin,
        WaterSurfacePlugin,
        SkyDomePlugin,
        GameLightPlugin,
        sun_shafts::SunShaftsPlugin,
        effects::EffectsPlugin,
    ))
    .insert_resource(PendingTerrainLook(look))
    .insert_resource(PendingLightTextures(caustics))
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
        fov: args
            .fov
            .unwrap_or_else(|| camera_fov(args.game_dir.clone())),
        fog_end: args.view,
        free_camera: !play,
    })
    .insert_resource(aurora::GameClock::at_hours(args.time, args.time_scale))
    .add_systems(Startup, setup)
    .add_systems(Update, aurora::tick_clock)
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
        let state = sky::state(manager, light, args.time);
        info!(
            "sky at {:.2} h (sky timeline {:.2} h): sun {:?} (light passes {:?}) from {:?}, top ambient {:?}, local light {:.3}",
            args.time,
            state.timeline,
            state.sun,
            state.sun_light,
            state.to_sun,
            state.top_ambient,
            state.local_light
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
                app.insert_resource(ObjectStreamer::start(
                    game,
                    !args.no_local_lights,
                    args.slot_seed,
                    !args.no_placeholders,
                    args.scenes,
                    play.then_some(BodyOptions {
                        head_visible: args.third_person,
                    }),
                    !args.no_animation,
                ));
                app.init_resource::<animation::AnimationStats>()
                    .add_systems(Update, animation::animate.after(objects::stream_objects));
                if !args.no_lod {
                    app.init_resource::<objects::LodStats>()
                        .add_systems(Update, objects::switch_lods.after(objects::stream_objects));
                }
                if args.scenes.is_some() {
                    let start = match args.aurora_held {
                        Some(exploded) => aurora::AuroraStart::Held { exploded },
                        None => aurora::AuroraStart::NewGame {
                            seed: args.lifepod_seed,
                            countdown: args.aurora_countdown,
                        },
                    };
                    // With the fly camera there is no player: in the pod
                    // when starting at its spawn point.
                    let free_cam_in_pod = !play && !args.start_given;
                    app.insert_resource(lifepod_light::FreeCamInPod(free_cam_in_pod))
                        .add_systems(
                            Update,
                            lifepod_light::update
                                .after(objects::stream_objects)
                                .after(player::update),
                        );
                    app.insert_resource(aurora::AuroraState::new(start))
                        .add_systems(
                            Update,
                            aurora::update
                                .after(objects::stream_objects)
                                .after(aurora::tick_clock),
                        );
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return AppExit::error();
            }
        }
    }
    if play {
        let lifepod = args
            .scenes
            .as_ref()
            .and_then(|o| o.lifepod)
            .filter(|_| !args.start_given);
        match GameData::locate(args.game_dir.clone()) {
            Ok(game) => {
                let start = player::Start {
                    lifepod,
                    position: args.start.to_array(),
                    slot_seed: args.slot_seed,
                    third_person: args.third_person,
                    look_down: f64::from(args.look_down),
                    use_hatch: args.use_hatch.clone(),
                    hatch_name: args.hatch_name.clone(),
                    hold_forward: args.hold_forward.map(f64::from),
                    kill: args.kill.map(f64::from),
                };
                app.insert_resource(player::PlayerSim::start(game, start))
                    .init_resource::<hud::Hud>()
                    .init_resource::<body::BodyDrive>()
                    .add_systems(
                        Update,
                        body::drive
                            .after(player::update)
                            .after(objects::stream_objects)
                            .before(animation::animate),
                    )
                    .add_systems(Startup, hud::setup)
                    .add_systems(Update, player::update.before(terrain::stream))
                    .add_systems(Update, hud::update.after(player::update));
                if let Some((seconds, name)) = args.shot.clone() {
                    app.insert_resource(PlayShot {
                        seconds: f64::from(seconds),
                        name,
                        taken: false,
                    })
                    .add_systems(Update, play_shot.after(player::update));
                }
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

/// The game's vertical field of view in degrees: `MiscSettings.fieldOfView`
/// from the player's DLL (M7f4f). If it cannot be read, Bevy's default
/// (45°, **not the game's**) with a warning.
fn camera_fov(game_dir: Option<PathBuf>) -> f32 {
    let read = GameData::locate(game_dir)
        .map_err(|e| e.to_string())
        .and_then(|game| sn_assets::read_assembly(&game))
        .and_then(|bytes| sn_assets::field_of_view_code(&bytes));
    match read {
        Ok(fov) => {
            info!("camera: field of view {fov}° (MiscSettings.fieldOfView)");
            fov
        }
        Err(e) => {
            warn!("camera: field of view not read ({e}); Bevy's 45°, not the game's");
            45.0
        }
    }
}

/// `--shot`: a screenshot after some seconds of play, then exit.
#[derive(Resource)]
struct PlayShot {
    seconds: f64,
    name: String,
    taken: bool,
}

fn play_shot(mut commands: Commands, mut shot: ResMut<PlayShot>, sim: Res<player::PlayerSim>) {
    if shot.taken || sim.play_seconds() < shot.seconds {
        return;
    }
    shot.taken = true;
    if let Err(e) = std::fs::create_dir_all("out") {
        error!("out/: {e}");
    }
    let path = format!("out/{}.png", shot.name);
    info!("shot: {path} after {:.1} s of play", sim.play_seconds());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path))
        .observe(
            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::Success);
            },
        );
}

#[derive(Resource)]
struct Setup {
    start: Vec3,
    look: Vec3,
    /// Vertical field of view, degrees (`MiscSettings.fieldOfView`).
    fov: f32,
    fog_end: f32,
    /// The fly camera (else the player moves the camera).
    free_camera: bool,
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
        Projection::Perspective(PerspectiveProjection {
            fov: settings.fov.to_radians(),
            ..default()
        }),
        Transform::from_translation(settings.start).looking_at(settings.look, Vec3::Y),
    ));
    // Measurements keep the camera where they put it (mouse or keyboard
    // input in the window would otherwise move it).
    if measuring.is_none() && settings.free_camera {
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
        // The camera's layer and the "shadows only" one: those renderers
        // cast the sun's shadow without being drawn (M9g4).
        bevy::camera::visibility::RenderLayers::from_layers(&[0, objects::SHADOW_ONLY_LAYER]),
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

/// The objects' lights, for `log_stats`.
#[derive(bevy::ecs::system::SystemParam)]
struct SpawnedLights<'w, 's> {
    point: Query<'w, 's, (), With<PointLight>>,
    spot: Query<'w, 's, (), With<SpotLight>>,
    directional: Query<'w, 's, &'static Transform, With<objects::GameDirectionalLight>>,
}

/// Logs frame rate, streaming state and memory every 2 seconds.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn log_stats(
    time: Res<Time>,
    mut last: Local<Duration>,
    diagnostics: Res<DiagnosticsStore>,
    streamer: Res<TerrainStreamer>,
    objects: Option<Res<ObjectStreamer>>,
    camera: Query<&Transform, With<Camera3d>>,
    lights: SpawnedLights,
    animation: Option<Res<animation::AnimationStats>>,
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
    info!(
        "grass: {} tufts, {} triangles",
        s.grass_tufts, s.grass_triangles
    );
    if let Some(objects) = objects {
        let o = objects.stats();
        info!(
            "objects: {} entities (cell levels 0..3, batch objects {:?}; {} objects from spawn slots; {} scene entities) in {} batches, {} queued | {} prefabs, {} meshes, {} materials, {} textures | {} warnings",
            o.entities,
            o.per_level,
            o.slot_objects,
            o.scene_entities,
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
        if let Some(a) = animation {
            info!(
                "animation: {} animators, {} updated, {} moved their Transforms, {} blend shape parts weighted in the last frame; {:.0} µs (worst {:.0} µs)",
                a.rigs, a.updated, a.applied, a.shaped, a.micros, a.worst_micros
            );
        }
        info!(
            "objects: {} point lights, {} spot lights, {} directional lights (directions {:?}) spawned",
            lights.point.iter().count(),
            lights.spot.iter().count(),
            lights.directional.iter().count(),
            lights
                .directional
                .iter()
                .take(4)
                .map(|t| t.forward().as_vec3())
                .collect::<Vec<_>>()
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
                        "measure: objects: {} entities (cell levels 0..3, batch objects {:?}; {} objects from spawn slots; {} scene entities), {} prefabs, {} meshes, {} materials, {} textures, {} warnings",
                        s.entities,
                        s.per_level,
                        s.slot_objects,
                        s.scene_entities,
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
    info!(
        "measure: grass {} tufts, {} triangles in {} batches at level of detail 0",
        stats.grass_tufts, stats.grass_triangles, stats.per_lod[0],
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
