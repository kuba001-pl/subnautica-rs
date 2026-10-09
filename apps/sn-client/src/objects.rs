//! World objects (coral, rocks, plants, wrecks): placements from the game's
//! baked cells (`CellsCache`), drawn with the terrain batches.
//!
//! A worker thread reads a batch's cells, loads each placed prefab once
//! (meshes, materials, textures) and sends everything new to the main
//! thread, followed by the batch's instances. The main thread uploads assets
//! once and spawns one entity per placed mesh part. Cell level *n* is shown
//! while its batch's terrain level of detail is at most *n* (our choice; see
//! `docs/DESIGN.md`, M7c).
//!
//! The cells' spawn slots (`EntitySlotsPlaceholder`) are filled as the game
//! does when a cell first loads, with our own seeded random numbers
//! (`sn_world::fill_slots`, M7d); the fillers show at their own cell level.

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
use sn_assets::{
    Assets as GameAssets, LootTable, MarmoSkies, ObjectRef, Prefab, TerrainTexture, marmo_skies,
};
use sn_install::GameData;
use sn_unity::{Catalog, DayNightLight, LightKind, Material, SKIES_AUTO};
use sn_world::{BatchCoord, EntityInfo, SLOTS_COMPONENT, Transform as Placement};

use crate::game_light::GameLightImages;
use crate::object_look::{ObjectExtension, ObjectMaterial, ObjectParams};
use crate::terrain::TerrainStreamer;
use crate::terrain_look::TerrainLook;
use crate::textures::{linear, to_image};
use crate::water::WaterWorld;

/// Cell levels in the game's data (0 = small, near objects … 3 = far).
const LEVELS: usize = 4;

/// The slot of a batch's batch-objects placements (atmosphere volumes and
/// some lights), after the cell levels.
const BATCH_OBJECTS: usize = LEVELS;

/// Cell levels plus the batch objects.
const SLOTS: usize = LEVELS + 1;

/// Batches around the camera's whose batch objects the game loads
/// (`LargeWorldStreamer.batchLoadRings`, `streaming-*.json`: 1).
const BATCH_LOAD_RINGS: i32 = 1;

/// Entities spawned per frame at most, so a burst of finished batches is
/// spread over several frames.
const SPAWN_BUDGET: usize = 20_000;

/// The worker forgets loaded bundles beyond this many bytes.
const BUNDLE_CACHE_BYTES: usize = 512 << 20;

/// Whether cell `level` is shown for a batch at terrain level of detail `lod`.
fn shows(level: usize, lod: u32) -> bool {
    lod as usize <= level
}

/// Whether slot `slot` (a cell level or the batch objects) of batch `coord`
/// is shown: batch objects within the game's batch load rings of the
/// camera's batch.
fn shows_slot(slot: usize, lod: u32, coord: BatchCoord, camera: Option<BatchCoord>) -> bool {
    if slot == BATCH_OBJECTS {
        camera.is_some_and(|c| {
            (coord.x - c.x).abs() <= BATCH_LOAD_RINGS
                && (coord.y - c.y).abs() <= BATCH_LOAD_RINGS
                && (coord.z - c.z).abs() <= BATCH_LOAD_RINGS
        })
    } else {
        shows(slot, lod)
    }
}

