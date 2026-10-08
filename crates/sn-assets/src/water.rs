//! The water settings of every biome, from the main scene's
//! `WaterBiomeManager`. See `docs/formats/water.md`.

use sn_unity::{SkyLight, SkyManager, WaterBiomeManager, WaterSurface, WaterscapeVolume};

use crate::terrain::{TerrainTexture, behaviours, load_texture, main_scene};
use crate::{Assets, Result};

/// Texture2D class id.
const TEXTURE_2D: i32 = 28;

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

/// What the water surface needs: the scene's `WaterSurface`, its baked wave
/// frames (in the game's order) and its foam textures.
pub struct WaterSurfaceData {
    pub surface: WaterSurface,
    /// `WaterFrame00…63`: displacement, RGBA8 (see `docs/formats/water.md`).
    pub frames: Vec<TerrainTexture>,
    pub foam: TerrainTexture,
    pub foam_mask: TerrainTexture,
}

/// Reads the main scene's `WaterSurface` and the textures it uses. The wave
/// frames are the Addressables entries labelled `WaterDisplacement`, as the
/// game loads them ("Medium" water quality, the default).
pub fn water_surface(assets: &Assets) -> Result<WaterSurfaceData> {
    let scene = main_scene(assets)?;
    let surface = WaterSurface::parse(
        single_behaviour(assets, &scene, "WaterSurface")?,
        scene.file().big_endian,
    )
    .map_err(|e| format!("WaterSurface: {e}"))?;
    let texture = |pptr, what: &str| -> Result<TerrainTexture> {
        let object = assets
            .resolve(&scene, pptr)?
            .ok_or_else(|| format!("WaterSurface: no {what}"))?;
        load_texture(assets, &object)
    };
    let foam = texture(surface.foam_texture, "foam texture")?;
    let foam_mask = texture(surface.foam_mask_texture, "foam mask texture")?;

    let catalog = assets.catalog()?;
    let mut frames = Vec::new();
    for location in catalog.locate("WaterDisplacement") {
        let object = assets
            .catalog_object(&location, TEXTURE_2D)?
            .ok_or_else(|| format!("{}: not found in its bundle", location.internal_id))?;
        frames.push(load_texture(assets, &object)?);
    }
    if frames.is_empty() {
        return Err("no WaterDisplacement frames in the catalog".into());
    }
    Ok(WaterSurfaceData {
        surface,
        frames,
        foam,
        foam_mask,
    })
}

/// Resource manager and text asset class ids.
const RESOURCE_MANAGER: i32 = 147;
const TEXT_ASSET: i32 = 49;

/// The objects `Resources.Load` finds under `path` (lower case).
fn resources(assets: &Assets, path: &str) -> Result<(crate::FileRef, Vec<sn_unity::PPtr>)> {
    let ggm = assets.standalone("globalgamemanagers")?;
    let info = ggm
        .objects()
        .iter()
        .find(|o| o.class_id == RESOURCE_MANAGER)
        .ok_or("globalgamemanagers: no ResourceManager")?;
    let (_, data) = ggm
        .object(info.path_id)
        .ok_or("globalgamemanagers: ResourceManager unreadable")?;
    let container = sn_unity::parse_resource_container(data, ggm.file().big_endian)
        .map_err(|e| format!("ResourceManager: {e}"))?;
    let found = container
        .iter()
        .filter(|(p, _)| p.eq_ignore_ascii_case(path))
        .map(|(_, pptr)| *pptr)
        .collect();
    Ok((ggm, found))
}

/// The texture `Resources.Load` finds under `path` (lower case), e.g. the
/// water caustics `data/watercaustics00`.
pub fn resource_texture(assets: &Assets, path: &str) -> Result<TerrainTexture> {
    let (file, found) = resources(assets, path)?;
    for pptr in found {
        if let Some(object) = assets.resolve(&file, pptr)?
            && object.data()?.0.class_id == TEXTURE_2D
        {
            return load_texture(assets, &object);
        }
    }
    Err(format!("no texture resource {path}"))
}

/// The bytes of the text asset `Resources.Load` finds under `path` (lower
/// case), e.g. uSky's star catalogue `starsdata`.
pub fn resource_bytes(assets: &Assets, path: &str) -> Result<Vec<u8>> {
    let (file, found) = resources(assets, path)?;
    for pptr in found {
        if let Some(object) = assets.resolve(&file, pptr)? {
            let (info, data) = object.data()?;
            if info.class_id == TEXT_ASSET {
                let (_, bytes) = sn_unity::parse_text_asset(data, object.file.file().big_endian)
                    .map_err(|e| format!("{path}: {e}"))?;
                return Ok(bytes);
            }
        }
    }
    Err(format!("no text resource {path}"))
}

/// The water caustics frames (`Data/WaterCaustics00…`, `count` of them; the
/// scene's `WaterSurface` says 64).
pub fn water_caustics(assets: &Assets, count: usize) -> Result<Vec<TerrainTexture>> {
    (0..count)
        .map(|i| resource_texture(assets, &format!("data/watercaustics{i:02}")))
        .collect()
}

/// The sky dome's textures, referenced by the main scene's `uSkyManager`.
pub struct SkyTextures {
    pub planet: TerrainTexture,
    pub sun_burst: TerrainTexture,
    pub moon: TerrainTexture,
    pub clouds: TerrainTexture,
}

/// Reads the textures of the sky dome (`uSkyManager`).
pub fn sky_textures(assets: &Assets, manager: &SkyManager) -> Result<SkyTextures> {
    let scene = main_scene(assets)?;
    let texture = |pptr, what: &str| -> Result<TerrainTexture> {
        let object = assets
            .resolve(&scene, pptr)?
            .ok_or_else(|| format!("uSkyManager: no {what}"))?;
        load_texture(assets, &object)
    };
    let d = &manager.dome;
    Ok(SkyTextures {
        planet: texture(d.planet_texture, "planet texture")?,
        sun_burst: texture(d.sun_burst_texture, "sun burst texture")?,
        moon: texture(d.moon_texture, "moon texture")?,
        clouds: texture(d.clouds_texture, "clouds texture")?,
    })
}
