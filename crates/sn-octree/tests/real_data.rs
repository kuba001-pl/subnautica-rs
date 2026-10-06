//! Tests against the player's own install. Opt-in:
//!
//! ```text
//! SUBNAUTICA_DIR=<folder containing Subnautica.exe> cargo test -p sn-octree -- --ignored
//! ```
//!
//! The expected numbers below were measured on game build 10 (data `Build18`).
//! They are facts about the data, not game content.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use sn_octree::{Batch, FORMAT_VERSION, VOXELS_PER_OCTREE};

fn octree_dir() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("SUBNAUTICA_DIR")?);
    let unmanaged = root.join("Subnautica_Data/StreamingAssets/SNUnmanagedData");
    let build = std::fs::read_dir(&unmanaged)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n: u32 = e
                .file_name()
                .to_str()?
                .strip_prefix("Build")?
                .parse()
                .ok()?;
            Some((n, e.path()))
        })
        .max_by_key(|(n, _)| *n)?
        .1;
    Some(build.join("CompiledOctreesCache"))
}

#[derive(Default)]
struct Totals {
    files: usize,
    full: usize,
    partial: usize,
    max_nodes: usize,
    errors: Vec<String>,
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn every_batch_decodes_and_validates() {
    let Some(dir) = octree_dir() else {
        eprintln!("SUBNAUTICA_DIR not set or not a Subnautica install; skipping");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "optoctrees"))
        .collect();
    files.sort();

    let next = AtomicUsize::new(0);
    let totals = Mutex::new(Totals::default());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut local = Totals::default();
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let name = path.file_name().unwrap().to_string_lossy();
                    local.files += 1;
                    let bytes = std::fs::read(path).unwrap();
                    let batch = match Batch::parse(&bytes) {
                        Ok(batch) => batch,
                        Err(e) => {
                            local.errors.push(format!("{name}: {e}"));
                            continue;
                        }
                    };
                    assert_eq!(batch.version, FORMAT_VERSION);
                    match batch.octrees.len() {
                        125 => local.full += 1,
                        75 => local.partial += 1,
                        n => local.errors.push(format!("{name}: {n} octrees")),
                    }
                    for (i, octree) in batch.octrees.iter().enumerate() {
                        match octree.validate() {
                            Ok(stats) if stats.covered_voxels == VOXELS_PER_OCTREE => {
                                local.max_nodes = local.max_nodes.max(stats.nodes);
                            }
                            Ok(stats) => local.errors.push(format!(
                                "{name} octree {i}: covers {} voxels",
                                stats.covered_voxels
                            )),
                            Err(e) => local.errors.push(format!("{name} octree {i}: {e}")),
                        }
                    }
                }
                let mut t = totals.lock().unwrap();
                t.files += local.files;
                t.full += local.full;
                t.partial += local.partial;
                t.max_nodes = t.max_nodes.max(local.max_nodes);
                t.errors.extend(local.errors);
            });
        }
    });
    let t = totals.into_inner().unwrap();

    assert!(
        t.errors.is_empty(),
        "{} errors, first: {:?}",
        t.errors.len(),
        &t.errors[..t.errors.len().min(10)]
    );
    assert_eq!(t.files, 5416);
    assert_eq!((t.full, t.partial), (5259, 157));
    assert_eq!(t.max_nodes, 37_449);
}
