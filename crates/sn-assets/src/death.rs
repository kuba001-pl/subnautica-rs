//! The death, respawn and overlay screens' nodes, fields and numbers
//! (M9u6e1; `docs/formats/ui.md` § Death, respawn and overlays), for
//! `sn_ui::death`'s rules: `uGUI_PlayerDeath`'s `blackOverlay` and its
//! code's delays, rates and thresholds (IL), `uGUI_SceneRespawning`'s
//! `loadingText` and `loadingBackground` (a `uGUI_Fader` and its fields),
//! `uGUI_Overlays`' graphics and curves, and the `Loading` string.

use std::collections::HashMap;

use sn_dotnet::Assembly;
use sn_ui::death::{DeathConfig, FaderConfig, OverlayConfig, VignetteTimes};
use sn_unity::{Fields, PPtr, Value};

use crate::hud::curve_field;
use crate::{Assets, Layouts, Result, UiTree};

/// `uGUI_SceneRespawning.loadingMessage`'s key.
const LOADING: &str = "Loading";

fn pptr(f: &Fields, script: &str, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
        .ok_or_else(|| format!("{script}.{name}: not set"))
}

/// `uGUI_PlayerDeath`'s numbers from its code: the two `Invoke` delays
/// (`TriggerDeathVignette`), `Update`'s two rates and two thresholds.
pub fn vignette_times(code: &[u8]) -> Result<VignetteTimes> {
    let e = |e: sn_dotnet::Error| format!("{}: {e}", crate::GAME_ASSEMBLY);
    let asm = Assembly::parse(code).map_err(e)?;
    let ty = "uGUI_PlayerDeath";
    let calls = sn_dotnet::invokes(&asm, "", ty, "TriggerDeathVignette").map_err(e)?;
    let delay = |name: &str| -> Result<f32> {
        let mut d = calls.iter().filter(|(n, _)| n == name).map(|(_, v)| *v);
        let first = d
            .next()
            .ok_or_else(|| format!("{ty}.TriggerDeathVignette: no Invoke({name:?})"))?;
        if d.any(|v| v != first) {
            return Err(format!(
                "{ty}.TriggerDeathVignette: Invoke({name:?}) with two delays"
            ));
        }
        Ok(first)
    };
    let rates = sn_dotnet::multipliers_f32(&asm, "", ty, "Update").map_err(e)?;
    let limits =
        sn_dotnet::field_compares_f32(&asm, "", ty, "Update", ("UnityEngine", "Color", "a"))
            .map_err(e)?;
    let (&[rate_in, rate_out], &[full, clear]) = (&rates[..], &limits[..]) else {
        return Err(format!(
            "{ty}.Update: {} rates and {} alpha limits, expected 2 and 2",
            rates.len(),
            limits.len()
        ));
    };
    Ok(VignetteTimes {
        begin_fade_delay: delay("BeginDeathFade")?,
        fade_out_delay: delay("BeginFadeOut")?,
        rate_in,
        rate_out,
        full,
        clear,
    })
}

impl Assets<'_> {
    /// The three scripts in `tree` (`uGUI`; M9u6e1), the vignette's numbers
    /// from `code` (the game's assembly) and `Loading` from `language` (a
    /// key the file lacks is shown as itself).
    pub fn death_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        code: &[u8],
        language: &HashMap<String, String>,
    ) -> Result<DeathConfig> {
        // `uGUI_PlayerDeath`.
        let ty = "uGUI_PlayerDeath";
        let (_, c) = tree.scripts(ty).next().ok_or("no uGUI_PlayerDeath")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let black_overlay = self
            .node_of(tree, &c.object, pptr(&f, ty, "blackOverlay")?)?
            .ok_or("uGUI_PlayerDeath.blackOverlay: not in the tree")?;

        // `uGUI_SceneRespawning` and its fader.
        let ty = "uGUI_SceneRespawning";
        let (_, c) = tree.scripts(ty).next().ok_or("no uGUI_SceneRespawning")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let loading_text = self
            .node_of(tree, &c.object, pptr(&f, ty, "loadingText")?)?
            .ok_or("uGUI_SceneRespawning.loadingText: not in the tree")?;
        let p = pptr(&f, ty, "loadingBackground")?;
        let fader = self
            .resolve(&c.object.file, p)?
            .ok_or("uGUI_SceneRespawning.loadingBackground: not found")?;
        let (_, ff) = self.object_fields(layouts, &fader)?;
        let group = self
            .node_of(tree, &fader, pptr(&ff, "uGUI_Fader", "canvasGroup")?)?
            .ok_or("uGUI_Fader.canvasGroup: not in the tree")?;
        let num = |name: &str| {
            ff.get(name)
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .ok_or_else(|| format!("uGUI_Fader.{name}: not read"))
        };
        let flag = |name: &str| {
            ff.get(name)
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("uGUI_Fader.{name}: not read"))
        };
        let loading_background = FaderConfig {
            group,
            fade_in_time: num("fadeInTime")?,
            fade_out_time: num("fadeOutTime")?,
            use_unscaled_time: flag("useUnscaledTime")?,
            initial_state: flag("initialState")?,
        };

        // `uGUI_Overlays`.
        let ty = "uGUI_Overlays";
        let (_, c) = tree.scripts(ty).next().ok_or("no uGUI_Overlays")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let mut overlays = Vec::new();
        for (k, o) in f
            .get("overlays")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let o = o
                .as_struct()
                .ok_or_else(|| format!("uGUI_Overlays.overlays[{k}]: not a struct"))?;
            let p = o
                .get("graphic")
                .and_then(Value::as_pptr)
                .filter(|p| !p.is_null())
                .ok_or_else(|| format!("uGUI_Overlays.overlays[{k}].graphic: not set"))?;
            let graphic = self
                .node_of(tree, &c.object, p)?
                .ok_or_else(|| format!("uGUI_Overlays.overlays[{k}].graphic: not in the tree"))?;
            let curve =
                curve_field(o, "curve").map_err(|e| format!("uGUI_Overlays.overlays[{k}]: {e}"))?;
            overlays.push(OverlayConfig { graphic, curve });
        }
        if overlays.is_empty() {
            return Err("uGUI_Overlays: no overlays".into());
        }
        Ok(DeathConfig {
            black_overlay,
            vignette: vignette_times(code)?,
            loading_text,
            loading_background,
            loading: language
                .get(LOADING)
                .cloned()
                .unwrap_or_else(|| LOADING.to_string()),
            overlays,
        })
    }
}
