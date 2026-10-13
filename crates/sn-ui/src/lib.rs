//! The game's UI, headless (M9u, `docs/DESIGN.md` "M9u plan",
//! `docs/formats/ui.md`). Pure: no engine, no files. It takes the bytes
//! and values `sn-assets` reads and gives numbers and pixels that can be
//! checked without a screen.
//!
//! - [`font`] (M9u3): TextMesh Pro's dynamic font assets: face metrics,
//!   glyph metrics, glyph distance fields and the atlas they are packed
//!   into, made from the game's font files as the game makes them.

pub mod font;
