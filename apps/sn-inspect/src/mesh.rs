//! `mesh` command: mesh terrain batches and export them as OBJ to `out/`.
//!
//! Besides writing the file, it checks the result from numbers:
//! - holes: after joining the batch meshes, every open edge must lie on the
//!   outside of the exported region (or next to a batch with no file);
//! - orientation: triangles must face away from the solid, i.e. a step along
//!   the normal must land in empty space.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use sn_install::GameData;
use sn_mesh::{Mesh, edge_report, surface_nets};
use sn_terrain::{MAX_LOD, Neighbourhood, batch_field, batch_voxels, debug_colour};
use sn_world::{BatchCoord, VOXEL_WORLD_OFFSET, voxel_to_world};

use crate::Result;

/// A batch face whose neighbour was not meshed: open edges near it are expected.
struct OpenFace {
    axis: usize,
    /// How far from the face open edges may lie: 1.5 samples.
    tolerance: f32,
    plane: f32,
    min: [f32; 3],
    max: [f32; 3],
}

impl OpenFace {
    fn contains(&self, p: [f32; 3]) -> bool {
        let t = self.tolerance;
        (p[self.axis] - self.plane).abs() <= t
            && (0..3).all(|a| a == self.axis || (self.min[a] - t..=self.max[a] + t).contains(&p[a]))
    }
}

pub fn run(game: &GameData, center: BatchCoord, radius: i32, lod: u32) -> Result<ExitCode> {
    if lod > MAX_LOD {
        return Err(format!("--lod must be 0..={MAX_LOD}"));
    }
    let start = Instant::now();
    let index = game.read_index()?;
    let size = batch_voxels(&index);

    // Load the region plus a 1-batch margin (the margin feeds the aprons).
    let mut grids = HashMap::new();
    let reach = radius + 1;
    for dz in -reach..=reach {
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let c = center.offset(dx, dy, dz);
                if let Some(grid) = game.load_batch(&index, c)? {
                    grids.insert(c, grid);
                }
            }
        }
    }
    let mut coords: Vec<BatchCoord> = grids
        .keys()
        .copied()
        .filter(|c| {
            [c.x - center.x, c.y - center.y, c.z - center.z]
                .iter()
                .all(|d| d.abs() <= radius)
        })
        .collect();
    coords.sort();
    if coords.is_empty() {
        return Err(format!("no batch files within radius {radius} of {center}"));
    }
    let exported: HashSet<BatchCoord> = coords.iter().copied().collect();
    println!(
        "meshing {} batches around {center} (radius {radius}, level of detail {lod})...",
        coords.len()
    );

    let mut combined = Mesh::default();
    let mut open_faces = Vec::new();
    let (mut facing_out, mut facing_checked) = (0u64, 0u64);
    for &coord in &coords {
        let around = Neighbourhood::new(coord, size, |n| grids.get(&n));
        let Some(batch_field) = batch_field(&around, lod) else {
            continue;
        };
        let field = &batch_field.field;
        let mesh = surface_nets(field);

        // Orientation: step 1 voxel along each triangle's normal.
        let origin = field.origin();
        for tri in &mesh.triangles {
            let (n, centroid) = triangle_normal(&mesh, *tri);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len == 0.0 {
                continue;
            }
            let step = field.step() as f32;
            let probe = [0, 1, 2]
                .map(|a| ((centroid[a] + step * n[a] / len - origin[a] as f32) / step).round());
            if (0..3).all(|a| probe[a] >= 0.0 && (probe[a] as usize) < field.dims()[a]) {
                facing_checked += 1;
                if field.value(probe.map(|p| p as usize)) <= 0.0 {
                    facing_out += 1;
                }
            }
        }

        let dims = grids[&coord].voxel_dims();
        let lo = [coord.x, coord.y, coord.z].map(|c| c as f32);
        let lo = [0, 1, 2].map(|a| lo[a] * size[a] as f32);
        let hi = [0, 1, 2].map(|a| lo[a] + dims[a] as f32);
        for axis in 0..3 {
            for (step, plane) in [(-1, lo[axis]), (1, hi[axis])] {
                let mut d = [0; 3];
                d[axis] = step;
                if !exported.contains(&coord.offset(d[0], d[1], d[2])) {
                    open_faces.push(OpenFace {
                        axis,
                        tolerance: 1.5 * (1u32 << lod) as f32,
                        plane,
                        min: lo,
                        max: hi,
                    });
                }
            }
        }

        println!(
            "  {coord}: {:>7} vertices, {:>7} triangles, {} apron samples from missing neighbours",
            mesh.positions.len(),
            mesh.triangles.len(),
            batch_field.clamped_samples
        );
        append(&mut combined, mesh);
    }

    // Join the batch meshes by welding bit-identical positions, then look for holes.
    let welded = weld(&combined);
    let report = edge_report(&welded);
    let unexpected_open = count_unexpected_open_edges(&welded, &open_faces);
    let facing = facing_out as f64 / facing_checked.max(1) as f64;

    std::fs::create_dir_all("out").map_err(|e| format!("out/: {e}"))?;
    let name = format!(
        "terrain-{}-{}-{}-r{radius}-lod{lod}",
        center.x, center.y, center.z
    );
    let obj_path = Path::new("out").join(format!("{name}.obj"));
    write_obj(&combined, &obj_path, &name).map_err(|e| format!("{}: {e}", obj_path.display()))?;

    println!();
    println!(
        "vertices:         {} ({} after welding batch seams)",
        combined.positions.len(),
        welded.positions.len()
    );
    println!("triangles:        {}", combined.triangles.len());
    println!(
        "edges:            {} manifold, {} open, {} flipped, {} shared by 3+ triangles",
        report.manifold, report.boundary, report.inconsistent, report.non_manifold
    );
    println!("open edges away from the region border / missing batches: {unexpected_open}");
    println!(
        "facing out:       {:.3}% of {facing_checked} triangles have empty space 1 voxel along their normal",
        facing * 100.0
    );
    println!("wrote:            {}", obj_path.display());
    println!("time:             {:.2} s", start.elapsed().as_secs_f64());
    println!("(OBJ is Y-up; Unity's left-handed z was flipped. Positions use the unverified");
    println!(
        " world offset hypothesis voxel - {VOXEL_WORLD_OFFSET:?}. Never commit or share out/.)"
    );

    // Coarse levels cross thin features more often when stepping one sample
    // along the normal; a flipped mesh would score near 0 either way.
    let min_facing = if lod == 0 { 0.98 } else { 0.90 };
    let ok = unexpected_open == 0 && report.inconsistent == 0 && facing > min_facing;
    println!(
        "result:           {}",
        if ok { "OK" } else { "PROBLEMS (see above)" }
    );
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
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
    (n, [0, 1, 2].map(|i| (a[i] + b[i] + c[i]) / 3.0))
}

