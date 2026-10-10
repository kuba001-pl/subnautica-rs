//! Gameplay rules shared by the client and (from M10) the server
//! (`docs/DESIGN.md` § 4.3). So far: collision of a capsule against
//! triangles and primitive shapes (`collide`, M9a).
//!
//! Pure: no files, no network, no engine. Positions are Unity world
//! coordinates (left-handed, y up), computed in `f64` so that a few
//! kilometres from the origin a 1 cm skin is still far above rounding.

pub mod collide;
mod math;
pub mod player;

pub use math::V3;
