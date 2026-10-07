//! World objects (coral, rocks, plants, wrecks): placements from the game's
//! baked cells (`CellsCache`), drawn with the terrain batches.
//!
//! A worker thread reads a batch's cells, loads each placed prefab once
//! (meshes, materials, textures) and sends everything new to the main
//! thread, followed by the batch's instances. The main thread uploads assets
//! once and spawns one entity per placed mesh part. Cell level *n* is shown
//! while its batch's terrain level of detail is at most *n* (our choice; see
//! `docs/DESIGN.md`, M7c).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};
use sn_assets::{Assets as GameAssets, ObjectRef, TerrainTexture};
use sn_install::GameData;
use sn_unity::{Catalog, Material};
use sn_world::{BatchCoord, Transform as Placement};

use crate::object_look::{ObjectExtension, ObjectMaterial, ObjectParams};
use crate::terrain::TerrainStreamer;
use crate::textures::{linear, to_image};

/// Cell levels in the game's data (0 = small, near objects … 3 = far).
const LEVELS: usize = 4;

/// Entities spawned per frame at most, so a burst of finished batches is
/// spread over several frames.
const SPAWN_BUDGET: usize = 20_000;

/// The worker forgets loaded bundles beyond this many bytes.
const BUNDLE_CACHE_BYTES: usize = 512 << 20;

/// Whether cell `level` is shown for a batch at terrain level of detail `lod`.
fn shows(level: usize, lod: u32) -> bool {
    lod as usize <= level
}

type Key = (PathBuf, String, i64);

/// Mesh data already in Bevy's coordinates (z flipped, winding reversed).
pub struct MeshData {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

#[derive(Clone, Copy)]
enum Alpha {
    Opaque,
    Mask(f32),
    Blend,
}

struct MaterialDesc {
    albedo: Option<u32>,
    normal: Option<u32>,
    /// As stored (sRGB).
    color: [f32; 4],
    /// uv × (x, y) + (z, w).
    albedo_st: [f32; 4],
    normal_st: [f32; 4],
    alpha: Alpha,
    double_sided: bool,
}

/// One mesh part of a prefab: a sub-mesh with its material, placed relative
/// to the prefab root.
#[derive(Clone, Copy)]
struct Part {
    mesh: u32,
    material: u32,
    local: Placement,
}

#[derive(Clone, Copy)]
struct Instance {
    level: usize,
    prefab: u32,
    transform: Placement,
}

enum Update {
    Texture {
        id: u32,
        texture: TerrainTexture,
        srgb: bool,
    },
    Material {
        id: u32,
        desc: MaterialDesc,
    },
    Mesh {
        id: u32,
        data: MeshData,
    },
    Prefab {
        id: u32,
        parts: Vec<Part>,
    },
    Batch {
        coord: BatchCoord,
        instances: Vec<Instance>,
        ms: f32,
    },
    Ready {
        ms: f32,
    },
    Warning(String),
    Error(String),
}

/// What the worker knows: the game's assets and everything already sent.
struct Library {
    assets: GameAssets<'static>,
    catalog: Catalog,
    /// ClassId → prefab path (`prefabs.db`).
    class_paths: HashMap<String, String>,
    /// Prefab path → id (`None`: nothing to draw, or failed to load).
    prefabs: HashMap<String, Option<u32>>,
    meshes: HashMap<(Key, usize), Option<u32>>,
    materials: HashMap<Key, u32>,
    textures: HashMap<(Key, bool), Option<u32>>,
    next_id: u32,
}

impl Library {
    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn texture(&mut self, object: &ObjectRef, srgb: bool, out: &mut Vec<Update>) -> Option<u32> {
        let key = (object.key(), srgb);
        if let Some(id) = self.textures.get(&key) {
            return *id;
        }
        let id = match self.assets.texture(object) {
            Ok(texture) => {
                let id = self.id();
                out.push(Update::Texture { id, texture, srgb });
                Some(id)
            }
            Err(e) => {
                out.push(Update::Warning(format!("texture: {e}")));
                None
            }
        };
        self.textures.insert(key, id);
        id
    }

