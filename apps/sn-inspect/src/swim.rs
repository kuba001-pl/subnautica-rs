//! `swim` (M9a): a scripted swim from the lifepod to the nearest Kelp
//! Forest, headless, through our collision (`sn_sim::collide`). Terrain and
//! objects are loaded for the batches within the game's collision range of
//! the player as it moves (`crate::collision`). Logs contacts,
//! penetrations, the smallest gap and the cost per step. See
//! `docs/DESIGN.md` § 4.3 "M9a plan".

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_install::GameData;
use sn_sim::V3;
use sn_sim::collide::Capsule;
use sn_terrain::batch_voxels;
use sn_world::{voxel_to_world, world_to_voxel};

use crate::Result;
use crate::collision::{Streamed, kind};

/// Steps per second of the scripted swim.
const RATE: f64 = 60.0;

/// The script aims this deep, below the seabed of the shallows and the
/// Kelp Forest, so the capsule presses on the terrain and slides along it
/// the whole way (a test of the collision, not how players swim).
const AIM_DEPTH: f64 = -150.0;

/// The target is a Kelp Forest cell with Kelp Forest this many map cells
/// (4 m each) around it, so arriving means being inside the forest.
const INSIDE_CELLS: i64 = 5;

/// The swim ends this close to the target (horizontal distance, metres).
const ARRIVED: f64 = 3.0;

/// A gap below this counts as a penetration.
pub const PENETRATION: f64 = -0.001;

pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// The loader's and the world's totals.
pub fn print_load_stats(s: &Streamed) {
    let mut loads = s.load_ms.clone();
    loads.sort_by(f64::total_cmp);
    println!(
        "batches loaded: {} (terrain + objects, mean {:.0} ms, max {:.0} ms); terrain triangles {}, object triangles {}, object shapes {}, skipped {}",
        loads.len(),
        loads.iter().sum::<f64>() / loads.len().max(1) as f64,
        loads.last().copied().unwrap_or(0.0),
        s.terrain_triangles,
        s.object_triangles,
        s.object_shapes,
        s.skipped
    );
    let st = s.loader.stats();
    let c = &st.counts;
    println!(
        "{} objects placed from {} prefabs read; their colliders (counted once per prefab): solid {}, triggers {}, disabled {}, inactive {}, null mesh {}, mesh errors {}, convex {}, layout errors {}; solid by layer {:?}; placeholder spawns {}, placements already holding them {}; creatures skipped {}, unreadable prefabs {}",
        st.objects_placed,
        st.prefabs_read,
        c.kept,
        c.triggers,
        c.disabled,
        c.inactive,
        c.null_mesh,
        c.mesh_errors,
        c.convex,
        c.layout_errors,
        c.layers,
        st.placeholders_spawned,
        st.placeholders_already_saved,
        st.creatures_skipped,
        st.unreadable_prefabs
    );
}

