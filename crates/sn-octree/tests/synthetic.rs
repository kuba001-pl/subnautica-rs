//! Tests on octrees we generate ourselves. These never need the game.

use sn_octree::{
    AxisOrder, Batch, FORMAT_VERSION, Node, OCTREE_SIZE, Octree, ParseErrorKind, VOXELS_PER_OCTREE,
    ValidationError, Voxel, VoxelGrid, octree_origin,
};

/// Tiny deterministic PRNG so tests need no dependencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn grid_from(mut f: impl FnMut([usize; 3]) -> Voxel) -> VoxelGrid {
    let mut grid = VoxelGrid::default();
    for z in 0..OCTREE_SIZE {
        for y in 0..OCTREE_SIZE {
            for x in 0..OCTREE_SIZE {
                grid.set([x, y, z], f([x, y, z]));
            }
        }
    }
    grid
}

/// A lumpy seabed with a few materials and a tunnel: lots of uniform regions
/// plus detail near the surface, like real terrain. Deliberately asymmetric so
/// that interpreting it with the wrong axis order gives a different grid.
fn seabed() -> VoxelGrid {
    grid_from(|[x, y, z]| {
        let height = 8 + (x * 3 + z) % 11 + x / 4;
        let tunnel = (10..14).contains(&y) && (6..20).contains(&z) && x > 3;
        if y < height && !tunnel {
            Voxel {
                ty: 1 + ((x / 8 + z / 16) % 3) as u8,
                density: (height - y).min(255) as u8,
            }
        } else {
            Voxel::default()
        }
    })
}

fn noise(seed: u64) -> VoxelGrid {
    let mut rng = Lcg(seed);
    grid_from(|_| Voxel {
        ty: rng.next() as u8,
        density: rng.next() as u8,
    })
}

#[test]
fn rasterize_inverts_from_voxels_for_every_order() {
    for grid in [seabed(), noise(1)] {
        for order in AxisOrder::ALL {
            let octree = Octree::from_voxels(&grid, order);
            assert_eq!(octree.rasterize(order).unwrap(), grid, "order {order}");
        }
    }
}

#[test]
fn wrong_order_gives_a_different_grid() {
    let grid = seabed();
    let octree = Octree::from_voxels(&grid, AxisOrder::XYZ);
    for order in AxisOrder::ALL {
        let same = octree.rasterize(order).unwrap() == grid;
        assert_eq!(same, order == AxisOrder::XYZ, "order {order}");
    }
}

#[test]
fn encode_parse_round_trip() {
    let batch = Batch {
        version: FORMAT_VERSION,
        octrees: vec![
            Octree::from_voxels(&seabed(), AxisOrder::XYZ),
            Octree::from_voxels(&VoxelGrid::default(), AxisOrder::XYZ),
            Octree::from_voxels(&noise(7), AxisOrder::ZYX),
        ],
    };
    let bytes = batch.encode();
    assert_eq!(Batch::parse(&bytes).unwrap(), batch);
}

#[test]
fn validate_reports_full_coverage() {
    let grid = seabed();
    let octree = Octree::from_voxels(&grid, AxisOrder::XYZ);
    let stats = octree.validate().unwrap();
    assert_eq!(stats.nodes, octree.nodes.len());
    assert_eq!(stats.covered_voxels, VOXELS_PER_OCTREE);
    let solid = grid.voxels().iter().filter(|v| v.ty != 0).count();
    assert_eq!(stats.non_empty_voxels, solid);
    assert_eq!(stats.leaves_per_depth.iter().sum::<usize>(), stats.leaves);
}

#[test]
fn uniform_grid_is_a_single_leaf() {
    let octree = Octree::from_voxels(&VoxelGrid::default(), AxisOrder::XYZ);
    assert_eq!(octree.nodes, vec![Node::default()]);
    assert_eq!(octree.validate().unwrap().leaves_per_depth[0], 1);
}

#[test]
fn noise_gives_a_complete_tree() {
    // 1 + 8 + 64 + 512 + 4096 + 32768: the largest octree seen in game data.
    let octree = Octree::from_voxels(&noise(3), AxisOrder::XYZ);
    assert_eq!(octree.nodes.len(), 37_449);
    assert_eq!(octree.validate().unwrap().leaves_per_depth[5], 32_768);
}

#[test]
fn parse_rejects_bad_version() {
    let err = Batch::parse(&3i32.to_le_bytes()).unwrap_err();
    assert_eq!(err.kind, ParseErrorKind::UnsupportedVersion(3));
}

#[test]
fn parse_reports_truncation_offset() {
    assert_eq!(
        Batch::parse(&[]).unwrap_err().kind,
        ParseErrorKind::UnexpectedEof { needed: 4 }
    );
    let batch = Batch {
        version: FORMAT_VERSION,
        octrees: vec![Octree::from_voxels(&seabed(), AxisOrder::XYZ)],
    };
    let bytes = batch.encode();
    let err = Batch::parse(&bytes[..bytes.len() - 1]).unwrap_err();
    assert_eq!(err.offset, 6, "node data starts after version + count");
    assert_eq!(err.kind, ParseErrorKind::UnexpectedEof { needed: 1 });
}

