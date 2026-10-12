//! What a click on an item in the inventory does (M9d5b):
//! `Inventory.GetAllItemActions`, `GetItemAction` and
//! `GetOppositeContainer`. `ExecuteItemAction` runs `AddOrSwap(item,
//! opposite)` for equip, unequip, swap and switch ([`super::add_or_swap`]
//! with [`super::Target::Store`]) and `InternalDropItem` for a drop; use,
//! eat and assign are not ported.

use super::{Container, DropPlace, Inventory, ItemId, TechType, Techs, can_drop_here};

/// `ItemAction` (a flag set).
pub const USE: u32 = 1;
pub const EAT: u32 = 2;
pub const EQUIP: u32 = 4;
pub const UNEQUIP: u32 = 8;
pub const ASSIGN: u32 = 16;
pub const SWITCH: u32 = 32;
pub const SWAP: u32 = 64;
pub const DROP: u32 = 128;

/// The tech types `GetAllItemActions` names: `FirstAidKit` is used, and
/// `Bladderfish` eaten even without survival.
pub const FIRST_AID_KIT: TechType = 4514;

/// Where an item is, or where a click sends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The player's grid (`Inventory._container`).
    Grid,
    /// The player's equipment.
    Slots,
    /// A storage the player has open (`usedStorage`; ours: the
    /// lifepod's locker).
    Storage,
}

/// `GetOppositeContainer`: from the grid, the storage in use (the first
/// with room, else the first; we have at most one) or else the
/// equipment; from anywhere else, the grid.
pub fn opposite(from: Place, storage_open: bool) -> Place {
    match from {
        Place::Grid if storage_open => Place::Storage,
        Place::Grid => Place::Slots,
        Place::Slots | Place::Storage => Place::Grid,
    }
}

/// Where `item` is: the player's grid, slots, or `storage`.
pub fn place_of(inv: &Inventory, storage: Option<&Container>, item: ItemId) -> Option<Place> {
    if inv.container.contains(item) {
        Some(Place::Grid)
    } else if inv.equipment.slot_of(item).is_some() {
        Some(Place::Slots)
    } else if storage.is_some_and(|s| s.contains(item)) {
        Some(Place::Storage)
    } else {
        None
    }
}

/// `GetAllItemActions` for an item of the player's grid or slots or of
/// the open `storage` (none: 0). In the game's survival mode (no
/// `NoSurvival` option), an eatable item in the grid with no storage open
/// can be eaten. Not ported: `Assign` (quick slots), `CanDrag`'s refusals
/// (no container of ours refuses a removal), a storage's tech filter
/// (`AllowedToAdd`; the lifepod's locker has none).
pub fn item_actions(
    techs: &Techs,
    inv: &Inventory,
    storage: Option<&Container>,
    item: ItemId,
    eatable: bool,
    place: DropPlace,
) -> u32 {
    let Some(from) = place_of(inv, storage, item) else {
        return 0;
    };
    let tech = match from {
        Place::Grid => inv.container.get(item).map(|i| i.tech),
        Place::Slots => inv.equipment.items().find(|i| i.id == item).map(|i| i.tech),
        Place::Storage => storage.and_then(|s| s.get(item)).map(|i| i.tech),
    };
    let Some(tech) = tech else { return 0 };
    let to = opposite(from, storage.is_some());
    let mut actions = 0;
    if from == Place::Grid {
        if to == Place::Slots {
            if eatable {
                actions |= EAT;
            }
            if tech == FIRST_AID_KIT {
                actions |= USE;
            }
        }
        if can_drop_here(place) {
            actions |= DROP;
        }
    }
    match (from, to) {
        (Place::Slots, _) => {
            if inv.container.has_room_for(techs, tech) {
                actions |= UNEQUIP;
            }
        }
        (_, Place::Slots) => {
            let kind = techs.equipment_type(tech);
            if inv.equipment.free_slot(kind, None).is_some() {
                actions |= EQUIP;
            } else if inv.equipment.compatible_slot(kind).is_some() {
                actions |= SWAP;
            }
        }
        // Into a container: `AllowedToAdd` asks its filter, not its room.
        _ => actions |= SWITCH,
    }
    actions
}

/// A mouse button, as `uGUI_ItemIcon` numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// `GetItemAction(item, button)` with the keyboard and mouse: left takes
/// the first of use, eat, equip, unequip, swap, switch; right drops;
/// middle does nothing. 0: no action.
pub fn item_action(actions: u32, button: Button) -> u32 {
    match button {
        Button::Left => [USE, EAT, EQUIP, UNEQUIP, SWAP, SWITCH]
            .into_iter()
            .find(|&a| actions & a != 0)
            .unwrap_or(0),
        Button::Right => actions & DROP,
        Button::Middle => 0,
    }
}

