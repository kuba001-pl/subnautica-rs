//! The session (M10g, M10h; `docs/DESIGN.md` § M10 plan): every game runs
//! through `sn-session`. Solo and `--host` run its server in-process and
//! join it over a direct in-memory link (every message encoded and
//! decoded, as on the network); `--host` also opens it to the network on
//! UDP; `--join` connects to another one.
//!
//! The server, the client and the socket live on a network thread that
//! runs every millisecond, so a slow frame (or the load before the first
//! one) cannot time a connection out or blur the clock samples. The main
//! thread takes the player's steps, pushes its states and events and
//! samples the other players under a lock.
//!
//! The in-process server's tick 0 is the client's first frame (loading
//! before it does not count). The player's physics steps are the server's
//! ticks (`Client::steps`), the game clock is the server's time
//! ([`crate::aurora::GameClock`]), and the world's values (Lifepod 5's
//! point, the Aurora, the slot seed, the clock) come from the `Welcome`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bevy::log::Level;
use bevy::prelude::*;
use sn_net::sim::Impairment;
use sn_net::{
    Config, DisconnectReason, Endpoint, Event, ImpairedTransport, PeerId, UdpTransport, random_seed,
};
use sn_protocol::{
    PlayerEventKind, PlayerId, PlayerState, Tick, WorldEvent, WorldEventKind, WorldSettings,
};
use sn_session::{
    Action, Channel, Client, ClientConfig, ClientNotice, ConnId, Notice, RemoteSample, Server,
    ServerConfig,
};

/// The local client's connection on the in-process server (network
/// peers are numbered from 0).
const LOCAL: ConnId = ConnId(u32::MAX);

/// A status line every this many seconds of session time.
const REPORT_US: u64 = 60_000_000;

/// `--join` gives up on the `Welcome` and the first clock sample after
/// this long.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The network thread's period.
const PERIOD: Duration = Duration::from_millis(1);

type Socket = Endpoint<ImpairedTransport<UdpTransport>>;

/// How this client takes part.
#[derive(Clone, Debug)]
pub enum Mode {
    Solo,
    /// The in-process server, also on this UDP port.
    Host {
        port: u16,
    },
    /// Another machine's server.
    Join {
        addr: SocketAddr,
    },
}

/// Everything the network thread and the main thread share.
struct Inner {
    server: Option<Server>,
    socket: Option<Socket>,
    /// `--join`: the server's peer on the socket.
    server_peer: Option<PeerId>,
    client: Client,
    /// The session's time 0 (`None`: not started, the time is 0).
    epoch: Option<Instant>,
    /// Session time of the main thread's frame, microseconds.
    frame_now: u64,
    /// Log lines from the network thread, logged by the main thread.
    log: Vec<(Level, String)>,
    /// World events the drawn tick has reached, for the main thread.
    events: VecDeque<WorldEvent>,
    /// The connection to the server is gone (`--join`).
    lost: bool,
    stop: bool,
    to_server: u64,
    to_client: u64,
    next_report: u64,
}

impl Inner {
    fn now(&self) -> u64 {
        self.epoch.map_or(0, |e| e.elapsed().as_micros() as u64)
    }

    fn say(&mut self, level: Level, line: String) {
        self.log.push((level, line));
    }

