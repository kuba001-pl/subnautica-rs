use std::fmt;

use crate::{MAX_DEPTH, OCTREE_SIZE, Octree, VOXELS_PER_OCTREE};

/// Structural facts about one valid octree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OctreeStats {
    pub nodes: usize,
    pub leaves: usize,
    /// Leaf count per depth (0 = the root is a leaf, 5 = single voxels).
    pub leaves_per_depth: [usize; MAX_DEPTH + 1],
    /// Voxels covered by all leaves. Always [`VOXELS_PER_OCTREE`] when valid.
    pub covered_voxels: usize,
    /// Voxels covered by leaves whose type is not 0.
    pub non_empty_voxels: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// The octree has no nodes, so it has no root.
    Empty,
    /// `node`'s children `first_child..first_child + 8` run past the node list.
    ChildOutOfBounds {
        node: usize,
        first_child: usize,
        len: usize,
    },
    /// `node` sits at single-voxel depth but still claims to have children.
    TooDeep { node: usize },
    /// `node` is the child of more than one parent (or of itself).
    NodeReachedTwice { node: usize },
    /// Some nodes are never referenced from the root.
    Unreachable { count: usize },
    /// A batch holds a different number of octrees than its size implies.
    WrongOctreeCount { expected: [usize; 3], found: usize },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "octree has no nodes"),
            Self::ChildOutOfBounds {
                node,
                first_child,
                len,
            } => write!(
                f,
                "node {node}: children {first_child}..{} out of bounds (len {len})",
                first_child + 8
            ),
            Self::TooDeep { node } => {
                write!(f, "node {node}: has children below single-voxel depth")
            }
            Self::NodeReachedTwice { node } => write!(f, "node {node}: has more than one parent"),
            Self::Unreachable { count } => write!(f, "{count} nodes unreachable from the root"),
            Self::WrongOctreeCount { expected, found } => {
                write!(f, "batch holds {found} octrees, expected {expected:?}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl Octree {
    /// Checks that the nodes form a proper octree: every child link is in
    /// bounds, every node except the root has exactly one parent, no node is
    /// unreachable, and the tree is no deeper than single voxels. A tree that
    /// passes covers the 32³ cube exactly once.
    pub fn validate(&self) -> Result<OctreeStats, ValidationError> {
        let len = self.nodes.len();
        if len == 0 {
            return Err(ValidationError::Empty);
        }
        let mut seen = vec![false; len];
        seen[0] = true;
        let mut stats = OctreeStats {
            nodes: len,
            ..Default::default()
        };
        let mut stack = vec![(0usize, 0usize)];
        while let Some((index, depth)) = stack.pop() {
            let node = self.nodes[index];
            if node.is_leaf() {
                let size = OCTREE_SIZE >> depth;
                let volume = size * size * size;
                stats.leaves += 1;
                stats.leaves_per_depth[depth] += 1;
                stats.covered_voxels += volume;
                if node.ty != 0 {
                    stats.non_empty_voxels += volume;
                }
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
            for (child, seen) in seen.iter_mut().enumerate().skip(first).take(8) {
                if *seen {
                    return Err(ValidationError::NodeReachedTwice { node: child });
                }
                *seen = true;
                stack.push((child, depth + 1));
            }
        }
        let unreachable = seen.iter().filter(|s| !**s).count();
        if unreachable > 0 {
            return Err(ValidationError::Unreachable { count: unreachable });
        }
        debug_assert_eq!(stats.covered_voxels, VOXELS_PER_OCTREE);
        Ok(stats)
    }
}
