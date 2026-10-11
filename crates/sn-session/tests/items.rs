//! M9d3b's checks: the item events' effects on the shared state, refusals
//! that change nothing, the snapshot's round trip and hash, and two
//! states fed the same random log staying equal. Synthetic rules only.

use sn_protocol::codec::Reader;
use sn_protocol::{
    AuroraStart, DeathDrop, DropAt, EntityKey, HitKind, ItemEvent, ItemId, MoveTarget, PlayerId,
    RefuseReason, Store, Tick, WorldEvent, WorldEventKind, WorldSettings,
};
use sn_session::items::ItemsState;
use sn_session::{ApplyError, WorldState};
use sn_sim::inventory::DropPlace;

mod common;

use common::rules;

const A: PlayerId = PlayerId(0);
const B: PlayerId = PlayerId(1);

fn state() -> ItemsState {
    let mut s = ItemsState::new(rules(), true);
    s.join(A);
    s.join(B);
    s
}

fn placed(n: u32) -> EntityKey {
    EntityKey::Placed(format!("obj-{n}"))
}

fn pick(key: EntityKey, class: &str) -> ItemEvent {
    ItemEvent::PickedUp {
        key,
        class: class.into(),
    }
}

/// Applies, and on a refusal checks nothing changed.
fn apply(s: &mut ItemsState, p: PlayerId, e: &ItemEvent) -> Result<(), RefuseReason> {
    let before = s.clone();
    let r = s.apply(p, e);
    if r.is_err() {
        assert_eq!(*s, before, "a refused {e:?} changed the state");
    }
    r
}

#[test]
fn a_new_world_fills_the_pod_locker() {
    let s = state();
    assert_eq!(s.locker.len(), 3);
    assert_eq!(s.next_item, 3);
    assert_eq!(s.classes.len(), 3);
    assert_eq!(ItemsState::new(rules(), false).locker.len(), 0);
}

#[test]
fn picking_up_removes_the_object_once() {
    let mut s = state();
    apply(&mut s, A, &pick(placed(1), "quartz")).unwrap();
    assert!(s.gone.contains(&placed(1)));
    assert!(s.owns(A, ItemId(3)));
    let known: Vec<i32> = s.players[&A].inventory.known.known().collect();
    assert_eq!(known, [100, 500]);
    // Someone was first.
    assert_eq!(
        apply(&mut s, B, &pick(placed(1), "quartz")),
        Err(RefuseReason::Gone)
    );
    // Not an item the rules know, or not pickupable.
    assert_eq!(
        apply(&mut s, B, &pick(placed(2), "rock")),
        Err(RefuseReason::Unknown)
    );
    assert_eq!(
        apply(&mut s, B, &pick(placed(2), "prop")),
        Err(RefuseReason::Unknown)
    );
    // A tank goes to its slot.
    apply(&mut s, B, &pick(placed(3), "tank")).unwrap();
    assert_eq!(
        s.players[&B]
            .inventory
            .equipment
            .item_in("Tank")
            .map(|i| i.id.0),
        Some(4)
    );
    // A full grid refuses.
    for n in 0..48 {
        apply(&mut s, B, &pick(placed(100 + n), "quartz")).unwrap();
    }
    assert_eq!(
        apply(&mut s, B, &pick(placed(999), "quartz")),
        Err(RefuseReason::NoRoom)
    );
}

