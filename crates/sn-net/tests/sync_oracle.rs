//! M10d's sync oracle: a server and two clients (`sn-session`) over the
//! simulated network (`sn-net`) with the plan's presets. Each client
//! moves on a scripted path (straight swims, sharp turns, stops, a dive,
//! a zigzag, a 30 m teleport every 12 s) in the server's ticks; each
//! draws the other at its interpolation delay, and every whole tick it
//! draws is compared with the state the other player itself recorded for
//! that tick. Printed with `cargo test -p sn-net --test sync_oracle --
//! --nocapture`.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;

use sn_net::sim::{Impairment, SimNetwork, SimSocket};
use sn_net::{Config, Endpoint, Event, PeerId};
use sn_protocol::*;
use sn_session::{
    Action, Channel, Client, ClientConfig, ClientNotice, ConnId, SampleKind, Server, ServerConfig,
};

const TICK_US: u64 = 20_000;
const TICK_S: f64 = 0.02;
const RUN_US: u64 = 70_000_000;
const FRAME_US: u64 = 16_000;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([10, 0, 0, 1], port))
}

fn settings() -> WorldSettings {
    WorldSettings {
        slot_seed: Some(1),
        lifepod: Some([0.0, 0.0, 0.0]),
        aurora: AuroraStart::Countdown { seconds: 3000.0 },
        clock_start: 480.0,
        time_scale: 1.0,
    }
}

/// The scripted path: (position, velocity) at step `i` from the start.
/// The velocity is the one of the step from `i` to `i + 1`.
fn path(phase: usize, n: usize) -> Vec<([f64; 3], [f64; 3])> {
    let mut out = Vec::with_capacity(n);
    let mut p = [10.0 * phase as f64, -10.0, 0.0];
    for i in 0..n {
        let k = (i + phase * 300) % 600;
        let v = match k {
            0..100 => [7.6, 0.0, 0.0],
            100..200 => [0.0, 0.0, 7.6],
            200..250 => [0.0; 3],
            250..350 => [-3.0, -4.0, 0.0],
            // Zigzag: a sharp turn every 25 steps.
            _ => {
                if (k / 25) % 2 == 0 {
                    [5.4, 0.0, 5.4]
                } else {
                    [-5.4, 0.0, 5.4]
                }
            }
        };
        out.push((p, v));
        for a in 0..3 {
            p[a] += v[a] * TICK_S;
        }
        if k == 350 {
            // Through a hatch.
            p[0] += 30.0;
        }
    }
    out
}

