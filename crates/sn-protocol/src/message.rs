//! Every message, and its encoding (`docs/formats/protocol.md`). A
//! message is a tag byte and its fields; `decode` reads exactly one
//! message and rejects trailing bytes. Framing several messages into a
//! packet, acks and resends are `sn-net`'s.

use crate::anim::{AnimState, BOOL_MASK, FLOATS, TRIGGERS};
use crate::codec::{Reader, Writer};
use crate::{PlayerId, Result, Tick};

/// Longest player name, game build text and password, in UTF-8 bytes.
pub const MAX_NAME: usize = 32;
pub const MAX_BUILD: usize = 32;
pub const MAX_PASSWORD: usize = 64;
/// Most states in one message (the plan sends the last 4).
pub const MAX_STATES: usize = 8;
/// Most players in a world (`docs/DESIGN.md` § 9, decision 8), and so
/// the most a `Welcome` lists.
pub const MAX_PLAYERS: usize = 8;

/// Where a player is in the death and respawn timeline
/// (`sn_sim::vitals::Life` without its times).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    Alive,
    Dead,
    Respawning,
    Restored,
}

/// A player's state at one physics tick, as its owner sends it. The
/// fields are the wire's quantized values (`crate::quant` converts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub tick: Tick,
    /// The player's transform, Unity world metres.
    pub position: [f32; 3],
    /// `PlayerController.velocity`, cm/s.
    pub velocity: [i16; 3],
    /// The look: `rotationX` (yaw) in 1/65,536 turns, `rotationY` (pitch,
    /// up positive) in 0.01°.
    pub yaw: u16,
    pub pitch: i16,
    /// The player transform's rotation while a cinematic or its ease-out
    /// turns it (x, y, z, w, each × 32,767); `None`: identity.
    pub rotation: Option<[i16; 4]>,
    /// The walking motor runs (else the swimming one).
    pub walking: bool,
    /// `Player.isUnderwaterForSwimming`.
    pub swimming: bool,
    pub grounded: bool,
    /// `GroundMotor.IsGrounded()`.
    pub walk_grounded: bool,
    /// `Player.escapePod`.
    pub in_pod: bool,
    /// `Player.cinematicModeActive`.
    pub cinematic: bool,
    /// Input, movement and look are enabled.
    pub controls: bool,
    pub life: Life,
    pub anim: AnimState,
}

/// What can happen to a player, sent by its owner (reliable) and relayed
/// by the server inside [`WorldEventKind::Player`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerEventKind {
    /// The player used the lifepod's hatch trigger with this index (its
    /// cinematic starts at the event's tick; the pod's animator follows).
    HatchUsed {
        hatch: u16,
    },
    CinematicEnded,
    Died {
        cause: DeathCause,
    },
    Respawned,
    /// `SetTrigger` on the player's animator: an index into
    /// [`TRIGGERS`].
    AnimatorTrigger {
        trigger: u8,
    },
    /// The hatch cinematic's `interpolateAnimParam` bool (`prepare_…`) on
    /// the pod's animator, at the owner's step (protocol 3, M10h3).
    CinematicPrepare {
        on: bool,
    },
    /// The hatch cinematic's `animParam` on the pod's animator and the
    /// player's `playerViewAnimationName` bool (protocol 3, M10h3).
    CinematicPlay {
        on: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathCause {
    Normal,
    Fire,
    Explosion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerEvent {
    pub tick: Tick,
    pub kind: PlayerEventKind,
}

/// How the Aurora starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AuroraStart {
    /// A new game's: the countdown this many game seconds after the start.
    Countdown { seconds: f32 },
    /// Held intact or exploded (a debug choice).
    Held { exploded: bool },
}

/// What the server decides for the whole world (`docs/DESIGN.md` § M10
/// plan, "Who decides what"). Values, not seeds, where they are small.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSettings {
    /// Seed of the spawn slots' fill (`None`: no fill).
    pub slot_seed: Option<u64>,
    /// Lifepod 5's point (`None`: no lifepod).
    pub lifepod: Option<[f32; 3]>,
    pub aurora: AuroraStart,
    /// `DayNightCycle.timePassed` at tick 0, game seconds.
    pub clock_start: f64,
    /// Game seconds per real second.
    pub time_scale: f32,
}