#[test]
fn hits_break_the_outcrop_with_the_servers_draws() {
    let mut s = state();
    let key = placed(7);
    let hit = |draws: Vec<f32>| ItemEvent::Hit {
        key: placed(7),
        class: "lime".into(),
        kind: HitKind::Hand,
        spawn: [1.0, 2.0, 3.0],
        draws,
    };
    let mut never = || -> f32 { panic!("drew before the break") };
    for left in [2, 1] {
        let d = s
            .draws_for_hit(A, &key, "lime", HitKind::Hand, &mut never)
            .unwrap();
        assert!(d.is_empty());
        apply(&mut s, A, &hit(d)).unwrap();
        assert_eq!(s.outcrops[&key], left);
    }
    // Draws on a hit that does not break, or the wrong number of them.
    let mut t = state();
    assert_eq!(
        apply(&mut t, A, &hit(vec![0.5])),
        Err(RefuseReason::NotAllowed)
    );
    assert_eq!(
        apply(&mut s, A, &hit(vec![])),
        Err(RefuseReason::NotAllowed)
    );
    // The third hit breaks it: quartz at 0.5 needs a draw below 0.5.
    let mut low = || 0.1;
    let d = s
        .draws_for_hit(A, &key, "lime", HitKind::Hand, &mut low)
        .unwrap();
    assert_eq!(d, [0.1]);
    apply(&mut s, A, &hit(d)).unwrap();
    assert!(s.gone.contains(&key) && !s.outcrops.contains_key(&key));
    let spawned = &s.objects[&ItemId(3)];
    assert_eq!(
        (spawned.class.as_str(), spawned.point),
        ("quartz", [1.0, 2.0, 3.0])
    );
    assert_eq!(s.players[&A].luck.randomizers[0].1.entropy, -0.5);
    assert_eq!(apply(&mut s, A, &hit(vec![])), Err(RefuseReason::Gone));
    // The resource is picked up under its own number.
    apply(&mut s, B, &pick(EntityKey::Spawned(ItemId(3)), "quartz")).unwrap();
    assert!(s.owns(B, ItemId(3)) && s.objects.is_empty());
    assert_eq!(
        apply(&mut s, A, &pick(EntityKey::Spawned(ItemId(3)), "quartz")),
        Err(RefuseReason::Gone)
    );
}

#[test]
fn drops_and_moves() {
    let mut s = state();
    apply(&mut s, A, &pick(placed(1), "quartz")).unwrap();
    let at = DropAt {
        point: [5.0, 6.0, 7.0],
        rotation: [0, 0, 0, 32_767],
    };
    assert_eq!(
        apply(
            &mut s,
            B,
            &ItemEvent::Dropped {
                item: ItemId(3),
                at
            }
        ),
        Err(RefuseReason::NoItem)
    );
    apply(
        &mut s,
        A,
        &ItemEvent::Dropped {
            item: ItemId(3),
            at,
        },
    )
    .unwrap();
    assert_eq!(s.objects[&ItemId(3)].point, [5.0, 6.0, 7.0]);
    // Picked up again at once by another player, wrong class refused.
    assert_eq!(
        apply(&mut s, B, &pick(EntityKey::Spawned(ItemId(3)), "big")),
        Err(RefuseReason::Unknown)
    );
    apply(&mut s, B, &pick(EntityKey::Spawned(ItemId(3)), "quartz")).unwrap();
    // The pod's tank (item 2) to B's grid, then onto its slot.
    let mv = |item, to, target| ItemEvent::Moved {
        item: ItemId(item),
        to,
        target,
    };
    apply(&mut s, B, &mv(2, Store::Grid, MoveTarget::Store)).unwrap();
    assert!(s.owns(B, ItemId(2)) && !s.in_locker(ItemId(2)));
    apply(&mut s, B, &mv(2, Store::Slots, MoveTarget::Store)).unwrap();
    assert_eq!(
        s.players[&B]
            .inventory
            .equipment
            .slot_of(sn_sim::inventory::ItemId(2)),
        Some("Tank")
    );
    // Quartz into the tank slot: the rules refuse.
    assert_eq!(
        apply(
            &mut s,
            B,
            &mv(3, Store::Slots, MoveTarget::Slot("Tank".into()))
        ),
        Err(RefuseReason::NotAllowed)
    );
    // An item that is not there.
    assert_eq!(
        apply(&mut s, B, &mv(77, Store::Grid, MoveTarget::Store)),
        Err(RefuseReason::NoItem)
    );
    // Into the locker.
    apply(&mut s, B, &mv(3, Store::PodLocker, MoveTarget::Store)).unwrap();
    assert!(s.in_locker(ItemId(3)));
}

