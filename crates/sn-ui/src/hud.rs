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
//!   overlays;
//! - the pulses (M9u6b2): each bar's `pulseTween` (a looping
//!   `CoroutineTween` of `pulseDelay + pulseTime` from its two curves at
//!   `1 − value / pulseReferenceCapacity`) sets its legacy clip's
//!   `normalizedTime`, and the clip is sampled each frame
//!   (`OxygenBarPulse`, `IconPulse`: scale and `CanvasGroup` alpha);
//! - the health, food and water icons (M9u6b2): the punch (a 2.5 s
//!   `CoroutineTween` of `Oscillation` added to the icon's scale, on
//!   healing, eating, drinking) and the coin turn (`CoinRotation` to 180°
//!   while the PDA is open, with a random kick each time it opens or
//!   closes).
//!
//! The frame's order is Unity's: `uGUI_SceneHUD.Update` shows the bars
//! (`OnEnable`), the game's events (`onHealDamage`, `onEat`, `onDrink`),
//! the coroutines resume (`yield null`), the legacy animation is sampled,
//! then each bar's `LateUpdate`. A script runs only while its object is
//! active: a hidden bar keeps its value until it shows again.
//!
//! `UnityEngine.Random` (the punch's seed, the turn's kick) is not
//! reproduced: [`Hud`] draws from its own generator, in the same ranges.

use crate::bar::CircularBar;
use crate::clip::{Clip, Curve};
use crate::state::{MaterialValues, UiState};
use crate::tween::{CoroutineTween, TweenMode, coin_rotation, oscillation, smooth_damp};

/// `uGUI_*Bar.Punch`'s duration (`OnHealDamage`, `OnEat`, `OnDrink`).
const PUNCH_TIME: f32 = 2.5;
/// `punchDamp`, `puchFrequency` (`Oscillation`'s reduction and frequency).
const PUNCH_DAMP: f32 = 100.0;
const PUNCH_FREQUENCY: f32 = 5.0;

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

/// A bar's pulse: `pulseReferenceCapacity`, its two curves and its clip
/// (the `Animation`'s only state).
#[derive(Clone, Debug, PartialEq)]
pub struct Pulse {
    pub reference_capacity: f32,
    pub delay: Curve,
    pub time: Curve,
    pub clip: Clip,
}

/// The health, food and water bars' `icon` and its turn's fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconMotion {
    pub icon: usize,
    /// The icon's stored `localScale`.
    pub scale: [f32; 3],
    pub spring_coef: f32,
    pub velocity_damp: f32,
    pub velocity_max: f32,
    pub random_velocity: f32,
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
    pub pulse: Pulse,
    /// Health, food, water.
    pub icon: Option<IconMotion>,
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
    /// `PDA.isInUse` (the health, food and water numbers turn to face).
    pub pda_open: bool,
    /// This frame's `LiveMixin.onHealDamage` (`AddHealth`: the health
    /// gained), `Survival.onEat`, `onDrink` (the food or water value).
    pub healed: Option<f32>,
    pub ate: Option<f32>,
    pub drank: Option<f32>,
    /// `Ocean.GetDepthOf(player)`: `max(0, oceanLevel − y)` (M9u6c1).
    pub depth: f32,
    /// The equipment holds a `Compass` (M9u6c1; `IsCompassEnabled`'s
    /// equipment test).
    pub compass: bool,
    /// `MainCamera.camera.transform.eulerAngles.y`, degrees (M9u6c2: the
    /// compass's direction).
    pub yaw: f32,
}

/// One bar's script state.
#[derive(Clone, Debug, PartialEq)]
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
    /// Shown last frame (`OnEnable` ran).
    pub enabled: bool,
    /// `subscribed`: the event handler is added in the first `LateUpdate`.
    pub subscribed: bool,
    pub pulse_tween: CoroutineTween,
    pub pulse_delay: f32,
    pub pulse_time: f32,
    /// The clip state's `normalizedTime`.
    pub normalized: f32,
    /// The clip as last sampled.
    pub pulse: UiState,
    pub punch_tween: CoroutineTween,
    pub punch_seed: f32,
    pub punch_max_scale: f32,
    pub punch_initial_scale: [f32; 3],
    pub punch_scale: [f32; 3],
    /// The icon's `localScale` as last set (the clip's, plus the punch).
    pub icon_scale: Option<[f32; 3]>,
    pub show_numbers: bool,
    pub rotation_current: f32,
    pub rotation_velocity: f32,
    pub last_fixed_update_time: f32,
    /// The icon's turn about y as last set, degrees.
    pub rotation: Option<f32>,
}