/// Why the server turned a `Hello` down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// The server speaks this protocol version.
    Protocol {
        server: u16,
    },
    /// The server's game build (`__buildnumber.txt`).
    Build {
        server: String,
    },
    /// The rules read from the install differ.
    Fingerprint,
    Full {
        max: u16,
    },
    Password,
    /// Empty, too long, control characters or taken.
    Name,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::Protocol { server } => write!(
                f,
                "the server speaks protocol {server}, this client {}",
                crate::PROTOCOL_VERSION
            ),
            RejectReason::Build { server } => write!(
                f,
                "the server's game is build {server:?}; both players need the same game build"
            ),
            RejectReason::Fingerprint => write!(
                f,
                "the game data differs from the server's (same build, different files?)"
            ),
            RejectReason::Full { max } => write!(
                f,
                "the server is full ({max} {})",
                if *max == 1 { "player" } else { "players" }
            ),
            RejectReason::Password => write!(f, "wrong password"),
            RejectReason::Name => write!(f, "that name is taken or not allowed"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveReason {
    Left,
    TimedOut,
    Kicked,
}

/// A player already in the world, as a `Welcome` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
    pub ready: bool,
    /// Its newest state, if it sent one.
    pub state: Option<PlayerState>,
    /// A cinematic it is in: the hatch and the tick it started.
    pub cinematic: Option<(u16, Tick)>,
}

/// What every client receives in the server's order.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEventKind {
    PlayerJoined {
        player: PlayerId,
        name: String,
    },
    PlayerReady {
        player: PlayerId,
    },
    PlayerLeft {
        player: PlayerId,
        reason: LeaveReason,
    },
    Player {
        player: PlayerId,
        event: PlayerEventKind,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldEvent {
    /// The server's count of world events: 0, 1, 2, … with no gaps.
    pub seq: u32,
    pub tick: Tick,
    pub kind: WorldEventKind,
}

/// Client → server.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    /// The first message. Its tag and the version's two bytes never
    /// change, so any server can read them and answer.
    Hello {
        protocol: u16,
        build: String,
        fingerprint: u64,
        name: String,
        password: Option<String>,
    },
    /// Loaded and spawned: other players may see this one now.
    Ready,
    /// Clock sync: the client's own time, echoed in `Pong`.
    Ping {
        client_time: u64,
    },
    /// This player's newest states, oldest first, ticks increasing.
    States(Vec<PlayerState>),
    Event(PlayerEvent),
    /// The spawn-slot fill of one terrain batch, hashed (a check).
    SlotFillHash {
        batch: [i32; 3],
        hash: u64,
    },
    /// The client's world-state hash after applying the first `events`
    /// world events (a check; the shared state is a function of the
    /// ordered event log, so a count, not a tick, names it).
    WorldHash {
        events: u32,
        hash: u64,
    },
    Bye,
}

/// Server → client.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    /// The answer to a `Hello` that fails. Its tag and `Protocol`'s
    /// encoding never change (see `Hello`).
    Reject(RejectReason),
    Welcome {
        player: PlayerId,
        /// The server's tick when it sent this.
        tick: Tick,
        /// The `seq` of the next world event.
        next_seq: u32,
        settings: WorldSettings,
        players: Vec<PlayerInfo>,
    },
    /// `client_time` from the `Ping`; the server's clock: `tick` plus a
    /// fraction of the next tick in 1/65,536.
    Pong {
        client_time: u64,
        tick: Tick,
        fraction: u16,
    },
    /// Another player's states, as it sent them.
    States {
        player: PlayerId,
        states: Vec<PlayerState>,
    },
    Event(WorldEvent),
}

