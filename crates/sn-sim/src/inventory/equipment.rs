//! `Equipment`: named slots, each taking items of one `EquipmentType`.

use super::{EQUIPMENT_NONE, Item, ItemId, TechType, Techs};

/// `Equipment.slotMapping` (slot name → `EquipmentType`, read from the
/// game's code) and the three `EquipmentType` values
/// `Equipment.IsCompatible` names: a `VehicleModule` item also fits
/// `SeamothModule` and `ExosuitModule` slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotTable {
    pub slots: Vec<(String, i32)>,
    pub vehicle_module: i32,
    pub seamoth_module: i32,
    pub exosuit_module: i32,
}

impl SlotTable {
    /// `Equipment.GetSlotType`: `None` for an unknown slot.
    pub fn slot_type(&self, slot: &str) -> i32 {
        self.slots
            .iter()
            .find(|(n, _)| n == slot)
            .map_or(EQUIPMENT_NONE, |&(_, t)| t)
    }

    /// `Equipment.IsCompatible`.
    pub fn is_compatible(&self, item_type: i32, slot_type: i32) -> bool {
        item_type == slot_type
            || (item_type == self.vehicle_module
                && (slot_type == self.seamoth_module || slot_type == self.exosuit_module))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Slot {
    name: String,
    kind: i32,
    item: Option<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Equipment {
    table: SlotTable,
    /// `equipment`, in the order the slots were added (the player's are
    /// never removed, so this is the dictionary's order).
    slots: Vec<Slot>,
    /// `typeToSlots`: each type in the order it first appeared, with its
    /// slots in order.
    by_type: Vec<(i32, Vec<usize>)>,
}

impl Equipment {
    /// `new Equipment(…)` then `AddSlots(names)`: names without a type
    /// (`AddSlot` logs an error) and repeats are skipped.
    pub fn new<S: AsRef<str>>(table: &SlotTable, names: &[S]) -> Equipment {
        let mut e = Equipment {
            table: table.clone(),
            slots: Vec::new(),
            by_type: Vec::new(),
        };
        for name in names {
            let name = name.as_ref();
            let kind = table.slot_type(name);
            if kind == EQUIPMENT_NONE || e.index(name).is_some() {
                continue;
            }
            let i = e.slots.len();
            e.slots.push(Slot {
                name: name.to_owned(),
                kind,
                item: None,
            });
            match e.by_type.iter_mut().find(|(t, _)| *t == kind) {
                Some((_, list)) => list.push(i),
                None => e.by_type.push((kind, vec![i])),
            }
        }
        e
    }

    pub fn table(&self) -> &SlotTable {
        &self.table
    }

    fn index(&self, slot: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.name == slot)
    }

    /// The slots in order, with what each holds.
    pub fn slots(&self) -> impl Iterator<Item = (&str, Option<&Item>)> {
        self.slots
            .iter()
            .map(|s| (s.name.as_str(), s.item.as_ref()))
    }

    /// The items, in slot order (the game's enumeration).
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.slots.iter().filter_map(|s| s.item.as_ref())
    }

    pub fn has_slot(&self, slot: &str) -> bool {
        self.index(slot).is_some()
    }

    pub fn item_in(&self, slot: &str) -> Option<&Item> {
        self.slots.get(self.index(slot)?)?.item.as_ref()
    }

    /// `GetItemSlot`.
    pub fn slot_of(&self, id: ItemId) -> Option<&str> {
        self.slots
            .iter()
            .find(|s| s.item.as_ref().is_some_and(|i| i.id == id))
            .map(|s| s.name.as_str())
    }

    /// The compatible slots, type by type in `typeToSlots`' order.
    fn compatible(&self, item_type: i32) -> impl Iterator<Item = &[usize]> {
        self.by_type
            .iter()
            .filter(move |(t, _)| self.table.is_compatible(item_type, *t))
            .map(|(_, list)| list.as_slice())
    }

    /// `GetFreeSlot`: the first compatible slot that is empty (or holds
    /// `ignore`).
    pub fn free_slot(&self, item_type: i32, ignore: Option<ItemId>) -> Option<&str> {
        self.compatible(item_type)
            .flatten()
            .filter_map(|&i| self.slots.get(i))
            .find(|s| s.item.as_ref().is_none_or(|it| Some(it.id) == ignore))
            .map(|s| s.name.as_str())
    }

    /// `GetCompatibleSlotDefault`: the first empty compatible slot, else
    /// the first slot of the first compatible type.
    pub fn compatible_slot(&self, item_type: i32) -> Option<&str> {
        let mut first = None;
        for list in self.compatible(item_type) {
            for s in list.iter().filter_map(|&i| self.slots.get(i)) {
                if s.item.is_none() {
                    return Some(&s.name);
                }
            }
            if first.is_none() {
                first = list.first().and_then(|&i| self.slots.get(i));
            }
        }
        first.map(|s| s.name.as_str())
    }

    /// `IItemsContainer.HasRoomFor`: a free compatible slot.
    pub fn has_room_for(&self, techs: &Techs, tech: TechType, ignore: Option<ItemId>) -> bool {
        self.free_slot(techs.equipment_type(tech), ignore).is_some()
    }

    /// `AddItem(slot, item, forced)` for an item in no container: the
    /// slot must exist and be empty, and unless `forced` take the item's
    /// type.
    pub fn add_to(
        &mut self,
        techs: &Techs,
        slot: &str,
        item: Item,
        forced: bool,
    ) -> Result<(), Item> {
        let Some(i) = self.index(slot) else {
            return Err(item);
        };
        let kind = self.slots[i].kind;
        if self.slots[i].item.is_some()
            || (!forced
                && !self
                    .table
                    .is_compatible(techs.equipment_type(item.tech), kind))
        {
            return Err(item);
        }
        self.slots[i].item = Some(item);
        Ok(())
    }

    /// `IItemsContainer.AddItem`: into the first free compatible slot,
    /// whose name is returned.
    pub fn add(&mut self, techs: &Techs, item: Item) -> Result<String, Item> {
        let Some(slot) = self.free_slot(techs.equipment_type(item.tech), None) else {
            return Err(item);
        };
        let slot = slot.to_owned();
        self.add_to(techs, &slot, item, false)?;
        Ok(slot)
    }

    /// `RemoveItem(slot)`.
    pub fn remove_slot(&mut self, slot: &str) -> Option<Item> {
        let i = self.index(slot)?;
        self.slots[i].item.take()
    }

    /// `RemoveItem(Pickupable)`.
    pub fn remove(&mut self, id: ItemId) -> Option<Item> {
        let slot = self.slot_of(id)?.to_owned();
        self.remove_slot(&slot)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Synthetic types: 1 head, 2 tank, 3 chip, 9 vehicle module,
    // 13 seamoth module.
    pub(crate) fn table() -> SlotTable {
        SlotTable {
            slots: [
                ("Head", 1),
                ("Tank", 2),
                ("Chip1", 3),
                ("Chip2", 3),
                ("SeamothModule1", 13),
            ]
            .iter()
            .map(|&(n, t)| (n.to_owned(), t))
            .collect(),
            vehicle_module: 9,
            seamoth_module: 13,
            exosuit_module: 14,
        }
    }

    /// Tech 30: a tank; 31, 32: chips; 33: a vehicle module; 34: a
    /// helmet; 1: nothing.
    pub(crate) fn techs() -> Techs {
        let mut t = Techs::default();
        for (tech, ty) in [(30, 2), (31, 3), (32, 3), (33, 9), (34, 1)] {
            t.equipment.insert(tech, ty);
        }
        t.sizes.insert(30, (2, 3));
        t
    }

    #[test]
    fn slots_without_a_type_or_repeated_are_skipped() {
        let e = Equipment::new(&table(), &["Head", "Nope", "Chip1", "Head", "Chip2"]);
        assert_eq!(
            e.slots().map(|(n, _)| n).collect::<Vec<_>>(),
            ["Head", "Chip1", "Chip2"]
        );
    }

    #[test]
    fn items_go_to_a_free_slot_of_their_type() {
        let t = techs();
        let mut e = Equipment::new(&table(), &["Head", "Tank", "Chip1", "Chip2"]);
        e.add(&t, Item::new(1, 30)).unwrap();
        assert_eq!(e.slot_of(ItemId(1)), Some("Tank"));
        e.add(&t, Item::new(2, 31)).unwrap();
        e.add(&t, Item::new(3, 32)).unwrap();
        assert_eq!(e.slot_of(ItemId(3)), Some("Chip2"));
        // Full, or no slot of the type at all.
        assert!(e.add(&t, Item::new(4, 31)).is_err());
        assert!(e.add(&t, Item::new(5, 1)).is_err());
        assert!(e.add(&t, Item::new(6, 33)).is_err());
        assert!(e.has_room_for(&t, 31, Some(ItemId(2))));
        assert_eq!(e.compatible_slot(3), Some("Chip1"));
        assert_eq!(e.remove(ItemId(2)).unwrap().tech, 31);
        assert_eq!(e.compatible_slot(3), Some("Chip1"));
    }

    #[test]
    fn a_vehicle_module_fits_a_seamoth_slot() {
        let t = techs();
        let mut e = Equipment::new(&table(), &["SeamothModule1"]);
        assert!(
            e.add_to(&t, "SeamothModule1", Item::new(1, 34), false)
                .is_err()
        );
        e.add_to(&t, "SeamothModule1", Item::new(2, 33), false)
            .unwrap();
        assert!(
            e.add_to(&t, "SeamothModule1", Item::new(3, 33), false)
                .is_err()
        );
        assert!(e.add_to(&t, "Nope", Item::new(4, 33), true).is_err());
    }
}
