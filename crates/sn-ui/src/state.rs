//! A UI tree's live state (M9u6a, moved here in M9u6b1; `docs/formats/ui.md`
//! § The live UI): the changes the game's scripts make to a tree through
//! Unity's API, by node. `sn-assets`' `UiScene` draws a tree for a state;
//! the screens' rules ([`crate::hud`]) write one each frame. Pure: sprites
//! are ids the scene gives out, not objects.

use std::collections::BTreeMap;

/// A sprite a script assigns (`Image.sprite`, `uGUI_Icon.sprite`), by the
/// id the scene gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteId(pub u64);

/// A node's local transform as a script sets it; `None` keeps the stored
/// value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalTransform {
    pub anchored_position: Option<[f32; 2]>,
    pub z: Option<f32>,
    /// A unit quaternion (x, y, z, w).
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    /// `sizeDelta` (M9u7c1: `SetSizeWithCurrentAnchors` with equal
    /// anchors sets it to the size).
    pub size_delta: Option<[f32; 2]>,
}

/// Values a script writes into its graphic's own material instance
/// (`material.SetFloat`, `SetVector`/`SetColor`, keywords).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialValues {
    pub floats: BTreeMap<String, f32>,
    /// Vectors and colours (colours as stored, not linear).
    pub vectors: BTreeMap<String, [f32; 4]>,
    /// Keywords switched on (`true`) or off.
    pub keywords: BTreeMap<String, bool>,
}

impl MaterialValues {
    pub fn is_empty(&self) -> bool {
        self.floats.is_empty() && self.vectors.is_empty() && self.keywords.is_empty()
    }

    /// `other`'s values over these.
    pub fn apply(&mut self, other: &MaterialValues) {
        self.floats
            .extend(other.floats.iter().map(|(k, v)| (k.clone(), *v)));
        self.vectors
            .extend(other.vectors.iter().map(|(k, v)| (k.clone(), *v)));
        self.keywords
            .extend(other.keywords.iter().map(|(k, v)| (k.clone(), *v)));
    }
}

/// The scripts' changes to a tree, by node. A node not in a map keeps its
/// stored value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiState {
    /// `GameObject.SetActive`.
    pub active: BTreeMap<usize, bool>,
    /// `CanvasGroup.alpha`.
    pub group_alpha: BTreeMap<usize, f32>,
    /// `CanvasRenderer.SetAlpha`.
    pub renderer_alpha: BTreeMap<usize, f32>,
    /// `Graphic.enabled` (images, raw images, texts, the game's graphics).
    pub enabled: BTreeMap<usize, bool>,
    /// `Graphic.color`, as Unity's `Color` (stored, not linear).
    pub color: BTreeMap<usize, [f32; 4]>,
    /// `Image.sprite` or `uGUI_Icon.sprite` (`None`: no sprite).
    pub sprite: BTreeMap<usize, Option<SpriteId>>,
    /// `Image.fillAmount`.
    pub fill_amount: BTreeMap<usize, f32>,
    /// `RawImage.uvRect` (x, y, width, height; M9u8b1).
    pub uv_rect: BTreeMap<usize, [f32; 4]>,
    /// TMP's `text`.
    pub text: BTreeMap<usize, String>,
    pub transform: BTreeMap<usize, LocalTransform>,
    /// The graphic's own material values.
    pub material: BTreeMap<usize, MaterialValues>,
}

impl UiState {
    /// `other`'s changes over this state's.
    pub fn apply(&mut self, other: &UiState) {
        self.active
            .extend(other.active.iter().map(|(k, v)| (*k, *v)));
        self.group_alpha
            .extend(other.group_alpha.iter().map(|(k, v)| (*k, *v)));
        self.renderer_alpha
            .extend(other.renderer_alpha.iter().map(|(k, v)| (*k, *v)));
        self.enabled
            .extend(other.enabled.iter().map(|(k, v)| (*k, *v)));
        self.color.extend(other.color.iter().map(|(k, v)| (*k, *v)));
        self.sprite
            .extend(other.sprite.iter().map(|(k, v)| (*k, *v)));
        self.fill_amount
            .extend(other.fill_amount.iter().map(|(k, v)| (*k, *v)));
        self.uv_rect
            .extend(other.uv_rect.iter().map(|(k, v)| (*k, *v)));
        self.text
            .extend(other.text.iter().map(|(k, v)| (*k, v.clone())));
        for (k, t) in &other.transform {
            let e = self.transform.entry(*k).or_default();
            e.anchored_position = t.anchored_position.or(e.anchored_position);
            e.z = t.z.or(e.z);
            e.rotation = t.rotation.or(e.rotation);
            e.scale = t.scale.or(e.scale);
            e.size_delta = t.size_delta.or(e.size_delta);
        }
        for (k, m) in &other.material {
            self.material.entry(*k).or_default().apply(m);
        }
    }
}

pub use crate::mesh::color32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color32_rounds_half_to_even_and_clamps() {
        // 0.5 × 255 = 127.5 → 128 (even); 1.5/255 × 255 = 1.5 → 2;
        // 2.5/255 × 255 = 2.5 → 2.
        assert_eq!(
            color32([0.5, 1.5 / 255.0, 2.5 / 255.0, 2.0]),
            [128, 2, 2, 255]
        );
        assert_eq!(color32([-1.0, 0.0, 1.0, 0.999]), [0, 0, 255, 255]);
    }

    #[test]
    fn a_later_state_overrides_an_earlier_one_field_by_field() {
        let mut a = UiState::default();
        a.active.insert(1, false);
        a.transform.insert(
            2,
            LocalTransform {
                scale: Some([2.0, 2.0, 1.0]),
                ..LocalTransform::default()
            },
        );
        a.material
            .entry(4)
            .or_default()
            .floats
            .insert("_Value".into(), 0.5);
        let mut b = UiState::default();
        b.active.insert(1, true);
        b.text.insert(3, "12".into());
        b.transform.insert(
            2,
            LocalTransform {
                z: Some(5.0),
                ..LocalTransform::default()
            },
        );
        b.material
            .entry(4)
            .or_default()
            .vectors
            .insert("_Overlay1_ST".into(), [1.0, 1.0, 0.0, -0.25]);
        a.apply(&b);
        assert!(a.active[&1]);
        assert_eq!(a.text[&3], "12");
        assert_eq!(a.transform[&2].scale, Some([2.0, 2.0, 1.0]));
        assert_eq!(a.transform[&2].z, Some(5.0));
        assert_eq!(a.material[&4].floats["_Value"], 0.5);
        assert_eq!(a.material[&4].vectors["_Overlay1_ST"][3], -0.25);
    }
}
