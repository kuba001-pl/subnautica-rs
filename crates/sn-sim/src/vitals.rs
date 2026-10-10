//! Oxygen, health, suffocation, death and respawn (M9c, `docs/DESIGN.md`
//! § 4.3 "M9c plan"): `Player.Update` (breaths by depth class,
//! `SuffocationUpdate`), `OxygenManager.Update` (refill at the surface),
//! `LiveMixin` (health, `TakeDamage`, `Kill`), `Player.OnLand` (fall
//! damage) and `Player.ResetPlayerOnDeath` (the respawn timeline). Our own
//! code after reading the game's; the numbers that are data come from the
//! install ([`VitalsParams`]), the constants inside the game's methods are
//! ported here and named after their method.
//!
//! One call to [`Vitals::step`] covers `dt` seconds of game time. The game
//! runs these rules once per frame; we run them once per physics step.
//! Only the player's own lungs are an oxygen source (no tanks, subs,
//! vehicles or rebreather yet), and the game mode is Survival.

/// What the rules need, from the game's data (`sn-assets`'
/// `vitals_params`).
#[derive(Clone, Debug, PartialEq)]
pub struct VitalsParams {
    /// The player's `Oxygen.oxygenCapacity`.
    pub oxygen_capacity: f64,
    /// `OxygenManager.oxygenUnitsPerSecondSurface`.
    pub refill_per_second: f64,
    /// How far the `Oxygen` object is above the player's transform.
    pub oxygen_above_player: f64,
    /// `Player.suffocationTime`, `suffocationRecoveryTime`.
    pub suffocation_time: f64,
    pub suffocation_recovery_time: f64,
    /// `LiveMixinData.maxHealth`, `LiveMixin.startHealthPercent`.
    pub max_health: f64,
    pub start_health_percent: f64,
    /// `Ocean.GetOceanLevel()`.
    pub ocean_level: f64,
}

/// `Ocean.DepthClass` (the values are the game's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DepthClass {
    Surface = 0,
    Safe = 1,
    Unsafe = 2,
    Crush = 3,
}

/// `Player.GetSurfaceDepth`.
pub const SURFACE_DEPTH: f64 = 0.1;

/// `Player.GetDepthClass` without a `CrushDamage` (the player on foot or
/// swimming): > 200 m crush, > 100 m unsafe, > 0.1 m safe.
pub fn depth_class(depth: f64) -> DepthClass {
    if depth > 200.0 {
        DepthClass::Crush
    } else if depth > 100.0 {
        DepthClass::Unsafe
    } else if depth > SURFACE_DEPTH {
        DepthClass::Safe
    } else {
        DepthClass::Surface
    }
}

/// `Player.GetBreathPeriod` (normal mode, no rebreather, no water park).
pub fn breath_period(class: DepthClass) -> f64 {
    match class {
        DepthClass::Crush => 1.5,
        DepthClass::Unsafe => 2.25,
        DepthClass::Safe => 3.0,
        DepthClass::Surface => 99999.0,
    }
}

/// `Player.GetOxygenPerBreath` (no rebreather, not piloting, Survival).
pub fn oxygen_per_breath(period: f64, class: DepthClass) -> f64 {
    let cost = match class {
        DepthClass::Unsafe => 1.5,
        DepthClass::Crush => 2.0,
        _ => 1.0,
    };
    period * cost
}

/// `Ocean.GetDepthOf`.
pub fn depth_of(params: &VitalsParams, y: f64) -> f64 {
    (params.ocean_level - y).max(0.0)
}

/// `Player.OnLand`: damage from a landing at vertical speed `impact_y`
/// (negative falling), before `DamageSystem.CalculateDamage`
/// (`damageMultiplier` 1, no `DamageModifier` on the player).
pub fn fall_damage(impact_y: f64) -> f64 {
    -(impact_y + 10.0).min(0.0) * 2.5
}

/// `Player.ResetPlayerOnDeath`: seconds from death to the move to the
/// respawn point, from there to health and oxygen back (plus the world
/// settling), and from there to the controls back.
pub const DEATH_TO_RESPAWN: f64 = 5.0;
pub const RESPAWN_TO_RESTORE: f64 = 1.0;
pub const RESTORE_TO_CONTROLS: f64 = 1.0;

/// `Player.SuffocationState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suffocation {
    None,
    Suffocating,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callback {
    Die,
    Recover,
}

/// The game's `Sequence`: `t` runs towards 0 or 1 over `time` seconds and
/// calls back on the update after it gets there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sequence {
    t: f64,
    time: f64,
    target: bool,
    active: bool,
    callback: Option<Callback>,
}