#[test]
fn death_drops_all_but_one() {
    let mut s = state();
    for n in 0..4 {
        apply(&mut s, A, &pick(placed(n), "quartz")).unwrap();
    }
    assert!(
        s.lose_list(
            A,
            DropPlace {
                in_escape_pod: true,
                ..Default::default()
            }
        )
        .is_empty()
    );
    let list = s.lose_list(A, DropPlace::default());
    assert_eq!(list.len(), 3);
    let drops = list
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            (
                id,
                DeathDrop {
                    offset: [i as i16 * 100, 0, -1500],
                    rotation: [0, 0, 0, 32_767],
                },
            )
        })
        .collect();
    apply(
        &mut s,
        A,
        &ItemEvent::Lost {
            origin: [10.0, 0.0, 0.0],
            drops,
        },
    )
    .unwrap();
    assert_eq!(s.players[&A].inventory.container.len(), 1);
    assert_eq!(s.objects[&list[1]].point, [10.1, 0.0, -1.5]);
}

#[test]
fn the_snapshot_round_trips() {
    let mut s = state();
    apply(&mut s, A, &pick(placed(1), "quartz")).unwrap();
    apply(&mut s, A, &pick(placed(2), "big")).unwrap();
    apply(&mut s, A, &pick(placed(3), "tank")).unwrap();
    // A freed dictionary entry in A's grid.
    apply(
        &mut s,
        A,
        &ItemEvent::Dropped {
            item: ItemId(3),
            at: DropAt {
                point: [0.0; 3],
                rotation: [0, 0, 0, 32_767],
            },
        },
    )
    .unwrap();
    let mut never = || 0.0;
    let d = s
        .draws_for_hit(A, &placed(9), "lime", HitKind::Hand, &mut never)
        .unwrap();
    apply(
        &mut s,
        A,
        &ItemEvent::Hit {
            key: placed(9),
            class: "lime".into(),
            kind: HitKind::Hand,
            spawn: [0.0; 3],
            draws: d,
        },
    )
    .unwrap();
    let bytes = s.to_bytes();
    let mut r = Reader::new(&bytes);
    let back = ItemsState::decode(rules(), &mut r).unwrap();
    r.finish().unwrap();
    assert_eq!(back, s);
    assert_eq!(back.to_bytes(), bytes);
    println!("snapshot: {} B", bytes.len());
}

#[test]
fn bad_snapshots_are_errors_not_panics() {
    let mut s = state();
    for n in 0..10 {
        apply(
            &mut s,
            A,
            &pick(placed(n), if n % 3 == 0 { "big" } else { "quartz" }),
        )
        .unwrap();
    }
    let bytes = s.to_bytes();
    let mut x: u64 = 1;
    let mut errors = 0;
    for i in 0..200_000u32 {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mut b = bytes.clone();
        if i % 2 == 0 {
            b.truncate((x >> 33) as usize % (bytes.len() + 1));
        } else {
            let at = (x >> 33) as usize % b.len();
            b[at] ^= 1 << ((x >> 20) % 8);
        }
        let mut r = Reader::new(&b);
        match ItemsState::decode(rules(), &mut r) {
            Ok(st) => {
                let _ = st.to_bytes();
            }
            Err(e) => {
                assert!(e.offset <= b.len());
                errors += 1;
            }
        }
    }
    assert!(errors > 50_000, "{errors}");
}

fn settings() -> WorldSettings {
    WorldSettings {
        slot_seed: Some(1),
        lifepod: Some([0.0, 0.0, 0.0]),
        aurora: AuroraStart::Countdown { seconds: 3000.0 },
        clock_start: 480.0,
        time_scale: 1.0,
    }
}

