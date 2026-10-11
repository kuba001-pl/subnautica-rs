//! `sn-server`: a headless multiplayer server (M10e). It reads the host's
//! own install for the game build, the rules fingerprint and a new world's
//! values, then listens on UDP. It never sends game data: only ids,
//! numbers, seeds and hashes (AGENTS.md rule 5).

use std::io::BufRead;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sn_install::GameData;
use sn_net::{Config, Endpoint, Transport, UdpTransport, random_seed};
use sn_protocol::{AuroraStart, MAX_PLAYERS, WorldSettings};
use sn_server::Host;
use sn_session::ServerConfig;

const USAGE: &str = "\
sn-server: a headless subnautica-rs multiplayer server (experimental)

Usage: sn-server [options]

  --game-dir     the Subnautica folder (default: the SUBNAUTICA_DIR variable)
  --bind         address to listen on (default 0.0.0.0)
  --port         UDP port (default 27420)
  --max-players  at most this many players (default 8, the most)
  --ask-password read a password from the console at start (not stored;
                 shown as typed)
  --lifepod-seed seed of the new world's draws: Lifepod 5's point and the
                 Aurora's countdown (default 1, as the client)
  --slot-seed    seed of the spawn slots' fill (default 1); --no-slots: none
  --time         the game clock at the start, hours (default 9.6)
  --time-scale   game seconds per real second (default 1)
  --aurora       intact | exploded: hold the Aurora so (default: a new
                 game's countdown)
  --aurora-countdown  <S>: the countdown S game seconds after the start
  --report       seconds between status reports (default 10)
  --run-for      stop after this many seconds (default: run until killed)
  --help         this text
";

struct Args {
    game_dir: Option<PathBuf>,
    bind: String,
    port: u16,
    max_players: usize,
    ask_password: bool,
    lifepod_seed: u64,
    slot_seed: Option<u64>,
    time: f64,
    time_scale: f32,
    aurora_held: Option<bool>,
    aurora_countdown: Option<f32>,
    report_s: f64,
    run_for: Option<f64>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        game_dir: None,
        bind: "0.0.0.0".into(),
        port: 27420,
        max_players: MAX_PLAYERS,
        ask_password: false,
        lifepod_seed: 1,
        slot_seed: Some(1),
        time: 9.6,
        time_scale: 1.0,
        aurora_held: None,
        aurora_countdown: None,
        report_s: 10.0,
        run_for: None,
    };
    let mut it = std::env::args().skip(1);
    fn value<T: std::str::FromStr>(v: Option<String>, flag: &str) -> Result<T, String> {
        v.and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("{flag} needs a value; see --help"))
    }
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game-dir" => a.game_dir = Some(value(it.next(), "--game-dir")?),
            "--bind" => a.bind = value(it.next(), "--bind")?,
            "--port" => a.port = value(it.next(), "--port")?,
            "--max-players" => a.max_players = value(it.next(), "--max-players")?,
            "--ask-password" => a.ask_password = true,
            "--lifepod-seed" => a.lifepod_seed = value(it.next(), "--lifepod-seed")?,
            "--slot-seed" => a.slot_seed = Some(value(it.next(), "--slot-seed")?),
            "--no-slots" => a.slot_seed = None,
            "--time" => a.time = value(it.next(), "--time")?,
            "--time-scale" => a.time_scale = value(it.next(), "--time-scale")?,
            "--aurora" => {
                a.aurora_held = Some(match it.next().as_deref() {
                    Some("intact") => false,
                    Some("exploded") => true,
                    _ => return Err("--aurora takes intact or exploded".into()),
                })
            }
            "--aurora-countdown" => {
                a.aurora_countdown = Some(value(it.next(), "--aurora-countdown")?)
            }
            "--report" => a.report_s = value(it.next(), "--report")?,
            "--run-for" => a.run_for = Some(value(it.next(), "--run-for")?),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown option {other}; see --help")),
        }
    }
    if a.max_players == 0 || a.max_players > MAX_PLAYERS {
        return Err(format!("--max-players must be 1 to {MAX_PLAYERS}"));
    }
    if a.time_scale < 0.0 || a.report_s <= 0.0 {
        return Err("--time-scale must be 0 or more and --report above 0".into());
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(if m == USAGE { 0 } else { 2 });
        }
    };
    if let Err(e) = run(args) {
        eprintln!("sn-server: {e}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), String> {
    let started = Instant::now();
    let game = GameData::locate(args.game_dir.clone()).map_err(|e| e.to_string())?;
    let assets = sn_assets::Assets::index(&game)?;
    let rules = sn_assets::rules(&game, &assets)?;
    println!(
        "game build {:?}; rules fingerprint {:016x} ({}); tick {} µs; implausible above {:.1} m/s",
        rules.build,
        rules.fingerprint,
        rules
            .parts
            .iter()
            .map(|(n, h)| format!("{n} {h:016x}"))
            .collect::<Vec<_>>()
            .join(", "),
        rules.tick_us,
        rules.max_speed
    );
    let (lifepod, tries) = sn_assets::lifepod_point(&assets, args.lifepod_seed)?;
    let aurora = match (args.aurora_held, args.aurora_countdown) {
        (Some(exploded), _) => AuroraStart::Held { exploded },
        (None, Some(seconds)) => AuroraStart::Countdown { seconds },
        (None, None) => AuroraStart::Countdown {
            seconds: sn_assets::aurora_countdown(&rules.exploder, args.lifepod_seed),
        },
    };
    // The game clock runs `day_seconds` per day (read from the code).
    let clock_start = args.time * f64::from(rules.exploder.day_seconds) / 24.0;
    let settings = WorldSettings {
        slot_seed: args.slot_seed,
        lifepod: Some(lifepod),
        aurora,
        clock_start,
        time_scale: args.time_scale,
    };
    println!(
        "new world: lifepod seed {} → Lifepod 5 at {lifepod:?} ({tries} draws); Aurora {aurora:?}; slot seed {:?}; clock {clock_start:.1} game s × {}",
        args.lifepod_seed, args.slot_seed, args.time_scale
    );
    let password = if args.ask_password {
        print!("password: ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| format!("reading the password: {e}"))?;
        Some(line.trim_end_matches(['\r', '\n']).to_owned())
    } else {
        None
    };
    let transport = UdpTransport::bind((args.bind.as_str(), args.port))
        .map_err(|e| format!("can't listen on {}:{}: {e}", args.bind, args.port))?;
    let addr = transport.local_addr().map_err(|e| e.to_string())?;
    let endpoint = Endpoint::new(transport, Config::server(args.max_players, random_seed()));
    let config = ServerConfig {
        build: rules.build.clone(),
        fingerprint: rules.fingerprint,
        password,
        max_players: args.max_players,
        settings,
        tick_us: rules.tick_us,
        max_speed: rules.max_speed,
        rules: rules.items.clone(),
    };
    let clock = Instant::now();
    let now = || clock.elapsed().as_micros() as u64;
    let mut host = Host::new(endpoint, config, now());
    println!(
        "listening on UDP {addr} (up to {} players{}); ready {:.1} s after start",
        args.max_players,
        if args.ask_password { ", password" } else { "" },
        started.elapsed().as_secs_f64()
    );
    host.report(now());
    let report_us = (args.report_s * 1e6) as u64;
    let mut next_report = now() + report_us;
    // Work per pass (wall time around `Host::step`), per report.
    let mut passes: Vec<u64> = Vec::new();
    loop {
        let t0 = Instant::now();
        let t = now();
        for line in host.step(t) {
            println!("[{:8.1} s] {line}", t as f64 / 1e6);
        }
        passes.push(t0.elapsed().as_micros() as u64);
        if t >= next_report {
            next_report += report_us;
            let n = passes.len();
            let mean = passes.iter().sum::<u64>() as f64 / n.max(1) as f64;
            let over = passes.iter().filter(|&&p| p > 1000).count();
            passes.sort_unstable();
            let q = |f: f64| {
                passes
                    .get(((n as f64 * f) as usize).min(n.saturating_sub(1)))
                    .copied()
                    .unwrap_or(0)
            };
            println!(
                "[{:8.1} s] loop: {n} passes; work per pass mean {mean:.1} µs, median {} µs, p99 {} µs, p99.9 {} µs, max {} µs; {over} passes over 1 ms",
                t as f64 / 1e6,
                q(0.5),
                q(0.99),
                q(0.999),
                passes.last().copied().unwrap_or(0)
            );
            for line in host.report(t) {
                println!("[{:8.1} s]   {line}", t as f64 / 1e6);
            }
            passes.clear();
        }
        if args.run_for.is_some_and(|s| t as f64 / 1e6 >= s) {
            println!("[{:8.1} s] --run-for reached; stopping", t as f64 / 1e6);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
