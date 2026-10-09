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
            if node.skinned {
                line.push_str(&format!(" [skinned: {} bones]", node.bones.len()));
            }
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
                match prefab.skinned_geometry(i, &m, &g) {
                    // Already in the prefab root's space.
                    Some(skinned) => append_obj(&mut obj, &skinned, &root, &mut base),
                    None => append_obj(&mut obj, &g, &root.then(&node.in_prefab), &mut base),
                }
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
/// Every prefab placed in the world (baked cells and batch objects), with
/// its number of placements.
fn placed_prefabs(game: &GameData) -> Result<BTreeMap<String, usize>> {
    Ok(placed_positions(game)?
        .into_iter()
        .map(|(k, v)| (k, v.len()))
        .collect())
}

/// Every placed prefab with the world (Unity) positions of its placements.
pub fn placed_positions(game: &GameData) -> Result<BTreeMap<String, Vec<[f32; 3]>>> {
    let prefabs = game.read_prefab_database()?;
    let mut keys: BTreeMap<String, Vec<[f32; 3]>> = BTreeMap::new();
    let (batches, _) = game.cell_batches()?;
    for coord in batches {
        let Some(file) = game.read_batch_cells(coord)? else {
            continue;
        };
        for tree in file.cells.iter().filter_map(|c| c.objects.as_ref()) {
            let (world, _) = tree.world_transforms();
            for (o, t) in tree.objects.iter().zip(world) {
                if let Some(path) = prefabs.get(&o.class_id) {
                    keys.entry(path.clone()).or_default().push(t.position);
                }
            }
        }
    }
    let (batches, _) = game.object_batches()?;
    for coord in batches {
        let Some(mut tree) = game.read_batch_objects(coord)? else {
            continue;
        };
        // Roots sit at the batch's corner (docs/formats/entities.md).
        let corner = [coord.x, coord.y, coord.z]
            .map(|c| c as f32 * 160.0)
            .iter()
            .zip(sn_world::VOXEL_WORLD_OFFSET)
            .map(|(c, o)| c - o)
            .collect::<Vec<_>>();
        for o in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
            o.transform.position = [corner[0], corner[1], corner[2]];
        }
        let (world, _) = tree.world_transforms();
        for (o, t) in tree.objects.iter().zip(world) {
            if let Some(path) = prefabs.get(&o.class_id) {
                keys.entry(path.clone()).or_default().push(t.position);
            }
        }
    }
    Ok(keys)
}

