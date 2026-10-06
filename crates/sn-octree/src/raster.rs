use std::fmt;

use crate::{MAX_DEPTH, OCTREE_SIZE, Octree, VOXELS_PER_OCTREE, ValidationError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// A permutation of the axes, listed from fastest- to slowest-varying.
///
/// Used for two things whose real order in the game data is still being
/// established (see `docs/formats/optoctrees.md`):
/// - child index bits: bit 0 of a child index moves along `self.0[0]`,
///   bit 1 along `self.0[1]`, bit 2 along `self.0[2]`;
/// - octree order inside a batch: file index `i` is laid out with
///   `self.0[0]` varying fastest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AxisOrder(pub [Axis; 3]);

impl AxisOrder {
    pub const XYZ: Self = Self([Axis::X, Axis::Y, Axis::Z]);
    pub const XZY: Self = Self([Axis::X, Axis::Z, Axis::Y]);
    pub const YXZ: Self = Self([Axis::Y, Axis::X, Axis::Z]);
    pub const YZX: Self = Self([Axis::Y, Axis::Z, Axis::X]);
    pub const ZXY: Self = Self([Axis::Z, Axis::X, Axis::Y]);
    pub const ZYX: Self = Self([Axis::Z, Axis::Y, Axis::X]);
    pub const ALL: [Self; 6] = [
        Self::XYZ,
        Self::XZY,
        Self::YXZ,
        Self::YZX,
        Self::ZXY,
        Self::ZYX,
    ];

    /// Splits a linear index into `[x, y, z]` for a grid `dims` large.
    pub fn delinearize(self, mut index: usize, dims: [usize; 3]) -> [usize; 3] {
        let mut pos = [0; 3];
        for axis in self.0 {
            let a = axis.index();
            pos[a] = index % dims[a];
            index /= dims[a];
        }
        pos
    }

    /// Inverse of [`AxisOrder::delinearize`].
    pub fn linearize(self, pos: [usize; 3], dims: [usize; 3]) -> usize {
        self.0
            .iter()
            .rev()
            .fold(0, |acc, axis| acc * dims[axis.index()] + pos[axis.index()])
    }

    /// Offset (0 or 1 per axis) of child `child` (0..8) within its parent.
    pub fn child_offset(self, child: usize) -> [usize; 3] {
        self.delinearize(child, [2, 2, 2])
    }
}

impl fmt::Display for AxisOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for axis in self.0 {
            write!(f, "{axis:?}")?;
        }
        Ok(())
    }
}

/// The value a leaf assigns to every voxel it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Voxel {
    pub ty: u8,
    pub density: u8,
}

/// A dense 32³ voxel grid, stored x-fastest, then y, then z.
#[derive(Clone, PartialEq, Eq)]
pub struct VoxelGrid {
    voxels: Vec<Voxel>,
}

impl fmt::Debug for VoxelGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoxelGrid").finish_non_exhaustive()
    }
}

impl Default for VoxelGrid {
    fn default() -> Self {
        Self {
            voxels: vec![Voxel::default(); VOXELS_PER_OCTREE],
        }
    }
}

impl VoxelGrid {
    fn index([x, y, z]: [usize; 3]) -> usize {
        debug_assert!(x < OCTREE_SIZE && y < OCTREE_SIZE && z < OCTREE_SIZE);
        x + OCTREE_SIZE * (y + OCTREE_SIZE * z)
    }

    /// Panics if any coordinate is ≥ 32.
    pub fn get(&self, pos: [usize; 3]) -> Voxel {
        self.voxels[Self::index(pos)]
    }

    /// Panics if any coordinate is ≥ 32.
    pub fn set(&mut self, pos: [usize; 3], voxel: Voxel) {
        self.voxels[Self::index(pos)] = voxel;
    }

    pub fn voxels(&self) -> &[Voxel] {
        &self.voxels
    }

    fn fill_cube(&mut self, origin: [usize; 3], size: usize, voxel: Voxel) {
        for z in origin[2]..origin[2] + size {
            for y in origin[1]..origin[1] + size {
                let row = Self::index([origin[0], y, z]);
                self.voxels[row..row + size].fill(voxel);
            }
        }
    }
}

impl Octree {
    /// Expands the octree into a dense 32³ grid, interpreting child indices
    /// with `order`.
    pub fn rasterize(&self, order: AxisOrder) -> Result<VoxelGrid, ValidationError> {
        self.rasterize_with(|_, child| order.child_offset(child))
    }

    /// Like [`Octree::rasterize`], but `child_offset(node, child)` decides where
    /// child `child` (0..8) of node `node` goes. Meant for diagnostics, e.g.
    /// rasterizing with a deliberately scrambled order as a control.
    ///
    /// Bounds and depth are checked, so malformed input returns an error
    /// instead of panicking. (Shared children are tolerated here; use
    /// [`Octree::validate`] to reject them.)
    pub fn rasterize_with(
        &self,
        child_offset: impl Fn(usize, usize) -> [usize; 3],
    ) -> Result<VoxelGrid, ValidationError> {
        let len = self.nodes.len();
        if len == 0 {
            return Err(ValidationError::Empty);
        }
        let mut grid = VoxelGrid::default();
        let mut stack = vec![(0usize, 0usize, [0usize; 3])];
        while let Some((index, depth, origin)) = stack.pop() {
            let node = self.nodes[index];
            let size = OCTREE_SIZE >> depth;
            if node.is_leaf() {
                let voxel = Voxel {
                    ty: node.ty,
                    density: node.density,
                };
                grid.fill_cube(origin, size, voxel);
                continue;
            }
            if depth == MAX_DEPTH {
                return Err(ValidationError::TooDeep { node: index });
            }
            let first = usize::from(node.first_child);
            if first + 8 > len {
                return Err(ValidationError::ChildOutOfBounds {
                    node: index,
                    first_child: first,
                    len,
                });
            }
            let half = size / 2;
            for child in 0..8 {
                let offset = child_offset(index, child);
                let child_origin = [0, 1, 2].map(|a| origin[a] + (offset[a] & 1) * half);
                stack.push((first + child, depth + 1, child_origin));
            }
        }
        Ok(grid)
    }
}
