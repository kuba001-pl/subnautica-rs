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
//!
//! Items (M9d4b): with item rules, each pickup and outcrop instance keeps
//! its `EntityKey`; a gone key's entities are despawned (and never
//! spawned again), and the objects the server made are drawn at their
//! point ([`ObjectStreamer::hide_keys`], [`ObjectStreamer::set_spawned`]).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{DynamicSkinnedMeshBounds, NoAutoAabb, RenderLayers};
use bevy::math::Affine2;
use bevy::mesh::morph::{MAX_MORPH_WEIGHTS, MeshMorphWeights, MorphAttributes};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};
use sn_anim::{Animator, Program, SlotKind};
use sn_assets::{
    Assets as GameAssets, AuroraShow, BiomeSky, LootTable, MarmoSkies, ObjectRef, Prefab,
    SHADOWS_OFF, SHADOWS_ON, SHADOWS_ONLY, marmo_skies,
};
use sn_install::GameData;
use sn_protocol::EntityKey;
use sn_sim::inventory::ItemRules;
use sn_unity::{Catalog, DayNightLight, LightKind, Material, SKIES_AUTO, Shader};
use sn_world::{BatchCoord, EntityInfo, SLOTS_COMPONENT, Transform as Placement};

use crate::animation::{AnimatedRig, RigDesc, ShapeChannel, ShapeDesc, SkinDesc, bindpose_to_bevy};
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
const SPAWN_BUDGET: usize = 1_500;

/// Maximum texture bytes added to `Assets<Image>` per frame so GPU texture
/// uploads in `RenderSystems::PrepareAssets` do not stall a frame (P1d).
const TEXTURE_UPLOAD_BYTES_PER_FRAME: usize = 8 * 1024 * 1024;

/// Maximum object mesh vertices added to `Assets<Mesh>` per frame so vertex
/// slab uploads in `allocate_and_free_meshes` do not stall a frame (P1d).
const MESH_UPLOAD_VERTICES_PER_FRAME: usize = 32_000;

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
    /// Bone indices and weights for GPU skinning (empty for still
    /// meshes; M7f4c).
    pub(crate) joints: Vec<[u16; 4]>,
    pub(crate) weights: Vec<[f32; 4]>,
    /// Blend shape frames as morph targets, one target after another
    /// (`targets × vertices`; empty for most meshes; M7f4d): position,
    /// normal and tangent offsets.
    pub(crate) morphs: Vec<[[f32; 3]; 3]>,
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
    cull_mode: Option<Face>,
    /// MarmosetUBER's values; `None` for materials of other shaders.
    pub(crate) uber: Option<UberValues>,
    sig: Option<crate::sig::SigLook>,
    /// Values of a shader drawn by our effect pass (`effects.rs`).
    pub(crate) effect: Option<EffectLook>,
    /// A material of the player's view model (M9u8a4a: the PDA's); blended,
    /// it writes depth ([`crate::object_look::ObjectExtension::depth_write`]).
    pub(crate) view_model: bool,
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
    pub(crate) exposure: [f32; 4],
    rotation: [f32; 4],
    pub(crate) affected: bool,
    outdoors: bool,
    pub(crate) sh: [[f32; 3]; 9],
}

/// Which sky a material is made with (no sky known: Marmoset's defaults).
/// `Global` and `Pod` change at run time (M7f4g): their materials are
/// relit in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SkyKey {
    /// One of the biome skies.
    Fixed(usize),
    /// `SkyManager.GlobalSky`: Safe Shallows, or the lifepod's sky while
    /// the player is in the pod (`MarmoLifepodSky`).
    Global,
    /// The lifepod's sky (`anchorSky`, its intensities set by the pod's
    /// `LightingController`), for appliers below the pod.
    Pod,
}

impl SkyLook {
    pub(crate) fn new(s: &BiomeSky) -> SkyLook {
        SkyLook {
            exposure: s.sky.exposure(),
            rotation: s.rotation,
            affected: s.sky.affected_by_day_night,
            outdoors: s.sky.outdoors,
            sh: s.sky.sh_buffer(),
        }
    }
}

/// The skies objects are lit with, as the main thread needs them.
#[derive(Default)]
struct SkySet {
    looks: Vec<SkyLook>,
    skies: MarmoSkies,
    /// The lifepod's sky now (M7f4g).
    pod: Option<SkyLook>,
    /// The player is in the pod: the global sky is the pod's.
    in_pod: bool,
}

impl SkySet {
    fn new(skies: MarmoSkies) -> SkySet {
        let looks = skies.skies.iter().map(SkyLook::new).collect();
        SkySet {
            looks,
            skies,
            pod: None,
            in_pod: false,
        }
    }

    /// The sky of a part: for parts a `SkyApplier` covers, the pod's below
    /// the pod, else the biome's at the object; else the global one.
    fn pick(&self, biome_sky: bool, biome: Option<&str>, pod_sky: bool) -> SkyKey {
        if biome_sky && pod_sky {
            SkyKey::Pod
        } else if biome_sky {
            match self.skies.for_biome(biome) {
                Some(i) => SkyKey::Fixed(i),
                None => SkyKey::Global,
            }
        } else {
            SkyKey::Global
        }
    }

    /// The values of a sky key now.
    fn look(&self, key: SkyKey) -> Option<&SkyLook> {
        match key {
            SkyKey::Fixed(i) => self.looks.get(i),
            SkyKey::Global if self.in_pod => self.pod.as_ref(),
            SkyKey::Global => self.skies.global.and_then(|i| self.looks.get(i)),
            SkyKey::Pod => self.pod.as_ref(),
        }
    }
}

/// A point or spot light of a prefab, placed relative to the prefab root.
#[derive(Clone, Copy)]
pub(crate) struct LocalLight {
    pub(crate) spot: bool,
    /// Unity's `_LightColor`: `linear(colour × intensity)` (the game's
    /// `m_LightsUseLinearIntensity` is off).
    pub(crate) color: [f32; 3],
    pub(crate) range: f32,
    /// Full cone angle, degrees (spots).
    pub(crate) spot_angle: f32,
    pub(crate) local: Placement,
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
    waterfall_cap_height: Option<f32>,
    /// Lit with the sky of the biome it stands in (a `SkyApplier` lists
    /// it); else with the global sky.
    biome_sky: bool,
    sky_position: [f32; 3],
    /// Drawn by our effect pass (`effects.rs`): `mesh` is an effect mesh id
    /// (`Update::EffectMesh`) and the material has `effect` values.
    effect: bool,
    /// Moved by an animator (M7f4c): the prefab's rig and, for a still
    /// mesh, the rig node it hangs on (`local` is then the identity);
    /// for a skinned mesh, its skin in the rig.
    rig: Option<RigPart>,
    /// Its blend shapes driven by that rig (M7f4d): the index in the rig's
    /// `shapes`; `mesh` then has the morph targets.
    shape: Option<u16>,
    /// The renderer's `m_CastShadows` (M7f4e): off casts no sun shadow.
    shadows: u8,
    /// Its LOD group (index in the prefab's [`LodDesc`]s) and the levels
    /// that show it (bit `n`: level `n`), M7f4f.
    lod: Option<(u16, u32)>,
}

/// A `LODGroup` of a prefab (M7f4f), relative to the prefab's root.
#[derive(Clone, Debug)]
pub struct LodDesc {
    /// The group's node.
    in_prefab: Placement,
    reference_point: [f32; 3],
    size: f32,
    heights: Arc<[f32]>,
}

/// One LOD group of a spawned instance (Bevy world space).
struct LodState {
    center: Vec3,
    size: f32,
    heights: Arc<[f32]>,
    /// The level shown (`None`: culled).
    level: Option<usize>,
}

/// A spawned instance's LOD groups and the parts they switch (M7f4f).
#[derive(Component)]
pub struct LodSwitch {
    groups: Vec<LodState>,
    /// (group, levels mask, entity, triangles).
    members: Vec<(u16, u32, Entity, u32)>,
}

/// The game's LOD settings: `QualitySettings` level High (M7f4f).
#[derive(Resource, Clone, Copy, Debug)]
pub struct LodSettings {
    pub bias: f32,
    pub max_level: usize,
}

/// LOD switching over the last frames.
#[derive(Resource, Default)]
pub struct LodStats {
    /// Groups per shown level (index 0–3, then 4 and more), culled.
    levels: [usize; 5],
    culled: usize,
    groups: usize,
    switches: usize,
    /// Triangles of LOD group parts shown now, and with every group at
    /// LOD 0.
    triangles: u64,
    triangles_lod0: u64,
    micros: Vec<u128>,
}

impl LodStats {
    /// Current selection only, excluding timing samples and interval counters.
    pub fn snapshot(&self) -> ([usize; 5], usize, usize) {
        (self.levels, self.culled, self.groups)
    }
}

/// What the client's exploder needs from the Aurora scene and the game's
/// code (M7f4e).
#[derive(Clone, Copy, Debug)]
pub struct AuroraData {
    pub code: sn_assets::ExploderCode,
    /// `ShipExteriorCullManager.updateEveryXFrames`; `None`: no manager runs.
    pub cull_every: Option<i32>,
}

/// The Aurora's parts that show only in some states, with their entities.
pub type AuroraParts<'a> = Vec<(AuroraShow, &'a [Entity])>;

/// A box of a `ShipExteriorCull` (M7f4e), relative to the prefab's root.
#[derive(Clone, Copy, Debug)]
pub struct CullDesc {
    /// The box collider's node.
    in_prefab: Placement,
    center: [f32; 3],
    size: [f32; 3],
}

impl CullDesc {
    /// In the world with the prefab's root at `placement`.
    pub fn world(&self, placement: &Placement) -> sn_sim::aurora::CullBox {
        let t = placement.then(&self.in_prefab);
        let v = sn_sim::V3::from_f32;
        sn_sim::aurora::CullBox {
            center: v(t.transform_point(self.center)),
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
                .map(|a| v(t.rotate_vector(a))),
            half: [0, 1, 2].map(|a| f64::from(self.size[a] * t.scale[a].abs() * 0.5)),
        }
    }
}

/// A spawned `ShipExteriorCull`: registered with the Aurora's cull while
/// it exists, as the game's `Start` and `OnDestroy` do.
#[derive(Component)]
pub struct ExteriorCullVolume(pub Vec<sn_sim::aurora::CullBox>);

/// A drawn part whose renderer casts no shadow (`m_CastShadows` off): the
/// batches' shadow switch leaves it alone.
#[derive(Component)]
pub struct ShadowsOff;

/// The render layer of "shadows only" renderers (`m_CastShadows` 3, M9g4):
/// the sun sees it (its shadow pass picks casters by its `RenderLayers`),
/// the camera (layer 0) does not, so they cast shadows and are not drawn.
pub const SHADOW_ONLY_LAYER: usize = 1;

/// A drawn part on [`SHADOW_ONLY_LAYER`].
#[derive(Component)]
pub struct ShadowsOnly;

/// The base of the player body's rig (M9g4): `crate::body` moves it and
/// sets its animator's parameters every frame.
#[derive(Component)]
pub struct PlayerBodyRig;

/// On a rig's base: the node entities of [`RigDesc::switched`], spawned
/// hidden (M9u8a3: `crate::body` shows the local player's PDA while the
/// game's would be active).
#[derive(Component)]
pub struct SwitchedNodes(pub Vec<Entity>);

/// On a rig's base: the node entities of [`RigDesc::named`] by name
/// (M9u8a4a: the PDA's `ScreenAnchor`, where its screen's canvas sits).
#[derive(Component)]
pub struct NamedNodes(pub Vec<(String, Entity)>);

/// The base of another player's body rig (M10h2), by player id:
/// `crate::remote` moves it and sets its animator's parameters.
#[derive(Component)]
pub struct RemoteBodyRig(pub u16);

/// The player's body (M9g4): the `Player` hierarchy of the main scene.
#[derive(Clone, Copy)]
pub struct BodyOptions {
    /// Draw the head (the game's `Player.SetHeadVisible(true)`, as its
    /// Cyclops cameras do; our third-person camera); else it casts
    /// shadows only, as the game stores it.
    pub head_visible: bool,
}

/// The body's prefab, and whether it was spawned.
#[derive(Clone, Copy)]
struct BodyObjects {
    prefab: u32,
    spawned: bool,
}

#[derive(Clone, Copy)]
enum RigPart {
    Node { rig: u16, node: u16 },
    Skin { rig: u16, skin: u16 },
}

impl RigPart {
    fn rig(self) -> u16 {
        match self {
            RigPart::Node { rig, .. } | RigPart::Skin { rig, .. } => rig,
        }
    }

