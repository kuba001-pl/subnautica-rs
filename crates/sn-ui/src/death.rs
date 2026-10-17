//! Death, respawn and the overlays (M9u6e1; `docs/formats/ui.md` § Death,
//! respawn and overlays): `uGUI_PlayerDeath` (the black vignette),
//! `uGUI_Fader` with `uGUI_SceneRespawning` (the respawn screen) and
//! `uGUI_Overlays` (the out-of-oxygen overlay), ported from the decompiled
//! scripts (read only; the code is ours). Each frame [`DeathScreens::update`]
//! runs the scripts' `Update`s, then the `Invoke`s that are due, then the
//! calls the frame made ([`DeathInputs`]), and gives what the scripts have
//! set so far.
//!
//! Unity's frame order assumed (*hypothesis*, Unity's documented player
//! loop): the scripts' `Update`s, then the delayed calls (`Invoke`, and the
//! coroutines' `WaitForSeconds`, which is where `Player.ResetPlayerOnDeath`
//! shows and hides the respawn screen), then the animators (whose events
//! trigger the vignette). So a call made in a frame changes what is drawn
//! from the next frame's `Update` on, and an `Invoke` queued in a frame
//! runs at the earliest in the next frame, once `Time.time` reaches it.

use crate::clip::Curve;
use crate::state::UiState;
use crate::tween::Sequence;

/// `uGUI_PlayerDeath.DeathTypes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathType {
    FadeToBlack,
    CutToBlack,
}

impl DeathType {
    /// The animation event that triggers it (`PlayerAnimationEventManager`:
    /// `DeathFade`, `DeathCut`).
    pub fn from_event(function: &str) -> Option<DeathType> {
        match function {
            "DeathFade" => Some(DeathType::FadeToBlack),
            "DeathCut" => Some(DeathType::CutToBlack),
            _ => None,
        }
    }
}

/// `uGUI_PlayerDeath`'s numbers, read from its code (IL).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignetteTimes {
    /// `Invoke("BeginDeathFade", d)` (`FadeToBlack`).
    pub begin_fade_delay: f32,
    /// `Invoke("BeginFadeOut", d)` (both types).
    pub fade_out_delay: f32,
    /// `Update`'s `Time.deltaTime × in` and `× out`.
    pub rate_in: f32,
    pub rate_out: f32,
    /// Fading in, `color.a < full` lerps, else alpha 1; fading out, a
    /// colour below `clear` (before the step) resets the overlay.
    pub full: f32,
    pub clear: f32,
}

/// A `uGUI_Fader`'s fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaderConfig {
    /// `canvasGroup`'s node.
    pub group: usize,
    pub fade_in_time: f32,
    pub fade_out_time: f32,
    pub use_unscaled_time: bool,
    pub initial_state: bool,
}

/// One of `uGUI_Overlays.overlays`.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayConfig {
    pub graphic: usize,
    pub curve: Curve,
}

/// The three scripts' nodes and fields, and the language's string.
#[derive(Clone, Debug, PartialEq)]
pub struct DeathConfig {
    /// `uGUI_PlayerDeath.blackOverlay`.
    pub black_overlay: usize,
    pub vignette: VignetteTimes,
    /// `uGUI_SceneRespawning.loadingText` and `loadingBackground`.
    pub loading_text: usize,
    pub loading_background: FaderConfig,
    /// `Language.main.Get("Loading")`.
    pub loading: String,
    /// `uGUI_Overlays.overlays`, by id (`OutOfOxygen` = 0).
    pub overlays: Vec<OverlayConfig>,
}

/// What the frame's game code called.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeathInputs {
    /// `Time.deltaTime`, `Time.unscaledDeltaTime`, `Time.time`.
    pub delta_time: f32,
    pub unscaled_delta_time: f32,
    pub time: f32,
    /// `Player.SuffocationUpdate`'s `overlays.Set(0, 1 − t)`, every frame
    /// the player runs (`None`: no player).
    pub suffocation: Option<f32>,
    /// `Player.OnKill`'s `overlays.Set(0, 1)` (after the suffocation's).
    pub killed: bool,
    /// `TriggerDeathVignette` from the player animator's event.
    pub vignette: Option<DeathType>,
    /// `Player.ResetPlayerOnDeath`'s `respawning.Show()` and `.Hide()`.
    pub respawn_show: bool,
    pub respawn_hide: bool,
}

