//! M10a's checks (M9d3a's for the item messages): every message round-trips and re-encodes to the same
//! bytes; the largest messages fit in a 1,200-byte packet; a million
//! seeded random, truncated and bit-flipped inputs never panic. Sizes are
//! printed (`cargo test -p sn-protocol -- --nocapture`).

use sn_protocol::anim::{BOOLS, FLOATS, TRIGGERS};
use sn_protocol::quant;
use sn_protocol::*;

/// A small seeded generator (splitmix64), so every run is the same.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

/// A plausible state at `tick`, all fields quantized from sim values.
fn state(rng: &mut Rng, tick: u32) -> PlayerState {
    let mut anim = AnimState::default();
    for (name, _) in FLOATS {
        anim.set_float(name, (rng.unit() - 0.5) * 20.0);
    }
    for name in BOOLS {
        anim.set_bool(name, rng.coin());
    }
    let life = [Life::Alive, Life::Dead, Life::Respawning, Life::Restored][rng.below(4) as usize];
    PlayerState {
        tick: Tick(tick),
        position: [
            quant::position((rng.unit() - 0.5) * 4000.0),
            quant::position((rng.unit() - 0.9) * 500.0),
            quant::position((rng.unit() - 0.5) * 4000.0),
        ],
        velocity: [
            quant::velocity((rng.unit() - 0.5) * 16.0),
            quant::velocity((rng.unit() - 0.5) * 16.0),
            quant::velocity((rng.unit() - 0.5) * 16.0),
        ],
        yaw: quant::turn(rng.unit() * 720.0 - 360.0),
        pitch: quant::fixed((rng.unit() - 0.5) * 174.0, quant::PITCH_SCALE),
        rotation: rng
            .coin()
            .then(|| [0.1, -0.7, 0.05, 0.7].map(|c: f64| quant::fixed(c, quant::QUATERNION_SCALE))),
        walking: rng.coin(),
        swimming: rng.coin(),
        grounded: rng.coin(),
        walk_grounded: rng.coin(),
        in_pod: rng.coin(),
        cinematic: rng.coin(),
        controls: rng.coin(),
        life,
        anim,
    }
}

fn states(rng: &mut Rng, n: usize) -> Vec<PlayerState> {
    let mut tick = rng.below(1_000_000) as u32;
    (0..n)
        .map(|_| {
            tick += 1 + rng.below(3) as u32;
            state(rng, tick)
        })
        .collect()
}

fn player_events() -> Vec<PlayerEventKind> {
    let mut v = vec![
        PlayerEventKind::HatchUsed { hatch: 3 },
        PlayerEventKind::CinematicEnded,
        PlayerEventKind::Respawned,
    ];
    for cause in [DeathCause::Normal, DeathCause::Fire, DeathCause::Explosion] {
        v.push(PlayerEventKind::Died { cause });
    }
    for t in 0..TRIGGERS.len() {
        v.push(PlayerEventKind::AnimatorTrigger { trigger: t as u8 });
    }
    for on in [false, true] {
        v.push(PlayerEventKind::CinematicPrepare { on });
        v.push(PlayerEventKind::CinematicPlay { on });
    }
    v
}

fn client_samples(rng: &mut Rng) -> Vec<ClientMessage> {
    let mut v = vec![
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            build: "10".into(),
            fingerprint: 0x0123_4567_89ab_cdef,
            name: "Ryley".into(),
            password: None,
        },
        ClientMessage::Hello {
            protocol: 7,
            build: "b".repeat(MAX_BUILD),
            fingerprint: u64::MAX,
            name: "ż".repeat(MAX_NAME / 2),
            password: Some("p".repeat(MAX_PASSWORD)),
        },
        ClientMessage::Ready,
        ClientMessage::Ping { client_time: 0 },
        ClientMessage::Ping {
            client_time: u64::MAX,
        },
        ClientMessage::States(Vec::new()),
        ClientMessage::States(states(rng, 1)),
        ClientMessage::States(states(rng, 4)),
        ClientMessage::States(states(rng, MAX_STATES)),
        ClientMessage::SlotFillHash {
            batch: [i32::MIN, 0, i32::MAX],
            hash: 42,
        },
        ClientMessage::WorldHash {
            events: u32::MAX,
            hash: 7,
        },
        ClientMessage::Bye,
    ];
    for kind in player_events() {
        v.push(ClientMessage::Event(PlayerEvent {
            tick: Tick(rng.below(u64::from(u32::MAX)) as u32),
            kind,
        }));
    }
    for (i, request) in requests(rng).into_iter().enumerate() {
        v.push(ClientMessage::Request {
            req: [0, 1, u32::MAX][i % 3],
            request,
        });
    }
    v
}

