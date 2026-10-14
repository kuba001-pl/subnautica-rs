//! The game's UI, headless (M9u, `docs/DESIGN.md` "M9u plan",
//! `docs/formats/ui.md`). Pure: no engine, no files. It takes the bytes
//! and values `sn-assets` reads and gives numbers and pixels that can be
//! checked without a screen.
//!
//! - [`font`] (M9u3): TextMesh Pro's dynamic font assets: face metrics,
//!   glyph metrics, glyph distance fields and the atlas they are packed
//!   into, made from the game's font files as the game makes them.
//! - [`layout`] (M9u4a): RectTransform rects and uGUI's layout system
//!   (layout groups, fitters, layout elements).
//! - [`mesh`] (M9u4b): the meshes `Image` and `RawImage` build.
//! - [`draw`] (M9u4d): the draw list: order, canvas space, alpha, clipping.
//! - [`material`] (M9u5a): UI materials and TextMesh Pro's material rules.
//! - [`scaler`] (M9u5b): the UI camera and `uGUI_CanvasScaler`'s placement.
//! - [`text`] (M9u4c): TextMesh Pro's text layout, auto-size and
//!   preferred sizes.
//! - [`tween`] (M9u6a): `Mathf.SmoothDamp` and UWE's `Sequence`, which the
//!   UI's scripts share.

pub mod draw;
pub mod font;
pub mod layout;
pub mod material;
pub mod mesh;
pub mod scaler;
pub mod text;
pub mod tween;
