//! `density` command: gather evidence about what the density byte means.
//!
//! For every single-voxel leaf in a batch, relate its density byte to
//! (a) whether the voxel is empty (type 0) or solid, and (b) whether it sits
//! on the surface, i.e. has a face neighbour of the other kind.

use std::process::ExitCode;

use sn_octree::{Batch, GAME_CHILD_ORDER, OCTREE_SIZE};
use sn_world::BatchCoord;

use crate::Result;
use sn_install::GameData;

#[derive(Clone, Copy, Default)]
struct Bucket {
    empty: u64,
    solid: u64,
    empty_on_surface: u64,
    solid_on_surface: u64,
}

pub fn run(game: &GameData, coord: BatchCoord) -> Result<ExitCode> {
    let bytes = game
        .read_batch(coord)?
        .ok_or_else(|| format!("batch {coord} has no octree file"))?;
    let batch = Batch::parse(&bytes).map_err(|e| format!("batch {coord}: {e}"))?;

    let mut buckets = [Bucket::default(); 256];
    for octree in &batch.octrees {
        let grid = octree
            .rasterize(GAME_CHILD_ORDER)
            .map_err(|e| e.to_string())?;
        const S: usize = OCTREE_SIZE;
        for z in 0..S {
            for y in 0..S {
                for x in 0..S {
                    let v = grid.get([x, y, z]);
                    let solid = v.ty != 0;
                    let mut on_surface = false;
                    for (a, d) in [(0, -1i32), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)] {
                        let mut p = [x as i32, y as i32, z as i32];
                        p[a] += d;
                        if p.iter().all(|c| (0..S as i32).contains(c)) {
                            let n = grid.get(p.map(|c| c as usize));
                            on_surface |= (n.ty != 0) != solid;
                        }
                    }
                    let b = &mut buckets[usize::from(v.density)];
                    match (solid, on_surface) {
                        (false, false) => b.empty += 1,
                        (false, true) => b.empty_on_surface += 1,
                        (true, false) => b.solid += 1,
                        (true, true) => b.solid_on_surface += 1,
                    }
                }
            }
        }
    }

    println!(
        "batch {coord}: voxels per density value (surface = has a face neighbour of the other kind)"
    );
    println!("density | empty  (on surface) | solid  (on surface)");
    let row = |label: String, b: Bucket| {
        println!(
            "{label:>7} | {:>9} ({:>9}) | {:>9} ({:>9})",
            b.empty + b.empty_on_surface,
            b.empty_on_surface,
            b.solid + b.solid_on_surface,
            b.solid_on_surface
        );
    };
    row("0".into(), buckets[0]);
    for start in (1..256).step_by(16) {
        let end = (start + 15).min(255);
        let mut sum = Bucket::default();
        for b in &buckets[start..=end] {
            sum.empty += b.empty;
            sum.solid += b.solid;
            sum.empty_on_surface += b.empty_on_surface;
            sum.solid_on_surface += b.solid_on_surface;
        }
        row(format!("{start}-{end}"), sum);
    }
    Ok(ExitCode::SUCCESS)
}
