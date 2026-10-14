//! The HUD's rules (M9u6b1 on; `docs/formats/ui.md` § The HUD's bars),
//! ported from the decompiled scripts (read only; the code is ours). Each
//! frame [`Hud::update`] takes what the scripts read from the game
//! ([`HudInputs`]) and writes what they set into a [`UiState`]; `sn-assets`
//! reads the scripts' fields and nodes into a [`HudConfig`].
//!
//! - `uGUI_SceneHUD`: the bars and their background by game mode, shown
//!   while `IsActive`;
//! - `uGUI_OxygenBar`, `uGUI_HealthBar`, `uGUI_FoodBar`, `uGUI_WaterBar`:
//!   `SetValue` (the bar's value smoothed by `Mathf.SmoothDamp` over
//!   `PDA.deltaTime`, and its number), and the oxygen bar's two scrolling
//!   overlays. Their pulses, the icons' coin turn and punch are M9u6b2.
//!
//! A script runs only while its object is active: a hidden bar keeps its
//! value until it shows again.

use crate::bar::CircularBar;
use crate::state::{MaterialValues, UiState};
use crate::tween::smooth_damp;

/// `uGUI_SceneHUD`'s mode from the game mode's options
/// (`GameModeChanged`): 0 none, 1 oxygen and health over the double
/// background, 2 all four over the quad background.
pub fn hud_mode(no_survival: bool, no_oxygen: bool, no_aggression: bool) -> u8 {
    if no_survival {
        if no_oxygen && no_aggression { 0 } else { 1 }
    } else {
        2
    }
}

/// Which bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarKind {
    Oxygen,
    Health,
    Food,
    Water,
}

/// `uGUI_OxygenBar`'s overlays: scroll speeds and alphas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlays {
    pub speed: [f32; 2],
    pub alpha: [f32; 2],
}

/// One bar script's nodes and fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BarConfig {
    pub kind: BarKind,
    /// The script's object (`uGUI_SceneHUD.barOxygen`, …).
    pub root: usize,
    /// The `uGUI_CircularBar`'s node and fields.
    pub bar: usize,
    pub circular: CircularBar,
    /// The number's TMP text.
    pub text: usize,
    pub damp_speed: f32,
    /// The oxygen bar's.
    pub overlays: Option<Overlays>,
}

/// What the HUD's rules need of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct HudConfig {
    /// `uGUI_SceneHUD.backgroundBarsDouble`, `backgroundBarsQuad`.
    pub background_double: usize,
    pub background_quad: usize,
    /// Oxygen, health, food, water.
    pub bars: Vec<BarConfig>,
}

/// What the scripts read from the game, this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudInputs {
    /// `uGUI_SceneHUD.IsActive`: the main level, not the intro, not
    /// piloting, no cinematic, no camera drone.
    pub active: bool,
    /// [`hud_mode`].
    pub mode: u8,
    /// `Player.GetOxygenAvailable`, `GetOxygenCapacity`.
    pub oxygen: f32,
    pub oxygen_capacity: f32,
    /// `LiveMixin.health − tempDamage`, `maxHealth`.
    pub health: f32,
    pub max_health: f32,
    /// `Survival.food`, `water` (capacity 100 each).
    pub food: f32,
    pub water: f32,
    /// `PDA.deltaTime` (`Time.deltaTime` unless the PDA pauses the game).
    pub delta_time: f32,
    /// `Time.time`.
    pub time: f32,
}

/// One bar's script state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarState {
    pub curr: f32,
    pub vel: f32,
    /// The ints the text was last made from (oxygen: value and capacity;
    /// food, water: value), `None` before the first.
    pub cached: Option<(i32, i32)>,
    /// The text last set.
    pub text: Option<String>,
    /// `bar.value`, as last set.
    pub value: Option<f32>,
    /// The oxygen overlays as last set: `overlay1ST`, `overlay2ST`, and
    /// the two alphas.
    pub overlay: Option<([f32; 4], [f32; 4], [f32; 2])>,
}

/// The HUD's scripts' state.
#[derive(Clone, Debug, PartialEq)]
pub struct Hud {
    pub config: HudConfig,
    pub bars: Vec<BarState>,
    /// `uGUI_SceneHUD._active`.
    pub active: bool,
}

/// `Mathf.RoundToInt`: `Math.Round`, half to even.
fn round_to_int(v: f32) -> i32 {
    v.round_ties_even() as i32
}

/// `Mathf.CeilToInt`.
fn ceil_to_int(v: f32) -> i32 {
    v.ceil() as i32
}

