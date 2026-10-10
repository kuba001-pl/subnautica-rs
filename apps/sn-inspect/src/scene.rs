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

    for info in scene.file.objects().iter().filter(|i| i.class_id == 20) {
        let Some((_, data)) = scene.file.object(info.path_id) else {
            continue;
        };
        let camera = match sn_unity::Camera::parse(data, scene.file.file().big_endian) {
            Ok(c) => c,
            Err(e) => {
                println!("camera {}: {e}", info.path_id);
                continue;
            }
        };
        let name = scene
            .roots
            .iter()
            .flat_map(|r| &r.nodes)
            .find(|n| n.object.2 == camera.game_object.path_id)
            .map_or("?", |n| n.name.as_str());
        println!(
            "camera {name:?}: enabled {}, near {}, far {}, field of view {}, culling mask {:#010x}",
            camera.enabled, camera.near, camera.far, camera.field_of_view, camera.culling_mask
        );
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
    // The bundle's other files (`.sharedAssets`: prefabs the scene's
    // scripts reference) for the dump.
    if let Some(class) = dump_script {
        let names: Vec<String> = scene.file.bundle.file_names().map(String::from).collect();
        for name in names.iter().filter(|n| **n != scene.file.name) {
            let file = assets.file(&scene.file.bundle.path, name)?;
            for info in file
                .objects()
                .iter()
                .filter(|o| o.class_id == MONO_BEHAVIOUR)
            {
                let object = sn_assets::ObjectRef {
                    file: file.clone(),
                    path_id: info.path_id,
                };
                if assets.script_class(&object).as_deref() == Some(class) {
                    println!("in {name}:");
                    dump(&object)?;
                }
            }
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

/// Lifepod 5 in a new game with world seed `seed`: the start map, the
/// start point, the player spawn and what the pod's spawners put in it.
pub fn lifepod(game: &GameData, seed: u64) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let map = assets.start_map()?;
    println!(
        "start map: {:.2} % of the pixels are valid starts",
        map.valid_share() * 100.0
    );
    let (point, tries) = map.random_start(seed);
    println!(
        "seed {seed}: start point {:?} after {tries} draws",
        point.map(|v| (v * 100.0).round() / 100.0)
    );
    let mut scene = assets.scene("escapepod")?;
    scene.spawn_lightmapped_prefab();
    let spawn = scene.place_escape_pod(&assets, point)?;
    println!(
        "player spawn at {:?}",
        spawn.position.map(|v| (v * 100.0).round() / 100.0)
    );
    let following = scene.follow_targets(&assets)?;
    println!("objects following a target: {following}");
    let spawns = scene.spawns(&assets)?;
    println!("{} spawners fire in a new game:", spawns.len());
    for s in &spawns {
        let what = match (&s.spawner.prefab, &s.object) {
            (sn_unity::SpawnPrefab::Address(guid), _) => catalog
                .locate(guid)
                .into_iter()
                .find(|l| l.resource_type == "UnityEngine.GameObject")
                .map_or(format!("{guid} (not in the catalog)"), |l| l.internal_id),
            (_, Some(o)) => format!("object {} in {}", o.path_id, o.file.name),
            _ => "(nothing)".into(),
        };
        println!(
            "  {:?} (type {}): {what}; parent at {:?}",
            s.name,
            s.spawner.spawn_type,
            s.parent.position.map(|v| (v * 100.0).round() / 100.0)
        );
    }
    println!("({:.2} s)", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}

/// `scene NAME --cinematics` (M9g5): every `PlayerCinematicController`
/// of the scene with its fields and the nodes they point at, and the
/// clips with animation events of the scene's animators (and the
/// player's, from the main scene) with each event's time and function.
pub fn cinematics(game: &GameData, name: &str) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let mut scene = assets.scene(name)?;
    scene.spawn_lightmapped_prefab();
    let path = |at: Option<(usize, usize)>| -> String {
        let Some((r, n)) = at else {
            return "(none)".into();
        };
        let nodes = &scene.roots[r].nodes;
        let mut parts = Vec::new();
        let mut i = Some(n);
        while let Some(k) = i {
            parts.push(nodes[k].name.as_str());
            i = nodes[k].parent;
        }
        parts.reverse();
        parts.join("/")
    };
    let controllers = scene.behaviours(&assets, "PlayerCinematicController");
    println!("{} PlayerCinematicControllers:", controllers.len());
    for b in &controllers {
        let big_endian = b.file.file().big_endian;
        let (_, data) = b.data()?;
        let c = sn_unity::PlayerCinematicController::parse(data, big_endian)
            .map_err(|e| format!("PlayerCinematicController: {e}"))?;
        let on = scene.behaviour_node(&assets, b)?;
        let animator_node = match assets.resolve(&b.file, c.animator)? {
            Some(a) => {
                let (_, d) = a.data()?;
                let animator = sn_unity::Animator::parse(d, a.file.file().big_endian)
                    .map_err(|e| format!("Animator: {e}"))?;
                assets
                    .resolve(&a.file, animator.game_object)?
                    .and_then(|go| scene.locate(&go))
            }
            None => None,
        };
        let inform = assets
            .resolve(&b.file, c.inform_game_object)?
            .and_then(|go| scene.locate(&go));
        println!("- on {}", path(on));
        println!(
            "    animatedTransform {}; animator {}; endTransform {} (only in VR {}); inform {}",
            path(scene.locate_transform(&assets, c.animated_transform)?),
            path(animator_node),
            path(scene.locate_transform(&assets, c.end_transform)?),
            c.only_use_end_transform_in_vr,
            path(inform)
        );
        println!(
            "    animParam {:?}, interpolateAnimParam {:?}, playerViewAnimationName {:?}, playerViewInterpolateAnimParam {:?}, receiversAnimParam {:?} ({} receivers)",
            c.anim_param,
            c.interpolate_anim_param,
            c.player_view_animation_name,
            c.player_view_interpolate_anim_param,
            c.receivers_anim_param,
            c.anim_param_receivers.len()
        );
        println!(
            "    interpolation {} s in, {} s out; interpolateDuringAnimation {}, enforceCinematicModeEnd {}, playInVr {}, interruptAutoMove {}",
            c.interpolation_time,
            c.interpolation_time_out,
            c.interpolate_during_animation,
            c.enforce_cinematic_mode_end,
            c.play_in_vr,
            c.interrupt_auto_move
        );
    }
    // The triggers: their calls at the start and the end, and which
    // forwarder passes their end event on.
    let forwards = scene.cinematic_forwards(&assets)?;
    for f in &forwards {
        println!(
            "OnPlayerCinematicModeEndForward on {}: {} controllers",
            path(Some(f.node)),
            f.forward.len()
        );
    }
    for t in scene.cinematic_triggers(&assets)? {
        let calls = |list: &[sn_unity::PersistentCall]| -> Vec<String> {
            list.iter().map(|c| c.method_name.clone()).collect()
        };
        let forwarded = forwards
            .iter()
            .filter(|f| f.forward.contains(&t.cinematic_key))
            .count();
        println!(
            "trigger {:?} (hand {}): start {:?}, end {:?}; animated node {}, animator {}; forwarded by {forwarded}",
            t.name,
            t.hand,
            calls(&t.trigger.on_cinematic_start),
            calls(&t.trigger.on_cinematic_end),
            path(t.animated_node),
            path(t.animator_node)
        );
    }
    // Clips with events, per controller (each controller once).
    let mut seen = std::collections::BTreeSet::new();
    let mut cache = std::collections::HashMap::new();
    let main = assets.player_body()?;
    let mut sets = Vec::new();
    for root in &scene.roots {
        for (i, node) in root.nodes.iter().enumerate() {
            if let Some(controller) = node.animator.as_ref().and_then(|a| a.controller.clone()) {
                sets.push((format!("{} ({})", node.name, i), controller));
            }
        }
    }
    sets.push(("the player (main scene)".into(), main.controller.clone()));
    for (owner, controller) in sets {
        if !seen.insert(controller.key()) {
            continue;
        }
        let set = assets.animation_set(&controller, &mut cache)?;
        println!(
            "controller {:?} on {owner}: clips with events:",
            set.controller.name
        );
        for clip in set.clips.iter().flatten() {
            if clip.events.is_empty() {
                continue;
            }
            let events: Vec<String> = clip
                .events
                .iter()
                .map(|e| {
                    let arg = if e.data.is_empty() {
                        String::new()
                    } else {
                        format!("({:?})", e.data)
                    };
                    format!("{:.3} s {}{arg}", e.time, e.function)
                })
                .collect();
            println!(
                "  {:?} {:.3} s: {}",
                clip.name,
                clip.stop_time - clip.start_time,
                events.join(", ")
            );
        }
    }
    println!("({:.2} s)", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
