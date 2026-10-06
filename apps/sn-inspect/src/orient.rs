//! `orient` command: work out how octree data maps onto space.
//!
//! Two orders are unknown: which axis each bit of a child index moves along
//! (child order), and how octrees are laid out inside a batch file (octree
//! order). Correct orders make terrain continuous across octree seams and
//! across batch seams; wrong ones tear it apart there. So we rasterize under
//! every combination and compare how often empty/solid flips across seams
//! versus inside octrees.

use std::fmt;
use std::ops::AddAssign;
use std::process::ExitCode;

use sn_octree::{AxisOrder, Batch, OCTREE_SIZE, Voxel, VoxelGrid, octree_origin};
use sn_world::BatchCoord;

use crate::Result;
use sn_install::GameData;

/// Below this many empty/solid faces a batch has too little surface to tell
/// orders apart.
const MIN_SURFACE_PAIRS: u64 = 1000;

fn solid(v: Voxel) -> bool {
    v.ty != 0
}

/// Count of face-adjacent voxel pairs, and how many of them differ in
/// empty-vs-solid.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pairs {
    pub total: u64,
    pub differ: u64,
}

impl Pairs {
    pub fn agree(&self) -> f64 {
        1.0 - self.differ_rate()
    }

    pub fn differ_rate(&self) -> f64 {
        self.differ as f64 / self.total.max(1) as f64
    }

    fn add(&mut self, a: bool, b: bool) {
        self.total += 1;
        self.differ += u64::from(a != b);
    }

    pub fn within_octree(grid: &VoxelGrid) -> Pairs {
        const S: usize = OCTREE_SIZE;
        let v = grid.voxels();
        let mut pairs = Pairs::default();
        for z in 0..S {
            for y in 0..S {
                for x in 0..S {
                    let i = x + S * (y + S * z);
                    let here = solid(v[i]);
                    if x + 1 < S {
                        pairs.add(here, solid(v[i + 1]));
                    }
                    if y + 1 < S {
                        pairs.add(here, solid(v[i + S]));
                    }
                    if z + 1 < S {
                        pairs.add(here, solid(v[i + S * S]));
                    }
                }
            }
        }
        pairs
    }
}

impl AddAssign for Pairs {
    fn add_assign(&mut self, other: Pairs) {
        self.total += other.total;
        self.differ += other.differ;
    }
}

impl fmt::Display for Pairs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:.4} agree ({} of {} pairs differ)",
            self.agree(),
            self.differ,
            self.total
        )
    }
}

