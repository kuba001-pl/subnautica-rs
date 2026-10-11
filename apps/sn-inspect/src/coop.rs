//! `sn-inspect coop` (M10f): bot players with the player's own rules and
//! collision against a running `sn-server`, over UDP with the plan's
//! impairment presets (`sn_net::ImpairedTransport` at each bot).
//!
//! Each bot first runs its script headless with the real rules (`walk`'s
//! pieces: spawn in Lifepod 5, walk to the hatch, its cinematic, swim
//! out; even bots swim back and board, odd ones are killed and respawn in
//! the pod), recording every physics step. Bots do not touch each other
//! (players pass through each other, `docs/DESIGN.md` § 9, decision 7) and
//! the world's collision is static, so the record is what the bot would
//! do live. Then they join and replay it in the server's ticks: each
//! sends its states and events as the client will, draws every other bot,
//! and every drawn tick is compared with the owner's own state; every
//! event each bot sees is compared with what its owner sent.

use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, ToSocketAddrs};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use sn_install::GameData;
use sn_net::sim::Impairment;
use sn_net::{Config, Endpoint, Event, ImpairedTransport, PeerId, UdpTransport, random_seed};
use sn_protocol::{
    AnimState, DeathCause, Life, PlayerEventKind, PlayerId, PlayerState, Tick, WorldEventKind,
    anim, quant,
};
use sn_session::{Channel, Client, ClientConfig, ClientNotice, SampleKind};
use sn_sim::V3;
use sn_sim::body::AnimValue;
use sn_sim::player::Input;
use sn_sim::vitals::{Vitals, VitalsEvent};

use crate::Result;
use crate::collision::Streamed;
use crate::walk::{self, StepRecord};

const FRAME_US: u64 = 16_000;
/// After the script: held still this long so late events and states land.
const SETTLE_S: f64 = 4.0;

/// One bot's script, recorded: the states per step and the events with
/// their step.
struct Script {
    states: Vec<PlayerState>,
    events: Vec<(usize, PlayerEventKind)>,
    lifepod: [f32; 3],
    slot_fills: Vec<([i32; 3], u64)>,
    summary: String,
}

fn life(l: Option<sn_sim::vitals::Life>) -> Life {
    match l {
        None | Some(sn_sim::vitals::Life::Alive) => Life::Alive,
        Some(sn_sim::vitals::Life::Dead { .. }) => Life::Dead,
        Some(sn_sim::vitals::Life::Respawning { .. }) => Life::Respawning,
        Some(sn_sim::vitals::Life::Restored { .. }) => Life::Restored,
    }
}

/// A record as the protocol's state (tick filled in at replay).
fn to_state(r: &StepRecord) -> (PlayerState, Vec<u8>) {
    let mut anim_state = AnimState::default();
    let mut triggers = Vec::new();
    for &(name, v) in &r.values {
        match v {
            AnimValue::Float(x) => {
                anim_state.set_float(name, x);
            }
            AnimValue::Bool(b) => {
                anim_state.set_bool(name, b);
            }
            AnimValue::Trigger => triggers.extend(anim::trigger_index(name)),
        }
    }
    let q = r.rotation;
    let identity = (q.w.abs() - 1.0).abs() < 1e-9;
    let s = PlayerState {
        tick: Tick(0),
        position: [r.position.x, r.position.y, r.position.z].map(quant::position),
        velocity: [r.velocity.x, r.velocity.y, r.velocity.z].map(quant::velocity),
        yaw: quant::turn(r.look.rotation_x),
        pitch: quant::fixed(r.look.rotation_y, quant::PITCH_SCALE),
        rotation: (!identity)
            .then(|| [q.x, q.y, q.z, q.w].map(|c| quant::fixed(c, quant::QUATERNION_SCALE))),
        walking: r.walking,
        swimming: r.swimming,
        grounded: r.grounded,
        walk_grounded: r.walk_grounded,
        in_pod: r.in_pod,
        cinematic: r.cinematic,
        controls: r.controls,
        life: life(r.life),
        anim: anim_state,
    };
    (s, triggers)
}

