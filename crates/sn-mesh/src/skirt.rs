//! Skirts: strips hanging from a mesh's open edges down into the solid.
//!
//! Where a chunk meets a neighbour meshed at a different level of detail, the
//! two surfaces don't quite line up and a crack opens. A skirt under each
//! open edge fills the crack with solid-coloured geometry. Between chunks at
//! the same level of detail there is no crack, and the skirt stays hidden
//! inside the rock.

use std::collections::HashMap;

use crate::Mesh;

/// How an undirected edge is used: (triangles using it, the directed edge as
/// first seen, that triangle's material).
type EdgeUse = (u32, (u32, u32), u8);

/// Adds a skirt under every edge used by exactly one triangle: a quad from
/// the edge to a copy of it moved `depth` units against the vertex normals
/// (into the solid). Skirts continue the surface's winding. Returns the
/// number of skirt quads added.
pub fn add_skirts(mesh: &mut Mesh, depth: f32) -> usize {
    let mut edges: HashMap<(u32, u32), EdgeUse> = HashMap::new();
    for (tri, &material) in mesh.triangles.iter().zip(&mesh.triangle_materials) {
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            edges
                .entry((a.min(b), a.max(b)))
                .or_insert((0, (a, b), material))
                .0 += 1;
        }
    }
    let mut open: Vec<((u32, u32), u8)> = edges
        .into_values()
        .filter(|(uses, _, _)| *uses == 1)
        .map(|(_, edge, material)| (edge, material))
        .collect();
    open.sort_unstable(); // deterministic output

    let mut sunk: HashMap<u32, u32> = HashMap::new();
    let mut sink = |mesh: &mut Mesh, v: u32| -> u32 {
        *sunk.entry(v).or_insert_with(|| {
            let p = mesh.positions[v as usize];
            let n = mesh.normals[v as usize];
            mesh.positions.push([0, 1, 2].map(|a| p[a] - n[a] * depth));
            mesh.normals.push(n);
            mesh.positions.len() as u32 - 1
        })
    };
    for &((a, b), material) in &open {
        let (a2, b2) = (sink(mesh, a), sink(mesh, b));
        // The existing triangle uses a→b, so the skirt uses b→a.
        mesh.triangles.push([b, a, a2]);
        mesh.triangles.push([b, a2, b2]);
        mesh.triangle_materials.extend([material, material]);
    }
    open.len()
}
