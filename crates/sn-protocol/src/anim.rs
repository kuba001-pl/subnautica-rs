//! The player animator's values on the wire: a fixed table of the
//! parameters `sn_sim::body` gives the animator (M9g). A value's index in
//! its table is its slot on the wire; changing a table changes
//! [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION).

use crate::quant::{fixed, fixed_back};

/// Float parameters and their wire scale (steps per unit; `i16`, so the
/// range is ±32,767 / scale).
pub const FLOATS: [(&str, f64); 7] = [
    // Metres per second, 1 mm/s steps (±32.8 m/s).
    ("move_speed", 1000.0),
    ("move_speed_x", 1000.0),
    ("move_speed_y", 1000.0),
    ("move_speed_z", 1000.0),
    // Degrees, 0.01° steps.
    ("view_pitch", 100.0),
    // Degrees per second, 0.1°/s steps (±3,277°/s; a faster flick
    // saturates).
    ("view_turn", 10.0),
    // Metres, 0.1 mm steps (the landing bob).
    ("verticalOffset", 10_000.0),
];

/// Bool parameters, one bit each.
pub const BOOLS: [&str; 19] = [
    "grab",
    "bash",
    "is_underwater",
    "cinematics_enabled",
    "using_tool",
    "using_tool_alt",
    "holding_tool",
    "in_seamoth",
    "in_exosuit",
    "cyclops_steering",
    "bleeder",
    "jump",
    "diving",
    "diving_land",
    "on_surface",
    "holding_welder",
    "using_pda",
    "using_builder",
    "vr_active",
];

/// Triggers (`SetTrigger`), sent as events by index.
pub const TRIGGERS: [&str; 3] = [
    "player_death",
    "player_death_fire",
    "player_death_explosion",
];

/// Bits of [`AnimState::bools`] that name a parameter.
pub const BOOL_MASK: u32 = (1 << BOOLS.len()) - 1;

/// The animator's values at one tick, quantized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimState {
    /// [`FLOATS`] in order, each `round(value · scale)`.
    pub floats: [i16; FLOATS.len()],
    /// Bit `i` is [`BOOLS`]`[i]`; no other bit is set.
    pub bools: u32,
}

impl AnimState {
    /// Sets a parameter by its animator name; `false` if the table has
    /// no such parameter of that kind (the caller logs it).
    pub fn set_float(&mut self, name: &str, value: f64) -> bool {
        match FLOATS.iter().position(|(n, _)| *n == name) {
            Some(i) => {
                self.floats[i] = fixed(value, FLOATS[i].1);
                true
            }
            None => false,
        }
    }

    pub fn set_bool(&mut self, name: &str, value: bool) -> bool {
        match BOOLS.iter().position(|n| *n == name) {
            Some(i) => {
                if value {
                    self.bools |= 1 << i;
                } else {
                    self.bools &= !(1 << i);
                }
                true
            }
            None => false,
        }
    }

    /// Every float parameter as the receiver's animator gets it.
    pub fn floats(&self) -> impl Iterator<Item = (&'static str, f64)> + '_ {
        FLOATS
            .iter()
            .zip(self.floats)
            .map(|(&(name, scale), v)| (name, fixed_back(v, scale)))
    }

    pub fn bools(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        BOOLS
            .iter()
            .enumerate()
            .map(|(i, &name)| (name, self.bools & (1 << i) != 0))
    }
}

/// A trigger's slot, or `None` if the table has no such trigger.
pub fn trigger_index(name: &str) -> Option<u8> {
    TRIGGERS.iter().position(|n| *n == name).map(|i| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_values_come_back_quantized() {
        let mut all: Vec<&str> = FLOATS.iter().map(|f| f.0).collect();
        all.extend(BOOLS);
        all.extend(TRIGGERS);
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n);

        let mut a = AnimState::default();
        assert!(a.set_float("move_speed_z", 7.224_9));
        assert!(a.set_float("view_pitch", -87.0));
        assert!(a.set_float("view_turn", 1e9));
        assert!(a.set_bool("diving", true));
        assert!(a.set_bool("vr_active", true));
        assert!(a.set_bool("vr_active", false));
        assert!(!a.set_float("diving", 1.0));
        assert!(!a.set_bool("no_such", true));
        let f: Vec<_> = a.floats().collect();
        assert_eq!(f[3], ("move_speed_z", 7.225));
        assert_eq!(f[4], ("view_pitch", -87.0));
        assert_eq!(f[5], ("view_turn", 3276.7));
        let on: Vec<_> = a.bools().filter(|b| b.1).map(|b| b.0).collect();
        assert_eq!(on, ["diving"]);
        assert_eq!(a.bools & !BOOL_MASK, 0);
        assert_eq!(trigger_index("player_death_fire"), Some(1));
        assert_eq!(trigger_index("jump"), None);
    }
}
