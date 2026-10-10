//! The game's `LightingController` (M7f4g): lighting states (0
//! Operational, 1 Danger, 2 Damaged) of a set of Marmoset skies, lights
//! and emissive renderers, snapped or faded between
//! (`docs/formats/gameplay.md` § The lifepod's light). Lifepod 5 has one;
//! bases and the Cyclops have their own (later).
//!
//! A port of `LightingController`, its `Timer`, `MultiStatesSky`,
//! `MultiStatesLight` and `MultiStatesEmissive`, in `f32` as the game.

/// `Mathf.Lerp`: `t` clamped to 0..1.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * clamp01(t)
}

/// `Mathf.Clamp01` (NaN passes through, as in Unity and `f32::clamp`).
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// `LightingController.Timer`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Timer {
    elapsed: f32,
    time_to_reach: f32,
    finished: bool,
    started: bool,
}

impl Timer {
    fn start(&mut self, time_to_reach: f32) {
        self.elapsed = 0.0;
        self.time_to_reach = time_to_reach;
        self.started = true;
        self.finished = false;
    }

    fn stop(&mut self) {
        self.started = false;
    }

    fn update(&mut self, dt: f32) {
        if self.started && !self.finished {
            self.elapsed += dt;
        }
        if self.elapsed >= self.time_to_reach {
            self.finished = true;
        }
    }

    fn fraction(&self) -> f32 {
        clamp01(self.elapsed / self.time_to_reach)
    }
}

/// A sky's three intensities (`mset.Sky`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyIntensities {
    pub master: f32,
    pub diffuse: f32,
    pub specular: f32,
}

/// `MultiStatesSky`: the values per state, the sky's current values.
#[derive(Clone, Debug, PartialEq)]
pub struct StatesSky {
    pub master: Vec<f32>,
    pub diffuse: Vec<f32>,
    pub specular: Vec<f32>,
    /// The sky's values now.
    pub current: SkyIntensities,
    start: SkyIntensities,
}

impl StatesSky {
    /// `stored`: the sky's own values before the controller touches it.
    pub fn new(
        master: Vec<f32>,
        diffuse: Vec<f32>,
        specular: Vec<f32>,
        stored: SkyIntensities,
    ) -> Self {
        StatesSky {
            master,
            diffuse,
            specular,
            current: stored,
            // The class's field initialisers.
            start: SkyIntensities {
                master: 1.0,
                diffuse: 1.0,
                specular: 1.0,
            },
        }
    }

    fn init_lerp(&mut self) {
        self.start = self.current;
    }

    fn update(&mut self, state: usize, t: f32) {
        if let Some(&v) = self.master.get(state) {
            self.current.master = lerp(self.start.master, v, t);
        }
        if let Some(&v) = self.diffuse.get(state) {
            self.current.diffuse = lerp(self.start.diffuse, v, t);
        }
        if let Some(&v) = self.specular.get(state) {
            self.current.specular = lerp(self.start.specular, v, t);
        }
    }

    fn set(&mut self, state: usize) {
        if let Some(&v) = self.master.get(state) {
            self.current.master = v;
        }
        if let Some(&v) = self.diffuse.get(state) {
            self.current.diffuse = v;
        }
        if let Some(&v) = self.specular.get(state) {
            self.current.specular = v;
        }
    }
}

/// `MultiStatesLight`: the light's intensity per state; its GameObject is
/// switched on above 0 and off at 0.
#[derive(Clone, Debug, PartialEq)]
pub struct StatesLight {
    pub intensities: Vec<f32>,
    /// `light.intensity` now.
    pub intensity: f32,
    /// `light.gameObject.activeSelf` now.
    pub active: bool,
    start: f32,
}

impl StatesLight {
    /// The light's stored intensity and its GameObject's active flag.
    pub fn new(intensities: Vec<f32>, intensity: f32, active: bool) -> Self {
        StatesLight {
            intensities,
            intensity,
            active,
            start: 1.0,
        }
    }

    fn init_lerp(&mut self) {
        self.start = self.intensity;
    }

    fn toggle(&mut self) {
        if self.intensity > 0.0 && !self.active {
            self.active = true;
        } else if self.intensity == 0.0 && self.active {
            self.active = false;
        }
    }

    fn update(&mut self, state: usize, t: f32) {
        if let Some(&v) = self.intensities.get(state) {
            self.intensity = lerp(self.start, v, t);
            self.toggle();
        }
    }

    fn set(&mut self, state: usize) {
        if let Some(&v) = self.intensities.get(state) {
            self.intensity = v;
            self.toggle();
        }
    }
}

