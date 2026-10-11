//! What moves items: the player's inventory (`Inventory`), picking up
//! (`Inventory.Pickup`), dragging between containers and slots
//! (`Inventory.AddOrSwap`), dropping (`InternalDropItem`), death
//! (`LoseItems`), the lifepod's supplies (`SpawnEscapePodSupplies`) and
//! the `grab` animator bool.

use super::{
    AnalysisEntry, Container, DropPlace, Equipment, Item, ItemId, KnownTech, SlotTable, TechType,
    Techs, can_drop_here,
};

/// The player's items and blueprints.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    /// The grid (`new ItemsContainer(6, 8, …)` in `Inventory.Awake`).
    pub container: Container,
    pub equipment: Equipment,
    pub known: KnownTech,
}

impl Inventory {
    /// `Inventory.Awake`: the grid of `size` and the slots of
    /// `UnlockDefaultEquipmentSlots`.
    pub fn new<S: AsRef<str>>(
        size: (i32, i32),
        table: &SlotTable,
        slots: &[S],
        known: KnownTech,
    ) -> Inventory {
        Inventory {
            container: Container::new(size.0, size.1),
            equipment: Equipment::new(table, slots),
            known,
        }
    }

    pub fn contains(&self, id: ItemId) -> bool {
        self.container.contains(id) || self.equipment.slot_of(id).is_some()
    }

    /// `InternalDropItem` for one item: out of the grid or its slot, if
    /// dropping is allowed here. Where it lands is [`super::drop_ray`]
    /// then [`super::find_drop_position`].
    pub fn drop_item(&mut self, id: ItemId, place: DropPlace) -> Option<Item> {
        if !can_drop_here(place) {
            return None;
        }
        self.container
            .remove(id)
            .or_else(|| self.equipment.remove(id))
    }
}

/// Where a pick-up went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickedInto {
    Slot(String),
    Grid,
}

/// `Inventory.Pickup` (the hand's `AllowedToPickUp` asks the same room
/// first; the caller checks `isPickupable`). Refused, and the item given
/// back, without room in the grid, even for an item that would go to a
/// slot. Unless eatable the item goes into the first free slot of its
/// type, else into the grid; then it is analysed (the blueprints newly
/// known are returned). Picked up in a sub or base (`in_sub`) it is no
/// longer lost on death.
///
/// `AllowedToPickUp` also asks for more than 1 s since the item was
/// dropped, but `Pickupable.Drop` sets `timeDropped` to 0, not to the
/// time: the check passes once the game has run 1 s and is not ported.
pub fn pickup(
    inv: &mut Inventory,
    techs: &Techs,
    analysis: &[AnalysisEntry],
    mut item: Item,
    in_sub: bool,
) -> Result<(PickedInto, Vec<TechType>), Item> {
    if !inv.container.has_room_for(techs, item.tech) {
        return Err(item);
    }
    let tech = item.tech;
    if in_sub {
        item.destroy_on_death = false;
    }
    let into = if item.eatable {
        inv.container.unsafe_add(techs, item);
        PickedInto::Grid
    } else {
        match inv.equipment.add(techs, item) {
            Ok(slot) => PickedInto::Slot(slot),
            Err(item) => {
                inv.container.unsafe_add(techs, item);
                PickedInto::Grid
            }
        }
    };
    let unlocked = inv.known.analyze(analysis, tech);
    Ok((into, unlocked))
}

/// The `grab` animator bool (`ArmsController.Update`): true for 0.4 s
/// after `PlayGrab` (`timeGrabbed`, 0 when never).
pub fn grab_active(time_grabbed: f64, now: f64) -> bool {
    time_grabbed != 0.0 && time_grabbed + GRAB_TIME > now
}

pub const GRAB_TIME: f64 = 0.4;

/// `LoseItems` on death: every item lost on death (`destroyOnDeath`), the
/// grid's in its order then the slots', is dropped where dropping is
/// allowed, except the last one: the game's loop stops one short
/// (`i < Count − 1`), so one item is always kept. Ported as it is.
pub fn lose_items(inv: &mut Inventory, place: DropPlace) -> Vec<Item> {
    let list: Vec<ItemId> = inv
        .container
        .items()
        .chain(inv.equipment.items())
        .filter(|i| i.destroy_on_death)
        .map(|i| i.id)
        .collect();
    let n = list.len().saturating_sub(1);
    list.into_iter()
        .take(n)
        .filter_map(|id| inv.drop_item(id, place))
        .collect()
}

