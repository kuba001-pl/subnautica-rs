//! Battery charge and `EnergyMixin` rules (M9s3): `Battery` (`_capacity`,
//! `_charge`, `set_charge`, prefab initialization vs save restore),
//! `EnergyMixin` (`SpawnDefaultAsync`/`OnCraftEnd`, `ModifyCharge`,
//! `AddEnergy`, `ConsumeEnergy`, `Filter`, `Sort`, `Select`), and outgoing
//! battery retention through [`crate::inventory::force_pickup`]
//! (`Inventory.ForcePickup` / `_pendingItems`). Tool-specific energy costs
//! belong to their callers; there is no universal passive drain here.

use super::RuleError;
use crate::inventory::{
    AnalysisEntry, EnergySlot, Inventory, Item, ItemId, TECH_NONE, TechType, Techs, force_pickup,
};

/// Code-only defaults of `Battery` and `EnergyMixin`, read from
/// `Assembly-CSharp.dll` at runtime (no balance defaults here).
#[derive(Clone, Debug, PartialEq)]
pub struct EnergyRules {
    /// `EnergyMixin.defaultBattery` field initializer (`TechType.Battery`).
    pub default_battery_tech: TechType,
    /// `EnergyMixin.defaultBatteryCharge` field initializer (`1.0`).
    pub default_battery_charge: f32,
    /// `Battery._capacity` / `EnergyMixin.maxEnergy` field initializer
    /// (`100.0`).
    pub default_battery_capacity: f32,
    /// `EnergyMixin.get_charge`: recovery ramp seconds after electronics
    /// re-enable (`2.0`).
    pub recovery_time: f32,
}

impl EnergyRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        let ok = self.default_battery_tech != TECH_NONE
            && self.default_battery_charge.is_finite()
            && (0.0..=1.0).contains(&self.default_battery_charge)
            && self.default_battery_capacity.is_finite()
            && self.default_battery_capacity > 0.0
            && self.recovery_time.is_finite()
            && self.recovery_time > 0.0;
        if ok {
            Ok(())
        } else {
            Err(RuleError("invalid energy rules"))
        }
    }
}

/// An `EnergyMixin` prefab's serialized battery policy.
#[derive(Clone, Debug, PartialEq)]
pub struct EnergyMixinInfo {
    pub default_battery: TechType,
    pub default_battery_charge: f32,
    pub compatible_batteries: Vec<TechType>,
    pub allow_replacement: bool,
}

impl EnergyMixinInfo {
    pub fn validate(&self) -> Result<(), RuleError> {
        let default_ok = self.default_battery == TECH_NONE
            || self.compatible_batteries.contains(&self.default_battery);
        if self.default_battery_charge.is_finite() && default_ok {
            Ok(())
        } else {
            Err(RuleError("invalid EnergyMixin"))
        }
    }
}

fn check_capacity(capacity: f32) -> Result<f32, RuleError> {
    if capacity.is_finite() && capacity > 0.0 {
        Ok(capacity)
    } else {
        Err(RuleError("invalid battery capacity"))
    }
}

/// `Battery.set_charge`: `_charge = Mathf.Min(value, _capacity)`.
pub fn set_battery_charge(battery: &mut Item, capacity: f32, value: f32) -> Result<f32, RuleError> {
    let cap = check_capacity(capacity)?;
    if !value.is_finite() || battery.charge.is_none() {
        return Err(RuleError("invalid battery charge"));
    }
    let next = value.min(cap);
    battery.charge = Some(next);
    Ok(next)
}

/// `EnergyMixin.SetBatteryAsync` / `SpawnBatteryAsync`: spawns a battery
/// item with `normalized_charge` (`< 0` means `1.0`, else clamped to
/// `0..=1`) times `capacity`.
pub fn make_battery(
    id: ItemId,
    tech: TechType,
    capacity: f32,
    normalized_charge: f32,
) -> Result<Item, RuleError> {
    let cap = check_capacity(capacity)?;
    if tech == TECH_NONE || !normalized_charge.is_finite() {
        return Err(RuleError("invalid battery creation"));
    }
    let norm = if normalized_charge < 0.0 {
        1.0
    } else {
        normalized_charge.clamp(0.0, 1.0)
    };
    let mut item = Item::new(id.0, tech);
    item.charge = Some((norm * cap).min(cap));
    Ok(item)
}

