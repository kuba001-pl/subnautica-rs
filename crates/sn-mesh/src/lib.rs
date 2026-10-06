//! Turns a signed scalar field into a triangle mesh using surface nets.
//!
//! Convention: field values are **positive inside solid** and negative (or
//! zero) in empty space; the surface is where the value crosses zero. Meshes
//! face outward, i.e. from solid towards empty space.
//!
//! Pure: no I/O, no engine types. The sample at local index `i` sits at
//! `field.origin() + i * field.step()` (step 1 unless set, e.g. 2/4/8 for
//! coarser levels of detail).
//!
//! # Chunking
//! The outermost layer of samples on every side of a field is an *apron*: it
//! is read, but quads are only emitted for grid edges starting strictly inside
//! it. To mesh a big volume in pieces without gaps, give neighbouring fields
//! an overlap of **2 samples** (each piece owns samples `start..end` and
//! includes one extra sample on each side). Shared vertices then come out
//! bit-identical in both pieces.

mod check;
mod skirt;

pub use check::{EdgeReport, edge_report};
pub use skirt::add_skirts;

/// A dense grid of samples with a value and a material id each, x fastest.
#[derive(Clone, Debug)]
pub struct Field {
    origin: [i32; 3],
    step: i32,
    dims: [usize; 3],
    values: Vec<f32>,
    materials: Vec<u8>,
}

impl Field {
    /// A field of `dims` samples, all empty (value -1, material 0).
    pub fn new(dims: [usize; 3]) -> Field {
        let n = dims.iter().product();
        Field {
            origin: [0; 3],
            step: 1,
            dims,
            values: vec![-1.0; n],
            materials: vec![0; n],
        }
    }

    /// Places local sample `[0, 0, 0]` at `origin` in the output mesh.
    pub fn with_origin(mut self, origin: [i32; 3]) -> Field {
        self.origin = origin;
        self
    }

    pub fn origin(&self) -> [i32; 3] {
        self.origin
    }

    /// Spaces samples `step` units apart in the output mesh (default 1).
    ///
    /// # Panics
    /// If `step` is not positive.
    pub fn with_step(mut self, step: i32) -> Field {
        assert!(step > 0, "field step must be positive");
        self.step = step;
        self
    }

    pub fn step(&self) -> i32 {
        self.step
    }

    pub fn dims(&self) -> [usize; 3] {
        self.dims
    }

    fn index(&self, [x, y, z]: [usize; 3]) -> usize {
        debug_assert!(x < self.dims[0] && y < self.dims[1] && z < self.dims[2]);
        x + self.dims[0] * (y + self.dims[1] * z)
    }

    /// Panics if `pos` is out of bounds.
    pub fn set(&mut self, pos: [usize; 3], value: f32, material: u8) {
        let i = self.index(pos);
        self.values[i] = value;
        self.materials[i] = material;
    }

    /// Panics if `pos` is out of bounds.
    pub fn value(&self, pos: [usize; 3]) -> f32 {
        self.values[self.index(pos)]
    }

    /// Panics if `pos` is out of bounds.
    pub fn material(&self, pos: [usize; 3]) -> u8 {
        self.materials[self.index(pos)]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    /// Unit normals pointing out of the solid, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Counter-clockwise when seen from outside (right-handed coordinates).
    pub triangles: Vec<[u32; 3]>,
    /// Material id of the solid side, one per triangle.
    pub triangle_materials: Vec<u8>,
}

/// Corner `i` of a cell is offset by bit 0 → x, bit 1 → y, bit 2 → z.
const CORNERS: [[usize; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
    [1, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [0, 1, 1],
    [1, 1, 1],
];

/// The 12 cell edges as pairs of corner indices.
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

fn inside(value: f32) -> bool {
    value > 0.0
}

/// Meshes the zero crossing of `field`.
///
/// One vertex is placed in every cell (cube of 8 neighbouring samples) the
/// surface passes through, at the average of the edge crossings. Every grid
/// edge that crosses the surface then gets a quad joining the four cells
/// around it, but only for edges this field owns (see the crate docs on
/// chunking), so surfaces reaching the apron are left open there.
pub fn surface_nets(field: &Field) -> Mesh {
    let dims = field.dims;
    let mut mesh = Mesh::default();
    if dims.iter().any(|&n| n < 2) {
        return mesh;
    }
    let cells = dims.map(|n| n - 1);
    let cell_index = |[x, y, z]: [usize; 3]| x + cells[0] * (y + cells[1] * z);
    let mut cell_vertex = vec![u32::MAX; cells.iter().product()];

    for cz in 0..cells[2] {
        for cy in 0..cells[1] {
            for cx in 0..cells[0] {
                let base = [cx, cy, cz];
                let f = CORNERS.map(|c| field.value([cx + c[0], cy + c[1], cz + c[2]]));
                let solid = f.iter().filter(|v| inside(**v)).count();
                if solid == 0 || solid == 8 {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                let mut crossings = 0.0;
                for (a, b) in EDGES {
                    if inside(f[a]) == inside(f[b]) {
                        continue;
                    }
                    let t = f[a] / (f[a] - f[b]);
                    for axis in 0..3 {
                        let ca = CORNERS[a][axis] as f32;
                        let cb = CORNERS[b][axis] as f32;
                        sum[axis] += ca + t * (cb - ca);
                    }
                    crossings += 1.0;
                }
                // The field grows into the solid, so outward is minus its gradient.
                let mut gradient = [0.0f32; 3];
                for (corner, value) in CORNERS.iter().zip(f) {
                    for axis in 0..3 {
                        gradient[axis] += if corner[axis] == 1 { value } else { -value };
                    }
                }
                cell_vertex[cell_index(base)] = mesh.positions.len() as u32;
                // Integer corner first, then the fraction: neighbouring fields
                // with the same step then produce bit-identical positions.
                let step = field.step as f32;
                let global =
                    [0, 1, 2].map(|a| (field.origin[a] + base[a] as i32 * field.step) as f32);
                mesh.positions
                    .push([0, 1, 2].map(|a| global[a] + sum[a] / crossings * step));
                mesh.normals.push(normalize(gradient.map(|g| -g)));
            }
        }
    }

    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let p = [x, y, z];
                let here = inside(field.value(p));
                // Only edges starting inside the apron belong to this field.
                if (0..3).any(|i| p[i] == 0 || p[i] + 1 >= dims[i]) {
                    continue;
                }
                for a in 0..3 {
                    let (u, v) = ((a + 1) % 3, (a + 2) % 3);
                    let mut q = p;
                    q[a] += 1;
                    if inside(field.value(q)) == here {
                        continue;
                    }
                    let cell = |du: usize, dv: usize| {
                        let mut c = p;
                        c[u] -= 1 - du;
                        c[v] -= 1 - dv;
                        cell_vertex[cell_index(c)]
                    };
                    // Counter-clockwise around +a: (-u,-v) (+u,-v) (+u,+v) (-u,+v).
                    let mut quad = [cell(0, 0), cell(1, 0), cell(1, 1), cell(0, 1)];
                    if !here {
                        // Solid is on the +a side, so the outside faces -a.
                        quad.reverse();
                    }
                    let material = field.material(if here { p } else { q });
                    mesh.triangles.push([quad[0], quad[1], quad[2]]);
                    mesh.triangles.push([quad[0], quad[2], quad[3]]);
                    mesh.triangle_materials.extend([material, material]);
                }
            }
        }
    }
    mesh
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        v.map(|c| c / len)
    } else {
        [0.0, 1.0, 0.0]
    }
}
