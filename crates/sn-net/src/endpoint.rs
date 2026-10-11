//! An endpoint: one transport, any number of peers. It accepts or makes
//! connections (handshake), routes datagrams to their connection, times
//! peers out, and reports everything as [`Event`]s. Driven by
//! [`Endpoint::update`] with the caller's clock in microseconds.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io;
use std::net::SocketAddr;

use crate::connection::{Connection, Delivery, SendError, Stats};
use crate::packet::{self, ControlKind, MAX_PACKET, Packet, RefuseReason};
use crate::transport::Transport;

/// No packet from a peer for this long: it is gone.
pub const TIMEOUT_US: u64 = 5_000_000;
/// A connecting client repeats its request this often.
pub const CONNECT_RESEND_US: u64 = 100_000;
/// Datagrams read per update at most (a flood cannot stall the caller).
const MAX_RECV_PER_UPDATE: usize = 4096;
/// Copies of a goodbye (it is not acknowledged).
const DISCONNECT_COPIES: usize = 3;

/// A peer of this endpoint, numbered from 0 in the order they appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Nothing heard for [`TIMEOUT_US`] (or no answer to connecting).
    TimedOut,
    /// The peer said goodbye.
    Closed,
    /// The server is full.
    Refused,
    /// The same address connected again (a restarted client).
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Connected(PeerId),
    Disconnected(PeerId, DisconnectReason),
    Reliable(PeerId, Vec<u8>),
    Latest(PeerId, u16, Vec<u8>),
    Unordered(PeerId, Vec<u8>),
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Accept incoming connections (a server).
    pub accept: bool,
    /// Most peers at once; more are refused.
    pub max_peers: usize,
    /// Seeds the connection ids (any value; [`random_seed`] for real use).
    pub seed: u64,
    pub timeout_us: u64,
}

impl Config {
    pub fn server(max_peers: usize, seed: u64) -> Config {
        Config {
            accept: true,
            max_peers,
            seed,
            timeout_us: TIMEOUT_US,
        }
    }

    pub fn client(seed: u64) -> Config {
        Config {
            accept: false,
            max_peers: 1,
            seed,
            timeout_us: TIMEOUT_US,
        }
    }
}

/// A seed that differs between runs (the clock and the standard library's
/// per-process random hash keys).
pub fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    h.write_u64(t);
    h.finish()
}

/// Counters of the endpoint itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndpointStats {
    /// Datagrams that were not valid packets.
    pub bad_packets: u64,
    /// Valid packets from no known peer, or for another connection id.
    pub stray_packets: u64,
    pub send_errors: u64,
    pub recv_errors: u64,
    pub refused: u64,
}

enum State {
    /// We asked; no answer yet. When we last asked.
    Connecting {
        last_ask: Option<u64>,
    },
    Connected,
}

struct Peer {
    addr: SocketAddr,
    state: State,
    conn: Connection,
}

pub struct Endpoint<T: Transport> {
    transport: T,
    config: Config,
    rng: u64,
    next_peer: u32,
    peers: BTreeMap<PeerId, Peer>,
    by_addr: HashMap<SocketAddr, PeerId>,
    events: VecDeque<Event>,
    stats: EndpointStats,
}

impl<T: Transport> Endpoint<T> {
    pub fn new(transport: T, config: Config) -> Endpoint<T> {
        Endpoint {
            transport,
            config,
            rng: config.seed,
            next_peer: 0,
            peers: BTreeMap::new(),
            by_addr: HashMap::new(),
            events: VecDeque::new(),
            stats: EndpointStats::default(),
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.transport.local_addr()
    }

    pub fn stats(&self) -> EndpointStats {
        self.stats
    }

    fn conn_id(&mut self) -> u32 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as u32
    }

    fn add_peer(&mut self, addr: SocketAddr, state: State, conn: u32, now: u64) -> PeerId {
        let id = PeerId(self.next_peer);
        self.next_peer += 1;
        self.peers.insert(
            id,
            Peer {
                addr,
                state,
                conn: Connection::new(conn, now),
            },
        );
        self.by_addr.insert(addr, id);
        id
    }