/// Runs bot `k`'s script with the player's rules and records it.
fn record(
    game: &GameData,
    k: usize,
    count: usize,
    seed: u64,
    slot_seed: u64,
    steps: usize,
) -> Result<Script> {
    let started = Instant::now();
    let mut s = Streamed::new(game, slot_seed)?;
    let assets = s.loader.assets();
    let data = sn_assets::player_data(assets)?;
    let code = sn_assets::player_code(&sn_assets::read_assembly(game)?)?;
    let vp = sn_assets::vitals_params(&data, &code);
    let mut r = walk::start(&mut s, seed)?;
    r.vitals = Some((vp.clone(), Vitals::new(&vp)));
    r.record = Some(Vec::with_capacity(steps));
    let lifepod = r.pod_point.to_f32();
    let mut what = Vec::new();
    // Out of the pod through the hatch (its first-use cinematic).
    let out = walk::leave_pod(&mut r)?;
    what.push(format!("left the pod: {out}"));
    // Swim out in this bot's own direction.
    let angle = std::f64::consts::TAU * k as f64 / count.max(1) as f64;
    let dir = V3::new(angle.sin(), 0.0, angle.cos());
    let far = r.player.position + dir * 100.0 + V3::new(0.0, -2.0, 0.0);
    r.go_to(far, 8.0, None, 0.5)?;
    if k % 2 == 0 {
        let boarded = walk::board(&mut r)?;
        what.push(format!("boarded: {boarded}"));
    } else {
        if let Some((vp, v)) = r.vitals.as_mut() {
            let mut events = Vec::new();
            v.take_damage(vp.max_health, &mut events);
            if events.contains(&VitalsEvent::Died) {
                r.body.died();
                what.push("killed".into());
            }
        }
    }
    // Then idle (floating or standing) to the end.
    while r.record.as_ref().is_some_and(|v| v.len() < steps) {
        r.step(&Input::default())?;
    }
    let rec = r.record.take().unwrap_or_default();
    let mut states = Vec::with_capacity(rec.len());
    let mut events = Vec::new();
    let mut prev: Option<&StepRecord> = None;
    for (i, step) in rec.iter().enumerate() {
        let (st, triggers) = to_state(step);
        for t in triggers {
            events.push((i, PlayerEventKind::AnimatorTrigger { trigger: t }));
        }
        let (was_hatch, was_life) = prev.map_or((None, Life::Alive), |p| (p.hatch, life(p.life)));
        match (was_hatch, step.hatch) {
            (None, Some(h)) => events.push((i, PlayerEventKind::HatchUsed { hatch: h as u16 })),
            (Some(_), None) => events.push((i, PlayerEventKind::CinematicEnded)),
            _ => {}
        }
        match (was_life, st.life) {
            (Life::Alive | Life::Restored, Life::Dead) => events.push((
                i,
                PlayerEventKind::Died {
                    cause: DeathCause::Normal,
                },
            )),
            (Life::Dead, Life::Respawning) => events.push((i, PlayerEventKind::Respawned)),
            _ => {}
        }
        states.push(st);
        prev = Some(step);
    }
    let slot_fills =
        r.s.loader
            .slot_fill_hashes()
            .iter()
            .map(|(c, h)| ([c.x, c.y, c.z], *h))
            .collect();
    let summary = format!(
        "bot{k}: recorded {} steps in {:.1} s ({}); {} events: {:?}",
        states.len(),
        started.elapsed().as_secs_f64(),
        what.join(", "),
        events.len(),
        events
            .iter()
            .filter(|e| !matches!(e.1, PlayerEventKind::AnimatorTrigger { .. }))
            .map(|(i, e)| format!("{e:?} @ step {i}"))
            .collect::<Vec<_>>()
    );
    Ok(Script {
        states,
        events,
        lifepod,
        slot_fills,
        summary,
    })
}

struct Bot<'a> {
    name: String,
    ep: Endpoint<ImpairedTransport<UdpTransport>>,
    server: PeerId,
    client: Client,
    script: &'a Script,
    start: Option<u32>,
    sent_events: Vec<(Tick, PlayerEventKind)>,
    slots_sent: bool,
}

#[derive(Default)]
struct Pair {
    drawn: u64,
    exact: u64,
    max_error: f64,
    /// Player events seen of the owner: (tick, kind).
    events: Vec<(Tick, PlayerEventKind)>,
}

