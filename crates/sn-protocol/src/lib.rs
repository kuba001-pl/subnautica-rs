//! The multiplayer messages and their binary codec (M10a, `docs/DESIGN.md`
//! § Phase D "M10 plan"; the format in `docs/formats/protocol.md`). Our own
//! protocol, not Nitrox's.
//!
//! Pure: messages in, bytes out and back; no sockets, no files. Decoding
//! never panics: malformed input is an [`Error`] with its byte offset.
//! Encodings are canonical (one encoding per message), so a decoded
//! message re-encodes to the same bytes. No game data is ever in a
//! message, only ids, numbers, seeds and hashes (AGENTS.md rule 5).

pub mod anim;
pub mod codec;
mod hash;
pub mod items;
pub mod message;
pub mod quant;

use std::fmt;

pub use anim::AnimState;
pub use hash::Fnv64;
pub use items::{
    DeathDrop, DropAt, EntityKey, HitKind, ItemEvent, ItemId, MoveTarget, RefuseReason, Request,
    Store,
};
pub use message::{
    AuroraStart, ClientMessage, DeathCause, LeaveReason, Life, MAX_BUILD, MAX_CHUNK, MAX_NAME,
    MAX_PASSWORD, MAX_PLAYERS, MAX_STATES, PlayerEvent, PlayerEventKind, PlayerInfo, PlayerState,
    PlayerVitals, RejectReason, ServerMessage, WorldEvent, WorldEventKind, WorldSettings,
    read_settings, write_settings,
};

/// The protocol's version, sent first in `Hello`. Raised whenever any
/// encoding or table changes (2: `WorldHash` names an event count, not a
/// tick; M10c; 3: the cinematic signals, M10h3; 4: items, M9d3;
/// 5: persistence vitals/restore, M9f).
pub const PROTOCOL_VERSION: u16 = 5;

/// A physics step of the shared timeline, counted by the server from the
/// world's start (`Time.fixedDeltaTime` each; 0.02 s in Subnautica, so
/// `u32` lasts about 2.7 years of play).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u32);

/// A player, numbered by the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u16);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Byte offset in the message.
    pub offset: usize,
    pub message: String,
}

impl Error {
    pub(crate) fn new(offset: usize, message: impl Into<String>) -> Error {
        Error {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