/// An action's name, for logs.
pub fn action_name(action: u32) -> &'static str {
    match action {
        USE => "use",
        EAT => "eat",
        EQUIP => "equip",
        UNEQUIP => "unequip",
        ASSIGN => "assign",
        SWITCH => "switch",
        SWAP => "swap",
        DROP => "drop",
        _ => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Item, KnownTech, SlotTable};
    use super::*;

    const TANK: TechType = 10;
    const ORE: TechType = 11;
    const FISH: TechType = 12;
    /// Equipment type of the tank and its slot.
    const TANK_TYPE: i32 = 3;

    fn techs() -> Techs {
        let mut t = Techs::default();
        t.equipment.insert(TANK, TANK_TYPE);
        t
    }

    fn inventory() -> Inventory {
        let table = SlotTable {
            slots: vec![("Tank".to_string(), TANK_TYPE)],
            vehicle_module: -1,
            seamoth_module: -2,
            exosuit_module: -3,
        };
        Inventory::new((2, 1), &table, &["Tank"], KnownTech::default())
    }

    fn put(inv: &mut Inventory, id: u64, tech: TechType) {
        inv.container.unsafe_add(&techs(), Item::new(id, tech));
    }

    #[test]
    fn a_click_does_what_the_game_does_in_each_store() {
        let t = techs();
        let out = DropPlace::default();
        let pod = DropPlace {
            in_escape_pod: true,
            ..Default::default()
        };
        let mut inv = inventory();
        put(&mut inv, 1, TANK);
        put(&mut inv, 2, ORE);

        // A tank in the grid, nothing open: equip (a free slot), drop.
        let a = item_actions(&t, &inv, None, ItemId(1), false, out);
        assert_eq!(a, EQUIP | DROP);
        assert_eq!(item_action(a, Button::Left), EQUIP);
        assert_eq!(item_action(a, Button::Right), DROP);
        assert_eq!(item_action(a, Button::Middle), 0);
        // In the lifepod: no drop.
        let a = item_actions(&t, &inv, None, ItemId(1), false, pod);
        assert_eq!(a, EQUIP);
        assert_eq!(item_action(a, Button::Right), 0);
        // Ore fits no slot: only the drop.
        let a = item_actions(&t, &inv, None, ItemId(2), false, out);
        assert_eq!(a, DROP);
        assert_eq!(item_action(a, Button::Left), 0);

        // The slot taken: a second tank swaps.
        let tank = inv.container.remove(ItemId(1)).unwrap();
        inv.equipment.add(&t, tank).unwrap();
        put(&mut inv, 3, TANK);
        let a = item_actions(&t, &inv, None, ItemId(3), false, out);
        assert_eq!(item_action(a, Button::Left), SWAP);
        // The equipped tank unequips while the grid has room, and has no
        // drop of its own.
        let a = item_actions(&t, &inv, None, ItemId(1), false, out);
        assert_eq!(a, 0, "the 2 × 1 grid is full");
        inv.container.remove(ItemId(2));
        let a = item_actions(&t, &inv, None, ItemId(1), false, out);
        assert_eq!(a, UNEQUIP);

        // A storage open: from the grid everything switches there (room
        // not asked), and nothing is eaten; from the storage, back.
        let mut locker = Container::new(1, 1);
        locker.unsafe_add(&t, Item::new(4, ORE));
        let a = item_actions(&t, &inv, Some(&locker), ItemId(3), false, pod);
        assert_eq!(a, SWITCH);
        let a = item_actions(&t, &inv, Some(&locker), ItemId(4), false, pod);
        assert_eq!((a, item_action(a, Button::Left)), (SWITCH, SWITCH));
        assert_eq!(
            item_actions(&t, &inv, Some(&locker), ItemId(9), false, pod),
            0
        );

        // Eatable, nothing open: eat comes before everything else; the
        // first aid kit is used.
        put(&mut inv, 5, FISH);
        let a = item_actions(&t, &inv, None, ItemId(5), true, out);
        assert_eq!((a, item_action(a, Button::Left)), (EAT | DROP, EAT));
        let mut inv = inventory();
        put(&mut inv, 6, FIRST_AID_KIT);
        let a = item_actions(&t, &inv, None, ItemId(6), false, out);
        assert_eq!(item_action(a, Button::Left), USE);
        let a = item_actions(&t, &inv, Some(&locker), ItemId(6), false, out);
        assert_eq!(a, SWITCH | DROP);
    }

    #[test]
    fn the_opposite_store() {
        assert_eq!(opposite(Place::Grid, false), Place::Slots);
        assert_eq!(opposite(Place::Grid, true), Place::Storage);
        assert_eq!(opposite(Place::Slots, true), Place::Grid);
        assert_eq!(opposite(Place::Storage, true), Place::Grid);
    }
}