    fn remove(&mut self, id: PeerId, reason: Option<DisconnectReason>) {
        if let Some(p) = self.peers.remove(&id) {
            self.by_addr.remove(&p.addr);
            if let Some(r) = reason {
                self.events.push_back(Event::Disconnected(id, r));
            }
        }
    }

    /// Starts connecting to a server; [`Event::Connected`] follows, or
    /// [`Event::Disconnected`] with `TimedOut` or `Refused`. Messages may
    /// be queued right away.
    pub fn connect(&mut self, addr: SocketAddr, now: u64) -> PeerId {
        if let Some(&old) = self.by_addr.get(&addr) {
            self.remove(old, Some(DisconnectReason::Replaced));
        }
        let conn = self.conn_id();
        self.add_peer(addr, State::Connecting { last_ask: None }, conn, now)
    }

    /// Says goodbye and forgets the peer (no event for it).
    pub fn disconnect(&mut self, id: PeerId) {
        if let Some(p) = self.peers.get(&id) {
            let bye = packet::control(ControlKind::Disconnect, p.conn.conn_id);
            for _ in 0..DISCONNECT_COPIES {
                if self.transport.send(p.addr, &bye).is_err() {
                    self.stats.send_errors += 1;
                }
            }
        }
        self.remove(id, None);
    }

    pub fn send_reliable(&mut self, id: PeerId, data: Vec<u8>) -> Result<(), SendError> {
        match self.peers.get_mut(&id) {
            Some(p) => p.conn.send_reliable(data),
            None => Err(SendError::UnknownPeer),
        }
    }

    pub fn send_latest(&mut self, id: PeerId, key: u16, data: Vec<u8>) -> Result<(), SendError> {
        match self.peers.get_mut(&id) {
            Some(p) => p.conn.send_latest(key, data),
            None => Err(SendError::UnknownPeer),
        }
    }

    pub fn send_unordered(&mut self, id: PeerId, data: Vec<u8>) -> Result<(), SendError> {
        match self.peers.get_mut(&id) {
            Some(p) => p.conn.send_unordered(data),
            None => Err(SendError::UnknownPeer),
        }
    }

    pub fn peers(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.peers
            .iter()
            .filter(|(_, p)| matches!(p.state, State::Connected))
            .map(|(&id, _)| id)
    }

    pub fn peer_addr(&self, id: PeerId) -> Option<SocketAddr> {
        self.peers.get(&id).map(|p| p.addr)
    }

    pub fn peer_stats(&self, id: PeerId) -> Option<Stats> {
        self.peers.get(&id).map(|p| p.conn.stats)
    }

    /// The smoothed round trip to a peer, microseconds.
    pub fn rtt_us(&self, id: PeerId) -> Option<f64> {
        self.peers.get(&id).and_then(|p| p.conn.rtt_us())
    }

    /// When a packet last came from a peer (the caller's clock).
    pub fn last_received_at(&self, id: PeerId) -> Option<u64> {
        self.peers.get(&id).map(|p| p.conn.last_received_at)
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Reads everything that arrived, times peers out, then sends what is
    /// queued or due.
    pub fn update(&mut self, now: u64) {
        let mut buf = [0u8; MAX_PACKET + 1];
        for _ in 0..MAX_RECV_PER_UPDATE {
            match self.transport.recv(&mut buf) {
                Ok(Some((n, from))) => self.handle(now, from, &buf[..n]),
                Ok(None) => break,
                Err(_) => {
                    self.stats.recv_errors += 1;
                    break;
                }
            }
        }
        let timed_out: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(_, p)| now.saturating_sub(p.conn.last_received_at) >= self.config.timeout_us)
            .map(|(&id, _)| id)
            .collect();
        for id in timed_out {
            self.remove(id, Some(DisconnectReason::TimedOut));
        }
        let mut out: Vec<(SocketAddr, Vec<u8>)> = Vec::new();
        for p in self.peers.values_mut() {
            match &mut p.state {
                State::Connecting { last_ask } => {
                    if last_ask.is_none_or(|t| now.saturating_sub(t) >= CONNECT_RESEND_US) {
                        *last_ask = Some(now);
                        out.push((
                            p.addr,
                            packet::control(ControlKind::Connect, p.conn.conn_id),
                        ));
                    }
                }
                State::Connected => {
                    for packet in p.conn.poll(now) {
                        out.push((p.addr, packet));
                    }
                }
            }
        }
        for (to, data) in out {
            if self.transport.send(to, &data).is_err() {
                self.stats.send_errors += 1;
            }
        }
    }

