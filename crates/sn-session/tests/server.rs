//! M10c's checks of the server's logic (and M9d3c's of the item requests),
//! with simulated clients that keep their own copy of the world state from
//! what they receive. Messages go
//! through the codec both ways, as on the wire. Counts are printed
//! (`cargo test -p sn-session -- --nocapture`).

use std::collections::BTreeMap;
use std::sync::Arc;

use sn_protocol::*;
use sn_session::*;

mod common;

const TICK_US: u64 = 20_000;

fn config() -> ServerConfig {
    ServerConfig {
        build: "10".into(),
        fingerprint: 0xfeed,
        password: None,
        max_players: 8,
        settings: WorldSettings {
            slot_seed: Some(1),
            lifepod: Some([-30.5, 0.0, 120.25]),
            aurora: AuroraStart::Countdown { seconds: 3456.0 },
            clock_start: 480.0,
            time_scale: 1.0,
        },
        tick_us: TICK_US,
        max_speed: 30.0,
        rules: Default::default(),
    }
}

/// [`config`] with the synthetic item rules.
fn item_config() -> ServerConfig {
    ServerConfig {
        rules: common::rules(),
        ..config()
    }
}

fn hello(name: &str) -> ClientMessage {
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        build: "10".into(),
        fingerprint: 0xfeed,
        name: name.into(),
        password: None,
    }
}

/// A client as far as the session is concerned.
#[derive(Default)]
struct Client {
    player: Option<PlayerId>,
    world: Option<WorldState>,
    /// World events received, in order.
    log: Vec<WorldEvent>,
    rejected: Option<RejectReason>,
    /// The `WorldChunk`s, and whether all came.
    chunks: Vec<Vec<u8>>,
    loaded: bool,
    refused: Vec<(u32, RefuseReason)>,
    rules: Arc<ItemRules>,
    /// States received per player (on the latest channel).
    states: BTreeMap<PlayerId, Vec<Tick>>,
    pongs: Vec<(Tick, u16)>,
    closed: bool,
}

impl Client {
    fn receive(&mut self, channel: Channel, m: &ServerMessage) {
        // As on the wire.
        let m = ServerMessage::decode(&m.encode()).unwrap();
        match m {
            ServerMessage::Reject(r) => self.rejected = Some(r),
            ServerMessage::Welcome {
                player,
                next_seq,
                settings,
                players,
                ..
            } => {
                assert_eq!(channel, Channel::Reliable);
                self.player = Some(player);
                self.world = Some(WorldState::from_welcome(
                    settings,
                    next_seq,
                    &players,
                    self.rules.clone(),
                ));
            }
            ServerMessage::Refused { req, reason } => {
                assert_eq!(channel, Channel::Reliable);
                self.refused.push((req, reason));
            }
            ServerMessage::WorldChunk {
                index,
                total,
                bytes,
            } => {
                assert_eq!(channel, Channel::Reliable);
                assert_eq!(index as usize, self.chunks.len(), "chunks in order");
                self.chunks.push(bytes);
                if index + 1 == total {
                    let all: Vec<u8> = self.chunks.concat();
                    let w = self.world.as_mut().expect("a chunk before Welcome");
                    w.load_items(&all).unwrap_or_else(|e| panic!("{e}"));
                    self.loaded = true;
                }
            }
            ServerMessage::Event(e) => {
                assert_eq!(channel, Channel::Reliable);
                assert!(self.loaded, "an event before the world state");
                let w = self.world.as_mut().expect("an event before Welcome");
                w.apply(&e).unwrap_or_else(|err| panic!("{err}"));
                self.log.push(e);
            }
            ServerMessage::States { player, states } => {
                assert_eq!(channel, Channel::Unordered);
                self.states
                    .entry(player)
                    .or_default()
                    .extend(states.iter().map(|s| s.tick));
            }
            ServerMessage::Pong { tick, fraction, .. } => {
                assert_eq!(channel, Channel::Latest(PING_KEY));
                self.pongs.push((tick, fraction));
            }
        }
    }
}

