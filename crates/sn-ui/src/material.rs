//! UI materials (M9u5a, `docs/formats/ui.md` § UI shaders and materials):
//! what a draw needs to pick its shader program and fill its constants, and
//! TextMesh Pro's rules that read a material (ported from the decompiled
//! `TMPro.ShaderUtilities` and `TMP_MaterialManager`, read only; the code
//! is ours):
//! - [`update_shader_ratios`]: `_ScaleRatioA`, `B`, `C` from the face,
//!   outline, glow and underlay settings (`UpdateShaderRatios`);
//! - [`tmp_padding`]: how far a character's quad reaches past its glyph
//!   (`GetPadding`);
//! - [`fallback_material`]: the material of characters drawn from a
//!   fallback font (`GetFallbackMaterial`);
//! - [`pack_uv`]: TMP's packing of a face-texture UV into one float.

use std::collections::BTreeMap;

/// The shader every `Graphic` without a material draws with
/// (`Canvas.GetDefaultCanvasMaterial`).
pub const UI_DEFAULT: &str = "UI/Default";
/// TMP's SDF shader.
pub const TMP_SDF: &str = "TextMeshPro/Distance Field";

/// A material as the UI draws it: its saved properties.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiMaterial {
    pub name: String,
    pub shader: String,
    /// `m_ShaderKeywords`, split.
    pub keywords: Vec<String>,
    pub floats: BTreeMap<String, f32>,
    pub colors: BTreeMap<String, [f32; 4]>,
    /// Texture property → the caller's texture key (0: none).
    pub textures: BTreeMap<String, u64>,
}

impl UiMaterial {
    /// `Canvas.GetDefaultCanvasMaterial()`: `UI/Default` with its shader's
    /// defaults (the programs read `_Color` white and `_MainTex_ST`
    /// identity).
    pub fn ui_default() -> UiMaterial {
        let mut m = UiMaterial {
            name: "Default UI Material".into(),
            shader: UI_DEFAULT.into(),
            ..UiMaterial::default()
        };
        m.colors.insert("_Color".into(), [1.0; 4]);
        m
    }

    /// `Material.HasProperty`, for saved properties (an editor-saved
    /// material holds every property of its shader).
    pub fn has(&self, name: &str) -> bool {
        self.floats.contains_key(name)
            || self.colors.contains_key(name)
            || self.textures.contains_key(name)
    }

    pub fn float(&self, name: &str) -> f32 {
        self.floats.get(name).copied().unwrap_or(0.0)
    }

    pub fn has_keyword(&self, k: &str) -> bool {
        self.keywords.iter().any(|w| w == k)
    }

    /// The keyword set a draw selects its program by: the material's
    /// keywords, plus `UNITY_UI_CLIP_RECT` when the graphic is clipped by
    /// a `RectMask2D` (set natively by `CanvasRenderer.EnableRectClipping`;
    /// Unity's documented behaviour, *hypothesis* until drawn), sorted.
    pub fn draw_keywords(&self, clipped: bool) -> Vec<String> {
        let mut k = self.keywords.clone();
        if clipped && !self.has_keyword("UNITY_UI_CLIP_RECT") {
            k.push("UNITY_UI_CLIP_RECT".into());
        }
        k.sort();
        k.dedup();
        k
    }
}

/// `ShaderUtilities.m_clamp`.
const CLAMP: f32 = 1.0;

/// `ShaderUtilities.UpdateShaderRatios`: sets `_ScaleRatioA` (and `B`
/// with a glow, `C` with an underlay) unless the material has
/// `RATIOS_OFF`, in which case they are 1.
pub fn update_shader_ratios(m: &mut UiMaterial) {
    if !(m.has("_GradientScale") && m.has("_FaceDilate")) {
        return;
    }
    let ratios = !m.has_keyword("RATIOS_OFF");
    let gradient = m.float("_GradientScale");
    let dilate = m.float("_FaceDilate");
    let weight = m.float("_WeightNormal").max(m.float("_WeightBold")) / 4.0;
    let range = 1f32.max(weight + dilate + m.float("_OutlineWidth") + m.float("_OutlineSoftness"));
    let a = if ratios {
        (gradient - CLAMP) / (gradient * range)
    } else {
        1.0
    };
    m.floats.insert("_ScaleRatioA".into(), a);
    if m.has("_GlowOffset") {
        let inner = (weight + dilate) * (gradient - CLAMP);
        let range = 1f32.max(m.float("_GlowOffset") + m.float("_GlowOuter"));
        let b = if ratios {
            0f32.max(gradient - CLAMP - inner) / (gradient * range)
        } else {
            1.0
        };
        m.floats.insert("_ScaleRatioB".into(), b);
    }
    if m.has("_UnderlayOffsetX") {
        let inner = (weight + dilate) * (gradient - CLAMP);
        let offset = m
            .float("_UnderlayOffsetX")
            .abs()
            .max(m.float("_UnderlayOffsetY").abs());
        let range = 1f32.max(offset + m.float("_UnderlayDilate") + m.float("_UnderlaySoftness"));
        let c = if ratios {
            0f32.max(gradient - CLAMP - inner) / (gradient * range)
        } else {
            1.0
        };
        m.floats.insert("_ScaleRatioC".into(), c);
    }
}

