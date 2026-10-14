//! Small time helpers the UI's scripts share (M9u6a; `docs/formats/ui.md`
//! § The live UI), ported from the decompiled code (read only; the code
//! is ours):
//! - [`smooth_damp`]: `UnityEngine.Mathf.SmoothDamp` (`UnityEngine.
//!   CoreModule`), a critically damped spring;
//! - [`Sequence`]: UWE's `Sequence` (`Assembly-CSharp`), a value `t`
//!   moving between 0 and 1 over a time, used by faders and icons.
//!
//! Single precision, as the game computes them.

/// `Mathf.SmoothDamp(current, target, ref velocity, smoothTime, maxSpeed,
/// deltaTime)`: the next value; `velocity` is updated.
pub fn smooth_damp(
    current: f32,
    target: f32,
    velocity: &mut f32,
    smooth_time: f32,
    max_speed: f32,
    delta_time: f32,
) -> f32 {
    let smooth_time = smooth_time.max(0.0001);
    let omega = 2.0 / smooth_time;
    let x = omega * delta_time;
    let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let original_target = target;
    let max_change = max_speed * smooth_time;
    let change = (current - target).clamp(-max_change, max_change);
    let target = current - change;
    let temp = (*velocity + omega * change) * delta_time;
    *velocity = (*velocity - omega * temp) * exp;
    let mut out = target + (change + temp) * exp;
    // Never overshoot the target.
    if (original_target - current > 0.0) == (out > original_target) {
        out = original_target;
        *velocity = (out - original_target) / delta_time;
    }
    out
}

/// UWE's `Sequence`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sequence {
    t: f32,
    time: f32,
    target: bool,
    dir: f32,
    active: bool,
}

impl Default for Sequence {
    /// `new Sequence()` (`Reset`).
    fn default() -> Sequence {
        Sequence {
            t: 0.0,
            time: 1.0,
            target: true,
            dir: 1.0,
            active: false,
        }
    }
}

impl Sequence {
    pub fn t(&self) -> f32 {
        self.t
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn target(&self) -> bool {
        self.target
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    /// `ForceState`: `t` jumps to the state; the next `update` ends it.
    pub fn force_state(&mut self, state: bool) {
        self.t = if state { 1.0 } else { 0.0 };
        self.target = state;
        self.dir = if state { 1.0 } else { -1.0 };
        self.time = 1.0;
        self.active = true;
    }

    /// `Set(time, current, target)`: from `current`'s end towards
    /// `target`'s.
    pub fn set_from(&mut self, time: f32, current: bool, target: bool) {
        self.t = if current { 1.0 } else { 0.0 };
        self.set(time, target);
    }

    /// `Set(time, target)`: from where `t` is towards `target`'s end.
    pub fn set(&mut self, time: f32, target: bool) {
        self.time = time;
        self.target = target;
        self.dir = if target { 1.0 } else { -1.0 };
        self.active = true;
    }

    /// `SetT`.
    pub fn set_t(&mut self, t: f32) {
        self.t = t.clamp(0.0, 1.0);
    }

    /// `Update(dT)`. Returns true when this call ended the sequence (where
    /// the game calls its callback): one call after `t` reached the
    /// target, as in the game.
    pub fn update(&mut self, dt: f32) -> bool {
        if !self.active {
            return false;
        }
        let end = if self.target { 1.0 } else { 0.0 };
        if self.t == end {
            self.active = false;
            return true;
        }
        if self.time == 0.0 {
            self.t = end;
        } else {
            self.t = (self.t + self.dir * dt / self.time).clamp(0.0, 1.0);
        }
        false
    }
}

#[cfg(test)]
mod tests;
