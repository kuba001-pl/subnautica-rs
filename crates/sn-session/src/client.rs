//! The client's half (M10d): the join, clock sync with the server, a tick
//! buffer per remote player and what to draw of it (interpolation,
//! bridging gaps, extrapolation), the world state from `Welcome` and the
//! events, events released in order at their tick, and sending the own
//! player's states (`docs/DESIGN.md` § M10 plan, "The timeline: ticks").
//! A state machine on the caller's clock, like [`crate::Server`].

use std::collections::{BTreeMap, VecDeque};

use sn_protocol::{
    ClientMessage, PROTOCOL_VERSION, PlayerEvent, PlayerEventKind, PlayerId, PlayerState,
    RejectReason, ServerMessage, Tick, WorldEvent, WorldEventKind, quant,
};

use crate::world::{ApplyError, WorldState};
use crate::{Channel, PING_KEY};

/// Clock: pings this often once joined (faster at first, to settle).
const PING_FAST_US: u64 = 50_000;
const PING_FAST_COUNT: u32 = 20;
const PING_US: u64 = 250_000;
/// Clock samples kept (by local time); the one with the shortest round
/// trip gives the offset.
const CLOCK_WINDOW_US: u64 = 10_000_000;
/// The applied offset moves towards the estimate by at most 1 % of the
/// time passed (the plan's slew), or jumps when off by more than this.
const CLOCK_JUMP_US: f64 = 250_000.0;
const CLOCK_SLEW: f64 = 0.01;
/// Interpolation delay: start, bounds, and how fast it may grow (10 % of
/// the time passed) or shrink (1 %), in ticks.
const DELAY_START: f64 = 5.0;
const DELAY_MIN: f64 = 2.0;
const DELAY_MAX: f64 = 50.0;
const DELAY_GROW: f64 = 0.10;
const DELAY_SHRINK: f64 = 0.01;
/// For this long after the first remote state the delay follows the
/// measured lateness at once (it only grows), then by its slew.
const DELAY_WARMUP_US: u64 = 1_000_000;
/// Lateness samples kept for the delay.
const LATENESS_WINDOW_US: u64 = 10_000_000;
/// The delay covers the latest arrival of a tick seen (each new tick's
/// lateness, which already includes waiting for the next packet and a
/// resend in the redundant copy) plus this many ticks.
const DELAY_MARGIN: f64 = 1.0;
/// Extrapolation past the newest state, then hold.
const EXTRAPOLATE_US: f64 = 250_000.0;
/// Own states: one message every this many ticks with the newest
/// `STATES_PER_MESSAGE`, so each tick travels in three messages (with
/// two, the sync oracle missed 0.1 % of ticks at 1 % loss: M10d).
const SEND_EVERY: u32 = 2;
const STATES_PER_MESSAGE: usize = 6;
/// World-state hash report every this many ticks.
const HASH_EVERY: u32 = 50;
/// States kept per remote player behind the drawn tick.
const KEEP_BEHIND: u32 = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct ClientConfig {
    pub name: String,
    pub build: String,
    pub fingerprint: u64,
    pub password: Option<String>,
    /// One tick, microseconds (the install's `fixedDeltaTime`).
    pub tick_us: u64,
    /// Faster than this between two states that are more than a tick
    /// apart is a teleport: not interpolated (metres per second).
    pub max_speed: f64,
}

/// How a drawn state was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    /// The state of exactly this tick.
    Exact,
    /// Between two consecutive received ticks.
    Interpolated,
    /// Across missing ticks: cubic with the received velocities.
    Bridged,
    /// Across missing ticks that contain a teleport: the earlier state.
    Snapped,
    /// Past the newest state, along its velocity (at most 250 ms).
    Extrapolated,
    /// Past the extrapolation limit, or before the oldest state.
    Held,
}

