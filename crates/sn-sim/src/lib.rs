//! Gameplay rules shared by the client and (from M10) the server
//! (`docs/DESIGN.md` § 4.3). So far: collision of a capsule against
//! triangles and primitive shapes (`collide`, M9a), the player's movement
//! (`player`, M9b), oxygen, health and death (`vitals`) and the mouse look
//! (`look`, M9c), the Aurora's explosion and exterior cull (`aurora`,
//! M7f4e), the lighting states of the lifepod (`lighting`, M7f4g), the
//! player's body: its animator's values and the camera rig (`body`, M9g2).
//!
//! Pure: no files, no network, no engine. Positions are Unity world
//! coordinates (left-handed, y up), computed in `f64` so that a few
//! kilometres from the origin a 1 cm skin is still far above rounding.

pub mod aurora;
pub mod body;
pub mod collide;
pub mod lighting;
pub mod look;
mod math;
pub mod player;
pub mod vitals;

pub use math::V3;
