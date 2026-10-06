//! Material layers: how the game blends terrain materials.
//!
//! The game draws a terrain chunk once per block type used in it, in a fixed
//! order (block-type `layer`, then type id). The first draw is opaque and
//! covers every face. Each later draw covers only the faces near its type
//! and is alpha-blended on top, with a per-vertex weight that the material's
//! shader turns into alpha (see `docs/formats/terrain-materials.md`).
//!
//! - A vertex's weight for type `T` is the share of its adjacent faces whose
//!   type is `T`.
//! - Faces (the quads of the mesh) are grouped into cubic chunks; the order
//!   of draws, and which type goes first, is decided per chunk.
//! - Near the camera every face is split into 8 triangles around 9 vertices
//!   (corners, edge midpoints, centre), so a lone face of one type still
//!   reaches full weight at its centre.
//!
//! Pure: no I/O, no engine types.

use std::collections::HashMap;

use crate::Mesh;

/// What the layer builder needs to know about block types.
pub struct LayerSettings<'a> {
    /// `VoxelandBlockType.layer` per type id; lower draws first.
    pub layer: &'a [i32; 256],
    /// Gloss per type id, averaged over a vertex's faces.
    pub gloss: &'a [f32; 256],
    /// Chunk edge length in mesh units. A face belongs to the chunk holding
    /// the solid cell behind it: its centre moved half a `cell` inwards.
    pub chunk: f32,
    /// Sample spacing in mesh units.
    pub cell: f32,
    /// Split faces into 9 vertices (the game's high-resolution chunks).
    pub subdivide: bool,
    /// At most this many types get a draw per chunk: the ones with the most
    /// faces. Faces of the others show the chunk's first type.
    pub max_types: usize,
}

/// The faces of one draw: one material at one place in the chunk order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerMesh {
    pub material: u8,
    /// Position in its chunk's draw order; 0 is the opaque first draw.
    pub rank: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// The vertex's weight for `material` (1 everywhere for rank 0).
    pub blend: Vec<f32>,
    pub gloss: Vec<f32>,
    /// Same winding as the source mesh.
    pub triangles: Vec<[u32; 3]>,
}

/// A face: four corners counter-clockwise (or three, with the last repeated,
/// for a lone triangle) and its block type.
struct Face {
    corners: [u32; 4],
    material: u8,
    quad: bool,
}

/// Recovers the mesh's faces. Surface nets and skirts emit quads as two
/// triangles `[a, b, c]`, `[a, c, d]`; anything else counts as a lone
/// triangle.
fn faces(mesh: &Mesh) -> Vec<Face> {
    let mut out = Vec::with_capacity(mesh.triangles.len() / 2);
    let mut i = 0;
    while i < mesh.triangles.len() {
        let t = mesh.triangles[i];
        let material = mesh.triangle_materials.get(i).copied().unwrap_or(0);
        if let (Some(u), Some(&m2)) = (
            mesh.triangles.get(i + 1),
            mesh.triangle_materials.get(i + 1),
        ) && u[0] == t[0]
            && u[1] == t[2]
            && m2 == material
        {
            out.push(Face {
                corners: [t[0], t[1], t[2], u[2]],
                material,
                quad: true,
            });
            i += 2;
        } else {
            out.push(Face {
                corners: [t[0], t[1], t[2], t[2]],
                material,
                quad: false,
            });
            i += 1;
        }
    }
    out
}

impl Face {
    fn corners(&self) -> &[u32] {
        if self.quad {
            &self.corners
        } else {
            &self.corners[..3]
        }
    }
}

/// Where a vertex of a layer mesh comes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    Corner(u32),
    /// Midpoint of the edge between two corners (smaller index first).
    Mid(u32, u32),
    Centre(u32),
}

/// Faces around each corner vertex, in compressed rows.
struct Adjacency {
    start: Vec<u32>,
    faces: Vec<u32>,
}

