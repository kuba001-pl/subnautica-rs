//! `pickups` (M9d1): the item data M9d needs, headless. Every prefab the
//! world places and every prefab the spawn slots can place: which can be
//! picked up, its tech type by the game's rule and where it came from, item
//! sizes, food; the outcrops and what they break into (each resource
//! resolved to its prefab and checked against the listed tech type); the
//! player's luck, the lifepod's locker and starting supplies, the analysis
//! unlocks; the rules in the game's code; the item text.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{
    Assets, GameCode, ItemPrefab, TechLookup, TechSource, addressable_prefab, item_prefab,
};
use sn_install::GameData;

use crate::Result;

/// One prefab with item scripts.
struct Found {
    key: String,
    placements: usize,
    placed: bool,
    item: ItemPrefab,
}

fn name(code: &GameCode, t: i32) -> String {
    code.tech_name(t)
        .map_or_else(|| format!("#{t}"), str::to_string)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let class_paths = game.read_prefab_database()?;
    let ent_tech = sn_assets::ent_tech_data(&assets)?;
    let lookup = TechLookup::new(&ent_tech, class_paths.clone());
    let dll = sn_assets::read_assembly(game)?;
    let code = sn_assets::game_code(&dll)?;
    let items = sn_assets::item_code(&dll)?;
    let tech = sn_assets::tech_data(&assets)?;
    let english = sn_assets::language(game, "English")?;

    // Placed prefabs, then those only the spawn slots place.
    let placed = crate::prefab::placed_positions(game)?;
    let loot = sn_assets::loot_table(&assets)?;
    let mut loot_keys: BTreeSet<String> = BTreeSet::new();
    for (_, entries) in loot.distribution.biomes() {
        for e in entries {
            if let Some(path) = class_paths.get(&e.class_id) {
                loot_keys.insert(path.clone());
            }
        }
    }
    let jobs: Vec<(String, usize, bool)> = placed
        .iter()
        .map(|(k, p)| (k.clone(), p.len(), true))
        .chain(
            loot_keys
                .iter()
                .filter(|k| !placed.contains_key(*k))
                .map(|k| (k.clone(), 0, false)),
        )
        .collect();
    let mut found = Vec::new();
    let (mut unreadable, mut errors) = (0, Vec::new());
    for (key, placements, is_placed) in &jobs {
        let Ok(prefab) = assets.prefab(&catalog, key) else {
            unreadable += 1;
            continue;
        };
        match item_prefab(&assets, &prefab, &lookup) {
            Ok(item) if item.pickupables > 0 || item.breakable.is_some() => found.push(Found {
                key: key.clone(),
                placements: *placements,
                placed: *is_placed,
                item,
            }),
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }
    println!(
        "prefabs checked: {} ({} placed, {} only from slots); unreadable {unreadable}; read errors {}",
        jobs.len(),
        placed.len(),
        jobs.len() - placed.len(),
        errors.len()
    );
    for e in errors.iter().take(10) {
        println!("  error: {e}");
    }

    // Pick-ups.
    let pickups: Vec<&Found> = found.iter().filter(|f| f.item.pickupables > 0).collect();
    let mut by_source: BTreeMap<TechSource, usize> = BTreeMap::new();
    let (mut not_pickupable, mut kept_on_death, mut eatable, mut several, mut pack_up) =
        (0, 0, 0, 0, 0);
    let mut no_rotation = 0;
    let mut misses = Vec::new();
    let mut tech_types: BTreeSet<i32> = BTreeSet::new();
    for f in &pickups {
        *by_source.entry(f.item.source).or_default() += 1;
        if let Some(p) = f.item.pickupable {
            not_pickupable += usize::from(!p.is_pickupable);
            kept_on_death += usize::from(!p.destroy_on_death);
            no_rotation += usize::from(!p.randomize_rotation_when_dropped);
            pack_up += usize::from(p.use_pack_up_icon);
        }
        eatable += usize::from(f.item.eatable);
        several += usize::from(f.item.pickupables > 1);
        if f.item.tech_type == 0 {
            misses.push(f);
        } else {
            tech_types.insert(f.item.tech_type);
        }
    }
    let placed_pickups = pickups.iter().filter(|f| f.placed).count();
    println!(
        "Pickupable on {} prefabs ({placed_pickups} placed, {} placements; {} only from slots)",
        pickups.len(),
        pickups.iter().map(|f| f.placements).sum::<usize>(),
        pickups.len() - placed_pickups
    );
    println!("  tech type found by: {by_source:?}");
    println!(
        "  isPickupable false {not_pickupable}; destroyOnDeath false {kept_on_death}; randomizeRotationWhenDropped false {no_rotation}; usePackUpIcon {pack_up}; Eatable {eatable}; more than one Pickupable {several}"
    );
    println!(
        "  tech type None (misses): {}{}",
        misses.len(),
        if misses.is_empty() { "" } else { ":" }
    );
    for f in &misses {
        println!(
            "    {} ({:?}, {} placements, isPickupable {:?})",
            f.key,
            f.item.source,
            f.placements,
            f.item.pickupable.map(|p| p.is_pickupable)
        );
        let prefab = assets.prefab(&catalog, &f.key)?;
        for node in &prefab.nodes {
            for c in assets.node_components(node)? {
                if assets.script_class(&c).as_deref() == Some("Pickupable") {
                    println!(
                        "      Pickupable on node {:?} (active {})",
                        node.name, node.active
                    );
                }
            }
        }
    }

    // Item sizes.
    let defaults = &code.tech_defaults;
    let mut sizes: BTreeMap<[i32; 2], usize> = BTreeMap::new();
    let (mut with_entry, mut with_size) = (0, 0);
    let mut equipment: BTreeMap<i32, usize> = BTreeMap::new();
    for &t in &tech_types {
        let entry = tech.get(t);
        with_entry += usize::from(entry.is_some());
        let size = entry.and_then(|e| e.item_size);
        with_size += usize::from(size.is_some());
        *sizes.entry(size.unwrap_or(defaults.item_size)).or_default() += 1;
        let eq = entry
            .and_then(|e| e.equipment_type)
            .unwrap_or(defaults.equipment_type);
        *equipment.entry(eq).or_default() += 1;
    }
    println!(
        "pick-up tech types: {}; with a TechData entry {with_entry}; itemSize given {with_size}, default {:?} for the rest",
        tech_types.len(),
        defaults.item_size
    );
    println!("  sizes (cells: tech types): {sizes:?}");
    println!("  equipment types (value: tech types): {equipment:?}");

    // Outcrops.
    let breakables: Vec<&Found> = found
        .iter()
        .filter(|f| f.item.breakable.is_some())
        .collect();
    println!("BreakableResource on {} prefabs:", breakables.len());
    let mut resolved = (0, 0);
    for f in &breakables {
        let Some(b) = &f.item.breakable else { continue };
        println!(
            "  {} ({} placements{}): tech {} ({:?}), hitsToBreak {}, numChances {}, verticalSpawnOffset {}, breakText key {:?} in English: {}",
            f.key,
            f.placements,
            if f.placed { "" } else { ", slots only" },
            name(&code, f.item.tech_type),
            f.item.source,
            b.hits_to_break,
            b.num_chances,
            b.vertical_spawn_offset,
            b.break_text,
            english.contains_key(&b.break_text)
        );
        let rolls = b
            .prefab_list
            .iter()
            .map(|r| (r.prefab.as_str(), r.tech_type, Some(r.chance)))
            .chain([(b.default_prefab.as_str(), b.default_tech_type, None)]);
        for (guid, listed, chance) in rolls {
            resolved.1 += 1;
            let what = chance.map_or_else(|| "default".to_string(), |c| format!("chance {c}"));
            match addressable_prefab(&assets, &catalog, guid)? {
                Some(p) => {
                    let item = item_prefab(&assets, &p, &lookup)?;
                    let same = item.tech_type == listed;
                    resolved.0 += usize::from(same);
                    println!(
                        "    {what}: {} listed {}, prefab {} is {} ({:?}){}",
                        guid,
                        name(&code, listed),
                        p.key,
                        name(&code, item.tech_type),
                        item.source,
                        if same { "" } else { "  <-- differs" }
                    );
                }
                None => println!("    {what}: {guid} does not resolve"),
            }
        }
    }
    println!(
        "  resources resolved to a prefab of the listed tech type: {} of {}",
        resolved.0, resolved.1
    );

    // The rules in the code.
    let eq_names = {
        let asm = sn_dotnet::Assembly::parse(&dll).map_err(|e| e.to_string())?;
        sn_dotnet::enum_values(&asm, "", "EquipmentType").map_err(|e| e.to_string())?
    };
    let eq_name = |v: i32| {
        eq_names
            .iter()
            .find(|(_, x)| *x == i64::from(v))
            .map_or_else(|| format!("#{v}"), |(n, _)| n.clone())
    };
    println!(
        "inventory (Inventory.Awake): {} x {}",
        items.inventory_size[0], items.inventory_size[1]
    );
    println!(
        "equipment slots (Equipment.slotMapping): {}",
        items.slot_types.len()
    );
    for (slot, t) in &items.slot_types {
        println!("  {slot}: {} ({t})", eq_name(*t));
    }
    let unknown: Vec<&String> = items
        .player_slots
        .iter()
        .filter(|s| !items.slot_types.iter().any(|(n, _)| n == *s))
        .collect();
    println!(
        "player's slots (UnlockDefaultEquipmentSlots): {:?}; not in slotMapping: {unknown:?}",
        items.player_slots
    );

    // The player, the pod, the PDA.
    let player = sn_assets::player_items(&assets)?;
    println!("PlayerEntropy: {} randomizers", player.entropy.len());
    for (t, f) in &player.entropy {
        println!(
            "  {}: entropy {}, timeLastCheck {}",
            name(&code, *t),
            f.entropy,
            f.time_last_check
        );
    }
    let names = |v: &[i32]| v.iter().map(|&t| name(&code, t)).collect::<Vec<_>>();
    println!(
        "LootSpawner (on {:?}): escapePodTechTypes {} {:?}; supplyTechTypes {}",
        player.loot_prefab,
        player.loot.escape_pod.len(),
        names(&player.loot.escape_pod),
        player.loot.supply.len()
    );
    let l = &player.pod_locker;
    println!(
        "lifepod locker {:?}: {} x {}, hoverText {:?} (in English: {}), storageLabel {:?} (in English: {})",
        player.pod_locker_node,
        l.width,
        l.height,
        l.hover_text,
        english.contains_key(&l.hover_text),
        l.storage_label,
        english.contains_key(&l.storage_label)
    );
    let pda = sn_assets::player_data(&assets)?.pda;
    let unlocks: usize = pda
        .analysis_tech
        .iter()
        .map(|a| a.unlock_tech_types.len())
        .sum();
    let pickup_analysis = pda
        .analysis_tech
        .iter()
        .filter(|a| tech_types.contains(&a.tech_type))
        .count();
    println!(
        "PDAData.analysisTech: {} entries, {unlocks} unlocks; entries for a pick-up tech type: {pickup_analysis}",
        pda.analysis_tech.len()
    );

    // Text.
    let named = tech_types
        .iter()
        .filter(|&&t| code.tech_name(t).is_some_and(|n| english.contains_key(n)))
        .count();
    println!(
        "English.json: {} keys; pick-up tech types with a name: {named} of {}",
        english.len(),
        tech_types.len()
    );
    for key in [
        "PickUpFormat",
        "PackUpFormat",
        "InventoryFull",
        "InventoryCantDropHere",
        "InventoryLabel",
        "EquipmentLabel",
    ] {
        println!("  {key}: {}", english.contains_key(key));
    }
    println!("({:.1} s)", start.elapsed().as_secs_f32());
    Ok(ExitCode::SUCCESS)
}
