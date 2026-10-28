//! Borrowed oxygen sources and ordered swimming equipment effects (M9s4).
//! Inputs are supplied by runtime readers; no balance defaults live here.
use super::{RuleError, modify_charge, nonnegative};
use crate::{
    V3,
    inventory::{EnergySlot, ItemId, TechType},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OxygenId {
    Player,
    Item(ItemId),
}

/// A registration borrows the authoritative item/reserve amount. Unregistering
/// or dropping the manager cannot reset or duplicate that amount.
pub struct OxygenSource<'a> {
    pub id: OxygenId,
    pub breathable: bool,
    pub capacity: f64,
    pub available: &'a mut f64,
}

#[derive(Default)]
pub struct OxygenSources<'a> {
    sources: Vec<OxygenSource<'a>>,
}
impl<'a> OxygenSources<'a> {
    pub fn register(&mut self, source: OxygenSource<'a>) -> Result<bool, RuleError> {
        if !nonnegative(&[source.capacity, *source.available])
            || *source.available > source.capacity
        {
            return Err(RuleError("invalid oxygen source"));
        }
        if self.sources.iter().any(|s| s.id == source.id) {
            return Ok(false);
        }
        self.sources.push(source);
        // Equal priority order is our registration order, not a proven game rule.
        self.sources.sort_by_key(|s| match (s.id, s.breathable) {
            (OxygenId::Player, _) => 0,
            (_, true) => 1,
            (_, false) => 2,
        });
        Ok(true)
    }
    pub fn unregister(&mut self, id: OxygenId) -> Option<OxygenSource<'a>> {
        let at = self.sources.iter().position(|s| s.id == id)?;
        Some(self.sources.remove(at))
    }
    pub fn total(&self) -> (f64, f64) {
        self.sources
            .iter()
            .filter(|s| s.breathable)
            .fold((0.0, 0.0), |(a, c), s| (a + *s.available, c + s.capacity))
    }
    pub fn fill(&mut self, amount: f64) -> Result<f64, RuleError> {
        if !nonnegative(&[amount]) {
            return Err(RuleError("invalid oxygen addition"));
        }
        let mut remaining = amount;
        for s in &mut self.sources {
            let n = remaining.min(s.capacity - *s.available);
            *s.available += n;
            remaining -= n;
        }
        Ok(amount - remaining)
    }
    pub fn drain(&mut self, amount: f64) -> Result<f64, RuleError> {
        if !nonnegative(&[amount]) {
            return Err(RuleError("invalid oxygen removal"));
        }
        let mut remaining = amount;
        for s in self.sources.iter_mut().rev().filter(|s| s.breathable) {
            let n = remaining.min(*s.available);
            *s.available -= n;
            remaining -= n;
        }
        Ok(amount - remaining)
    }
    /// Caller resolves per-source surface heights, water park, CanBreathe,
    /// and cinematic state; this is OxygenManager's final gated fill.
    pub fn surface_fill(
        &mut self,
        rules: &EquipmentRules,
        dt: f64,
        can_fill: bool,
        cinematic: bool,
    ) -> Result<f64, RuleError> {
        rules.validate()?;
        let amount = dt * rules.surface_fill;
        if !nonnegative(&[dt, amount]) {
            return Err(RuleError("invalid oxygen elapsed time"));
        }
        if can_fill && !cinematic {
            self.fill(amount)
        } else {
            Ok(0.0)
        }
    }
}

/// Oxygen.Awake/OnProtoDeserialize: negative sentinel fills; zero stays empty.
pub fn restore_oxygen(available: f64, capacity: f64) -> Result<f64, RuleError> {
    if !available.is_finite() || !nonnegative(&[capacity]) || available > capacity {
        return Err(RuleError("invalid oxygen state"));
    }
    Ok(if available < 0.0 { capacity } else { available })
}

