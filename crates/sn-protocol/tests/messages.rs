//! M10a's checks: every message round-trips and re-encodes to the same
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
    v
}

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
