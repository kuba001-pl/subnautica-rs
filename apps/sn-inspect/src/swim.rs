//! `swim` (M9a): a scripted swim from the lifepod to the nearest Kelp
//! Forest, headless, through our collision (`sn_sim::collide`). Terrain and
//! objects are loaded for the batches within the game's collision range of
//! the player as it moves. Logs contacts, penetrations, the smallest gap
//! and the cost per step. See `docs/DESIGN.md` § 4.3 "M9a plan".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use sn_assets::{Assets, ColliderCounts, ColliderMeshes, PrefabCollider, WorldCollider};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::collide::{Body, Capsule, Shape, World};
use sn_terrain::{Neighbourhood, TerrainBatch, batch_voxels, collision_triangles};
use sn_world::{BatchCoord, SLOTS_COMPONENT, Transform, voxel_to_world, world_to_voxel};

use crate::Result;

/// Steps per second of the scripted swim.
const RATE: f64 = 60.0;

/// Half the side of the region with terrain collision: the game's finest
/// clipmap level, 7 chunks of 16 m (`clipmaps-high.json`).
const COLLISION_REACH: f64 = 56.0;

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
const PENETRATION: f64 = -0.001;

/// Body ids: terrain batches and the objects of a batch; the lifepod.
fn terrain_id(c: BatchCoord) -> u64 {
    ((c.x as u64 & 0xffff) << 32) | ((c.y as u64 & 0xffff) << 16) | (c.z as u64 & 0xffff)
}
const OBJECTS: u64 = 1 << 62;
const LIFEPOD: u64 = 1 << 61;

fn shape_of(c: &WorldCollider, tris: &mut Vec<[[f32; 3]; 3]>, shapes: &mut Vec<Shape>) {
    let v = V3::from_f32;
    match c {
        WorldCollider::Box { center, axes, half } => shapes.push(Shape::Box {
            center: v(*center),
            axes: axes.map(v),
            half: half.map(f64::from),
        }),
        WorldCollider::Sphere { center, radius } => shapes.push(Shape::Sphere {
            center: v(*center),
            radius: f64::from(*radius),
        }),
        WorldCollider::Capsule { a, b, radius } => shapes.push(Shape::Capsule {
            a: v(*a),
            b: v(*b),
            radius: f64::from(*radius),
        }),
        WorldCollider::Triangles(t) => tris.extend_from_slice(t),
    }
}

fn body_of(colliders: &[(Arc<Vec<PrefabCollider>>, Transform)]) -> Body {
    let (mut tris, mut shapes) = (Vec::new(), Vec::new());
    for (list, at) in colliders {
        for c in list.iter() {
            shape_of(&c.world(at), &mut tris, &mut shapes);
        }
    }
    Body::new(tris, shapes)
}

struct Loader<'a> {
    game: &'a GameData,
    assets: Assets<'a>,
    catalog: sn_unity::Catalog,
    index: sn_world::WorldIndex,
    class_paths: HashMap<String, String>,
    seed: u64,
    loot: sn_assets::LootTable,
    infos: HashMap<String, sn_world::EntityInfo>,
    batches: HashMap<BatchCoord, Arc<Option<TerrainBatch>>>,
    prefabs: HashMap<String, Option<Arc<Vec<PrefabCollider>>>>,
    meshes: ColliderMeshes,
    counts: ColliderCounts,
    unreadable_prefabs: usize,
    creatures_skipped: usize,
    objects_placed: usize,
}