impl Default for BarState {
    fn default() -> BarState {
        BarState {
            curr: 0.0,
            vel: 0.0,
            cached: None,
            text: None,
            value: None,
            overlay: None,
            enabled: false,
            subscribed: false,
            pulse_tween: CoroutineTween::new(TweenMode::Loop, 0.0),
            pulse_delay: -1.0,
            pulse_time: -1.0,
            normalized: 0.0,
            pulse: UiState::default(),
            punch_tween: CoroutineTween::new(TweenMode::Once, 1.0),
            punch_seed: 0.0,
            punch_max_scale: 2.0,
            punch_initial_scale: [0.0; 3],
            punch_scale: [0.0; 3],
            icon_scale: None,
            show_numbers: false,
            rotation_current: 0.0,
            rotation_velocity: 0.0,
            last_fixed_update_time: 0.0,
            rotation: None,
        }
    }
}

/// Our stand-in for `UnityEngine.Random` (xorshift32).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Random(u32);

impl Random {
    /// `Random.value`: 0…1.
    pub fn value(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / ((1u32 << 24) - 1) as f32
    }

    /// `Random.Range(min, max)` for floats.
    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.value()
    }
}

/// The HUD's scripts' state.
#[derive(Clone, Debug, PartialEq)]
pub struct Hud {
    pub config: HudConfig,
    pub bars: Vec<BarState>,
    /// `uGUI_SceneHUD._active`.
    pub active: bool,
    /// `PDA.time` (the sum of the frames' `PDA.deltaTime`).
    pub pda_time: f32,
    pub random: Random,
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
            pda_time: 0.0,
            random: Random(0x9e37_79b9),
        }
    }

    /// One frame: `uGUI_SceneHUD.Update`, then each active bar's
    /// `LateUpdate`. The returned state holds every change the scripts
    /// have made so far (a state is the whole picture, not a delta).
    pub fn update(&mut self, inputs: &HudInputs) -> UiState {
        self.active = inputs.active;
        self.pda_time += inputs.delta_time;
        let pda_time = self.pda_time;
        let random = &mut self.random;
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
            let dt = inputs.delta_time;
            if shown != s.enabled {
                s.enabled = shown;
                if shown {
                    on_enable(s, pda_time, dt);
                } else {
                    on_disable(s);
                }
            }
            if shown {
                // The game's events; a punch started now has had its first
                // `MoveNext`.
                let event = match b.kind {
                    BarKind::Oxygen => None,
                    BarKind::Health => inputs.healed,
                    BarKind::Food => inputs.ate,
                    BarKind::Water => inputs.drank,
                };
                let punched = match (event, b.icon) {
                    (Some(amount), Some(_)) if s.subscribed => {
                        punch(s, amount, dt, random);
                        true
                    }
                    _ => false,
                };
                // The coroutines (`yield null`).
                if let Some(scalar) = s.pulse_tween.move_next(dt).update {
                    on_pulse(s, scalar);
                }
                if !punched {
                    let calls = s.punch_tween.move_next(dt);
                    if let Some(t) = calls.update {
                        on_punch_update(s, t);
                    }
                    if calls.stopped {
                        on_punch_stop(s);
                    }
                }
                // The legacy animation.
                s.pulse = UiState::default();
                let clip = &b.pulse.clip;
                clip.sample(clip.loop_time(s.normalized), &mut s.pulse);
                if let Some(m) = b.icon {
                    if let Some(sc) = s.pulse.transform.get(&m.icon).and_then(|t| t.scale) {
                        s.icon_scale = Some(sc);
                    }
                }
                late_update(b, s, inputs);
                pulse_update(b, s, inputs);
                if let Some(m) = b.icon {
                    icon_update(&m, s, inputs, pda_time, random);
                }
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

/// `OnEnable`: the turn's clock, the clip state enabled, `pulseTween.
/// Start()` (its first `MoveNext` at once).
fn on_enable(s: &mut BarState, pda_time: f32, dt: f32) {
    s.last_fixed_update_time = pda_time;
    let (_, calls) = s.pulse_tween.start(dt);
    if let Some(scalar) = calls.update {
        on_pulse(s, scalar);
    }
}

/// `OnDisable`: both tweens stop (the punch's `onStop` resets the icon),
/// the handler is removed.
fn on_disable(s: &mut BarState) {
    if s.punch_tween.stop() {
        on_punch_stop(s);
    }
    s.pulse_tween.stop();
    s.subscribed = false;
}

/// `OnPulse(scalar)`.
fn on_pulse(s: &mut BarState, scalar: f32) {
    let n = (s.pulse_tween.duration() * scalar - s.pulse_delay) / s.pulse_time;
    // A pulse time of 0 with a running tween is not met with the game's
    // curves (the delay is 0 then too); kept at 0 rather than NaN.
    s.normalized = if n.is_finite() {
        n.clamp(0.0, 1.0)
    } else {
        0.0
    };
}

/// `Punch(2.5, 1 + Clamp01(amount / 100))`.
fn punch(s: &mut BarState, amount: f32, dt: f32, random: &mut Random) {
    s.punch_tween.set_duration(PUNCH_TIME);
    s.punch_max_scale = 1.0 + (amount / 100.0).clamp(0.0, 1.0);
    let (stopped, calls) = s.punch_tween.start(dt);
    if stopped {
        on_punch_stop(s);
    }
    // `OnPunchStart`.
    s.punch_initial_scale = s.icon_scale.unwrap_or(s.punch_initial_scale);
    s.punch_seed = random.value();
    if let Some(t) = calls.update {
        on_punch_update(s, t);
    }
    if calls.stopped {
        on_punch_stop(s);
    }
}

fn on_punch_update(s: &mut BarState, t: f32) {
    let (o, o1) = oscillation(PUNCH_DAMP, PUNCH_FREQUENCY, s.punch_seed, t);
    s.punch_scale = [o * s.punch_max_scale, o1 * s.punch_max_scale, 0.0];
}

fn on_punch_stop(s: &mut BarState) {
    s.punch_scale = [0.0; 3];
    if s.icon_scale.is_some() {
        s.icon_scale = Some(s.punch_initial_scale);
    }
}

/// `LateUpdate`'s pulse: the delay and time from the curves, the tween's
/// duration (a pulse ending resets the clip to its start).
fn pulse_update(b: &BarConfig, s: &mut BarState, i: &HudInputs) {
    let has = match b.kind {
        BarKind::Oxygen => i.oxygen,
        BarKind::Health => i.health,
        BarKind::Food => i.food,
        BarKind::Water => i.water,
    };
    let p = &b.pulse;
    let time = 1.0 - (has / p.reference_capacity).clamp(0.0, 1.0);
    s.pulse_delay = p.delay.evaluate(time).max(0.0);
    s.pulse_time = p.time.evaluate(time).max(0.0);
    let duration = s.pulse_delay + s.pulse_time;
    if s.pulse_tween.duration() > 0.0 && duration <= 0.0 {
        s.normalized = 0.0;
    }
    s.pulse_tween.set_duration(duration);
    s.subscribed = true;
}

/// `LateUpdate`'s icon: the punch added to the clip's scale, the coin
/// turn.
fn icon_update(
    m: &IconMotion,
    s: &mut BarState,
    i: &HudInputs,
    pda_time: f32,
    random: &mut Random,
) {
    let shown_before = s.show_numbers;
    s.show_numbers = i.pda_open;
    // The clip is enabled: `icon.localScale += punchScale`.
    let base = s.icon_scale.unwrap_or(m.scale);
    s.icon_scale = Some([
        base[0] + s.punch_scale[0],
        base[1] + s.punch_scale[1],
        base[2] + s.punch_scale[2],
    ]);
    if shown_before != s.show_numbers {
        s.rotation_velocity += random.range(-m.random_velocity, m.random_velocity);
    }
    let target = if s.show_numbers { 180.0 } else { 0.0 };
    if coin_rotation(
        &mut s.rotation_current,
        target,
        &mut s.last_fixed_update_time,
        pda_time,
        &mut s.rotation_velocity,
        m.spring_coef,
        m.velocity_damp,
        m.velocity_max,
    ) {
        s.rotation = Some(s.rotation_current);
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
    // The clip as last sampled, then the icon's scale and turn.
    out.apply(&s.pulse);
    if let Some(icon) = b.icon.map(|m| m.icon) {
        let t = out.transform.entry(icon).or_default();
        if let Some(sc) = s.icon_scale {
            t.scale = Some(sc);
        }
        if let Some(deg) = s.rotation {
            let h = deg.to_radians() / 2.0;
            t.rotation = Some([0.0, h.sin(), 0.0, h.cos()]);
        }
    }
}

#[cfg(test)]
mod tests;
