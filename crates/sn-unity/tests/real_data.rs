//! Tests against the player's own install. Opt-in:
//!
//! ```text
//! SUBNAUTICA_DIR=<folder containing Subnautica.exe> cargo test -p sn-unity -- --ignored
//! ```
//!
//! Expected numbers were measured on game build 10 and cross-checked against
//! UnityPy (see MODLOG, M5). They are facts about the data, not game content.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use sn_unity::{Bundle, SerializedFile};

fn unity_files() -> Option<Vec<PathBuf>> {
    let data = PathBuf::from(std::env::var_os("SUBNAUTICA_DIR")?).join("Subnautica_Data");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&data).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name()?.to_str()?.to_string();
        if name.ends_with(".assets") || name == "level0" || name == "globalgamemanagers" {
            files.push(path);
        }
    }
    let bundles = data.join("StreamingAssets/aa/StandaloneWindows64");
    for entry in std::fs::read_dir(bundles).ok()? {
        let path = entry.ok()?.path();
        if path.extension().is_some_and(|e| e == "bundle") {
            files.push(path);
        }
    }
    files.sort();
    Some(files)
}

#[derive(Default)]
struct Totals {
    files: usize,
    serialized: usize,
    objects: usize,
    with_type_trees: usize,
    errors: Vec<String>,
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn every_unity_file_parses() {
    let Some(files) = unity_files() else {
        eprintln!("SUBNAUTICA_DIR not set or not a Subnautica install; skipping");
        return;
    };
    let next = AtomicUsize::new(0);
    let totals = Mutex::new(Totals::default());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut t = Totals::default();
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    t.files += 1;
                    let bytes = std::fs::read(path).unwrap();
                    let mut check = |data: &[u8], name: &str| match SerializedFile::parse(data) {
                        Ok(file) => {
                            t.serialized += 1;
                            t.objects += file.objects.len();
                            t.with_type_trees += usize::from(file.type_tree_enabled);
                            for o in &file.objects {
                                if file.object_data(data, o).is_none() {
                                    t.errors
                                        .push(format!("{name}: object {} out of range", o.path_id));
                                }
                            }
                        }
                        Err(e) => t.errors.push(format!("{name}: {e}")),
                    };
                    let name = path.display().to_string();
                    if bytes.starts_with(b"UnityFS\0") {
                        match Bundle::parse(&bytes) {
                            Ok(bundle) => {
                                for node in bundle.nodes.iter().filter(|n| n.is_serialized_file()) {
                                    check(bundle.node_data(node), &format!("{name}/{}", node.path));
                                }
                            }
                            Err(e) => t.errors.push(format!("{name}: {e}")),
                        }
                    } else {
                        check(&bytes, &name);
                    }
                }
                let mut all = totals.lock().unwrap();
                all.files += t.files;
                all.serialized += t.serialized;
                all.objects += t.objects;
                all.with_type_trees += t.with_type_trees;
                all.errors.extend(t.errors);
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
    assert_eq!(t.files, 5472);
    assert_eq!(t.serialized, 5485);
    assert_eq!(t.objects, 423_677);
    assert_eq!(
        t.with_type_trees, 0,
        "type trees appeared: the game data changed"
    );
}
