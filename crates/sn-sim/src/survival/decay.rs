//! Food and water decay and world rotten despawn (M9s3):
//! `Eatable.GetFoodValue`/`GetWaterValue`, `Eatable.IsRotten`,
//! `Eatable.GetSecondaryTooltip` (freshness labels), `Eatable.SetDecomposes`
//! and `Eatable.StartDespawnInvoke`/`IterateDespawn`.
//!
//! Stored items (`Pickupable.Initialize` / `StorageSlot.UnsafeAdd` /
//! `Inventory.AddPending`) have `gameObject.activeSelf == false`, so
//! `IterateDespawn` resets `wasActive = false` and never destroys a rotten
//! item in a container; only an active rotten object in the world starts
//! `timeDespawnStart` and is destroyed once elapsed active time is strictly
//! greater than `despawnDelay`.

use super::{EatableInfo, RuleError, finite, nonnegative};

/// The code-only constants of `Eatable`, read from the player's
/// `Assembly-CSharp.dll` at runtime (no balance defaults here).
#[derive(Clone, Debug, PartialEq)]
pub struct DecayRules {
    /// `Eatable.GetFoodValue`/`GetWaterValue` floor (`-25.0`).
    pub decay_floor: f64,
    /// `Eatable.GetSecondaryTooltip`: `num < ripe_fraction * foodValue`
    /// (`0.5`) -> `"Ripe"`.
    pub ripe_fraction: f64,
    /// `Eatable.GetSecondaryTooltip`: `num < decomposing_fraction * foodValue`
    /// (`0.9`) -> `"Decomposing"`.
    pub decomposing_fraction: f64,
    /// `Eatable.StartDespawnInvoke`: max initial delay and repeat interval
    /// (`5.0` s).
    pub despawn_check_interval: f64,
    /// `Eatable.despawnDelay` field initializer (`300.0` s).
    pub default_despawn_delay: f64,
}

impl DecayRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        let ok = finite(&[self.decay_floor])
            && nonnegative(&[
                self.ripe_fraction,
                self.decomposing_fraction,
                self.despawn_check_interval,
                self.default_despawn_delay,
            ])
            && self.decay_floor <= 0.0
            && self.ripe_fraction > 0.0
            && self.ripe_fraction < self.decomposing_fraction
            && self.decomposing_fraction <= 1.0
            && self.despawn_check_interval > 0.0
            && self.default_despawn_delay > 0.0;
        if ok {
            Ok(())
        } else {
            Err(RuleError("invalid decay rules"))
        }
    }
}

/// `Eatable.GetSecondaryTooltip`'s freshness classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Empty string (fresh, non-decomposing, or survival not required).
    Fresh,
    /// `"Decomposing"`: `ripe_fraction * base_food <= food < decomposing_fraction * base_food`.
    Decomposing,
    /// `"Ripe"`: `food < ripe_fraction * base_food` (and not both negative).
    Ripe,
    /// `"Rotting"`: `food < 0.0 && water < 0.0`.
    Rotting,
}

impl Freshness {
    /// The localization key `Eatable.GetSecondaryTooltip` looks up, or
    /// `None` for `string.Empty`.
    pub fn language_key(self) -> Option<&'static str> {
        match self {
            Freshness::Fresh => None,
            Freshness::Decomposing => Some("Decomposing"),
            Freshness::Ripe => Some("Ripe"),
            Freshness::Rotting => Some("Rotting"),
        }
    }
}

/// Runtime-only despawn state of an `Eatable` (`wasActive`,
/// `timeDespawnStart`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DespawnState {
    pub was_active: bool,
    pub despawn_start: f64,
}

/// `Eatable.IsRotten`: both current food and current water strictly below 0.
pub fn is_rotten(
    info: &EatableInfo,
    decay_start: f64,
    now: f64,
    rules: &DecayRules,
) -> Result<bool, RuleError> {
    info.validate()?;
    rules.validate()?;
    if !finite(&[decay_start, now]) {
        return Err(RuleError("invalid decay time"));
    }
    let (food, water) = info.values(decay_start, now, rules.decay_floor);
    Ok(food < 0.0 && water < 0.0)
}