    /// One pass: the socket, both state machines, the link between them,
    /// until nothing moves.
    fn pump(&mut self) {
        let now = self.now();
        if let Some(socket) = self.socket.as_mut() {
            socket.update(now);
        }
        while let Some(e) = self.socket.as_mut().and_then(Socket::poll_event) {
            match (e, self.server.as_mut()) {
                (Event::Connected(p), Some(server)) => server.connected(ConnId(p.0), now),
                (Event::Disconnected(p, r), Some(server)) => {
                    server.disconnected(ConnId(p.0), r == DisconnectReason::TimedOut, now);
                }
                (
                    Event::Reliable(p, b) | Event::Latest(p, _, b) | Event::Unordered(p, b),
                    Some(server),
                ) => server.received(ConnId(p.0), &b, now),
                (Event::Connected(_), None) => {
                    self.say(Level::INFO, "session: connected to the server".into());
                }
                (Event::Disconnected(_, r), None) => {
                    self.lost = true;
                    self.say(
                        Level::ERROR,
                        format!("session: lost the connection to the server ({r:?})"),
                    );
                }
                (Event::Reliable(_, b) | Event::Latest(_, _, b) | Event::Unordered(_, b), None) => {
                    self.to_client += 1;
                    self.client.received(&b, now);
                }
            }
        }
        self.client.update(now);
        if let Some(server) = self.server.as_mut() {
            server.update(now);
        }
        loop {
            let mut moved = false;
            for (channel, m) in self.client.take_outgoing() {
                moved = true;
                self.to_server += 1;
                match (self.server.as_mut(), self.socket.as_mut(), self.server_peer) {
                    (Some(server), _, _) => server.received(LOCAL, &m.encode(), now),
                    (None, Some(socket), Some(peer)) => {
                        let _ = send(socket, peer, channel, m.encode());
                    }
                    _ => {}
                }
            }
            let actions = self
                .server
                .as_mut()
                .map(Server::take_actions)
                .unwrap_or_default();
            for a in actions {
                match a {
                    Action::Send { to, message, .. } if to == LOCAL => {
                        moved = true;
                        self.to_client += 1;
                        self.client.received(&message.encode(), now);
                    }
                    Action::Send {
                        to,
                        channel,
                        message,
                    } => {
                        if let Some(socket) = self.socket.as_mut()
                            && let Err(e) = send(socket, PeerId(to.0), channel, message.encode())
                        {
                            self.say(Level::WARN, format!("session: send to {}: {e}", to.0));
                        }
                    }
                    Action::Close(c) if c == LOCAL => {
                        self.say(
                            Level::ERROR,
                            "session: the server closed the local client".into(),
                        );
                    }
                    Action::Close(c) => {
                        if let Some(socket) = self.socket.as_mut() {
                            socket.disconnect(PeerId(c.0));
                        }
                    }
                }
            }
            if !moved {
                break;
            }
        }
        if let Some(socket) = self.socket.as_mut() {
            socket.update(now);
        }
        self.notices();
        if now >= self.next_report && self.epoch.is_some() {
            self.next_report += REPORT_US;
            self.report(now);
        }
    }

    fn notices(&mut self) {
        let mut lines = Vec::new();
        for n in self
            .server
            .as_mut()
            .map(Server::take_notices)
            .unwrap_or_default()
        {
            lines.push(match n {
                Notice::Joined { conn, player, name } => (
                    Level::INFO,
                    format!(
                        "session: {name:?} joined as player {} from {}",
                        player.0,
                        if conn == LOCAL {
                            "this machine".to_string()
                        } else {
                            self.socket
                                .as_ref()
                                .and_then(|s| s.peer_addr(PeerId(conn.0)))
                                .map_or("?".into(), |a| a.to_string())
                        }
                    ),
                ),
                Notice::Ready { player } => {
                    (Level::INFO, format!("session: player {} ready", player.0))
                }
                Notice::Rejected { reason, .. } => (
                    Level::WARN,
                    format!("session: turned a player down: {reason}"),
                ),
                Notice::Left { player, reason } => (
                    Level::INFO,
                    format!("session: player {} left ({reason:?})", player.0),
                ),
                Notice::HashMismatch { player, events } => (
                    Level::ERROR,
                    format!(
                        "session: WORLD STATE DIFFERS: player {} after {events} events",
                        player.0
                    ),
                ),
                Notice::SlotFillMismatch { player, batch } => (
                    Level::ERROR,
                    format!(
                        "session: SLOT FILL DIFFERS: player {} in batch {batch:?}",
                        player.0
                    ),
                ),
                Notice::Implausible { player, speed } => (
                    Level::WARN,
                    format!("session: player {} moved at {speed:.1} m/s", player.0),
                ),
                Notice::Ignored { why, .. } => (
                    Level::WARN,
                    format!("session: server ignored a message: {why}"),
                ),
                Notice::Refused {
                    player,
                    req,
                    reason,
                } => (
                    Level::INFO,
                    format!(
                        "session: refused player {}'s request {req} ({reason:?})",
                        player.0
                    ),
                ),
            });
        }
        for n in self.client.take_notices() {
            lines.push(match n {
                ClientNotice::Welcomed { player } => {
                    let s = self.client.world().map(|w| w.settings);
                    (
                        Level::INFO,
                        format!("session: welcomed as player {}; world {s:?}", player.0),
                    )
                }
                ClientNotice::Rejected(r) => (Level::ERROR, format!("session: rejected: {r}")),
                ClientNotice::Apply(e) => (
                    Level::ERROR,
                    format!("session: world event does not apply: {e}"),
                ),
                ClientNotice::Ignored(why) => (
                    Level::WARN,
                    format!("session: client ignored a message: {why}"),
                ),
                ClientNotice::Loaded { bytes } => (
                    Level::INFO,
                    format!("session: world state loaded ({bytes} B)"),
                ),
                ClientNotice::Refused { req, reason } => (
                    Level::INFO,
                    format!("session: request {req} refused ({reason:?})"),
                ),
            });
        }
        self.log.extend(lines);
    }

