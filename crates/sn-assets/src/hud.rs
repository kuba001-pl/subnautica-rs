//! The HUD's scripts' nodes and fields (M9u6b1; `docs/formats/ui.md`
//! § The HUD's bars), for `sn_ui::hud`'s rules: `uGUI_SceneHUD`'s elements
//! and the four bar scripts' `bar`, `text` and fields. (`Survival`'s start
//! food and water are in its code: `crate::survival_start`.)

use sn_ui::hud::{BarConfig, BarKind, HudConfig, Overlays};
use sn_unity::{Fields, PPtr, Value};

use crate::{Assets, Layouts, Result, UiGraphicKind, UiTree};

fn pptr(f: &Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
        .ok_or_else(|| format!("no reference {name}"))
}

fn num(f: &Fields, name: &str) -> Result<f32> {
    f.get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| format!("no number {name}"))
}

impl Assets<'_> {
    /// `uGUI_SceneHUD` and its bars in `tree` (`uGUI`).
    pub fn hud_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<HudConfig> {
        let (_, scene_hud) = tree
            .scripts("uGUI_SceneHUD")
            .next()
            .ok_or("no uGUI_SceneHUD")?;
        let (_, f) = self.script_fields(layouts, scene_hud)?;
        let from = &scene_hud.object;
        let node = |name: &str| -> Result<usize> {
            self.node_of(tree, from, pptr(&f, name)?)?
                .ok_or_else(|| format!("uGUI_SceneHUD.{name}: not in the tree"))
        };
        let graphics = self.ui_graphics(layouts, tree)?;
        let mut bars = Vec::new();
        for (kind, field, script) in [
            (BarKind::Oxygen, "barOxygen", "uGUI_OxygenBar"),
            (BarKind::Health, "barHealth", "uGUI_HealthBar"),
            (BarKind::Food, "barFood", "uGUI_FoodBar"),
            (BarKind::Water, "barWater", "uGUI_WaterBar"),
        ] {
            let root = node(field)?;
            let (_, c) = tree
                .scripts(script)
                .find(|(n, _)| *n == root)
                .ok_or_else(|| format!("{script} not on {}", tree.nodes[root].path))?;
            let (_, b) = self.script_fields(layouts, c)?;
            let of = |name: &str| -> Result<usize> {
                self.node_of(tree, &c.object, pptr(&b, name)?)?
                    .ok_or_else(|| format!("{script}.{name}: not in the tree"))
            };
            let bar = of("bar")?;
            let circular = graphics
                .iter()
                .find_map(|g| match &g.kind {
                    UiGraphicKind::CircularBar { bar: cb, .. } if g.node == bar => Some(*cb),
                    _ => None,
                })
                .ok_or_else(|| format!("{script}.bar is not a uGUI_CircularBar"))?;
            let overlays = if kind == BarKind::Oxygen {
                Some(Overlays {
                    speed: [num(&b, "overlay1Speed")?, num(&b, "overlay2Speed")?],
                    alpha: [num(&b, "overlay1Alpha")?, num(&b, "overlay2Alpha")?],
                })
            } else {
                None
            };
            bars.push(BarConfig {
                kind,
                root,
                bar,
                circular,
                text: of("text")?,
                damp_speed: num(&b, "dampSpeed")?,
                overlays,
            });
        }
        Ok(HudConfig {
            background_double: node("backgroundBarsDouble")?,
            background_quad: node("backgroundBarsQuad")?,
            bars,
        })
    }
}
