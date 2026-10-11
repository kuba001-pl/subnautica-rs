//! Items in the protocol (protocol 4, M9d3a): which world object a
//! request names, the requests themselves, the item events every machine
//! replays, and why the server refuses a request
//! (`docs/formats/protocol.md` § Items).

use crate::Result;
use crate::codec::{Reader, Writer};

/// Longest object id or prefab class id, in UTF-8 bytes (the game's are
/// 32- to 36-character GUIDs).
pub const MAX_ID: usize = 64;
/// Longest equipment slot name.
pub const MAX_SLOT: usize = 32;
/// Most luck draws in one hit (one per `prefabList` entry per chance).
pub const MAX_DRAWS: usize = 64;
/// Most items one death drops (a 6 × 8 grid and 7 slots, less the one
/// kept; a little room above).
pub const MAX_DROPS: usize = 56;

/// An item, numbered by the server. A world object the server made has
/// the same number (`EntityKey::Spawned`): a dropped item keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u32);

/// A world object.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKey {
    /// Saved in a cell: its `SavedObject.id`.
    Placed(String),
    /// Filled into a slot: the placeholder's id, the slot's index in it
    /// and the copy (what `sn_world::fill_slots` is seeded from).
    Slot {
        placeholder: String,
        slot: u16,
        copy: u16,
    },
    /// Made by the server (an outcrop's resource, a dropped item).
    Spawned(ItemId),
}

/// How an outcrop was hit (`sn_sim::inventory::HitKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Hand,
    Bash,
    Mech,
}

/// A player's item stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Grid,
    Slots,
    /// The lifepod's locker (one, shared).
    PodLocker,
}

/// Where in the store a moved item goes (`sn_sim::inventory::Target`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveTarget {
    Store,
    Item(ItemId),
    Slot(String),
}

/// Where a dropped item lands, as its owner computed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropAt {
    pub point: [f32; 3],
    /// x, y, z, w, each × 32,767 (`quant::QUATERNION_SCALE`).
    pub rotation: [i16; 4],
}

/// One item a death drops: millimetres from the drops' origin (the
/// rules put every drop within 1.5 m of the camera, so ±32 m is ample)
/// and the rotation as in [`DropAt`]. 14 bytes, so a full inventory's
/// drops fit one message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeathDrop {
    pub offset: [i16; 3],
    pub rotation: [i16; 4],
}

/// What a player asks the server for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Pick up a world object, which the client says is the prefab
    /// `class` (empty for `Spawned`: the server knows it).
    PickUp {
        key: EntityKey,
        class: String,
    },
    /// Hit an outcrop; `spawn` is where its resources appear if it
    /// breaks (`position + up × verticalSpawnOffset`).
    Hit {
        key: EntityKey,
        class: String,
        kind: HitKind,
        spawn: [f32; 3],
    },
    Drop {
        item: ItemId,
        at: DropAt,
    },
    Move {
        item: ItemId,
        to: Store,
        target: MoveTarget,
    },
    /// Died: where each item `LoseItems` drops lands, in its order.
    Lose {
        origin: [f32; 3],
        drops: Vec<DeathDrop>,
    },
}

/// An accepted request, as every machine replays it.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemEvent {
    /// The object left the world into the player's inventory; a world
    /// object gets the next item number, a `Spawned` one keeps its own.
    PickedUp {
        key: EntityKey,
        class: String,
    },
    /// A hit; `draws` are the `Random.value`s the server drew for the
    /// luck if it broke (empty if not).
    Hit {
        key: EntityKey,
        class: String,
        kind: HitKind,
        spawn: [f32; 3],
        draws: Vec<f32>,
    },
    Dropped {
        item: ItemId,
        at: DropAt,
    },
    Moved {
        item: ItemId,
        to: Store,
        target: MoveTarget,
    },
    /// The items a death dropped, and where.
    Lost {
        origin: [f32; 3],
        drops: Vec<(ItemId, DeathDrop)>,
    },
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseReason {
    /// The object is no longer in the world (someone was first).
    Gone,
    /// Not an object the rules know as this kind (not pickupable, not
    /// an outcrop) or a key that cannot be.
    Unknown,
    /// No room in the inventory or the target store.
    NoRoom,
    /// Not allowed where the player is (dropping in the lifepod, the
    /// locker away from it).
    NotHere,
    /// The player does not have this item.
    NoItem,
    /// The move breaks the rules (wrong slot type, same place).
    NotAllowed,
    /// Before `Ready`.
    NotReady,
}

const REFUSE: [RefuseReason; 7] = [
    RefuseReason::Gone,
    RefuseReason::Unknown,
    RefuseReason::NoRoom,
    RefuseReason::NotHere,
    RefuseReason::NoItem,
    RefuseReason::NotAllowed,
    RefuseReason::NotReady,
];

pub(crate) fn write_refuse(w: &mut Writer, r: RefuseReason) {
    w.u8(REFUSE.iter().position(|&x| x == r).unwrap_or(0) as u8);
}

