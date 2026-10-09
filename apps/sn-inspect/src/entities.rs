//! `entities` command: the world's saved objects (batch objects and baked
//! cells), with their prefabs resolved through `prefabs.db`.

use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::time::Instant;

use sn_install::GameData;
use sn_world::{BatchCoord, ObjectTree, Transform, VOXEL_WORLD_OFFSET};

use crate::Result;

/// Objects whose world position is further than this outside their batch
/// count as misplaced.
const MARGIN: f32 = 1.0;

/// World-space box of a batch (Unity coordinates).
pub(crate) fn batch_box(coord: BatchCoord, size: [i32; 3]) -> ([f32; 3], [f32; 3]) {
    let c = [coord.x, coord.y, coord.z];
    let lo: [f32; 3] = std::array::from_fn(|a| (c[a] * size[a]) as f32 - VOXEL_WORLD_OFFSET[a]);
    let hi: [f32; 3] = std::array::from_fn(|a| lo[a] + size[a] as f32);
    (lo, hi)
}

/// How far `p` lies outside the box (0 inside).
pub(crate) fn distance_outside(p: [f32; 3], (lo, hi): ([f32; 3], [f32; 3])) -> f32 {
    (0..3)
        .map(|a| (lo[a] - p[a]).max(p[a] - hi[a]).max(0.0))
        .fold(0.0, f32::max)
}

fn outside(p: [f32; 3], bounds: ([f32; 3], [f32; 3])) -> bool {
    distance_outside(p, bounds) > MARGIN
}

/// FNV-1a, to tell whether two runs saw exactly the same data.
#[derive(Clone, Copy)]
pub(crate) struct Hash(pub(crate) u64);

