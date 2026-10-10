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

#[derive(Default)]
struct AnimTotals {
    clips: usize,
    controllers: usize,
    avatars: usize,
    animators: usize,
    humanoid: usize,
    keys: usize,
    /// Streamed segments whose end value differs from the next key's by
    /// more than 1e-3 (relative), not counting stepped keys.
    gaps: usize,
    stepped: usize,
    errors: Vec<String>,
}

/// Every animation object of the game parses to its last byte
/// (`docs/formats/animation.md`), with the checks the format allows.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn every_animation_object_parses() {
    use sn_unity::{
        ANIMATION_CLIP, ANIMATOR, ANIMATOR_CONTROLLER, AVATAR, AnimationClip, Animator,
        AnimatorController, Avatar,
    };
    let Some(files) = unity_files() else {
        eprintln!("SUBNAUTICA_DIR not set or not a Subnautica install; skipping");
        return;
    };
    let next = AtomicUsize::new(0);
    let totals = Mutex::new(AnimTotals::default());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut t = AnimTotals::default();
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let bytes = std::fs::read(path).unwrap();
                    let mut check = |data: &[u8], name: &str| {
                        let Ok(file) = SerializedFile::parse(data) else {
                            return;
                        };
                        for o in &file.objects {
                            let Some(obj) = file.object_data(data, o) else {
                                continue;
                            };
                            let at = format!("{name} object {}", o.path_id);
                            match o.class_id {
                                ANIMATION_CLIP => match AnimationClip::parse(obj, false) {
                                    Ok(c) => {
                                        t.clips += 1;
                                        clip_checks(&c, &at, &mut t);
                                    }
                                    Err(e) => t.errors.push(format!("{at}: clip: {e}")),
                                },
                                ANIMATOR_CONTROLLER => {
                                    match AnimatorController::parse(obj, false) {
                                        Ok(c) => {
                                            t.controllers += 1;
                                            controller_checks(&c, &at, &mut t.errors);
                                        }
                                        Err(e) => t.errors.push(format!("{at}: controller: {e}")),
                                    }
                                }
                                AVATAR => match Avatar::parse(obj, false) {
                                    Ok(a) => {
                                        t.avatars += 1;
                                        t.humanoid += usize::from(a.human_nodes > 0);
                                        if a.skeleton_parents.len() != a.skeleton_ids.len() {
                                            t.errors.push(format!("{at}: skeleton sizes"));
                                        }
                                    }
                                    Err(e) => t.errors.push(format!("{at}: avatar: {e}")),
                                },
                                ANIMATOR => match Animator::parse(obj, false) {
                                    Ok(_) => t.animators += 1,
                                    Err(e) => t.errors.push(format!("{at}: animator: {e}")),
                                },
                                _ => {}
                            }
                        }
                    };
                    let name = path.display().to_string();
                    if bytes.starts_with(b"UnityFS\0") {
                        if let Ok(bundle) = Bundle::parse(&bytes) {
                            for node in bundle.nodes.iter().filter(|n| n.is_serialized_file()) {
                                check(bundle.node_data(node), &format!("{name}/{}", node.path));
                            }
                        }
                    } else {
                        check(&bytes, &name);
                    }
                }
                let mut all = totals.lock().unwrap();
                all.clips += t.clips;
                all.controllers += t.controllers;
                all.avatars += t.avatars;
                all.animators += t.animators;
                all.humanoid += t.humanoid;
                all.keys += t.keys;
                all.gaps += t.gaps;
                all.stepped += t.stepped;
                all.errors.extend(t.errors);
            });
        }
    });
    let t = totals.into_inner().unwrap();
    eprintln!(
        "clips {} controllers {} avatars {} animators {}; streamed keys {}, stepped jumps {}, gaps {}",
        t.clips, t.controllers, t.avatars, t.animators, t.keys, t.stepped, t.gaps
    );
    assert!(
        t.errors.is_empty(),
        "{} errors, first: {:?}",
        t.errors.len(),
        &t.errors[..t.errors.len().min(10)]
    );
    assert_eq!(t.clips, 2294);
    assert_eq!(t.controllers, 289);
    assert_eq!(t.avatars, 334);
    assert_eq!(t.animators, 600);
    assert_eq!(t.humanoid, 0, "a humanoid avatar appeared");
    assert_eq!(t.gaps, 0, "streamed curves not continuous");
}

