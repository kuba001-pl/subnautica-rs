//! `terrain-materials` command: the terrain's block types, their materials
//! and textures, cross-checked against the type ids used by the octrees.

use std::collections::BTreeSet;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, terrain_materials};
use sn_install::GameData;
use sn_octree::Batch;

use crate::Result;

/// Every node type id that occurs in the terrain octrees.
fn octree_type_ids(game: &GameData) -> Result<BTreeSet<u8>> {
    let (batches, _) = game.octree_batches()?;
    let mut used = BTreeSet::new();
    for coord in batches {
        let Some(bytes) = game.read_batch(coord)? else {
            continue;
        };
        let batch = Batch::parse(&bytes).map_err(|e| format!("batch {coord}: {e}"))?;
        for octree in &batch.octrees {
            for node in octree.nodes.iter().filter(|n| n.is_leaf()) {
                used.insert(node.ty);
            }
        }
    }
    Ok(used)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    println!(
        "indexed {} bundles ({} internal files) in {:.2} s",
        assets.bundle_paths().len(),
        assets.indexed_files(),
        start.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    let materials = terrain_materials(&assets)?;
    println!(
        "block types: {} from the scene's Voxeland table, {} from BlockPrefabs ({} more without a type id); {} conflicts {:?}",
        materials.scene_types,
        materials.prefab_types,
        materials.prefabs_without_id,
        materials.conflicts.len(),
        materials.conflicts
    );
    println!(
        "surfaceDensityValue = {}; {} textures loaded in {:.2} s",
        materials.surface_density_value,
        materials.texture_count,
        t.elapsed().as_secs_f64()
    );

    println!();
    println!(
        "type  layer  material                       cap (scale)                              side (scale)"
    );
    for m in materials.types.iter().flatten() {
        let describe = |t: &Option<std::sync::Arc<sn_assets::TerrainTexture>>| match t {
            Some(t) => format!(
                "{} {}x{} f{}",
                t.texture.name, t.texture.width, t.texture.height, t.texture.format
            ),
            None => "-".into(),
        };
        let cap = format!("{} ({:.2})", describe(&m.cap.albedo), m.cap.scale);
        let side = format!("{} ({:.2})", describe(&m.side.albedo), m.side.scale);
        println!(
            "{:>4}  {:>5}  {:<30} {cap:<40} {side}",
            m.type_id, m.layer, m.name
        );
    }

    let used = octree_type_ids(game)?;
    let with_material: BTreeSet<u8> = materials
        .types
        .iter()
        .flatten()
        .map(|m| m.type_id as u8)
        .collect();
    let missing: Vec<u8> = used
        .iter()
        .copied()
        .filter(|&t| t != 0 && !with_material.contains(&t))
        .collect();
    let unused = with_material.difference(&used).count();
    println!();
    println!(
        "octrees use {} type ids (incl. 0 = empty); {} block types have materials; {} of those never occur in the octrees",
        used.len(),
        with_material.len(),
        unused
    );
    println!("type ids used by the octrees but without a material: {missing:?}");
    for w in &materials.warnings {
        println!("warning: {w}");
    }
    let without_texture: Vec<usize> = materials
        .types
        .iter()
        .flatten()
        .filter(|m| m.cap.albedo.is_none() || m.side.albedo.is_none())
        .map(|m| m.type_id)
        .collect();
    println!("materials missing a cap or side texture: {without_texture:?}");
    let ok = missing.is_empty() && materials.warnings.is_empty() && without_texture.is_empty();
    println!("result: {}", if ok { "OK" } else { "PROBLEMS (see above)" });
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Every material property (textures, floats, colours, keywords) of every
/// block type, one block per type.
pub fn props(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let materials = terrain_materials(&assets)?;
    for m in materials.types.iter().flatten() {
        println!(
            "type {} layer {} {} [{}]",
            m.type_id, m.layer, m.name, m.shader_keywords
        );
        println!("  textures: {}", m.texture_slots.join(" "));
        for (name, value) in &m.floats {
            println!("  {name} = {value}");
        }
        for (name, c) in &m.colors {
            println!(
                "  {name} = ({:.3}, {:.3}, {:.3}, {:.3})",
                c[0], c[1], c[2], c[3]
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Which block types make up the terrain *surface* (solid voxels next to an
/// empty one) in a cube of batches, with their materials.
pub fn region(game: &GameData, center: sn_world::BatchCoord, radius: i32) -> Result<ExitCode> {
    let index = game.read_index()?;
    let mut counts = [0u64; 256];
    let mut total = 0u64;
    for dz in -radius..=radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let coord = center.offset(dx, dy, dz);
                let Some(batch) = game.load_batch(&index, coord)? else {
                    continue;
                };
                let grid = batch
                    .batch
                    .rasterize(batch.octree_dims)
                    .map_err(|e| format!("batch {coord}: {e}"))?;
                let [nx, ny, nz] = grid.dims;
                for z in 1..nz - 1 {
                    for y in 1..ny - 1 {
                        for x in 1..nx - 1 {
                            let v = grid.get([x, y, z]);
                            if !v.is_solid() {
                                continue;
                            }
                            let open = [
                                [x - 1, y, z],
                                [x + 1, y, z],
                                [x, y - 1, z],
                                [x, y + 1, z],
                                [x, y, z - 1],
                                [x, y, z + 1],
                            ]
                            .iter()
                            .any(|p| !grid.get(*p).is_solid());
                            if open {
                                counts[usize::from(v.ty)] += 1;
                                total += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    let assets = Assets::index(game)?;
    let materials = terrain_materials(&assets)?;
    println!("surface voxels around batch {center} (radius {radius}): {total}");
    println!(
        " type   layer  share  source  material                       cap texture / side texture"
    );
    let mut order: Vec<usize> = (0..256).filter(|&t| counts[t] > 0).collect();
    order.sort_by_key(|&t| std::cmp::Reverse(counts[t]));
    for t in order {
        let share = 100.0 * counts[t] as f64 / total.max(1) as f64;
        match &materials.types[t] {
            Some(m) => {
                let name = |l: &sn_assets::SurfaceLayer| {
                    l.albedo
                        .as_ref()
                        .map_or("-".to_string(), |a| a.texture.name.clone())
                };
                let conflict = if materials.conflicts.contains(&t) {
                    " CONFLICT"
                } else {
                    ""
                };
                println!(
                    "{t:>5}  {:>5} {share:>6.2}%  {:<6}  {:<30} {} / {}{conflict}",
                    m.layer,
                    format!("{:?}", m.source),
                    m.name,
                    name(&m.cap),
                    name(&m.side)
                );
            }
            None => println!("{t:>5}         {share:>6.2}%  -       (no material)"),
        }
    }
    Ok(ExitCode::SUCCESS)
}
