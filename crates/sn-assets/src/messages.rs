//! `ErrorMessage`'s nodes, fields and numbers (M9u7b; `docs/formats/ui.md`
//! § Messages), for `sn_ui::messages`' rules: `messageCanvas`, `offset`,
//! `ySpacing` (stored), the four times (its constructor's initialisers),
//! `OnLateUpdate`'s slide rate and snap (IL), and `prefabMessage` (a root
//! of `uGUI.prefab`'s file) read as a scene to graft, with its font colour.

use std::collections::HashMap;

use sn_dotnet::Assembly;
use sn_ui::messages::{MessageLayout, MessageTimes, MessagesConfig};
use sn_ui::scaler::{Placement, SCALE_DONT};
use sn_ui::text::FontSet;
use sn_unity::{Fields, Value};

use crate::{Assets, FontAssets, Layouts, Result, UiScene, UiState, UiTree};

const TYPE: &str = "ErrorMessage";

/// `ErrorMessage`'s times and `OnLateUpdate`'s two constants from its
/// code: `offsetY * 7f` (the one `ldc.r4; mul`) and `offsetY < 1f` (the
/// other constant read after `offsetY`).
pub fn message_times(code: &[u8]) -> Result<MessageTimes> {
    let e = |e: sn_dotnet::Error| format!("{}: {e}", crate::GAME_ASSEMBLY);
    let asm = Assembly::parse(code).map_err(e)?;
    let init = |f: &str| sn_dotnet::field_initializer_f32(&asm, "", TYPE, f).map_err(e);
    let rates = sn_dotnet::multipliers_f32(&asm, "", TYPE, "OnLateUpdate").map_err(e)?;
    let after =
        sn_dotnet::field_compares_f32(&asm, "", TYPE, "OnLateUpdate", ("", TYPE, "offsetY"))
            .map_err(e)?;
    let (&[rate], &[r, snap]) = (&rates[..], &after[..]) else {
        return Err(format!(
            "{TYPE}.OnLateUpdate: {} multipliers and {} constants after offsetY, expected 1 and 2",
            rates.len(),
            after.len()
        ));
    };
    if r != rate {
        return Err(format!(
            "{TYPE}.OnLateUpdate: offsetY × {r} is not the multiplier {rate}"
        ));
    }
    Ok(MessageTimes {
        fly_in: init("timeFlyIn")?,
        delay: init("timeDelay")?,
        fade_out: init("timeFadeOut")?,
        invisible: init("timeInvisible")?,
        slide_rate: rate,
        snap,
    })
}

fn vector(f: &Fields, name: &str) -> Result<[f32; 4]> {
    let s = f
        .get(name)
        .and_then(Value::as_struct)
        .ok_or_else(|| format!("{name}: not read"))?;
    let mut out = [1.0; 4];
    for (k, c) in ["r", "g", "b", "a"].iter().enumerate() {
        out[k] = s
            .get(c)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .ok_or_else(|| format!("{name}.{c}: not read"))?;
    }
    Ok(out)
}

impl Assets<'_> {
    /// `ErrorMessage` in `tree` (`uGUI`) and its code, and its
    /// `prefabMessage` read as a scene (with `fonts` and the TagManager's
    /// `sorting_layers`) for `UiScene::graft`.
    pub fn messages_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        code: &[u8],
        fonts: &FontAssets,
        sorting_layers: &[(String, u32)],
    ) -> Result<(MessagesConfig, UiScene)> {
        let (_, c) = tree.scripts(TYPE).next().ok_or("no ErrorMessage")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let ptr = |n: &str| {
            f.get(n)
                .and_then(Value::as_pptr)
                .filter(|p| !p.is_null())
                .ok_or_else(|| format!("{TYPE}.{n}: not set"))
        };
        let num = |s: &Fields, n: &str| {
            s.get(n)
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .ok_or_else(|| format!("{TYPE}.{n}: not read"))
        };
        // `messageCanvas` is a RectTransform: the node holding it.
        let rect = self
            .resolve(&c.object.file, ptr("messageCanvas")?)?
            .ok_or("ErrorMessage.messageCanvas: not found")?;
        let canvas = tree
            .nodes
            .iter()
            .position(|n| n.components.iter().any(|c| c.object.key() == rect.key()))
            .ok_or("ErrorMessage.messageCanvas: not in the tree")?;
        let offset = f
            .get("offset")
            .and_then(Value::as_struct)
            .ok_or("ErrorMessage.offset: not read")?;
        let prefab = self
            .resolve(&c.object.file, ptr("prefabMessage")?)?
            .ok_or("ErrorMessage.prefabMessage: not found")?;
        let template_tree = self.ui_tree("MessageInstance", &prefab)?;
        // `CompileTimeCheck`: the prefab's own `TextMeshProUGUI`.
        let (_, text) = template_tree
            .scripts("TextMeshProUGUI")
            .find(|(n, _)| *n == 0)
            .ok_or("ErrorMessage.prefabMessage: no TextMeshProUGUI")?;
        let (_, tf) = self.script_fields(layouts, text)?;
        let font_color = vector(&tf, "m_fontColor").map_err(|e| format!("prefabMessage: {e}"))?;
        let template = self.ui_scene(layouts, &template_tree, fonts, sorting_layers)?;
        Ok((
            MessagesConfig {
                canvas,
                offset: [num(offset, "x")?, num(offset, "y")?],
                y_spacing: num(&f, "ySpacing")?,
                times: message_times(code)?,
                font_color,
            },
            template,
        ))
    }
}

/// What `OnLateUpdate` reads of `scene`'s layout for `state`, with the
/// canvases placed: `messageCanvas.rect`, and an entry's width and anchor
/// point from `template`'s stored `RectTransform` under that canvas (the
/// copies keep it: `SetParent(…, false)`, and nothing drives their size).
/// The scaler: `uGUI_CanvasScaler.GetInverseScale`, the nearest scaler at
/// or above the canvas (`scale_modes`, by node) at `ui_scale`.
#[allow(clippy::too_many_arguments)]
pub fn message_layout(
    assets: &Assets,
    scene: &mut UiScene,
    set: &FontSet,
    placements: &HashMap<usize, Placement>,
    state: &UiState,
    config: &MessagesConfig,
    template: &UiScene,
    scale_modes: &HashMap<usize, i32>,
    ui_scale: f32,
) -> Result<MessageLayout> {
    let tree = scene.layout_for(assets, set, placements, state)?;
    let canvas_rect = tree.rect(config.canvas);
    let r = template
        .stored_rect(0)
        .ok_or("prefabMessage: no RectTransform")?;
    let size = canvas_rect.size();
    let corner = [canvas_rect.x, canvas_rect.y];
    let mut anchor_base = [0.0; 2];
    for a in 0..2 {
        let anchor = r.anchor_min[a] + (r.anchor_max[a] - r.anchor_min[a]) * r.pivot[a];
        anchor_base[a] = corner[a] + size[a] * anchor;
    }
    let entry_width = size[0] * (r.anchor_max[0] - r.anchor_min[0]) + r.size_delta[0];
    let mut inverse_scale = 1.0;
    let mut at = Some(config.canvas);
    while let Some(n) = at {
        if let Some(&mode) = scale_modes.get(&n) {
            if mode > SCALE_DONT {
                inverse_scale = 1.0 / ui_scale;
            }
            break;
        }
        at = tree.nodes.get(n).and_then(|n| n.parent);
    }
    Ok(MessageLayout {
        canvas_rect,
        entry_width,
        anchor_base,
        inverse_scale,
    })
}