fn state_at(tick: u32, (p, v): ([f64; 3], [f64; 3])) -> PlayerState {
    let mut anim = AnimState::default();
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    anim.set_float("move_speed", speed);
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

struct Node {
    name: &'static str,
    ep: Endpoint<SimSocket>,
    server: PeerId,
    client: Client,
    phase: usize,
    start_tick: Option<u32>,
    path: Vec<([f64; 3], [f64; 3])>,
    clock_error_10s: Option<f64>,
    clock_error_max: f64,
}

#[derive(Default, Debug)]
struct Pair {
    drawn: u64,
    exact: u64,
    kinds: BTreeMap<String, u64>,
    no_sample: u64,
    max_error: f64,
    errors: Vec<f64>,
    events: u64,
    event_lateness_max: f64,
}

struct Report {
    pairs: Vec<(String, Pair)>,
    hash_mismatches: u64,
    hash_checks: u64,
    clock_10s: Vec<f64>,
    clock_max: Vec<f64>,
    delays: Vec<f64>,
    /// Bytes per second each client sent, with 28 B of IPv4/UDP header
    /// per packet.
    upload: Vec<f64>,
}

fn run(preset: &str, seed: u64) -> Report {
    let imp = Impairment::preset(preset).unwrap();
    let net = SimNetwork::new(seed, imp);
    let mut sep = Endpoint::new(net.socket(addr(1)), Config::server(8, seed));
    let mut server = Server::new(
        ServerConfig {
            build: "10".into(),
            fingerprint: 9,
            password: None,
            max_players: 8,
            settings: settings(),
            tick_us: TICK_US,
            max_speed: 30.0,
            rules: Default::default(),
        },
        0,
    );
    let mut nodes: Vec<Node> = ["a", "b"]
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let mut ep = Endpoint::new(
                net.socket(addr(10 + i as u16)),
                Config::client(seed + 1 + i as u64),
            );
            let server_peer = ep.connect(addr(1), 0);
            Node {
                name,
                ep,
                server: server_peer,
                client: Client::new(
                    ClientConfig {
                        name: name.into(),
                        build: "10".into(),
                        fingerprint: 9,
                        password: None,
                        tick_us: TICK_US,
                        max_speed: 30.0,
                        rules: Default::default(),
                    },
                    0,
                ),
                phase: i,
                start_tick: None,
                path: path(i, 5000),
                clock_error_10s: None,
                clock_error_max: 0.0,
            }
        })
        .collect();
    // Each player's own record of its states, by player id.
    let mut truth: HashMap<PlayerId, BTreeMap<u32, PlayerState>> = HashMap::new();
    let mut pairs: BTreeMap<(usize, usize), Pair> = BTreeMap::new();
    // First tick each observer received of each owner.
    let mut first_seen: HashMap<(usize, PlayerId), u32> = HashMap::new();
    let mut joined_at: Vec<Option<u64>> = vec![None; nodes.len()];

    let mut now = 0u64;
    let mut next_ms = 0u64;
    let mut next_frame = 0u64;
    while now < RUN_US {
        let arrival = net.next_arrival().unwrap_or(u64::MAX);
        now = next_ms.min(arrival).min(RUN_US);
        net.set_time(now);
        let app_step = now == next_ms;
        if app_step {
            next_ms += 1_000;
        }

        // Server side.
        sep.update(now);
        while let Some(e) = sep.poll_event() {
            match e {
                Event::Connected(p) => server.connected(ConnId(p.0), now),
                Event::Disconnected(p, r) => {
                    server.disconnected(ConnId(p.0), r == sn_net::DisconnectReason::TimedOut, now)
                }
                Event::Reliable(p, b) | Event::Latest(p, _, b) | Event::Unordered(p, b) => {
                    server.received(ConnId(p.0), &b, now)
                }
            }
        }
        server.update(now);
        for a in server.take_actions() {
            match a {
                Action::Send {
                    to,
                    channel,
                    message,
                } => {
                    let bytes = message.encode();
                    let peer = PeerId(to.0);
                    match channel {
                        Channel::Reliable => sep.send_reliable(peer, bytes).unwrap(),
                        Channel::Latest(k) => sep.send_latest(peer, k, bytes).unwrap(),
                        Channel::Unordered => sep.send_unordered(peer, bytes).unwrap(),
                    }
                }
                Action::Close(c) => sep.disconnect(PeerId(c.0)),
            }
        }
        sep.update(now);

        // Clients.
        let frame = app_step && now >= next_frame;
        if frame {
            next_frame += FRAME_US;
        }
        for (ni, n) in nodes.iter_mut().enumerate() {
            n.ep.update(now);
            while let Some(e) = n.ep.poll_event() {
                match e {
                    Event::Reliable(_, b) | Event::Latest(_, _, b) | Event::Unordered(_, b) => {
                        n.client.received(&b, now);
                    }
                    Event::Connected(_) => {}
                    Event::Disconnected(_, r) => panic!("{}: disconnected {r:?}", n.name),
                }
            }
            for notice in n.client.take_notices() {
                if let ClientNotice::Apply(e) = notice {
                    panic!("{}: {e}", n.name);
                }
            }
            if app_step {
                n.client.update(now);
                if let Some(t) = n.client.server_us(now) {
                    let err = t - now as f64;
                    let since = now - *joined_at[ni].get_or_insert(now);
                    if since >= 10_000_000 {
                        n.clock_error_10s.get_or_insert(err);
                        n.clock_error_max = n.clock_error_max.max(err.abs());
                    }
                    n.client.ready();
                }
                let me = n.client.player();
                for t in n.client.steps(now) {
                    let start = *n.start_tick.get_or_insert(t);
                    let i = (t - start) as usize;
                    let s = state_at(t, n.path[i.min(n.path.len() - 1)]);
                    let k = (i + n.phase * 300) % 600;
                    if k == 340 {
                        n.client
                            .event(Tick(t), PlayerEventKind::HatchUsed { hatch: 1 });
                    }
                    if let Some(me) = me {
                        truth.entry(me).or_default().insert(t, s);
                    }
                    n.client.push_state(s);
                }
            }
            for (ch, m) in n.client.take_outgoing() {
                let bytes = m.encode();
                match ch {
                    Channel::Reliable => n.ep.send_reliable(n.server, bytes).unwrap(),
                    Channel::Latest(k) => n.ep.send_latest(n.server, k, bytes).unwrap(),
                    Channel::Unordered => n.ep.send_unordered(n.server, bytes).unwrap(),
                }
            }
            n.ep.update(now);
        }

        // Frames: every whole tick passed since the last frame is checked.
        if frame {
            for oi in 0..nodes.len() {
                let Some(d) = nodes[oi].client.draw_tick(now) else {
                    continue;
                };
                // Events due now, and how late.
                let released = nodes[oi].client.take_events(now);
                for e in released {
                    if let WorldEventKind::Player { player, .. } = e.kind
                        && Some(player) != nodes[oi].client.player()
                    {
                        let owner = nodes
                            .iter()
                            .position(|n| n.client.player() == Some(player))
                            .unwrap();
                        let p = pairs.entry((oi, owner)).or_default();
                        p.events += 1;
                        p.event_lateness_max = p.event_lateness_max.max(d - f64::from(e.tick.0));
                    }
                }
                for owner in 0..nodes.len() {
                    if owner == oi {
                        continue;
                    }
                    let Some(pid) = nodes[owner].client.player() else {
                        continue;
                    };
                    let c = &nodes[oi].client;
                    // Checking starts at the first state this observer got.
                    let from = match first_seen.get(&(oi, pid)) {
                        Some(&f) => f,
                        None => {
                            let Some(f) =
                                (0..=d.max(0.0) as u32 + 64).find(|&t| c.has_state(pid, t))
                            else {
                                continue;
                            };
                            first_seen.insert((oi, pid), f);
                            f
                        }
                    };
                    let to = d.floor().max(0.0) as u32;
                    if to < from {
                        continue;
                    }
                    first_seen.insert((oi, pid), to + 1);
                    for r in from..=to {
                        let Some(want) = truth.get(&pid).and_then(|t| t.get(&r)) else {
                            continue;
                        };
                        let p = pairs.entry((oi, owner)).or_default();
                        p.drawn += 1;
                        let Some(s) = c.sample(pid, f64::from(r)) else {
                            p.no_sample += 1;
                            continue;
                        };
                        *p.kinds.entry(format!("{:?}", s.kind)).or_default() += 1;
                        if s.kind == SampleKind::Exact && s.base == *want {
                            p.exact += 1;
                        }
                        let w = want.position.map(f64::from);
                        let err = ((s.position[0] - w[0]).powi(2)
                            + (s.position[1] - w[1]).powi(2)
                            + (s.position[2] - w[2]).powi(2))
                        .sqrt();
                        if s.kind != SampleKind::Snapped {
                            p.max_error = p.max_error.max(err);
                            p.errors.push(err);
                        }
                    }
                }
            }
        }
    }
    let stats = server.stats();
    Report {
        pairs: pairs
            .into_iter()
            .map(|((o, w), p)| (format!("{} draws {}", nodes[o].name, nodes[w].name), p))
            .collect(),
        hash_mismatches: stats.hash_mismatches,
        hash_checks: stats.hash_checks,
        clock_10s: nodes
            .iter()
            .map(|n| n.clock_error_10s.unwrap_or(f64::NAN))
            .collect(),
        clock_max: nodes.iter().map(|n| n.clock_error_max).collect(),
        delays: nodes.iter().map(|n| n.client.delay()).collect(),
        upload: nodes
            .iter()
            .map(|n| {
                let st = n.ep.peer_stats(n.server).unwrap();
                (st.bytes_sent + 28 * st.packets_sent) as f64 / (RUN_US as f64 / 1e6)
            })
            .collect(),
    }
}

