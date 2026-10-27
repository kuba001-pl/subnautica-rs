//! Survival (M9s, `docs/DESIGN.md` "M9s plan"): food and water
//! (`Survival.UpdateHunger`, `UpdateStats`, `ResetStats`,
//! `OnHealTempDamage`, M9s1). Our own code after reading the game's.
//!
//! Unlike the older modules, no balance number is ported here: every rule
//! input is a typed value the caller loads from the player's install
//! (`sn-assets`' `hunger_rules` reads them from the game's code), and a
//! missing or invalid input refuses explicitly without changing state.
//! Nothing here runs in live play yet (M9s8/M9s11 wire it in).

mod hunger;

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
