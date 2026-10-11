//! The server's loop (M10e): `sn-session`'s server behind an `sn-net`
//! endpoint, on any transport (UDP in the app, the simulated network or
//! UDP on 127.0.0.1 in tests). `main.rs` reads the install and the flags.

use std::collections::HashMap;
use std::net::SocketAddr;

use sn_net::{DisconnectReason, Endpoint, Event, PeerId, Transport};
use sn_session::{Action, Channel, ConnId, Notice, Server, ServerConfig, ServerStats};

/// The server and its endpoint.
pub struct Host<T: Transport> {
    pub endpoint: Endpoint<T>,
    pub server: Server,
    names: HashMap<ConnId, String>,
    /// Notices aggregated per report: implausible moves per player (count,
    /// fastest) and ignored messages.
    implausible: HashMap<u16, (u64, f64)>,
    ignored: u64,
    last_report: Option<Snapshot>,
}

/// Counters at a report.
struct Snapshot {
    at_us: u64,
    stats: ServerStats,
    bytes: HashMap<PeerId, (u64, u64)>,
}

impl<T: Transport> Host<T> {
    pub fn new(endpoint: Endpoint<T>, config: ServerConfig, now_us: u64) -> Host<T> {
        Host {
            endpoint,
            server: Server::new(config, now_us),
            names: HashMap::new(),
            implausible: HashMap::new(),
            ignored: 0,
            last_report: None,
        }
    }

    fn addr(&self, conn: ConnId) -> String {
        self.endpoint
            .peer_addr(PeerId(conn.0))
            .map_or_else(|| "?".into(), |a: SocketAddr| a.to_string())
    }

    /// One pass: receive, run the server, send. Returns log lines.
    pub fn step(&mut self, now_us: u64) -> Vec<String> {
        let mut log = Vec::new();
        self.endpoint.update(now_us);
        while let Some(e) = self.endpoint.poll_event() {
            match e {
                Event::Connected(p) => self.server.connected(ConnId(p.0), now_us),
                Event::Disconnected(p, r) => {
                    self.server
                        .disconnected(ConnId(p.0), r == DisconnectReason::TimedOut, now_us);
                }
                Event::Reliable(p, b) | Event::Latest(p, _, b) | Event::Unordered(p, b) => {
                    self.server.received(ConnId(p.0), &b, now_us);
                }
            }
        }
        self.server.update(now_us);
        for a in self.server.take_actions() {
            match a {
                Action::Send {
                    to,
                    channel,
                    message,
                } => {
                    let peer = PeerId(to.0);
                    let bytes = message.encode();
                    let sent = match channel {
                        Channel::Reliable => self.endpoint.send_reliable(peer, bytes),
                        Channel::Latest(k) => self.endpoint.send_latest(peer, k, bytes),
                        Channel::Unordered => self.endpoint.send_unordered(peer, bytes),
                    };
                    if let Err(e) = sent {
                        log.push(format!("send to {}: {e}", self.addr(to)));
                    }
                }
                Action::Close(c) => self.endpoint.disconnect(PeerId(c.0)),
            }
        }
        for n in self.server.take_notices() {
            match n {
                Notice::Joined { conn, player, name } => {
                    log.push(format!(
                        "joined: {name:?} as player {} from {}",
                        player.0,
                        self.addr(conn)
                    ));
                    self.names.insert(conn, name);
                }
                Notice::Rejected { conn, reason } => {
                    log.push(format!("rejected {}: {reason}", self.addr(conn)));
                }
                Notice::Ready { player } => log.push(format!("ready: player {}", player.0)),
                Notice::Left { player, reason } => {
                    log.push(format!("left: player {} ({reason:?})", player.0));
                }
                Notice::HashMismatch { player, events } => log.push(format!(
                    "WORLD STATE DIFFERS: player {} after {events} events",
                    player.0
                )),
                Notice::SlotFillMismatch { player, batch } => log.push(format!(
                    "SLOT FILL DIFFERS: player {} in batch {batch:?}",
                    player.0
                )),
                Notice::Implausible { player, speed } => {
                    let e = self.implausible.entry(player.0).or_default();
                    e.0 += 1;
                    e.1 = e.1.max(speed);
                }
                Notice::Ignored { .. } => self.ignored += 1,
            }
        }
        self.endpoint.update(now_us);
        log
    }

    /// A summary since the last one: players, bandwidth per player, the
    /// server's counters.
    pub fn report(&mut self, now_us: u64) -> Vec<String> {
        let stats = self.server.stats();
        let peers: Vec<PeerId> = self.endpoint.peers().collect();
        let bytes: HashMap<PeerId, (u64, u64)> = peers
            .iter()
            .filter_map(|&p| {
                self.endpoint
                    .peer_stats(p)
                    .map(|s| (p, (s.bytes_sent, s.bytes_received)))
            })
            .collect();
        let mut out = Vec::new();
        if let Some(last) = &self.last_report {
            let secs = (now_us.saturating_sub(last.at_us)) as f64 / 1e6;
            let mut lines = Vec::new();
            for &p in &peers {
                let (sent, recv) = bytes[&p];
                let (s0, r0) = last.bytes.get(&p).copied().unwrap_or((0, 0));
                let name = self
                    .names
                    .get(&ConnId(p.0))
                    .map_or("(joining)", String::as_str);
                lines.push(format!(
                    "{name}: up {:.0} B/s, down {:.0} B/s, rtt {}",
                    (recv - r0) as f64 / secs,
                    (sent - s0) as f64 / secs,
                    self.endpoint
                        .rtt_us(p)
                        .map_or("-".into(), |r| format!("{:.1} ms", r / 1000.0))
                ));
            }
            let d = |a: u64, b: u64| a - b;
            out.push(format!(
                "{} players, {} world events; last {secs:.0} s: {} states relayed, {} events, {} hash checks ({} differ), {} slot-fill checks ({} differ), {} ignored messages",
                self.server.world().players.len(),
                self.server.world().events,
                d(stats.states_relayed, last.stats.states_relayed),
                d(stats.events, last.stats.events),
                d(stats.hash_checks, last.stats.hash_checks),
                d(stats.hash_mismatches, last.stats.hash_mismatches),
                d(stats.slot_fill_checks, last.stats.slot_fill_checks),
                d(stats.slot_fill_mismatches, last.stats.slot_fill_mismatches),
                self.ignored
            ));
            out.extend(lines);
            for (player, (n, top)) in self.implausible.drain() {
                out.push(format!(
                    "implausible moves: player {player} × {n}, up to {top:.1} m/s"
                ));
            }
            self.ignored = 0;
        }
        self.last_report = Some(Snapshot {
            at_us: now_us,
            stats,
            bytes,
        });
        out
    }
}
