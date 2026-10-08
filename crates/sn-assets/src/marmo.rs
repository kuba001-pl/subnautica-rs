//! The Marmoset skies of the world objects: one per biome (the
//! `WaterBiomeManager`'s sky prefabs) and the global one. See
//! `docs/formats/lighting.md` § Objects.

use sn_unity::{GameObject, MarmoSkiesPrefabs, MarmoSky, PPtr, TransformNode};

use crate::terrain::{behaviours, main_scene, script_class};
use crate::water::water_biomes;
use crate::{Assets, FileRef, Result};

const TRANSFORM: i32 = 4;
const MONO_BEHAVIOUR: i32 = 114;

/// A sky prefab's `mset.Sky`.
#[derive(Clone, Debug, PartialEq)]
pub struct BiomeSky {
    /// The prefab's name, e.g. `SkySafeShallows`.
    pub name: String,
    pub sky: MarmoSky,
    /// The prefab root's rotation (x, y, z, w): the sky's frame for its SH.
    pub rotation: [f32; 4],
}

/// Every sky the world's objects can use.
#[derive(Clone, Debug, Default)]
pub struct MarmoSkies {
    /// Distinct skies, in first-use order.
    pub skies: Vec<BiomeSky>,
    /// Per `WaterBiomeManager` biome, in its order: the biome's name and its
    /// sky (`None`: the biome has no sky prefab).
    pub biomes: Vec<(String, Option<usize>)>,
    /// The global sky outside the lifepod: `MarmoSkies.skySafeShallowsPrefab`.
    pub global: Option<usize>,
}

impl MarmoSkies {
    /// `WaterBiomeManager.GetBiomeEnvironment`: the sky of the named biome
    /// (names compared ignoring case); unknown biomes take the first biome's.
    pub fn for_biome(&self, biome: Option<&str>) -> Option<usize> {
        let found = biome.and_then(|b| {
            self.biomes
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(b))
        });
        match found {
            Some((_, sky)) => *sky,
            None => self.biomes.first().and_then(|(_, sky)| *sky),
        }
    }
}

/// Reads the biome skies and the global sky.
pub fn marmo_skies(assets: &Assets) -> Result<MarmoSkies> {
    let scene = main_scene(assets)?;
    let manager = water_biomes(assets)?;
    let mut out = MarmoSkies::default();
    let mut by_prefab: Vec<((std::path::PathBuf, String, i64), usize)> = Vec::new();
    let mut add = |pptr: PPtr, out: &mut MarmoSkies| -> Result<Option<usize>> {
        let Some(prefab) = assets.resolve(&scene, pptr)? else {
            return Ok(None);
        };
        if let Some((_, i)) = by_prefab.iter().find(|(k, _)| *k == prefab.key()) {
            return Ok(Some(*i));
        }
        let sky = prefab_sky(assets, &prefab.file, prefab.path_id)?;
        out.skies.push(sky);
        let i = out.skies.len() - 1;
        by_prefab.push((prefab.key(), i));
        Ok(Some(i))
    };
    for biome in &manager.biomes {
        let sky = add(biome.sky_prefab, &mut out).map_err(|e| format!("{}: {e}", biome.name))?;
        out.biomes.push((biome.name.clone(), sky));
    }
    if let [one] = behaviours(assets, &scene, "MarmoSkies")?.as_slice() {
        let prefabs = MarmoSkiesPrefabs::parse(one, scene.file().big_endian)
            .map_err(|e| format!("MarmoSkies: {e}"))?;
        out.global = add(prefabs.safe_shallows, &mut out)?;
    }
    Ok(out)
}

/// The `mset.Sky` on a sky prefab's root GameObject.
fn prefab_sky(assets: &Assets, file: &FileRef, path_id: i64) -> Result<BiomeSky> {
    let big_endian = file.file().big_endian;
    let (_, data) = file
        .object(path_id)
        .ok_or_else(|| format!("sky prefab {path_id} not in {}", file.name))?;
    let go = GameObject::parse(data, big_endian).map_err(|e| format!("GameObject: {e}"))?;
    let mut sky = None;
    let mut rotation = [0.0, 0.0, 0.0, 1.0];
    for component in &go.components {
        let Some(c) = assets.resolve(file, *component)? else {
            continue;
        };
        let (info, data) = c.data()?;
        match info.class_id {
            TRANSFORM => {
                if let Ok(t) = TransformNode::parse(data, big_endian) {
                    rotation = t.rotation;
                }
            }
            MONO_BEHAVIOUR if script_class(assets, &c).as_deref() == Some("Sky") => {
                sky = Some(
                    MarmoSky::parse(data, big_endian)
                        .map_err(|e| format!("{}: Sky: {e}", go.name))?,
                );
            }
            _ => {}
        }
    }
    let sky = sky.ok_or_else(|| format!("{}: no Sky component", go.name))?;
    Ok(BiomeSky {
        name: go.name,
        sky,
        rotation,
    })
}
