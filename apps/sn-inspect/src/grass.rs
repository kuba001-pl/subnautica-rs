//! `grass` command: the terrain grass of a batch at full resolution, as
//! `sn_terrain::build_grass` scatters it (`docs/formats/terrain-materials.md`
//! § Grass).

use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, terrain_materials};
use sn_install::GameData;
use sn_terrain::{GrassBudget, Neighbourhood, batch_mesh, batch_voxels, build_grass};
use sn_world::BatchCoord;

use crate::Result;
use crate::entities::Hash;

pub fn run(game: &GameData, coord: BatchCoord, seed: u64) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let materials = terrain_materials(&assets)?;
    let types = materials.grass_types();
    println!(
        "grass types: {} of {} block types",
        types.iter().flatten().count(),
        materials.types.iter().flatten().count()
    );

    let index = game.read_index()?;
    let mut around = Vec::new();
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = coord.offset(dx, dy, dz);
                if let Some(b) = game.load_batch(&index, c)? {
                    around.push((c, b));
                }
            }
        }
    }
    let lookup = |c: BatchCoord| around.iter().find(|(k, _)| *k == c).map(|(_, b)| b);
    let neighbourhood = Neighbourhood::new(coord, batch_voxels(&index), lookup);
    let t = Instant::now();
    let Some(mesh) = batch_mesh(&neighbourhood, 0) else {
        println!("batch {coord}: no terrain");
        return Ok(ExitCode::SUCCESS);
    };
    let mesh_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let budget = GrassBudget::default();
    let grass = build_grass(&mesh, &types, &budget, seed);
    let grass_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "batch {coord}: {} faces at full resolution (meshed in {mesh_ms:.0} ms); grass built in {grass_ms:.0} ms, seed {seed}",
        mesh.triangles.len() / 2
    );
    let mut hash = Hash::new();
    let (mut tufts, mut verts, mut tris) = (0, 0, 0);
    let mut high = 0.0f32;
    println!("  type  name                          tufts   vertices  triangles  mesh");
    for g in &grass {
        let m = materials.types[usize::from(g.ty)].as_ref();
        let name = m.map_or("?", |m| m.name.as_str());
        let mesh_name = m
            .and_then(|m| m.grass.as_ref())
            .map_or("?", |g| g.mesh_name.as_str());
        println!(
            "  {:>4}  {:<28} {:>6} {:>10} {:>10}  {mesh_name}",
            g.ty,
            name,
            g.tufts,
            g.positions.len(),
            g.indices.len() / 3
        );
        tufts += g.tufts;
        verts += g.positions.len();
        tris += g.indices.len() / 3;
        high = g.colors.iter().fold(high, |h, c| h.max(c[3]));
        for p in &g.positions {
            for v in p {
                hash.add(&v.to_le_bytes());
            }
        }
    }
    println!(
        "  total: {tufts} tufts, {verts} vertices, {tris} triangles; highest vertex alpha {high:.2}"
    );
    println!("hash of all grass vertices: {:016x}", hash.0);
    println!("time: {:.2} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
