//! `QualitySettings` (class 47, in `globalgamemanagers`): the project's
//! quality levels (Unity 2019.4 layout; see `docs/formats/unity.md`
//! § Quality settings).

use crate::reader::Reader;
use crate::{ErrorKind, Result};

/// One quality level; the fields we use.
#[derive(Clone, Debug, PartialEq)]
pub struct QualityLevel {
    pub name: String,
    pub shadow_distance: f32,
    pub shadow_cascades: i32,
    /// Scales every `LODGroup`'s screen height (above 1: detailed levels
    /// stay longer).
    pub lod_bias: f32,
    /// The most detailed LOD level drawn (0: all).
    pub maximum_lod_level: i32,
}

/// `QualitySettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct QualitySettings {
    /// The level the project starts with (the player's saved choice
    /// replaces it).
    pub current: i32,
    pub levels: Vec<QualityLevel>,
}

impl QualitySettings {
    /// Parses the whole object; leftover bytes are an error.
    pub fn parse(data: &[u8], big_endian: bool) -> Result<QualitySettings> {
        let mut r = Reader::new(data, big_endian);
        let current = r.i32()?;
        // A level is at least 132 bytes besides its name.
        let n = r.count(136)?;
        let mut levels = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.aligned_string()?;
            // pixel light count, shadows, shadow resolution, projection
            r.bytes(16)?;
            let shadow_cascades = r.i32()?;
            let shadow_distance = r.f32()?;
            // near plane offset, cascade 2 split, cascade 4 split (3)
            r.bytes(20)?;
            // shadowmask mode, skin weights, texture quality, anisotropic
            // textures, anti-aliasing
            r.bytes(20)?;
            // soft particles, soft vegetation, realtime reflection probes,
            // billboards face camera position
            r.bytes(4)?;
            r.align(4)?;
            r.i32()?; // vsync count
            let lod_bias = r.f32()?;
            let maximum_lod_level = r.i32()?;
            // streaming mipmaps active, add all cameras
            r.bytes(2)?;
            r.align(4)?;
            // memory budget, renderers per frame, max level reduction, max
            // file IO requests, particle raycast budget, async upload time
            // slice, buffer size
            r.bytes(28)?;
            r.u8()?; // async upload persistent buffer
            r.align(4)?;
            r.f32()?; // resolution scaling fixed DPI factor
            r.bytes(12)?; // custom render pipeline (PPtr)
            levels.push(QualityLevel {
                name,
                shadow_distance,
                shadow_cascades,
                lod_bias,
                maximum_lod_level,
            });
        }
        r.i32()?; // stripped maximum LOD level
        if r.pos() != data.len() {
            return Err(r.error(ErrorKind::Invalid(format!(
                "{} bytes left after QualitySettings",
                data.len() - r.pos()
            ))));
        }
        Ok(QualitySettings { current, levels })
    }

    /// The level named `name` (e.g. `High`).
    pub fn level(&self, name: &str) -> Option<&QualityLevel> {
        self.levels.iter().find(|l| l.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(name: &str, bias: f32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend((name.len() as i32).to_le_bytes());
        b.extend(name.as_bytes());
        while b.len() % 4 != 0 {
            b.push(0);
        }
        for v in [2i32, 2, 2, 1, 4] {
            b.extend(v.to_le_bytes());
        }
        for v in [50.0f32, 2.0, 0.33, 0.07, 0.2, 0.47] {
            b.extend(v.to_le_bytes());
        }
        for v in [0i32, 4, 0, 1, 0] {
            b.extend(v.to_le_bytes());
        }
        b.extend([1, 1, 0, 0]);
        b.extend(0i32.to_le_bytes());
        b.extend(bias.to_le_bytes());
        b.extend(1i32.to_le_bytes());
        b.extend([0, 1, 0, 0]);
        b.extend(3072.0f32.to_le_bytes());
        for v in [512i32, 2, 1024, 256, 2, 4] {
            b.extend(v.to_le_bytes());
        }
        b.extend([1, 0, 0, 0]);
        b.extend(1.0f32.to_le_bytes());
        b.extend([0; 12]);
        b
    }

    fn settings() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(2i32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(level("Low", 0.66));
        b.extend(level("High", 10.0));
        b.extend(0i32.to_le_bytes());
        b
    }

    #[test]
    fn reads_the_levels_to_the_last_byte() {
        let q = QualitySettings::parse(&settings(), false).unwrap();
        assert_eq!(q.current, 2);
        assert_eq!(q.levels.len(), 2);
        let high = q.level("High").unwrap();
        assert_eq!(high.lod_bias, 10.0);
        assert_eq!(high.maximum_lod_level, 1);
        assert_eq!(high.shadow_distance, 50.0);
        assert_eq!(high.shadow_cascades, 4);
        assert_eq!(q.level("Low").unwrap().lod_bias, 0.66);
        assert!(q.level("Ultra").is_none());
    }

    #[test]
    fn truncated_or_padded_bytes_are_errors() {
        let b = settings();
        for len in 0..b.len() {
            assert!(QualitySettings::parse(&b[..len], false).is_err());
        }
        let mut longer = b.clone();
        longer.push(0);
        assert!(QualitySettings::parse(&longer, false).is_err());
    }
}
