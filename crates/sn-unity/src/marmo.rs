//! Marmoset Skyshop's image-based lighting as the game uses it: `mset.Sky`
//! (a sky's exposures, spherical-harmonics ambient and specular cube) and the
//! game's `SkyApplier` (which renderers take the sky of the biome they stand
//! in). Field layouts from the game's assembly (decompiled once on the dev
//! machine); bools are each padded to 4 bytes. See
//! `docs/formats/lighting.md` § Objects.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// `SHEncoding.sEquationConstants`: the factor of each of the 9 coefficients
/// (bands 0–2) when copied into the shader's `_SH0…_SH8`.
pub const SH_CONSTANTS: [f32; 9] = [
    0.282_094_78,
    0.488_602_5,
    0.488_602_5,
    0.488_602_5,
    1.092_548_5,
    1.092_548_5,
    0.315_391_57,
    1.092_548_5,
    0.546_274_24,
];

/// `mset.Sky`.
#[derive(Clone, Debug, PartialEq)]
pub struct MarmoSky {
    pub specular_cube: PPtr,
    /// `_AffectedByDayNightCycle`: outdoor skies (1) leave the ambient to the
    /// deferred light pass; others (0) add their own and mark the surface
    /// unlit for that pass.
    pub affected_by_day_night: bool,
    pub outdoors: bool,
    pub master_intensity: f32,
    pub sky_intensity: f32,
    pub spec_intensity: f32,
    pub diff_intensity: f32,
    pub cam_exposure: f32,
    /// The 27 SH coefficients (9 × RGB), before `SH_CONSTANTS`.
    pub sh: [f32; 27],
}

impl MarmoSky {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MarmoSky> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let bool4 = |r: &mut Reader| -> Result<bool> {
            let b = r.u8()? != 0;
            r.align(4)?;
            Ok(b)
        };
        let specular_cube = PPtr::read(&mut r)?;
        let _is_probe = bool4(&mut r)?;
        // dimensions: Bounds (centre, extent).
        r.bytes(24)?;
        let affected_by_day_night = bool4(&mut r)?;
        let outdoors = bool4(&mut r)?;
        let master_intensity = r.f32()?;
        let sky_intensity = r.f32()?;
        let spec_intensity = r.f32()?;
        let diff_intensity = r.f32()?;
        let cam_exposure = r.f32()?;
        let _spec_intensity_lm = r.f32()?;
        let _diff_intensity_lm = r.f32()?;
        // hdrSky, hdrSpec, linearSpace, autoDetectColorSpace, hasDimensions.
        for _ in 0..5 {
            bool4(&mut r)?;
        }
        let at = r.pos();
        let n = r.count(4)?;
        if n != 27 {
            return Err(r.error(crate::ErrorKind::Invalid(format!(
                "SH at {at}: {n} coefficients, expected 27"
            ))));
        }
        let mut sh = [0.0; 27];
        for c in &mut sh {
            *c = r.f32()?;
        }
        Ok(MarmoSky {
            specular_cube,
            affected_by_day_night,
            outdoors,
            master_intensity,
            sky_intensity,
            spec_intensity,
            diff_intensity,
            cam_exposure,
            sh,
        })
    }

    /// `_ExposureIBL` (`Sky.ComputeExposureVector`): diffuse, specular, sky
    /// (× camera exposure), camera exposure.
    pub fn exposure(&self) -> [f32; 4] {
        let m = self.master_intensity;
        [
            m * self.diff_intensity,
            m * self.spec_intensity,
            m * self.sky_intensity * self.cam_exposure,
            self.cam_exposure,
        ]
    }

    /// `_SH0…_SH8` (`SHEncoding.copyToBuffer`): RGB per coefficient.
    pub fn sh_buffer(&self) -> [[f32; 3]; 9] {
        let mut out = [[0.0; 3]; 9];
        for (i, o) in out.iter_mut().enumerate() {
            for (c, v) in o.iter_mut().enumerate() {
                *v = self.sh[i * 3 + c] * SH_CONSTANTS[i];
            }
        }
        out
    }
}

/// The game's `Skies` enum: which sky a `SkyApplier` anchors to.
pub const SKIES_AUTO: i32 = 0;

/// The game's `SkyApplier`: applies a sky to its renderers (by default the
/// sky of the biome at the object's position).
#[derive(Clone, Debug, PartialEq)]
pub struct SkyApplier {
    /// `Skies`: 0 Auto, 1 Custom, 2 SafeShallow, 3 BaseInterior, 4
    /// BaseGlass, 5 ExplorableWreck.
    pub anchor_sky: i32,
    pub custom_sky_prefab: PPtr,
    pub dynamic: bool,
    pub emissive_from_power: bool,
    pub renderers: Vec<PPtr>,
}

