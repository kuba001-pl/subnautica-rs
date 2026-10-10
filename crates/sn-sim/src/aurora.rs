//! The Aurora's explosion and the cull of its exploded exterior (M7f4e),
//! as the game's `CrashedShipExploder`, `ShipExteriorCullManager` and
//! `ShipExteriorCull` do it (`docs/formats/gameplay.md` § The Aurora's
//! explosion).
//!
//! Times are the game clock (`DayNightCycle.timePassedAsFloat`, seconds;
//! 1,200 per day), in `f32` as the game keeps them.

use crate::V3;

/// The exploder's numbers, all from the game's code (`Assembly-CSharp`):
/// `SetExplodeTime`'s `Random.Range(2.3f, 4f) * 1200f` and the `const`
/// delays after the countdown starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplosionTimes {
    /// `Random.Range` bounds, in days.
    pub range: (f32, f32),
    /// Seconds per day of the range.
    pub day_seconds: f32,
    /// `delayBeforeExplosionSound`.
    pub sound_delay: f32,
    /// `delayBeforeExplosionFX`: the explosion (effects, shake, force,
    /// damage).
    pub fx_delay: f32,
    /// `delayBeforeSwap`: the intact models are swapped for the wreck.
    pub swap_delay: f32,
}

impl ExplosionTimes {
    /// Seconds from the start of a new game to the countdown, for a draw
    /// `value` in [0, 1] (Unity's `Random.Range(min, max)` is
    /// `min + (max − min) · value`, both ends included).
    pub fn delay(&self, value: f32) -> f32 {
        let (min, max) = self.range;
        let days = min + (max - min) * value.clamp(0.0, 1.0);
        days * self.day_seconds
    }
}

/// What an update started (`CrashedShipExploder.Update`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplosionEvent {
    /// The countdown: its sound, `OnShipExplode`, the warning effects.
    Countdown,
    /// The explosion's sound.
    Sound,
    /// The explosion: effects, camera shake, explosive force and damage.
    Explosion,
    /// `SwapModels(exploded: true)`.
    Swap,
}

/// `CrashedShipExploder`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct Exploder {
    times: ExplosionTimes,
    /// `timeToStartWarning`.
    warning: f32,
    /// `timeToStartCountdown`.
    countdown: f32,
    /// `timeMonitor`: the clock at the last two updates.
    previous: f32,
    current: f32,
    initialized: bool,
    deserialized: bool,
    /// Delay used when a new game initialises.
    first_delay: f32,
}

impl Exploder {
    /// A new game. `Start` shows the intact ship; the first update once
    /// the world is ready sets the countdown `delay` seconds after the
    /// clock at that update (`SetExplodeTime`; `delay` from
    /// [`ExplosionTimes::delay`]).
    pub fn new_game(times: ExplosionTimes, delay: f32) -> Exploder {
        Exploder {
            times,
            warning: 0.0,
            countdown: 0.0,
            previous: 0.0,
            current: 0.0,
            initialized: false,
            deserialized: false,
            first_delay: delay,
        }
    }

    /// As after loading a save (`OnProtoDeserialize`, version 2) whose
    /// clock is `now` and countdown `countdown`. Shows the ship as
    /// [`Exploder::is_exploded`] says; the countdown is not drawn again.
    pub fn loaded(times: ExplosionTimes, now: f32, countdown: f32) -> Exploder {
        Exploder {
            times,
            warning: countdown,
            countdown,
            previous: now,
            current: now,
            initialized: false,
            deserialized: true,
            first_delay: 0.0,
        }
    }

    /// `Update` with the world streamer ready and the clock at `now`.
    /// Like the game's `else if` chain, at most one event per update: if
    /// the clock jumps past several, only the latest one fires.
    pub fn update(&mut self, now: f32) -> Option<ExplosionEvent> {
        if !self.initialized && !self.deserialized {
            // `SetExplodeTime`: from the clock as it is now, before the
            // monitor takes this frame's value.
            self.warning = now;
            self.countdown = self.warning + self.first_delay;
            self.initialized = true;
        }
        self.previous = self.current;
        self.current = now;
        let t = self.times;
        let c = self.countdown;
        if self.just_went_above(c + t.swap_delay) {
            Some(ExplosionEvent::Swap)
        } else if self.just_went_above(c + t.fx_delay) {
            Some(ExplosionEvent::Explosion)
        } else if self.just_went_above(c + t.sound_delay) {
            Some(ExplosionEvent::Sound)
        } else if self.just_went_above(c) {
            Some(ExplosionEvent::Countdown)
        } else {
            None
        }
    }

