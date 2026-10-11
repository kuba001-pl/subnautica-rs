//! `sn-inspect bots`: N simple bot players against a running `sn-server`
//! over real UDP (M10e). Each joins with the build and rules fingerprint
//! of this install, moves on a scripted path (no collision: M10f's bots
//! use the player's rules), uses a hatch every 12 s, and draws every
//! other bot; since all bots live in this process, every drawn tick is
//! compared with the owner's own state, as in the sync oracle.

use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use sn_install::GameData;
use sn_net::{Config, Endpoint, Event, PeerId, UdpTransport, random_seed};
use sn_protocol::{AnimState, Life, PlayerEventKind, PlayerId, PlayerState, Tick, quant};
use sn_session::{Channel, Client, ClientConfig, ClientNotice, SampleKind};

const FRAME_US: u64 = 16_000;

/// The scripted path at step `i` (position, velocity of the step), the
/// sync oracle's: swims, sharp turns, a stop, a dive, a zigzag, a 30 m
/// jump (a hatch) every 600 steps.
fn path_at(origin: [f64; 3], phase: usize, steps: usize, dt: f64) -> Vec<([f64; 3], [f64; 3])> {
    let mut out = Vec::with_capacity(steps);
    let mut p = origin;
    for i in 0..steps {
        let k = (i + phase * 75) % 600;
        let v = match k {
            0..100 => [7.6, 0.0, 0.0],
            100..200 => [0.0, 0.0, 7.6],
            200..250 => [0.0; 3],
            250..350 => [-3.0, -4.0, 0.0],
            _ if (k / 25) % 2 == 0 => [5.4, 0.0, 5.4],
            _ => [-5.4, 0.0, 5.4],
        };
        out.push((p, v));
        for a in 0..3 {
            p[a] += v[a] * dt;
        }
        if k == 350 {
            p[0] += 30.0;
        }
    }
    out
}

fn state_at(tick: u32, (p, v): ([f64; 3], [f64; 3])) -> PlayerState {
    let mut anim = AnimState::default();
    anim.set_float(
        "move_speed",
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(),
    );
    anim.set_bool("is_underwater", true);
    PlayerState {
        tick: Tick(tick),
        position: p.map(quant::position),
        velocity: v.map(quant::velocity),
        yaw: quant::turn(v[0].atan2(v[2]).to_degrees()),
        pitch: 0,
        rotation: None,
        walking: false,
        swimming: true,
        grounded: false,
        walk_grounded: true,
        in_pod: false,
        cinematic: false,
        controls: true,
        life: Life::Alive,
        anim,
    }
}

struct Bot {
    ep: Endpoint<UdpTransport>,
    server: PeerId,
    client: Client,
    phase: usize,
    start: Option<u32>,
    path: Vec<([f64; 3], [f64; 3])>,
}

#[derive(Default)]
struct PairStats {
    drawn: u64,
    exact: u64,
    max_error: f64,
}

