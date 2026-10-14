//! Small time helpers the UI's scripts share (M9u6a; `docs/formats/ui.md`
//! § The live UI), ported from the decompiled code (read only; the code
//! is ours):
//! - [`smooth_damp`]: `UnityEngine.Mathf.SmoothDamp` (`UnityEngine.
//!   CoreModule`), a critically damped spring;
//! - [`Sequence`]: UWE's `Sequence` (`Assembly-CSharp`), a value `t`
//!   moving between 0 and 1 over a time, used by faders and icons.
//! - [`CoroutineTween`]: UWE's `CoroutineTween` (M9u6b2), a coroutine
//!   calling back a scalar from 0 to 1 over a duration, once or in a loop;
//! - [`coin_rotation`], [`spring`] and [`oscillation`]: `MathExtensions`
//!   (`Assembly-CSharp-firstpass`), the bars' icon turn and punch;
//! - [`approximately`], `Mathf.Approximately`.
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

/// `CoroutineTween.Mode` (`PingPong` is not used by the HUD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TweenMode {
    Once,
    Loop,
}

/// What a [`CoroutineTween::move_next`] asks its owner to do, in order:
/// `onUpdate(scalar)`, then `onStop`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TweenCalls {
    pub update: Option<f32>,
    pub stopped: bool,
}

/// UWE's `CoroutineTween`: Unity resumes it once a frame after the
/// `Update`s (`yield null`), and once more when it is started
/// (`StartCoroutine` runs an iterator to its first yield at once).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoroutineTween {
    pub mode: TweenMode,
    time: f32,
    duration: f32,
    running: bool,
}

impl CoroutineTween {
    /// `new CoroutineTween(container) { mode, duration }` (the default
    /// duration is 1).
    pub fn new(mode: TweenMode, duration: f32) -> CoroutineTween {
        let mut t = CoroutineTween {
            mode,
            time: 0.0,
            duration: 1.0,
            running: false,
        };
        t.set_duration(duration);
        t
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// The `duration` setter: at least 0; the time keeps its fraction.
    pub fn set_duration(&mut self, value: f32) {
        let value = value.max(0.0);
        if self.duration != value {
            let f = if self.duration > 0.0 {
                self.time / self.duration
            } else {
                0.0
            };
            self.duration = value;
            self.time = f * self.duration;
        }
    }

    /// `Start` with the container active: `Stop` (true: the old run's
    /// `onStop` is called first), time 0, `onStart`, then the first
    /// `MoveNext` with `dt` (its calls returned).
    pub fn start(&mut self, dt: f32) -> (bool, TweenCalls) {
        let stopped = self.stop();
        self.time = 0.0;
        self.running = true;
        (stopped, self.move_next(dt))
    }

    /// `Stop`: true if it was running (`onStop` is called).
    pub fn stop(&mut self) -> bool {
        std::mem::replace(&mut self.running, false)
    }

    /// `MoveNext` with this frame's `dt`.
    pub fn move_next(&mut self, dt: f32) -> TweenCalls {
        let mut calls = TweenCalls::default();
        if !self.running || self.duration <= 0.0 {
            return calls;
        }
        self.time += dt;
        match self.mode {
            TweenMode::Once => {
                if self.time > self.duration {
                    self.time = self.duration;
                }
                calls.update = Some(self.time / self.duration);
                if self.time >= self.duration {
                    self.running = false;
                    calls.stopped = true;
                }
            }
            TweenMode::Loop => {
                self.time %= self.duration;
                calls.update = Some(self.time / self.duration);
            }
        }
        calls
    }
}

/// `Mathf.Approximately`.
pub fn approximately(a: f32, b: f32) -> bool {
    // `Mathf.Epsilon` is the smallest positive subnormal.
    (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(f32::from_bits(1) * 8.0)
}

/// `MathExtensions.Spring`: one step of a damped spring.
pub fn spring(
    velocity: &mut f32,
    current: &mut f32,
    target: f32,
    coef: f32,
    dt: f32,
    velocity_damp: f32,
    velocity_max: f32,
) {
    let force = coef * (target - *current);
    *velocity = (*velocity + force * dt) * velocity_damp;
    if velocity_max > 0.0 {
        let sign = if *velocity >= 0.0 { 1.0 } else { -1.0 };
        if *velocity * sign > velocity_max {
            *velocity = velocity_max * sign;
        }
    }
    *current += *velocity * dt;
}

/// `MathExtensions.CoinRotation`: springs `current` towards `target` in
/// 0.02 s steps; true if it moved. As decompiled, the number of steps is
/// the whole seconds since `time_prev` (`FloorToInt(timeNow −
/// timePrev)`), not the 0.02 s steps in that time: the turn only moves
/// once a second has built up, then one step per frame while it stays
/// above a second (ported as the game runs it).
#[allow(clippy::too_many_arguments)]
pub fn coin_rotation(
    current: &mut f32,
    target: f32,
    time_prev: &mut f32,
    time_now: f32,
    velocity: &mut f32,
    spring_coef: f32,
    velocity_damp: f32,
    velocity_max: f32,
) -> bool {
    let mut step = 0.02f32;
    let elapsed = time_now - *time_prev;
    let mut steps = elapsed.floor() as i32;
    if steps > 20 {
        steps = 1;
        step = elapsed;
    }
    *time_prev += steps as f32 * step;
    let before = *current;
    for _ in 0..steps.max(0) {
        spring(
            velocity,
            current,
            target,
            spring_coef,
            step,
            velocity_damp,
            velocity_max,
        );
        if (target - *current).abs() < 1.0 && velocity.abs() < 1.0 {
            *velocity = 0.0;
            *current = target;
        }
    }
    !approximately(before, *current)
}

/// `MathExtensions.Oscillation(reduction, frequency, seed, t, out o, out
/// o1)`: two opposite decaying waves.
pub fn oscillation(reduction: f32, frequency: f32, seed: f32, t: f32) -> (f32, f32) {
    let seed = seed.clamp(0.0, 1.0);
    let amplitude = 0.5 * reduction.powf(-t);
    let wave = ((t * frequency + seed) * 2.0 * std::f32::consts::PI).sin();
    (amplitude * (1.0 + wave), amplitude * (1.0 - wave))
}

#[cfg(test)]
mod tests;
