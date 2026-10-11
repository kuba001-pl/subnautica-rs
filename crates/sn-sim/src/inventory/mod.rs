//! Items, the inventory and what moves them (M9d2, `docs/DESIGN.md` "M9d
//! plan"): `ItemsContainer` (groups, the sort order, the packing,
//! `HasRoomFor`), `Equipment` (named slots by `EquipmentType`),
//! `Inventory.Pickup`, `Inventory.AddOrSwap`, `KnownTech.Analyze`,
//! `FairRandomizer` and `PlayerEntropy`, `BreakableResource` (the
//! outcrops), `Inventory.CanDropItemHere` / `RayCast` /
//! `Pickupable.FindDropPosition`, `Inventory.LoseItems` and the `grab`
//! animator bool. Our own code after reading the game's; the data (item
//! sizes, equipment types, slots, analysis lists, outcrop lists) comes
//! from the install through the caller, the constants inside the game's
//! methods are ported here and named after their method.
//!
//! Random draws are parameters (`value` in [0, 1], as Unity's
//! `Random.value`): the game's `Random` is unseeded, so any draw is the
//! game's behaviour, and the server feeds its own seeded random.
//!
//! Item ids are the caller's: the server gives each item one, the rules
//! only keep them apart.

mod container;
mod drop;
mod equipment;
mod known;
mod luck;
mod moves;
mod rules;

pub use container::{Container, ContainerParts, GroupParts, MAX_SIDE, Placed};
pub use drop::{DropPlace, can_drop_here, drop_ray, find_drop_position};
pub use equipment::{Equipment, SlotTable};
pub use known::{AnalysisEntry, KnownTech};
pub use luck::{FairRandomizer, HitKind, Outcrop, PlayerEntropy, Resource, draw_resources};
pub use moves::{
    GRAB_TIME, Inventory, PickedInto, Store, Target, add_or_swap, grab_active, lose_items, pickup,
    spawn_supplies,
};
pub use rules::{ItemRules, OutcropDrop, OutcropInfo, PickupInfo};

use std::collections::BTreeMap;

/// The game's `TechType` number.
pub type TechType = i32;

/// `TechType.None`.
pub const TECH_NONE: TechType = 0;

/// `EquipmentType.None`: an item with it goes into no slot.
pub const EQUIPMENT_NONE: i32 = 0;

/// One item, wherever it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u64);

/// An item in a container, a slot or the hand: what the rules need of its
/// `Pickupable`.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: ItemId,
    pub tech: TechType,
    /// `Pickupable.destroyOnDeath`: lost (dropped) on death.
    pub destroy_on_death: bool,
    /// Has an `Eatable`: a pick-up never goes into the equipment.
    pub eatable: bool,
    /// `IBattery.charge`, for the items that are batteries
    /// (`ItemsContainer.RemoveItem(TechType)` takes the emptiest).
    pub charge: Option<f32>,
}

impl Item {
    pub fn new(id: u64, tech: TechType) -> Item {
        Item {
            id: ItemId(id),
            tech,
            destroy_on_death: true,
            eatable: false,
            charge: None,
        }
    }
}

/// What the rules read of TechData: each tech type's `itemSize` and
/// `equipmentType`, and the defaults (`TechData.defaults`) for the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Techs {
    pub sizes: BTreeMap<TechType, (i32, i32)>,
    pub equipment: BTreeMap<TechType, i32>,
    pub default_size: (i32, i32),
    pub default_equipment: i32,
}

impl Default for Techs {
    fn default() -> Techs {
        Techs {
            sizes: BTreeMap::new(),
            equipment: BTreeMap::new(),
            default_size: (1, 1),
            default_equipment: EQUIPMENT_NONE,
        }
    }
}

impl Techs {
    /// `TechData.GetItemSize`.
    pub fn size(&self, tech: TechType) -> (i32, i32) {
        self.sizes.get(&tech).copied().unwrap_or(self.default_size)
    }

    /// `TechData.GetEquipmentType`.
    pub fn equipment_type(&self, tech: TechType) -> i32 {
        self.equipment
            .get(&tech)
            .copied()
            .unwrap_or(self.default_equipment)
    }
}

/// A .NET `Dictionary`'s enumeration order: entries in their slots, a
/// removed entry's slot reused by the next insert (the runtime's free
/// list is last in, first out). The game walks its containers in this
/// order (`LoseItems` keeps the last item it meets).
#[derive(Clone, Debug, PartialEq)]
struct DictOrder<T> {
    entries: Vec<Option<T>>,
    free: Vec<usize>,
}

impl<T> Default for DictOrder<T> {
    fn default() -> DictOrder<T> {
        DictOrder {
            entries: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> DictOrder<T> {
    fn insert(&mut self, value: T) -> usize {
        match self.free.pop() {
            Some(i) => {
                self.entries[i] = Some(value);
                i
            }
            None => {
                self.entries.push(Some(value));
                self.entries.len() - 1
            }
        }
    }

    fn remove(&mut self, i: usize) -> Option<T> {
        let v = self.entries.get_mut(i)?.take()?;
        self.free.push(i);
        Some(v)
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().flatten()
    }

    fn position(&self, f: impl Fn(&T) -> bool) -> Option<usize> {
        self.entries.iter().position(|e| e.as_ref().is_some_and(&f))
    }

    fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        self.entries.get_mut(i)?.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_order_reuses_the_last_freed_slot() {
        let mut d = DictOrder::default();
        for v in [1, 2, 3, 4] {
            d.insert(v);
        }
        d.remove(1);
        d.remove(2);
        d.insert(5);
        d.insert(6);
        d.insert(7);
        assert_eq!(d.iter().copied().collect::<Vec<_>>(), [1, 6, 5, 4, 7]);
    }
}