// Tags. Never reuse a retired number.
mod tag {
    pub const HELLO: u8 = 1;
    pub const READY: u8 = 2;
    pub const PING: u8 = 3;
    pub const STATES: u8 = 4;
    pub const EVENT: u8 = 5;
    pub const SLOT_FILL_HASH: u8 = 6;
    pub const WORLD_HASH: u8 = 7;
    pub const BYE: u8 = 8;

    pub const REJECT: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const PONG: u8 = 3;
    pub const SERVER_STATES: u8 = 4;
    pub const WORLD_EVENT: u8 = 5;
}

// The state's flag bits (u16).
const WALKING: u16 = 1 << 0;
const SWIMMING: u16 = 1 << 1;
const GROUNDED: u16 = 1 << 2;
const WALK_GROUNDED: u16 = 1 << 3;
const IN_POD: u16 = 1 << 4;
const CINEMATIC: u16 = 1 << 5;
const CONTROLS: u16 = 1 << 6;
const ROTATION: u16 = 1 << 7;
const LIFE_SHIFT: u16 = 8;
const FLAG_MASK: u16 = 0x3ff;

impl ClientMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            ClientMessage::Hello {
                protocol,
                build,
                fingerprint,
                name,
                password,
            } => {
                w.u8(tag::HELLO);
                w.u16(*protocol);
                w.str(build);
                w.u64_fixed(*fingerprint);
                w.str(name);
                match password {
                    Some(p) => {
                        w.bool(true);
                        w.str(p);
                    }
                    None => w.bool(false),
                }
            }
            ClientMessage::Ready => w.u8(tag::READY),
            ClientMessage::Ping { client_time } => {
                w.u8(tag::PING);
                w.varint(*client_time);
            }
            ClientMessage::States(states) => {
                w.u8(tag::STATES);
                write_states(&mut w, states);
            }
            ClientMessage::Event(e) => {
                w.u8(tag::EVENT);
                w.varint(u64::from(e.tick.0));
                write_player_event(&mut w, &e.kind);
            }
            ClientMessage::SlotFillHash { batch, hash } => {
                w.u8(tag::SLOT_FILL_HASH);
                for c in batch {
                    w.varint_signed(i64::from(*c));
                }
                w.u64_fixed(*hash);
            }
            ClientMessage::WorldHash { events, hash } => {
                w.u8(tag::WORLD_HASH);
                w.varint(u64::from(*events));
                w.u64_fixed(*hash);
            }
            ClientMessage::Bye => w.u8(tag::BYE),
        }
        w.out
    }

    pub fn decode(data: &[u8]) -> Result<ClientMessage> {
        let mut r = Reader::new(data);
        let at = r.pos;
        let m = match r.u8("message tag")? {
            tag::HELLO => {
                let protocol = r.u16("protocol version")?;
                let build = r.str(MAX_BUILD, "game build")?;
                let fingerprint = r.u64_fixed("rules fingerprint")?;
                let name = r.str(MAX_NAME, "name")?;
                let password = if r.bool("password present")? {
                    Some(r.str(MAX_PASSWORD, "password")?)
                } else {
                    None
                };
                ClientMessage::Hello {
                    protocol,
                    build,
                    fingerprint,
                    name,
                    password,
                }
            }
            tag::READY => ClientMessage::Ready,
            tag::PING => ClientMessage::Ping {
                client_time: r.varint("client time")?,
            },
            tag::STATES => ClientMessage::States(read_states(&mut r)?),
            tag::EVENT => ClientMessage::Event(PlayerEvent {
                tick: Tick(r.varint_u32("event tick")?),
                kind: read_player_event(&mut r)?,
            }),
            tag::SLOT_FILL_HASH => ClientMessage::SlotFillHash {
                batch: [
                    r.varint_i32("batch x")?,
                    r.varint_i32("batch y")?,
                    r.varint_i32("batch z")?,
                ],
                hash: r.u64_fixed("slot fill hash")?,
            },
            tag::WORLD_HASH => ClientMessage::WorldHash {
                events: r.varint_u32("hash event count")?,
                hash: r.u64_fixed("world hash")?,
            },
            tag::BYE => ClientMessage::Bye,
            t => return Err(r.error(at, format!("unknown client message tag {t}"))),
        };
        r.finish()?;
        Ok(m)
    }
}

