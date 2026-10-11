//! The Aurora on the game clock (M7f4e): the game's `CrashedShipExploder`
//! counts down from the start of a new game and swaps the intact ship for
//! the wreck; `ShipExteriorCullManager` may hide the wreck's exterior while
//! the camera is in one of the `ShipExteriorCull` volumes. The rules are in
//! `sn_sim::aurora`; this shows and hides the scene's parts.
//!
//! Since M10g the clock is the session's (the server's time since its
//! tick 0, from the first frame) and the countdown counts from the
//! world's start, so every player's Aurora goes at the same tick.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use sn_protocol::AuroraStart;
use sn_sim::V3;
use sn_sim::aurora::{Exploder, ExplosionEvent, ExplosionTimes, cull_check_frame, in_any_volume};

use crate::objects::{ExteriorCullVolume, ObjectStreamer};
use crate::session::Session;

/// `DayNightCycle.timePassed`: game seconds, `start` at the server's tick
/// 0, advancing `speed` (`dayNightSpeed`; `--time-scale`) times the
/// server's time. The sky does not follow it yet (it stays at its start
/// time).
#[derive(Resource, Debug)]
pub struct GameClock {
    pub start: f64,
    pub seconds: f64,
    pub speed: f64,
}

impl GameClock {
    /// The world's settings: `clock_start`, `time_scale`.
    pub fn new(start: f64, speed: f32) -> GameClock {
        GameClock {
            start,
            seconds: start,
            speed: f64::from(speed),
        }
    }

    /// The server's time (seconds since its tick 0) at a clock time.
    pub fn server_seconds(&self, at: f64) -> f64 {
        if self.speed > 0.0 {
            (at - self.start) / self.speed
        } else {
            f64::INFINITY
        }
    }

    /// `timePassedAsFloat`.
    pub fn as_f32(&self) -> f32 {
        self.seconds as f32
    }
}

pub fn tick_clock(session: Res<Session>, mut clock: ResMut<GameClock>) {
    clock.seconds = clock.start + session.server_seconds() * clock.speed;
}

#[derive(Resource)]
pub struct AuroraState {
    start: AuroraStart,
    exploder: Option<Exploder>,
    cull_every: Option<i32>,
    /// The camera was in a cull volume at the last check.
    inside: bool,
    exterior_hidden: bool,
    /// (exploded, exterior hidden) as last applied to the parts.
    applied: Option<(bool, bool)>,
}

impl AuroraState {
    pub fn new(start: AuroraStart) -> AuroraState {
        AuroraState {
            start,
            exploder: None,
            cull_every: None,
            inside: false,
            exterior_hidden: false,
            applied: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    clock: Res<GameClock>,
    session: Res<Session>,
    frame: Res<FrameCount>,
    streamer: Res<ObjectStreamer>,
    mut state: ResMut<AuroraState>,
    camera: Query<&Transform, With<Camera3d>>,
    volumes: Query<&ExteriorCullVolume>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some((data, parts)) = streamer.aurora() else {
        return;
    };
    let now = clock.as_f32();
    let tick_s = session.tick_seconds();
    // The server's tick at a clock time (the shared time base).
    let tick = |at: f32| clock.server_seconds(f64::from(at)) / tick_s;
    let state = &mut *state;
    let exploder = state.exploder.get_or_insert_with(|| {
        let c = data.code;
        let times = ExplosionTimes {
            range: c.range,
            day_seconds: c.day_seconds,
            sound_delay: c.sound_delay,
            fx_delay: c.fx_delay,
            swap_delay: c.swap_delay,
        };
        state.cull_every = data.cull_every;
        let exploder = match state.start {
            // `--aurora-countdown`, or our draw from the lifepod seed
            // (`sn_assets::aurora_countdown`), after the world's start.
            AuroraStart::Countdown { seconds } => {
                Exploder::new_game_at(times, clock.start as f32, seconds)
            }
            AuroraStart::Held { exploded: true } => {
                Exploder::loaded(times, now, now - times.swap_delay - 1.0)
            }
            AuroraStart::Held { exploded: false } => Exploder::loaded(times, now, f32::MAX),
        };
        info!(
            "aurora: {:?}; code: Random.Range({}, {}) × {} s, sound +{} s, explosion +{} s, swap +{} s; cull manager every {:?} frames; game clock {now:.1} s × {}",
            state.start,
            c.range.0,
            c.range.1,
            c.day_seconds,
            c.sound_delay,
            c.fx_delay,
            c.swap_delay,
            data.cull_every,
            clock.speed
        );
        if exploder.initialized() {
            info!(
                "aurora: countdown at {:.1} s (in {:.1} s; server tick {:.1}), swap at {:.1} s (server tick {:.1})",
                exploder.countdown(),
                exploder.countdown() - now,
                tick(exploder.countdown()),
                exploder.swap_time(),
                tick(exploder.swap_time())
            );
        }
        exploder
    });

    let event = exploder.update(now);
    if let Some(event) = event {
        let what = match event {
            ExplosionEvent::Countdown => "countdown starts (its sound and effects: not ported)",
            ExplosionEvent::Sound => "explosion sound (not ported)",
            ExplosionEvent::Explosion => {
                "explosion (effects, camera shake, force, damage: not ported)"
            }
            ExplosionEvent::Swap => "models swapped",
        };
        info!(
            "aurora: {what} at game time {now:.2} s (server tick {:.1})",
            tick(now)
        );
    }

    // `ShipExteriorCullManager.Update`.
    if let (Some(every), Ok(cam)) = (state.cull_every, camera.single())
        && cull_check_frame(u64::from(frame.0), every)
    {
        let p = cam.translation;
        let at = V3::new(f64::from(p.x), f64::from(p.y), -f64::from(p.z));
        let inside = volumes.iter().any(|v| in_any_volume(&v.0, at));
        if inside != state.inside {
            info!(
                "aurora: camera {} a ship exterior cull volume ({} registered)",
                if inside { "entered" } else { "left" },
                volumes.iter().map(|v| v.0.len()).sum::<usize>()
            );
            state.inside = inside;
        }
        if let Some(visible) = exploder.cull_exterior(!inside) {
            state.exterior_hidden = !visible;
        }
    }

    let want = (exploder.is_exploded(), state.exterior_hidden);
    if state.applied != Some(want) {
        let (mut shown, mut hidden) = (0, 0);
        for (show, entities) in &parts {
            let on = show.shown(want.0, want.1);
            if on {
                shown += 1;
            } else {
                hidden += 1;
            }
            for e in *entities {
                if let Ok(mut v) = visibility.get_mut(*e) {
                    *v = if on {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                }
            }
        }
        info!(
            "aurora: {} (exterior hidden: {}), {shown} parts shown, {hidden} hidden",
            if want.0 { "exploded" } else { "intact" },
            want.1
        );
        state.applied = Some(want);
    }
}
