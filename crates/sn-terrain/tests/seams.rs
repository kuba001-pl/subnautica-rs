//! Synthetic batches: no game files needed. Each test batch is a single
//! 32³ octree encoded with our own writer.

use std::collections::HashMap;

use sn_mesh::{Mesh, add_skirts, edge_report};
use sn_octree::{Batch, FORMAT_VERSION, GAME_CHILD_ORDER, Octree, Voxel, VoxelGrid};
use sn_terrain::{Neighbourhood, TerrainBatch, batch_field, batch_mesh};
use sn_world::BatchCoord;

const SIZE: i32 = 32;

/// A one-octree batch whose voxels sample a sphere given in global voxel
/// coordinates, encoded like the game (density 126+ solid, ~15 units/voxel).
fn sphere_batch(coord: BatchCoord, center: [f32; 3], radius: f32) -> TerrainBatch {
    let mut grid = VoxelGrid::default();
    for z in 0..32 {
        for y in 0..32 {
            for x in 0..32 {
                let g = [
                    coord.x * SIZE + x as i32,
                    coord.y * SIZE + y as i32,
                    coord.z * SIZE + z as i32,
                ];
                let dist: f32 = (0..3)
                    .map(|a| (g[a] as f32 - center[a]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                let density = (125.5 + 15.0 * (radius - dist)).clamp(1.0, 252.0) as u8;
                let ty = if density >= 126 { 7 } else { 0 };
                grid.set([x, y, z], Voxel { ty, density });
            }
        }
    }
    TerrainBatch {
        batch: Batch {
            version: FORMAT_VERSION,
            octrees: vec![Octree::from_voxels(&grid, GAME_CHILD_ORDER)],
        },
        octree_dims: [1, 1, 1],
    }
}

/// Eight batches around the corner they share, with a sphere crossing every
/// kind of seam.
fn eight_batches() -> HashMap<BatchCoord, TerrainBatch> {
    let center = [31.6, 32.3, 31.9];
    let mut batches = HashMap::new();
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let c = BatchCoord::new(x, y, z);
                batches.insert(c, sphere_batch(c, center, 13.2));
            }
        }
    }
    batches
}

fn mesh_all(
    batches: &HashMap<BatchCoord, TerrainBatch>,
    lod_of: impl Fn(BatchCoord) -> u32,
) -> Vec<Mesh> {
    batches
        .keys()
        .map(|&c| {
            let around = Neighbourhood::new(c, [SIZE; 3], |n| batches.get(&n));
            batch_mesh(&around, lod_of(c)).unwrap()
        })
        .collect()
}

fn weld(meshes: &[Mesh]) -> Mesh {
    let mut out = Mesh::default();
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    for mesh in meshes {
        for tri in &mesh.triangles {
            out.triangles.push(tri.map(|i| {
                let p = mesh.positions[i as usize];
                *ids.entry(p.map(f32::to_bits)).or_insert_with(|| {
                    out.positions.push(p);
                    out.positions.len() as u32 - 1
                })
            }));
        }
    }
    out
}

#[test]
fn sphere_across_eight_batches_is_closed_at_every_level_of_detail() {
    let batches = eight_batches();
    for lod in 0..=2 {
        let meshes = mesh_all(&batches, |_| lod);
        assert!(meshes.iter().all(|m| !m.triangles.is_empty()), "lod {lod}");
        let welded = weld(&meshes);
        let report = edge_report(&welded);
        assert_eq!(
            (report.boundary, report.inconsistent, report.non_manifold),
            (0, 0, 0),
            "lod {lod}: {report:?}"
        );
        assert_eq!(report.euler_characteristic(&welded), 2, "lod {lod}");
    }
}

#[test]
fn coarser_levels_have_fewer_triangles() {
    let batches = eight_batches();
    let count = |lod| -> usize {
        mesh_all(&batches, |_| lod)
            .iter()
            .map(|m| m.triangles.len())
            .sum()
    };
    let (l0, l1, l2) = (count(0), count(1), count(2));
    assert!(l0 > 3 * l1 && l1 > 3 * l2, "{l0} {l1} {l2}");
}

#[test]
fn mixed_levels_leave_cracks_that_skirts_cover() {
    // Half the batches fine, half coarse: the seam between them no longer
    // welds, so open edges appear inside the volume.
    let batches = eight_batches();
    let mut meshes = mesh_all(&batches, |c| if c.x == 0 { 0 } else { 1 });
    assert!(edge_report(&weld(&meshes)).boundary > 0);
    let quads: usize = meshes.iter_mut().map(|m| add_skirts(m, 2.0)).sum();
    assert!(quads > 0);
}

#[test]
fn missing_neighbours_are_clamped_without_adding_surface() {
    // A sphere well inside one batch with no neighbours at all.
    let c = BatchCoord::new(3, 2, 1);
    let batch = sphere_batch(c, [111.5, 79.5, 47.5], 6.5);
    let around = Neighbourhood::new(c, [SIZE; 3], |n| (n == c).then_some(&batch));
    let field = batch_field(&around, 0).unwrap();
    assert_eq!(field.clamped_samples, 34usize.pow(3) - 32usize.pow(3));
    let report = edge_report(&batch_mesh(&around, 0).unwrap());
    assert_eq!((report.boundary, report.inconsistent), (0, 0));
}

#[test]
fn absent_center_gives_nothing() {
    let around = Neighbourhood::new(BatchCoord::new(0, 0, 0), [SIZE; 3], |_| None);
    assert!(batch_field(&around, 0).is_none());
}
