//! Food and water (M9s1): `Survival.Start` schedules `UpdateHunger` with
//! `InvokeRepeating(…, 0, interval)` (a call at once, then one every
//! interval) and calls `ResetStats`; each call, when the game mode
//! requires survival and the stats are not frozen, runs `UpdateStats`
//! (drain, starvation, warnings), then `LiveMixin.TakeDamage` (Starve) and
//! the regeneration's `AddHealth`. Freezing suppresses the calls' effects,
//! not their schedule; a respawn resets the stats, not the schedule.

use super::{RuleError, finite, nonnegative};

/// The rules, every one from the install (no defaults here).
#[derive(Clone, Debug, PartialEq)]
pub struct HungerRules {
    /// `Survival.kUpdateHungerInterval`: seconds between calls.
    pub interval: f64,
    /// Seconds for food (`kFoodTime`) and water (`kWaterTime`) to lose
    /// `stat_max` units.
    pub food_time: f64,
    pub water_time: f64,
    /// `kMaxStat`: the drain's scale and water's cap.
    pub stat_max: f64,
    /// `kMaxOverfillStat`: food's cap.
    pub food_max: f64,
    /// `kStarveDamage`: damage per unit a stat would go below 0.
    pub starve_damage: f64,
    /// `kFoodWaterHealThreshold`: food + water at least this regenerates.
    pub heal_threshold: f64,
    /// `kBaseRegenRate`: health per second of the interval.
    pub regen_per_second: f64,
    /// `ResetStats`: a new game's and a respawned player's stats.
    pub start_food: f64,
    pub start_water: f64,
    /// The warning thresholds (`kLow…Threshold`, `kCritical…Threshold`;
    /// the same for food and water).
    pub low: f64,
    pub critical: f64,
    /// `kFoodPerTempDamage`: food per unit of temporary damage healed.
    pub food_per_temp_damage: f64,
}

impl HungerRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        let ok = nonnegative(&[
            self.interval,
            self.food_time,
            self.water_time,
            self.stat_max,
            self.food_max,
            self.starve_damage,
            self.heal_threshold,
            self.regen_per_second,
            self.start_food,
            self.start_water,
            self.low,
            self.critical,
            self.food_per_temp_damage,
        ]) && self.interval > 0.0
            && self.food_time > 0.0
            && self.water_time > 0.0
            && self.critical <= self.low
            && self.stat_max <= self.food_max
            && self.start_food <= self.food_max
            && self.start_water <= self.stat_max;
        if ok {
            Ok(())
        } else {
            Err(RuleError("invalid hunger rules"))
        }
    }
}

/// Which stat a warning is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    Food,
    Water,
}

/// `Survival.UpdateWarningSounds`' index into the stat's notifications:
/// 0 low, 1 critical, 2 empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    Low,
    Critical,
    Empty,
}

/// `Survival.UpdateWarningSounds`: at most one warning for a change from
/// `previous` to `value`; reaching 0 first, then going strictly below
/// `critical`, then strictly below `low`.
pub fn crossing(previous: f64, value: f64, low: f64, critical: f64) -> Option<Warning> {
    if value <= 0.0 && previous > 0.0 {
        Some(Warning::Empty)
    } else if value < critical && previous >= critical {
        Some(Warning::Critical)
    } else if value < low && previous >= low {
        Some(Warning::Low)
    } else {
        None
    }
}

/// What one call asks of the rest of the game, in the game's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HungerEffect {
    /// A notification of `UpdateStats` (food's before water's).
    Warning(Stat, Warning),
    /// `LiveMixin.TakeDamage(…, DamageType.Starve)`.
    Damage(f64),
    /// `LiveMixin.AddHealth`.
    Heal(f64),
}

/// The player's food and water, and the schedule's phase.
#[derive(Clone, Debug, PartialEq)]
pub struct Survival {
    pub food: f64,
    pub water: f64,
    /// Seconds since the last scheduled call, in `[0, interval)`.
    pub phase: f64,
}

/// How close (in seconds) to a call's time counts as reaching it, so that
/// many small steps summing to an interval reach it as one step does.
const PHASE_EPSILON: f64 = 1e-9;

/// Most calls one [`Survival::advance`] may run.
const MAX_CALLS: f64 = 1_000_000.0;