    fn material(&mut self, object: &ObjectRef, out: &mut Vec<Update>) -> Option<u32> {
        if let Some(id) = self.materials.get(&object.key()) {
            return Some(*id);
        }
        let (_, data) = object.data().ok()?;
        let material = Material::parse(data, object.file.file().big_endian).ok()?;
        let slot = |name: &str| {
            material.texture(name).map(|t| {
                (
                    t.texture,
                    [t.scale[0], t.scale[1], t.offset[0], t.offset[1]],
                )
            })
        };
        let mut texture = |name: &str, srgb: bool, out: &mut Vec<Update>| {
            let (pptr, st) = slot(name)?;
            let target = self.assets.resolve(&object.file, pptr).ok()??;
            Some((self.texture(&target, srgb, out)?, st))
        };
        let albedo = texture("_MainTex", true, out);
        let normal = texture("_BumpMap", false, out);
        let keywords: Vec<&str> = material.keywords.split_whitespace().collect();
        let has = |k: &str| keywords.contains(&k);
        let alpha = if has("MARMO_ALPHA_CLIP") {
            Alpha::Mask(material.float("_Cutoff").unwrap_or(0.5))
        } else if material.custom_render_queue >= 2500
            || has("MARMO_ALPHA")
            || has("_ALPHAPREMULTIPLY_ON")
            || has("WBOIT")
        {
            Alpha::Blend
        } else {
            Alpha::Opaque
        };
        let desc = MaterialDesc {
            albedo: albedo.map(|(id, _)| id),
            normal: normal.map(|(id, _)| id),
            color: material.color("_Color").unwrap_or([1.0; 4]),
            albedo_st: albedo.map_or([1.0, 1.0, 0.0, 0.0], |(_, st)| st),
            normal_st: normal.map_or([1.0, 1.0, 0.0, 0.0], |(_, st)| st),
            alpha,
            // `_MyCullVariable`: 0 = two-sided, 2 = back faces culled.
            double_sided: material.float("_MyCullVariable") == Some(0.0),
        };
        let id = self.id();
        out.push(Update::Material { id, desc });
        self.materials.insert(object.key(), id);
        Some(id)
    }

    /// Ids of the mesh's sub-meshes (`None` for sub-meshes that failed).
    fn mesh(&mut self, object: &ObjectRef, out: &mut Vec<Update>) -> Vec<Option<u32>> {
        let key = object.key();
        if self.meshes.contains_key(&(key.clone(), 0)) {
            return (0..)
                .map_while(|i| self.meshes.get(&(key.clone(), i)).copied())
                .collect();
        }
        let geometry = match self.assets.mesh(object) {
            Ok((_, g)) => g,
            Err(e) => {
                out.push(Update::Warning(format!("mesh: {e}")));
                self.meshes.insert((key, 0), None);
                return vec![None];
            }
        };
        let flip = |p: [f32; 3]| [p[0], p[1], -p[2]];
        let positions: Vec<[f32; 3]> = geometry.positions.iter().map(|&p| flip(p)).collect();
        let normals: Vec<[f32; 3]> = geometry.normals.iter().map(|&n| flip(n)).collect();
        // Mirroring flips the bitangent: negate w too.
        let tangents: Vec<[f32; 4]> = geometry
            .tangents
            .iter()
            .map(|t| [t[0], t[1], -t[2], -t[3]])
            .collect();
        let mut ids = Vec::new();
        for (i, indices) in geometry.sub_meshes.iter().enumerate() {
            // Each sub-mesh becomes its own mesh with only the vertices it
            // uses.
            let mut remap = vec![u32::MAX; positions.len()];
            let mut used = Vec::new();
            let mut local = Vec::with_capacity(indices.len());
            for &v in indices {
                let slot = &mut remap[v as usize];
                if *slot == u32::MAX {
                    *slot = used.len() as u32;
                    used.push(v as usize);
                }
                local.push(*slot);
            }
            let pick = |attr: &[[f32; 3]]| -> Vec<[f32; 3]> {
                if attr.len() == positions.len() {
                    used.iter().map(|&v| attr[v]).collect()
                } else {
                    Vec::new()
                }
            };
            let id = self.id();
            out.push(Update::Mesh {
                id,
                data: MeshData {
                    positions: pick(&positions),
                    normals: pick(&normals),
                    tangents: if tangents.len() == positions.len() {
                        used.iter().map(|&v| tangents[v]).collect()
                    } else {
                        Vec::new()
                    },
                    uvs: if geometry.uv0.len() == positions.len() {
                        used.iter().map(|&v| geometry.uv0[v]).collect()
                    } else {
                        Vec::new()
                    },
                    indices: local
                        .chunks_exact(3)
                        .flat_map(|t| [t[0], t[2], t[1]])
                        .collect(),
                },
            });
            self.meshes.insert((key.clone(), i), Some(id));
            ids.push(Some(id));
        }
        ids
    }

