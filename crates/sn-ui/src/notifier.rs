//! `uGUI_IconNotifier` (M9u7c2; `docs/formats/ui.md` § Picked-up icons),
//! the item icons that rise from the screen's centre when the player
//! picks something up, ported from the decompiled script (read only; the
//! code is ours).
//!
//! - `Play(techType, animation)` queues a request.
//! - `Update` (each frame, after `PDA.UpdateTime`: *hypothesis*, see
//!   [`Notifier::update`]) takes one request from the queue when
//!   `PDA.time − time ≥ interval` (`time` starts at −interval, so the
//!   first goes at once): an icon from the pool (`GetInstance` makes four
//!   `NotifierIcon`s at a time by code), its sprite, its size, last among
//!   its siblings, shown, a `Random.value` seed. Then every active request
//!   is animated, last first.
//! - `Animate`: at t, the icon's anchored position is the animator's x and
//!   y curves × the canvas rect's size; its scale the scale curve plus
//!   `Oscillation` × `oscScale` (from `oscOffset` on); its alpha the alpha
//!   curve. On the frame t = 1 it is released (hidden, back in the pool);
//!   otherwise t += `PDA.deltaTime` ÷ duration, clamped to 1.
//!
//! The icon is `uGUI_ItemIcon` ([`crate::icon`]): `SetForegroundSprite`
//! sizes the foreground with `Fit` keeping the sprite's aspect, `SetSize`
//! sizes the icon, `SetAlpha` sets the foreground's `CanvasRenderer`
//! colour (white, so only its alpha).
//!
//! `UnityEngine.Random` is not reproduced: our generator, the same range
//! (as M9u6b2).

use std::collections::VecDeque;

use crate::clip::Curve;
use crate::hud::Random;
use crate::icon::{fill_rect, fit};
use crate::layout::{Rect, V2};
use crate::state::{SpriteId, UiState};
use crate::tween::oscillation;

/// `uGUI_IconNotifier.IconAnimator`.
#[derive(Clone, Debug, PartialEq)]
pub struct IconAnimator {
    pub duration: f32,
    pub x: Curve,
    pub y: Curve,
    pub scale: Curve,
    pub alpha: Curve,
    pub oscillate: bool,
    pub osc_offset: f32,
    pub osc_reduction: f32,
    pub osc_frequency: f32,
    pub osc_scale: f32,
}

/// `uGUI_IconNotifier.AnimationType`: the index into `animators`.
pub const FROM: usize = 0;
pub const TO: usize = 1;

/// The notifier's stored fields.
#[derive(Clone, Debug, PartialEq)]
pub struct NotifierConfig {
    /// `canvas`'s node (the icons' parent).
    pub canvas: usize,
    pub interval: f32,
    pub icon_size: f32,
    pub animators: Vec<IconAnimator>,
}

/// An icon made by `GetInstance`: its `NotifierIcon` node and its
/// foreground's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon {
    pub root: usize,
    pub foreground: usize,
}

/// A `Request`.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// `SpriteManager.Get(techType)`, as the id the scene gave it, and the
    /// sprite's `rect` size.
    pub sprite: SpriteId,
    pub sprite_size: V2,
    pub animation: usize,
    pub icon: Option<Icon>,
    pub random: f32,
    pub t: f32,
}

/// What happened in a frame, for the logs and the checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotifierEvents {
    /// Requests taken from the queue: (icon, `PDA.time`).
    pub shown: Vec<(Icon, f32)>,
    /// Icons released.
    pub released: Vec<Icon>,
    /// Nodes given `SetAsLastSibling`, in call order.
    pub last_sibling: Vec<usize>,
}

/// The notifier's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Notifier {
    pub config: NotifierConfig,
    /// `PDA.time`.
    pub pda_time: f32,
    /// The last dequeue's `PDA.time`.
    pub time: f32,
    pub queue: VecDeque<Request>,
    pub active: Vec<Request>,
    pub pool: Vec<Icon>,
    /// Every icon made, in order.
    pub icons: Vec<Icon>,
    pub random: Random,
    /// The icons' and foregrounds' changes.
    pub state: UiState,
    pub events: NotifierEvents,
}

impl Notifier {
    /// `Awake`: `time = −interval`.
    pub fn new(config: NotifierConfig, seed: u32) -> Notifier {
        Notifier {
            time: -config.interval,
            config,
            pda_time: 0.0,
            queue: VecDeque::new(),
            active: Vec::new(),
            pool: Vec::new(),
            icons: Vec::new(),
            random: Random::new(seed),
            state: UiState::default(),
            events: NotifierEvents::default(),
        }
    }