/// `uGUI_Fader`'s state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fader {
    pub config: FaderConfig,
    pub sequence: Sequence,
    /// `canvasGroup.alpha` once `ApplyState` has set it.
    pub alpha: Option<f32>,
}

impl Fader {
    /// `Awake`: `SetState(initialState)`.
    pub fn new(config: FaderConfig) -> Fader {
        let mut f = Fader {
            config,
            sequence: Sequence::default(),
            alpha: None,
        };
        f.set_state(config.initial_state);
        f
    }

    /// `Update` (`ApplyState`).
    pub fn update(&mut self, delta_time: f32, unscaled_delta_time: f32) {
        if self.sequence.active() {
            let dt = if self.config.use_unscaled_time {
                unscaled_delta_time
            } else {
                delta_time
            };
            self.sequence.update(dt);
            self.alpha = Some(fader_alpha(self.sequence.t()));
        }
    }

    pub fn set_state(&mut self, enabled: bool) {
        self.sequence.force_state(enabled);
    }

    /// `FadeOut()`: over `fadeInTime`, as the code has it (and `FadeIn()`
    /// over `fadeOutTime`).
    pub fn fade_out(&mut self) {
        self.sequence.set(self.config.fade_in_time, false);
    }

    pub fn fade_in(&mut self) {
        self.sequence.set(self.config.fade_out_time, true);
    }
}

/// `0.5 (1 − cos π t)`, `ApplyState`'s curve.
pub fn fader_alpha(t: f32) -> f32 {
    0.5 * (1.0 - (std::f32::consts::PI * t).cos())
}

/// The vignette's delayed calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    BeginDeathFade,
    BeginFadeOut,
}

/// `uGUI_PlayerDeath`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerDeath {
    pub active: bool,
    pub fade_in: bool,
    /// `blackOverlay.color.a` (its colour is always black).
    pub alpha: f32,
    /// `blackOverlay.enabled` once a call set it.
    pub enabled: Option<bool>,
    /// Queued `Invoke`s: (due time, the frame's time it was queued in,
    /// the call).
    invokes: Vec<(f32, f32, Call)>,
}

impl PlayerDeath {
    /// `Start`: the colour (0, 0, 0, 0).
    pub fn new() -> PlayerDeath {
        PlayerDeath {
            active: false,
            fade_in: false,
            alpha: 0.0,
            enabled: None,
            invokes: Vec::new(),
        }
    }

    /// `TriggerDeathVignette(type)`, at game time `time`.
    pub fn trigger(&mut self, v: &VignetteTimes, kind: DeathType, time: f32) {
        if self.active {
            return;
        }
        match kind {
            DeathType::CutToBlack => {
                self.fade_in = true;
                self.active = true;
                self.alpha = 1.0;
                self.enabled = Some(true);
            }
            DeathType::FadeToBlack => {
                self.invokes
                    .push((time + v.begin_fade_delay, time, Call::BeginDeathFade));
            }
        }
        self.invokes
            .push((time + v.fade_out_delay, time, Call::BeginFadeOut));
    }

    /// `Update`.
    pub fn update(&mut self, v: &VignetteTimes, delta_time: f32) {
        if !self.active {
            return;
        }
        if self.fade_in {
            if self.alpha < v.full {
                self.enabled = Some(true);
                self.alpha = lerp(self.alpha, 1.0, delta_time * v.rate_in);
            } else {
                self.alpha = 1.0;
            }
        } else {
            let before = self.alpha;
            self.alpha = lerp(before, 0.0, delta_time * v.rate_out);
            if before < v.clear {
                self.reset();
            }
        }
    }

    /// The delayed calls due at `time` and queued in an earlier frame.
    fn run_invokes(&mut self, time: f32) {
        let mut due: Vec<(f32, f32, Call)> = Vec::new();
        self.invokes.retain(|&(at, queued, call)| {
            let run = queued < time && at <= time;
            if run {
                due.push((at, queued, call));
            }
            !run
        });
        // Unity runs them in due order.
        due.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, _, call) in due {
            match call {
                Call::BeginDeathFade => {
                    self.fade_in = true;
                    self.active = true;
                    self.enabled = Some(true);
                }
                Call::BeginFadeOut => self.fade_in = false,
            }
        }
    }

    /// `ResetOverlay`.
    pub fn reset(&mut self) {
        self.active = false;
        self.fade_in = false;
        self.enabled = Some(false);
        self.alpha = 0.0;
    }
}