    /// `ScalarMonitor.JustWentAbove`.
    fn just_went_above(&self, threshold: f32) -> bool {
        self.previous <= threshold && self.current > threshold
    }

    /// `IsExploded`: the clock is past the swap.
    pub fn is_exploded(&self) -> bool {
        self.current > self.countdown + self.times.swap_delay
    }

    /// `timeToStartCountdown` (0 until a new game initialises).
    pub fn countdown(&self) -> f32 {
        self.countdown
    }

    /// `timeToStartWarning`.
    pub fn warning(&self) -> f32 {
        self.warning
    }

    /// When the models are swapped.
    pub fn swap_time(&self) -> f32 {
        self.countdown + self.times.swap_delay
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }

    /// `CullExplodedExterior(visible)`: `Some(active)` when the game sets
    /// the exploded exterior's active flag, `None` when it leaves it. It
    /// acts only when the exploder is both initialised (a new game's first
    /// update) and deserialised (loaded from a save) and the ship has
    /// exploded, so never in a game started in this session.
    pub fn cull_exterior(&self, visible: bool) -> Option<bool> {
        (self.initialized && self.deserialized && self.is_exploded()).then_some(visible)
    }
}

/// `ShipExteriorCullManager.Update` runs its check this frame
/// (`Time.frameCount % updateEveryXFrames == 0`). C#'s `% 0` throws, so
/// the manager then never checks.
pub fn cull_check_frame(frame: u64, every: i32) -> bool {
    u64::try_from(every).is_ok_and(|n| n > 0 && frame % n == 0)
}

/// One box of a `ShipExteriorCull`, in the world: a `BoxCollider`'s box
/// through its Transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullBox {
    pub center: V3,
    /// Unit and perpendicular: the Transform's axes.
    pub axes: [V3; 3],
    /// `size / 2` along each axis, scaled by the Transform's scale (its
    /// absolute value). A negative size gives a box nothing is inside.
    pub half: [f64; 3],
}

impl CullBox {
    /// `ShipExteriorCull.PointInOABB`: strictly inside on every axis.
    pub fn contains(&self, p: V3) -> bool {
        let d = p - self.center;
        (0..3).all(|i| {
            let x = d.dot(self.axes[i]);
            x < self.half[i] && x > -self.half[i]
        })
    }
}