/// Every key kind, at its longest where it has text.
fn keys() -> Vec<EntityKey> {
    vec![
        EntityKey::Placed("a1b2c3d4-0000-4000-8000-0123456789ab".into()),
        EntityKey::Placed("x".repeat(items::MAX_ID)),
        EntityKey::Slot {
            placeholder: "p".repeat(items::MAX_ID),
            slot: u16::MAX,
            copy: 3,
        },
        EntityKey::Spawned(ItemId(u32::MAX)),
    ]
}

fn drop_at(rng: &mut Rng) -> DropAt {
    DropAt {
        point: [0; 3].map(|_| quant::position(rng.unit() * 4000.0 - 2000.0)),
        rotation: [0; 4].map(|_| (rng.next() as u16) as i16),
    }
}

fn death_drops(rng: &mut Rng, n: usize) -> Vec<DeathDrop> {
    (0..n)
        .map(|_| DeathDrop {
            offset: [0; 3].map(|_| quant::fixed(rng.unit() * 3.0 - 1.5, quant::DROP_OFFSET_SCALE)),
            rotation: [0; 4].map(|_| (rng.next() as u16) as i16),
        })
        .collect()
}

fn requests(rng: &mut Rng) -> Vec<Request> {
    let mut v = Vec::new();
    for key in keys() {
        v.push(Request::PickUp {
            key: key.clone(),
            class: "c".repeat(items::MAX_ID),
        });
        for kind in [HitKind::Hand, HitKind::Bash, HitKind::Mech] {
            v.push(Request::Hit {
                key: key.clone(),
                class: String::new(),
                kind,
                spawn: [1.0, -2.5, 3.25],
            });
        }
    }
    v.push(Request::Drop {
        item: ItemId(7),
        at: drop_at(rng),
    });
    for to in [Store::Grid, Store::Slots, Store::PodLocker] {
        for target in [
            MoveTarget::Store,
            MoveTarget::Item(ItemId(300)),
            MoveTarget::Slot("s".repeat(items::MAX_SLOT)),
        ] {
            v.push(Request::Move {
                item: ItemId(9),
                to,
                target,
            });
        }
    }
    for n in [0, 1, items::MAX_DROPS] {
        v.push(Request::Lose {
            origin: [10.0, -5.0, 20.0],
            drops: death_drops(rng, n),
        });
    }
    v
}

fn item_events(rng: &mut Rng) -> Vec<ItemEvent> {
    let mut v = Vec::new();
    for key in keys() {
        v.push(ItemEvent::PickedUp {
            key: key.clone(),
            class: "c".into(),
        });
        for n in [0, 1, items::MAX_DRAWS] {
            v.push(ItemEvent::Hit {
                key: key.clone(),
                class: "c".into(),
                kind: HitKind::Hand,
                spawn: [0.0, 0.5, 0.0],
                draws: (0..n)
                    .map(|i| [0.0, 1.0, rng.unit() as f32][i % 3])
                    .collect(),
            });
        }
    }
    v.push(ItemEvent::Dropped {
        item: ItemId(1),
        at: drop_at(rng),
    });
    v.push(ItemEvent::Moved {
        item: ItemId(2),
        to: Store::PodLocker,
        target: MoveTarget::Slot("Tank".into()),
    });
    for n in [0, items::MAX_DROPS] {
        v.push(ItemEvent::Lost {
            origin: [1.0, 2.0, 3.0],
            drops: death_drops(rng, n)
                .into_iter()
                .enumerate()
                .map(|(i, d)| (ItemId(1_000_000 + i as u32), d))
                .collect(),
        });
    }
    v
}

