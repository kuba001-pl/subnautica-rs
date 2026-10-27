//! Runtime pod modules used by crafting. No generated game data is stored.
use crate::{Assets, Result, ScriptAssemblies};
use sn_install::GameData;

/// Initial power of the three instantiated pod cells. Scene.behaviours only
/// sees the scene file, whereas these sources live on addressable spawns.
pub fn pod_crafting_power(game: &GameData, assets: &Assets) -> Result<f32> {
    let scene = assets.scene("escapepod")?;
    let catalog = assets.catalog()?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let layout = layouts.script("Assembly-CSharp", "", "PowerSource")?;
    let mut total = 0.0;
    let mut count = 0;
    for spawn in scene.spawns(assets)? {
        if spawn.spawner.deactivate_on_spawn {
            continue;
        }
        let prefab = match (&spawn.spawner.prefab, &spawn.object) {
            (sn_unity::SpawnPrefab::Address(guid), _) => assets.prefab(&catalog, guid)?,
            (_, Some(object)) => assets.hierarchy(&spawn.name, object)?,
            _ => continue,
        };
        for node in &prefab.nodes {
            if !node.active {
                continue;
            }
            for object in assets.node_components(node)? {
                if assets.script_class(&object).as_deref() != Some("PowerSource") {
                    continue;
                }
                let (_, bytes) = object.data()?;
                let (_, f) = sn_unity::read_script(bytes, object.file.file().big_endian, &layout)
                    .map_err(|e| e.to_string())?;
                let power = f
                    .get("power")
                    .and_then(|v| v.as_f64())
                    .ok_or("PowerSource.power missing")? as f32;
                if !power.is_finite() || power < 0.0 {
                    return Err("invalid pod power".into());
                }
                total += power;
                count += 1;
            }
        }
    }
    if count == 0 || !total.is_finite() {
        return Err("no pod power sources".into());
    }
    Ok(total)
}
