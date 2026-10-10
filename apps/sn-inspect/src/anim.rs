//! `anim` command (M7f4): the animators of a prefab or scene, their
//! controllers (layers, states, parameters by name) and clips (length,
//! loop, curves by kind, bindings found in the hierarchy).

use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::sync::Arc;

use sn_anim::{Animator, Program, SlotKind};
use sn_assets::{AnimationSet, Assets, Prefab};
use sn_install::GameData;
use sn_unity::{
    ATTR_EULER, ATTR_POSITION, ATTR_ROTATION, ATTR_SCALE, AnimationClip, AnimatorController,
    BIND_TRANSFORM, BlendType, ParamKind, SELECTOR_BASE,
};

use crate::Result;

/// The hierarchies named by `target`: `scene:<name>` (each top-level
/// object of a scene) or a prefab key.
fn load(assets: &Assets, target: &str) -> Result<Vec<Prefab>> {
    if let Some(name) = target.strip_prefix("scene:") {
        Ok(assets.scene(name)?.roots)
    } else {
        let catalog = assets.catalog()?;
        Ok(vec![assets.prefab(&catalog, target)?])
    }
}

fn node_path(prefab: &Prefab, node: usize) -> String {
    let mut parts = Vec::new();
    let mut at = Some(node);
    while let Some(i) = at {
        parts.push(prefab.nodes[i].name.as_str());
        at = prefab.nodes[i].parent;
    }
    parts.reverse();
    parts.join("/")
}

/// What a binding animates, for counting.
pub fn binding_kind(type_id: i32, attribute: u32) -> &'static str {
    match (type_id, attribute) {
        (BIND_TRANSFORM, ATTR_POSITION) => "position",
        (BIND_TRANSFORM, ATTR_ROTATION) => "rotation",
        (BIND_TRANSFORM, ATTR_SCALE) => "scale",
        (BIND_TRANSFORM, ATTR_EULER) => "euler",
        (1, _) => "active",
        (23, _) => "material",
        (95, _) => "animator parameter",
        (108, _) => "light",
        (114, _) => "script",
        (137, _) => "blend shape",
        (224, _) => "rect transform",
        _ => "other",
    }
}

fn name(c: &AnimatorController, hash: u32) -> String {
    c.name_of(hash)
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{hash}"))
}

fn clip_name(set: &AnimationSet, index: u32) -> String {
    match set.clips.get(index as usize) {
        Some(Some(c)) => c.name.clone(),
        Some(None) => format!("clip {index} (missing)"),
        None if index == u32::MAX => "(none)".into(),
        None => format!("clip {index} (out of range)"),
    }
}