/// One run: `count` bots replay their scripts with `preset` on each bot's
/// link. True if every check passed.
fn replay(
    server: SocketAddr,
    rules: &sn_assets::Rules,
    scripts: &[Script],
    count: usize,
    preset: &str,
) -> Result<bool> {
    let imp = Impairment::preset(preset).ok_or_else(|| format!("no preset {preset}"))?;
    let clock = Instant::now();
    let now = || clock.elapsed().as_micros() as u64;
    let mut bots: Vec<Bot> = (0..count)
        .map(|k| {
            let udp = UdpTransport::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            let mut ep = Endpoint::new(
                ImpairedTransport::new(udp, imp, random_seed()),
                Config::client(random_seed()),
            );
            let peer = ep.connect(server, now());
            Ok(Bot {
                name: format!("coop{k}"),
                ep,
                server: peer,
                client: Client::new(
                    ClientConfig {
                        name: format!("coop{k}"),
                        build: rules.build.clone(),
                        fingerprint: rules.fingerprint,
                        password: None,
                        tick_us: rules.tick_us,
                        max_speed: rules.max_speed,
                        rules: rules.items.clone(),
                    },
                    now(),
                ),
                script: &scripts[k],
                start: None,
                sent_events: Vec::new(),
                slots_sent: false,
            })
        })
        .collect::<Result<_>>()?;
    let steps = scripts[..count]
        .iter()
        .map(|s| s.states.len())
        .max()
        .unwrap_or(0);
    let tick_s = rules.tick_us as f64 / 1e6;
    let mut truth: HashMap<PlayerId, BTreeMap<u32, PlayerState>> = HashMap::new();
    let mut next_check: HashMap<(usize, PlayerId), u32> = HashMap::new();
    let mut pairs: BTreeMap<(usize, PlayerId), Pair> = BTreeMap::new();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut problems: Vec<String> = Vec::new();
    let mut next_frame = 0;
    let deadline = now() + ((steps as f64 * tick_s + SETTLE_S + 20.0) * 1e6) as u64;
    let mut done_at: Option<u64> = None;
    loop {
        let t = now();
        if t > deadline {
            problems.push("did not finish in time".into());
            break;
        }
        if done_at.is_some_and(|d| t >= d) {
            break;
        }
        for b in &mut bots {
            b.ep.update(t);
            while let Some(e) = b.ep.poll_event() {
                match e {
                    Event::Reliable(_, m) | Event::Latest(_, _, m) | Event::Unordered(_, m) => {
                        b.client.received(&m, t)
                    }
                    Event::Connected(_) => {}
                    Event::Disconnected(_, r) => {
                        problems.push(format!("{}: disconnected {r:?}", b.name))
                    }
                }
            }
            for n in b.client.take_notices() {
                match n {
                    ClientNotice::Welcomed { .. } => {
                        let w = b.client.world().map(|w| w.settings);
                        if w.and_then(|s| s.lifepod) != Some(b.script.lifepod) {
                            problems.push(format!(
                                "{}: the server's lifepod {:?} is not the recorded {:?} (run both with the same seed)",
                                b.name,
                                w.and_then(|s| s.lifepod),
                                b.script.lifepod
                            ));
                        }
                    }
                    // The world state after `Welcome` (protocol 4).
                    ClientNotice::Loaded { .. } => {}
                    other => problems.push(format!("{}: {other:?}", b.name)),
                }
            }
            b.client.update(t);
            if b.client.server_us(t).is_some() {
                b.client.ready();
                if !b.slots_sent {
                    b.slots_sent = true;
                    for &(batch, hash) in &b.script.slot_fills {
                        b.client.slot_fill(batch, hash);
                    }
                }
            }
            let me = b.client.player();
            for tick in b.client.steps(t) {
                let Some(me) = me else { continue };
                let start = *b.start.get_or_insert(tick);
                let i = (tick - start) as usize;
                let mut s = b.script.states[i.min(b.script.states.len() - 1)];
                s.tick = Tick(tick);
                for (_, kind) in b.script.events.iter().filter(|e| e.0 == i) {
                    b.client.event(Tick(tick), *kind);
                    b.sent_events.push((Tick(tick), *kind));
                }
                truth.entry(me).or_default().insert(tick, s);
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
                    problems.push(format!("{}: send {e}", b.name));
                }
            }
            b.ep.update(t);
        }
        if t >= next_frame {
            next_frame = t + FRAME_US;
            for (oi, o) in bots.iter_mut().enumerate() {
                for e in o.client.take_events(t) {
                    if let WorldEventKind::Player { player, event } = e.kind
                        && Some(player) != o.client.player()
                    {
                        pairs
                            .entry((oi, player))
                            .or_default()
                            .events
                            .push((e.tick, event));
                    }
                }
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
                        continue;
                    }
                    next_check.insert((oi, pid), to + 1);
                    let p = pairs.entry((oi, pid)).or_default();
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
        }
        // Done when every bot played its script and settled.
        if done_at.is_none()
            && bots.iter().all(|b| {
                b.start.is_some_and(|s| {
                    b.client.steps_taken().saturating_sub(s) as usize >= b.script.states.len()
                })
            })
        {
            done_at = Some(t + (SETTLE_S * 1e6) as u64);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // Counters before the goodbyes.
    let finals: Vec<Option<sn_net::Stats>> =
        bots.iter().map(|b| b.ep.peer_stats(b.server)).collect();
    let seconds = clock.elapsed().as_secs_f64();
    for b in &mut bots {
        b.client.bye();
        for (_, m) in b.client.take_outgoing() {
            let _ = b.ep.send_reliable(b.server, m.encode());
        }
    }
    let until = now() + 500_000;
    while now() < until {
        for b in &mut bots {
            b.ep.update(now());
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    // Report and checks.
    let ids: Vec<Option<PlayerId>> = bots.iter().map(|b| b.client.player()).collect();
    let mut ok = problems.is_empty();
    let (drawn, exact) = pairs
        .values()
        .fold((0, 0), |a, p| (a.0 + p.drawn, a.1 + p.exact));
    let exact_share = exact as f64 / drawn.max(1) as f64;
    let target = match preset {
        "lan" => 1.0,
        "internet" => 0.999,
        _ => 0.99,
    };
    let max_error = pairs.values().map(|p| p.max_error).fold(0.0, f64::max);
    println!(
        "{preset}, {count} bots: {} pairs, {drawn} ticks drawn, {exact} exact ({:.3} %; target {:.1} %), kinds {kinds:?}, max error {max_error:.4} m (snapped excluded)",
        pairs.len(),
        100.0 * exact_share,
        100.0 * target
    );
    for ((oi, pid), p) in &pairs {
        println!(
            "  {} draws player {}: {} ticks, {:.3} % exact, max error {:.4} m",
            bots[*oi].name,
            pid.0,
            p.drawn,
            100.0 * p.exact as f64 / p.drawn.max(1) as f64,
            p.max_error
        );
    }
    ok &= exact_share >= target && (preset != "bad" || max_error < 0.5);
    // Events: every bot saw exactly what each other bot sent, same ticks.
    let mut event_lines = 0;
    for (oi, o) in bots.iter().enumerate() {
        for (wi, w) in bots.iter().enumerate() {
            if oi == wi {
                continue;
            }
            let Some(pid) = ids[wi] else { continue };
            let seen = pairs
                .get(&(oi, pid))
                .map_or(&[][..], |p| p.events.as_slice());
            let same = seen == w.sent_events.as_slice();
            if !same {
                ok = false;
                println!(
                    "  EVENTS DIFFER: {} saw {:?}; {} sent {:?}",
                    o.name, seen, w.name, w.sent_events
                );
            }
            event_lines += 1;
        }
    }
    let key: Vec<String> = bots
        .iter()
        .map(|b| {
            format!(
                "{}: {:?}",
                b.name,
                b.sent_events
                    .iter()
                    .filter(|e| !matches!(e.1, PlayerEventKind::AnimatorTrigger { .. }))
                    .map(|(t, e)| format!("{e:?}@{}", t.0))
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    println!("  events sent (ticks): {key:?}; {event_lines} observer/owner lists compared");
    for (b, st) in bots.iter().zip(&finals) {
        let c = b.client.stats();
        let up = st.map_or(0.0, |s| {
            (s.bytes_sent + 28 * s.packets_sent) as f64 / seconds
        });
        println!(
            "  {}: player {:?}, up {up:.0} B/s, delay {:.1} ticks, {} states received, {} events, {} hash reports",
            b.name,
            b.client.player().map(|p| p.0),
            b.client.delay(),
            c.states_received,
            c.events,
            c.hash_reports
        );
        ok &= up < 10_000.0;
    }
    if !problems.is_empty() {
        println!("  problems: {:?}", &problems[..problems.len().min(10)]);
    }
    println!(
        "{preset}, {count} bots: {}",
        if ok { "ok" } else { "FAILED" }
    );
    Ok(ok)
}

pub fn run(
    game: &GameData,
    server: &str,
    counts: &[usize],
    presets: &[String],
    seconds: f64,
    seed: u64,
) -> Result<ExitCode> {
    let server: SocketAddr = server
        .to_socket_addrs()
        .map_err(|e| format!("{server}: {e}"))?
        .next()
        .ok_or_else(|| format!("{server}: no address"))?;
    let assets = sn_assets::Assets::index(game)?;
    let rules = sn_assets::rules(game, &assets)?;
    drop(assets);
    let most = counts.iter().copied().max().unwrap_or(2);
    let steps = (seconds * 1e6 / rules.tick_us as f64) as usize;
    println!(
        "coop: recording {most} bots, {steps} steps each ({seconds} s), lifepod seed {seed}, slot seed 1"
    );
    let mut scripts = Vec::new();
    for k in 0..most {
        let s = record(game, k, most, seed, 1, steps)?;
        println!("{}", s.summary);
        scripts.push(s);
    }
    let mut all = true;
    for preset in presets {
        for &count in counts {
            all &= replay(server, &rules, &scripts, count, preset)?;
        }
    }
    println!("coop: {}", if all { "all ok" } else { "SOME FAILED" });
    Ok(if all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