#[test]
fn parse_accepts_a_batch_with_no_octrees() {
    let batch = Batch::parse(&FORMAT_VERSION.to_le_bytes()).unwrap();
    assert!(batch.octrees.is_empty());
}

#[test]
fn parse_and_validate_never_panic_on_garbage() {
    let mut rng = Lcg(42);
    for len in 0..300 {
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if len >= 4 && len % 2 == 0 {
            bytes[..4].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        }
        if let Ok(batch) = Batch::parse(&bytes) {
            for octree in &batch.octrees {
                let _ = octree.validate();
                let _ = octree.rasterize(AxisOrder::XYZ);
            }
        }
    }
}

fn leaf() -> Node {
    Node::default()
}

fn branch(first_child: u16) -> Node {
    Node {
        first_child,
        ..Node::default()
    }
}

#[test]
fn validate_rejects_broken_links() {
    assert_eq!(Octree::default().validate(), Err(ValidationError::Empty));

    let out_of_bounds = Octree {
        nodes: vec![branch(1), leaf(), leaf()],
    };
    assert_eq!(
        out_of_bounds.validate(),
        Err(ValidationError::ChildOutOfBounds {
            node: 0,
            first_child: 1,
            len: 3
        })
    );
    assert!(out_of_bounds.rasterize(AxisOrder::XYZ).is_err());

    // Node 1 is listed as a child of the root and also as node 2's child.
    let mut shared = vec![branch(1), leaf(), branch(1)];
    shared.extend(std::iter::repeat_n(leaf(), 6));
    assert!(matches!(
        Octree { nodes: shared }.validate(),
        Err(ValidationError::NodeReachedTwice { .. })
    ));

    let mut orphan = vec![branch(1)];
    orphan.extend(std::iter::repeat_n(leaf(), 9));
    assert_eq!(
        Octree { nodes: orphan }.validate(),
        Err(ValidationError::Unreachable { count: 1 })
    );
}

#[test]
fn validate_rejects_trees_deeper_than_one_voxel() {
    // A chain where the first child of every level is a branch, 6 levels deep.
    let mut nodes = vec![branch(1)];
    for level in 0..6u16 {
        nodes.push(branch(1 + 8 * (level + 1)));
        nodes.extend(std::iter::repeat_n(leaf(), 7));
    }
    nodes.extend(std::iter::repeat_n(leaf(), 8));
    let octree = Octree { nodes };
    assert!(matches!(
        octree.validate(),
        Err(ValidationError::TooDeep { .. })
    ));
    assert!(matches!(
        octree.rasterize(AxisOrder::XYZ),
        Err(ValidationError::TooDeep { .. })
    ));
}

#[test]
fn axis_order_linearize_round_trips() {
    let dims = [3, 5, 4];
    for order in AxisOrder::ALL {
        for i in 0..dims.iter().product() {
            assert_eq!(order.linearize(order.delinearize(i, dims), dims), i);
        }
        assert_eq!(order.delinearize(1, dims)[order.0[0].index()], 1);
    }
}

#[test]
fn octree_origin_steps_by_octree_size() {
    assert_eq!(octree_origin(0, [5, 5, 5], AxisOrder::XYZ), [0, 0, 0]);
    assert_eq!(octree_origin(1, [5, 5, 5], AxisOrder::XYZ), [32, 0, 0]);
    assert_eq!(octree_origin(5, [5, 5, 5], AxisOrder::XYZ), [0, 32, 0]);
    assert_eq!(octree_origin(1, [5, 5, 5], AxisOrder::ZYX), [0, 0, 32]);
}

#[test]
fn signed_density_has_the_surface_between_125_and_126() {
    let v = |ty, density| Voxel { ty, density }.signed_density();
    assert!(v(0, 125) < 0.0 && v(5, 126) > 0.0);
    assert_eq!(v(0, 0), -sn_octree::FAR_DENSITY);
    assert_eq!(v(7, 0), sn_octree::FAR_DENSITY);
}

#[test]
fn batch_rasterize_places_octrees_in_game_order() {
    use sn_octree::{GAME_CHILD_ORDER, GAME_OCTREE_ORDER};
    // A batch 2 octrees wide in x and 2 deep in z: mark each octree with
    // its own type and check where it lands.
    let dims = [2, 1, 2];
    let octrees = (0..4)
        .map(|i| {
            let mut grid = VoxelGrid::default();
            grid.set(
                [1, 2, 3],
                Voxel {
                    ty: 10 + i as u8,
                    density: 200,
                },
            );
            Octree::from_voxels(&grid, GAME_CHILD_ORDER)
        })
        .collect();
    let batch = Batch {
        version: FORMAT_VERSION,
        octrees,
    };
    let grid = batch.rasterize(dims).unwrap();
    assert_eq!(grid.dims, [64, 32, 64]);
    for i in 0..4 {
        let [ox, oy, oz] = octree_origin(i, dims, GAME_OCTREE_ORDER);
        assert_eq!(grid.get([ox + 1, oy + 2, oz + 3]).ty, 10 + i as u8);
    }
    // ZYX: z varies fastest, so octree 1 is at z = 32.
    assert_eq!(grid.get([1, 2, 32 + 3]).ty, 11);
    assert!(batch.rasterize([5, 5, 5]).is_err());
}