const REASONS: [RefuseReason; 7] = [
    RefuseReason::Gone,
    RefuseReason::Unknown,
    RefuseReason::NoRoom,
    RefuseReason::NotHere,
    RefuseReason::NoItem,
    RefuseReason::NotAllowed,
    RefuseReason::NotReady,
];

fn welcome(rng: &mut Rng, players: usize, long: bool) -> ServerMessage {
    ServerMessage::Welcome {
        player: PlayerId(u16::MAX),
        tick: Tick(u32::MAX),
        next_seq: u32::MAX,
        settings: WorldSettings {
            slot_seed: Some(u64::MAX),
            lifepod: Some([-1200.5, 0.25, 30.0]),
            aurora: AuroraStart::Countdown { seconds: 3456.7 },
            clock_start: 480.0,
            time_scale: 1.0,
        },
        players: (0..players)
            .map(|i| PlayerInfo {
                id: PlayerId(if long { u16::MAX } else { i as u16 }),
                name: if long {
                    "n".repeat(MAX_NAME)
                } else {
                    format!("p{i}")
                },
                ready: rng.coin(),
                state: (long || rng.coin()).then(|| {
                    let mut s = state(rng, u32::MAX);
                    if long {
                        s.rotation = Some([i16::MAX; 4]);
                    }
                    s
                }),
                cinematic: (long || rng.coin()).then_some((u16::MAX, Tick(u32::MAX))),
            })
            .collect(),
    }
}

fn server_samples(rng: &mut Rng) -> Vec<ServerMessage> {
    let mut v = vec![
        ServerMessage::Reject(RejectReason::Protocol { server: 2 }),
        ServerMessage::Reject(RejectReason::Build {
            server: "10".into(),
        }),
        ServerMessage::Reject(RejectReason::Fingerprint),
        ServerMessage::Reject(RejectReason::Full { max: 8 }),
        ServerMessage::Reject(RejectReason::Password),
        ServerMessage::Reject(RejectReason::Name),
        welcome(rng, 0, false),
        welcome(rng, 3, false),
        welcome(rng, MAX_PLAYERS, true),
        ServerMessage::Pong {
            client_time: 123_456_789,
            tick: Tick(50),
            fraction: 0x8000,
        },
        ServerMessage::States {
            player: PlayerId(2),
            states: states(rng, 4),
        },
    ];
    let mut aurora = welcome(rng, 1, false);
    if let ServerMessage::Welcome { settings, .. } = &mut aurora {
        settings.aurora = AuroraStart::Held { exploded: true };
        settings.slot_seed = None;
        settings.lifepod = None;
    }
    v.push(aurora);
    let mut kinds = vec![
        WorldEventKind::PlayerJoined {
            player: PlayerId(1),
            name: "Ryley".into(),
        },
        WorldEventKind::PlayerReady {
            player: PlayerId(1),
        },
    ];
    for reason in [
        LeaveReason::Left,
        LeaveReason::TimedOut,
        LeaveReason::Kicked,
    ] {
        kinds.push(WorldEventKind::PlayerLeft {
            player: PlayerId(1),
            reason,
        });
    }
    for event in player_events() {
        kinds.push(WorldEventKind::Player {
            player: PlayerId(3),
            event,
        });
    }
    for (i, event) in item_events(rng).into_iter().enumerate() {
        kinds.push(WorldEventKind::Item {
            player: PlayerId(7),
            req: i as u32 * 1000,
            event,
        });
    }
    for reason in REASONS {
        v.push(ServerMessage::Refused { req: 77, reason });
    }
    for (index, total, n) in [(0, 1, 0), (2, 3, 100), (0, u32::MAX, MAX_CHUNK)] {
        v.push(ServerMessage::WorldChunk {
            index,
            total,
            bytes: (0..n).map(|i| (i * 31) as u8).collect(),
        });
    }
    for (i, kind) in kinds.into_iter().enumerate() {
        v.push(ServerMessage::Event(WorldEvent {
            seq: i as u32,
            tick: Tick(1000 + i as u32),
            kind,
        }));
    }
    v
}