pub(crate) fn read_refuse(r: &mut Reader) -> Result<RefuseReason> {
    let at = r.pos;
    let k = r.u8("refuse reason")?;
    REFUSE
        .get(usize::from(k))
        .copied()
        .ok_or_else(|| r.error(at, format!("unknown refuse reason {k}")))
}

pub fn write_item(w: &mut Writer, id: ItemId) {
    w.varint(u64::from(id.0));
}

pub fn read_item(r: &mut Reader) -> Result<ItemId> {
    Ok(ItemId(r.varint_u32("item")?))
}

pub fn write_key(w: &mut Writer, k: &EntityKey) {
    match k {
        EntityKey::Placed(id) => {
            w.u8(0);
            w.str(id);
        }
        EntityKey::Slot {
            placeholder,
            slot,
            copy,
        } => {
            w.u8(1);
            w.str(placeholder);
            w.varint(u64::from(*slot));
            w.varint(u64::from(*copy));
        }
        EntityKey::Spawned(id) => {
            w.u8(2);
            write_item(w, *id);
        }
    }
}

pub fn read_key(r: &mut Reader) -> Result<EntityKey> {
    let at = r.pos;
    Ok(match r.u8("key kind")? {
        0 => EntityKey::Placed(r.str(MAX_ID, "object id")?),
        1 => EntityKey::Slot {
            placeholder: r.str(MAX_ID, "placeholder id")?,
            slot: r.varint_u16("slot")?,
            copy: r.varint_u16("copy")?,
        },
        2 => EntityKey::Spawned(read_item(r)?),
        k => return Err(r.error(at, format!("unknown key kind {k}"))),
    })
}

fn write_hit_kind(w: &mut Writer, k: HitKind) {
    w.u8(match k {
        HitKind::Hand => 0,
        HitKind::Bash => 1,
        HitKind::Mech => 2,
    });
}

fn read_hit_kind(r: &mut Reader) -> Result<HitKind> {
    let at = r.pos;
    Ok(match r.u8("hit kind")? {
        0 => HitKind::Hand,
        1 => HitKind::Bash,
        2 => HitKind::Mech,
        k => return Err(r.error(at, format!("unknown hit kind {k}"))),
    })
}

pub fn write_store(w: &mut Writer, s: Store) {
    w.u8(match s {
        Store::Grid => 0,
        Store::Slots => 1,
        Store::PodLocker => 2,
    });
}

pub fn read_store(r: &mut Reader) -> Result<Store> {
    let at = r.pos;
    Ok(match r.u8("store")? {
        0 => Store::Grid,
        1 => Store::Slots,
        2 => Store::PodLocker,
        k => return Err(r.error(at, format!("unknown store {k}"))),
    })
}

fn write_target(w: &mut Writer, t: &MoveTarget) {
    match t {
        MoveTarget::Store => w.u8(0),
        MoveTarget::Item(id) => {
            w.u8(1);
            write_item(w, *id);
        }
        MoveTarget::Slot(s) => {
            w.u8(2);
            w.str(s);
        }
    }
}

fn read_target(r: &mut Reader) -> Result<MoveTarget> {
    let at = r.pos;
    Ok(match r.u8("move target")? {
        0 => MoveTarget::Store,
        1 => MoveTarget::Item(read_item(r)?),
        2 => MoveTarget::Slot(r.str(MAX_SLOT, "slot name")?),
        k => return Err(r.error(at, format!("unknown move target {k}"))),
    })
}

pub fn write_point(w: &mut Writer, p: [f32; 3]) {
    for c in p {
        w.f32(c);
    }
}

pub fn read_point(r: &mut Reader, what: &str) -> Result<[f32; 3]> {
    Ok([r.f32(what)?, r.f32(what)?, r.f32(what)?])
}

fn write_rotation(w: &mut Writer, q: [i16; 4]) {
    for c in q {
        w.i16(c);
    }
}

fn read_rotation(r: &mut Reader) -> Result<[i16; 4]> {
    Ok([
        r.i16("rotation x")?,
        r.i16("rotation y")?,
        r.i16("rotation z")?,
        r.i16("rotation w")?,
    ])
}

pub fn write_drop(w: &mut Writer, d: &DropAt) {
    write_point(w, d.point);
    write_rotation(w, d.rotation);
}

pub fn read_drop(r: &mut Reader) -> Result<DropAt> {
    Ok(DropAt {
        point: read_point(r, "drop point")?,
        rotation: read_rotation(r)?,
    })
}

pub fn write_death_drop(w: &mut Writer, d: &DeathDrop) {
    for c in d.offset {
        w.i16(c);
    }
    write_rotation(w, d.rotation);
}

pub fn read_death_drop(r: &mut Reader) -> Result<DeathDrop> {
    Ok(DeathDrop {
        offset: [
            r.i16("drop offset x")?,
            r.i16("drop offset y")?,
            r.i16("drop offset z")?,
        ],
        rotation: read_rotation(r)?,
    })
}