impl SkyApplier {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SkyApplier> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let anchor_sky = r.i32()?;
        let custom_sky_prefab = PPtr::read(&mut r)?;
        let dynamic = r.u8()? != 0;
        r.align(4)?;
        let emissive_from_power = r.u8()? != 0;
        r.align(4)?;
        let n = r.count(12)?;
        let mut renderers = Vec::with_capacity(n);
        for _ in 0..n {
            renderers.push(PPtr::read(&mut r)?);
        }
        Ok(SkyApplier {
            anchor_sky,
            custom_sky_prefab,
            dynamic,
            emissive_from_power,
            renderers,
        })
    }
}

/// `MarmoSkies`: the prefabs of the skies the game uses by name (the first is
/// the safe shallows sky, also the global sky outside the lifepod).
#[derive(Clone, Debug, PartialEq)]
pub struct MarmoSkiesPrefabs {
    pub safe_shallows: PPtr,
    pub base_interior: PPtr,
    pub base_glass: PPtr,
    pub explorable_wreck: PPtr,
}

impl MarmoSkiesPrefabs {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MarmoSkiesPrefabs> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        Ok(MarmoSkiesPrefabs {
            safe_shallows: PPtr::read(&mut r)?,
            base_interior: PPtr::read(&mut r)?,
            base_glass: PPtr::read(&mut r)?,
            explorable_wreck: PPtr::read(&mut r)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MonoBehaviour header (GameObject, enabled, script, empty name).
    fn header() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&5i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&7i64.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b
    }

    fn pptr(b: &mut Vec<u8>, file: i32, id: i64) {
        b.extend_from_slice(&file.to_le_bytes());
        b.extend_from_slice(&id.to_le_bytes());
    }

    fn f(b: &mut Vec<u8>, v: f32) {
        b.extend_from_slice(&v.to_le_bytes());
    }

    fn sky_bytes(sh_count: i32) -> Vec<u8> {
        let mut b = header();
        pptr(&mut b, 1, 99);
        b.extend_from_slice(&[0, 0, 0, 0]); // IsProbe
        for v in [0.0, 0.0, 0.0, 0.5, 0.5, 0.5] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0]);
        for v in [2.0, 0.5, 0.25, 4.0, 3.0, 0.2, 0.05] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[0; 20]);
        b.extend_from_slice(&sh_count.to_le_bytes());
        for i in 0..27 {
            f(&mut b, i as f32);
        }
        b
    }

    #[test]
    fn reads_a_sky() {
        let s = MarmoSky::parse(&sky_bytes(27), false).unwrap();
        assert_eq!(
            s.specular_cube,
            PPtr {
                file_id: 1,
                path_id: 99
            }
        );
        assert!(s.affected_by_day_night && s.outdoors);
        assert_eq!(s.exposure(), [8.0, 0.5, 3.0, 3.0]);
        let sh = s.sh_buffer();
        assert_eq!(sh[0][2], 2.0 * SH_CONSTANTS[0]);
        assert_eq!(sh[8][0], 24.0 * SH_CONSTANTS[8]);
    }

    #[test]
    fn rejects_bad_skies() {
        assert!(MarmoSky::parse(&sky_bytes(26), false).is_err());
        let b = sky_bytes(27);
        assert!(MarmoSky::parse(&b[..b.len() - 3], false).is_err());
    }

    #[test]
    fn reads_a_sky_applier() {
        let mut b = header();
        b.extend_from_slice(&0i32.to_le_bytes());
        pptr(&mut b, 0, 0);
        b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        b.extend_from_slice(&2i32.to_le_bytes());
        pptr(&mut b, 0, 11);
        pptr(&mut b, 0, 12);
        let a = SkyApplier::parse(&b, false).unwrap();
        assert_eq!(a.anchor_sky, SKIES_AUTO);
        assert!(!a.dynamic && a.emissive_from_power);
        assert_eq!(a.renderers.len(), 2);
        assert_eq!(a.renderers[1].path_id, 12);
        // A count larger than the data is an error, not a panic.
        let n = b.len() - 28;
        b[n..n + 4].copy_from_slice(&1000i32.to_le_bytes());
        assert!(SkyApplier::parse(&b, false).is_err());
    }
}
