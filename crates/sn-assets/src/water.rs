//! The water settings of every biome, from the main scene's
//! `WaterBiomeManager`. See `docs/formats/water.md`.

use sn_unity::{SkyLight, SkyManager, WaterBiomeManager, WaterscapeVolume};

use crate::terrain::{behaviours, main_scene};
use crate::{Assets, Result};

/// Reads the main scene's `WaterBiomeManager`.
pub fn water_biomes(assets: &Assets) -> Result<WaterBiomeManager> {
    let scene = main_scene(assets)?;
    let found = behaviours(assets, &scene, "WaterBiomeManager")?;
    let data = match found.as_slice() {
        [one] => *one,
        [] => return Err("no WaterBiomeManager in the main scene".into()),
        more => {
            return Err(format!(
                "{} WaterBiomeManagers in the main scene",
                more.len()
            ));
        }
    };
    WaterBiomeManager::parse(data, scene.file().big_endian)
        .map_err(|e| format!("WaterBiomeManager: {e}"))
}

/// Reads the main scene's `WaterscapeVolume` (global fog settings).
pub fn water_volume(assets: &Assets) -> Result<WaterscapeVolume> {
    let scene = main_scene(assets)?;
    let found = behaviours(assets, &scene, "WaterscapeVolume")?;
    let data = match found.as_slice() {
        [one] => *one,
        [] => return Err("no WaterscapeVolume in the main scene".into()),
        more => {
            return Err(format!(
                "{} WaterscapeVolumes in the main scene",
                more.len()
            ));
        }
    };
    WaterscapeVolume::parse(data, scene.file().big_endian)
        .map_err(|e| format!("WaterscapeVolume: {e}"))
}

/// The single MonoBehaviour of class `class_name` in the main scene.
fn single_behaviour<'f>(
    assets: &Assets,
    scene: &'f crate::FileRef,
    class_name: &str,
) -> Result<&'f [u8]> {
    match behaviours(assets, scene, class_name)?.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!("no {class_name} in the main scene")),
        more => Err(format!("{} {class_name}s in the main scene", more.len())),
    }
}

/// The main scene's sky system: `uSkyManager` (time, sun path, exposure,
/// sky fog) and `uSkyLight` (sun and ambient colours over the day).
pub fn sky(assets: &Assets) -> Result<(SkyManager, SkyLight)> {
    let scene = main_scene(assets)?;
    let be = scene.file().big_endian;
    let manager = SkyManager::parse(single_behaviour(assets, &scene, "uSkyManager")?, be)
        .map_err(|e| format!("uSkyManager: {e}"))?;
    let light = SkyLight::parse(single_behaviour(assets, &scene, "uSkyLight")?, be)
        .map_err(|e| format!("uSkyLight: {e}"))?;
    Ok((manager, light))
}