impl Loader<'_> {
    fn batch(&mut self, c: BatchCoord) -> Result<Arc<Option<TerrainBatch>>> {
        if let Some(b) = self.batches.get(&c) {
            return Ok(b.clone());
        }
        let b = Arc::new(self.game.load_batch(&self.index, c)?);
        self.batches.insert(c, b.clone());
        Ok(b)
    }

    fn terrain(&mut self, c: BatchCoord) -> Result<Option<Body>> {
        let mut around = Vec::with_capacity(27);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = c.offset(dx, dy, dz);
                    around.push((n, self.batch(n)?));
                }
            }
        }
        let lookup = |n: BatchCoord| {
            around
                .iter()
                .find(|(coord, _)| *coord == n)
                .and_then(|(_, b)| b.as_ref().as_ref())
        };
        let hood = Neighbourhood::new(c, batch_voxels(&self.index), lookup);
        Ok(collision_triangles(&hood).map(|t| Body::new(t, Vec::new())))
    }

    fn prefab(&mut self, path: &str) -> Option<Arc<Vec<PrefabCollider>>> {
        if path.starts_with("WorldEntities/Creatures/") {
            self.creatures_skipped += 1;
            return None;
        }
        if let Some(p) = self.prefabs.get(path) {
            return p.clone();
        }
        let read = self.assets.prefab(&self.catalog, path).and_then(|p| {
            let (list, counts) = self.assets.prefab_colliders(&p, &mut self.meshes)?;
            self.counts.add(&counts);
            Ok(Arc::new(list))
        });
        let p = match read {
            Ok(p) => Some(p),
            Err(_) => {
                self.unreadable_prefabs += 1;
                None
            }
        };
        self.prefabs.insert(path.to_string(), p.clone());
        p
    }

    /// Every placed object of a batch (its cells, slot spawns and batch
    /// objects), as the client places them; creatures and placeholder
    /// spawns left out.
    fn objects(&mut self, c: BatchCoord) -> Result<Body> {
        let mut placed: Vec<(String, Transform)> = Vec::new();
        if let Some(file) = self.game.read_batch_cells(c)? {
            for cell in &file.cells {
                let Some(tree) = &cell.objects else { continue };
                let (world, _) = tree.world_transforms();
                for (object, transform) in tree.objects.iter().zip(world) {
                    for comp in &object.components {
                        if comp.type_name != SLOTS_COMPONENT {
                            continue;
                        }
                        let Ok(slots) = sn_world::parse_slots(&comp.data) else {
                            continue;
                        };
                        let spawns = sn_world::fill_slots(
                            self.seed,
                            &object.id,
                            &slots,
                            &self.loot.distribution,
                            &self.infos,
                        );
                        for s in spawns {
                            if let Some(path) = self.class_paths.get(s.class_id) {
                                placed.push((path.clone(), transform.then(&s.transform)));
                            }
                        }
                    }
                    if let Some(path) = self.class_paths.get(&object.class_id) {
                        placed.push((path.clone(), transform));
                    }
                }
            }
        }
        if let Some(mut tree) = self.game.read_batch_objects(c)? {
            // Roots sit at the batch's corner (docs/formats/entities.md).
            let corner = [c.x, c.y, c.z]
                .map(|v| v as f32 * 160.0)
                .iter()
                .zip(sn_world::VOXEL_WORLD_OFFSET)
                .map(|(v, o)| v - o)
                .collect::<Vec<_>>();
            for o in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
                o.transform.position = [corner[0], corner[1], corner[2]];
            }
            let (world, _) = tree.world_transforms();
            for (object, transform) in tree.objects.iter().zip(world) {
                if let Some(path) = self.class_paths.get(&object.class_id) {
                    placed.push((path.clone(), transform));
                }
            }
        }
        let mut colliders = Vec::new();
        for (path, at) in placed {
            if let Some(list) = self.prefab(&path) {
                self.objects_placed += 1;
                colliders.push((list, at));
            }
        }
        Ok(body_of(&colliders))
    }
}

fn batch_of(index: &sn_world::WorldIndex, p: V3) -> BatchCoord {
    let size = batch_voxels(index);
    let v = world_to_voxel(p.to_f32());
    BatchCoord::new(
        (v[0].floor() as i32).div_euclid(size[0]),
        (v[1].floor() as i32).div_euclid(size[1]),
        (v[2].floor() as i32).div_euclid(size[2]),
    )
}

/// Batches touched by the cube of half side `reach` around `p`.
fn batches_around(index: &sn_world::WorldIndex, p: V3, reach: f64) -> Vec<BatchCoord> {
    let r = V3::new(reach, reach, reach);
    let (lo, hi) = (batch_of(index, p - r), batch_of(index, p + r));
    let mut out = Vec::new();
    for z in lo.z..=hi.z {
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                out.push(BatchCoord::new(x, y, z));
            }
        }
    }
    out
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