fn print_controller(set: &AnimationSet, states: bool) {
    let c = &set.controller;
    println!(
        "  controller {:?}: {} layers, {} parameters, {} clips{}",
        c.name,
        c.layers.len(),
        c.params.len(),
        c.clips.len(),
        if c.state_machine_behaviours.is_empty() {
            String::new()
        } else {
            format!(
                ", {} state machine behaviours",
                c.state_machine_behaviours.len()
            )
        }
    );
    let defaults = &c.defaults;
    let params: Vec<String> = c
        .params
        .iter()
        .map(|p| {
            let i = p.index as usize;
            let value = match p.kind {
                ParamKind::Float => format!("{}", defaults.floats.get(i).copied().unwrap_or(0.0)),
                ParamKind::Int => format!("{}", defaults.ints.get(i).copied().unwrap_or(0)),
                ParamKind::Bool | ParamKind::Trigger => {
                    format!("{}", defaults.bools.get(i).copied().unwrap_or(false))
                }
                ParamKind::Other(k) => format!("kind {k}"),
            };
            format!("{} ({:?} {value})", name(c, p.id), p.kind)
        })
        .collect();
    println!("    parameters: {}", params.join(", "));
    for (li, layer) in c.layers.iter().enumerate() {
        let Some(sm) = c.state_machines.get(layer.state_machine as usize) else {
            continue;
        };
        println!(
            "    layer {li} {:?}: {:?}, weight {}, mask {} paths, {} states, {} any-state \
             transitions, default {:?}",
            name(c, layer.binding),
            layer.blending,
            layer.default_weight,
            layer.skeleton_mask.len(),
            sm.states.len(),
            sm.any_state_transitions.len(),
            sm.states
                .get(sm.default_state as usize)
                .map(|s| name(c, s.name_id))
                .unwrap_or_default()
        );
        if !states {
            continue;
        }
        for s in &sm.states {
            let motion = s
                .blend_trees
                .first()
                .map(|tree| match tree.first() {
                    Some(n) if n.is_leaf() => clip_name(set, n.clip),
                    Some(n) => {
                        let kind = match n.kind {
                            BlendType::Simple1D => "1D",
                            BlendType::SimpleDirectional2D => "2D simple directional",
                            BlendType::FreeformDirectional2D => "2D freeform directional",
                            BlendType::FreeformCartesian2D => "2D freeform cartesian",
                            BlendType::Direct => "direct",
                            BlendType::Other(_) => "?",
                        };
                        let leaves: Vec<String> = tree
                            .iter()
                            .filter(|n| n.is_leaf())
                            .map(|n| clip_name(set, n.clip))
                            .collect();
                        format!(
                            "blend {kind} on {} of [{}]",
                            name(c, n.param),
                            leaves.join(", ")
                        )
                    }
                    None => "(empty)".into(),
                })
                .unwrap_or_else(|| "(no motion)".into());
            let transitions: Vec<String> = s
                .transitions
                .iter()
                .map(|t| {
                    let to = if t.destination >= SELECTOR_BASE {
                        format!("selector {}", t.destination - SELECTOR_BASE)
                    } else {
                        sm.states
                            .get(t.destination as usize)
                            .map(|d| name(c, d.name_id))
                            .unwrap_or_else(|| "?".into())
                    };
                    let conds: Vec<String> = t
                        .conditions
                        .iter()
                        .map(|k| format!("{:?} {} {}", k.mode, name(c, k.param), k.threshold))
                        .collect();
                    let exit = if t.has_exit_time {
                        format!(" exit {}", t.exit_time)
                    } else {
                        String::new()
                    };
                    format!("→ {to} [{}]{exit} {}s", conds.join(" & "), t.duration)
                })
                .collect();
            println!(
                "      {:?} speed {}{}: {motion}; {}",
                name(c, s.name_id),
                s.speed,
                if s.write_default_values {
                    ""
                } else {
                    " (no write defaults)"
                },
                transitions.join("; ")
            );
        }
        // Entry and exit nodes: their transitions, in order.
        for (i, sel) in sm.selectors.iter().enumerate() {
            let list: Vec<String> = sel
                .transitions
                .iter()
                .map(|t| {
                    let to = if t.destination == u32::MAX {
                        "nowhere".to_string()
                    } else if t.destination >= SELECTOR_BASE {
                        format!("selector {}", t.destination - SELECTOR_BASE)
                    } else {
                        sm.states
                            .get(t.destination as usize)
                            .map(|d| name(c, d.name_id))
                            .unwrap_or_else(|| "?".into())
                    };
                    let conds: Vec<String> = t
                        .conditions
                        .iter()
                        .map(|k| format!("{:?} {} {}", k.mode, name(c, k.param), k.threshold))
                        .collect();
                    format!("→ {to} [{}]", conds.join(" & "))
                })
                .collect();
            println!(
                "      selector {i} ({}, {}): {}",
                if sel.is_entry { "entry" } else { "exit" },
                name(c, sel.full_path_id),
                list.join("; ")
            );
        }
    }
}

