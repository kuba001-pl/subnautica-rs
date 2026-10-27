//! The server's logic (M10c): join checks and player ids, the clock,
//! relaying states, ordering events into the world event log, the
//! world-state and slot-fill checks; from M9d3c the item requests (checked
//! by applying them to its own state) and the world state's snapshot for
//! a joiner. A state machine: the caller feeds it
//! connections, received bytes and the time, and sends what it returns.
//! The same code runs in `sn-server` and inside the client (solo and
//! `--host`).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use sn_protocol::{
    ClientMessage, DeathDrop, EntityKey, ItemEvent, LeaveReason, MAX_CHUNK, MAX_PLAYERS,
    PROTOCOL_VERSION, PlayerEvent, PlayerId, PlayerInfo, PlayerState, RefuseReason, RejectReason,
    Request, ServerMessage, Store, Tick, WorldEvent, WorldEventKind, WorldSettings,
};
use sn_sim::inventory::DropPlace;

use crate::items::ItemRules;
use crate::world::{ApplyError, WorldState};
use crate::{Channel, ConnId, PING_KEY};

/// A connection that has not said `Hello` within this long is closed.
#[path = "save.rs"]
pub mod save;

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
    /// The item rules of the host's install (M9d3).
    pub rules: Arc<ItemRules>,
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
    /// An item request refused.
    Refused {
        player: PlayerId,
        req: u32,
        reason: RefuseReason,
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
    pub requests: u64,
    pub refused: u64,
    /// `WorldChunk`s sent, and their bytes.
    pub chunks: u64,
    pub chunk_bytes: u64,
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
    /// The server's own random (SplitMix64) for the luck draws.
    random: u64,
    saved_states: BTreeMap<PlayerId, PlayerState>,
    vitals: BTreeMap<PlayerId, sn_protocol::PlayerVitals>,
    base_tick: u32,
}

/// The server's random: SplitMix64, as a `Random.value` in [0, 1).
fn next_value(state: &mut u64) -> f32 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

