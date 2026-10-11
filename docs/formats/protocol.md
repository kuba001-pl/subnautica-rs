# Multiplayer protocol (our own format)

This is **our** format, not one of the game's: the messages that clients and
the server of subnautica-rs exchange (`docs/DESIGN.md` § Phase D, "M10
plan"). It is not wire-compatible with Nitrox. Everything here is defined
by `crates/sn-protocol`, so nothing is *hypothesis*; where this page and
the code disagree, the code is right and this page is a bug.

Messages carry ids, numbers, seeds and hashes only. They never carry game
data (AGENTS.md rule 5): every player reads the world from their own
install.

Protocol version: **2** (`PROTOCOL_VERSION`). Any change to an encoding or
to a table below raises it. History: 1 (M10a); 2 (M10c): `WorldHash`
carries the number of world events applied instead of a tick.

## Scope

`sn-protocol` encodes and decodes **one message** to and from bytes. Packets
(several messages framed together), sequence numbers, acks, resends and
channels are `sn-net`'s ([Packets](#packets-sn-net), M10b). Which channel
each message uses (decided by M10c's session code): the reliable channel
for `Hello`, `Ready`, `Event`, `SlotFillHash`, `WorldHash`, `Bye`,
`Reject`, `Welcome` and world `Event`s; the **unordered** channel for
`States` (both ways: they carry their ticks, so order does not matter and
an overtaken message still has ticks to give; M10d); the latest-only
channel for `Ping`/`Pong` (key 65,535; matched by `client_time`).

## Encoding rules

- Integers: `u8`; `u16`, `i16`, `u64 fixed` little-endian; **varint**:
  unsigned LEB128 (7 bits per byte, low first, high bit = more), at most
  10 bytes, **minimal** (no trailing zero byte), at most 64 bits;
  **signed varint**: zigzag (`(v << 1) ^ (v >> 63)`), then varint.
  `u32`/`u16` values in varints are range-checked.
- `bool`: one byte, 0 or 1; anything else is an error.
- `f32`, `f64`: IEEE 754 bits, little-endian; NaN and ±infinity are
  errors (a NaN position would poison the receiver's rules).
- Text: a varint byte length, then UTF-8; each field has a maximum
  (names 32 bytes, game build 32, password 64).
- Lists: a varint count with a maximum, then the items.
- `Option<T>`: a `bool`, then `T` if true.
- A message is a tag byte, then its fields in order; bytes after the
  last field are an error.

Every value has exactly one encoding, so a message that decodes
re-encodes to the same bytes. The tests check this on every sample and on
every one of a million random, truncated and bit-flipped inputs that
happens to decode. Every decoding error names its byte offset.

## Client → server

| Tag | Message | Fields |
|---|---|---|
| 1 | `Hello` | `protocol: u16`, `build: text ≤ 32` (`__buildnumber.txt`), `fingerprint: u64 fixed` (rules fingerprint), `name: text ≤ 32`, `password: Option<text ≤ 64>` |
| 2 | `Ready` | — |
| 3 | `Ping` | `client_time: varint` (the client's own clock; echoed) |
| 4 | `States` | a [state list](#state-lists) |
| 5 | `Event` | `tick: varint u32`, a [player event](#player-events) |
| 6 | `SlotFillHash` | `batch: 3 × signed varint i32`, `hash: u64 fixed` |
| 7 | `WorldHash` | `events: varint u32` (world events applied), `hash: u64 fixed` (the [world state](#world-state)'s) |
| 8 | `Bye` | — |

**Stable forever:** `Hello`'s tag 1 and its `protocol` in bytes 1–2, so a
server of any version can read the version and answer with `Reject`.

## Server → client

| Tag | Message | Fields |
|---|---|---|
| 1 | `Reject` | `reason: u8`, then: 0 `Protocol { server: u16 }`, 1 `Build { server: text ≤ 32 }`, 2 `Fingerprint`, 3 `Full { max: varint u16 }`, 4 `Password`, 5 `Name` |
| 2 | `Welcome` | `player: varint u16`, `tick: varint u32` (the server's tick when sent), `next_seq: varint u32`, [world settings](#world-settings), `players: count ≤ 8` × [player info](#player-info) |
| 3 | `Pong` | `client_time: varint`, `tick: varint u32`, `fraction: u16` (of the next tick, in 1/65,536) |
| 4 | `States` | `player: varint u16`, a [state list](#state-lists) |
| 5 | `Event` | `seq: varint u32` (0, 1, 2, … without gaps), `tick: varint u32`, `kind: u8`, then: 0 `PlayerJoined { player: varint u16, name: text ≤ 32 }`, 1 `PlayerReady { player }`, 2 `PlayerLeft { player, reason: u8 }` (0 left, 1 timed out, 2 kicked), 3 `Player { player, event: player event }` |

**Stable forever:** `Reject`'s tag 1 and reason 0 `Protocol` with its
`u16`, so a client of any version can tell the user which version the
server speaks.

### World settings

`slot_seed: Option<u64 fixed>`; `lifepod: Option<3 × f32>` (Lifepod 5's
point, Unity world metres); `aurora: u8`, then 0 `Countdown { seconds: f32 }`
(game seconds after the start) or 1 `Held { exploded: bool }`;
`clock_start: f64` (`DayNightCycle.timePassed` at tick 0, game seconds);
`time_scale: f32` (game seconds per real second).

### Player info

`id: varint u16`, `name: text ≤ 32`, `ready: bool`,
`state: Option<tick: varint u32, state body>`,
`cinematic: Option<hatch: varint u16, start tick: varint u32>`.

### Player events

`kind: u8`, then: 0 `HatchUsed { hatch: varint u16 }` (the index of the
lifepod's hatch trigger), 1 `CinematicEnded`, 2 `Died { cause: u8 }`
(0 normal, 1 fire, 2 explosion), 3 `Respawned`,
4 `AnimatorTrigger { trigger: u8 }` (an index into the trigger table
below; out of range is an error).

## State lists

A count (at most 8), then for each state its tick and its body. The first
tick is a varint `u32`; each later one is a varint step from the previous
tick, at least 1 (ticks strictly increase; a step of 0 or an overflow is
an error).

### State body (46 bytes, 54 with a rotation)

| Bytes | Field | Unit |
|---|---|---|
| 2 | flags `u16` | bit 0 walking motor, 1 swimming, 2 grounded, 3 walk-grounded, 4 in the pod, 5 cinematic, 6 controls on, 7 rotation follows, bits 8–9 life (0 alive, 1 dead, 2 respawning, 3 restored); bits 10–15 must be 0 |
| 12 | position, 3 × `f32` | Unity world metres |
| 6 | velocity, 3 × `i16` | cm/s |
| 2 | yaw `u16` | the look's `rotationX` in 1/65,536 turns |
| 2 | pitch `i16` | the look's `rotationY` (up positive) in 0.01° |
| 8 | rotation, 4 × `i16` (only with flag bit 7) | the player transform's quaternion x, y, z, w × 32,767 |
| 14 | animator floats, 7 × `i16` | the float table below |
| 3 | animator bools, 24 bits | the bool table below; bits 19–23 must be 0 |

Measured (`cargo test -p sn-protocol -- --nocapture`, M10a): one state in
a message 46 B (54 B with a rotation); 4 states 188 B; 8 states 356 B; the
largest possible `Welcome` (8 players, longest names, every option
present) 862 B, under the 1,200 B packet limit.

## Quantization

The receiver only ever sees quantized values, so "the same state on both
machines" means the same quantized value. Out-of-range values saturate at
±32,767 (never −32,768); NaN becomes 0.

| Value | Wire | Step | Range / error |
|---|---|---|---|
| position | `f32` (round to nearest) | — | error ≤ 2⁻¹³ m (0.12 mm) below 4,096 m from the origin, ≤ 2⁻¹⁴ m below 2,048 m (tested) |
| velocity | `i16` | 1 cm/s | ±327.67 m/s |
| yaw | `u16` | 1/65,536 turn (≈ 0.0055°) | wraps; 0 ≤ back < 360° |
| pitch | `i16` | 0.01° | ±327.67° |
| rotation | `i16` | 1/32,767 | the receiver normalizes |

### Animator tables

The parameters `sn_sim::body` gives the player's animator (M9g). A value's
index is its slot.

Floats (scale = wire steps per unit):

| Slot | Parameter | Scale | Range |
|---|---|---|---|
| 0 | `move_speed` | 1,000 (1 mm/s) | ±32.8 m/s |
| 1 | `move_speed_x` | 1,000 | ±32.8 m/s |
| 2 | `move_speed_y` | 1,000 | ±32.8 m/s |
| 3 | `move_speed_z` | 1,000 | ±32.8 m/s |
| 4 | `view_pitch` | 100 (0.01°) | ±327.67° |
| 5 | `view_turn` | 10 (0.1°/s) | ±3,276.7°/s (a faster flick saturates) |
| 6 | `verticalOffset` | 10,000 (0.1 mm) | ±3.28 m |

Bools (bit = slot): 0 `grab`, 1 `bash`, 2 `is_underwater`,
3 `cinematics_enabled`, 4 `using_tool`, 5 `using_tool_alt`,
6 `holding_tool`, 7 `in_seamoth`, 8 `in_exosuit`, 9 `cyclops_steering`,
10 `bleeder`, 11 `jump`, 12 `diving`, 13 `diving_land`, 14 `on_surface`,
15 `holding_welder`, 16 `using_pda`, 17 `using_builder`, 18 `vr_active`.

Triggers (sent as `AnimatorTrigger` events): 0 `player_death`,
1 `player_death_fire`, 2 `player_death_explosion`.

A name the sender's rules produce that is not in a table cannot be sent;
`AnimState::set_*` returns `false` and the caller logs it (M10h).

## Hash

FNV-1a, 64 bits (offset basis `0xcbf29ce484222325`, prime
`0x100000001b3`), over bytes; numbers little-endian, floats by their bits,
text with its length first. Used for the rules fingerprint, the slot-fill
hash and the world-state hash; what goes into each is defined where it is
computed (M10c, M10e). A check against accidental differences, not a
security measure.

## Bandwidth

M10a's estimate, for 25 packets a second of 4 states each: 188 B per
message + 28 B of IPv4/UDP headers = 5,400 B/s up per player. M10d sends
6 states per message (each tick three times) and measured, in the sync
oracle, 7,969–8,144 B/s up per player including `sn-net`'s headers,
acks, pings and IPv4/UDP headers. M10f's target is < 10 KB/s.

## Rules fingerprint

`Hello.fingerprint` (M10e; `sn_assets::rules`, the same code in the
server and the client): FNV-1a 64 over, in order, each part's name
(length-prefixed) and its own FNV-1a 64 hash:

| Part | What is hashed |
|---|---|
| `player` | the `Debug` text of `sn_sim::player::PlayerParams` from the install (speeds, sizes, the walking motor, ocean level, the physics step) |
| `vitals` | the `Debug` text of `sn_sim::vitals::VitalsParams` |
| `look` | the `Debug` text of `sn_sim::look::LookParams` |
| `techdata` | the raw bytes of the `Balance/TechData` text asset |
| `exploder` | the `Debug` text of `sn_assets::ExploderCode` (the Aurora's numbers from the code) |

Rust prints floats exactly and the same on every platform, and these
structs hold no hash maps, so the text is canonical. The server logs every
part's hash, so a mismatch can be traced to its part. Build 10 on the dev
machine: `d269297a0830e68b` (*confirmed* stable across two reads and
between a debug and a release build, M10e). Not in it yet: the cinematic
clips (M10h plays them on remote bodies) and the body's animator rules
(remote bodies use the owner's values, so they cannot desync).

The tick length is `round(fixedDeltaTime × 10⁶)` µs (20,000 on build 10),
and the speed above which a move is implausible (server) or a teleport
(client) is twice the fastest of the swimming, vertical, walking and
falling speeds (100 m/s on build 10).

## Slot-fill hash

`SlotFillHash { batch, hash }` (M10f; `sn_assets::CollisionLoader::
slot_fill_hashes`): per terrain batch a client loaded, FNV-1a 64 over each
spawn the batch's slots were filled with, in load order: the slot
object's id (length-prefixed), the spawned class id (length-prefixed),
and the spawn's world transform: position, rotation and scale as `f32`
bits. The server keeps the first report per batch and counts any later
one that differs. Clients fill slots from the server's slot seed on their
own, so this is the check that they filled them alike.

## World state

What every machine must hold identically (`crates/sn-session`,
`WorldState`, M10c). It is a function of the world settings and the
ordered world event log: a client builds it from `Welcome` (the state
after `next_seq` events: settings and `players`) and applies each world
`Event` in `seq` order; an event whose `seq` is not the next one, or that
names an unknown player or adds a known one again, is an error (none can
happen on the reliable channel; the tests check it).

It holds the settings, the number of events applied, and per player: id,
name, ready, and the hatch cinematic it is in (hatch, start tick). Events
change it so: `PlayerJoined` adds a player (not ready, no cinematic),
`PlayerReady` sets ready, `PlayerLeft` removes, `HatchUsed` sets the
cinematic (the event's tick is the start), `CinematicEnded` clears it;
`Died`, `Respawned`, `AnimatorTrigger` only pass through (life is in each
player state).

**Hash** (FNV-1a 64, numbers as `u64` LE unless said): slot seed (`1, seed`
or `0`); lifepod (`1`, then x, y, z as `f32` bits, or `0`); Aurora (`0`,
seconds as `f32` bits; or `1`, exploded); clock start as `f64` bits, time
scale as `f32` bits; events applied; player count; then per player in id
order: id, name (length-prefixed), ready, cinematic (`1`, hatch, start
tick, or `0`). Clients report it as `WorldHash { events, hash }`; the
server keeps its own after each of the last 4,096 event counts and
compares.

## Server rules (M10c)

- **Join:** `Hello` is checked in this order: protocol, build, rules
  fingerprint, password (if the server has one), room (at most
  `max_players`, never above 8), name (not empty, no leading or trailing
  spaces, no control characters, not in use by a present player, case
  ignored). A failure answers `Reject` and closes the connection 1 s
  later. Success answers `Welcome`, then the world event `PlayerJoined`
  (to everyone, the joiner included). A name keeps its player id for the
  server's life. No `Hello` within 5 s: closed.
- **Ready:** `PlayerReady` to everyone. Before it, a player's states and
  events are ignored (others do not see it yet).
- **States** (unordered channel): relayed to every other
  joined player as sent; the newest is kept for `Welcome`s. States more
  than 5 s ahead of the server's clock are dropped. A speed between two
  states above the configured maximum, outside cinematics, is counted and
  logged, not refused.
- **Events:** a player's event becomes a world event with the next `seq`
  and the event's own tick, sent to everyone (the sender included, so its
  state and hash follow the same log).
- **Leaving:** `Bye`, or the transport's close or timeout: `PlayerLeft`
  (`Left` or `TimedOut`) to everyone else.
- **Clock:** tick = time since the server started ÷ the tick length (the
  install's `fixedDeltaTime`); `Pong` carries it and the fraction of the
  next tick (latest channel, key 65,535).
- **Checks:** `WorldHash` against the server's hash after the same event
  count; `SlotFillHash` against the first report for that batch.

## Client rules (M10d)

`crates/sn-session`, `Client`: a state machine on the caller's clock.

- **Join:** `Hello` is queued at creation; `Welcome` gives the player id
  and the world state; `ready()` sends `Ready` once.
- **Clock:** pings every 50 ms for the first 20, then every 250 ms. Each
  `Pong` gives a sample: round trip = now − `client_time`; offset = the
  server's time + half the round trip − now. The sample with the
  shortest round trip in the last 10 s is the estimate (queueing only
  ever adds delay). The applied offset moves towards the estimate by at
  most 1 % of the time passed, or jumps if more than 250 ms off.
- **Own steps:** `steps(now)` gives the physics steps due, numbered in
  the server's ticks: consecutive, never repeated, at most 10 per call.
- **Own states:** after each step `push_state`; every second step sends
  the newest 6 (unordered channel), once `Ready` is sent.
- **Drawing others:** the drawn tick is the server's tick minus a delay.
  Each newly arrived tick's lateness (the server's tick now − that tick)
  is kept for 10 s; the delay wants their maximum + 1 tick, between 2 and
  50 ticks. For 1 s after the first remote state it takes that at once
  (only growing); after that it grows by at most 10 % and shrinks by at
  most 1 % of the time passed. `sample(player, tick)`: the exact state
  if that tick arrived; between two consecutive ticks linear; across
  missing ticks a cubic Hermite with the received velocities, unless the
  two are further apart than 30 m/s allows + 1 m (a teleport: the earlier
  state is kept); past the newest state along its velocity for 250 ms,
  then held. The look is interpolated the short way round. States more
  than 64 ticks behind the drawn tick are dropped (the newest is kept).
- **Events:** applied to the world state on arrival (the hash follows
  the log); released to the caller in `seq` order once the drawn tick
  reaches each event's tick (an event not yet due holds back the ones
  after it).
- **Checks:** every 50 ticks of the server's clock, `WorldHash` with the
  event count.

## Packets (`sn-net`)

The datagrams that carry the messages (`crates/sn-net`, M10b). At most
1,200 bytes each (`MAX_PACKET`); a message is at most 1,161 bytes
(`MAX_MESSAGE`). No fragmentation: a larger message is refused when it is
queued. No encryption.

**Header** (every packet): magic `snrs` (4 bytes), packet-layer version
`u8` = 1, kind `u8`, connection id `u32` LE (random, chosen by the client;
a packet with another id is ignored, so a restarted client's old packets
cannot mix in).

| Kind | Packet | After the header |
|---|---|---|
| 1 | `Connect` | — (sent every 100 ms until answered) |
| 2 | `Accept` | — (sent again if the same `Connect` comes again) |
| 3 | `Refuse` | reason `u8`: 0 full |
| 4 | `Data` | seq `u32` LE; acks flag `u8`, then if 1: ack `u32`, ack bits `u32`, ack delay varint (µs); then frames to the end |
| 5 | `Disconnect` | — (sent 3 times; not acknowledged) |

**Frames:** kind `u8`, then 1 **reliable** `{ id: varint u32, length:
varint, bytes }`, 2 **latest** `{ key: varint u16, length: varint,
bytes }` or 3 **unordered** `{ length: varint, bytes }` (M10d). Varints as in `sn-protocol`, minimal; anything malformed
drops the whole datagram (counted, never a panic).

**Acks:** `ack` is the newest packet received, bit `i` of the bits is
packet `ack − 1 − i`. Every data packet carries them. The ack delay is
how long `ack` waited before this packet left, so the round trip is
`now − sent − delay` (RFC 6298's smoothed estimate). A packet with a
reliable frame is acknowledged within 10 ms, with an empty data packet if
nothing else is going; with nothing to send for 250 ms, an empty packet
keeps the connection alive. Nothing heard for 5 s: the peer is gone.

**Reliable channel:** messages numbered from 0; each is sent until a
packet carrying it is acknowledged, again after the resend timeout
(smoothed round trip + max(4 × variance, 10 ms) + 10 ms, between 50 ms and
2 s; before the first sample 250 ms; doubling per resend of the same
message, up to 2 s). The receiver delivers in order, exactly once
(numbers behind the next expected one are duplicates; up to 8,192 ahead
are kept). At most 4,096 waiting for acks per connection, and at most 32
packets with reliable frames in flight (the ack window is 33 packets, so
a longer burst could not be acknowledged and would be resent for
nothing).

**Unordered channel** (M10d): every queued message goes once (at most 256
wait; beyond that the oldest is dropped and counted); the receiver
delivers everything that arrives, in arrival order (duplicate datagrams
are already dropped by sequence number). Lost ones are not resent.

**Latest-only channel:** per key, only the newest queued message goes
out; the receiver delivers a frame only if its packet is newer than the
last one delivered for that key. Lost frames are not resent.

Measured on the simulated network (`cargo test -p sn-net -- --nocapture
--test-threads=1`, 60 s of traffic: about 20 reliable messages a second
each way plus a burst of 300, two latest keys every 40 ms; then 10 s to
settle):

| Preset (one way) | Round trip estimate vs configured | Reliable resent / sent (client) | Latest delivered (client) |
|---|---|---|---|
| lan (1 ms) | 2.00 / 2.00 ms | 0 / 1,458 | 3,000 of 3,000 |
| internet (40 ± 10 ms, 1 % loss) | 78.6 / 80 ms (−1.8 %) | 25 / 1,559 | 2,956 of 3,000 |
| bad (150 ± 50 ms, 5 % loss, 1 % duplicates) | 290.4–317.6 / 300 ms, both ends, 4 seeds (−3.2 … +5.9 %) | 149–288 / 1,603–1,790 | 2,300–2,362 of 2,994 (the rest lost, or overtaken and dropped as stale) |

Every reliable message arrived exactly once and in order in every run;
the latest channel never went back.