/// `SpawnEscapePodSupplies.OnNewBorn`: each item into the locker if it
/// has room; the ones that do not fit are returned (the game destroys
/// them).
pub fn spawn_supplies(locker: &mut Container, techs: &Techs, items: Vec<Item>) -> Vec<Item> {
    items
        .into_iter()
        .filter_map(|item| locker.add(techs, item).err())
        .collect()
}

/// A container or an equipment, for [`add_or_swap`].
pub enum Store<'a> {
    Grid(&'a mut Container),
    Slots(&'a mut Equipment),
}

/// Where a dragged item goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Into the store: a grid, or a compatible slot (the first free one,
    /// else the first one, swapping).
    Store,
    /// Onto an item of the store: a swap.
    Item(ItemId),
    /// Into a named slot (swapping with what it holds).
    Slot(String),
}

/// `Inventory.AddOrSwap`: moves the item `item` of `stores[from]` to
/// `stores[to]`, swapping with what is there when the target is taken.
/// True if it moved; on a refusal everything is back where it was.
pub fn add_or_swap(
    techs: &Techs,
    stores: &mut [Store],
    from: usize,
    item: ItemId,
    to: usize,
    target: &Target,
) -> bool {
    if from >= stores.len() || to >= stores.len() || find(stores, from, item).is_none() {
        return false;
    }
    let from_slot = match &stores[from] {
        Store::Slots(e) => match e.slot_of(item) {
            Some(s) => Some(s.to_owned()),
            None => return false,
        },
        Store::Grid(_) => None,
    };
    match (&stores[to], target) {
        (Store::Slots(e), Target::Item(b)) => match e.slot_of(*b) {
            Some(s) => {
                let s = s.to_owned();
                swap_into_slot(techs, stores, from, from_slot, item, to, Some(s))
            }
            None => false,
        },
        (Store::Slots(_), Target::Store) => {
            swap_into_slot(techs, stores, from, from_slot, item, to, None)
        }
        (Store::Slots(_), Target::Slot(s)) => {
            swap_into_slot(techs, stores, from, from_slot, item, to, Some(s.clone()))
        }
        (Store::Grid(_), Target::Slot(_)) => false,
        (Store::Grid(_), Target::Store) => {
            swap_into_grid(techs, stores, from, from_slot, item, to, None)
        }
        (Store::Grid(_), Target::Item(b)) => {
            swap_into_grid(techs, stores, from, from_slot, item, to, Some(*b))
        }
    }
}

/// `AddOrSwap(itemA, containerB, itemB)` with a grid `to`.
fn swap_into_grid(
    techs: &Techs,
    stores: &mut [Store],
    from: usize,
    from_slot: Option<String>,
    a: ItemId,
    to: usize,
    b: Option<ItemId>,
) -> bool {
    if from == to {
        return false;
    }
    let b = match b {
        Some(b) => match take(stores, to, b) {
            Some(item) => Some(item),
            None => return false,
        },
        None => None,
    };
    if !move_to_grid(techs, stores, from, a, to) {
        if let Some(b) = b {
            restore(techs, stores, to, None, b);
        }
        return false;
    }
    let Some(b) = b else { return true };
    let back = match &from_slot {
        Some(slot) => put(techs, stores, from, Some(slot), b, false),
        None => put(techs, stores, from, None, b, false),
    };
    let Err(b) = back else { return true };
    // Undo: a back where it was, b back into the grid.
    match &from_slot {
        Some(slot) => move_to_slot(techs, stores, to, a, from, slot, true),
        None => move_to_grid(techs, stores, to, a, from),
    };
    restore(techs, stores, to, None, b);
    false
}

/// `AddOrSwap(itemA, equipmentB, slotB)`.
fn swap_into_slot(
    techs: &Techs,
    stores: &mut [Store],
    from: usize,
    from_slot: Option<String>,
    a: ItemId,
    to: usize,
    slot: Option<String>,
) -> bool {
    let Some(tech) = find(stores, from, a) else {
        return false;
    };
    let Store::Slots(e) = &stores[to] else {
        return false;
    };
    let kind = techs.equipment_type(tech);
    let Some(slot) = slot.or_else(|| e.compatible_slot(kind).map(str::to_owned)) else {
        return false;
    };
    if from == to && from_slot.as_deref() == Some(slot.as_str()) {
        return false;
    }
    if !e.table().is_compatible(kind, e.table().slot_type(&slot)) {
        return false;
    }
    let removed = match &mut stores[to] {
        Store::Slots(e) => e.remove_slot(&slot),
        Store::Grid(_) => None,
    };
    let Some(removed) = removed else {
        return move_to_slot(techs, stores, from, a, to, &slot, false);
    };
    if !move_to_slot(techs, stores, from, a, to, &slot, false) {
        restore(techs, stores, to, Some(&slot), removed);
        return false;
    }
    let back = put(techs, stores, from, from_slot.as_deref(), removed, false);
    let Err(removed) = back else { return true };
    match &from_slot {
        Some(s) => move_to_slot(techs, stores, to, a, from, s, true),
        None => move_to_grid(techs, stores, to, a, from),
    };
    restore(techs, stores, to, Some(&slot), removed);
    false
}

