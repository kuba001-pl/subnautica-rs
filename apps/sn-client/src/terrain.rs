//! Terrain streaming: picks a level of detail for every batch around the
//! camera, meshes batches on worker threads (nearest first), swaps meshes in
//! when they are ready and unloads batches that fall out of view.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sn_install::GameData;
use sn_mesh::{LayerSettings, add_skirts, build_layers};
use sn_terrain::{MAX_LOD, Neighbourhood, TerrainBatch, batch_mesh, batch_voxels, debug_colour};
use sn_world::{BatchCoord, WorldIndex, voxel_to_world, world_to_voxel};

use crate::terrain_look::{TerrainLook, TerrainMaterial};

/// Distances (metres from the camera to the nearest point of a batch) up to
/// which each level of detail is used. Beyond the last, batches unload.
#[derive(Clone, Copy, Debug)]
pub struct LodRanges(pub [f32; MAX_LOD as usize + 1]);

impl Default for LodRanges {
    fn default() -> Self {
        LodRanges([100.0, 260.0, 600.0, 1200.0])
    }
}

/// A batch moves to a finer level only when this much closer than the range
/// boundary, so batches near a boundary don't flip back and forth.
const HYSTERESIS: f32 = 20.0;

/// Parsed batches kept in memory for meshing. Average batch ≈ 0.2 MB.
const CACHE_LIMIT: usize = 1200;

/// The triangles of one batch drawn with one material at one place in the
/// draw order, in Bevy coordinates.
pub struct TerrainPart {
    pub material: u8,
    /// 0: opaque; higher ranks are blended over lower ones (see `sn_mesh::build_layers`).
    pub rank: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Job {
    coord: BatchCoord,
    lod: u32,
    requested: Instant,
}

struct Finished {
    job: Job,
    parts: Vec<TerrainPart>,
    mesh_ms: f32,
}

enum WorkerMessage {
    Finished(Finished),
    Error(String),
}

/// Parsed batches with least-recently-used eviction.
#[derive(Default)]
struct BatchCache {
    tick: u64,
    entries: HashMap<BatchCoord, (Arc<Option<TerrainBatch>>, u64)>,
}

impl BatchCache {
    fn get(&mut self, coord: BatchCoord) -> Option<Arc<Option<TerrainBatch>>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(&coord).map(|(batch, used)| {
            *used = tick;
            batch.clone()
        })
    }

    fn insert(&mut self, coord: BatchCoord, batch: Arc<Option<TerrainBatch>>) {
        self.tick += 1;
        self.entries.insert(coord, (batch, self.tick));
        if self.entries.len() > CACHE_LIMIT {
            // Evict the least recently used tenth in one go.
            let mut ages: Vec<(u64, BatchCoord)> = self
                .entries
                .iter()
                .map(|(c, (_, used))| (*used, *c))
                .collect();
            ages.sort_unstable();
            for (_, coord) in ages.iter().take(CACHE_LIMIT / 10) {
                self.entries.remove(coord);
            }
        }
    }
}

/// State shared with the worker threads.
struct Shared {
    game: GameData,
    index: WorldIndex,
    /// Block-type settings for material layers; `None` with debug colours.
    blocks: Option<BlockSettings>,
    /// Pending jobs, most urgent last.
    queue: Mutex<Vec<Job>>,
    wake: Condvar,
    cache: Mutex<BatchCache>,
    shutdown: AtomicBool,
}

impl Shared {
    fn batch(&self, coord: BatchCoord) -> Result<Arc<Option<TerrainBatch>>, String> {
        if let Some(batch) = self.cache.lock().unwrap().get(coord) {
            return Ok(batch);
        }
        // Load outside the lock; two workers may occasionally load the same batch.
        let batch = Arc::new(self.game.load_batch(&self.index, coord)?);
        self.cache.lock().unwrap().insert(coord, batch.clone());
        Ok(batch)
    }