/// A server and its clients, with messages delivered at once.
struct World {
    server: Server,
    clients: BTreeMap<ConnId, Client>,
    next_conn: u32,
    now: u64,
    rules: Arc<ItemRules>,
}

impl World {
    fn new() -> World {
        World::with(config())
    }

    fn with(config: ServerConfig) -> World {
        World {
            rules: config.rules.clone(),
            server: Server::new(config, 1_000_000),
            clients: BTreeMap::new(),
            next_conn: 0,
            now: 1_000_000,
        }
    }

    fn connect(&mut self) -> ConnId {
        let c = ConnId(self.next_conn);
        self.next_conn += 1;
        self.clients.insert(
            c,
            Client {
                rules: self.rules.clone(),
                ..Client::default()
            },
        );
        self.server.connected(c, self.now);
        c
    }

    fn send(&mut self, from: ConnId, m: &ClientMessage) {
        self.server.received(from, &m.encode(), self.now);
        self.deliver();
    }

    fn deliver(&mut self) {
        for a in self.server.take_actions() {
            match a {
                Action::Send {
                    to,
                    channel,
                    message,
                } => {
                    if let Some(c) = self.clients.get_mut(&to)
                        && !c.closed
                    {
                        c.receive(channel, &message);
                    }
                }
                Action::Close(to) => {
                    if let Some(c) = self.clients.get_mut(&to) {
                        c.closed = true;
                    }
                }
            }
        }
    }

    fn join(&mut self, name: &str) -> ConnId {
        let c = self.connect();
        self.send(c, &hello(name));
        c
    }

    fn advance(&mut self, us: u64) {
        self.now += us;
        self.server.update(self.now);
        self.deliver();
    }

    fn tick(&self) -> Tick {
        self.server.clock(self.now).0
    }
}

fn state(tick: Tick, x: f32) -> PlayerState {
    PlayerState {
        tick,
        position: [x, -5.0, 0.0],
        velocity: [0; 3],
        yaw: 0,
        pitch: 0,
        rotation: None,
        walking: false,
        swimming: true,
        grounded: false,
        walk_grounded: true,
        in_pod: false,
        cinematic: false,
        controls: true,
        life: Life::Alive,
        anim: AnimState::default(),
    }
}

#[test]
fn hellos_that_do_not_fit_are_rejected_with_their_reason() {
    let mut cfg = config();
    cfg.password = Some("pw".into());
    cfg.max_players = 1;
    let mut w = World::with(cfg);
    let ok = |name: &str| ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        build: "10".into(),
        fingerprint: 0xfeed,
        name: name.into(),
        password: Some("pw".into()),
    };
    let first = w.connect();
    w.send(first, &ok("Ryley"));
    assert_eq!(w.clients[&first].player, Some(PlayerId(0)));
    let mut cases: Vec<(ClientMessage, RejectReason)> = Vec::new();
    let with = |f: &dyn Fn(&mut ClientMessage)| {
        let mut m = ok("Other");
        f(&mut m);
        m
    };
    cases.push((
        with(&|m| {
            if let ClientMessage::Hello { protocol, .. } = m {
                *protocol = 1;
            }
        }),
        RejectReason::Protocol {
            server: PROTOCOL_VERSION,
        },
    ));
    cases.push((
        with(&|m| {
            if let ClientMessage::Hello { build, .. } = m {
                *build = "9".into();
            }
        }),
        RejectReason::Build {
            server: "10".into(),
        },
    ));
    cases.push((
        with(&|m| {
            if let ClientMessage::Hello { fingerprint, .. } = m {
                *fingerprint = 1;
            }
        }),
        RejectReason::Fingerprint,
    ));
    cases.push((
        with(&|m| {
            if let ClientMessage::Hello { password, .. } = m {
                *password = None;
            }
        }),
        RejectReason::Password,
    ));
    cases.push((ok("Other"), RejectReason::Full { max: 1 }));
    for (m, want) in cases {
        let c = w.connect();
        w.send(c, &m);
        let got = w.clients[&c].rejected.clone();
        println!(
            "rejected: {}",
            got.as_ref().map_or("-".into(), ToString::to_string)
        );
        assert_eq!(got, Some(want));
        assert_eq!(w.clients[&c].player, None);
    }
    // Names (with room on the server).
    let mut w = World::new();
    w.join("Ryley");
    for bad in ["", " Ryley", "ryley", "a\tb"] {
        let c = w.join(bad);
        assert_eq!(w.clients[&c].rejected, Some(RejectReason::Name), "{bad:?}");
    }
    // A rejected connection is closed after the linger, not before.
    let c = w.join("");
    w.advance(REJECT_LINGER - 1);
    assert!(!w.clients[&c].closed);
    w.advance(1);
    assert!(w.clients[&c].closed);
    assert_eq!(w.server.stats().rejects, 5);
}