fn find(stores: &[Store], i: usize, id: ItemId) -> Option<TechType> {
    match stores.get(i)? {
        Store::Grid(c) => c.get(id).map(|it| it.tech),
        Store::Slots(e) => e.items().find(|it| it.id == id).map(|it| it.tech),
    }
}

fn take(stores: &mut [Store], i: usize, id: ItemId) -> Option<Item> {
    match stores.get_mut(i)? {
        Store::Grid(c) => c.remove(id),
        Store::Slots(e) => e.remove(id),
    }
}

/// `ItemsContainer.AddItem` for an item in `src`: room is checked with
/// the item still where it was, then it moves.
fn move_to_grid(techs: &Techs, stores: &mut [Store], src: usize, id: ItemId, dst: usize) -> bool {
    let Some(tech) = find(stores, src, id) else {
        return false;
    };
    match stores.get(dst) {
        Some(Store::Grid(c)) if c.has_room_for(techs, tech) => {}
        _ => return false,
    }
    let Some(item) = take(stores, src, id) else {
        return false;
    };
    if let Some(Store::Grid(c)) = stores.get_mut(dst) {
        c.unsafe_add(techs, item);
    }
    true
}

/// `Equipment.AddItem(slot, item, forced)` for an item in `src`.
fn move_to_slot(
    techs: &Techs,
    stores: &mut [Store],
    src: usize,
    id: ItemId,
    dst: usize,
    slot: &str,
    forced: bool,
) -> bool {
    let Some(tech) = find(stores, src, id) else {
        return false;
    };
    match stores.get(dst) {
        Some(Store::Slots(e))
            if e.has_slot(slot)
                && e.item_in(slot).is_none()
                && (forced
                    || e.table()
                        .is_compatible(techs.equipment_type(tech), e.table().slot_type(slot))) => {}
        _ => return false,
    }
    let Some(item) = take(stores, src, id) else {
        return false;
    };
    match stores.get_mut(dst) {
        Some(Store::Slots(e)) => e.add_to(techs, slot, item, true).is_ok(),
        _ => false,
    }
}

/// `AddItem` for an item in no container: into the grid, or the slot.
fn put(
    techs: &Techs,
    stores: &mut [Store],
    dst: usize,
    slot: Option<&str>,
    item: Item,
    forced: bool,
) -> Result<(), Item> {
    match (stores.get_mut(dst), slot) {
        (Some(Store::Grid(c)), None) => c.add(techs, item),
        (Some(Store::Slots(e)), Some(slot)) => e.add_to(techs, slot, item, forced),
        _ => Err(item),
    }
}

/// Puts an item back where it just was (the game's undo adds without
/// checking the result). The place was emptied in this same move, so it
/// fits; should it not, the item goes into the grid past its room
/// rather than vanish.
fn restore(techs: &Techs, stores: &mut [Store], dst: usize, slot: Option<&str>, item: Item) {
    let Err(item) = put(techs, stores, dst, slot, item, true) else {
        return;
    };
    if let Some(Store::Grid(c)) = stores.get_mut(dst) {
        c.unsafe_add(techs, item);
    }
}

#[cfg(test)]
mod tests {
    use super::super::equipment::tests::{table, techs};
    use super::*;

    const PLAYER_SLOTS: [&str; 3] = ["Head", "Tank", "Chip1"];

    fn inventory() -> Inventory {
        Inventory::new((6, 8), &table(), &PLAYER_SLOTS, KnownTech::default())
    }

