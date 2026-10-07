//! `WaterBiomeManager` (a MonoBehaviour in the main scene): the water
//! settings of every biome. Field layout from the game's class declaration;
//! see `docs/formats/water.md`.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// `WaterscapeVolume.Settings`: how light travels through a biome's water.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterSettings {
    /// Attenuation of light per colour channel (the game's tooltip: 1/cm).
    pub absorption: [f32; 3],
    pub scattering: f32,
    /// RGBA, as stored (sRGB).
    pub scattering_color: [f32; 4],
    pub murkiness: f32,
    /// RGBA, as stored (sRGB).
    pub emissive: [f32; 4],
    pub emissive_scale: f32,
    pub start_distance: f32,
    pub sunlight_scale: f32,
    pub ambient_scale: f32,
    /// Celsius.
    pub temperature: f32,
}

impl WaterSettings {
    fn read(r: &mut Reader) -> Result<WaterSettings> {
        let v3 = |r: &mut Reader| -> Result<[f32; 3]> { Ok([r.f32()?, r.f32()?, r.f32()?]) };
        let v4 =
            |r: &mut Reader| -> Result<[f32; 4]> { Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]) };
        Ok(WaterSettings {
            absorption: v3(r)?,
            scattering: r.f32()?,
            scattering_color: v4(r)?,
            murkiness: r.f32()?,
            emissive: v4(r)?,
            emissive_scale: r.f32()?,
            start_distance: r.f32()?,
            sunlight_scale: r.f32()?,
            ambient_scale: r.f32()?,
            temperature: r.f32()?,
        })
    }
}

/// One entry of `WaterBiomeManager.biomeSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct BiomeWater {
    pub name: String,
    pub settings: WaterSettings,
    pub sky_prefab: PPtr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaterBiomeManager {
    pub header: MonoBehaviourHeader,
    pub biomes: Vec<BiomeWater>,
    /// Size of the settings volume around the camera (cells per side), its
    /// upsampled size, and its half-size in metres.
    pub texture_size: i32,
    pub upsampled_size: i32,
    pub region_bounds: f32,
    pub enable_blur: bool,
}

impl WaterBiomeManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<WaterBiomeManager> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        // blur, unwrap, lookup, debug, atmosphere-volume and resize shaders,
        // then the LargeWorld.
        for _ in 0..7 {
            PPtr::read(&mut r)?;
        }
        let n = r.count(84)?;
        let mut biomes = Vec::with_capacity(n);
        for _ in 0..n {
            biomes.push(BiomeWater {
                name: r.aligned_string()?,
                settings: WaterSettings::read(&mut r)?,
                sky_prefab: PPtr::read(&mut r)?,
            });
        }
        r.align(4)?;
        let texture_size = r.i32()?;
        let upsampled_size = r.i32()?;
        let region_bounds = r.f32()?;
        let enable_blur = r.bool_aligned()?;
        Ok(WaterBiomeManager {
            header,
            biomes,
            texture_size,
            upsampled_size,
            region_bounds,
            enable_blur,
        })
    }
}

/// `WaterscapeVolume` (a MonoBehaviour in the main scene): global settings
/// of the underwater fog.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterscapeVolume {
    pub fog_shader: PPtr,
    pub water_offset: f32,
    /// Share of sunlight that gets below the surface.
    pub water_transmission: f32,
    pub emission_ambient_scale: f32,
    pub above_water_start_distance: f32,
    /// Henyey–Greenstein asymmetry, −1..0.
    pub scattering_phase: f32,
    pub sun_attenuation: f32,
    pub sun_light_amount: f32,
    pub color_cast_distance_factor: f32,
    pub color_cast_depth_factor: f32,
    pub caustics_scale: f32,
    pub caustics_amount: f32,
    pub above_water_density_scale: f32,
    pub above_water_min_height: f32,
    pub above_water_max_height: f32,
}

impl WaterscapeVolume {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<WaterscapeVolume> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let fog_shader = PPtr::read(&mut r)?;
        // sky, biome manager, surface, water plane
        for _ in 0..4 {
            PPtr::read(&mut r)?;
        }
        Ok(WaterscapeVolume {
            fog_shader,
            water_offset: r.f32()?,
            water_transmission: r.f32()?,
            emission_ambient_scale: r.f32()?,
            above_water_start_distance: r.f32()?,
            scattering_phase: r.f32()?,
            sun_attenuation: r.f32()?,
            sun_light_amount: r.f32()?,
            color_cast_distance_factor: r.f32()?,
            color_cast_depth_factor: r.f32()?,
            caustics_scale: r.f32()?,
            caustics_amount: r.f32()?,
            above_water_density_scale: r.f32()?,
            above_water_min_height: r.f32()?,
            above_water_max_height: r.f32()?,
        })
    }
}