#[test]
fn every_message_round_trips_canonically() {
    let mut rng = Rng(1);
    for m in client_samples(&mut rng) {
        let bytes = m.encode();
        let back = ClientMessage::decode(&bytes).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        assert_eq!(back, m);
        assert_eq!(back.encode(), bytes);
    }
    for m in server_samples(&mut rng) {
        let bytes = m.encode();
        let back = ServerMessage::decode(&bytes).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        assert_eq!(back, m);
        assert_eq!(back.encode(), bytes);
    }
}

#[test]
fn hello_and_reject_keep_their_first_bytes() {
    // Any version's server reads these, so they must never move.
    let hello = ClientMessage::Hello {
        protocol: 0x0201,
        build: String::new(),
        fingerprint: 0,
        name: String::new(),
        password: None,
    };
    assert_eq!(hello.encode()[..3], [1, 0x01, 0x02]);
    let reject = ServerMessage::Reject(RejectReason::Protocol { server: 0x0201 });
    assert_eq!(reject.encode(), [1, 0, 0x01, 0x02]);
    assert!(
        RejectReason::Build { server: "9".into() }
            .to_string()
            .contains("same game build")
    );
}

#[test]
fn malformed_messages_name_their_offset() {
    let e = ClientMessage::decode(&[]).unwrap_err();
    assert_eq!(e.offset, 0);
    let e = ClientMessage::decode(&[99]).unwrap_err();
    assert!(e.message.contains("unknown client message tag"), "{e}");
    let e = ClientMessage::decode(&[2, 0]).unwrap_err();
    assert_eq!(e.offset, 1);
    // A second state with the same tick: tag, count, then tick 100,000
    // (a 3-byte varint) and the first state's body.
    let mut rng = Rng(2);
    let (first, second) = (state(&mut rng, 100_000), state(&mut rng, 100_000));
    let mut bytes = ClientMessage::States(vec![first]).encode();
    bytes[1] = 2;
    let step_at = bytes.len();
    bytes.push(0);
    bytes.extend_from_slice(&ClientMessage::States(vec![second]).encode()[2 + 3..]);
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, step_at, "{e}");
    assert!(e.message.contains("increase"), "{e}");
    // An unknown flag bit in a state.
    let mut bytes = ClientMessage::States(vec![state(&mut rng, 100_000)]).encode();
    let flags_at = 2 + 3;
    bytes[flags_at + 1] |= 0x80;
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, flags_at, "{e}");
}

#[test]
fn malformed_item_messages_name_their_offset() {
    // A chunk whose index is not below its total: the error is at the
    // total.
    let mut bytes = ServerMessage::WorldChunk {
        index: 1,
        total: 2,
        bytes: vec![],
    }
    .encode();
    bytes[2] = 1;
    let e = ServerMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, 2, "{e}");
    // A luck draw above 1.
    let mut rng = Rng(5);
    let m = ServerMessage::Event(WorldEvent {
        seq: 0,
        tick: Tick(0),
        kind: WorldEventKind::Item {
            player: PlayerId(0),
            req: 0,
            event: ItemEvent::Hit {
                key: EntityKey::Spawned(ItemId(1)),
                class: String::new(),
                kind: HitKind::Hand,
                spawn: [0.0; 3],
                draws: vec![0.5],
            },
        },
    });
    let mut bytes = m.encode();
    let at = bytes.len() - 4;
    bytes[at..].copy_from_slice(&1.5f32.to_le_bytes());
    let e = ServerMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, at, "{e}");
    assert!(e.message.contains("draw"), "{e}");
    // Unknown kinds of key, store, request; too many drops; an unknown
    // refusal.
    let req = |request| ClientMessage::Request { req: 0, request }.encode();
    let mut bytes = req(Request::PickUp {
        key: EntityKey::Spawned(ItemId(0)),
        class: String::new(),
    });
    bytes[3] = 9;
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, 3, "{e}");
    assert!(e.message.contains("key kind"), "{e}");
    let mut bytes = req(Request::Move {
        item: ItemId(0),
        to: Store::Grid,
        target: MoveTarget::Store,
    });
    bytes[4] = 3;
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, 4, "{e}");
    let mut bytes = req(Request::Lose {
        origin: [0.0; 3],
        drops: death_drops(&mut rng, 1),
    });
    bytes[2] = 5;
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, 2, "{e}");
    let mut bytes = req(Request::Lose {
        origin: [0.0; 3],
        drops: vec![],
    });
    let n = bytes.len() - 1;
    bytes[n] = items::MAX_DROPS as u8 + 1;
    let e = ClientMessage::decode(&bytes).unwrap_err();
    assert_eq!(e.offset, n, "{e}");
    let e = ServerMessage::decode(&[6, 0, 7]).unwrap_err();
    assert_eq!(e.offset, 2, "{e}");
}

