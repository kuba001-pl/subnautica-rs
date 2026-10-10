//! The game's animation, headless (M7f4b, `docs/formats/animation.md`):
//! sampling an `AnimationClip`'s curves, blend tree weights, and the
//! `Animator` — a controller's state machines, transitions, parameters
//! and layers run over time into one value per animated property.
//!
//! Pure: no files, no engine. Everything in the game is "generic"
//! animation (no humanoid rigs), so a pose is a flat list of property
//! values; the caller applies them (Transforms, blend shapes, …).

pub mod blend;
mod machine;
pub mod math;
pub mod sample;

pub use machine::{
    Animator, FiredEvent, LayerInfo, ParamValue, Program, Slot, SlotKind, event_crossed,
};
