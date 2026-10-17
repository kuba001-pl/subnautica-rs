//! `ErrorMessage` (M9u7b; `docs/formats/ui.md` § Messages): the HUD's
//! text messages ("Inventory full", "You died"…), ported from the
//! decompiled script (read only; the code is ours). Each message flies in
//! from the canvas's centre to its place in the column at the top left,
//! stays, fades out and goes back to the pool; an equal text already shown
//! is renewed and counted (`"{text} (x{n})"`); when one goes, the ones
//! below slide up (`offsetY`).
//!
//! The entries are copies of `prefabMessage` the scene makes
//! (`UiScene::graft`, four at a time, as `GetEntry` does); this module
//! keeps their node indices and writes their state. What it reads of the
//! layout ([`MessageLayout`]) and of the texts (each text's preferred
//! size) the caller gives.
//!
//! A frame ([`Messages::update`]) in the game's order (*hypothesis* for
//! the place of the coroutines, Unity's documented player loop; the
//! queues' order is `ManagedUpdate.Queue`'s, read in the code):
//! 1. the calls made from `GUIHand` (queue `UpdateGUIHand`, e.g.
//!    `Pickupable.OnHandClick`'s `InventoryFull`), at the time before the
//!    step;
//! 2. `PDA.UpdateTime` (queue `UpdatePDA`): `time += deltaTime`;
//! 3. the calls made from coroutines (`Player.ResetPlayerOnDeath`'s
//!    `YouDied`), after the `Update`s;
//! 4. `OnLateUpdate` (queue `PreCanvasRectTransform`, before the canvases
//!    are drawn): expiry, the slide, each entry's position and alpha.

use crate::layout::Rect;
use crate::state::{LocalTransform, UiState};

/// `ErrorMessage`'s times (its constructor) and `OnLateUpdate`'s numbers
/// (IL).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageTimes {
    pub fly_in: f32,
    pub delay: f32,
    pub fade_out: f32,
    pub invisible: f32,
    /// `offsetY -= offsetY × rate × PDA.deltaTime`.
    pub slide_rate: f32,
    /// `offsetY < snap` → 0.
    pub snap: f32,
}

/// `ErrorMessage`'s nodes and fields.
#[derive(Clone, Debug, PartialEq)]
pub struct MessagesConfig {
    /// `messageCanvas`'s node.
    pub canvas: usize,
    pub offset: [f32; 2],
    pub y_spacing: f32,
    pub times: MessageTimes,
    /// `prefabMessage`'s `m_fontColor` (`SetAlpha` changes its alpha).
    pub font_color: [f32; 4],
}

/// What `OnLateUpdate` reads of the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageLayout {
    /// `messageCanvas.rect`.
    pub canvas_rect: Rect,
    /// An entry's `rectTransform.rect.width` (the copies are equal).
    pub entry_width: f32,
    /// Where an entry's anchored position is measured from in the
    /// canvas's space: `localPosition` = this + `anchoredPosition`.
    pub anchor_base: [f32; 2],
    /// `uGUI_CanvasScaler.GetInverseScale(messageCanvas)`.
    pub inverse_scale: f32,
}

/// A frame's calls (`AddError`, `AddMessage`, `AddWarning`, `AddDebug`:
/// all the same), already through `Language.Get`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageInputs {
    pub delta_time: f32,
    /// Made before `PDA.UpdateTime` (from `GUIHand`).
    pub early: Vec<String>,
    /// Made after it (from coroutines).
    pub late: Vec<String>,
}

/// `ErrorMessage._Message`.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// Its entry's node.
    pub entry: usize,
    pub text: String,
    pub num: i32,
    pub time_end: f32,
}

/// `ErrorMessage`'s state.
#[derive(Clone, Debug)]
pub struct Messages {
    pub config: MessagesConfig,
    /// `PDA.time`.
    pub time: f32,
    pub messages: Vec<Message>,
    pub offset_y: f32,
    /// The free entries (`pool`).
    pub pool: Vec<usize>,
    /// Every entry made, in order.
    pub entries: Vec<usize>,
    /// What the script has set on the entries so far.
    pub state: UiState,
    /// Each entry's text as shown (`entry.text`).
    texts: Vec<(usize, String)>,
}

/// `MathExtensions.EaseOutSine`: `Mathf.Sin(v × π × 0.5)` (in floats;
/// `Mathf.Sin` goes through doubles).
pub fn ease_out_sine(v: f32) -> f32 {
    ((v * std::f32::consts::PI * 0.5) as f64).sin() as f32
}

