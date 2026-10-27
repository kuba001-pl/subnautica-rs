//! Runtime pod modules used by crafting. No generated game data is stored.
use crate::{Assets, Result, ScriptAssemblies};
use sn_install::GameData;

/// The installed pod fabricator, including its actual hand colliders.
pub struct PodFabricator {
    pub at: sn_world::Transform,
    pub item_spawn: sn_world::Transform,
    pub spawn_node: usize,
    pub beam_nodes: [usize; 2],
    pub beams: [sn_world::Transform; 2],
    pub close_distance: f32,
    pub hand_text: String,
    pub hand_world: sn_sim::collide::World,
    /// Filled by the caller with [`fabricator_ghosts`] when it draws them.
    pub ghosts: std::collections::BTreeMap<String, Option<Ghost>>,
}

/// Where `CrafterGhostModel.SetupGhostModelAsync` puts an output's ghost:
/// the node it copies, and its placement in `itemSpawnPoint`'s space after
/// the copied subtree is detached and its root transform reset.
#[derive(Clone, Debug)]
pub struct Ghost {
    pub index: usize,
    pub node: String,
    /// `Constructable.model` (true) or the `VFXFabricating` node.
    pub constructable: bool,
    /// The node is active in the prefab (the game activates its copy).
    pub active: bool,
    pub root: sn_world::Transform,
}

/// [`Ghost`]s of `classes` (prefab class ids). `None`: the prefab has
/// neither a `Constructable.model` nor a `VFXFabricating`, and the game
/// shows no ghost. Classes absent from the prefab database are skipped
/// (the game's loot cube: not read).
pub fn fabricator_ghosts(
    game: &GameData,
    assets: &Assets,
    classes: &[String],
) -> Result<std::collections::BTreeMap<String, Option<Ghost>>> {
    use sn_unity::Value;
    let catalog = assets.catalog()?;
    let paths = game.read_prefab_database().map_err(|e| e.to_string())?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let read = |object: &crate::ObjectRef, class: &str| -> Result<sn_unity::Fields> {
        let layout = layouts.script("Assembly-CSharp", "", class)?;
        let (_, bytes) = object.data()?;
        sn_unity::read_script(bytes, object.file.file().big_endian, &layout)
            .map(|(_, f)| f)
            .map_err(|e| e.to_string())
    };
    let num = |f: &sn_unity::Fields, name: &str| -> Result<f32> {
        f.get(name)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .ok_or_else(|| format!("VFXFabricating.{name} missing"))
    };
    let vec3 = |f: &sn_unity::Fields, name: &str| -> Result<[f32; 3]> {
        let v = f
            .get(name)
            .and_then(Value::as_struct)
            .ok_or_else(|| format!("VFXFabricating.{name} missing"))?;
        Ok([num(v, "x")?, num(v, "y")?, num(v, "z")?])
    };
    let mut out = std::collections::BTreeMap::new();
    for class in classes {
        let Some(path) = paths.get(class) else {
            continue;
        };
        let prefab = assets.prefab(&catalog, path)?;
        // (node index, its transform in itemSpawnPoint, constructable)
        let mut model = None;
        let mut vfx = None;
        for (i, node) in prefab.nodes.iter().enumerate() {
            for object in assets.node_components(node)? {
                match assets.script_class(&object).as_deref() {
                    Some("Constructable") if i == 0 => {
                        let f = read(&object, "Constructable")?;
                        if let Some(p) = f.get("model").and_then(Value::as_pptr)
                            && let Some(go) = assets.resolve(&object.file, p)?
                            && let Some(n) = prefab.node_of(&go)
                        {
                            model = Some((n, sn_world::Transform::default(), true));
                        }
                    }
                    // GetComponentInChildren: the first in depth order.
                    Some("VFXFabricating") if vfx.is_none() => {
                        let f = read(&object, "VFXFabricating")?;
                        let s = num(&f, "scaleFactor")?;
                        let at = sn_world::Transform {
                            position: vec3(&f, "posOffset")?,
                            rotation: sn_ui::compass::euler(vec3(&f, "eulerOffset")?),
                            scale: [s; 3],
                        };
                        vfx = Some((i, at, false));
                    }
                    _ => {}
                }
            }
        }
        let ghost = match model.or(vfx) {
            Some((n, at, constructable)) => {
                let node = &prefab.nodes[n];
                Some(Ghost {
                    index: n,
                    node: node.name.clone(),
                    constructable,
                    active: node.active,
                    root: at,
                })
            }
            None => None,
        };
        out.insert(class.clone(), ghost);
    }
    Ok(out)
}