/// `MathExtensions.EvaluateLine`: the line through (x1, y1) and (x2, y2)
/// at x.
pub fn evaluate_line(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let slope = (y2 - y1) / (x2 - x1);
    let intercept = y1 - slope * x1;
    slope * x + intercept
}

impl Hud {
    /// The scripts before their first frame: `uGUI_SceneHUD.Awake` hides
    /// every element (`_active` false); the bars' `curr` is 0.
    pub fn new(config: HudConfig) -> Hud {
        let bars = vec![BarState::default(); config.bars.len()];
        Hud {
            config,
            bars,
            active: false,
        }
    }

    /// One frame: `uGUI_SceneHUD.Update`, then each active bar's
    /// `LateUpdate`. The returned state holds every change the scripts
    /// have made so far (a state is the whole picture, not a delta).
    pub fn update(&mut self, inputs: &HudInputs) -> UiState {
        self.active = inputs.active;
        let mut out = UiState::default();
        let c = &self.config;
        let mode = inputs.mode;
        let show = |kind: BarKind| match kind {
            BarKind::Oxygen | BarKind::Health => mode >= 1,
            BarKind::Food | BarKind::Water => mode == 2,
        };
        out.active
            .insert(c.background_double, self.active && mode == 1);
        out.active
            .insert(c.background_quad, self.active && mode == 2);
        for (b, s) in c.bars.iter().zip(self.bars.iter_mut()) {
            let shown = self.active && show(b.kind);
            out.active.insert(b.root, shown);
            if shown {
                late_update(b, s, inputs);
            }
            write(b, s, &mut out);
        }
        out
    }
}

/// A bar's `LateUpdate`: `SetValue`.
fn late_update(b: &BarConfig, s: &mut BarState, i: &HudInputs) {
    let dt = i.delta_time;
    let damp = |s: &mut BarState, target: f32| {
        let mut vel = s.vel;
        s.curr = smooth_damp(s.curr, target, &mut vel, b.damp_speed, f32::INFINITY, dt);
        s.vel = vel;
        s.value = Some(s.curr);
    };
    match b.kind {
        BarKind::Oxygen => {
            let (has, capacity) = (i.oxygen, i.oxygen_capacity);
            let ints = (round_to_int(has), round_to_int(capacity));
            damp(s, has / capacity);
            if s.cached != Some(ints) {
                s.cached = Some(ints);
                s.text = Some(ints.0.to_string());
            }
            if let Some(o) = b.overlays {
                // `overlay1ST.w = −Time.time × overlay1Speed` (and 2); the
                // alphas from the oxygen fraction (the setters clamp).
                let mut st1 = b.circular.overlay1_st;
                st1[3] = -i.time * o.speed[0];
                let mut st2 = b.circular.overlay2_st;
                st2[3] = -i.time * o.speed[1];
                let k = evaluate_line(0.5, 1.0, 1.0, 0.0, has / capacity).clamp(0.0, 1.0);
                let a = [
                    (k * o.alpha[0]).clamp(0.0, 1.0),
                    (k * o.alpha[1]).clamp(0.0, 1.0),
                ];
                s.overlay = Some((st1, st2, a));
            }
        }
        BarKind::Health => {
            let (has, capacity) = (i.health, i.max_health);
            damp(s, (has / capacity).clamp(0.0, 1.0));
            s.text = Some(ceil_to_int(s.curr * capacity).to_string());
        }
        BarKind::Food | BarKind::Water => {
            let has = if b.kind == BarKind::Food {
                i.food
            } else {
                i.water
            };
            let capacity = 100.0;
            damp(s, has / capacity);
            let n = ceil_to_int(s.curr * capacity);
            if s.cached.map(|c| c.0) != Some(n) {
                s.cached = Some((n, 0));
                s.text = Some(n.to_string());
            }
        }
    }
}

/// What a bar's scripts have set so far, into the state: the text, the
/// bar's `_Value` and the oxygen overlays.
fn write(b: &BarConfig, s: &BarState, out: &mut UiState) {
    if let Some(t) = &s.text {
        out.text.insert(b.text, t.clone());
    }
    let mut m = MaterialValues::default();
    if let Some(v) = s.value {
        m.floats.insert("_Value".into(), b.circular.value(v));
    }
    if let Some((st1, st2, a)) = s.overlay {
        m.vectors.insert("_Overlay1_ST".into(), st1);
        m.vectors.insert("_Overlay2_ST".into(), st2);
        m.vectors
            .insert("_OverlayAlpha".into(), [a[0], a[1], 0.0, 0.0]);
    }
    if !m.is_empty() {
        out.material.insert(b.bar, m);
    }
}

#[cfg(test)]
mod tests;