    #[test]
    fn a_tank_goes_to_its_slot_and_food_never_does() {
        let t = techs();
        let mut inv = inventory();
        let (into, _) = pickup(&mut inv, &t, &[], Item::new(1, 30), false).unwrap();
        assert_eq!(into, PickedInto::Slot("Tank".into()));
        // A second tank: the slot is taken, into the grid (2 × 3).
        let (into, _) = pickup(&mut inv, &t, &[], Item::new(2, 30), false).unwrap();
        assert_eq!(into, PickedInto::Grid);
        // A chip that is eatable (synthetic): never into the slot.
        let mut food = Item::new(3, 31);
        food.eatable = true;
        assert_eq!(
            pickup(&mut inv, &t, &[], food, false).unwrap().0,
            PickedInto::Grid
        );
        assert_eq!(inv.equipment.item_in("Chip1"), None);
        // Nothing equipable: the grid.
        assert_eq!(
            pickup(&mut inv, &t, &[], Item::new(4, 1), false).unwrap().0,
            PickedInto::Grid
        );
        assert!(inv.contains(ItemId(4)));
    }

    #[test]
    fn a_full_grid_refuses_even_an_item_for_a_free_slot() {
        let t = techs();
        let mut inv = inventory();
        for i in 0..48 {
            pickup(&mut inv, &t, &[], Item::new(100 + i, 1), false).unwrap();
        }
        let refused = pickup(&mut inv, &t, &[], Item::new(1, 30), false).unwrap_err();
        assert_eq!(refused.id, ItemId(1));
        assert_eq!(inv.equipment.item_in("Tank"), None);
    }

    #[test]
    fn picking_up_analyses_and_in_a_sub_keeps_on_death() {
        let t = techs();
        let mut inv = inventory();
        let analysis = [AnalysisEntry {
            tech: 1,
            unlocks: vec![500, 501],
        }];
        let (_, unlocked) = pickup(&mut inv, &t, &analysis, Item::new(1, 1), true).unwrap();
        assert_eq!(unlocked, [500, 501]);
        assert!(!inv.container.get(ItemId(1)).unwrap().destroy_on_death);
        let (_, unlocked) = pickup(&mut inv, &t, &analysis, Item::new(2, 1), false).unwrap();
        assert!(unlocked.is_empty());
        assert!(inv.container.get(ItemId(2)).unwrap().destroy_on_death);
    }

    #[test]
    fn swap_between_grid_and_locker() {
        let t = techs();
        let mut inv = inventory();
        let mut locker = Container::new(1, 1);
        inv.container.add(&t, Item::new(1, 1)).unwrap();
        locker.add(&t, Item::new(2, 1)).unwrap();
        {
            let mut s = [Store::Grid(&mut inv.container), Store::Grid(&mut locker)];
            // The locker is full: a plain move is refused…
            assert!(!add_or_swap(&t, &mut s, 0, ItemId(1), 1, &Target::Store));
            // …a swap onto its item works.
            assert!(add_or_swap(
                &t,
                &mut s,
                0,
                ItemId(1),
                1,
                &Target::Item(ItemId(2))
            ));
            // Into the same container: refused.
            assert!(!add_or_swap(&t, &mut s, 1, ItemId(1), 1, &Target::Store));
        }
        assert!(locker.contains(ItemId(1)) && inv.container.contains(ItemId(2)));
        // A 2 × 3 tank cannot swap with the locker's 1 × 1 item: undone.
        inv.container.add(&t, Item::new(3, 30)).unwrap();
        let before = (inv.clone(), locker.clone());
        {
            let mut s = [Store::Grid(&mut inv.container), Store::Grid(&mut locker)];
            assert!(!add_or_swap(
                &t,
                &mut s,
                0,
                ItemId(3),
                1,
                &Target::Item(ItemId(1))
            ));
        }
        assert_eq!((inv, locker), before);
    }

    #[test]
    fn slots_swap_with_the_grid_and_each_other() {
        let t = techs();
        let mut inv = Inventory::new(
            (6, 8),
            &table(),
            &["Chip1", "Chip2", "Tank"],
            KnownTech::default(),
        );
        inv.container.add(&t, Item::new(1, 31)).unwrap();
        inv.container.add(&t, Item::new(2, 32)).unwrap();
        inv.container.add(&t, Item::new(3, 1)).unwrap();
        let mut s = [
            Store::Grid(&mut inv.container),
            Store::Slots(&mut inv.equipment),
        ];
        assert!(add_or_swap(&t, &mut s, 0, ItemId(1), 1, &Target::Store));
        assert!(add_or_swap(&t, &mut s, 0, ItemId(2), 1, &Target::Store));
        // Chip2 onto the taken Chip1: the two swap.
        assert!(add_or_swap(
            &t,
            &mut s,
            1,
            ItemId(2),
            1,
            &Target::Slot("Chip1".into())
        ));
        // Not a chip: no slot of its type.
        assert!(!add_or_swap(&t, &mut s, 0, ItemId(3), 1, &Target::Store));
        assert!(!add_or_swap(
            &t,
            &mut s,
            0,
            ItemId(3),
            1,
            &Target::Slot("Tank".into())
        ));
        // Onto the same slot: nothing.
        assert!(!add_or_swap(
            &t,
            &mut s,
            1,
            ItemId(2),
            1,
            &Target::Slot("Chip1".into())
        ));
        // Back to the grid onto an item: the swap would put item 3 in a
        // chip slot, refused, all undone.
        assert!(!add_or_swap(
            &t,
            &mut s,
            1,
            ItemId(2),
            0,
            &Target::Item(ItemId(3))
        ));
        assert_eq!(inv.equipment.item_in("Chip1").unwrap().id, ItemId(2));
        assert_eq!(inv.equipment.item_in("Chip2").unwrap().id, ItemId(1));
        assert!(inv.container.contains(ItemId(3)));
    }

