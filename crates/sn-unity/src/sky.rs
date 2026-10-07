//! The game's sky system in the main scene: `uSkyLight` and the start of
//! `uSkyManager` (MonoBehaviours), and Unity's `Gradient`. Field layouts
//! were generated once from the game's assembly (UnityPy's type-tree
//! generator, dev machine only); see `docs/formats/water.md` § Sky.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// Unity's `Gradient` (2019): 8 colour keys (RGBA), 8 colour-key times and 8
/// alpha-key times (u16, 0..65535 → 0..1), mode, key counts.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub keys: [[f32; 4]; 8],
    pub color_times: [u16; 8],
    pub alpha_times: [u16; 8],
    /// 0 = blend, 1 = fixed (steps).
    pub mode: i32,
    pub color_keys: u8,
    pub alpha_keys: u8,
}

impl Gradient {
    fn read(r: &mut Reader) -> Result<Gradient> {
        let mut keys = [[0.0; 4]; 8];
        for key in &mut keys {
            *key = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        }
        let mut color_times = [0u16; 8];
        for t in &mut color_times {
            *t = r.u16()?;
        }
        let mut alpha_times = [0u16; 8];
        for t in &mut alpha_times {
            *t = r.u16()?;
        }
        let mode = r.i32()?;
        let color_keys = r.u8()?;
        let alpha_keys = r.u8()?;
        r.align(4)?;
        Ok(Gradient {
            keys,
            color_times,
            alpha_times,
            mode,
            color_keys,
            alpha_keys,
        })
    }

    /// One channel group between keys at time `t` (0..1): clamped at both
    /// ends, linear between keys (blend) or the next key's value (fixed).
    fn sample(
        &self,
        t: f32,
        times: &[u16; 8],
        count: u8,
        value: impl Fn(usize) -> Vec<f32>,
    ) -> Vec<f32> {
        let n = usize::from(count.clamp(1, 8));
        let time = |i: usize| f32::from(times[i]) / 65535.0;
        if t <= time(0) {
            return value(0);
        }
        for i in 1..n {
            if t <= time(i) {
                if self.mode == 1 {
                    return value(i);
                }
                let (a, b) = (time(i - 1), time(i));
                let f = if b > a { (t - a) / (b - a) } else { 1.0 };
                let (va, vb) = (value(i - 1), value(i));
                return va.iter().zip(&vb).map(|(x, y)| x + (y - x) * f).collect();
            }
        }
        value(n - 1)
    }

    /// RGBA at `t` (0..1), in the stored (sRGB) values.
    pub fn evaluate(&self, t: f32) -> [f32; 4] {
        let rgb = self.sample(t, &self.color_times, self.color_keys, |i| {
            self.keys[i][..3].to_vec()
        });
        let a = self.sample(t, &self.alpha_times, self.alpha_keys, |i| {
            vec![self.keys[i][3]]
        });
        [rgb[0], rgb[1], rgb[2], a[0]]
    }
}

/// `uSkyLight`: the sun's colour over the day and the ambient colours.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyLight {
    pub sun_intensity: f32,
    pub light_color: Gradient,
    pub moon_intensity: f32,
    pub sky_color: Gradient,
    pub equator_color: Gradient,
    pub ground_color: Gradient,
    pub ambient_light: f32,
}

impl SkyLight {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SkyLight> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        Ok(SkyLight {
            sun_intensity: r.f32()?,
            light_color: Gradient::read(&mut r)?,
            moon_intensity: r.f32()?,
            sky_color: Gradient::read(&mut r)?,
            equator_color: Gradient::read(&mut r)?,
            ground_color: Gradient::read(&mut r)?,
            ambient_light: r.f32()?,
        })
    }
}