impl Survival {
    /// `Survival.Start`: the stats reset, then the call at once (the
    /// scheduled call runs on a later frame than `ResetStats`). Returns the
    /// state and that call's effects.
    pub fn start(
        r: &HungerRules,
        requires_survival: bool,
        frozen: bool,
    ) -> Result<(Survival, Vec<HungerEffect>), RuleError> {
        r.validate()?;
        let mut s = Survival {
            food: r.start_food,
            water: r.start_water,
            phase: 0.0,
        };
        let mut effects = Vec::new();
        s.call(r, requires_survival && !frozen, &mut effects);
        Ok((s, effects))
    }

    /// `Survival.ResetStats` (`OnRespawn`): the stats, not the phase.
    pub fn respawn(&mut self, r: &HungerRules) -> Result<(), RuleError> {
        r.validate()?;
        self.food = r.start_food;
        self.water = r.start_water;
        Ok(())
    }

    /// `Survival.OnHealTempDamage`: healing temporary damage costs food.
    pub fn heal_temp_damage(&mut self, r: &HungerRules, amount: f64) -> Result<(), RuleError> {
        r.validate()?;
        if !finite(&[amount, self.food]) {
            return Err(RuleError("invalid temporary-damage healing"));
        }
        self.food = (self.food - amount * r.food_per_temp_damage).clamp(0.0, r.food_max);
        Ok(())
    }

    fn check(&self, r: &HungerRules) -> Result<(), RuleError> {
        r.validate()?;
        if !finite(&[self.food, self.water])
            || !nonnegative(&[self.phase])
            || self.phase >= r.interval
        {
            return Err(RuleError("invalid survival state"));
        }
        Ok(())
    }

    /// One `UpdateHunger`; `effective` is "survival required and not
    /// frozen".
    fn call(&mut self, r: &HungerRules, effective: bool, out: &mut Vec<HungerEffect>) {
        if !effective {
            return;
        }
        // `UpdateStats(interval)`.
        let mut damage = 0.0;
        let (food, water) = (self.food, self.water);
        let food_loss = r.interval / r.food_time * r.stat_max;
        if food_loss > food {
            damage += (food_loss - food) * r.starve_damage;
        }
        self.food = (food - food_loss).clamp(0.0, r.food_max);
        let water_loss = r.interval / r.water_time * r.stat_max;
        if water_loss > water {
            damage += (water_loss - water) * r.starve_damage;
        }
        self.water = (water - water_loss).clamp(0.0, r.stat_max);
        for (stat, before, after) in [
            (Stat::Food, food, self.food),
            (Stat::Water, water, self.water),
        ] {
            if let Some(w) = crossing(before, after, r.low, r.critical) {
                out.push(HungerEffect::Warning(stat, w));
            }
        }
        // `num > float.Epsilon`: any positive damage.
        if damage > 0.0 {
            out.push(HungerEffect::Damage(damage));
        }
        // The gate reads the stats after the drain.
        if self.food + self.water >= r.heal_threshold {
            out.push(HungerEffect::Heal(r.regen_per_second * r.interval));
        }
    }

    /// `dt` seconds of game time: every scheduled call that comes due, in
    /// order. Refuses invalid input without changing state.
    pub fn advance(
        &mut self,
        r: &HungerRules,
        dt: f64,
        requires_survival: bool,
        frozen: bool,
    ) -> Result<Vec<HungerEffect>, RuleError> {
        self.check(r)?;
        if !nonnegative(&[dt]) {
            return Err(RuleError("invalid time step"));
        }
        let elapsed = self.phase + dt;
        let calls = ((elapsed + PHASE_EPSILON) / r.interval).floor();
        if !calls.is_finite() || calls > MAX_CALLS {
            return Err(RuleError("too many hunger intervals in one step"));
        }
        self.phase = (elapsed - calls * r.interval).max(0.0);
        let mut out = Vec::new();
        for _ in 0..calls as u64 {
            self.call(r, requires_survival && !frozen, &mut out);
        }
        Ok(out)
    }

