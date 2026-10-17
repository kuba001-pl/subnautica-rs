//! What the UI's scripts change in its first frames (M9u5c, M9u6s;
//! `docs/formats/ui.md` § Start-up state). The stored `uGUI.prefab` keeps
//! many screens visible that the game's scripts hide as it starts:
//! full-screen black panels (the intro and respawn faders, the hardcore
//! game-over blocker, the out-of-oxygen overlay), the feedback panel, the
//! confirmation dialog, pop-ups, the vehicles' and cameras' screens, every
//! hand reticle icon at once. Drawing the stored state would show screens
//! the game never shows at start. Ported from the decompiled scripts (read
//! only; the code is ours), their start only (`Awake`, `OnEnable`,
//! `Start` and their first `Update`s, settled; their later calls are the
//! screens' own steps):
//! - `uGUI_Fader.Awake` → `SetState(initialState)` (`Sequence.ForceState`)
//!   and its first `Update` (`ApplyState`): its `canvasGroup`'s alpha
//!   becomes 1 or 0;
//! - `uGUI_SceneIntro.Start`: `fader.SetState(enabled)`, then
//!   `fader.FadeOut()`; settled (after the fader's `fadeInTime`), alpha 0;
//!   `mainText.SetState(false)` (its text disabled), `skipText` alpha 0,
//!   `skipProgress.value = 0` (the bar draws nothing at 0);
//! - `uGUI_Overlays.Awake`: each overlay's `SetAlpha(0)`: the graphic is
//!   disabled unless its curve is above 0 at 0;
//! - `uGUI_HardcoreGameOver.Awake`: `inputBlocker` and `message`
//!   inactive;
//! - the screens in [`RULES`]: a field's node made inactive, its
//!   `CanvasGroup` at alpha 0 or its graphic disabled, each with the
//!   method and condition it comes from;
//! - `uGUI_DepthCompass.LateUpdate` with no compass: `compass.SetVisible
//!   (false)` (its mesh empty: drawn as disabled) and its labels' texts
//!   disabled;
//! - `HandReticle`: `Awake` hides every icon (`SetActive(false, 0)`), the
//!   second `LateUpdate` shows the `Default` icon (0.1 s fade; settled:
//!   its object active, its graphics' alpha 1, the others inactive).
//!
//! A rule whose script's node is inactive at start is not applied (Unity
//! runs `Awake` when the object first becomes active) and is noted.

use std::collections::HashSet;

use sn_ui::state::{MaterialValues, UiState};
use sn_unity::{Fields, PPtr, Value};

use crate::{Assets, Layouts, ObjectRef, Result, UiTree};

/// The first frames' changes over the stored state.
#[derive(Clone, Debug, Default)]
pub struct UiStartState {
    pub state: UiState,
    /// Each change, for the log.
    pub notes: Vec<String>,
}

/// What a rule does to its field's node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    /// `SetActive(false)`.
    Inactive,
    /// `CanvasGroup.alpha = 0`.
    Transparent,
    /// `Graphic.enabled = false`.
    Disabled,
}

/// A script's start rule: `fields` (`""`: the script's own GameObject)
/// get `effect`; `why` is the method and the condition, for the log.
struct Rule {
    script: &'static str,
    fields: &'static [&'static str],
    effect: Effect,
    why: &'static str,
}

