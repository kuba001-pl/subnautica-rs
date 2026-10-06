//! `index` command.

use std::process::ExitCode;

use sn_install::GameData;

use crate::Result;

pub fn print_index(game: &GameData) -> Result<ExitCode> {
    let index = game.read_index()?;
    let [bx, by, bz] = index.batches();
    println!("data folder:      {}", game.build_dir.display());
    println!("header value:     {}", index.header);
    println!("world voxels:     {:?}", index.world_voxels);
    println!("world octrees:    {:?}", index.world_octrees);
    println!("octree size:      {}", index.octree_size);
    println!("batch octrees:    {:?}", index.batch_octrees);
    println!(
        "batches:          {bx} x {by} x {bz} = {}",
        index.batch_count()
    );

    let values = &index.batch_values;
    let non_zero: Vec<f32> = values.iter().copied().filter(|v| *v != 0.0).collect();
    let max = non_zero.iter().copied().fold(f32::MIN, f32::max);
    let min = non_zero.iter().copied().fold(f32::MAX, f32::min);
    println!(
        "per-batch values: {} total, {} non-zero (min {min}, max {max})",
        values.len(),
        non_zero.len()
    );
    let (files, _) = game.octree_batches()?;
    println!("octree files:     {}", files.len());
    Ok(ExitCode::SUCCESS)
}
