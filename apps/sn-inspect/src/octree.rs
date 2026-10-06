//! `octree` command: decode terrain batches and report statistics.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use sn_octree::{AxisOrder, Batch, GAME_CHILD_ORDER, MAX_DEPTH, VOXELS_PER_OCTREE};
use sn_world::BatchCoord;

use crate::Result;
use crate::orient::{Pairs, scrambled_child_offset};
use sn_install::GameData;

pub fn one(game: &GameData, coord: BatchCoord) -> Result<ExitCode> {
    let index = game.read_index()?;
    let dims = index
        .batch_octree_dims(coord)
        .ok_or_else(|| format!("batch {coord} is outside the world"))?;
    let bytes = game
        .read_batch(coord)?
        .ok_or_else(|| format!("batch {coord} has no octree file"))?;
    let batch = Batch::parse(&bytes).map_err(|e| format!("batch {coord}: {e}"))?;

    let expected: usize = dims.iter().product();
    println!(
        "batch {coord}: {} bytes, version {}, {} octrees (expected {}x{}x{} = {expected})",
        bytes.len(),
        batch.version,
        batch.octrees.len(),
        dims[0],
        dims[1],
        dims[2],
    );

    let mut node_types = [0u64; 256];
    let mut voxel_types = [0u64; 256];
    let mut leaf_densities = [0u64; 256];
    let mut leaves_per_depth = [0usize; MAX_DEPTH + 1];
    let (mut nodes, mut leaves, mut non_empty) = (0, 0, 0);
    let mut sizes = Vec::new();
    let mut invalid = 0;
    for (i, octree) in batch.octrees.iter().enumerate() {
        for node in &octree.nodes {
            node_types[usize::from(node.ty)] += 1;
            if node.is_leaf() {
                leaf_densities[usize::from(node.density)] += 1;
            }
        }
        match octree.validate() {
            Ok(stats) => {
                nodes += stats.nodes;
                leaves += stats.leaves;
                non_empty += stats.non_empty_voxels;
                sizes.push(stats.nodes);
                for (total, n) in leaves_per_depth.iter_mut().zip(stats.leaves_per_depth) {
                    *total += n;
                }
            }
            Err(e) => {
                invalid += 1;
                println!("  octree {i}: INVALID: {e}");
            }
        }
    }
    let valid = batch.octrees.len() - invalid;
    let total_voxels = valid * VOXELS_PER_OCTREE;
    println!("valid octrees:    {valid} of {}", batch.octrees.len());
    println!(
        "nodes:            {nodes} total, {leaves} leaves; per octree min {} / max {}",
        sizes.iter().min().unwrap_or(&0),
        sizes.iter().max().unwrap_or(&0)
    );
    println!("leaves by depth:  {leaves_per_depth:?}  (depth 0 = whole octree, 5 = one voxel)");
    println!(
        "voxels:           {total_voxels} total, {non_empty} non-empty (type != 0) = {:.2}%",
        100.0 * non_empty as f64 / total_voxels.max(1) as f64
    );

    let mut grids = Vec::new();
    for octree in &batch.octrees {
        if let Ok(grid) = octree.rasterize(GAME_CHILD_ORDER) {
            for voxel in grid.voxels() {
                voxel_types[usize::from(voxel.ty)] += 1;
            }
            grids.push((octree, grid));
        }
    }
    print_histogram("node types (all nodes, incl. inner)", &node_types);
    print_histogram("voxel types (expanded leaves)", &voxel_types);
    print_histogram("leaf densities (leaf count)", &leaf_densities);

    println!();
    println!("child-order check: face-adjacent voxel pairs inside each octree,");
    println!("compared as empty (type 0) vs non-empty:");
    for order in AxisOrder::ALL {
        let mut pairs = Pairs::default();
        for (octree, _) in &grids {
            if let Ok(grid) = octree.rasterize(order) {
                pairs += Pairs::within_octree(&grid);
            }
        }
        println!("  order {order}:         {pairs}");
    }
    let mut control = Pairs::default();
    for (octree, _) in &grids {
        if let Ok(grid) = octree.rasterize_with(scrambled_child_offset) {
            control += Pairs::within_octree(&grid);
        }
    }
    let mut correct = Pairs::default();
    for (_, grid) in &grids {
        correct += Pairs::within_octree(grid);
    }
    println!("  scrambled control: {control}");
    println!(
        "  -> scrambling child order creates {:.2}x as many empty/solid faces",
        control.differ as f64 / correct.differ.max(1) as f64
    );
    println!("  (all six axis orders score the same: swapping axes only transposes an");
    println!("   octree. Use `orient` to tell them apart via octree and batch seams.)");

    Ok(if invalid == 0 && batch.octrees.len() == expected {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn print_histogram(title: &str, counts: &[u64; 256]) {
    let entries: Vec<String> = counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(value, n)| format!("{value}:{n}"))
        .collect();
    println!("{title}: {} distinct", entries.len());
    for line in entries.chunks(8) {
        println!("  {}", line.join("  "));
    }
}

#[derive(Default)]
struct Totals {
    files: usize,
    bytes: u64,
    versions: BTreeMap<i32, usize>,
    octrees_per_file: BTreeMap<usize, usize>,
    octrees: usize,
    nodes: u64,
    leaves: u64,
    leaves_per_depth: [u64; MAX_DEPTH + 1],
    max_nodes: usize,
    covered_voxels: u64,
    /// Batch Y row → (non-empty voxels, total voxels).
    by_row: BTreeMap<i32, (u64, u64)>,
    node_types: BTreeMap<u8, u64>,
    /// Leaves whose density says one side of the surface and type the other.
    side_conflicts: u64,
    errors: Vec<String>,
}

impl Totals {
    fn merge(&mut self, other: Totals) {
        self.files += other.files;
        self.bytes += other.bytes;
        for (k, v) in other.versions {
            *self.versions.entry(k).or_default() += v;
        }
        for (k, v) in other.octrees_per_file {
            *self.octrees_per_file.entry(k).or_default() += v;
        }
        self.octrees += other.octrees;
        self.nodes += other.nodes;
        self.leaves += other.leaves;
        for (a, b) in self.leaves_per_depth.iter_mut().zip(other.leaves_per_depth) {
            *a += b;
        }
        self.max_nodes = self.max_nodes.max(other.max_nodes);
        self.covered_voxels += other.covered_voxels;
        for (k, (a, b)) in other.by_row {
            let entry = self.by_row.entry(k).or_default();
            entry.0 += a;
            entry.1 += b;
        }
        for (k, v) in other.node_types {
            *self.node_types.entry(k).or_default() += v;
        }
        self.side_conflicts += other.side_conflicts;
        self.errors.extend(other.errors);
    }

    fn add_file(&mut self, game: &GameData, coord: BatchCoord, expected: Option<[usize; 3]>) {
        let bytes = match game.read_batch(coord) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return self.errors.push(format!("{coord}: file disappeared")),
            Err(e) => return self.errors.push(e.to_string()),
        };
        self.files += 1;
        self.bytes += bytes.len() as u64;
        let batch = match Batch::parse(&bytes) {
            Ok(batch) => batch,
            Err(e) => return self.errors.push(format!("{coord}: {e}")),
        };
        *self.versions.entry(batch.version).or_default() += 1;
        *self
            .octrees_per_file
            .entry(batch.octrees.len())
            .or_default() += 1;
        match expected {
            Some(dims) if dims.iter().product::<usize>() == batch.octrees.len() => {}
            Some(dims) => self.errors.push(format!(
                "{coord}: {} octrees, expected {dims:?}",
                batch.octrees.len()
            )),
            None => self
                .errors
                .push(format!("{coord}: batch outside the world")),
        }
        for (i, octree) in batch.octrees.iter().enumerate() {
            self.octrees += 1;
            for node in &octree.nodes {
                *self.node_types.entry(node.ty).or_default() += 1;
                let voxel = sn_octree::Voxel {
                    ty: node.ty,
                    density: node.density,
                };
                if node.is_leaf()
                    && node.density != 0
                    && (voxel.signed_density() > 0.0) != voxel.is_solid()
                {
                    self.side_conflicts += 1;
                }
            }
            match octree.validate() {
                Ok(stats) => {
                    self.nodes += stats.nodes as u64;
                    self.leaves += stats.leaves as u64;
                    self.max_nodes = self.max_nodes.max(stats.nodes);
                    self.covered_voxels += stats.covered_voxels as u64;
                    for (a, b) in self.leaves_per_depth.iter_mut().zip(stats.leaves_per_depth) {
                        *a += b as u64;
                    }
                    let row = self.by_row.entry(coord.y).or_default();
                    row.0 += stats.non_empty_voxels as u64;
                    row.1 += stats.covered_voxels as u64;
                    if stats.covered_voxels != VOXELS_PER_OCTREE {
                        self.errors.push(format!(
                            "{coord} octree {i}: covers {} voxels",
                            stats.covered_voxels
                        ));
                    }
                }
                Err(e) => self.errors.push(format!("{coord} octree {i}: {e}")),
            }
        }
    }
}

pub fn all(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let index = game.read_index()?;
    let (batches, unknown) = game.octree_batches()?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    println!(
        "decoding {} batch files from {} on {threads} threads...",
        batches.len(),
        game.octree_dir().display()
    );

    let next = AtomicUsize::new(0);
    let totals = Mutex::new(Totals::default());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut local = Totals::default();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&coord) = batches.get(i) else { break };
                    local.add_file(game, coord, index.batch_octree_dims(coord));
                }
                totals.lock().unwrap().merge(local);
            });
        }
    });
    let t = totals.into_inner().unwrap();
    let elapsed = start.elapsed();

    println!(
        "files:            {} ({:.1} MiB)",
        t.files,
        t.bytes as f64 / 1048576.0
    );
    println!("versions:         {:?}", t.versions);
    println!("octrees per file: {:?}", t.octrees_per_file);
    println!("octrees:          {}", t.octrees);
    println!(
        "nodes:            {} ({} leaves), largest octree {} nodes",
        t.nodes, t.leaves, t.max_nodes
    );
    println!("leaves by depth:  {:?}", t.leaves_per_depth);
    println!(
        "voxels covered:   {} (= {} octrees x {VOXELS_PER_OCTREE})",
        t.covered_voxels,
        t.covered_voxels / VOXELS_PER_OCTREE as u64
    );
    println!("distinct node types: {}", t.node_types.len());
    println!(
        "density vs type:  {} leaves where density (>= 126 solid) and type (0 empty) disagree",
        t.side_conflicts
    );
    println!("non-empty voxel share by batch row (Y), top row last:");
    for (y, (solid, total)) in &t.by_row {
        let share = *solid as f64 / (*total).max(1) as f64;
        let bar = "#".repeat((share * 50.0).round() as usize);
        println!("  Y={y:>2}  {:>6.2}%  {bar}", share * 100.0);
    }
    if !unknown.is_empty() {
        println!("unrecognised files in folder: {}", unknown.len());
    }
    println!("time:             {:.2} s", elapsed.as_secs_f64());

    if t.errors.is_empty() {
        println!("result:           OK, no errors");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("result:           {} ERRORS", t.errors.len());
        for e in t.errors.iter().take(20) {
            println!("  {e}");
        }
        Ok(ExitCode::FAILURE)
    }
}