impl Hash {
    pub(crate) fn new() -> Hash {
        Hash(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn add(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub(crate) fn add_transform(&mut self, t: &Transform) {
        for v in t.position.iter().chain(&t.rotation).chain(&t.scale) {
            self.add(&v.to_le_bytes());
        }
    }
}

#[derive(Default)]
struct Totals {
    files: usize,
    cells: usize,
    cells_per_level: BTreeMap<u32, usize>,
    objects: usize,
    roots: usize,
    no_class: usize,
    unresolved: BTreeMap<String, usize>,
    orphans: usize,
    misplaced: usize,
    /// Objects saved at exactly the world origin, far from their batch.
    at_origin: usize,
    /// Batch roots stored away from their batch's corner (moved back there).
    roots_moved: usize,
    /// Furthest distance outside the batch, in metres.
    misplaced_max: f32,
    /// The furthest few: (distance, description).
    misplaced_worst: Vec<(f32, String)>,
    components: BTreeMap<String, usize>,
    prefabs: HashMap<String, usize>,
    /// Per cell level: the cell roots' offsets from their batch corner, per axis.
    root_offsets: BTreeMap<u32, [Vec<f32>; 3]>,
}

impl Totals {
    fn add_tree(
        &mut self,
        tree: &ObjectTree,
        prefabs: &HashMap<String, String>,
        bounds: ([f32; 3], [f32; 3]),
        hash: &mut Hash,
    ) -> Vec<Transform> {
        let (world, orphans) = tree.world_transforms();
        self.orphans += orphans;
        for (object, t) in tree.objects.iter().zip(&world) {
            self.objects += 1;
            if object.parent.is_none() {
                self.roots += 1;
            }
            for c in &object.components {
                *self.components.entry(c.type_name.clone()).or_default() += 1;
            }
            if object.class_id.is_empty() {
                self.no_class += 1;
            } else {
                match prefabs.get(&object.class_id) {
                    Some(path) => *self.prefabs.entry(path.clone()).or_default() += 1,
                    None => *self.unresolved.entry(object.class_id.clone()).or_default() += 1,
                }
            }
            if outside(t.position, bounds) && t.position.iter().all(|v| v.abs() < 1e-3) {
                self.at_origin += 1;
            } else if outside(t.position, bounds) {
                self.misplaced += 1;
                let d = distance_outside(t.position, bounds);
                self.misplaced_max = self.misplaced_max.max(d);
                self.misplaced_worst.push((
                    d,
                    format!(
                        "{:.0} m: {} at {:?}, parent {}",
                        d,
                        prefab_name(prefabs, &object.class_id),
                        t.position.map(|v| v.round()),
                        object.parent.is_some()
                    ),
                ));
                self.misplaced_worst.sort_by(|a, b| b.0.total_cmp(&a.0));
                self.misplaced_worst.truncate(6);
            }
            hash.add(object.class_id.as_bytes());
            hash.add_transform(t);
        }
        world
    }
}

fn prefab_name<'a>(prefabs: &'a HashMap<String, String>, class_id: &str) -> &'a str {
    if class_id.is_empty() {
        return "(no prefab)";
    }
    prefabs
        .get(class_id)
        .map_or("(unknown ClassId)", String::as_str)
}

/// One batch: its objects and cells, with prefab paths and world positions.
pub fn one(game: &GameData, coord: BatchCoord) -> Result<ExitCode> {
    let index = game.read_index()?;
    let size = sn_terrain::batch_voxels(&index);
    let bounds = batch_box(coord, size);
    let prefabs = game.read_prefab_database()?;
    println!("batch {coord}: world box {:?} .. {:?}", bounds.0, bounds.1);
    let show = |tree: &ObjectTree, limit: usize| {
        let (world, _) = tree.world_transforms();
        for (object, t) in tree.objects.iter().zip(&world).take(limit) {
            let p = t.position;
            println!(
                "    {:<8} ({:8.2} {:8.2} {:8.2}) scale {:.2}  {}{}",
                &object.id.get(..8).unwrap_or(&object.id),
                p[0],
                p[1],
                p[2],
                t.scale[0],
                prefab_name(&prefabs, &object.class_id),
                if outside(p, bounds) { "  OUTSIDE" } else { "" }
            );
        }
        if tree.objects.len() > limit {
            println!("    … {} more", tree.objects.len() - limit);
        }
    };
    match game.read_batch_objects(coord)? {
        Some(tree) => {
            println!(
                "batch objects: {} (version {})",
                tree.objects.len(),
                tree.version
            );
            show(&tree, 20);
        }
        None => println!("batch objects: no file"),
    }
    match game.read_batch_cells(coord)? {
        Some(cells) => {
            let objects: usize = cells
                .cells
                .iter()
                .filter_map(|c| c.objects.as_ref())
                .map(|t| t.objects.len())
                .sum();
            println!(
                "baked cells: {} cells, {} objects (version {})",
                cells.cells.len(),
                objects,
                cells.version
            );
            for cell in cells.cells.iter().filter(|c| c.objects.is_some()).take(6) {
                let tree = cell.objects.as_ref().map_or(0, |t| t.objects.len());
                println!("  cell {:?} level {}: {tree} objects", cell.id, cell.level);
                if let Some(tree) = &cell.objects {
                    show(tree, 6);
                }
            }
        }
        None => println!("baked cells: no file"),
    }
    Ok(ExitCode::SUCCESS)
}

/// Smallest positive gap between sorted distinct values (the grid spacing).
fn spacing(values: &mut [f32]) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    values
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 1e-3)
        .min_by(f32::total_cmp)
}