    fn report(&mut self, now: u64) {
        let c = self.client.stats();
        let mut line = format!(
            "session: {:.0} s, tick {:.0}: {} messages to the server, {} back; {} world events; delay {:.1} ticks; {} hash reports; clock jumps {}; own steps skipped {}",
            now as f64 / 1e6,
            self.client.server_tick(now).unwrap_or(0.0),
            self.to_server,
            self.to_client,
            self.client.world().map_or(0, |w| w.events),
            self.client.delay(),
            c.hash_reports,
            c.clock_jumps,
            c.steps_skipped
        );
        if let Some(s) = self.server.as_ref().map(Server::stats) {
            line += &format!(
                "; server: {} hash checks ({} differ), {} states relayed",
                s.hash_checks, s.hash_mismatches, s.states_relayed
            );
        }
        if let Some(socket) = self.socket.as_ref() {
            for p in socket.peers() {
                if let Some(st) = socket.peer_stats(p) {
                    line += &format!(
                        "; peer {} {}: sent {} B, received {} B, rtt {}",
                        p.0,
                        socket.peer_addr(p).map_or("?".into(), |a| a.to_string()),
                        st.bytes_sent,
                        st.bytes_received,
                        socket
                            .rtt_us(p)
                            .map_or("-".into(), |r| format!("{:.1} ms", r / 1000.0))
                    );
                }
            }
        }
        self.say(Level::INFO, line);
    }
}

fn send(socket: &mut Socket, peer: PeerId, channel: Channel, bytes: Vec<u8>) -> Result<(), String> {
    match channel {
        Channel::Reliable => socket.send_reliable(peer, bytes),
        Channel::Latest(k) => socket.send_latest(peer, k, bytes),
        Channel::Unordered => socket.send_unordered(peer, bytes),
    }
    .map_err(|e| format!("{e:?}"))
}

fn socket(bind: (&str, u16), config: Config, sim: Option<Impairment>) -> Result<Socket, String> {
    let udp = UdpTransport::bind(bind)
        .map_err(|e| format!("can't use UDP {}:{}: {e}", bind.0, bind.1))?;
    Ok(Endpoint::new(
        ImpairedTransport::new(udp, sim.unwrap_or(Impairment::NONE), random_seed()),
        config,
    ))
}

#[derive(Resource)]
pub struct Session {
    inner: Arc<Mutex<Inner>>,
    settings: WorldSettings,
    tick_us: u64,
    /// Drawn ticks logged up to here, per remote player.
    logged: Vec<(PlayerId, u32)>,
    /// The world events the drawn tick reached this frame (for
    /// `crate::remote`).
    pub due: Vec<WorldEvent>,
    /// Remote players' drawn states and the own states are logged at
    /// every this many ticks (the same ticks on every machine, for
    /// comparing logs; `--log-ticks`, default 50).
    pub log_every: u32,
}

