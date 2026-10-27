//! Eating, drinking and first aid (M9s2): `Survival.Eat`, `Survival.Use`
//! (the first aid kit), `Eatable.GetFoodValue`/`GetWaterValue`,
//! `LiveMixin.AddHealth`, `Inventory.ExecuteItemAction`'s Eat and Use
//! (the exact item leaves the grid and is destroyed only on success), and
//! a new object's component state (`Eatable.Awake`/`SetDecomposes`).

use super::{HungerRules, RuleError, Survival, finite, nonnegative};
use crate::inventory::{EnergySlot, Inventory, Item, ItemId, ItemState, TechType};

/// The numbers and tech types of `Survival.Eat`/`Use` and `Eatable`'s
/// value getters, from the install (no defaults here).
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumeRules {
    /// Food rises only while it is at most this (`food <= 99f`).
    pub food_gate: f64,
    /// `vitalsOkNotification` plays when a stat rises over this.
    pub vitals_ok: f64,
    /// `TechType.FirstAidKit`, and the health it asks for.
    pub first_aid: TechType,
    pub first_aid_health: f64,
    /// A kit is used up only if it healed strictly more than this.
    pub heal_success: f64,
    /// `TechType.Bladderfish`, and the oxygen eating it adds.
    pub oxygen_food: TechType,
    pub oxygen_on_eat: f64,
    /// `Eatable`'s floor of a decomposing item's values.
    pub decay_floor: f64,
}

impl ConsumeRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        if nonnegative(&[
            self.food_gate,
            self.vitals_ok,
            self.first_aid_health,
            self.heal_success,
            self.oxygen_on_eat,
        ]) && finite(&[self.decay_floor])
            && self.decay_floor <= 0.0
        {
            Ok(())
        } else {
            Err(RuleError("invalid consume rules"))
        }
    }
}

/// An `Eatable` prefab's serialized fields (per prefab, from the install).
#[derive(Clone, Debug, PartialEq)]
pub struct EatableInfo {
    pub food: f64,
    pub water: f64,
    pub decomposes: bool,
    /// `kDecayRate`: value lost per game second while decomposing.
    pub decay_rate: f64,
    pub despawns: bool,
    /// `despawnDelay`: active rotten seconds before it is destroyed.
    pub despawn_delay: f64,
}

impl EatableInfo {
    pub fn validate(&self) -> Result<(), RuleError> {
        if finite(&[self.food, self.water]) && nonnegative(&[self.decay_rate, self.despawn_delay]) {
            Ok(())
        } else {
            Err(RuleError("invalid Eatable"))
        }
    }

    /// `GetFoodValue`, `GetWaterValue` at game time `now` for an item
    /// whose `timeDecayStart` is `decay_start`: each value minus the
    /// decay since then, each floored at `floor`; the stored values when
    /// it does not decompose.
    pub fn values(&self, decay_start: f64, now: f64, floor: f64) -> (f64, f64) {
        if !self.decomposes {
            return (self.food, self.water);
        }
        let lost = (now - decay_start) * self.decay_rate;
        (
            (self.food - lost).max(floor),
            (self.water - lost).max(floor),
        )
    }
}

/// What a successful `Survival.Eat` asks of the rest of the game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Eaten {
    /// `onEat`/`onDrink` with the item's value (also when a cap or the
    /// food gate kept the stat from rising).
    pub on_eat: Option<f64>,
    pub on_drink: Option<f64>,
    /// The `Eat_Something`/`Drink_Something` goals (a positive value).
    pub eat_goal: bool,
    pub drink_goal: bool,
    /// `vitalsOkNotification`.
    pub vitals_ok: bool,
    /// Oxygen for `OxygenManager.AddOxygen` (the oxygen food's).
    pub oxygen: f64,
}

