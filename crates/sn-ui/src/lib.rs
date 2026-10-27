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
//! - [`bar`] (M9u6b1): `uGUI_CircularBar`, the HUD's round bars.
//! - [`compass`] (M9u6c2): `uGUI_Compass`, the HUD's compass ring.
//! - [`death`] (M9u6e1): the death vignette, the respawn screen and the
//!   out-of-oxygen overlay.
//! - [`depth`] (M9u6c1): `uGUI_DepthCompass`, the depth readout's rules.
//! - [`draw`] (M9u4d): the draw list: order, canvas space, alpha, clipping.
//! - [`hud`] (M9u6b1): the HUD's rules (`uGUI_SceneHUD`, the bars).
//! - [`icon`] (M9u7c1): `uGUI_Icon`'s mesh and the item icon's sizing.
//! - [`reticle`] (M9u6d2): `HandReticle`, the hand's icon and texts.
//! - [`material`] (M9u5a): UI materials and TextMesh Pro's material rules.
//! - [`messages`] (M9u7b): `ErrorMessage`, the HUD's text messages.
//! - [`notifier`] (M9u7c2): `uGUI_IconNotifier`, the picked-up item icons.
//! - [`pda`] (M9u8a4a): `uGUI_PDA`'s screen states as the PDA opens and
//!   closes, and its tabs.
//! - [`popup`] (M9u7d1): `uGUI_PopupNotification`, the unlock pop-up.
//! - [`scaler`] (M9u5b): the UI camera and `uGUI_CanvasScaler`'s placement.
//! - [`tooltip`] (M9u8b3b): `uGUI_Tooltip` and `TooltipFactory`'s texts.
//! - [`text`] (M9u4c): TextMesh Pro's text layout, auto-size and
//!   preferred sizes.
//! - [`state`] (M9u6a): a tree's live state, the scripts' changes.
//! - [`tween`] (M9u6a): `Mathf.SmoothDamp` and UWE's `Sequence`, which the
//!   UI's scripts share.

pub mod bar;
pub mod clip;
pub mod compass;
pub mod death;
pub mod depth;
pub mod drag;
pub mod draw;
pub mod font;
pub mod hud;
pub mod icon;
pub mod inventory_tab;
pub mod item_icons;
pub mod layout;
pub mod material;
pub mod mesh;
pub mod messages;
pub mod notifications;
pub mod notifier;
pub mod pda;
pub mod popup;
pub mod reticle;
pub mod scaler;
pub mod state;
pub mod text;
pub mod toolbar;
pub mod tooltip;
pub mod tween;