/// The screens' start rules (M9u6s), each from its decompiled script.
/// The conditions are a new game's start in our client: in the main
/// level, no vehicle, no camera, no PDA open, no messages.
const RULES: &[Rule] = &[
    Rule {
        script: "uGUI_FeedbackCollector",
        fields: &["inputBlocker", "root", "repliesPanel"],
        effect: Effect::Inactive,
        why: "Awake",
    },
    Rule {
        script: "uGUI_PopupMessage",
        fields: &["root"],
        effect: Effect::Inactive,
        why: "Awake",
    },
    Rule {
        script: "uGUI_SceneConfirmation",
        fields: &[""],
        effect: Effect::Inactive,
        why: "Start",
    },
    Rule {
        script: "uGUI_BuildWatermark",
        fields: &[""],
        effect: Effect::Inactive,
        why: "Start: once in the main level and no wait screen",
    },
    Rule {
        script: "uGUI_Tooltip",
        fields: &["canvasGroup"],
        effect: Effect::Transparent,
        why: "Awake",
    },
    Rule {
        script: "uGUI_PopupNotification",
        fields: &["canvasGroup"],
        effect: Effect::Transparent,
        why: "Awake",
    },
    Rule {
        script: "uGUI_MessageQueue",
        fields: &["canvasGroup"],
        effect: Effect::Transparent,
        why: "OnEnable: time = timeFadeOut; Update: no messages",
    },
    Rule {
        script: "uGUI_CameraCyclops",
        fields: &["content"],
        effect: Effect::Inactive,
        why: "Awake",
    },
    Rule {
        script: "uGUI_CameraDrone",
        fields: &["content"],
        effect: Effect::Inactive,
        why: "Awake; LateUpdate: no camera",
    },
    Rule {
        script: "uGUI_SeamothHUD",
        fields: &["root"],
        effect: Effect::Inactive,
        why: "Update: not in a Seamoth (vehicles not ported)",
    },
    Rule {
        script: "uGUI_ExosuitHUD",
        fields: &["root"],
        effect: Effect::Inactive,
        why: "Update: not in an exosuit (vehicles not ported)",
    },
    Rule {
        script: "uGUI_RadiationWarning",
        fields: &["warning"],
        effect: Effect::Inactive,
        why: "Update: not radiated (radiation not ported; a new game's start)",
    },
    Rule {
        script: "uGUI_PowerIndicator",
        fields: &["text"],
        effect: Effect::Disabled,
        why: "LateUpdate: IsPowerEnabled false (not in a base or sub)",
    },
    Rule {
        script: "uGUI_DepthCompass",
        fields: &[
            "submersibleDepth",
            "submersibleDepthSuffix",
            "submersibleCrushDepth",
            "submersibleBackground",
        ],
        effect: Effect::Disabled,
        why: "LateUpdate: DepthMode.Player",
    },
    Rule {
        script: "HandReticle",
        fields: &[
            "compTextHand",
            "compTextHandSubscript",
            "compTextUse",
            "compTextUseSubscript",
        ],
        effect: Effect::Inactive,
        why: "Start: empty texts; LateUpdate: UpdateText hides an empty one",
    },
];

/// `HandReticle.IconType.Default`.
const ICON_DEFAULT: i64 = 1;

fn pptr(f: &Fields, name: &str) -> Option<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
}

/// An `AnimationCurve`'s value at time 0, clamped before its first key
/// (Unity's default pre-infinity); `None` without keys or a later first key
/// than 0 that is not the only one (not met; logged by the caller).
fn curve_at_zero(curve: &Fields) -> Option<f32> {
    let keys = curve.get("m_Curve").and_then(Value::as_array)?;
    let first = keys.first()?.as_struct()?;
    let time = first.get("time").and_then(Value::as_f64)?;
    let value = first.get("value").and_then(Value::as_f64)?;
    (time >= 0.0).then_some(value as f32)
}

/// A `Color` struct's channels.
fn color(f: &Fields, name: &str) -> Option<[f32; 4]> {
    let c = f.get(name).and_then(Value::as_struct)?;
    let ch = |k: &str| c.get(k).and_then(Value::as_f64).map(|v| v as f32);
    Some([ch("r")?, ch("g")?, ch("b")?, ch("a")?])
}

