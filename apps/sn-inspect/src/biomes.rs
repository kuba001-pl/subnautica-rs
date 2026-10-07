//! `biomes` command: the biome map, each batch's override biome, and the
//! water settings of every biome (`WaterBiomeManager`).

use std::collections::BTreeMap;
use std::process::ExitCode;

use sn_assets::{Assets, water_biomes};
use sn_install::GameData;
use sn_world::{BatchRootSettings, BiomeMap, world_to_voxel};

use crate::Result;

/// The biome at a Unity world position: the batch's override, else the map.
fn biome_at(
    game: &GameData,
    map: &BiomeMap,
    names: &[String],
    land_size: usize,
    batch_size: [i32; 3],
    world: [f32; 3],
) -> Result<(String, &'static str)> {
    let v = world_to_voxel(world).map(|c| c.floor() as i32);
    let coord = sn_world::BatchCoord::new(
        v[0].div_euclid(batch_size[0]),
        v[1].div_euclid(batch_size[1]),
        v[2].div_euclid(batch_size[2]),
    );
    if let Some(tree) = game.read_batch_objects(coord)? {
        for o in tree.objects.iter().filter(|o| o.parent.is_none()) {
            for c in o
                .components
                .iter()
                .filter(|c| c.type_name == "LargeWorldBatchRoot")
            {
                let s = BatchRootSettings::parse(&c.data).map_err(|e| e.to_string())?;
                if let Some(b) = s.override_biome {
                    return Ok((b, "batch override"));
                }
            }
        }
    }
    Ok(match map.index_at(v[0], v[2], land_size) {
        Some(i) => (
            names
                .get(usize::from(i))
                .cloned()
                .unwrap_or(format!("(index {i})")),
            "biome map",
        ),
        None => ("(outside the map)".into(), "-"),
    })
}

pub fn run(game: &GameData, at: Option<[f32; 3]>) -> Result<ExitCode> {
    let index = game.read_index()?;
    let batch_size = sn_terrain::batch_voxels(&index);
    let land_size = (index.batches()[0] as i32 * batch_size[0]) as usize;
    let (map, names) = game.read_biome_map()?;
    let assets = Assets::index(game)?;
    let water = water_biomes(&assets)?;
    let has_settings = |name: &str| {
        water
            .biomes
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(name))
    };

    if let Some(p) = at {
        let (biome, source) = biome_at(game, &map, &names, land_size, batch_size, p)?;
        println!("{p:?}: {biome} (from the {source})");
        match water
            .biomes
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&biome))
        {
            Some(b) => println!("  {:?}", b.settings),
            None => println!("  no water settings for this biome"),
        }
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "WaterBiomeManager: {} biomes; settings volume {}³ cells upsampled to {}³ over ±{} m, blur {}",
        water.biomes.len(),
        water.texture_size,
        water.upsampled_size,
        water.region_bounds,
        water.enable_blur
    );
    println!(
        "{:<28} {:>24} {:>6} {:>6} {:>6} {:>6} {:>6} {:>5}  scattering colour / emissive",
        "biome", "absorption", "scat", "murk", "start", "sun", "amb", "°C"
    );
    for b in &water.biomes {
        let s = &b.settings;
        println!(
            "{:<28} {:>24} {:>6.2} {:>6.2} {:>6.1} {:>6.2} {:>6.2} {:>5.1}  {:?} / {:?} ×{}",
            b.name,
            format!("{:.2?}", s.absorption),
            s.scattering,
            s.murkiness,
            s.start_distance,
            s.sunlight_scale,
            s.ambient_scale,
            s.temperature,
            s.scattering_color.map(|v| (v * 100.0).round() / 100.0),
            s.emissive.map(|v| (v * 100.0).round() / 100.0),
            s.emissive_scale
        );
    }

    println!();
    println!(
        "biome map: {}×{} cells, {} voxels per cell; {} names in biomes.csv",
        map.width,
        map.height,
        land_size / map.width,
        names.len()
    );
    let mut counts = [0usize; 256];
    for &c in &map.cells {
        counts[usize::from(c)] += 1;
    }
    let mut missing = Vec::new();
    for (i, &n) in counts.iter().enumerate().filter(|(_, n)| **n > 0) {
        let name = names.get(i).map_or("(no name)", String::as_str);
        let ok = has_settings(name);
        if !ok {
            missing.push(name.to_string());
        }
        println!(
            "  {i:>3} {name:<24} {:>6.2}% {}",
            100.0 * n as f64 / map.cells.len() as f64,
            if ok { "" } else { "NO WATER SETTINGS" }
        );
    }

    let (batches, _) = game.object_batches()?;
    let mut overrides: BTreeMap<String, usize> = BTreeMap::new();
    for coord in batches {
        let Some(tree) = game.read_batch_objects(coord)? else {
            continue;
        };
        for o in tree.objects.iter().filter(|o| o.parent.is_none()) {
            for c in o
                .components
                .iter()
                .filter(|c| c.type_name == "LargeWorldBatchRoot")
            {
                let s = BatchRootSettings::parse(&c.data).map_err(|e| format!("{coord}: {e}"))?;
                if let Some(b) = s.override_biome {
                    *overrides.entry(b).or_default() += 1;
                }
            }
        }
    }
    println!(
        "batch overrides: {} batches",
        overrides.values().sum::<usize>()
    );
    for (name, n) in &overrides {
        let ok = has_settings(name);
        if !ok {
            missing.push(name.clone());
        }
        println!(
            "  {name:<28} {n:>5} {}",
            if ok { "" } else { "NO WATER SETTINGS" }
        );
    }
    missing.sort();
    missing.dedup();
    println!("biomes without water settings: {missing:?}");
    Ok(ExitCode::SUCCESS)
}
