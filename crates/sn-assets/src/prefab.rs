//! Prefabs: an addressable prefab path → its bundle → the GameObject
//! hierarchy, with each node's mesh and materials. See
//! `docs/formats/unity.md`.

use sn_unity::{
    ANIMATOR, Animator, AssetBundleManifest, Catalog, Collider, ColliderShape, DayNightLight,
    GameObject, Light, Location, LodGroup, Mesh, MeshFilter, MeshGeometry, MeshRenderer,
    MonoBehaviourHeader, PPtr, PrefabPlaceholder, PrefabPlaceholdersGroup, SkinnedMeshRenderer,
    SkyApplier, TransformNode, VfxVolumetricLight, parse_ship_exterior_cull,
};
use sn_world::Transform;

use crate::terrain::script_class;
use crate::{Assets, FileRef, ObjectRef, Result};

/// Apply the startup rule before composing the placement: positions and the
/// middle remain stretched, while cap thickness uses its configured height.
pub fn waterfall_local(mut local: Transform, height: Option<f32>, root_y: f32) -> Transform {
    if let Some(height) = height {
        // A collapsed placement cannot produce a useful inverse scale.
        // Keep it collapsed instead of sending infinities to the renderer.
        local.scale = [1.0, if root_y != 0.0 { height / root_y } else { 0.0 }, 1.0];
    }
    local
}

const GAME_OBJECT: i32 = 1;
const TRANSFORM: i32 = 4;
/// A `RectTransform` starts with a Transform's fields.
const RECT_TRANSFORM: i32 = 224;
const MESH_RENDERER: i32 = 23;
const MESH_FILTER: i32 = 33;
const MESH: i32 = 43;
const ASSET_BUNDLE: i32 = 142;
const LOD_GROUP: i32 = 205;
const SKINNED_MESH_RENDERER: i32 = 137;
const MONO_BEHAVIOUR: i32 = 114;
const LIGHT: i32 = 108;
const BOX_COLLIDER: i32 = 65;

/// `Renderer.m_CastShadows` (`ShadowCastingMode`).
pub const SHADOWS_OFF: u8 = 0;
pub const SHADOWS_ON: u8 = 1;
pub const SHADOWS_TWO_SIDED: u8 = 2;
pub const SHADOWS_ONLY: u8 = 3;

/// Hierarchies deeper than this are treated as broken.
const MAX_DEPTH: usize = 64;

/// One GameObject of a prefab.
#[derive(Clone)]
pub struct PrefabNode {
    /// The GameObject (bundle, file, path id).
    pub object: (std::path::PathBuf, String, i64),
    pub name: String,
    /// Index of the parent node; `None` for the root.
    pub parent: Option<usize>,
    pub local: Transform,
    /// Placement relative to the prefab root (parents applied, the root's
    /// own transform left out, as the game replaces it when placing).
    pub in_prefab: Transform,
    /// Active, and so are all its ancestors.
    pub active: bool,
    /// The GameObject's own active flag.
    pub active_self: bool,
    pub layer: u32,
    /// From a MeshFilter; `None` without one or with a null mesh.
    pub mesh: Option<ObjectRef>,
    /// From an enabled MeshRenderer, one per sub-mesh.
    pub materials: Vec<Option<ObjectRef>>,
    pub renderer_enabled: bool,
    /// The renderer's `m_CastShadows` ([`SHADOWS_OFF`], [`SHADOWS_ON`],
    /// [`SHADOWS_TWO_SIDED`], [`SHADOWS_ONLY`]); 0 without a renderer.
    pub cast_shadows: u8,
    /// Has a SkinnedMeshRenderer (its mesh and materials are `mesh` and
    /// `materials`; draw it through [`Prefab::skinned_geometry`]).
    pub skinned: bool,
    /// The skinned renderer's bones as node indices (`None`: a bone outside
    /// this hierarchy or missing).
    pub bones: Vec<Option<usize>>,
    /// Level in its LOD group (0 = most detailed); `None` if not in one.
    /// The most detailed level listing it, if several do.
    pub lod: Option<usize>,
    /// Its LOD group (index in [`Prefab::lod_groups`]) and the levels
    /// listing its renderer (bit `n`: level `n`), M7f4f.
    pub lod_group: Option<(usize, u32)>,
    /// The `anchorSky` of a `SkyApplier` listing this node's renderer (its
    /// sky then comes from the biome at the object); `None`: the global sky.
    pub sky_applier: Option<i32>,
    /// Position of the owning SkyApplier relative to the prefab root.
    pub sky_applier_position: [f32; 3],
    /// The node's `Light` components (any state; check `enabled`,
    /// `is_realtime()` and the node's `active`).
    pub lights: Vec<Light>,
    /// A `DayNightLight` driving this node's light over the day.
    pub day_night_light: Option<DayNightLight>,
    /// The `VFXVolumetricLight` whose glow this node's renderer draws.
    pub volumetric_light: Option<VolumetricGlow>,
    /// Root waterfall cap height; divide by each placement's local Y scale.
    pub waterfall_cap_height: Option<f32>,
    /// The `prefabClassId` of a `PrefabPlaceholder` on this node: the game
    /// spawns that prefab here (`PlaceholderGroup`).
    pub placeholder: Option<String>,
    /// The node's `Animator` (M7f4), with its controller and avatar.
    pub animator: Option<NodeAnimator>,
    /// A skinned renderer's stored blend shape weights (0–100), one per
    /// blend shape channel of its mesh.
    pub blend_shape_weights: Vec<f32>,
}

/// A `PrefabPlaceholdersGroup`: on `Start` (the first time the prefab is
/// active) it spawns its placeholders' prefabs, each under the
/// placeholder's parent with the placeholder's local transform, when the
/// placeholder's GameObject is active (`activeSelf`) and its class id has a
/// world entity info (`docs/DESIGN.md` M7h).
#[derive(Clone, Debug)]
pub struct PlaceholderGroup {
    /// The node holding the group (`Start` needs it active).
    pub node: usize,
    pub enabled: bool,
    /// The placeholder nodes it lists, in order (references that don't
    /// resolve to a node of this hierarchy are left out).
    pub placeholders: Vec<usize>,
}

/// A `VFXVolumetricLight` as seen from the node drawing its glow.
#[derive(Clone, Debug)]
pub struct VolumetricGlow {
    pub script: VfxVolumetricLight,
    /// Its `lightSource` (else the `Light` on the script's node); `None`
    /// if neither resolves.
    pub light: Option<Light>,
    /// `Awake` sets the glow's property block: the script's node is
    /// active, the light resolves and both materials are set.
    pub sets_block: bool,
    /// And the script is enabled, so `LateUpdate` keeps the glow's
    /// renderer enabled as the light is (applied to the node's
    /// `renderer_enabled`).
    pub updates: bool,
}