/// Bindings take every curve; streamed segments end where the next key
/// starts (stepped keys, a constant up to a jump, are counted apart).
fn clip_checks(c: &sn_unity::AnimationClip, at: &str, t: &mut AnimTotals) {
    if c.curve_count() != c.bound_curve_count() {
        t.errors.push(format!(
            "{at}: {} curves, bindings take {}",
            c.curve_count(),
            c.bound_curve_count()
        ));
    }
    for keys in &c.streamed {
        t.keys += keys.len();
        for w in keys.windows(2) {
            let (k, n) = (w[0], w[1]);
            if k.time < -1e30 || n.time > 1e30 {
                continue;
            }
            let dt = n.time - k.time;
            let [a, b, cc, d] = k.coeff;
            let v = ((a * dt + b) * dt + cc) * dt + d;
            let gap = (v - n.coeff[3]).abs() / n.coeff[3].abs().max(1.0);
            if gap > 1e-3 {
                if a == 0.0 && b == 0.0 && cc == 0.0 {
                    t.stepped += 1;
                } else {
                    t.gaps += 1;
                }
            }
        }
    }
}

/// Indices inside a controller point at things that exist.
fn controller_checks(c: &sn_unity::AnimatorController, at: &str, errors: &mut Vec<String>) {
    let params: std::collections::HashSet<u32> = c.params.iter().map(|p| p.id).collect();
    for l in &c.layers {
        if l.state_machine as usize >= c.state_machines.len() {
            errors.push(format!("{at}: layer state machine {}", l.state_machine));
        }
    }
    for sm in &c.state_machines {
        let dest_ok = |d: u32| {
            if d >= sn_unity::SELECTOR_BASE {
                ((d - sn_unity::SELECTOR_BASE) as usize) < sm.selectors.len()
            } else {
                (d as usize) < sm.states.len()
            }
        };
        let mut transitions: Vec<&sn_unity::Transition> = sm.any_state_transitions.iter().collect();
        for s in &sm.states {
            transitions.extend(&s.transitions);
            for tree in &s.blend_trees {
                for n in tree {
                    if n.children.iter().any(|&ch| ch as usize >= tree.len()) {
                        errors.push(format!("{at}: blend child out of range"));
                    }
                    if n.is_leaf() && n.clip != u32::MAX && n.clip as usize >= c.clips.len() {
                        errors.push(format!("{at}: blend leaf clip {}", n.clip));
                    }
                    if !n.is_leaf() && !params.contains(&n.param) {
                        errors.push(format!("{at}: blend parameter {} unknown", n.param));
                    }
                }
            }
            if s.blend_tree_index
                .iter()
                .any(|&i| i >= 0 && i as usize >= s.blend_trees.len())
            {
                errors.push(format!("{at}: blend tree index out of range"));
            }
        }
        for tr in transitions {
            if !dest_ok(tr.destination) {
                errors.push(format!("{at}: transition to {}", tr.destination));
            }
            for cond in &tr.conditions {
                if !params.contains(&cond.param) {
                    errors.push(format!(
                        "{at}: condition on unknown parameter {}",
                        cond.param
                    ));
                }
            }
        }
        for sel in &sm.selectors {
            // `u32::MAX`: no destination (2 exit selectors in the game).
            for tr in sel.transitions.iter().filter(|t| t.destination != u32::MAX) {
                if !dest_ok(tr.destination) {
                    errors.push(format!("{at}: selector to {}", tr.destination));
                }
            }
        }
        if sm.default_state as usize >= sm.states.len().max(1) {
            errors.push(format!("{at}: default state {}", sm.default_state));
        }
    }
}
