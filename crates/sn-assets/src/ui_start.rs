//! What the HUD's scripts change in its first frame (M9u5c,
//! `docs/formats/ui.md` § Start-up state). The stored `uGUI.prefab` keeps
//! several full-screen black panels visible (the intro and respawn
//! faders, the hardcore game-over blocker, the out-of-oxygen overlay) that
//! the game's scripts hide as it starts; drawing the stored state would
//! show a black screen the game never shows. Ported from the decompiled
//! scripts (read only; the code is ours), their start only (their
//! animations and later calls are M9u6):
//! - `uGUI_Fader.Awake` → `SetState(initialState)` (`Sequence.ForceState`)
//!   and its first `Update` (`ApplyState`): its `canvasGroup`'s alpha
//!   becomes 1 or 0;
//! - `uGUI_SceneIntro.Start`: `fader.SetState(enabled)`, then
//!   `fader.FadeOut()`; settled (after the fader's `fadeInTime`), alpha 0;
//! - `uGUI_Overlays.Awake`: each overlay's `SetAlpha(0)`: the graphic is
//!   disabled unless its curve is above 0 at 0;
//! - `uGUI_HardcoreGameOver.Awake`: `inputBlocker` and `message`
//!   inactive.

use std::collections::{HashMap, HashSet};

use sn_unity::{Fields, PPtr, Value};

use crate::{Assets, Layouts, ObjectRef, Result, UiTree};

/// Overrides of the stored state, by node.
#[derive(Clone, Debug, Default)]
pub struct UiStartState {
    /// `CanvasGroup` alpha.
    pub group_alpha: HashMap<usize, f32>,
    /// Nodes made inactive (`SetActive(false)`).
    pub inactive: HashSet<usize>,
    /// Graphics disabled (`graphic.enabled = false`).
    pub disabled: HashSet<usize>,
    /// Each change, for the log.
    pub notes: Vec<String>,
}

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
            if n.game_object.key() == key || n.components.iter().any(|c| c.object.key() == key) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// The first-frame changes of `tree`'s faders, overlays and game-over
    /// screen.
    pub fn ui_start_state(&self, layouts: &Layouts, tree: &UiTree) -> Result<UiStartState> {
        let mut out = UiStartState::default();
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
            out.group_alpha.insert(g, alpha);
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
                        out.disabled.insert(g);
                        out.notes.push(format!(
                            "{}: overlay {} disabled (SetAlpha 0)",
                            tree.nodes[node].path, tree.nodes[g].path
                        ));
                    }
                }
            }
        }
        for (node, c) in tree.scripts("uGUI_HardcoreGameOver") {
            let (_, f) = self.script_fields(layouts, c)?;
            for name in ["inputBlocker", "message"] {
                let Some(p) = pptr(&f, name) else {
                    continue;
                };
                if let Some(g) = self.node_of(tree, &c.object, p)? {
                    out.inactive.insert(g);
                    out.notes.push(format!(
                        "{}: {name} {} inactive",
                        tree.nodes[node].path, tree.nodes[g].path
                    ));
                }
            }
        }
        Ok(out)
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
}
