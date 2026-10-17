//! `uGUI_PopupNotification` and its default skin (M9u7d1;
//! `docs/formats/ui.md` § The unlock pop-up): the HUD's pop-up at the
//! left ("Blueprint unlocked", a picture, the item's name), ported from the
//! decompiled scripts (read only; the code is ours). An entry slides in
//! from beyond the left edge over `timeIn`, stays its duration, slides out
//! over `timeOut`; the next queued entry follows once it is done.
//!
//! A frame ([`Popup::update`]) in the game's order (`ManagedUpdate.Queue`,
//! read in the code):
//! 1. the frame's `Enqueue` calls (our callers make them before the
//!    popup's update);
//! 2. `OnUpdate` (queue `UpdateAfterInput`), at `PDA.time` before the step:
//!    the phase, the transition value, the next entry;
//! 3. `PDA.UpdateTime` (queue `UpdatePDA`): `time += deltaTime`;
//! 4. `OnLayoutComplete` (queue `UILayoutComplete`), when `OnUpdate` changed
//!    the value: the skin's `SetTransition`.
//!
//! Sounds are not ported: each one the game would start or play is named
//! in [`PopupEvents::sounds`].

use crate::layout::Rect;
use crate::messages::ease_out_sine;
use crate::state::{LocalTransform, SpriteId, UiState};

/// `timeIn` and `timeOut` (`Show`'s constants).
pub const TIME_IN: f32 = 0.25;
pub const TIME_OUT: f32 = 0.25;

/// `PopupNotificationSkin`.
pub const UNLOCK: usize = 0;
pub const JOURNAL: usize = 1;

/// A `uGUI_PopupNotificationDefault`'s nodes and fields.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupSkin {
    /// The skin's own node (its `RectTransform`, which `SetTransition`
    /// moves).
    pub root: usize,
    /// `canvasGroup`'s node (`SetVisible`).
    pub group: usize,
    pub ox: f32,
    pub oy: f32,
    pub title: usize,
    pub text: usize,
    pub controls: usize,
    pub image: Option<usize>,
    pub default_sprite: Option<SpriteId>,
    /// `soundSlideIn`, `soundSlideOut`, `defaultSound` (`None`: not set).
    pub sound_slide_in: Option<String>,
    pub sound_slide_out: Option<String>,
    pub default_sound: Option<String>,
}

/// `uGUI_PopupNotification`'s nodes and fields.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupConfig {
    /// Its own node (the skins' parent).
    pub root: usize,
    /// `canvasGroup`'s node.
    pub group: usize,
    pub skins: Vec<PopupSkin>,
    pub default_duration: f32,
}

/// `uGUI_PopupNotification.Entry`, its texts already through
/// `Language.Get`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub duration: f32,
    pub skin: usize,
    pub title: String,
    pub text: String,
    pub controls: String,
    pub sprite: Option<SpriteId>,
    /// The sound's name (`None`: no sound).
    pub sound: Option<String>,
}

/// `uGUI_PopupNotification.Phase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    None,
    Zero,
    In,
    One,
    Out,
    Done,
}

/// What a frame did, for the logs and checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PopupEvents {
    /// Entries shown (`Show`), at `PDA.time`.
    pub shown: Vec<(Entry, f32)>,
    /// Phase changes (the new phase), at `PDA.time`.
    pub phases: Vec<(Phase, f32)>,
    /// Sounds the game would start or play here (not ported).
    pub sounds: Vec<String>,
}

/// `uGUI_PopupNotification`'s state.
#[derive(Clone, Debug)]
pub struct Popup {
    pub config: PopupConfig,
    /// `PDA.time`.
    pub time: f32,
    pub time_in: f32,
    pub time_duration: f32,
    pub time_out: f32,
    pub start: f32,
    pub phase: Phase,
    pub value: f32,
    pub queue: Vec<Entry>,
    pub current: Option<Entry>,
    /// The skin set last (`SetSkin`).
    pub skin: Option<usize>,
    /// `OnLayoutComplete` is subscribed (the value changed this frame).
    pub layout_pending: bool,
    /// What the scripts have set so far.
    pub state: UiState,
    pub events: PopupEvents,
}

