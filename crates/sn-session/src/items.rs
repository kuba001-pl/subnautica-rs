//! Items as shared world state (M9d3b, `docs/DESIGN.md` "M9d3 plan"):
//! every player's inventory, equipment, blueprints and luck, the lifepod's
//! locker, the world objects that are gone, the ones the server made, and
//! the outcrops' hits. Every machine applies the same `Item` world events
//! through `sn-sim::inventory` with the same [`ItemRules`], so all hold
//! the same state; its canonical encoding is both the join snapshot and
//! what the world hash covers.

use std::collections::BTreeMap;
use std::sync::Arc;

use sn_protocol::Fnv64;
use sn_protocol::codec::{Reader, Writer};
use sn_protocol::items::{read_item, read_key, read_point, write_item, write_key, write_point};
use sn_protocol::{
    DeathDrop, EntityKey, HitKind, ItemEvent, ItemId, MoveTarget, PlayerId, RefuseReason, Store,
    quant,
};
use sn_sim::inventory::{
    self as inv, AnalysisEntry, Container, ContainerParts, DropPlace, FairRandomizer, Inventory,
    Item, KnownTech, Outcrop, PlayerEntropy, Resource, SlotTable, TechType, Techs,
};

/// What the rules need to know of a pickupable prefab.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupInfo {
    /// By the game's rule (`CraftData.GetTechType`).
    pub tech: TechType,
    /// `Pickupable.isPickupable`.
    pub pickupable: bool,
    pub destroy_on_death: bool,
    /// Has an `Eatable`.
    pub eatable: bool,
}

/// One entry of an outcrop's `prefabList`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutcropDrop {
    pub tech: TechType,
    pub chance: f32,
    pub class: String,
}

/// An outcrop prefab's `BreakableResource`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutcropInfo {
    pub hits_to_break: i32,
    pub num_chances: i32,
    pub list: Vec<OutcropDrop>,
    pub default_class: String,
}

/// The item rules read from the install (M9d3d builds them; tests make
/// their own). Two machines with the same rules fingerprint hold the
/// same.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRules {
    pub techs: Techs,
    pub slots: SlotTable,
    /// `Inventory.UnlockDefaultEquipmentSlots`.
    pub player_slots: Vec<String>,
    /// `Inventory.Awake`'s grid.
    pub inventory_size: (i32, i32),
    pub analysis: Vec<AnalysisEntry>,
    /// `PDAData.defaultTech`.
    pub default_tech: Vec<TechType>,
    /// By prefab class id.
    pub pickups: BTreeMap<String, PickupInfo>,
    pub outcrops: BTreeMap<String, OutcropInfo>,
    pub locker_size: (i32, i32),
    /// `LootSpawner.escapePodTechTypes`.
    pub supplies: Vec<TechType>,
    /// `PlayerEntropy.randomizers` at the start.
    pub luck: Vec<(TechType, f32)>,
    /// The prefab the server spawns for a tech type (the supplies).
    pub tech_class: BTreeMap<TechType, String>,
}

/// An object the server made, in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldObject {
    pub class: String,
    pub point: [f32; 3],
    pub rotation: [i16; 4],
}

/// Identity, as the wire's rotation.
pub const NO_ROTATION: [i16; 4] = [0, 0, 0, 32_767];

/// An entry of a [`Summed`] map, as the snapshot encodes it.
pub trait Entry<V> {
    fn write(&self, value: &V, w: &mut Writer);
}

impl Entry<()> for EntityKey {
    fn write(&self, _: &(), w: &mut Writer) {
        write_key(w, self);
    }
}

impl Entry<i32> for EntityKey {
    fn write(&self, hits: &i32, w: &mut Writer) {
        write_key(w, self);
        w.varint_signed(i64::from(*hits));
    }
}

impl Entry<WorldObject> for ItemId {
    fn write(&self, o: &WorldObject, w: &mut Writer) {
        write_item(w, *self);
        w.str(&o.class);
        write_point(w, o.point);
        for c in o.rotation {
            w.i16(c);
        }
    }
}