    /// The same part when the prefab's rigs come after `first` others.
    fn shifted(self, first: u16) -> RigPart {
        match self {
            RigPart::Node { rig, node } => RigPart::Node {
                rig: rig + first,
                node,
            },
            RigPart::Skin { rig, skin } => RigPart::Skin {
                rig: rig + first,
                skin,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Instance {
    level: usize,
    prefab: u32,
    transform: Placement,
    /// Spawned by a spawn slot (not placed in the cells).
    from_slot: bool,
    /// Below Lifepod 5 (its spawned modules): appliers take the pod's sky.
    pod_sky: bool,
}

/// Which scenes the worker loads, and their state.
#[derive(Clone, Copy)]
pub struct SceneOptions {
    /// Where Lifepod 5 starts (`None`: no lifepod).
    pub lifepod: Option<[f32; 3]>,
    /// The pod's lighting state (M7f4g, `sn_sim::lighting`).
    pub lifepod_state: usize,
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

/// A spawned rig: its base, its nodes, its drawn parts, and those with
/// blend shapes (with their index in [`RigDesc::shapes`]).
type RigEntities = (Entity, Vec<Entity>, Vec<Entity>, Vec<(Entity, u16)>);

/// A skinned mesh bent by a rig on the GPU: (rig, skin, blend shapes,
/// sub-meshes); see `Library::gpu_skin`.
type GpuSkin = (u16, u16, Option<u16>, Vec<Option<u32>>);

/// A rig while a prefab's content is put together.
struct RigBuild {
    /// The animator's node in the prefab.
    animator: usize,
    /// Prefab node → rig node.
    node_map: HashMap<usize, u16>,
    /// Prefab node → its blend shapes in `desc.shapes`.
    shape_nodes: HashMap<usize, u16>,
    desc: RigDesc,
}

/// What a prefab draws and lights, relative to its root.
struct PrefabContent {
    parts: Vec<Part>,
    lights: Vec<LocalLight>,
    directional: Vec<DirectionalSource>,
    rigs: Vec<Arc<RigDesc>>,
    culls: Vec<CullDesc>,
    atmospheres: Vec<sn_assets::AtmosphereTemplate>,
    lods: Vec<LodDesc>,
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
        image: Option<Image>,
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
        rigs: Vec<Arc<RigDesc>>,
        culls: Vec<CullDesc>,
        atmospheres: Vec<sn_assets::AtmosphereTemplate>,
        lods: Vec<LodDesc>,
    },
    /// The game's LOD bias and most detailed level (M7f4f).
    Lod(LodSettings),
    Batch {
        coord: BatchCoord,
        instances: Vec<Instance>,
        /// The instances that are items: index and key (M9d4b).
        keys: Vec<(usize, EntityKey)>,
        ms: f32,
    },
    /// A server-made object's instance (`None`: no prefab for its class).
    Spawned {
        id: u32,
        instance: Option<Instance>,
    },
    /// A scene's top-level objects; `shows`: for each instance, the
    /// Aurora states it shows in (M7f4e; [`AuroraShow::ALWAYS`] outside
    /// the Aurora).
    Scene {
        summary: String,
        instances: Vec<Instance>,
        shows: Vec<AuroraShow>,
        aurora: Option<AuroraData>,
        /// Lifepod 5's sky, lighting controller and lights (M7f4g).
        lifepod: Option<Box<crate::lifepod_light::PodLightData>>,
        ms: f32,
    },
    /// The player's body (M9g4; `None`: nothing to draw).
    Body {
        prefab: Option<u32>,
        summary: String,
        ms: f32,
    },
    /// The game's main camera: its near plane (m).
    Camera {
        near: f32,
    },
    Ready {
        ms: f32,
    },
    Warning(String),
    Error(String),
}

/// Sends worker updates to the main thread as soon as each asset is ready.
struct Out<'a>(&'a Sender<Update>);

impl Out<'_> {
    fn push(&mut self, update: Update) {
        let _ = self.0.send(update);
    }
}

/// What the worker knows: the game's assets and everything already sent.
struct Library {
    assets: GameAssets<'static>,
    atmosphere_layout: Arc<sn_unity::StructLayout>,
    /// Which classes are items (M9d4b).
    items: Option<Arc<ItemRules>>,
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
    /// Prefab nodes `Library::rigs` marks as switched (M9u8a3) and by
    /// name (M9u8a4a), set only while the player's body is built.
    switch_nodes: Vec<usize>,
    name_nodes: Vec<(usize, String)>,
    /// The PDA's materials (M9u8a4a): [`MaterialDesc::view_model`].
    view_model_materials: std::collections::HashSet<Key>,
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
    /// Run the objects' animators (off with `--no-animation`; M7f4c).
    animate: bool,
    /// Controller → its compiled program (`None`: failed to load).
    programs: HashMap<Key, Option<Arc<Program>>>,
    /// Clips shared between controllers.
    clips: HashMap<Key, Arc<sn_unity::AnimationClip>>,
    /// Skinned meshes with their bone weights, as `meshes`.
    skin_meshes: HashMap<(Key, usize), Option<u32>>,
    /// Meshes with their blend shapes as morph targets (and their bone
    /// weights if the bool is set), as `meshes` (M7f4d).
    morph_meshes: HashMap<(Key, bool, usize), Option<u32>>,
    /// The project clamps blend shape weights to 0–100.
    blend_clamp: bool,
}

/// How far a shader is ported (`docs/formats/materials.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderPort {
    /// MarmosetUBER or the two opaque SIG families: a supported surface path.
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
    pub cull: Option<sn_unity::ShaderValue>,
}

fn cull_mode(value: f32) -> Option<Face> {
    match value {
        0.0 => None,
        1.0 => Some(Face::Front),
        _ => Some(Face::Back),
    }
}

/// A fixed pass state wins over obsolete saved properties; a property
/// state takes its material override, then the shader's declared default.
fn shader_cull(material: &Material, shader: &ShaderInfo) -> Option<Face> {
    let value = match &shader.cull {
        Some(state) if state.property.is_empty() => state.value,
        Some(state) => material
            .float(&state.property)
            .or_else(|| shader.defaults.get(&state.property).map(|v| v[0]))
            .unwrap_or(state.value),
        None => material.float("_MyCullVariable").unwrap_or(2.0),
    };
    cull_mode(value)
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

/// What a decoded `UWE/Particles/UBER` keyword set switches on: each
/// keyword is one term of the same program.
#[derive(Clone, Copy, Default)]
struct ParticlesVariant {
    pixel_lighting: bool,
    fresnel_clip: bool,
    mul_map: bool,
    soft_edges: bool,
    deform: bool,
    refract: bool,
    triplanar: bool,
    near_clip: bool,
    add_fog: bool,
    alpha_pow: bool,
    scroll: bool,
}

/// The `UWE/Particles/UBER` keyword sets our effect pass draws, each read
/// from the game's compiled programs (`docs/formats/materials.md`
/// § `UWE/Particles/UBER` meshes): the consoles' holograms, the doors' and
/// control room's force fields, Tree Cove and Lost River brine (M7g4–c),
/// the tech light cones, the Gun elevator's smoke, the thermal reactor's
/// electricity, the lava falls and the orange key terminal's unscrolled
/// symbol (M7g4d). Lost River's saved
/// FX_LIGHT_2SIDED/FX_LIGHT_NORMALMAP keywords are absent from the installed
/// shader's compiled keyword table; they do not select lighting programs.
/// Only these exact sets are drawn: a keyword's term is not assumed to
/// combine with others until its program is read.
const PARTICLES_SETS: [&str; 15] = [
    "FX_LIGHTMODE_PIXEL FX_MULMAP FX_NEARCLIP FX_REFRACTMAP FX_SCROLL WBOIT",
    "FX_ADDFOG FX_FRESNELCLIP FX_MULMAP FX_SCROLL WBOIT",
    "FX_ADDFOG FX_MULMAP FX_SCROLL WBOIT",
    "FX_ADDFOG FX_SCROLL WBOIT",
    "FX_ADDFOG FX_DEFORM FX_MULMAP FX_REFRACTMAP FX_SCROLL FX_SOFTEDGES WBOIT",
    "FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT",
    "FX_ADDFOG FX_DEFORM FX_MULMAP FX_REFRACTMAP FX_SCROLL WBOIT",
    "FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL WBOIT",
    "FX_DEFORM FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT",
    "FX_DEFORM FX_LIGHT_2SIDED FX_LIGHT_NORMALMAP FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT",
    "FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT",
    "FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT",
    "FX_ADDFOG FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_SCROLL WBOIT",
    "FX_ADDFOG FX_ALPHAPOW FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT",
    "FX_ADDFOG WBOIT",
];

/// The terms of `keywords` (sorted, space-separated) if it is one of
/// [`PARTICLES_SETS`].
fn particles_variant(keywords: &str) -> Option<ParticlesVariant> {
    if !PARTICLES_SETS.contains(&keywords) {
        return None;
    }
    let mut v = ParticlesVariant::default();
    for k in keywords.split(' ') {
        match k {
            "FX_LIGHTMODE_PIXEL" => v.pixel_lighting = true,
            "FX_FRESNELCLIP" => v.fresnel_clip = true,
            "FX_MULMAP" => v.mul_map = true,
            "FX_SOFTEDGES" => v.soft_edges = true,
            "FX_DEFORM" => v.deform = true,
            "FX_REFRACTMAP" => v.refract = true,
            "FX_TRIPLANAR" => v.triplanar = true,
            "FX_NEARCLIP" => v.near_clip = true,
            "FX_ADDFOG" => v.add_fog = true,
            "FX_ALPHAPOW" => v.alpha_pow = true,
            "FX_SCROLL" => v.scroll = true,
            // On in every set; inactive saved keywords (see above).
            "WBOIT" | "FX_LIGHT_2SIDED" | "FX_LIGHT_NORMALMAP" => {}
            _ => return None,
        }
    }
    Some(v)
}

/// The game's mesh-effect shader; ported per material (`particles_values`).
const PARTICLES_UBER: &str = "UWE/Particles/UBER";

/// Confirmed lit programs. Recognition is an input diagnostic, not yet
/// permission to draw them: their four per-renderer lights are still missing.
fn particles_lighting_contract(
    material: &Material,
    shader: &ShaderInfo,
    keywords: &str,
) -> Option<Result<String, String>> {
    let (stage, program) = match keywords {
        "FX_FRESNELCLIP FX_LIGHTMODE_OFF FX_LIGHTMODE_VERTEX FX_MULMAP FX_REFRACTMAP FX_SCROLL FX_SOFTEDGES WBOIT" => {
            ("vertex", "119/437")
        }
        "FX_DEFORM FX_FRESNELCLIP FX_LIGHTMODE_PIXEL FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRAILNOISE WBOIT" => {
            ("pixel", "142/460")
        }
        "FX_LIGHTMODE_PIXEL FX_MULMAP FX_NEARCLIP FX_REFRACTMAP FX_SCROLL WBOIT" => {
            ("pixel", "148/466")
        }
        "FX_DEFORM FX_ISMESH FX_LIGHTMODE_PIXEL FX_LIGHT_2SIDED FX_MULMAP FX_QUALITY_HIGH FX_SCROLL FX_TRIPLANAR WBOIT" => {
            ("pixel", "154/472")
        }
        _ => return None,
    };
    let mut values = Vec::new();
    for name in [
        "_AffectedByDayNightCycle",
        "_LightDesaturation",
        "_NormalSharpness",
        "_SelfIllumination",
        "_LightWrapAround",
    ] {
        let Some(value) = material
            .float(name)
            .or_else(|| shader.defaults.get(name).map(|v| v[0]))
        else {
            return Some(Err(format!("lit program {program}: missing {name}")));
        };
        if !value.is_finite() {
            return Some(Err(format!("lit program {program}: non-finite {name}")));
        }
        values.push(format!("{name}={value}"));
    }
    Some(Ok(format!(
        "lit program {program} ({stage}), {}; renderer light inputs not connected (M7g4e)",
        values.join(", ")
    )))
}

/// A `UWE/Particles/UBER` material's values if our effect pass draws it as
/// the game does: one of the decoded keyword sets, and the render state the
/// pass implements (blend `_SrcBlend` 1 `_DstBlend` 1 `_SrcBlend2` 0
/// `_DstBlend2` 10, `_Ztest` 2 = Less, `_MyCullVariable` 0 = both sides or
/// 2 = back culling; culling is pass state, so any decoded set takes either).
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
    if let Some(contract) = particles_lighting_contract(material, shader, &keywords) {
        match contract {
            Err(error) => return Err(error),
            Ok(diagnostic) if keywords != PARTICLES_SETS[0] => return Err(diagnostic),
            Ok(_) => {}
        }
    }
    let Some(variant) = particles_variant(&keywords) else {
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
    ];
    for (name, wanted) in state {
        let v = float(name);
        if v != wanted {
            return Err(format!("{name} {v} (the pass draws {wanted})"));
        }
    }
    let back_cull = match float("_MyCullVariable") {
        0.0 => false,
        2.0 => true,
        v => return Err(format!("_MyCullVariable {v} (the pass draws 0 or 2)")),
    };
    // The pass's missing-texture fallback is white, while this shader
    // declares a neutral bump for refraction. Do not invent an offset when
    // an active refraction map is absent or failed to load.
    if variant.refract && textures[3].is_none() {
        return Err("missing _RefractMap (neutral bump default not implemented)".into());
    }
    let vector = |name: &str, default: [f32; 4]| {
        material
            .color(name)
            .or_else(|| shader.defaults.get(name).copied())
            .unwrap_or(default)
    };
    let st = |t: Option<(u32, [f32; 4])>| t.map_or([1.0, 1.0, 0.0, 0.0], |(_, st)| st);
    let [main, main2, deform, refract] = textures;
    // Without FX_SCROLL the program has no `frac(t × speed)` term: the
    // saved speeds are not read.
    let speed = |name: &str| {
        if variant.scroll {
            vector(name, [0.0; 4])
        } else {
            [0.0; 4]
        }
    };
    let speed2 = speed("_MainTex2_Speed");
    let deform_speed = speed("_DeformMap_Speed");
    let refract_speed = speed("_RefractMap_Speed");
    let speed = speed("_MainTex_Speed");
    Ok(ParticlesValues {
        pixel_lighting: variant.pixel_lighting,
        color: vector("_Color", [1.0; 4]),
        strength: vector("_ColorStrength", [1.0; 4]),
        strength_night: vector("_ColorStrengthAtNight", [1.0; 4]),
        main_st: if variant.triplanar {
            vector("_MainTex_TriplanarUV", [1.0; 4])
        } else {
            st(main)
        },
        main2_st: if variant.triplanar {
            vector("_MainTex2_TriplanarUV", [1.0; 4])
        } else {
            st(main2)
        },
        speed: [speed[0], speed[1], speed2[0], speed2[1]],
        fresnel_fade: float("_FresnelFade"),
        fresnel_pow: float("_FresnelPow"),
        cutoff: float("_Cutoff"),
        fresnel_clip: variant.fresnel_clip,
        mul_map: variant.mul_map,
        soft_edges: variant.soft_edges,
        deform: variant.deform,
        refract: variant.refract,
        triplanar: variant.triplanar,
        near_clip: variant.near_clip,
        add_fog: variant.add_fog,
        alpha_pow: variant.alpha_pow.then(|| float("_AlphaPow")),
        back_cull,
        clip: [
            float("_ClipOffset"),
            float("_ClipFade"),
            if variant.pixel_lighting {
                float("_LightWrapAround")
            } else {
                speed[2]
            },
            speed2[2],
        ],
        deform_speed_z: deform_speed[2],
        inv_fade: float("_InvFade"),
        deform_strength: float("_DeformStrength"),
        refract_strength: float("_RefractStrength"),
        deform_st: if variant.pixel_lighting {
            [
                float("_AffectedByDayNightCycle"),
                float("_LightDesaturation"),
                float("_NormalSharpness"),
                float("_SelfIllumination"),
            ]
        } else if variant.triplanar {
            vector("_DeformMap_TriplanarUV", [1.0; 4])
        } else {
            st(deform)
        },
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
        "MarmosetUBER" | crate::sig::PLAIN | crate::sig::CAPPED => ShaderPort::Ported,
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
        out: &mut Out<'_>,
    ) -> Option<u32> {
        let key = (object.key(), srgb);
        if let Some(id) = self.textures.get(&key) {
            return *id;
        }
        let id = match self.assets.texture(object) {
            Ok(texture) => {
                let id = self.id();
                let srgb = srgb.unwrap_or(texture.texture.color_space == 1);
                // Decode/copy on this asset worker, not in the frame's drain
                // of a burst of completed prefabs. GPU upload still belongs
                // to Bevy's render world; this only constructs CPU data.
                let image = to_image(&texture, srgb);
                out.push(Update::Texture { id, image });
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

    fn material(&mut self, object: &ObjectRef, out: &mut Out<'_>) -> Option<u32> {
        if let Some(id) = self.materials.get(&object.key()) {
            return Some(*id);
        }
        let (_, data) = object.data().ok()?;
        let material = Material::parse(data, object.file.file().big_endian).ok()?;
        let shader = self.shader_info(object, &material, out);
        let view_model = self.view_model_materials.contains(&object.key());
        let slot = |name: &str| {
            material.texture(name).map(|t| {
                (
                    t.texture,
                    [t.scale[0], t.scale[1], t.offset[0], t.offset[1]],
                )
            })
        };
        let mut texture = |name: &str, srgb: Option<bool>| {
            let (pptr, st) = slot(name)?;
            let target = self.assets.resolve(&object.file, pptr).ok()??;
            Some((self.texture(&target, srgb, out)?, st))
        };
        let mut desc = material_desc(&material, &mut texture);
        desc.view_model = view_model;
        desc.cull_mode = shader_cull(&material, &shader);
        desc.sig = crate::sig::read(&material, &shader, &mut texture);
        if let Some(sig) = &desc.sig {
            desc.color = sig.color;
            desc.alpha = Alpha::Opaque;
            desc.cull_mode = Some(Face::Back);
            desc.uber = None;
        }
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
        out: &mut Out<'_>,
    ) -> Arc<ShaderInfo> {
        let named = |name: &str| {
            Arc::new(ShaderInfo {
                name: name.into(),
                defaults: HashMap::new(),
                cull: None,
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
                cull: s.passes().first().map(|p| p.state.cull.clone()),
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
        out: &mut Out<'_>,
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
    fn effect_mesh(&mut self, object: &ObjectRef, out: &mut Out<'_>) -> Vec<Option<u32>> {
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
        for (i, data) in split_geometry(&geometry, None, None)
            .into_iter()
            .enumerate()
        {
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
    fn mesh(&mut self, object: &ObjectRef, out: &mut Out<'_>) -> Vec<Option<u32>> {
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
        out: &mut Out<'_>,
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
        out: &mut Out<'_>,
    ) -> Vec<Option<u32>> {
        let mut ids = Vec::new();
        for (i, data) in split_geometry(geometry, None, None).into_iter().enumerate() {
            let id = self.id();
            out.push(Update::Mesh { id, data });
            self.meshes.insert((key.clone(), i), Some(id));
            ids.push(Some(id));
        }
        ids
    }

    /// A prefab's id; `spawn`: with what its placeholders spawn (false for a
    /// placement whose saved objects already hold it, `placeholders_saved`).
    fn prefab_as(&mut self, path: &str, spawn: bool, out: &mut Out<'_>) -> Option<u32> {
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
    fn placeholders_saved(&mut self, path: &str, below: &[&str], out: &mut Out<'_>) -> bool {
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
    fn prefab_parts(&mut self, prefab: &Prefab, out: &mut Out<'_>) -> Option<u32> {
        self.prefab_parts_as(prefab, true, out)
    }

    /// `prefab_parts`; `spawn`: with what its placeholders spawn.
    fn prefab_parts_as(&mut self, prefab: &Prefab, spawn: bool, out: &mut Out<'_>) -> Option<u32> {
        let PrefabContent {
            parts,
            lights,
            directional,
            rigs,
            culls,
            atmospheres,
            lods,
        } = self.prefab_content(prefab, out, if spawn { 0 } else { NO_SPAWN });
        let id = (!parts.is_empty()
            || !lights.is_empty()
            || !directional.is_empty()
            || !culls.is_empty()
            || !atmospheres.is_empty())
        .then(|| self.id());
        if let Some(id) = id {
            out.push(Update::Prefab {
                id,
                parts,
                lights,
                directional,
                rigs,
                culls,
                atmospheres,
                lods,
            });
        }
        id
    }

    /// A controller compiled with its clips (cached; `None` if it failed
    /// to load).
    fn program(&mut self, controller: &ObjectRef, out: &mut Out<'_>) -> Option<Arc<Program>> {
        let key = controller.key();
        if let Some(p) = self.programs.get(&key) {
            return p.clone();
        }
        let program = match self.assets.animation_set(controller, &mut self.clips) {
            Ok(set) => {
                for e in &set.errors {
                    out.push(Update::Warning(format!(
                        "animator {}: {e}",
                        set.controller.name
                    )));
                }
                Some(Arc::new(Program::new(Arc::new(set.controller), &set.clips)))
            }
            Err(e) => {
                out.push(Update::Warning(format!("animator: {e}")));
                None
            }
        };
        self.programs.insert(key, program.clone());
        program
    }

    /// The prefab's animators that move a Transform or drive blend shapes
    /// (active, enabled, with a controller), each with its nodes (M7f4c,
    /// M7f4d). `shapes`: the blend shapes of the prefab's skinned nodes.
    /// Animators that change only other properties are left out.
    fn rigs(
        &mut self,
        prefab: &Prefab,
        shapes: &HashMap<usize, sn_unity::BlendShapes>,
        out: &mut Out<'_>,
    ) -> Vec<RigBuild> {
        let mut rigs = Vec::new();
        for (i, node) in prefab.nodes.iter().enumerate() {
            let Some(a) = &node.animator else { continue };
            if !node.active || !a.component.enabled {
                continue;
            }
            let Some(controller) = &a.controller else {
                continue;
            };
            let Some(program) = self.program(controller, out) else {
                continue;
            };
            // The animator's node and everything below it (stored parents
            // first).
            let mut node_map: HashMap<usize, u16> = HashMap::from([(i, 0)]);
            // A root animator's own placement is the instance's.
            let own = if node.parent.is_some() {
                node.local
            } else {
                Placement::default()
            };
            let mut nodes = vec![(None, own)];
            for j in i + 1..prefab.nodes.len() {
                let Some(parent) = prefab.nodes[j].parent else {
                    continue;
                };
                let Some(&pk) = node_map.get(&parent) else {
                    continue;
                };
                if nodes.len() >= usize::from(u16::MAX) {
                    break;
                }
                node_map.insert(j, nodes.len() as u16);
                nodes.push((Some(pk), prefab.nodes[j].local));
            }
            let names = |n: usize| {
                shapes.get(&n).map_or(Vec::new(), |s| {
                    s.channels.iter().map(|c| c.name.clone()).collect()
                })
            };
            let binding = prefab.bind_animator(i, &program, &names);
            let slot_nodes: Vec<Option<u16>> = binding
                .nodes
                .iter()
                .zip(&program.slots)
                .map(|(n, s)| {
                    n.filter(|_| s.kind.is_transform())
                        .and_then(|n| node_map.get(&n).copied())
                })
                .collect();
            let (shape_nodes, shape_descs) =
                self.shape_descs(prefab, shapes, &program, &binding.nodes, &node_map, out);
            if slot_nodes.iter().all(Option::is_none) && shape_descs.is_empty() {
                continue;
            }
            let base = node
                .parent
                .map_or(Placement::default(), |p| prefab.nodes[p].in_prefab);
            let switched = self
                .switch_nodes
                .iter()
                .filter_map(|n| node_map.get(n).copied())
                .collect();
            let named = self
                .name_nodes
                .iter()
                .filter_map(|(n, name)| Some((*node_map.get(n)?, name.clone())))
                .collect();
            rigs.push(RigBuild {
                animator: i,
                node_map,
                shape_nodes,
                desc: RigDesc {
                    base,
                    nodes,
                    name: format!("{} ({})", prefab.key, program.controller.name),
                    program,
                    slot_nodes,
                    defaults: binding.defaults,
                    culling: a.component.culling_mode,
                    skins: Vec::new(),
                    shapes: shape_descs,
                    switched,
                    named,
                },
            });
        }
        rigs
    }

    /// The renderers below an animator whose blend shapes its slots drive
    /// (a float slot on a skinned mesh, class 137 custom type 20, whose
    /// attribute is a channel's name hash), as prefab node → index, and
    /// each one's [`ShapeDesc`]. `slot_nodes`: each slot's prefab node.
    fn shape_descs(
        &self,
        prefab: &Prefab,
        shapes: &HashMap<usize, sn_unity::BlendShapes>,
        program: &Program,
        slot_nodes: &[Option<usize>],
        node_map: &HashMap<usize, u16>,
        out: &mut Out<'_>,
    ) -> (HashMap<usize, u16>, Vec<ShapeDesc>) {
        // Prefab node → (channel, pose offset) of each driven channel.
        let mut driven: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        for (slot, node) in program.slots.iter().zip(slot_nodes) {
            let SlotKind::Float {
                type_id: 137,
                attribute,
                custom_type: 20,
            } = slot.kind
            else {
                continue;
            };
            let Some(n) = node.filter(|n| node_map.contains_key(n)) else {
                continue;
            };
            let Some(c) = shapes
                .get(&n)
                .and_then(|s| s.channels.iter().position(|c| c.name_hash == attribute))
            else {
                continue;
            };
            driven.entry(n).or_default().push((c, slot.offset));
        }
        let mut map = HashMap::new();
        let mut descs = Vec::new();
        for (n, channels) in driven {
            let s = &shapes[&n];
            if s.frames.len() > MAX_MORPH_WEIGHTS {
                out.push(Update::Warning(format!(
                    "{}: {:?} has {} blend shape frames, more than Bevy's {MAX_MORPH_WEIGHTS} morph targets; not animated",
                    prefab.key,
                    prefab.nodes[n].name,
                    s.frames.len(),
                )));
                continue;
            }
            let stored = &prefab.nodes[n].blend_shape_weights;
            let desc = ShapeDesc {
                channels: s
                    .channels
                    .iter()
                    .enumerate()
                    .map(|(c, ch)| {
                        let first = ch.frame_index as usize;
                        let count = ch.frame_count as usize;
                        ShapeChannel {
                            slot: channels.iter().find(|d| d.0 == c).map(|d| d.1),
                            default: stored.get(c).copied().unwrap_or(0.0),
                            first,
                            full: s
                                .full_weights
                                .get(first..first + count)
                                .unwrap_or(&[])
                                .to_vec(),
                        }
                    })
                    .collect(),
                targets: s.frames.len(),
                clamp: self.blend_clamp,
            };
            map.insert(n, descs.len() as u16);
            descs.push(desc);
        }
        (map, descs)
    }

    /// A skinned node bent on the GPU by a rig that holds all its bones:
    /// (rig, skin, its blend shapes if that rig drives them, sub-meshes
    /// with bone weights and then morph targets). `None`: drawn in its
    /// stored pose instead (no such rig, too many bones for Bevy, bind
    /// poses missing).
    fn gpu_skin(
        &mut self,
        prefab: &Prefab,
        node: usize,
        rigs: &mut [RigBuild],
        out: &mut Out<'_>,
    ) -> Option<GpuSkin> {
        let n = &prefab.nodes[node];
        if n.bones.is_empty() || n.bones.len() > 256 {
            return None;
        }
        let r = rigs.iter().position(|rig| {
            n.bones
                .iter()
                .all(|b| b.is_some_and(|b| rig.node_map.contains_key(&b)))
        })?;
        let object = n.mesh.as_ref()?;
        let (mesh, geometry) = match self.assets.mesh(object) {
            Ok(x) => x,
            Err(e) => {
                out.push(Update::Warning(format!("mesh: {e}")));
                return None;
            }
        };
        let vertices = geometry.positions.len();
        if mesh.bind_poses.len() < n.bones.len()
            || geometry.bone_weights.len() != vertices
            || geometry.bone_indices.len() != vertices
        {
            return None;
        }
        let rig = &mut rigs[r];
        let joints: Vec<u16> = n
            .bones
            .iter()
            .filter_map(|b| b.and_then(|b| rig.node_map.get(&b).copied()))
            .collect();
        let inverse_bindposes = mesh.bind_poses[..n.bones.len()]
            .iter()
            .map(bindpose_to_bevy)
            .collect();
        let key = object.key();
        let shape = rig.shape_nodes.get(&node).copied();
        let ids = if shape.is_some() {
            self.morph_mesh(key, &geometry, &mesh.blend_shapes, Some(joints.len()), out)
        } else if self.skin_meshes.contains_key(&(key.clone(), 0)) {
            (0..)
                .map_while(|i| self.skin_meshes.get(&(key.clone(), i)).copied())
                .collect()
        } else {
            let mut ids = Vec::new();
            for (i, data) in split_geometry(&geometry, Some(joints.len()), None)
                .into_iter()
                .enumerate()
            {
                let id = self.id();
                out.push(Update::Mesh { id, data });
                self.skin_meshes.insert((key.clone(), i), Some(id));
                ids.push(Some(id));
            }
            ids
        };
        let skin = rig.desc.skins.len() as u16;
        rig.desc.skins.push(SkinDesc {
            joints,
            inverse_bindposes,
        });
        Some((r as u16, skin, shape, ids))
    }

    /// A mesh's sub-meshes with its blend shapes as morph targets (and
    /// bone weights for `joints` bones), cached under `key` (M7f4d).
    fn morph_mesh(
        &mut self,
        key: Key,
        geometry: &sn_unity::MeshGeometry,
        shapes: &sn_unity::BlendShapes,
        joints: Option<usize>,
        out: &mut Out<'_>,
    ) -> Vec<Option<u32>> {
        let skinned = joints.is_some();
        if self.morph_meshes.contains_key(&(key.clone(), skinned, 0)) {
            return (0..)
                .map_while(|i| self.morph_meshes.get(&(key.clone(), skinned, i)).copied())
                .collect();
        }
        let mut ids = Vec::new();
        for (i, data) in split_geometry(geometry, joints, Some(shapes))
            .into_iter()
            .enumerate()
        {
            let id = self.id();
            out.push(Update::Mesh { id, data });
            self.morph_meshes
                .insert((key.clone(), skinned, i), Some(id));
            ids.push(Some(id));
        }
        ids
    }

    /// The blend shapes of a prefab's skinned nodes, by node, if the prefab
    /// has an animator that could drive them (meshes that fail to read are
    /// left out; they fail again, with a warning, when drawn).
    fn node_shapes(&self, prefab: &Prefab) -> HashMap<usize, sn_unity::BlendShapes> {
        let animated = prefab
            .nodes
            .iter()
            .any(|n| n.active && n.animator.as_ref().is_some_and(|a| a.component.enabled));
        if !self.animate || !animated {
            return HashMap::new();
        }
        prefab
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.skinned)
            .filter_map(|(i, n)| {
                let mesh = self.assets.mesh_info(n.mesh.as_ref()?).ok()?;
                (!mesh.blend_shapes.is_empty()).then_some((i, mesh.blend_shapes))
            })
            .collect()
    }

    /// A hierarchy's drawn parts and lights relative to its root, with the
    /// prefabs its placeholders spawn (`depth`: how deep in spawned prefabs
    /// this one is).
    fn prefab_content(
        &mut self,
        prefab: &Prefab,
        out: &mut Out<'_>,
        depth: usize,
    ) -> PrefabContent {
        let mut parts = Vec::new();
        let shapes = self.node_shapes(prefab);
        let mut rigs = if self.animate {
            self.rigs(prefab, &shapes, out)
        } else {
            Vec::new()
        };
        // Each node's nearest animator at or above it with a rig: (rig,
        // the node in it).
        let rig_of = |index: usize, rigs: &[RigBuild]| -> Option<(u16, u16)> {
            let mut at = Some(index);
            while let Some(i) = at {
                if let Some(r) = rigs.iter().position(|r| r.animator == i) {
                    return rigs[r].node_map.get(&index).map(|&n| (r as u16, n));
                }
                at = prefab.nodes[i].parent;
            }
            None
        };
        for (index, node) in prefab.drawn() {
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
            // Skinned meshes bend with an animated rig on the GPU, or come
            // out in their stored pose in the prefab root's space.
            let gpu = if node.skinned {
                self.gpu_skin(prefab, index, &mut rigs, out)
            } else {
                None
            };
            let skinned = if node.skinned && gpu.is_none() {
                self.skinned_mesh(prefab, index, out)
            } else {
                None
            };
            // Still meshes, and skinned ones without bones (drawn at their
            // node, as Unity draws them), hang on their rig node.
            let on_node = if node.skinned && !node.bones.is_empty() {
                None
            } else {
                rig_of(index, &rigs)
            };
            // Blend shapes of a bone-less skinned mesh driven by that rig.
            let node_shape = on_node
                .filter(|_| gpu.is_none() && skinned.is_none())
                .and_then(|(r, _)| rigs[usize::from(r)].shape_nodes.get(&index).copied());
            let shape = match &gpu {
                Some((_, _, shape, _)) => *shape,
                None => node_shape,
            };
            if shape.is_none() && rigs.iter().any(|r| r.shape_nodes.contains_key(&index)) {
                info!(
                    "objects: blend shapes of {:?} in {} not animated: its skin is not bent by the rig that drives them",
                    node.name, prefab.key
                );
            }
            let rig = match (&gpu, on_node) {
                (Some((rig, skin, _, _)), _) => Some(RigPart::Skin {
                    rig: *rig,
                    skin: *skin,
                }),
                (None, Some((rig, node))) => Some(RigPart::Node { rig, node }),
                (None, None) => None,
            };
            let local = if skinned.is_some() || rig.is_some() {
                Placement::default()
            } else {
                node.in_prefab
            };
            let sub_meshes = match (gpu, skinned, node_shape.and(shapes.get(&index))) {
                (Some((_, _, _, ids)), _, _) => ids,
                (None, Some(ids), _) => ids,
                (None, None, Some(s)) => match self.assets.mesh(mesh) {
                    Ok((_, geometry)) => self.morph_mesh(mesh.key(), &geometry, s, None, out),
                    Err(e) => {
                        out.push(Update::Warning(format!("mesh: {e}")));
                        continue;
                    }
                },
                (None, None, None) => self.mesh(mesh, out),
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
                    waterfall_cap_height: node.waterfall_cap_height,
                    mesh: mesh_id,
                    material,
                    local,
                    biome_sky,
                    sky_position: node.sky_applier_position,
                    effect,
                    rig,
                    shape: shape.filter(|_| !effect),
                    shadows: node.cast_shadows,
                    lod: node.lod_group.map(|(g, m)| (g as u16, m)),
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
        // `ShipExteriorCull`s that register when the prefab starts.
        let culls = prefab
            .exterior_culls
            .iter()
            .filter(|c| c.registers)
            .flat_map(|c| &c.boxes)
            .filter_map(|b| {
                Some(CullDesc {
                    in_prefab: prefab.nodes.get(b.node)?.in_prefab,
                    center: b.center,
                    size: b.size,
                })
            })
            .collect();
        let lods = prefab
            .lod_groups
            .iter()
            .map(|g| LodDesc {
                in_prefab: prefab
                    .nodes
                    .get(g.node)
                    .map_or_else(Placement::default, |n| n.in_prefab),
                reference_point: g.reference_point,
                size: g.size,
                heights: g.heights.clone().into(),
            })
            .collect();
        let mut content = PrefabContent {
            parts,
            lights,
            directional,
            rigs: rigs.into_iter().map(|r| Arc::new(r.desc)).collect(),
            culls,
            atmospheres: match self
                .assets
                .prefab_atmospheres(prefab, &self.atmosphere_layout)
            {
                Ok(v) => v,
                Err(e) => {
                    out.push(Update::Warning(e));
                    Vec::new()
                }
            },
            lods,
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
        out: &mut Out<'_>,
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
                // The spawned root takes the placeholder's place; its rigs
                // hang there too.
                let at = node.in_prefab;
                content
                    .atmospheres
                    .extend(child.atmospheres.iter().map(|v| v.under(&at)));
                let first_rig = content.rigs.len() as u16;
                let first_lod = content.lods.len() as u16;
                content.lods.extend(child.lods.iter().map(|l| LodDesc {
                    in_prefab: at.then(&l.in_prefab),
                    ..l.clone()
                }));
                content.rigs.extend(child.rigs.iter().map(|r| {
                    Arc::new(RigDesc {
                        base: at.then(&r.base),
                        ..RigDesc::clone(r)
                    })
                }));
                content.parts.extend(child.parts.iter().map(|p| {
                    let lod = p.lod.map(|(g, m)| (g + first_lod, m));
                    match p.rig {
                        Some(rig) => Part {
                            sky_position: at.transform_point(p.sky_position),
                            rig: Some(rig.shifted(first_rig)),
                            lod,
                            ..*p
                        },
                        None => Part {
                            sky_position: at.transform_point(p.sky_position),
                            local: at.then(&sn_assets::waterfall_local(
                                p.local,
                                p.waterfall_cap_height,
                                at.scale[1],
                            )),
                            waterfall_cap_height: None,
                            lod,
                            ..*p
                        },
                    }
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
                content.culls.extend(child.culls.iter().map(|c| CullDesc {
                    in_prefab: at.then(&c.in_prefab),
                    ..*c
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
        out: &mut Out<'_>,
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
        out: &mut Out<'_>,
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
                    pod_sky: true,
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
    fn scenes(&mut self, options: &SceneOptions, out: &mut Out<'_>) {
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
            // The Aurora: its parts by the states they show in (M7f4e); the
            // client's exploder decides which are shown.
            let groups = match scene.aurora_groups(&self.assets) {
                Ok(g) => g,
                Err(e) => {
                    out.push(Update::Warning(format!("scene {}: {e}", scene.name)));
                    Vec::new()
                }
            };
            let mut instances = Vec::new();
            let mut shows = Vec::new();
            let mut aurora = None;
            if !groups.is_empty() {
                let code = sn_assets::read_assembly(self.assets.game())
                    .and_then(|bytes| sn_assets::exploder_code(&bytes));
                let manager = scene.ship_exterior_cull_manager(&self.assets);
                match (code, manager) {
                    (Ok(code), Ok(manager)) => {
                        aurora = Some(AuroraData {
                            code,
                            cull_every: manager.map(|m| m.update_every_x_frames),
                        });
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        out.push(Update::Warning(format!("Aurora: {e}")));
                    }
                }
                let drawn = |exploded: bool| -> usize {
                    groups
                        .iter()
                        .filter(|g| g.show.shown(exploded, false))
                        .map(|g| g.prefab.visible_nodes().count())
                        .sum()
                };
                state = format!(
                    ", Aurora: {} parts by state, {} nodes drawn intact, {} exploded",
                    groups.len(),
                    drawn(false),
                    drawn(true)
                );
                for g in &groups {
                    if let Some(prefab) = self.prefab_parts(&g.prefab, out) {
                        instances.push(Instance {
                            level: 0,
                            prefab,
                            transform: g.prefab.nodes[0].local,
                            from_slot: false,
                            pod_sky: false,
                        });
                        shows.push(g.show);
                    }
                }
            }
            let mut lifepod = None;
            if let (true, Some(point)) = (is_pod, options.lifepod) {
                match self.place_pod(&mut scene, point, &mut instances, out) {
                    Ok(summary) => state = summary,
                    Err(e) => out.push(Update::Warning(format!("lifepod: {e}"))),
                }
                match crate::lifepod_light::PodLightData::read(
                    &self.assets,
                    &mut scene,
                    point,
                    options.lifepod_state,
                ) {
                    Ok(Some(data)) => lifepod = Some(Box::new(data)),
                    Ok(None) => out.push(Update::Warning("lifepod: no lighting controller".into())),
                    Err(e) => out.push(Update::Warning(format!("lifepod light: {e}"))),
                }
            }
            let mut drawn: usize = groups
                .iter()
                .filter(|g| g.show.intact)
                .map(|g| g.prefab.visible_nodes().count())
                .sum();
            for root in scene.roots.iter().filter(|_| groups.is_empty()) {
                drawn += root.visible_nodes().count();
                if let Some(prefab) = self.prefab_parts(root, out) {
                    instances.push(Instance {
                        level: 0,
                        prefab,
                        transform: root.nodes[0].local,
                        from_slot: false,
                        pod_sky: false,
                    });
                }
            }
            // The pod's own instances (placed before) and the roots: shown
            // always.
            shows.resize(instances.len(), AuroraShow::ALWAYS);
            let nodes: usize = scene.roots.iter().map(|r| r.nodes.len()).sum();
            out.push(Update::Scene {
                summary: format!(
                    "scene {}: {} top-level objects, {nodes} nodes, {drawn} drawn{state}",
                    scene.name,
                    scene.roots.len()
                ),
                instances,
                shows,
                aurora,
                lifepod,
                ms: start.elapsed().as_secs_f32() * 1000.0,
            });
        }
        self.assets.trim_cache(BUNDLE_CACHE_BYTES);
    }

    /// The player's body (M9g4): the main scene's `Player` hierarchy with
    /// the equipment rule for a new game, as one prefab at the identity
    /// (the client moves its rig to the player). Its placeholders are not
    /// spawned (it has none in use; the tools come with their items).
    fn body(&mut self, options: &BodyOptions, out: &mut Out<'_>) {
        let start = Instant::now();
        let mut body = match self.assets.player_body() {
            Ok(b) => b,
            Err(e) => {
                out.push(Update::Warning(format!("player body: {e}")));
                return;
            }
        };
        if options.head_visible
            && let Some(head) = body.prefab.nodes.get_mut(body.head_node)
        {
            head.cast_shadows = SHADOWS_ON;
        }
        // `MainCameraControl` turns and bobs the view model; the client
        // moves the animator's parent, so the two must be the same node.
        let parent = body
            .prefab
            .nodes
            .get(body.animator_node)
            .and_then(|n| n.parent);
        if parent != Some(body.view_model_node) {
            out.push(Update::Warning(format!(
                "player body: the animator hangs on node {parent:?}, not the view model ({}); the body will not turn with the view",
                body.view_model_node
            )));
        }
        let mask = self.culling_mask;
        let visible: Vec<&sn_assets::PrefabNode> = body.prefab.visible_nodes().collect();
        let culled = visible
            .iter()
            .filter(|n| n.layer >= 32 || mask & (1 << n.layer) == 0)
            .count();
        let shadows_only = visible
            .iter()
            .filter(|n| n.cast_shadows == SHADOWS_ONLY)
            .count();
        let names: Vec<&str> = visible.iter().map(|n| n.name.as_str()).collect();
        let summary = format!(
            "player body: {} nodes, {} renderers drawn {names:?} ({culled} on layers the camera leaves out, {shadows_only} shadows only), head {:?}{}",
            body.prefab.nodes.len(),
            visible.len(),
            body.prefab
                .nodes
                .get(body.head_node)
                .map_or("?", |n| n.name.as_str()),
            if options.head_visible { " drawn" } else { "" }
        );
        // The PDA in the hand (M9u8a3): stored inactive, `PDA.Open`
        // switches it on. Built active so its parts exist, its node
        // marked switched (hidden until the client shows it).
        body.prefab.set_active(body.pda.node, true);
        self.switch_nodes = vec![body.pda.node];
        if let Some(mesh) = body.prefab.nodes.get(body.pda.mesh_node) {
            for m in mesh.materials.iter().flatten() {
                if self.materials.contains_key(&m.key()) {
                    out.push(Update::Warning(format!(
                        "player body: the PDA's material {} was loaded before; it does not write depth",
                        m.path_id
                    )));
                }
                self.view_model_materials.insert(m.key());
            }
        }
        self.name_nodes = vec![(body.pda.screen_anchor_node, "ScreenAnchor".into())];
        let prefab = self.prefab_parts_as(&body.prefab, false, out);
        self.switch_nodes.clear();
        self.name_nodes.clear();
        out.push(Update::Body {
            prefab,
            summary,
            ms: start.elapsed().as_secs_f32() * 1000.0,
        });
        self.assets.trim_cache(BUNDLE_CACHE_BYTES);
    }

    /// A prefab drawn as a still object: creatures move and animate, so
    /// they are left out (a later milestone).
    fn still_prefab(&mut self, path: &str, out: &mut Out<'_>) -> Option<u32> {
        self.still_prefab_as(path, true, out)
    }

    /// `still_prefab`; `spawn`: with what its placeholders spawn.
    fn still_prefab_as(&mut self, path: &str, spawn: bool, out: &mut Out<'_>) -> Option<u32> {
        if path.starts_with("WorldEntities/Creatures/") {
            return None;
        }
        self.prefab_as(path, spawn, out)
    }

    /// Fills the slots of a placeholder (component `data`, placed at
    /// `placeholder` in world space); each filler shows at its own cell
    /// level (`WorldEntityInfo.cellLevel`; batch and global levels at the
    /// farthest).
    /// Whether objects of this class are items the hand acts on.
    fn is_item(&self, class: &str) -> bool {
        self.items.as_ref().is_some_and(|r| {
            r.outcrops.contains_key(class) || r.pickups.get(class).is_some_and(|p| p.pickupable)
        })
    }

    /// A server-made object of `class` at `at` (M9d4b).
    fn spawned(&mut self, id: u32, class: &str, at: Placement, out: &mut Out<'_>) {
        let instance = self
            .class_paths
            .get(class)
            .cloned()
            .and_then(|path| self.still_prefab_as(&path, true, out))
            .map(|prefab| Instance {
                level: 0,
                prefab,
                transform: at,
                from_slot: false,
                pod_sky: false,
            });
        out.push(Update::Spawned { id, instance });
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_slots(
        &mut self,
        id: &str,
        data: &[u8],
        placeholder: &Placement,
        instances: &mut Vec<Instance>,
        keys: &mut Vec<(usize, EntityKey)>,
        out: &mut Out<'_>,
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
        let wanted: Vec<(String, Placement, usize, Option<EntityKey>)> = spawns
            .iter()
            .filter_map(|s| {
                let path = self.class_paths.get(s.class_id)?;
                let level = usize::try_from(s.info.cell_level).map_or(0, |l| l.min(LEVELS - 1));
                let key = match (u16::try_from(s.slot), u16::try_from(s.copy)) {
                    (Ok(slot), Ok(copy)) if self.is_item(s.class_id) => Some(EntityKey::Slot {
                        placeholder: id.to_owned(),
                        slot,
                        copy,
                    }),
                    _ => None,
                };
                Some((path.clone(), placeholder.then(&s.transform), level, key))
            })
            .collect();
        for (path, transform, level, key) in wanted {
            if let Some(prefab) = self.still_prefab(&path, out) {
                if let Some(key) = key {
                    keys.push((instances.len(), key));
                }
                instances.push(Instance {
                    level,
                    prefab,
                    transform,
                    from_slot: true,
                    pod_sky: false,
                });
            }
        }
    }

    fn batch(&mut self, game: &GameData, coord: BatchCoord, out: &mut Out<'_>) {
        let start = Instant::now();
        let mut instances = Vec::new();
        let mut keys = Vec::new();
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
                                    &mut keys,
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
                            if self.is_item(&object.class_id) {
                                keys.push((instances.len(), EntityKey::Placed(object.id.clone())));
                            }
                            instances.push(Instance {
                                level: (cell.level as usize).min(LEVELS - 1),
                                prefab,
                                transform,
                                from_slot: false,
                                pod_sky: false,
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
                        if self.is_item(&object.class_id) {
                            keys.push((instances.len(), EntityKey::Placed(object.id.clone())));
                        }
                        instances.push(Instance {
                            level: BATCH_OBJECTS,
                            prefab,
                            transform,
                            from_slot: false,
                            pod_sky: false,
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
            keys,
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
        sig: None,
        effect: None,
        alpha,
        cull_mode: cull_mode(material.float("_MyCullVariable").unwrap_or(2.0)),
        view_model: false,
    }
}

struct Shared {
    /// Batches to load, most urgent last.
    queue: Mutex<Vec<BatchCoord>>,
    /// Server-made objects to load (M9d4b): id, class, placement. Under
    /// the same lock's wake-up as the queue.
    spawn: Mutex<Vec<(u32, String, Placement)>>,
    wake: Condvar,
    shutdown: AtomicBool,
}

#[allow(clippy::too_many_arguments)]
fn worker(
    game: &'static GameData,
    shared: Arc<Shared>,
    tx: Sender<Update>,
    slot_seed: Option<u64>,
    placeholders: bool,
    scenes: Option<SceneOptions>,
    body: Option<BodyOptions>,
    animate: bool,
    items: Option<Arc<ItemRules>>,
) {
    let start = Instant::now();
    let setup = || -> Result<Library, String> {
        let assets = GameAssets::index(game)?;
        let catalog = assets.catalog()?;
        let class_paths = game.read_prefab_database().map_err(|e| e.0)?;
        let entity_infos = sn_assets::entity_infos(&assets)?;
        let assemblies = sn_assets::ScriptAssemblies::load(game)?;
        let atmosphere_layout =
            assemblies
                .layouts()?
                .script("Assembly-CSharp", "", "AtmosphereVolume")?;
        Ok(Library {
            assets,
            atmosphere_layout,
            items: items.clone(),
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
            switch_nodes: Vec::new(),
            name_nodes: Vec::new(),
            view_model_materials: std::collections::HashSet::new(),
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
            animate,
            programs: HashMap::new(),
            clips: HashMap::new(),
            skin_meshes: HashMap::new(),
            morph_meshes: HashMap::new(),
            blend_clamp: true,
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
    match sn_assets::blend_shape_clamp(&library.assets) {
        Ok(clamp) => library.blend_clamp = clamp,
        Err(e) => {
            let _ = tx.send(Update::Warning(format!(
                "blend shapes: {e}; weights clamped to 0–100"
            )));
        }
    }
    match library.assets.main_camera() {
        Ok(camera) => {
            let hidden: Vec<u32> = (0..32).filter(|&l| !camera.draws_layer(l)).collect();
            info!(
                "main camera: culling mask {:#010x}, layers not drawn {hidden:?}",
                camera.culling_mask
            );
            library.culling_mask = camera.culling_mask;
            let _ = tx.send(Update::Camera { near: camera.near });
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
    match sn_assets::quality_settings(&library.assets) {
        Ok(q) => match q.level("High") {
            Some(high) => {
                let _ = tx.send(Update::Lod(LodSettings {
                    bias: high.lod_bias,
                    max_level: usize::try_from(high.maximum_lod_level).unwrap_or(0),
                }));
            }
            None => {
                let _ = tx.send(Update::Warning(
                    "QualitySettings: no level High; objects stay at LOD 0".into(),
                ));
            }
        },
        Err(e) => {
            let _ = tx.send(Update::Warning(format!("{e}; objects stay at LOD 0")));
        }
    }
    let _ = tx.send(Update::Ready {
        ms: start.elapsed().as_secs_f32() * 1000.0,
    });
    let mut out = Out(&tx);
    if let Some(options) = scenes {
        library.scenes(&options, &mut out);
    }
    if let Some(options) = body {
        library.body(&options, &mut out);
    }
    loop {
        let coord = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                let spawns: Vec<(u32, String, Placement)> =
                    std::mem::take(&mut *shared.spawn.lock().unwrap());
                if !spawns.is_empty() {
                    break Err(spawns);
                }
                if let Some(c) = queue.pop() {
                    break Ok(c);
                }
                // Idle: everything asked for is loaded.
                library.log_unported();
                queue = shared.wake.wait(queue).unwrap();
            }
        };
        match coord {
            Ok(coord) => library.batch(game, coord, &mut out),
            Err(spawns) => {
                for (id, class, at) in spawns {
                    library.spawned(id, &class, at, &mut out);
                }
            }
        }
    }
}

/// A batch's objects on the main thread.
#[derive(Default)]
struct BatchObjects {
    /// `None` until the worker has sent them.
    instances: Option<Vec<Instance>>,
    /// Instance index → its key, for items (M9d4b).
    keys: HashMap<usize, EntityKey>,
    /// Per cell level, the keys of its spawned items.
    level_keys: [Vec<EntityKey>; SLOTS],
    /// Spawned entities per cell level, `None` when not spawned.
    shown: [Option<Vec<Entity>>; SLOTS],
    /// Whether its entities cast sun shadows (only the nearest level of
    /// detail: the game's shadows reach 50 m).
    casting: bool,
}

/// Takes entities despawned outside the streaming (picked-up items) out of
/// their batches' shown lists. Those lists later insert and remove
/// components on them (the shadow switch at LOD 0) and despawn them again
/// when the batch goes, and a command on a despawned entity panics.
fn forget_despawned<'b>(
    batches: impl Iterator<Item = &'b mut BatchObjects>,
    despawned: &HashSet<Entity>,
) {
    if despawned.is_empty() {
        return;
    }
    for b in batches {
        for list in b.shown.iter_mut().flatten() {
            list.retain(|e| !despawned.contains(e));
        }
    }
}

/// A scene's top-level objects on the main thread.
struct SceneObjects {
    instances: Vec<Instance>,
    /// Per instance: the Aurora states it shows in.
    shows: Vec<AuroraShow>,
    aurora: Option<AuroraData>,
    /// Per instance, its entities; `None` until spawned.
    shown: Option<Vec<Vec<Entity>>>,
}

#[derive(Default, Clone, Debug)]
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
    /// Plain SIG, capped SIG, enabled SIG maps, missing required albedo slots.
    pub sig: [usize; 4],
    /// Materials made per sky (a material is made once per sky it's lit by),
    /// by sky name.
    pub per_sky: Vec<(String, usize)>,
}

#[derive(Resource)]
pub struct ObjectStreamer {
    shared: Arc<Shared>,
    rx: Mutex<Receiver<Update>>,
    pending: VecDeque<Update>,
    ready: bool,
    textures: HashMap<u32, Handle<Image>>,
    /// Materials as the worker described them, made per sky on first use.
    material_descs: HashMap<u32, MaterialDesc>,
    /// (material, sky) → material.
    materials: HashMap<(u32, SkyKey), Handle<ObjectMaterial>>,
    skies: SkySet,
    meshes: HashMap<u32, Handle<Mesh>>,
    prefabs: HashMap<u32, Vec<Part>>,
    /// A prefab's `ShipExteriorCull` boxes (M7f4e).
    prefab_culls: HashMap<u32, Vec<CullDesc>>,
    prefab_atmospheres: HashMap<u32, Vec<sn_assets::AtmosphereTemplate>>,
    /// A prefab's LOD groups (M7f4f).
    prefab_lods: HashMap<u32, Vec<LodDesc>>,
    /// Triangles per mesh id (for the LOD statistics).
    mesh_triangles: HashMap<u32, u32>,
    /// A prefab's animated rigs (M7f4c).
    prefab_rigs: HashMap<u32, Vec<Arc<RigDesc>>>,
    /// (prefab, rig, skin) → its inverse bind poses.
    bindposes: HashMap<(u32, u16, u16), Handle<SkinnedMeshInverseBindposes>>,
    /// Meshes with morph targets: bounds at any weights (M7f4d).
    morph_bounds: HashMap<u32, Aabb>,
    prefab_lights: HashMap<u32, Vec<LocalLight>>,
    prefab_directional: HashMap<u32, Vec<DirectionalSource>>,
    batches: HashMap<BatchCoord, BatchObjects>,
    scenes: Vec<SceneObjects>,
    /// The player's body (M9g4).
    body: Option<BodyObjects>,
    /// Other players' bodies to spawn (M10h2), by player id.
    remote_wanted: Vec<u16>,
    /// Other players' bodies spawned: the id and the instance's entities.
    remote_bodies: Vec<(u16, Vec<Entity>)>,
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
    /// Items (M9d4b): keys gone, the spawned entities of each item
    /// instance, the server-made objects wanted and drawn.
    gone: HashSet<EntityKey>,
    keyed: HashMap<EntityKey, Vec<Entity>>,
    spawned_wanted: HashSet<u32>,
    /// The world event index at which each wanted object was asked for
    /// (M9d6c: logged when it is drawn, to compare two clients).
    spawned_asked: HashMap<u32, u32>,
    spawned: HashMap<u32, Vec<Entity>>,
    spawned_pending: Vec<(u32, Instance)>,
    pub item_stats: ItemDrawStats,
}

/// What the item drawing did (M9d4b), for the logs.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemDrawStats {
    /// Keys whose entities were despawned when they went, and the
    /// entities.
    pub keys_hidden: usize,
    pub entities_hidden: usize,
    /// Item instances not spawned because their key was gone.
    pub skipped_gone: usize,
    /// Server-made objects drawn (each once), their entities, and
    /// objects removed.
    pub spawned_drawn: usize,
    pub spawned_entities: usize,
    pub spawned_removed: usize,
}

impl ObjectStreamer {
    pub(crate) fn effect_sky(&self, key: SkyKey) -> Option<SkyLook> {
        self.skies.look(key).copied()
    }
    /// `lights`: spawn the objects' point and spot lights. `slot_seed`: fill
    /// the spawn slots with this world seed (`None`: leave them empty).
    /// `placeholders`: spawn what the objects' placeholders hold.
    /// `scenes`: load the scenes the game spawns at start (`None`: none).
    /// `body`: spawn the player's body (M9g4; `None`: none).
    /// `animate`: run the objects' animators (M7f4c). `items`: the item
    /// rules (M9d4b; `None`: no instance is an item).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        game: GameData,
        lights: bool,
        slot_seed: Option<u64>,
        placeholders: bool,
        scenes: Option<SceneOptions>,
        body: Option<BodyOptions>,
        animate: bool,
        items: Option<Arc<ItemRules>>,
    ) -> ObjectStreamer {
        // The worker's asset index borrows the install for the whole run.
        let game: &'static GameData = Box::leak(Box::new(game));
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            spawn: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let worker_shared = shared.clone();
        std::thread::spawn(move || {
            worker(
                game,
                worker_shared,
                tx,
                slot_seed,
                placeholders,
                scenes,
                body,
                animate,
                items,
            )
        });
        Self::receiving(shared, rx, lights)
    }

    fn receiving(shared: Arc<Shared>, rx: Receiver<Update>, lights: bool) -> Self {
        ObjectStreamer {
            shared,
            rx: Mutex::new(rx),
            pending: VecDeque::new(),
            ready: false,
            textures: HashMap::new(),
            material_descs: HashMap::new(),
            materials: HashMap::new(),
            skies: SkySet::default(),
            meshes: HashMap::new(),
            prefabs: HashMap::new(),
            prefab_culls: HashMap::new(),
            prefab_atmospheres: HashMap::new(),
            prefab_lods: HashMap::new(),
            mesh_triangles: HashMap::new(),
            prefab_rigs: HashMap::new(),
            morph_bounds: HashMap::new(),
            bindposes: HashMap::new(),
            prefab_lights: HashMap::new(),
            prefab_directional: HashMap::new(),
            batches: HashMap::new(),
            scenes: Vec::new(),
            body: None,
            remote_wanted: Vec::new(),
            remote_bodies: Vec::new(),
            requested: Vec::new(),
            defaults: None,
            lights,
            camera_batch: None,
            warnings: 0,
            batch_ms: Vec::new(),
            gone: HashSet::new(),
            keyed: HashMap::new(),
            spawned_wanted: HashSet::new(),
            spawned_asked: HashMap::new(),
            spawned: HashMap::new(),
            spawned_pending: Vec::new(),
            item_stats: ItemDrawStats::default(),
        }
    }

    /// These objects left the world: their entities go now and they are
    /// not spawned again (M9d4b).
    pub fn hide_keys(&mut self, commands: &mut Commands, keys: &[EntityKey]) {
        let mut despawned = HashSet::new();
        for key in keys {
            self.gone.insert(key.clone());
            if let Some(entities) = self.keyed.remove(key) {
                self.item_stats.keys_hidden += 1;
                self.item_stats.entities_hidden += entities.len();
                for e in entities {
                    commands.entity(e).despawn();
                    despawned.insert(e);
                }
            }
        }
        forget_despawned(self.batches.values_mut(), &despawned);
    }

    /// The server-made objects now (id, class, placement): new ones are
    /// loaded and drawn, missing ones despawned (M9d4b).
    /// `events`: the world events applied (for the logs).
    pub fn set_spawned(
        &mut self,
        commands: &mut Commands,
        objects: &[(u32, String, Placement)],
        events: u32,
    ) {
        let now: HashSet<u32> = objects.iter().map(|o| o.0).collect();
        let gone: Vec<u32> = self
            .spawned_wanted
            .iter()
            .filter(|id| !now.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.spawned_wanted.remove(&id);
            self.spawned_asked.remove(&id);
            self.spawned_pending.retain(|p| p.0 != id);
            if let Some(entities) = self.spawned.remove(&id) {
                self.item_stats.spawned_removed += 1;
                for e in entities {
                    commands.entity(e).despawn();
                }
                info!(
                    "items: server-made object {id} removed at world event {events}; {} server-made objects drawn",
                    self.spawned.len()
                );
            }
        }
        let mut asked = Vec::new();
        for (id, class, at) in objects {
            if self.spawned_wanted.insert(*id) {
                self.spawned_asked.insert(*id, events);
                asked.push((*id, class.clone(), *at));
            }
        }
        if !asked.is_empty() {
            self.shared.spawn.lock().unwrap().extend(asked);
            // Wake the worker under the queue's lock (it waits on it).
            let _queue = self.shared.queue.lock().unwrap();
            self.shared.wake.notify_all();
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
        for look in self.material_descs.values().filter_map(|d| d.sig.as_ref()) {
            s.sig[usize::from(look.capped)] += 1;
            s.sig[2] += usize::from(look.enabled);
            s.sig[3] += usize::from(look.albedo.is_none());
            s.sig[3] += usize::from(look.capped && look.side.is_none());
        }
        let mut per_sky: HashMap<String, usize> = HashMap::new();
        for (_, sky) in self.materials.keys() {
            let name = match *sky {
                SkyKey::Fixed(i) => self
                    .skies
                    .skies
                    .skies
                    .get(i)
                    .map_or("none", |s| s.name.as_str()),
                SkyKey::Global => "(global)",
                SkyKey::Pod => "(lifepod)",
            };
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
            .flat_map(|sc| sc.shown.iter().flatten())
            .map(Vec::len)
            .sum();
        s
    }

    /// True when every wanted batch has its objects on screen.
    pub fn settled(&self, terrain: &TerrainStreamer) -> bool {
        self.ready
            && self.pending.is_empty()
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

pub(crate) fn to_bevy(t: &Placement) -> Transform {
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
pub(crate) fn spawn_light(
    commands: &mut Commands,
    light: &LocalLight,
    transform: Transform,
) -> Entity {
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
/// each with only the vertices it uses. `joints`: keep the bone weights for
/// GPU skinning with that many bones (indices past it lose their weight;
/// weights are normalised).
fn split_geometry(
    geometry: &sn_unity::MeshGeometry,
    joints: Option<usize>,
    shapes: Option<&sn_unity::BlendShapes>,
) -> Vec<MeshData> {
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
    let (bone_joints, bone_weights): (Vec<[u16; 4]>, Vec<[f32; 4]>) = match joints {
        Some(count) if geometry.bone_indices.len() == n && geometry.bone_weights.len() == n => {
            geometry
                .bone_indices
                .iter()
                .zip(&geometry.bone_weights)
                .map(|(i, w)| {
                    let mut j = [0u16; 4];
                    let mut wt = [0.0f32; 4];
                    for k in 0..4 {
                        if (i[k] as usize) < count && w[k] > 0.0 {
                            j[k] = i[k] as u16;
                            wt[k] = w[k];
                        }
                    }
                    let sum: f32 = wt.iter().sum();
                    if sum > 0.0 {
                        wt.iter_mut().for_each(|x| *x /= sum);
                    } else {
                        wt[0] = 1.0;
                    }
                    (j, wt)
                })
                .unzip()
        }
        _ => (Vec::new(), Vec::new()),
    };
    // Every blend shape frame's offsets for every vertex, mirrored as the
    // vertices are.
    let frames: Vec<Vec<[[f32; 3]; 3]>> = shapes.map_or(Vec::new(), |s| {
        (0..s.frames.len())
            .map(|f| {
                let mut dense = s.dense_frame(f, n);
                for v in &mut dense {
                    *v = v.map(flip);
                }
                dense
            })
            .collect()
    });
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
            joints: pick(&bone_joints, n, &used),
            weights: pick(&bone_weights, n, &used),
            morphs: frames.iter().flat_map(|f| pick(f, n, &used)).collect(),
            indices: local
                .chunks_exact(3)
                .flat_map(|t| [t[0], t[2], t[1]])
                .collect(),
        });
    }
    meshes
}

pub(crate) fn build_mesh(data: MeshData) -> Mesh {
    let _perf = crate::perf::span("objects::build_mesh");
    let n = data.positions.len();
    let skinned = data.joints.len() == n && data.weights.len() == n && n > 0;
    // A later streamed placement can need this mesh with reversed winding,
    // long after its first GPU upload. Keep the original's CPU data available
    // for that lazy variant (also required by dynamic skinned bounds).
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all());
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
    if skinned {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            bevy::mesh::VertexAttributeValues::Uint16x4(data.joints),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, data.weights);
        if let Err(e) = mesh.generate_skinned_mesh_bounds() {
            warn!("objects: skinned mesh bounds: {e}");
        }
    }
    if !data.morphs.is_empty() {
        mesh.set_morph_targets(
            data.morphs
                .iter()
                .map(|&[p, n, t]| MorphAttributes::new(Vec3::from(p), Vec3::from(n), Vec3::from(t)))
                .collect(),
        );
    }
    mesh
}

/// Bounds that hold a morphed mesh at any target weights in 0–1 (each
/// vertex at its base position plus every target's offsets that push it
/// one way, per axis). `None` without morph targets or vertices.
fn morph_bounds(data: &MeshData) -> Option<Aabb> {
    let n = data.positions.len();
    if data.morphs.is_empty() || n == 0 || data.morphs.len() % n != 0 {
        return None;
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for (v, &p) in data.positions.iter().enumerate() {
        let (mut down, mut up) = (Vec3::ZERO, Vec3::ZERO);
        for target in data.morphs.chunks_exact(n) {
            let d = Vec3::from(target[v][0]);
            down += d.min(Vec3::ZERO);
            up += d.max(Vec3::ZERO);
        }
        lo = lo.min(Vec3::from(p) + down);
        hi = hi.max(Vec3::from(p) + up);
    }
    Some(Aabb::from_min_max(lo, hi))
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
    let normal = texture(desc.sig.as_ref().map_or(desc.normal, |s| s.normal));
    let spec = texture(desc.sig.as_ref().map_or(desc.spec, |s| s.map));
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
        double_sided: desc.cull_mode.is_none(),
        cull_mode: desc.cull_mode,
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
    if let Some(s) = &desc.sig {
        params.sig_settings = Vec4::new(
            if s.capped { 2.0 } else { 1.0 },
            s.gloss,
            s.emission,
            f32::from(u8::from(s.enabled)),
        );
        params.spec_color = linear(s.specular);
        params.spec_st = Vec4::from(s.map_st);
        params.cap_color = linear(s.cap_color);
        params.cap_specular = linear(s.cap_specular);
        params.border_tint = linear(s.border_tint);
        params.cap_scales = Vec4::from(if s.capped { s.scales } else { desc.albedo_st });
        params.glow_color = linear(desc.color);
        params.cap_border = Vec4::from(s.cap_border);
        params.inner_border = Vec4::from(s.inner_border);
    }
    ObjectMaterial {
        base,
        extension: ObjectExtension {
            depth_write: desc.view_model && matches!(desc.alpha, Alpha::Blend),
            params,
            normal_map: normal.unwrap_or_else(|| defaults.flat.clone()),
            spec_map: spec.unwrap_or_else(|| defaults.white.clone()),
            illum_map: illum.unwrap_or_else(|| defaults.white.clone()),
            sig_albedo: texture(desc.sig.as_ref().and_then(|s| s.albedo))
                .unwrap_or_else(|| defaults.white.clone()),
            side_albedo: texture(desc.sig.as_ref().and_then(|s| s.side))
                .unwrap_or_else(|| defaults.white.clone()),
            side_normal: texture(desc.sig.as_ref().and_then(|s| s.side_normal))
                .unwrap_or_else(|| defaults.flat.clone()),
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
    // A global sky switch must not turn SIG into a Marmoset material.
    if params.sig_settings.x != 0.0 {
        return;
    }
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

fn update_upload_cost(update: &Update) -> (usize, usize) {
    match update {
        Update::Texture {
            image: Some(image), ..
        } => (image.data.as_ref().map_or(0, Vec::len), 0),
        Update::Mesh { data, .. } => (0, data.positions.len()),
        _ => (0, 0),
    }
}

fn within_upload_budget(uploaded_bytes: usize, uploaded_vertices: usize, next: &Update) -> bool {
    let (bytes, vertices) = update_upload_cost(next);
    if bytes == 0 && vertices == 0 {
        return true;
    }
    if uploaded_bytes == 0 && uploaded_vertices == 0 {
        return true;
    }
    uploaded_bytes < TEXTURE_UPLOAD_BYTES_PER_FRAME
        && uploaded_vertices < MESH_UPLOAD_VERTICES_PER_FRAME
        && uploaded_bytes.saturating_add(bytes) <= TEXTURE_UPLOAD_BYTES_PER_FRAME
        && uploaded_vertices.saturating_add(vertices) <= MESH_UPLOAD_VERTICES_PER_FRAME
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
    mut projection: Query<&mut Projection, With<Camera3d>>,
    shadows_off: Query<(), With<ShadowsOff>>,
    mut terrain_look: Option<ResMut<TerrainLook>>,
    mut effect_meshes: ResMut<EffectMeshes>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut exit: MessageWriter<AppExit>,
) {
    let _perf = crate::perf::span("objects::stream_objects");
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

    streamer
        .pending
        .extend(streamer.rx.lock().unwrap().try_iter());
    let mut uploaded_bytes = 0usize;
    let mut uploaded_vertices = 0usize;
    while let Some(next) = streamer.pending.front() {
        if !within_upload_budget(uploaded_bytes, uploaded_vertices, next) {
            break;
        }
        let (bytes, vertices) = update_upload_cost(next);
        uploaded_bytes = uploaded_bytes.saturating_add(bytes);
        uploaded_vertices = uploaded_vertices.saturating_add(vertices);
        let update = streamer.pending.pop_front().unwrap();
        match update {
            Update::Ready { ms } => {
                info!("objects: asset index ready after {ms:.0} ms");
                streamer.ready = true;
            }
            Update::Lod(settings) => {
                info!("objects: LOD settings (QualitySettings High) {settings:?}");
                commands.insert_resource(settings);
            }
            Update::Texture { id, image } => {
                if let Some(image) = image {
                    crate::perf::count(
                        "objects::image_bytes",
                        image.data.as_ref().map_or(0, Vec::len),
                    );
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
                crate::perf::count("objects::mesh_vertices", data.positions.len());
                streamer
                    .mesh_triangles
                    .insert(id, (data.indices.len() / 3) as u32);
                if let Some(aabb) = morph_bounds(&data) {
                    streamer.morph_bounds.insert(id, aabb);
                }
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
                rigs,
                culls,
                atmospheres,
                lods,
            } => {
                streamer.prefabs.insert(id, parts);
                if !atmospheres.is_empty() {
                    info!(
                        "atmosphere: prefab {id}: {} visual volumes",
                        atmospheres.len()
                    );
                    streamer.prefab_atmospheres.insert(id, atmospheres);
                }
                if !lods.is_empty() {
                    streamer.prefab_lods.insert(id, lods);
                }
                if !culls.is_empty() {
                    streamer.prefab_culls.insert(id, culls);
                }
                if !rigs.is_empty() {
                    for (r, rig) in rigs.iter().enumerate() {
                        info!(
                            "animation: {}: {} nodes, {} of {} slots move Transforms, {} skinned meshes on the GPU, {} renderers with blend shapes ({} channels driven), culling mode {}",
                            rig.name,
                            rig.nodes.len(),
                            rig.slot_nodes.iter().flatten().count(),
                            rig.slot_nodes.len(),
                            rig.skins.len(),
                            rig.shapes.len(),
                            rig.shapes
                                .iter()
                                .flat_map(|s| &s.channels)
                                .filter(|c| c.slot.is_some())
                                .count(),
                            rig.culling
                        );
                        for (k, skin) in rig.skins.iter().enumerate() {
                            let handle = bindposes.add(SkinnedMeshInverseBindposes::from(
                                skin.inverse_bindposes.clone(),
                            ));
                            streamer.bindposes.insert((id, r as u16, k as u16), handle);
                        }
                    }
                    streamer.prefab_rigs.insert(id, rigs);
                }
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
                keys,
                ms,
            } => {
                streamer.batch_ms.push(ms);
                if let Some(b) = streamer.batches.get_mut(&coord) {
                    b.instances = Some(instances);
                    b.keys = keys.into_iter().collect();
                }
            }
            Update::Spawned { id, instance } => {
                if let Some(inst) = instance.filter(|_| streamer.spawned_wanted.contains(&id)) {
                    streamer.spawned_pending.push((id, inst));
                }
            }
            Update::Scene {
                summary,
                instances,
                shows,
                aurora,
                lifepod,
                ms,
            } => {
                info!("objects: {summary} ({ms:.0} ms)");
                if let Some(data) = lifepod {
                    // Before the scene's parts are spawned: they are made
                    // with the pod's sky.
                    streamer.skies.pod = Some(data.look());
                    commands.insert_resource(crate::lifepod_light::PodLight::new(*data));
                }
                streamer.scenes.push(SceneObjects {
                    instances,
                    shows,
                    aurora,
                    shown: None,
                });
            }
            Update::Body {
                prefab,
                summary,
                ms,
            } => {
                info!("objects: {summary} ({ms:.0} ms)");
                if prefab.is_none() {
                    warn!("objects: the player body has nothing to draw");
                }
                streamer.body = prefab.map(|prefab| BodyObjects {
                    prefab,
                    spawned: false,
                });
            }
            Update::Camera { near } => {
                for mut p in &mut projection {
                    if let Projection::Perspective(p) = &mut *p {
                        info!(
                            "camera: near plane {near} m (the game's main camera; was {})",
                            p.near
                        );
                        p.near = near;
                    }
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
            for key in b.level_keys.iter().flatten() {
                streamer.keyed.remove(key);
            }
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
                if shadows_off.contains(*entity) {
                    continue;
                }
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
                    for key in std::mem::take(&mut b.level_keys[level]) {
                        streamer.keyed.remove(&key);
                    }
                    for entity in b.shown[level].take().into_iter().flatten() {
                        commands.entity(entity).despawn();
                    }
                }
                (None, true) if budget > 0 => {
                    let todo: Vec<(Instance, Option<EntityKey>)> = b
                        .instances
                        .iter()
                        .flatten()
                        .enumerate()
                        .filter(|(_, i)| i.level == level)
                        .map(|(n, i)| (*i, b.keys.get(&n).cloned()))
                        .collect();
                    let mut entities = Vec::new();
                    let mut level_keys = Vec::new();
                    let mut spawner = Spawner {
                        commands: &mut commands,
                        materials: &mut materials,
                        defaults: &defaults,
                        light: &light,
                        water: water.as_deref(),
                    };
                    for (inst, key) in &todo {
                        if key.as_ref().is_some_and(|k| streamer.gone.contains(k)) {
                            streamer.item_stats.skipped_gone += 1;
                            continue;
                        }
                        let first = entities.len();
                        streamer.spawn_instance(inst, casting, &mut spawner, &mut entities);
                        if let Some(key) = key {
                            streamer
                                .keyed
                                .entry(key.clone())
                                .or_default()
                                .extend_from_slice(&entities[first..]);
                            level_keys.push(key.clone());
                        }
                    }
                    crate::perf::count("objects::spawn_entities", entities.len());
                    budget = budget.saturating_sub(entities.len().max(1));
                    if let Some(b) = streamer.batches.get_mut(&coord) {
                        b.shown[level] = Some(entities);
                        b.level_keys[level] = level_keys;
                    }
                }
                _ => {}
            }
        }
    }

    // Scenes: spawned once (the Aurora's parts are shown or hidden by
    // `aurora::update`).
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
        let mut shown = Vec::with_capacity(todo.len());
        for inst in &todo {
            let mut entities = Vec::new();
            streamer.spawn_instance(inst, true, &mut spawner, &mut entities);
            // One root per instance (its entities keep their world
            // placement): the Aurora's swap hides the root, the LOD switch
            // the parts below it.
            let root = spawner
                .commands
                .spawn((Transform::IDENTITY, Visibility::default()))
                .id();
            for e in entities {
                spawner.commands.entity(e).insert(ChildOf(root));
            }
            shown.push(vec![root]);
        }
        streamer.scenes[i].shown = Some(shown);
    }

    // Server-made objects (M9d4b), each once.
    for (id, inst) in std::mem::take(&mut streamer.spawned_pending) {
        if !streamer.spawned_wanted.contains(&id) || streamer.spawned.contains_key(&id) {
            continue;
        }
        let mut entities = Vec::new();
        streamer.spawn_instance(&inst, true, &mut spawner, &mut entities);
        streamer.item_stats.spawned_drawn += 1;
        streamer.item_stats.spawned_entities += entities.len();
        let n = entities.len();
        streamer.spawned.insert(id, entities);
        info!(
            "items: server-made object {id} drawn ({n} entities), asked at world event {}; {} server-made objects drawn; drawing {:?}",
            streamer
                .spawned_asked
                .get(&id)
                .map_or("?".into(), |e| e.to_string()),
            streamer.spawned.len(),
            streamer.item_stats
        );
    }

    // The player's body: spawned once, hidden until `crate::body` places it.
    if let Some(prefab) = streamer.body.filter(|b| !b.spawned).map(|b| b.prefab) {
        let inst = Instance {
            level: 0,
            prefab,
            transform: Placement::default(),
            from_slot: false,
            pod_sky: false,
        };
        let mut entities = Vec::new();
        streamer.spawn_instance(&inst, true, &mut spawner, &mut entities);
        let rigs = streamer
            .prefab_rigs
            .get(&prefab)
            .cloned()
            .unwrap_or_default();
        // `spawn_instance` spawns each rig's base first.
        for &base in entities.iter().take(rigs.len()) {
            spawner
                .commands
                .entity(base)
                .insert((PlayerBodyRig, Visibility::Hidden));
        }
        let parts = streamer.prefabs.get(&prefab).map_or(&[][..], Vec::as_slice);
        let skinned = parts
            .iter()
            .filter(|p| matches!(p.rig, Some(RigPart::Skin { .. })))
            .count();
        let still = parts.iter().filter(|p| p.rig.is_none()).count();
        let only = parts.iter().filter(|p| p.shadows == SHADOWS_ONLY).count();
        let rig_list: Vec<String> = rigs
            .iter()
            .map(|r| {
                format!(
                    "{}: {} nodes, {} slots, skins with {:?} bones, culling mode {}",
                    r.name,
                    r.nodes.len(),
                    r.program.slots.len(),
                    r.skins.iter().map(|s| s.joints.len()).collect::<Vec<_>>(),
                    r.culling
                )
            })
            .collect();
        info!(
            "objects: player body spawned: {} sub-meshes ({skinned} skinned, {still} not on a rig, {only} shadows only), rigs {rig_list:?}",
            parts.len()
        );
        if still > 0 {
            warn!("objects: {still} body sub-meshes are not on the rig and stay where spawned");
        }
        streamer.body = Some(BodyObjects {
            prefab,
            spawned: true,
        });
    }

    // Other players' bodies (M10h2): the same prefab, hidden until
    // `crate::remote` places them.
    if let Some(prefab) = streamer.body.filter(|b| b.spawned).map(|b| b.prefab) {
        let rigs = streamer.prefab_rigs.get(&prefab).map_or(0, |r| r.len());
        for id in std::mem::take(&mut streamer.remote_wanted) {
            let inst = Instance {
                level: 0,
                prefab,
                transform: Placement::default(),
                from_slot: false,
                pod_sky: false,
            };
            let mut entities = Vec::new();
            streamer.spawn_instance(&inst, true, &mut spawner, &mut entities);
            for &base in entities.iter().take(rigs) {
                spawner
                    .commands
                    .entity(base)
                    .insert((RemoteBodyRig(id), Visibility::Hidden));
            }
            info!(
                "objects: player {id}'s body spawned ({} entities, {rigs} rigs)",
                entities.len()
            );
            streamer.remote_bodies.push((id, entities));
        }
    }
}

impl ObjectStreamer {
    /// Spawns another player's body (the player's own prefab) in the next
    /// update; nothing if it is there or asked for already.
    pub fn want_remote_body(&mut self, id: u16) {
        if !self.remote_wanted.contains(&id) && !self.remote_bodies.iter().any(|b| b.0 == id) {
            self.remote_wanted.push(id);
        }
    }

    /// Forgets another player's body; returns its entities to despawn.
    pub fn remove_remote_body(&mut self, id: u16) -> Vec<Entity> {
        self.remote_wanted.retain(|&w| w != id);
        match self.remote_bodies.iter().position(|b| b.0 == id) {
            Some(i) => self.remote_bodies.swap_remove(i).1,
            None => Vec::new(),
        }
    }

    /// The ids of the other players' bodies spawned or asked for.
    pub fn remote_body_ids(&self) -> Vec<u16> {
        self.remote_bodies
            .iter()
            .map(|b| b.0)
            .chain(self.remote_wanted.iter().copied())
            .collect()
    }
}

/// Records a part of a LOD group; parts not in level 0 start hidden.
fn lod_member(
    commands: &mut Commands,
    members: &mut Vec<(u16, u32, Entity, u32)>,
    lod: Option<(u16, u32)>,
    entity: Entity,
    triangles: u32,
) {
    if let Some((group, levels)) = lod {
        if levels & 1 == 0 {
            commands.entity(entity).insert(Visibility::Hidden);
        }
        members.push((group, levels, entity, triangles));
    }
}

/// M7f4f: each spawned instance's LOD groups show the level Unity would
/// for the camera (`sn_world::lod`), with the game's field of view and
/// `lodBias`.
pub fn switch_lods(
    time: Res<Time>,
    settings: Option<Res<LodSettings>>,
    mut stats: ResMut<LodStats>,
    camera: Query<(&Transform, &Projection), With<Camera3d>>,
    mut switches: Query<&mut LodSwitch>,
    mut visibility: Query<&mut Visibility>,
    mut last_log: Local<f32>,
) {
    let _perf = crate::perf::span("objects::switch_lods");
    let (Some(settings), Ok((cam, projection))) = (settings, camera.single()) else {
        return;
    };
    let fov = match projection {
        Projection::Perspective(p) => p.fov.to_degrees(),
        _ => return,
    };
    let start = Instant::now();
    let mut levels = [0usize; 5];
    let (mut culled, mut groups, mut switched) = (0, 0, 0);
    let (mut triangles, mut triangles_lod0) = (0u64, 0u64);
    // Triangle tallies only when logging (they are not the switch's cost).
    *last_log += time.delta_secs();
    let log_now = *last_log >= 2.0;
    for mut switch in &mut switches {
        let switch = &mut *switch;
        for &(group, mask, _, tris) in switch.members.iter().filter(|_| log_now) {
            let shown = switch
                .groups
                .get(usize::from(group))
                .and_then(|g| g.level)
                .is_some_and(|l| l < 32 && mask & (1 << l) != 0);
            triangles += if shown { u64::from(tris) } else { 0 };
            triangles_lod0 += if mask & 1 != 0 { u64::from(tris) } else { 0 };
        }
        for (g, state) in switch.groups.iter_mut().enumerate() {
            let d = state.center.distance(cam.translation);
            let h = sn_world::lod::relative_height(state.size, d, fov, settings.bias);
            let level = sn_world::lod::level(&state.heights, h, settings.max_level);
            groups += 1;
            match level {
                Some(l) => levels[l.min(4)] += 1,
                None => culled += 1,
            }
            if level == state.level {
                continue;
            }
            state.level = level;
            switched += 1;
            let bit = level.map_or(0, |l| if l < 32 { 1u32 << l } else { 0 });
            for &(group, mask, entity, _) in &switch.members {
                if usize::from(group) != g {
                    continue;
                }
                if let Ok(mut v) = visibility.get_mut(entity) {
                    let want = if mask & bit != 0 {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                    if *v != want {
                        *v = want;
                    }
                }
            }
        }
    }
    stats.levels = levels;
    stats.culled = culled;
    stats.groups = groups;
    stats.switches += switched;
    stats.triangles = triangles;
    stats.triangles_lod0 = triangles_lod0;
    stats.micros.push(start.elapsed().as_micros());
    if log_now {
        *last_log = 0.0;
        let mut us = std::mem::take(&mut stats.micros);
        us.sort_unstable();
        info!(
            "lod: {} groups, by level {:?} (4: 4 and more), culled {}; {} switches in 2 s; their parts' triangles {} (all at LOD 0: {}); {} µs median, {} µs worst per frame",
            stats.groups,
            stats.levels,
            stats.culled,
            stats.switches,
            stats.triangles,
            stats.triangles_lod0,
            us.get(us.len() / 2).copied().unwrap_or(0),
            us.last().copied().unwrap_or(0)
        );
        stats.switches = 0;
    }
}

/// A drawn part's sun shadow (M7f4e): none when its batch is too far
/// (`casting` false) or its renderer's `m_CastShadows` is off. Shadows-only
/// renderers (1 in the placed prefabs) are drawn as normal renderers (not
/// 1:1: Bevy has no shadow-only draw).
fn shadow_mode(commands: &mut Commands, entity: Entity, casting: bool, mode: u8) {
    let mut e = commands.entity(entity);
    if mode == SHADOWS_OFF {
        e.insert((bevy::light::NotShadowCaster, ShadowsOff));
        return;
    }
    if mode == SHADOWS_ONLY {
        e.insert((RenderLayers::layer(SHADOW_ONLY_LAYER), ShadowsOnly));
    }
    if !casting {
        e.insert(bevy::light::NotShadowCaster);
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

#[derive(Component)]
pub struct PlacedAtmosphere(sn_sim::atmosphere::Volume);

#[derive(Component)]
pub struct AutoSkyBinding {
    position: [f32; 3],
    material: u32,
    sky: SkyKey,
}

/// Run after streaming commands are applied. Adding/removing an invisible
/// volume relights existing meshes as well as meshes loaded in this frame.
/// Cached materials are keyed by sky so objects outside a volume keep theirs.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_atmosphere_skies(
    mut streamer: ResMut<ObjectStreamer>,
    water: Option<Res<WaterWorld>>,
    light: Res<GameLightImages>,
    mut materials: ResMut<Assets<ObjectMaterial>>,
    volumes: Query<(Entity, &PlacedAtmosphere)>,
    mut bindings: Query<(
        Mut<AutoSkyBinding>,
        Option<&mut MeshMaterial3d<ObjectMaterial>>,
        Option<&mut EffectPart>,
    )>,
    mut previous: Local<Vec<sn_sim::atmosphere::Volume>>,
) {
    let _perf = crate::perf::span("objects::update_atmosphere_skies");
    let mut current: Vec<_> = volumes
        .iter()
        .map(|(entity, volume)| {
            let mut v = volume.0.clone();
            v.tie = entity.to_bits();
            v
        })
        .collect();
    current.sort_by_key(|v| v.tie);
    let changed = *previous != current;
    if !changed && !bindings.iter().any(|(b, _, _)| b.is_added()) {
        return;
    }
    let streamer = &mut *streamer;
    let Some(defaults) = streamer.defaults.as_ref() else {
        return;
    };
    let mut positions = HashMap::new();
    let mut rebound = 0;
    let mut enclosed = 0;
    let mut ambiguous = 0;
    let mut selections = BTreeMap::<String, usize>::new();
    for (mut binding, material, effect) in &mut bindings {
        if !changed && !binding.is_added() {
            continue;
        }
        let key = binding.position.map(f32::to_bits);
        let sky = *positions.entry(key).or_insert_with(|| {
            let position = sn_sim::V3::from_f32(binding.position);
            let selected = sn_sim::atmosphere::enclosing(&current, position);
            let biome = match selected {
                Some(v) => {
                    enclosed += 1;
                    *selections.entry(v.biome.clone()).or_default() += 1;
                    if current.iter().any(|other| {
                        other.tie != v.tie
                            && other.priority == v.priority
                            && other.biome != v.biome
                            && sn_sim::atmosphere::contains(&other.shape, position)
                    }) {
                        ambiguous += 1;
                    }
                    Some(v.biome.as_str())
                }
                None => water.as_ref().and_then(|w| w.biome_at(binding.position)),
            };
            streamer.skies.pick(true, biome, false)
        });
        if sky == binding.sky {
            continue;
        }
        if let Some(mut material) = material {
            let handle = match streamer.materials.get(&(binding.material, sky)) {
                Some(handle) => handle.clone(),
                None => {
                    let Some(desc) = streamer.material_descs.get(&binding.material) else {
                        continue;
                    };
                    let handle = materials.add(object_material(
                        desc,
                        streamer.skies.look(sky),
                        &streamer.textures,
                        defaults,
                        &light,
                    ));
                    streamer
                        .materials
                        .insert((binding.material, sky), handle.clone());
                    handle
                }
            };
            material.0 = handle;
        }
        if let Some(mut effect) = effect {
            effect.sky = sky;
        }
        binding.sky = sky;
        rebound += 1;
    }
    if changed || rebound > 0 {
        info!(
            "atmosphere: {} active volumes; {} queried roots, {} enclosed, {} equal-priority different-biome ambiguities; {} part bindings changed; selected {:?}",
            current.len(),
            positions.len(),
            enclosed,
            ambiguous,
            rebound,
            selections
        );
    }
    *previous = current;
}

impl ObjectStreamer {
    /// The lifepod's sky now (M7f4g): relights the materials made with it,
    /// and the global ones while the player is in the pod. Returns how
    /// many materials were relit.
    pub(crate) fn set_pod_sky(
        &mut self,
        look: SkyLook,
        materials: &mut Assets<ObjectMaterial>,
        terrain_look: Option<&mut TerrainLook>,
    ) -> usize {
        self.skies.pod = Some(look);
        let mut n = self.relight(SkyKey::Pod, materials);
        if self.skies.in_pod {
            n += self.relight_global(materials, terrain_look);
        }
        n
    }

    /// The player went in or out of the pod (`MarmoLifepodSky`): the global
    /// sky's materials are relit. Returns how many.
    pub(crate) fn set_in_pod(
        &mut self,
        in_pod: bool,
        materials: &mut Assets<ObjectMaterial>,
        terrain_look: Option<&mut TerrainLook>,
    ) -> usize {
        if self.skies.in_pod == in_pod {
            return 0;
        }
        self.skies.in_pod = in_pod;
        self.relight_global(materials, terrain_look)
    }

    /// Whether the objects' point and spot lights are spawned
    /// (`--no-local-lights` off).
    pub(crate) fn lights_enabled(&self) -> bool {
        self.lights
    }

    /// The global sky's name now.
    pub(crate) fn global_sky_name(&self, pod: &str) -> String {
        if self.skies.in_pod {
            return pod.to_string();
        }
        self.skies
            .skies
            .global
            .and_then(|i| self.skies.skies.skies.get(i))
            .map_or_else(|| "none".to_string(), |s| s.name.clone())
    }

    fn relight_global(
        &mut self,
        materials: &mut Assets<ObjectMaterial>,
        terrain_look: Option<&mut TerrainLook>,
    ) -> usize {
        // Terrain grass has no `SkyApplier`: the global sky too.
        if let Some(t) = terrain_look {
            t.set_global_sky(self.skies.look(SkyKey::Global), materials);
        }
        self.relight(SkyKey::Global, materials)
    }

    fn relight(&mut self, key: SkyKey, materials: &mut Assets<ObjectMaterial>) -> usize {
        let look = self.skies.look(key).copied();
        let mut n = 0;
        for ((_, k), handle) in &self.materials {
            if *k == key
                && let Some(mut m) = materials.get_mut(handle)
            {
                apply_sky(&mut m.extension.params, look.as_ref());
                n += 1;
            }
        }
        n
    }

    /// The Aurora scene once it is spawned: its data and its parts that
    /// show only in some states, with their entities.
    pub fn aurora(&self) -> Option<(AuroraData, AuroraParts<'_>)> {
        let scene = self.scenes.iter().find(|s| s.aurora.is_some())?;
        let shown = scene.shown.as_ref()?;
        let parts = scene
            .shows
            .iter()
            .zip(shown)
            .filter(|(show, _)| **show != AuroraShow::ALWAYS)
            .map(|(show, entities)| (*show, entities.as_slice()))
            .collect();
        Some((scene.aurora?, parts))
    }

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
        if let Some(volumes) = self.prefab_atmospheres.get(&inst.prefab) {
            for template in volumes {
                match template.placed(&inst.transform, 0) {
                    Ok(volume) => {
                        let entity = s.commands.spawn(PlacedAtmosphere(volume)).id();
                        entities.push(entity);
                    }
                    Err(e) => warn!("{e}"),
                }
            }
        }
        let mut lod_members: Vec<(u16, u32, Entity, u32)> = Vec::new();
        // Animated rigs: a base entity (listed, so despawning it takes the
        // rest) and the rig's nodes below it.
        let rigs = self
            .prefab_rigs
            .get(&inst.prefab)
            .cloned()
            .unwrap_or_default();
        // (base, nodes, drawn parts, drawn parts with blend shapes).
        let mut rig_entities: Vec<RigEntities> = Vec::new();
        for rig in &rigs {
            let base = s
                .commands
                .spawn((
                    to_bevy(&inst.transform.then(&rig.base)),
                    Visibility::default(),
                ))
                .id();
            entities.push(base);
            let mut nodes: Vec<Entity> = Vec::with_capacity(rig.nodes.len());
            for (parent, local) in &rig.nodes {
                let parent = parent.map_or(base, |p| nodes[usize::from(p)]);
                nodes.push(
                    s.commands
                        .spawn((to_bevy(local), Visibility::default(), ChildOf(parent)))
                        .id(),
                );
            }
            if !rig.switched.is_empty() {
                let switched: Vec<Entity> = rig
                    .switched
                    .iter()
                    .filter_map(|&n| nodes.get(usize::from(n)).copied())
                    .collect();
                for &e in &switched {
                    s.commands.entity(e).insert(Visibility::Hidden);
                }
                s.commands.entity(base).insert(SwitchedNodes(switched));
            }
            if !rig.named.is_empty() {
                let named = rig
                    .named
                    .iter()
                    .filter_map(|(n, name)| Some((name.clone(), *nodes.get(usize::from(*n))?)))
                    .collect();
                s.commands.entity(base).insert(NamedNodes(named));
            }
            rig_entities.push((base, nodes, Vec::new(), Vec::new()));
        }
        let mut effect_root = None;
        for part in parts {
            let sky_position = inst.transform.transform_point(part.sky_position);
            let biome = s.water.and_then(|w| w.biome_at(sky_position));
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
                let world = inst.transform.then(&sn_assets::waterfall_local(
                    part.local,
                    part.waterfall_cap_height,
                    inst.transform.scale[1],
                ));
                if let Some(height) = part.waterfall_cap_height {
                    info!(
                        "objects: waterfall cap root_y={} configured_height={} world_y_scale={}",
                        inst.transform.scale[1], height, world.scale[1]
                    );
                }
                let marker = EffectPart {
                    mesh: part.mesh,
                    look,
                    textures,
                    sky: self.skies.pick(part.biome_sky, biome, inst.pod_sky),
                };
                // A rigid animated mesh belongs to its animated node, just
                // as on the ordinary object path. Its local placement is
                // already held by that node; applying it again would move
                // it twice and a world-space marker would stop following it.
                if let Some(RigPart::Node { rig, node }) = part.rig {
                    let Some((_, nodes, drawn, _)) = rig_entities.get_mut(usize::from(rig)) else {
                        continue;
                    };
                    let Some(&parent) = nodes.get(usize::from(node)) else {
                        continue;
                    };
                    let entity = s
                        .commands
                        .spawn((marker, Transform::IDENTITY, ChildOf(parent)))
                        .id();
                    if part.biome_sky && !inst.pod_sky {
                        s.commands.entity(entity).insert(AutoSkyBinding {
                            position: sky_position,
                            material: part.material,
                            sky: self.skies.pick(true, biome, false),
                        });
                    }
                    lod_member(s.commands, &mut lod_members, part.lod, entity, 0);
                    drawn.push(entity);
                    info!(
                        "objects: rigid effect mesh {} attached to rig {rig} node {node}",
                        part.mesh
                    );
                    continue;
                }
                // Keep the transforms separate: nonuniform parent scale and
                // a rotated child cannot be flattened by multiplying TRS scales.
                // Bevy propagates the full affine product, including shear.
                let parent = *effect_root.get_or_insert_with(|| {
                    let base = s
                        .commands
                        .spawn((to_bevy(&inst.transform), Visibility::default()))
                        .id();
                    entities.push(base); // Despawning the base removes its children.
                    base
                });
                let local = sn_assets::waterfall_local(
                    part.local,
                    part.waterfall_cap_height,
                    inst.transform.scale[1],
                );
                let entity = s
                    .commands
                    .spawn((marker, to_bevy(&local), ChildOf(parent)))
                    .id();
                if part.biome_sky && !inst.pod_sky {
                    s.commands.entity(entity).insert(AutoSkyBinding {
                        position: sky_position,
                        material: part.material,
                        sky: self.skies.pick(true, biome, false),
                    });
                }
                lod_member(s.commands, &mut lod_members, part.lod, entity, 0);
                continue;
            }
            let sky = self.skies.pick(part.biome_sky, biome, inst.pod_sky);
            let Some(mesh) = self.meshes.get(&part.mesh) else {
                continue;
            };
            let material = match self.materials.get(&(part.material, sky)) {
                Some(m) => m.clone(),
                None => {
                    let Some(desc) = self.material_descs.get(&part.material) else {
                        continue;
                    };
                    let look = self.skies.look(sky);
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
            if let Some(rig_part) = part.rig {
                let r = usize::from(rig_part.rig());
                let Some((base, nodes, drawn, shaped)) = rig_entities.get_mut(r) else {
                    continue;
                };
                let entity = match rig_part {
                    RigPart::Node { node, .. } => {
                        let Some(&parent) = nodes.get(usize::from(node)) else {
                            continue;
                        };
                        s.commands
                            .spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                                Transform::IDENTITY,
                                ChildOf(parent),
                            ))
                            .id()
                    }
                    RigPart::Skin { skin, .. } => {
                        let (Some(desc), Some(handle)) = (
                            rigs[r].skins.get(usize::from(skin)),
                            self.bindposes.get(&(inst.prefab, rig_part.rig(), skin)),
                        ) else {
                            continue;
                        };
                        let joints = desc
                            .joints
                            .iter()
                            .filter_map(|&j| nodes.get(usize::from(j)).copied())
                            .collect();
                        s.commands
                            .spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                                Transform::IDENTITY,
                                ChildOf(*base),
                                SkinnedMesh {
                                    inverse_bindposes: handle.clone(),
                                    joints,
                                },
                                DynamicSkinnedMeshBounds,
                            ))
                            .id()
                    }
                };
                if part.biome_sky && !inst.pod_sky {
                    s.commands.entity(entity).insert(AutoSkyBinding {
                        position: sky_position,
                        material: part.material,
                        sky,
                    });
                }
                shadow_mode(s.commands, entity, casting, part.shadows);
                // Blend shapes: the weights before the first update, and
                // (for a mesh not skinned on the GPU) bounds that hold
                // every shape.
                let shape = part
                    .shape
                    .and_then(|k| rigs[r].shapes.get(usize::from(k)).map(|d| (k, d)));
                if let Some((k, desc)) = shape {
                    let mut weights = Vec::new();
                    desc.morph_weights(None, &mut weights);
                    let mut e = s.commands.entity(entity);
                    e.insert(MeshMorphWeights::Value { weights });
                    if let (RigPart::Node { .. }, Some(aabb)) =
                        (rig_part, self.morph_bounds.get(&part.mesh))
                    {
                        e.insert((*aabb, NoAutoAabb));
                    }
                    shaped.push((entity, k));
                }
                let tris = self.mesh_triangles.get(&part.mesh).copied().unwrap_or(0);
                lod_member(s.commands, &mut lod_members, part.lod, entity, tris);
                drawn.push(entity);
                continue;
            }
            let world = inst.transform.then(&sn_assets::waterfall_local(
                part.local,
                part.waterfall_cap_height,
                inst.transform.scale[1],
            ));
            let entity = s
                .commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    to_bevy(&world),
                ))
                .id();
            if part.biome_sky && !inst.pod_sky {
                s.commands.entity(entity).insert(AutoSkyBinding {
                    position: sky_position,
                    material: part.material,
                    sky,
                });
            }
            shadow_mode(s.commands, entity, casting, part.shadows);
            let tris = self.mesh_triangles.get(&part.mesh).copied().unwrap_or(0);
            lod_member(s.commands, &mut lod_members, part.lod, entity, tris);
            entities.push(entity);
        }
        // LOD groups: switched per frame by `switch_lods`, starting at LOD 0.
        if let Some(lods) = self.prefab_lods.get(&inst.prefab)
            && !lod_members.is_empty()
        {
            let groups = lods
                .iter()
                .map(|l| {
                    let t = inst.transform.then(&l.in_prefab);
                    let scale = t.scale.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    let c = t.transform_point(l.reference_point);
                    LodState {
                        center: Vec3::new(c[0], c[1], -c[2]),
                        size: l.size * scale,
                        heights: l.heights.clone(),
                        level: Some(0),
                    }
                })
                .collect();
            entities.push(
                s.commands
                    .spawn(LodSwitch {
                        groups,
                        members: lod_members,
                    })
                    .id(),
            );
        }
        // `ShipExteriorCull`s: registered while the instance exists.
        if let Some(culls) = self.prefab_culls.get(&inst.prefab) {
            let boxes = culls.iter().map(|c| c.world(&inst.transform)).collect();
            entities.push(s.commands.spawn(ExteriorCullVolume(boxes)).id());
        }
        // Each rig's animator, on its base.
        for (rig, (base, nodes, drawn, shaped)) in rigs.iter().zip(rig_entities) {
            let slots = rig
                .slot_nodes
                .iter()
                .map(|n| n.and_then(|n| nodes.get(usize::from(n)).copied()))
                .collect();
            s.commands.entity(base).insert(AnimatedRig {
                animator: Animator::new(rig.program.clone(), rig.defaults.clone()),
                slots,
                culling: rig.culling,
                parts: drawn,
                shape_parts: shaped,
                desc: rig.clone(),
            });
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
    #[test]
    fn atmosphere_load_unload_rebinds_existing_meshes_and_effects() {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            spawn: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let (_, rx) = channel();
        let mut streamer = ObjectStreamer::receiving(shared, rx, false);
        streamer.defaults = Some(Defaults {
            flat: Handle::default(),
            white: Handle::default(),
        });
        streamer.skies.skies.biomes = vec![("outside".into(), Some(0)), ("inside".into(), Some(1))];
        let materials = Assets::<ObjectMaterial>::default();
        let outside = materials.reserve_handle();
        let inside = materials.reserve_handle();
        streamer
            .materials
            .insert((7, SkyKey::Fixed(0)), outside.clone());
        streamer
            .materials
            .insert((7, SkyKey::Fixed(1)), inside.clone());
        let mut app = App::new();
        app.insert_resource(streamer)
            .insert_resource(materials)
            .insert_resource(GameLightImages {
                params: Handle::default(),
                caustics: Handle::default(),
                spot_cookie: Handle::default(),
                has_caustics: false,
            })
            .add_systems(Update, update_atmosphere_skies);
        let binding = |position| AutoSkyBinding {
            position,
            material: 7,
            sky: SkyKey::Fixed(0),
        };
        let mesh = app
            .world_mut()
            .spawn((binding([0.0; 3]), MeshMaterial3d(outside.clone())))
            .id();
        let far = app
            .world_mut()
            .spawn((binding([20.0, 0.0, 0.0]), MeshMaterial3d(outside.clone())))
            .id();
        let effect = app
            .world_mut()
            .spawn((
                binding([0.0; 3]),
                EffectPart {
                    mesh: 0,
                    textures: std::array::from_fn(|_| Handle::default()),
                    sky: SkyKey::Fixed(0),
                    look: EffectLook::Glow(EffectValues {
                        color: [1.0; 4],
                        intensity: 1.0,
                        fresnel_fade: 0.0,
                        fresnel_pow: 1.0,
                        clip_offset: 0.0,
                        clip_fade: 0.0,
                        offset: 0.0,
                        fallof: 1.0,
                        inv_fade: 1.0,
                    }),
                },
            ))
            .id();
        app.update();
        let volume = app
            .world_mut()
            .spawn(PlacedAtmosphere(sn_sim::atmosphere::Volume {
                priority: 10,
                tie: 0,
                biome: "inside".into(),
                shape: sn_sim::collide::Shape::Sphere {
                    center: sn_sim::V3::ZERO,
                    radius: 2.0,
                },
            }))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<ObjectMaterial>>(mesh)
                .unwrap()
                .0,
            inside
        );
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<ObjectMaterial>>(far)
                .unwrap()
                .0,
            outside
        );
        assert_eq!(
            app.world().get::<EffectPart>(effect).unwrap().sky,
            SkyKey::Fixed(1)
        );
        // New bindings must be checked even while the volume roster is unchanged.
        let late = app
            .world_mut()
            .spawn((binding([0.0; 3]), MeshMaterial3d(outside.clone())))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<ObjectMaterial>>(late)
                .unwrap()
                .0,
            inside
        );
        app.world_mut().despawn(volume);
        app.update();
        for entity in [mesh, late, far] {
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<ObjectMaterial>>(entity)
                    .unwrap()
                    .0,
                outside
            );
        }
        assert_eq!(
            app.world().get::<EffectPart>(effect).unwrap().sky,
            SkyKey::Fixed(0)
        );
        // No shared material was replaced or duplicated to affect the far object.
        assert_eq!(app.world().resource::<ObjectStreamer>().materials.len(), 2);
    }

    #[test]
    fn rigid_effect_hierarchy_preserves_stretched_and_rotated_footprints() {
        use bevy::prelude::*;
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin);
        let parent = app
            .world_mut()
            .spawn(Transform::from_scale(Vec3::new(2.0, 1.0, 6.0)))
            .id();
        let local = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let child = app.world_mut().spawn((local, ChildOf(parent))).id();
        app.update();
        let global = app.world().get::<GlobalTransform>(child).unwrap();
        assert!(
            global
                .transform_point(Vec3::X)
                .distance(Vec3::new(0.0, 0.0, -6.0))
                < 1e-5
        );
        // A non-axis-aligned rotation adds shear. Keeping a hierarchy must
        // still match sequential application, including reflected parents.
        let parent_transform = Transform::from_scale(Vec3::new(-3.0, 1.0, 7.0));
        let child_transform = Transform::from_rotation(Quat::from_rotation_y(0.7));
        *app.world_mut().get_mut::<Transform>(parent).unwrap() = parent_transform;
        *app.world_mut().get_mut::<Transform>(child).unwrap() = child_transform;
        app.update();
        let global = app.world().get::<GlobalTransform>(child).unwrap();
        let point = Vec3::new(4.0, 0.0, 2.0);
        let expected = parent_transform.transform_point(child_transform.transform_point(point));
        assert!(global.transform_point(point).distance(expected) < 1e-5);
        // ChildOf owns the effect, so streaming out its base removes it too.
        app.world_mut().despawn(parent);
        assert!(app.world().get_entity(child).is_err());
    }
    use super::*;

    /// The crash of 2026-10-12 (a picked-up item, then its batch's shadow
    /// switch): every command the streaming later gives a batch's shown
    /// entities must find them.
    #[test]
    fn picked_up_entities_leave_their_batch() {
        use bevy::ecs::world::CommandQueue;
        let mut world = World::new();
        let kept = world.spawn_empty().id();
        let picked = world.spawn_empty().id();
        let mut batch = BatchObjects::default();
        batch.shown[0] = Some(vec![kept, picked]);
        // The pick-up: `hide_keys` despawns the item's entities.
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.entity(picked).despawn();
        queue.apply(&mut world);
        forget_despawned(std::iter::once(&mut batch), &HashSet::from([picked]));
        assert_eq!(batch.shown[0].as_deref(), Some(&[kept][..]));
        // The batch leaves LOD 0, then goes; without the fix both panic.
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        for e in batch.shown.iter().flatten().flatten() {
            commands.entity(*e).insert(bevy::light::NotShadowCaster);
        }
        for e in batch.shown[0].take().into_iter().flatten() {
            commands.entity(e).despawn();
        }
        queue.apply(&mut world);
        assert!(world.get_entity(kept).is_err());
        // Nothing to forget leaves the lists alone.
        let mut other = BatchObjects::default();
        other.shown[1] = Some(vec![kept]);
        forget_despawned(std::iter::once(&mut other), &HashSet::new());
        assert_eq!(other.shown[1].as_deref(), Some(&[kept][..]));
    }

    /// The same commands on a list still holding a despawned entity panic
    /// (what the fix prevents; if Bevy stops panicking here, this test
    /// says so).
    #[test]
    #[should_panic]
    fn a_command_on_a_despawned_entity_panics() {
        use bevy::ecs::world::CommandQueue;
        let mut world = World::new();
        let picked = world.spawn_empty().id();
        world.despawn(picked);
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.entity(picked).insert(bevy::light::NotShadowCaster);
        queue.apply(&mut world);
    }

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
            cull: None,
        }
    }

    #[test]
    fn shader_culling_keeps_front_back_off_and_property_defaults() {
        let mut material = particles_material("", &[]);
        let mut shader = uber_shader();
        shader.cull = Some(sn_unity::ShaderValue {
            value: 2.0,
            property: "_FaceMode".into(),
        });
        shader.defaults.insert("_FaceMode".into(), [0.0; 4]);
        assert_eq!(shader_cull(&material, &shader), None);
        for (value, face) in [
            (0.0, None),
            (1.0, Some(Face::Front)),
            (2.0, Some(Face::Back)),
        ] {
            material.floats.retain(|(name, _)| name != "_FaceMode");
            material.floats.push(("_FaceMode".into(), value));
            assert_eq!(shader_cull(&material, &shader), face);
        }
        // A fixed pass must ignore a conflicting saved material setting.
        shader.cull = Some(sn_unity::ShaderValue {
            value: 0.0,
            property: String::new(),
        });
        assert_eq!(shader_cull(&material, &shader), None);
        shader.cull = Some(sn_unity::ShaderValue {
            value: 1.0,
            property: String::new(),
        });
        assert_eq!(shader_cull(&material, &shader), Some(Face::Front));
        shader.cull = None;
        material
            .floats
            .retain(|(name, _)| name != "_MyCullVariable");
        assert_eq!(shader_cull(&material, &shader), Some(Face::Back));
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
    fn control_room_deformation_does_not_enable_soft_intersection_fading() {
        let base = "FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL WBOIT";
        let textures = [
            Some((2, [3.0, 4.0, 0.2, 0.3])),
            None,
            Some((5, [0.5, 0.6, 0.7, 0.8])),
            Some((6, [7.0, 8.0, 0.9, 0.1])),
        ];
        let mut shader = uber_shader();
        shader.defaults.insert("_DeformStrength".into(), [0.7; 4]);
        shader.defaults.insert("_RefractStrength".into(), [0.03; 4]);
        for refract in [false, true] {
            let keywords = if refract {
                format!("FX_REFRACTMAP {base}")
            } else {
                base.into()
            };
            let mut material = particles_material(&keywords, &[("_InvFade", 999.0)]);
            // A material vector with the same name must not override the
            // scalar refraction constant read by the compiled program.
            material.colors.push(("_RefractStrength".into(), [9.0; 4]));
            let values = particles_values(&material, &shader, textures).unwrap();
            assert!(values.deform && values.mul_map);
            assert!(!values.soft_edges && !values.triplanar && !values.fresnel_clip);
            assert!(values.add_fog && !values.near_clip && !values.back_cull);
            assert_eq!(values.refract, refract);
            assert_eq!(
                (values.deform_strength, values.refract_strength),
                (0.7, 0.03)
            );
            assert_eq!(values.textures, [Some(2), None, Some(5), Some(6)]);
            assert_eq!(values.main_st, [3.0, 4.0, 0.2, 0.3]);
            assert_eq!(values.main2_st, [1.0, 1.0, 0.0, 0.0]);
            assert_eq!(values.deform_st, [0.5, 0.6, 0.7, 0.8]);
            assert_eq!(values.refract_st, [7.0, 8.0, 0.9, 0.1]);
            let mut missing = textures;
            missing[3] = None;
            assert_eq!(
                particles_values(&material, &shader, missing).is_err(),
                refract
            );
            material.keywords.push_str(" FX_SOFTEDGES");
            let door = particles_values(&material, &shader, textures).unwrap();
            assert!(door.soft_edges && door.deform);
            assert_eq!(door.refract, refract);
            for extra in ["FX_TRIPLANAR", "FX_LIGHTMODE_PIXEL", "UNKNOWN"] {
                material.keywords = format!("{keywords} {extra}");
                assert!(particles_values(&material, &shader, textures).is_err());
            }
            material.keywords = keywords;
            // Culling is pass state: the column's back-culled inner layer
            // uses the same program.
            material.floats.push(("_MyCullVariable".into(), 2.0));
            material
                .floats
                .retain(|(name, value)| name != "_MyCullVariable" || *value == 2.0);
            assert!(
                particles_values(&material, &shader, textures)
                    .unwrap()
                    .back_cull
            );
        }
    }

    #[test]
    fn brine_preserves_projection_vectors_and_requires_its_decoded_state() {
        let keywords = "FX_DEFORM FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT";
        let mut m = particles_material(keywords, &[("_MyCullVariable", 2.0), ("_ClipFade", 3.0)]);
        m.colors
            .push(("_MainTex_TriplanarUV".into(), [0.2, 0.3, 0.4, 7.0]));
        m.colors
            .push(("_DeformMap_Speed".into(), [-0.2, 0.1, 0.6, 0.0]));
        for (name, vector) in &mut m.colors {
            if name == "_MainTex_Speed" {
                vector[2] = -0.7;
            } else if name == "_MainTex2_Speed" {
                vector[2] = 0.8;
            }
        }
        let mut shader = uber_shader();
        shader
            .defaults
            .insert("_MainTex2_TriplanarUV".into(), [0.5, 0.6, 0.7, 1.0]);
        let v = particles_values(&m, &shader, [None; 4]).unwrap();
        assert!(v.triplanar && v.near_clip && v.back_cull);
        assert!(v.deform && v.soft_edges && v.mul_map);
        assert!(!v.refract && !v.fresnel_clip && !v.add_fog && v.alpha_pow.is_none());
        assert_eq!(v.main_st, [0.2, 0.3, 0.4, 7.0]);
        assert_eq!(v.main2_st, [0.5, 0.6, 0.7, 1.0]);
        assert_eq!(
            (v.speed2[0], v.speed2[1], v.deform_speed_z),
            (-0.2, 0.1, 0.6)
        );
        assert_eq!(v.clip, [0.0, 3.0, -0.7, 0.8]);
        let both = particles_material(keywords, &[("_MyCullVariable", 0.0)]);
        assert!(
            !particles_values(&both, &shader, [None; 4])
                .unwrap()
                .back_cull
        );
        let front = particles_material(keywords, &[("_MyCullVariable", 1.0)]);
        assert!(
            particles_values(&front, &shader, [None; 4])
                .unwrap_err()
                .starts_with("_MyCullVariable")
        );
    }

    #[test]
    fn saved_brine_lighting_keywords_do_not_invent_normal_mapped_lighting() {
        let base = "FX_DEFORM FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT";
        let mut m = particles_material(base, &[("_MyCullVariable", 2.0)]);
        let shader = uber_shader();
        let textures = [Some((3, [1.0, 1.0, 0.0, 0.0])), None, None, None];
        let unlit = particles_values(&m, &shader, textures).unwrap();
        m.keywords = format!("FX_LIGHT_NORMALMAP {base} FX_LIGHT_2SIDED");
        // These saved properties cannot affect a program with no normal
        // sampler or lighting constants. Use synthetic, non-game values.
        m.floats.extend([
            ("_NormalStrength".into(), 7.0),
            ("_LightWrapAround".into(), 0.75),
            ("_SelfIllumination".into(), 8.0),
        ]);
        m.colors.push(("_NormalMap_Speed".into(), [9.0; 4]));
        assert_eq!(particles_values(&m, &shader, textures).unwrap(), unlit);
        // No global removal of unknown keywords: actual lighting modes and
        // partial saved sets remain undecoded, as do altered render states.
        for extra in [
            "FX_LIGHTMODE_PIXEL",
            "FX_LIGHTMODE_VERTEX",
            "FX_FORCELIGHT_VERTEX",
            "UNKNOWN",
        ] {
            let mut wrong = m.clone();
            wrong.keywords.push(' ');
            wrong.keywords.push_str(extra);
            assert!(
                particles_values(&wrong, &shader, textures)
                    .unwrap_err()
                    .contains("not decoded")
            );
        }
        for only in ["FX_LIGHT_NORMALMAP", "FX_LIGHT_2SIDED"] {
            let mut wrong = m.clone();
            wrong.keywords = format!("{base} {only}");
            assert!(particles_values(&wrong, &shader, textures).is_err());
        }
        let mut wrong = m.clone();
        wrong.floats.retain(|(name, _)| name != "_MyCullVariable");
        wrong.floats.push(("_MyCullVariable".into(), 1.0));
        assert!(
            particles_values(&wrong, &shader, textures)
                .unwrap_err()
                .starts_with("_MyCullVariable")
        );
    }

    #[test]
    fn m7g4d_sets_switch_on_their_programs_terms() {
        let shader = uber_shader();
        let values = |keywords: &str, floats: &[(&str, f32)]| {
            particles_values(&particles_material(keywords, floats), &shader, [None; 4]).unwrap()
        };
        // Tech light cone: the halo without _MainTex2, soft edges and the
        // near-clip fade.
        let cone = values(
            "FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT",
            &[("_ClipOffset", 0.5), ("_ClipFade", 3.0), ("_InvFade", 0.4)],
        );
        assert!(cone.fresnel_clip && cone.soft_edges && cone.near_clip && cone.add_fog);
        assert!(!cone.mul_map && !cone.deform && !cone.triplanar && !cone.back_cull);
        assert_eq!((cone.clip[0], cone.clip[1], cone.inv_fade), (0.5, 3.0, 0.4));
        // Gun elevator smoke: UV deformation without FX_ADDFOG, back culled.
        let smoke = values(
            "FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT",
            &[("_MyCullVariable", 2.0)],
        );
        assert!(smoke.deform && smoke.near_clip && !smoke.add_fog && !smoke.triplanar);
        assert!(smoke.back_cull);
        // Thermal reactor electricity: the control-room program with Fresnel.
        let elec = values(
            "FX_ADDFOG FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_SCROLL WBOIT",
            &[],
        );
        assert!(elec.fresnel_clip && elec.deform && !elec.soft_edges && !elec.near_clip);
        // Lava fall: projected, brightness opacity, alpha power, no near clip.
        let lava = values(
            "FX_ADDFOG FX_ALPHAPOW FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT",
            &[("_AlphaPow", 6.0)],
        );
        assert!(lava.triplanar && lava.add_fog && lava.soft_edges && !lava.near_clip);
        assert_eq!(lava.alpha_pow, Some(6.0));
        // FX_ALPHAPOW is not assumed to combine with other sets.
        let m = particles_material("FX_ADDFOG FX_ALPHAPOW FX_SCROLL WBOIT", &[]);
        assert!(
            particles_values(&m, &shader, [None; 4])
                .unwrap_err()
                .contains("not decoded")
        );
        // The orange key symbol: no FX_SCROLL, so its saved speed is unread.
        let symbol = values("FX_ADDFOG WBOIT", &[("_MyCullVariable", 2.0)]);
        assert_eq!((symbol.speed, symbol.speed2), ([0.0; 4], [0.0; 4]));
        assert!(symbol.back_cull && symbol.add_fog && !symbol.mul_map);
        assert_ne!(cone.speed, [0.0; 4]);
        // Every listed set is sorted, as the lookup expects.
        for set in PARTICLES_SETS {
            let mut sorted: Vec<&str> = set.split(' ').collect();
            sorted.sort_unstable();
            assert_eq!(sorted.join(" "), set);
            assert!(particles_variant(set).is_some());
        }
    }

    #[test]
    fn lit_particle_contract_requires_exact_sets_and_finite_input_defaults() {
        let mut shader = uber_shader();
        for (name, value) in [
            ("_AffectedByDayNightCycle", 1.0),
            ("_LightDesaturation", 0.75),
            ("_NormalSharpness", 0.35),
            ("_SelfIllumination", 0.003),
            ("_LightWrapAround", 0.2),
        ] {
            shader.defaults.insert(name.into(), [value; 4]);
        }
        let sets = [
            (
                "FX_FRESNELCLIP FX_LIGHTMODE_OFF FX_LIGHTMODE_VERTEX FX_MULMAP FX_REFRACTMAP FX_SCROLL FX_SOFTEDGES WBOIT",
                "119/437",
            ),
            (
                "FX_DEFORM FX_FRESNELCLIP FX_LIGHTMODE_PIXEL FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES FX_TRAILNOISE WBOIT",
                "142/460",
            ),
            (
                "FX_LIGHTMODE_PIXEL FX_MULMAP FX_NEARCLIP FX_REFRACTMAP FX_SCROLL WBOIT",
                "148/466",
            ),
            (
                "FX_DEFORM FX_ISMESH FX_LIGHTMODE_PIXEL FX_LIGHT_2SIDED FX_MULMAP FX_QUALITY_HIGH FX_SCROLL FX_TRIPLANAR WBOIT",
                "154/472",
            ),
        ];
        for (keywords, program) in sets {
            let m = particles_material(keywords, &[]);
            if program == "148/466" {
                let v = particles_values(
                    &m,
                    &shader,
                    [None, None, None, Some((42, [1.0, 1.0, 0.0, 0.0]))],
                )
                .unwrap();
                assert!(v.pixel_lighting && v.near_clip && v.refract && v.mul_map);
                assert!(!v.deform && !v.triplanar && !v.add_fog);
                assert_eq!(v.deform_st, [1.0, 0.75, 0.35, 0.003]);
                assert_eq!(v.clip[2], 0.2);
                assert!(
                    particles_values(&m, &shader, [None; 4])
                        .unwrap_err()
                        .contains("missing _RefractMap")
                );
                continue;
            }
            let diagnostic = particles_values(&m, &shader, [None; 4]).unwrap_err();
            assert!(diagnostic.contains(program));
            assert!(diagnostic.contains("_SelfIllumination=0.003"));
            assert!(diagnostic.contains("not connected"));
        }
        let m = particles_material("FX_LIGHTMODE_PIXEL WBOIT", &[]);
        assert!(
            particles_values(&m, &shader, [None; 4])
                .unwrap_err()
                .contains("not decoded")
        );
        let mut m = particles_material(sets[2].0, &[("_SelfIllumination", 0.5)]);
        assert_eq!(
            particles_values(&m, &shader, [None, None, None, Some((42, [1.0; 4]))])
                .unwrap()
                .deform_st[3],
            0.5
        );
        m.floats.push(("_LightDesaturation".into(), f32::NAN));
        assert!(
            particles_values(&m, &shader, [None; 4])
                .unwrap_err()
                .contains("non-finite")
        );
        shader.defaults.remove("_NormalSharpness");
        let m = particles_material(sets[2].0, &[]);
        assert!(
            particles_values(&m, &shader, [None; 4])
                .unwrap_err()
                .contains("missing _NormalSharpness")
        );
    }

    #[test]
    fn other_particles_materials_keep_the_stand_in_look() {
        // A keyword set not decoded (here: no FX_MULMAP but FX_FRESNELCLIP).
        let m = particles_material("FX_ADDFOG FX_FRESNELCLIP FX_SCROLL WBOIT", &[]);
        assert!(particles_values(&m, &uber_shader(), [None; 4]).is_err());
        // Render state the pass does not implement.
        for (name, v) in [
            ("_MyCullVariable", 1.0),
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

    #[test]
    fn upload_budget_caps_textures_and_meshes_without_starving_large_assets() {
        let make_tex = |len: usize| Update::Texture {
            id: 1,
            image: Some(Image {
                data: Some(vec![0u8; len]),
                ..Default::default()
            }),
        };
        let make_mesh = |verts: usize| Update::Mesh {
            id: 1,
            data: MeshData {
                positions: vec![[0.0; 3]; verts],
                normals: Vec::new(),
                tangents: Vec::new(),
                colors: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
                joints: Vec::new(),
                weights: Vec::new(),
                morphs: Vec::new(),
            },
        };
        // Even an oversized single texture or mesh is allowed when nothing has been uploaded yet.
        assert!(within_upload_budget(
            0,
            0,
            &make_tex(TEXTURE_UPLOAD_BYTES_PER_FRAME * 2)
        ));
        assert!(within_upload_budget(
            0,
            0,
            &make_mesh(MESH_UPLOAD_VERTICES_PER_FRAME * 2)
        ));
        // Once part of the budget is used, an update that would exceed the cap waits for next frame.
        let half_tex = TEXTURE_UPLOAD_BYTES_PER_FRAME / 2;
        assert!(within_upload_budget(half_tex, 0, &make_tex(half_tex)));
        assert!(!within_upload_budget(half_tex + 1, 0, &make_tex(half_tex)));
        let half_mesh = MESH_UPLOAD_VERTICES_PER_FRAME / 2;
        assert!(within_upload_budget(0, half_mesh, &make_mesh(half_mesh)));
        assert!(!within_upload_budget(
            0,
            half_mesh + 1,
            &make_mesh(half_mesh)
        ));
        // Non-GPU-upload metadata updates pass through without consuming upload budget.
        assert!(within_upload_budget(
            TEXTURE_UPLOAD_BYTES_PER_FRAME,
            MESH_UPLOAD_VERTICES_PER_FRAME,
            &Update::Ready { ms: 1.0 }
        ));
    }
}

#[cfg(test)]
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn audit_brine_placement_composition() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        return;
    };
    let game = sn_install::GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = sn_assets::Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let paths = game.read_prefab_database().unwrap();
    let mut templates = std::collections::HashMap::new();
    let mut placements = 0;
    let mut distorted = 0;
    let mut worst = 0.0f32;
    let mut corrected_worst = 0.0f32;
    let (batches, _) = game.object_batches().unwrap();
    for batch in batches {
        let Some(mut tree) = game.read_batch_objects(batch).unwrap() else {
            continue;
        };
        let corner = [batch.x, batch.y, batch.z].map(|v| v as f32 * 160.0);
        for object in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
            object.transform.position =
                std::array::from_fn(|i| corner[i] - sn_world::VOXEL_WORLD_OFFSET[i]);
        }
        {
            let (world, _) = tree.world_transforms();
            for (object, root) in tree.objects.iter().zip(world) {
                let Some(path) = paths.get(&object.class_id) else {
                    continue;
                };
                if !path.contains("LostRiver") || !path.contains("Lake") {
                    continue;
                }
                if !templates.contains_key(path) {
                    templates.insert(path.clone(), assets.prefab(&catalog, path).unwrap());
                }
                let prefab = &templates[path];
                for node in prefab.visible_nodes() {
                    let Some(mesh) = &node.mesh else {
                        continue;
                    };
                    let (_, geometry) = assets.mesh(mesh).unwrap();
                    let flattened = root.then(&node.in_prefab);
                    let affine = GlobalTransform::from(to_bevy(&root))
                        .mul_transform(to_bevy(&node.in_prefab));
                    let mut error = 0.0f32;
                    for &vertex in &geometry.positions {
                        let correct = root.transform_point(node.in_prefab.transform_point(vertex));
                        let actual = flattened.transform_point(vertex);
                        let bevy_point =
                            affine.transform_point(Vec3::new(vertex[0], vertex[1], -vertex[2]));
                        let expected = Vec3::new(correct[0], correct[1], -correct[2]);
                        corrected_worst = corrected_worst.max(bevy_point.distance(expected));
                        let d = (0..3)
                            .map(|i| (correct[i] - actual[i]).powi(2))
                            .sum::<f32>()
                            .sqrt();
                        error = error.max(d);
                    }
                    placements += 1;
                    if error > 0.01 {
                        distorted += 1;
                    }
                    if error > worst {
                        worst = error;
                        eprintln!(
                            "brine composition new worst={error}m root_scale={:?} root_position={:?}",
                            root.scale, root.position
                        );
                    }
                }
            }
        }
    }
    eprintln!("brine surfaces={placements} distorted={distorted} worst={worst}m");
    assert_eq!(placements, 328);
    assert_eq!(distorted, 16);
    assert!(worst > 17.0);
    eprintln!("brine corrected worst={corrected_worst}m");
    assert!(corrected_worst < 0.002);
}