pub(crate) fn write_request(w: &mut Writer, q: &Request) {
    match q {
        Request::PickUp { key, class } => {
            w.u8(0);
            write_key(w, key);
            w.str(class);
        }
        Request::Hit {
            key,
            class,
            kind,
            spawn,
        } => {
            w.u8(1);
            write_key(w, key);
            w.str(class);
            write_hit_kind(w, *kind);
            write_point(w, *spawn);
        }
        Request::Drop { item, at } => {
            w.u8(2);
            write_item(w, *item);
            write_drop(w, at);
        }
        Request::Move { item, to, target } => {
            w.u8(3);
            write_item(w, *item);
            write_store(w, *to);
            write_target(w, target);
        }
        Request::Lose { origin, drops } => {
            w.u8(4);
            write_point(w, *origin);
            w.varint(drops.len() as u64);
            for d in drops {
                write_death_drop(w, d);
            }
        }
    }
}

pub(crate) fn read_request(r: &mut Reader) -> Result<Request> {
    let at = r.pos;
    Ok(match r.u8("request kind")? {
        0 => Request::PickUp {
            key: read_key(r)?,
            class: r.str(MAX_ID, "class id")?,
        },
        1 => Request::Hit {
            key: read_key(r)?,
            class: r.str(MAX_ID, "class id")?,
            kind: read_hit_kind(r)?,
            spawn: read_point(r, "spawn point")?,
        },
        2 => Request::Drop {
            item: read_item(r)?,
            at: read_drop(r)?,
        },
        3 => Request::Move {
            item: read_item(r)?,
            to: read_store(r)?,
            target: read_target(r)?,
        },
        4 => {
            let origin = read_point(r, "drops origin")?;
            let n = r.count(MAX_DROPS, "drop count")?;
            let mut drops = Vec::with_capacity(n);
            for _ in 0..n {
                drops.push(read_death_drop(r)?);
            }
            Request::Lose { origin, drops }
        }
        k => return Err(r.error(at, format!("unknown request kind {k}"))),
    })
}

pub(crate) fn write_item_event(w: &mut Writer, e: &ItemEvent) {
    match e {
        ItemEvent::PickedUp { key, class } => {
            w.u8(0);
            write_key(w, key);
            w.str(class);
        }
        ItemEvent::Hit {
            key,
            class,
            kind,
            spawn,
            draws,
        } => {
            w.u8(1);
            write_key(w, key);
            w.str(class);
            write_hit_kind(w, *kind);
            write_point(w, *spawn);
            w.varint(draws.len() as u64);
            for d in draws {
                w.f32(*d);
            }
        }
        ItemEvent::Dropped { item, at } => {
            w.u8(2);
            write_item(w, *item);
            write_drop(w, at);
        }
        ItemEvent::Moved { item, to, target } => {
            w.u8(3);
            write_item(w, *item);
            write_store(w, *to);
            write_target(w, target);
        }
        ItemEvent::Lost { origin, drops } => {
            w.u8(4);
            write_point(w, *origin);
            w.varint(drops.len() as u64);
            for (item, d) in drops {
                write_item(w, *item);
                write_death_drop(w, d);
            }
        }
    }
}

pub(crate) fn read_item_event(r: &mut Reader) -> Result<ItemEvent> {
    let at = r.pos;
    Ok(match r.u8("item event kind")? {
        0 => ItemEvent::PickedUp {
            key: read_key(r)?,
            class: r.str(MAX_ID, "class id")?,
        },
        1 => {
            let key = read_key(r)?;
            let class = r.str(MAX_ID, "class id")?;
            let kind = read_hit_kind(r)?;
            let spawn = read_point(r, "spawn point")?;
            let n = r.count(MAX_DRAWS, "draw count")?;
            let mut draws = Vec::with_capacity(n);
            for _ in 0..n {
                let at = r.pos;
                let d = r.f32("draw")?;
                if !(0.0..=1.0).contains(&d) {
                    return Err(r.error(at, format!("draw {d} is outside 0…1")));
                }
                draws.push(d);
            }
            ItemEvent::Hit {
                key,
                class,
                kind,
                spawn,
                draws,
            }
        }
        2 => ItemEvent::Dropped {
            item: read_item(r)?,
            at: read_drop(r)?,
        },
        3 => ItemEvent::Moved {
            item: read_item(r)?,
            to: read_store(r)?,
            target: read_target(r)?,
        },
        4 => {
            let origin = read_point(r, "drops origin")?;
            let n = r.count(MAX_DROPS, "drop count")?;
            let mut drops = Vec::with_capacity(n);
            for _ in 0..n {
                drops.push((read_item(r)?, read_death_drop(r)?));
            }
            ItemEvent::Lost { origin, drops }
        }
        k => return Err(r.error(at, format!("unknown item event kind {k}"))),
    })
}