impl Sequence {
    fn new(state: bool) -> Sequence {
        Sequence {
            t: if state { 1.0 } else { 0.0 },
            time: 0.0,
            target: state,
            active: true,
            callback: None,
        }
    }

    fn set(&mut self, time: f64, target: bool, callback: Option<Callback>) {
        self.time = time;
        self.target = target;
        self.callback = callback;
        self.active = true;
    }

    /// Returns the callback when it fires.
    fn update(&mut self, dt: f64) -> Option<Callback> {
        if !self.active {
            return None;
        }
        let goal = if self.target { 1.0 } else { 0.0 };
        if self.t == goal {
            self.active = false;
            return self.callback;
        }
        if self.time == 0.0 {
            self.t = goal;
        } else {
            let dir = if self.target { 1.0 } else { -1.0 };
            self.t = (self.t + dir * dt / self.time).clamp(0.0, 1.0);
        }
        None
    }
}

/// Where the player is in the death and respawn timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Life {
    Alive,
    /// Died at this game time; the body stays, controls off.
    Dead {
        at: f64,
    },
    /// Moved to the respawn point at this time; waiting for the world.
    Respawning {
        at: f64,
    },
    /// Health and oxygen back at this time; controls come back after
    /// [`RESTORE_TO_CONTROLS`].
    Restored {
        at: f64,
    },
}

/// What happened in a step, for logs, the HUD and the caller's moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VitalsEvent {
    /// A breath took this much oxygen.
    Breath(f64),
    SuffocationStarted,
    RecoveryStarted,
    Recovered,
    Damaged(f64),
    Died,
    /// The caller moves the player to the respawn point now (the lifepod's
    /// `playerSpawn`, inside the pod) and holds it still.
    MoveToRespawn,
    /// Health and oxygen are full again.
    Restored,
    /// Input, movement and mouse look are back.
    ControlsBack,
}

/// The player's surroundings in a step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Situation {
    /// The player's transform's height.
    pub y: f64,
    /// Inside the lifepod (`Player.escapePod`).
    pub in_pod: bool,
    /// The walking motor landed this step, at this vertical velocity.
    pub landed: Option<f64>,
    /// The world around the player is loaded (`IsWorldSettled`).
    pub world_settled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vitals {
    pub oxygen: f64,
    pub health: f64,
    pub suffocation: Suffocation,
    pub life: Life,
    /// Game time (`Time.time`), seconds.
    pub time: f64,
    /// The breath clock's previous value (`timeMonitor.prevValue`).
    prev_time: f64,
    sequence: Sequence,
    /// `Player.IsFrozenStats`: no breaths.
    frozen: bool,
}

impl Vitals {
    /// Full oxygen and health (a new game).
    pub fn new(params: &VitalsParams) -> Vitals {
        Vitals {
            oxygen: params.oxygen_capacity,
            health: params.max_health,
            suffocation: Suffocation::None,
            life: Life::Alive,
            time: 0.0,
            prev_time: 0.0,
            sequence: Sequence::new(true),
            frozen: false,
        }
    }

    /// `Player.IsUnderwater`: never in the pod, else below the ocean.
    pub fn underwater(params: &VitalsParams, s: &Situation) -> bool {
        !s.in_pod && s.y < params.ocean_level
    }

    /// Movement, input and mouse look run (off from death until the
    /// respawn is over).
    pub fn controls_enabled(&self) -> bool {
        self.life == Life::Alive
    }

    /// The suffocation overlay's opacity (`uGUI.main.overlays.Set(0, 1 −
    /// t)`): 0 normally, 1 when passed out.
    pub fn overlay(&self) -> f64 {
        1.0 - self.sequence.t
    }

    /// Seconds of breathing left as the game shows them
    /// (`Oxygen.GetSecondsLeft`).
    pub fn seconds_left(&self) -> i64 {
        if self.oxygen > 0.5 {
            // Mathf.RoundToInt rounds half to even.
            self.oxygen.round_ties_even() as i64
        } else {
            0
        }
    }

    /// `LiveMixin.TakeDamage` for the player (no shield, not invincible):
    /// returns the damage done.
    pub fn take_damage(&mut self, amount: f64, events: &mut Vec<VitalsEvent>) -> f64 {
        if self.health <= 0.0 {
            return 0.0;
        }
        let done = amount.max(0.0);
        self.health = (self.health - done).max(0.0);
        events.push(VitalsEvent::Damaged(done));
        if self.health <= 0.0 {
            self.kill(events);
        }
        done
    }

