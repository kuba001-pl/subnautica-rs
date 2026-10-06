//! Synthetic batches: no game files needed.

use std::collections::HashMap;

use sn_mesh::{Mesh, edge_report};
use sn_octree::{BatchGrid, Voxel};
use sn_terrain::{Neighbourhood, batch_field, batch_mesh};
use sn_world::BatchCoord;

const SIZE: i32 = 8;

/// A batch whose voxels sample a sphere given in global voxel coordinates,
/// encoded like the game does (density 126+ solid, ~15 units per voxel).
fn sphere_batch(coord: BatchCoord, center: [f32; 3], radius: f32) -> BatchGrid {
    let n = SIZE as usize;
    let mut voxels = Vec::with_capacity(n * n * n);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
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
                voxels.push(Voxel { ty, density });
            }
        }
    }
    BatchGrid {
        dims: [n, n, n],
        voxels,
    }
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
fn sphere_across_eight_batches_is_closed() {
    // Centred on the corner shared by 8 batches, so it crosses every kind of seam.
    let center = [7.6, 8.3, 7.9];
    let mut grids = HashMap::new();
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let c = BatchCoord::new(x, y, z);
                grids.insert(c, sphere_batch(c, center, 5.2));
            }
        }
    }
    let meshes: Vec<Mesh> = grids
        .keys()
        .map(|&c| {
            let around = Neighbourhood::new(c, [SIZE; 3], |n| grids.get(&n));
            batch_mesh(&around).unwrap()
        })
        .collect();
    assert!(meshes.iter().all(|m| !m.triangles.is_empty()));
    let welded = weld(&meshes);
    let report = edge_report(&welded);
    assert_eq!(
        (report.boundary, report.inconsistent, report.non_manifold),
        (0, 0, 0),
        "{report:?}"
    );
    assert_eq!(report.euler_characteristic(&welded), 2);
}

#[test]
fn missing_neighbours_are_clamped_without_adding_surface() {
    // A sphere well inside one batch with no neighbours at all.
    let c = BatchCoord::new(3, 2, 1);
    let grid = sphere_batch(c, [27.5, 19.5, 11.5], 2.5);
    let around = Neighbourhood::new(c, [SIZE; 3], |n| (n == c).then_some(&grid));
    let field = batch_field(&around).unwrap();
    let apron = 10usize.pow(3) - 8usize.pow(3);
    assert_eq!(field.clamped_samples, apron);
    let report = edge_report(&batch_mesh(&around).unwrap());
    assert_eq!((report.boundary, report.inconsistent), (0, 0));
}

#[test]
fn absent_center_gives_nothing() {
    let around = Neighbourhood::new(BatchCoord::new(0, 0, 0), [SIZE; 3], |_| None);
    assert!(batch_field(&around).is_none());
}