    fn prefab(&mut self, path: &str, out: &mut Vec<Update>) -> Option<u32> {
        if let Some(id) = self.prefabs.get(path) {
            return *id;
        }
        let prefab = match self.assets.prefab(&self.catalog, path) {
            Ok(p) => p,
            Err(e) => {
                out.push(Update::Warning(format!("prefab: {e}")));
                self.prefabs.insert(path.to_string(), None);
                return None;
            }
        };
        let mut parts = Vec::new();
        for node in prefab.visible_nodes() {
            let Some(mesh) = &node.mesh else { continue };
            let sub_meshes = self.mesh(mesh, out);
            // One material per sub-mesh; extra materials (multi-pass) are
            // not drawn, sub-meshes without a material neither.
            for (sub, material) in sub_meshes.iter().zip(&node.materials) {
                let (Some(mesh), Some(material)) = (sub, material) else {
                    continue;
                };
                let Some(material) = self.material(material, out) else {
                    continue;
                };
                parts.push(Part {
                    mesh: *mesh,
                    material,
                    local: node.in_prefab,
                });
            }
        }
        let id = (!parts.is_empty()).then(|| self.id());
        if let Some(id) = id {
            out.push(Update::Prefab { id, parts });
        }
        self.prefabs.insert(path.to_string(), id);
        id
    }

