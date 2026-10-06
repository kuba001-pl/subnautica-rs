//! Topology checks, so mesh quality can be verified from numbers.

use std::collections::HashMap;

use crate::Mesh;

/// How the triangle edges of a mesh are shared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeReport {
    /// Distinct undirected edges.
    pub edges: usize,
    /// Edges used by exactly one triangle: holes or open borders.
    pub boundary: usize,
    /// Edges used by exactly two triangles in opposite directions: good.
    pub manifold: usize,
    /// Edges used by two triangles in the same direction: flipped faces.
    pub inconsistent: usize,
    /// Edges used by more than two triangles.
    pub non_manifold: usize,
}

impl EdgeReport {
    /// Vertices − edges + faces. 2 for a closed surface without handles.
    pub fn euler_characteristic(&self, mesh: &Mesh) -> i64 {
        mesh.positions.len() as i64 - self.edges as i64 + mesh.triangles.len() as i64
    }
}

pub fn edge_report(mesh: &Mesh) -> EdgeReport {
    // Undirected edge → (uses as low→high, uses as high→low).
    let mut uses: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    for tri in &mesh.triangles {
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            let entry = uses.entry((a.min(b), a.max(b))).or_default();
            if a < b {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
        }
    }
    let mut report = EdgeReport {
        edges: uses.len(),
        ..Default::default()
    };
    for (forward, backward) in uses.into_values() {
        match (forward, backward) {
            (1, 0) | (0, 1) => report.boundary += 1,
            (1, 1) => report.manifold += 1,
            (2, 0) | (0, 2) => report.inconsistent += 1,
            _ => report.non_manifold += 1,
        }
    }
    report
}