fn clip_summary(clip: &AnimationClip) -> String {
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for b in &clip.bindings {
        *kinds
            .entry(binding_kind(b.type_id, b.attribute))
            .or_default() += 1;
    }
    let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
    format!(
        "{:?}: {:.3} s{}, curves {} streamed / {} dense / {} constant; {}",
        clip.name,
        clip.stop_time - clip.start_time,
        if clip.loop_time { " loop" } else { "" },
        clip.streamed.len(),
        clip.dense.curve_count,
        clip.constant.len(),
        kinds.join(", ")
    )
}

/// Runs an animator from its defaults for `seconds` at 60 updates per
/// second: transitions taken, NaNs, quaternion lengths, how far each
/// Transform moved from its stored place, time per update.
fn play(prefab: &Prefab, node: usize, set: &AnimationSet, seconds: f32) {
    let c = Arc::new(set.controller.clone());
    let program = Arc::new(Program::new(c.clone(), &set.clips));
    let binding = prefab.bind_animator(node, &program, &|_| Vec::new());
    let mut animator = Animator::new(program.clone(), binding.defaults.clone());
    let unread = program
        .slots
        .iter()
        .filter(|s| matches!(s.kind, SlotKind::Float { type_id, .. } if type_id != 137))
        .count();
    println!(
        "  play {seconds} s: {} slots ({} values), {} not in the hierarchy, {} non-Transform properties without a read default (0 used)",
        program.slots.len(),
        program.width,
        binding.missing,
        unread
    );
    let layer_name = |l: usize| {
        c.layers
            .get(l)
            .map(|l| name(&c, l.binding))
            .unwrap_or_default()
    };
    let dt = 1.0 / 60.0;
    let steps = (seconds / dt).round() as usize;
    let mut worst_q = 0.0f32;
    let mut nans = 0usize;
    let mut moved = vec![0.0f32; program.slots.len()];
    let start = std::time::Instant::now();
    for step in 0..steps {
        animator.update(dt);
        for &(layer, from, to) in &animator.started {
            println!(
                "    {:7.3} s layer {:?}: {} → {}",
                (step + 1) as f32 * dt,
                layer_name(layer),
                name(&c, from),
                name(&c, to)
            );
        }
        let pose = animator.pose();
        nans += pose.iter().filter(|v| !v.is_finite()).count();
        for (i, slot) in program.slots.iter().enumerate() {
            let at = slot.offset;
            match slot.kind {
                SlotKind::Rotation => {
                    let len = pose[at..at + 4].iter().map(|v| v * v).sum::<f32>().sqrt();
                    worst_q = worst_q.max((len - 1.0).abs());
                    let d: f32 = (0..4)
                        .map(|k| pose[at + k] * binding.defaults[at + k])
                        .sum::<f32>()
                        .abs()
                        .min(1.0);
                    moved[i] = moved[i].max(2.0 * d.acos().to_degrees());
                }
                SlotKind::Position | SlotKind::Scale => {
                    let d = (0..3)
                        .map(|k| (pose[at + k] - binding.defaults[at + k]).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    moved[i] = moved[i].max(d);
                }
                SlotKind::Float { .. } => {
                    moved[i] = moved[i].max((pose[at] - binding.defaults[at]).abs());
                }
            }
        }
    }
    let per = start.elapsed().as_secs_f64() * 1e6 / steps.max(1) as f64;
    for l in 0..animator.layer_count() {
        if let Some(info) = animator.layer_info(l) {
            println!(
                "    end: layer {:?} in {} at {:.3}{}",
                layer_name(l),
                info.state
                    .map(|s| name(&c, s))
                    .unwrap_or_else(|| "(blend)".into()),
                info.normalized_time,
                info.next
                    .map(|(n, f)| format!(", blending to {} ({:.0} %)", name(&c, n), f * 100.0))
                    .unwrap_or_default()
            );
        }
    }
    let max_of = |k: fn(&SlotKind) -> bool| {
        program
            .slots
            .iter()
            .zip(&moved)
            .filter(|(s, _)| k(&s.kind))
            .map(|(_, m)| *m)
            .fold(0.0f32, f32::max)
    };
    let moving = moved.iter().filter(|m| **m > 1e-4).count();
    println!(
        "    {moving} slots moved from their stored values; largest: position {:.3} m, rotation {:.1}°, scale {:.3}; NaNs {nans}; worst |q| − 1 {worst_q:.2e}; {per:.1} µs per update",
        max_of(|k| *k == SlotKind::Position),
        max_of(|k| *k == SlotKind::Rotation),
        max_of(|k| *k == SlotKind::Scale),
    );
    let mut order: Vec<usize> = (0..moved.len()).filter(|&i| moved[i] > 1e-4).collect();
    order.sort_by(|&a, &b| moved[b].total_cmp(&moved[a]));
    for &i in order.iter().take(8) {
        let slot = program.slots[i];
        let at = binding.nodes[i]
            .map(|n| node_path(prefab, n))
            .unwrap_or_else(|| format!("#{}", slot.path));
        println!("      {:?} {:?}: {:.3}", at, slot.kind, moved[i]);
    }
}

pub fn run(game: &GameData, target: &str, states: bool, play_for: Option<f32>) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let prefabs = load(&assets, target)?;
    let mut cache = HashMap::new();
    let mut animators = 0;
    let (mut found, mut missing) = (0usize, 0usize);
    for prefab in &prefabs {
        for (i, node) in prefab.nodes.iter().enumerate() {
            let Some(a) = &node.animator else {
                continue;
            };
            animators += 1;
            println!(
                "animator on {:?} (enabled {}, culling {}, active {}){}",
                node_path(prefab, i),
                a.component.enabled,
                a.component.culling_mode,
                node.active,
                if a.component.apply_root_motion {
                    ", root motion"
                } else {
                    ""
                }
            );
            if let Some(avatar) = &a.avatar {
                match assets.avatar(avatar) {
                    Ok(av) => println!(
                        "  avatar {:?}: {} skeleton nodes, {} human",
                        av.name,
                        av.skeleton_ids.len(),
                        av.human_nodes
                    ),
                    Err(e) => println!("  avatar: {e}"),
                }
            }
            let Some(controller) = &a.controller else {
                println!("  no controller");
                continue;
            };
            let set = match assets.animation_set(controller, &mut cache) {
                Ok(s) => s,
                Err(e) => {
                    println!("  controller: {e}");
                    continue;
                }
            };
            for e in &set.errors {
                println!("  {e}");
            }
            print_controller(&set, states);
            if let Some(seconds) = play_for {
                play(prefab, i, &set, seconds);
            }
            // Bindings against the hierarchy below the animator.
            let paths = prefab.binding_paths(i);
            let mut unique: Vec<&Arc<AnimationClip>> = Vec::new();
            for c in set.clips.iter().flatten() {
                if !unique.iter().any(|u| Arc::ptr_eq(u, c)) {
                    unique.push(c);
                }
            }
            let (mut here_found, mut here_missing) = (0, 0);
            let mut missing_paths: BTreeMap<u32, usize> = BTreeMap::new();
            for c in &unique {
                for b in &c.bindings {
                    if paths.contains_key(&b.path) {
                        here_found += 1;
                    } else {
                        here_missing += 1;
                        *missing_paths.entry(b.path).or_default() += 1;
                    }
                }
            }
            found += here_found;
            missing += here_missing;
            println!(
                "  {} clips; bindings found in the hierarchy {here_found}, missing {here_missing} \
                 ({} paths)",
                unique.len(),
                missing_paths.len()
            );
            if !missing_paths.is_empty() {
                println!("    missing path hashes (CRC-32): {missing_paths:?}");
            }
            for c in &unique {
                println!("    {}", clip_summary(c));
            }
        }
    }
    println!("{animators} animators; bindings found {found}, missing {missing}");
    Ok(ExitCode::SUCCESS)
}