    fn mesh(&self, job: Job) -> Result<Vec<TerrainPart>, String> {
        let mut around = Vec::with_capacity(27);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let c = job.coord.offset(dx, dy, dz);
                    around.push((c, self.batch(c)?));
                }
            }
        }
        let lookup = |c: BatchCoord| {
            around
                .iter()
                .find(|(coord, _)| *coord == c)
                .and_then(|(_, b)| b.as_ref().as_ref())
        };
        let neighbourhood = Neighbourhood::new(job.coord, batch_voxels(&self.index), lookup);
        let Some(mut mesh) = batch_mesh(&neighbourhood, job.lod) else {
            return Ok(Vec::new());
        };
        // Skirts hide cracks next to batches at another level of detail.
        add_skirts(&mut mesh, 2.0 * (1u32 << job.lod) as f32);
        Ok(match &self.blocks {
            Some(blocks) => layered_parts(&mesh, blocks, job.lod),
            None => split_by_material(&mesh),
        })
    }
}

fn worker(shared: Arc<Shared>, tx: Sender<WorkerMessage>) {
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(job) = queue.pop() {
                    break job;
                }
                queue = shared.wake.wait(queue).unwrap();
            }
        };
        let start = Instant::now();
        let message = match shared.mesh(job) {
            Ok(parts) => WorkerMessage::Finished(Finished {
                job,
                parts,
                mesh_ms: start.elapsed().as_secs_f32() * 1000.0,
            }),
            Err(e) => WorkerMessage::Error(e),
        };
        if tx.send(message).is_err() {
            return;
        }
    }
}

/// What is on screen for one batch.
struct Shown {
    lod: u32,
    entities: Vec<Entity>,
    triangles: usize,
}

#[derive(Default, Clone, Copy)]
pub struct StreamStats {
    pub shown_batches: usize,
    pub shown_triangles: usize,
    /// Meshes (draws) on screen: one per material layer per batch.
    pub shown_meshes: usize,
    pub per_lod: [usize; MAX_LOD as usize + 1],
    pub queued: usize,
    pub in_flight: usize,
    pub cached_batches: usize,
}

#[derive(Resource)]
pub struct TerrainStreamer {
    shared: Arc<Shared>,
    rx: Mutex<Receiver<WorkerMessage>>,
    ranges: LodRanges,
    /// Batches that have an octree file.
    existing: HashSet<BatchCoord>,
    batch_size: [i32; 3],
    shown: HashMap<BatchCoord, Shown>,
    /// Requested and not yet back: coord → level of detail.
    in_flight: HashMap<BatchCoord, u32>,
    desired: HashMap<BatchCoord, u32>,
    last_update_at: Option<Vec3>,
    materials: HashMap<u8, Handle<StandardMaterial>>,
    /// Finished meshes waiting for upload (see UPLOAD_BUDGET).
    ready: VecDeque<Finished>,
    /// Request-to-screen latencies (ms) and pure meshing times (ms), per level.
    pub latencies: Vec<(u32, f32, f32)>,
}