/// `MathExtensions.EvaluateLine`: the line through (x1, y1), (x2, y2) at x.
pub fn evaluate_line(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let k = (y2 - y1) / (x2 - x1);
    let b = y1 - k * x1;
    k * x + b
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

impl Messages {
    pub fn new(config: MessagesConfig) -> Messages {
        Messages {
            config,
            time: 0.0,
            messages: Vec::new(),
            offset_y: 0.0,
            pool: Vec::new(),
            entries: Vec::new(),
            state: UiState::default(),
            texts: Vec::new(),
        }
    }

    /// The text an entry shows (as last set).
    pub fn text_of(&self, entry: usize) -> &str {
        self.texts
            .iter()
            .find(|(e, _)| *e == entry)
            .map_or("", |(_, t)| t.as_str())
    }

    fn set_text(&mut self, entry: usize, text: String) {
        self.state.text.insert(entry, text.clone());
        match self.texts.iter_mut().find(|(e, _)| *e == entry) {
            Some(t) => t.1 = text,
            None => self.texts.push((entry, text)),
        }
    }

    /// `GetEntry`: a free entry; four new ones (`make`: a copy of the
    /// template under `messageCanvas`, its node) when none is free.
    fn get_entry(&mut self, make: &mut dyn FnMut() -> usize) -> usize {
        if self.pool.is_empty() {
            for _ in 0..4 {
                let e = make();
                self.state.active.insert(e, false);
                self.entries.push(e);
                self.pool.push(e);
            }
        }
        self.pool.pop().unwrap_or_default()
    }

    /// `_AddMessage`.
    pub fn add(&mut self, text: &str, make: &mut dyn FnMut() -> usize) {
        if text.is_empty() {
            return;
        }
        let t = self.config.times;
        if let Some(i) = self.messages.iter().position(|m| m.text == text) {
            let m = &mut self.messages[i];
            m.time_end = self.time + t.delay + t.fade_out + t.invisible;
            m.num += 1;
            let (entry, shown) = (m.entry, format!("{text} (x{})", m.num));
            self.set_text(entry, shown);
            return;
        }
        let entry = self.get_entry(make);
        self.state.active.insert(entry, true);
        self.set_text(entry, text.to_string());
        self.messages.push(Message {
            entry,
            text: text.to_string(),
            num: 1,
            time_end: self.time + t.fly_in + t.delay + t.fade_out + t.invisible,
        });
    }

    /// `GetYPos(index)`; `height`: an entry's `preferredHeight`.
    fn y_pos(&self, index: usize, inv: f32, height: &dyn Fn(usize) -> f32) -> f32 {
        let mut sum = 0.0;
        for m in self.messages.iter().take(index) {
            sum += height(m.entry);
        }
        -(self.config.offset[1] * inv + self.offset_y + sum + index as f32 * self.config.y_spacing)
    }

    /// `OnLateUpdate`. `measure`: a text's preferred (width, height) in its
    /// entry.
    pub fn late_update(
        &mut self,
        delta_time: f32,
        layout: &MessageLayout,
        measure: &dyn Fn(&str) -> [f32; 2],
    ) {
        let time = self.time;
        // The game's loop removes while it walks: the one after a removed
        // message is skipped this frame.
        let mut i = 0;
        while i < self.messages.len() {
            if time > self.messages[i].time_end {
                let m = self.messages.remove(i);
                self.offset_y += measure(self.text_of(m.entry))[1];
                // `ReleaseEntry`.
                self.state.active.insert(m.entry, false);
                self.pool.push(m.entry);
            }
            i += 1;
        }
        let slide = self.offset_y * self.config.times.slide_rate;
        self.offset_y -= slide * delta_time;
        if self.offset_y < self.config.times.snap {
            self.offset_y = 0.0;
        }
        let t = self.config.times;
        let rect = layout.canvas_rect;
        let inv = layout.inverse_scale;
        let sizes: Vec<(usize, [f32; 2])> = self
            .messages
            .iter()
            .map(|m| (m.entry, measure(self.text_of(m.entry))))
            .collect();
        let size = |e: usize| {
            sizes
                .iter()
                .find(|(n, _)| *n == e)
                .map_or([0.0, 0.0], |(_, s)| *s)
        };
        let height = |e: usize| size(e)[1];
        for j in 0..self.messages.len() {
            let m = &self.messages[j];
            let (entry, end) = (m.entry, m.time_end);
            let [w, h] = size(entry);
            let a = [-0.5 * layout.entry_width.min(w), 0.5 * h];
            let b = [
                rect.x + self.config.offset[0] * inv,
                -rect.y + self.y_pos(j, inv, &height),
            ];
            let fly = clamp01(evaluate_line(
                end - t.invisible - t.fade_out - t.delay - t.fly_in,
                0.0,
                end - t.invisible - t.fade_out - t.delay,
                1.0,
                time,
            ));
            // `Vector2.Lerp` clamps its t.
            let k = clamp01(ease_out_sine(fly));
            let local = [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
            let fade = clamp01(evaluate_line(
                end - t.invisible - t.fade_out,
                1.0,
                end - t.invisible,
                0.0,
                time,
            ));
            let alpha = ease_out_sine(fade);
            // `localPosition` from a `Vector2`: z is 0.
            self.state.transform.insert(
                entry,
                LocalTransform {
                    anchored_position: Some([
                        local[0] - layout.anchor_base[0],
                        local[1] - layout.anchor_base[1],
                    ]),
                    z: Some(0.0),
                    ..LocalTransform::default()
                },
            );
            let c = self.config.font_color;
            self.state.color.insert(entry, [c[0], c[1], c[2], alpha]);
        }
    }

    /// A frame (see the module's notes): the early calls, the time step,
    /// the late calls, `OnLateUpdate`.
    pub fn update(
        &mut self,
        inputs: &MessageInputs,
        layout: &MessageLayout,
        measure: &dyn Fn(&str) -> [f32; 2],
        make: &mut dyn FnMut() -> usize,
    ) -> &UiState {
        for m in &inputs.early {
            self.add(m, make);
        }
        self.time += inputs.delta_time;
        for m in &inputs.late {
            self.add(m, make);
        }
        self.late_update(inputs.delta_time, layout, measure);
        &self.state
    }
}

#[cfg(test)]
mod tests;
