//! `slots` command: the spawn slots of the baked cells (`EntitySlotsPlaceholder`)
//! and what they fill with for a seed (`docs/formats/entities.md` § Spawn slots).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::Assets;
use sn_install::GameData;
use sn_world::{BatchCoord, FILLER_CLASS_ID, SLOTS_COMPONENT, fill_slots, parse_slots};

use crate::Result;
use crate::entities::{Hash, batch_box, distance_outside};

#[derive(Default)]
struct BiomeStats {
    slots: usize,
    filled: usize,
    spawned: usize,
}

/// Slots per batch (or every batch), filled with `seed`.
pub fn run(game: &GameData, batch: Option<BatchCoord>, seed: u64) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let table = sn_assets::loot_table(&assets)?;
    let infos = sn_assets::entity_infos(&assets)?;
    let paths = game.read_prefab_database()?;
    println!(
        "loot distribution: {} prefab entries, {} rows, {} biomes ({} named in comments)",
        table.prefabs,
        table.rows,
        table.distribution.biome_count(),
        table.biome_names.len()
    );
    println!("world entity infos: {}", infos.len());

    // Every prefab of the distribution should be known to both tables.
    let mut listed = HashSet::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, entries) in table.distribution.biomes() {
        for e in entries {
            listed.insert(e.class_id.clone());
        }
    }
    let (mut no_info, mut no_path) = (Vec::new(), Vec::new());
    for id in listed.iter().filter(|id| *id != FILLER_CLASS_ID) {
        match infos.get(id) {
            Some(i) => *kinds.entry(format!("{:?}", i.kind)).or_default() += 1,
            None => no_info.push(id.clone()),
        }
        if !paths.contains_key(id) {
            no_path.push(id.clone());
        }
    }
    println!(
        "distribution prefabs: {} (slot types {kinds:?}); without info: {}, without prefabs.db path: {}",
        listed
            .len()
            .saturating_sub(usize::from(listed.contains(FILLER_CLASS_ID))),
        no_info.len(),
        no_path.len()
    );

    let batches = match batch {
        Some(b) => vec![b],
        None => game.cell_batches()?.0,
    };
    let mut errors = Vec::new();
    let mut hash = Hash::new();
    let (mut placeholders, mut slots_total, mut spawned_total) = (0usize, 0usize, 0usize);
    let mut by_biome: BTreeMap<u32, BiomeStats> = BTreeMap::new();
    let mut by_allowed: BTreeMap<u32, usize> = BTreeMap::new();
    let mut by_level: BTreeMap<i32, usize> = BTreeMap::new();
    let mut by_prefab: HashMap<&str, usize> = HashMap::new();
    let mut creatures = 0usize;
    let (mut density_min, mut density_max) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut reach = 0.0f32;
    let mut no_table: BTreeMap<u32, usize> = BTreeMap::new();
    // One batch: where its still (non-creature) fillers stand.
    let mut still: Vec<([f32; 3], &str)> = Vec::new();
    let size = sn_terrain::batch_voxels(&game.read_index()?);
    // Slots whose world position lies outside their batch (> 1 m), the
    // furthest, and placeholders saved at the world origin.
    let (mut outside, mut outside_max, mut at_origin) = (0usize, 0.0f32, 0usize);
    for &coord in &batches {
        let bounds = batch_box(coord, size);
        let file = match game.read_batch_cells(coord) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(e) => {
                errors.push(e.0);
                continue;
            }
        };
        for cell in &file.cells {
            let Some(tree) = &cell.objects else { continue };
            let (world, _) = tree.world_transforms();
            for (object, placed) in tree.objects.iter().zip(&world) {
                for c in object
                    .components
                    .iter()
                    .filter(|c| c.type_name == SLOTS_COMPONENT)
                {
                    placeholders += 1;
                    let slots = match parse_slots(&c.data) {
                        Ok(s) => s,
                        Err(e) => {
                            errors.push(format!("{coord} {}: {e}", object.id));
                            continue;
                        }
                    };
                    if placed.position.iter().all(|v| v.abs() < 1e-3) {
                        at_origin += 1;
                    }
                    for s in &slots {
                        let at = placed.then(&sn_world::Transform {
                            position: s.position,
                            ..Default::default()
                        });
                        let d = distance_outside(at.position, bounds);
                        if d > 1.0 {
                            outside += 1;
                            outside_max = outside_max.max(d);
                        }
                        slots_total += 1;
                        by_biome.entry(s.biome).or_default().slots += 1;
                        *by_allowed.entry(s.allowed).or_default() += 1;
                        density_min = density_min.min(s.density);
                        density_max = density_max.max(s.density);
                        reach = reach.max(s.position.iter().map(|v| v * v).sum::<f32>().sqrt());
                        if table.distribution.biome(s.biome).is_none() {
                            *no_table.entry(s.biome).or_default() += 1;
                        }
                    }
                    let spawns = fill_slots(seed, &object.id, &slots, &table.distribution, &infos);
                    let mut filled = HashSet::new();
                    for sp in &spawns {
                        let biome = slots[sp.slot].biome;
                        let stats = by_biome.entry(biome).or_default();
                        stats.spawned += 1;
                        if filled.insert(sp.slot) {
                            stats.filled += 1;
                        }
                        spawned_total += 1;
                        *by_level.entry(sp.info.cell_level).or_default() += 1;
                        let path = paths.get(sp.class_id).map_or("(no path)", String::as_str);
                        *by_prefab.entry(path).or_default() += 1;
                        if path.starts_with("WorldEntities/Creatures/") {
                            creatures += 1;
                        }
                        hash.add(sp.class_id.as_bytes());
                        let world = placed.then(&sp.transform);
                        hash.add_transform(&world);
                        if batch.is_some() && !path.starts_with("WorldEntities/Creatures/") {
                            still.push((world.position, path));
                        }
                    }
                }
            }
        }
    }

    println!(
        "seed {seed}: {} batches, {placeholders} placeholders, {slots_total} slots, \
         {spawned_total} objects spawned ({creatures} creatures)",
        batches.len()
    );
    if slots_total > 0 {
        println!(
            "  density {density_min:.3} .. {density_max:.3}; slots up to {reach:.1} m from their placeholder"
        );
    }
    println!(
        "  slots outside their batch (> 1 m): {outside} (up to {outside_max:.0} m); \
         placeholders at the world origin: {at_origin}"
    );
    let flags: Vec<String> = by_allowed
        .iter()
        .map(|(a, n)| format!("{a:#04x}: {n}"))
        .collect();
    println!(
        "  slots by allowed types (1 small, 2 medium, 4 large, 8 tall, 16 creature): {}",
        flags.join(", ")
    );
    println!("  spawns by cell level: {by_level:?}");
    let missing: usize = no_table.values().sum();
    println!(
        "  slots whose biome has no loot table: {missing} in {} biomes",
        no_table.len()
    );
    for (biome, n) in no_table.iter().take(8) {
        println!("    {n:>6}  {}", table.biome_label(*biome));
    }

    let mut biomes: Vec<(&u32, &BiomeStats)> = by_biome.iter().collect();
    biomes.sort_by(|a, b| b.1.slots.cmp(&a.1.slots).then(a.0.cmp(b.0)));
    println!(
        "  per biome (slots, filled slots, objects); the 25 biggest, then all of Safe Shallows:"
    );
    let safe_shallows = |b: u32| (100..200).contains(&b);
    for (i, (biome, s)) in biomes.iter().enumerate() {
        if i < 25 || safe_shallows(**biome) {
            println!(
                "    {:>6} {:>6} {:>6}  {}",
                s.slots,
                s.filled,
                s.spawned,
                table.biome_label(**biome)
            );
        }
    }
    let mut top: Vec<(&&str, &usize)> = by_prefab.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("  most spawned prefabs:");
    for (path, n) in top.iter().take(15) {
        println!("    {n:>7}  {path}");
    }
    // By folder (`WorldEntities/<folder>/…`): creatures, outcrops, flora, …
    let mut folders: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, n) in &by_prefab {
        let folder = path
            .split('/')
            .nth(1)
            .filter(|_| path.matches('/').count() > 1);
        *folders.entry(folder.unwrap_or("(other)")).or_default() += n;
    }
    println!("  spawns by prefab folder: {folders:?}");
    if !still.is_empty() {
        println!("  still objects spawned (world position, prefab), first 20:");
        for (p, path) in still.iter().take(20) {
            println!("    ({:8.2} {:8.2} {:8.2})  {path}", p[0], p[1], p[2]);
        }
    }

    println!("errors: {}", errors.len());
    for e in errors.iter().take(10) {
        println!("  {e}");
    }
    println!("hash of all spawns: {:016x}", hash.0);
    println!("time: {:.2} s", start.elapsed().as_secs_f64());
    let ok = errors.is_empty() && no_info.is_empty() && no_path.is_empty();
    println!("result: {}", if ok { "OK" } else { "PROBLEMS (see above)" });
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