fn append(into: &mut Mesh, mesh: Mesh) {
    let offset = into.positions.len() as u32;
    into.positions.extend(mesh.positions);
    into.normals.extend(mesh.normals);
    into.triangles
        .extend(mesh.triangles.iter().map(|t| t.map(|i| i + offset)));
    into.triangle_materials.extend(mesh.triangle_materials);
}

/// Merges vertices with bit-identical positions and drops unused ones.
fn weld(mesh: &Mesh) -> Mesh {
    let mut out = Mesh::default();
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    for tri in &mesh.triangles {
        out.triangles.push(tri.map(|i| {
            let p = mesh.positions[i as usize];
            *ids.entry(p.map(f32::to_bits)).or_insert_with(|| {
                out.positions.push(p);
                out.positions.len() as u32 - 1
            })
        }));
    }
    out
}

fn count_unexpected_open_edges(mesh: &Mesh, open_faces: &[OpenFace]) -> usize {
    let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
    for tri in &mesh.triangles {
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    uses.into_iter()
        .filter(|(_, n)| *n == 1)
        .filter(|((a, b), _)| {
            let (pa, pb) = (mesh.positions[*a as usize], mesh.positions[*b as usize]);
            let mid = [0, 1, 2].map(|i| (pa[i] + pb[i]) / 2.0);
            !open_faces.iter().any(|f| f.contains(mid))
        })
        .count()
}

/// Sample-grid position → OBJ position: voxel centre, world offset, and
/// Unity (left-handed, y up) → OBJ (right-handed, y up) by flipping z.
fn to_obj(p: [f32; 3]) -> [f32; 3] {
    let w = voxel_to_world(p);
    [w[0], w[1], -w[2]]
}

fn write_obj(mesh: &Mesh, path: &Path, name: &str) -> std::io::Result<()> {
    let mtl_name = format!("{name}.mtl");
    let mut materials: Vec<u8> = mesh.triangle_materials.clone();
    materials.sort_unstable();
    materials.dedup();

    let mut mtl = BufWriter::new(File::create(path.with_extension("mtl"))?);
    writeln!(
        mtl,
        "# Terrain material ids, false colours. Generated locally."
    )?;
    for &ty in &materials {
        let [r, g, b] = debug_colour(ty);
        writeln!(mtl, "newmtl type_{ty}\nKd {r:.3} {g:.3} {b:.3}\n")?;
    }
    mtl.flush()?;

    let mut out = BufWriter::new(File::create(path)?);
    writeln!(
        out,
        "# subnautica-rs terrain export, generated from YOUR local game files."
    )?;
    writeln!(out, "# Do not commit or redistribute.")?;
    writeln!(out, "mtllib {mtl_name}")?;
    writeln!(out, "o {name}")?;
    for p in &mesh.positions {
        let [x, y, z] = to_obj(*p);
        writeln!(out, "v {x:.3} {y:.3} {z:.3}")?;
    }
    for n in &mesh.normals {
        writeln!(out, "vn {:.3} {:.3} {:.3}", n[0], n[1], -n[2])?;
    }
    let mut order: Vec<usize> = (0..mesh.triangles.len()).collect();
    order.sort_by_key(|&i| mesh.triangle_materials[i]);
    let mut current = None;
    for i in order {
        let ty = mesh.triangle_materials[i];
        if current != Some(ty) {
            writeln!(out, "usemtl type_{ty}")?;
            current = Some(ty);
        }
        // Flipping z mirrors the mesh, so reverse the winding to keep it facing out.
        let [a, b, c] = mesh.triangles[i].map(|v| v + 1);
        writeln!(out, "f {a}//{a} {c}//{c} {b}//{b}")?;
    }
    out.flush()
}
