//! `voxel` command: what is at a Unity world position, and where the terrain
//! surface is in that column.

use std::process::ExitCode;

use sn_install::GameData;
use sn_terrain::batch_voxels;
use sn_world::{BatchCoord, world_to_voxel};

use crate::Result;

pub fn run(game: &GameData, world: [f32; 3]) -> Result<ExitCode> {
    let index = game.read_index()?;
    let size = batch_voxels(&index);
    let voxel = world_to_voxel(world).map(|v| v.round() as i32);
    let at = |v: [i32; 3]| -> Result<Option<sn_octree::Voxel>> {
        let c = [0, 1, 2].map(|a| v[a].div_euclid(size[a]));
        let coord = BatchCoord::new(c[0], c[1], c[2]);
        let Some(batch) = game.load_batch(&index, coord)? else {
            return Ok(None);
        };
        let local = [0, 1, 2].map(|a| (v[a] - c[a] * size[a]) as usize);
        Ok(batch.sample(local))
    };
    let describe = |v: Option<sn_octree::Voxel>| match v {
        Some(v) => format!(
            "{} (type {}, density {})",
            if v.is_solid() { "SOLID" } else { "empty" },
            v.ty,
            v.density
        ),
        None => "no batch file here (uniform: solid or empty, unknown)".into(),
    };
    println!(
        "world {world:?} = voxel {voxel:?}: {}",
        describe(at(voxel)?)
    );

    // Scan the column top-down for surfaces (empty above, solid below).
    let world_y = |vy: i32| vy as f32 + 0.5 - sn_world::VOXEL_WORLD_OFFSET[1];
    let mut surfaces = Vec::new();
    let mut above_solid = None;
    let top = index.world_voxels[1] as i32 - 1;
    for vy in (0..=top).rev() {
        let solid = at([voxel[0], vy, voxel[2]])?.map(|v| v.is_solid());
        if above_solid == Some(Some(false)) && solid == Some(true) {
            surfaces.push(world_y(vy) + 0.5);
        }
        above_solid = Some(solid);
    }
    println!(
        "terrain surfaces in this column (world y, top first): {:?}",
        surfaces
            .iter()
            .take(8)
            .map(|y| y.round())
            .collect::<Vec<_>>()
    );
    Ok(ExitCode::SUCCESS)
}
