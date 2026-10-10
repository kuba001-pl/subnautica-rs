//! `prefab --skinned`: every skinned mesh of the placed prefabs and of the
//! escape pod scene: skin data checks, and the skinned LOD 0's bounds next
//! to the static LOD 1's where a prefab has both.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use sn_anim::{Animator, Program, SlotKind};
use sn_assets::{Assets, Prefab};
use sn_install::GameData;
use sn_world::Transform;

use crate::Result;

type Bounds = ([f32; 3], [f32; 3]);

fn grow(b: &mut Option<Bounds>, p: [f32; 3]) {
    let (lo, hi) = b.get_or_insert((p, p));
    for a in 0..3 {
        lo[a] = lo[a].min(p[a]);
        hi[a] = hi[a].max(p[a]);
    }
}

fn place(t: &Transform, p: [f32; 3]) -> [f32; 3] {
    t.then(&Transform {
        position: p,
        ..Default::default()
    })
    .position
}

#[derive(Default)]
struct Totals {
    renderers: usize,
    skinned: usize,
    no_skin: usize,
    why: std::collections::BTreeMap<&'static str, usize>,
    bone_mismatch: usize,
    missing_bones: usize,
    bad_layout: usize,
    errors: Vec<String>,
    /// (prefab, LOD 0 size, LOD 1 size, distance of the centres)
    compared: Vec<(String, [f32; 3], [f32; 3], f32)>,
}

