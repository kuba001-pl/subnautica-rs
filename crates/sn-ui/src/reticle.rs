//! The hand reticle's rules (M9u6d2; `docs/formats/ui.md` § The hand
//! reticle): `HandReticle` and `uGUI_HandReticleIcon`, ported from the
//! decompiled scripts (read only; the code is ours). Each frame
//! [`HandReticle::update`] runs `LateUpdate` on what the frame's
//! `OnHandHover` calls set ([`ReticleInputs`]) and gives what the script
//! has set so far.
//!
//! `LateUpdate` writes the four texts (each shown only if not empty),
//! sets the icon (`None` while the crosshair is hidden or the cursor is
//! free), lerps the icon canvas's scale, sets the progress, fades the
//! icons (`Sequence`, 0.1 s; 0 to `None`) and then resets what the
//! hovers set: icon `Default` at scale 1, empty texts, progress 0.
//! [`ReticleInputs`] is that reset state plus the frame's calls.
//!
//! [`get_text`] is `GetText`'s format: the text, then the button's
//! display text (`GameInput.FormatButton`, from `sn-assets`) in
//! `HandReticleAddButtonFormat`.

use crate::state::UiState;
use crate::tween::Sequence;

/// `HandReticle.IconType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconType {
    None = 0,
    Default = 1,
    Hand = 2,
    HandDeny = 3,
    Scan = 4,
    Progress = 5,
    Info = 6,
    Drill = 7,
    PackUp = 8,
    Rename = 9,
    Interact = 10,
}

impl IconType {
    /// The stored `type` field's value.
    pub fn from_i32(v: i32) -> Option<IconType> {
        use IconType::*;
        [
            None, Default, Hand, HandDeny, Scan, Progress, Info, Drill, PackUp, Rename, Interact,
        ]
        .get(usize::try_from(v).ok()?)
        .copied()
    }
}

/// `uGUI_HandReticleIcon`: its type, its game object and its graphics.
#[derive(Clone, Debug, PartialEq)]
pub struct ReticleIcon {
    pub kind: IconType,
    pub node: usize,
    pub graphics: Vec<usize>,
}

/// The script's nodes and fields, and the language's strings.
#[derive(Clone, Debug, PartialEq)]
pub struct ReticleConfig {
    /// `iconCanvas` and its stored `localScale`.
    pub icon_canvas: usize,
    pub icon_canvas_scale: [f32; 3],
    pub icons: Vec<ReticleIcon>,
    pub icon_scale_speed: f32,
    /// `compTextHand`, `compTextHandSubscript`, `compTextUse`,
    /// `compTextUseSubscript` (`TextType` order).
    pub texts: [usize; 4],
    pub progress_image: usize,
    pub progress_text: usize,
    /// `HandReticleAddButtonFormat`, `HandReticleProgressPercentFormat`.
    pub add_button_format: String,
    pub progress_format: String,
}

/// What this frame's hovers set before `LateUpdate` (the defaults are
/// `LateUpdate`'s reset: icon `Default`, scale 1, empty texts, 0).
#[derive(Clone, Debug, PartialEq)]
pub struct ReticleInputs {
    /// `Time.deltaTime`.
    pub delta_time: f32,
    /// `SetIcon(type, size)`.
    pub icon: IconType,
    pub icon_scale: f32,
    /// The texts as `SetText` / `SetTextRaw` left them (`TextType` order).
    pub texts: [String; 4],
    /// `SetProgress`.
    pub progress: f32,
    /// `hideCount > 0 || !AvatarInputHandler.main.IsEnabled()` (the
    /// cursor is free: the PDA is open, or the player let it go).
    pub hidden: bool,
}

impl Default for ReticleInputs {
    fn default() -> ReticleInputs {
        ReticleInputs {
            delta_time: 0.0,
            icon: IconType::Default,
            icon_scale: 1.0,
            texts: Default::default(),
            progress: 0.0,
            hidden: false,
        }
    }
}

/// `HandReticle.TextType.Hand`.
pub const HAND: usize = 0;
/// `HandReticle.TextType.HandSubscript`.
pub const HAND_SUBSCRIPT: usize = 1;

/// The script's state.
#[derive(Clone, Debug, PartialEq)]
pub struct HandReticle {
    pub config: ReticleConfig,
    /// `iconType`.
    pub icon_type: IconType,
    /// Each icon's `sequence`, in `config.icons`' order.
    pub sequences: Vec<Sequence>,
    /// `iconCanvas.localScale.x` (= `.y`).
    pub scale: f32,
    /// `cachedProgress` (`float.MinValue` before the first).
    pub cached_progress: f32,
    /// Every change made so far.
    pub state: UiState,
}