const REJECT_LINGER: u64 = sn_session::server::REJECT_LINGER_US;

#[test]
fn ids_stay_with_names_through_leaves_timeouts_and_rejoins() {
    let mut w = World::new();
    let conns: Vec<ConnId> = (0..8).map(|i| w.join(&format!("p{i}"))).collect();
    let ids: Vec<u16> = conns
        .iter()
        .map(|c| w.clients[c].player.unwrap().0)
        .collect();
    assert_eq!(ids, (0..8).collect::<Vec<_>>());
    let ninth = w.join("p8");
    assert_eq!(
        w.clients[&ninth].rejected,
        Some(RejectReason::Full { max: 8 })
    );
    // p3 says goodbye, p5 times out.
    w.send(conns[3], &ClientMessage::Bye);
    w.server.disconnected(conns[5], true, w.now);
    w.deliver();
    let last: Vec<&WorldEventKind> = w.clients[&conns[0]]
        .log
        .iter()
        .rev()
        .take(2)
        .map(|e| &e.kind)
        .collect();
    assert_eq!(
        last,
        [
            &WorldEventKind::PlayerLeft {
                player: PlayerId(5),
                reason: LeaveReason::TimedOut
            },
            &WorldEventKind::PlayerLeft {
                player: PlayerId(3),
                reason: LeaveReason::Left
            },
        ]
    );
    // p5 comes back: same id; a new name gets the next new id.
    let p5 = w.join("p5");
    assert_eq!(w.clients[&p5].player, Some(PlayerId(5)));
    let new = w.join("p9");
    assert_eq!(w.clients[&new].player, Some(PlayerId(8)));
    // The newcomer's Welcome lists the 7 others; its own join comes next.
    let world = w.clients[&new].world.as_ref().unwrap();
    assert_eq!(world.players.len(), 8);
    assert_eq!(w.server.world().players.len(), 8);
    assert_eq!(world.hash(), w.server.world().hash());
    // A connection that never says Hello is closed after 5 s.
    let silent = w.connect();
    w.advance(sn_session::server::HELLO_TIMEOUT_US);
    assert!(w.clients[&silent].closed);
}

