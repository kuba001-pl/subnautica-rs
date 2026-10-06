//! Writer for our own synthetic octrees. Used by tests so they never need
//! files from the game.

use std::collections::VecDeque;

use crate::{AxisOrder, Batch, NODE_BYTES, Node, OCTREE_SIZE, Octree, Voxel, VoxelGrid};

impl Batch {
    /// Serializes the batch in the on-disk layout.
    ///
    /// # Panics
    /// If an octree has more than `u16::MAX` nodes (a complete 32³ octree has
    /// 37,449, so this only happens with hand-built invalid data).
    pub fn encode(&self) -> Vec<u8> {
        let nodes: usize = self.octrees.iter().map(|o| o.nodes.len()).sum();
        let mut out = Vec::with_capacity(4 + self.octrees.len() * 2 + nodes * NODE_BYTES);
        out.extend_from_slice(&self.version.to_le_bytes());
        for octree in &self.octrees {
            let count = u16::try_from(octree.nodes.len()).expect("octree has > 65535 nodes");
            out.extend_from_slice(&count.to_le_bytes());
            for node in &octree.nodes {
                out.push(node.ty);
                out.push(node.density);
                out.extend_from_slice(&node.first_child.to_le_bytes());
            }
        }
        out
    }
}

enum Tree {
    Leaf(Voxel),
    Branch(Voxel, Box<[Tree; 8]>),
}

impl Tree {
    fn value(&self) -> Voxel {
        match self {
            Tree::Leaf(v) | Tree::Branch(v, _) => *v,
        }
    }
}

impl Octree {
    /// Builds the smallest octree that rasterizes back to `grid` under
    /// `order`, merging uniform regions into single leaves. Children are laid
    /// out breadth-first. Inner nodes copy the value of their first child —
    /// the real game data stores some other summary there (not yet known).
    pub fn from_voxels(grid: &VoxelGrid, order: AxisOrder) -> Octree {
        let tree = build(grid, order, [0; 3], OCTREE_SIZE);
        let mut nodes = vec![Node::default()];
        let mut queue = VecDeque::from([(&tree, 0usize)]);
        while let Some((tree, index)) = queue.pop_front() {
            let value = tree.value();
            let first_child = match tree {
                Tree::Leaf(_) => 0,
                Tree::Branch(_, children) => {
                    let first = nodes.len();
                    nodes.resize(first + 8, Node::default());
                    for (i, child) in children.iter().enumerate() {
                        queue.push_back((child, first + i));
                    }
                    u16::try_from(first).expect("octree has > 65535 nodes")
                }
            };
            nodes[index] = Node {
                ty: value.ty,
                density: value.density,
                first_child,
            };
        }
        Octree { nodes }
    }
}

fn build(grid: &VoxelGrid, order: AxisOrder, origin: [usize; 3], size: usize) -> Tree {
    if size == 1 {
        return Tree::Leaf(grid.get(origin));
    }
    let half = size / 2;
    let children: [Tree; 8] = std::array::from_fn(|child| {
        let offset = order.child_offset(child);
        let child_origin = [0, 1, 2].map(|a| origin[a] + offset[a] * half);
        build(grid, order, child_origin, half)
    });
    let first = children[0].value();
    if children
        .iter()
        .all(|c| matches!(c, Tree::Leaf(v) if *v == first))
    {
        Tree::Leaf(first)
    } else {
        Tree::Branch(first, Box::new(children))
    }
}