/// What `SetTransition` reads of the layout: the skin's parent's rect
/// (`uGUI_PopupNotification`'s own) and the skin's rect, by skin.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupLayout {
    pub parent: Rect,
    pub skins: Vec<Rect>,
}

/// `Mathf.Approximately`: `|b − a| < max(1e-6 × max(|a|, |b|), ε × 8)`
/// (ε `Mathf.Epsilon`, the smallest denormal on our platform).
pub fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(f32::from_bits(1) * 8.0)
}

/// `MathExtensions.Trapezoid(a, b, c, d, t, wrap: false)`: 0 → 1 over
/// [a, a + b], 1 over c, 1 → 0 over d.
pub fn trapezoid(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let total = a + b + c + d;
    let up = if b > 0.0 { (t - a) / b } else { 0.0 };
    let down = if d > 0.0 { (total - t) / d } else { 0.0 };
    up.min(down).clamp(0.0, 1.0)
}

/// `MathExtensions.Step(a, b, t, wrap: false)`.
pub fn step(a: f32, b: f32, t: f32) -> f32 {
    (if b > 0.0 { (t - a) / b } else { 0.0 }).clamp(0.0, 1.0)
}

/// `RectTransformExtensions.GetRectPositions` at `TextAnchor.MiddleLeft`:
/// beyond the parent's left edge, and `ox` inside it, both centred.
pub fn middle_left_positions(parent: Rect, rect: Rect, ox: f32) -> ([f32; 2], [f32; 2]) {
    let y = 0.5 * (parent.height - rect.height);
    ([-rect.width, y], [ox, y])
}

impl Popup {
    pub fn new(config: PopupConfig) -> Popup {
        Popup {
            config,
            time: 0.0,
            time_in: 0.0,
            time_duration: 0.0,
            time_out: 0.0,
            start: 0.0,
            phase: Phase::None,
            value: 0.0,
            queue: Vec::new(),
            current: None,
            skin: None,
            layout_pending: false,
            state: UiState::default(),
            events: PopupEvents::default(),
        }
    }

    /// `isShowingMessage`.
    pub fn is_showing(&self) -> bool {
        self.phase >= Phase::In && self.phase <= Phase::Out
    }

    /// Nothing queued, nothing shown and nothing left to move: skipping
    /// `OnUpdate` then changes nothing but `PDA.time`. (At `Done` with an
    /// entry just shown, the next `OnUpdate` starts its slide in.)
    pub fn idle(&self) -> bool {
        self.queue.is_empty()
            && self.current.is_none()
            && matches!(self.phase, Phase::None | Phase::Done)
            && self.value == 0.0
            && !self.layout_pending
    }

    /// `Enqueue`: a sound already queued is not played twice.
    pub fn enqueue(&mut self, mut entry: Entry) {
        if entry.sound.is_some() && self.queue.iter().any(|q| q.sound == entry.sound) {
            entry.sound = None;
        }
        self.queue.push(entry);
    }

    /// `Hide`: to the start of the slide out, from where the value is.
    pub fn hide(&mut self) {
        if self.time_duration < 0.0 {
            self.time_duration = 0.0;
        }
        self.start = self.time
            - (self.time_in + self.time_duration.max(0.0) + self.time_out * (1.0 - self.value));
    }

    /// `ShowNext`: hide the current entry (no voiced sound is ported, so
    /// always) and put this one first.
    pub fn show_next(&mut self, entry: Entry) {
        self.hide();
        self.queue.insert(0, entry);
    }

    /// `SetSkin` and the skin's `SetVisible`.
    fn set_skin(&mut self, skin: usize) {
        for (i, s) in self.config.skins.iter().enumerate() {
            self.state
                .group_alpha
                .insert(s.group, if i == skin { 1.0 } else { 0.0 });
        }
        self.skin = Some(skin);
    }

