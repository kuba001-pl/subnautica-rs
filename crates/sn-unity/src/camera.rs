//! `Camera` (class 20), Unity 2019.4 layout, the fields up to the culling
//! mask (`docs/formats/unity.md` § Cameras and layers).

use crate::Result;
use crate::objects::PPtr;
use crate::reader::Reader;

/// The built-in tag `MainCamera`: `Camera.main` is the enabled camera whose
/// GameObject has it.
pub const TAG_MAIN_CAMERA: u16 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub game_object: PPtr,
    pub enabled: bool,
    pub near: f32,
    pub far: f32,
    /// Vertical, degrees.
    pub field_of_view: f32,
    pub orthographic: bool,
    pub depth: f32,
    /// Bit `n` set: the camera draws renderers on layer `n`.
    pub culling_mask: u32,
}

impl Camera {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Camera> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let enabled = r.u8()? != 0;
        r.align(4)?;
        let _clear_flags = r.u32()?;
        let _background = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let _projection_matrix_mode = r.i32()?;
        let _gate_fit = r.i32()?;
        let _sensor_size = [r.f32()?, r.f32()?];
        let _lens_shift = [r.f32()?, r.f32()?];
        let _focal_length = r.f32()?;
        let _viewport = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let near = r.f32()?;
        let far = r.f32()?;
        let field_of_view = r.f32()?;
        let orthographic = r.u8()? != 0;
        r.align(4)?;
        let _orthographic_size = r.f32()?;
        let depth = r.f32()?;
        let culling_mask = r.u32()?;
        Ok(Camera {
            game_object,
            enabled,
            near,
            far,
            field_of_view,
            orthographic,
            depth,
            culling_mask,
        })
    }

    /// Whether the camera draws renderers on `layer` (0–31).
    pub fn draws_layer(&self, layer: u32) -> bool {
        layer < 32 && self.culling_mask & (1 << layer) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera as Unity 2019.4 stores it, up to the culling mask (108
    /// bytes), followed by a few bytes of the fields we don't read.
    fn camera_bytes(mask: u32) -> Vec<u8> {
        let mut b = Vec::new();
        let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&34i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1u32.to_le_bytes());
        for v in [0.0, 0.0, 0.0, 1.0] {
            f(&mut b, v);
        }
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        for v in [
            36.0, 24.0, 0.0, 0.0, 50.0, 0.0, 0.0, 1.0, 1.0, 0.03, 1700.0, 60.0,
        ] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[0, 0, 0, 0]);
        f(&mut b, 100.0);
        f(&mut b, -1.0);
        b.extend_from_slice(&mask.to_le_bytes());
        b.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    #[test]
    fn reads_the_fields() {
        let b = camera_bytes(0x65ff_ff17);
        assert_eq!(b.len(), 116);
        let c = Camera::parse(&b, false).unwrap();
        assert_eq!(c.game_object.path_id, 34);
        assert!(c.enabled);
        assert_eq!((c.near, c.far, c.field_of_view), (0.03, 1700.0, 60.0));
        assert!(!c.orthographic);
        assert_eq!(c.depth, -1.0);
        assert_eq!(c.culling_mask, 0x65ff_ff17);
        assert!(c.draws_layer(0));
        assert!(c.draws_layer(8));
        assert!(!c.draws_layer(27));
        assert!(!c.draws_layer(5));
        assert!(!c.draws_layer(40));
    }

    #[test]
    fn truncated_cameras_are_errors() {
        let b = camera_bytes(1);
        for len in [0, 11, 50, 105] {
            assert!(Camera::parse(&b[..len], false).is_err(), "length {len}");
        }
    }
}