pub fn run(game: &GameData, seed: u64, max_seconds: f64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
    let assets = s.loader.assets();
    let index = s.loader.index().clone();

    // The player's capsule under water and its speed (P0's numbers).
    let player = sn_assets::player_data(assets)?;
    let pc = &player.controller;
    let height = f64::from(pc.swim_height - pc.camera_offset);
    let center = V3::new(0.0, -height * 0.5 - f64::from(pc.camera_offset), 0.0);
    let capsule = Capsule::unity(f64::from(pc.controller_radius), height, 1, center);
    let speed = f64::from(pc.swim_forward_max_speed);
    println!(
        "player capsule: radius {}, height {height} (swim height {} − camera offset {}), centre {:.3} m below the camera; swim speed {speed:.2} m/s",
        pc.controller_radius, pc.swim_height, pc.camera_offset, -center.y
    );

    // Lifepod and the nearest Kelp Forest.
    let map = assets.start_map()?;
    let (point, _) = map.random_start(seed);
    let mut scene = assets.scene("escapepod")?;
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(assets, point)?;
    scene.follow_targets(assets)?;
    let (biomes, names) = game.read_biome_map()?;
    let kelp = names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("kelpForest"))
        .ok_or("no kelpForest in biomes.csv")?;
    let land = index.batches()[0] as usize * batch_voxels(&index)[0] as usize;
    let factor = (land / biomes.width).max(1);
    let is_kelp = |cx: i64, cz: i64| {
        cx >= 0
            && cz >= 0
            && (cx as usize) < biomes.width
            && (cz as usize) < biomes.height
            && usize::from(biomes.cells[cz as usize * biomes.width + cx as usize]) == kelp
    };
    let mut target: Option<(f64, V3)> = None;
    for (i, &b) in biomes.cells.iter().enumerate() {
        if usize::from(b) != kelp {
            continue;
        }
        let (cx, cz) = (i % biomes.width, i / biomes.width);
        let r = INSIDE_CELLS;
        let inside = (-r..=r).all(|dz| (-r..=r).all(|dx| is_kelp(cx as i64 + dx, cz as i64 + dz)));
        if !inside {
            continue;
        }
        let vx = ((cx as f64 + 0.5) * factor as f64) as f32;
        let vz = ((cz as f64 + 0.5) * factor as f64) as f32;
        let w = voxel_to_world([vx, 0.0, vz]);
        let p = V3::new(f64::from(w[0]), AIM_DEPTH, f64::from(w[2]));
        let d = ((p.x - f64::from(point[0])).powi(2) + (p.z - f64::from(point[2])).powi(2)).sqrt();
        if target.is_none_or(|(best, _)| d < best) {
            target = Some((d, p));
        }
    }
    let (distance, target) = target.ok_or("no Kelp Forest cell on the biome map")?;
    println!(
        "seed {seed}: lifepod at ({:.1}, {:.1}); nearest Kelp Forest cell with Kelp Forest {} m around: ({:.1}, {:.1}), {distance:.0} m away",
        point[0],
        point[2],
        INSIDE_CELLS * factor as i64,
        target.x,
        target.z
    );

    let (pod, _) = s.add_scene(&mut scene, &[])?;
    println!("lifepod scene and modules: {pod} colliders");

    // Start beside the pod, 2 m under water, on the side facing the target.
    let p0 = V3::new(f64::from(point[0]), -2.0, f64::from(point[2]));
    let away = V3::new(target.x - p0.x, 0.0, target.z - p0.z)
        .normalized()
        .unwrap_or(V3::new(1.0, 0.0, 0.0));
    let mut pos = p0 + away * 4.0;
    s.stream(pos)?;
    let (start, first) = s.settle(&capsule, pos);
    println!(
        "start ({:.2}, {:.2}, {:.2}): nearest gap {}",
        pos.x,
        pos.y,
        pos.z,
        first.map_or("none within 1 cm".into(), |g| format!(
            "{g:.3} m (pushed out)"
        ))
    );
    pos = start;

    let dt = 1.0 / RATE;
    let max_steps = (max_seconds * RATE) as usize;
    let (mut steps, mut penetrations, mut contacts) = (0usize, 0usize, 0usize);
    // Surfaces the capsule's centre passed through (one-sided triangles
    // hide those from the clearance check).
    let mut crossings = 0usize;
    let centre = (capsule.a + capsule.b) * 0.5;
    let mut contacts_by: BTreeMap<&str, usize> = BTreeMap::new();
    let mut min_gap = f64::INFINITY;
    let mut step_us: Vec<f64> = Vec::new();
    let mut travelled = 0.0;
    let mut stuck_steps = 0usize;
    let mut rise = 0.0f64;
    // Steps blocked in a row; detour steps left and the detour's side.
    let (mut blocked_run, mut detour_left, mut detour_side, mut detours) =
        (0usize, 0usize, 1.0, 0usize);
    let mut arrived = false;
    let mut last_log = 0usize;
    while steps < max_steps {
        let flat = V3::new(target.x - pos.x, 0.0, target.z - pos.z);
        if flat.length() < ARRIVED {
            arrived = true;
            break;
        }
        if steps % 30 == 0 {
            s.stream(pos)?;
        }
        // Head for the target along the seabed; climb while blocked (what
        // a player does at a steep slope), easing off once free again.
        let aim = target - pos;
        let mut dir = aim.normalized().unwrap_or(V3::ZERO);
        dir = (dir + V3::Y * rise).normalized().unwrap_or(dir);
        // Wedged (a slot narrower than the capsule): back out sideways and
        // up for a while, as a player would, switching sides each time.
        if detour_left > 0 {
            detour_left -= 1;
            let side = V3::Y.cross(flat).normalized().unwrap_or(V3::ZERO) * detour_side;
            dir = (side + V3::Y * 0.5 - flat.normalized().unwrap_or(V3::ZERO) * 0.3)
                .normalized()
                .unwrap_or(dir);
        }
        let want = dir * (speed * dt);

        let t = Instant::now();
        let slide = s.world.move_and_slide(&capsule, pos, want);
        step_us.push(t.elapsed().as_secs_f64() * 1e6);

        crossings += s.world.crossings(pos + centre, slide.position + centre);
        let moved = (slide.position - pos).length();
        travelled += moved;
        pos = slide.position;
        contacts += slide.contacts.len();
        for h in &slide.contacts {
            *contacts_by.entry(kind(h.body)).or_default() += 1;
        }
        if moved < 0.3 * want.length() {
            stuck_steps += 1;
            blocked_run += 1;
            if blocked_run >= RATE as usize && detour_left == 0 {
                blocked_run = 0;
                detours += 1;
                detour_side = -detour_side;
                detour_left = (1.5 * RATE) as usize;
            }
            rise = (rise + 0.05).min(3.0);
        } else {
            blocked_run = 0;
            rise = (rise - 0.02).max(0.0);
        }
        if let Some(c) = s.world.clearance(&capsule, pos, 0.5) {
            min_gap = min_gap.min(c.gap);
            if c.gap < PENETRATION {
                penetrations += 1;
                if penetrations <= 5 {
                    println!(
                        "  PENETRATION at step {steps}: gap {:.4} m at ({:.2}, {:.2}, {:.2})",
                        c.gap, pos.x, pos.y, pos.z
                    );
                }
            }
        }
        steps += 1;
        if steps - last_log >= 10 * RATE as usize {
            last_log = steps;
            println!(
                "  t {:>5.1} s: ({:>8.2}, {:>6.2}, {:>8.2}), {:>5.0} m to go, {contacts} contacts, {} bodies",
                steps as f64 * dt,
                pos.x,
                pos.y,
                pos.z,
                flat.length(),
                s.world.len()
            );
        }
    }

    let biome_here: String = {
        let v = world_to_voxel(pos.to_f32());
        biomes
            .index_at(v[0] as i32, v[2] as i32, land)
            .and_then(|i| names.get(usize::from(i)).cloned())
            .unwrap_or_else(|| "(outside the map)".into())
    };
    let arrived = arrived && biome_here == names[kelp];
    println!(
        "{}: {steps} steps ({:.1} s simulated), {travelled:.0} m travelled, ended at ({:.2}, {:.2}, {:.2}) in {biome_here}",
        if arrived { "ARRIVED" } else { "NOT ARRIVED" },
        steps as f64 * dt,
        pos.x,
        pos.y,
        pos.z
    );
    println!(
        "contacts: {contacts} ({contacts_by:?}); steps blocked (< 30 % of the move): {stuck_steps}; detours: {detours}"
    );
    println!(
        "penetrations (gap < {PENETRATION} m): {penetrations}; smallest gap {min_gap:.4} m; surfaces passed through: {crossings}"
    );
    let mut sorted = step_us.clone();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
    println!(
        "cost per step (move_and_slide): mean {mean:.1} µs, p50 {:.1}, p99 {:.1}, max {:.1}",
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0)
    );
    print_load_stats(&s);
    println!("total {:.1} s", started.elapsed().as_secs_f64());
    Ok(if penetrations == 0 && crossings == 0 && arrived {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