impl ServerMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            ServerMessage::Reject(reason) => {
                w.u8(tag::REJECT);
                match reason {
                    RejectReason::Protocol { server } => {
                        w.u8(0);
                        w.u16(*server);
                    }
                    RejectReason::Build { server } => {
                        w.u8(1);
                        w.str(server);
                    }
                    RejectReason::Fingerprint => w.u8(2),
                    RejectReason::Full { max } => {
                        w.u8(3);
                        w.varint(u64::from(*max));
                    }
                    RejectReason::Password => w.u8(4),
                    RejectReason::Name => w.u8(5),
                }
            }
            ServerMessage::Welcome {
                player,
                tick,
                next_seq,
                settings,
                players,
            } => {
                w.u8(tag::WELCOME);
                w.varint(u64::from(player.0));
                w.varint(u64::from(tick.0));
                w.varint(u64::from(*next_seq));
                write_settings(&mut w, settings);
                w.varint(players.len() as u64);
                for p in players {
                    w.varint(u64::from(p.id.0));
                    w.str(&p.name);
                    w.bool(p.ready);
                    match &p.state {
                        Some(s) => {
                            w.bool(true);
                            w.varint(u64::from(s.tick.0));
                            write_state_body(&mut w, s);
                        }
                        None => w.bool(false),
                    }
                    match p.cinematic {
                        Some((hatch, start)) => {
                            w.bool(true);
                            w.varint(u64::from(hatch));
                            w.varint(u64::from(start.0));
                        }
                        None => w.bool(false),
                    }
                }
            }
            ServerMessage::Pong {
                client_time,
                tick,
                fraction,
            } => {
                w.u8(tag::PONG);
                w.varint(*client_time);
                w.varint(u64::from(tick.0));
                w.u16(*fraction);
            }
            ServerMessage::States { player, states } => {
                w.u8(tag::SERVER_STATES);
                w.varint(u64::from(player.0));
                write_states(&mut w, states);
            }
            ServerMessage::Event(e) => {
                w.u8(tag::WORLD_EVENT);
                w.varint(u64::from(e.seq));
                w.varint(u64::from(e.tick.0));
                match &e.kind {
                    WorldEventKind::PlayerJoined { player, name } => {
                        w.u8(0);
                        w.varint(u64::from(player.0));
                        w.str(name);
                    }
                    WorldEventKind::PlayerReady { player } => {
                        w.u8(1);
                        w.varint(u64::from(player.0));
                    }
                    WorldEventKind::PlayerLeft { player, reason } => {
                        w.u8(2);
                        w.varint(u64::from(player.0));
                        w.u8(match reason {
                            LeaveReason::Left => 0,
                            LeaveReason::TimedOut => 1,
                            LeaveReason::Kicked => 2,
                        });
                    }
                    WorldEventKind::Player { player, event } => {
                        w.u8(3);
                        w.varint(u64::from(player.0));
                        write_player_event(&mut w, event);
                    }
                }
            }
        }
        w.out
    }

    pub fn decode(data: &[u8]) -> Result<ServerMessage> {
        let mut r = Reader::new(data);
        let at = r.pos;
        let m = match r.u8("message tag")? {
            tag::REJECT => {
                let at = r.pos;
                ServerMessage::Reject(match r.u8("reject reason")? {
                    0 => RejectReason::Protocol {
                        server: r.u16("server protocol")?,
                    },
                    1 => RejectReason::Build {
                        server: r.str(MAX_BUILD, "server build")?,
                    },
                    2 => RejectReason::Fingerprint,
                    3 => RejectReason::Full {
                        max: r.varint_u16("max players")?,
                    },
                    4 => RejectReason::Password,
                    5 => RejectReason::Name,
                    k => return Err(r.error(at, format!("unknown reject reason {k}"))),
                })
            }
            tag::WELCOME => {
                let player = PlayerId(r.varint_u16("player id")?);
                let tick = Tick(r.varint_u32("server tick")?);
                let next_seq = r.varint_u32("next event seq")?;
                let settings = read_settings(&mut r)?;
                let n = r.count(MAX_PLAYERS, "player count")?;
                let mut players = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = PlayerId(r.varint_u16("player id")?);
                    let name = r.str(MAX_NAME, "player name")?;
                    let ready = r.bool("ready")?;
                    let state = if r.bool("state present")? {
                        let tick = Tick(r.varint_u32("state tick")?);
                        Some(read_state_body(&mut r, tick)?)
                    } else {
                        None
                    };
                    let cinematic = if r.bool("cinematic present")? {
                        Some((
                            r.varint_u16("cinematic hatch")?,
                            Tick(r.varint_u32("cinematic start")?),
                        ))
                    } else {
                        None
                    };
                    players.push(PlayerInfo {
                        id,
                        name,
                        ready,
                        state,
                        cinematic,
                    });
                }
                ServerMessage::Welcome {
                    player,
                    tick,
                    next_seq,
                    settings,
                    players,
                }
            }
            tag::PONG => ServerMessage::Pong {
                client_time: r.varint("client time")?,
                tick: Tick(r.varint_u32("server tick")?),
                fraction: r.u16("tick fraction")?,
            },
            tag::SERVER_STATES => ServerMessage::States {
                player: PlayerId(r.varint_u16("player id")?),
                states: read_states(&mut r)?,
            },
            tag::WORLD_EVENT => {
                let seq = r.varint_u32("event seq")?;
                let tick = Tick(r.varint_u32("event tick")?);
                let at = r.pos;
                let kind = match r.u8("world event kind")? {
                    0 => WorldEventKind::PlayerJoined {
                        player: PlayerId(r.varint_u16("player id")?),
                        name: r.str(MAX_NAME, "player name")?,
                    },
                    1 => WorldEventKind::PlayerReady {
                        player: PlayerId(r.varint_u16("player id")?),
                    },
                    2 => {
                        let player = PlayerId(r.varint_u16("player id")?);
                        let at = r.pos;
                        let reason = match r.u8("leave reason")? {
                            0 => LeaveReason::Left,
                            1 => LeaveReason::TimedOut,
                            2 => LeaveReason::Kicked,
                            k => return Err(r.error(at, format!("unknown leave reason {k}"))),
                        };
                        WorldEventKind::PlayerLeft { player, reason }
                    }
                    3 => WorldEventKind::Player {
                        player: PlayerId(r.varint_u16("player id")?),
                        event: read_player_event(&mut r)?,
                    },
                    k => return Err(r.error(at, format!("unknown world event kind {k}"))),
                };
                ServerMessage::Event(WorldEvent { seq, tick, kind })
            }
            t => return Err(r.error(at, format!("unknown server message tag {t}"))),
        };
        r.finish()?;
        Ok(m)
    }
}