/// `MultiStatesEmissive`.
#[derive(Clone, Debug, PartialEq)]
pub struct StatesEmissive {
    pub intensities: Vec<f32>,
    pub current: f32,
    start: f32,
}

impl StatesEmissive {
    pub fn new(intensities: Vec<f32>) -> Self {
        StatesEmissive {
            intensities,
            current: 0.0,
            start: 0.0,
        }
    }

    /// `_UwePowerLoss` on the registered renderers.
    pub fn power_loss(&self) -> f32 {
        clamp01(1.0 - self.current)
    }

    fn update(&mut self, state: usize, t: f32) {
        if let Some(&v) = self.intensities.get(state) {
            self.current = lerp(self.start, v, t);
        }
    }

    fn set(&mut self, state: usize) {
        if let Some(&v) = self.intensities.get(state) {
            self.current = v;
        }
    }
}

/// `LightingState`.
pub const OPERATIONAL: usize = 0;
pub const DANGER: usize = 1;
pub const DAMAGED: usize = 2;

/// The name of a `LightingState`.
pub fn state_name(state: usize) -> &'static str {
    match state {
        OPERATIONAL => "Operational",
        DANGER => "Danger",
        DAMAGED => "Damaged",
        _ => "?",
    }
}

/// `LightingController`.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingController {
    pub state: usize,
    pub fade_duration: f32,
    pub skies: Vec<StatesSky>,
    pub lights: Vec<StatesLight>,
    pub emissive: StatesEmissive,
    prev_state: Option<usize>,
    timer: Timer,
    lerp_finalized: bool,
    appliers_to_refresh: bool,
}

impl LightingController {
    pub fn new(
        state: usize,
        fade_duration: f32,
        skies: Vec<StatesSky>,
        lights: Vec<StatesLight>,
        emissive: StatesEmissive,
    ) -> Self {
        LightingController {
            state,
            fade_duration,
            skies,
            lights,
            emissive,
            prev_state: None,
            timer: Timer::default(),
            lerp_finalized: false,
            appliers_to_refresh: true,
        }
    }

    /// `SnapToState`: the state's values at once.
    pub fn snap_to_state(&mut self, state: usize) {
        self.timer.stop();
        self.state = state;
        for s in &mut self.skies {
            s.set(state);
        }
        for l in &mut self.lights {
            l.set(state);
        }
        self.emissive.set(state);
        self.appliers_to_refresh = false;
    }

    /// `LerpToState`: from the current values to the state's over
    /// `seconds` (`None`: the fade duration).
    pub fn lerp_to_state(&mut self, state: usize, seconds: Option<f32>) {
        self.timer.stop();
        for s in &mut self.skies {
            s.init_lerp();
        }
        for l in &mut self.lights {
            l.init_lerp();
        }
        self.emissive.start = self.emissive.current;
        self.state = state;
        self.timer.start(seconds.unwrap_or(self.fade_duration));
        self.lerp_finalized = false;
    }

    fn update_intensities(&mut self) {
        if self.timer.started && !self.timer.finished {
            let (state, t) = (self.state, self.timer.fraction());
            for s in &mut self.skies {
                s.update(state, t);
            }
            for l in &mut self.lights {
                l.update(state, t);
            }
            self.emissive.update(state, t);
        } else if !self.lerp_finalized || self.appliers_to_refresh {
            self.snap_to_state(self.state);
            self.lerp_finalized = true;
        }
        self.appliers_to_refresh = false;
    }