/// What to draw of a remote player at a (fractional) tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteSample {
    pub position: [f64; 3],
    /// Look, degrees (`rotationX` 0…360, `rotationY` up positive).
    pub yaw: f64,
    pub pitch: f64,
    /// The received state at or before the tick (flags, animator values,
    /// rotation); the nearest after it when there is none before.
    pub base: PlayerState,
    pub kind: SampleKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientNotice {
    Welcomed {
        player: PlayerId,
    },
    Rejected(RejectReason),
    /// The world state could not apply an event (a bug: the reliable
    /// channel delivers them in order).
    Apply(ApplyError),
    /// A message from the server that could not be decoded or made no
    /// sense now.
    Ignored(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientStats {
    pub pongs: u64,
    pub states_received: u64,
    /// States for a tick already in the buffer (redundant copies).
    pub states_duplicate: u64,
    pub events: u64,
    pub hash_reports: u64,
    pub clock_jumps: u64,
}

#[derive(Default)]
struct Remote {
    buffer: BTreeMap<u32, PlayerState>,
}

pub struct Client {
    config: ClientConfig,
    player: Option<PlayerId>,
    world: Option<WorldState>,
    rejected: bool,
    ready_sent: bool,
    remotes: BTreeMap<PlayerId, Remote>,
    // Clock.
    pings_sent: u32,
    last_ping: Option<u64>,
    samples: VecDeque<(u64, u64, f64)>,
    applied: Option<f64>,
    target: Option<f64>,
    last_clock_update: u64,
    // Interpolation delay, ticks.
    delay: f64,
    lateness: VecDeque<(u64, f64)>,
    /// When the first remote state arrived (the delay's warm-up).
    first_state_at: Option<u64>,
    last_delay_update: u64,
    // Events released by the drawn tick, in seq order.
    pending: VecDeque<WorldEvent>,
    // Own states.
    own: VecDeque<PlayerState>,
    own_pushed: u32,
    next_step: Option<u32>,
    last_hash_tick: Option<u32>,
    outgoing: Vec<(Channel, ClientMessage)>,
    notices: Vec<ClientNotice>,
    stats: ClientStats,
}

impl Client {
    /// A client that has just connected; its `Hello` is queued.
    pub fn new(config: ClientConfig, now_us: u64) -> Client {
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            build: config.build.clone(),
            fingerprint: config.fingerprint,
            name: config.name.clone(),
            password: config.password.clone(),
        };
        Client {
            config,
            player: None,
            world: None,
            rejected: false,
            ready_sent: false,
            remotes: BTreeMap::new(),
            pings_sent: 0,
            last_ping: None,
            samples: VecDeque::new(),
            applied: None,
            target: None,
            last_clock_update: now_us,
            delay: DELAY_START,
            lateness: VecDeque::new(),
            first_state_at: None,
            last_delay_update: now_us,
            pending: VecDeque::new(),
            own: VecDeque::new(),
            own_pushed: 0,
            next_step: None,
            last_hash_tick: None,
            outgoing: vec![(Channel::Reliable, hello)],
            notices: Vec::new(),
            stats: ClientStats::default(),
        }
    }

    pub fn player(&self) -> Option<PlayerId> {
        self.player
    }

    pub fn world(&self) -> Option<&WorldState> {
        self.world.as_ref()
    }

    pub fn stats(&self) -> ClientStats {
        self.stats
    }

    pub fn is_rejected(&self) -> bool {
        self.rejected
    }

    /// The interpolation delay, ticks.
    pub fn delay(&self) -> f64 {
        self.delay
    }

    pub fn take_outgoing(&mut self) -> Vec<(Channel, ClientMessage)> {
        std::mem::take(&mut self.outgoing)
    }

    pub fn take_notices(&mut self) -> Vec<ClientNotice> {
        std::mem::take(&mut self.notices)
    }

    /// The server's time estimate, microseconds since its tick 0.
    pub fn server_us(&self, now_us: u64) -> Option<f64> {
        self.applied.map(|o| now_us as f64 + o)
    }

    /// The server's tick at `now_us`, with its fraction.
    pub fn server_tick(&self, now_us: u64) -> Option<f64> {
        self.server_us(now_us)
            .map(|t| t / self.config.tick_us as f64)
    }

    /// The tick remote players are drawn at.
    pub fn draw_tick(&self, now_us: u64) -> Option<f64> {
        self.server_tick(now_us).map(|t| t - self.delay)
    }

    /// The own player's physics steps due by `now_us`, numbered in the
    /// server's ticks: consecutive, never repeated, at most 10 per call
    /// (after a clock jump forward the rest come in later calls; after
    /// one backward, none until the clock catches up).
    pub fn steps(&mut self, now_us: u64) -> std::ops::Range<u32> {
        let Some(t) = self.server_tick(now_us) else {
            return 0..0;
        };
        let due = t.floor().max(0.0) as u32;
        let next = *self.next_step.get_or_insert(due);
        if due < next {
            return next..next;
        }
        let end = (due + 1).min(next + 10);
        self.next_step = Some(end);
        next..end
    }

    /// Loaded and placed: others may see this player now.
    pub fn ready(&mut self) {
        if self.player.is_some() && !self.ready_sent {
            self.ready_sent = true;
            self.outgoing
                .push((Channel::Reliable, ClientMessage::Ready));
        }
    }

    /// The own player's state after a physics step; every second one
    /// sends the newest six.
    pub fn push_state(&mut self, state: PlayerState) {
        if self.own.back().is_some_and(|s| s.tick >= state.tick) {
            return;
        }
        self.own.push_back(state);
        if self.own.len() > STATES_PER_MESSAGE {
            self.own.pop_front();
        }
        self.own_pushed += 1;
        if self.ready_sent && self.own_pushed % SEND_EVERY == 0 {
            self.outgoing.push((
                Channel::Unordered,
                ClientMessage::States(self.own.iter().copied().collect()),
            ));
        }
    }

    /// Something happened to the own player.
    pub fn event(&mut self, tick: Tick, kind: PlayerEventKind) {
        self.outgoing.push((
            Channel::Reliable,
            ClientMessage::Event(PlayerEvent { tick, kind }),
        ));
    }

    /// Leaving.
    pub fn bye(&mut self) {
        self.outgoing.push((Channel::Reliable, ClientMessage::Bye));
    }

    /// Bytes from the server.
    pub fn received(&mut self, bytes: &[u8], now_us: u64) {
        let m = match ServerMessage::decode(bytes) {
            Ok(m) => m,
            Err(e) => {
                self.notices
                    .push(ClientNotice::Ignored(format!("undecodable: {e}")));
                return;
            }
        };
        match m {
            ServerMessage::Reject(r) => {
                self.rejected = true;
                self.notices.push(ClientNotice::Rejected(r));
            }
            ServerMessage::Welcome {
                player,
                next_seq,
                settings,
                players,
                ..
            } => {
                if self.player.is_some() {
                    self.notices
                        .push(ClientNotice::Ignored("second Welcome".into()));
                    return;
                }
                self.player = Some(player);
                self.world = Some(WorldState::from_welcome(settings, next_seq, &players));
                for p in &players {
                    let r = self.remotes.entry(p.id).or_default();
                    if let Some(s) = p.state {
                        r.buffer.insert(s.tick.0, s);
                    }
                }
                self.notices.push(ClientNotice::Welcomed { player });
            }
            ServerMessage::Pong {
                client_time,
                tick,
                fraction,
            } => {
                self.stats.pongs += 1;
                let rtt = now_us.saturating_sub(client_time);
                let tick_us = self.config.tick_us as f64;
                let server = f64::from(tick.0) * tick_us + f64::from(fraction) / 65_536.0 * tick_us;
                let offset = server + rtt as f64 / 2.0 - now_us as f64;
                self.samples.push_back((now_us, rtt, offset));
                while self
                    .samples
                    .front()
                    .is_some_and(|s| now_us.saturating_sub(s.0) > CLOCK_WINDOW_US)
                {
                    self.samples.pop_front();
                }
                self.target = self.samples.iter().min_by_key(|s| s.1).map(|s| s.2);
                if self.applied.is_none() {
                    self.applied = self.target;
                    self.last_clock_update = now_us;
                }
            }
            ServerMessage::States { player, states } => {
                if Some(player) == self.player {
                    return;
                }
                let now_tick = self.server_tick(now_us);
                self.first_state_at.get_or_insert(now_us);
                let r = self.remotes.entry(player).or_default();
                for s in states {
                    self.stats.states_received += 1;
                    if r.buffer.insert(s.tick.0, s).is_some() {
                        self.stats.states_duplicate += 1;
                    } else if let Some(t) = now_tick {
                        // How late this tick arrived: the delay must
                        // cover it for the tick to be drawn exactly.
                        self.lateness.push_back((now_us, t - f64::from(s.tick.0)));
                    }
                }
            }
            ServerMessage::Event(e) => {
                let Some(w) = self.world.as_mut() else {
                    self.notices
                        .push(ClientNotice::Ignored("event before Welcome".into()));
                    return;
                };
                if let Err(err) = w.apply(&e) {
                    self.notices.push(ClientNotice::Apply(err));
                    return;
                }
                self.stats.events += 1;
                if let WorldEventKind::PlayerJoined { player, .. } = &e.kind
                    && Some(*player) != self.player
                {
                    self.remotes.entry(*player).or_default();
                }
                self.pending.push_back(e);
            }
        }
    }

    /// Pings, the clock's slew, the delay, hash reports, pruning.
    pub fn update(&mut self, now_us: u64) {
        if self.player.is_none() {
            return;
        }
        let every = if self.pings_sent < PING_FAST_COUNT {
            PING_FAST_US
        } else {
            PING_US
        };
        if self
            .last_ping
            .is_none_or(|t| now_us.saturating_sub(t) >= every)
        {
            self.last_ping = Some(now_us);
            self.pings_sent += 1;
            self.outgoing.push((
                Channel::Latest(PING_KEY),
                ClientMessage::Ping {
                    client_time: now_us,
                },
            ));
        }
        // Clock slew.
        let dt = now_us.saturating_sub(self.last_clock_update) as f64;
        self.last_clock_update = now_us;
        if let (Some(applied), Some(target)) = (self.applied, self.target) {
            let diff = target - applied;
            if diff.abs() > CLOCK_JUMP_US {
                self.applied = Some(target);
                self.stats.clock_jumps += 1;
            } else {
                let step = CLOCK_SLEW * dt;
                self.applied = Some(applied + diff.clamp(-step, step));
            }
        }
        // Delay.
        while self
            .lateness
            .front()
            .is_some_and(|s| now_us.saturating_sub(s.0) > LATENESS_WINDOW_US)
        {
            self.lateness.pop_front();
        }
        let ddt = now_us.saturating_sub(self.last_delay_update) as f64 / self.config.tick_us as f64;
        self.last_delay_update = now_us;
        if let Some(late) = self.lateness.iter().map(|s| s.1).reduce(f64::max) {
            let want = (late + DELAY_MARGIN).clamp(DELAY_MIN, DELAY_MAX);
            let warming = self
                .first_state_at
                .is_some_and(|t| now_us.saturating_sub(t) < DELAY_WARMUP_US);
            if warming {
                // Nothing has been on screen for long: take the measured
                // delay at once.
                self.delay = self.delay.max(want);
            } else {
                let diff = want - self.delay;
                let step = if diff > 0.0 { DELAY_GROW } else { DELAY_SHRINK } * ddt;
                self.delay += diff.clamp(-step, step);
            }
        }
        // Hash report.
        if let (Some(t), Some(w)) = (self.server_tick(now_us), self.world.as_ref()) {
            let bucket = (t.max(0.0) as u32) / HASH_EVERY;
            if self.last_hash_tick != Some(bucket) {
                self.last_hash_tick = Some(bucket);
                self.stats.hash_reports += 1;
                self.outgoing.push((
                    Channel::Reliable,
                    ClientMessage::WorldHash {
                        events: w.events,
                        hash: w.hash(),
                    },
                ));
            }
        }
        // Prune what is far behind the drawn tick.
        if let Some(d) = self.draw_tick(now_us) {
            let keep_from = (d.max(0.0) as u32).saturating_sub(KEEP_BEHIND);
            for r in self.remotes.values_mut() {
                // Keep at least the newest state.
                while r.buffer.len() > 1
                    && r.buffer
                        .first_key_value()
                        .is_some_and(|(&t, _)| t < keep_from)
                {
                    r.buffer.pop_first();
                }
            }
        }
        if let Some(w) = self.world.as_ref() {
            self.remotes.retain(|id, _| {
                w.players.contains_key(id) || self.pending.iter().any(|e| left(e, *id))
            });
        }
    }

    /// World events whose tick the drawn tick has reached, in `seq`
    /// order (one that is not due yet holds back those after it).
    pub fn take_events(&mut self, now_us: u64) -> Vec<WorldEvent> {
        let Some(d) = self.draw_tick(now_us) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while self
            .pending
            .front()
            .is_some_and(|e| f64::from(e.tick.0) <= d)
        {
            out.extend(self.pending.pop_front());
        }
        out
    }

    /// Remote players with any state.
    pub fn remotes(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.remotes
            .iter()
            .filter(|(_, r)| !r.buffer.is_empty())
            .map(|(&id, _)| id)
    }

    /// Whether the state of exactly `tick` has arrived.
    pub fn has_state(&self, player: PlayerId, tick: u32) -> bool {
        self.remotes
            .get(&player)
            .is_some_and(|r| r.buffer.contains_key(&tick))
    }

    /// What to draw of `player` at the (fractional) tick `at`.
    pub fn sample(&self, player: PlayerId, at: f64) -> Option<RemoteSample> {
        let r = self.remotes.get(&player)?;
        let floor = at.floor().max(0.0) as u32;
        let a = r.buffer.range(..=floor).next_back().map(|(_, s)| *s);
        let b = r
            .buffer
            .range(floor.saturating_add(1)..)
            .next()
            .map(|(_, s)| *s);
        let tick_s = self.config.tick_us as f64 / 1e6;
        Some(match (a, b) {
            (Some(a), _) if f64::from(a.tick.0) == at => plain(a, SampleKind::Exact),
            (Some(a), Some(b)) => {
                let span = f64::from(b.tick.0 - a.tick.0);
                let u = (at - f64::from(a.tick.0)) / span;
                let (pa, pb) = (pos(&a), pos(&b));
                if span == 1.0 {
                    mix(a, b, u, lerp3(pa, pb, u), SampleKind::Interpolated)
                } else if dist(pa, pb) > self.config.max_speed * span * tick_s + 1.0 {
                    plain(a, SampleKind::Snapped)
                } else {
                    let d = span * tick_s;
                    let p = hermite(pa, vel(&a), pb, vel(&b), u, d);
                    mix(a, b, u, p, SampleKind::Bridged)
                }
            }
            (Some(a), None) => {
                let ahead = (at - f64::from(a.tick.0)) * tick_s;
                let limit = EXTRAPOLATE_US / 1e6;
                let (k, s) = if ahead <= limit {
                    (SampleKind::Extrapolated, ahead)
                } else {
                    (SampleKind::Held, limit)
                };
                let p = pos(&a);
                let v = vel(&a);
                let mut out = plain(a, k);
                out.position = [p[0] + v[0] * s, p[1] + v[1] * s, p[2] + v[2] * s];
                out
            }
            (None, Some(b)) => plain(b, SampleKind::Held),
            (None, None) => return None,
        })
    }
}

fn left(e: &WorldEvent, id: PlayerId) -> bool {
    matches!(e.kind, WorldEventKind::PlayerLeft { player, .. } if player == id)
}

fn pos(s: &PlayerState) -> [f64; 3] {
    s.position.map(f64::from)
}

fn vel(s: &PlayerState) -> [f64; 3] {
    s.velocity.map(quant::velocity_back)
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn lerp3(a: [f64; 3], b: [f64; 3], u: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * u)
}

/// Cubic Hermite between two points with velocities, over `d` seconds.
fn hermite(p0: [f64; 3], v0: [f64; 3], p1: [f64; 3], v1: [f64; 3], u: f64, d: f64) -> [f64; 3] {
    let (u2, u3) = (u * u, u * u * u);
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    [0, 1, 2].map(|i| h00 * p0[i] + h10 * d * v0[i] + h01 * p1[i] + h11 * d * v1[i])
}

fn plain(s: PlayerState, kind: SampleKind) -> RemoteSample {
    RemoteSample {
        position: pos(&s),
        yaw: quant::turn_back(s.yaw),
        pitch: quant::fixed_back(s.pitch, quant::PITCH_SCALE),
        base: s,
        kind,
    }
}

/// `a`'s fields with position `p` and the look between `a` and `b`.
fn mix(a: PlayerState, b: PlayerState, u: f64, p: [f64; 3], kind: SampleKind) -> RemoteSample {
    let ya = quant::turn_back(a.yaw);
    let yb = quant::turn_back(b.yaw);
    let dy = (yb - ya + 180.0).rem_euclid(360.0) - 180.0;
    let pa = quant::fixed_back(a.pitch, quant::PITCH_SCALE);
    let pb = quant::fixed_back(b.pitch, quant::PITCH_SCALE);
    RemoteSample {
        position: p,
        yaw: (ya + dy * u).rem_euclid(360.0),
        pitch: pa + (pb - pa) * u,
        base: a,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_protocol::{AnimState, Life, WorldSettings};

    fn config() -> ClientConfig {
        ClientConfig {
            name: "b".into(),
            build: "10".into(),
            fingerprint: 1,
            password: None,
            tick_us: 20_000,
            max_speed: 30.0,
        }
    }

    fn state(tick: u32, p: [f32; 3], v: [f64; 3]) -> PlayerState {
        PlayerState {
            tick: Tick(tick),
            position: p,
            velocity: v.map(quant::velocity),
            yaw: quant::turn(350.0 + f64::from(tick)),
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
            anim: AnimState::default(),
        }
    }

    fn welcomed() -> Client {
        let mut c = Client::new(config(), 0);
        let welcome = ServerMessage::Welcome {
            player: PlayerId(1),
            tick: Tick(0),
            next_seq: 0,
            settings: WorldSettings {
                slot_seed: None,
                lifepod: None,
                aurora: sn_protocol::AuroraStart::Held { exploded: false },
                clock_start: 0.0,
                time_scale: 1.0,
            },
            players: vec![],
        };
        c.received(&welcome.encode(), 0);
        c
    }

    fn give(c: &mut Client, states: Vec<PlayerState>, now: u64) {
        let m = ServerMessage::States {
            player: PlayerId(0),
            states,
        };
        c.received(&m.encode(), now);
    }

    #[test]
    fn the_shortest_round_trip_sets_the_clock_and_it_slews() {
        let mut c = welcomed();
        let pong = |client_time: u64, server_us: u64| ServerMessage::Pong {
            client_time,
            tick: Tick((server_us / 20_000) as u32),
            fraction: ((server_us % 20_000) * 65_536 / 20_000) as u16,
        };
        // The server is 1 s ahead. A slow sample first (asymmetric: 90 ms
        // out, 10 ms back), then a fast symmetric one.
        // Round trip 100 ms, so it guesses 50 ms each way: 40 ms early.
        c.received(&pong(1_000_000, 2_090_000).encode(), 1_100_000);
        assert_eq!(c.server_us(1_100_000), Some(2_140_000.0));
        c.received(&pong(1_200_000, 2_205_000).encode(), 1_210_000);
        // The target is right, the applied offset moves 1 % of the time.
        c.update(1_210_000);
        let before = c.server_us(1_210_000).unwrap();
        c.update(1_310_000);
        let after = c.server_us(1_310_000).unwrap() - 100_000.0;
        assert!((before - after - 1_000.0).abs() < 1e-6, "{before} {after}");
        c.update(10_000_000);
        assert!((c.server_us(10_000_000).unwrap() - 11_000_000.0).abs() < 1e-6);
        assert_eq!(c.stats().clock_jumps, 0);
        // Steps are consecutive server ticks.
        assert_eq!(c.steps(10_000_000), 550..551);
        assert_eq!(c.steps(10_050_000), 551..553);
        assert_eq!(c.steps(10_050_000), 553..553);
    }

    #[test]
    fn samples_interpolate_bridge_snap_and_extrapolate() {
        let mut c = welcomed();
        let v = [7.6, 0.0, 0.0];
        // Ticks 10, 11 and 14 (12, 13 missing): constant speed.
        let p = |t: u32| [(f64::from(t) * 0.152) as f32, -5.0, 0.0];
        give(
            &mut c,
            vec![
                state(10, p(10), v),
                state(11, p(11), v),
                state(14, p(14), v),
            ],
            0,
        );
        let pid = PlayerId(0);
        let s = c.sample(pid, 11.0).unwrap();
        assert_eq!((s.kind, s.base.tick), (SampleKind::Exact, Tick(11)));
        let s = c.sample(pid, 10.5).unwrap();
        assert_eq!(s.kind, SampleKind::Interpolated);
        assert!((s.position[0] - 10.5 * 0.152).abs() < 1e-5);
        // Yaw 360 → 361 crosses 0 the short way.
        assert!(
            (s.yaw - 0.5).abs() < 0.01 || (s.yaw - 360.5).abs() < 0.01,
            "{}",
            s.yaw
        );
        let s = c.sample(pid, 12.0).unwrap();
        assert_eq!(s.kind, SampleKind::Bridged);
        assert!(
            (s.position[0] - 12.0 * 0.152).abs() < 1e-4,
            "{:?}",
            s.position
        );
        let s = c.sample(pid, 16.0).unwrap();
        assert_eq!(s.kind, SampleKind::Extrapolated);
        assert!((s.position[0] - 16.0 * 0.152).abs() < 1e-4);
        let s = c.sample(pid, 40.0).unwrap();
        assert_eq!(s.kind, SampleKind::Held);
        assert!((s.position[0] - (14.0 * 0.152 + 7.6 * 0.25)).abs() < 1e-4);
        assert_eq!(c.sample(pid, 5.0).unwrap().kind, SampleKind::Held);
        // A 30 m jump across a gap is a teleport.
        give(&mut c, vec![state(20, [100.0, 0.0, 0.0], v)], 0);
        assert_eq!(c.sample(pid, 17.0).unwrap().kind, SampleKind::Snapped);
        assert!(c.has_state(pid, 14) && !c.has_state(pid, 13));
    }

    #[test]
    fn events_wait_for_their_tick_in_order() {
        let mut c = welcomed();
        // Clock: server tick 100 at local 0.
        c.received(
            &ServerMessage::Pong {
                client_time: 0,
                tick: Tick(100),
                fraction: 0,
            }
            .encode(),
            0,
        );
        for (seq, tick) in [(0u32, 90u32), (1, 99), (2, 94)] {
            let kind = WorldEventKind::PlayerJoined {
                player: PlayerId(10 + seq as u16),
                name: format!("n{seq}"),
            };
            c.received(
                &ServerMessage::Event(WorldEvent {
                    seq,
                    tick: Tick(tick),
                    kind,
                })
                .encode(),
                0,
            );
        }
        assert_eq!(c.world().unwrap().events, 3);
        // Drawn at 95 (delay 5): seq 0 is due, seq 1 (99) holds back seq 2.
        let got: Vec<u32> = c.take_events(0).iter().map(|e| e.seq).collect();
        assert_eq!(got, [0]);
        let got: Vec<u32> = c.take_events(80_000).iter().map(|e| e.seq).collect();
        assert_eq!(got, [1, 2]);
    }

    #[test]
    fn own_states_go_every_second_tick_once_ready() {
        let mut c = welcomed();
        c.take_outgoing();
        c.push_state(state(1, [0.0; 3], [0.0; 3]));
        c.push_state(state(2, [0.0; 3], [0.0; 3]));
        assert!(c.take_outgoing().is_empty(), "not ready");
        c.ready();
        for t in 3..=8 {
            c.push_state(state(t, [0.0; 3], [0.0; 3]));
        }
        let sent: Vec<Vec<u32>> = c
            .take_outgoing()
            .into_iter()
            .filter_map(|(_, m)| match m {
                ClientMessage::States(s) => Some(s.iter().map(|s| s.tick.0).collect()),
                _ => None,
            })
            .collect();
        assert_eq!(
            sent,
            [
                vec![1, 2, 3, 4],
                vec![1, 2, 3, 4, 5, 6],
                vec![3, 4, 5, 6, 7, 8]
            ]
        );
    }
}