pub fn run(game: &GameData, server: &str, count: usize, seconds: f64) -> crate::Result<()> {
    let server: SocketAddr = server
        .to_socket_addrs()
        .map_err(|e| format!("{server}: {e}"))?
        .next()
        .ok_or_else(|| format!("{server}: no address"))?;
    let assets = sn_assets::Assets::index(game)?;
    let rules = sn_assets::rules(game, &assets)?;
    let dt = rules.tick_us as f64 / 1e6;
    println!(
        "bots: {count} against {server} for {seconds} s; build {:?}, fingerprint {:016x}, tick {} µs",
        rules.build, rules.fingerprint, rules.tick_us
    );
    let steps = (seconds / dt) as usize + 1000;
    let clock = Instant::now();
    let now = || clock.elapsed().as_micros() as u64;
    let mut bots: Vec<Bot> = (0..count)
        .map(|i| {
            let mut ep = Endpoint::new(
                UdpTransport::bind("0.0.0.0:0").map_err(|e| e.to_string())?,
                Config::client(random_seed()),
            );
            let peer = ep.connect(server, now());
            let client = Client::new(
                ClientConfig {
                    name: format!("bot{i}"),
                    build: rules.build.clone(),
                    fingerprint: rules.fingerprint,
                    password: None,
                    tick_us: rules.tick_us,
                    max_speed: rules.max_speed,
                    rules: Default::default(),
                },
                now(),
            );
            let origin = [-127.7 + 3.0 * i as f64, -10.0, -49.7];
            Ok(Bot {
                ep,
                server: peer,
                client,
                phase: i,
                start: None,
                path: path_at(origin, i, steps, dt),
            })
        })
        .collect::<crate::Result<_>>()?;
    let mut truth: HashMap<PlayerId, BTreeMap<u32, PlayerState>> = HashMap::new();
    let mut next_check: HashMap<(usize, PlayerId), u32> = HashMap::new();
    let mut pairs: BTreeMap<(usize, usize), PairStats> = BTreeMap::new();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut problems: Vec<String> = Vec::new();
    let end = (seconds * 1e6) as u64;
    let mut next_frame = 0;
    let mut next_report = 10_000_000;
    while now() < end {
        let t = now();
        for b in &mut bots {
            b.ep.update(t);
            while let Some(e) = b.ep.poll_event() {
                match e {
                    Event::Reliable(_, m) | Event::Latest(_, _, m) | Event::Unordered(_, m) => {
                        b.client.received(&m, t)
                    }
                    Event::Connected(_) => {}
                    Event::Disconnected(_, r) => problems.push(format!("disconnected: {r:?}")),
                }
            }
            for n in b.client.take_notices() {
                match n {
                    ClientNotice::Welcomed { .. } => {}
                    other => problems.push(format!("{other:?}")),
                }
            }
            b.client.update(t);
            if b.client.server_us(t).is_some() {
                b.client.ready();
            }
            let me = b.client.player();
            for tick in b.client.steps(t) {
                let start = *b.start.get_or_insert(tick);
                let i = ((tick - start) as usize).min(b.path.len() - 1);
                let s = state_at(tick, b.path[i]);
                if (i + b.phase * 75) % 600 == 340 {
                    b.client
                        .event(Tick(tick), PlayerEventKind::HatchUsed { hatch: 1 });
                }
                if let Some(me) = me {
                    truth.entry(me).or_default().insert(tick, s);
                }
                b.client.push_state(s);
            }
            for (ch, m) in b.client.take_outgoing() {
                let bytes = m.encode();
                let sent = match ch {
                    Channel::Reliable => b.ep.send_reliable(b.server, bytes),
                    Channel::Latest(k) => b.ep.send_latest(b.server, k, bytes),
                    Channel::Unordered => b.ep.send_unordered(b.server, bytes),
                };
                if let Err(e) = sent {
                    problems.push(format!("send: {e}"));
                }
            }
            b.ep.update(t);
        }
        if t >= next_frame {
            next_frame = t + FRAME_US;
            for (oi, o) in bots.iter_mut().enumerate() {
                o.client.take_events(t);
                let Some(d) = o.client.draw_tick(t) else {
                    continue;
                };
                let to = d.floor().max(0.0) as u32;
                let remotes: Vec<PlayerId> = o.client.remotes().collect();
                for pid in remotes {
                    let Some(owner) = truth.get(&pid) else {
                        continue;
                    };
                    let from = match next_check.get(&(oi, pid)) {
                        Some(&f) => f,
                        None => match (0..=to + 64).find(|&k| o.client.has_state(pid, k)) {
                            Some(f) => f,
                            None => continue,
                        },
                    };
                    if to < from {
                        next_check.insert((oi, pid), from);
                        continue;
                    }
                    next_check.insert((oi, pid), to + 1);
                    let p = pairs.entry((oi, pid.0 as usize)).or_default();
                    for r in from..=to {
                        let Some(want) = owner.get(&r) else {
                            continue;
                        };
                        let Some(s) = o.client.sample(pid, f64::from(r)) else {
                            continue;
                        };
                        p.drawn += 1;
                        *kinds.entry(format!("{:?}", s.kind)).or_default() += 1;
                        if s.kind == SampleKind::Exact && s.base == *want {
                            p.exact += 1;
                        }
                        if s.kind != SampleKind::Snapped {
                            let w = want.position.map(f64::from);
                            let e = (0..3)
                                .map(|a| (s.position[a] - w[a]).powi(2))
                                .sum::<f64>()
                                .sqrt();
                            p.max_error = p.max_error.max(e);
                        }
                    }
                }
            }
            // Keep the truth small.
            if let Some(oldest) = bots
                .iter()
                .filter_map(|b| b.client.draw_tick(t))
                .reduce(f64::min)
            {
                let keep = (oldest.max(0.0) as u32).saturating_sub(200);
                for m in truth.values_mut() {
                    *m = m.split_off(&keep);
                }
            }
        }
        if t >= next_report {
            next_report += 10_000_000;
            let (drawn, exact) = pairs
                .values()
                .fold((0, 0), |a, p| (a.0 + p.drawn, a.1 + p.exact));
            println!(
                "[{:6.1} s] {} of {count} joined; {drawn} ticks drawn, {:.3} % exact; delay {:.1}–{:.1} ticks; {} problems",
                t as f64 / 1e6,
                bots.iter().filter(|b| b.client.player().is_some()).count(),
                100.0 * exact as f64 / drawn.max(1) as f64,
                bots.iter()
                    .map(|b| b.client.delay())
                    .fold(f64::MAX, f64::min),
                bots.iter().map(|b| b.client.delay()).fold(0.0, f64::max),
                problems.len()
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // Counters before the goodbyes (the server then closes, and the
    // connections and their counters go).
    let finals: Vec<(Option<sn_net::Stats>, Option<f64>)> = bots
        .iter()
        .map(|b| (b.ep.peer_stats(b.server), b.ep.rtt_us(b.server)))
        .collect();
    for b in &mut bots {
        b.client.bye();
        for (_, m) in b.client.take_outgoing() {
            let _ = b.ep.send_reliable(b.server, m.encode());
        }
    }
    // Let the goodbyes go out.
    let until = now() + 300_000;
    while now() < until {
        for b in &mut bots {
            b.ep.update(now());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let (drawn, exact) = pairs
        .values()
        .fold((0, 0), |a, p| (a.0 + p.drawn, a.1 + p.exact));
    let worst = pairs
        .iter()
        .map(|(k, p)| (k, p.exact as f64 / p.drawn.max(1) as f64))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let max_error = pairs.values().map(|p| p.max_error).fold(0.0, f64::max);
    println!(
        "bots done: {} pairs, {drawn} ticks drawn, {exact} exact ({:.4} %), worst pair {:?}; kinds {kinds:?}; max error {max_error:.4} m (snapped excluded)",
        pairs.len(),
        100.0 * exact as f64 / drawn.max(1) as f64,
        worst.map(|(k, e)| (k, (e * 1e4).round() / 1e2))
    );
    for (i, (b, (st, rtt))) in bots.iter().zip(finals).enumerate() {
        let rtt = rtt.map_or(f64::NAN, |r| r / 1000.0);
        let c = b.client.stats();
        println!(
            "bot{i}: player {:?}, rtt {rtt:.2} ms, {} pongs, {} states received ({} duplicates), {} events, {} hash reports; up {:.0} B/s",
            b.client.player().map(|p| p.0),
            c.pongs,
            c.states_received,
            c.states_duplicate,
            c.events,
            c.hash_reports,
            st.map_or(0.0, |s| (s.bytes_sent + 28 * s.packets_sent) as f64
                / seconds)
        );
    }
    if problems.is_empty() {
        println!("problems: none");
    } else {
        println!(
            "problems ({}): {:?}",
            problems.len(),
            &problems[..problems.len().min(20)]
        );
    }
    Ok(())
}