/// Prefabs, placements, µs per update, culling modes, slots by kind.
type ControllerRow = (usize, usize, f64, BTreeMap<i32, usize>, String);

/// `anim --placed`: the animators of every prefab placed in the world
/// (creatures left out, as the client does): how many placements animate,
/// by controller, their culling modes, what they animate, and the cost of
/// one update of each.
pub fn placed(game: &GameData) -> Result<ExitCode> {
    let placed = crate::prefab::placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut cache = HashMap::new();
    // controller → (prefabs, placements, µs per update, culling modes, slots by kind)
    let mut by_controller: BTreeMap<String, ControllerRow> = BTreeMap::new();
    let (mut prefabs_with, mut placements_with, mut errors) = (0usize, 0usize, 0usize);
    let mut skinned_under = 0usize;
    let mut rigid_under = 0usize;
    for (key, positions) in &placed {
        if key.starts_with("WorldEntities/Creatures/") {
            continue;
        }
        let Ok(prefab) = assets.prefab(&catalog, key) else {
            errors += 1;
            continue;
        };
        let mut any = false;
        for (i, node) in prefab.nodes.iter().enumerate() {
            let Some(a) = &node.animator else { continue };
            if !node.active || !a.component.enabled {
                continue;
            }
            let Some(controller) = &a.controller else {
                continue;
            };
            let Ok(set) = assets.animation_set(controller, &mut cache) else {
                errors += 1;
                continue;
            };
            any = true;
            let program = Arc::new(Program::new(Arc::new(set.controller.clone()), &set.clips));
            let binding = prefab.bind_animator(i, &program, &|_| Vec::new());
            let mut animator = Animator::new(program.clone(), binding.defaults);
            let start = std::time::Instant::now();
            for _ in 0..60 {
                animator.update(1.0 / 60.0);
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / 60.0;
            let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
            for s in &program.slots {
                let k = match s.kind {
                    SlotKind::Position => "position",
                    SlotKind::Rotation => "rotation",
                    SlotKind::Scale => "scale",
                    SlotKind::Float {
                        type_id, attribute, ..
                    } => binding_kind(type_id, attribute),
                };
                *kinds.entry(k).or_default() += 1;
            }
            let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
            // Drawn parts below this animator.
            let below = |mut n: usize| loop {
                if n == i {
                    break true;
                }
                match prefab.nodes[n].parent {
                    Some(p) => n = p,
                    None => break false,
                }
            };
            for (j, v) in prefab.visible() {
                if below(j) {
                    if v.skinned {
                        skinned_under += positions.len();
                    } else {
                        rigid_under += positions.len();
                    }
                }
            }
            let e = by_controller.entry(set.controller.name.clone()).or_insert((
                0,
                0,
                0.0,
                BTreeMap::new(),
                kinds.join(", "),
            ));
            e.0 += 1;
            e.1 += positions.len();
            e.2 = e.2.max(us);
            *e.3.entry(a.component.culling_mode).or_default() += positions.len();
        }
        if any {
            prefabs_with += 1;
            placements_with += positions.len();
        }
    }
    println!(
        "placed prefabs with an active, enabled animator: {prefabs_with} ({placements_with} placements); drawn parts below them in the world: {rigid_under} rigid, {skinned_under} skinned; {errors} errors"
    );
    let mut rows: Vec<_> = by_controller.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.1));
    let mut total_us = 0.0;
    for (name, (prefabs, placements, us, culling, kinds)) in &rows {
        total_us += us * *placements as f64;
        println!(
            "  {name:?}: {prefabs} prefabs, {placements} placements, culling {culling:?}, {us:.1} µs per update; {kinds}"
        );
    }
    println!(
        "all placements updated at once: {:.1} ms per frame (one thread)",
        total_us / 1000.0
    );
    Ok(ExitCode::SUCCESS)
}
