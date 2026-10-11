//! The item rules every machine replays item events with (M9d3d,
//! `docs/DESIGN.md` "M9d3 plan"), built from M9d1's readers: what each
//! prefab the world can hold is as an item, the outcrops and what they
//! break into, TechData's sizes and equipment types, the slots and the
//! grid from the code, the analysis unlocks, the player's luck, the
//! lifepod's locker and supplies, and the game's tech type → prefab map.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sn_install::GameData;
use sn_sim::inventory::{
    AnalysisEntry, ItemRules, OutcropDrop, OutcropInfo, PickupInfo, SlotTable, Techs,
};

use crate::items::{ItemPrefab, TechLookup, addressable_prefab, item_prefab};
use crate::{Assets, Result};

/// What [`item_rules`] found, besides the rules: counts for the logs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRulesReport {
    /// Distinct prefabs the world places or the slots can place, and how
    /// many could not be read (skipped, as the census did).
    pub candidates: usize,
    pub unreadable: usize,
    /// Prefabs read only because an outcrop or the supplies name them.
    pub extra: usize,
    /// Seconds: the batches' saved objects; the candidate prefabs.
    pub scan_s: f64,
    pub prefabs_s: f64,
}

/// A candidate prefab's path and what it is as an item (`None`: not
/// readable).
type Read = (String, Option<ItemPrefab>);

/// Every class id saved in a cell or an object batch (batches read in
/// parallel).
fn placed_class_ids(game: &GameData) -> Result<BTreeSet<String>> {
    let (cells, _) = game.cell_batches().map_err(|e| e.to_string())?;
    let (objects, _) = game.object_batches().map_err(|e| e.to_string())?;
    let jobs: Vec<(bool, sn_world::BatchCoord)> = cells
        .into_iter()
        .map(|c| (true, c))
        .chain(objects.into_iter().map(|c| (false, c)))
        .collect();
    let read = |&(cell, coord): &(bool, sn_world::BatchCoord)| -> Result<Vec<String>> {
        let mut ids = Vec::new();
        if cell {
            if let Some(file) = game.read_batch_cells(coord).map_err(|e| e.to_string())? {
                for tree in file.cells.iter().filter_map(|c| c.objects.as_ref()) {
                    ids.extend(tree.objects.iter().map(|o| o.class_id.clone()));
                }
            }
        } else if let Some(tree) = game.read_batch_objects(coord).map_err(|e| e.to_string())? {
            ids.extend(tree.objects.iter().map(|o| o.class_id.clone()));
        }
        Ok(ids)
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = jobs.len().div_ceil(threads).max(1);
    let parts: Vec<Result<Vec<String>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || -> Result<Vec<String>> {
                    let mut ids = Vec::new();
                    for job in part {
                        ids.extend(read(job)?);
                    }
                    Ok(ids)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a batch reader panicked".into()))
            })
            .collect()
    });
    let mut ids = BTreeSet::new();
    for part in parts {
        ids.extend(part?);
    }
    Ok(ids)
}