#[test]
fn states_go_from_ready_players_to_the_others_and_into_welcomes() {
    let mut w = World::new();
    let a = w.join("a");
    let b = w.join("b");
    let pa = w.clients[&a].player.unwrap();
    // Not ready yet: dropped.
    w.send(a, &ClientMessage::States(vec![state(w.tick(), 0.0)]));
    assert!(w.clients[&b].states.is_empty());
    w.send(a, &ClientMessage::Ready);
    w.advance(TICK_US);
    let t = w.tick();
    w.send(
        a,
        &ClientMessage::States(vec![state(Tick(t.0 - 1), 0.0), state(t, 0.1)]),
    );
    assert_eq!(w.clients[&b].states[&pa], [Tick(t.0 - 1), t]);
    assert!(w.clients[&a].states.is_empty(), "not echoed");
    // Too far ahead: dropped.
    let ahead = Tick(t.0 + 1 + (sn_session::server::MAX_STATE_LEAD_US / TICK_US) as u32);
    w.send(a, &ClientMessage::States(vec![state(ahead, 0.2)]));
    assert_eq!(w.clients[&b].states[&pa].len(), 2);
    // 1 m in one tick is 50 m/s: implausible (logged, still relayed).
    w.advance(TICK_US);
    w.send(a, &ClientMessage::States(vec![state(w.tick(), 1.1)]));
    assert_eq!(w.server.stats().implausible_moves, 1);
    assert_eq!(w.clients[&b].states[&pa].len(), 3);
    // A respawn's jump (not alive on either side) is the game's own move.
    w.advance(TICK_US);
    let mut dead = state(w.tick(), 1.1);
    dead.life = Life::Respawning;
    w.send(a, &ClientMessage::States(vec![dead]));
    w.advance(TICK_US);
    let mut back = state(w.tick(), 500.0);
    back.life = Life::Respawning;
    w.send(a, &ClientMessage::States(vec![back]));
    assert_eq!(w.server.stats().implausible_moves, 1);
    // A late joiner gets the newest state in its Welcome.
    let c = w.connect();
    w.server.received(c, &hello("c").encode(), w.now);
    let welcome = w.server.take_actions().into_iter().find_map(|a| match a {
        Action::Send {
            message: ServerMessage::Welcome { players, .. },
            ..
        } => Some(players),
        _ => None,
    });
    let players = welcome.unwrap();
    let pa_info = players.iter().find(|p| p.id == pa).unwrap();
    assert!(pa_info.ready);
    assert_eq!(pa_info.state.unwrap().tick, w.tick());
}

#[test]
fn pings_get_the_servers_clock() {
    let mut w = World::new();
    let a = w.join("a");
    w.advance(3 * TICK_US + TICK_US / 4);
    w.send(a, &ClientMessage::Ping { client_time: 77 });
    assert_eq!(w.clients[&a].pongs, [(Tick(3), 16_384)]);
}

#[test]
fn garbage_and_out_of_turn_messages_are_ignored() {
    let mut w = World::new();
    let c = w.connect();
    w.server.received(c, &[0xff, 1, 2], w.now);
    w.send(c, &ClientMessage::Ready);
    w.send(c, &hello("a"));
    w.send(c, &hello("a"));
    w.send(
        c,
        &ClientMessage::Event(PlayerEvent {
            tick: Tick(0),
            kind: PlayerEventKind::Respawned,
        }),
    );
    w.server
        .received(ConnId(999), &ClientMessage::Ready.encode(), w.now);
    let ignored: Vec<String> = w
        .server
        .take_notices()
        .into_iter()
        .filter_map(|n| match n {
            Notice::Ignored { why, .. } => Some(why),
            _ => None,
        })
        .collect();
    println!("ignored: {ignored:?}");
    assert_eq!(ignored.len(), 5);
    assert_eq!(w.server.world().events, 1);
}

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
}