/// Every file of both caches: totals, problems, and a hash of everything read.
pub fn all(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let index = game.read_index()?;
    let size = sn_terrain::batch_voxels(&index);
    let prefabs = game.read_prefab_database()?;
    println!("prefabs.db: {} ClassIds", prefabs.len());

    let mut hash = Hash::new();
    let mut errors = Vec::new();

    let (batches, unknown) = game.object_batches()?;
    let mut objects = Totals::default();
    for &coord in &batches {
        match game.read_batch_objects(coord) {
            Ok(Some(mut tree)) => {
                objects.files += 1;
                let bounds = batch_box(coord, size);
                // Hypothesis: the game puts the batch root at the batch's
                // corner itself; a few files store it elsewhere.
                for root in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
                    if root.transform.position != bounds.0 {
                        objects.roots_moved += 1;
                        root.transform.position = bounds.0;
                    }
                }
                objects.add_tree(&tree, &prefabs, bounds, &mut hash);
            }
            Ok(None) => {}
            Err(e) => errors.push(e.0),
        }
    }
    report("BatchObjectsCache", &objects, &unknown);

    let (batches, unknown) = game.cell_batches()?;
    let mut cells = Totals::default();
    for &coord in &batches {
        let bounds = batch_box(coord, size);
        match game.read_batch_cells(coord) {
            Ok(Some(file)) => {
                cells.files += 1;
                for cell in &file.cells {
                    cells.cells += 1;
                    *cells.cells_per_level.entry(cell.level).or_default() += 1;
                    hash.add(&cell.level.to_le_bytes());
                    for v in cell.id {
                        hash.add(&v.to_le_bytes());
                    }
                    let Some(tree) = &cell.objects else { continue };
                    let world = cells.add_tree(tree, &prefabs, bounds, &mut hash);
                    let offsets = cells.root_offsets.entry(cell.level).or_default();
                    for (object, t) in tree.objects.iter().zip(&world) {
                        if object.parent.is_none() {
                            for (a, axis) in offsets.iter_mut().enumerate() {
                                axis.push(t.position[a] - bounds.0[a]);
                            }
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => errors.push(e.0),
        }
    }
    report("CellsCache", &cells, &unknown);
    println!("  cells per level: {:?}", cells.cells_per_level);
    for (level, offsets) in &mut cells.root_offsets {
        let gaps: Vec<String> = offsets
            .iter_mut()
            .map(|v| spacing(v).map_or("-".into(), |s| format!("{s}")))
            .collect();
        let range: Vec<String> = offsets
            .iter()
            .map(|v| match (v.first(), v.last()) {
                (Some(a), Some(b)) => format!("{a}..{b}"),
                _ => "-".into(),
            })
            .collect();
        println!(
            "  level {level}: cell roots at offsets {} (x, y, z) from the batch corner, spacing {}",
            range.join(" / "),
            gaps.join(" / ")
        );
    }

    let mut top: Vec<(&String, &usize)> = cells.prefabs.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("  most placed prefabs:");
    for (path, n) in top.iter().take(12) {
        println!("    {n:>7}  {path}");
    }

    println!("errors: {}", errors.len());
    for e in errors.iter().take(10) {
        println!("  {e}");
    }
    println!("hash of all objects read: {:016x}", hash.0);
    println!("time: {:.2} s", start.elapsed().as_secs_f64());
    let ok = errors.is_empty()
        && objects.unresolved.is_empty()
        && cells.unresolved.is_empty()
        && objects.orphans + cells.orphans == 0;
    println!("result: {}", if ok { "OK" } else { "PROBLEMS (see above)" });
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn report(name: &str, t: &Totals, unknown: &[String]) {
    println!(
        "{name}: {} files ({} other files), {} objects ({} roots), {} without a prefab, {} distinct prefabs",
        t.files,
        unknown.len(),
        t.objects,
        t.roots,
        t.no_class,
        t.prefabs.len()
    );
    println!(
        "  unresolved ClassIds: {} ({} objects); missing parents: {}",
        t.unresolved.len(),
        t.unresolved.values().sum::<usize>(),
        t.orphans,
    );
    println!(
        "  batch roots moved to their corner: {}; at the world origin: {}; otherwise outside their batch (> {MARGIN} m): {} (up to {:.1} m)",
        t.roots_moved, t.at_origin, t.misplaced, t.misplaced_max
    );
    for (_, w) in &t.misplaced_worst {
        println!("    {w}");
    }
    let components: Vec<String> = t
        .components
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    println!("  components: {}", components.join(", "));
}
