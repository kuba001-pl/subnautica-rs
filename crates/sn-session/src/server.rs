//! The server's logic (M10c): join checks and player ids, the clock,
//! relaying states, ordering events into the world event log, the
//! world-state and slot-fill checks. A state machine: the caller feeds it
//! connections, received bytes and the time, and sends what it returns.
//! The same code runs in `sn-server` and inside the client (solo and
//! `--host`).

use std::collections::{BTreeMap, HashMap, VecDeque};

use sn_protocol::{
    ClientMessage, LeaveReason, MAX_PLAYERS, PROTOCOL_VERSION, PlayerEvent, PlayerId, PlayerInfo,
    PlayerState, RejectReason, ServerMessage, Tick, WorldEvent, WorldEventKind, WorldSettings,
};

use crate::world::WorldState;
use crate::{Channel, ConnId, PING_KEY};

/// A connection that has not said `Hello` within this long is closed.
pub const HELLO_TIMEOUT_US: u64 = 5_000_000;
/// A rejected connection is closed this long after its `Reject`, so the
/// answer gets through first.
pub const REJECT_LINGER_US: u64 = 1_000_000;
/// States further ahead of the server's clock than this are dropped.
pub const MAX_STATE_LEAD_US: u64 = 5_000_000;
/// World-state hashes kept for checking clients' reports, by event count.
const HASH_HISTORY: usize = 4096;

/// What the server is told at start (`apps/sn-server` or the client).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// The host's game build (`__buildnumber.txt`).
    pub build: String,
    /// The rules fingerprint of the host's install.
    pub fingerprint: u64,
    pub password: Option<String>,
    /// At most [`MAX_PLAYERS`].
    pub max_players: usize,
    pub settings: WorldSettings,
    /// One tick, microseconds (`Time.fixedDeltaTime` from the install).
    pub tick_us: u64,
    /// Speeds above this between two states of a player are logged as
    /// implausible (metres per second; not rejected, `docs/DESIGN.md`
    /// § 9, decision 6).
    pub max_speed: f64,
}

/// What the caller does after feeding the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Send {
        to: ConnId,
        channel: Channel,
        message: ServerMessage,
    },
    /// Disconnect this connection (its messages are no longer wanted).
    Close(ConnId),
}