/// 10,000 ticks of random joins (12 names for 8 places), leaves,
/// timeouts, readies and player events. Every client must see the
/// server's log from its join on, in the same order with the same ticks,
/// and hold the server's hash after every event.
#[test]
fn every_client_sees_the_same_events_and_hash_over_10000_ticks() {
    let mut w = World::new();
    let mut rng = Rng(42);
    // Server log by seq, to compare against.
    let mut server_log: Vec<WorldEvent> = Vec::new();
    let mut by_name: BTreeMap<String, ConnId> = BTreeMap::new();
    let mut checked_events = 0u64;
    let mut hash_reports = 0u64;
    let mut max_present = 0;
    for _ in 0..10_000 {
        w.advance(TICK_US);
        let r = rng.below(1000);
        let name = format!("p{}", rng.below(12));
        let present = by_name.get(&name).copied();
        match (r, present) {
            (0..=19, None) => {
                let c = w.join(&name);
                if w.clients[&c].player.is_some() {
                    by_name.insert(name, c);
                }
            }
            (20..=27, Some(c)) => {
                w.send(c, &ClientMessage::Bye);
                by_name.remove(&name);
            }
            (28..=33, Some(c)) => {
                w.server.disconnected(c, true, w.now);
                w.deliver();
                by_name.remove(&name);
            }
            (34..=80, Some(c)) => w.send(c, &ClientMessage::Ready),
            (81..=200, Some(c)) => {
                let kind = match rng.below(5) {
                    0 => PlayerEventKind::HatchUsed {
                        hatch: rng.below(6) as u16,
                    },
                    1 => PlayerEventKind::CinematicEnded,
                    2 => PlayerEventKind::Died {
                        cause: DeathCause::Normal,
                    },
                    3 => PlayerEventKind::Respawned,
                    _ => PlayerEventKind::AnimatorTrigger { trigger: 0 },
                };
                let tick = Tick(w.tick().0.saturating_sub(rng.below(10) as u32));
                w.send(c, &ClientMessage::Event(PlayerEvent { tick, kind }));
            }
            (201..=300, Some(c)) => {
                // A client reports its hash.
                let world = w.clients[&c].world.as_ref().unwrap();
                let m = ClientMessage::WorldHash {
                    events: world.events,
                    hash: world.hash(),
                };
                w.send(c, &m);
                hash_reports += 1;
            }
            _ => {}
        }
        max_present = max_present.max(by_name.len());
        // The server's log grows only by appending.
        let sw = w.server.world();
        assert!(sw.events as usize >= server_log.len());
        // Collect the server's events from any present client that has
        // been there since the start of the missing range; simplest: the
        // first client with the newest seqs.
        for c in w.clients.values() {
            for e in &c.log {
                let i = e.seq as usize;
                if i == server_log.len() {
                    server_log.push(e.clone());
                } else if i < server_log.len() {
                    assert_eq!(&server_log[i], e, "client log differs at seq {i}");
                }
            }
        }
        // Every present client: same hash as the server after the same
        // number of events.
        for c in by_name.values() {
            let world = w.clients[c].world.as_ref().unwrap();
            assert_eq!(world.events, sw.events);
            assert_eq!(world.hash(), sw.hash());
            checked_events += 1;
        }
    }
    let stats = w.server.stats();
    println!(
        "10,000 ticks: {} events, {} joins, {} rejects, {} leaves, at most {max_present} players at once, {checked_events} client-tick hash comparisons equal; hash reports {hash_reports}: {} checked, {} mismatches, {} unchecked; {} ignored",
        stats.events,
        stats.joins,
        stats.rejects,
        stats.leaves,
        stats.hash_checks,
        stats.hash_mismatches,
        stats.hash_unchecked,
        stats.ignored_messages
    );
    assert_eq!(server_log.len() as u32, w.server.world().events);
    assert!(stats.joins > 50 && stats.leaves > 40 && max_present >= 6);
    assert_eq!(stats.hash_checks, hash_reports);
    assert_eq!(stats.hash_mismatches, 0);

    // A wrong report is caught.
    let c = *by_name.values().next().unwrap();
    let events = w.server.world().events;
    w.send(c, &ClientMessage::WorldHash { events, hash: 1 });
    assert_eq!(w.server.stats().hash_mismatches, 1);
}

#[test]
fn slot_fill_hashes_are_compared_per_batch() {
    let mut w = World::new();
    let a = w.join("a");
    let b = w.join("b");
    for (c, batch, hash) in [
        (a, [1, 2, 3], 10),
        (b, [1, 2, 3], 10),
        (b, [4, 0, -1], 7),
        (a, [4, 0, -1], 8),
    ] {
        w.send(c, &ClientMessage::SlotFillHash { batch, hash });
    }
    let s = w.server.stats();
    assert_eq!((s.slot_fill_checks, s.slot_fill_mismatches), (4, 1));
}

impl World {
    /// Joined and ready.
    fn ready(&mut self, name: &str) -> ConnId {
        let c = self.join(name);
        self.send(c, &ClientMessage::Ready);
        c
    }

    fn ask(&mut self, c: ConnId, req: u32, request: Request) {
        self.send(c, &ClientMessage::Request { req, request });
    }