impl Adjacency {
    fn new(vertices: usize, faces: &[Face]) -> Adjacency {
        let mut count = vec![0u32; vertices + 1];
        for face in faces {
            for &v in face.corners() {
                count[v as usize + 1] += 1;
            }
        }
        for i in 1..count.len() {
            count[i] += count[i - 1];
        }
        let mut fill = count.clone();
        let mut list = vec![0u32; *count.last().unwrap_or(&0) as usize];
        for (f, face) in faces.iter().enumerate() {
            for &v in face.corners() {
                list[fill[v as usize] as usize] = f as u32;
                fill[v as usize] += 1;
            }
        }
        Adjacency {
            start: count,
            faces: list,
        }
    }

    fn around(&self, v: u32) -> &[u32] {
        let (a, b) = (self.start[v as usize], self.start[v as usize + 1]);
        &self.faces[a as usize..b as usize]
    }
}

/// Up to two faces inline, or a borrowed list.
enum Around<'a> {
    Slice(&'a [u32]),
    Two([u32; 2], usize),
}

impl Around<'_> {
    fn as_slice(&self) -> &[u32] {
        match self {
            Around::Slice(s) => s,
            Around::Two(f, n) => &f[..*n],
        }
    }
}

/// Builds the game's material layers for `mesh`.
///
/// Output is sorted by (rank, material); meshes with the same material and
/// rank from different chunks are merged.
pub fn build_layers(mesh: &Mesh, settings: &LayerSettings) -> Vec<LayerMesh> {
    let faces = faces(mesh);
    if faces.is_empty() {
        return Vec::new();
    }
    let adjacency = Adjacency::new(mesh.positions.len(), &faces);

    // Faces sharing each edge (for the edge midpoints).
    let mut edge_faces: HashMap<(u32, u32), [u32; 2]> = HashMap::new();
    if settings.subdivide {
        for (f, face) in faces.iter().enumerate() {
            let c = face.corners();
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                let entry = edge_faces
                    .entry((a.min(b), a.max(b)))
                    .or_insert([u32::MAX; 2]);
                if entry[0] == u32::MAX {
                    entry[0] = f as u32;
                } else {
                    entry[1] = f as u32;
                }
            }
        }
    }

    // Faces around a layer vertex, without allocating.
    let faces_of = |source: Source| -> Around<'_> {
        match source {
            Source::Corner(v) => Around::Slice(adjacency.around(v)),
            Source::Mid(a, b) => match edge_faces.get(&(a, b)) {
                Some(&[f, u32::MAX]) => Around::Two([f, f], 1),
                Some(&pair) => Around::Two(pair, 2),
                None => Around::Two([0, 0], 0),
            },
            Source::Centre(f) => Around::Two([f, f], 1),
        }
    };
    let weight = |source: Source, material: u8| -> f32 {
        let around = faces_of(source);
        let around = around.as_slice();
        if around.is_empty() {
            return 0.0;
        }
        let same = around
            .iter()
            .filter(|&&f| faces[f as usize].material == material)
            .count();
        same as f32 / around.len() as f32
    };
    let gloss = |source: Source| -> f32 {
        let around = faces_of(source);
        let around = around.as_slice();
        if around.is_empty() {
            return 0.0;
        }
        around
            .iter()
            .map(|&f| settings.gloss[usize::from(faces[f as usize].material)])
            .sum::<f32>()
            / around.len() as f32
    };
    let corner_has = |v: u32, material: u8| {
        adjacency
            .around(v)
            .iter()
            .any(|&f| faces[f as usize].material == material)
    };

    // Faces per chunk, by face centre.
    let mut chunks: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
    for (f, face) in faces.iter().enumerate() {
        let c = face.corners();
        let mut centre = [0.0f32; 3];
        let mut normal = [0.0f32; 3];
        for &v in c {
            for a in 0..3 {
                centre[a] += mesh.positions[v as usize][a] / c.len() as f32;
                normal[a] += mesh.normals[v as usize][a];
            }
        }
        let normal = normalize(normal);
        let key: [i32; 3] = std::array::from_fn(|a| {
            ((centre[a] - 0.5 * settings.cell * normal[a]) / settings.chunk).floor() as i32
        });
        chunks.entry(key).or_default().push(f as u32);
    }

    let mut out: HashMap<(u8, u8), (LayerMesh, HashMap<Source, u32>)> = HashMap::new();
    let mut keys: Vec<[i32; 3]> = chunks.keys().copied().collect();
    keys.sort_unstable(); // deterministic output
    for key in keys {
        let chunk = &chunks[&key];
        let mut count = [0u32; 256];
        for &f in chunk {
            count[usize::from(faces[f as usize].material)] += 1;
        }
        let mut used: Vec<u8> = (0..=255u8).filter(|&t| count[usize::from(t)] > 0).collect();
        if used.len() > settings.max_types.max(1) {
            used.sort_by_key(|&t| (std::cmp::Reverse(count[usize::from(t)]), t));
            used.truncate(settings.max_types.max(1));
        }
        used.sort_by_key(|&t| (settings.layer[usize::from(t)], t));
        for (rank, &material) in used.iter().enumerate() {
            let rank = rank.min(255) as u8;
            let (layer, remap) = out.entry((material, rank)).or_insert_with(|| {
                (
                    LayerMesh {
                        material,
                        rank,
                        ..LayerMesh::default()
                    },
                    HashMap::new(),
                )
            });
            for &f in chunk {
                let face = &faces[f as usize];
                if rank > 0
                    && face.material != material
                    && !face.corners().iter().any(|&v| corner_has(v, material))
                {
                    continue;
                }
                let mut vertex = |source: Source| -> u32 {
                    *remap.entry(source).or_insert_with(|| {
                        let points: Vec<u32> = match source {
                            Source::Corner(v) => vec![v],
                            Source::Mid(a, b) => vec![a, b],
                            Source::Centre(_) => face.corners().to_vec(),
                        };
                        let mut p = [0.0f32; 3];
                        let mut n = [0.0f32; 3];
                        for &v in &points {
                            for a in 0..3 {
                                p[a] += mesh.positions[v as usize][a];
                                n[a] += mesh.normals[v as usize][a];
                            }
                        }
                        layer.positions.push(p.map(|x| x / points.len() as f32));
                        layer.normals.push(normalize(n));
                        layer.blend.push(if rank == 0 {
                            1.0
                        } else {
                            weight(source, material)
                        });
                        layer.gloss.push(gloss(source));
                        layer.positions.len() as u32 - 1
                    })
                };
                let c = face.corners();
                if settings.subdivide && face.quad {
                    let corner = c
                        .iter()
                        .map(|&v| vertex(Source::Corner(v)))
                        .collect::<Vec<_>>();
                    let mid = (0..4)
                        .map(|i| {
                            let (a, b) = (c[i], c[(i + 1) % 4]);
                            vertex(Source::Mid(a.min(b), a.max(b)))
                        })
                        .collect::<Vec<_>>();
                    let centre = vertex(Source::Centre(f));
                    for i in 0..4 {
                        layer.triangles.push([corner[i], mid[i], centre]);
                        layer.triangles.push([mid[i], corner[(i + 1) % 4], centre]);
                    }
                } else {
                    let v: Vec<u32> = c.iter().map(|&v| vertex(Source::Corner(v))).collect();
                    layer.triangles.push([v[0], v[1], v[2]]);
                    if face.quad {
                        layer.triangles.push([v[0], v[2], v[3]]);
                    }
                }
            }
        }
    }
    let mut layers: Vec<LayerMesh> = out
        .into_values()
        .map(|(layer, _)| layer)
        .filter(|l| !l.triangles.is_empty())
        .collect();
    layers.sort_by_key(|l| (l.rank, l.material));
    layers
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        v.map(|c| c / len)
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid of `n`×`n` unit quads in the XZ plane facing +Y, with the
    /// material of quad (x, z) from `material`.
    fn grid(n: u32, material: impl Fn(u32, u32) -> u8) -> Mesh {
        let mut mesh = Mesh::default();
        for z in 0..=n {
            for x in 0..=n {
                mesh.positions.push([x as f32, 0.0, z as f32]);
                mesh.normals.push([0.0, 1.0, 0.0]);
            }
        }
        let at = |x: u32, z: u32| z * (n + 1) + x;
        for z in 0..n {
            for x in 0..n {
                // Counter-clockwise seen from +Y.
                let q = [at(x, z), at(x, z + 1), at(x + 1, z + 1), at(x + 1, z)];
                let m = material(x, z);
                mesh.triangles.push([q[0], q[1], q[2]]);
                mesh.triangles.push([q[0], q[2], q[3]]);
                mesh.triangle_materials.extend([m, m]);
            }
        }
        mesh
    }

    fn settings<'a>(
        layer: &'a [i32; 256],
        gloss: &'a [f32; 256],
        subdivide: bool,
    ) -> LayerSettings<'a> {
        LayerSettings {
            layer,
            gloss,
            chunk: 16.0,
            cell: 1.0,
            subdivide,
            max_types: 32,
        }
    }

    #[test]
    fn single_material_is_one_opaque_layer() {
        let mesh = grid(4, |_, _| 7);
        let (layer, gloss) = ([0; 256], [0.5; 256]);
        let layers = build_layers(&mesh, &settings(&layer, &gloss, false));
        assert_eq!(layers.len(), 1);
        assert_eq!((layers[0].material, layers[0].rank), (7, 0));
        assert_eq!(layers[0].triangles.len(), 32);
        assert!(layers[0].blend.iter().all(|&b| b == 1.0));
        assert!(layers[0].gloss.iter().all(|&g| g == 0.5));
    }

    #[test]
    fn order_is_layer_then_type_id() {
        // Type 9 covers more faces, but 3 has the lower id: 3 goes first.
        let mesh = grid(4, |x, _| if x == 0 { 3 } else { 9 });
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let layers = build_layers(&mesh, &settings(&layer, &gloss, false));
        let order: Vec<(u8, u8)> = layers.iter().map(|l| (l.material, l.rank)).collect();
        assert_eq!(order, vec![(3, 0), (9, 1)]);

        // A lower `layer` wins over the type id.
        let mut layer = [0; 256];
        layer[9] = -100;
        let layers = build_layers(&mesh, &settings(&layer, &gloss, false));
        let order: Vec<(u8, u8)> = layers.iter().map(|l| (l.material, l.rank)).collect();
        assert_eq!(order, vec![(9, 0), (3, 1)]);
    }

    #[test]
    fn weights_are_shares_of_adjacent_faces() {
        // Column x = 0 is type 1, the rest type 2: overlay 2 covers its own
        // faces plus the faces of type 1 touching them.
        let mesh = grid(4, |x, _| if x == 0 { 1 } else { 2 });
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let layers = build_layers(&mesh, &settings(&layer, &gloss, false));
        let base = &layers[0];
        assert_eq!(base.triangles.len(), 32);
        let overlay = &layers[1];
        assert_eq!(overlay.material, 2);
        assert_eq!(overlay.triangles.len(), 32);
        for (p, &w) in overlay.positions.iter().zip(&overlay.blend) {
            let expected = match (p[0], p[2]) {
                (0.0, _) => 0.0,
                // Interior vertices on the border touch 2 faces of each type,
                // grid-edge ones 1 of each.
                (1.0, _) => 0.5,
                _ => 1.0,
            };
            assert_eq!(w, expected, "vertex {p:?}");
        }
    }

    #[test]
    fn lone_face_peaks_at_its_centre_when_subdivided() {
        let mesh = grid(3, |x, z| if (x, z) == (1, 1) { 5 } else { 4 });
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let lo = build_layers(&mesh, &settings(&layer, &gloss, false));
        let hi = build_layers(&mesh, &settings(&layer, &gloss, true));
        let max = |l: &LayerMesh| l.blend.iter().copied().fold(0.0, f32::max);
        // Corners touch 4 faces, one of them type 5.
        assert_eq!(max(&lo[1]), 0.25);
        assert_eq!(max(&hi[1]), 1.0);
        // 9 faces → 72 triangles in the base layer, 8 per face.
        assert_eq!(hi[0].triangles.len(), 72);
        // The overlay covers every face touching the lone face: all 9.
        assert_eq!(hi[1].triangles.len(), 72);
        // Shared edge midpoints are not duplicated: 16 corners + 24 edges + 9 centres.
        assert_eq!(hi[0].positions.len(), 16 + 24 + 9);
    }

    #[test]
    fn subdivided_faces_keep_their_area_and_winding() {
        let mesh = grid(2, |_, _| 1);
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let hi = build_layers(&mesh, &settings(&layer, &gloss, true));
        let mut area = 0.0;
        for t in &hi[0].triangles {
            let [a, b, c] = t.map(|i| hi[0].positions[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            // y component of u × v: positive when counter-clockwise seen from +Y.
            let cross_y = u[2] * v[0] - u[0] * v[2];
            assert!(cross_y > 0.0);
            area += cross_y / 2.0;
        }
        assert!((area - 4.0).abs() < 1e-5);
    }

    #[test]
    fn chunks_order_their_own_materials() {
        // Two chunks of 2×4 faces: left all type 8, right types 6 and 8.
        // Type 8 is the base on the left but an overlay on the right.
        let mesh = grid(4, |x, z| if x >= 2 && z == 0 { 6 } else { 8 });
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let mut s = settings(&layer, &gloss, false);
        s.chunk = 2.0;
        let layers = build_layers(&mesh, &s);
        let order: Vec<(u8, u8)> = layers.iter().map(|l| (l.material, l.rank)).collect();
        assert_eq!(order, vec![(6, 0), (8, 0), (8, 1)]);
    }

    #[test]
    fn lone_triangles_are_kept() {
        let mut mesh = grid(1, |_, _| 2);
        mesh.positions.push([0.0, 1.0, 0.0]);
        mesh.normals.push([0.0, 1.0, 0.0]);
        mesh.triangles.push([0, 1, 4]);
        mesh.triangle_materials.push(2);
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let layers = build_layers(&mesh, &settings(&layer, &gloss, true));
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].triangles.len(), 8 + 1);
    }

    #[test]
    fn type_cap_keeps_the_most_used_types() {
        // Type 1: 1 face, type 2: 3 faces, type 3: 12 faces. Capped at 2,
        // type 1 gets no draw and its face shows type 2 (first by id).
        let mesh = grid(4, |x, z| match (x, z) {
            (0, 0) => 1,
            (1..=3, 0) => 2,
            _ => 3,
        });
        let (layer, gloss) = ([0; 256], [0.0; 256]);
        let mut s = settings(&layer, &gloss, false);
        s.max_types = 2;
        let layers = build_layers(&mesh, &s);
        let order: Vec<(u8, u8)> = layers.iter().map(|l| (l.material, l.rank)).collect();
        assert_eq!(order, vec![(2, 0), (3, 1)]);
        assert_eq!(layers[0].triangles.len(), 32);
        s.max_types = 1;
        let layers = build_layers(&mesh, &s);
        let order: Vec<(u8, u8)> = layers.iter().map(|l| (l.material, l.rank)).collect();
        assert_eq!(order, vec![(3, 0)]);
    }
}
