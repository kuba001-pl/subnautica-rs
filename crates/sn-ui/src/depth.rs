//! The depth readout's rules (M9u6c1; `docs/formats/ui.md` § The depth and
//! compass): `uGUI_DepthCompass`, ported from the decompiled script (read
//! only; the code is ours). Each frame [`DepthCompass::update`] runs its
//! `LateUpdate` (`Initialize`, `UpdateDepth`, `UpdateCompass`) on the
//! HUD's [`HudInputs`] and gives what the script has set so far.
//!
//! Only the player's mode is reachable in our client (no vehicles, camera
//! drones, rocket or intro), so the mode is `Player` while the HUD is
//! active and the PDA is closed, and `None` otherwise. `OnDepthClassChanged`
//! forces the class to `Safe` in the game's code (IL), so the danger
//! sprites and colour are never shown and are not read. `UpdateCompass`
//! shows or hides the compass (`SetVisible`: the graphic and its labels'
//! texts) and, while shown, sets its `direction` from the camera's yaw
//! (M9u6c2): `uGUI_Compass`'s material values and labels
//! ([`crate::compass`]).

use crate::compass::{Compass, wrap};
use crate::hud::HudInputs;
use crate::state::{MaterialValues, SpriteId, UiState};

/// `BlendMode.DstColor`, `BlendMode.OneMinusSrcAlpha`.
const DST_COLOR: f32 = 2.0;
const ONE_MINUS_SRC_ALPHA: f32 = 10.0;

/// The script's nodes, sprites and colour, and the language's strings.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthConfig {
    /// `shadow`, `halfMoon` (images), `depthText`, `suffixText`.
    pub shadow: usize,
    pub half_moon: usize,
    pub depth_text: usize,
    pub suffix_text: usize,
    /// `submersibleBackground`, `submersibleDepth`, `submersibleDepthSuffix`,
    /// `submersibleCrushDepth`.
    pub submersible: [usize; 4],
    /// `compass` (the `uGUI_Compass` graphic) and its labels' texts, each
    /// with its language text (`CompassDirection{name}`).
    pub compass: usize,
    pub labels: Vec<(usize, String)>,
    /// The `uGUI_Compass`'s fields.
    pub ring: Compass,
    pub shadow_normal: SpriteId,
    pub half_moon_normal: SpriteId,
    pub half_moon_compass_normal: SpriteId,
    /// `textColorNormal` (a `Color32`).
    pub text_color_normal: [u8; 4],
    /// `Language.Get("MeterSuffix")`.
    pub meter_suffix: String,
}

/// `uGUI_DepthCompass.DepthMode`, as far as our client reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthMode {
    Undefined,
    None,
    Player,
}

/// The script's state.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthCompass {
    pub config: DepthConfig,
    pub initialized: bool,
    pub mode: DepthMode,
    /// `_playerDepthValue` (`int.MinValue` before the first).
    pub player_depth: i32,
    /// `_cachedCompassEnabled`: −1 before the first `UpdateCompass`.
    pub compass_cached: i32,
    /// `uGUI_Compass._visible` (true until `SetVisible(false)`).
    pub compass_visible: bool,
    /// `uGUI_Compass._direction` (0 until first set).
    pub direction: f32,
    /// Every change made so far.
    pub state: UiState,
}

/// `Mathf.FloorToInt(Ocean.GetDepthOf(…))`: `GetDepthOf` is
/// `max(0, oceanLevel − y)`; the client passes it.
pub fn floor_depth(depth: f32) -> i32 {
    depth.floor() as i32
}

impl DepthCompass {
    pub fn new(config: DepthConfig) -> DepthCompass {
        DepthCompass {
            config,
            initialized: false,
            mode: DepthMode::Undefined,
            player_depth: i32::MIN,
            compass_cached: -1,
            compass_visible: true,
            direction: 0.0,
            state: UiState::default(),
        }
    }

    /// The mode `GetDepthInfo` gives this frame.
    pub fn mode_of(&self, i: &HudInputs) -> DepthMode {
        if !self.initialized || !i.active || i.pda_open {
            DepthMode::None
        } else {
            DepthMode::Player
        }
    }