impl Entry<String> for ItemId {
    fn write(&self, class: &String, w: &mut Writer) {
        write_item(w, *self);
        w.str(class);
    }
}

/// A map that keeps the sum of its entries' hashes (FNV-1a 64 of each
/// entry's encoding, added with wrap-around), so the world hash costs
/// the same however many objects are gone (M9d3c: hashing the whole
/// encoding after every event made a long game's events ever slower).
#[derive(Clone, Debug, PartialEq)]
pub struct Summed<K, V> {
    map: BTreeMap<K, V>,
    sum: u64,
}

impl<K, V> Default for Summed<K, V> {
    fn default() -> Self {
        Summed {
            map: BTreeMap::new(),
            sum: 0,
        }
    }
}

impl<K: Ord + Entry<V>, V> Summed<K, V> {
    fn entry_hash(k: &K, v: &V) -> u64 {
        let mut w = Writer::default();
        k.write(v, &mut w);
        Fnv64::new().bytes(&w.out).finish()
    }

    /// Adds or replaces; the replaced value.
    pub(crate) fn insert(&mut self, k: K, v: V) -> Option<V> {
        if let Some(old) = self.map.get(&k) {
            self.sum = self.sum.wrapping_sub(Self::entry_hash(&k, old));
        }
        self.sum = self.sum.wrapping_add(Self::entry_hash(&k, &v));
        self.map.insert(k, v)
    }

    pub(crate) fn remove(&mut self, k: &K) -> Option<V> {
        let v = self.map.remove(k)?;
        self.sum = self.sum.wrapping_sub(Self::entry_hash(k, &v));
        Some(v)
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k)
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    pub fn contains_key(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.map.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    /// The sum of the entries' hashes.
    pub fn sum(&self) -> u64 {
        self.sum
    }

    fn write(&self, w: &mut Writer) {
        w.varint(self.map.len() as u64);
        for (k, v) in &self.map {
            k.write(v, w);
        }
    }
}

impl<K: Ord + Entry<V>, V> std::ops::Index<&K> for Summed<K, V> {
    type Output = V;

    fn index(&self, k: &K) -> &V {
        &self.map[k]
    }
}

/// One player's items.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerItems {
    pub inventory: Inventory,
    pub luck: PlayerEntropy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemsState {
    rules: Arc<ItemRules>,
    /// The number the next item or spawned object gets.
    pub next_item: u32,
    /// World objects (placed or slot-filled) that are no longer there.
    pub gone: Summed<EntityKey, ()>,
    /// Objects the server made that are in the world.
    pub objects: Summed<ItemId, WorldObject>,
    /// The prefab of every item in a store (for when it is dropped).
    pub classes: Summed<ItemId, String>,
    /// Outcrops hit and not broken: their hits left.
    pub outcrops: Summed<EntityKey, i32>,
    pub locker: Container,
    /// Kept after a player leaves: a rejoin is the same player.
    pub players: BTreeMap<PlayerId, PlayerItems>,
}

fn sim(id: ItemId) -> inv::ItemId {
    inv::ItemId(u64::from(id.0))
}

/// The rules' item for a prefab, numbered `id`.
fn make_item(id: ItemId, info: &PickupInfo) -> Item {
    Item {
        id: sim(id),
        tech: info.tech,
        destroy_on_death: info.destroy_on_death,
        eatable: info.eatable,
        charge: None,
    }
}

impl ItemsState {
    /// The items of a world: for a new one (`supplies`), the lifepod's
    /// locker gets its supplies, each if it has room.
    pub fn new(rules: Arc<ItemRules>, supplies: bool) -> ItemsState {
        let (w, h) = rules.locker_size;
        let mut s = ItemsState {
            locker: Container::new(w, h),
            rules,
            next_item: 0,
            gone: Summed::default(),
            objects: Summed::default(),
            classes: Summed::default(),
            outcrops: Summed::default(),
            players: BTreeMap::new(),
        };
        if supplies {
            let rules = s.rules.clone();
            for tech in &rules.supplies {
                let Some(class) = rules.tech_class.get(tech) else {
                    continue;
                };
                let Some(info) = rules.pickups.get(class) else {
                    continue;
                };
                let id = ItemId(s.next_item);
                s.next_item += 1;
                if s.locker.add(&rules.techs, make_item(id, info)).is_ok() {
                    s.classes.insert(id, class.clone());
                }
            }
        }
        s
    }

