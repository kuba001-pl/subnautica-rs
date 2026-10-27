//! The multiplayer logic as state machines (`docs/DESIGN.md` § Phase D
//! "M10 plan"): [`server`] (M10c) and [`client`] (M10d).
//! Pure: no sockets, files, threads or clocks of its own; the caller
//! passes bytes and times in and sends what comes out, so everything runs
//! headless in tests with as many simulated clients as wanted.

pub mod client;
pub mod craft;
pub mod drops;
pub mod items;
pub mod server;
pub mod world;

pub use client::{Client, ClientConfig, ClientNotice, ClientStats, RemoteSample, SampleKind};
pub use items::{ItemRules, ItemsState};
pub use server::save;
pub use server::{Action, Notice, Server, ServerConfig, ServerStats};
pub use world::{ApplyError, PlayerRecord, WorldState};

/// A transport connection, as the caller numbers them (`sn_net::PeerId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub u32);

/// Which `sn-net` channel a message goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Every message, once, in order.
    Reliable,
    /// Only the newest per key (pings).
    Latest(u16),
    /// Everything that arrives, in any order (player states: they carry
    /// their ticks, so an overtaken message still has ticks to give).
    Unordered,
}

/// The latest-only key of `Ping`/`Pong` (player ids are far below it).
pub const PING_KEY: u16 = u16::MAX;