    /// One `LateUpdate`. Returns the whole picture so far.
    pub fn update(&mut self, i: &HudInputs) -> &UiState {
        self.initialize();
        self.update_depth(i);
        self.update_compass(i);
        &self.state
    }

    /// `Initialize`: `OnLanguageChanged`, then `OnDepthClassChanged` with
    /// the class forced to `Safe`.
    fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        let c = &self.config;
        let s = &mut self.state;
        s.text.insert(c.submersible[2], c.meter_suffix.clone());
        for (node, text) in &c.labels {
            s.text.insert(*node, text.clone());
        }
        // `_cachedDepthClass` is already `Safe`: the half moon is not
        // re-picked. The shadow's own material gets the multiplicative
        // blend, premultiplied.
        s.sprite.insert(c.shadow, Some(c.shadow_normal));
        let mut m = MaterialValues::default();
        m.floats.insert("_SrcFactor".into(), DST_COLOR);
        m.floats.insert("_DstFactor".into(), ONE_MINUS_SRC_ALPHA);
        m.keywords.insert("ALPHA_PREMULTIPLY".into(), true);
        s.material.entry(c.shadow).or_default().apply(&m);
        let color = c.text_color_normal.map(|v| v as f32 / 255.0);
        s.color.insert(c.depth_text, color);
        s.color.insert(c.suffix_text, color);
    }

    fn update_half_moon(&mut self) {
        let c = &self.config;
        let sprite = if self.compass_cached == 1 {
            c.half_moon_compass_normal
        } else {
            c.half_moon_normal
        };
        self.state.sprite.insert(c.half_moon, Some(sprite));
    }

    /// `UpdateDepth`.
    fn update_depth(&mut self, i: &HudInputs) {
        let mode = self.mode_of(i);
        if self.mode != mode {
            self.mode = mode;
            let player = mode == DepthMode::Player;
            let c = &self.config;
            for n in [c.shadow, c.half_moon, c.depth_text, c.suffix_text] {
                self.state.enabled.insert(n, player);
            }
            if player {
                self.update_half_moon();
            }
            for n in self.config.submersible {
                self.state.enabled.insert(n, false);
            }
        }
        if self.mode == DepthMode::Player {
            let depth = floor_depth(i.depth);
            if self.player_depth != depth {
                self.player_depth = depth;
                let c = &self.config;
                self.state.text.insert(c.depth_text, depth.to_string());
                self.state
                    .text
                    .insert(c.suffix_text, c.meter_suffix.clone());
            }
        }
    }

    /// `IsCompassEnabled`.
    pub fn compass_enabled(&self, i: &HudInputs) -> bool {
        self.initialized && i.active && !i.pda_open && i.compass
    }

    /// `UpdateCompass`: the half moon on a change, `compass.SetVisible`,
    /// and while enabled `compass.direction = eulerAngles.y / 360`.
    fn update_compass(&mut self, i: &HudInputs) {
        let on = self.compass_enabled(i);
        let n = i32::from(on);
        if self.compass_cached != n {
            self.compass_cached = n;
            if self.mode == DepthMode::Player {
                self.update_half_moon();
            }
        }
        // `SetVisible` acts on a change of `_visible`; the state always
        // says what it is, since a start state (M9u6s) may already have
        // applied a first `SetVisible(false)`.
        self.compass_visible = on;
        self.state.enabled.insert(self.config.compass, on);
        for (node, _) in &self.config.labels {
            self.state.enabled.insert(*node, on);
        }
        if on {
            self.set_direction(i.yaw / 360.0);
        }
    }

    /// The `direction` setter: wrapped to 0…1; on a change `UpdateAngle`
    /// (the material instance's values) and `UpdateLabels`.
    fn set_direction(&mut self, value: f32) {
        let value = wrap(value);
        if self.direction == value {
            return;
        }
        self.direction = value;
        let ring = &self.config.ring;
        let s = &mut self.state;
        s.material
            .entry(self.config.compass)
            .or_default()
            .apply(&ring.angle_values(value));
        for l in &ring.labels {
            let (pos, alpha) = ring.label(l.angle, value);
            s.transform.entry(l.node).or_default().anchored_position = Some(pos);
            s.color.insert(l.node, [1.0, 1.0, 1.0, alpha]);
        }
    }
}

#[cfg(test)]
mod tests;