    /// `Show`.
    pub fn show(&mut self, entry: Entry) {
        self.set_skin(entry.skin);
        if let Some(s) = self.config.skins.get(entry.skin) {
            // `uGUI_PopupNotificationDefault.OnShow`.
            self.state.text.insert(s.title, entry.title.clone());
            if let Some(image) = s.image {
                self.state
                    .sprite
                    .insert(image, entry.sprite.or(s.default_sprite));
            }
            self.state.text.insert(s.text, entry.text.clone());
            self.state.text.insert(s.controls, entry.controls.clone());
            if let Some(sound) = entry.sound.clone().or_else(|| s.default_sound.clone()) {
                self.events.sounds.push(format!("start {sound}"));
            }
        }
        self.time_in = TIME_IN;
        self.time_duration = entry.duration;
        self.time_out = TIME_OUT;
        if self.phase == Phase::None {
            self.phase = Phase::Zero;
        }
        self.start = self.time - self.time_in * self.value;
        self.events.shown.push((entry.clone(), self.time));
        self.current = Some(entry);
    }

    fn skin_sound(&self, f: impl Fn(&PopupSkin) -> &Option<String>) -> Option<String> {
        self.skin
            .and_then(|s| self.config.skins.get(s))
            .and_then(|s| f(s).clone())
    }

    /// `OnUpdate`.
    pub fn on_update(&mut self) {
        let num = self.time - self.start;
        let phase = if self.phase == Phase::None {
            Phase::None
        } else if num < 0.0 {
            Phase::Zero
        } else if num < self.time_in {
            Phase::In
        } else if self.time_duration < 0.0 || num < self.time_in + self.time_duration {
            Phase::One
        } else if num < self.time_in + self.time_duration + self.time_out {
            Phase::Out
        } else {
            Phase::Done
        };
        if self.phase != phase {
            let was = self.phase;
            let shown = |p: Phase| p == Phase::In || p == Phase::One;
            if (was <= Phase::Zero || was == Phase::Done) && shown(phase) {
                if let Some(s) = self.skin_sound(|s| &s.sound_slide_in) {
                    self.events.sounds.push(format!("play {s}"));
                }
            } else if shown(was) && phase == Phase::Out {
                if let Some(s) = self.skin_sound(|s| &s.sound_slide_out) {
                    self.events.sounds.push(format!("play {s}"));
                }
            }
            self.phase = phase;
            let alpha = if self.is_showing() { 1.0 } else { 0.0 };
            self.state.group_alpha.insert(self.config.group, alpha);
            if phase == Phase::Out && self.current.as_ref().is_some_and(|c| c.sound.is_some()) {
                self.events.sounds.push("stop (fade out)".into());
            }
            if phase == Phase::Done {
                self.current = None;
            }
            self.events.phases.push((phase, self.time));
        }
        let b = if self.time_duration < 0.0 {
            step(0.0, self.time_in, num)
        } else {
            trapezoid(0.0, self.time_in, self.time_duration, self.time_out, num)
        };
        if !approximately(self.value, b) {
            self.value = b;
            self.layout_pending = true;
        }
        if matches!(self.phase, Phase::None | Phase::Done) && !self.queue.is_empty() {
            if self.phase == Phase::None {
                self.phase = Phase::Zero;
            }
            let entry = self.queue.remove(0);
            self.show(entry);
        }
    }

    /// `OnLayoutComplete`: the skin's `SetTransition(value)`.
    pub fn on_layout_complete(&mut self, layout: &PopupLayout) {
        self.layout_pending = false;
        let Some(k) = self.skin else {
            return;
        };
        let (Some(skin), Some(&rect)) = (self.config.skins.get(k), layout.skins.get(k)) else {
            return;
        };
        let (p0, p1) = middle_left_positions(layout.parent, rect, skin.ox);
        // `Vector2.Lerp` clamps its t.
        let t = ease_out_sine(self.value).clamp(0.0, 1.0);
        let at = [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t];
        self.state.transform.insert(
            skin.root,
            LocalTransform {
                anchored_position: Some(at),
                ..LocalTransform::default()
            },
        );
    }

    /// A frame (see the module's notes).
    pub fn update(&mut self, enqueue: &[Entry], delta_time: f32, layout: &PopupLayout) -> &UiState {
        self.events = PopupEvents::default();
        for e in enqueue {
            self.enqueue(e.clone());
        }
        self.on_update();
        self.time += delta_time;
        if self.layout_pending {
            self.on_layout_complete(layout);
        }
        &self.state
    }
}

#[cfg(test)]
mod tests;