/// Things worth a log line.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Joined {
        conn: ConnId,
        player: PlayerId,
        name: String,
    },
    Rejected {
        conn: ConnId,
        reason: RejectReason,
    },
    Ready {
        player: PlayerId,
    },
    Left {
        player: PlayerId,
        reason: LeaveReason,
    },
    /// A client's world-state hash differs from the server's after the
    /// same number of events.
    HashMismatch {
        player: PlayerId,
        events: u32,
    },
    /// Two clients filled a batch's spawn slots differently.
    SlotFillMismatch {
        player: PlayerId,
        batch: [i32; 3],
    },
    Implausible {
        player: PlayerId,
        speed: f64,
    },
    /// A message that could not be decoded or made no sense now.
    Ignored {
        conn: ConnId,
        why: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerStats {
    pub joins: u64,
    pub rejects: u64,
    pub leaves: u64,
    pub states_relayed: u64,
    pub states_dropped: u64,
    pub events: u64,
    pub hash_checks: u64,
    pub hash_mismatches: u64,
    /// Reports for event counts no longer (or not yet) known.
    pub hash_unchecked: u64,
    pub slot_fill_checks: u64,
    pub slot_fill_mismatches: u64,
    pub implausible_moves: u64,
    pub ignored_messages: u64,
}

enum Stage {
    AwaitingHello { since: u64 },
    Rejected { at: u64 },
    Joined { player: PlayerId },
}

struct Joined {
    conn: ConnId,
    /// Newest state received (by tick).
    state: Option<PlayerState>,
}

pub struct Server {
    config: ServerConfig,
    start_us: u64,
    conns: BTreeMap<ConnId, Stage>,
    players: BTreeMap<PlayerId, Joined>,
    /// A name keeps its id for the server's life (a rejoin is the same
    /// player).
    ids: HashMap<String, PlayerId>,
    next_id: u16,
    world: WorldState,
    /// Hashes after each event count, the newest last.
    hashes: VecDeque<(u32, u64)>,
    slot_fills: HashMap<[i32; 3], u64>,
    actions: Vec<Action>,
    notices: Vec<Notice>,
    stats: ServerStats,
}

impl Server {
    /// A new world whose tick 0 is `now_us` on the caller's clock.
    pub fn new(config: ServerConfig, now_us: u64) -> Server {
        let world = WorldState::new(config.settings);
        let mut hashes = VecDeque::new();
        hashes.push_back((0, world.hash()));
        Server {
            config,
            start_us: now_us,
            conns: BTreeMap::new(),
            players: BTreeMap::new(),
            ids: HashMap::new(),
            next_id: 0,
            world,
            hashes,
            slot_fills: HashMap::new(),
            actions: Vec::new(),
            notices: Vec::new(),
            stats: ServerStats::default(),
        }
    }

    pub fn world(&self) -> &WorldState {
        &self.world
    }

    pub fn stats(&self) -> ServerStats {
        self.stats
    }

    /// The server's tick at `now_us`, and the fraction of the next one in
    /// 1/65,536.
    pub fn clock(&self, now_us: u64) -> (Tick, u16) {
        let t = now_us.saturating_sub(self.start_us);
        let tick = t / self.config.tick_us;
        let fraction = (t % self.config.tick_us) * 65_536 / self.config.tick_us;
        (Tick(tick.min(u64::from(u32::MAX)) as u32), fraction as u16)
    }

    /// What to send and close since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    fn send(&mut self, to: ConnId, channel: Channel, message: ServerMessage) {
        self.actions.push(Action::Send {
            to,
            channel,
            message,
        });
    }

    fn ignore(&mut self, conn: ConnId, why: impl Into<String>) {
        self.stats.ignored_messages += 1;
        self.notices.push(Notice::Ignored {
            conn,
            why: why.into(),
        });
    }

    /// Appends an event to the log, applies it and sends it to every
    /// joined client.
    fn emit(&mut self, tick: Tick, kind: WorldEventKind) {
        let e = WorldEvent {
            seq: self.world.events,
            tick,
            kind,
        };
        // The server makes every event itself, so it always fits.
        if let Err(err) = self.world.apply(&e) {
            debug_assert!(false, "server event does not apply: {err}");
            return;
        }
        self.stats.events += 1;
        self.hashes
            .push_back((self.world.events, self.world.hash()));
        if self.hashes.len() > HASH_HISTORY {
            self.hashes.pop_front();
        }
        let to: Vec<ConnId> = self.players.values().map(|p| p.conn).collect();
        for conn in to {
            self.send(conn, Channel::Reliable, ServerMessage::Event(e.clone()));
        }
    }

    /// A transport connection opened.
    pub fn connected(&mut self, conn: ConnId, now_us: u64) {
        self.conns
            .insert(conn, Stage::AwaitingHello { since: now_us });
    }

    /// A transport connection closed (timed out, or the peer said
    /// goodbye at the transport level).
    pub fn disconnected(&mut self, conn: ConnId, timed_out: bool, now_us: u64) {
        if let Some(Stage::Joined { player }) = self.conns.remove(&conn) {
            let reason = if timed_out {
                LeaveReason::TimedOut
            } else {
                LeaveReason::Left
            };
            self.leave(player, reason, now_us);
        }
    }

    fn leave(&mut self, player: PlayerId, reason: LeaveReason, now_us: u64) {
        if self.players.remove(&player).is_none() {
            return;
        }
        self.stats.leaves += 1;
        self.notices.push(Notice::Left { player, reason });
        let tick = self.clock(now_us).0;
        self.emit(tick, WorldEventKind::PlayerLeft { player, reason });
    }

    /// Hello timeouts and closing rejected connections.
    pub fn update(&mut self, now_us: u64) {
        let mut close = Vec::new();
        for (&conn, stage) in &self.conns {
            match *stage {
                Stage::AwaitingHello { since }
                    if now_us.saturating_sub(since) >= HELLO_TIMEOUT_US =>
                {
                    close.push(conn)
                }
                Stage::Rejected { at } if now_us.saturating_sub(at) >= REJECT_LINGER_US => {
                    close.push(conn)
                }
                _ => {}
            }
        }
        for conn in close {
            self.conns.remove(&conn);
            self.actions.push(Action::Close(conn));
        }
    }

    /// Bytes arrived from a connection.
    pub fn received(&mut self, conn: ConnId, bytes: &[u8], now_us: u64) {
        let message = match ClientMessage::decode(bytes) {
            Ok(m) => m,
            Err(e) => {
                self.ignore(conn, format!("undecodable: {e}"));
                return;
            }
        };
        let player = match self.conns.get(&conn) {
            None => {
                self.ignore(conn, "unknown connection");
                return;
            }
            Some(Stage::Rejected { .. }) => return,
            Some(Stage::AwaitingHello { .. }) => {
                if let ClientMessage::Hello {
                    protocol,
                    build,
                    fingerprint,
                    name,
                    password,
                } = message
                {
                    self.hello(conn, protocol, build, fingerprint, name, password, now_us);
                } else {
                    self.ignore(conn, "message before Hello");
                }
                return;
            }
            Some(Stage::Joined { player }) => *player,
        };
        match message {
            ClientMessage::Hello { .. } => self.ignore(conn, "second Hello"),
            ClientMessage::Ready => {
                let ready = self.world.players.get(&player).is_some_and(|p| p.ready);
                if ready {
                    self.ignore(conn, "second Ready");
                } else {
                    self.notices.push(Notice::Ready { player });
                    let tick = self.clock(now_us).0;
                    self.emit(tick, WorldEventKind::PlayerReady { player });
                }
            }
            ClientMessage::Ping { client_time } => {
                let (tick, fraction) = self.clock(now_us);
                self.send(
                    conn,
                    Channel::Latest(PING_KEY),
                    ServerMessage::Pong {
                        client_time,
                        tick,
                        fraction,
                    },
                );
            }
            ClientMessage::States(states) => self.states(conn, player, states, now_us),
            ClientMessage::Event(e) => self.player_event(conn, player, e),
            ClientMessage::SlotFillHash { batch, hash } => {
                self.stats.slot_fill_checks += 1;
                match self.slot_fills.get(&batch) {
                    Some(&first) if first != hash => {
                        self.stats.slot_fill_mismatches += 1;
                        self.notices
                            .push(Notice::SlotFillMismatch { player, batch });
                    }
                    Some(_) => {}
                    None => {
                        self.slot_fills.insert(batch, hash);
                    }
                }
            }
            ClientMessage::WorldHash { events, hash } => {
                match self.hashes.iter().find(|(n, _)| *n == events) {
                    Some(&(_, ours)) => {
                        self.stats.hash_checks += 1;
                        if ours != hash {
                            self.stats.hash_mismatches += 1;
                            self.notices.push(Notice::HashMismatch { player, events });
                        }
                    }
                    None => self.stats.hash_unchecked += 1,
                }
            }
            ClientMessage::Bye => {
                self.conns.remove(&conn);
                self.leave(player, LeaveReason::Left, now_us);
                self.actions.push(Action::Close(conn));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn hello(
        &mut self,
        conn: ConnId,
        protocol: u16,
        build: String,
        fingerprint: u64,
        name: String,
        password: Option<String>,
        now_us: u64,
    ) {
        let name_taken = self
            .world
            .players
            .values()
            .any(|p| p.name.eq_ignore_ascii_case(&name));
        let reason = if protocol != PROTOCOL_VERSION {
            Some(RejectReason::Protocol {
                server: PROTOCOL_VERSION,
            })
        } else if build != self.config.build {
            Some(RejectReason::Build {
                server: self.config.build.clone(),
            })
        } else if fingerprint != self.config.fingerprint {
            Some(RejectReason::Fingerprint)
        } else if self.config.password.is_some() && password != self.config.password {
            Some(RejectReason::Password)
        } else if self.players.len() >= self.config.max_players.min(MAX_PLAYERS) {
            Some(RejectReason::Full {
                max: self.config.max_players.min(MAX_PLAYERS) as u16,
            })
        } else if name.trim().is_empty()
            || name.trim() != name
            || name.chars().any(char::is_control)
            || name_taken
            || (!self.ids.contains_key(&name) && self.next_id == u16::MAX)
        {
            Some(RejectReason::Name)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.stats.rejects += 1;
            self.conns.insert(conn, Stage::Rejected { at: now_us });
            self.notices.push(Notice::Rejected {
                conn,
                reason: reason.clone(),
            });
            self.send(conn, Channel::Reliable, ServerMessage::Reject(reason));
            return;
        }
        let player = match self.ids.get(&name) {
            Some(&id) => id,
            None => {
                let id = PlayerId(self.next_id);
                self.next_id += 1;
                self.ids.insert(name.clone(), id);
                id
            }
        };
        // The snapshot after `world.events` events; the join is the next.
        let players: Vec<PlayerInfo> = self
            .world
            .players
            .iter()
            .map(|(id, p)| PlayerInfo {
                id: *id,
                name: p.name.clone(),
                ready: p.ready,
                state: self.players.get(id).and_then(|j| j.state),
                cinematic: p.cinematic,
            })
            .collect();
        let (tick, _) = self.clock(now_us);
        self.send(
            conn,
            Channel::Reliable,
            ServerMessage::Welcome {
                player,
                tick,
                next_seq: self.world.events,
                settings: self.config.settings,
                players,
            },
        );
        self.conns.insert(conn, Stage::Joined { player });
        self.players.insert(player, Joined { conn, state: None });
        self.stats.joins += 1;
        self.notices.push(Notice::Joined {
            conn,
            player,
            name: name.clone(),
        });
        self.emit(tick, WorldEventKind::PlayerJoined { player, name });
    }

    fn states(&mut self, conn: ConnId, player: PlayerId, states: Vec<PlayerState>, now_us: u64) {
        let ready = self.world.players.get(&player).is_some_and(|p| p.ready);
        if !ready {
            self.stats.states_dropped += states.len() as u64;
            self.ignore(conn, "states before Ready");
            return;
        }
        let lead_ticks = MAX_STATE_LEAD_US / self.config.tick_us;
        let limit = u64::from(self.clock(now_us).0.0) + lead_ticks;
        let (kept, ahead): (Vec<PlayerState>, Vec<PlayerState>) = states
            .into_iter()
            .partition(|s| u64::from(s.tick.0) <= limit);
        if !ahead.is_empty() {
            self.stats.states_dropped += ahead.len() as u64;
            self.ignore(conn, format!("{} states too far ahead", ahead.len()));
        }
        let Some(newest) = kept.last().copied() else {
            return;
        };
        let tick_s = self.config.tick_us as f64 / 1e6;
        let mut previous = self.players.get(&player).and_then(|j| j.state);
        for s in &kept {
            if let Some(p) = previous.filter(|p| s.tick > p.tick) {
                let d: f64 = (0..3)
                    .map(|i| (f64::from(s.position[i]) - f64::from(p.position[i])).powi(2))
                    .sum::<f64>()
                    .sqrt();
                let speed = d / (f64::from(s.tick.0 - p.tick.0) * tick_s);
                if speed > self.config.max_speed && !s.cinematic && !p.cinematic {
                    self.stats.implausible_moves += 1;
                    self.notices.push(Notice::Implausible { player, speed });
                }
            }
            if previous.is_none_or(|p| s.tick > p.tick) {
                previous = Some(*s);
            }
        }
        if let Some(j) = self.players.get_mut(&player)
            && j.state.is_none_or(|old| newest.tick > old.tick)
        {
            j.state = Some(newest);
        }
        let to: Vec<ConnId> = self
            .players
            .values()
            .map(|p| p.conn)
            .filter(|&c| c != conn)
            .collect();
        for c in to {
            self.stats.states_relayed += 1;
            self.send(
                c,
                Channel::Unordered,
                ServerMessage::States {
                    player,
                    states: kept.clone(),
                },
            );
        }
    }

    fn player_event(&mut self, conn: ConnId, player: PlayerId, e: PlayerEvent) {
        let ready = self.world.players.get(&player).is_some_and(|p| p.ready);
        if !ready {
            self.ignore(conn, "event before Ready");
            return;
        }
        self.emit(
            e.tick,
            WorldEventKind::Player {
                player,
                event: e.kind,
            },
        );
    }
}