    /// `Update`, one frame of `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.timer.update(dt);
        if self.prev_state != Some(self.state) {
            self.lerp_to_state(self.state, None);
            self.prev_state = Some(self.state);
        }
        self.update_intensities();
    }

    /// Whether a fade is running.
    pub fn fading(&self) -> bool {
        self.timer.started && !self.timer.finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lifepod's controller as stored (state 0, fade 1 s).
    fn pod() -> LightingController {
        let stored = SkyIntensities {
            master: 10.0,
            diffuse: 2.0,
            specular: 1.5,
        };
        LightingController::new(
            OPERATIONAL,
            1.0,
            vec![StatesSky::new(
                vec![10.0, 0.8, 2.5],
                vec![2.0, 0.5, 0.8],
                vec![1.5, 3.0, 1.0],
                stored,
            )],
            vec![
                StatesLight::new(vec![0.0, 1.25, 0.0], 0.0, false),
                StatesLight::new(vec![0.0, 0.22, 0.0], 0.0, false),
            ],
            StatesEmissive::new(vec![0.0, 1.0, 1.0]),
        )
    }

    #[test]
    fn snap_sets_every_value_and_switches_lights() {
        let mut c = pod();
        c.snap_to_state(DANGER);
        let s = c.skies[0].current;
        assert_eq!((s.master, s.diffuse, s.specular), (0.8, 0.5, 3.0));
        assert_eq!((c.lights[0].intensity, c.lights[0].active), (1.25, true));
        assert_eq!((c.lights[1].intensity, c.lights[1].active), (0.22, true));
        assert_eq!(c.emissive.power_loss(), 0.0);
        c.snap_to_state(DAMAGED);
        assert_eq!(c.skies[0].current.master, 2.5);
        assert!(c.lights.iter().all(|l| !l.active && l.intensity == 0.0));
        c.snap_to_state(OPERATIONAL);
        assert_eq!(c.emissive.power_loss(), 1.0);
    }

    #[test]
    fn first_update_fades_into_the_stored_state() {
        // Already at the stored state's values: the fade changes nothing,
        // then snaps once it has run its second.
        let mut c = pod();
        c.update(0.016);
        assert!(c.fading());
        assert_eq!(c.skies[0].current.master, 10.0);
        for _ in 0..70 {
            c.update(0.016);
        }
        assert!(!c.fading());
        assert_eq!(c.skies[0].current.master, 10.0);
    }

    #[test]
    fn lerp_runs_linearly_then_snaps() {
        let mut c = pod();
        c.update(0.1);
        c.snap_to_state(DANGER);
        c.update(0.1); // the state changed since the last frame: a 1 s fade
        for _ in 0..12 {
            c.update(0.1);
        }
        assert!(!c.fading());
        // Inside `Update` nothing restarts it: 5 s, linear.
        c.lerp_to_state(DAMAGED, Some(5.0));
        c.prev_state = Some(DAMAGED);
        for _ in 0..25 {
            c.update(0.1);
        }
        // Half way: 0.8 → 2.5 and 1.25 → 0.
        let s = c.skies[0].current;
        assert!((s.master - 1.65).abs() < 1e-4, "{s:?}");
        assert!((c.lights[0].intensity - 0.625).abs() < 1e-4);
        assert!(c.lights[0].active);
        for _ in 0..26 {
            c.update(0.1);
        }
        assert!(!c.fading());
        assert_eq!(c.skies[0].current.master, 2.5);
        assert_eq!((c.lights[0].intensity, c.lights[0].active), (0.0, false));
    }

    #[test]
    fn a_lerp_called_between_frames_restarts_with_the_fade_duration() {
        // The game's `EscapePod` and `IntroLifepodDirector` call
        // `LerpToState(s, 5)`; the next `Update` sees the state differ from
        // `prevState` and starts `LerpToState(s)` again with `fadeDuration`
        // (1 s), from the values reached so far.
        let mut c = pod();
        c.update(0.1);
        c.snap_to_state(DANGER);
        for _ in 0..13 {
            c.update(0.1);
        }
        c.lerp_to_state(DAMAGED, Some(5.0));
        for _ in 0..6 {
            c.update(0.1);
        }
        // 0.5 s of a 1 s fade (the first frame restarted it, at 0 s): half
        // way, where a 5 s fade would be at a tenth.
        assert!((c.skies[0].current.master - 1.65).abs() < 1e-3);
        for _ in 0..6 {
            c.update(0.1);
        }
        assert!(!c.fading());
        assert_eq!(c.skies[0].current.master, 2.5);
    }

    #[test]
    fn a_state_change_without_a_call_fades_in_the_fade_duration() {
        let mut c = pod();
        c.update(0.1);
        c.update(1.0);
        c.state = DAMAGED; // as `SubRoot` does through `LerpToState`, or a field set
        c.update(0.5);
        assert!(c.fading());
        // The timer starts in this frame: 0 s of 1 s so far.
        assert_eq!(c.skies[0].current.master, 10.0);
        c.update(0.5);
        assert!((c.skies[0].current.master - 6.25).abs() < 1e-4);
        c.update(0.5);
        c.update(0.1);
        assert_eq!(c.skies[0].current.master, 2.5);
    }

    #[test]
    fn zero_or_negative_frames_do_nothing() {
        let mut c = pod();
        c.update(0.0);
        c.update(-1.0);
        assert!(!c.fading());
        assert_eq!(c, pod());
    }

    #[test]
    fn short_value_lists_leave_values_alone() {
        let mut c = pod();
        c.skies[0].specular.truncate(1);
        c.lights[1].intensities.clear();
        c.snap_to_state(DANGER);
        assert_eq!(c.skies[0].current.specular, 1.5);
        assert_eq!((c.lights[1].intensity, c.lights[1].active), (0.0, false));
        c.snap_to_state(7);
        assert_eq!(c.skies[0].current.master, 0.8);
    }
}