    /// The player's newest state says whether it is in the lifepod.
    fn in_pod(&mut self, c: ConnId, on: bool) {
        // The server keeps only a newer tick's state.
        self.advance(TICK_US);
        let mut s = state(self.tick(), 0.0);
        s.in_pod = on;
        self.send(c, &ClientMessage::States(vec![s]));
    }

    /// Every client still connected holds the server's state.
    fn all_equal(&self) {
        let sw = self.server.world();
        for c in self.clients.values().filter(|c| !c.closed && c.loaded) {
            let w = c.world.as_ref().unwrap();
            assert_eq!(w, sw);
            assert_eq!(w.hash(), sw.hash());
        }
    }

    fn player(&self, c: ConnId) -> PlayerId {
        self.clients[&c].player.unwrap()
    }
}

fn placed(n: u32) -> EntityKey {
    EntityKey::Placed(format!("obj-{n}"))
}

fn pick(key: EntityKey, class: &str) -> Request {
    Request::PickUp {
        key,
        class: class.into(),
    }
}

const AT: DropAt = DropAt {
    point: [1.0, 2.0, 3.0],
    rotation: [0, 0, 0, 32_767],
};

#[test]
fn two_players_reach_for_one_item_and_one_gets_it() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    let b = w.ready("b");
    // The same tick: the server takes them in its order.
    w.ask(a, 0, pick(placed(1), "quartz"));
    w.ask(b, 0, pick(placed(1), "quartz"));
    assert!(w.clients[&a].refused.is_empty());
    assert_eq!(w.clients[&b].refused, [(0, RefuseReason::Gone)]);
    let pa = w.player(a);
    assert!(w.server.world().items.owns(pa, ItemId(3)));
    // Before Ready: refused.
    let c = w.join("c");
    w.ask(c, 5, pick(placed(2), "quartz"));
    assert_eq!(w.clients[&c].refused, [(5, RefuseReason::NotReady)]);
    w.all_equal();
}

#[test]
fn a_full_inventory_refuses() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    for n in 0..48 {
        w.ask(a, n, pick(placed(n), "quartz"));
    }
    w.ask(a, 48, pick(placed(48), "quartz"));
    assert_eq!(w.clients[&a].refused, [(48, RefuseReason::NoRoom)]);
    w.all_equal();
}

#[test]
fn a_dropped_item_can_be_picked_up_by_another_at_once() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    let b = w.ready("b");
    w.ask(a, 0, pick(placed(1), "quartz"));
    w.ask(
        a,
        1,
        Request::Drop {
            item: ItemId(3),
            at: AT,
        },
    );
    // The same tick, no wait (the game's rule, M9d2); the class the
    // client gives for a spawned object does not matter.
    w.ask(b, 0, pick(EntityKey::Spawned(ItemId(3)), ""));
    assert!(w.clients[&a].refused.is_empty() && w.clients[&b].refused.is_empty());
    assert!(w.server.world().items.owns(w.player(b), ItemId(3)));
    // Gone now.
    w.ask(a, 2, pick(EntityKey::Spawned(ItemId(3)), ""));
    assert_eq!(w.clients[&a].refused, [(2, RefuseReason::Gone)]);
    w.all_equal();
}

#[test]
fn the_pod_decides_drops_and_the_locker() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    w.ask(a, 0, pick(placed(1), "quartz"));
    // The locker (supplies 0–2) only from inside the pod.
    let take = Request::Move {
        item: ItemId(0),
        to: Store::Grid,
        target: MoveTarget::Store,
    };
    w.ask(a, 1, take.clone());
    assert_eq!(w.clients[&a].refused, [(1, RefuseReason::NotHere)]);
    w.in_pod(a, true);
    w.ask(a, 2, take);
    // No drop inside it.
    w.ask(
        a,
        3,
        Request::Drop {
            item: ItemId(3),
            at: AT,
        },
    );
    assert_eq!(
        w.clients[&a].refused,
        [(1, RefuseReason::NotHere), (3, RefuseReason::NotHere)]
    );
    // Dying in it drops nothing; outside, all but one.
    w.ask(
        a,
        4,
        Request::Lose {
            origin: [0.0; 3],
            drops: vec![],
        },
    );
    let pa = w.player(a);
    assert_eq!(
        w.server.world().items.players[&pa]
            .inventory
            .container
            .len(),
        2
    );
    w.in_pod(a, false);
    w.ask(
        a,
        5,
        Request::Lose {
            origin: [0.0; 3],
            drops: vec![],
        },
    );
    assert_eq!(
        w.server.world().items.players[&pa]
            .inventory
            .container
            .len(),
        1
    );
    assert_eq!(w.server.world().items.objects.len(), 1);
    w.all_equal();
}

