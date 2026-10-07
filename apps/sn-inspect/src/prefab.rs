//! `prefab` command: load prefabs through the Addressables catalog, decode
//! their meshes, export them as OBJ to `out/`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, Prefab};
use sn_install::GameData;
use sn_unity::{Material, MeshGeometry};

use crate::Result;

/// Unity (left-handed) → OBJ (right-handed): flip z.
fn to_obj(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

fn material_name(object: &Option<sn_assets::ObjectRef>) -> String {
    let Some(object) = object else {
        return "(none)".into();
    };
    object
        .data()
        .ok()
        .and_then(|(_, d)| Material::parse(d, object.file.file().big_endian).ok())
        .map_or("(unreadable)".into(), |m| m.name)
}

/// One prefab: its hierarchy, meshes and materials, and an OBJ export of
/// what the game draws at full detail.
pub fn one(game: &GameData, key: &str) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let prefab = assets.prefab(&catalog, key)?;
    println!(
        "{key}: {} nodes (loaded in {:.2} s)",
        prefab.nodes.len(),
        start.elapsed().as_secs_f64()
    );
    let mut obj = String::new();
    let mut base = 1u32;
    for (i, node) in prefab.nodes.iter().enumerate() {
        let depth = std::iter::successors(node.parent, |&p| prefab.nodes[p].parent).count();
        let t = &node.local;
        let mut line = format!(
            "{}{} (pos {:?} rot {:?} scale {:?}) ",
            "  ".repeat(depth + 1),
            node.name,
            t.position.map(|v| (v * 1000.0).round() / 1000.0),
            t.rotation.map(|v| (v * 1000.0).round() / 1000.0),
            t.scale.map(|v| (v * 1000.0).round() / 1000.0)
        );
        if !node.active {
            line.push_str("[inactive] ");
        }
        if let Some(l) = node.lod {
            let _ = write!(line, "[LOD {l}] ");
        }
        if let Some(mesh) = &node.mesh {
            let (m, g) = assets.mesh(mesh)?;
            let triangles: usize = g.sub_meshes.iter().map(|s| s.len() / 3).sum();
            let _ = write!(
                line,
                "mesh {:?}: {} vertices, {} triangles in {} sub-meshes{}; materials {:?}",
                m.name,
                g.positions.len(),
                triangles,
                g.sub_meshes.len(),
                if node.renderer_enabled {
                    ""
                } else {
                    " (renderer off)"
                },
                node.materials.iter().map(material_name).collect::<Vec<_>>()
            );
            if node.active && node.renderer_enabled && node.lod.is_none_or(|l| l == 0) {
                let _ = writeln!(obj, "o {}_{i}", node.name.replace(' ', "_"));
                // Standing alone, the prefab keeps its root's rotation and
                // scale (placed in the world, the placement replaces them).
                // The root's position is where it sat in the artist's
                // scene, so it is left out: the pivot stays at the origin.
                let root = sn_world::Transform {
                    position: [0.0; 3],
                    ..prefab.nodes[0].local
                };
                let t = root.then(&node.in_prefab);
                append_obj(&mut obj, &g, &t, &mut base);
            }
        }
        println!("{line}");
    }
    let name = key
        .rsplit('/')
        .next()
        .unwrap_or(key)
        .trim_end_matches(".prefab");
    let path = Path::new("out/prefabs").join(format!("{}.obj", name.replace(' ', "_")));
    std::fs::create_dir_all("out/prefabs").map_err(|e| e.to_string())?;
    std::fs::write(&path, obj).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "wrote {} (Y up, Unity's z flipped; LOD 0 of active renderers)",
        path.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn append_obj(obj: &mut String, g: &MeshGeometry, t: &sn_world::Transform, base: &mut u32) {
    for p in &g.positions {
        let w = t.then(&sn_world::Transform {
            position: *p,
            ..Default::default()
        });
        let [x, y, z] = to_obj(w.position);
        let _ = writeln!(obj, "v {x} {y} {z}");
    }
    for s in &g.sub_meshes {
        for tri in s.chunks_exact(3) {
            // Flipping z mirrors the mesh, so reverse the winding.
            let _ = writeln!(
                obj,
                "f {} {} {}",
                *base + tri[0],
                *base + tri[2],
                *base + tri[1],
            );
        }
    }
    *base += g.positions.len() as u32;
}

/// Every prefab placed in the world (from `CellsCache` and
/// `BatchObjectsCache`): load it, decode every mesh, report totals.
/// With `oracle`, also write one line per decoded mesh to
/// `out/mesh-check-rust.txt` for comparison with UnityPy.
pub fn placed(game: &GameData, oracle: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let prefabs = game.read_prefab_database()?;
    let mut keys = BTreeSet::new();
    let (batches, _) = game.cell_batches()?;
    for coord in batches {
        let Some(file) = game.read_batch_cells(coord)? else {
            continue;
        };
        for tree in file.cells.iter().filter_map(|c| c.objects.as_ref()) {
            for o in &tree.objects {
                if let Some(path) = prefabs.get(&o.class_id) {
                    keys.insert(path.clone());
                }
            }
        }
    }
    let (batches, _) = game.object_batches()?;
    for coord in batches {
        let Some(tree) = game.read_batch_objects(coord)? else {
            continue;
        };
        for o in &tree.objects {
            if let Some(path) = prefabs.get(&o.class_id) {
                keys.insert(path.clone());
            }
        }
    }
    println!("placed prefabs: {}", keys.len());
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    println!(
        "catalog: {} keys, {} entries; indexing took {:.1} s",
        catalog.key_count(),
        catalog.entry_count(),
        start.elapsed().as_secs_f64()
    );

    let mut errors: Vec<String> = Vec::new();
    let mut loaded = 0;
    let mut without_mesh = Vec::new();
    let (mut nodes, mut visible, mut vertices, mut triangles) = (0usize, 0usize, 0usize, 0usize);
    let mut meshes: BTreeMap<(std::path::PathBuf, String, i64), String> = BTreeMap::new();
    let mut lod_groups = 0;
    let mut skinned: Vec<String> = Vec::new();
    for key in &keys {
        let prefab: Prefab = match assets.prefab(&catalog, key) {
            Ok(p) => p,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        loaded += 1;
        nodes += prefab.nodes.len();
        if prefab.nodes.iter().any(|n| n.lod.is_some()) {
            lod_groups += 1;
        }
        if prefab.nodes.iter().any(|n| n.active && n.skinned) {
            skinned.push(key.clone());
        }
        let mut any = false;
        for node in prefab.visible_nodes() {
            let Some(mesh) = &node.mesh else { continue };
            any = true;
            visible += 1;
            if meshes.contains_key(&mesh.key()) {
                continue;
            }
            match assets.mesh(mesh) {
                Ok((m, g)) => {
                    vertices += g.positions.len();
                    let tris: Vec<usize> = g.sub_meshes.iter().map(|s| s.len() / 3).collect();
                    triangles += tris.iter().sum::<usize>();
                    let sum = g.positions.iter().fold([0f64; 3], |mut acc, p| {
                        for a in 0..3 {
                            acc[a] += f64::from(p[a]);
                        }
                        acc
                    });
                    meshes.insert(
                        mesh.key(),
                        format!(
                            "{}	{}	{}	{}	{:?}	{:.2}	{:.2}	{:.2}",
                            mesh.file
                                .bundle
                                .path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("?"),
                            mesh.path_id,
                            m.name,
                            g.positions.len(),
                            tris,
                            sum[0],
                            sum[1],
                            sum[2]
                        ),
                    );
                }
                Err(e) => errors.push(format!("{key}: {e}")),
            }
        }
        if !any {
            without_mesh.push(key.clone());
        }
    }
    println!(
        "loaded {loaded} prefabs ({lod_groups} with LOD groups): {nodes} nodes, {visible} visible mesh nodes"
    );
    println!(
        "distinct meshes decoded: {} ({vertices} vertices, {triangles} triangles)",
        meshes.len()
    );
    println!(
        "prefabs without a visible mesh: {} (e.g. {:?})",
        without_mesh.len(),
        without_mesh.iter().take(5).collect::<Vec<_>>()
    );
    let skinned_only: Vec<&String> = skinned
        .iter()
        .filter(|k| without_mesh.contains(k))
        .collect();
    println!(
        "prefabs with skinned meshes (not read yet): {} ({} with nothing else to draw, e.g. {:?})",
        skinned.len(),
        skinned_only.len(),
        skinned_only.iter().take(4).collect::<Vec<_>>()
    );
    println!("errors: {}", errors.len());
    // Group by message with numbers and ids removed.
    let mut kinds: BTreeMap<String, (usize, &String)> = BTreeMap::new();
    for e in &errors {
        let kind: String = e
            .split(": ")
            .last()
            .unwrap_or(e)
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect();
        kinds.entry(kind).or_insert((0, e)).0 += 1;
    }
    for (kind, (n, example)) in &kinds {
        println!(
            "  {n} × {kind}
      e.g. {example}"
        );
    }
    if oracle {
        let text: String = meshes.values().map(|l| format!("{l}\n")).collect();
        std::fs::create_dir_all("out").map_err(|e| e.to_string())?;
        std::fs::write("out/mesh-check-rust.txt", text).map_err(|e| e.to_string())?;
        println!("wrote out/mesh-check-rust.txt");
    }
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