    fn batch(&mut self, game: &GameData, coord: BatchCoord, out: &mut Vec<Update>) {
        let start = Instant::now();
        let mut instances = Vec::new();
        match game.read_batch_cells(coord) {
            Ok(Some(file)) => {
                for cell in &file.cells {
                    let Some(tree) = &cell.objects else { continue };
                    let (world, _) = tree.world_transforms();
                    for (object, transform) in tree.objects.iter().zip(world) {
                        let Some(path) = self.class_paths.get(&object.class_id).cloned() else {
                            continue;
                        };
                        // Creatures move and animate; not drawn as still
                        // objects (a later milestone).
                        if path.starts_with("WorldEntities/Creatures/") {
                            continue;
                        }
                        if let Some(prefab) = self.prefab(&path, out) {
                            instances.push(Instance {
                                level: (cell.level as usize).min(LEVELS - 1),
                                prefab,
                                transform,
                            });
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => out.push(Update::Warning(e.0)),
        }
        // Loaded bundles are only needed while new prefabs are read.
        self.assets.trim_cache(BUNDLE_CACHE_BYTES);
        out.push(Update::Batch {
            coord,
            instances,
            ms: start.elapsed().as_secs_f32() * 1000.0,
        });
    }
}

struct Shared {
    /// Batches to load, most urgent last.
    queue: Mutex<Vec<BatchCoord>>,
    wake: Condvar,
    shutdown: AtomicBool,
}

fn worker(game: &'static GameData, shared: Arc<Shared>, tx: Sender<Update>) {
    let start = Instant::now();
    let setup = || -> Result<Library, String> {
        let assets = GameAssets::index(game)?;
        let catalog = assets.catalog()?;
        let class_paths = game.read_prefab_database().map_err(|e| e.0)?;
        Ok(Library {
            assets,
            catalog,
            class_paths,
            prefabs: HashMap::new(),
            meshes: HashMap::new(),
            materials: HashMap::new(),
            textures: HashMap::new(),
            next_id: 0,
        })
    };
    let mut library = match setup() {
        Ok(l) => l,
        Err(e) => {
            let _ = tx.send(Update::Error(e));
            return;
        }
    };
    let _ = tx.send(Update::Ready {
        ms: start.elapsed().as_secs_f32() * 1000.0,
    });
    loop {
        let coord = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(c) = queue.pop() {
                    break c;
                }
                queue = shared.wake.wait(queue).unwrap();
            }
        };
        let mut out = Vec::new();
        library.batch(game, coord, &mut out);
        for update in out {
            if tx.send(update).is_err() {
                return;
            }
        }
    }
}

/// A batch's objects on the main thread.
#[derive(Default)]
struct BatchObjects {
    /// `None` until the worker has sent them.
    instances: Option<Vec<Instance>>,
    /// Spawned entities per cell level, `None` when not spawned.
    shown: [Option<Vec<Entity>>; LEVELS],
}

#[derive(Default, Clone, Copy)]
pub struct ObjectStats {
    pub batches: usize,
    pub entities: usize,
    pub per_level: [usize; LEVELS],
    pub queued: usize,
    pub prefabs: usize,
    pub meshes: usize,
    pub materials: usize,
    pub textures: usize,
    pub warnings: usize,
}

#[derive(Resource)]
pub struct ObjectStreamer {
    shared: Arc<Shared>,
    rx: Mutex<Receiver<Update>>,
    ready: bool,
    textures: HashMap<u32, Handle<Image>>,
    materials: HashMap<u32, Handle<ObjectMaterial>>,
    meshes: HashMap<u32, Handle<Mesh>>,
    prefabs: HashMap<u32, Vec<Part>>,
    batches: HashMap<BatchCoord, BatchObjects>,
    /// What was last put in the worker's queue.
    requested: Vec<BatchCoord>,
    flat_normal: Option<Handle<Image>>,
    warnings: usize,
    /// Worker time per batch (ms).
    pub batch_ms: Vec<f32>,
}

impl ObjectStreamer {
    pub fn start(game: GameData) -> ObjectStreamer {
        // The worker's asset index borrows the install for the whole run.
        let game: &'static GameData = Box::leak(Box::new(game));
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let worker_shared = shared.clone();
        std::thread::spawn(move || worker(game, worker_shared, tx));
        ObjectStreamer {
            shared,
            rx: Mutex::new(rx),
            ready: false,
            textures: HashMap::new(),
            materials: HashMap::new(),
            meshes: HashMap::new(),
            prefabs: HashMap::new(),
            batches: HashMap::new(),
            requested: Vec::new(),
            flat_normal: None,
            warnings: 0,
            batch_ms: Vec::new(),
        }
    }

    pub fn stats(&self) -> ObjectStats {
        let mut s = ObjectStats {
            batches: self.batches.len(),
            queued: self.shared.queue.lock().unwrap().len(),
            prefabs: self.prefabs.len(),
            meshes: self.meshes.len(),
            materials: self.materials.len(),
            textures: self.textures.len(),
            warnings: self.warnings,
            ..default()
        };
        for b in self.batches.values() {
            for (level, shown) in b.shown.iter().enumerate() {
                if let Some(e) = shown {
                    s.entities += e.len();
                    s.per_level[level] += e.len();
                }
            }
        }
        s
    }

    /// True when every wanted batch has its objects on screen.
    pub fn settled(&self, terrain: &TerrainStreamer) -> bool {
        self.ready
            && terrain.shown_lods().all(|(coord, lod)| {
                self.batches.get(&coord).is_some_and(|b| {
                    b.instances.is_some()
                        && (0..LEVELS).all(|l| b.shown[l].is_some() == shows(l, lod))
                })
            })
    }
}

impl Drop for ObjectStreamer {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

fn to_bevy(t: &Placement) -> Transform {
    let [x, y, z] = t.position;
    let [qx, qy, qz, qw] = t.rotation;
    Transform {
        translation: Vec3::new(x, y, -z),
        // Mirroring z: conjugate the rotation by the mirror.
        rotation: Quat::from_xyzw(-qx, -qy, qz, qw).normalize(),
        scale: Vec3::from(t.scale),
    }
}

fn build_mesh(data: MeshData) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let n = data.positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
    let has_normals = data.normals.len() == n;
    if has_normals {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals);
    }
    if data.uvs.len() == n {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs);
        if data.tangents.len() == n {
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, data.tangents);
        }
    }
    mesh.insert_indices(Indices::U32(data.indices));
    if !has_normals {
        mesh.compute_smooth_normals();
    }
    mesh
}

