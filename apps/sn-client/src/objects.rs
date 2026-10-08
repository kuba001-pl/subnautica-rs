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
use sn_assets::{Assets as GameAssets, MarmoSkies, ObjectRef, TerrainTexture, marmo_skies};
use sn_install::GameData;
use sn_unity::{Catalog, Material, SKIES_AUTO};
use sn_world::{BatchCoord, Transform as Placement};

use crate::game_light::GameLightImages;
use crate::object_look::{ObjectExtension, ObjectMaterial, ObjectParams};
use crate::terrain::TerrainStreamer;
use crate::textures::{linear, to_image};
use crate::water::WaterWorld;

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

#[derive(Clone)]
struct MaterialDesc {
    albedo: Option<u32>,
    normal: Option<u32>,
    spec: Option<u32>,
    illum: Option<u32>,
    /// As stored (sRGB).
    color: [f32; 4],
    /// uv × (x, y) + (z, w).
    albedo_st: [f32; 4],
    normal_st: [f32; 4],
    spec_st: [f32; 4],
    illum_st: [f32; 4],
    alpha: Alpha,
    double_sided: bool,
    /// MarmosetUBER's values; `None` for materials of other shaders.
    uber: Option<UberValues>,
}

/// A MarmosetUBER material's values (the shader's defaults where the
/// material has none). Colours as stored (sRGB).
#[derive(Clone, Copy)]
struct UberValues {
    spec_color: [f32; 4],
    spec_int: f32,
    shininess: f32,
    fresnel: f32,
    glow_color: [f32; 4],
    glow_strength: f32,
    glow_strength_night: f32,
    emission_lm: f32,
    emission_lm_night: f32,
    ibl_reduction_at_night: f32,
    simple_glass: f32,
}

/// What a material needs from its Marmoset sky.
#[derive(Clone, Copy)]
struct SkyLook {
    exposure: [f32; 4],
    rotation: [f32; 4],
    affected: bool,
    outdoors: bool,
    sh: [[f32; 3]; 9],
}

/// The skies objects are lit with, as the main thread needs them.
#[derive(Default)]
struct SkySet {
    looks: Vec<SkyLook>,
    skies: MarmoSkies,
}

impl SkySet {
    fn new(skies: MarmoSkies) -> SkySet {
        let looks = skies
            .skies
            .iter()
            .map(|s| SkyLook {
                exposure: s.sky.exposure(),
                rotation: s.rotation,
                affected: s.sky.affected_by_day_night,
                outdoors: s.sky.outdoors,
                sh: s.sky.sh_buffer(),
            })
            .collect();
        SkySet { looks, skies }
    }

    /// The sky of a part: the biome's at the object for parts a
    /// `SkyApplier` covers, else the global one (`None`: no sky known).
    fn pick(&self, biome_sky: bool, biome: Option<&str>) -> Option<usize> {
        if biome_sky {
            self.skies.for_biome(biome).or(self.skies.global)
        } else {
            self.skies.global
        }
    }
}

/// One mesh part of a prefab: a sub-mesh with its material, placed relative
/// to the prefab root.
#[derive(Clone, Copy)]
struct Part {
    mesh: u32,
    material: u32,
    local: Placement,
    /// Lit with the sky of the biome it stands in (a `SkyApplier` lists
    /// it); else with the global sky.
    biome_sky: bool,
}

#[derive(Clone, Copy)]
struct Instance {
    level: usize,
    prefab: u32,
    transform: Placement,
}

enum Update {
    Skies(SkySet),
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
    /// `None`: the texture's own colour space.
    textures: HashMap<(Key, Option<bool>), Option<u32>>,
    next_id: u32,
}

