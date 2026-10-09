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

use std::collections::{BTreeMap, HashMap, HashSet};
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
use sn_unity::{Catalog, DayNightLight, LightKind, Material, SKIES_AUTO, Shader};
use sn_world::{BatchCoord, EntityInfo, SLOTS_COMPONENT, Transform as Placement};

use crate::effects::{
    EffectLook, EffectMeshes, EffectPart, EffectValues, PARTICLES_TEXTURES, ParticlesValues,
};
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
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) normals: Vec<[f32; 3]>,
    pub(crate) tangents: Vec<[f32; 4]>,
    /// Vertex colours (empty if the mesh has none); only the effect meshes
    /// use them (`effects.rs`).
    pub(crate) colors: Vec<[f32; 4]>,
    pub(crate) uvs: Vec<[f32; 2]>,
    pub(crate) indices: Vec<u32>,
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
    /// Values of a shader drawn by our effect pass (`effects.rs`).
    pub(crate) effect: Option<EffectLook>,
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
    /// Drawn by our effect pass (`effects.rs`): `mesh` is an effect mesh id
    /// (`Update::EffectMesh`) and the material has `effect` values.
    effect: bool,
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
/// A saved tree's objects by parent id.
fn saved_children(tree: &sn_world::ObjectTree) -> HashMap<&str, Vec<usize>> {
    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, o) in tree.objects.iter().enumerate() {
        if let Some(p) = &o.parent {
            children.entry(p.as_str()).or_default().push(i);
        }
    }
    children
}

/// The class ids of every saved object below the object `id`.
fn saved_below<'t>(
    tree: &'t sn_world::ObjectTree,
    children: &HashMap<&str, Vec<usize>>,
    id: &str,
) -> Vec<&'t str> {
    let mut below = Vec::new();
    let mut todo = vec![id.to_string()];
    while let Some(id) = todo.pop() {
        for &c in children.get(id.as_str()).into_iter().flatten() {
            below.push(tree.objects[c].class_id.as_str());
            todo.push(tree.objects[c].id.clone());
        }
    }
    below
}

/// What a prefab draws and lights, relative to its root.
struct PrefabContent {
    parts: Vec<Part>,
    lights: Vec<LocalLight>,
    directional: Vec<DirectionalSource>,
}

/// Prefabs spawned by placeholders inside prefabs spawned by placeholders
/// … deeper than this are taken as a loop.
const MAX_PLACEHOLDER_DEPTH: usize = 8;
/// `prefab_content` depth meaning: don't spawn the placeholders.
const NO_SPAWN: usize = usize::MAX;

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
        desc: Box<MaterialDesc>,
    },
    Mesh {
        id: u32,
        data: MeshData,
    },
    /// A sub-mesh drawn by the effect pass (kept as data, not a Bevy mesh).
    EffectMesh {
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
    /// (prefab path, its placeholders spawn) → id (`None`: nothing to draw,
    /// or failed to load).
    prefabs: HashMap<(String, bool), Option<u32>>,
    /// Prefab path → the class ids its placeholders spawn (recorded when it
    /// loads).
    placeholder_ids: HashMap<String, Vec<String>>,
    meshes: HashMap<(Key, usize), Option<u32>>,
    materials: HashMap<Key, u32>,
    /// `None`: the texture's own colour space.
    textures: HashMap<(Key, Option<bool>), Option<u32>>,
    next_id: u32,
    /// `None`: spawn slots stay empty (`--no-slots`, or the tables failed
    /// to load).
    slots: Option<SlotTables>,
    /// The game's main camera's culling mask: renderers on other layers
    /// never reach the picture (e.g. the occluder shells on layer 27,
    /// `docs/formats/materials.md`).
    culling_mask: u32,
    /// Shader object → its name and property defaults.
    shader_infos: HashMap<Key, Arc<ShaderInfo>>,
    /// Materials drawn by the effect pass, with their description.
    effect_materials: HashMap<u32, MaterialDesc>,
    /// (effect material, its values as bits) → the material with those
    /// values, as a `VFXVolumetricLight`'s property block sets them.
    glow_materials: HashMap<(u32, [u32; 9]), u32>,
    /// (mesh, sub-mesh) → effect mesh id, as `meshes`.
    effect_meshes: HashMap<(Key, usize), Option<u32>>,
    /// Material id → its shader's name.
    material_shaders: HashMap<u32, String>,
    /// Shaders we draw with a stand-in look (not ported): parts drawn in
    /// the prefabs and scenes loaded so far, per shader (`docs/DESIGN.md`
    /// § 4.2: they stay drawn, and are logged so they're not forgotten).
    unported: BTreeMap<String, usize>,
    /// `unported` changed since it was last logged.
    unported_changed: bool,
    /// `WorldEntities/WorldEntityData`: a placeholder spawns only a prefab
    /// with an info (`PrefabPlaceholder.Spawn`).
    entity_infos: HashMap<String, EntityInfo>,
    /// Prefabs spawned by placeholders, by path.
    spawned_contents: HashMap<String, Option<Arc<PrefabContent>>>,
    /// Spawn what placeholders hold (off with `--no-placeholders`).
    spawn_placeholders: bool,
    /// Placeholders spawned so far (each time a prefab holding one loads).
    placeholders_spawned: usize,
    /// `placeholders_spawned` when it was last logged.
    placeholders_logged: usize,
}