    /// `Play(techType, animation)`.
    pub fn play(&mut self, sprite: SpriteId, sprite_size: V2, animation: usize) {
        self.queue.push_back(Request {
            sprite,
            sprite_size,
            animation,
            icon: None,
            random: 0.0,
            t: 0.0,
        });
    }

    /// Whether a frame would change anything (a request queued or shown).
    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || !self.active.is_empty()
    }

    /// `GetInstance`: four icons made at a time when the pool is empty
    /// (`make` makes one, hidden), the pool's last taken.
    fn get_instance(&mut self, make: &mut dyn FnMut() -> Icon) -> Icon {
        if self.pool.is_empty() {
            for _ in 0..4 {
                let icon = make();
                self.state.active.insert(icon.root, false);
                self.icons.push(icon);
                self.pool.push(icon);
            }
        }
        self.pool.pop().expect("the pool was just filled")
    }

    /// One frame: this frame's `plays` (called before `PDA.UpdateTime`:
    /// `Inventory.Pickup` from the hand), `PDA.UpdateTime` by
    /// `delta_time`, then `Update` (*hypothesis*: after `ManagedUpdate`'s
    /// `PDA.UpdateTime`; both scripts have execution order 0, so Unity's
    /// order between them is not defined) with the canvas's rect.
    pub fn update(
        &mut self,
        plays: &[(SpriteId, V2, usize)],
        delta_time: f32,
        canvas_rect: Rect,
        make: &mut dyn FnMut() -> Icon,
    ) -> &UiState {
        self.events = NotifierEvents::default();
        for &(s, size, a) in plays {
            self.play(s, size, a);
        }
        self.pda_time += delta_time;
        if !self.queue.is_empty() && self.pda_time - self.time >= self.config.interval {
            let mut request = self.queue.pop_front().expect("not empty");
            let icon = self.get_instance(make);
            let size = self.config.icon_size;
            // `SetForegroundSprite`: the sprite, enabled, `Fit`; `SetSize`:
            // the icon and the foreground (`Fit` again, the same size).
            self.state
                .sprite
                .insert(icon.foreground, Some(request.sprite));
            self.state.enabled.insert(icon.foreground, true);
            let fg = fit([size, size], request.sprite_size, true);
            self.state
                .transform
                .entry(icon.foreground)
                .or_default()
                .size_delta = Some(fg);
            self.state
                .material
                .entry(icon.foreground)
                .or_default()
                .vectors
                .insert("_FillRect".into(), fill_rect([-1.0, -1.0], fg));
            self.state
                .transform
                .entry(icon.root)
                .or_default()
                .size_delta = Some([size, size]);
            self.events.last_sibling.push(icon.root);
            self.state.active.insert(icon.root, true);
            request.icon = Some(icon);
            request.random = self.random.value();
            self.events.shown.push((icon, self.pda_time));
            self.active.push(request);
            self.time = self.pda_time;
        }
        for k in (0..self.active.len()).rev() {
            if self.animate(k, delta_time, canvas_rect) {
                self.active.remove(k);
            }
        }
        &self.state
    }

    /// `Animate(request)`: true when it is done (released).
    fn animate(&mut self, k: usize, delta_time: f32, rect: Rect) -> bool {
        let request = &mut self.active[k];
        let Some(a) = self.config.animators.get(request.animation) else {
            return true;
        };
        let Some(icon) = request.icon else {
            return true;
        };
        let t = request.t;
        let pos = [a.x.evaluate(t) * rect.width, a.y.evaluate(t) * rect.height];
        let scale = a.scale.evaluate(t);
        let (mut o, mut o1) = (0.0, 0.0);
        if a.oscillate && t >= a.osc_offset {
            let tt = (t - a.osc_offset).clamp(0.0, 1.0) / (1.0 - a.osc_offset) * a.duration;
            (o, o1) = oscillation(a.osc_reduction, a.osc_frequency, request.random, tt);
        }
        let e = self.state.transform.entry(icon.root).or_default();
        e.anchored_position = Some(pos);
        e.scale = Some([scale + o * a.osc_scale, scale + o1 * a.osc_scale, 1.0]);
        let alpha = a.alpha.evaluate(t);
        self.state.renderer_alpha.insert(icon.foreground, alpha);
        if request.t == 1.0 {
            self.state.active.insert(icon.root, false);
            self.pool.push(icon);
            request.icon = None;
            self.events.released.push(icon);
            return true;
        }
        request.t = (request.t + delta_time / a.duration).clamp(0.0, 1.0);
        false
    }
}

#[cfg(test)]
mod tests;
