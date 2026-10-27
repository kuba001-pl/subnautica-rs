//! Survival (M9s, `docs/DESIGN.md` "M9s plan"): food and water
//! (`Survival.UpdateHunger`, `UpdateStats`, `ResetStats`,
//! `OnHealTempDamage`, M9s1); eating, drinking, first aid and a new
//! object's component state (`consume`, M9s2; the state itself is
//! [`crate::inventory::ItemState`]). Our own code after reading the game's.
//!
//! Unlike the older modules, no balance number is ported here: every rule
//! input is a typed value the caller loads from the player's install
//! (`sn-assets`' `hunger_rules` reads them from the game's code), and a
//! missing or invalid input refuses explicitly without changing state.
//! Nothing here runs in live play yet (M9s8/M9s11 wire it in).

mod consume;
mod decay;
mod energy;
mod equipment;
mod hunger;
pub use equipment::{
    EquipmentRules, OxygenId, OxygenSource, OxygenSources, SwimCharge, SwimEquipment,
    restore_oxygen, swim_charge, swim_max_speed,
};

pub use consume::{
    ConsumeRules, EatableInfo, Eaten, Refused, Used, UsedItem, add_health, eat, eat_item,
    initial_state, use_item, use_tech,
};
pub use decay::{
    DecayRules, DespawnState, Freshness, freshness, is_rotten, iterate_despawn, set_decomposes,
};
pub use energy::{
    EnergyMixinInfo, EnergyRules, ReloadOutcome, ReloadRefused, add_energy, consume_energy,
    energy_capacity, energy_charge, energy_scalar, make_battery, modify_charge, restore_battery,
    select_battery, set_battery_charge, sort_reload_candidates, spawn_default_battery,
};
pub use hunger::{HungerEffect, HungerRules, Stat, Survival, Warning, crossing};

/// Invalid or missing runtime rule input, or invalid state. No mutation
/// occurs on refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleError(pub &'static str);

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for RuleError {}

/// All finite and at least 0.
pub(crate) fn nonnegative(values: &[f64]) -> bool {
    values.iter().all(|x| x.is_finite() && *x >= 0.0)
}

/// All finite.
pub(crate) fn finite(values: &[f64]) -> bool {
    values.iter().all(|x| x.is_finite())
}

mod quickslots;
pub use quickslots::{
    ArmsState, QuickEvent, QuickItem, QuickRules, QuickSlots, QuickUpdate, ToolTiming,
};