#[test]
fn largest_messages_fit_one_packet_and_sizes_are_logged() {
    let mut rng = Rng(3);
    let one = ClientMessage::States(states(&mut rng, 1)).encode().len();
    let four = ClientMessage::States(states(&mut rng, 4)).encode().len();
    let mut rot = states(&mut rng, 1);
    rot[0].rotation = Some([1; 4]);
    let one_rotated = ClientMessage::States(rot).encode().len();
    let max_states = ClientMessage::States(states(&mut rng, MAX_STATES))
        .encode()
        .len();
    let welcome = welcome(&mut rng, MAX_PLAYERS, true).encode().len();
    println!(
        "sizes: 1 state {one} B (with rotation {one_rotated} B), 4 states {four} B, {MAX_STATES} states {max_states} B, largest Welcome ({MAX_PLAYERS} players) {welcome} B"
    );
    // 25 packets a second of 4 states, plus 28 B of IPv4 + UDP headers
    // each (sn-net's own header not counted yet).
    let up = 25 * (four + 28);
    println!("player upload at 25 packets/s with 4 states each: {up} B/s");
    assert!(max_states <= 1200, "{max_states}");
    assert!(welcome <= 1200, "{welcome}");
    // Every message fits `sn-net`'s largest message (1,161 B).
    let mut largest = 0;
    for m in client_samples(&mut rng) {
        largest = largest.max(m.encode().len());
    }
    for m in server_samples(&mut rng) {
        largest = largest.max(m.encode().len());
    }
    println!("largest message of all samples (items included): {largest} B");
    assert!(largest <= 1161, "{largest}");
}

/// Decodes and, if that works, checks the canonical round trip.
fn probe(bytes: &[u8], ok: &mut [u64; 2]) {
    if let Ok(m) = ClientMessage::decode(bytes) {
        assert_eq!(m.encode(), bytes, "client {m:?}");
        ok[0] += 1;
    }
    if let Ok(m) = ServerMessage::decode(bytes) {
        assert_eq!(m.encode(), bytes, "server {m:?}");
        ok[1] += 1;
    }
}

#[test]
fn a_million_bad_inputs_never_panic() {
    let mut rng = Rng(4);
    let mut valid: Vec<Vec<u8>> = client_samples(&mut rng)
        .iter()
        .map(|m| m.encode())
        .collect();
    valid.extend(server_samples(&mut rng).iter().map(|m| m.encode()));
    let mut ok = [0u64; 2];
    let (mut random, mut truncated, mut flipped) = (0, 0, 0);
    for i in 0..1_000_000u32 {
        match i % 3 {
            0 => {
                // Random bytes, starting with a real tag half the time.
                let n = rng.below(80) as usize;
                let mut b: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                if rng.coin() && !b.is_empty() {
                    b[0] = 1 + rng.below(8) as u8;
                }
                probe(&b, &mut ok);
                random += 1;
            }
            1 => {
                let v = &valid[rng.below(valid.len() as u64) as usize];
                let n = rng.below(v.len() as u64) as usize;
                probe(&v[..n], &mut ok);
                truncated += 1;
            }
            _ => {
                let mut b = valid[rng.below(valid.len() as u64) as usize].clone();
                for _ in 0..=rng.below(3) {
                    let at = rng.below(b.len() as u64) as usize;
                    b[at] ^= 1 << rng.below(8);
                }
                probe(&b, &mut ok);
                flipped += 1;
            }
        }
    }
    println!(
        "robustness: {random} random, {truncated} truncated, {flipped} bit-flipped inputs; decoded anyway (and re-encoded identically): {} as client, {} as server messages",
        ok[0], ok[1]
    );
}