/// .NET's composite format with `{n}` items only (no alignment or format
/// strings; `{{` and `}}` escape): `None` for anything else.
pub fn composite(format: &str, args: &[&str]) -> Option<String> {
    let mut out = String::new();
    let mut it = format.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut n = String::new();
                loop {
                    match it.next()? {
                        '}' => break,
                        d if d.is_ascii_digit() => n.push(d),
                        _ => return None,
                    }
                }
                out.push_str(args.get(n.parse::<usize>().ok()?)?);
            }
            '}' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// `GetText(text, translate, button)` with `text` already translated:
/// the text alone without a button (empty stays empty), else
/// `HandReticleAddButtonFormat` of it and the button's display text.
pub fn get_text(config: &ReticleConfig, text: &str, button: Option<&str>) -> String {
    match button {
        None => text.to_string(),
        Some(b) => composite(&config.add_button_format, &[text, b])
            .unwrap_or_else(|| config.add_button_format.clone()),
    }
}

/// `{0:P0}` of `value` (`HandReticleProgressPercentFormat` in English):
/// × 100, rounded half away from zero, then `%`. *Hypothesis*: Mono's
/// `en` culture puts no space before the sign. Other formats are given
/// as they are.
pub fn percent(format: &str, value: f32) -> String {
    let n = (f64::from(value) * 100.0).round() as i64;
    format.replace("{0:P0}", &format!("{n}%"))
}

impl HandReticle {
    /// After `Awake` and `Start`, settled as the start state has it
    /// (M9u6s: the `Default` icon faded in, the others hidden, the texts
    /// empty and inactive).
    pub fn new(config: ReticleConfig) -> HandReticle {
        let sequences = config
            .icons
            .iter()
            .map(|i| {
                let mut s = Sequence::default();
                // `SetActive(false)` in `Awake`, `SetActive(true, 0.1)` for
                // `Default` on the second frame, both run to their end.
                s.set_from(0.0, false, false);
                if i.kind == IconType::Default {
                    s.set_from(0.1, true, true);
                }
                s.update(0.0);
                s
            })
            .collect();
        HandReticle {
            scale: config.icon_canvas_scale[0],
            config,
            icon_type: IconType::Default,
            sequences,
            cached_progress: f32::MIN,
            state: UiState::default(),
        }
    }

    /// One `LateUpdate` after the frame's hovers.
    pub fn update(&mut self, i: &ReticleInputs) -> &UiState {
        for (k, node) in self.config.texts.iter().enumerate() {
            // `UpdateText`: `ShouldHideAll` is false (no screenshot mode).
            let text = &i.texts[k];
            self.state.text.insert(*node, text.clone());
            self.state.active.insert(*node, !text.is_empty());
        }
        let desired = if i.hidden { IconType::None } else { i.icon };
        self.set_icon_internal(desired);
        self.update_scale(i.icon_scale, i.delta_time);
        self.update_progress(i.progress);
        for (k, icon) in self.config.icons.iter().enumerate() {
            let s = &mut self.sequences[k];
            // `UpdateIcon`.
            if s.active() {
                s.update(i.delta_time);
                let t = s.t();
                self.state.active.insert(icon.node, t > 0.0);
                for g in &icon.graphics {
                    self.state.renderer_alpha.insert(*g, t);
                }
            }
        }
        &self.state
    }

    /// `SetIconInternal`: on a change, the old icon fades out and the new
    /// one in over 0.1 s (at once when the new one is `None`).
    fn set_icon_internal(&mut self, new: IconType) {
        if self.icon_type == new {
            return;
        }
        let duration = if new == IconType::None { 0.0 } else { 0.1 };
        let set = |me: &mut HandReticle, kind: IconType, active: bool| {
            if kind == IconType::None {
                return;
            }
            // `_icons` keeps the first icon of each type.
            if let Some(k) = me.config.icons.iter().position(|i| i.kind == kind) {
                let s = &mut me.sequences[k];
                if duration == 0.0 {
                    s.set_from(duration, active, active);
                } else {
                    s.set(duration, active);
                }
            }
        };
        set(self, self.icon_type, false);
        self.icon_type = new;
        set(self, new, true);
    }

    /// `UpdateScale`: `Mathf.Lerp(x, clamp(desired, 0, 2), dt × speed)`.
    fn update_scale(&mut self, desired: f32, dt: f32) {
        let t = (dt * self.config.icon_scale_speed).clamp(0.0, 1.0);
        let to = desired.clamp(0.0, 2.0);
        self.scale += (to - self.scale) * t;
        let z = self.config.icon_canvas_scale[2];
        self.state
            .transform
            .entry(self.config.icon_canvas)
            .or_default()
            .scale = Some([self.scale, self.scale, z]);
    }

    /// `UpdateProgress`: on a change, the image's fill and the text.
    fn update_progress(&mut self, progress: f32) {
        let p = progress.clamp(0.0, 1.0);
        if self.cached_progress != p {
            self.cached_progress = p;
            self.state.fill_amount.insert(self.config.progress_image, p);
            self.state.text.insert(
                self.config.progress_text,
                percent(&self.config.progress_format, p),
            );
        }
    }
}

#[cfg(test)]
mod tests;