/// `prefab --lights`: the Light components of every placed prefab, and how
/// many lights the world's placements hold.
pub fn lights(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let placed = placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut errors = 0;
    let mut with_lights = 0;
    // 50 m columns (x, z) → lit-at-start realtime lights placed there, the
    // sum of their heights (positions of the placements, not of the lights).
    let mut columns: BTreeMap<(i32, i32), (usize, f32)> = BTreeMap::new();
    // 50 m column → prefab → its lights there.
    let mut column_prefabs: BTreeMap<(i32, i32), BTreeMap<&str, usize>> = BTreeMap::new();
    // The same for the glowing coral alone (Doodads/Coral_reef_Light).
    let mut coral: BTreeMap<(i32, i32), (usize, f32)> = BTreeMap::new();
    // (kind, shadows, realtime, enabled and active) → (lights, placed lights)
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_dir: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut masks: BTreeMap<u32, usize> = BTreeMap::new();
    let mut ranges: Vec<f32> = Vec::new();
    for (key, positions) in &placed {
        let count = positions.len();
        let prefab = match assets.prefab(&catalog, key) {
            Ok(p) => p,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        let lights: Vec<(&sn_unity::Light, bool)> = prefab
            .nodes
            .iter()
            .flat_map(|n| n.lights.iter().map(move |l| (l, n.active)))
            .collect();
        if lights.is_empty() {
            continue;
        }
        with_lights += 1;
        let dir = key.split('/').take(3).collect::<Vec<_>>().join("/");
        let d = by_dir.entry(dir).or_default();
        d.0 += 1;
        d.1 += count * lights.len();
        for (l, active) in lights {
            let on = l.enabled && active;
            let name = format!(
                "{:?} shadows {:?} {} {}{}",
                l.kind,
                l.shadows,
                if l.is_realtime() { "realtime" } else { "baked" },
                if on { "on" } else { "off at start" },
                if l.cookie.is_null() {
                    ""
                } else {
                    " with cookie"
                }
            );
            let e = kinds.entry(name).or_default();
            e.0 += 1;
            e.1 += count;
            *masks.entry(l.culling_mask).or_default() += 1;
            if on && l.is_realtime() {
                ranges.push(l.range);
                if !key.starts_with("WorldEntities/Creatures/") {
                    for p in positions {
                        let c = ((p[0] / 50.0).floor() as i32, (p[2] / 50.0).floor() as i32);
                        let e = columns.entry(c).or_default();
                        e.0 += 1;
                        e.1 += p[1];
                        *column_prefabs.entry(c).or_default().entry(key).or_default() += 1;
                        if key.starts_with("WorldEntities/Doodads/Coral_reef_Light/") {
                            let e = coral.entry(c).or_default();
                            e.0 += 1;
                            e.1 += p[1];
                        }
                    }
                }
            }
        }
        assets.trim_cache(512 << 20);
    }
    println!(
        "placed prefabs: {}; with lights: {with_lights}; unreadable: {errors}",
        placed.len()
    );
    println!("lights (in prefabs, in the world's placements):");
    for (k, (n, w)) in &kinds {
        println!("  {k}: {n}, {w}");
    }
    println!("culling masks: {masks:?}");
    ranges.sort_by(f32::total_cmp);
    if let (Some(first), Some(last)) = (ranges.first(), ranges.last()) {
        println!(
            "range of lit-at-start realtime lights: min {first}, median {}, max {last}",
            ranges[ranges.len() / 2]
        );
    }
    println!("by folder (prefabs with lights, lights in placements):");
    for (d, (n, w)) in &by_dir {
        println!("  {d}: {n}, {w}");
    }
    let mut dense: Vec<_> = columns.into_iter().collect();
    dense.sort_by_key(|e| std::cmp::Reverse(e.1.0));
    println!(
        "densest 50 m columns of lit-at-start lights, without creatures (x, z, lights, mean y):"
    );
    for ((x, z), (n, y)) in dense.iter().take(10) {
        let top = column_prefabs
            .get(&(*x, *z))
            .and_then(|m| m.iter().max_by_key(|(_, n)| **n))
            .map(|(k, n)| format!("{k} ({n})"))
            .unwrap_or_default();
        println!(
            "  {}, {}: {n}, {:.0}; most: {top}",
            x * 50 + 25,
            z * 50 + 25,
            y / *n as f32
        );
    }
    let mut dense: Vec<_> = coral.into_iter().collect();
    dense.sort_by_key(|e| std::cmp::Reverse(e.1.0));
    println!("densest 50 m columns of glowing coral lights (x, z, lights, mean y):");
    for ((x, z), (n, y)) in dense.iter().take(10) {
        println!(
            "  {}, {}: {n}, {:.0}",
            x * 50 + 25,
            z * 50 + 25,
            y / *n as f32
        );
    }
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}

pub fn placed(game: &GameData, oracle: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let keys: BTreeSet<String> = placed_prefabs(game)?.into_keys().collect();
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
        "prefabs with skinned meshes: {} ({} with nothing else to draw, e.g. {:?})",
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

/// The strings of a shader object that look like a shader name
/// (`Group/Name`): a length-prefixed run of name characters. A heuristic for
/// inspection only: the name is stored deep in the serialized shader
/// (`m_ParsedForm.m_Name`), which we don't parse.
fn shader_names(object: &sn_assets::ObjectRef) -> String {
    let Ok((_, data)) = object.data() else {
        return "(unreadable)".into();
    };
    let mut names = BTreeSet::new();
    for at in 0..data.len().saturating_sub(4) {
        let Some(len) = data.get(at..at + 4) else {
            break;
        };
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        if !(5..=96).contains(&len) {
            continue;
        }
        let Some(run) = data.get(at + 4..at + 4 + len) else {
            continue;
        };
        let Ok(s) = std::str::from_utf8(run) else {
            continue;
        };
        if s.contains('/')
            && s.starts_with(|c: char| c.is_ascii_uppercase())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_- ()".contains(c))
        {
            names.insert(s.to_string());
        }
    }
    let names: Vec<_> = names.into_iter().take(3).collect();
    if names.is_empty() {
        format!("(no name found, {} bytes)", data.len())
    } else {
        names.join(" | ")
    }
}

/// `prefab --materials`: every material on a drawn node of the placed
/// prefabs and the startup scenes (Aurora intact, escape pod at the origin),
/// with what our object shader makes of it.
pub fn materials(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let placed = placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    #[derive(Default)]
    struct Use {
        nodes: usize,
        placements: usize,
        node_names: BTreeSet<String>,
        meshes: BTreeSet<String>,
        prefabs: BTreeSet<String>,
        layers: BTreeSet<u32>,
        /// Where the first placement's root is (world, Unity axes).
        first: Option<[f32; 3]>,
        desc: String,
    }
    let mut uses: BTreeMap<String, Use> = BTreeMap::new();
    let mut mesh_names: BTreeMap<(std::path::PathBuf, String, i64), String> = BTreeMap::new();
    let mut add = |prefab: &Prefab, key: &str, count: usize, at: Option<[f32; 3]>| {
        for node in prefab.visible_nodes() {
            for material in node.materials.iter().flatten() {
                let Ok((_, data)) = material.data() else {
                    continue;
                };
                let Ok(m) = Material::parse(data, material.file.file().big_endian) else {
                    continue;
                };
                let u = uses.entry(m.name.clone()).or_default();
                if u.desc.is_empty() {
                    let uber =
                        m.float("_Shininess").is_some() && m.float("_GlowStrengthNight").is_some();
                    let main = m.texture("_MainTex").is_some_and(|t| !t.texture.is_null());
                    let shader = match assets.resolve(&material.file, m.shader) {
                        Ok(Some(s)) => shader_names(&s),
                        _ => "(unresolved)".into(),
                    };
                    let color = m
                        .color("_Color")
                        .map(|c| c.map(|v| (v * 1000.0).round() / 1000.0));
                    let slots: Vec<&str> = m
                        .textures
                        .iter()
                        .filter(|t| !t.texture.is_null())
                        .map(|t| t.name.as_str())
                        .collect();
                    u.desc = format!(
                        "uber {uber}, _MainTex {main}, _Color {color:?}, queue {}, keywords [{}], textures [{}], shader {shader}",
                        m.custom_render_queue,
                        m.keywords,
                        slots.join(" ")
                    );
                }
                u.nodes += 1;
                u.placements += count;
                u.node_names.insert(node.name.clone());
                if let Some(mesh) = &node.mesh {
                    let name = mesh_names.entry(mesh.key()).or_insert_with(|| {
                        assets
                            .mesh(mesh)
                            .map_or("(unreadable)".into(), |(m, _)| m.name)
                    });
                    u.meshes.insert(name.clone());
                }
                u.prefabs.insert(key.to_string());
                u.layers.insert(node.layer);
                u.first = u.first.or(at);
            }
        }
    };
    let mut errors = 0;
    for (key, positions) in &placed {
        match assets.prefab(&catalog, key) {
            Ok(p) => add(&p, key, positions.len(), positions.first().copied()),
            Err(_) => errors += 1,
        }
        assets.trim_cache(512 << 20);
    }
    for name in ["aurora", "escapepod"] {
        let mut scene = assets.scene(name)?;
        scene.spawn_lightmapped_prefab();
        if name == "aurora" {
            scene.swap_aurora_models(&assets, false)?;
        }
        for root in &scene.roots {
            add(
                root,
                &format!("scene {name}"),
                1,
                Some(root.nodes[0].local.position),
            );
        }
    }
    println!("placed prefabs: {}; unreadable: {errors}", placed.len());
    println!("materials on drawn nodes: {}", uses.len());
    println!(
        "material\tdrawn nodes\tplacements\tdescription\tnode names\tmeshes\tprefabs\tlayers\tfirst placement"
    );
    for (name, u) in &uses {
        let few = |s: &BTreeSet<String>| {
            let v: Vec<_> = s.iter().take(3).cloned().collect();
            format!("{} ({})", v.join(", "), s.len())
        };
        println!(
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{:?}\t{:?}",
            u.nodes,
            u.placements,
            u.desc,
            few(&u.node_names),
            few(&u.meshes),
            few(&u.prefabs),
            u.layers,
            u.first.map(|p| p.map(f32::round))
        );
    }
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
