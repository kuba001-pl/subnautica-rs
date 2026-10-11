//! The multiplayer transport (M10b, `docs/DESIGN.md` § Phase D "M10
//! plan"; the packet format in `docs/formats/protocol.md` § Packets): our
//! own thin layer on UDP, with no async runtime (§ 9, decision 5).
//!
//! - [`Endpoint`]: one socket, many peers: the handshake, timeouts, and
//!   three channels per peer: **reliable-ordered** (every message exactly
//!   once, in order), **latest-only** (per key, never an older message
//!   after a newer one; lost ones are not resent) and **unordered**
//!   (everything that arrives, in any order; lost ones are not resent:
//!   for messages that carry their own tick, such as player states).
//! - [`Transport`]: where datagrams go: [`UdpTransport`] or
//!   [`sim::SimNetwork`], an in-memory network with seeded latency,
//!   jitter, loss and duplication for tests.
//!
//! Everything runs on the caller's clock (microseconds), so tests run on
//! simulated time. The bytes carried are opaque here (`sn-protocol`
//! encodes them). No encryption: M10 is for friends on trust.

mod connection;
mod endpoint;
mod impaired;
mod packet;
pub mod sim;
mod transport;

pub use connection::{
    Delivery, KEEPALIVE_US, MAX_ACK_DELAY_US, MAX_PENDING_RELIABLE, MAX_PENDING_UNORDERED,
    SendError, Stats,
};
pub use endpoint::{
    CONNECT_RESEND_US, Config, DisconnectReason, Endpoint, EndpointStats, Event, PeerId,
    TIMEOUT_US, random_seed,
};
pub use impaired::ImpairedTransport;
pub use packet::{MAX_MESSAGE, MAX_PACKET};
pub use transport::{Transport, UdpTransport};