#[test]
fn a_late_joiner_gets_the_servers_state() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    for n in 0..30 {
        w.ask(
            a,
            n,
            pick(placed(n), if n % 4 == 0 { "big" } else { "quartz" }),
        );
    }
    for n in 0..3 {
        w.ask(
            a,
            100 + n,
            Request::Hit {
                key: placed(500),
                class: "lime".into(),
                kind: HitKind::Hand,
                spawn: [0.0; 3],
            },
        );
    }
    w.ask(
        a,
        200,
        Request::Drop {
            item: ItemId(5),
            at: AT,
        },
    );
    let late = w.ready("late");
    assert!(w.clients[&late].loaded);
    w.all_equal();
    let stats = w.server.stats();
    println!(
        "late joiner: {} chunks, {} B; world {} gone, {} objects",
        stats.chunks,
        stats.chunk_bytes,
        w.server.world().items.gone.len(),
        w.server.world().items.objects.len()
    );
}

/// A request a client might send, from its own copy of the world.
fn random_request(rng: &mut Rng, w: &WorldState, me: PlayerId) -> Request {
    let items = &w.items;
    let mine: Vec<ItemId> = items.classes.keys().copied().collect();
    let any = |rng: &mut Rng| {
        if mine.is_empty() {
            ItemId(0)
        } else {
            mine[rng.below(mine.len() as u64) as usize]
        }
    };
    match rng.below(10) {
        0..=2 => {
            let class = ["quartz", "big", "tank", "food", "prop"][rng.below(5) as usize];
            pick(placed(rng.below(400) as u32), class)
        }
        3 => match items
            .objects
            .keys()
            .nth(rng.below(items.objects.len().max(1) as u64) as usize)
        {
            Some(id) => pick(EntityKey::Spawned(*id), ""),
            None => pick(placed(0), "quartz"),
        },
        4..=5 => Request::Hit {
            key: placed(1000 + rng.below(30) as u32),
            class: "lime".into(),
            kind: [HitKind::Hand, HitKind::Hand, HitKind::Bash][rng.below(3) as usize],
            spawn: [rng.below(100) as f32, 0.0, 0.0],
        },
        6 => Request::Drop {
            item: any(rng),
            at: AT,
        },
        7..=8 => Request::Move {
            item: any(rng),
            to: [Store::Grid, Store::Slots, Store::PodLocker][rng.below(3) as usize],
            target: match rng.below(3) {
                0 => MoveTarget::Store,
                1 => MoveTarget::Item(any(rng)),
                _ => MoveTarget::Slot(
                    ["Head", "Tank", "Chip1", "Nope"][rng.below(4) as usize].into(),
                ),
            },
        },
        _ => {
            let n = items
                .lose_list(me, sn_sim::inventory::DropPlace::default())
                .len();
            Request::Lose {
                origin: [5.0, -3.0, 2.0],
                drops: vec![
                    DeathDrop {
                        offset: [100, -200, 300],
                        rotation: [0, 0, 0, 32_767],
                    };
                    n
                ],
            }
        }
    }
}

