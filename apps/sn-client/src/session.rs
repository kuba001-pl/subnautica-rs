//! The session (M10g, `docs/DESIGN.md` § M10 plan): the client always
//! runs `sn-session`'s server in-process and joins it over a direct
//! in-memory link (no sockets, no impairment), so solo play goes through
//! the same handshake, clock and world settings as multiplayer.
//!
//! The server's tick 0 is the client's first frame (loading before it
//! does not count). The player's physics steps are the server's ticks
//! (`Client::steps`), the game clock is the server's time
//! ([`crate::aurora::GameClock`]), and the new world's values (Lifepod 5's
//! point, the Aurora, the slot seed, the clock) come from the `Welcome`.

use std::ops::Range;
use std::time::Instant;

use bevy::prelude::*;
use sn_protocol::WorldSettings;
use sn_session::{
    Action, Client, ClientConfig, ClientNotice, ConnId, Notice, Server, ServerConfig,
};

/// The client's connection on the in-process server.
const LOCAL: ConnId = ConnId(0);

/// A status line every this many seconds of session time.
const REPORT_US: u64 = 60_000_000;

#[derive(Resource)]
pub struct Session {
    server: Server,
    client: Client,
    settings: WorldSettings,
    tick_us: u64,
    /// The first frame: the session's time 0.
    epoch: Option<Instant>,
    /// Session time of this frame, microseconds.
    now_us: u64,
    /// Messages carried over the link, each way.
    to_server: u64,
    to_client: u64,
    next_report: u64,
}

impl Session {
    /// Starts the server and joins it as `name`. Fails when the server
    /// turns the client down (it never should: one install, one build).
    pub fn solo(config: ServerConfig, name: &str) -> Result<Session, String> {
        let client = ClientConfig {
            name: name.to_owned(),
            build: config.build.clone(),
            fingerprint: config.fingerprint,
            password: config.password.clone(),
            tick_us: config.tick_us,
            max_speed: config.max_speed,
        };
        let tick_us = config.tick_us;
        let settings = config.settings;
        let mut s = Session {
            server: Server::new(config, 0),
            client: Client::new(client, 0),
            settings,
            tick_us,
            epoch: None,
            now_us: 0,
            to_server: 0,
            to_client: 0,
            next_report: REPORT_US,
        };
        s.server.connected(LOCAL, 0);
        s.pump();
        // The first ping, so the clock is known before the first frame.
        s.client.update(0);
        s.pump();
        // The notices are logged in the first frame (Bevy's log is not up
        // yet).
        if s.client.is_rejected() {
            return Err("the in-process server turned the client down".into());
        }
        let Some(world) = s.client.world() else {
            return Err("no Welcome from the in-process server".into());
        };
        s.settings = world.settings;
        if s.client.server_us(0).is_none() {
            return Err("no clock from the in-process server".into());
        }
        Ok(s)
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
        self.client.server_us(self.now_us).unwrap_or(0.0) / 1e6
    }

    /// The server's tick this frame, with its fraction.
    pub fn server_tick(&self) -> f64 {
        self.client.server_tick(self.now_us).unwrap_or(0.0)
    }

    /// The own player's physics steps due this frame, in the server's
    /// ticks (at most 10; the rest come in later frames).
    pub fn steps(&mut self) -> Range<u32> {
        self.client.steps(self.now_us)
    }

    /// The next step [`Session::steps`] will give.
    pub fn next_step(&self) -> u32 {
        self.client.steps_taken()
    }

    /// The own player is loaded and placed.
    pub fn ready(&mut self) {
        self.client.ready();
        self.pump();
    }

    /// Carries messages both ways until the link is quiet (every message
    /// is encoded and decoded, as on the network).
    fn pump(&mut self) {
        let now = self.now_us;
        loop {
            let mut moved = false;
            for (_, m) in self.client.take_outgoing() {
                moved = true;
                self.to_server += 1;
                self.server.received(LOCAL, &m.encode(), now);
            }
            for a in self.server.take_actions() {
                match a {
                    Action::Send { to, message, .. } if to == LOCAL => {
                        moved = true;
                        self.to_client += 1;
                        self.client.received(&message.encode(), now);
                    }
                    Action::Send { to, .. } => {
                        warn!("session: message for unknown connection {}", to.0);
                    }
                    Action::Close(c) => warn!("session: server closed connection {}", c.0),
                }
            }
            if !moved {
                break;
            }
        }
    }

    fn log_notices(&mut self) {
        for n in self.server.take_notices() {
            match n {
                Notice::Joined { player, name, .. } => {
                    info!(
                        "session: {name:?} joined the in-process server as player {}",
                        player.0
                    );
                }
                Notice::Ready { player } => info!("session: player {} ready", player.0),
                Notice::Rejected { reason, .. } => error!("session: rejected: {reason}"),
                Notice::Left { player, reason } => {
                    info!("session: player {} left ({reason:?})", player.0);
                }
                Notice::HashMismatch { player, events } => error!(
                    "session: WORLD STATE DIFFERS: player {} after {events} events",
                    player.0
                ),
                Notice::SlotFillMismatch { player, batch } => error!(
                    "session: SLOT FILL DIFFERS: player {} in batch {batch:?}",
                    player.0
                ),
                Notice::Implausible { player, speed } => {
                    warn!("session: player {} moved at {speed:.1} m/s", player.0);
                }
                Notice::Ignored { why, .. } => warn!("session: server ignored a message: {why}"),
            }
        }
        for n in self.client.take_notices() {
            match n {
                ClientNotice::Welcomed { player } => {
                    let s = self.client.world().map(|w| w.settings);
                    info!("session: welcomed as player {}; world {s:?}", player.0);
                }
                ClientNotice::Rejected(r) => error!("session: rejected: {r}"),
                ClientNotice::Apply(e) => error!("session: world event does not apply: {e}"),
                ClientNotice::Ignored(why) => warn!("session: client ignored a message: {why}"),
            }
        }
    }
}

/// First in every frame: the session's time, the link both ways, the
/// server's and client's own work.
pub fn update(mut session: ResMut<Session>) {
    let s = &mut *session;
    let epoch = *s.epoch.get_or_insert_with(Instant::now);
    s.now_us = epoch.elapsed().as_micros() as u64;
    let now = s.now_us;
    s.client.update(now);
    s.pump();
    s.server.update(now);
    s.pump();
    s.log_notices();
    if now >= s.next_report {
        s.next_report += REPORT_US;
        let (server, client) = (s.server.stats(), s.client.stats());
        info!(
            "session: {:.0} s, tick {:.0}: {} messages to the server, {} back; {} world events; hash reports {}, checked {} ({} differ); clock jumps {}",
            now as f64 / 1e6,
            s.server_tick(),
            s.to_server,
            s.to_client,
            server.events,
            client.hash_reports,
            server.hash_checks,
            server.hash_mismatches,
            client.clock_jumps
        );
    }
}