/// `Survival.Eat` with the item's current values: `None` when both are
/// zero (nothing happens and the item stays).
pub fn eat(
    s: &mut Survival,
    h: &HungerRules,
    c: &ConsumeRules,
    tech: TechType,
    food: f64,
    water: f64,
) -> Result<Option<Eaten>, RuleError> {
    h.validate()?;
    c.validate()?;
    if !finite(&[food, water, s.food, s.water]) {
        return Err(RuleError("invalid eating values"));
    }
    let mut out = Eaten::default();
    let mut eaten = false;
    if food != 0.0 {
        if s.food <= c.food_gate {
            s.food = (s.food + food).clamp(0.0, h.food_max);
        }
        out.on_eat = Some(food);
        out.eat_goal = food > 0.0;
        eaten = true;
    }
    if water != 0.0 {
        s.water = (s.water + water).clamp(0.0, h.stat_max);
        out.on_drink = Some(water);
        out.drink_goal = water > 0.0;
        eaten = true;
    }
    // The notification compares the new stat less the item's value, not
    // the stat before (they differ when clamped or gated).
    out.vitals_ok = (s.food > c.vitals_ok && s.food - food < c.vitals_ok)
        || (s.water > c.vitals_ok && s.water - water < c.vitals_ok);
    if !eaten {
        return Ok(None);
    }
    if tech == c.oxygen_food {
        out.oxygen = c.oxygen_on_eat;
    }
    Ok(Some(out))
}

/// `LiveMixin.AddHealth` for a player (no resurrection): alive and below
/// the maximum, health rises by up to `amount`. Returns what it rose by.
pub fn add_health(health: &mut f64, max_health: f64, amount: f64) -> f64 {
    if *health > 0.0 && *health < max_health {
        let before = *health;
        *health = (*health + amount).min(max_health);
        *health - before
    } else {
        0.0
    }
}

/// What `Survival.Use` did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Used {
    /// Healed by this much; the kit is used up.
    Healed(f64),
    /// Healed by this much (at most the threshold, maybe 0): the
    /// `HealthFull` message, and the kit stays.
    Full(f64),
}

/// `Survival.Use`: `None` for a tech type it does not use (the cure ball
/// is unreachable in the game and not ported).
pub fn use_tech(
    c: &ConsumeRules,
    tech: TechType,
    health: &mut f64,
    max_health: f64,
) -> Result<Option<Used>, RuleError> {
    c.validate()?;
    if !finite(&[*health, max_health]) {
        return Err(RuleError("invalid health"));
    }
    if tech != c.first_aid {
        return Ok(None);
    }
    let healed = add_health(health, max_health, c.first_aid_health);
    Ok(Some(if healed > c.heal_success {
        Used::Healed(healed)
    } else {
        Used::Full(healed)
    }))
}

/// Why an Eat or Use on an inventory item did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not in the player's grid (the only place these actions apply).
    NotInGrid,
    /// No `Eatable` rules for its tech type, or no decay state.
    NotEatable,
}

/// `ExecuteItemAction(Eat)` on the grid item `id` at game time `now`:
/// on success the item leaves the grid and is returned (to be
/// destroyed) with what eating it asks; `Ok(Ok(None))` when both values
/// are zero (the item stays).
pub fn eat_item(
    inv: &mut Inventory,
    id: ItemId,
    s: &mut Survival,
    h: &HungerRules,
    c: &ConsumeRules,
    info: impl Fn(TechType) -> Option<EatableInfo>,
    now: f64,
) -> Result<Result<Option<(Item, Eaten)>, Refused>, RuleError> {
    let Some(item) = inv.container.get(id) else {
        return Ok(Err(Refused::NotInGrid));
    };
    let (Some(e), Some(start)) = (info(item.tech), item.state.decay_start) else {
        return Ok(Err(Refused::NotEatable));
    };
    e.validate()?;
    if !finite(&[now, start]) {
        return Err(RuleError("invalid game time"));
    }
    let (food, water) = e.values(start, now, c.decay_floor);
    let tech = item.tech;
    match eat(s, h, c, tech, food, water)? {
        Some(eaten) => {
            let item = inv
                .container
                .remove(id)
                .ok_or(RuleError("eaten item vanished"))?;
            Ok(Ok(Some((item, eaten))))
        }
        None => Ok(Ok(None)),
    }
}

