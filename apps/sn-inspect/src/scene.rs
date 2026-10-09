//! `scene` command: a scene bundle's objects (`aurora`, `escapepod`, …):
//! class counts, top-level objects, what is drawable, script census.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::Assets;
use sn_install::GameData;
use sn_unity::class_name;

use crate::Result;

const MONO_BEHAVIOUR: i32 = 114;

pub fn run(
    game: &GameData,
    name: &str,
    tree_depth: Option<usize>,
    dump_script: Option<&str>,
) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let scene = assets.scene(name)?;
    println!(
        "scene {} ({}): {} top-level objects (loaded in {:.2} s)",
        scene.name,
        scene.file.bundle.path.display(),
        scene.roots.len(),
        start.elapsed().as_secs_f64()
    );
    println!("objects in the scene file by class:");
    for (class, n) in &scene.class_counts {
        println!("  {class:>4} {:<24} {n}", class_name(*class).unwrap_or("?"));
    }

    let (mut nodes, mut active, mut meshes, mut drawn, mut in_lod, mut skinned) =
        (0, 0, 0, 0, 0, 0);
    let (mut lights, mut lights_on) = (0, 0);
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for root in &scene.roots {
        nodes += root.nodes.len();
        active += root.nodes.iter().filter(|n| n.active).count();
        meshes += root.nodes.iter().filter(|n| n.mesh.is_some()).count();
        in_lod += root.nodes.iter().filter(|n| n.lod.is_some()).count();
        skinned += root.nodes.iter().filter(|n| n.active && n.skinned).count();
        for n in &root.nodes {
            lights += n.lights.len();
            if n.active {
                lights_on += n.lights.iter().filter(|l| l.enabled).count();
            }
        }
        let base = root.nodes[0].local;
        for n in root.visible_nodes() {
            drawn += 1;
            let p = base.then(&n.in_prefab).position;
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    println!(
        "nodes {nodes} ({active} active), with a mesh {meshes}, drawn at full detail {drawn}, \
         in LOD groups {in_lod}, active skinned {skinned}, lights {lights} ({lights_on} on)"
    );
    if drawn > 0 {
        println!(
            "drawn node pivots between {:?} and {:?}",
            lo.map(|v| v.round()),
            hi.map(|v| v.round())
        );
    }

    println!("top-level objects:");
    for root in &scene.roots {
        let r = &root.nodes[0];
        println!(
            "  {:?}{} at {:?}: {} nodes, {} drawn",
            r.name,
            if r.active { "" } else { " [inactive]" },
            r.local.position.map(|v| (v * 100.0).round() / 100.0),
            root.nodes.len(),
            root.visible_nodes().count()
        );
        if let Some(max) = tree_depth {
            for n in root.nodes.iter().skip(1) {
                let depth = std::iter::successors(n.parent, |&p| root.nodes[p].parent).count();
                if depth <= max {
                    println!(
                        "  {}{}{}{}{}",
                        "  ".repeat(depth + 1),
                        n.name,
                        if n.active { "" } else { " [inactive]" },
                        if n.mesh.is_some() { " [mesh]" } else { "" },
                        if n.skinned { " [skinned]" } else { "" },
                    );
                }
            }
        }
    }

    let mut scripts: BTreeMap<String, usize> = BTreeMap::new();
    for info in scene.file.objects() {
        if info.class_id == MONO_BEHAVIOUR {
            let object = sn_assets::ObjectRef {
                file: scene.file.clone(),
                path_id: info.path_id,
            };
            let class = assets
                .script_class(&object)
                .unwrap_or_else(|| "(unknown)".into());
            if dump_script == Some(class.as_str()) {
                dump(&object)?;
            }
            *scripts.entry(class).or_default() += 1;
        }
    }
    let mut scripts: Vec<_> = scripts.into_iter().collect();
    scripts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("MonoBehaviours by script ({} scripts):", scripts.len());
    for (class, n) in &scripts {
        println!("  {n:>5} {class}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Hex and text of a MonoBehaviour's bytes, for reading its fields by hand.
fn dump(object: &sn_assets::ObjectRef) -> Result<()> {
    let (_, data) = object.data()?;
    println!("MonoBehaviour {} ({} bytes):", object.path_id, data.len());
    for (i, row) in data.chunks(16).enumerate().take(64) {
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        let text: String = row
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  {:05x}  {:<48} {text}", i * 16, hex.join(" "));
    }
    Ok(())
}

/// The scenes the game loads at start, and what each spawned one draws in
/// a new game (the Aurora intact).
pub fn startup(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let (additional, autoload) = assets.startup_scenes()?;
    println!("main scene loads additively: {additional:?}");
    for a in &autoload {
        println!(
            "lightmapped prefab scene {:?}: {}",
            a.scene_name,
            if a.spawn_on_start {
                "spawned at start"
            } else {
                "kept as a template"
            }
        );
    }
    for a in autoload.iter().filter(|a| a.spawn_on_start) {
        let mut scene = assets.scene(&a.scene_name)?;
        let found = scene.spawn_lightmapped_prefab();
        let mut line = format!(
            "{}: {} top-level objects, prefab object {}",
            scene.name,
            scene.roots.len(),
            if found { "found" } else { "MISSING" }
        );
        if !scene.behaviours(&assets, "CrashedShipExploder").is_empty() {
            for exploded in [true, false] {
                let (off, on) = scene.swap_aurora_models(&assets, exploded)?;
                let drawn: usize = scene.roots.iter().map(|r| r.visible_nodes().count()).sum();
                line += &format!(
                    "; {}: {off} objects off, {on} on, {drawn} nodes drawn",
                    if exploded { "exploded" } else { "intact" }
                );
            }
        }
        let drawn: usize = scene.roots.iter().map(|r| r.visible_nodes().count()).sum();
        let skinned: usize = scene
            .roots
            .iter()
            .flat_map(|r| &r.nodes)
            .filter(|n| n.active && n.skinned)
            .count();
        println!("{line}; drawn {drawn}, active skinned {skinned}");
    }
    println!("({:.2} s)", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