/// A deliberately wrong child layout: each node gets its own pseudo-random
/// axis order and mirroring. Used as a control for the coherence metric.
pub fn scrambled_child_offset(node: usize, child: usize) -> [usize; 3] {
    let mut h = (node as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    let order = AxisOrder::ALL[(h % 6) as usize];
    order.child_offset(child ^ ((h >> 8) & 7) as usize)
}

/// Empty/solid flags for a whole batch, x fastest.
struct SolidGrid {
    dims: [usize; 3],
    data: Vec<bool>,
}

impl SolidGrid {
    fn assemble(grids: &[VoxelGrid], octree_dims: [usize; 3], order: AxisOrder) -> SolidGrid {
        let dims = octree_dims.map(|n| n * OCTREE_SIZE);
        let mut data = vec![false; dims.iter().product()];
        for (i, grid) in grids.iter().enumerate() {
            let [ox, oy, oz] = octree_origin(i, octree_dims, order);
            for z in 0..OCTREE_SIZE {
                for y in 0..OCTREE_SIZE {
                    for x in 0..OCTREE_SIZE {
                        let at = (ox + x) + dims[0] * ((oy + y) + dims[1] * (oz + z));
                        data[at] = solid(grid.get([x, y, z]));
                    }
                }
            }
        }
        SolidGrid { dims, data }
    }

    fn get(&self, [x, y, z]: [usize; 3]) -> bool {
        self.data[x + self.dims[0] * (y + self.dims[1] * z)]
    }

    /// Pairs inside the batch, split into (within one octree, across an
    /// octree seam).
    fn pairs(&self) -> (Pairs, Pairs) {
        let (mut interior, mut seams) = (Pairs::default(), Pairs::default());
        let [dx, dy, dz] = self.dims;
        let strides = [1, dx, dx * dy];
        for z in 0..dz {
            for y in 0..dy {
                for x in 0..dx {
                    let p = [x, y, z];
                    let i = x + dx * (y + dy * z);
                    let here = self.data[i];
                    for a in 0..3 {
                        if p[a] + 1 >= self.dims[a] {
                            continue;
                        }
                        let there = self.data[i + strides[a]];
                        if (p[a] + 1) % OCTREE_SIZE == 0 {
                            seams.add(here, there);
                        } else {
                            interior.add(here, there);
                        }
                    }
                }
            }
        }
        (interior, seams)
    }

    /// Pairs across the seam between this batch's far face along `axis` and
    /// `next`'s near face. `None` if the faces have different sizes.
    fn face_pairs(&self, next: &SolidGrid, axis: usize) -> Option<Pairs> {
        let (u, v) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        if self.dims[u] != next.dims[u] || self.dims[v] != next.dims[v] {
            return None;
        }
        let mut pairs = Pairs::default();
        for j in 0..self.dims[v] {
            for i in 0..self.dims[u] {
                let mut a = [0; 3];
                a[u] = i;
                a[v] = j;
                let mut b = a;
                a[axis] = self.dims[axis] - 1;
                b[axis] = 0;
                pairs.add(self.get(a), next.get(b));
            }
        }
        Some(pairs)
    }
}

struct Loaded {
    dims: [usize; 3],
    batch: Batch,
}

struct Candidate {
    child: AxisOrder,
    octree: AxisOrder,
    interior: Pairs,
    seams: Pairs,
    cross: [Option<Pairs>; 3],
}

impl Candidate {
    fn boundary(&self) -> Pairs {
        let mut total = self.seams;
        for pairs in self.cross.iter().flatten() {
            total += *pairs;
        }
        total
    }
}

pub fn run(game: &GameData, coord: BatchCoord) -> Result<ExitCode> {
    let index = game.read_index()?;
    let load = |c: BatchCoord| -> Result<Option<Loaded>> {
        let Some(dims) = index.batch_octree_dims(c) else {
            return Ok(None);
        };
        let Some(bytes) = game.read_batch(c)? else {
            return Ok(None);
        };
        let batch = Batch::parse(&bytes).map_err(|e| format!("batch {c}: {e}"))?;
        Ok(Some(Loaded { dims, batch }))
    };
    let base = load(coord)?.ok_or_else(|| format!("batch {coord} has no octree file"))?;
    let neighbours = [
        load(coord.offset(1, 0, 0))?,
        load(coord.offset(0, 1, 0))?,
        load(coord.offset(0, 0, 1))?,
    ];
    println!("orientation check on batch {coord}");
    for (name, n) in ["+X", "+Y", "+Z"].iter().zip(&neighbours) {
        println!(
            "  neighbour {name}: {}",
            if n.is_some() {
                "loaded"
            } else {
                "missing (skipped)"
            }
        );
    }

    let rasterize = |loaded: &Loaded, order: AxisOrder| -> Result<Vec<VoxelGrid>> {
        loaded
            .batch
            .octrees
            .iter()
            .map(|o| o.rasterize(order).map_err(|e| e.to_string()))
            .collect()
    };

    let mut candidates = Vec::new();
    for child in AxisOrder::ALL {
        let base_grids = rasterize(&base, child)?;
        let neighbour_grids: Vec<Option<Vec<VoxelGrid>>> = neighbours
            .iter()
            .map(|n| n.as_ref().map(|n| rasterize(n, child)).transpose())
            .collect::<Result<_>>()?;
        for octree in AxisOrder::ALL {
            let grid = SolidGrid::assemble(&base_grids, base.dims, octree);
            let (interior, seams) = grid.pairs();
            let mut cross = [None; 3];
            for axis in 0..3 {
                if let (Some(n), Some(grids)) = (&neighbours[axis], &neighbour_grids[axis]) {
                    let next = SolidGrid::assemble(grids, n.dims, octree);
                    cross[axis] = grid.face_pairs(&next, axis);
                }
            }
            candidates.push(Candidate {
                child,
                octree,
                interior,
                seams,
                cross,
            });
        }
    }
    candidates.sort_by(|a, b| {
        a.boundary()
            .differ_rate()
            .total_cmp(&b.boundary().differ_rate())
    });

    println!();
    println!("Fraction of face-adjacent voxel pairs that disagree (empty vs solid).");
    println!("Correct orders: seams look like the interior. Wrong: seams disagree far more.");
    println!();
    println!(
        "child octree | interior | octree seams | batch +X | batch +Y | batch +Z | seams/interior"
    );
    let fmt =
        |p: &Option<Pairs>| p.map_or("     -  ".into(), |p| format!("{:8.5}", p.differ_rate()));
    for c in &candidates {
        let ratio = c.boundary().differ_rate() / c.interior.differ_rate().max(1e-12);
        println!(
            " {}   {}  | {:8.5} |     {:8.5} | {} | {} | {} | {ratio:8.2}",
            c.child,
            c.octree,
            c.interior.differ_rate(),
            c.seams.differ_rate(),
            fmt(&c.cross[0]),
            fmt(&c.cross[1]),
            fmt(&c.cross[2]),
        );
    }

    let best = &candidates[0];
    let runner_up = &candidates[1];
    println!();
    if best.interior.differ < MIN_SURFACE_PAIRS {
        println!(
            "INCONCLUSIVE: only {} empty/solid faces inside this batch (need {MIN_SURFACE_PAIRS}).",
            best.interior.differ
        );
        println!("The batch is (almost) uniform, so every order scores the same. Pick a");
        println!("batch with more terrain surface (larger .optoctrees files have more).");
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "best: child order {}, octree order {} (boundary disagreement {:.5}; next best {:.5} = {}/{})",
        best.child,
        best.octree,
        best.boundary().differ_rate(),
        runner_up.boundary().differ_rate(),
        runner_up.child,
        runner_up.octree,
    );

    // Sanity check of "up": with the best orders, how solid is each octree
    // layer of the batch, from bottom (y = 0) to top?
    let grids = rasterize(&base, best.child)?;
    let grid = SolidGrid::assemble(&grids, base.dims, best.octree);
    println!("non-empty share per 32-voxel layer of local y (bottom first):");
    for layer in 0..base.dims[1] {
        let mut count = 0usize;
        let mut total = 0usize;
        for z in 0..grid.dims[2] {
            for y in layer * OCTREE_SIZE..(layer + 1) * OCTREE_SIZE {
                for x in 0..grid.dims[0] {
                    count += usize::from(grid.get([x, y, z]));
                    total += 1;
                }
            }
        }
        println!(
            "  layer {layer}: {:6.2}%",
            100.0 * count as f64 / total as f64
        );
    }
    Ok(ExitCode::SUCCESS)
}
