//! Prefabs: an addressable prefab path → its bundle → the GameObject
//! hierarchy, with each node's mesh and materials. See
//! `docs/formats/unity.md`.

use sn_unity::{
    ANIMATOR, Animator, AssetBundleManifest, Catalog, DayNightLight, GameObject, Light, Location,
    LodGroup, Mesh, MeshFilter, MeshGeometry, MeshRenderer, PrefabPlaceholder,
    PrefabPlaceholdersGroup, SkinnedMeshRenderer, SkyApplier, TransformNode, VfxVolumetricLight,
};
use sn_world::Transform;

use crate::terrain::script_class;
use crate::{Assets, FileRef, ObjectRef, Result};

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

/// Hierarchies deeper than this are treated as broken.
const MAX_DEPTH: usize = 64;

/// One GameObject of a prefab.
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
    /// Has a SkinnedMeshRenderer (its mesh and materials are `mesh` and
    /// `materials`; draw it through [`Prefab::skinned_geometry`]).
    pub skinned: bool,
    /// The skinned renderer's bones as node indices (`None`: a bone outside
    /// this hierarchy or missing).
    pub bones: Vec<Option<usize>>,
    /// Level in its LOD group (0 = most detailed); `None` if not in one.
    pub lod: Option<usize>,
    /// The `anchorSky` of a `SkyApplier` listing this node's renderer (its
    /// sky then comes from the biome at the object); `None`: the global sky.
    pub sky_applier: Option<i32>,
    /// The node's `Light` components (any state; check `enabled`,
    /// `is_realtime()` and the node's `active`).
    pub lights: Vec<Light>,
    /// A `DayNightLight` driving this node's light over the day.
    pub day_night_light: Option<DayNightLight>,
    /// The `VFXVolumetricLight` whose glow this node's renderer draws.
    pub volumetric_light: Option<VolumetricGlow>,
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

pub struct Prefab {
    /// The addressable key, e.g. `WorldEntities/…/X.prefab`.
    pub key: String,
    /// Node 0 is the root.
    pub nodes: Vec<PrefabNode>,
    pub placeholder_groups: Vec<PlaceholderGroup>,
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
        crate::skin::skin(geometry, &mesh.bind_poses, &bones)
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

        // LOD levels: match each group's renderers to nodes by object.
        for (file_of, group) in &lods {
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
                        node.lod = Some(node.lod.map_or(level, |l: usize| l.min(level)));
                    }
                }
            }
        }
        // Sky appliers: mark the nodes of the renderers they list.
        for (file_of, applier) in std::mem::take(&mut prefab.sky_appliers) {
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
                    skinned: n.skinned,
                    lod: n.lod,
                    sky_applier: n.sky_applier,
                    lights: n.lights,
                    day_night_light: n.day_night_light,
                    volumetric_light: n.volumetric_light,
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
                    for m in r.materials {
                        materials.push(self.resolve(file, m)?);
                    }
                }
                SKINNED_MESH_RENDERER => {
                    let r = SkinnedMeshRenderer::parse(data, big_endian)
                        .map_err(|e| format!("SkinnedMeshRenderer {}: {e}", c.path_id))?;
                    skinned = true;
                    renderer_enabled = r.renderer.enabled;
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
                    Some("SkyApplier") => {
                        let a = SkyApplier::parse(data, big_endian)
                            .map_err(|e| format!("SkyApplier {}: {e}", c.path_id))?;
                        prefab.sky_appliers.push((c.file.clone(), a));
                    }
                    Some("DayNightLight") => {
                        day_night_light = Some(
                            DayNightLight::parse(data, big_endian)
                                .map_err(|e| format!("DayNightLight {}: {e}", c.path_id))?,
                        );
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
            skinned,
            bone_keys,
            lod: None,
            sky_applier: None,
            lights,
            day_night_light,
            volumetric_light: None,
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
    skinned: bool,
    bone_keys: Vec<Option<(std::path::PathBuf, String, i64)>>,
    lod: Option<usize>,
    sky_applier: Option<i32>,
    lights: Vec<Light>,
    day_night_light: Option<DayNightLight>,
    volumetric_light: Option<VolumetricGlow>,
    placeholder: Option<String>,
    animator: Option<NodeAnimator>,
    blend_shape_weights: Vec<f32>,
}

struct Building {
    key: String,
    nodes: Vec<BuildingNode>,
    /// `SkyApplier`s found, with the file their references are relative to.
    sky_appliers: Vec<(FileRef, SkyApplier)>,
    /// `VFXVolumetricLight`s found: their file, the script, its node.
    volumetric_lights: Vec<(FileRef, VfxVolumetricLight, usize)>,
    /// `PrefabPlaceholder` components → their node.
    placeholder_components: std::collections::HashMap<(std::path::PathBuf, String, i64), usize>,
    /// `PrefabPlaceholdersGroup`s found: their file, the group, its node.
    placeholder_groups: Vec<(FileRef, PrefabPlaceholdersGroup, usize)>,
}

impl Building {
    fn new(key: &str) -> Building {
        Building {
            key: key.to_string(),
            nodes: Vec::new(),
            sky_appliers: Vec::new(),
            volumetric_lights: Vec::new(),
            placeholder_components: std::collections::HashMap::new(),
            placeholder_groups: Vec::new(),
        }
    }
}