/// `Eatable.GetSecondaryTooltip`: Rotting first (both current values
/// negative), then Ripe (`food < ripe_fraction * foodValue`), then
/// Decomposing (`food < decomposing_fraction * foodValue`), else Fresh.
pub fn freshness(
    info: &EatableInfo,
    decay_start: f64,
    now: f64,
    requires_survival: bool,
    rules: &DecayRules,
) -> Result<Freshness, RuleError> {
    info.validate()?;
    rules.validate()?;
    if !finite(&[decay_start, now]) {
        return Err(RuleError("invalid decay time"));
    }
    if !requires_survival || !info.decomposes {
        return Ok(Freshness::Fresh);
    }
    let (food, water) = info.values(decay_start, now, rules.decay_floor);
    if food < 0.0 && water < 0.0 {
        Ok(Freshness::Rotting)
    } else if food < rules.ripe_fraction * info.food {
        Ok(Freshness::Ripe)
    } else if food < rules.decomposing_fraction * info.food {
        Ok(Freshness::Decomposing)
    } else {
        Ok(Freshness::Fresh)
    }
}

/// `Eatable.SetDecomposes(value)`: disabling clears `timeDecayStart` to 0;
/// enabling sets `timeDecayStart` to `now` only when it is 0.
pub fn set_decomposes(
    info: &mut EatableInfo,
    decay_start: &mut f64,
    decomposes: bool,
    now: f64,
) -> Result<(), RuleError> {
    info.validate()?;
    if !finite(&[*decay_start, now]) {
        return Err(RuleError("invalid decay time"));
    }
    if !decomposes {
        *decay_start = 0.0;
    } else if *decay_start == 0.0 {
        *decay_start = now;
    }
    info.decomposes = decomposes;
    Ok(())
}