    #[test]
    fn death_keeps_one_item() {
        let t = techs();
        let mut inv = inventory();
        for i in 1..=4 {
            pickup(&mut inv, &t, &[], Item::new(i, 1), false).unwrap();
        }
        pickup(&mut inv, &t, &[], Item::new(5, 30), false).unwrap();
        let mut kept = Item::new(6, 1);
        kept.destroy_on_death = false;
        pickup(&mut inv, &t, &[], kept, false).unwrap();
        // In the pod nothing is dropped.
        let pod = DropPlace {
            in_escape_pod: true,
            ..Default::default()
        };
        assert!(lose_items(&mut inv, pod).is_empty());
        // Grid items first, then the slots: the tank, last, is kept.
        let lost: Vec<u64> = lose_items(&mut inv, DropPlace::default())
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(lost, [1, 2, 3, 4]);
        assert!(inv.contains(ItemId(5)) && inv.contains(ItemId(6)));
        assert!(lose_items(&mut inv, DropPlace::default()).is_empty());
    }

    #[test]
    fn the_pod_locker_takes_what_fits() {
        let t = techs();
        let mut locker = Container::new(2, 4);
        let items = vec![
            Item::new(1, 30),
            Item::new(2, 1),
            Item::new(3, 1),
            Item::new(4, 30),
        ];
        let left = spawn_supplies(&mut locker, &t, items);
        assert_eq!(left.iter().map(|i| i.id.0).collect::<Vec<_>>(), [4]);
        assert_eq!(locker.len(), 3);
    }

    #[test]
    fn grab_is_true_for_0_4_s() {
        assert!(!grab_active(0.0, 0.1));
        assert!(grab_active(10.0, 10.0));
        assert!(grab_active(10.0, 10.39));
        assert!(!grab_active(10.0, 10.41));
    }

    /// Random drags between a grid, a locker and the slots never lose or
    /// copy an item.
    #[test]
    fn random_moves_keep_every_item() {
        let t = techs();
        let mut inv = Inventory::new(
            (3, 4),
            &table(),
            &["Head", "Tank", "Chip1", "Chip2"],
            KnownTech::default(),
        );
        let mut locker = Container::new(2, 3);
        let kinds = [1, 30, 31, 32, 34, 1, 1];
        let mut all = Vec::new();
        for (i, &tech) in kinds.iter().enumerate() {
            let item = Item::new(i as u64, tech);
            all.push(item.id);
            if pickup(&mut inv, &t, &[], item.clone(), false).is_err() {
                locker.add(&t, item).unwrap();
            }
        }
        let mut s: u64 = 12345;
        let mut next = |n: usize| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as usize) % n
        };
        let slot_names = ["Head", "Tank", "Chip1", "Chip2", "Nope"];
        let mut moved = 0;
        for _ in 0..20_000 {
            let (from, to) = (next(3), next(3));
            let item = all[next(all.len())];
            let target = match next(3) {
                0 => Target::Store,
                1 => Target::Item(all[next(all.len())]),
                _ => Target::Slot(slot_names[next(slot_names.len())].into()),
            };
            let mut stores = [
                Store::Grid(&mut inv.container),
                Store::Slots(&mut inv.equipment),
                Store::Grid(&mut locker),
            ];
            moved += usize::from(add_or_swap(&t, &mut stores, from, item, to, &target));
            let mut ids: Vec<ItemId> = inv
                .container
                .items()
                .chain(inv.equipment.items())
                .chain(locker.items())
                .map(|i| i.id)
                .collect();
            ids.sort();
            assert_eq!(ids, all);
            assert!(inv.container.layout().1 && locker.layout().1, "over-full");
        }
        assert!(moved > 1000, "{moved}");
    }
}