impl Default for PlayerDeath {
    fn default() -> PlayerDeath {
        PlayerDeath::new()
    }
}

/// `Mathf.Lerp` / `Color.Lerp`'s channel: `t` clamped to 0…1.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// `uGUI_Overlays.SetAlpha`: the graphic enabled and its renderer's alpha
/// set iff the curve's value (clamped) is above 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayState {
    pub enabled: Option<bool>,
    pub alpha: Option<f32>,
}

/// The three scripts.
#[derive(Clone, Debug, PartialEq)]
pub struct DeathScreens {
    pub config: DeathConfig,
    pub death: PlayerDeath,
    pub fader: Fader,
    /// `loadingText`'s text once `Show` set it.
    pub loading_text: Option<String>,
    pub overlays: Vec<OverlayState>,
    /// The last `Set` value per overlay (for the logs).
    pub overlay_values: Vec<Option<f32>>,
}

impl DeathScreens {
    /// The scripts after `Awake` and `Start`.
    pub fn new(config: DeathConfig) -> DeathScreens {
        let fader = Fader::new(config.loading_background);
        let n = config.overlays.len();
        let mut s = DeathScreens {
            config,
            death: PlayerDeath::new(),
            fader,
            loading_text: None,
            overlays: vec![OverlayState::default(); n],
            overlay_values: vec![None; n],
        };
        // `uGUI_Overlays.Awake`.
        for id in 0..n {
            s.set_overlay(id, 0.0);
        }
        s
    }

    /// `uGUI_Overlays.Set(id, value)`.
    pub fn set_overlay(&mut self, id: usize, value: f32) {
        let (Some(o), Some(state)) = (self.config.overlays.get(id), self.overlays.get_mut(id))
        else {
            return;
        };
        let a = o.curve.evaluate(value).clamp(0.0, 1.0);
        if a > 0.0 {
            state.enabled = Some(true);
            state.alpha = Some(a);
        } else {
            state.enabled = Some(false);
        }
        self.overlay_values[id] = Some(value);
    }

    /// The overlay's alpha as drawn (0 while disabled).
    pub fn overlay_alpha(&self, id: usize) -> f32 {
        match self.overlays.get(id) {
            Some(OverlayState {
                enabled: Some(true),
                alpha,
            }) => alpha.unwrap_or(1.0),
            _ => 0.0,
        }
    }

    /// One frame; the state holds every change the scripts made so far.
    pub fn update(&mut self, i: &DeathInputs) -> UiState {
        let v = self.config.vignette;
        // The `Update`s.
        self.fader.update(i.delta_time, i.unscaled_delta_time);
        self.death.update(&v, i.delta_time);
        if let Some(value) = i.suffocation {
            self.set_overlay(0, value);
        }
        // The delayed calls, then the frame's own.
        self.death.run_invokes(i.time);
        if i.killed {
            self.set_overlay(0, 1.0);
        }
        if i.respawn_show {
            // `Show`: the text, then `SetState(true)`.
            self.loading_text = Some(self.config.loading.clone());
            self.fader.set_state(true);
        }
        if i.respawn_hide {
            self.fader.fade_out();
        }
        if let Some(kind) = i.vignette {
            self.death.trigger(&v, kind, i.time);
        }
        self.state()
    }

    /// What the scripts have set: the vignette's colour from `Start` on.
    pub fn state(&self) -> UiState {
        let c = &self.config;
        let mut out = UiState::default();
        out.color
            .insert(c.black_overlay, [0.0, 0.0, 0.0, self.death.alpha]);
        if let Some(e) = self.death.enabled {
            out.enabled.insert(c.black_overlay, e);
        }
        if let Some(a) = self.fader.alpha {
            out.group_alpha.insert(c.loading_background.group, a);
        }
        if let Some(t) = &self.loading_text {
            out.text.insert(c.loading_text, t.clone());
        }
        for (o, s) in c.overlays.iter().zip(&self.overlays) {
            if let Some(e) = s.enabled {
                out.enabled.insert(o.graphic, e);
            }
            if let Some(a) = s.alpha {
                out.renderer_alpha.insert(o.graphic, a);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