    /// How many calls a step of `dt` would run (for logs and tests).
    pub fn calls_in(&self, r: &HungerRules, dt: f64) -> u64 {
        let calls = ((self.phase + dt + PHASE_EPSILON) / r.interval).floor();
        if calls.is_finite() && calls >= 0.0 {
            calls as u64
        } else {
            0
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic rule set for tests, not a balance default: our own
    /// round numbers, chosen to match the hand calculations in DESIGN's
    /// M9s1 row. Real runs load the install's.
    pub(crate) fn rules() -> HungerRules {
        HungerRules {
            interval: 10.0,
            food_time: 2520.0,
            water_time: 1800.0,
            stat_max: 100.0,
            food_max: 200.0,
            starve_damage: 25.0,
            heal_threshold: 150.0,
            regen_per_second: 1.0 / 24.0,
            start_food: 50.5,
            start_water: 90.5,
            low: 20.0,
            critical: 10.0,
            food_per_temp_damage: 0.25,
        }
    }

    fn at(food: f64, water: f64) -> Survival {
        Survival {
            food,
            water,
            phase: 0.0,
        }
    }

    fn damage(e: &[HungerEffect]) -> f64 {
        e.iter()
            .map(|e| match e {
                HungerEffect::Damage(d) => *d,
                _ => 0.0,
            })
            .sum()
    }

    fn healing(e: &[HungerEffect]) -> f64 {
        e.iter()
            .map(|e| match e {
                HungerEffect::Heal(h) => *h,
                _ => 0.0,
            })
            .sum()
    }

    #[test]
    fn sixty_seconds_drain_by_the_hand_calculation() {
        let r = rules();
        let mut a = at(100.0, 100.0);
        assert_eq!(a.calls_in(&r, 60.0), 6);
        a.advance(&r, 60.0, true, false).unwrap();
        // 6 × 10 / 2520 × 100 and 6 × 10 / 1800 × 100.
        assert!((100.0 - a.food - 2.380952).abs() < 1e-6, "{}", a.food);
        assert!((100.0 - a.water - 3.333333).abs() < 1e-6, "{}", a.water);
        assert_eq!(a.phase, 0.0);
    }

    #[test]
    fn partitioned_steps_keep_the_count_and_phase() {
        let r = rules();
        let mut one = at(100.0, 100.0);
        let mut many = one.clone();
        one.advance(&r, 60.02, true, false).unwrap();
        let mut calls = 0;
        for _ in 0..3001 {
            calls += many.calls_in(&r, 0.02);
            many.advance(&r, 0.02, true, false).unwrap();
        }
        assert_eq!(calls, 6);
        assert!((one.food - many.food).abs() < 1e-9);
        assert!((one.water - many.water).abs() < 1e-9);
        assert!((one.phase - many.phase).abs() < 1e-6, "{}", many.phase);
        assert!((many.phase - 0.02).abs() < 1e-6);
        // Uneven steps: the same.
        let mut odd = at(100.0, 100.0);
        let mut calls = 0;
        for dt in [3.0, 7.0, 0.0, 9.999, 0.001, 25.0, 15.02] {
            calls += odd.calls_in(&r, dt);
            odd.advance(&r, dt, true, false).unwrap();
        }
        assert_eq!(calls, 6);
        assert!((odd.food - one.food).abs() < 1e-9);
    }

    #[test]
    fn both_empty_starve_by_the_shortfall() {
        let r = rules();
        let mut s = at(0.0, 0.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        // (10/2520 + 10/1800) × 100 × 25.
        assert!((damage(&e) - 23.809524).abs() < 1e-6, "{e:?}");
        assert_eq!((s.food, s.water), (0.0, 0.0));
        // A partial shortfall: only what is missing.
        let mut s = at(0.2, 100.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert!((damage(&e) - (1000.0 / 2520.0 - 0.2) * 25.0).abs() < 1e-9);
        // Enough food and water: no damage effect at all.
        let mut s = at(50.0, 50.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert!(!e.iter().any(|e| matches!(e, HungerEffect::Damage(_))));
    }

    #[test]
    fn warnings_cross_exactly_and_once() {
        assert_eq!(crossing(21.0, 20.0, 20.0, 10.0), None);
        assert_eq!(crossing(20.0, 19.9, 20.0, 10.0), Some(Warning::Low));
        assert_eq!(crossing(11.0, 10.0, 20.0, 10.0), None);
        assert_eq!(crossing(10.0, 9.9, 20.0, 10.0), Some(Warning::Critical));
        // Over both thresholds at once: the more severe only.
        assert_eq!(crossing(25.0, 5.0, 20.0, 10.0), Some(Warning::Critical));
        assert_eq!(crossing(21.0, 0.0, 20.0, 10.0), Some(Warning::Empty));
        assert_eq!(crossing(0.0, 0.0, 20.0, 10.0), None);
        // Through the rules: food first, then water, then damage.
        let r = rules();
        let mut s = at(20.2, 20.3);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert_eq!(
            e,
            [
                HungerEffect::Warning(Stat::Food, Warning::Low),
                HungerEffect::Warning(Stat::Water, Warning::Low)
            ]
        );
        let mut s = at(0.1, 50.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert_eq!(e[0], HungerEffect::Warning(Stat::Food, Warning::Empty));
        assert!(matches!(e[1], HungerEffect::Damage(_)));
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn the_healing_gate_reads_the_stats_after_the_drain() {
        let r = rules();
        let loss = 1000.0 / 2520.0 + 1000.0 / 1800.0;
        // 150 after the drain (to rounding) heals 10/24.
        let mut s = at(100.0, 50.0 + loss + 1e-9);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert!((s.food + s.water - 150.0).abs() < 1e-8);
        assert!((healing(&e) - 10.0 / 24.0).abs() < 1e-12, "{e:?}");
        // 150 before, below after: no healing.
        let mut s = at(100.0, 50.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert_eq!(healing(&e), 0.0);
        // Overfilled food counts towards the gate.
        let mut s = at(180.0, 0.0);
        let e = s.advance(&r, 10.0, true, false).unwrap();
        assert!(healing(&e) > 0.0 && damage(&e) > 0.0);
        // Damage comes before healing.
        assert!(matches!(e[0], HungerEffect::Damage(_)));
        assert!(matches!(e[1], HungerEffect::Heal(_)));
    }

    #[test]
    fn start_freeze_mode_and_respawn() {
        let r = rules();
        // Start: reset stats, then a call at once.
        let (mut s, e) = Survival::start(&r, true, false).unwrap();
        assert!(e.is_empty());
        assert!((s.food - (50.5 - 1000.0 / 2520.0)).abs() < 1e-12);
        let (frozen, _) = Survival::start(&r, true, true).unwrap();
        assert_eq!((frozen.food, frozen.water), (50.5, 90.5));
        // Frozen or no survival: the phase runs, the stats do not.
        let before = (s.food, s.water);
        let e = s.advance(&r, 15.0, true, true).unwrap();
        assert!(e.is_empty());
        assert_eq!((s.food, s.water), before);
        assert_eq!(s.phase, 5.0);
        s.advance(&r, 10.0, false, false).unwrap();
        assert_eq!((s.food, s.water), before);
        assert_eq!(s.phase, 5.0);
        // Unfrozen, the next call comes 5 s later, not 10.
        s.advance(&r, 5.0, true, false).unwrap();
        assert!(s.food < before.0);
        // Respawn: the stats, not the phase.
        s.advance(&r, 3.0, true, false).unwrap();
        s.respawn(&r).unwrap();
        assert_eq!((s.food, s.water, s.phase), (50.5, 90.5, 3.0));
        s.heal_temp_damage(&r, 4.0).unwrap();
        assert_eq!(s.food, 49.5);
        s.heal_temp_damage(&r, 1000.0).unwrap();
        assert_eq!(s.food, 0.0);
    }

    #[test]
    fn invalid_input_refuses_without_mutation() {
        let r = rules();
        let mut s = at(40.0, 40.0);
        s.phase = 2.0;
        let before = s.clone();
        assert!(s.advance(&r, f64::NAN, true, false).is_err());
        assert!(s.advance(&r, -1.0, true, false).is_err());
        assert!(s.advance(&r, f64::INFINITY, true, false).is_err());
        assert!(s.advance(&r, 1e12, true, false).is_err());
        assert!(s.heal_temp_damage(&r, f64::NAN).is_err());
        for broken in [
            HungerRules {
                interval: 0.0,
                ..rules()
            },
            HungerRules {
                food_time: f64::NAN,
                ..rules()
            },
            HungerRules {
                critical: 30.0,
                ..rules()
            },
            HungerRules {
                start_water: 150.0,
                ..rules()
            },
        ] {
            assert!(s.advance(&broken, 10.0, true, false).is_err());
            assert!(s.respawn(&broken).is_err());
            assert!(Survival::start(&broken, true, false).is_err());
        }
        assert_eq!(s, before);
        s.phase = 10.0;
        assert!(s.advance(&r, 1.0, true, false).is_err());
    }
}