impl Session {
    /// Starts or joins the session. `config` is this install's server
    /// (used for solo and `--host`; for `--join` its build, fingerprint
    /// and tick go into the `Hello`). Blocks until the `Welcome` and the
    /// first clock sample (at most [`JOIN_TIMEOUT`] when joining).
    pub fn start(
        config: ServerConfig,
        mode: &Mode,
        name: &str,
        sim: Option<Impairment>,
    ) -> Result<Session, String> {
        let client = Client::new(
            ClientConfig {
                name: name.to_owned(),
                build: config.build.clone(),
                fingerprint: config.fingerprint,
                password: config.password.clone(),
                tick_us: config.tick_us,
                max_speed: config.max_speed,
                rules: config.rules.clone(),
            },
            0,
        );
        let tick_us = config.tick_us;
        let mut inner = Inner {
            server: None,
            socket: None,
            server_peer: None,
            client,
            epoch: None,
            frame_now: 0,
            log: Vec::new(),
            events: VecDeque::new(),
            lost: false,
            stop: false,
            to_server: 0,
            to_client: 0,
            next_report: REPORT_US,
        };
        match *mode {
            Mode::Solo => {
                inner.server = Some(Server::new(config, 0));
            }
            Mode::Host { port } => {
                let max = config.max_players;
                inner.server = Some(Server::new(config, 0));
                let s = socket(("0.0.0.0", port), Config::server(max, random_seed()), sim)?;
                inner.say(
                    Level::INFO,
                    format!(
                        "session: hosting on UDP {} (up to {max} players)",
                        s.local_addr().map_or("?".into(), |a| a.to_string())
                    ),
                );
                inner.socket = Some(s);
            }
            Mode::Join { addr } => {
                // The joiner's clock runs from now: the server's decides.
                inner.epoch = Some(Instant::now());
                let mut s = socket(("0.0.0.0", 0), Config::client(random_seed()), sim)?;
                inner.server_peer = Some(s.connect(addr, 0));
                inner.socket = Some(s);
            }
        }
        if let Some(server) = inner.server.as_mut() {
            server.connected(LOCAL, 0);
        }
        // Until welcomed with a clock (at once over the direct link).
        let started = Instant::now();
        loop {
            inner.pump();
            let now = inner.now();
            if inner.client.is_rejected() {
                let why = inner
                    .log
                    .iter()
                    .rev()
                    .find(|(l, _)| *l == Level::ERROR)
                    .map_or("rejected".into(), |(_, m)| m.clone());
                return Err(why);
            }
            if inner.lost {
                return Err(format!("could not reach the server at {mode:?}"));
            }
            if inner.client.world().is_some() && inner.client.server_us(now).is_some() {
                break;
            }
            if started.elapsed() > JOIN_TIMEOUT {
                return Err(format!(
                    "no answer from the server in {} s",
                    JOIN_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(PERIOD);
        }
        let Some(settings) = inner.client.world().map(|w| w.settings) else {
            return Err("no Welcome".into());
        };
        let inner = Arc::new(Mutex::new(inner));
        let shared = inner.clone();
        std::thread::Builder::new()
            .name("session".into())
            .spawn(move || {
                loop {
                    {
                        let Ok(mut i) = shared.lock() else { return };
                        if i.stop {
                            return;
                        }
                        i.pump();
                    }
                    std::thread::sleep(PERIOD);
                }
            })
            .map_err(|e| format!("can't start the network thread: {e}"))?;
        Ok(Session {
            inner,
            settings,
            tick_us,
            logged: Vec::new(),
            due: Vec::new(),
            log_every: 50,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic on the network thread poisons the lock; the state is
        // still the last one written, so go on with it.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The world's settings, as the `Welcome` gave them.
    pub fn settings(&self) -> WorldSettings {
        self.settings
    }

    /// One tick, seconds.
    pub fn tick_seconds(&self) -> f64 {
        self.tick_us as f64 / 1e6
    }

    /// The server's time this frame, seconds since its tick 0.
    pub fn server_seconds(&self) -> f64 {
        let i = self.lock();
        i.client.server_us(i.frame_now).unwrap_or(0.0) / 1e6
    }

    /// The server's tick this frame, with its fraction.
    pub fn server_tick(&self) -> f64 {
        let i = self.lock();
        i.client.server_tick(i.frame_now).unwrap_or(0.0)
    }

    /// The own player's physics steps due this frame, in the server's
    /// ticks (at most 10; the rest come in later frames).
    pub fn steps(&mut self) -> Range<u32> {
        let mut i = self.lock();
        let now = i.frame_now;
        i.client.steps(now)
    }

    /// The next step [`Session::steps`] will give.
    pub fn next_step(&self) -> u32 {
        self.lock().client.steps_taken()
    }

    /// The own player is loaded and placed.
    pub fn ready(&mut self) {
        self.lock().client.ready();
    }

    /// The own player's state after the step of its tick.
    pub fn push_state(&mut self, state: PlayerState) {
        self.lock().client.push_state(state);
    }

    /// Something happened to the own player at `tick`.
    pub fn event(&mut self, tick: u32, kind: PlayerEventKind) {
        info!("session: own event at tick {tick}: {kind:?}");
        self.lock().client.event(Tick(tick), kind);
    }

    /// The other players with a state: id, name, and whether they are
    /// in the world (joined and `Ready`; a player who left is not).
    pub fn remotes(&self) -> Vec<(PlayerId, String, bool)> {
        let i = self.lock();
        let Some(w) = i.client.world() else {
            return Vec::new();
        };
        i.client
            .remotes()
            .map(|id| match w.players.get(&id) {
                Some(p) => (id, p.name.clone(), p.ready),
                None => (id, "?".into(), false),
            })
            .collect()
    }

    /// The tick remote players are drawn at this frame.
    pub fn draw_tick(&self) -> Option<f64> {
        let i = self.lock();
        i.client.draw_tick(i.frame_now)
    }

    /// What to draw of a remote player at a (fractional) tick.
    pub fn sample(&self, player: PlayerId, at: f64) -> Option<RemoteSample> {
        self.lock().client.sample(player, at)
    }

    /// World events the drawn tick reached this frame, in order.
    pub fn take_events(&mut self) -> Vec<WorldEvent> {
        self.lock().events.drain(..).collect()
    }

    /// The own player's id (once welcomed).
    pub fn player(&self) -> Option<PlayerId> {
        self.lock().client.player()
    }

    /// Runs `f` on the shared items and the number of world events
    /// applied (`None` until the world state arrived, M9d3).
    pub fn with_items<R>(&self, f: impl FnOnce(&sn_session::ItemsState, u32) -> R) -> Option<R> {
        let i = self.lock();
        if !i.client.loaded() {
            return None;
        }
        let w = i.client.world()?;
        Some(f(&w.items, w.events))
    }

    /// Sends an item request (M9d3); its number.
    pub fn request(&mut self, request: sn_protocol::Request) -> u32 {
        let text = format!("{request:?}");
        let req = self.lock().client.request(request);
        // The answer is the world event with this `req`, or "request
        // {req} refused".
        info!("session: request {req}: {text}");
        req
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let mut i = self.lock();
        // A last report (runs shorter than its period print none).
        let now = i.now();
        i.report(now);
        for (_, line) in std::mem::take(&mut i.log) {
            info!("{line}");
        }
        i.client.bye();
        i.pump();
        i.stop = true;
    }
}

/// First in every frame: the session's time (it starts with the first
/// frame), the network thread's log lines, the world events now due.
pub fn update(session: Res<Session>) {
    let lines = {
        let mut i = session.lock();
        let epoch = *i.epoch.get_or_insert_with(Instant::now);
        i.frame_now = epoch.elapsed().as_micros() as u64;
        let now = i.frame_now;
        let due = i.client.take_events(now);
        i.events.extend(due);
        std::mem::take(&mut i.log)
    };
    for (level, line) in lines {
        match level {
            Level::ERROR => error!("{line}"),
            Level::WARN => warn!("{line}"),
            _ => info!("{line}"),
        }
    }
}

/// After the player: every remote player's drawn state at each logged
/// tick the drawn tick passed (`LOG_EVERY_TICKS`), and the world events
/// that came due, for comparing with the owners' own logs.
pub fn log_remotes(mut session: ResMut<Session>) {
    let due = session.take_events();
    for e in &due {
        let what = match &e.kind {
            WorldEventKind::Player { player, event } => format!("player {} {event:?}", player.0),
            other => format!("{other:?}"),
        };
        info!(
            "session: world event {} at tick {}: {what}",
            e.seq, e.tick.0
        );
    }
    session.due = due;
    let Some(draw) = session.draw_tick() else {
        return;
    };
    let draw = draw.max(0.0) as u32;
    let every = session.log_every;
    for (id, name, _) in session.remotes() {
        let from = match session.logged.iter().find(|(p, _)| *p == id) {
            Some(&(_, t)) => t,
            None => {
                // From the next logged tick on.
                let t = draw - draw % every;
                session.logged.push((id, t));
                t
            }
        };
        let mut t = from + every;
        while t <= draw {
            if let Some(s) = session.sample(id, f64::from(t)) {
                let p = s.position.map(|v| v as f32);
                info!(
                    "session: remote {} {name:?} tick {t}: at ({:?}, {:?}, {:?}), look ({:.2}, {:.2}), {:?}, life {:?}, cinematic {}, {:?}",
                    id.0,
                    p[0],
                    p[1],
                    p[2],
                    s.yaw,
                    s.pitch,
                    s.kind,
                    s.base.life,
                    s.base.cinematic,
                    s.base.anim.bools
                );
            }
            t += every;
        }
        if let Some(entry) = session.logged.iter_mut().find(|(p, _)| *p == id) {
            entry.1 = t - every;
        }
    }
}