#[test]
fn four_players_and_10000_ticks_of_items_keep_every_state_equal() {
    let mut w = World::with(item_config());
    let mut rng = Rng(7);
    let mut conns: Vec<ConnId> = (0..4).map(|i| w.ready(&format!("p{i}"))).collect();
    let mut req = 0u32;
    let mut hash_reports = 0u64;
    let mut rejoins = 0;
    for tick in 0..10_000u32 {
        w.advance(TICK_US);
        let k = rng.below(4) as usize;
        let c = conns[k];
        match rng.below(100) {
            0..=59 => {
                let me = w.player(c);
                let q = random_request(&mut rng, w.clients[&c].world.as_ref().unwrap(), me);
                w.ask(c, req, q);
                req += 1;
            }
            60..=64 => w.in_pod(c, rng.below(2) == 0),
            65 => {
                // Leaves and comes back: the same player, its items kept.
                w.send(c, &ClientMessage::Bye);
                conns[k] = w.ready(&format!("p{k}"));
                rejoins += 1;
            }
            66..=75 => {
                let world = w.clients[&c].world.as_ref().unwrap();
                let m = ClientMessage::WorldHash {
                    events: world.events,
                    hash: world.hash(),
                };
                w.send(c, &m);
                hash_reports += 1;
            }
            _ => {}
        }
        if tick % 500 == 0 {
            w.all_equal();
        }
    }
    w.all_equal();
    let stats = w.server.stats();
    let items = &w.server.world().items;
    let refused: usize = w.clients.values().map(|c| c.refused.len()).sum();
    println!(
        "10,000 ticks, 4 players: {} requests, {} refused ({refused} seen by clients), {} events, {rejoins} rejoins; {} gone, {} objects, {} items held; hash reports {hash_reports}: {} checked, {} mismatches; {} chunks ({} B)",
        stats.requests,
        stats.refused,
        stats.events,
        items.gone.len(),
        items.objects.len(),
        items.classes.len(),
        stats.hash_checks,
        stats.hash_mismatches,
        stats.chunks,
        stats.chunk_bytes
    );
    assert_eq!(stats.hash_mismatches, 0);
    assert!(stats.hash_checks > 500);
    assert!(stats.requests > 5000 && stats.refused > 500 && stats.events > 1000);
    assert_eq!(refused as u64, stats.refused);
}

#[test]
fn a_million_bad_requests_never_panic() {
    let mut w = World::with(item_config());
    let a = w.ready("a");
    let mut rng = Rng(9);
    let mut valid: Vec<Vec<u8>> = Vec::new();
    for _ in 0..200 {
        let me = w.player(a);
        let q = random_request(&mut rng, w.server.world(), me);
        valid.push(ClientMessage::Request { req: 1, request: q }.encode());
    }
    for i in 0..1_000_000u32 {
        let mut b = valid[rng.below(valid.len() as u64) as usize].clone();
        match i % 3 {
            0 => {
                let at = rng.below(b.len() as u64) as usize;
                b[at] ^= 1 << rng.below(8);
            }
            1 => b.truncate(rng.below(b.len() as u64 + 1) as usize),
            _ => {
                let at = rng.below(b.len() as u64) as usize;
                b[at] = rng.below(256) as u8;
            }
        }
        w.server.received(a, &b, w.now);
        if i % 1000 == 0 {
            w.server.take_actions();
            w.server.take_notices();
        }
    }
    let stats = w.server.stats();
    println!(
        "1,000,000 bad requests: {} decoded as requests, {} refused, {} events, {} ignored",
        stats.requests, stats.refused, stats.events, stats.ignored_messages
    );
    assert!(stats.requests > 10_000);
}

/// M9u6c2's debug aid: test supplies go into a new world's locker before
/// anyone joins and reach a joiner in the world it is sent; refused
/// afterwards and for an unknown tech.
#[test]
fn test_supplies_go_into_the_locker_and_reach_joiners() {
    let mut w = World::with(item_config());
    let locker = |w: &World| w.server.world().items.locker.len();
    let before = locker(&w);
    assert_eq!(
        w.server.add_test_supplies(&[common::TANK, 12345]),
        Ok(vec![true, false])
    );
    assert_eq!(locker(&w), before + 1);
    let a = w.ready("a");
    w.all_equal();
    let world = w.clients[&a].world.as_ref().unwrap();
    assert_eq!(world.items.locker.len(), before + 1);
    assert!(w.server.add_test_supplies(&[common::TANK]).is_err());
}
