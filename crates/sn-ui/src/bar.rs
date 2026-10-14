//! `uGUI_CircularBar` (M9u6b1; `docs/formats/ui.md` § The HUD's bars), the
//! game's own `Graphic` that draws the HUD's round bars, ported from the
//! decompiled script (read only; the code is ours). It builds no mesh of
//! its own (`Graphic`'s quad, [`crate::mesh::graphic_quad`]) and draws with
//! its own material instance of `Hidden/UI/CircularBar`, whose values its
//! property setters write ([`CircularBar::material`]). Its rect is sized
//! by its layout (`size` on both axes, `crate::layout`).

use crate::material::UiMaterial;

/// The shader (`ShaderManager.preloadedShaders.UICircularBar`).
pub const SHADER: &str = "Hidden/UI/CircularBar";

/// `_value` before a script sets it (a field initializer, not
/// serialized).
pub const START_VALUE: f32 = 0.75;

/// A bar's serialized fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularBar {
    /// `Graphic.color`.
    pub color: [f32; 4],
    pub border: bool,
    pub border_color: [f32; 4],
    pub edge_width: f32,
    pub border_width: f32,
    pub round_corners: bool,
    pub size: f32,
    pub width: f32,
    pub max_value: f32,
    pub overlay1_st: [f32; 4],
    pub overlay2_st: [f32; 4],
    pub overlay_shift: [f32; 2],
    pub overlay_alpha: [f32; 2],
}

impl CircularBar {
    /// `materialForRendering` after `UpdateMaterialAll`: the keywords
    /// (`BORDER`, `ROUND_CORNERS`, `OVERLAY` when there is an overlay
    /// texture) and the values each setter writes, for `value`.
    /// `main_texture` and `overlay` are the caller's texture keys (0: the
    /// white texture, or no overlay); `reference_ppu` is the canvas's
    /// `referencePixelsPerUnit`.
    pub fn material(
        &self,
        value: f32,
        main_texture: u64,
        overlay: u64,
        reference_ppu: f32,
    ) -> UiMaterial {
        let mut m = UiMaterial {
            name: "uGUI_CircularBar".into(),
            shader: SHADER.into(),
            ..UiMaterial::default()
        };
        for (k, on) in [
            ("BORDER", self.border),
            ("ROUND_CORNERS", self.round_corners),
            ("OVERLAY", overlay != 0),
        ] {
            if on {
                m.keywords.push(k.into());
            }
        }
        // `UpdateMaterialWidth`: (width / ppu) / (size / ppu).
        let unit = 1.0 / reference_ppu;
        m.floats
            .insert("_Width".into(), (self.width * unit) / (self.size * unit));
        m.floats
            .insert("_EdgeWidth".into(), self.edge_width.clamp(0.0, 1.0));
        m.floats
            .insert("_BorderWidth".into(), self.border_width.clamp(0.0, 1.0));
        m.floats.insert("_Value".into(), self.value(value));
        m.colors.insert("_Color".into(), [1.0; 4]);
        m.colors.insert("_BorderColor".into(), self.border_color);
        m.colors.insert("_Overlay1_ST".into(), self.overlay1_st);
        m.colors.insert("_Overlay2_ST".into(), self.overlay2_st);
        let [sx, sy] = self.overlay_shift;
        m.colors.insert("_OverlayShift".into(), [sx, sy, 0.0, 0.0]);
        let [ax, ay] = self.overlay_alpha;
        m.colors.insert("_OverlayAlpha".into(), [ax, ay, 0.0, 0.0]);
        m.textures.insert("_MainTex".into(), main_texture);
        m.textures.insert("_OverlayTex".into(), overlay);
        m
    }

    /// `UpdateMaterialValue`: `_Value` = `Clamp01(value × maxValue)`.
    pub fn value(&self, value: f32) -> f32 {
        (value * self.max_value).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> CircularBar {
        CircularBar {
            color: [1.0; 4],
            border: true,
            border_color: [1.0, 0.5, 0.0, 1.0],
            edge_width: 0.15,
            border_width: 1.4,
            round_corners: true,
            size: 52.0,
            width: 20.0,
            max_value: 1.0,
            overlay1_st: [1.0, 1.0, 0.0, 0.0],
            overlay2_st: [1.0, 1.0, 0.0, 0.0],
            overlay_shift: [0.3, 0.07],
            overlay_alpha: [0.5, 0.25],
        }
    }

    #[test]
    fn the_material_holds_what_the_setters_write() {
        let m = bar().material(0.5, 0, 0, 100.0);
        assert_eq!(m.shader, SHADER);
        assert_eq!(m.keywords, ["BORDER", "ROUND_CORNERS"]);
        // 20/52, as (20 × 0.01) / (52 × 0.01) in single precision.
        assert!((m.float("_Width") - 20.0 / 52.0).abs() < 1e-6);
        assert_eq!(m.float("_Value"), 0.5);
        // Widths are clamped to 0…1.
        assert_eq!(m.float("_BorderWidth"), 1.0);
        assert_eq!(m.colors["_OverlayShift"], [0.3, 0.07, 0.0, 0.0]);
        let with_overlay = bar().material(0.5, 0, 7, 100.0);
        assert!(with_overlay.has_keyword("OVERLAY"));
        assert_eq!(with_overlay.textures["_OverlayTex"], 7);
    }

    #[test]
    fn the_value_is_scaled_by_max_value_and_clamped() {
        let mut b = bar();
        b.max_value = 2.0;
        assert_eq!(b.value(0.25), 0.5);
        assert_eq!(b.value(0.75), 1.0);
        assert_eq!(b.value(-1.0), 0.0);
    }
}