/// States: a count, the first tick in full, later ticks as the step from
/// the previous one (at least 1).
fn write_states(w: &mut Writer, states: &[PlayerState]) {
    w.varint(states.len() as u64);
    let mut previous: Option<Tick> = None;
    for s in states {
        match previous {
            None => w.varint(u64::from(s.tick.0)),
            // Callers keep ticks increasing; a wrong order would not
            // decode, so it is a bug at the sender.
            Some(p) => {
                debug_assert!(s.tick > p, "state ticks must increase");
                w.varint(u64::from(s.tick.0.wrapping_sub(p.0)));
            }
        }
        previous = Some(s.tick);
        write_state_body(w, s);
    }
}

fn read_states(r: &mut Reader) -> Result<Vec<PlayerState>> {
    let n = r.count(MAX_STATES, "state count")?;
    let mut states = Vec::with_capacity(n);
    let mut previous: Option<Tick> = None;
    for _ in 0..n {
        let at = r.pos;
        let tick = match previous {
            None => Tick(r.varint_u32("state tick")?),
            Some(p) => {
                let step = r.varint_u32("state tick step")?;
                if step == 0 {
                    return Err(r.error(at, "state ticks must increase"));
                }
                match p.0.checked_add(step) {
                    Some(t) => Tick(t),
                    None => return Err(r.error(at, "state tick overflows")),
                }
            }
        };
        previous = Some(tick);
        states.push(read_state_body(r, tick)?);
    }
    Ok(states)
}