/// How far a shader is ported (`docs/formats/materials.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderPort {
    /// MarmosetUBER: our object shader is its port.
    Ported,
    /// A MarmosetUBER variant: drawn as UBER, its own properties not read.
    AsUber,
    /// Ported in our effect pass (`effects.rs`).
    Effect,
    /// Drawn as UBER-less `_MainTex` × `_Color` in our object shader.
    NotPorted,
}

/// What the client keeps of a material's shader.
pub(crate) struct ShaderInfo {
    pub name: String,
    /// Property name → its declared default (colours and vectors as is,
    /// floats in `x`).
    pub defaults: HashMap<String, [f32; 4]>,
}

/// An effect material's values: the material's own, else the shader's
/// declared defaults (as Unity does).
fn effect_values(material: &Material, shader: &ShaderInfo) -> EffectValues {
    let float = |name: &str| {
        material
            .float(name)
            .or_else(|| shader.defaults.get(name).map(|d| d[0]))
            .unwrap_or(0.0)
    };
    EffectValues {
        color: material
            .color("_Color")
            .or_else(|| shader.defaults.get("_Color").copied())
            .unwrap_or([1.0; 4]),
        intensity: float("_Intensity"),
        fresnel_fade: float("_FresnelFade"),
        fresnel_pow: float("_FresnelPow"),
        clip_offset: float("_ClipOffset"),
        clip_fade: float("_ClipFade"),
        offset: float("_Offset"),
        fallof: float("_Fallof"),
        inv_fade: float("_InvFade"),
    }
}

/// What a decoded `UWE/Particles/UBER` keyword set switches on.
#[derive(Clone, Copy)]
struct ParticlesVariant {
    fresnel_clip: bool,
    mul_map: bool,
    soft_edges: bool,
    deform: bool,
    refract: bool,
}

/// The `UWE/Particles/UBER` keyword sets our effect pass draws, decoded
/// from the game's compiled programs (`docs/formats/materials.md`
/// § `UWE/Particles/UBER` meshes): the consoles' holograms and the doors'
/// force fields.
const PARTICLES_VARIANTS: [(&str, ParticlesVariant); 5] = {
    const fn v(fresnel_clip: bool, mul_map: bool, rest: bool, refract: bool) -> ParticlesVariant {
        ParticlesVariant {
            fresnel_clip,
            mul_map,
            soft_edges: rest,
            deform: rest,
            refract,
        }
    }
    [
        (
            "FX_ADDFOG FX_FRESNELCLIP FX_MULMAP FX_SCROLL WBOIT",
            v(true, true, false, false),
        ),
        (
            "FX_ADDFOG FX_MULMAP FX_SCROLL WBOIT",
            v(false, true, false, false),
        ),
        ("FX_ADDFOG FX_SCROLL WBOIT", v(false, false, false, false)),
        (
            "FX_ADDFOG FX_DEFORM FX_MULMAP FX_REFRACTMAP FX_SCROLL FX_SOFTEDGES WBOIT",
            v(false, true, true, true),
        ),
        (
            "FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT",
            v(false, true, true, false),
        ),
    ]
};

/// The game's mesh-effect shader; ported per material (`particles_values`).
const PARTICLES_UBER: &str = "UWE/Particles/UBER";

/// A `UWE/Particles/UBER` material's values if our effect pass draws it as
/// the game does: one of the decoded keyword sets, and the render state the
/// pass implements (blend `_SrcBlend` 1 `_DstBlend` 1 `_SrcBlend2` 0
/// `_DstBlend2` 10, `_Ztest` 2 = Less, `_MyCullVariable` 0 = both sides).
/// Else why not. `textures`: the ids of `_MainTex`, `_MainTex2`,
/// `_DeformMap`, `_RefractMap` with their scale/offset.
fn particles_values(
    material: &Material,
    shader: &ShaderInfo,
    textures: [Option<(u32, [f32; 4])>; PARTICLES_TEXTURES],
) -> Result<ParticlesValues, String> {
    let mut keywords: Vec<&str> = material.keywords.split_whitespace().collect();
    keywords.sort_unstable();
    let keywords = keywords.join(" ");
    let Some(&(_, variant)) = PARTICLES_VARIANTS.iter().find(|(k, _)| *k == keywords) else {
        return Err(format!("keywords [{keywords}] not decoded"));
    };
    let float = |name: &str| {
        material
            .float(name)
            .or_else(|| shader.defaults.get(name).map(|d| d[0]))
            .unwrap_or(0.0)
    };
    let state = [
        ("_SrcBlend", 1.0),
        ("_DstBlend", 1.0),
        ("_SrcBlend2", 0.0),
        ("_DstBlend2", 10.0),
        ("_Ztest", 2.0),
        ("_MyCullVariable", 0.0),
    ];
    for (name, wanted) in state {
        let v = float(name);
        if v != wanted {
            return Err(format!("{name} {v} (the pass draws {wanted})"));
        }
    }
    let vector = |name: &str, default: [f32; 4]| {
        material
            .color(name)
            .or_else(|| shader.defaults.get(name).copied())
            .unwrap_or(default)
    };
    let st = |t: Option<(u32, [f32; 4])>| t.map_or([1.0, 1.0, 0.0, 0.0], |(_, st)| st);
    let [main, main2, deform, refract] = textures;
    let speed = vector("_MainTex_Speed", [0.0; 4]);
    let speed2 = vector("_MainTex2_Speed", [0.0; 4]);
    let deform_speed = vector("_DeformMap_Speed", [0.0; 4]);
    let refract_speed = vector("_RefractMap_Speed", [0.0; 4]);
    Ok(ParticlesValues {
        color: vector("_Color", [1.0; 4]),
        strength: vector("_ColorStrength", [1.0; 4]),
        strength_night: vector("_ColorStrengthAtNight", [1.0; 4]),
        main_st: st(main),
        main2_st: st(main2),
        speed: [speed[0], speed[1], speed2[0], speed2[1]],
        fresnel_fade: float("_FresnelFade"),
        fresnel_pow: float("_FresnelPow"),
        cutoff: float("_Cutoff"),
        fresnel_clip: variant.fresnel_clip,
        mul_map: variant.mul_map,
        soft_edges: variant.soft_edges,
        deform: variant.deform,
        refract: variant.refract,
        inv_fade: float("_InvFade"),
        deform_strength: float("_DeformStrength"),
        refract_strength: float("_RefractStrength"),
        deform_st: st(deform),
        refract_st: st(refract),
        speed2: [
            deform_speed[0],
            deform_speed[1],
            refract_speed[0],
            refract_speed[1],
        ],
        textures: textures.map(|t| t.map(|(id, _)| id)),
    })
}