impl Library {
    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// A texture sampled as sRGB or linear (`None`: as its colour space
    /// says).
    fn texture(
        &mut self,
        object: &ObjectRef,
        srgb: Option<bool>,
        out: &mut Vec<Update>,
    ) -> Option<u32> {
        let key = (object.key(), srgb);
        if let Some(id) = self.textures.get(&key) {
            return *id;
        }
        let id = match self.assets.texture(object) {
            Ok(texture) => {
                let id = self.id();
                let srgb = srgb.unwrap_or(texture.texture.color_space == 1);
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
        let mut texture = |name: &str, srgb: Option<bool>, out: &mut Vec<Update>| {
            let (pptr, st) = slot(name)?;
            let target = self.assets.resolve(&object.file, pptr).ok()??;
            Some((self.texture(&target, srgb, out)?, st))
        };
        let keywords: Vec<&str> = material.keywords.split_whitespace().collect();
        let has = |k: &str| keywords.contains(&k);
        // MarmosetUBER (the game's object shader) has these two properties;
        // its specular and glow maps are used with these keywords only.
        let is_uber = material.float("_Shininess").is_some()
            && material.float("_GlowStrengthNight").is_some();
        let albedo = texture("_MainTex", Some(true), out);
        let normal = texture("_BumpMap", Some(false), out);
        let spec = (is_uber && has("MARMO_SPECMAP"))
            .then(|| texture("_SpecTex", None, out))
            .flatten();
        let illum = (is_uber && has("MARMO_EMISSION"))
            .then(|| texture("_Illum", None, out))
            .flatten();
        let value = |name: &str, default: f32| material.float(name).unwrap_or(default);
        let uber = is_uber.then(|| UberValues {
            spec_color: material.color("_SpecColor").unwrap_or([1.0; 4]),
            spec_int: value("_SpecInt", 1.0),
            shininess: value("_Shininess", 4.0),
            fresnel: value("_Fresnel", 0.0),
            glow_color: material.color("_GlowColor").unwrap_or([1.0; 4]),
            glow_strength: value("_GlowStrength", 1.0),
            glow_strength_night: value("_GlowStrengthNight", 1.0),
            emission_lm: value("_EmissionLM", 0.0),
            emission_lm_night: value("_EmissionLMNight", 0.0),
            ibl_reduction_at_night: value("_IBLreductionAtNight", 0.99),
            simple_glass: value("_EnableSimpleGlass", 0.0),
        });
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
        let st = |t: Option<(u32, [f32; 4])>| t.map_or([1.0, 1.0, 0.0, 0.0], |(_, st)| st);
        let desc = MaterialDesc {
            albedo: albedo.map(|(id, _)| id),
            normal: normal.map(|(id, _)| id),
            spec: spec.map(|(id, _)| id),
            illum: illum.map(|(id, _)| id),
            color: material.color("_Color").unwrap_or([1.0; 4]),
            albedo_st: st(albedo),
            normal_st: st(normal),
            spec_st: st(spec),
            illum_st: st(illum),
            uber,
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
            // Other anchors (a fixed sky) are taken as the global sky.
            let biome_sky = node.sky_applier == Some(SKIES_AUTO);
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
                    biome_sky,
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
    let skies = match marmo_skies(&library.assets) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Update::Warning(format!("skies: {e}")));
            MarmoSkies::default()
        }
    };
    let _ = tx.send(Update::Skies(SkySet::new(skies)));
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
    /// Whether its entities cast sun shadows (only the nearest level of
    /// detail: the game's shadows reach 50 m).
    casting: bool,
}

#[derive(Default, Clone)]
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
    /// MarmosetUBER materials, those with a specular map, with a glow map.
    pub uber: [usize; 3],
    /// Materials made per sky (a material is made once per sky it's lit by),
    /// by sky name.
    pub per_sky: Vec<(String, usize)>,
}

#[derive(Resource)]
pub struct ObjectStreamer {
    shared: Arc<Shared>,
    rx: Mutex<Receiver<Update>>,
    ready: bool,
    textures: HashMap<u32, Handle<Image>>,
    /// Materials as the worker described them, made per sky on first use.
    material_descs: HashMap<u32, MaterialDesc>,
    /// (material, sky) → material.
    materials: HashMap<(u32, Option<usize>), Handle<ObjectMaterial>>,
    skies: SkySet,
    meshes: HashMap<u32, Handle<Mesh>>,
    prefabs: HashMap<u32, Vec<Part>>,
    batches: HashMap<BatchCoord, BatchObjects>,
    /// What was last put in the worker's queue.
    requested: Vec<BatchCoord>,
    defaults: Option<Defaults>,
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
            material_descs: HashMap::new(),
            materials: HashMap::new(),
            skies: SkySet::default(),
            meshes: HashMap::new(),
            prefabs: HashMap::new(),
            batches: HashMap::new(),
            requested: Vec::new(),
            defaults: None,
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
            materials: self.material_descs.len(),
            textures: self.textures.len(),
            warnings: self.warnings,
            ..default()
        };
        for d in self.material_descs.values().filter(|d| d.uber.is_some()) {
            s.uber[0] += 1;
            s.uber[1] += usize::from(d.spec.is_some());
            s.uber[2] += usize::from(d.illum.is_some());
        }
        let mut per_sky: HashMap<String, usize> = HashMap::new();
        for (_, sky) in self.materials.keys() {
            let name = sky
                .and_then(|i| self.skies.skies.skies.get(i))
                .map_or("none", |s| s.name.as_str());
            *per_sky.entry(name.to_string()).or_default() += 1;
        }
        s.per_sky = per_sky.into_iter().collect();
        s.per_sky.sort();
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

/// Default textures of the object material.
struct Defaults {
    /// A flat normal in the game's packing.
    flat: Handle<Image>,
    /// White: the shader default of the specular and glow maps.
    white: Handle<Image>,
}

