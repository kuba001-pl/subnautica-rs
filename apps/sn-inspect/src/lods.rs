//! `lods` command (M7f4f): the LOD groups of the placed prefabs and the
//! scenes, the quality levels' LOD settings, the camera's field of view,
//! and which level each placed group shows at a set of distances.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, Prefab};
use sn_install::GameData;
use sn_world::lod;

use crate::Result;

/// Distances (m) the level census is taken at.
const DISTANCES: [f32; 8] = [5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0];

#[derive(Default)]
struct Census {
    prefabs: usize,
    groups: usize,
    levels: BTreeMap<usize, usize>,
    fade_modes: BTreeMap<i32, usize>,
    conflicts: usize,
    /// Renderers listed in more than one level of their group.
    multi_level: usize,
    nodes_in_groups: usize,
}

impl Census {
    fn add(&mut self, prefab: &Prefab) {
        if prefab.lod_groups.is_empty() {
            return;
        }
        self.prefabs += 1;
        self.groups += prefab.lod_groups.len();
        for g in &prefab.lod_groups {
            *self.levels.entry(g.heights.len()).or_default() += 1;
            *self.fade_modes.entry(g.fade_mode).or_default() += 1;
        }
        self.conflicts += prefab.lod_conflicts;
        for (_, n) in prefab.drawn() {
            if let Some((_, levels)) = n.lod_group {
                self.nodes_in_groups += 1;
                self.multi_level += usize::from(levels.count_ones() > 1);
            }
        }
    }

    fn print(&self, what: &str) {
        println!(
            "{what}: {} prefabs with LOD groups, {} groups; levels per group {:?}; fade modes {:?}; \
             drawn renderers in groups {}, in several levels {}; listed by two groups {}",
            self.prefabs,
            self.groups,
            self.levels,
            self.fade_modes,
            self.nodes_in_groups,
            self.multi_level,
            self.conflicts
        );
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;

    let quality = sn_assets::quality_settings(&assets)?;
    println!("QualitySettings: current level {}", quality.current);
    for l in &quality.levels {
        println!(
            "  {}: lodBias {}, maximumLODLevel {}, shadow distance {} m, {} cascades",
            l.name, l.lod_bias, l.maximum_lod_level, l.shadow_distance, l.shadow_cascades
        );
    }
    let high = quality
        .level("High")
        .ok_or("QualitySettings: no level High")?;
    let fov = sn_assets::field_of_view_code(&sn_assets::read_assembly(game)?)?;
    println!("camera: MiscSettings.fieldOfView {fov}° (vertical)");

    let placed = crate::prefab::placed_transforms(game)?;
    let mut census = Census::default();
    let mut unreadable = 0;
    // Distance → level → group placements.
    let mut bands: Vec<BTreeMap<Option<usize>, usize>> = vec![BTreeMap::new(); DISTANCES.len()];
    let mut sizes: Vec<f32> = Vec::new();
    for (key, placements) in &placed {
        let Ok(prefab) = assets.prefab(&catalog, key) else {
            unreadable += 1;
            continue;
        };
        census.add(&prefab);
        for t in placements {
            for g in &prefab.lod_groups {
                let (_, size) = g.world(&prefab, t);
                sizes.push(size);
                for (k, &d) in DISTANCES.iter().enumerate() {
                    let h = lod::relative_height(size, d, fov, high.lod_bias);
                    let level = lod::level(&g.heights, h, high.maximum_lod_level.max(0) as usize);
                    *bands[k].entry(level).or_default() += 1;
                }
            }
        }
    }
    census.print("placed prefabs");
    sizes.sort_by(f32::total_cmp);
    if let (Some(lo), Some(hi)) = (sizes.first(), sizes.last()) {
        println!(
            "placed groups: {} placements, world size min {lo:.2} m, median {:.2} m, max {hi:.1} m",
            sizes.len(),
            sizes[sizes.len() / 2]
        );
    }
    println!(
        "levels shown at High (lodBias {}, {fov}° view), placed groups by distance:",
        high.lod_bias
    );
    for (k, d) in DISTANCES.iter().enumerate() {
        let line: Vec<String> = bands[k]
            .iter()
            .map(|(level, n)| match level {
                Some(l) => format!("LOD {l}: {n}"),
                None => format!("culled: {n}"),
            })
            .collect();
        println!("  {d:>5} m: {}", line.join(", "));
    }

    let mut scenes = Census::default();
    for name in ["aurora", "escapepod"] {
        let mut scene = assets.scene(name)?;
        scene.spawn_lightmapped_prefab();
        for root in &scene.roots {
            scenes.add(root);
        }
    }
    scenes.print("the Aurora and escape pod scenes");
    println!(
        "{} placed prefabs, {unreadable} unreadable ({:.2} s)",
        placed.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(ExitCode::SUCCESS)
}