pub fn run(game: &GameData, seed: u64, max_seconds: f64) -> Result<ExitCode> {
    let started = Instant::now();
    let assets = Assets::index(game)?;
    let index = game.read_index()?;

    // The player's capsule under water and its speed (P0's numbers).
    let player = sn_assets::player_data(&assets)?;
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
    scene.place_escape_pod(&assets, point)?;
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

    let mut loader = Loader {
        game,
        catalog: assets.catalog()?,
        class_paths: game.read_prefab_database()?,
        loot: sn_assets::loot_table(&assets)?,
        infos: sn_assets::entity_infos(&assets)?,
        assets,
        index,
        seed,
        batches: HashMap::new(),
        prefabs: HashMap::new(),
        meshes: ColliderMeshes::new(),
        counts: ColliderCounts::default(),
        unreadable_prefabs: 0,
        creatures_skipped: 0,
        objects_placed: 0,
    };

    let mut world = World::new();
    // The lifepod's own colliders (its spawned modules left out).
    let mut pod = Vec::new();
    for root in &scene.roots {
        let (list, counts) = loader.assets.prefab_colliders(root, &mut loader.meshes)?;
        loader.counts.add(&counts);
        pod.push((Arc::new(list), root.nodes[0].local));
    }
    let pod_body = body_of(&pod);
    println!(
        "lifepod scene: {} triangles, {} primitive shapes",
        pod_body.triangle_count(),
        pod_body.shape_count()
    );
    world.insert(LIFEPOD, pod_body);

    // Start beside the pod, 2 m under water, on the side facing the target.
    let p0 = V3::new(f64::from(point[0]), -2.0, f64::from(point[2]));
    let away = V3::new(target.x - p0.x, 0.0, target.z - p0.z)
        .normalized()
        .unwrap_or(V3::new(1.0, 0.0, 0.0));
    let mut pos = p0 + away * 4.0;

    let mut loaded: HashSet<BatchCoord> = HashSet::new();
    let mut load_ms: Vec<f64> = Vec::new();
    let (mut terrain_triangles, mut object_triangles, mut object_shapes, mut skipped) =
        (0, 0, 0, 0);
    let mut stream = |world: &mut World, pos: V3, loader: &mut Loader| -> Result<()> {
        let want: HashSet<BatchCoord> = batches_around(&loader.index, pos, COLLISION_REACH)
            .into_iter()
            .collect();
        for &c in want.difference(&loaded.clone()) {
            let t = Instant::now();
            if let Some(body) = loader.terrain(c)? {
                terrain_triangles += body.triangle_count();
                skipped += body.skipped();
                world.insert(terrain_id(c), body);
            }
            let body = loader.objects(c)?;
            object_triangles += body.triangle_count();
            object_shapes += body.shape_count();
            skipped += body.skipped();
            world.insert(OBJECTS | terrain_id(c), body);
            loaded.insert(c);
            load_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        for c in loaded.clone().difference(&want) {
            world.remove(terrain_id(*c));
            world.remove(OBJECTS | terrain_id(*c));
            loaded.remove(c);
        }
        Ok(())
    };
    stream(&mut world, pos, &mut loader)?;

    let (start, first) = world.push_out(&capsule, pos);
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
            stream(&mut world, pos, &mut loader)?;
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
        let slide = world.move_and_slide(&capsule, pos, want);
        step_us.push(t.elapsed().as_secs_f64() * 1e6);

        let moved = (slide.position - pos).length();
        travelled += moved;
        pos = slide.position;
        contacts += slide.contacts.len();
        for h in &slide.contacts {
            let kind = match h.body {
                LIFEPOD => "lifepod",
                b if b & OBJECTS != 0 => "objects",
                _ => "terrain",
            };
            *contacts_by.entry(kind).or_default() += 1;
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
        if let Some(c) = world.clearance(&capsule, pos, 0.5) {
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
                world.len()
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
    println!("penetrations (gap < {PENETRATION} m): {penetrations}; smallest gap {min_gap:.4} m");
    let mut sorted = step_us.clone();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len().max(1) as f64;
    println!(
        "cost per step (move_and_slide): mean {mean:.1} µs, p50 {:.1}, p99 {:.1}, max {:.1}",
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0)
    );
    let mut loads = load_ms.clone();
    loads.sort_by(f64::total_cmp);
    println!(
        "batches loaded: {} (terrain + objects, mean {:.0} ms, max {:.0} ms); terrain triangles {terrain_triangles}, object triangles {object_triangles}, object shapes {object_shapes}, skipped {skipped}",
        loads.len(),
        loads.iter().sum::<f64>() / loads.len().max(1) as f64,
        loads.last().copied().unwrap_or(0.0)
    );
    let c = &loader.counts;
    println!(
        "{} objects placed from {} distinct prefabs; their colliders (counted once per prefab): kept {}, triggers {}, disabled {}, inactive {}, null mesh {}, mesh errors {}, convex {}, layout errors {}; by layer {:?}; creatures skipped {}, unreadable prefabs {}",
        loader.objects_placed,
        loader.prefabs.values().filter(|p| p.is_some()).count(),
        c.kept,
        c.triggers,
        c.disabled,
        c.inactive,
        c.null_mesh,
        c.mesh_errors,
        c.convex,
        c.layout_errors,
        c.layers,
        loader.creatures_skipped,
        loader.unreadable_prefabs
    );
    println!("total {:.1} s", started.elapsed().as_secs_f64());
    Ok(if penetrations == 0 && arrived {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