    fn reply(&mut self, to: SocketAddr, kind: ControlKind, conn: u32) {
        if self
            .transport
            .send(to, &packet::control(kind, conn))
            .is_err()
        {
            self.stats.send_errors += 1;
        }
    }

    fn handle(&mut self, now: u64, from: SocketAddr, data: &[u8]) {
        let Ok(packet) = packet::decode(data) else {
            self.stats.bad_packets += 1;
            return;
        };
        let known = self.by_addr.get(&from).copied();
        let conn_of = |e: &Self, id: PeerId| e.peers.get(&id).map(|p| p.conn.conn_id);
        match packet {
            Packet::Connect { conn } => {
                if !self.config.accept {
                    self.stats.stray_packets += 1;
                    return;
                }
                if let Some(id) = known {
                    if conn_of(self, id) == Some(conn) {
                        // Our accept was lost: say it again.
                        self.reply(from, ControlKind::Accept, conn);
                        return;
                    }
                    self.remove(id, Some(DisconnectReason::Replaced));
                }
                if self.peers.len() >= self.config.max_peers {
                    self.stats.refused += 1;
                    self.reply(from, ControlKind::Refuse(RefuseReason::Full), conn);
                    return;
                }
                let id = self.add_peer(from, State::Connected, conn, now);
                self.events.push_back(Event::Connected(id));
                self.reply(from, ControlKind::Accept, conn);
            }
            Packet::Accept { conn } => match known {
                Some(id) if conn_of(self, id) == Some(conn) => self.accepted(id, now),
                _ => self.stats.stray_packets += 1,
            },
            Packet::Refuse { conn, .. } => match known {
                Some(id) if conn_of(self, id) == Some(conn) => {
                    if matches!(
                        self.peers.get(&id).map(|p| &p.state),
                        Some(State::Connecting { .. })
                    ) {
                        self.remove(id, Some(DisconnectReason::Refused));
                    }
                }
                _ => self.stats.stray_packets += 1,
            },
            Packet::Disconnect { conn } => match known {
                Some(id) if conn_of(self, id) == Some(conn) => {
                    self.remove(id, Some(DisconnectReason::Closed));
                }
                _ => self.stats.stray_packets += 1,
            },
            Packet::Data {
                conn,
                seq,
                acks,
                frames,
            } => {
                let Some(id) = known.filter(|&id| conn_of(self, id) == Some(conn)) else {
                    self.stats.stray_packets += 1;
                    return;
                };
                // Data before the accept: the accept was lost, the server
                // took us anyway.
                self.accepted(id, now);
                let Some(p) = self.peers.get_mut(&id) else {
                    return;
                };
                p.conn.receive(now, data.len(), seq, acks, &frames);
                for d in p.conn.delivered.drain(..) {
                    self.events.push_back(match d {
                        Delivery::Reliable(bytes) => Event::Reliable(id, bytes),
                        Delivery::Latest { key, data } => Event::Latest(id, key, data),
                        Delivery::Unordered(data) => Event::Unordered(id, data),
                    });
                }
            }
        }
    }

    fn accepted(&mut self, id: PeerId, now: u64) {
        if let Some(p) = self.peers.get_mut(&id)
            && matches!(p.state, State::Connecting { .. })
        {
            p.state = State::Connected;
            p.conn.last_received_at = now;
            self.events.push_back(Event::Connected(id));
        }
    }
}