pub(crate) fn shader_port(name: &str) -> ShaderPort {
    match name {
        "MarmosetUBER" => ShaderPort::Ported,
        "UWE/Marmoset/IonCrystal" | "UWE/Marmoset/Mesmer" => ShaderPort::AsUber,
        "UWE/Particles/WBOIT-FakeVolumetricLight" => ShaderPort::Effect,
        _ => ShaderPort::NotPorted,
    }
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
        let mut desc = material_desc(&material, texture);
        let shader = self.shader_info(object, &material, out);
        if shader_port(&shader.name) == ShaderPort::Effect {
            let values = effect_values(&material, &shader);
            info!(
                "objects: effect material {:?} ({}): {values:?}",
                material.name, shader.name
            );
            desc.effect = Some(EffectLook::Glow(values));
        } else if shader.name == PARTICLES_UBER {
            // Each texture in its own colour space, as Unity samples it.
            let mut texture = |name: &str| {
                let t = material.texture(name).filter(|t| !t.texture.is_null())?;
                let target = self.assets.resolve(&object.file, t.texture).ok()??;
                let id = self.texture(&target, None, out)?;
                Some((id, [t.scale[0], t.scale[1], t.offset[0], t.offset[1]]))
            };
            let textures = [
                texture("_MainTex"),
                texture("_MainTex2"),
                texture("_DeformMap"),
                texture("_RefractMap"),
            ];
            match particles_values(&material, &shader, textures) {
                Ok(values) => {
                    info!(
                        "objects: effect material {:?} ({}): {values:?}",
                        material.name, shader.name
                    );
                    desc.effect = Some(EffectLook::Particles(values));
                }
                Err(why) => info!(
                    "objects: {:?} ({}) stays on the stand-in look: {why}",
                    material.name, shader.name
                ),
            }
        }
        let effect = desc.effect.is_some();
        let id = self.id();
        if effect {
            self.effect_materials.insert(id, desc.clone());
        }
        out.push(Update::Material {
            id,
            desc: Box::new(desc),
        });
        self.materials.insert(object.key(), id);
        self.material_shaders.insert(id, shader.name.clone());
        Some(id)
    }

    /// A material's shader: its name and property defaults, read once per
    /// shader object.
    fn shader_info(
        &mut self,
        material_object: &ObjectRef,
        material: &Material,
        out: &mut Vec<Update>,
    ) -> Arc<ShaderInfo> {
        let named = |name: &str| {
            Arc::new(ShaderInfo {
                name: name.into(),
                defaults: HashMap::new(),
            })
        };
        let shader = match self.assets.resolve(&material_object.file, material.shader) {
            Ok(Some(s)) => s,
            Ok(None) => return named("(no shader)"),
            Err(e) => {
                out.push(Update::Warning(format!("shader of {}: {e}", material.name)));
                return named("(unresolved shader)");
            }
        };
        if let Some(info) = self.shader_infos.get(&shader.key()) {
            return info.clone();
        }
        let parsed = shader.data().and_then(|(_, d)| {
            Shader::parse(d, shader.file.file().big_endian).map_err(|e| e.to_string())
        });
        let info = match parsed {
            Ok(s) => Arc::new(ShaderInfo {
                defaults: s
                    .properties
                    .iter()
                    .map(|p| (p.name.clone(), p.default))
                    .collect(),
                name: s.name,
            }),
            Err(e) => {
                out.push(Update::Warning(format!("shader of {}: {e}", material.name)));
                named("(unreadable shader)")
            }
        };
        self.shader_infos.insert(shader.key(), info.clone());
        info
    }

    /// The effect material `base` with the values a `VFXVolumetricLight`
    /// puts in its glow's property block (`docs/formats/materials.md`
    /// § Fake volumetric lights): `_Color` = the light's colour with alpha
    /// × light intensity / 8, and the script's intensity, start offset,
    /// start falloff, soft edges and near clip.
    fn glow_material(
        &mut self,
        base: u32,
        glow: &sn_assets::VolumetricGlow,
        out: &mut Vec<Update>,
    ) -> u32 {
        let (Some(light), Some(mut desc)) =
            (&glow.light, self.effect_materials.get(&base).cloned())
        else {
            return base;
        };
        let Some(EffectLook::Glow(v)) = desc.effect.as_mut() else {
            return base;
        };
        let s = &glow.script;
        let c = light.color;
        v.color = [c[0], c[1], c[2], c[3] * light.intensity / 8.0];
        v.intensity = s.intensity;
        v.offset = s.start_offset;
        v.fallof = s.start_fallof;
        v.inv_fade = s.soft_edges;
        v.clip_fade = s.near_clip;
        let values = *v;
        let bits = [
            values.color[0],
            values.color[1],
            values.color[2],
            values.color[3],
            values.intensity,
            values.offset,
            values.fallof,
            values.inv_fade,
            values.clip_fade,
        ]
        .map(f32::to_bits);
        if let Some(id) = self.glow_materials.get(&(base, bits)) {
            return *id;
        }
        info!("objects: volumetric light glow: {values:?}");
        let id = self.id();
        self.effect_materials.insert(id, desc.clone());
        if let Some(shader) = self.material_shaders.get(&base).cloned() {
            self.material_shaders.insert(id, shader);
        }
        out.push(Update::Material {
            id,
            desc: Box::new(desc),
        });
        self.glow_materials.insert((base, bits), id);
        id
    }

    /// The sub-meshes of an effect mesh (with vertex colours), sent as
    /// `Update::EffectMesh`; cached like `mesh`.
    fn effect_mesh(&mut self, object: &ObjectRef, out: &mut Vec<Update>) -> Vec<Option<u32>> {
        let key = object.key();
        if self.effect_meshes.contains_key(&(key.clone(), 0)) {
            return (0..)
                .map_while(|i| self.effect_meshes.get(&(key.clone(), i)).copied())
                .collect();
        }
        let geometry = match self.assets.mesh(object) {
            Ok((_, g)) => g,
            Err(e) => {
                out.push(Update::Warning(format!("effect mesh: {e}")));
                self.effect_meshes.insert((key, 0), None);
                return vec![None];
            }
        };
        if geometry.colors.len() != geometry.positions.len() {
            info!(
                "objects: effect mesh {} has no vertex colours; drawn with white",
                object.path_id
            );
        }
        let mut ids = Vec::new();
        for (i, data) in split_geometry(&geometry).into_iter().enumerate() {
            let id = self.id();
            out.push(Update::EffectMesh { id, data });
            self.effect_meshes.insert((key.clone(), i), Some(id));
            ids.push(Some(id));
        }
        ids
    }

    /// Counts a drawn part whose shader is not ported; logs a shader the
    /// first time it is seen.
    fn count_unported(&mut self, material: u32, node: &str, prefab: &str) {
        let Some(shader) = self.material_shaders.get(&material) else {
            return;
        };
        // Effect materials are ported (some `UWE/Particles/UBER` materials
        // are, others not: decided per material).
        if self.effect_materials.contains_key(&material)
            || matches!(shader_port(shader), ShaderPort::Ported | ShaderPort::Effect)
        {
            return;
        }
        let n = self.unported.entry(shader.clone()).or_default();
        *n += 1;
        self.unported_changed = true;
        if *n == 1 {
            info!(
                "objects: shader {shader:?} not ported yet, drawn with a stand-in look (first: {node:?} in {prefab})"
            );
        }
    }

    /// Logs the not-ported totals if they changed (called when the worker
    /// has loaded everything asked for).
    fn log_unported(&mut self) {
        if self.placeholders_spawned != self.placeholders_logged {
            self.placeholders_logged = self.placeholders_spawned;
            info!(
                "objects: placeholders spawned so far: {} ({} distinct prefabs)",
                self.placeholders_spawned,
                self.spawned_contents
                    .values()
                    .filter(|c| c.is_some())
                    .count()
            );
        }
        if std::mem::take(&mut self.unported_changed) {
            info!(
                "objects: shaders not ported (drawn parts in the prefabs and scenes loaded so far): {:?}",
                self.unported
            );
        }
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
        let mut ids = Vec::new();
        for (i, data) in split_geometry(geometry).into_iter().enumerate() {
            let id = self.id();
            out.push(Update::Mesh { id, data });
            self.meshes.insert((key.clone(), i), Some(id));
            ids.push(Some(id));
        }
        ids
    }

    /// A prefab's id; `spawn`: with what its placeholders spawn (false for a
    /// placement whose saved objects already hold it, `placeholders_saved`).
    fn prefab_as(&mut self, path: &str, spawn: bool, out: &mut Vec<Update>) -> Option<u32> {
        let key = (path.to_string(), spawn);
        if let Some(id) = self.prefabs.get(&key) {
            return *id;
        }
        let prefab = match self.assets.prefab(&self.catalog, path) {
            Ok(p) => p,
            Err(e) => {
                out.push(Update::Warning(format!("prefab: {e}")));
                self.prefabs.insert(key, None);
                self.placeholder_ids.insert(path.to_string(), Vec::new());
                return None;
            }
        };
        let ids = prefab
            .placeholder_groups
            .iter()
            .flat_map(|g| &g.placeholders)
            .filter_map(|&n| prefab.nodes[n].placeholder.clone())
            .collect();
        self.placeholder_ids.insert(path.to_string(), ids);
        let id = self.prefab_parts_as(&prefab, spawn, out);
        self.prefabs.insert(key, id);
        id
    }

    /// Whether the world's saved objects below `object` already hold what
    /// its prefab's placeholders spawn, so the game takes the group as
    /// initialized and spawns nothing
    /// (`PrefabPlaceholdersGroup.OnProtoDeserializeObjectTree`: more of its
    /// placeholders' prefabs found than missing, those of "Slots" prefabs
    /// not counted). The game wants each found object under the
    /// placeholder's own parent; the saved tree is matched as "below the
    /// object" (4 placements in the world, all found that way: `sn-inspect
    /// prefab --placeholders`). `below`: the class ids of the saved objects
    /// below it.
    fn placeholders_saved(&mut self, path: &str, below: &[&str], out: &mut Vec<Update>) -> bool {
        if below.is_empty() || path.starts_with("WorldEntities/Creatures/") {
            return false;
        }
        if !self.placeholder_ids.contains_key(path) {
            self.prefab_as(path, true, out);
        }
        let Some(ids) = self.placeholder_ids.get(path) else {
            return false;
        };
        let (mut found, mut missing) = (0, 0);
        for id in ids {
            let slots = self
                .class_paths
                .get(id)
                .is_some_and(|p| p.to_ascii_lowercase().contains("slots"));
            if slots {
                continue;
            }
            if below.contains(&id.as_str()) {
                found += 1;
            } else {
                missing += 1;
            }
        }
        if found > missing {
            info!("objects: placeholders of {path} already spawned in the saved world");
        }
        found > missing
    }

    /// Sends a loaded hierarchy's drawn parts and lights, with what its
    /// placeholders spawn; its id, `None` if it has nothing to draw.
    fn prefab_parts(&mut self, prefab: &Prefab, out: &mut Vec<Update>) -> Option<u32> {
        self.prefab_parts_as(prefab, true, out)
    }

    /// `prefab_parts`; `spawn`: with what its placeholders spawn.
    fn prefab_parts_as(
        &mut self,
        prefab: &Prefab,
        spawn: bool,
        out: &mut Vec<Update>,
    ) -> Option<u32> {
        let PrefabContent {
            parts,
            lights,
            directional,
        } = self.prefab_content(prefab, out, if spawn { 0 } else { NO_SPAWN });
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

    /// A hierarchy's drawn parts and lights relative to its root, with the
    /// prefabs its placeholders spawn (`depth`: how deep in spawned prefabs
    /// this one is).
    fn prefab_content(
        &mut self,
        prefab: &Prefab,
        out: &mut Vec<Update>,
        depth: usize,
    ) -> PrefabContent {
        let mut parts = Vec::new();
        for (index, node) in prefab.visible() {
            if node.layer >= 32 || self.culling_mask & (1 << node.layer) == 0 {
                info!(
                    "objects: {:?} in {} not drawn: layer {} is not in the main camera's culling mask",
                    node.name, prefab.key, node.layer
                );
                continue;
            }
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
            // Effect sub-meshes come from their own copy (with vertex
            // colours), made when the first one is met.
            let mut effect_meshes: Option<Vec<Option<u32>>> = None;
            // One material per sub-mesh; as Unity does, materials beyond
            // the sub-mesh count draw the last sub-mesh again (an extra
            // pass, e.g. the door force fields' second layer). Sub-meshes
            // without a material are not drawn.
            for (m, material) in node.materials.iter().enumerate() {
                let Some(last) = sub_meshes.len().checked_sub(1) else {
                    break;
                };
                let i = m.min(last);
                let (Some(sub_mesh), Some(material)) = (&sub_meshes[i], material) else {
                    continue;
                };
                if m > last {
                    info!(
                        "objects: {:?} in {}: material {m} draws sub-mesh {i} again (an extra pass)",
                        node.name, prefab.key
                    );
                }
                let Some(material) = self.material(material, out) else {
                    continue;
                };
                self.count_unported(material, &node.name, &prefab.key);
                let effect = self.effect_materials.contains_key(&material) && !node.skinned;
                let material = match &node.volumetric_light {
                    Some(glow) if effect && glow.sets_block => {
                        self.glow_material(material, glow, out)
                    }
                    _ => material,
                };
                let mesh_id = if effect {
                    let ids = effect_meshes.get_or_insert_with(|| self.effect_mesh(mesh, out));
                    match ids.get(i).copied().flatten() {
                        Some(id) => id,
                        None => continue,
                    }
                } else {
                    *sub_mesh
                };
                parts.push(Part {
                    mesh: mesh_id,
                    material,
                    local,
                    biome_sky,
                    effect,
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
        let mut content = PrefabContent {
            parts,
            lights,
            directional,
        };
        if depth != NO_SPAWN && self.spawn_placeholders {
            self.add_placeholders(prefab, &mut content, out, depth);
        }
        content
    }

    /// What `PrefabPlaceholdersGroup.Start` spawns into `prefab` in a new
    /// game (`docs/DESIGN.md` M7h), added to `content`: for an enabled group
    /// on an active node, each listed placeholder whose GameObject is active
    /// (`activeSelf`) and whose `prefabClassId` has a world entity info
    /// spawns that prefab under the placeholder's parent, at the
    /// placeholder's local transform. Spawned prefabs start too, so their
    /// own placeholders spawn. Creatures are left out, as for placed
    /// objects; a spawned prefab under an inactive parent is not drawn.
    fn add_placeholders(
        &mut self,
        prefab: &Prefab,
        content: &mut PrefabContent,
        out: &mut Vec<Update>,
        depth: usize,
    ) {
        for group in &prefab.placeholder_groups {
            let group_active = prefab.nodes.get(group.node).is_some_and(|n| n.active);
            if !group.enabled || !group_active {
                continue;
            }
            for &n in &group.placeholders {
                let Some(node) = prefab.nodes.get(n) else {
                    continue;
                };
                let Some(class_id) = node.placeholder.as_deref().filter(|c| !c.is_empty()) else {
                    continue;
                };
                if !node.active_self {
                    continue;
                }
                let skip = |why: &str| {
                    info!(
                        "objects: placeholder {:?} in {} not spawned: {why}",
                        node.name, prefab.key
                    );
                };
                if !self.entity_infos.contains_key(class_id) {
                    // `PrefabPlaceholder.Spawn` logs an error and returns.
                    skip(&format!("no world entity info for {class_id}"));
                    continue;
                }
                let Some(path) = self.class_paths.get(class_id).cloned() else {
                    skip(&format!("{class_id} is not in the prefab database"));
                    continue;
                };
                if path.starts_with("WorldEntities/Creatures/") {
                    continue;
                }
                if node.parent.is_some_and(|p| !prefab.nodes[p].active) {
                    continue;
                }
                if depth >= MAX_PLACEHOLDER_DEPTH {
                    out.push(Update::Warning(format!(
                        "placeholder {:?} in {}: spawned prefabs nested deeper than {MAX_PLACEHOLDER_DEPTH}",
                        node.name, prefab.key
                    )));
                    continue;
                }
                let Some(child) = self.placeholder_content(&path, out, depth + 1) else {
                    continue;
                };
                // The spawned root takes the placeholder's place.
                let at = node.in_prefab;
                content.parts.extend(child.parts.iter().map(|p| Part {
                    local: at.then(&p.local),
                    ..*p
                }));
                content
                    .lights
                    .extend(child.lights.iter().map(|l| LocalLight {
                        local: at.then(&l.local),
                        ..*l
                    }));
                content
                    .directional
                    .extend(child.directional.iter().map(|d| {
                        let mut d = d.clone();
                        d.local = at.then(&d.local);
                        d
                    }));
                self.placeholders_spawned += 1;
            }
        }
    }

    /// The content of a prefab spawned by a placeholder, cached by path
    /// (`None`: it failed to load, or it is being built further up, i.e. it
    /// would spawn itself).
    fn placeholder_content(
        &mut self,
        path: &str,
        out: &mut Vec<Update>,
        depth: usize,
    ) -> Option<Arc<PrefabContent>> {
        if let Some(c) = self.spawned_contents.get(path) {
            return c.clone();
        }
        self.spawned_contents.insert(path.to_string(), None);
        let prefab = match self.assets.prefab(&self.catalog, path) {
            Ok(p) => p,
            Err(e) => {
                out.push(Update::Warning(format!("placeholder prefab: {e}")));
                return None;
            }
        };
        let content = Arc::new(self.prefab_content(&prefab, out, depth));
        self.spawned_contents
            .insert(path.to_string(), Some(content.clone()));
        Some(content)
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
        self.still_prefab_as(path, true, out)
    }

    /// `still_prefab`; `spawn`: with what its placeholders spawn.
    fn still_prefab_as(&mut self, path: &str, spawn: bool, out: &mut Vec<Update>) -> Option<u32> {
        if path.starts_with("WorldEntities/Creatures/") {
            return None;
        }
        self.prefab_as(path, spawn, out)
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
                    let children = saved_children(tree);
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
                        let below = saved_below(tree, &children, &object.id);
                        let spawn = !self.placeholders_saved(&path, &below, out);
                        if let Some(prefab) = self.still_prefab_as(&path, spawn, out) {
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
                let children = saved_children(&tree);
                for (object, transform) in tree.objects.iter().zip(world) {
                    let Some(path) = self.class_paths.get(&object.class_id).cloned() else {
                        continue;
                    };
                    let below = saved_below(&tree, &children, &object.id);
                    let spawn = !self.placeholders_saved(&path, &below, out);
                    if let Some(prefab) = self.prefab_as(&path, spawn, out) {
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
        effect: None,
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
    placeholders: bool,
    scenes: Option<SceneOptions>,
) {
    let start = Instant::now();
    let setup = || -> Result<Library, String> {
        let assets = GameAssets::index(game)?;
        let catalog = assets.catalog()?;
        let class_paths = game.read_prefab_database().map_err(|e| e.0)?;
        let entity_infos = sn_assets::entity_infos(&assets)?;
        Ok(Library {
            assets,
            catalog,
            class_paths,
            prefabs: HashMap::new(),
            placeholder_ids: HashMap::new(),
            meshes: HashMap::new(),
            materials: HashMap::new(),
            textures: HashMap::new(),
            next_id: 0,
            slots: None,
            culling_mask: u32::MAX,
            shader_infos: HashMap::new(),
            effect_materials: HashMap::new(),
            glow_materials: HashMap::new(),
            effect_meshes: HashMap::new(),
            material_shaders: HashMap::new(),
            unported: BTreeMap::new(),
            unported_changed: false,
            entity_infos,
            spawned_contents: HashMap::new(),
            spawn_placeholders: placeholders,
            placeholders_spawned: 0,
            placeholders_logged: 0,
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
    match library.assets.main_camera() {
        Ok(camera) => {
            let hidden: Vec<u32> = (0..32).filter(|&l| !camera.draws_layer(l)).collect();
            info!(
                "main camera: culling mask {:#010x}, layers not drawn {hidden:?}",
                camera.culling_mask
            );
            library.culling_mask = camera.culling_mask;
        }
        Err(e) => {
            let _ = tx.send(Update::Warning(format!(
                "main camera: {e}; drawing every layer"
            )));
        }
    }
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
                // Idle: everything asked for is loaded.
                library.log_unported();
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
    /// `placeholders`: spawn what the objects' placeholders hold.
    /// `scenes`: load the scenes the game spawns at start (`None`: none).
    pub fn start(
        game: GameData,
        lights: bool,
        slot_seed: Option<u64>,
        placeholders: bool,
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
        std::thread::spawn(move || {
            worker(game, worker_shared, tx, slot_seed, placeholders, scenes)
        });
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

/// A mesh's sub-meshes in Bevy's coordinates (z flipped, winding reversed),
/// each with only the vertices it uses.
fn split_geometry(geometry: &sn_unity::MeshGeometry) -> Vec<MeshData> {
    fn pick<T: Copy>(attr: &[T], n: usize, used: &[usize]) -> Vec<T> {
        if attr.len() == n {
            used.iter().map(|&v| attr[v]).collect()
        } else {
            Vec::new()
        }
    }
    let flip = |p: [f32; 3]| [p[0], p[1], -p[2]];
    let positions: Vec<[f32; 3]> = geometry.positions.iter().map(|&p| flip(p)).collect();
    let normals: Vec<[f32; 3]> = geometry.normals.iter().map(|&n| flip(n)).collect();
    // Mirroring flips the bitangent: negate w too.
    let tangents: Vec<[f32; 4]> = geometry
        .tangents
        .iter()
        .map(|t| [t[0], t[1], -t[2], -t[3]])
        .collect();
    let n = positions.len();
    let mut meshes = Vec::new();
    for indices in &geometry.sub_meshes {
        let mut remap = vec![u32::MAX; n];
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
        meshes.push(MeshData {
            positions: pick(&positions, n, &used),
            normals: pick(&normals, n, &used),
            tangents: pick(&tangents, n, &used),
            colors: pick(&geometry.colors, n, &used),
            uvs: pick(&geometry.uv0, n, &used),
            indices: local
                .chunks_exact(3)
                .flat_map(|t| [t[0], t[2], t[1]])
                .collect(),
        });
    }
    meshes
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
    mut effect_meshes: ResMut<EffectMeshes>,
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
                streamer.material_descs.insert(id, *desc);
            }
            Update::Mesh { id, data } => {
                streamer.meshes.insert(id, meshes.add(build_mesh(data)));
            }
            Update::EffectMesh { id, data } => {
                effect_meshes.0.insert(id, Arc::new(data));
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
            if part.effect {
                let Some(look) = self
                    .material_descs
                    .get(&part.material)
                    .and_then(|d| d.effect)
                else {
                    continue;
                };
                // A texture slot without a texture: the shader's default,
                // white.
                let texture = |id: Option<u32>| {
                    id.and_then(|id| self.textures.get(&id).cloned())
                        .unwrap_or_else(|| s.defaults.white.clone())
                };
                let textures = match look {
                    EffectLook::Particles(v) => v.textures.map(&texture),
                    EffectLook::Glow(_) => std::array::from_fn(|_| s.defaults.white.clone()),
                };
                let world = inst.transform.then(&part.local);
                let marker = EffectPart {
                    mesh: part.mesh,
                    look,
                    textures,
                };
                entities.push(s.commands.spawn((marker, to_bevy(&world))).id());
                continue;
            }
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

    /// A `UWE/Particles/UBER` material as stored: `keywords`, floats and
    /// colours (vectors live with the colours in Unity's material).
    fn particles_material(keywords: &str, floats: &[(&str, f32)]) -> Material {
        let mut all = vec![
            ("_SrcBlend", 1.0),
            ("_DstBlend", 1.0),
            ("_SrcBlend2", 0.0),
            ("_DstBlend2", 10.0),
            ("_Ztest", 2.0),
            ("_MyCullVariable", 0.0),
            ("_FresnelFade", 3.0),
            ("_FresnelPow", 1.0),
        ];
        for &(name, v) in floats {
            all.retain(|(n, _)| *n != name);
            all.push((name, v));
        }
        Material {
            name: "test".into(),
            shader: sn_unity::PPtr {
                file_id: 0,
                path_id: 0,
            },
            keywords: keywords.into(),
            custom_render_queue: 3101,
            tags: Vec::new(),
            textures: Vec::new(),
            floats: all.into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
            colors: vec![
                ("_Color".into(), [0.2, 0.9, 0.3, 1.0]),
                ("_ColorStrength".into(), [1.0, 1.0, 1.0, 0.25]),
                ("_MainTex_Speed".into(), [0.1, -0.2, 0.0, 0.0]),
                ("_MainTex2_Speed".into(), [0.3, 0.4, 0.0, 0.0]),
            ],
        }
    }

    fn uber_shader() -> ShaderInfo {
        ShaderInfo {
            name: PARTICLES_UBER.into(),
            defaults: [("_ColorStrengthAtNight".to_string(), [2.0, 2.0, 2.0, 1.0])].into(),
        }
    }

    #[test]
    fn decoded_particles_variants_are_drawn_by_the_effect_pass() {
        // The halo's set, in any keyword order.
        let m = particles_material("WBOIT FX_SCROLL FX_MULMAP FX_FRESNELCLIP FX_ADDFOG", &[]);
        let tex = [Some((7, [20.0, 2.0, 0.0, 0.0])), None, None, None];
        let v = particles_values(&m, &uber_shader(), tex).unwrap();
        assert!(v.fresnel_clip && v.mul_map);
        assert_eq!(v.color, [0.2, 0.9, 0.3, 1.0]);
        // Vectors as stored, the shader's default where the material has
        // none.
        assert_eq!(v.strength, [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(v.strength_night, [2.0, 2.0, 2.0, 1.0]);
        assert_eq!(v.speed, [0.1, -0.2, 0.3, 0.4]);
        assert_eq!(v.main_st, [20.0, 2.0, 0.0, 0.0]);
        assert_eq!(v.main2_st, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(v.textures, [Some(7), None, None, None]);
        assert!(!v.soft_edges && !v.deform && !v.refract);
        assert_eq!((v.fresnel_fade, v.fresnel_pow), (3.0, 1.0));
        let m = particles_material("FX_ADDFOG FX_MULMAP FX_SCROLL WBOIT", &[]);
        let v = particles_values(&m, &uber_shader(), [None; 4]).unwrap();
        assert!(!v.fresnel_clip && v.mul_map);
        let m = particles_material("FX_ADDFOG FX_SCROLL WBOIT", &[]);
        let v = particles_values(&m, &uber_shader(), [None; 4]).unwrap();
        assert!(!v.fresnel_clip && !v.mul_map);
    }

    #[test]
    fn door_force_field_variants_are_drawn_by_the_effect_pass() {
        let floats = [
            ("_InvFade", 1.6),
            ("_DeformStrength", 0.005),
            ("_RefractStrength", 0.02),
        ];
        let mut m = particles_material(
            "FX_ADDFOG FX_DEFORM FX_MULMAP FX_REFRACTMAP FX_SCROLL FX_SOFTEDGES WBOIT",
            &floats,
        );
        // The materials also hold a vector named `_RefractStrength`; the
        // programs read the float.
        m.colors
            .push(("_RefractStrength".into(), [0.01, 0.01, 0.01, 0.005]));
        m.colors
            .push(("_DeformMap_Speed".into(), [0.0, 0.5, 0.0, 0.0]));
        m.colors
            .push(("_RefractMap_Speed".into(), [0.0, 0.25, 0.0, 0.0]));
        let tex = [
            None,
            None,
            Some((3, [0.2, 30.0, 0.0, 0.0])),
            Some((4, [0.1, 60.0, 0.0, 0.0])),
        ];
        let v = particles_values(&m, &uber_shader(), tex).unwrap();
        assert!(v.mul_map && v.soft_edges && v.deform && v.refract && !v.fresnel_clip);
        assert_eq!(
            (v.inv_fade, v.deform_strength, v.refract_strength),
            (1.6, 0.005, 0.02)
        );
        assert_eq!(v.deform_st, [0.2, 30.0, 0.0, 0.0]);
        assert_eq!(v.refract_st, [0.1, 60.0, 0.0, 0.0]);
        assert_eq!(v.speed2, [0.0, 0.5, 0.0, 0.25]);
        assert_eq!(v.textures, [None, None, Some(3), Some(4)]);
        // The second layer: the same without FX_REFRACTMAP.
        let m = particles_material(
            "FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT",
            &floats,
        );
        let v = particles_values(&m, &uber_shader(), [None; 4]).unwrap();
        assert!(v.soft_edges && v.deform && !v.refract);
    }

    #[test]
    fn other_particles_materials_keep_the_stand_in_look() {
        // A keyword set not decoded (here: no FX_MULMAP but FX_FRESNELCLIP).
        let m = particles_material("FX_ADDFOG FX_FRESNELCLIP FX_SCROLL WBOIT", &[]);
        assert!(particles_values(&m, &uber_shader(), [None; 4]).is_err());
        // Render state the pass does not implement.
        for (name, v) in [
            ("_MyCullVariable", 2.0),
            ("_Ztest", 4.0),
            ("_DstBlend", 10.0),
            ("_SrcBlend2", 1.0),
        ] {
            let m = particles_material("FX_ADDFOG FX_SCROLL WBOIT", &[(name, v)]);
            let why = particles_values(&m, &uber_shader(), [None; 4]).unwrap_err();
            assert!(why.starts_with(name), "{why}");
        }
    }

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