fn write_state_body(w: &mut Writer, s: &PlayerState) {
    let life = match s.life {
        Life::Alive => 0u16,
        Life::Dead => 1,
        Life::Respawning => 2,
        Life::Restored => 3,
    };
    let bit = |on: bool, b: u16| if on { b } else { 0 };
    let flags = bit(s.walking, WALKING)
        | bit(s.swimming, SWIMMING)
        | bit(s.grounded, GROUNDED)
        | bit(s.walk_grounded, WALK_GROUNDED)
        | bit(s.in_pod, IN_POD)
        | bit(s.cinematic, CINEMATIC)
        | bit(s.controls, CONTROLS)
        | bit(s.rotation.is_some(), ROTATION)
        | (life << LIFE_SHIFT);
    w.u16(flags);
    for c in s.position {
        w.f32(c);
    }
    for c in s.velocity {
        w.i16(c);
    }
    w.u16(s.yaw);
    w.i16(s.pitch);
    if let Some(q) = s.rotation {
        for c in q {
            w.i16(c);
        }
    }
    for f in s.anim.floats {
        w.i16(f);
    }
    // 19 bools in 3 bytes.
    let b = s.anim.bools.to_le_bytes();
    w.u8(b[0]);
    w.u8(b[1]);
    w.u8(b[2]);
}

fn read_state_body(r: &mut Reader, tick: Tick) -> Result<PlayerState> {
    let at = r.pos;
    let flags = r.u16("state flags")?;
    if flags & !FLAG_MASK != 0 {
        return Err(r.error(at, format!("unknown state flag bits {flags:#06x}")));
    }
    let life = match flags >> LIFE_SHIFT {
        0 => Life::Alive,
        1 => Life::Dead,
        2 => Life::Respawning,
        _ => Life::Restored,
    };
    let position = [
        r.f32("position x")?,
        r.f32("position y")?,
        r.f32("position z")?,
    ];
    let velocity = [
        r.i16("velocity x")?,
        r.i16("velocity y")?,
        r.i16("velocity z")?,
    ];
    let yaw = r.u16("yaw")?;
    let pitch = r.i16("pitch")?;
    let rotation = if flags & ROTATION != 0 {
        Some([
            r.i16("rotation x")?,
            r.i16("rotation y")?,
            r.i16("rotation z")?,
            r.i16("rotation w")?,
        ])
    } else {
        None
    };
    let mut anim = AnimState::default();
    for (i, f) in anim.floats.iter_mut().enumerate() {
        *f = r.i16(FLOATS[i].0)?;
    }
    let at = r.pos;
    let bools = u32::from_le_bytes([
        r.u8("animator bools")?,
        r.u8("animator bools")?,
        r.u8("animator bools")?,
        0,
    ]);
    if bools & !BOOL_MASK != 0 {
        return Err(r.error(at, format!("unknown animator bool bits {bools:#08x}")));
    }
    anim.bools = bools;
    Ok(PlayerState {
        tick,
        position,
        velocity,
        yaw,
        pitch,
        rotation,
        walking: flags & WALKING != 0,
        swimming: flags & SWIMMING != 0,
        grounded: flags & GROUNDED != 0,
        walk_grounded: flags & WALK_GROUNDED != 0,
        in_pod: flags & IN_POD != 0,
        cinematic: flags & CINEMATIC != 0,
        controls: flags & CONTROLS != 0,
        life,
        anim,
    })
}