/// `ShipExteriorCull.IsInVolume` over every registered volume.
pub fn in_any_volume(boxes: &[CullBox], camera: V3) -> bool {
    boxes.iter().any(|b| b.contains(camera))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMES: ExplosionTimes = ExplosionTimes {
        range: (2.3, 4.0),
        day_seconds: 1200.0,
        sound_delay: 24.0,
        fx_delay: 25.0,
        swap_delay: 27.0,
    };

    #[test]
    fn the_delay_spans_the_range() {
        assert!((TIMES.delay(0.0) - 2760.0).abs() < 1e-3);
        assert!((TIMES.delay(1.0) - 4800.0).abs() < 1e-3);
        assert!((TIMES.delay(0.5) - 3780.0).abs() < 1e-3);
        // Out-of-range draws are kept inside it.
        assert_eq!(TIMES.delay(2.0), TIMES.delay(1.0));
    }

    #[test]
    fn a_new_game_counts_down_then_explodes_then_swaps() {
        let mut e = Exploder::new_game(TIMES, 100.0);
        assert!(!e.initialized());
        // The first update sets the countdown from the clock then.
        assert_eq!(e.update(480.0), None);
        assert_eq!(e.warning(), 480.0);
        assert_eq!(e.countdown(), 580.0);
        assert_eq!(e.swap_time(), 607.0);
        let mut events = Vec::new();
        let mut t = 480.0;
        while t < 700.0 {
            t += 0.5;
            if let Some(ev) = e.update(t) {
                events.push((t, ev));
            }
        }
        assert_eq!(
            events,
            [
                (580.5, ExplosionEvent::Countdown),
                (604.5, ExplosionEvent::Sound),
                (605.5, ExplosionEvent::Explosion),
                (607.5, ExplosionEvent::Swap),
            ]
        );
        assert!(e.is_exploded());
    }

    #[test]
    fn a_jump_past_several_times_fires_only_the_latest() {
        let mut e = Exploder::new_game(TIMES, 10.0);
        e.update(0.0);
        // Exactly at a threshold is not above it.
        assert_eq!(e.update(10.0), None);
        assert!(!e.is_exploded());
        assert_eq!(e.update(100.0), Some(ExplosionEvent::Swap));
        assert_eq!(e.update(200.0), None);
        assert!(e.is_exploded());
    }

    #[test]
    fn the_exterior_cull_needs_a_loaded_and_initialised_exploder() {
        // A new game: initialised, not deserialised: never culls.
        let mut e = Exploder::new_game(TIMES, 0.0);
        e.update(0.0);
        e.update(100.0);
        assert!(e.is_exploded());
        assert_eq!(e.cull_exterior(false), None);
        // A loaded save: deserialised, never initialised: never culls.
        let mut l = Exploder::loaded(TIMES, 5000.0, 1000.0);
        assert!(l.is_exploded());
        l.update(5001.0);
        assert!(!l.initialized());
        assert_eq!(l.countdown(), 1000.0);
        assert_eq!(l.cull_exterior(false), None);
        // Both (the game's legacy-save path): culls once exploded.
        let mut both = Exploder::new_game(TIMES, 0.0);
        both.update(0.0);
        both.deserialized = true;
        assert_eq!(both.cull_exterior(false), None);
        both.update(100.0);
        assert_eq!(both.cull_exterior(false), Some(false));
        assert_eq!(both.cull_exterior(true), Some(true));
    }

    #[test]
    fn a_loaded_save_keeps_its_countdown() {
        let mut e = Exploder::loaded(TIMES, 100.0, 200.0);
        assert!(!e.is_exploded());
        assert_eq!(e.update(150.0), None);
        assert_eq!(e.update(201.0), Some(ExplosionEvent::Countdown));
        assert_eq!(e.update(228.0), Some(ExplosionEvent::Swap));
    }

    #[test]
    fn the_manager_checks_every_nth_frame() {
        let frames: Vec<u64> = (0..35).filter(|&f| cull_check_frame(f, 10)).collect();
        assert_eq!(frames, [0, 10, 20, 30]);
        assert!(!cull_check_frame(10, 0));
        assert!(!cull_check_frame(10, -10));
    }

    #[test]
    fn cull_boxes_are_oriented_and_strict() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        // Turned 45° about y, 4 × 2 × 2 m around (10, 0, 0).
        let b = CullBox {
            center: V3::new(10.0, 0.0, 0.0),
            axes: [
                V3::new(s, 0.0, -s),
                V3::new(0.0, 1.0, 0.0),
                V3::new(s, 0.0, s),
            ],
            half: [2.0, 1.0, 1.0],
        };
        assert!(b.contains(V3::new(10.0, 0.0, 0.0)));
        // 1.9 m along the long axis: inside; across it: outside.
        assert!(b.contains(V3::new(10.0 + 1.9 * s, 0.0, -1.9 * s)));
        assert!(!b.contains(V3::new(10.0 + 1.9 * s, 0.0, 1.9 * s)));
        // On the face: not inside (strict, as the game).
        assert!(!b.contains(V3::new(10.0, 1.0, 0.0)));
        // A negative size: nothing inside.
        let empty = CullBox {
            half: [-2.0, 1.0, 1.0],
            ..b
        };
        assert!(!empty.contains(V3::new(10.0, 0.0, 0.0)));
        assert!(in_any_volume(&[empty, b], V3::new(10.0, 0.5, 0.0)));
        assert!(!in_any_volume(&[], V3::new(10.0, 0.5, 0.0)));
    }
}
