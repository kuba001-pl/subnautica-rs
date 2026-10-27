//! `sn-client`: the desktop client. It streams the world around the camera
//! from the player's install (terrain with levels of detail, objects, the
//! scenes) and puts you in it as the player (M9b), or lets you fly around
//! with `--free-cam`.

mod animation;
mod aurora;
mod baseline;
mod body;
mod effects;
mod game_light;
mod game_ui;
mod grass_look;
mod inventory;
mod items;
mod lifepod_light;
mod object_look;
mod objects;
mod perf;
mod player;
mod remote;
mod session;
mod sidedness;
mod sig;
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
use sn_protocol::{AuroraStart, WorldSettings};

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

  --save-dir     our server save slot (default: platform user data folder)
  --no-save      ephemeral solo/host session (use for scripted fresh runs)
  --load-backup  explicitly recover the slot's backup
  --autosave     seconds between solo/host saves (default 30)
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
  --log-sky      log sky samples at fixed session ticks (--log-ticks interval)
  --gpu-timings  with --benchmark/--flythrough: log GPU time per render pass
  --time         game clock in hours for the sun and sky (default 9.6, i.e.
                 09:36, when a new game starts); the clock runs from there
                 (sun and sky follow the shared session clock)
  --time-scale   game seconds per real second (default 1, as the game)
  --view         view distance in metres (default 1200); level-of-detail
                 ranges scale with it
  --benchmark    once the start area is loaded, render FRAMES frames without
                 vsync, log frame times, save out/client-benchmark.png, exit
  --baseline     NAME: with --benchmark, save report.txt, frames.csv and
                 capture.png in out/baselines/run-NAME (must be a new name)
  --baseline-revision  LABEL: source revision/build label, required for --baseline
                 (names/labels: 1–96 letters, digits, - or _)
  --resolution   WIDTH HEIGHT: physical pixels (default 1600 900)
  --warmup       FRAMES: consecutive settled frames before measurement
                 (default 120 for --baseline, 0 otherwise; 0 skips warm-up)
  --free-cam     a fly camera instead of the player (also with --benchmark
                 and --flythrough)
  --third-person a debug camera 2.5 m behind the player's eye that shows
                 the head (the player's body is drawn in either view)
  --look-down    the player's starting pitch, degrees (negative looks up;
                 clamped to the game's limits)
  --no-game-ui   don't draw the game's own HUD (drawn by default from its
                 UI files: the four bars, the depth and compass, the hand
                 reticle, the death and respawn screens and the
                 out-of-oxygen overlay driven by the game's rules, M9u6;
                 other screens in their start state); then no HUD is
                 drawn. --game-ui (the default) is still accepted
  --shot         --shot <SECONDS> <NAME>: after SECONDS of play, save
                 out/NAME.png and exit (a check for the human)
  --use-item     --use-item <SECONDS[,SECONDS…]>: at each time, turn to the
                 nearest pickup, outcrop or the lifepod's locker and use
                 the hand on it, as a click (a debug check, M9d5)
  --panel        --panel <SECONDS=STEP[,SECONDS=STEP…]>: inventory panel
                 steps at those times (open, close, take-all: a left click
                 on every locker item, equip: a left click on the first
                 grid item that equips, drop: a right click on the first
                 grid item, unequip: a left click on the first slot item,
                 tabN: a left click on the PDA toolbar's icon N, 0–5;
                 hoverN: the pointer rests on grid item N's icon,
                 hover-empty: on the grid's last cell, until the next
                 hover step or the close; a debug check, M9d5b, M9u8a4b,
                 M9u8b3b3)
  --use-hatch    --use-hatch <SECONDS[,SECONDS…]>: at each time (seconds of
                 play), aim at the nearest usable lifepod hatch and use it
                 (a debug check of the hatch cinematics, M9g5e)
  --hatch-name   --hatch-name <TEXT>: --use-hatch picks only hatches whose
                 trigger name contains TEXT (e.g. bot_out)
  --hold-forward --hold-forward <SECONDS>: from then on, hold W (a debug
                 check that the player moves)
  --hold-back    --hold-back <SECONDS>: from then on, hold S (M9u8a3: walk
                 away from the locker with the PDA open)
  --craft        --craft SECONDS=TECH[,SECONDS=TECH…]; TECH=take collects
                 finished output (debug hook, server checks the request)
  --test-supply  --test-supply <TECH>[,<TECH>…]: a debug aid for a solo or
                 host run: one item of each tech type (by its name in the
                 game's code, e.g. Compass) put into the new world's
                 lifepod locker, logged; the item's pickup joins the
                 rules, so a joiner passes the same flag
  --turn         --turn <DEG_PER_S>: the player's look turns right at this
                 rate (a debug check of the compass)
  --kill         --kill <SECONDS>: after SECONDS of play the player takes
                 its full health as damage (a debug check of the death)
  --pick-up      --pick-up <SECONDS>: after SECONDS of play, ask to pick up
                 the nearest pickups within 3 m (a debug check, M9d4b)
  --drop         --drop <SECONDS>: after SECONDS of play, ask to drop the
                 first item held where the game drops it (a debug check,
                 M9d4b)
  --analyze      --analyze <TECHTYPE>[,…]: once the HUD is active, show the
                 unlock pop-up of each tech type's PDAData.analysisTech
                 entry, as a verbose KnownTech.Add would (a test hook,
                 M9u7d2: crafting and scanning are not ported)
  --host         --host [PORT]: open this game to other players on UDP PORT
                 (default 27420; experimental, M10h: no encryption, for
                 friends on trust)
  --join         --join <ADDRESS[:PORT]>: join the game hosted there (its
                 world settings replace the flags'; the same game build
                 and rules are needed)
  --name         --name <NAME>: the name other players see (default: the
                 USERNAME or USER variable, else player)
  --net-sim      lan | internet | bad: delay, jitter and loss on this
                 client's own link (testing)
  --log-ticks    --log-ticks <N>: log the own state and each other
                 player's drawn state every N ticks (default 50; testing)
  --flythrough   once loaded, fly in a straight line to X Y Z (Unity world
                 coordinates) at --speed (default 40 m/s) without vsync, then
                 log frame times, memory and streaming latency, save
                 out/client-flythrough.png and exit

Controls (player): click to capture the mouse, Esc to release it; mouse to
look, WASD to move, Space up / jump, C down, E or left click to use (the
lifepod's hatches, items, the locker); Tab opens the inventory (left click
on an item: its action, right click: drop; Tab or Esc closes it).
Controls (--free-cam): right mouse (hold) or M (toggle) to look around, WASD
to move, Q/E down/up, Shift to go fast, mouse wheel to change speed.
";

struct Args {
    game_dir: Option<PathBuf>,
    save_dir: Option<PathBuf>,
    no_save: bool,
    load_backup: bool,
    autosave_s: f64,
    start: Vec3,
    /// `--start` given (else the player's spawn in the lifepod).
    start_given: bool,
    lifepod_seed: u64,
    /// `--lifepod X Z`.
    lifepod: Option<[f32; 2]>,
    look: Option<Vec3>,
    view: f32,
    benchmark: Option<usize>,
    baseline: Option<String>,
    baseline_revision: Option<String>,
    resolution: [u32; 2],
    resolution_given: bool,
    warmup: Option<usize>,
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
    /// `--game-ui`: the game's UI drawn (M9u5c).
    game_ui: bool,
    /// Game clock, hours.
    time: f32,
    /// `--time-scale`: the clock's speed.
    time_scale: f32,
    log_sky: bool,
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
    /// `--use-item` (M9d5).
    use_item: Vec<f64>,
    /// `--panel` (M9d5b): seconds of play and the step.
    panel: Vec<(f64, inventory::Scripted)>,
    /// `--hatch-name`: only hatches whose name contains this.
    hatch_name: Option<String>,
    /// `--hold-forward`: hold W from this much play on.
    hold_forward: Option<f32>,
    /// `--hold-back` (M9u8a3): hold S from this much play on.
    hold_back: Option<f32>,
    /// `--test-supply`: tech type names put into a new world's locker.
    test_supply: Vec<String>,
    craft: Vec<(f64, String)>,
    /// `--turn`: the player's look turns at this many degrees a second.
    turn: Option<f32>,
    /// `--kill`: seconds of play before the player dies.
    kill: Option<f32>,
    /// `--pick-up`, `--drop`: seconds of play (M9d4b).
    pick_up: Option<f32>,
    drop: Option<f32>,
    /// `--analyze`: tech type names (M9u7d2).
    analyze: Vec<String>,
    /// `--shot`: seconds of play, then `out/<name>.png`.
    shot: Option<(f32, String)>,
    /// `--lifepod-state`: the pod's `LightingController` state.
    lifepod_state: usize,
    /// Solo, `--host` or `--join`.
    mode: session::Mode,
    /// `--name`.
    name: String,
    /// `--net-sim`.
    net_sim: Option<sn_net::sim::Impairment>,
    /// `--log-ticks`.
    log_ticks: u32,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn parse_args_from(raw: &[String]) -> Result<Args, String> {
    let mut args = Args {
        game_dir: None,
        save_dir: None,
        no_save: false,
        load_backup: false,
        autosave_s: 30.0,
        start: Vec3::new(0.0, -10.0, 0.0),
        start_given: false,
        lifepod_seed: 1,
        lifepod: None,
        look: None,
        view: 1200.0,
        benchmark: None,
        baseline: None,
        baseline_revision: None,
        resolution: [1600, 900],
        resolution_given: false,
        warmup: None,
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
        game_ui: true,
        time: sky::NEW_GAME_HOURS,
        time_scale: 1.0,
        log_sky: false,
        aurora_held: None,
        aurora_countdown: None,
        free_cam: false,
        third_person: false,
        look_down: 0.0,
        use_hatch: Vec::new(),
        use_item: Vec::new(),
        panel: Vec::new(),
        hatch_name: None,
        hold_forward: None,
        hold_back: None,
        test_supply: Vec::new(),
        craft: Vec::new(),
        turn: None,
        kill: None,
        pick_up: None,
        drop: None,
        analyze: Vec::new(),
        shot: None,
        lifepod_state: sn_sim::lighting::DAMAGED,
        mode: session::Mode::Solo,
        name: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "player".into()),
        net_sim: None,
        log_ticks: 50,
    };
    let mut it = raw.iter().peekable();
    let number = |s: Option<&String>, what: &str| -> Result<f32, String> {
        s.ok_or(format!("{what} needs a value"))?
            .parse()
            .map_err(|_| format!("{what} needs a number"))
    };
    let vec3 = |it: &mut std::iter::Peekable<std::slice::Iter<String>>,
                what: &str|
     -> Result<Vec3, String> {
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
            "--benchmark" => {
                args.benchmark = Some(baseline::positive_integer(it.next(), "--benchmark")?);
            }
            "--baseline" | "--baseline-revision" => {
                let value = it
                    .next()
                    .filter(|v| baseline::valid_label(v))
                    .ok_or_else(|| format!("{arg} needs 1–96 letters, digits, - or _"))?;
                if arg == "--baseline" {
                    args.baseline = Some(value.clone());
                } else {
                    args.baseline_revision = Some(value.clone());
                }
            }
            "--resolution" => {
                args.resolution_given = true;
                for dim in &mut args.resolution {
                    *dim = it
                        .next()
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|&n| (64..=8192).contains(&n))
                        .ok_or("--resolution needs WIDTH HEIGHT, each 64–8192 physical pixels")?;
                }
            }
            "--warmup" => {
                args.warmup = Some(
                    it.next()
                        .and_then(|v| v.parse::<usize>().ok())
                        .ok_or("--warmup needs a whole number, 0 or more")?,
                );
            }
            "--flythrough" => args.flythrough = Some(vec3(&mut it, "--flythrough")?),
            "--speed" => args.speed = number(it.next(), "--speed")?,
            "--debug-colours" => args.debug_colours = true,
            "--no-objects" => args.no_objects = true,
            "--no-local-lights" => args.no_local_lights = true,
            "--slot-seed" => {
                let seed = it.next().and_then(|v| v.parse().ok());
                args.slot_seed = Some(seed.ok_or("--slot-seed needs a whole number")?);
            }
            "--save-dir" => {
                args.save_dir = Some(PathBuf::from(
                    it.next().ok_or("--save-dir needs a directory")?,
                ))
            }
            "--no-save" => args.no_save = true,
            "--load-backup" => args.load_backup = true,
            "--autosave" => args.autosave_s = f64::from(number(it.next(), "--autosave")?),
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
            "--log-sky" => args.log_sky = true,
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
            // The default since M9u6e2; still accepted.
            "--game-ui" => args.game_ui = true,
            "--no-game-ui" => args.game_ui = false,
            "--free-cam" => args.free_cam = true,
            "--third-person" => args.third_person = true,
            "--look-down" => args.look_down = number(it.next(), "--look-down")?,
            "--use-item" => {
                let list = it.next().ok_or("--use-item takes seconds")?;
                args.use_item = list
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse::<f64>()
                            .map_err(|_| format!("--use-item: {t:?} is not a number"))
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--panel" => {
                let list = it.next().ok_or("--panel takes SECONDS=STEP,…")?;
                for entry in list.split(',') {
                    let (t, step) = entry
                        .split_once('=')
                        .ok_or_else(|| format!("--panel: {entry:?} is not SECONDS=STEP"))?;
                    let t = t
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| format!("--panel: {t:?} is not a number"))?;
                    let step = inventory::Scripted::parse(step.trim())
                        .ok_or_else(|| format!("--panel: unknown step {step:?}"))?;
                    args.panel.push((t, step));
                }
            }
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
            "--craft" => {
                let list = it.next().ok_or(
                    "--craft takes SECONDS=TECH[,SECONDS=TECH]; TECH=take collects the result",
                )?;
                for entry in list.split(',') {
                    let (at, name) = entry
                        .split_once('=')
                        .ok_or("--craft requires SECONDS=TECH")?;
                    let at: f64 = at.parse().map_err(|_| "invalid --craft time")?;
                    if !at.is_finite() || at < 0.0 {
                        return Err("invalid --craft time".into());
                    }
                    args.craft.push((at, name.to_owned()));
                }
                args.craft.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
            "--test-supply" => {
                let list = it.next().ok_or("--test-supply takes TECH[,TECH…]")?;
                args.test_supply = list.split(',').map(str::to_owned).collect();
            }
            "--turn" => args.turn = Some(number(it.next(), "--turn")?),
            "--hold-forward" => {
                args.hold_forward = Some(number(it.next(), "--hold-forward")?);
            }
            "--hold-back" => args.hold_back = Some(number(it.next(), "--hold-back")?),
            "--kill" => args.kill = Some(number(it.next(), "--kill")?),
            "--pick-up" => args.pick_up = Some(number(it.next(), "--pick-up")?),
            "--drop" => args.drop = Some(number(it.next(), "--drop")?),
            "--analyze" => {
                let v = it.next().ok_or("--analyze needs tech type names")?;
                args.analyze = v.split(',').map(|t| t.trim().to_string()).collect();
            }
            "--host" => {
                let port = match it.peek().and_then(|v| v.parse::<u16>().ok()) {
                    Some(p) => {
                        it.next();
                        p
                    }
                    None => DEFAULT_PORT,
                };
                args.mode = session::Mode::Host { port };
            }
            "--join" => {
                let a = it.next().ok_or("--join takes ADDRESS[:PORT]")?;
                let with_port = if a.contains(':') {
                    a.clone()
                } else {
                    format!("{a}:{DEFAULT_PORT}")
                };
                let addr = std::net::ToSocketAddrs::to_socket_addrs(&with_port)
                    .map_err(|e| format!("--join {a}: {e}"))?
                    .find(|a| a.is_ipv4())
                    .ok_or(format!("--join {a}: no IPv4 address"))?;
                args.mode = session::Mode::Join { addr };
            }
            "--name" => args.name = it.next().ok_or("--name takes a name")?.clone(),
            "--log-ticks" => {
                args.log_ticks = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--log-ticks takes a whole number above 0")?;
            }
            "--net-sim" => {
                let p = it.next().ok_or("--net-sim takes lan, internet or bad")?;
                args.net_sim = Some(
                    sn_net::sim::Impairment::preset(p)
                        .ok_or("--net-sim takes lan, internet or bad")?,
                );
            }
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
    if args.benchmark.is_some() && args.flythrough.is_some() {
        return Err("choose --benchmark or --flythrough".into());
    }
    if args.warmup.is_some() && args.benchmark.is_none() && args.flythrough.is_none() {
        return Err("--warmup requires --benchmark or --flythrough".into());
    }
    if args.baseline.is_some() {
        if args.benchmark.is_none() || !matches!(args.mode, session::Mode::Solo) {
            return Err("--baseline requires a solo --benchmark run".into());
        }
        if args.baseline_revision.is_none() {
            return Err("--baseline requires --baseline-revision LABEL".into());
        }
        if !args.start.is_finite()
            || args.look.is_some_and(|v| !v.is_finite())
            || !args.view.is_finite()
            || !args.time.is_finite()
            || !args.fog_unit.is_finite()
            || args
                .lifepod
                .is_some_and(|p| p.iter().any(|v| !v.is_finite()))
        {
            return Err(
                "baseline camera, view, time, fog and lifepod values must be finite".into(),
            );
        }
    } else if args.baseline_revision.is_some() {
        return Err("--baseline-revision requires --baseline".into());
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

/// The UDP port of `--host` and `--join` without one (`sn-server`'s too).
const DEFAULT_PORT: u16 = 27420;

/// The session at start (M10g, M10h): solo or `--host`, the in-process
/// server for a new world as the flags describe it (Lifepod 5's point from
/// `RandomStart` with our seeded draw or the chosen x, z; the Aurora; the
/// slot seed; the clock), joined over the direct link; `--join`, the
/// host's. Returns it with Lifepod 5's point and the player's place in
/// it, both from the `Welcome`'s settings.
fn start_session(args: &Args, lifepod: bool) -> Result<Started, String> {
    let started = std::time::Instant::now();
    let game = GameData::locate(args.game_dir.clone()).map_err(|e| e.to_string())?;
    let assets = sn_assets::Assets::index(&game)?;
    // The item text (M9d5): English, the only language read for now.
    let text = match sn_assets::read_assembly(&game)
        .and_then(|dll| sn_assets::item_text(&game, &dll, "English"))
    {
        Ok(t) => {
            println!(
                "item text: {} tech type names, {} English strings",
                t.tech_names.len(),
                t.strings.len()
            );
            t
        }
        Err(e) => {
            eprintln!("warning: item text not read ({e}); names show as keys");
            sn_assets::ItemText::default()
        }
    };
    // `--test-supply`: tech types by their names in the game's code; their
    // pickups join the rules (a joiner passes the same flag so the rules
    // match; only a solo or host world gets the items).
    let mut test_supplies = Vec::new();
    for name in &args.test_supply {
        let tech = text
            .tech_names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(v, _)| *v)
            .ok_or_else(|| format!("--test-supply: no tech type {name:?}"))?;
        test_supplies.push(tech);
    }
    let rules = sn_assets::rules_with(&game, &assets, &test_supplies)?;
    let joining = matches!(args.mode, session::Mode::Join { .. });
    let point = match (lifepod && !joining, args.lifepod) {
        (false, _) => None,
        (true, Some([x, z])) => Some([x, 0.0, z]),
        (true, None) => {
            let map = assets.start_map()?;
            let (p, tries) = map.random_start(args.lifepod_seed);
            println!(
                "lifepod: seed {}: start {p:?} after {tries} draws ({:.2} % of the start map valid)",
                args.lifepod_seed,
                map.valid_share() * 100.0
            );
            Some(p)
        }
    };
    let aurora = match (args.aurora_held, args.aurora_countdown) {
        (Some(exploded), _) => AuroraStart::Held { exploded },
        (None, Some(seconds)) => AuroraStart::Countdown { seconds },
        (None, None) => AuroraStart::Countdown {
            seconds: sn_assets::aurora_countdown(&rules.exploder, args.lifepod_seed),
        },
    };
    let day = rules.exploder.day_seconds;
    let config = sn_session::ServerConfig {
        build: rules.build.clone(),
        fingerprint: rules.fingerprint,
        password: None,
        max_players: match args.mode {
            session::Mode::Solo => 1,
            _ => sn_protocol::MAX_PLAYERS,
        },
        settings: WorldSettings {
            slot_seed: args.slot_seed,
            lifepod: point,
            aurora,
            clock_start: f64::from(args.time) * f64::from(day) / 24.0,
            time_scale: args.time_scale,
        },
        tick_us: rules.tick_us,
        max_speed: rules.max_speed,
        rules: rules.items.clone(),
    };
    if !args.autosave_s.is_finite()
        || args.autosave_s <= 0.0
        || (args.no_save && (args.save_dir.is_some() || args.load_backup))
    {
        return Err(
            "--autosave must be finite and positive; --no-save conflicts with save options".into(),
        );
    }
    let save = if joining || args.no_save {
        None
    } else {
        let dir = args
            .save_dir
            .clone()
            .map_or_else(sn_server::saves::default_dir, Ok)?;
        Some((
            sn_server::saves::SaveStore::open(
                dir,
                game.data_dir.parent().ok_or("game root missing")?,
            )?,
            args.load_backup,
            (args.autosave_s * 1e6) as u64,
        ))
    };
    let mut session = session::Session::start(
        config,
        &args.mode,
        &args.name,
        args.net_sim,
        if joining { &[] } else { &test_supplies },
        save,
    )?;
    session.log_every = args.log_ticks;
    let pod = match session.settings().lifepod {
        Some(point) => {
            let mut scene = assets.scene("escapepod")?;
            scene.spawn_lightmapped_prefab();
            Some((point, scene.place_escape_pod(&assets, point)?))
        }
        None => None,
    };
    println!(
        "session: {:?} as {:?} (build {:?}, rules {:016x}, tick {} µs) in {:.2} s",
        args.mode,
        args.name,
        rules.build,
        rules.fingerprint,
        rules.tick_us,
        started.elapsed().as_secs_f64()
    );
    Ok(Started {
        session,
        pod,
        day_seconds: day,
        items: rules.items.clone(),
        text: std::sync::Arc::new(text),
        build: rules.build,
        fingerprint: rules.fingerprint,
    })
}

/// What [`start_session`] gives.
struct Started {
    session: session::Session,
    /// Lifepod 5's point and the player's spawn in it.
    pod: Option<([f32; 3], sn_world::Transform)>,
    /// Game seconds per day (`DayNightCycle`, read from the code).
    day_seconds: f32,
    /// The install's item rules (M9d3d).
    items: std::sync::Arc<sn_sim::inventory::ItemRules>,
    /// Item names and the hand's text (M9d5).
    text: std::sync::Arc<sn_assets::ItemText>,
    build: String,
    fingerprint: u64,
}

fn main() -> AppExit {
    let cpu_timings = perf::init();
    let mut args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return AppExit::error();
        }
    };
    let lifepod = !args.debug_colours && !args.no_objects && args.scenes.is_some();
    let mut baseline = match (&args.baseline, &args.baseline_revision) {
        (Some(name), Some(revision)) => match baseline::Baseline::create(name, revision) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("error: {e}");
                return AppExit::error();
            }
        },
        _ => None,
    };
    let Started {
        session,
        pod,
        day_seconds,
        items,
        text,
        build,
        fingerprint,
    } = match start_session(&args, lifepod) {
        Ok(s) => s,
        Err(message) => {
            eprintln!("error: session: {message}");
            return AppExit::error();
        }
    };
    // The world as the server's settings give it (equal to the flags in
    // solo play).
    let settings = session.settings();
    args.slot_seed = settings.slot_seed;
    args.time = (settings.clock_start * 24.0 / f64::from(day_seconds)) as f32;
    args.time_scale = settings.time_scale;
    if lifepod && pod.is_none() {
        eprintln!("warning: no lifepod");
    }
    if let (Some(options), Some((point, spawn))) = (args.scenes.as_mut(), pod) {
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
    let ranges = lod_ranges(args.view);
    if let Some(b) = &mut baseline {
        b.configure(&args, &build, fingerprint, &settings);
    }
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
    let setup_fov = args
        .fov
        .unwrap_or_else(|| camera_fov(args.game_dir.clone()));
    let mut app = App::new();
    let mut resolution =
        bevy::window::WindowResolution::new(args.resolution[0], args.resolution[1]);
    if args.resolution_given || baseline.is_some() {
        resolution = resolution.with_scale_factor_override(1.0);
    }
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "subnautica-rs".into(),
            resolution,
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
        fov: setup_fov,
        fog_end: args.view,
        free_camera: !play,
    })
    .insert_resource(aurora::GameClock::new(
        settings.clock_start,
        settings.time_scale,
    ))
    .insert_resource(session)
    .add_systems(First, session::update)
    .add_systems(Update, session::log_remotes.after(player::update))
    .add_systems(Startup, setup)
    .add_systems(Update, aurora::tick_clock)
    .add_systems(
        Update,
        (
            terrain::stream,
            objects::stream_objects
                .after(terrain::stream)
                .run_if(resource_exists::<ObjectStreamer>),
            objects::update_atmosphere_skies
                .after(objects::stream_objects)
                .run_if(resource_exists::<ObjectStreamer>),
            log_stats,
        ),
    );
    let game_ui = args.game_ui;
    if game_ui {
        app.add_plugins(game_ui::GameUiPlugin {
            game_dir: args.game_dir.clone(),
            field_of_view: setup_fov,
            analyze: args.analyze.clone(),
        });
    }
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
        app.insert_resource(state)
            .insert_resource(sky::SkyCycle {
                manager: manager.clone(),
                light: light.clone(),
                day_seconds: f64::from(day_seconds),
            })
            .add_systems(
                Update,
                sky::update_cycle
                    .in_set(sky::SkyUpdated)
                    .after(aurora::tick_clock),
            );
        if args.log_sky {
            app.init_resource::<sky::ClockProbe>()
                .add_systems(Update, sky::log_clock.after(sky::SkyUpdated));
        }
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
                    Some(items.clone()),
                ));
                app.init_resource::<animation::AnimationStats>()
                    .add_systems(Update, animation::animate.after(objects::stream_objects));
                if !args.no_lod {
                    app.init_resource::<objects::LodStats>()
                        .add_systems(Update, objects::switch_lods.after(objects::stream_objects));
                }
                if args.scenes.is_some() {
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
                    app.insert_resource(aurora::AuroraState::new(settings.aurora))
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
        let lifepod = args.scenes.as_ref().and_then(|o| o.lifepod);
        match GameData::locate(args.game_dir.clone()) {
            Ok(game) => {
                let start = player::Start {
                    lifepod,
                    in_pod: !args.start_given,
                    position: args.start.to_array(),
                    slot_seed: args.slot_seed,
                    third_person: args.third_person,
                    look_down: f64::from(args.look_down),
                    use_hatch: args.use_hatch.clone(),
                    use_item: args.use_item.clone(),
                    hatch_name: args.hatch_name.clone(),
                    hold_forward: args.hold_forward.map(f64::from),
                    hold_back: args.hold_back.map(f64::from),
                    turn: args.turn.map(f64::from),
                    kill: args.kill.map(f64::from),
                    items: Some(items.clone()),
                };
                app.insert_resource(player::PlayerSim::start(game, start))
                    .init_resource::<player::Hud>()
                    .init_resource::<body::BodyDrive>()
                    .add_systems(
                        Update,
                        body::drive
                            .after(player::update)
                            .after(objects::stream_objects)
                            .before(animation::animate),
                    )
                    .add_systems(
                        Update,
                        remote::update
                            .after(session::log_remotes)
                            .before(body::drive)
                            .after(objects::stream_objects)
                            .before(animation::animate)
                            .run_if(resource_exists::<ObjectStreamer>),
                    )
                    .insert_resource(inventory::Panel::new(
                        args.panel.clone(),
                        sn_net::random_seed(),
                    ))
                    .add_systems(Update, inventory::input.before(player::update))
                    .add_systems(Update, player::update.before(terrain::stream))
                    // The death vignette's animation events (M9u6e1).
                    .add_systems(
                        Update,
                        body::death_events
                            .after(animation::animate)
                            .run_if(move || game_ui),
                    )
                    // The arms' states with the PDA (M9u8a3).
                    .add_systems(Update, body::arm_states.after(animation::animate))
                    .insert_resource(items::Text(text.clone()))
                    .insert_resource(items::ItemsView::new(
                        args.pick_up.map(f64::from),
                        args.drop.map(f64::from),
                        args.craft.clone(),
                    ))
                    .add_systems(
                        Update,
                        items::sync
                            .after(player::update)
                            .before(objects::stream_objects),
                    );
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
        app.insert_resource(Measurement::new(
            Mode::Benchmark { frames },
            args.warmup
                .unwrap_or(if baseline.is_some() { 120 } else { 0 }),
        ))
        .add_systems(Last, measure);
    } else if let Some(to) = args.flythrough {
        let from = unity_to_bevy(args.start);
        app.insert_resource(Measurement::new(
            Mode::Flythrough {
                from,
                to: unity_to_bevy(to),
                speed: args.speed,
            },
            args.warmup.unwrap_or(0),
        ))
        .add_systems(Last, measure);
    }
    if let Some(baseline) = baseline {
        app.insert_resource(baseline);
    }
    if cpu_timings {
        perf::install(&mut app);
        app.add_systems(Last, perf::frame.before(measure));
    }
    let result = app.run();
    perf::report();
    result
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

#[cfg(test)]
mod baseline_arg_tests {
    use super::*;

    fn parse(flags: &str) -> Result<Args, String> {
        parse_args_from(
            &flags
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn baseline_requires_solo_benchmark_and_revision() {
        for flags in [
            "--baseline test",
            "--baseline test --benchmark 10",
            "--baseline-revision test",
            "--warmup 10",
            "--baseline test --baseline-revision test --benchmark 10 --host",
            "--benchmark 10 --flythrough 0 1 2",
            "--benchmark NaN",
            "--resolution 0 900",
            "--baseline ../escape",
            "--baseline test --baseline-revision test --benchmark 10 --start NaN 0 0",
        ] {
            assert!(parse(flags).is_err(), "{flags}");
        }
        let args = parse("--baseline shallows-1 --baseline-revision abc-dirty --benchmark 300 --warmup 0 --resolution 1280 720").unwrap();
        assert_eq!(args.resolution, [1280, 720]);
        assert_eq!(args.benchmark, Some(300));
        assert_eq!(args.warmup, Some(0));
        assert!(parse("").unwrap().baseline.is_none());
    }
}

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
    perf::capture_started();
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
        Some(s) => sky::sun_light(s),
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
        sky::GameSun,
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
        info!(
            "objects: SIG materials: {} plain, {} capped, {} with SIG keyword; {} missing albedo slots",
            o.sig[0], o.sig[1], o.sig[2], o.sig[3]
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
    Warming,
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
    warmup: baseline::Warmup,
    loaded_at: Duration,
    running_at: Duration,
    start_snapshot: String,
    unsettled_frames: usize,
}

impl Measurement {
    fn new(mode: Mode, warmup: usize) -> Self {
        Measurement {
            mode,
            phase: Phase::Loading,
            frame_ms: Vec::new(),
            travelled: 0.0,
            max_triangles: 0,
            max_cached: 0,
            max_memory_gib: 0.0,
            phase_started: Duration::ZERO,
            warmup: baseline::Warmup::new(warmup),
            loaded_at: Duration::ZERO,
            running_at: Duration::ZERO,
            start_snapshot: String::new(),
            unsettled_frames: 0,
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

#[allow(clippy::too_many_arguments)] // Bevy system resources and report snapshot
fn measure(
    mut commands: Commands,
    time: Res<Time<Real>>,
    diagnostics: Res<DiagnosticsStore>,
    mut streamer: ResMut<TerrainStreamer>,
    objects: Option<Res<ObjectStreamer>>,
    mut m: ResMut<Measurement>,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<objects::GameDirectionalLight>)>,
    baseline: Option<Res<baseline::Baseline>>,
    snapshot: baseline::Snapshot,
    mut exit: MessageWriter<AppExit>,
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
                m.loaded_at = now;
                m.phase = if m.warmup.target == 0 {
                    Phase::Running
                } else {
                    Phase::Warming
                };
            }
        }
        Phase::Warming => {
            if m.warmup.ready(settled) {
                info!(
                    "measure: {} consecutive settled warm-up frames, {:.2} s since first settled",
                    m.warmup.target,
                    (now - m.loaded_at).as_secs_f64()
                );
                m.phase = Phase::Running;
            }
        }
        Phase::Running => {
            m.frame_ms.push(time.delta_secs() * 1000.0);
            m.unsettled_frames += usize::from(!settled);
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
    if m.phase == Phase::Running && m.running_at == Duration::ZERO {
        m.running_at = now;
        m.phase_started = now;
        streamer.latencies.clear();
        if baseline.is_some() {
            m.start_snapshot = snapshot.describe(&camera, stats, objects.as_deref());
        }
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
    if let Some(summary) = baseline::frame_summary(&m.frame_ms) {
        info!(
            "measure: real frame median {:.3} ms, p99 {:.3} ms; {} unsettled measurement frames",
            summary.median, summary.p99, m.unsettled_frames
        );
    }
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
    let name = if let Some(baseline) = baseline {
        let report = format!(
            "loading_seconds_since_app_start={:.6}\nwarmup_seconds={:.6}\n\
             measurement_seconds={:.6}\nmeasured_frames={}\nunsettled_measurement_frames={}\n\
             frame_ms_summary={:?}\npeak_process_memory_gib={:.6}\n\
             --- measurement_start ---\n{}--- measurement_end ---\n{}{}",
            m.loaded_at.as_secs_f64(),
            (m.running_at - m.loaded_at).as_secs_f64(),
            (now - m.running_at).as_secs_f64(),
            m.frame_ms.len(),
            m.unsettled_frames,
            baseline::frame_summary(&m.frame_ms),
            m.max_memory_gib,
            m.start_snapshot,
            snapshot.describe(&camera, stats, objects.as_deref()),
            baseline::gpu_report(&diagnostics),
        );
        if let Err(e) = baseline.save(&m.frame_ms, &report) {
            error!("baseline: {e}");
            exit.write(AppExit::error());
            return;
        }
        info!(
            "baseline: saved report and {} real frame samples to {}",
            m.frame_ms.len(),
            baseline.directory.display()
        );
        baseline.directory.join("capture.png")
    } else {
        PathBuf::from(match m.mode {
            Mode::Benchmark { .. } => "out/client-benchmark.png",
            Mode::Flythrough { .. } => "out/client-flythrough.png",
        })
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
