//! `unity` command: list what is inside Unity files. The per-file output
//! matches the dev-time UnityPy oracle line for line, so the two can be diffed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use sn_install::GameData;
use sn_unity::{Bundle, SerializedFile, class_name};

use crate::Result;

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Facts gathered while describing files, for `--all` totals.
#[derive(Default)]
struct Totals {
    files: usize,
    bytes: u64,
    serialized: usize,
    objects: usize,
    resources: usize,
    typetree_files: usize,
    classes: BTreeMap<i32, usize>,
    versions: BTreeMap<String, usize>,
    errors: Vec<String>,
}

impl Totals {
    fn merge(&mut self, t: Totals) {
        self.files += t.files;
        self.bytes += t.bytes;
        self.serialized += t.serialized;
        self.objects += t.objects;
        self.resources += t.resources;
        self.typetree_files += t.typetree_files;
        for (k, v) in t.classes {
            *self.classes.entry(k).or_default() += v;
        }
        for (k, v) in t.versions {
            *self.versions.entry(k).or_default() += v;
        }
        self.errors.extend(t.errors);
    }
}

fn describe_serialized(
    name: &str,
    file: &SerializedFile,
    out: &mut Vec<String>,
    totals: &mut Totals,
) {
    let bytes: u64 = file.objects.iter().map(|o| u64::from(o.byte_size)).sum();
    out.push(format!(
        "SERIALIZED {name} version={} unity={} typetree={} types={} objects={} object_bytes={bytes} externals={}",
        file.version,
        file.unity_version,
        u8::from(file.type_tree_enabled),
        file.types.len(),
        file.objects.len(),
        file.externals.len(),
    ));
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for o in &file.objects {
        *counts.entry(o.class_id).or_default() += 1;
    }
    for (&class, &n) in &counts {
        out.push(format!("  class {class}: {n}"));
        *totals.classes.entry(class).or_default() += n;
    }
    totals.serialized += 1;
    totals.objects += file.objects.len();
    totals.typetree_files += usize::from(file.type_tree_enabled);
    *totals
        .versions
        .entry(format!("v{} {}", file.version, file.unity_version))
        .or_default() += 1;
}

/// Canonical description of one file (bundle or serialized file).
fn describe(
    path: &Path,
    bytes: &[u8],
    totals: &mut Totals,
) -> std::result::Result<Vec<String>, String> {
    let name = file_name(path);
    let mut out = Vec::new();
    if bytes.starts_with(b"UnityFS\0") {
        let bundle = Bundle::parse(bytes).map_err(|e| format!("{name}: {e}"))?;
        out.push(format!(
            "BUNDLE {name} format={} unity={} nodes={}",
            bundle.format,
            bundle.unity_version,
            bundle.nodes.len()
        ));
        for node in &bundle.nodes {
            let data = bundle.node_data(node);
            if node.is_serialized_file() {
                let file = SerializedFile::parse(data)
                    .map_err(|e| format!("{name}/{}: {e}", node.path))?;
                describe_serialized(&node.path, &file, &mut out, totals);
            } else {
                out.push(format!("  RESOURCE {} size={}", node.path, data.len()));
                totals.resources += 1;
            }
        }
    } else {
        let file = SerializedFile::parse(bytes).map_err(|e| format!("{name}: {e}"))?;
        describe_serialized(&name, &file, &mut out, totals);
    }
    Ok(out)
}

/// Lists the given files (relative to Subnautica_Data, or absolute).
pub fn list(game: &GameData, paths: &[&str]) -> Result<ExitCode> {
    for path in paths {
        let path = PathBuf::from(path);
        let bytes = game.read_file(&path)?;
        for line in describe(&path, &bytes, &mut Totals::default())? {
            println!("{line}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Like `list`, but names each class (for humans, not for diffing).
pub fn types(game: &GameData, path: &str) -> Result<ExitCode> {
    let bytes = game.read_file(Path::new(path))?;
    for line in describe(Path::new(path), &bytes, &mut Totals::default())? {
        let class = line
            .trim()
            .strip_prefix("class ")
            .and_then(|l| l.split(':').next())
            .and_then(|id| id.parse().ok());
        match class {
            Some(id) => println!("{line}  ({})", class_name(id).unwrap_or("?")),
            None => println!("{line}"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Parses every serialized file and bundle of the game and prints totals.
pub fn all(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let mut paths = game.serialized_files()?;
    paths.extend(game.bundles()?);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    println!(
        "parsing {} Unity files on {threads} threads...",
        paths.len()
    );

    let next = AtomicUsize::new(0);
    let totals = Mutex::new(Totals::default());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut t = Totals::default();
                while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                    match game.read_file(path) {
                        Ok(bytes) => {
                            t.files += 1;
                            t.bytes += bytes.len() as u64;
                            if let Err(e) = describe(path, &bytes, &mut t) {
                                t.errors.push(e);
                            }
                        }
                        Err(e) => t.errors.push(e.to_string()),
                    }
                }
                totals.lock().unwrap().merge(t);
            });
        }
    });
    let t = totals.into_inner().unwrap();
    println!(
        "files:              {} ({:.2} GiB)",
        t.files,
        t.bytes as f64 / (1u64 << 30) as f64
    );
    println!(
        "serialized files:   {} (with type trees: {})",
        t.serialized, t.typetree_files
    );
    println!("resource blobs:     {}", t.resources);
    println!("objects:            {}", t.objects);
    println!("format versions:    {:?}", t.versions);
    let mut by_count: Vec<(i32, usize)> = t.classes.into_iter().collect();
    by_count.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("most common classes:");
    for (class, n) in by_count.iter().take(25) {
        println!("  {class:>5} {:<26} {n}", class_name(*class).unwrap_or("?"));
    }
    println!("time:               {:.2} s", start.elapsed().as_secs_f64());
    if t.errors.is_empty() {
        println!("result:             OK, no errors");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("result:             {} ERRORS", t.errors.len());
        for e in t.errors.iter().take(20) {
            println!("  {e}");
        }
        Ok(ExitCode::FAILURE)
    }
}
