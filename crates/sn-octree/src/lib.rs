//! Reader and writer for Subnautica's compiled terrain octrees (`.optoctrees`).
//!
//! Each file holds one *batch*: up to 5×5×5 octrees, each covering 32³ voxels.
//! See `docs/formats/optoctrees.md` for the format and which parts of it are
//! confirmed versus hypotheses.
//!
//! This crate is pure: it takes bytes and returns data. It never touches the
//! filesystem and never panics on malformed input.

mod encode;
mod parse;
mod raster;
mod validate;

pub use parse::{ParseError, ParseErrorKind};
pub use raster::{Axis, AxisOrder, Voxel, VoxelGrid};
pub use validate::{OctreeStats, ValidationError};

/// The only file version we have seen (every file in build 18 / game build 10).
pub const FORMAT_VERSION: i32 = 4;
/// Edge length of one octree, in voxels.
pub const OCTREE_SIZE: usize = 32;
/// Depth of a single-voxel leaf (32 → 16 → 8 → 4 → 2 → 1).
pub const MAX_DEPTH: usize = 5;
/// Number of voxels covered by one octree.
pub const VOXELS_PER_OCTREE: usize = OCTREE_SIZE * OCTREE_SIZE * OCTREE_SIZE;
/// Size of one serialized node.
pub const NODE_BYTES: usize = 4;

/// How the game orders children: bit 0 of a child index moves along z,
/// bit 1 along y, bit 2 along x. Confirmed with `sn-inspect orient` on six
/// detailed batches (seam continuity; see `docs/formats/optoctrees.md`).
pub const GAME_CHILD_ORDER: AxisOrder = AxisOrder::ZYX;
/// How the game orders octrees inside a batch file: z fastest, then y, then
/// x. Confirmed together with [`GAME_CHILD_ORDER`].
pub const GAME_OCTREE_ORDER: AxisOrder = AxisOrder::ZYX;

/// One octree node as stored on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Node {
    /// Block/material type id. 0 appears to mean empty (water/air) — hypothesis.
    pub ty: u8,
    /// Quantised distance to the surface. Meaning not yet established (M2).
    pub density: u8,
    /// Index of the first of 8 consecutive children, or 0 for a leaf.
    pub first_child: u16,
}

impl Node {
    pub fn is_leaf(&self) -> bool {
        self.first_child == 0
    }
}

/// One octree. Node 0 is the root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Octree {
    pub nodes: Vec<Node>,
}

/// The contents of one `.optoctrees` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub version: i32,
    /// Octrees in file order. How file order maps to positions inside the
    /// batch is described by an [`AxisOrder`] (see [`octree_origin`]).
    pub octrees: Vec<Octree>,
}

/// Voxel-space origin (inside the batch) of the octree at `index` in file
/// order, for a batch that is `dims` octrees large along x, y, z.
pub fn octree_origin(index: usize, dims: [usize; 3], order: AxisOrder) -> [usize; 3] {
    order
        .delinearize(index, dims)
        .map(|octrees| octrees * OCTREE_SIZE)
}

/// Density value of the surface: every voxel with density 1..=125 is empty
/// and every voxel with density ≥ 126 is solid (confirmed; see the format doc).
pub const SURFACE_DENSITY: f32 = 125.5;

/// Signed value used for "far from the surface" voxels (density byte 0).
pub const FAR_DENSITY: f32 = 126.0;

impl Voxel {
    /// Type 0 is empty (water or air); everything else is solid ground.
    pub fn is_solid(&self) -> bool {
        self.ty != 0
    }

    /// Signed distance-like value: positive inside solid ground, negative in
    /// empty space, zero on the surface. Near the surface one voxel is about
    /// 15 units. Density 0 means "far from the surface"; then only the type
    /// tells the side, and we return ±[`FAR_DENSITY`].
    pub fn signed_density(&self) -> f32 {
        match (self.density, self.is_solid()) {
            (0, true) => FAR_DENSITY,
            (0, false) => -FAR_DENSITY,
            (d, _) => f32::from(d) - SURFACE_DENSITY,
        }
    }
}

/// All voxels of one batch, x fastest, in the game's spatial layout.
#[derive(Clone, Debug)]
pub struct BatchGrid {
    /// Size in voxels (160 per full axis, 96 on partial edge batches).
    pub dims: [usize; 3],
    pub voxels: Vec<Voxel>,
}

impl BatchGrid {
    /// Panics if `pos` is outside `dims`.
    pub fn get(&self, [x, y, z]: [usize; 3]) -> Voxel {
        self.voxels[x + self.dims[0] * (y + self.dims[1] * z)]
    }
}

impl Batch {
    /// Expands every octree into one grid using [`GAME_CHILD_ORDER`] and
    /// [`GAME_OCTREE_ORDER`]. `octree_dims` is the batch size in octrees
    /// (from the world index); it must match the number of octrees.
    pub fn rasterize(&self, octree_dims: [usize; 3]) -> Result<BatchGrid, ValidationError> {
        if octree_dims.iter().product::<usize>() != self.octrees.len() {
            return Err(ValidationError::WrongOctreeCount {
                expected: octree_dims,
                found: self.octrees.len(),
            });
        }
        let dims = octree_dims.map(|n| n * OCTREE_SIZE);
        let mut voxels = vec![Voxel::default(); dims.iter().product()];
        for (i, octree) in self.octrees.iter().enumerate() {
            let grid = octree.rasterize(GAME_CHILD_ORDER)?;
            let [ox, oy, oz] = octree_origin(i, octree_dims, GAME_OCTREE_ORDER);
            for z in 0..OCTREE_SIZE {
                for y in 0..OCTREE_SIZE {
                    let row = ox + dims[0] * ((oy + y) + dims[1] * (oz + z));
                    for x in 0..OCTREE_SIZE {
                        voxels[row + x] = grid.get([x, y, z]);
                    }
                }
            }
        }
        Ok(BatchGrid { dims, voxels })
    }
}