/// One `Eatable.IterateDespawn` check at game time `now`. `active` is
/// `gameObject.activeSelf` (true in the world, false in any container).
/// Returns `Ok(true)` when the object should be destroyed
/// (`now - timeDespawnStart > despawnDelay`).
pub fn iterate_despawn(
    info: &EatableInfo,
    decay_start: f64,
    state: &mut DespawnState,
    active: bool,
    now: f64,
    rules: &DecayRules,
) -> Result<bool, RuleError> {
    if !finite(&[state.despawn_start]) {
        return Err(RuleError("invalid despawn state"));
    }
    let rotten = is_rotten(info, decay_start, now, rules)?;
    if !info.decomposes || !rotten {
        return Ok(false);
    }
    if !active {
        state.was_active = false;
        return Ok(false);
    }
    if !state.was_active {
        state.despawn_start = now;
        state.was_active = true;
    }
    Ok(now - state.despawn_start > info.despawn_delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> DecayRules {
        DecayRules {
            decay_floor: -25.0,
            ripe_fraction: 0.5,
            decomposing_fraction: 0.9,
            despawn_check_interval: 5.0,
            default_despawn_delay: 300.0,
        }
    }

    fn fish() -> EatableInfo {
        EatableInfo {
            food: 20.0,
            water: 10.0,
            decomposes: true,
            decay_rate: 1.0,
            despawns: true,
            despawn_delay: 300.0,
        }
    }

    #[test]
    fn exact_half_and_nine_tenths_freshness_boundaries_and_floor() {
        let r = rules();
        let e = fish();
        // Base food = 20, water = 10, rate = 1/s from t = 0:
        // At t = 2: food = 18 == 0.9 * 20 -> still Fresh (strictly `<` for Decomposing).
        assert_eq!(freshness(&e, 0.0, 2.0, true, &r).unwrap(), Freshness::Fresh);
        assert_eq!(Freshness::Fresh.language_key(), None);
        // Just past t = 2: food < 18 -> Decomposing.
        assert_eq!(
            freshness(&e, 0.0, 2.0001, true, &r).unwrap(),
            Freshness::Decomposing
        );
        assert_eq!(Freshness::Decomposing.language_key(), Some("Decomposing"));
        // At t = 10: food = 10 == 0.5 * 20 -> still Decomposing (strictly `<` for Ripe).
        assert_eq!(
            freshness(&e, 0.0, 10.0, true, &r).unwrap(),
            Freshness::Decomposing
        );
        // Just past t = 10: food < 10 -> Ripe.
        assert_eq!(
            freshness(&e, 0.0, 10.0001, true, &r).unwrap(),
            Freshness::Ripe
        );
        assert_eq!(Freshness::Ripe.language_key(), Some("Ripe"));
        // At t = 20.5: food = -0.5 (< 0), water = -10.5 (< 0) -> Rotting.
        // Wait: if water were still >= 0, it would stay Ripe, not Rotting!
        let watery = EatableInfo {
            food: 10.0,
            water: 15.0,
            ..fish()
        };
        // At t = 12: food = -2 (< 0), water = 3 (>= 0) -> Ripe, not Rotting.
        assert!(!is_rotten(&watery, 0.0, 12.0, &r).unwrap());
        assert_eq!(
            freshness(&watery, 0.0, 12.0, true, &r).unwrap(),
            Freshness::Ripe
        );
        // At t = 15: food = -5, water = 0 -> still Ripe (must be strictly < 0).
        assert!(!is_rotten(&watery, 0.0, 15.0, &r).unwrap());
        // Just past t = 15: both < 0 -> Rotting.
        assert!(is_rotten(&watery, 0.0, 15.0001, &r).unwrap());
        assert_eq!(
            freshness(&watery, 0.0, 15.0001, true, &r).unwrap(),
            Freshness::Rotting
        );
        assert_eq!(Freshness::Rotting.language_key(), Some("Rotting"));
        // Survival off or non-decomposing -> Fresh.
        assert_eq!(
            freshness(&watery, 0.0, 100.0, false, &r).unwrap(),
            Freshness::Fresh
        );
        // Floor at -25 for both stats independently.
        assert_eq!(watery.values(0.0, 38.0, r.decay_floor), (-25.0, -23.0));
        assert_eq!(watery.values(0.0, 1000.0, r.decay_floor), (-25.0, -25.0));
    }

    #[test]
    fn inactive_rotten_retention_versus_active_deadline_and_disabling_decomposition() {
        let r = rules();
        let mut e = fish();
        let mut start = 10.0;
        let mut state = DespawnState::default();
        // Rotten at t = 35 (food = -5, water = -15).
        assert!(is_rotten(&e, start, 35.0, &r).unwrap());
        // Inactive (stored in inventory/locker): never despawns, even after 10,000 s.
        assert!(!iterate_despawn(&e, start, &mut state, false, 35.0, &r).unwrap());
        assert!(!iterate_despawn(&e, start, &mut state, false, 10_000.0, &r).unwrap());
        assert!(!state.was_active);
        // Dropped into the active world at t = 100: starts timer at 100.
        assert!(!iterate_despawn(&e, start, &mut state, true, 100.0, &r).unwrap());
        assert!(state.was_active);
        assert_eq!(state.despawn_start, 100.0);
        // Picked back up (inactive) at t = 250: resets `was_active`, preserves `decay_start`.
        assert!(!iterate_despawn(&e, start, &mut state, false, 250.0, &r).unwrap());
        assert!(!state.was_active);
        // Dropped again at t = 500: timer restarts from 500, not 100.
        assert!(!iterate_despawn(&e, start, &mut state, true, 500.0, &r).unwrap());
        assert_eq!(state.despawn_start, 500.0);
        // Exactly at deadline (500 + 300 = 800): strictly `>`, so not destroyed yet.
        assert!(!iterate_despawn(&e, start, &mut state, true, 800.0, &r).unwrap());
        // Past deadline: destroyed.
        assert!(iterate_despawn(&e, start, &mut state, true, 800.001, &r).unwrap());

        // Disabling decomposition clears `decay_start` to 0 and restores base values;
        // re-enabling when 0 sets `decay_start` to `now`, and when non-zero keeps it.
        set_decomposes(&mut e, &mut start, false, 900.0).unwrap();
        assert!(!e.decomposes);
        assert_eq!(start, 0.0);
        assert_eq!(e.values(start, 900.0, r.decay_floor), (20.0, 10.0));
        assert!(!iterate_despawn(&e, start, &mut state, true, 900.0, &r).unwrap());
        set_decomposes(&mut e, &mut start, true, 950.0).unwrap();
        assert!(e.decomposes);
        assert_eq!(start, 950.0);
        set_decomposes(&mut e, &mut start, true, 1200.0).unwrap();
        assert_eq!(start, 950.0);
    }

    #[test]
    fn missing_or_invalid_decay_rules_refuse() {
        let e = fish();
        for bad in [
            DecayRules {
                decay_floor: 1.0,
                ..rules()
            },
            DecayRules {
                ripe_fraction: 0.95,
                ..rules()
            },
            DecayRules {
                decomposing_fraction: 1.5,
                ..rules()
            },
            DecayRules {
                despawn_check_interval: 0.0,
                ..rules()
            },
            DecayRules {
                default_despawn_delay: f64::NAN,
                ..rules()
            },
        ] {
            assert!(freshness(&e, 0.0, 1.0, true, &bad).is_err());
            assert!(is_rotten(&e, 0.0, 1.0, &bad).is_err());
        }
        assert!(freshness(&e, f64::NAN, 1.0, true, &rules()).is_err());
    }
}