/// The batch containing a Unity world position (160 m batches from the
/// voxel origin).
fn batch_of(p: [f32; 3]) -> BatchCoord {
    let b = |i: usize| ((p[i] + sn_world::VOXEL_WORLD_OFFSET[i]) / 160.0).floor() as i32;
    BatchCoord::new(b(0), b(1), b(2))
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
pub(crate) enum Alpha {
    Opaque,
    Mask(f32),
    Blend,
}

#[derive(Clone)]
pub(crate) struct MaterialDesc {
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
    pub(crate) uber: Option<UberValues>,
}

/// A MarmosetUBER material's values (the shader's defaults where the
/// material has none). Colours as stored (sRGB).
#[derive(Clone, Copy)]
pub(crate) struct UberValues {
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
pub(crate) struct SkyLook {
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

/// A point or spot light of a prefab, placed relative to the prefab root.
#[derive(Clone, Copy)]
struct LocalLight {
    spot: bool,
    /// Unity's `_LightColor`: `linear(colour × intensity)` (the game's
    /// `m_LightsUseLinearIntensity` is off).
    color: [f32; 3],
    range: f32,
    /// Full cone angle, degrees (spots).
    spot_angle: f32,
    local: Placement,
}

/// A directional light of a prefab (the atmosphere volumes' "Bounce"
/// lights), placed relative to the prefab root. It lights everything while
/// its object is loaded (see `game_light.rs`).
#[derive(Clone)]
pub struct DirectionalSource {
    /// Colour and intensity as stored (sRGB colour, gamma intensity).
    pub color: [f32; 3],
    pub intensity: f32,
    /// Drives colour and intensity over the day, if present.
    pub day_night: Option<Arc<DayNightLight>>,
    /// The light's GameObject name.
    name: String,
    local: Placement,
}

/// A placed directional light: a marker entity whose transform's forward is
/// the way the light travels (Unity's +z).
#[derive(Component, Clone)]
pub struct GameDirectionalLight(pub DirectionalSource);

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
    /// Spawned by a spawn slot (not placed in the cells).
    from_slot: bool,
}

/// Which scenes the worker loads, and their state.
#[derive(Clone, Copy)]
pub struct SceneOptions {
    /// The Aurora after its explosion (a new game starts before it).
    pub aurora_exploded: bool,
    /// Where Lifepod 5 starts (`None`: no lifepod).
    pub lifepod: Option<[f32; 3]>,
}

/// What the worker fills spawn slots with.
struct SlotTables {
    seed: u64,
    loot: LootTable,
    infos: HashMap<String, EntityInfo>,
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
        lights: Vec<LocalLight>,
        directional: Vec<DirectionalSource>,
    },
    Batch {
        coord: BatchCoord,
        instances: Vec<Instance>,
        ms: f32,
    },
    /// A scene's top-level objects, shown always.
    Scene {
        summary: String,
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
    /// `None`: spawn slots stay empty (`--no-slots`, or the tables failed
    /// to load).
    slots: Option<SlotTables>,
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
        let texture = |name: &str, srgb: Option<bool>| {
            let (pptr, st) = slot(name)?;
            let target = self.assets.resolve(&object.file, pptr).ok()??;
            Some((self.texture(&target, srgb, out)?, st))
        };
        let desc = material_desc(&material, texture);
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
        self.send_geometry(key, &geometry, out)
    }

    /// A skinned node's mesh in its still pose, in the prefab root's space
    /// (cached by the node's GameObject); `None` if it has no skin (then it
    /// is drawn as a plain mesh).
    fn skinned_mesh(
        &mut self,
        prefab: &Prefab,
        node: usize,
        out: &mut Vec<Update>,
    ) -> Option<Vec<Option<u32>>> {
        let n = &prefab.nodes[node];
        let key = n.object.clone();
        if self.meshes.contains_key(&(key.clone(), 0)) {
            return Some(
                (0..)
                    .map_while(|i| self.meshes.get(&(key.clone(), i)).copied())
                    .collect(),
            );
        }
        let (mesh, geometry) = match self.assets.mesh(n.mesh.as_ref()?) {
            Ok(x) => x,
            Err(e) => {
                out.push(Update::Warning(format!("mesh: {e}")));
                return None;
            }
        };
        let skinned = prefab.skinned_geometry(node, &mesh, &geometry)?;
        Some(self.send_geometry(key, &skinned, out))
    }

    /// Sends a mesh's sub-meshes (Unity space in, Bevy's out), cached
    /// under `key`.
    fn send_geometry(
        &mut self,
        key: Key,
        geometry: &sn_unity::MeshGeometry,
        out: &mut Vec<Update>,
    ) -> Vec<Option<u32>> {
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
        let id = self.prefab_parts(&prefab, out);
        self.prefabs.insert(path.to_string(), id);
        id
    }

    /// Sends a loaded hierarchy's drawn parts and lights; its id, `None`
    /// if it has nothing to draw.
    fn prefab_parts(&mut self, prefab: &Prefab, out: &mut Vec<Update>) -> Option<u32> {
        let mut parts = Vec::new();
        for (index, node) in prefab.visible() {
            // Other anchors (a fixed sky) are taken as the global sky.
            let biome_sky = node.sky_applier == Some(SKIES_AUTO);
            let Some(mesh) = &node.mesh else { continue };
            // Skinned meshes come out in the prefab root's space.
            let skinned = if node.skinned {
                self.skinned_mesh(prefab, index, out)
            } else {
                None
            };
            let local = if skinned.is_some() {
                Placement::default()
            } else {
                node.in_prefab
            };
            let sub_meshes = match skinned {
                Some(ids) => ids,
                None => self.mesh(mesh, out),
            };
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
                    local,
                    biome_sky,
                });
            }
        }
        // Realtime point and spot lights that are on when the prefab is
        // placed (directional ones are handled elsewhere).
        let mut lights = Vec::new();
        let mut directional = Vec::new();
        for node in prefab.nodes.iter().filter(|n| n.active) {
            for l in node.lights.iter().filter(|l| l.enabled && l.is_realtime()) {
                let spot = match l.kind {
                    LightKind::Point => false,
                    LightKind::Spot => true,
                    LightKind::Directional => {
                        // (`LargeWorldStreamer.OnBatchObjectsLoaded` destroys
                        // some directional "bounce" lights, but only in the
                        // batch-objects placements, which we don't stream;
                        // see docs/formats/lighting.md.)
                        directional.push(DirectionalSource {
                            color: [l.color[0], l.color[1], l.color[2]],
                            intensity: l.intensity,
                            day_night: node.day_night_light.clone().map(Arc::new),
                            name: node.name.clone(),
                            local: node.in_prefab,
                        });
                        continue;
                    }
                    _ => continue,
                };
                if l.range <= 0.0 || l.intensity <= 0.0 {
                    continue;
                }
                let gamma = [l.color[0], l.color[1], l.color[2]].map(|c| c * l.intensity);
                let c = crate::sky::linear(Vec3::from(gamma));
                lights.push(LocalLight {
                    spot,
                    color: c.to_array(),
                    range: l.range,
                    spot_angle: l.spot_angle,
                    local: node.in_prefab,
                });
            }
        }
        let id =
            (!parts.is_empty() || !lights.is_empty() || !directional.is_empty()).then(|| self.id());
        if let Some(id) = id {
            out.push(Update::Prefab {
                id,
                parts,
                lights,
                directional,
            });
        }
        id
    }

    /// `EscapePod.ChooseRandomStart` at `point`, the objects following
    /// their targets, and what the pod's spawners put in it in a new game
    /// (added to `instances`). Returns a summary for the log.
    fn place_pod(
        &mut self,
        scene: &mut sn_assets::Scene,
        point: [f32; 3],
        instances: &mut Vec<Instance>,
        out: &mut Vec<Update>,
    ) -> Result<String, String> {
        let spawn = scene.place_escape_pod(&self.assets, point)?;
        let following = scene.follow_targets(&self.assets)?;
        let mut spawned = Vec::new();
        for s in scene.spawns(&self.assets)? {
            if s.spawner.deactivate_on_spawn {
                continue;
            }
            let prefab = match (&s.spawner.prefab, &s.object) {
                (sn_unity::SpawnPrefab::Address(guid), _) => {
                    self.assets.prefab(&self.catalog, guid)
                }
                (_, Some(object)) => self.assets.hierarchy(&s.name, object),
                _ => continue,
            };
            let prefab = match prefab {
                Ok(p) => p,
                Err(e) => {
                    out.push(Update::Warning(format!("lifepod {}: {e}", s.name)));
                    continue;
                }
            };
            let transform = s.placement(&prefab.nodes[0].local);
            if let Some(id) = self.prefab_parts(&prefab, out) {
                instances.push(Instance {
                    level: 0,
                    prefab: id,
                    transform,
                    from_slot: false,
                });
                spawned.push(s.name.clone());
            }
        }
        Ok(format!(
            ", Lifepod 5 at {:?}, player spawn {:?}, {following} objects on their targets, spawned {spawned:?}",
            point.map(|v| (v * 100.0).round() / 100.0),
            spawn.position.map(|v| (v * 100.0).round() / 100.0)
        ))
    }

    /// The scenes the game spawns at start (`docs/DESIGN.md` M7f): each
    /// top-level object becomes an instance at its world placement.
    fn scenes(&mut self, options: &SceneOptions, out: &mut Vec<Update>) {
        let start = Instant::now();
        let autoload = match self.assets.startup_scenes() {
            Ok((_, autoload)) => autoload,
            Err(e) => {
                out.push(Update::Warning(format!("scenes: {e}")));
                return;
            }
        };
        for a in autoload.iter().filter(|a| a.spawn_on_start) {
            let is_pod = a.scene_name.eq_ignore_ascii_case("EscapePod");
            if is_pod && options.lifepod.is_none() {
                continue;
            }
            let mut scene = match self.assets.scene(&a.scene_name) {
                Ok(s) => s,
                Err(e) => {
                    out.push(Update::Warning(format!("scene {}: {e}", a.scene_name)));
                    continue;
                }
            };
            scene.spawn_lightmapped_prefab();
            let mut state = String::new();
            if !scene
                .behaviours(&self.assets, "CrashedShipExploder")
                .is_empty()
            {
                match scene.swap_aurora_models(&self.assets, options.aurora_exploded) {
                    Ok((off, on)) => {
                        state = format!(
                            ", Aurora {} ({off} objects off, {on} on)",
                            if options.aurora_exploded {
                                "exploded"
                            } else {
                                "intact"
                            }
                        );
                    }
                    Err(e) => out.push(Update::Warning(format!("scene {}: {e}", scene.name))),
                }
            }
            let mut instances = Vec::new();
            if let (true, Some(point)) = (is_pod, options.lifepod) {
                match self.place_pod(&mut scene, point, &mut instances, out) {
                    Ok(summary) => state = summary,
                    Err(e) => out.push(Update::Warning(format!("lifepod: {e}"))),
                }
            }
            let mut drawn = 0;
            for root in &scene.roots {
                drawn += root.visible_nodes().count();
                if let Some(prefab) = self.prefab_parts(root, out) {
                    instances.push(Instance {
                        level: 0,
                        prefab,
                        transform: root.nodes[0].local,
                        from_slot: false,
                    });
                }
            }
            let nodes: usize = scene.roots.iter().map(|r| r.nodes.len()).sum();
            out.push(Update::Scene {
                summary: format!(
                    "scene {}: {} top-level objects, {nodes} nodes, {drawn} drawn{state}",
                    scene.name,
                    scene.roots.len()
                ),
                instances,
                ms: start.elapsed().as_secs_f32() * 1000.0,
            });
        }
        self.assets.trim_cache(BUNDLE_CACHE_BYTES);
    }

    /// A prefab drawn as a still object: creatures move and animate, so
    /// they are left out (a later milestone).
    fn still_prefab(&mut self, path: &str, out: &mut Vec<Update>) -> Option<u32> {
        if path.starts_with("WorldEntities/Creatures/") {
            return None;
        }
        self.prefab(path, out)
    }

    /// Fills the slots of a placeholder (component `data`, placed at
    /// `placeholder` in world space); each filler shows at its own cell
    /// level (`WorldEntityInfo.cellLevel`; batch and global levels at the
    /// farthest).
    fn fill_slots(
        &mut self,
        id: &str,
        data: &[u8],
        placeholder: &Placement,
        instances: &mut Vec<Instance>,
        out: &mut Vec<Update>,
    ) {
        let Some(tables) = &self.slots else { return };
        let slots = match sn_world::parse_slots(data) {
            Ok(s) => s,
            Err(e) => {
                out.push(Update::Warning(format!("slots of {id}: {e}")));
                return;
            }
        };
        let spawns = sn_world::fill_slots(
            tables.seed,
            id,
            &slots,
            &tables.loot.distribution,
            &tables.infos,
        );
        let wanted: Vec<(String, Placement, usize)> = spawns
            .iter()
            .filter_map(|s| {
                let path = self.class_paths.get(s.class_id)?;
                let level = usize::try_from(s.info.cell_level).map_or(0, |l| l.min(LEVELS - 1));
                Some((path.clone(), placeholder.then(&s.transform), level))
            })
            .collect();
        for (path, transform, level) in wanted {
            if let Some(prefab) = self.still_prefab(&path, out) {
                instances.push(Instance {
                    level,
                    prefab,
                    transform,
                    from_slot: true,
                });
            }
        }
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
                        for c in &object.components {
                            if c.type_name == SLOTS_COMPONENT {
                                self.fill_slots(
                                    &object.id,
                                    &c.data,
                                    &transform,
                                    &mut instances,
                                    out,
                                );
                            }
                        }
                        let Some(path) = self.class_paths.get(&object.class_id).cloned() else {
                            continue;
                        };
                        if let Some(prefab) = self.still_prefab(&path, out) {
                            instances.push(Instance {
                                level: (cell.level as usize).min(LEVELS - 1),
                                prefab,
                                transform,
                                from_slot: false,
                            });
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => out.push(Update::Warning(e.0)),
        }
        // Batch objects: the root is moved to the batch's corner, as the
        // game does when it loads them (`docs/formats/entities.md`).
        match game.read_batch_objects(coord) {
            Ok(Some(mut tree)) => {
                let corner = [coord.x, coord.y, coord.z]
                    .map(|c| c as f32 * 160.0)
                    .iter()
                    .zip(sn_world::VOXEL_WORLD_OFFSET)
                    .map(|(c, o)| c - o)
                    .collect::<Vec<_>>();
                for o in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
                    o.transform.position = [corner[0], corner[1], corner[2]];
                }
                let (world, _) = tree.world_transforms();
                for (object, transform) in tree.objects.iter().zip(world) {
                    let Some(path) = self.class_paths.get(&object.class_id).cloned() else {
                        continue;
                    };
                    if let Some(prefab) = self.prefab(&path, out) {
                        instances.push(Instance {
                            level: BATCH_OBJECTS,
                            prefab,
                            transform,
                            from_slot: false,
                        });
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

/// A material as our object shader takes it (`object_material`), from the
/// game's `Material`; `texture(slot, srgb)` gives a texture id (`None`
/// colour space: the texture's own) and the slot's scale/offset.
pub(crate) fn material_desc(
    material: &Material,
    mut texture: impl FnMut(&str, Option<bool>) -> Option<(u32, [f32; 4])>,
) -> MaterialDesc {
    let keywords: Vec<&str> = material.keywords.split_whitespace().collect();
    let has = |k: &str| keywords.contains(&k);
    // MarmosetUBER (the game's object shader) has these two properties;
    // its specular and glow maps are used with these keywords only.
    let is_uber =
        material.float("_Shininess").is_some() && material.float("_GlowStrengthNight").is_some();
    let albedo = texture("_MainTex", Some(true));
    let normal = texture("_BumpMap", Some(false));
    let spec = (is_uber && has("MARMO_SPECMAP"))
        .then(|| texture("_SpecTex", None))
        .flatten();
    let illum = (is_uber && has("MARMO_EMISSION"))
        .then(|| texture("_Illum", None))
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
    MaterialDesc {
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
    }
}

struct Shared {
    /// Batches to load, most urgent last.
    queue: Mutex<Vec<BatchCoord>>,
    wake: Condvar,
    shutdown: AtomicBool,
}

fn worker(
    game: &'static GameData,
    shared: Arc<Shared>,
    tx: Sender<Update>,
    slot_seed: Option<u64>,
    scenes: Option<SceneOptions>,
) {
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
            slots: None,
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
    if let Some(seed) = slot_seed {
        let tables = sn_assets::loot_table(&library.assets).and_then(|loot| {
            Ok(SlotTables {
                seed,
                loot,
                infos: sn_assets::entity_infos(&library.assets)?,
            })
        });
        match tables {
            Ok(t) => {
                info!(
                    "slots: seed {seed}, loot table of {} prefabs in {} biomes, {} world entity infos",
                    t.loot.prefabs,
                    t.loot.distribution.biome_count(),
                    t.infos.len()
                );
                library.slots = Some(t);
            }
            Err(e) => {
                let _ = tx.send(Update::Warning(format!("slots stay empty: {e}")));
            }
        }
    }
    let _ = tx.send(Update::Ready {
        ms: start.elapsed().as_secs_f32() * 1000.0,
    });
    if let Some(options) = scenes {
        let mut out = Vec::new();
        library.scenes(&options, &mut out);
        for update in out {
            if tx.send(update).is_err() {
                return;
            }
        }
    }
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
    shown: [Option<Vec<Entity>>; SLOTS],
    /// Whether its entities cast sun shadows (only the nearest level of
    /// detail: the game's shadows reach 50 m).
    casting: bool,
}

/// A scene's top-level objects on the main thread.
struct SceneObjects {
    instances: Vec<Instance>,
    /// `None` until spawned.
    shown: Option<Vec<Entity>>,
}

#[derive(Default, Clone)]
pub struct ObjectStats {
    pub batches: usize,
    pub entities: usize,
    /// Cell levels 0–3, then batch objects.
    pub per_level: [usize; SLOTS],
    /// Shown objects (not entities) that spawn slots filled.
    pub slot_objects: usize,
    /// Entities of the scenes (shown always, not in `entities`).
    pub scene_entities: usize,
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
    prefab_lights: HashMap<u32, Vec<LocalLight>>,
    prefab_directional: HashMap<u32, Vec<DirectionalSource>>,
    batches: HashMap<BatchCoord, BatchObjects>,
    scenes: Vec<SceneObjects>,
    /// What was last put in the worker's queue.
    requested: Vec<BatchCoord>,
    defaults: Option<Defaults>,
    /// Spawn the objects' point and spot lights.
    lights: bool,
    /// The camera's batch (for the batch objects).
    camera_batch: Option<BatchCoord>,
    warnings: usize,
    /// Worker time per batch (ms).
    pub batch_ms: Vec<f32>,
}

impl ObjectStreamer {
    /// `lights`: spawn the objects' point and spot lights. `slot_seed`: fill
    /// the spawn slots with this world seed (`None`: leave them empty).
    /// `scenes`: load the scenes the game spawns at start (`None`: none).
    pub fn start(
        game: GameData,
        lights: bool,
        slot_seed: Option<u64>,
        scenes: Option<SceneOptions>,
    ) -> ObjectStreamer {
        // The worker's asset index borrows the install for the whole run.
        let game: &'static GameData = Box::leak(Box::new(game));
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let worker_shared = shared.clone();
        std::thread::spawn(move || worker(game, worker_shared, tx, slot_seed, scenes));
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
            prefab_lights: HashMap::new(),
            prefab_directional: HashMap::new(),
            batches: HashMap::new(),
            scenes: Vec::new(),
            requested: Vec::new(),
            defaults: None,
            lights,
            camera_batch: None,
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
                    s.slot_objects += b
                        .instances
                        .iter()
                        .flatten()
                        .filter(|i| i.from_slot && i.level == level)
                        .count();
                }
            }
        }
        s.scene_entities = self
            .scenes
            .iter()
            .flat_map(|sc| &sc.shown)
            .map(Vec::len)
            .sum();
        s
    }

    /// True when every wanted batch has its objects on screen.
    pub fn settled(&self, terrain: &TerrainStreamer) -> bool {
        self.ready
            && terrain.shown_lods().all(|(coord, lod)| {
                self.batches.get(&coord).is_some_and(|b| {
                    b.instances.is_some()
                        && (0..SLOTS).all(|l| {
                            b.shown[l].is_some() == shows_slot(l, lod, coord, self.camera_batch)
                        })
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

/// Bevy's light, only so that Bevy culls and clusters it: our shaders read
/// the clustered lights and apply the game's formula (`game_light.wgsl`).
/// Bevy stores colour × intensity ÷ 4π, so intensity 4π keeps the game's
/// `_LightColor` unchanged. Shadows: not yet (M8e4).
fn spawn_light(commands: &mut Commands, light: &LocalLight, transform: Transform) -> Entity {
    let [r, g, b] = light.color;
    let color = Color::linear_rgb(r, g, b);
    let intensity = 4.0 * std::f32::consts::PI;
    if light.spot {
        // Unity's spot shines along its local +z, which `to_bevy` maps to
        // Bevy's forward (−z).
        commands
            .spawn((
                SpotLight {
                    color,
                    intensity,
                    range: light.range,
                    radius: 0.0,
                    shadow_maps_enabled: false,
                    outer_angle: (light.spot_angle * 0.5).to_radians(),
                    inner_angle: 0.0,
                    ..default()
                },
                transform,
            ))
            .id()
    } else {
        commands
            .spawn((
                PointLight {
                    color,
                    intensity,
                    range: light.range,
                    radius: 0.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                transform,
            ))
            .id()
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
pub(crate) struct Defaults {
    /// A flat normal in the game's packing.
    pub flat: Handle<Image>,
    /// White: the shader default of the specular and glow maps.
    pub white: Handle<Image>,
}

/// A material as the game draws it with the given Marmoset sky.
pub(crate) fn object_material(
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
        apply_sky(&mut params, sky);
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
            spot_cookie: light.spot_cookie.clone(),
        },
    }
}

/// Lights a MarmosetUBER material with `sky`. Without a known sky:
/// Marmoset's defaults (exposures 1, no ambient, outdoors and affected by
/// the day).
pub(crate) fn apply_sky(params: &mut ObjectParams, sky: Option<&SkyLook>) {
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
    camera: Query<&Transform, With<Camera3d>>,
    mut terrain_look: Option<ResMut<TerrainLook>>,
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
                // Terrain grass has no `SkyApplier`: the global sky.
                if let Some(look) = terrain_look.as_mut() {
                    let global = skies.skies.global.and_then(|i| skies.looks.get(i));
                    look.set_global_sky(global, &mut materials);
                }
                streamer.skies = skies;
            }
            Update::Material { id, desc } => {
                streamer.material_descs.insert(id, desc);
            }
            Update::Mesh { id, data } => {
                streamer.meshes.insert(id, meshes.add(build_mesh(data)));
            }
            Update::Prefab {
                id,
                parts,
                lights,
                directional,
            } => {
                streamer.prefabs.insert(id, parts);
                if !lights.is_empty() {
                    streamer.prefab_lights.insert(id, lights);
                }
                if !directional.is_empty() {
                    streamer.prefab_directional.insert(id, directional);
                }
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
            Update::Scene {
                summary,
                instances,
                ms,
            } => {
                info!("objects: {summary} ({ms:.0} ms)");
                streamer.scenes.push(SceneObjects {
                    instances,
                    shown: None,
                });
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
    if let Ok(t) = camera.single() {
        let p = t.translation;
        streamer.camera_batch = Some(batch_of([p.x, p.y, -p.z]));
    }
    let camera_batch = streamer.camera_batch;
    for (&coord, &lod) in &wanted {
        let Some(b) = streamer.batches.get_mut(&coord) else {
            continue;
        };
        if b.instances.is_none() {
            continue;
        }
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
        for level in 0..SLOTS {
            let want = shows_slot(level, lod, coord, camera_batch);
            let Some(b) = streamer.batches.get_mut(&coord) else {
                break;
            };
            match (&b.shown[level], want) {
                (Some(_), false) => {
                    for entity in b.shown[level].take().into_iter().flatten() {
                        commands.entity(entity).despawn();
                    }
                }
                (None, true) if budget > 0 => {
                    let todo: Vec<Instance> = b
                        .instances
                        .iter()
                        .flatten()
                        .filter(|i| i.level == level)
                        .copied()
                        .collect();
                    let mut entities = Vec::new();
                    let mut spawner = Spawner {
                        commands: &mut commands,
                        materials: &mut materials,
                        defaults: &defaults,
                        light: &light,
                        water: water.as_deref(),
                    };
                    for inst in &todo {
                        streamer.spawn_instance(inst, casting, &mut spawner, &mut entities);
                    }
                    budget = budget.saturating_sub(entities.len().max(1));
                    if let Some(b) = streamer.batches.get_mut(&coord) {
                        b.shown[level] = Some(entities);
                    }
                }
                _ => {}
            }
        }
    }

    // Scenes: spawned once, shown always.
    let mut spawner = Spawner {
        commands: &mut commands,
        materials: &mut materials,
        defaults: &defaults,
        light: &light,
        water: water.as_deref(),
    };
    for i in 0..streamer.scenes.len() {
        if streamer.scenes[i].shown.is_some() {
            continue;
        }
        let todo = streamer.scenes[i].instances.clone();
        let mut entities = Vec::new();
        for inst in &todo {
            streamer.spawn_instance(inst, true, &mut spawner, &mut entities);
        }
        streamer.scenes[i].shown = Some(entities);
    }
}

/// What spawning an instance needs from the frame's system.
struct Spawner<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    materials: &'a mut Assets<ObjectMaterial>,
    defaults: &'a Defaults,
    light: &'a GameLightImages,
    water: Option<&'a WaterWorld>,
}

impl ObjectStreamer {
    /// Spawns an instance's parts and lights, adding their entities to
    /// `entities`.
    fn spawn_instance(
        &mut self,
        inst: &Instance,
        casting: bool,
        s: &mut Spawner,
        entities: &mut Vec<Entity>,
    ) {
        let Some(parts) = self.prefabs.get(&inst.prefab) else {
            return;
        };
        // `SkyApplier`: the biome at the object's root.
        let biome = s.water.and_then(|w| w.biome_at(inst.transform.position));
        for part in parts {
            let sky = self.skies.pick(part.biome_sky, biome);
            let Some(mesh) = self.meshes.get(&part.mesh) else {
                continue;
            };
            let material = match self.materials.get(&(part.material, sky)) {
                Some(m) => m.clone(),
                None => {
                    let Some(desc) = self.material_descs.get(&part.material) else {
                        continue;
                    };
                    let look = sky.and_then(|i| self.skies.looks.get(i));
                    let m = s.materials.add(object_material(
                        desc,
                        look,
                        &self.textures,
                        s.defaults,
                        s.light,
                    ));
                    self.materials.insert((part.material, sky), m.clone());
                    m
                }
            };
            let world = inst.transform.then(&part.local);
            let mut entity = s.commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                to_bevy(&world),
            ));
            if !casting {
                entity.insert(bevy::light::NotShadowCaster);
            }
            entities.push(entity.id());
        }
        let lights = self.prefab_lights.get(&inst.prefab).filter(|_| self.lights);
        for light in lights.into_iter().flatten() {
            let world = to_bevy(&inst.transform.then(&light.local));
            entities.push(spawn_light(s.commands, light, world));
        }
        let directional = self
            .prefab_directional
            .get(&inst.prefab)
            .filter(|_| self.lights);
        for light in directional.into_iter().flatten() {
            // `LargeWorldStreamer.OnBatchObjectsLoaded` destroys batch
            // objects' directional lights whose name has "bounce" after its
            // first letter (`IndexOf(…) > 0`, ignoring case); "Bounce" stays.
            let lower = light.name.to_lowercase();
            if inst.level == BATCH_OBJECTS && lower.find("bounce").is_some_and(|i| i > 0) {
                continue;
            }
            let world = to_bevy(&inst.transform.then(&light.local));
            let marker = GameDirectionalLight(light.clone());
            entities.push(s.commands.spawn((marker, world)).id());
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