/// `ShaderUtilities.GetPadding(material, enableExtraPadding, isBold)`:
/// runs [`update_shader_ratios`] on `m` (as the game does), then gives the
/// padding in atlas texels (a text without a material has 0).
pub fn tmp_padding(m: &mut UiMaterial, extra_padding: bool) -> f32 {
    let extra = if extra_padding { 4.0 } else { 0.0 };
    if !m.has("_GradientScale") {
        let own = if m.has("_Padding") {
            m.float("_Padding").trunc()
        } else {
            0.0
        };
        return extra + own + 1.0;
    }
    update_shader_ratios(m);
    let ratio_a = if m.has("_ScaleRatioA") {
        m.float("_ScaleRatioA")
    } else {
        0.0
    };
    let dilate = if m.has("_FaceDilate") {
        m.float("_FaceDilate") * ratio_a
    } else {
        0.0
    };
    let softness = if m.has("_OutlineSoftness") {
        m.float("_OutlineSoftness") * ratio_a
    } else {
        0.0
    };
    let width = if m.has("_OutlineWidth") {
        m.float("_OutlineWidth") * ratio_a
    } else {
        0.0
    };
    let mut uniform = width + softness + dilate;
    if m.has("_GlowOffset") && m.has_keyword("GLOW_ON") {
        let ratio_b = if m.has("_ScaleRatioB") {
            m.float("_ScaleRatioB")
        } else {
            0.0
        };
        let glow = m.float("_GlowOffset") * ratio_b + m.float("_GlowOuter") * ratio_b;
        uniform = uniform.max(dilate + glow);
    }
    let mut sides = [0f32; 4];
    if m.has("_UnderlaySoftness") && m.has_keyword("UNDERLAY_ON") {
        let ratio_c = if m.has("_ScaleRatioC") {
            m.float("_ScaleRatioC")
        } else {
            0.0
        };
        let x = m.float("_UnderlayOffsetX") * ratio_c;
        let y = m.float("_UnderlayOffsetY") * ratio_c;
        let reach = dilate + (m.float("_UnderlayDilate") + m.float("_UnderlaySoftness")) * ratio_c;
        sides = [reach - x, reach - y, reach + x, reach + y].map(|s| s.max(0.0));
    }
    let gradient = m.float("_GradientScale");
    let mut out = 0f32;
    for s in sides {
        out = out.max((s.max(uniform) + extra).min(1.0) * gradient);
    }
    out + 1.25
}

/// `TMP_MaterialManager.GetFallbackMaterial(source, target)`: a copy of
/// the text's material with the fallback font's atlas, gradient scale,
/// atlas size and weights (when both are SDF materials), else a copy of
/// the fallback font's own material.
pub fn fallback_material(source: &UiMaterial, target: &UiMaterial) -> UiMaterial {
    if !(source.has("_GradientScale") && target.has("_GradientScale")) {
        return target.clone();
    }
    let mut m = source.clone();
    m.textures.insert(
        "_MainTex".into(),
        target.textures.get("_MainTex").copied().unwrap_or(0),
    );
    for k in [
        "_GradientScale",
        "_TextureWidth",
        "_TextureHeight",
        "_WeightNormal",
        "_WeightBold",
    ] {
        m.floats.insert(k.into(), target.float(k));
    }
    m
}

/// `TMP_Text.PackUV(x, y)`: each of `x`, `y` (0 to 1) to 9 bits, packed as
/// `x · 4096 + y`.
pub fn pack_uv(x: f32, y: f32) -> f32 {
    let a = f64::from((x * 511.0) as i32);
    let b = f64::from((y * 511.0) as i32);
    (a * 4096.0 + b) as f32
}

#[cfg(test)]
mod tests;
