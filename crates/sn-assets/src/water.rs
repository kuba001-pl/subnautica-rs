//! The water settings of every biome, from the main scene's
//! `WaterBiomeManager`. See `docs/formats/water.md`.

use sn_unity::{WaterBiomeManager, WaterscapeVolume};

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