impl Assets<'_> {
    /// The node of the component or GameObject `p` (from `from`'s file)
    /// points at.
    pub(crate) fn node_of(
        &self,
        tree: &UiTree,
        from: &ObjectRef,
        p: PPtr,
    ) -> Result<Option<usize>> {
        let Some(o) = self.resolve(&from.file, p)? else {
            return Ok(None);
        };
        let key = o.key();
        for (i, n) in tree.nodes.iter().enumerate() {
            if n.game_object.as_ref().is_some_and(|g| g.key() == key)
                || n.components.iter().any(|c| c.object.key() == key)
            {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// The fields of the object `p` (from `from`'s file) points at.
    pub(crate) fn fields_at(
        &self,
        layouts: &Layouts,
        from: &ObjectRef,
        p: PPtr,
    ) -> Result<Option<Fields>> {
        let Some(o) = self.resolve(&from.file, p)? else {
            return Ok(None);
        };
        Ok(Some(self.object_fields(layouts, &o)?.1))
    }

    /// The first frames' changes of `tree`'s screens.
    pub fn ui_start_state(&self, layouts: &Layouts, tree: &UiTree) -> Result<UiStartState> {
        let mut out = UiStartState::default();
        self.start_faders(layouts, tree, &mut out)?;
        self.start_intro(layouts, tree, &mut out)?;
        self.start_overlays(layouts, tree, &mut out)?;
        for (node, c) in tree.scripts("uGUI_HardcoreGameOver") {
            let (_, f) = self.script_fields(layouts, c)?;
            for name in ["inputBlocker", "message"] {
                let Some(p) = pptr(&f, name) else {
                    continue;
                };
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    out.state.active.insert(g, false);
                    out.notes.push(format!(
                        "{}: {name} {} inactive",
                        tree.nodes[node].path, tree.nodes[g].path
                    ));
                }
            }
        }
        for rule in RULES {
            for (node, c) in tree.scripts(rule.script) {
                if !self.awake(tree, node, rule.script, &mut out) {
                    continue;
                }
                let (_, f) = self.script_fields(layouts, c)?;
                for name in rule.fields {
                    let target = if name.is_empty() {
                        Some(node)
                    } else {
                        match pptr(&f, name) {
                            Some(p) => self.node_of(tree, &c.object, p)?,
                            None => None,
                        }
                    };
                    let Some(g) = target else {
                        out.notes.push(format!(
                            "{}: {}.{name} not set or not in the tree",
                            tree.nodes[node].path, rule.script
                        ));
                        continue;
                    };
                    let what = match rule.effect {
                        Effect::Inactive => {
                            out.state.active.insert(g, false);
                            "inactive"
                        }
                        Effect::Transparent => {
                            out.state.group_alpha.insert(g, 0.0);
                            "CanvasGroup alpha 0"
                        }
                        Effect::Disabled => {
                            out.state.enabled.insert(g, false);
                            "disabled"
                        }
                    };
                    let field = if name.is_empty() { "itself" } else { name };
                    out.notes.push(format!(
                        "{}: {} {field} {} {what} ({})",
                        tree.nodes[node].path, rule.script, tree.nodes[g].path, rule.why
                    ));
                }
            }
        }
        self.start_compass(layouts, tree, &mut out)?;
        self.start_hand_reticle(layouts, tree, &mut out)?;
        Ok(out)
    }

    /// Whether `node`'s script has run its start (its node active in the
    /// hierarchy); noted if not.
    fn awake(&self, tree: &UiTree, node: usize, script: &str, out: &mut UiStartState) -> bool {
        let active = tree.nodes[node].active;
        if !active {
            out.notes.push(format!(
                "{}: {script} not started (inactive at start)",
                tree.nodes[node].path
            ));
        }
        active
    }

    fn start_faders(&self, layouts: &Layouts, tree: &UiTree, out: &mut UiStartState) -> Result<()> {
        // The intro's fader settles at 0 (`Start`: fade out).
        let mut intro_faders = HashSet::new();
        for (_, c) in tree.scripts("uGUI_SceneIntro") {
            let (_, f) = self.script_fields(layouts, c)?;
            if let Some(p) = pptr(&f, "fader") {
                if let Some(o) = self.resolve(&c.object.file, p)? {
                    intro_faders.insert(o.key());
                }
            }
        }
        for (node, c) in tree.scripts("uGUI_Fader") {
            let (_, f) = self.script_fields(layouts, c)?;
            let initial = f
                .get("initialState")
                .and_then(Value::as_bool)
                .ok_or("uGUI_Fader without initialState")?;
            let Some(group) = pptr(&f, "canvasGroup") else {
                continue;
            };
            let Some(g) = self.node_of(tree, &c.object, group)? else {
                continue;
            };
            let intro = intro_faders.contains(&c.object.key());
            let alpha = if intro || !initial { 0.0 } else { 1.0 };
            out.state.group_alpha.insert(g, alpha);
            out.notes.push(format!(
                "{}: uGUI_Fader{} → CanvasGroup {} alpha {alpha}",
                tree.nodes[node].path,
                if intro {
                    " (the intro's, faded out)"
                } else {
                    ""
                },
                tree.nodes[g].path
            ));
        }
        Ok(())
    }

    /// `uGUI_SceneIntro.Start`'s texts and skip bar (its fader is in
    /// [`Assets::start_faders`]).
    fn start_intro(&self, layouts: &Layouts, tree: &UiTree, out: &mut UiStartState) -> Result<()> {
        for (node, c) in tree.scripts("uGUI_SceneIntro") {
            if !self.awake(tree, node, "uGUI_SceneIntro", out) {
                continue;
            }
            let path = &tree.nodes[node].path;
            let (_, f) = self.script_fields(layouts, c)?;
            // `mainText.SetState(false)`: the text on its GameObject
            // (`uGUI_Text.Awake`: `GetComponent`) disabled.
            if let Some(p) = pptr(&f, "mainText") {
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    out.state.enabled.insert(g, false);
                    out.notes.push(format!(
                        "{path}: uGUI_SceneIntro mainText {} disabled (Start: SetState(false))",
                        tree.nodes[g].path
                    ));
                }
            }
            // `skipText.SetAlpha(0)`: TMP's `alpha`, its font colour's.
            if let Some(p) = pptr(&f, "skipText") {
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    let stored = self
                        .fields_at(layouts, &c.object, p)?
                        .and_then(|tf| color(&tf, "m_fontColor"))
                        .ok_or("uGUI_SceneIntro.skipText without m_fontColor")?;
                    out.state
                        .color
                        .insert(g, [stored[0], stored[1], stored[2], 0.0]);
                    out.notes.push(format!(
                        "{path}: uGUI_SceneIntro skipText {} alpha 0 (Start)",
                        tree.nodes[g].path
                    ));
                }
            }
            // `skipProgress.value = 0`: `_Value` 0 (`value × maxValue`,
            // clamped).
            if let Some(p) = pptr(&f, "skipProgress") {
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    let mut m = MaterialValues::default();
                    m.floats.insert("_Value".into(), 0.0);
                    out.state.material.insert(g, m);
                    out.notes.push(format!(
                        "{path}: uGUI_SceneIntro skipProgress {} value 0 (Start; nothing drawn at 0)",
                        tree.nodes[g].path
                    ));
                }
            }
        }
        Ok(())
    }

    fn start_overlays(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        out: &mut UiStartState,
    ) -> Result<()> {
        for (node, c) in tree.scripts("uGUI_Overlays") {
            let (_, f) = self.script_fields(layouts, c)?;
            let overlays = f.get("overlays").and_then(Value::as_array).unwrap_or(&[]);
            for o in overlays {
                let Some(o) = o.as_struct() else {
                    continue;
                };
                let Some(g) = pptr(o, "graphic") else {
                    continue;
                };
                let Some(g) = self.node_of(tree, &c.object, g)? else {
                    continue;
                };
                let at_zero = o
                    .get("curve")
                    .and_then(Value::as_struct)
                    .and_then(curve_at_zero);
                match at_zero {
                    Some(v) if v > 0.0 => out.notes.push(format!(
                        "{}: overlay {} stays (curve at 0 = {v}; its alpha not ported)",
                        tree.nodes[node].path, tree.nodes[g].path
                    )),
                    _ => {
                        out.state.enabled.insert(g, false);
                        out.notes.push(format!(
                            "{}: overlay {} disabled (SetAlpha 0)",
                            tree.nodes[node].path, tree.nodes[g].path
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `uGUI_DepthCompass.LateUpdate` → `compass.SetVisible(false)`: no
    /// compass at a new game's start (the equipment's compass rule is
    /// M9u6c).
    fn start_compass(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        out: &mut UiStartState,
    ) -> Result<()> {
        for (node, c) in tree.scripts("uGUI_DepthCompass") {
            if !self.awake(tree, node, "uGUI_DepthCompass", out) {
                continue;
            }
            let (_, f) = self.script_fields(layouts, c)?;
            let Some(p) = pptr(&f, "compass") else {
                continue;
            };
            let Some(g) = self.node_of(tree, &c.object, p)? else {
                continue;
            };
            out.state.enabled.insert(g, false);
            let mut labels = 0;
            if let Some(cf) = self.fields_at(layouts, &c.object, p)? {
                for l in cf.get("labels").and_then(Value::as_array).unwrap_or(&[]) {
                    let Some(t) = l.as_struct().and_then(|l| pptr(l, "text")) else {
                        continue;
                    };
                    if let Some(t) = self.node_of(tree, &c.object, t)? {
                        out.state.enabled.insert(t, false);
                        labels += 1;
                    }
                }
            }
            out.notes.push(format!(
                "{}: uGUI_DepthCompass compass {} not visible, {labels} labels disabled (LateUpdate: no compass equipped)",
                tree.nodes[node].path, tree.nodes[g].path
            ));
        }
        Ok(())
    }

    /// `HandReticle`'s icons, settled: only `Default` shown.
    fn start_hand_reticle(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        out: &mut UiStartState,
    ) -> Result<()> {
        for (node, c) in tree.scripts("HandReticle") {
            if !self.awake(tree, node, "HandReticle", out) {
                continue;
            }
            let (_, f) = self.script_fields(layouts, c)?;
            let (mut shown, mut hidden) = (0, 0);
            for icon in f.get("icons").and_then(Value::as_array).unwrap_or(&[]) {
                let Some(p) = icon.as_pptr().filter(|p| !p.is_null()) else {
                    continue;
                };
                let Some(icon_ref) = self.resolve(&c.object.file, p)? else {
                    continue;
                };
                let (_, fi) = self.object_fields(layouts, &icon_ref)?;
                let kind = fi.get("type").and_then(Value::as_i64);
                let show = kind == Some(ICON_DEFAULT);
                // `go` is the icon's own game object (set in `Awake`, not
                // stored).
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    out.state.active.insert(g, show);
                }
                for gr in fi.get("graphic").and_then(Value::as_array).unwrap_or(&[]) {
                    let Some(gp) = gr.as_pptr().filter(|p| !p.is_null()) else {
                        continue;
                    };
                    if let Some(g) = self.node_of(tree, &icon_ref, gp)? {
                        out.state
                            .renderer_alpha
                            .insert(g, if show { 1.0 } else { 0.0 });
                    }
                }
                if show {
                    shown += 1;
                } else {
                    hidden += 1;
                }
            }
            out.notes.push(format!(
                "{}: HandReticle {shown} icon shown (Default, after its 0.1 s fade), {hidden} hidden (Awake: SetActive(false))",
                tree.nodes[node].path
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f64, value: f64) -> Value {
        Value::Struct(Fields(vec![
            ("time".into(), Value::Float(time)),
            ("value".into(), Value::Float(value)),
        ]))
    }

    #[test]
    fn a_curve_at_zero_is_its_first_key_from_zero_on() {
        let curve = |keys: Vec<Value>| Fields(vec![("m_Curve".into(), Value::Array(keys))]);
        assert_eq!(
            curve_at_zero(&curve(vec![key(0.0, 0.0), key(1.0, 1.0)])),
            Some(0.0)
        );
        assert_eq!(curve_at_zero(&curve(vec![key(0.5, 0.3)])), Some(0.3));
        assert_eq!(
            curve_at_zero(&curve(vec![key(-1.0, 0.3), key(1.0, 1.0)])),
            None
        );
        assert_eq!(curve_at_zero(&curve(Vec::new())), None);
    }

    #[test]
    fn a_colour_reads_its_four_channels() {
        let f = Fields(vec![(
            "c".into(),
            Value::Struct(Fields(vec![
                ("r".into(), Value::Float(0.25)),
                ("g".into(), Value::Float(0.5)),
                ("b".into(), Value::Float(0.75)),
                ("a".into(), Value::Float(1.0)),
            ])),
        )]);
        assert_eq!(color(&f, "c"), Some([0.25, 0.5, 0.75, 1.0]));
        assert_eq!(color(&f, "d"), None);
    }

    #[test]
    fn every_rule_names_a_script_and_fields() {
        for r in RULES {
            assert!(!r.script.is_empty() && !r.fields.is_empty() && !r.why.is_empty());
        }
    }
}