/// Reads the item rules (`dll`: the game's code assembly, as
/// `read_assembly` gives it). Several seconds: it reads every batch's
/// saved objects and every candidate prefab.
pub fn item_rules(
    game: &GameData,
    assets: &Assets,
    dll: &[u8],
) -> Result<(ItemRules, ItemRulesReport)> {
    let catalog = assets.catalog()?;
    let db = game
        .read_prefab_database_ordered()
        .map_err(|e| e.to_string())?;
    let class_paths: HashMap<String, String> = db.iter().cloned().collect();
    // A prefab path's first class id in file order.
    let mut path_class: HashMap<&str, &str> = HashMap::new();
    for (class, path) in &db {
        path_class.entry(path.as_str()).or_insert(class.as_str());
    }
    let ent_tech = crate::ent_tech_data(assets)?;
    let lookup = TechLookup::new(&ent_tech, class_paths.clone());
    let code = crate::game_code(dll)?;
    let items = crate::item_code(dll)?;
    let tech = crate::tech_data(assets)?;
    let player = crate::player_items(assets)?;
    let pda = crate::player_data(assets)?.pda;
    let eq_types = {
        let asm = sn_dotnet::Assembly::parse(dll).map_err(|e| e.to_string())?;
        sn_dotnet::enum_values(&asm, "", "EquipmentType").map_err(|e| e.to_string())?
    };
    let eq = |name: &str| -> Result<i32> {
        eq_types
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|&(_, v)| i32::try_from(v).ok())
            .ok_or_else(|| format!("EquipmentType.{name} not found"))
    };

    // Candidates: placed, or in the spawn slots' loot table.
    let t = std::time::Instant::now();
    let mut classes = placed_class_ids(game)?;
    let scan_s = t.elapsed().as_secs_f64();
    let loot = crate::loot_table(assets)?;
    for (_, entries) in loot.distribution.biomes() {
        classes.extend(entries.iter().map(|e| e.class_id.clone()));
    }
    let mut by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in classes {
        if let Some(path) = class_paths.get(&c) {
            by_path.entry(path.clone()).or_default().push(c);
        }
    }
    let mut report = ItemRulesReport {
        candidates: by_path.len(),
        scan_s,
        ..Default::default()
    };
    let t = std::time::Instant::now();
    // Read in parallel (most of the time is here); the result does not
    // depend on the order.
    let paths: Vec<&String> = by_path.keys().collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = paths.len().div_ceil(threads).max(1);
    let parts: Vec<Result<Vec<Read>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|part| {
                let (catalog, lookup) = (&catalog, &lookup);
                scope.spawn(move || -> Result<Vec<Read>> {
                    let mut out = Vec::with_capacity(part.len());
                    for &path in part {
                        let item = match assets.prefab(catalog, path) {
                            Ok(prefab) => Some(item_prefab(assets, &prefab, lookup)?),
                            Err(_) => None,
                        };
                        out.push((path.clone(), item));
                    }
                    Ok(out)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a prefab reader panicked".into()))
            })
            .collect()
    });
    let mut read: BTreeMap<String, ItemPrefab> = BTreeMap::new();
    for part in parts {
        for (path, item) in part? {
            match item {
                Some(item) => {
                    read.insert(path, item);
                }
                None => report.unreadable += 1,
            }
        }
    }
    report.prefabs_s = t.elapsed().as_secs_f64();

    // Outcrops: each resource's prefab, and its class id.
    let mut outcrops = BTreeMap::new();
    let mut more_paths: BTreeSet<String> = BTreeSet::new();
    for (path, item) in &read {
        let Some(b) = &item.breakable else { continue };
        let mut resolve = |guid: &str| -> Result<String> {
            let p = addressable_prefab(assets, &catalog, guid)?
                .ok_or_else(|| format!("{path}: resource {guid} does not resolve"))?;
            let class = path_class
                .get(p.key.as_str())
                .ok_or_else(|| format!("{path}: resource {} has no class id", p.key))?;
            more_paths.insert(p.key.clone());
            Ok((*class).to_owned())
        };
        let mut list = Vec::with_capacity(b.prefab_list.len());
        for r in &b.prefab_list {
            list.push(OutcropDrop {
                tech: r.tech_type,
                chance: r.chance,
                class: resolve(&r.prefab)?,
            });
        }
        let info = OutcropInfo {
            hits_to_break: b.hits_to_break,
            vertical_offset: b.vertical_spawn_offset,
            num_chances: b.num_chances,
            list,
            default_class: resolve(&b.default_prefab)?,
        };
        for class in &by_path[path] {
            outcrops.insert(class.clone(), info.clone());
        }
    }

    // CraftData's tech type → class id: prefabs.db in order, the last wins.
    let mut tech_class = BTreeMap::new();
    for (class, _) in &db {
        let t = lookup.by_class_id(class);
        if t != 0 {
            tech_class.insert(t, class.clone());
        }
    }
    for t in &player.loot.escape_pod {
        let class = tech_class
            .get(t)
            .ok_or_else(|| format!("supply tech type {t} has no prefab"))?;
        if let Some(path) = class_paths.get(class) {
            more_paths.insert(path.clone());
            by_path.entry(path.clone()).or_default().push(class.clone());
        }
    }
    // Resources and supplies: read if not yet, and their class ids are
    // candidates too.
    for path in &more_paths {
        if let Some(c) = path_class.get(path.as_str()) {
            let list = by_path.entry(path.clone()).or_default();
            if !list.iter().any(|x| x == c) {
                list.push((*c).to_owned());
            }
        }
        if !read.contains_key(path) {
            let prefab = assets.prefab(&catalog, path)?;
            read.insert(path.clone(), item_prefab(assets, &prefab, &lookup)?);
            report.extra += 1;
        }
    }

    let mut pickups = BTreeMap::new();
    for (path, item) in &read {
        let Some(p) = item.pickupable else { continue };
        let info = PickupInfo {
            tech: item.tech_type,
            pickupable: p.is_pickupable,
            destroy_on_death: p.destroy_on_death,
            eatable: item.eatable,
        };
        for class in by_path.get(path).into_iter().flatten() {
            pickups.insert(class.clone(), info.clone());
        }
    }

    let mut techs = Techs {
        default_size: (
            code.tech_defaults.item_size[0],
            code.tech_defaults.item_size[1],
        ),
        default_equipment: code.tech_defaults.equipment_type,
        ..Default::default()
    };
    for e in &tech.entries {
        if let Some([w, h]) = e.item_size {
            techs.sizes.insert(e.tech_type, (w, h));
        }
        if let Some(t) = e.equipment_type {
            techs.equipment.insert(e.tech_type, t);
        }
    }
    let rules = ItemRules {
        techs,
        slots: SlotTable {
            slots: items.slot_types.clone(),
            vehicle_module: eq("VehicleModule")?,
            seamoth_module: eq("SeamothModule")?,
            exosuit_module: eq("ExosuitModule")?,
        },
        player_slots: items.player_slots.clone(),
        inventory_size: (items.inventory_size[0], items.inventory_size[1]),
        analysis: pda
            .analysis_tech
            .iter()
            .map(|a| AnalysisEntry {
                tech: a.tech_type,
                unlocks: a.unlock_tech_types.clone(),
            })
            .collect(),
        default_tech: pda.default_tech.clone(),
        pickups,
        outcrops,
        locker_size: (player.pod_locker.width, player.pod_locker.height),
        supplies: player.loot.escape_pod.clone(),
        luck: player
            .entropy
            .iter()
            .map(|(t, f)| (*t, f.entropy))
            .collect(),
        tech_class,
    };
    Ok((rules, report))
}
