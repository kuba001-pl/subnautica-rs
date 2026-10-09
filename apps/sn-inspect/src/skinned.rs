//! `prefab --skinned`: every skinned mesh of the placed prefabs and of the
//! escape pod scene: skin data checks, and the skinned LOD 0's bounds next
//! to the static LOD 1's where a prefab has both.

use std::process::ExitCode;
use std::time::Instant;

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