/// Every frame: take what the worker sent, follow the terrain's batches,
/// spawn and despawn objects.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub fn stream_objects(
    mut commands: Commands,
    mut streamer: ResMut<ObjectStreamer>,
    terrain: Res<TerrainStreamer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ObjectMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut exit: MessageWriter<AppExit>,
) {
    let streamer = &mut *streamer;
    let flat = streamer
        .flat_normal
        .get_or_insert_with(|| {
            // (0.5, 0.5, 1, 1) in the game's packing: x = a·r, y = g → flat.
            images.add(Image::new(
                Extent3d::default(),
                TextureDimension::D2,
                vec![255, 128, 255, 128],
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            ))
        })
        .clone();

    let updates: Vec<Update> = streamer.rx.lock().unwrap().try_iter().collect();
    for update in updates {
        match update {
            Update::Ready { ms } => {
                info!("objects: asset index ready after {ms:.0} ms");
                streamer.ready = true;
            }
            Update::Texture { id, texture, srgb } => {
                if let Some(image) = to_image(&texture, srgb) {
                    streamer.textures.insert(id, images.add(image));
                }
            }
            Update::Material { id, desc } => {
                let texture = |t: Option<u32>| t.and_then(|t| streamer.textures.get(&t).cloned());
                let albedo = texture(desc.albedo);
                let normal = texture(desc.normal);
                let [sx, sy, ox, oy] = desc.albedo_st;
                let c = linear(desc.color);
                let base = StandardMaterial {
                    base_color: Color::linear_rgba(c.x, c.y, c.z, c.w),
                    base_color_texture: albedo,
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::new(sx, sy),
                        0.0,
                        Vec2::new(ox, oy),
                    ),
                    perceptual_roughness: 0.8,
                    alpha_mode: match desc.alpha {
                        Alpha::Opaque => AlphaMode::Opaque,
                        Alpha::Mask(cutoff) => AlphaMode::Mask(cutoff),
                        Alpha::Blend => AlphaMode::Blend,
                    },
                    double_sided: desc.double_sided,
                    cull_mode: if desc.double_sided {
                        None
                    } else {
                        Some(Face::Back)
                    },
                    ..default()
                };
                let handle = materials.add(ObjectMaterial {
                    base,
                    extension: ObjectExtension {
                        params: ObjectParams {
                            normal_st: Vec4::from(desc.normal_st),
                            has_normal: u32::from(normal.is_some()),
                        },
                        normal_map: normal.unwrap_or_else(|| flat.clone()),
                    },
                });
                streamer.materials.insert(id, handle);
            }
            Update::Mesh { id, data } => {
                streamer.meshes.insert(id, meshes.add(build_mesh(data)));
            }
            Update::Prefab { id, parts } => {
                streamer.prefabs.insert(id, parts);
            }
            Update::Batch {
                coord,
                instances,
                ms,
            } => {
                streamer.batch_ms.push(ms);
                if let Some(b) = streamer.batches.get_mut(&coord) {
                    b.instances = Some(instances);
                }
            }
            Update::Warning(w) => {
                streamer.warnings += 1;
                if streamer.warnings <= 20 {
                    warn!("objects: {w}");
                }
            }
            Update::Error(e) => {
                error!("objects: {e}");
                exit.write(AppExit::error());
            }
        }
    }

    // Follow the terrain: batches on screen, at their level of detail.
    let wanted: HashMap<BatchCoord, u32> = terrain.shown_lods().collect();
    let gone: Vec<BatchCoord> = streamer
        .batches
        .keys()
        .filter(|c| !wanted.contains_key(c))
        .copied()
        .collect();
    for coord in gone {
        if let Some(b) = streamer.batches.remove(&coord) {
            for entity in b.shown.into_iter().flatten().flatten() {
                commands.entity(entity).despawn();
            }
        }
    }
    let mut missing: Vec<(u32, BatchCoord)> = Vec::new();
    for (&coord, &lod) in &wanted {
        let b = streamer.batches.entry(coord).or_default();
        if b.instances.is_none() {
            missing.push((lod, coord));
        }
    }
    // Requeue only when the set changed: nearest (finest) batches first.
    missing.sort_by(|a, b| b.cmp(a));
    let order: Vec<BatchCoord> = missing.iter().map(|(_, c)| *c).collect();
    let pending: HashSet<BatchCoord> = order.iter().copied().collect();
    let requested: HashSet<BatchCoord> = streamer.requested.iter().copied().collect();
    if pending != requested {
        let mut queue = streamer.shared.queue.lock().unwrap();
        *queue = order.clone();
        drop(queue);
        streamer.shared.wake.notify_all();
        streamer.requested = order;
    }

    // Spawn and despawn per cell level.
    let mut budget = SPAWN_BUDGET;
    let streamer = &mut *streamer;
    for (&coord, &lod) in &wanted {
        let Some(b) = streamer.batches.get_mut(&coord) else {
            continue;
        };
        let Some(instances) = &b.instances else {
            continue;
        };
        for level in 0..LEVELS {
            let want = shows(level, lod);
            match (&b.shown[level], want) {
                (Some(_), false) => {
                    for entity in b.shown[level].take().into_iter().flatten() {
                        commands.entity(entity).despawn();
                    }
                }
                (None, true) if budget > 0 => {
                    let mut entities = Vec::new();
                    for inst in instances.iter().filter(|i| i.level == level) {
                        let Some(parts) = streamer.prefabs.get(&inst.prefab) else {
                            continue;
                        };
                        for part in parts {
                            let (Some(mesh), Some(material)) = (
                                streamer.meshes.get(&part.mesh),
                                streamer.materials.get(&part.material),
                            ) else {
                                continue;
                            };
                            let world = inst.transform.then(&part.local);
                            entities.push(
                                commands
                                    .spawn((
                                        Mesh3d(mesh.clone()),
                                        MeshMaterial3d(material.clone()),
                                        to_bevy(&world),
                                    ))
                                    .id(),
                            );
                        }
                    }
                    budget = budget.saturating_sub(entities.len().max(1));
                    b.shown[level] = Some(entities);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_transforms_match_mirrored_points() {
        // A Unity transform applied to a point, then mirrored, must equal the
        // Bevy transform applied to the mirrored point.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let unity = Placement {
            position: [1.0, 2.0, 3.0],
            rotation: [0.0, s, 0.0, s],
            scale: [2.0, 1.0, 0.5],
        };
        let p = [0.3, -0.7, 1.1];
        let placed = unity.then(&Placement {
            position: p,
            ..Placement::default()
        });
        let expected = Vec3::new(placed.position[0], placed.position[1], -placed.position[2]);
        let got = to_bevy(&unity).transform_point(Vec3::new(p[0], p[1], -p[2]));
        assert!((got - expected).length() < 1e-5, "{got} vs {expected}");
    }

    #[test]
    fn levels_follow_the_terrain_detail() {
        assert!(shows(0, 0) && shows(3, 0) && shows(3, 3));
        assert!(!shows(0, 1) && !shows(2, 3));
    }
}