/// `Battery.OnProtoDeserialize`: restores a saved battery with its saved
/// `_charge` (capped at `capacity`, never refilled to full when used or
/// empty).
pub fn restore_battery(
    id: ItemId,
    tech: TechType,
    capacity: f32,
    saved_charge: f32,
) -> Result<Item, RuleError> {
    let cap = check_capacity(capacity)?;
    if tech == TECH_NONE || !saved_charge.is_finite() || saved_charge < 0.0 {
        return Err(RuleError("invalid saved battery"));
    }
    let mut item = Item::new(id.0, tech);
    item.charge = Some(saved_charge.min(cap));
    Ok(item)
}

/// `EnergyMixin.SpawnDefaultAsync` (`OnCraftEnd`): spawns the default
/// battery only if `tool`'s battery slot is currently empty and
/// `default_battery != TechType.None`. Never replaces or refills an
/// already-present battery.
pub fn spawn_default_battery(
    tool: &mut Item,
    mixin: &EnergyMixinInfo,
    next_id: impl FnOnce() -> ItemId,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<bool, RuleError> {
    mixin.validate()?;
    let slot = tool
        .state
        .energy
        .as_mut()
        .ok_or(RuleError("item has no EnergyMixin"))?;
    if slot.battery.is_some() || mixin.default_battery == TECH_NONE {
        return Ok(false);
    }
    let cap = battery_capacity(mixin.default_battery)
        .ok_or(RuleError("missing default battery capacity"))?;
    let bat = make_battery(
        next_id(),
        mixin.default_battery,
        cap,
        mixin.default_battery_charge,
    )?;
    slot.battery = Some(Box::new(bat));
    Ok(true)
}

/// Current battery charge in `slot` (`0.0` when no battery is installed).
pub fn energy_charge(slot: &EnergySlot) -> Result<f32, RuleError> {
    let Some(b) = slot.battery.as_ref() else {
        return Ok(0.0);
    };
    let charge = b.charge.ok_or(RuleError("battery missing charge"))?;
    if !charge.is_finite() || charge < 0.0 {
        return Err(RuleError("invalid battery charge"));
    }
    Ok(charge)
}

/// Current battery capacity in `slot` (`0.0` when no battery is installed).
pub fn energy_capacity(
    slot: &EnergySlot,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<f32, RuleError> {
    let Some(b) = slot.battery.as_ref() else {
        return Ok(0.0);
    };
    let cap = battery_capacity(b.tech).ok_or(RuleError("missing battery capacity"))?;
    check_capacity(cap)
}

/// `EnergyMixin.GetEnergyScalar` / `GetBatteryChargeValue`: `0.0` when no
/// battery is installed, else `charge / capacity` clamped to `0..=1`.
pub fn energy_scalar(
    slot: &EnergySlot,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<f32, RuleError> {
    let charge = energy_charge(slot)?;
    let cap = energy_capacity(slot, battery_capacity)?;
    if cap <= 0.0 {
        Ok(0.0)
    } else {
        Ok((charge / cap).clamp(0.0, 1.0))
    }
}

/// `EnergyMixin.ModifyCharge`: adds up to `capacity - charge` (when
/// `amount >= 0`) or removes up to `charge` (when `amount < 0`), returning
/// the signed change actually applied (`0.0` when `!requires_power` or no
/// battery is installed).
pub fn modify_charge(
    slot: &mut EnergySlot,
    amount: f32,
    requires_power: bool,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<f32, RuleError> {
    if !amount.is_finite() {
        return Err(RuleError("invalid charge delta"));
    }
    let Some(b) = slot.battery.as_mut() else {
        return Ok(0.0);
    };
    let charge = b.charge.ok_or(RuleError("battery missing charge"))?;
    let cap =
        check_capacity(battery_capacity(b.tech).ok_or(RuleError("missing battery capacity"))?)?;
    if !charge.is_finite() || charge < 0.0 || charge > cap {
        return Err(RuleError("invalid battery charge"));
    }
    if !requires_power {
        return Ok(0.0);
    }
    let delta = if amount >= 0.0 {
        amount.min(cap - charge)
    } else {
        -((-amount).min(charge))
    };
    b.charge = Some((charge + delta).clamp(0.0, cap));
    Ok(delta)
}

/// `EnergyMixin.AddEnergy`: `ModifyCharge(|amount|) > 0`.
pub fn add_energy(
    slot: &mut EnergySlot,
    amount: f32,
    requires_power: bool,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<bool, RuleError> {
    Ok(modify_charge(slot, amount.abs(), requires_power, battery_capacity)? > 0.0)
}

/// `EnergyMixin.ConsumeEnergy`: `-ModifyCharge(-|amount|) > 0`. Succeeds
/// for any positive charge removed, even when less than `|amount|`.
pub fn consume_energy(
    slot: &mut EnergySlot,
    amount: f32,
    requires_power: bool,
    battery_capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<bool, RuleError> {
    Ok(-modify_charge(slot, -amount.abs(), requires_power, battery_capacity)? > 0.0)
}

/// `EnergyMixin.Filter` + `Sort`: compatible batteries from `inv.container`
/// and `slot.battery`, ordered with `slot.battery` first (returning
/// `Some(0)` as the active index, or `None` when unloaded), followed by
/// each compatible `TechType` in `mixin.compatible_batteries` order sorted
/// by descending `charge`.
pub fn sort_reload_candidates(
    inv: &Inventory,
    slot: &EnergySlot,
    mixin: &EnergyMixinInfo,
) -> Result<(Vec<ItemId>, Option<usize>), RuleError> {
    mixin.validate()?;
    let mut out = Vec::new();
    let active = if let Some(stored) = slot.battery.as_ref() {
        if stored.charge.is_none_or(|c| !c.is_finite()) {
            return Err(RuleError("invalid stored battery"));
        }
        out.push(stored.id);
        Some(0)
    } else {
        None
    };
    for &tech in &mixin.compatible_batteries {
        let mut group: Vec<(ItemId, f32)> = Vec::new();
        for item in inv.container.items().filter(|i| i.tech == tech) {
            let c = item
                .charge
                .ok_or(RuleError("compatible battery missing charge"))?;
            if !c.is_finite() {
                return Err(RuleError("invalid battery charge"));
            }
            group.push((item.id, c));
        }
        group.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.extend(group.into_iter().map(|(id, _)| id));
    }
    Ok((out, active))
}

/// Why a battery reload request was refused without changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadRefused {
    ReplacementDisallowed,
    NotInGrid,
    Incompatible,
    NotABattery,
}

/// What [`select_battery`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// Selecting the already-installed battery, or unloading an empty slot.
    NoChange,
    /// Installed `loaded` from the grid; if `returned` was previously
    /// installed, it was returned via `force_pickup` (`pending` is true
    /// when it went to `inv.pending` because the grid had no room).
    Swapped {
        loaded: ItemId,
        returned: Option<ItemId>,
        pending: bool,
    },
    /// Unloaded `returned` via `force_pickup` (`pending` is true when the
    /// grid was full).
    Unloaded {
        returned: ItemId,
        pending: bool,
    },
    Refused(ReloadRefused),
}

/// `EnergyMixin.Select`: replaces or unloads (`choice == None`) the battery
/// in `slot`. Outgoing batteries return through [`force_pickup`] so a full
/// grid retains them in `inv.pending` without losing or duplicating items.
pub fn select_battery(
    inv: &mut Inventory,
    techs: &Techs,
    analysis: &[AnalysisEntry],
    slot: &mut EnergySlot,
    mixin: &EnergyMixinInfo,
    choice: Option<ItemId>,
    in_sub: bool,
) -> Result<ReloadOutcome, RuleError> {
    mixin.validate()?;
    if !mixin.allow_replacement {
        return Ok(ReloadOutcome::Refused(ReloadRefused::ReplacementDisallowed));
    }
    match choice {
        None => {
            let Some(old) = slot.battery.take() else {
                return Ok(ReloadOutcome::NoChange);
            };
            let returned = old.id;
            let (into, _) = force_pickup(inv, techs, analysis, *old, in_sub);
            Ok(ReloadOutcome::Unloaded {
                returned,
                pending: into.is_none(),
            })
        }
        Some(target_id) => {
            if slot.battery.as_ref().is_some_and(|b| b.id == target_id) {
                return Ok(ReloadOutcome::NoChange);
            }
            let Some(candidate) = inv.container.get(target_id) else {
                return Ok(ReloadOutcome::Refused(ReloadRefused::NotInGrid));
            };
            if !mixin.compatible_batteries.contains(&candidate.tech) {
                return Ok(ReloadOutcome::Refused(ReloadRefused::Incompatible));
            }
            let Some(charge) = candidate.charge else {
                return Ok(ReloadOutcome::Refused(ReloadRefused::NotABattery));
            };
            if !charge.is_finite() || charge < 0.0 {
                return Err(RuleError("invalid candidate battery charge"));
            }
            // `batterySlot.RemoveItem(); batterySlot.AddItem(item); ForcePickup(old);`
            let old = slot.battery.take();
            let incoming = inv
                .container
                .remove(target_id)
                .ok_or(RuleError("selected battery vanished"))?;
            slot.battery = Some(Box::new(incoming));
            if let Some(old) = old {
                let old_id = old.id;
                let (into, _) = force_pickup(inv, techs, analysis, *old, in_sub);
                Ok(ReloadOutcome::Swapped {
                    loaded: target_id,
                    returned: Some(old_id),
                    pending: into.is_none(),
                })
            } else {
                Ok(ReloadOutcome::Swapped {
                    loaded: target_id,
                    returned: None,
                    pending: false,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{KnownTech, SlotTable, flush_pending};
    use crate::survival::initial_state;

    const TOOL: TechType = 10;
    const BAT: TechType = 504;
    const ION_BAT: TechType = 505;
    const POWER_CELL: TechType = 506;
    const ROCK: TechType = 999;

    fn mixin() -> EnergyMixinInfo {
        EnergyMixinInfo {
            default_battery: BAT,
            default_battery_charge: 1.0,
            compatible_batteries: vec![BAT, ION_BAT],
            allow_replacement: true,
        }
    }

    fn cap(tech: TechType) -> Option<f32> {
        match tech {
            BAT => Some(100.0),
            ION_BAT => Some(500.0),
            _ => None,
        }
    }

    fn techs() -> Techs {
        let mut t = Techs::default();
        t.sizes.insert(BAT, (1, 1));
        t.sizes.insert(ION_BAT, (1, 2));
        t.sizes.insert(ROCK, (1, 1));
        t
    }

    fn inv(size: (i32, i32)) -> Inventory {
        Inventory::new(
            size,
            &SlotTable::default(),
            &[] as &[&str],
            KnownTech::default(),
        )
    }

    #[test]
    fn default_battery_only_if_absent_and_restored_used_battery_never_fills() {
        let m = mixin();
        let mut tool = Item::new(1, TOOL);
        tool.state = initial_state(None, None, true, 0.0);
        // Empty slot: spawns default battery once with full capacity.
        assert!(spawn_default_battery(&mut tool, &m, || ItemId(2), cap).unwrap());
        let slot = tool.state.energy.as_mut().unwrap();
        assert_eq!(
            slot.battery.as_ref().map(|b| (b.id, b.tech, b.charge)),
            Some((ItemId(2), BAT, Some(100.0)))
        );

        // Drain to 25, then call `spawn_default_battery` again: must NOT replace or refill!
        assert!(consume_energy(slot, 75.0, true, cap).unwrap());
        assert_eq!(energy_charge(slot).unwrap(), 25.0);
        assert!(!spawn_default_battery(&mut tool, &m, || ItemId(99), cap).unwrap());
        let slot = tool.state.energy.as_ref().unwrap();
        assert_eq!(
            slot.battery.as_ref().map(|b| (b.id, b.charge)),
            Some((ItemId(2), Some(25.0)))
        );

        // Restoring a used or depleted battery from save preserves its exact charge (25 or 0),
        // never refilling to capacity.
        let used = restore_battery(ItemId(10), BAT, 100.0, 25.0).unwrap();
        assert_eq!(used.charge, Some(25.0));
        let empty = restore_battery(ItemId(11), BAT, 100.0, 0.0).unwrap();
        assert_eq!(empty.charge, Some(0.0));
        let mut restored_tool = Item::new(12, TOOL);
        restored_tool.state = initial_state(None, None, true, 0.0);
        restored_tool.state.energy.as_mut().unwrap().battery = Some(Box::new(empty));
        assert!(!spawn_default_battery(&mut restored_tool, &m, || ItemId(99), cap).unwrap());
        assert_eq!(
            energy_charge(restored_tool.state.energy.as_ref().unwrap()).unwrap(),
            0.0
        );
    }

    #[test]
    fn empty_full_and_partial_energy_changes_conserve_charge() {
        let mut slot = EnergySlot {
            battery: Some(Box::new(make_battery(ItemId(1), BAT, 100.0, 0.5).unwrap())),
        };
        assert_eq!(energy_charge(&slot).unwrap(), 50.0);
        assert_eq!(energy_capacity(&slot, cap).unwrap(), 100.0);
        assert_eq!(energy_scalar(&slot, cap).unwrap(), 0.5);

        // Partial add (50 + 30 = 80) and overflow add (80 + 50 -> +20 to 100) conserve charge.
        let d1 = modify_charge(&mut slot, 30.0, true, cap).unwrap();
        assert_eq!((d1, energy_charge(&slot).unwrap()), (30.0, 80.0));
        let d2 = modify_charge(&mut slot, 50.0, true, cap).unwrap();
        assert_eq!((d2, energy_charge(&slot).unwrap()), (20.0, 100.0));
        // Full battery rejects more energy (`AddEnergy` returns false, delta 0).
        assert!(!add_energy(&mut slot, 10.0, true, cap).unwrap());
        assert_eq!(energy_charge(&slot).unwrap(), 100.0);

        // Partial consume (100 - 95 = 5) and underflow consume (5 - 20 -> -5 to 0, still succeeds!).
        let d3 = modify_charge(&mut slot, -95.0, true, cap).unwrap();
        assert_eq!((d3, energy_charge(&slot).unwrap()), (-95.0, 5.0));
        assert!(consume_energy(&mut slot, 20.0, true, cap).unwrap());
        assert_eq!(energy_charge(&slot).unwrap(), 0.0);
        // Empty battery cannot consume (`ConsumeEnergy` returns false, delta 0).
        assert!(!consume_energy(&mut slot, 1.0, true, cap).unwrap());
        assert_eq!(energy_charge(&slot).unwrap(), 0.0);

        // Total accounting: initial 50 + 30 + 20 - 95 - 5 == 0.
        assert_eq!(50.0 + d1 + d2 + d3 - 5.0, energy_charge(&slot).unwrap());

        // No-power mode leaves charge unchanged.
        assert!(add_energy(&mut slot, 40.0, true, cap).unwrap());
        assert_eq!(modify_charge(&mut slot, -10.0, false, cap).unwrap(), 0.0);
        assert_eq!(energy_charge(&slot).unwrap(), 40.0);
    }

    #[test]
    fn reload_swaps_sorts_and_at_full_grid_retains_outgoing_battery_once() {
        let t = techs();
        let m = mixin();
        let mut i = inv((2, 2));
        // Tool starts with a 1x2 Ion Battery (id 1, charge 200).
        let mut slot = EnergySlot {
            battery: Some(Box::new(
                restore_battery(ItemId(1), ION_BAT, 500.0, 200.0).unwrap(),
            )),
        };
        // Grid (2x2 = 4 cells) holds two 1x1 batteries (ids 2 and 3), one 1x1 rock (id 4),
        // and one 1x1 incompatible item (id 5): grid is completely full (4/4 cells).
        let b2 = restore_battery(ItemId(2), BAT, 100.0, 30.0).unwrap();
        let b3 = restore_battery(ItemId(3), BAT, 100.0, 90.0).unwrap();
        let mut p5 = Item::new(5, POWER_CELL);
        p5.charge = Some(200.0);
        i.container.add(&t, b2).unwrap();
        i.container.add(&t, b3).unwrap();
        i.container.add(&t, Item::new(4, ROCK)).unwrap();
        i.container.add(&t, p5).unwrap();

        // Candidate sorting: stored battery (1) at index 0, then compatible BATs sorted
        // by descending charge (3 before 2); incompatible POWER_CELL (5) and ROCK (4) excluded.
        let (list, active) = sort_reload_candidates(&i, &slot, &m).unwrap();
        assert_eq!(active, Some(0));
        assert_eq!(list, vec![ItemId(1), ItemId(3), ItemId(2)]);

        // Incompatible or non-battery selection refuses without mutating anything.
        assert_eq!(
            select_battery(&mut i, &t, &[], &mut slot, &m, Some(ItemId(5)), false).unwrap(),
            ReloadOutcome::Refused(ReloadRefused::Incompatible)
        );
        assert_eq!(
            select_battery(&mut i, &t, &[], &mut slot, &m, Some(ItemId(4)), false).unwrap(),
            ReloadOutcome::Refused(ReloadRefused::Incompatible)
        );

        // Swap 1x1 battery (id 3, charge 90) into tool while outgoing battery is 1x2 (id 1, charge 200):
        // removing id 3 frees only one 1x1 cell, so 1x2 id 1 cannot fit in the 2x2 grid and goes to `pending`!
        let out = select_battery(&mut i, &t, &[], &mut slot, &m, Some(ItemId(3)), false).unwrap();
        assert_eq!(
            out,
            ReloadOutcome::Swapped {
                loaded: ItemId(3),
                returned: Some(ItemId(1)),
                pending: true,
            }
        );
        assert_eq!(
            slot.battery.as_ref().map(|b| (b.id, b.charge)),
            Some((ItemId(3), Some(90.0)))
        );
        assert!(!i.container.contains(ItemId(1)));
        assert_eq!(i.pending.len(), 1);
        assert_eq!(
            (i.pending[0].id, i.pending[0].charge),
            (ItemId(1), Some(200.0))
        );

        // Fill the freed 1x1 cell with another rock (id 6), then unload id 3 at a completely full grid:
        // id 3 also goes to `pending`!
        i.container.add(&t, Item::new(6, ROCK)).unwrap();
        let out = select_battery(&mut i, &t, &[], &mut slot, &m, None, false).unwrap();
        assert_eq!(
            out,
            ReloadOutcome::Unloaded {
                returned: ItemId(3),
                pending: true,
            }
        );
        assert!(slot.battery.is_none());
        assert_eq!(i.pending.len(), 2);

        // Every item id across slot + grid + pending exists exactly once, with its charge intact.
        let mut all_ids: Vec<ItemId> = slot
            .battery
            .iter()
            .map(|b| b.id)
            .chain(i.container.items().map(|it| it.id))
            .chain(i.pending.iter().map(|it| it.id))
            .collect();
        all_ids.sort();
        assert_eq!(
            all_ids,
            vec![
                ItemId(1),
                ItemId(2),
                ItemId(3),
                ItemId(4),
                ItemId(5),
                ItemId(6)
            ]
        );

        // Removing items from the grid and flushing pending moves pending batteries back into the grid once.
        i.container.remove(ItemId(4)).unwrap();
        i.container.remove(ItemId(5)).unwrap();
        i.container.remove(ItemId(6)).unwrap();
        let flushed = flush_pending(&mut i, &t, &[], false);
        assert_eq!(flushed.len(), 2);
        assert!(i.pending.is_empty());
        assert_eq!(
            i.container.get(ItemId(1)).and_then(|b| b.charge),
            Some(200.0)
        );
        assert_eq!(
            i.container.get(ItemId(3)).and_then(|b| b.charge),
            Some(90.0)
        );
    }

    #[test]
    fn missing_or_invalid_energy_inputs_refuse_explicitly() {
        let m = mixin();
        let mut plain = Item::new(1, ROCK);
        // Item without EnergyMixin refuses default battery spawn.
        assert!(spawn_default_battery(&mut plain, &m, || ItemId(2), cap).is_err());

        // Missing battery capacity refuses spawn, capacity, and modify_charge without mutating charge.
        let mut tool = Item::new(1, TOOL);
        tool.state = initial_state(None, None, true, 0.0);
        assert!(spawn_default_battery(&mut tool, &m, || ItemId(2), |_| None).is_err());
        assert!(tool.state.energy.as_ref().unwrap().battery.is_none());

        let mut slot = EnergySlot {
            battery: Some(Box::new(
                restore_battery(ItemId(2), BAT, 100.0, 40.0).unwrap(),
            )),
        };
        assert!(modify_charge(&mut slot, 10.0, true, |_| None).is_err());
        assert!(modify_charge(&mut slot, f32::NAN, true, cap).is_err());
        assert_eq!(energy_charge(&slot).unwrap(), 40.0);

        assert!(make_battery(ItemId(3), BAT, 0.0, 1.0).is_err());
        assert!(restore_battery(ItemId(3), BAT, 100.0, -1.0).is_err());
        assert!(
            EnergyRules {
                default_battery_capacity: 0.0,
                default_battery_tech: BAT,
                default_battery_charge: 1.0,
                recovery_time: 2.0,
            }
            .validate()
            .is_err()
        );
    }
}