    pub fn rules(&self) -> &Arc<ItemRules> {
        &self.rules
    }

    /// A player joined: its items, new or kept from before.
    pub fn join(&mut self, player: PlayerId) {
        if self.players.contains_key(&player) {
            return;
        }
        let r = &self.rules;
        let known = KnownTech::new(&r.analysis, &r.default_tech);
        let inventory = Inventory::new(r.inventory_size, &r.slots, &r.player_slots, known);
        let luck = PlayerEntropy {
            randomizers: r
                .luck
                .iter()
                .map(|&(tech, entropy)| (tech, FairRandomizer { entropy }))
                .collect(),
        };
        self.players.insert(player, PlayerItems { inventory, luck });
    }

    /// Where an item of `player` is: 0 the grid, 1 the slots, 2 the
    /// locker.
    fn find(&self, p: &PlayerItems, id: ItemId) -> Option<usize> {
        let id = sim(id);
        if p.inventory.container.contains(id) {
            Some(0)
        } else if p.inventory.equipment.slot_of(id).is_some() {
            Some(1)
        } else if self.locker.contains(id) {
            Some(2)
        } else {
            None
        }
    }

    /// Whether `player` holds `id` (grid or slots).
    pub fn owns(&self, player: PlayerId, id: ItemId) -> bool {
        self.players
            .get(&player)
            .is_some_and(|p| p.inventory.contains(sim(id)))
    }

    pub fn in_locker(&self, id: ItemId) -> bool {
        self.locker.contains(sim(id))
    }