pub fn pod_fabricator(game: &GameData, assets: &Assets, point: [f32; 3]) -> Result<PodFabricator> {
    let mut scene = assets.scene("escapepod")?;
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(assets, point)?;
    scene.follow_targets(assets)?;
    let catalog = assets.catalog()?;
    let assemblies = ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let read = |object: &crate::ObjectRef, class: &str| -> Result<sn_unity::Fields> {
        let layout = layouts.script("Assembly-CSharp", "", class)?;
        let (_, bytes) = object.data()?;
        sn_unity::read_script(bytes, object.file.file().big_endian, &layout)
            .map(|(_, f)| f)
            .map_err(|e| e.to_string())
    };
    let settings = crate::physics_settings(assets)?;
    let player = crate::player_data(assets)?;
    let rules = crate::LayerRules::new(&settings, player.player_layer);
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
            for object in assets.node_components(node)? {
                if assets.script_class(&object).as_deref() != Some("Fabricator") {
                    continue;
                }
                let fields = read(&object, "Fabricator")?;
                let root_at = spawn.placement(&prefab.nodes[0].local);
                let at = root_at.then(&node.in_prefab);
                let close_distance = fields
                    .get("closeDistance")
                    .and_then(|v| v.as_f64())
                    .ok_or("Fabricator.closeDistance missing")?
                    as f32;
                let hand_text = fields
                    .get("handOverText")
                    .and_then(|v| v.as_str())
                    .ok_or("Fabricator.handOverText missing")?
                    .to_owned();
                let ghost_ref = fields
                    .get("ghost")
                    .and_then(|v| v.as_pptr())
                    .ok_or("Fabricator.ghost missing")?;
                let ghost = assets
                    .resolve(&object.file, ghost_ref)?
                    .ok_or("Fabricator.ghost null")?;
                let gf = read(&ghost, "CrafterGhostModel")?;
                let spawn_ref = gf
                    .get("itemSpawnPoint")
                    .and_then(|v| v.as_pptr())
                    .ok_or("CrafterGhostModel.itemSpawnPoint missing")?;
                let transform = assets
                    .resolve(&ghost.file, spawn_ref)?
                    .ok_or("itemSpawnPoint null")?;
                let t = sn_unity::TransformNode::parse(
                    transform.data()?.1,
                    transform.file.file().big_endian,
                )
                .map_err(|e| e.to_string())?;
                let go = assets
                    .resolve(&transform.file, t.game_object)?
                    .ok_or("itemSpawnPoint GameObject null")?;
                let index = prefab
                    .node_of(&go)
                    .ok_or("itemSpawnPoint outside fabricator prefab")?;
                let item_spawn = root_at.then(&prefab.nodes[index].in_prefab);
                let mut beams = [sn_world::Transform::default(); 2];
                let mut beam_nodes = [0; 2];
                for ((out, index), name) in beams
                    .iter_mut()
                    .zip(&mut beam_nodes)
                    .zip(["leftBeam", "rightBeam"])
                {
                    let p = fields
                        .get(name)
                        .and_then(|v| v.as_pptr())
                        .ok_or("beam reference missing")?;
                    let go = assets
                        .resolve(&object.file, p)?
                        .ok_or("beam reference null")?;
                    let i = prefab
                        .node_of(&go)
                        .ok_or("beam outside fabricator prefab")?;
                    *out = root_at.then(&prefab.nodes[i].in_prefab);
                    *index = i;
                }
                let (colliders, counts) =
                    assets.prefab_colliders(&prefab, &mut crate::ColliderMeshes::new())?;
                if counts.layout_errors != 0 || counts.mesh_errors != 0 {
                    return Err(format!("fabricator collider errors: {counts:?}"));
                }
                let placed: Vec<_> = colliders
                    .iter()
                    .map(|c| crate::PlacedCollider {
                        collider: c.world(&root_at),
                        layer: c.layer,
                        trigger: c.trigger,
                    })
                    .collect();
                let mut hand_world = sn_sim::collide::World::new();
                for (i, body) in crate::bodies(&placed, &rules).into_iter().enumerate() {
                    hand_world.insert(i as u64, body);
                }
                if !close_distance.is_finite() || close_distance <= 0.0 {
                    return Err("invalid fabricator closeDistance".into());
                }
                return Ok(PodFabricator {
                    at,
                    item_spawn,
                    spawn_node: index,
                    beam_nodes,
                    beams,
                    close_distance,
                    hand_text,
                    hand_world,
                    ghosts: Default::default(),
                });
            }
        }
    }
    Err("no spawned pod Fabricator".into())
}

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