/// A seeded generator.
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

    fn value(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A random item event a client might ask for, with the server's part
/// (draws, the death list) filled in from `server`.
fn random_event(rng: &mut Rng, server: &WorldState, player: PlayerId) -> ItemEvent {
    let items = &server.items;
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
            pick(placed(rng.below(300) as u32), class)
        }
        3 => {
            let objs: Vec<ItemId> = items.objects.keys().copied().collect();
            if objs.is_empty() {
                return pick(placed(0), "quartz");
            }
            let id = objs[rng.below(objs.len() as u64) as usize];
            pick(EntityKey::Spawned(id), &items.objects[&id].class.clone())
        }
        4..=5 => {
            let key = placed(1000 + rng.below(20) as u32);
            let mut draw = || rng.value();
            let draws = items
                .draws_for_hit(player, &key, "lime", HitKind::Hand, &mut draw)
                .unwrap_or_default();
            ItemEvent::Hit {
                key,
                class: "lime".into(),
                kind: HitKind::Hand,
                spawn: [0.0, 1.0, 0.0],
                draws,
            }
        }
        6 => ItemEvent::Dropped {
            item: any(rng),
            at: DropAt {
                point: [rng.value(), 0.0, 0.0],
                rotation: [0, 0, 0, 32_767],
            },
        },
        7..=8 => {
            let to = [Store::Grid, Store::Slots, Store::PodLocker][rng.below(3) as usize];
            let target = match rng.below(3) {
                0 => MoveTarget::Store,
                1 => MoveTarget::Item(any(rng)),
                _ => MoveTarget::Slot(
                    ["Head", "Tank", "Chip1", "Nope"][rng.below(4) as usize].into(),
                ),
            };
            ItemEvent::Moved {
                item: any(rng),
                to,
                target,
            }
        }
        _ => ItemEvent::Lost {
            origin: [0.0; 3],
            drops: items
                .lose_list(player, DropPlace::default())
                .into_iter()
                .map(|id| (id, DeathDrop::default()))
                .collect(),
        },
    }
}

#[test]
fn two_states_fed_the_same_log_stay_equal() {
    let mut server = WorldState::new(settings(), rules());
    let mut seq = 0;
    let mut log = Vec::new();
    let mut emit = |w: &mut WorldState, kind: WorldEventKind, log: &mut Vec<WorldEvent>| {
        let e = WorldEvent {
            seq,
            tick: Tick(seq),
            kind,
        };
        match w.apply(&e) {
            Ok(()) => {
                seq += 1;
                log.push(e);
                true
            }
            Err(ApplyError::Item { .. }) => false,
            Err(e) => panic!("{e}"),
        }
    };
    let players = [PlayerId(0), PlayerId(1), PlayerId(2), PlayerId(3)];
    for p in players {
        assert!(emit(
            &mut server,
            WorldEventKind::PlayerJoined {
                player: p,
                name: format!("p{}", p.0),
            },
            &mut log
        ));
    }
    let mut rng = Rng(42);
    let (mut accepted, mut refused) = (0, 0);
    for _ in 0..5_000 {
        let player = players[rng.below(4) as usize];
        let event = random_event(&mut rng, &server, player);
        let kind = WorldEventKind::Item {
            player,
            req: 0,
            event,
        };
        if emit(&mut server, kind, &mut log) {
            accepted += 1;
        } else {
            refused += 1;
        }
    }
    // A client from the start, and one that joins halfway from a
    // snapshot; both through the wire's encoding.
    let mut client = WorldState::new(settings(), rules());
    let half = log.len() / 2;
    let mut late = None;
    for (i, e) in log.iter().enumerate() {
        let wire = sn_protocol::ServerMessage::decode(
            &sn_protocol::ServerMessage::Event(e.clone()).encode(),
        )
        .unwrap();
        let sn_protocol::ServerMessage::Event(e) = wire else {
            unreachable!()
        };
        client.apply(&e).unwrap();
        if i + 1 == half {
            let mut l = WorldState::from_welcome(settings(), client.events, &[], rules());
            l.players = client.players.clone();
            l.load_items(&client.items.to_bytes()).unwrap();
            assert_eq!(l.hash(), client.hash());
            late = Some(l);
        } else if let Some(l) = late.as_mut() {
            l.apply(&e).unwrap();
        }
    }
    let late = late.unwrap();
    assert_eq!(client, server);
    assert_eq!(late, server);
    assert_eq!(client.hash(), server.hash());
    println!(
        "{accepted} accepted, {refused} refused; {} objects gone, {} spawned, {} items held, snapshot {} B",
        server.items.gone.len(),
        server.items.objects.len(),
        server.items.classes.len(),
        server.items.to_bytes().len()
    );
    assert!(accepted > 1000 && refused > 100, "{accepted} {refused}");
}