    /// Applies one item event of `player`; on a refusal nothing changes.
    pub fn apply(&mut self, player: PlayerId, event: &ItemEvent) -> Result<(), RefuseReason> {
        let rules = self.rules.clone();
        let p = self.players.get(&player).ok_or(RefuseReason::Unknown)?;
        match event {
            ItemEvent::PickedUp { key, class } => {
                let (id, info) = match key {
                    EntityKey::Spawned(id) => {
                        let o = self.objects.get(id).ok_or(RefuseReason::Gone)?;
                        if &o.class != class {
                            return Err(RefuseReason::Unknown);
                        }
                        (*id, rules.pickups.get(class).ok_or(RefuseReason::Unknown)?)
                    }
                    _ => {
                        if self.gone.contains(key) {
                            return Err(RefuseReason::Gone);
                        }
                        let info = rules.pickups.get(class).ok_or(RefuseReason::Unknown)?;
                        (ItemId(self.next_item), info)
                    }
                };
                if !info.pickupable {
                    return Err(RefuseReason::Unknown);
                }
                let next = match key {
                    EntityKey::Spawned(_) => self.next_item,
                    _ => self
                        .next_item
                        .checked_add(1)
                        .ok_or(RefuseReason::NotAllowed)?,
                };
                let mut pi = p.clone();
                inv::pickup(
                    &mut pi.inventory,
                    &rules.techs,
                    &rules.analysis,
                    make_item(id, info),
                    false,
                )
                .map_err(|_| RefuseReason::NoRoom)?;
                self.players.insert(player, pi);
                self.classes.insert(id, class.clone());
                match key {
                    EntityKey::Spawned(_) => {
                        self.objects.remove(&id);
                    }
                    _ => {
                        self.gone.insert(key.clone(), ());
                    }
                }
                self.next_item = next;
            }
            ItemEvent::Hit {
                key,
                class,
                kind,
                spawn,
                draws,
            } => {
                if matches!(key, EntityKey::Spawned(_)) {
                    return Err(RefuseReason::Unknown);
                }
                if self.gone.contains(key) {
                    return Err(RefuseReason::Gone);
                }
                let info = rules.outcrops.get(class).ok_or(RefuseReason::Unknown)?;
                let mut o = Outcrop::new(
                    self.outcrops
                        .get(key)
                        .copied()
                        .unwrap_or(info.hits_to_break),
                );
                if !o.hit(sim_hit(*kind)) {
                    if !draws.is_empty() {
                        return Err(RefuseReason::NotAllowed);
                    }
                    self.outcrops.insert(key.clone(), o.hits_left);
                    return Ok(());
                }
                let mut luck = p.luck.clone();
                let (resources, used) = draw(info, &mut luck, draws.iter().copied());
                if used != Some(draws.len()) {
                    return Err(RefuseReason::NotAllowed);
                }
                let n = u32::try_from(resources.len()).map_err(|_| RefuseReason::NotAllowed)?;
                let next = self
                    .next_item
                    .checked_add(n)
                    .ok_or(RefuseReason::NotAllowed)?;
                for (i, r) in resources.iter().enumerate() {
                    let class = match r {
                        Resource::Listed(k) => {
                            info.list.get(*k).map_or(&info.default_class, |d| &d.class)
                        }
                        Resource::Default => &info.default_class,
                    };
                    self.objects.insert(
                        ItemId(self.next_item + i as u32),
                        WorldObject {
                            class: class.clone(),
                            point: *spawn,
                            rotation: NO_ROTATION,
                        },
                    );
                }
                self.next_item = next;
                if let Some(pi) = self.players.get_mut(&player) {
                    pi.luck = luck;
                }
                self.outcrops.remove(key);
                self.gone.insert(key.clone(), ());
            }
            ItemEvent::Dropped { item, at } => {
                let class = self.classes.get(item).ok_or(RefuseReason::NoItem)?.clone();
                let mut pi = p.clone();
                pi.inventory
                    .drop_item(sim(*item), DropPlace::default())
                    .ok_or(RefuseReason::NoItem)?;
                self.players.insert(player, pi);
                self.classes.remove(item);
                self.objects.insert(
                    *item,
                    WorldObject {
                        class,
                        point: at.point,
                        rotation: at.rotation,
                    },
                );
            }
            ItemEvent::Moved { item, to, target } => {
                let from = self.find(p, *item).ok_or(RefuseReason::NoItem)?;
                let to = match to {
                    Store::Grid => 0,
                    Store::Slots => 1,
                    Store::PodLocker => 2,
                };
                let target = match target {
                    MoveTarget::Store => inv::Target::Store,
                    MoveTarget::Item(b) => inv::Target::Item(sim(*b)),
                    MoveTarget::Slot(s) => inv::Target::Slot(s.clone()),
                };
                let mut pi = p.clone();
                let mut locker = self.locker.clone();
                let moved = {
                    let mut stores = [
                        inv::Store::Grid(&mut pi.inventory.container),
                        inv::Store::Slots(&mut pi.inventory.equipment),
                        inv::Store::Grid(&mut locker),
                    ];
                    inv::add_or_swap(&rules.techs, &mut stores, from, sim(*item), to, &target)
                };
                if !moved {
                    return Err(RefuseReason::NotAllowed);
                }
                // An item added to the player's grid is analysed
                // (`Inventory.OnAddItem`).
                let mut ids = vec![sim(*item)];
                if let inv::Target::Item(b) = target {
                    ids.push(b);
                }
                for id in ids {
                    if let Some(tech) = pi.inventory.container.get(id).map(|i| i.tech) {
                        pi.inventory.known.analyze(&rules.analysis, tech);
                    }
                }
                self.players.insert(player, pi);
                self.locker = locker;
            }
            ItemEvent::Lost { origin, drops } => {
                let mut pi = p.clone();
                let mut placed = Vec::with_capacity(drops.len());
                for (item, d) in drops {
                    let class = self.classes.get(item).ok_or(RefuseReason::NoItem)?;
                    pi.inventory
                        .drop_item(sim(*item), DropPlace::default())
                        .ok_or(RefuseReason::NoItem)?;
                    placed.push((*item, class.clone(), death_point(*origin, d)));
                }
                self.players.insert(player, pi);
                for (item, class, (point, rotation)) in placed {
                    self.classes.remove(&item);
                    self.objects.insert(
                        item,
                        WorldObject {
                            class,
                            point,
                            rotation,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// The server's side of a hit: the luck draws it must send, drawn
    /// from `random` (`Random.value`s), if the hit breaks the outcrop.
    /// Refused as [`ItemsState::apply`] would refuse it.
    pub fn draws_for_hit(
        &self,
        player: PlayerId,
        key: &EntityKey,
        class: &str,
        kind: HitKind,
        random: &mut impl FnMut() -> f32,
    ) -> Result<Vec<f32>, RefuseReason> {
        let p = self.players.get(&player).ok_or(RefuseReason::Unknown)?;
        if matches!(key, EntityKey::Spawned(_)) {
            return Err(RefuseReason::Unknown);
        }
        if self.gone.contains(key) {
            return Err(RefuseReason::Gone);
        }
        let info = self
            .rules
            .outcrops
            .get(class)
            .ok_or(RefuseReason::Unknown)?;
        let mut o = Outcrop::new(
            self.outcrops
                .get(key)
                .copied()
                .unwrap_or(info.hits_to_break),
        );
        if !o.hit(sim_hit(kind)) {
            return Ok(Vec::new());
        }
        let mut taken = Vec::new();
        let mut luck = p.luck.clone();
        let list: Vec<(TechType, f32)> = info.list.iter().map(|d| (d.tech, d.chance)).collect();
        inv::draw_resources(&list, info.num_chances, &mut luck, &mut || {
            let v = random().clamp(0.0, 1.0);
            taken.push(v);
            v
        });
        Ok(taken)
    }

    /// The items a death of `player` drops (`LoseItems`), in order.
    pub fn lose_list(&self, player: PlayerId, place: DropPlace) -> Vec<ItemId> {
        let Some(p) = self.players.get(&player) else {
            return Vec::new();
        };
        let mut copy = p.inventory.clone();
        inv::lose_items(&mut copy, place)
            .iter()
            .map(|i| ItemId(i.id.0 as u32))
            .collect()
    }

    /// The canonical encoding (`docs/formats/protocol.md` § Items state).
    pub fn encode(&self, w: &mut Writer) {
        w.varint(u64::from(self.next_item));
        self.gone.write(w);
        self.objects.write(w);
        self.classes.write(w);
        self.outcrops.write(w);
        self.write_rest(w);
    }

    /// The locker and the players (their part of the encoding).
    fn write_rest(&self, w: &mut Writer) {
        write_container(w, &self.locker);
        w.varint(self.players.len() as u64);
        for (id, p) in &self.players {
            w.varint(u64::from(id.0));
            write_container(w, &p.inventory.container);
            for (_, item) in p.inventory.equipment.slots() {
                match item {
                    Some(i) => {
                        w.bool(true);
                        write_sim_item(w, i);
                    }
                    None => w.bool(false),
                }
            }
            let known: Vec<TechType> = p.inventory.known.known().collect();
            write_techs(w, &known);
            let analyzed: Vec<TechType> = p.inventory.known.analyzed_all().collect();
            write_techs(w, &analyzed);
            w.varint(p.luck.randomizers.len() as u64);
            for (tech, r) in &p.luck.randomizers {
                w.varint_signed(i64::from(*tech));
                w.f32(r.entropy);
            }
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::default();
        self.encode(&mut w);
        w.out
    }

    /// Adds the state to a world hash: the next number, each large map's
    /// size and entry-hash sum, then the encoding of the locker and the
    /// players (small: bounded by the containers' sizes).
    pub fn hash_into(&self, h: &mut Fnv64) {
        h.u64(u64::from(self.next_item));
        for (len, sum) in [
            (self.gone.len(), self.gone.sum()),
            (self.objects.len(), self.objects.sum()),
            (self.classes.len(), self.classes.sum()),
            (self.outcrops.len(), self.outcrops.sum()),
        ] {
            h.u64(len as u64).u64(sum);
        }
        let mut w = Writer::default();
        self.write_rest(&mut w);
        h.bytes(&w.out);
    }

    /// The state [`ItemsState::encode`] wrote, under `rules`; every
    /// error names its offset.
    pub fn decode(rules: Arc<ItemRules>, r: &mut Reader) -> sn_protocol::Result<ItemsState> {
        let next_item = r.varint_u32("next item")?;
        let mut gone = Summed::default();
        for _ in 0..r.count(usize::MAX, "gone count")? {
            let at = r.pos;
            let k = read_key(r)?;
            if gone.insert(k, ()).is_some() {
                return Err(r.error(at, "gone key repeated"));
            }
        }
        let mut objects = Summed::default();
        for _ in 0..r.count(usize::MAX, "object count")? {
            let at = r.pos;
            let id = read_item(r)?;
            let class = r.str(sn_protocol::items::MAX_ID, "object class")?;
            let point = read_point(r, "object point")?;
            let rotation = [
                r.i16("object rotation")?,
                r.i16("object rotation")?,
                r.i16("object rotation")?,
                r.i16("object rotation")?,
            ];
            if objects
                .insert(
                    id,
                    WorldObject {
                        class,
                        point,
                        rotation,
                    },
                )
                .is_some()
            {
                return Err(r.error(at, "object repeated"));
            }
        }
        let mut classes = Summed::default();
        for _ in 0..r.count(usize::MAX, "class count")? {
            let at = r.pos;
            let id = read_item(r)?;
            let c = r.str(sn_protocol::items::MAX_ID, "item class")?;
            if classes.insert(id, c).is_some() {
                return Err(r.error(at, "item class repeated"));
            }
        }
        let mut outcrops = Summed::default();
        for _ in 0..r.count(usize::MAX, "outcrop count")? {
            let at = r.pos;
            let k = read_key(r)?;
            let hits = r.varint_i32("outcrop hits")?;
            if outcrops.insert(k, hits).is_some() {
                return Err(r.error(at, "outcrop repeated"));
            }
        }
        let locker = read_container(r)?;
        let mut players = BTreeMap::new();
        for _ in 0..r.count(usize::MAX, "player count")? {
            let at = r.pos;
            let id = PlayerId(r.varint_u16("player id")?);
            let container = read_container(r)?;
            let mut equipment = inv::Equipment::new(&rules.slots, &rules.player_slots);
            let names: Vec<String> = equipment.slots().map(|(n, _)| n.to_owned()).collect();
            for name in names {
                if r.bool("slot filled")? {
                    let at = r.pos;
                    let item = read_sim_item(r)?;
                    if equipment.add_to(&rules.techs, &name, item, true).is_err() {
                        return Err(r.error(at, "slot item does not fit"));
                    }
                }
            }
            let known = read_techs(r)?;
            let analyzed = read_techs(r)?;
            let mut randomizers = Vec::new();
            for _ in 0..r.count(usize::MAX, "luck count")? {
                let tech = r.varint_i32("luck tech")?;
                let entropy = r.f32("luck entropy")?;
                randomizers.push((tech, FairRandomizer { entropy }));
            }
            let inventory = Inventory {
                container,
                equipment,
                known: KnownTech::from_sets(known, analyzed),
            };
            let p = PlayerItems {
                inventory,
                luck: PlayerEntropy { randomizers },
            };
            if players.insert(id, p).is_some() {
                return Err(r.error(at, "player repeated"));
            }
        }
        Ok(ItemsState {
            rules,
            next_item,
            gone,
            objects,
            classes,
            outcrops,
            locker,
            players,
        })
    }
}

fn sim_hit(k: HitKind) -> inv::HitKind {
    match k {
        HitKind::Hand => inv::HitKind::Hand,
        HitKind::Bash => inv::HitKind::Bash,
        HitKind::Mech => inv::HitKind::Mech,
    }
}

/// The resources a broken outcrop gives with these draws, and how many
/// draws that took (`None` if they ran out).
fn draw(
    info: &OutcropInfo,
    luck: &mut PlayerEntropy,
    draws: impl Iterator<Item = f32>,
) -> (Vec<Resource>, Option<usize>) {
    let list: Vec<(TechType, f32)> = info.list.iter().map(|d| (d.tech, d.chance)).collect();
    let mut draws = draws;
    let mut used = 0;
    let mut short = false;
    let out = inv::draw_resources(&list, info.num_chances, luck, &mut || match draws.next() {
        Some(v) => {
            used += 1;
            v
        }
        None => {
            short = true;
            1.0
        }
    });
    (out, (!short).then_some(used))
}

/// A death drop's point and rotation.
pub fn death_point(origin: [f32; 3], d: &DeathDrop) -> ([f32; 3], [i16; 4]) {
    let scale = quant::DROP_OFFSET_SCALE as f32;
    (
        [0, 1, 2].map(|i| origin[i] + f32::from(d.offset[i]) / scale),
        d.rotation,
    )
}

fn write_techs(w: &mut Writer, techs: &[TechType]) {
    w.varint(techs.len() as u64);
    for t in techs {
        w.varint_signed(i64::from(*t));
    }
}

fn read_techs(r: &mut Reader) -> sn_protocol::Result<Vec<TechType>> {
    let mut out: Vec<TechType> = Vec::new();
    for _ in 0..r.count(usize::MAX, "tech count")? {
        let at = r.pos;
        let t = r.varint_i32("tech type")?;
        if out.last().is_some_and(|&l| l >= t) {
            return Err(r.error(at, "tech types not in increasing order"));
        }
        out.push(t);
    }
    Ok(out)
}

const DESTROY: u8 = 1;
const EATABLE: u8 = 2;
const CHARGE: u8 = 4;

fn write_sim_item(w: &mut Writer, i: &Item) {
    w.varint(i.id.0);
    w.varint_signed(i64::from(i.tech));
    let flags = if i.destroy_on_death { DESTROY } else { 0 }
        | if i.eatable { EATABLE } else { 0 }
        | if i.charge.is_some() { CHARGE } else { 0 };
    w.u8(flags);
    if let Some(c) = i.charge {
        w.f32(c);
    }
}

fn read_sim_item(r: &mut Reader) -> sn_protocol::Result<Item> {
    let id = r.varint_u32("item")?;
    let tech = r.varint_i32("item tech")?;
    let at = r.pos;
    let flags = r.u8("item flags")?;
    if flags & !(DESTROY | EATABLE | CHARGE) != 0 {
        return Err(r.error(at, format!("unknown item flags {flags:#04x}")));
    }
    let charge = if flags & CHARGE != 0 {
        Some(r.f32("item charge")?)
    } else {
        None
    };
    Ok(Item {
        id: inv::ItemId(u64::from(id)),
        tech,
        destroy_on_death: flags & DESTROY != 0,
        eatable: flags & EATABLE != 0,
        charge,
    })
}

fn write_container(w: &mut Writer, c: &Container) {
    let p = c.to_parts();
    w.varint_signed(i64::from(p.width));
    w.varint_signed(i64::from(p.height));
    w.varint(p.entries.len() as u64);
    for e in &p.entries {
        match e {
            None => w.bool(false),
            Some((tech, width, height, items)) => {
                w.bool(true);
                w.varint_signed(i64::from(*tech));
                w.varint_signed(i64::from(*width));
                w.varint_signed(i64::from(*height));
                w.varint(items.len() as u64);
                for i in items {
                    write_sim_item(w, i);
                }
            }
        }
    }
    w.varint(p.free.len() as u64);
    for f in &p.free {
        w.varint(*f as u64);
    }
}

fn read_container(r: &mut Reader) -> sn_protocol::Result<Container> {
    let at = r.pos;
    let width = r.varint_i32("container width")?;
    let height = r.varint_i32("container height")?;
    let mut entries = Vec::new();
    for _ in 0..r.count(usize::MAX, "group count")? {
        if r.bool("group present")? {
            let tech = r.varint_i32("group tech")?;
            let w = r.varint_i32("group width")?;
            let h = r.varint_i32("group height")?;
            let mut items = Vec::new();
            for _ in 0..r.count(usize::MAX, "item count")? {
                items.push(read_sim_item(r)?);
            }
            entries.push(Some((tech, w, h, items)));
        } else {
            entries.push(None);
        }
    }
    let mut free = Vec::new();
    for _ in 0..r.count(usize::MAX, "free count")? {
        free.push(r.count(usize::MAX, "free entry")?);
    }
    Container::from_parts(ContainerParts {
        width,
        height,
        entries,
        free,
    })
    .map_err(|e| r.error(at, format!("container: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(s: &ItemsState) -> u64 {
        let mut h = Fnv64::new();
        s.hash_into(&mut h);
        h.finish()
    }

    #[test]
    fn every_part_moves_the_hash_and_the_sums_follow_the_contents() {
        let rules = Arc::new(ItemRules {
            inventory_size: (2, 2),
            locker_size: (2, 2),
            slots: SlotTable {
                slots: vec![("Tank".into(), 2)],
                ..Default::default()
            },
            player_slots: vec!["Tank".into()],
            ..Default::default()
        });
        let t = &rules.techs.clone();
        let mut base = ItemsState::new(rules.clone(), false);
        base.join(PlayerId(0));
        let key = EntityKey::Placed("a".into());
        let object = WorldObject {
            class: "c".into(),
            point: [0.0; 3],
            rotation: NO_ROTATION,
        };
        let mut variants = Vec::new();
        let mut v = base.clone();
        v.next_item = 1;
        variants.push(v);
        let mut v = base.clone();
        v.gone.insert(key.clone(), ());
        variants.push(v);
        let mut v = base.clone();
        v.objects.insert(ItemId(1), object.clone());
        variants.push(v.clone());
        // The same key, another value.
        v.objects.insert(
            ItemId(1),
            WorldObject {
                point: [0.5, 0.0, 0.0],
                ..object.clone()
            },
        );
        variants.push(v);
        let mut v = base.clone();
        v.classes.insert(ItemId(1), "c".into());
        variants.push(v);
        let mut v = base.clone();
        v.outcrops.insert(key.clone(), 3);
        variants.push(v.clone());
        v.outcrops.insert(key.clone(), 2);
        variants.push(v);
        let mut v = base.clone();
        v.locker.add(t, Item::new(1, 5)).unwrap();
        variants.push(v);
        let p0 = PlayerId(0);
        let mut v = base.clone();
        let p = v.players.get_mut(&p0).unwrap();
        p.inventory.container.add(t, Item::new(1, 5)).unwrap();
        variants.push(v);
        let mut v = base.clone();
        let p = v.players.get_mut(&p0).unwrap();
        p.inventory
            .equipment
            .add_to(t, "Tank", Item::new(1, 5), true)
            .unwrap();
        variants.push(v);
        let mut v = base.clone();
        v.players.get_mut(&p0).unwrap().inventory.known.add(&[], 7);
        variants.push(v);
        let mut v = base.clone();
        v.players.get_mut(&p0).unwrap().inventory.known = KnownTech::from_sets([], [7]);
        variants.push(v);
        let mut v = base.clone();
        v.players
            .get_mut(&p0)
            .unwrap()
            .luck
            .randomizers
            .push((1, FairRandomizer { entropy: 0.5 }));
        variants.push(v);
        let mut v = base.clone();
        v.join(PlayerId(1));
        variants.push(v);
        let mut hashes: Vec<u64> = variants.iter().map(hash).collect();
        hashes.push(hash(&base));
        let n = hashes.len();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), n);

        // Removing what was added gives the same sums; so does decoding.
        let mut v = base.clone();
        v.gone.insert(key.clone(), ());
        v.objects.insert(ItemId(1), object.clone());
        v.objects.insert(ItemId(2), object);
        v.outcrops.insert(key.clone(), 1);
        let bytes = v.to_bytes();
        let back = ItemsState::decode(rules, &mut Reader::new(&bytes)).unwrap();
        assert_eq!(back, v);
        v.gone.remove(&key);
        v.objects.remove(&ItemId(1));
        v.objects.remove(&ItemId(2));
        v.outcrops.remove(&key);
        assert_eq!(v, base);
        assert_eq!(hash(&v), hash(&base));
    }
}
