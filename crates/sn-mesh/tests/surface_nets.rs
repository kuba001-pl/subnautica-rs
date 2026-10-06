use std::collections::HashMap;

use sn_mesh::{Field, Mesh, edge_report, surface_nets};

fn field_from(dims: [usize; 3], f: impl Fn([f32; 3]) -> f32) -> Field {
    let mut field = Field::new(dims);
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let value = f([x as f32, y as f32, z as f32]);
                let material = if x < dims[0] / 2 { 1 } else { 2 };
                field.set([x, y, z], value, material);
            }
        }
    }
    field
}

fn sphere(dims: [usize; 3], center: [f32; 3], radius: f32) -> Field {
    field_from(dims, |p| {
        let d: f32 = (0..3)
            .map(|a| (p[a] - center[a]).powi(2))
            .sum::<f32>()
            .sqrt();
        radius - d
    })
}

fn triangle_normal(mesh: &Mesh, tri: [u32; 3]) -> ([f32; 3], [f32; 3]) {
    let [a, b, c] = tri.map(|i| mesh.positions[i as usize]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let centroid = [0, 1, 2].map(|i| (a[i] + b[i] + c[i]) / 3.0);
    (n, centroid)
}

#[test]
fn sphere_is_closed_oriented_and_faces_out() {
    let center = [10.3, 9.7, 10.1];
    let mesh = surface_nets(&sphere([21, 21, 21], center, 7.2));
    assert!(mesh.triangles.len() > 500);

    let report = edge_report(&mesh);
    assert_eq!(report.boundary, 0, "{report:?}");
    assert_eq!(report.inconsistent, 0, "{report:?}");
    assert_eq!(report.non_manifold, 0, "{report:?}");
    assert_eq!(report.euler_characteristic(&mesh), 2);

    for tri in &mesh.triangles {
        let (n, centroid) = triangle_normal(&mesh, *tri);
        let out: f32 = (0..3).map(|a| n[a] * (centroid[a] - center[a])).sum();
        assert!(out > 0.0, "triangle {tri:?} faces inward");
    }
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        let dist: f32 = (0..3)
            .map(|a| (p[a] - center[a]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            (dist - 7.2).abs() < 0.2,
            "vertex off the surface by {}",
            dist - 7.2
        );
        let out: f32 = (0..3).map(|a| n[a] * (p[a] - center[a]) / dist).sum();
        assert!(out > 0.9, "vertex normal not outward: {out}");
    }
}

#[test]
fn torus_has_one_handle() {
    let mesh = surface_nets(&field_from([30, 14, 30], |[x, y, z]| {
        let (dx, dz) = (x - 14.6, z - 14.4);
        let ring = (dx * dx + dz * dz).sqrt() - 9.0;
        3.3 - (ring * ring + (y - 6.8).powi(2)).sqrt()
    }));
    let report = edge_report(&mesh);
    assert_eq!(
        (report.boundary, report.inconsistent, report.non_manifold),
        (0, 0, 0)
    );
    assert_eq!(report.euler_characteristic(&mesh), 0);
}

#[test]
fn uniform_fields_have_no_surface() {
    assert!(surface_nets(&Field::new([8, 8, 8])).triangles.is_empty());
    let solid = field_from([8, 8, 8], |_| 1.0);
    assert!(surface_nets(&solid).triangles.is_empty());
    assert!(surface_nets(&Field::new([1, 5, 5])).triangles.is_empty());
}

#[test]
fn triangles_take_the_material_of_the_solid_side() {
    let mesh = surface_nets(&sphere([21, 21, 21], [10.0, 10.0, 10.0], 7.0));
    for (tri, material) in mesh.triangles.iter().zip(&mesh.triangle_materials) {
        let (_, centroid) = triangle_normal(&mesh, *tri);
        if (centroid[0] - 10.0).abs() > 1.5 {
            let expected = if centroid[0] < 10.0 { 1 } else { 2 };
            assert_eq!(*material, expected, "at {centroid:?}");
        }
    }
}

/// Neighbouring fields that overlap by 2 samples must mesh into pieces that
/// join exactly: after welding equal positions the union is closed.
#[test]
fn neighbouring_fields_join_without_gaps() {
    let full = sphere([25, 21, 21], [12.3, 10.2, 9.8], 7.5);
    let whole = surface_nets(&full);
    // Piece A owns samples 1..12, piece B owns 12..24 (each plus 1-sample apron).
    let mut parts = Vec::new();
    // A: samples 0..=12, B: samples 11..=24 — a 2-sample overlap.
    for (start, len) in [(0, 13), (11, 14)] {
        let mut part = Field::new([len, 21, 21]).with_origin([start as i32, 0, 0]);
        for z in 0..21 {
            for y in 0..21 {
                for x in 0..len {
                    let p = [start + x, y, z];
                    part.set([x, y, z], full.value(p), full.material(p));
                }
            }
        }
        let mesh = surface_nets(&part);
        assert!(
            edge_report(&mesh).boundary > 0,
            "each piece is open on its own"
        );
        parts.push(mesh);
    }

    // Weld bit-identical positions; drop vertices no triangle uses.
    let mut welded = Mesh::default();
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    for mesh in &parts {
        for tri in &mesh.triangles {
            welded.triangles.push(tri.map(|i| {
                let p = mesh.positions[i as usize];
                *ids.entry(p.map(f32::to_bits)).or_insert_with(|| {
                    welded.positions.push(p);
                    welded.positions.len() as u32 - 1
                })
            }));
        }
    }
    let report = edge_report(&welded);
    assert_eq!(report.boundary, 0, "gap along the seam: {report:?}");
    assert_eq!((report.inconsistent, report.non_manifold), (0, 0));
    assert_eq!(report.euler_characteristic(&welded), 2);
    assert_eq!(welded.triangles.len(), whole.triangles.len());
}

#[test]
fn origin_offsets_positions() {
    let at_zero = surface_nets(&sphere([12, 12, 12], [5.5, 5.5, 5.5], 3.0));
    let moved =
        surface_nets(&sphere([12, 12, 12], [5.5, 5.5, 5.5], 3.0).with_origin([100, -50, 7]));
    assert_eq!(at_zero.triangles, moved.triangles);
    for (a, b) in at_zero.positions.iter().zip(&moved.positions) {
        let expected = [a[0] + 100.0, a[1] - 50.0, a[2] + 7.0];
        assert!(
            (0..3).all(|i| (expected[i] - b[i]).abs() < 1e-4),
            "{expected:?} vs {b:?}"
        );
    }
}