impl Server {
    /// A new world whose tick 0 is `now_us` on the caller's clock.
    pub fn new(config: ServerConfig, now_us: u64) -> Server {
        // Items get their rules in M9d3c.
        let world = WorldState::new(config.settings, config.rules.clone());
        // Seeded from the world, so a test run repeats; the game's own
        // random is unseeded, so any seed is the game's behaviour.
        let random = config.settings.slot_seed.unwrap_or(0) ^ 0x6c75_636b;
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
            random,
            saved_states: BTreeMap::new(),
            vitals: BTreeMap::new(),
            base_tick: 0,
        }
    }

    /// A debug aid (M9u6c2, `--test-supply`): one item of each tech put
    /// into the lifepod's locker of a new world, before anyone joins, as
    /// the supplies are; joiners get it in the world they are sent.
    /// Returns whether each went in. Refused once a player is in or the
    /// world has no lifepod.
    pub fn add_test_supplies(&mut self, techs: &[i32]) -> Result<Vec<bool>, String> {
        if !self.conns.is_empty() || self.world.events != 0 || !self.ids.is_empty() {
            return Err("test supplies only go into a new world before anyone joins".into());
        }
        if self.world.settings.lifepod.is_none() {
            return Err("test supplies need a lifepod (its locker)".into());
        }
        let added = techs
            .iter()
            .map(|t| self.world.items.add_supply(*t))
            .collect();
        self.hashes.clear();
        self.hashes.push_back((0, self.world.hash()));
        Ok(added)
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
        let tick = u64::from(self.base_tick) + t / self.config.tick_us;
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
        // The server makes these itself, so they always fit.
        if let Err(err) = self.try_emit(tick, kind) {
            debug_assert!(false, "server event does not apply: {err}");
        }
    }

    /// [`Server::emit`] for an event that may not apply (an item
    /// request): then nothing is logged or sent.
    fn try_emit(&mut self, tick: Tick, kind: WorldEventKind) -> Result<(), ApplyError> {
        let e = WorldEvent {
            seq: self.world.events,
            tick,
            kind,
        };
        self.world.apply(&e)?;
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
        Ok(())
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
        let Some(joined) = self.players.remove(&player) else {
            return;
        };
        if let Some(state) = joined.state {
            self.saved_states.insert(player, state);
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
            ClientMessage::Vitals(v) => {
                if self.world.players.get(&player).is_some_and(|p| p.ready) {
                    self.vitals.insert(player, v);
                }
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
            ClientMessage::Request { req, request } => {
                self.request(conn, player, req, request, now_us)
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
        self.send(
            conn,
            Channel::Reliable,
            ServerMessage::Restore {
                state: self.saved_states.get(&player).copied(),
                vitals: self.vitals.get(&player).copied(),
            },
        );
        // The items after the same events, in chunks (at least one).
        let bytes = self.world.items.to_bytes();
        let chunks: Vec<&[u8]> = if bytes.is_empty() {
            vec![&[]]
        } else {
            bytes.chunks(MAX_CHUNK).collect()
        };
        let total = chunks.len() as u32;
        for (index, c) in chunks.into_iter().enumerate() {
            self.stats.chunks += 1;
            self.stats.chunk_bytes += c.len() as u64;
            self.send(
                conn,
                Channel::Reliable,
                ServerMessage::WorldChunk {
                    index: index as u32,
                    total,
                    bytes: c.to_vec(),
                },
            );
        }
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
                // Cinematics and the death-to-respawn move teleport the
                // player on purpose (M10f: respawns showed as 2,700 m/s).
                let moved_by_the_game = s.cinematic
                    || p.cinematic
                    || s.life != sn_protocol::Life::Alive
                    || p.life != sn_protocol::Life::Alive;
                if speed > self.config.max_speed && !moved_by_the_game {
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

    fn refuse(&mut self, conn: ConnId, player: PlayerId, req: u32, reason: RefuseReason) {
        self.stats.refused += 1;
        self.notices.push(Notice::Refused {
            player,
            req,
            reason,
        });
        self.send(
            conn,
            Channel::Reliable,
            ServerMessage::Refused { req, reason },
        );
    }

    /// An item request: the server fills in what it decides (the luck
    /// draws, the death list, a spawned object's prefab), checks what
    /// only it knows (where the player is, from its newest state), and
    /// emits the event if the shared state takes it. The state's own
    /// checks are the rules; these are ours (`docs/formats/protocol.md`
    /// § Server rules).
    fn request(&mut self, conn: ConnId, player: PlayerId, req: u32, q: Request, now_us: u64) {
        self.stats.requests += 1;
        let ready = self.world.players.get(&player).is_some_and(|p| p.ready);
        if !ready {
            self.refuse(conn, player, req, RefuseReason::NotReady);
            return;
        }
        let in_pod = self
            .players
            .get(&player)
            .and_then(|j| j.state)
            .is_some_and(|s| s.in_pod);
        let items = &self.world.items;
        let event = match q {
            Request::PickUp { key, class } => {
                let class = match &key {
                    EntityKey::Spawned(id) => match items.objects.get(id) {
                        Some(o) => o.class.clone(),
                        None => return self.refuse(conn, player, req, RefuseReason::Gone),
                    },
                    _ => class,
                };
                ItemEvent::PickedUp { key, class }
            }
            Request::Hit {
                key,
                class,
                kind,
                spawn,
            } => {
                let mut random = self.random;
                let draws = items
                    .draws_for_hit(player, &key, &class, kind, &mut || next_value(&mut random));
                match draws {
                    Ok(draws) => {
                        self.random = random;
                        ItemEvent::Hit {
                            key,
                            class,
                            kind,
                            spawn,
                            draws,
                        }
                    }
                    Err(reason) => return self.refuse(conn, player, req, reason),
                }
            }
            Request::Drop { item, at } => {
                if in_pod {
                    return self.refuse(conn, player, req, RefuseReason::NotHere);
                }
                ItemEvent::Dropped { item, at }
            }
            Request::Move { item, to, target } => {
                // Ours: the locker only from inside the lifepod.
                if (to == Store::PodLocker || items.in_locker(item)) && !in_pod {
                    return self.refuse(conn, player, req, RefuseReason::NotHere);
                }
                ItemEvent::Moved { item, to, target }
            }
            Request::Lose { origin, drops } => {
                let place = DropPlace {
                    in_escape_pod: in_pod,
                    ..DropPlace::default()
                };
                let list = items.lose_list(player, place);
                if list.is_empty() {
                    return;
                }
                // Points the client did not send (its state differed):
                // at the origin.
                let drops = list
                    .into_iter()
                    .zip(
                        drops
                            .into_iter()
                            .chain(std::iter::repeat(DeathDrop::default())),
                    )
                    .collect();
                ItemEvent::Lost { origin, drops }
            }
        };
        let tick = self.clock(now_us).0;
        match self.try_emit(tick, WorldEventKind::Item { player, req, event }) {
            Ok(()) => {}
            Err(ApplyError::Item { reason, .. }) => self.refuse(conn, player, req, reason),
            Err(_) => self.refuse(conn, player, req, RefuseReason::Unknown),
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