/// The fields of `uSkyManager` up to the mean sky colour (the rest is the
/// sky dome's look: night sky, moon, …).
#[derive(Clone, Debug, PartialEq)]
pub struct SkyManager {
    /// Hours (editor value; the game drives it from its day/night cycle).
    pub timeline: f32,
    pub use_time_of_day: bool,
    /// Degrees: the sun path's heading.
    pub sun_direction: f32,
    /// Degrees from the zenith at sunrise/sunset.
    pub sun_max_angle: f32,
    pub north_pole_offset: f32,
    pub exposure: f32,
    pub rayleigh_scattering: f32,
    pub mie_scattering: f32,
    pub sun_anisotropy: f32,
    pub sun_size: f32,
    /// Nanometres.
    pub wavelengths: [f32; 3],
    pub sky_tint: [f32; 4],
    pub ground_color: [f32; 4],
    pub sky_fog_density: f32,
    pub sky_fog_color: Gradient,
    /// Average sky colour over the day (the water surface's reflection far
    /// away and without a sky map).
    pub mean_sky_color: Gradient,
}

impl SkyManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SkyManager> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let _sky_update = r.bool_aligned()?;
        let timeline = r.f32()?;
        let use_time_of_day = r.bool_aligned()?;
        let sun_direction = r.f32()?;
        let sun_max_angle = r.f32()?;
        let north_pole_offset = r.f32()?;
        let exposure = r.f32()?;
        let rayleigh_scattering = r.f32()?;
        let mie_scattering = r.f32()?;
        let sun_anisotropy = r.f32()?;
        let sun_size = r.f32()?;
        PPtr::read(&mut r)?; // sun burst texture
        let wavelengths = [r.f32()?, r.f32()?, r.f32()?];
        let sky_tint = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let ground_color = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        PPtr::read(&mut r)?; // sun light
        let sky_fog_density = r.f32()?;
        let sky_fog_color = Gradient::read(&mut r)?;
        // Planet: radius, texture, normal map, zenith, distance, rim colour,
        // ambient light, orbit speed, light wrap, inner and outer corona.
        r.f32()?;
        PPtr::read(&mut r)?;
        PPtr::read(&mut r)?;
        r.bytes(2 * 4 + 2 * 16 + 2 * 4 + 2 * 16)?;
        // Clouds: texture, then 8 floats (rotate speed … scattering exponent).
        PPtr::read(&mut r)?;
        r.bytes(8 * 4)?;
        let mean_sky_color = Gradient::read(&mut r)?;
        Ok(SkyManager {
            timeline,
            use_time_of_day,
            sun_direction,
            sun_max_angle,
            north_pole_offset,
            exposure,
            rayleigh_scattering,
            mie_scattering,
            sun_anisotropy,
            sun_size,
            wavelengths,
            sky_tint,
            ground_color,
            sky_fog_density,
            sky_fog_color,
            mean_sky_color,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(mode: i32) -> Gradient {
        let mut keys = [[0.0; 4]; 8];
        keys[0] = [0.0, 0.0, 0.0, 1.0];
        keys[1] = [1.0, 0.5, 0.0, 0.0];
        let mut color_times = [0; 8];
        color_times[0] = 16384; // ≈ 0.25
        color_times[1] = 49151; // ≈ 0.75
        let mut alpha_times = [0; 8];
        alpha_times[1] = 65535;
        Gradient {
            keys,
            color_times,
            alpha_times,
            mode,
            color_keys: 2,
            alpha_keys: 2,
        }
    }

    #[test]
    fn gradients_clamp_and_blend() {
        let g = gradient(0);
        assert_eq!(g.evaluate(0.0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(g.evaluate(1.0)[..3], [1.0, 0.5, 0.0]);
        let mid = g.evaluate(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-3 && (mid[1] - 0.25).abs() < 1e-3);
        // Alpha keys have their own times: 1 at 0, 0 at 1.
        assert!((mid[3] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn fixed_gradients_step() {
        let g = gradient(1);
        assert_eq!(g.evaluate(0.5)[..3], [1.0, 0.5, 0.0]);
        assert_eq!(g.evaluate(0.1)[..3], [0.0, 0.0, 0.0]);
    }
}