impl TerrainStreamer {
    pub fn start(
        game: GameData,
        ranges: LodRanges,
        blocks: Option<BlockSettings>,
    ) -> Result<Self, String> {
        let index = game.read_index()?;
        let (existing, _) = game.octree_batches()?;
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            game,
            index,
            blocks,
            queue: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            cache: Mutex::new(BatchCache::default()),
            shutdown: AtomicBool::new(false),
        });
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().max(2) - 1);
        for _ in 0..threads {
            let (shared, tx) = (shared.clone(), tx.clone());
            std::thread::spawn(move || worker(shared, tx));
        }
        info!(
            "terrain: {threads} worker threads, {} batch files",
            existing.len()
        );
        Ok(TerrainStreamer {
            batch_size: batch_voxels(&shared.index),
            shared,
            rx: Mutex::new(rx),
            ranges,
            existing: existing.into_iter().collect(),
            shown: HashMap::new(),
            in_flight: HashMap::new(),
            desired: HashMap::new(),
            last_update_at: None,
            materials: HashMap::new(),
            ready: VecDeque::new(),
            latencies: Vec::new(),
        })
    }

    pub fn stats(&self) -> StreamStats {
        let mut stats = StreamStats {
            shown_batches: self.shown.len(),
            queued: self.shared.queue.lock().unwrap().len(),
            in_flight: self.in_flight.len(),
            cached_batches: self.shared.cache.lock().unwrap().entries.len(),
            ..default()
        };
        for shown in self.shown.values() {
            stats.shown_triangles += shown.triangles;
            stats.shown_meshes += shown.entities.len();
            stats.per_lod[shown.lod as usize] += 1;
        }
        stats
    }

    /// Batches on screen and their level of detail.
    pub fn shown_lods(&self) -> impl Iterator<Item = (BatchCoord, u32)> + '_ {
        self.shown.iter().map(|(c, s)| (*c, s.lod))
    }

    /// True when everything wanted at the current position is on screen.
    pub fn settled(&self) -> bool {
        self.last_update_at.is_some()
            && self.shared.queue.lock().unwrap().is_empty()
            && self.in_flight.is_empty()
            && self.ready.is_empty()
            && self
                .desired
                .iter()
                .all(|(c, lod)| self.shown.get(c).is_some_and(|s| s.lod == *lod))
    }

    /// Distance in metres from `eye` (voxel space) to the nearest point of a batch.
    fn distance(&self, eye: [f32; 3], coord: BatchCoord) -> f32 {
        let c = [coord.x, coord.y, coord.z];
        let mut d2 = 0.0;
        for a in 0..3 {
            let lo = (c[a] * self.batch_size[a]) as f32;
            let hi = lo + self.batch_size[a] as f32;
            let d = (lo - eye[a]).max(0.0).max(eye[a] - hi);
            d2 += d * d;
        }
        d2.sqrt()
    }

    fn pick_lod(&self, distance: f32, current: Option<u32>) -> Option<u32> {
        let ranges = self.ranges.0;
        let mut lod = ranges.iter().position(|&r| distance < r)? as u32;
        // Hysteresis: keep a coarser current level until clearly inside the
        // finer range.
        if let Some(cur) = current
            && cur > lod
            && distance > ranges[lod as usize] - HYSTERESIS
        {
            lod = cur;
        }
        Some(lod)
    }

    /// Recomputes which batches should be shown at which level, and requeues work.
    fn update_desired(&mut self, eye: [f32; 3]) {
        let view = *self.ranges.0.last().unwrap();
        let size = self.batch_size;
        let lo = [0, 1, 2].map(|a| ((eye[a] - view) / size[a] as f32).floor() as i32);
        let hi = [0, 1, 2].map(|a| ((eye[a] + view) / size[a] as f32).floor() as i32);
        let mut desired = HashMap::new();
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let c = BatchCoord::new(x, y, z);
                    if !self.existing.contains(&c) {
                        continue;
                    }
                    let current = self.shown.get(&c).map(|s| s.lod);
                    if let Some(lod) = self.pick_lod(self.distance(eye, c), current) {
                        desired.insert(c, lod);
                    }
                }
            }
        }
        self.desired = desired;

        // Rebuild the queue from scratch; jobs still waiting are forgotten.
        let mut queue = self.shared.queue.lock().unwrap();
        // Remember when waiting jobs were first requested, for honest latencies.
        let mut requested_at = HashMap::new();
        for job in queue.drain(..) {
            if self.in_flight.get(&job.coord) == Some(&job.lod) {
                self.in_flight.remove(&job.coord);
            }
            requested_at.insert((job.coord, job.lod), job.requested);
        }
        let now = Instant::now();
        let mut jobs: Vec<(f32, Job)> = Vec::new();
        for (&coord, &lod) in &self.desired {
            let shown = self.shown.get(&coord).map(|s| s.lod);
            if shown == Some(lod) || self.in_flight.contains_key(&coord) {
                continue;
            }
            // Missing batches first, then by distance.
            let distance = self.distance(eye, coord);
            let priority = if shown.is_none() {
                distance
            } else {
                distance + 10_000.0
            };
            let requested = requested_at.get(&(coord, lod)).copied().unwrap_or(now);
            jobs.push((
                priority,
                Job {
                    coord,
                    lod,
                    requested,
                },
            ));
        }
        jobs.sort_by(|a, b| b.0.total_cmp(&a.0)); // most urgent last
        for (_, job) in jobs {
            self.in_flight.insert(job.coord, job.lod);
            queue.push(job);
        }
        drop(queue);
        self.shared.wake.notify_all();
    }
}