/// A `ShipExteriorCull` (M7f4e): while the camera is inside one of its
/// boxes, the Aurora's exploded exterior may be hidden
/// (`sn_sim::aurora`).
#[derive(Clone, Debug)]
pub struct ExteriorCull {
    /// The node holding the script.
    pub node: usize,
    /// `Start` registers it with the manager: the script is enabled and
    /// its node active.
    pub registers: bool,
    /// Its `colliders` that resolve to a `BoxCollider` of this hierarchy.
    pub boxes: Vec<CullBox>,
    /// Listed colliders that don't (not a box, or outside the hierarchy).
    pub missing: usize,
}

/// A `BoxCollider`'s box, in its node's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullBox {
    pub node: usize,
    pub center: [f32; 3],
    pub size: [f32; 3],
}

impl CullBox {
    /// In the world with the prefab's root at `placement`: the box's
    /// center, its unit axes and its half extents along them (`size / 2`
    /// times the absolute scale; a negative size stays negative, so
    /// nothing is inside, as in the game's test).
    pub fn world(&self, prefab: &Prefab, placement: &Transform) -> WorldBox {
        let t = match prefab.nodes.get(self.node) {
            Some(n) => placement.then(&n.in_prefab),
            None => *placement,
        };
        WorldBox {
            center: t.transform_point(self.center),
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|a| t.rotate_vector(a)),
            half: [0, 1, 2].map(|a| self.size[a] * t.scale[a].abs() * 0.5),
        }
    }
}

/// An oriented box in the world (Unity coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBox {
    pub center: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub half: [f32; 3],
}

/// An enabled `LODGroup` (M7f4f).
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabLodGroup {
    /// The node holding it.
    pub node: usize,
    /// In the node's space.
    pub reference_point: [f32; 3],
    /// In the node's space (times its largest scale in the world).
    pub size: f32,
    /// Each level's screen-relative height.
    pub heights: Vec<f32>,
    /// `LODFadeMode` (0 none, 1 cross-fade, 2 speed tree).
    pub fade_mode: i32,
}

impl PrefabLodGroup {
    /// The world reference point and world size with the prefab's root
    /// at `placement`.
    pub fn world(&self, prefab: &Prefab, placement: &Transform) -> ([f32; 3], f32) {
        let t = match prefab.nodes.get(self.node) {
            Some(n) => placement.then(&n.in_prefab),
            None => *placement,
        };
        let scale = t.scale.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        (t.transform_point(self.reference_point), self.size * scale)
    }
}

#[derive(Clone)]
pub struct Prefab {
    /// The addressable key, e.g. `WorldEntities/…/X.prefab`.
    pub key: String,
    /// Node 0 is the root.
    pub nodes: Vec<PrefabNode>,
    pub placeholder_groups: Vec<PlaceholderGroup>,
    pub exterior_culls: Vec<ExteriorCull>,
    pub lod_groups: Vec<PrefabLodGroup>,
    /// Renderers listed by more than one enabled LOD group (only the
    /// first counts).
    pub lod_conflicts: usize,
}

impl Prefab {
    /// Sets a node's own active flag, as `GameObject.SetActive` does, and
    /// updates `active` below it.
    pub fn set_active(&mut self, node: usize, active: bool) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.active_self = active;
        }
        // Nodes are stored parents first.
        for i in 0..self.nodes.len() {
            let parent = self.nodes[i].parent.is_none_or(|p| self.nodes[p].active);
            self.nodes[i].active = parent && self.nodes[i].active_self;
        }
    }

    /// A skinned node's mesh moved by its bones as the hierarchy stores
    /// them, in the prefab root's space (like `in_prefab`). `None` if the
    /// node is not skinned or its mesh has no skin or no bones: then it is
    /// drawn as a plain mesh at `in_prefab`.
    pub fn skinned_geometry(
        &self,
        node: usize,
        mesh: &Mesh,
        geometry: &MeshGeometry,
    ) -> Option<MeshGeometry> {
        let n = self.nodes.get(node)?;
        if !n.skinned {
            return None;
        }
        let bones: Vec<Option<Transform>> = n
            .bones
            .iter()
            .map(|b| b.and_then(|i| self.nodes.get(i)).map(|b| b.in_prefab))
            .collect();
        // Unity applies the blend shapes first, then the skin.
        let shaped = self.shaped_geometry(node, mesh, geometry);
        crate::skin::skin(
            shaped.as_ref().unwrap_or(geometry),
            &mesh.bind_poses,
            &bones,
        )
    }

    /// A skinned node's mesh with its renderer's stored blend shape weights
    /// applied, in mesh space (M7f4d). `None` if the node is not skinned,
    /// its mesh has no blend shapes, or every stored weight is 0 (the mesh
    /// is then drawn as it is). Weights are used as stored, without the
    /// project's 0–100 clamp: every stored weight in the game is 0
    /// (`sn-inspect prefab --shapes`).
    pub fn shaped_geometry(
        &self,
        node: usize,
        mesh: &Mesh,
        geometry: &MeshGeometry,
    ) -> Option<MeshGeometry> {
        let n = self.nodes.get(node)?;
        if !n.skinned
            || mesh.blend_shapes.is_empty()
            || n.blend_shape_weights.iter().all(|&w| w == 0.0)
        {
            return None;
        }
        let mut out = geometry.clone();
        mesh.blend_shapes.apply(&mut out, &n.blend_shape_weights);
        Some(out)
    }

    /// Sets a node's local placement and updates `in_prefab` of every node.
    pub fn set_local(&mut self, node: usize, local: Transform) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.local = local;
        }
        // Nodes are stored parents first; the root's own transform is left
        // out of `in_prefab`.
        for i in 1..self.nodes.len() {
            if let Some(p) = self.nodes[i].parent {
                self.nodes[i].in_prefab = self.nodes[p].in_prefab.then(&self.nodes[i].local);
            }
        }
    }

    /// A node's placement in the world when the root is placed at its own
    /// `local` (scenes).
    pub fn world(&self, node: usize) -> Transform {
        self.nodes[0].local.then(&self.nodes[node].in_prefab)
    }

    /// The node of a GameObject.
    pub fn node_of(&self, object: &ObjectRef) -> Option<usize> {
        let key = object.key();
        self.nodes.iter().position(|n| n.object == key)
    }

    /// Nodes we draw at full detail: active, with a mesh and an enabled
    /// renderer (skinned or not), outside LOD groups or in the most detailed
    /// LOD level that has such a node.
    pub fn visible_nodes(&self) -> impl Iterator<Item = &PrefabNode> {
        self.visible().map(|(_, n)| n)
    }

    /// Every node Unity may draw (active, with a mesh and an enabled
    /// renderer), at any LOD level, with its index (M7f4f; the level
    /// shown is chosen per frame from [`PrefabNode::lod_group`]).
    pub fn drawn(&self) -> impl Iterator<Item = (usize, &PrefabNode)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.active && n.renderer_enabled && n.mesh.is_some())
    }

    /// [`Prefab::visible_nodes`] with their indices.
    pub fn visible(&self) -> impl Iterator<Item = (usize, &PrefabNode)> {
        let drawable = |n: &PrefabNode| n.active && n.renderer_enabled && n.mesh.is_some();
        let best = self
            .nodes
            .iter()
            .filter(|n| drawable(n))
            .filter_map(|n| n.lod)
            .min();
        self.nodes
            .iter()
            .enumerate()
            .filter(move |(_, n)| drawable(n) && (n.lod.is_none() || n.lod == best))
    }
}