fn write_player_event(w: &mut Writer, e: &PlayerEventKind) {
    match e {
        PlayerEventKind::HatchUsed { hatch } => {
            w.u8(0);
            w.varint(u64::from(*hatch));
        }
        PlayerEventKind::CinematicEnded => w.u8(1),
        PlayerEventKind::Died { cause } => {
            w.u8(2);
            w.u8(match cause {
                DeathCause::Normal => 0,
                DeathCause::Fire => 1,
                DeathCause::Explosion => 2,
            });
        }
        PlayerEventKind::Respawned => w.u8(3),
        PlayerEventKind::AnimatorTrigger { trigger } => {
            w.u8(4);
            w.u8(*trigger);
        }
        PlayerEventKind::CinematicPrepare { on } => {
            w.u8(5);
            w.bool(*on);
        }
        PlayerEventKind::CinematicPlay { on } => {
            w.u8(6);
            w.bool(*on);
        }
    }
}

fn read_player_event(r: &mut Reader) -> Result<PlayerEventKind> {
    let at = r.pos;
    Ok(match r.u8("player event kind")? {
        0 => PlayerEventKind::HatchUsed {
            hatch: r.varint_u16("hatch")?,
        },
        1 => PlayerEventKind::CinematicEnded,
        2 => {
            let at = r.pos;
            PlayerEventKind::Died {
                cause: match r.u8("death cause")? {
                    0 => DeathCause::Normal,
                    1 => DeathCause::Fire,
                    2 => DeathCause::Explosion,
                    k => return Err(r.error(at, format!("unknown death cause {k}"))),
                },
            }
        }
        3 => PlayerEventKind::Respawned,
        4 => {
            let at = r.pos;
            let trigger = r.u8("animator trigger")?;
            if usize::from(trigger) >= TRIGGERS.len() {
                return Err(r.error(at, format!("unknown animator trigger {trigger}")));
            }
            PlayerEventKind::AnimatorTrigger { trigger }
        }
        5 => PlayerEventKind::CinematicPrepare {
            on: r.bool("cinematic prepare")?,
        },
        6 => PlayerEventKind::CinematicPlay {
            on: r.bool("cinematic play")?,
        },
        k => return Err(r.error(at, format!("unknown player event kind {k}"))),
    })
}

fn write_settings(w: &mut Writer, s: &WorldSettings) {
    match s.slot_seed {
        Some(seed) => {
            w.bool(true);
            w.u64_fixed(seed);
        }
        None => w.bool(false),
    }
    match s.lifepod {
        Some(p) => {
            w.bool(true);
            for c in p {
                w.f32(c);
            }
        }
        None => w.bool(false),
    }
    match s.aurora {
        AuroraStart::Countdown { seconds } => {
            w.u8(0);
            w.f32(seconds);
        }
        AuroraStart::Held { exploded } => {
            w.u8(1);
            w.bool(exploded);
        }
    }
    w.f64(s.clock_start);
    w.f32(s.time_scale);
}

fn read_settings(r: &mut Reader) -> Result<WorldSettings> {
    let slot_seed = if r.bool("slot seed present")? {
        Some(r.u64_fixed("slot seed")?)
    } else {
        None
    };
    let lifepod = if r.bool("lifepod present")? {
        Some([
            r.f32("lifepod x")?,
            r.f32("lifepod y")?,
            r.f32("lifepod z")?,
        ])
    } else {
        None
    };
    let at = r.pos;
    let aurora = match r.u8("aurora start")? {
        0 => AuroraStart::Countdown {
            seconds: r.f32("aurora countdown")?,
        },
        1 => AuroraStart::Held {
            exploded: r.bool("aurora exploded")?,
        },
        k => return Err(r.error(at, format!("unknown aurora start {k}"))),
    };
    Ok(WorldSettings {
        slot_seed,
        lifepod,
        aurora,
        clock_start: r.f64("clock start")?,
        time_scale: r.f32("time scale")?,
    })
}