/// What [`use_item`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum UsedItem {
    /// Healed by this much; the kit left the grid and is returned (to be
    /// destroyed).
    Healed(Item, f64),
    /// Healed by this much (at most the threshold, maybe 0): the
    /// `HealthFull` message, and the kit stays.
    Full(f64),
}

/// `ExecuteItemAction(Use)` on the grid item `id`: the kit leaves the
/// grid only when it healed enough.
pub fn use_item(
    inv: &mut Inventory,
    id: ItemId,
    c: &ConsumeRules,
    health: &mut f64,
    max_health: f64,
) -> Result<Result<Option<UsedItem>, Refused>, RuleError> {
    let Some(item) = inv.container.get(id) else {
        return Ok(Err(Refused::NotInGrid));
    };
    match use_tech(c, item.tech, health, max_health)? {
        Some(Used::Healed(healed)) => {
            let item = inv
                .container
                .remove(id)
                .ok_or(RuleError("used item vanished"))?;
            Ok(Ok(Some(UsedItem::Healed(item, healed))))
        }
        Some(Used::Full(healed)) => Ok(Ok(Some(UsedItem::Full(healed)))),
        None => Ok(Ok(None)),
    }
}

/// A newly made object's component state: `Eatable.Awake` →
/// `SetDecomposes(decomposes)` (a fresh object's `timeDecayStart` is 0,
/// so a decomposing one starts decaying `now`); an `Oxygen` with
/// `oxygen` available (where that number comes from is an open question,
/// `docs/formats/gameplay.md` § Tanks); an `EnergyMixin`'s empty slot
/// (its default battery is M9s3).
pub fn initial_state(
    eatable: Option<&EatableInfo>,
    oxygen: Option<f64>,
    energy: bool,
    now: f64,
) -> ItemState {
    ItemState {
        decay_start: eatable.map(|e| if e.decomposes { now } else { 0.0 }),
        oxygen,
        energy: energy.then(EnergySlot::default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{
        Container, DropPlace, KnownTech, SlotTable, Store, Target, Techs, add_or_swap, lose_items,
        pickup,
    };

    /// Synthetic: equipment type 2 goes into the "Tank" slot; tech 30 is
    /// of that type and 2×3 big.
    fn table() -> SlotTable {
        SlotTable {
            slots: vec![("Tank".to_owned(), 2)],
            vehicle_module: 9,
            seamoth_module: 13,
            exosuit_module: 14,
        }
    }

    fn techs() -> Techs {
        let mut t = Techs::default();
        t.equipment.insert(30, 2);
        t.sizes.insert(30, (2, 3));
        t
    }
    use crate::survival::hunger::tests::rules;

    /// Synthetic: tech 50 a kit, 51 an oxygen fish, 52 a food.
    fn consume() -> ConsumeRules {
        ConsumeRules {
            food_gate: 99.0,
            vitals_ok: 20.0,
            first_aid: 50,
            first_aid_health: 50.0,
            heal_success: 0.1,
            oxygen_food: 51,
            oxygen_on_eat: 15.0,
            decay_floor: -25.0,
        }
    }

    fn at(food: f64, water: f64) -> Survival {
        Survival {
            food,
            water,
            phase: 0.0,
        }
    }

    fn eat_at(s: &mut Survival, tech: TechType, food: f64, water: f64) -> Option<Eaten> {
        eat(s, &rules(), &consume(), tech, food, water).unwrap()
    }

    #[test]
    fn food_rises_only_at_or_below_the_gate() {
        let mut s = at(99.0, 50.0);
        let e = eat_at(&mut s, 52, 10.0, 0.0).unwrap();
        assert_eq!(s.food, 109.0);
        assert_eq!(e.on_eat, Some(10.0));
        let mut s = at(99.5, 50.0);
        let e = eat_at(&mut s, 52, 10.0, 0.0).unwrap();
        // Over the gate: still eaten, the event still fires.
        assert_eq!(s.food, 99.5);
        assert_eq!(e.on_eat, Some(10.0));
        assert!(e.eat_goal);
    }

    #[test]
    fn caps_negative_and_zero_values() {
        let mut s = at(95.0, 95.0);
        let e = eat_at(&mut s, 52, 500.0, 500.0).unwrap();
        assert_eq!((s.food, s.water), (200.0, 100.0));
        assert_eq!((e.on_eat, e.on_drink), (Some(500.0), Some(500.0)));
        // A full player still eats (and the item is still used up).
        let mut s = at(200.0, 100.0);
        assert!(eat_at(&mut s, 52, 5.0, 5.0).is_some());
        assert_eq!((s.food, s.water), (200.0, 100.0));
        // Rotten values lower the stats, to 0 at most; no goals.
        let mut s = at(10.0, 10.0);
        let e = eat_at(&mut s, 52, -25.0, -5.0).unwrap();
        assert_eq!((s.food, s.water), (0.0, 5.0));
        assert!(!e.eat_goal && !e.drink_goal);
        // Both zero: nothing at all.
        let mut s = at(10.0, 10.0);
        assert!(eat_at(&mut s, 52, 0.0, 0.0).is_none());
        assert_eq!((s.food, s.water), (10.0, 10.0));
        // Water only.
        let e = eat_at(&mut s, 52, 0.0, 5.0).unwrap();
        assert_eq!((e.on_eat, e.on_drink), (None, Some(5.0)));
    }

    #[test]
    fn vitals_ok_and_oxygen_food() {
        let mut s = at(15.0, 50.0);
        assert!(eat_at(&mut s, 52, 10.0, 0.0).unwrap().vitals_ok);
        let mut s = at(25.0, 50.0);
        assert!(!eat_at(&mut s, 52, 10.0, 0.0).unwrap().vitals_ok);
        let mut s = at(15.0, 50.0);
        assert_eq!(eat_at(&mut s, 51, 3.0, 0.0).unwrap().oxygen, 15.0);
        assert_eq!(eat_at(&mut s, 52, 3.0, 0.0).unwrap().oxygen, 0.0);
        assert!(eat_at(&mut s, 51, 0.0, 0.0).is_none());
    }

    #[test]
    fn first_aid_counts_the_actual_healing() {
        let c = consume();
        let mut h = 40.0;
        assert_eq!(
            use_tech(&c, 50, &mut h, 100.0).unwrap(),
            Some(Used::Healed(50.0))
        );
        assert_eq!(h, 90.0);
        let mut h = 99.85;
        assert!(matches!(
            use_tech(&c, 50, &mut h, 100.0).unwrap(),
            Some(Used::Healed(_))
        ));
        // At or below the threshold: healed, but the kit stays.
        let mut h = 99.95;
        let r = use_tech(&c, 50, &mut h, 100.0).unwrap();
        assert!(matches!(r, Some(Used::Full(x)) if (x - 0.05).abs() < 1e-9));
        assert_eq!(h, 100.0);
        assert_eq!(
            use_tech(&c, 50, &mut h, 100.0).unwrap(),
            Some(Used::Full(0.0))
        );
        // The dead do not heal; another tech is not used.
        let mut dead = 0.0;
        assert_eq!(
            use_tech(&c, 50, &mut dead, 100.0).unwrap(),
            Some(Used::Full(0.0))
        );
        assert_eq!(use_tech(&c, 52, &mut h, 100.0).unwrap(), None);
    }

    fn food_rules() -> EatableInfo {
        EatableInfo {
            food: 20.0,
            water: 5.0,
            decomposes: true,
            decay_rate: 0.01,
            despawns: true,
            despawn_delay: 300.0,
        }
    }

    fn inventory() -> Inventory {
        Inventory::new((6, 8), &table(), &["Tank"], KnownTech::default())
    }

    fn food_item(id: u64, start: f64) -> Item {
        let mut i = Item::new(id, 52);
        i.eatable = true;
        i.state = initial_state(Some(&food_rules()), None, false, start);
        i
    }

    #[test]
    fn eating_and_using_remove_exactly_that_item() {
        let t = techs();
        let mut inv = inventory();
        inv.container.add(&t, food_item(1, 0.0)).unwrap();
        inv.container.add(&t, food_item(2, 900.0)).unwrap();
        inv.container.add(&t, Item::new(3, 50)).unwrap();
        let info = |tech| (tech == 52).then(food_rules);
        let (h, c) = (rules(), consume());
        let mut s = at(50.0, 50.0);
        // Item 1 at t = 1000: 20 − 10 = 10 food, 5 − 10 → −5 water.
        let (item, e) = eat_item(&mut inv, ItemId(1), &mut s, &h, &c, info, 1000.0)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(item.id, ItemId(1));
        assert_eq!((e.on_eat, e.on_drink), (Some(10.0), Some(-5.0)));
        assert_eq!((s.food, s.water), (60.0, 45.0));
        assert!(inv.container.contains(ItemId(2)));
        // Not in the grid / not eatable.
        assert_eq!(
            eat_item(&mut inv, ItemId(1), &mut s, &h, &c, info, 1000.0).unwrap(),
            Err(Refused::NotInGrid)
        );
        assert_eq!(
            eat_item(&mut inv, ItemId(3), &mut s, &h, &c, info, 1000.0).unwrap(),
            Err(Refused::NotEatable)
        );
        // The kit: kept at full health, used up otherwise.
        let mut health = 100.0;
        let r = use_item(&mut inv, ItemId(3), &c, &mut health, 100.0).unwrap();
        assert_eq!(r, Ok(Some(UsedItem::Full(0.0))));
        assert!(inv.container.contains(ItemId(3)));
        health = 30.0;
        let used = use_item(&mut inv, ItemId(3), &c, &mut health, 100.0)
            .unwrap()
            .unwrap();
        assert!(
            matches!(used, Some(UsedItem::Healed(item, h)) if item.id == ItemId(3) && h == 50.0)
        );
        assert_eq!(inv.container.len(), 1);
    }

    #[test]
    fn decomposing_values_are_floored() {
        let e = food_rules();
        assert_eq!(e.values(100.0, 100.0, -25.0), (20.0, 5.0));
        assert_eq!(e.values(0.0, 1e6, -25.0), (-25.0, -25.0));
        let fresh = EatableInfo {
            decomposes: false,
            ..food_rules()
        };
        assert_eq!(fresh.values(0.0, 1e6, -25.0), (20.0, 5.0));
        assert_eq!(
            initial_state(Some(&fresh), None, false, 7.0).decay_start,
            Some(0.0)
        );
        assert_eq!(
            initial_state(Some(&e), None, false, 7.0).decay_start,
            Some(7.0)
        );
    }

    /// Two of each kind, same tech type, different state, through 1,000
    /// moves between the grid, the slots, a locker and the floor.
    #[test]
    fn identical_tech_types_keep_their_own_state_through_moves() {
        let t = techs();
        let mut inv = inventory();
        let mut locker = Container::new(4, 8);
        let tank = |id, o2| {
            let mut i = Item::new(id, 30);
            i.state = initial_state(None, Some(o2), false, 0.0);
            i
        };
        let tool = |id, battery: u64, charge| {
            let mut i = Item::new(id, 1);
            i.state = initial_state(None, None, true, 0.0);
            let mut b = Item::new(battery, 2);
            b.charge = Some(charge);
            i.state.energy.as_mut().unwrap().battery = Some(Box::new(b));
            i
        };
        let all = vec![
            tank(1, 10.0),
            tank(2, 20.0),
            food_item(3, 5.0),
            food_item(4, 50.0),
            tool(5, 6, 30.0),
            tool(7, 8, 70.0),
        ];
        let mut expected: Vec<(ItemId, Item)> = all.iter().map(|i| (i.id, i.clone())).collect();
        expected.sort_by_key(|e| e.0);
        for i in all {
            inv.container.add(&t, i).unwrap();
        }
        let mut floor: Vec<Item> = Vec::new();
        let mut seed = 12345u64;
        let mut next = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let mut moved = 0;
        for _ in 0..1000 {
            let id = ItemId([1, 2, 3, 4, 5, 7][next(6) as usize]);
            let op = next(4);
            if let Some(k) = floor.iter().position(|i| i.id == id) {
                let item = floor.remove(k);
                match pickup(&mut inv, &t, &[], item, false) {
                    Ok(_) => moved += 1,
                    Err(item) => floor.push(item),
                }
                continue;
            }
            let in_grid = inv.container.contains(id);
            let in_slots = inv.equipment.slot_of(id).is_some();
            let ok = {
                let mut stores = [
                    Store::Grid(&mut inv.container),
                    Store::Slots(&mut inv.equipment),
                    Store::Grid(&mut locker),
                ];
                let from = if in_grid {
                    0
                } else if in_slots {
                    1
                } else {
                    2
                };
                match op {
                    0 | 1 => {
                        let to = [0usize, 1, 2][next(3) as usize];
                        from != to && add_or_swap(&t, &mut stores, from, id, to, &Target::Store)
                    }
                    _ => false,
                }
            };
            if ok {
                moved += 1;
            } else if op == 2 && (in_grid || in_slots) {
                if let Some(item) = inv.drop_item(id, DropPlace::default()) {
                    floor.push(item);
                    moved += 1;
                }
            } else if op == 3 && (in_grid || in_slots) {
                let lost = lose_items(&mut inv, DropPlace::default());
                if !lost.is_empty() {
                    floor.extend(lost);
                    moved += 1;
                }
            }
            // Every item exactly once, each with its own state.
            let mut now: Vec<(ItemId, Item)> = inv
                .container
                .items()
                .chain(inv.equipment.items())
                .chain(locker.items())
                .chain(floor.iter())
                .map(|i| (i.id, i.clone()))
                .collect();
            now.sort_by_key(|e| e.0);
            assert_eq!(now, expected);
            let mut ids: Vec<ItemId> = now.iter().flat_map(|(_, i)| i.ids()).collect();
            let n = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), n, "an item id twice");
            assert_eq!(n, 8);
        }
        assert!(moved > 300, "{moved} moves");
    }

    #[test]
    fn invalid_consume_input_refuses_without_mutation() {
        let h = rules();
        let c = consume();
        let mut s = at(50.0, 50.0);
        let before = s.clone();
        assert!(eat(&mut s, &h, &c, 52, f64::NAN, 1.0).is_err());
        assert!(eat(&mut s, &h, &c, 52, 1.0, f64::INFINITY).is_err());
        assert!(
            eat(
                &mut s,
                &h,
                &ConsumeRules {
                    decay_floor: 1.0,
                    ..consume()
                },
                52,
                1.0,
                1.0
            )
            .is_err()
        );
        assert_eq!(s, before);
        let mut health = 50.0;
        assert!(use_tech(&c, 50, &mut health, f64::NAN).is_err());
        assert_eq!(health, 50.0);
        assert!(
            EatableInfo {
                decay_rate: -1.0,
                ..food_rules()
            }
            .validate()
            .is_err()
        );
    }
}