fn expect(object: &ObjectRef, class: i32) -> Result<&[u8]> {
    let (info, data) = object.data()?;
    if info.class_id != class {
        return Err(format!(
            "{} object {}: class {} where {class} was expected",
            object.file.name, object.path_id, info.class_id
        ));
    }
    Ok(data)
}

impl Assets<'_> {
    /// Terminal startup material selection and its optimized generic avatar.
    /// Scoped to this script until other optimized families are verified.
    fn prepare_key_terminals(&self, prefab: &mut Building) -> Result<()> {
        let terminals = std::mem::take(&mut prefab.key_terminals);
        if terminals.is_empty() {
            return Ok(());
        }
        for (file, terminal, at) in terminals {
            if !terminal.enabled || !prefab.nodes[at].active {
                continue;
            }
            let material = terminal
                .selected_material()
                .map(|p| self.resolve(&file, p))
                .transpose()?
                .flatten();
            if let (Some(material), Some(face)) =
                (material, self.resolve(&file, terminal.key_face)?)
            {
                let renderer = MeshRenderer::parse(face.data()?.1, face.file.file().big_endian)
                    .map_err(|e| format!("key face {}: {e}", face.path_id))?;
                if let Some(go) = self.resolve(&face.file, renderer.game_object)?
                    && let Some(n) = prefab.nodes.iter_mut().find(|n| n.key == go.key())
                {
                    if let Some(first) = n.materials.first_mut() {
                        *first = Some(material);
                    }
                }
            }
            let animators: Vec<usize> = prefab
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    below(&prefab.nodes, *i, at)
                        && n.animator
                            .as_ref()
                            .is_some_and(|a| !a.component.has_transform_hierarchy)
                })
                .map(|(i, _)| i)
                .collect();
            for animator in animators {
                let Some(reference) = prefab.nodes[animator]
                    .animator
                    .as_ref()
                    .and_then(|a| a.avatar.as_ref())
                else {
                    continue;
                };
                let avatar = self.avatar(reference)?;
                let mut nodes = prefab.nodes.clone();
                let map = restore_avatar(&mut nodes, animator, &avatar)?;
                for i in 0..nodes.len() {
                    if !below(&nodes, i, animator)
                        || !nodes[i].skinned
                        || !nodes[i].bone_keys.is_empty()
                    {
                        continue;
                    }
                    let Some(mesh) = nodes[i].mesh.as_ref() else {
                        continue;
                    };
                    let mesh = self.mesh_info(mesh)?;
                    if mesh.bind_poses.len() != mesh.bone_name_hashes.len() {
                        return Err("optimized terminal: palette/bind-pose count mismatch".into());
                    }
                    let keys: Option<Vec<_>> = mesh
                        .bone_name_hashes
                        .iter()
                        .map(|hash| map.get(hash).map(|&index| Some(nodes[index].key.clone())))
                        .collect();
                    nodes[i].bone_keys =
                        keys.ok_or("optimized terminal: unresolved skin palette hash")?;
                }
                prefab.nodes = nodes;
            }
        }
        prefab.sort_nodes()
    }

    /// Reads `StreamingAssets/aa/catalog.json`.
    pub fn catalog(&self) -> Result<Catalog> {
        let path = self.game().data_dir.join("StreamingAssets/aa/catalog.json");
        let bytes = self.game().read_file(&path)?;
        Catalog::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The object of class `class_id` that a catalog location names: the
    /// entry with its path in the container of its bundle (the location's
    /// first dependency).
    pub fn catalog_object(&self, location: &Location, class_id: i32) -> Result<Option<ObjectRef>> {
        let bundle_id = location
            .dependencies
            .first()
            .ok_or_else(|| format!("{}: no bundle", location.internal_id))?;
        let file_name = bundle_id
            .internal_id
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&bundle_id.internal_id);
        let bundle_path = self.game().bundle_dir().join(file_name);
        let loaded = self.bundle(&bundle_path)?;
        let names: Vec<String> = loaded.file_names().map(String::from).collect();
        for name in &names {
            let file = self.file(&bundle_path, name)?;
            for info in file.objects().iter().filter(|o| o.class_id == ASSET_BUNDLE) {
                let Some((_, data)) = file.object(info.path_id) else {
                    continue;
                };
                let manifest = AssetBundleManifest::parse(data, file.file().big_endian)
                    .map_err(|e| format!("{}: AssetBundle: {e}", bundle_path.display()))?;
                for (path, pptr) in &manifest.container {
                    if path.eq_ignore_ascii_case(&location.internal_id)
                        && let Some(object) = self.resolve(&file, *pptr)?
                        && object.data()?.0.class_id == class_id
                    {
                        return Ok(Some(object));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Loads the prefab registered under `key` (e.g. a path from `prefabs.db`).
    pub fn prefab(&self, catalog: &Catalog, key: &str) -> Result<Prefab> {
        let location = catalog
            .locate(key)
            .into_iter()
            .find(|l| l.resource_type == "UnityEngine.GameObject")
            .ok_or_else(|| format!("{key}: not a GameObject in the catalog"))?;
        let root = self
            .catalog_object(&location, GAME_OBJECT)?
            .ok_or_else(|| format!("{key}: {} not found in its bundle", location.internal_id))?;
        self.hierarchy(key, &root)
    }

    /// The hierarchy below the GameObject `root` (a prefab's root or a
    /// scene's root object), named `key`.
    pub fn hierarchy(&self, key: &str, root: &ObjectRef) -> Result<Prefab> {
        let mut prefab = Building::new(key);
        let mut lods: Vec<(ObjectRef, LodGroup)> = Vec::new();
        self.add_node(root, None, true, &mut prefab, &mut lods, 0)?;
        self.prepare_key_terminals(&mut prefab)?;

        for (file, script, owner) in std::mem::take(&mut prefab.waterfalls) {
            for (reference, height) in [
                (script.top, script.top_height),
                (script.bottom, script.bottom_height),
            ] {
                let transform = self
                    .resolve(&file, reference)?
                    .ok_or_else(|| format!("{key}: missing waterfall cap Transform"))?;
                let (_, bytes) = transform.data()?;
                let t = TransformNode::parse(bytes, transform.file.file().big_endian)
                    .map_err(|e| format!("{key}: waterfall cap: {e}"))?;
                let go = self
                    .resolve(&transform.file, t.game_object)?
                    .ok_or_else(|| format!("{key}: missing waterfall cap GameObject"))?;
                let cap = prefab
                    .nodes
                    .iter()
                    .position(|n| n.key == go.key())
                    .ok_or_else(|| format!("{key}: waterfall cap outside hierarchy"))?;
                // Current waterfall prefabs have direct children of their root.
                // Reject other layouts rather than silently applying a wrong divisor.
                if owner != 0 || prefab.nodes[cap].parent != Some(owner) {
                    return Err(format!("{key}: unsupported nested waterfall cap"));
                }
                if !height.is_finite() {
                    return Err(format!("{key}: non-finite waterfall height"));
                }
                prefab.nodes[cap].waterfall_cap_height = Some(height);
            }
        }

        // LOD levels: match each group's renderers to nodes by object.
        let mut lod_groups = Vec::new();
        let mut lod_conflicts = 0;
        for (file_of, group) in &lods {
            let Some(go) = self.resolve(&file_of.file, group.game_object)? else {
                continue;
            };
            let Some(at) = prefab.nodes.iter().position(|n| n.key == go.key()) else {
                continue;
            };
            let g = lod_groups.len();
            lod_groups.push(PrefabLodGroup {
                node: at,
                reference_point: group.local_reference_point,
                size: group.size,
                heights: group
                    .lods
                    .iter()
                    .map(|l| l.screen_relative_height)
                    .collect(),
                fade_mode: group.fade_mode,
            });
            for (level, lod) in group.lods.iter().enumerate() {
                for renderer in &lod.renderers {
                    let Some(r) = self.resolve(&file_of.file, *renderer)? else {
                        continue;
                    };
                    let (_, data) = r.data()?;
                    let Ok(mr) = MeshRenderer::parse(data, r.file.file().big_endian) else {
                        continue;
                    };
                    let Some(go) = self.resolve(&r.file, mr.game_object)? else {
                        continue;
                    };
                    if let Some(i) = prefab.nodes.iter().position(|n| n.key == go.key()) {
                        let node = &mut prefab.nodes[i];
                        match node.lod_group {
                            Some((other, _)) if other != g => {
                                lod_conflicts += 1;
                                continue;
                            }
                            _ => {}
                        }
                        node.lod = Some(node.lod.map_or(level, |l: usize| l.min(level)));
                        let bit = if level < 32 { 1u32 << level } else { 0 };
                        let levels = node.lod_group.map_or(0, |(_, m)| m);
                        node.lod_group = Some((g, levels | bit));
                    }
                }
            }
        }
        // Sky appliers: mark the nodes of the renderers they list.
        for (file_of, applier, owner) in std::mem::take(&mut prefab.sky_appliers) {
            let position = applier_position(&prefab.nodes, owner)?;
            for renderer in &applier.renderers {
                let Some(r) = self.resolve(&file_of, *renderer)? else {
                    continue;
                };
                let (_, data) = r.data()?;
                let Ok(mr) = MeshRenderer::parse(data, r.file.file().big_endian) else {
                    continue;
                };
                let Some(go) = self.resolve(&r.file, mr.game_object)? else {
                    continue;
                };
                if let Some(node) = prefab.nodes.iter_mut().find(|n| n.key == go.key()) {
                    node.sky_applier = Some(applier.anchor_sky);
                    node.sky_applier_position = position;
                }
            }
        }
        // Volumetric light scripts: their glow node and light.
        for (file_of, script, at) in std::mem::take(&mut prefab.volumetric_lights) {
            let Some(go) = self.resolve(&file_of, script.volum_go)? else {
                continue;
            };
            let Some(glow) = prefab.nodes.iter().position(|n| n.key == go.key()) else {
                continue;
            };
            let light = match self.resolve(&file_of, script.light_source)? {
                Some(l) if l.data()?.0.class_id == LIGHT => {
                    let (_, data) = l.data()?;
                    Some(
                        Light::parse(data, l.file.file().big_endian)
                            .map_err(|e| format!("Light {}: {e}", l.path_id))?,
                    )
                }
                Some(_) => None,
                None => prefab.nodes[at].lights.first().cloned(),
            };
            let awake = prefab.nodes[at].active;
            // `LateUpdate`: the glow's renderer is on as the light is.
            if awake
                && script.enabled
                && let Some(l) = &light
            {
                prefab.nodes[glow].renderer_enabled = l.enabled;
            }
            prefab.nodes[glow].volumetric_light = Some(VolumetricGlow {
                updates: awake && script.enabled,
                sets_block: awake
                    && light.is_some()
                    && !script.cone_mat.is_null()
                    && !script.sphere_mat.is_null(),
                script,
                light,
            });
        }
        // Ship exterior culls: their listed box colliders → nodes.
        let mut exterior_culls = Vec::new();
        for (file_of, enabled, colliders, at) in std::mem::take(&mut prefab.exterior_culls) {
            let mut boxes = Vec::new();
            let mut missing = 0;
            for pptr in colliders {
                let Some(c) = self.resolve(&file_of, pptr)? else {
                    missing += 1;
                    continue;
                };
                let (info, data) = c.data()?;
                if info.class_id != BOX_COLLIDER {
                    missing += 1;
                    continue;
                }
                let col = Collider::parse(info.class_id, data, c.file.file().big_endian)
                    .map_err(|e| format!("BoxCollider {}: {e}", c.path_id))?;
                let node = self
                    .resolve(&c.file, col.game_object)?
                    .and_then(|go| prefab.nodes.iter().position(|n| n.key == go.key()));
                match (node, col.shape) {
                    (Some(node), ColliderShape::Box { size, center }) => {
                        boxes.push(CullBox { node, center, size });
                    }
                    _ => missing += 1,
                }
            }
            exterior_culls.push(ExteriorCull {
                node: at,
                registers: enabled && prefab.nodes[at].active,
                boxes,
                missing,
            });
        }
        // Placeholder groups: their listed placeholder components → nodes.
        let mut placeholder_groups = Vec::new();
        for (file_of, group, at) in std::mem::take(&mut prefab.placeholder_groups) {
            let mut placeholders = Vec::new();
            for p in &group.placeholders {
                let Some(c) = self.resolve(&file_of, *p)? else {
                    continue;
                };
                if let Some(&node) = prefab.placeholder_components.get(&c.key()) {
                    placeholders.push(node);
                }
            }
            placeholder_groups.push(PlaceholderGroup {
                node: at,
                enabled: group.enabled,
                placeholders,
            });
        }
        let index: std::collections::HashMap<_, usize> = prefab
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.key.clone(), i))
            .collect();
        let mut result = Prefab {
            key: prefab.key,
            placeholder_groups,
            exterior_culls,
            lod_groups,
            lod_conflicts,
            nodes: prefab
                .nodes
                .into_iter()
                .map(|n| PrefabNode {
                    bones: n
                        .bone_keys
                        .iter()
                        .map(|k| k.as_ref().and_then(|k| index.get(k).copied()))
                        .collect(),
                    object: n.key,
                    name: n.name,
                    parent: n.parent,
                    local: n.local,
                    in_prefab: n.in_prefab,
                    active: n.active,
                    active_self: n.active_self,
                    layer: n.layer,
                    mesh: n.mesh,
                    materials: n.materials,
                    renderer_enabled: n.renderer_enabled,
                    cast_shadows: n.cast_shadows,
                    skinned: n.skinned,
                    lod: n.lod,
                    lod_group: n.lod_group,
                    sky_applier: n.sky_applier,
                    sky_applier_position: n.sky_applier_position,
                    lights: n.lights,
                    day_night_light: n.day_night_light,
                    volumetric_light: n.volumetric_light,
                    waterfall_cap_height: n.waterfall_cap_height,
                    placeholder: n.placeholder,
                    animator: n.animator,
                    blend_shape_weights: n.blend_shape_weights,
                })
                .collect(),
        };

        // Anchor Pods (`Coral_reef_floating_stones_*`): the meshes were authored in Z-up
        // coordinates with stems along +Z, while the prefab's light and placement expect
        // them extending vertically along +Y. Rotate the top-level `model` child by -90 deg
        // around X (`Quaternion.Euler(-90, 0, 0)`) so the pod and vine meshes point upright.
        if key
            .to_ascii_lowercase()
            .contains("coral_reef_floating_stones")
        {
            if let Some(model_idx) = result
                .nodes
                .iter()
                .position(|n| n.name == "model" && n.parent == Some(0))
            {
                let z_up_to_y_up = Transform {
                    position: [0.0; 3],
                    rotation: [
                        -std::f32::consts::FRAC_1_SQRT_2,
                        0.0,
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                    ],
                    scale: [1.0; 3],
                };
                let new_local = z_up_to_y_up.then(&result.nodes[model_idx].local);
                result.set_local(model_idx, new_local);
            }
        }

        Ok(result)
    }

    fn add_node(
        &self,
        game_object: &ObjectRef,
        parent: Option<usize>,
        parent_active: bool,
        prefab: &mut Building,
        lods: &mut Vec<(ObjectRef, LodGroup)>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(format!("{}: hierarchy too deep", prefab.key));
        }
        let file = &game_object.file;
        let big_endian = file.file().big_endian;
        let go = GameObject::parse(expect(game_object, GAME_OBJECT)?, big_endian)
            .map_err(|e| format!("GameObject {}: {e}", game_object.path_id))?;
        let mut transform = None;
        let mut mesh = None;
        let mut materials = Vec::new();
        let mut renderer_enabled = false;
        let mut cast_shadows = 0;
        let mut skinned = false;
        let mut lights = Vec::new();
        let mut day_night_light = None;
        let mut placeholder = None;
        let mut bone_keys = Vec::new();
        let mut animator = None;
        let mut blend_shape_weights = Vec::new();
        let index = prefab.nodes.len();
        for component in &go.components {
            let Some(c) = self.resolve(file, *component)? else {
                continue;
            };
            let (info, data) = c.data()?;
            match info.class_id {
                TRANSFORM | RECT_TRANSFORM => {
                    transform = Some(
                        TransformNode::parse(data, big_endian)
                            .map_err(|e| format!("Transform {}: {e}", c.path_id))?,
                    );
                }
                MESH_FILTER => {
                    let f = MeshFilter::parse(data, big_endian)
                        .map_err(|e| format!("MeshFilter {}: {e}", c.path_id))?;
                    mesh = self.resolve(file, f.mesh)?;
                }
                MESH_RENDERER => {
                    let r = MeshRenderer::parse(data, big_endian)
                        .map_err(|e| format!("MeshRenderer {}: {e}", c.path_id))?;
                    renderer_enabled = r.enabled;
                    cast_shadows = r.cast_shadows;
                    for m in r.materials {
                        materials.push(self.resolve(file, m)?);
                    }
                }
                SKINNED_MESH_RENDERER => {
                    let r = SkinnedMeshRenderer::parse(data, big_endian)
                        .map_err(|e| format!("SkinnedMeshRenderer {}: {e}", c.path_id))?;
                    skinned = true;
                    renderer_enabled = r.renderer.enabled;
                    cast_shadows = r.renderer.cast_shadows;
                    blend_shape_weights = r.blend_shape_weights.clone();
                    mesh = self.resolve(file, r.mesh)?;
                    materials.clear();
                    for m in r.renderer.materials {
                        materials.push(self.resolve(file, m)?);
                    }
                    for bone in r.bones {
                        // Bone Transform → its GameObject's key.
                        let key = match self.resolve(file, bone)? {
                            Some(t) => {
                                let (_, data) = t.data()?;
                                let tn = TransformNode::parse(data, t.file.file().big_endian)
                                    .map_err(|e| format!("bone {}: {e}", t.path_id))?;
                                self.resolve(&t.file, tn.game_object)?.map(|g| g.key())
                            }
                            None => None,
                        };
                        bone_keys.push(key);
                    }
                }
                LIGHT => lights.push(
                    Light::parse(data, big_endian)
                        .map_err(|e| format!("Light {}: {e}", c.path_id))?,
                ),
                MONO_BEHAVIOUR => match script_class(self, &c).as_deref() {
                    Some("PrecursorKeyTerminal") => {
                        let terminal = sn_unity::PrecursorKeyTerminal::parse(data, big_endian)
                            .map_err(|e| format!("PrecursorKeyTerminal {}: {e}", c.path_id))?;
                        prefab.key_terminals.push((c.file.clone(), terminal, index));
                    }
                    Some("SkyApplier") => {
                        let a = SkyApplier::parse(data, big_endian)
                            .map_err(|e| format!("SkyApplier {}: {e}", c.path_id))?;
                        prefab
                            .sky_appliers
                            .push((c.file.clone(), a, prefab.nodes.len()));
                    }
                    Some("DayNightLight") => {
                        day_night_light = Some(
                            DayNightLight::parse(data, big_endian)
                                .map_err(|e| format!("DayNightLight {}: {e}", c.path_id))?,
                        );
                    }
                    Some("VFXWaterFall") => {
                        let v = sn_unity::VfxWaterFall::parse(data, big_endian)
                            .map_err(|e| format!("VFXWaterFall {}: {e}", c.path_id))?;
                        prefab.waterfalls.push((c.file.clone(), v, index));
                    }
                    Some("VFXVolumetricLight") => {
                        let v = VfxVolumetricLight::parse(data, big_endian)
                            .map_err(|e| format!("VFXVolumetricLight {}: {e}", c.path_id))?;
                        prefab.volumetric_lights.push((c.file.clone(), v, index));
                    }
                    Some("PrefabPlaceholder") => {
                        let p = PrefabPlaceholder::parse(data, big_endian)
                            .map_err(|e| format!("PrefabPlaceholder {}: {e}", c.path_id))?;
                        prefab.placeholder_components.insert(c.key(), index);
                        placeholder = Some(p.prefab_class_id);
                    }
                    Some("PrefabPlaceholdersGroup") => {
                        let g = PrefabPlaceholdersGroup::parse(data, big_endian)
                            .map_err(|e| format!("PrefabPlaceholdersGroup {}: {e}", c.path_id))?;
                        prefab.placeholder_groups.push((c.file.clone(), g, index));
                    }
                    Some("ShipExteriorCull") => {
                        let header = MonoBehaviourHeader::parse(data, big_endian)
                            .map_err(|e| format!("ShipExteriorCull {}: {e}", c.path_id))?;
                        let colliders = parse_ship_exterior_cull(data, big_endian)
                            .map_err(|e| format!("ShipExteriorCull {}: {e}", c.path_id))?;
                        prefab.exterior_culls.push((
                            c.file.clone(),
                            header.enabled,
                            colliders,
                            index,
                        ));
                    }
                    _ => {}
                },
                ANIMATOR => {
                    let a = Animator::parse(data, big_endian)
                        .map_err(|e| format!("Animator {}: {e}", c.path_id))?;
                    animator = Some(NodeAnimator {
                        controller: self.resolve(file, a.controller)?,
                        avatar: self.resolve(file, a.avatar)?,
                        component: a,
                    });
                }
                LOD_GROUP => {
                    let g = LodGroup::parse(data, big_endian)
                        .map_err(|e| format!("LODGroup {}: {e}", c.path_id))?;
                    if g.enabled {
                        lods.push((c.clone(), g));
                    }
                }
                _ => {}
            }
        }
        let transform =
            transform.ok_or_else(|| format!("{}: GameObject without a Transform", go.name))?;
        let local = Transform {
            position: transform.position,
            rotation: transform.rotation,
            scale: transform.scale,
        };
        let in_prefab = match parent {
            None => Transform::default(),
            Some(p) => prefab.nodes[p].in_prefab.then(&local),
        };
        let active = parent_active && go.active;
        prefab.nodes.push(BuildingNode {
            key: game_object.key(),
            name: go.name,
            parent,
            local,
            in_prefab,
            active,
            active_self: go.active,
            layer: go.layer,
            mesh,
            materials,
            renderer_enabled,
            cast_shadows,
            skinned,
            bone_keys,
            lod: None,
            lod_group: None,
            sky_applier: None,
            sky_applier_position: [0.0; 3],
            lights,
            day_night_light,
            volumetric_light: None,
            waterfall_cap_height: None,
            placeholder,
            animator,
            blend_shape_weights,
        });
        for child in &transform.children {
            let Some(t) = self.resolve(file, *child)? else {
                continue;
            };
            let (info, data) = t.data()?;
            if info.class_id != TRANSFORM && info.class_id != RECT_TRANSFORM {
                return Err(format!(
                    "{} object {}: class {} is not a Transform",
                    t.file.name, t.path_id, info.class_id
                ));
            }
            let child = TransformNode::parse(data, t.file.file().big_endian)
                .map_err(|e| format!("Transform {}: {e}", t.path_id))?;
            let Some(child_go) = self.resolve(&t.file, child.game_object)? else {
                continue;
            };
            self.add_node(&child_go, Some(index), active, prefab, lods, depth + 1)?;
        }
        Ok(())
    }

    /// Reads a texture with its pixel data (inline or from a resource file).
    pub fn texture(&self, object: &ObjectRef) -> Result<crate::TerrainTexture> {
        crate::terrain::load_texture(self, object)
    }

    /// Reads a mesh without decoding its geometry.
    pub fn mesh_info(&self, object: &ObjectRef) -> Result<Mesh> {
        let data = expect(object, MESH)?;
        Mesh::parse(data, object.file.file().big_endian)
            .map_err(|e| format!("Mesh {}: {e}", object.path_id))
    }

    /// Reads a mesh and decodes its geometry (vertex data inline or from
    /// the bundle's resource file).
    pub fn mesh(&self, object: &ObjectRef) -> Result<(Mesh, MeshGeometry)> {
        let data = expect(object, MESH)?;
        let mesh = Mesh::parse(data, object.file.file().big_endian)
            .map_err(|e| format!("Mesh {}: {e}", object.path_id))?;
        let geometry = match &mesh.stream {
            None => mesh.decode(&mesh.vertex_data),
            Some(stream) => {
                let bytes = object.file.resource_range(
                    self.game(),
                    stream.file_name(),
                    stream.offset,
                    stream.size as usize,
                )?;
                mesh.decode(&bytes)
            }
        }
        .map_err(|e| format!("Mesh {} ({}): {e}", object.path_id, mesh.name))?;
        Ok((mesh, geometry))
    }
}

/// An `Animator` on a node, with what it references.
#[derive(Clone)]
pub struct NodeAnimator {
    pub component: Animator,
    pub controller: Option<ObjectRef>,
    pub avatar: Option<ObjectRef>,
}

/// A node while the hierarchy is being read (keeps the object key for LOD
/// matching).
#[derive(Clone)]
struct BuildingNode {
    key: (std::path::PathBuf, String, i64),
    name: String,
    parent: Option<usize>,
    local: Transform,
    in_prefab: Transform,
    active: bool,
    active_self: bool,
    layer: u32,
    mesh: Option<ObjectRef>,
    materials: Vec<Option<ObjectRef>>,
    renderer_enabled: bool,
    cast_shadows: u8,
    skinned: bool,
    bone_keys: Vec<Option<(std::path::PathBuf, String, i64)>>,
    lod: Option<usize>,
    lod_group: Option<(usize, u32)>,
    sky_applier: Option<i32>,
    sky_applier_position: [f32; 3],
    lights: Vec<Light>,
    day_night_light: Option<DayNightLight>,
    volumetric_light: Option<VolumetricGlow>,
    waterfall_cap_height: Option<f32>,
    placeholder: Option<String>,
    animator: Option<NodeAnimator>,
    blend_shape_weights: Vec<f32>,
}

struct Building {
    key: String,
    nodes: Vec<BuildingNode>,
    /// `SkyApplier`s found, with the file their references are relative to.
    sky_appliers: Vec<(FileRef, SkyApplier, usize)>,
    /// `VFXVolumetricLight`s found: their file, the script, its node.
    volumetric_lights: Vec<(FileRef, VfxVolumetricLight, usize)>,
    /// `PrefabPlaceholder` components → their node.
    placeholder_components: std::collections::HashMap<(std::path::PathBuf, String, i64), usize>,
    /// `PrefabPlaceholdersGroup`s found: their file, the group, its node.
    placeholder_groups: Vec<(FileRef, PrefabPlaceholdersGroup, usize)>,
    /// `ShipExteriorCull`s found: their file, enabled, colliders, node.
    exterior_culls: Vec<(FileRef, bool, Vec<PPtr>, usize)>,
    waterfalls: Vec<(FileRef, sn_unity::VfxWaterFall, usize)>,
    key_terminals: Vec<(FileRef, sn_unity::PrecursorKeyTerminal, usize)>,
}

/// Apply each local transform to the origin, leaving the replaced root out.
fn applier_position(nodes: &[BuildingNode], owner: usize) -> Result<[f32; 3]> {
    let mut point = [0.0; 3];
    let mut at = owner;
    for _ in 0..=nodes.len() {
        let node = nodes.get(at).ok_or("sky applier: missing owner")?;
        let Some(parent) = node.parent else {
            return Ok(point);
        };
        point = node.local.transform_point(point);
        at = parent;
    }
    Err("sky applier: cyclic owner hierarchy".into())
}

fn below(nodes: &[BuildingNode], node: usize, root: usize) -> bool {
    let mut at = Some(node);
    for _ in 0..=nodes.len() {
        let Some(i) = at else { return false };
        if i == root {
            return true;
        }
        at = nodes.get(i).and_then(|n| n.parent);
    }
    false
}

/// Reconstruct an optimized avatar's hidden nodes and restore exposed nodes
/// to their original paths. No game-specific paths or pose values are used.
fn restore_avatar(
    nodes: &mut Vec<BuildingNode>,
    root: usize,
    avatar: &sn_unity::Avatar,
) -> Result<std::collections::HashMap<u32, usize>> {
    use std::collections::{HashMap, HashSet};
    let count = avatar.skeleton_ids.len();
    if count == 0
        || avatar.human_nodes != 0
        || avatar.skeleton_parents.len() != count
        || avatar.default_pose.len() != count
        || nodes.get(root).is_none()
    {
        return Err("optimized terminal: invalid generic avatar arrays".into());
    }
    let names: HashMap<_, _> = avatar
        .names
        .iter()
        .map(|(id, path)| (*id, path.as_str()))
        .collect();
    let original = nodes.len();
    let template = nodes[root].clone();
    let mut indices = Vec::with_capacity(count);
    let mut used = HashSet::new();
    let mut map = HashMap::new();
    for (i, (&id, &parent)) in avatar
        .skeleton_ids
        .iter()
        .zip(&avatar.skeleton_parents)
        .enumerate()
    {
        let path = names
            .get(&id)
            .ok_or("optimized terminal: missing avatar path")?;
        if sn_unity::name_hash(path) != id || map.contains_key(&id) {
            return Err("optimized terminal: duplicate or invalid avatar path hash".into());
        }
        if i == 0 {
            if parent != -1 || !path.is_empty() {
                return Err("optimized terminal: invalid avatar root".into());
            }
            indices.push(root);
            used.insert(root);
            map.insert(id, root);
            continue;
        }
        let parent = usize::try_from(parent)
            .ok()
            .filter(|p| *p < i)
            .ok_or("optimized terminal: avatar parents are not topological")?;
        let name = path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .ok_or("optimized terminal: empty bone name")?;
        let parent_path = names
            .get(&avatar.skeleton_ids[parent])
            .ok_or("optimized terminal: missing parent path")?;
        let expected = if parent_path.is_empty() {
            name.to_string()
        } else {
            format!("{parent_path}/{name}")
        };
        if expected != *path {
            return Err("optimized terminal: path/parent mismatch".into());
        }
        let candidates: Vec<usize> = (0..original)
            .filter(|&n| !used.contains(&n) && below(nodes, n, root) && nodes[n].name == name)
            .collect();
        if candidates.len() > 1 {
            return Err("optimized terminal: ambiguous exposed bone name".into());
        }
        let index = if let Some(&index) = candidates.first() {
            index
        } else {
            let mut node = template.clone();
            // Synthetic identity is separate from serialized object identities.
            node.key.1 = format!("{}#avatar:{}", node.key.1, template.key.2);
            node.key.2 = i as i64;
            node.name = name.to_string();
            node.mesh = None;
            node.materials.clear();
            node.renderer_enabled = false;
            node.cast_shadows = 0;
            node.skinned = false;
            node.bone_keys.clear();
            node.lod = None;
            node.lod_group = None;
            node.lights.clear();
            node.day_night_light = None;
            node.volumetric_light = None;
            node.placeholder = None;
            node.animator = None;
            node.blend_shape_weights.clear();
            nodes.push(node);
            nodes.len() - 1
        };
        let pose = &avatar.default_pose[i];
        if pose
            .t
            .iter()
            .chain(&pose.q)
            .chain(&pose.s)
            .any(|v| !v.is_finite())
        {
            return Err("optimized terminal: non-finite avatar pose".into());
        }
        nodes[index].parent = Some(indices[parent]);
        nodes[index].local = Transform {
            position: pose.t,
            rotation: pose.q,
            scale: pose.s,
        };
        used.insert(index);
        indices.push(index);
        map.insert(id, index);
    }
    Ok(map)
}

impl Building {
    /// Re-parenting exposed avatar nodes can put parents after children.
    /// Restore parents-first order and update all cached node indices.
    fn sort_nodes(&mut self) -> Result<()> {
        let mut remap = vec![usize::MAX; self.nodes.len()];
        let mut order = Vec::with_capacity(self.nodes.len());
        while order.len() < self.nodes.len() {
            let before = order.len();
            for (i, node) in self.nodes.iter().enumerate() {
                if remap[i] == usize::MAX
                    && node
                        .parent
                        .is_none_or(|p| remap.get(p).is_some_and(|v| *v != usize::MAX))
                {
                    remap[i] = order.len();
                    order.push(i);
                }
            }
            if before == order.len() {
                return Err("optimized terminal: cyclic or invalid hierarchy".into());
            }
        }
        let mut sorted = Vec::<BuildingNode>::with_capacity(order.len());
        for i in order {
            let mut node = self.nodes[i].clone();
            node.parent = node.parent.map(|p| remap[p]);
            if let Some(parent) = node.parent {
                node.in_prefab = sorted[parent].in_prefab.then(&node.local);
                node.active = node.active_self && sorted[parent].active;
            } else {
                node.in_prefab = Transform::default();
            }
            sorted.push(node);
        }
        self.nodes = sorted;
        for (_, _, at) in &mut self.sky_appliers {
            *at = remap[*at];
        }
        for (_, _, at) in &mut self.volumetric_lights {
            *at = remap[*at];
        }
        for at in self.placeholder_components.values_mut() {
            *at = remap[*at];
        }
        for (_, _, at) in &mut self.placeholder_groups {
            *at = remap[*at];
        }
        for (_, _, _, at) in &mut self.exterior_culls {
            *at = remap[*at];
        }
        Ok(())
    }

    fn new(key: &str) -> Building {
        Building {
            key: key.to_string(),
            nodes: Vec::new(),
            sky_appliers: Vec::new(),
            volumetric_lights: Vec::new(),
            placeholder_components: std::collections::HashMap::new(),
            placeholder_groups: Vec::new(),
            exterior_culls: Vec::new(),
            key_terminals: Vec::new(),
            waterfalls: Vec::new(),
        }
    }
}

#[cfg(test)]
mod optimized_tests {
    use super::*;

    #[test]
    fn waterfall_caps_keep_thickness_and_stretch_their_positions() {
        let cap = Transform {
            position: [0.0, -10.0, 0.0],
            ..Transform::default()
        };
        for y in [0.25, 1.0, 12.0, -3.0] {
            let root = Transform {
                scale: [2.0, y, 0.5],
                ..Transform::default()
            };
            let world = root.then(&waterfall_local(cap, Some(2.5), y));
            assert!((world.scale[1] - 2.5).abs() < 1e-6);
            assert_eq!(world.scale[0], 2.0);
            assert_eq!(world.scale[2], 0.5);
            assert_eq!(world.position[1], -10.0 * y);
            assert_eq!(waterfall_local(cap, None, y), cap);
        }
        assert!(
            waterfall_local(cap, Some(2.5), 0.0)
                .scale
                .iter()
                .all(|v| v.is_finite())
        );
    }

    fn node(name: &str, parent: Option<usize>) -> BuildingNode {
        BuildingNode {
            key: (
                "synthetic".into(),
                "file".into(),
                if parent.is_none() { 1 } else { 2 },
            ),
            name: name.into(),
            parent,
            local: Transform::default(),
            in_prefab: Transform::default(),
            active: true,
            active_self: true,
            layer: 0,
            mesh: None,
            materials: Vec::new(),
            renderer_enabled: false,
            cast_shadows: 0,
            skinned: false,
            bone_keys: Vec::new(),
            lod: None,
            lod_group: None,
            sky_applier: None,
            sky_applier_position: [0.0; 3],
            lights: Vec::new(),
            day_night_light: None,
            volumetric_light: None,
            waterfall_cap_height: None,
            placeholder: None,
            animator: None,
            blend_shape_weights: Vec::new(),
        }
    }

    #[test]
    fn sky_query_uses_owner_position_and_preserves_transform_order() {
        let mut nodes = vec![
            node("root", None),
            node("room", Some(0)),
            node("applier", Some(1)),
        ];
        nodes[0].local.position = [100.0; 3]; // replaced by the placement
        nodes[1].local.scale = [2.0, 1.0, 6.0];
        nodes[2].local.position = [3.0, 0.0, 2.0];
        nodes[2].local.rotation = [
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        assert_eq!(applier_position(&nodes, 2).unwrap(), [6.0, 0.0, 12.0]);
        assert_eq!(applier_position(&nodes, 0).unwrap(), [0.0; 3]);
        nodes[1].parent = Some(2);
        assert!(applier_position(&nodes, 2).is_err());
    }

    fn avatar() -> sn_unity::Avatar {
        let paths = ["", "Hidden", "Hidden/Face"];
        let pose = sn_unity::Xform {
            t: [0.0; 3],
            q: [0.0, 0.0, 0.0, 1.0],
            s: [1.0; 3],
        };
        sn_unity::Avatar {
            name: "synthetic".into(),
            skeleton_parents: vec![-1, 0, 1],
            skeleton_ids: paths.iter().map(|p| sn_unity::name_hash(p)).collect(),
            skeleton_pose: vec![pose; 3],
            default_pose: vec![pose; 3],
            human_nodes: 0,
            root_motion_bone: -1,
            names: paths
                .iter()
                .map(|p| (sn_unity::name_hash(p), p.to_string()))
                .collect(),
        }
    }

    #[test]
    fn hidden_parent_restores_exposed_node_and_preserves_references() {
        let mut b = Building::new("synthetic");
        b.nodes = vec![node("Animator", None), node("Face", Some(0))];
        b.placeholder_components.insert(b.nodes[1].key.clone(), 1);
        let mut a = avatar();
        a.default_pose[1].t = [0.0, 2.0, 0.0];
        a.default_pose[2].t = [0.0, 1.0, 0.0];
        let map = restore_avatar(&mut b.nodes, 0, &a).unwrap();
        assert_eq!(map[&sn_unity::name_hash("Hidden/Face")], 1);
        assert_eq!(b.nodes.len(), 3);
        b.nodes[1].bone_keys = vec![Some(b.nodes[2].key.clone())];
        b.sort_nodes().unwrap();
        assert_eq!(b.nodes[1].name, "Hidden");
        assert_eq!(b.nodes[2].name, "Face");
        assert_eq!(b.nodes[2].parent, Some(1));
        assert_eq!(b.nodes[2].in_prefab.position, [0.0, 3.0, 0.0]);
        assert_eq!(*b.placeholder_components.values().next().unwrap(), 2);
        assert_eq!(b.nodes[2].bone_keys[0], Some(b.nodes[1].key.clone()));
    }

    #[test]
    fn invalid_avatar_arrays_parents_and_ambiguous_names_are_rejected() {
        let make = || vec![node("Animator", None), node("Face", Some(0))];
        let mut a = avatar();
        a.default_pose.pop();
        assert!(restore_avatar(&mut make(), 0, &a).is_err());
        let mut a = avatar();
        a.skeleton_parents[1] = 2;
        assert!(restore_avatar(&mut make(), 0, &a).is_err());
        let mut nodes = make();
        nodes.push(node("Face", Some(0)));
        assert!(restore_avatar(&mut nodes, 0, &avatar()).is_err());
        let mut a = avatar();
        a.default_pose[1].t[0] = f32::NAN;
        assert!(restore_avatar(&mut make(), 0, &a).is_err());
    }
}