#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentRules {
    pub tank_penalties: Vec<(TechType, f64)>,
    pub heavy_tank: TechType,
    pub spare_penalty: f64,
    pub minimum_speed: f64,
    pub reinforced_suit: TechType,
    pub suit_penalty: f64,
    pub fins_bonuses: Vec<(TechType, f64)>,
    pub held_penalty: f64,
    pub surface_multiplier: f64,
    pub modifier_transition: f64,
    pub surface_fill: f64,
    pub charge_speed_threshold: f64,
}
impl EquipmentRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        let mut numbers = vec![
            self.spare_penalty,
            self.minimum_speed,
            self.suit_penalty,
            self.held_penalty,
            self.surface_multiplier,
            self.modifier_transition,
            self.surface_fill,
            self.charge_speed_threshold,
        ];
        numbers.extend(
            self.tank_penalties
                .iter()
                .chain(&self.fins_bonuses)
                .map(|x| x.1),
        );
        if !nonnegative(&numbers) || self.minimum_speed == 0.0 {
            return Err(RuleError("invalid equipment rules"));
        }
        for list in [&self.tank_penalties, &self.fins_bonuses] {
            let mut seen = std::collections::BTreeSet::new();
            if list.iter().any(|(t, _)| !seen.insert(t)) {
                return Err(RuleError("duplicate equipment modifier"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SwimEquipment {
    pub tank: TechType,
    pub spare_heavy_tanks: u32,
    pub body: TechType,
    pub fins: TechType,
    pub held_tool: bool,
    pub seaglide: bool,
    pub above_water: bool,
}
/// Returns (altered maximum speed, new smoothed player multiplier).
/// Caller supplies the directional base speed used by the pure motor.
pub fn swim_max_speed(
    r: &EquipmentRules,
    e: SwimEquipment,
    base: f64,
    multiplier: f64,
    target: f64,
    dt: f64,
) -> Result<(f64, f64), RuleError> {
    r.validate()?;
    if !nonnegative(&[base, multiplier, target, dt]) {
        return Err(RuleError("invalid swimming input"));
    }
    let lookup = |list: &[(TechType, f64)], t| list.iter().find(|x| x.0 == t).map_or(0.0, |x| x.1);
    let mut speed =
        base - lookup(&r.tank_penalties, e.tank) - f64::from(e.spare_heavy_tanks) * r.spare_penalty;
    speed = speed.max(r.minimum_speed);
    if e.body == r.reinforced_suit {
        speed = (speed - r.suit_penalty).max(r.minimum_speed);
    }
    if !e.seaglide {
        speed += lookup(&r.fins_bonuses, e.fins);
        if e.held_tool {
            speed -= r.held_penalty;
        }
    }
    if e.above_water {
        speed *= r.surface_multiplier;
    }
    let next = multiplier
        + (target - multiplier).clamp(-r.modifier_transition * dt, r.modifier_transition * dt);
    let result = speed * next;
    if !result.is_finite() {
        return Err(RuleError("swimming overflow"));
    }
    Ok((result, next))
}

#[derive(Clone, Copy, Debug)]
pub struct SwimCharge {
    pub equipped: bool,
    pub underwater: bool,
    pub velocity: V3,
    /// Serialized UpdateSwimCharge.chargePerSecond, not its code initializer.
    pub rate: f64,
    pub dt: f64,
    pub requires_power: bool,
}
/// Returns actual energy accepted (also the sound gate). No held slot: zero.
pub fn swim_charge(
    r: &EquipmentRules,
    input: SwimCharge,
    held: Option<&mut EnergySlot>,
    capacity: impl Fn(TechType) -> Option<f32>,
) -> Result<f32, RuleError> {
    r.validate()?;
    let amount = input.rate * input.dt;
    let speed = input.velocity.length();
    if !nonnegative(&[input.rate, input.dt, amount, speed]) || amount > f64::from(f32::MAX) {
        return Err(RuleError("invalid charging input"));
    }
    if !input.equipped || !input.underwater || speed <= r.charge_speed_threshold {
        return Ok(0.0);
    }
    match held {
        Some(slot) => modify_charge(slot, amount as f32, input.requires_power, capacity),
        None => Ok(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Deliberately synthetic balance values, not the game's.
    fn rules() -> EquipmentRules {
        EquipmentRules {
            tank_penalties: vec![(10, 0.7)],
            heavy_tank: 11,
            spare_penalty: 1.1,
            minimum_speed: 2.0,
            reinforced_suit: 12,
            suit_penalty: 0.8,
            fins_bonuses: vec![(13, 1.5)],
            held_penalty: 0.6,
            surface_multiplier: 1.4,
            modifier_transition: 0.2,
            surface_fill: 7.0,
            charge_speed_threshold: 2.0,
        }
    }
    #[test]
    fn swap_retains_individual_oxygen_and_spare_never_contributes() {
        let (mut reserve, mut a, mut b) = (4.0, 9.0, 3.0);
        let mut sources = OxygenSources::default();
        sources
            .register(OxygenSource {
                id: OxygenId::Item(ItemId(1)),
                breathable: true,
                capacity: 15.0,
                available: &mut a,
            })
            .unwrap();
        sources
            .register(OxygenSource {
                id: OxygenId::Player,
                breathable: true,
                capacity: 8.0,
                available: &mut reserve,
            })
            .unwrap();
        assert_eq!(sources.total(), (13.0, 23.0));
        assert_eq!(sources.drain(11.0).unwrap(), 11.0);
        let a_source = sources.unregister(OxygenId::Item(ItemId(1))).unwrap();
        assert_eq!(*a_source.available, 0.0);
        sources
            .register(OxygenSource {
                id: OxygenId::Item(ItemId(2)),
                breathable: true,
                capacity: 15.0,
                available: &mut b,
            })
            .unwrap();
        assert_eq!(sources.total(), (5.0, 23.0));
        assert_eq!(sources.fill(7.0).unwrap(), 7.0); // player first: 2 -> 8, tank 3 -> 4
        assert_eq!(sources.total(), (12.0, 23.0));
        sources.unregister(OxygenId::Item(ItemId(2)));
        sources.register(a_source).unwrap();
        assert_eq!(sources.total(), (8.0, 23.0));
        assert_eq!(
            sources.surface_fill(&rules(), 1.0, true, true).unwrap(),
            0.0
        );
        assert_eq!(
            sources.surface_fill(&rules(), 1.0, true, false).unwrap(),
            7.0
        );
        drop(sources);
        assert_eq!((reserve, a, b), (8.0, 7.0, 4.0));
        assert_eq!(restore_oxygen(0.0, 15.0).unwrap(), 0.0);
        assert_eq!(restore_oxygen(-1.0, 15.0).unwrap(), 15.0);
    }
    #[test]
    fn nonbreathable_fills_but_never_drains_or_counts() {
        let mut amount = 0.0;
        let mut s = OxygenSources::default();
        s.register(OxygenSource {
            id: OxygenId::Item(ItemId(1)),
            breathable: false,
            capacity: 3.0,
            available: &mut amount,
        })
        .unwrap();
        assert_eq!(s.fill(9.0).unwrap(), 3.0);
        assert_eq!(s.total(), (0.0, 0.0));
        assert_eq!(s.drain(5.0).unwrap(), 0.0);
    }
    #[test]
    fn movement_modifiers_follow_the_game_order() {
        let r = rules();
        let e = SwimEquipment {
            tank: 10,
            spare_heavy_tanks: 2,
            body: 12,
            fins: 13,
            held_tool: true,
            above_water: true,
            ..Default::default()
        };
        // 4-.7-2.2=1.1 -> 2; suit -> 2; fins -> 3.5; hand -> 2.9; surface -> 4.06.
        let (speed, m) = swim_max_speed(&r, e, 4.0, 1.0, 1.0, 0.02).unwrap();
        assert!((speed - 4.06).abs() < 1e-10);
        assert_eq!(m, 1.0);
        assert_eq!(
            swim_max_speed(
                &r,
                SwimEquipment {
                    seaglide: true,
                    ..e
                },
                4.0,
                1.0,
                1.0,
                0.02
            )
            .unwrap()
            .0,
            2.8
        );
        let mut multiplier = 1.0;
        for _ in 0..10 {
            multiplier = swim_max_speed(&r, e, 4.0, multiplier, 0.5, 0.1).unwrap().1;
        }
        assert!((multiplier - 0.8).abs() < 1e-10);
        assert!(swim_max_speed(&r, e, f64::NAN, 1.0, 1.0, 0.1).is_err());
    }
    #[test]
    fn charging_uses_actual_velocity_strict_threshold_and_capacity() {
        let r = rules();
        let mut battery = crate::inventory::Item::new(2, 3);
        battery.charge = Some(8.0);
        let mut slot = EnergySlot {
            battery: Some(Box::new(battery)),
        };
        let mut i = SwimCharge {
            equipped: true,
            underwater: true,
            velocity: V3::new(2.0, 0.0, 0.0),
            rate: 3.0,
            dt: 1.0,
            requires_power: true,
        };
        assert_eq!(
            swim_charge(&r, i, Some(&mut slot), |_| Some(10.0)).unwrap(),
            0.0
        );
        i.velocity.x = 2.001;
        assert_eq!(swim_charge(&r, i, None, |_| Some(10.0)).unwrap(), 0.0);
        assert_eq!(
            swim_charge(
                &r,
                SwimCharge {
                    underwater: false,
                    ..i
                },
                Some(&mut slot),
                |_| Some(10.0)
            )
            .unwrap(),
            0.0
        );
        assert_eq!(
            swim_charge(&r, i, Some(&mut slot), |_| Some(10.0)).unwrap(),
            2.0
        );
        assert_eq!(
            swim_charge(&r, i, Some(&mut slot), |_| Some(10.0)).unwrap(),
            0.0
        );
    }
}