    /// `LiveMixin.Kill` then `Player.OnKill`.
    fn kill(&mut self, events: &mut Vec<VitalsEvent>) {
        self.health = 0.0;
        if self.life == Life::Alive {
            self.life = Life::Dead { at: self.time };
            self.frozen = true;
            events.push(VitalsEvent::Died);
        }
    }

    /// `Player.SuffocationReset`.
    fn suffocation_reset(&mut self) {
        self.sequence = Sequence {
            t: 1.0,
            time: 0.0,
            target: true,
            active: true,
            callback: None,
        };
        self.suffocation = Suffocation::None;
    }

    /// `OxygenManager.AddOxygen` with only the player's lungs.
    fn add_oxygen(&mut self, params: &VitalsParams, amount: f64) {
        self.oxygen += amount.min(params.oxygen_capacity - self.oxygen);
    }

    /// One step of `dt` seconds.
    pub fn step(&mut self, params: &VitalsParams, dt: f64, s: &Situation) -> Vec<VitalsEvent> {
        let mut events = Vec::new();
        self.prev_time = self.time;
        self.time += dt;
        let underwater = Self::underwater(params, s);
        let can_breathe = !underwater;
        let class = depth_class(depth_of(params, s.y));

        // `Player.SuffocationUpdate`: `Utils.NearlyEqual(x, 0)` is only
        // true for exactly 0.
        let has_oxygen = self.oxygen != 0.0;
        if has_oxygen != self.sequence.target {
            if has_oxygen {
                self.suffocation = Suffocation::Recovering;
                self.sequence.set(
                    params.suffocation_recovery_time,
                    true,
                    Some(Callback::Recover),
                );
                events.push(VitalsEvent::RecoveryStarted);
            } else {
                self.suffocation = Suffocation::Suffocating;
                self.sequence
                    .set(params.suffocation_time, false, Some(Callback::Die));
                events.push(VitalsEvent::SuffocationStarted);
            }
        }
        match self.sequence.update(dt) {
            Some(Callback::Die) => {
                if self.health > 0.0 {
                    self.kill(&mut events);
                }
            }
            Some(Callback::Recover) => {
                self.suffocation_reset();
                events.push(VitalsEvent::Recovered);
            }
            None => {}
        }

        // `Player.Update`: a breath each time the clock crosses a multiple
        // of the breath period.
        if !can_breathe && !self.frozen {
            let period = breath_period(class);
            if (self.time / period).floor() != (self.prev_time / period).floor() {
                let want = oxygen_per_breath(period, class);
                let removed = want.min(self.oxygen);
                self.oxygen = (self.oxygen - removed).max(0.0);
                events.push(VitalsEvent::Breath(removed));
            }
        }

        // `OxygenManager.Update` (no cinematic plays in our hatches).
        let source_y = s.y + params.oxygen_above_player;
        if source_y > params.ocean_level - 1.0 || can_breathe {
            self.add_oxygen(params, dt * params.refill_per_second);
        }

        // `Player.OnLand` (the walking motor), only out of the water.
        if let Some(impact_y) = s.landed
            && !underwater
        {
            let damage = fall_damage(impact_y);
            if damage > 0.0 {
                self.take_damage(damage, &mut events);
            }
        }

        // `Player.ResetPlayerOnDeath`.
        match self.life {
            Life::Dead { at } if self.time - at >= DEATH_TO_RESPAWN => {
                self.life = Life::Respawning { at: self.time };
                events.push(VitalsEvent::MoveToRespawn);
            }
            Life::Respawning { at } if self.time - at >= RESPAWN_TO_RESTORE && s.world_settled => {
                // `ResetHealth`, then the respawn event's `OnRespawn`.
                self.health = params.max_health * params.start_health_percent;
                self.oxygen = 0.0;
                self.add_oxygen(params, params.oxygen_capacity);
                self.suffocation_reset();
                self.life = Life::Restored { at: self.time };
                events.push(VitalsEvent::Restored);
            }
            Life::Restored { at } if self.time - at >= RESTORE_TO_CONTROLS => {
                self.frozen = false;
                self.life = Life::Alive;
                events.push(VitalsEvent::ControlsBack);
            }
            _ => {}
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 0.02;

    fn params() -> VitalsParams {
        VitalsParams {
            oxygen_capacity: 45.0,
            refill_per_second: 30.0,
            oxygen_above_player: 0.0,
            suffocation_time: 8.0,
            suffocation_recovery_time: 4.0,
            max_health: 100.0,
            start_health_percent: 1.0,
            ocean_level: 0.0,
        }
    }

    fn at(y: f64) -> Situation {
        Situation {
            y,
            world_settled: true,
            ..Situation::default()
        }
    }

    /// Runs `seconds` and returns every event with its time.
    fn run(
        v: &mut Vitals,
        p: &VitalsParams,
        s: Situation,
        seconds: f64,
    ) -> Vec<(f64, VitalsEvent)> {
        let mut out = Vec::new();
        for _ in 0..(seconds / DT).round() as usize {
            for e in v.step(p, DT, &s) {
                out.push((v.time, e));
            }
        }
        out
    }

    #[test]
    fn depth_classes_and_costs_match_the_code() {
        assert_eq!(depth_class(0.1), DepthClass::Surface);
        assert_eq!(depth_class(0.11), DepthClass::Safe);
        assert_eq!(depth_class(100.0), DepthClass::Safe);
        assert_eq!(depth_class(100.5), DepthClass::Unsafe);
        assert_eq!(depth_class(200.5), DepthClass::Crush);
        // Units per second: 1, 1.5, 2.
        for (class, rate) in [
            (DepthClass::Safe, 1.0),
            (DepthClass::Unsafe, 1.5),
            (DepthClass::Crush, 2.0),
        ] {
            let period = breath_period(class);
            assert!((oxygen_per_breath(period, class) / period - rate).abs() < 1e-12);
        }
    }

    #[test]
    fn no_drain_in_the_pod_or_at_the_surface() {
        let p = params();
        let mut v = Vitals::new(&p);
        let pod = Situation {
            in_pod: true,
            ..at(-5.0)
        };
        run(&mut v, &p, pod, 30.0);
        assert_eq!(v.oxygen, 45.0);
        // Floating with the head just under the surface: surface class,
        // and the source is above level − 1, so it refills.
        run(&mut v, &p, at(-0.05), 30.0);
        assert_eq!(v.oxygen, 45.0);
    }

    #[test]
    fn drain_by_depth() {
        let p = params();
        for (y, per_second) in [(-10.0, 1.0), (-150.0, 1.5), (-250.0, 2.0)] {
            let mut v = Vitals::new(&p);
            // 19 s: the breaths up to 18 s (off the boundary, so the
            // clock's rounding does not matter).
            let events = run(&mut v, &p, at(y), 19.0);
            let breaths: Vec<f64> = events
                .iter()
                .filter_map(|(_, e)| match e {
                    VitalsEvent::Breath(x) => Some(*x),
                    _ => None,
                })
                .collect();
            let used: f64 = breaths.iter().sum();
            assert!(
                (used - 18.0 * per_second).abs() < 1e-9,
                "{y}: {used} in {} breaths",
                breaths.len()
            );
            assert!((v.oxygen - (45.0 - used)).abs() < 1e-9);
        }
    }

    #[test]
    fn refills_at_the_read_rate() {
        let p = params();
        let mut v = Vitals::new(&p);
        run(&mut v, &p, at(-10.0), 31.0);
        assert!((v.oxygen - 15.0).abs() < 1e-9, "{}", v.oxygen);
        // Back at the surface: 30 units in 1 s.
        run(&mut v, &p, at(0.5), 0.5);
        assert!((v.oxygen - 30.0).abs() < 1e-9, "{}", v.oxygen);
        run(&mut v, &p, at(0.5), 1.0);
        assert_eq!(v.oxygen, 45.0);
        // The source is tested 1 m below the level.
        let deep_head = VitalsParams {
            oxygen_above_player: 0.5,
            ..p.clone()
        };
        let mut v = Vitals::new(&deep_head);
        v.oxygen = 10.0;
        run(&mut v, &deep_head, at(-1.4), 0.1);
        assert!(v.oxygen > 10.0);
        let mut v = Vitals::new(&deep_head);
        v.oxygen = 10.0;
        run(&mut v, &deep_head, at(-1.6), 0.1);
        assert_eq!(v.oxygen, 10.0);
    }

    #[test]
    fn suffocation_kills_after_the_read_time_and_respawns() {
        let p = params();
        let mut v = Vitals::new(&p);
        let events = run(&mut v, &p, at(-10.0), 61.0);
        let when = |want: VitalsEvent| {
            events
                .iter()
                .find(|(_, e)| *e == want)
                .map(|(t, _)| *t)
                .unwrap_or_else(|| panic!("no {want:?}"))
        };
        // 45 units at 1 per s, in breaths of 3 every 3 s: empty at 45 s.
        // (Each event can land one step later from rounding of the clock.)
        let empty = when(VitalsEvent::SuffocationStarted);
        assert!((empty - 45.02).abs() < 0.03, "{empty}");
        // t runs from 1 to 0 over 8 s, then the next update kills.
        let died = when(VitalsEvent::Died);
        assert!((died - empty - 8.02).abs() < 0.03, "{}", died - empty);
        let moved = when(VitalsEvent::MoveToRespawn);
        assert!((moved - died - DEATH_TO_RESPAWN).abs() < 1e-6);
        let restored = when(VitalsEvent::Restored);
        assert!((restored - moved - RESPAWN_TO_RESTORE).abs() < 1e-6);
        let back = when(VitalsEvent::ControlsBack);
        assert!((back - restored - RESTORE_TO_CONTROLS).abs() < 1e-6);
        // Only one death; stats stay frozen until the controls are back.
        assert_eq!(
            events
                .iter()
                .filter(|(_, e)| *e == VitalsEvent::Died)
                .count(),
            1
        );
        assert!(v.health == 100.0 && v.oxygen == 45.0 && v.controls_enabled());
    }

    #[test]
    fn waiting_for_the_world_delays_the_restore() {
        let p = params();
        let mut v = Vitals::new(&p);
        v.oxygen = 0.0;
        let unsettled = Situation {
            world_settled: false,
            ..at(-10.0)
        };
        let events = run(&mut v, &p, unsettled, 30.0);
        assert!(events.iter().any(|(_, e)| *e == VitalsEvent::MoveToRespawn));
        assert!(!events.iter().any(|(_, e)| *e == VitalsEvent::Restored));
        assert!(!v.controls_enabled());
        let events = run(&mut v, &p, at(-10.0), 0.04);
        assert!(events.iter().any(|(_, e)| *e == VitalsEvent::Restored));
    }

    #[test]
    fn air_before_the_end_recovers() {
        let p = params();
        let mut v = Vitals::new(&p);
        v.oxygen = 0.0;
        // 4 s of the 8 suffocating: t = 0.5.
        run(&mut v, &p, at(-10.0), 4.0);
        assert_eq!(v.suffocation, Suffocation::Suffocating);
        assert!((v.overlay() - 0.5).abs() < 0.01, "{}", v.overlay());
        // Surface: oxygen back, t climbs back to 1 over 4 s (2 s from
        // 0.5), then the reset on the next update.
        let events = run(&mut v, &p, at(0.5), 3.0);
        let recovered = events
            .iter()
            .find(|(_, e)| *e == VitalsEvent::Recovered)
            .map(|(t, _)| *t)
            .expect("recovered");
        assert!((recovered - 6.06).abs() < 0.05, "{recovered}");
        assert_eq!(v.suffocation, Suffocation::None);
        assert_eq!(v.overlay(), 0.0);
        assert!(v.life == Life::Alive && v.health == 100.0);
    }

    #[test]
    fn fall_damage_by_the_formula() {
        assert_eq!(fall_damage(-9.0), 0.0);
        assert_eq!(fall_damage(-14.0), 10.0);
        let p = params();
        let mut v = Vitals::new(&p);
        let land = |vy| Situation {
            landed: Some(vy),
            ..at(5.0)
        };
        let e = v.step(&p, DT, &land(-20.0));
        assert_eq!(e, vec![VitalsEvent::Damaged(25.0)]);
        assert_eq!(v.health, 75.0);
        // Under water a landing does not hurt.
        let wet = Situation {
            landed: Some(-30.0),
            ..at(-5.0)
        };
        v.step(&p, DT, &wet);
        assert_eq!(v.health, 75.0);
        // A fall at 40 m/s does 75 and kills.
        let e = v.step(&p, DT, &land(-40.0));
        assert!(e.contains(&VitalsEvent::Died) && v.health == 0.0);
        // Dead: no more damage.
        assert!(v.step(&p, DT, &land(-40.0)).is_empty());
    }

    #[test]
    fn respawn_health_uses_the_start_percent() {
        let p = VitalsParams {
            start_health_percent: 0.5,
            ..params()
        };
        let mut v = Vitals::new(&p);
        v.take_damage(500.0, &mut Vec::new());
        run(&mut v, &p, at(5.0), 7.1);
        assert_eq!(v.life, Life::Alive);
        assert_eq!(v.health, 50.0);
        assert_eq!(v.oxygen, 45.0);
    }

    #[test]
    fn seconds_left_rounds_like_unity() {
        let p = params();
        let mut v = Vitals::new(&p);
        for (o, s) in [(44.5, 44), (45.5, 46), (0.5, 0), (0.51, 1)] {
            v.oxygen = o;
            assert_eq!(v.seconds_left(), s, "{o}");
        }
        assert_eq!(p.oxygen_capacity, 45.0);
    }
}