impl Drop for TerrainStreamer {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

/// Unity position (left-handed, +Z into the screen) → Bevy (right-handed,
/// -Z into the screen).
pub fn unity_to_bevy(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

/// Bevy position → voxel-index space.
pub fn bevy_to_voxel(p: Vec3) -> [f32; 3] {
    world_to_voxel(unity_to_bevy(p.to_array()))
}

/// Block-type settings the layer builder needs, indexed by octree type id.
#[derive(Clone)]
pub struct BlockSettings {
    /// `VoxelandBlockType.layer`.
    pub layer: [i32; 256],
    /// The material's `_Gloss`.
    pub gloss: [f32; 256],
}

/// The game's chunk size in voxels (`Voxeland.chunkSize` in the main scene);
/// at coarser levels of detail a chunk covers as many coarser cells.
const CHUNK_CELLS: f32 = 16.0;

/// Types drawn per chunk at each level of detail. The game's "high" preset
/// (`clipmaps-high.json`) allows 32 at full resolution, 2 at half, 1 beyond;
/// we map its levels to ours by sample spacing.
const MAX_TYPES: [usize; MAX_LOD as usize + 1] = [32, 2, 1, 1];

/// The game's material layers for a batch mesh (see `sn_mesh::build_layers`).
/// Faces are split into 9 vertices only at the finest level, like the game's
/// high-resolution chunks.
fn layered_parts(mesh: &sn_mesh::Mesh, blocks: &BlockSettings, lod: u32) -> Vec<TerrainPart> {
    let cell = (1u32 << lod) as f32;
    let layers = build_layers(
        mesh,
        &LayerSettings {
            layer: &blocks.layer,
            gloss: &blocks.gloss,
            chunk: CHUNK_CELLS * cell,
            cell,
            subdivide: lod == 0,
            max_types: MAX_TYPES[lod as usize],
        },
    );
    let mut parts: Vec<TerrainPart> = layers
        .into_iter()
        .map(|l| TerrainPart {
            material: l.material,
            rank: l.rank,
            positions: l
                .positions
                .iter()
                .map(|&p| unity_to_bevy(voxel_to_world(p)))
                .collect(),
            normals: l.normals.iter().map(|&n| unity_to_bevy(n)).collect(),
            uvs: l
                .blend
                .iter()
                .zip(&l.gloss)
                .map(|(&b, &g)| [b, g])
                .collect(),
            // Mirroring z flips the winding.
            indices: l
                .triangles
                .iter()
                .flat_map(|t| [t[0], t[2], t[1]])
                .collect(),
        })
        .collect();
    share_bounds(&mut parts);
    parts
}

/// Gives every part the same bounding box (two unused vertices at the
/// corners of the union), so Bevy sorts a batch's blended layers by their
/// depth bias alone.
fn share_bounds(parts: &mut [TerrainPart]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in parts.iter().flat_map(|part| &part.positions) {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    if lo[0] > hi[0] {
        return;
    }
    for part in parts {
        for corner in [lo, hi] {
            part.positions.push(corner);
            part.normals.push([0.0, 1.0, 0.0]);
            part.uvs.push([0.0, 0.0]);
        }
    }
}

/// Debug colours: every triangle in its own material, no blending.
fn split_by_material(mesh: &sn_mesh::Mesh) -> Vec<TerrainPart> {
    let mut parts: HashMap<u8, (TerrainPart, Vec<u32>)> = HashMap::new();
    for (tri, &material) in mesh.triangles.iter().zip(&mesh.triangle_materials) {
        let (part, remap) = parts.entry(material).or_insert_with(|| {
            let part = TerrainPart {
                material,
                rank: 0,
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
            };
            (part, vec![u32::MAX; mesh.positions.len()])
        });
        let [a, b, c] = tri.map(|v| {
            let slot = &mut remap[v as usize];
            if *slot == u32::MAX {
                *slot = part.positions.len() as u32;
                part.positions
                    .push(unity_to_bevy(voxel_to_world(mesh.positions[v as usize])));
                part.normals.push(unity_to_bevy(mesh.normals[v as usize]));
                part.uvs.push([1.0, 0.0]);
            }
            *slot
        });
        part.indices.extend([a, c, b]);
    }
    parts.into_values().map(|(part, _)| part).collect()
}

/// Triangles uploaded to the GPU per frame at most, so a burst of finished
/// batches is spread over several frames instead of causing one long frame.
const UPLOAD_BUDGET: usize = 300_000;

/// Moves this far before the wanted set is recomputed.
const UPDATE_DISTANCE: f32 = 8.0;

/// Every frame: refresh what should be loaded, spawn finished meshes, unload
/// batches that left the view.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub fn stream(
    mut commands: Commands,
    mut streamer: ResMut<TerrainStreamer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut look: Option<ResMut<TerrainLook>>,
    camera: Query<&Transform, With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(camera) = camera.single() else { return };
    let moved = streamer
        .last_update_at
        .is_none_or(|last| last.distance(camera.translation) > UPDATE_DISTANCE);
    if moved {
        streamer.last_update_at = Some(camera.translation);
        streamer.update_desired(bevy_to_voxel(camera.translation));
    }

    // Unload batches that are no longer wanted.
    let gone: Vec<BatchCoord> = streamer
        .shown
        .keys()
        .filter(|c| !streamer.desired.contains_key(c))
        .copied()
        .collect();
    for coord in gone {
        if let Some(shown) = streamer.shown.remove(&coord) {
            for entity in shown.entities {
                commands.entity(entity).despawn();
            }
        }
    }

    let messages: Vec<WorkerMessage> = streamer.rx.lock().unwrap().try_iter().collect();
    for message in messages {
        match message {
            WorkerMessage::Error(e) => {
                error!("terrain streaming failed: {e}");
                exit.write(AppExit::error());
            }
            WorkerMessage::Finished(done) => streamer.ready.push_back(done),
        }
    }

    let mut uploaded = 0;
    while uploaded < UPLOAD_BUDGET {
        let Some(done) = streamer.ready.pop_front() else {
            break;
        };
        let job = done.job;
        if streamer.in_flight.get(&job.coord) == Some(&job.lod) {
            streamer.in_flight.remove(&job.coord);
        }
        if streamer.desired.get(&job.coord) != Some(&job.lod) {
            continue; // stale: the camera moved on
        }
        // Swap: the old mesh stays until the new one is ready, so no holes.
        if let Some(old) = streamer.shown.remove(&job.coord) {
            for entity in old.entities {
                commands.entity(entity).despawn();
            }
        }
        let mut entities = Vec::new();
        let mut triangles = 0;
        uploaded += done
            .parts
            .iter()
            .map(|p| p.indices.len() / 3)
            .sum::<usize>();
        for part in done.parts {
            triangles += part.indices.len() / 3;
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, part.normals);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs);
            mesh.insert_indices(Indices::U32(part.indices));
            let mesh = Mesh3d(meshes.add(mesh));
            // The game's material if we have one, else a debug colour (for
            // the opaque layer only: a blended layer can't fall back).
            let real = look
                .as_mut()
                .and_then(|l| l.material(part.material, part.rank, &mut terrain_materials));
            let entity = match real {
                Some(material) => commands.spawn((mesh, MeshMaterial3d(material))).id(),
                None if part.rank > 0 => continue,
                None => {
                    let material = streamer
                        .materials
                        .entry(part.material)
                        .or_insert_with(|| {
                            let [r, g, b] = debug_colour(part.material);
                            materials.add(StandardMaterial {
                                base_color: Color::srgb(r, g, b),
                                perceptual_roughness: 0.9,
                                ..default()
                            })
                        })
                        .clone();
                    commands.spawn((mesh, MeshMaterial3d(material))).id()
                }
            };
            // The game's shadows reach 50 m (`QualitySettings`, High): only
            // the nearest level of detail's opaque layer can cast into them.
            if job.lod > 0 || part.rank > 0 {
                commands.entity(entity).insert(bevy::light::NotShadowCaster);
            }
            entities.push(entity);
        }
        let latency = job.requested.elapsed().as_secs_f32() * 1000.0;
        streamer.latencies.push((job.lod, latency, done.mesh_ms));
        streamer.shown.insert(
            job.coord,
            Shown {
                lod: job.lod,
                entities,
                triangles,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two quads side by side in the XZ plane, facing +Y, types 1 and 2.
    fn two_quads() -> sn_mesh::Mesh {
        sn_mesh::Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 1.0],
                [2.0, 0.0, 0.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 6],
            triangles: vec![[0, 1, 2], [0, 2, 3], [3, 2, 4], [3, 4, 5]],
            triangle_materials: vec![1, 1, 2, 2],
        }
    }

    fn blocks() -> BlockSettings {
        BlockSettings {
            layer: [0; 256],
            gloss: [0.25; 256],
        }
    }

    #[test]
    fn layers_share_one_bounding_box() {
        let parts = layered_parts(&two_quads(), &blocks(), 1);
        let ranks: Vec<(u8, u8)> = parts.iter().map(|p| (p.material, p.rank)).collect();
        assert_eq!(ranks, vec![(1, 0), (2, 1)]);
        let bounds = |p: &TerrainPart| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for v in &p.positions {
                for a in 0..3 {
                    lo[a] = lo[a].min(v[a]);
                    hi[a] = hi[a].max(v[a]);
                }
            }
            (lo, hi)
        };
        assert_eq!(bounds(&parts[0]), bounds(&parts[1]));
        // The extra corners are not used by any triangle.
        for p in &parts {
            let used = p.indices.iter().max().copied().unwrap_or(0) as usize;
            assert!(used < p.positions.len() - 2);
        }
    }

    #[test]
    fn uvs_carry_weight_and_gloss() {
        let parts = layered_parts(&two_quads(), &blocks(), 1);
        assert!(
            parts[0]
                .uvs
                .iter()
                .rev()
                .skip(2)
                .all(|uv| *uv == [1.0, 0.25])
        );
        // The shared edge touches one face of each type.
        let overlay = &parts[1];
        let mut weights: Vec<f32> = overlay.uvs.iter().rev().skip(2).map(|uv| uv[0]).collect();
        weights.sort_by(f32::total_cmp);
        assert_eq!(weights, vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn mirroring_reverses_the_winding() {
        let parts = layered_parts(&two_quads(), &blocks(), 1);
        // Seen from above (+Y), Bevy's triangles must stay counter-clockwise
        // in its right-handed coordinates after z is flipped.
        for part in &parts {
            for t in part.indices.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(part.positions[i as usize]));
                assert!((b - a).cross(c - a).y > 0.0);
            }
        }
    }

    #[test]
    fn debug_colours_split_by_triangle_material() {
        let parts = split_by_material(&two_quads());
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.rank == 0 && p.indices.len() == 6));
    }
}