fn check(assets: &Assets, name: &str, prefab: &Prefab, t: &mut Totals) {
    let mut lod0: Option<Bounds> = None;
    let mut lod1: Option<Bounds> = None;
    let mut any_skinned_lod0 = false;
    for (i, node) in prefab.nodes.iter().enumerate() {
        let Some(mesh) = &node.mesh else { continue };
        if !node.active || !node.renderer_enabled {
            continue;
        }
        let (m, g) = match assets.mesh(mesh) {
            Ok(x) => x,
            Err(e) => {
                t.errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if node.skinned && name == "escapepod scene" {
            println!(
                "  escape pod: {:?} LOD {:?}, {} bones, {} bind poses, {} vertices",
                node.name,
                node.lod,
                node.bones.len(),
                m.bind_poses.len(),
                g.positions.len()
            );
        }
        let points: Vec<[f32; 3]> = if node.skinned {
            t.renderers += 1;
            if !node.bones.is_empty() && node.bones.len() != m.bind_poses.len() {
                t.bone_mismatch += 1;
            }
            t.missing_bones += node.bones.iter().filter(|b| b.is_none()).count();
            // Column by column: the bottom row is 0 0 0 1.
            if m.bind_poses.iter().any(|b| {
                b[3].abs() > 1e-4
                    || b[7].abs() > 1e-4
                    || b[11].abs() > 1e-4
                    || (b[15] - 1.0).abs() > 1e-4
            }) {
                t.bad_layout += 1;
            }
            match prefab.skinned_geometry(i, &m, &g) {
                Some(s) => {
                    t.skinned += 1;
                    if node.lod == Some(0) {
                        any_skinned_lod0 = true;
                    }
                    s.positions
                }
                None => {
                    t.no_skin += 1;
                    let why = if node.bones.is_empty() {
                        "no bones"
                    } else if m.compression != 0 {
                        "compressed mesh"
                    } else if g.bone_indices.is_empty() {
                        "no bone indices"
                    } else {
                        "no bone weights"
                    };
                    *t.why.entry(why).or_default() += 1;
                    g.positions
                        .iter()
                        .map(|&p| place(&node.in_prefab, p))
                        .collect()
                }
            }
        } else {
            g.positions
                .iter()
                .map(|&p| place(&node.in_prefab, p))
                .collect()
        };
        match node.lod {
            Some(0) => {
                for p in points {
                    grow(&mut lod0, p);
                }
            }
            Some(1) if !node.skinned => {
                for p in points {
                    grow(&mut lod1, p);
                }
            }
            _ => {}
        }
    }
    if let (true, Some(a), Some(b)) = (any_skinned_lod0, lod0, lod1) {
        let size = |(lo, hi): Bounds| [0, 1, 2].map(|k| hi[k] - lo[k]);
        let centre = |(lo, hi): Bounds| [0, 1, 2].map(|k| (hi[k] + lo[k]) / 2.0);
        let (ca, cb) = (centre(a), centre(b));
        let shift = [0, 1, 2].map(|k| ca[k] - cb[k]);
        let shift = (shift[0] * shift[0] + shift[1] * shift[1] + shift[2] * shift[2]).sqrt();
        t.compared.push((name.to_string(), size(a), size(b), shift));
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let positions = crate::prefab::placed_positions(game)?;
    let keys: Vec<&String> = positions.keys().collect();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut t = Totals::default();
    let mut prefabs = 0;
    for key in &keys {
        let prefab = match assets.prefab(&catalog, key) {
            Ok(p) => p,
            Err(e) => {
                t.errors.push(e);
                continue;
            }
        };
        if prefab.nodes.iter().any(|n| n.active && n.skinned) {
            prefabs += 1;
            let near = positions[*key]
                .iter()
                .min_by(|a, b| {
                    let d = |p: &[f32; 3]| p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
                    d(a).total_cmp(&d(b))
                })
                .map(|p| p.map(|v| v.round()));
            println!(
                "  {key}: {} placements, nearest the lifepod start at {near:?}",
                positions[*key].len()
            );
            check(&assets, key, &prefab, &mut t);
        }
    }
    let mut pod = assets.scene("escapepod")?;
    pod.spawn_lightmapped_prefab();
    for root in &pod.roots {
        check(&assets, "escapepod scene", root, &mut t);
    }
    println!(
        "placed prefabs with active skinned meshes: {prefabs}; skinned renderers drawn: {} \
         ({} skinned, {} without skin data drawn as plain meshes)",
        t.renderers, t.skinned, t.no_skin
    );
    println!("without skin data: {:?}", t.why);
    println!(
        "bones ≠ bind poses: {}; bones outside the hierarchy: {}; bind poses without a 0 0 0 1 bottom row: {}",
        t.bone_mismatch, t.missing_bones, t.bad_layout
    );
    let mut worst = 0.0f32;
    let mut worst_shift = 0.0f32;
    for (name, a, b, shift) in &t.compared {
        worst_shift = worst_shift.max(*shift / b.iter().fold(1e-3f32, |m, v| m.max(*v)));
        let ratio = [0, 1, 2].map(|k| if b[k] > 1e-3 { a[k] / b[k] } else { 1.0 });
        let off = ratio.iter().map(|r| (r - 1.0).abs()).fold(0.0, f32::max);
        worst = worst.max(off);
        println!(
            "  {name}: skinned LOD 0 size {:?}, static LOD 1 size {:?}, ratio {:?}, centres {shift:.3} m apart",
            a.map(|v| (v * 100.0).round() / 100.0),
            b.map(|v| (v * 100.0).round() / 100.0),
            ratio.map(|v| (v * 1000.0).round() / 1000.0)
        );
    }
    println!(
        "LOD 0 / LOD 1 compared: {}, largest size difference {:.1} %, largest centre distance {:.1} % of the size",
        t.compared.len(),
        worst * 100.0,
        worst_shift * 100.0
    );
    println!("errors: {}", t.errors.len());
    for e in t.errors.iter().take(10) {
        println!("  {e}");
    }
    println!("({:.1} s)", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}

type ClipCache = HashMap<(std::path::PathBuf, String, i64), Arc<sn_unity::AnimationClip>>;

#[derive(Default)]
struct ShapeTotals {
    /// Drawn skinned renderers whose mesh has blend shapes, and their
    /// placements in the world.
    renderers: usize,
    placements: usize,
    with_bones: usize,
    channels: usize,
    /// Stored weights: 0, in 0–100, outside 0–100.
    stored: [usize; 3],
    /// Renderers with a channel an active animator drives, their
    /// placements, and the channels driven.
    animated: usize,
    animated_placements: usize,
    animated_channels: usize,
    /// Blend shape slots that reach no channel: path not below the
    /// animator, node without blend shapes, no channel of that name.
    unmatched_slots: [usize; 3],
    /// Animated weights over 30 s from the defaults: lowest, highest.
    range: (f32, f32),
    errors: Vec<String>,
}

/// Mesh-space bounds, rounded to mm, as (min, max) per axis.
fn rounded_bounds(points: &[[f32; 3]]) -> Option<[(f32, f32); 3]> {
    let mut b = None;
    for &p in points {
        grow(&mut b, p);
    }
    let r = |v: f32| (v * 1000.0).round() / 1000.0;
    b.map(|(lo, hi): Bounds| [0, 1, 2].map(|k| (r(lo[k]), r(hi[k]))))
}

fn shapes_of(
    assets: &Assets,
    name: &str,
    prefab: &Prefab,
    placements: usize,
    cache: &mut ClipCache,
    t: &mut ShapeTotals,
) {
    let names = assets.blend_shape_names(prefab);
    if names.is_empty() {
        return;
    }
    // Channels an animator drives, per node.
    let mut animated: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (i, node) in prefab.nodes.iter().enumerate() {
        let Some(a) = &node.animator else { continue };
        if !node.active || !a.component.enabled {
            continue;
        }
        let Some(controller) = &a.controller else {
            continue;
        };
        let set = match assets.animation_set(controller, cache) {
            Ok(s) => s,
            Err(e) => {
                t.errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        let program = Arc::new(Program::new(Arc::new(set.controller), &set.clips));
        let binding =
            prefab.bind_animator(i, &program, &|n| names.get(&n).cloned().unwrap_or_default());
        let mut driven = Vec::new();
        for (s, slot) in program.slots.iter().enumerate() {
            let SlotKind::Float {
                type_id: 137,
                attribute,
                custom_type: 20,
            } = slot.kind
            else {
                continue;
            };
            let Some(n) = binding.nodes[s] else {
                t.unmatched_slots[0] += 1;
                continue;
            };
            let Some(channels) = names.get(&n) else {
                t.unmatched_slots[1] += 1;
                continue;
            };
            let found = channels
                .iter()
                .position(|c| sn_unity::name_hash(c) == attribute);
            match found {
                Some(c) => {
                    animated.entry(n).or_default().insert(c);
                    driven.push(slot.offset);
                }
                None => t.unmatched_slots[2] += 1,
            }
        }
        if driven.is_empty() {
            continue;
        }
        let mut animator = Animator::new(program, binding.defaults);
        let mut range = (f32::INFINITY, f32::NEG_INFINITY);
        for _ in 0..30 * 60 {
            animator.update(1.0 / 60.0);
            for &at in &driven {
                let v = animator.pose()[at];
                range.0 = range.0.min(v);
                range.1 = range.1.max(v);
            }
        }
        if range.0 < 0.0 || range.1 > 100.0 {
            println!(
                "  {name}: animator on {:?} drives blend shape weights outside 0–100: {:.2}..{:.2}",
                node.name, range.0, range.1
            );
        }
        t.range.0 = t.range.0.min(range.0);
        t.range.1 = t.range.1.max(range.1);
    }
    for (&i, channels) in &names {
        let node = &prefab.nodes[i];
        if !node.active || !node.renderer_enabled {
            continue;
        }
        t.renderers += 1;
        t.placements += placements;
        t.with_bones += usize::from(!node.bones.is_empty());
        t.channels += channels.len();
        for c in 0..channels.len() {
            let w = node.blend_shape_weights.get(c).copied().unwrap_or(0.0);
            let bucket = if w == 0.0 {
                0
            } else if (0.0..=100.0).contains(&w) {
                1
            } else {
                2
            };
            t.stored[bucket] += 1;
        }
        let driven = animated.get(&i);
        if let Some(a) = driven {
            t.animated += 1;
            t.animated_placements += placements;
            t.animated_channels += a.len();
        }
        let Some(object) = &node.mesh else { continue };
        let (m, g) = match assets.mesh(object) {
            Ok(x) => x,
            Err(e) => {
                t.errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        let shaped = prefab.shaped_geometry(i, &m, &g);
        println!(
            "  {name} {:?} (LOD {:?}, {} bones, {placements} placements): channels {:?}, stored weights {:?}, animated {:?}",
            node.name,
            node.lod,
            node.bones.len(),
            channels,
            node.blend_shape_weights,
            driven.map(|a| a.iter().map(|&c| channels[c].as_str()).collect::<Vec<_>>()),
        );
        println!(
            "    mesh-space bounds (min, max per axis): base {:?}; with the stored weights {:?}",
            rounded_bounds(&g.positions),
            shaped.map(|s| rounded_bounds(&s.positions))
        );
    }
}

/// `prefab --shapes`: blend shapes of the placed prefabs and the escape
/// pod (M7f4d): renderers, stored weights, which an animator drives, the
/// range of animated weights, and each renderer's bounds with and without
/// its stored weights.
pub fn shapes(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let positions = crate::prefab::placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut t = ShapeTotals {
        range: (f32::INFINITY, f32::NEG_INFINITY),
        ..Default::default()
    };
    let mut cache = ClipCache::new();
    // Creatures are counted apart: the client does not place them yet.
    let mut creatures = ShapeTotals {
        range: t.range,
        ..Default::default()
    };
    for (key, at) in &positions {
        let totals = if key.starts_with("WorldEntities/Creatures/") {
            &mut creatures
        } else {
            &mut t
        };
        match assets.prefab(&catalog, key) {
            Ok(prefab) => shapes_of(&assets, key, &prefab, at.len(), &mut cache, totals),
            Err(e) => totals.errors.push(e),
        }
    }
    let mut pod = assets.scene("escapepod")?;
    pod.spawn_lightmapped_prefab();
    for root in &pod.roots {
        shapes_of(&assets, "escapepod scene", root, 1, &mut cache, &mut t);
    }
    println!(
        "drawn renderers with blend shapes: {} ({} placements; {} with bones), channels {}",
        t.renderers, t.placements, t.with_bones, t.channels
    );
    println!(
        "stored weights: {} at 0, {} in 0–100, {} outside",
        t.stored[0], t.stored[1], t.stored[2]
    );
    println!(
        "animated: {} renderers ({} placements), {} channels; blendShape slots reaching no channel (path not below the animator, node without shapes, no such channel): {:?}; animated weights over 30 s in {:.2}..{:.2}",
        t.animated,
        t.animated_placements,
        t.animated_channels,
        t.unmatched_slots,
        t.range.0,
        t.range.1
    );
    println!(
        "creatures (not placed by the client yet): {} renderers with blend shapes ({} placements), {} animated, {} channels driven; slots reaching no channel {:?}; animated weights in {:.2}..{:.2}",
        creatures.renderers,
        creatures.placements,
        creatures.animated,
        creatures.animated_channels,
        creatures.unmatched_slots,
        creatures.range.0,
        creatures.range.1
    );
    t.errors.extend(creatures.errors);
    println!("errors: {}", t.errors.len());
    for e in t.errors.iter().take(10) {
        println!("  {e}");
    }
    println!("({:.1} s)", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