/// A material as the game draws it with the given Marmoset sky.
fn object_material(
    desc: &MaterialDesc,
    sky: Option<&SkyLook>,
    textures: &HashMap<u32, Handle<Image>>,
    defaults: &Defaults,
    light: &GameLightImages,
) -> ObjectMaterial {
    let texture = |t: Option<u32>| t.and_then(|t| textures.get(&t).cloned());
    let albedo = texture(desc.albedo);
    let normal = texture(desc.normal);
    let spec = texture(desc.spec);
    let illum = texture(desc.illum);
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
    let mut params = ObjectParams {
        normal_st: Vec4::from(desc.normal_st),
        spec_st: Vec4::from(desc.spec_st),
        illum_st: Vec4::from(desc.illum_st),
        has_normal: u32::from(normal.is_some()),
        has_spec: u32::from(spec.is_some()),
        has_illum: u32::from(illum.is_some()),
        ..default()
    };
    if let Some(u) = &desc.uber {
        params.spec_color = linear(u.spec_color).truncate().extend(u.spec_int);
        params.glow_color = linear(u.glow_color);
        params.surface = Vec4::new(
            u.shininess,
            u.fresnel,
            u.ibl_reduction_at_night,
            u.simple_glass,
        );
        params.glow = Vec4::new(
            u.glow_strength,
            u.glow_strength_night,
            u.emission_lm,
            u.emission_lm_night,
        );
        // Without a known sky: Marmoset's defaults (exposures 1, no
        // ambient, outdoors and affected by the day).
        let sky = sky.copied().unwrap_or(SkyLook {
            exposure: [1.0; 4],
            rotation: [0.0, 0.0, 0.0, 1.0],
            affected: true,
            outdoors: true,
            sh: [[0.0; 3]; 9],
        });
        params.exposure = Vec4::from(sky.exposure);
        params.sky_rotation = Vec4::from(sky.rotation);
        params.sky_flags = Vec4::new(
            f32::from(u8::from(sky.affected)),
            f32::from(u8::from(sky.outdoors)),
            1.0,
            0.0,
        );
        params.sh = sky.sh.map(|c| Vec3::from(c).extend(0.0));
    }
    ObjectMaterial {
        base,
        extension: ObjectExtension {
            params,
            normal_map: normal.unwrap_or_else(|| defaults.flat.clone()),
            spec_map: spec.unwrap_or_else(|| defaults.white.clone()),
            illum_map: illum.unwrap_or_else(|| defaults.white.clone()),
            light_params: light.params.clone(),
            caustics: light.caustics.clone(),
        },
    }
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
    light: Res<GameLightImages>,
    water: Option<Res<WaterWorld>>,
    mut exit: MessageWriter<AppExit>,
) {
    let streamer = &mut *streamer;
    let mut pixel = |rgba: [u8; 4]| {
        images.add(Image::new(
            Extent3d::default(),
            TextureDimension::D2,
            rgba.to_vec(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        ))
    };
    let defaults = streamer.defaults.get_or_insert_with(|| Defaults {
        // (0.5, 0.5, 1, 1) in the game's packing: x = a·r, y = g → flat.
        flat: pixel([255, 128, 255, 128]),
        white: pixel([255; 4]),
    });
    let defaults = Defaults {
        flat: defaults.flat.clone(),
        white: defaults.white.clone(),
    };

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
            Update::Skies(skies) => {
                info!(
                    "objects: {} Marmoset skies for {} biomes",
                    skies.looks.len(),
                    skies.skies.biomes.len()
                );
                streamer.skies = skies;
            }
            Update::Material { id, desc } => {
                streamer.material_descs.insert(id, desc);
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
        let casting = lod == 0;
        if casting != b.casting {
            for entity in b.shown.iter().flatten().flatten() {
                if casting {
                    commands
                        .entity(*entity)
                        .remove::<bevy::light::NotShadowCaster>();
                } else {
                    commands
                        .entity(*entity)
                        .insert(bevy::light::NotShadowCaster);
                }
            }
            b.casting = casting;
        }
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
                        // `SkyApplier`: the biome at the object's root.
                        let biome = water
                            .as_deref()
                            .and_then(|w| w.biome_at(inst.transform.position));
                        for part in parts {
                            let sky = streamer.skies.pick(part.biome_sky, biome);
                            let Some(mesh) = streamer.meshes.get(&part.mesh) else {
                                continue;
                            };
                            let material = match streamer.materials.get(&(part.material, sky)) {
                                Some(m) => m.clone(),
                                None => {
                                    let Some(desc) = streamer.material_descs.get(&part.material)
                                    else {
                                        continue;
                                    };
                                    let look = sky.and_then(|i| streamer.skies.looks.get(i));
                                    let m = materials.add(object_material(
                                        desc,
                                        look,
                                        &streamer.textures,
                                        &defaults,
                                        &light,
                                    ));
                                    streamer.materials.insert((part.material, sky), m.clone());
                                    m
                                }
                            };
                            let world = inst.transform.then(&part.local);
                            let mut entity = commands.spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                                to_bevy(&world),
                            ));
                            if !casting {
                                entity.insert(bevy::light::NotShadowCaster);
                            }
                            entities.push(entity.id());
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