fn print(preset: &str, seed: u64, r: &Report) {
    for (name, p) in &r.pairs {
        let mut e = p.errors.clone();
        e.sort_by(f64::total_cmp);
        let p99 = e.get(e.len() * 99 / 100).copied().unwrap_or(0.0);
        println!(
            "{preset} seed {seed}: {name}: {} ticks drawn, {} exact ({:.3} %), {:?}, no sample {}; error max {:.4} m, p99 {:.4} m (snapped excluded); {} events, latest released {:.2} ticks after their tick",
            p.drawn,
            p.exact,
            100.0 * p.exact as f64 / p.drawn.max(1) as f64,
            p.kinds,
            p.no_sample,
            p.max_error,
            p99,
            p.events,
            p.event_lateness_max
        );
    }
    println!(
        "{preset} seed {seed}: clock error after 10 s {:?} µs, max after 10 s {:?} µs; delay at the end {:?} ticks; hash checks {}, mismatches {}; upload {:?} B/s",
        r.clock_10s.iter().map(|e| e.round()).collect::<Vec<_>>(),
        r.clock_max.iter().map(|e| e.round()).collect::<Vec<_>>(),
        r.delays
            .iter()
            .map(|d| (d * 100.0).round() / 100.0)
            .collect::<Vec<_>>(),
        r.hash_checks,
        r.hash_mismatches,
        r.upload.iter().map(|b| b.round()).collect::<Vec<_>>()
    );
}

fn check(preset: &str, seed: u64, min_exact: f64, max_error: Option<f64>) {
    let r = run(preset, seed);
    print(preset, seed, &r);
    assert_eq!(r.pairs.len(), 2, "both directions drawn");
    for (name, p) in &r.pairs {
        assert!(p.drawn > 3000, "{name}: only {} ticks drawn", p.drawn);
        let exact = p.exact as f64 / p.drawn as f64;
        assert!(exact >= min_exact, "{preset} {name}: {exact} exact");
        if let Some(m) = max_error {
            assert!(
                p.max_error < m,
                "{preset} {name}: max error {}",
                p.max_error
            );
        }
        assert!(p.events >= 5, "{name}: {} events", p.events);
    }
    for e in &r.clock_10s {
        assert!(
            e.abs() < TICK_US as f64,
            "{preset}: clock off {e} µs after 10 s"
        );
    }
    assert!(r.hash_checks > 100);
    assert_eq!(r.hash_mismatches, 0);
}

#[test]
fn lan_every_drawn_tick_is_exact() {
    check("lan", 1, 1.0, Some(1e-9));
}

#[test]
fn internet_999_per_mille_exact() {
    check("internet", 2, 0.999, None);
}

#[test]
fn bad_99_percent_exact_and_under_half_a_metre() {
    for seed in [3, 4, 5] {
        check("bad", seed, 0.99, Some(0.5));
    }
}
