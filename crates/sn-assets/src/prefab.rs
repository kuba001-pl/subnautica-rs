//! Prefabs: an addressable prefab path → its bundle → the GameObject
//! hierarchy, with each node's mesh and materials. See
//! `docs/formats/unity.md`.

use sn_unity::{
    AssetBundleManifest, Catalog, DayNightLight, GameObject, Light, Location, LodGroup, Mesh,
    MeshFilter, MeshGeometry, MeshRenderer, SkyApplier, TransformNode,
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
    pub name: String,
    /// Index of the parent node; `None` for the root.
    pub parent: Option<usize>,
    pub local: Transform,
    /// Placement relative to the prefab root (parents applied, the root's
    /// own transform left out, as the game replaces it when placing).
    pub in_prefab: Transform,
    /// Active, and so are all its ancestors.
    pub active: bool,
    pub layer: u32,
    /// From a MeshFilter; `None` without one or with a null mesh.
    pub mesh: Option<ObjectRef>,
    /// From an enabled MeshRenderer, one per sub-mesh.
    pub materials: Vec<Option<ObjectRef>>,
    pub renderer_enabled: bool,
    /// Has a SkinnedMeshRenderer (animated mesh; not read yet).
    pub skinned: bool,
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
}

pub struct Prefab {
    /// The addressable key, e.g. `WorldEntities/…/X.prefab`.
    pub key: String,
    /// Node 0 is the root.
    pub nodes: Vec<PrefabNode>,
}

impl Prefab {
    /// Nodes we draw at full detail: active, with a mesh and an enabled
    /// renderer, outside LOD groups or in the most detailed LOD level that
    /// has such a node (LOD 0 is sometimes only a skinned mesh, which we
    /// don't read yet).
    pub fn visible_nodes(&self) -> impl Iterator<Item = &PrefabNode> {
        let drawable = |n: &&PrefabNode| n.active && n.renderer_enabled && n.mesh.is_some();
        let best = self
            .nodes
            .iter()
            .filter(drawable)
            .filter_map(|n| n.lod)
            .min();
        self.nodes
            .iter()
            .filter(drawable)
            .filter(move |n| n.lod.is_none() || n.lod == best)
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

        let mut prefab = Building::new(key);
        let mut lods: Vec<(ObjectRef, LodGroup)> = Vec::new();
        self.add_node(&root, None, true, &mut prefab, &mut lods, 0)?;

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
        Ok(Prefab {
            key: prefab.key,
            nodes: prefab
                .nodes
                .into_iter()
                .map(|n| PrefabNode {
                    name: n.name,
                    parent: n.parent,
                    local: n.local,
                    in_prefab: n.in_prefab,
                    active: n.active,
                    layer: n.layer,
                    mesh: n.mesh,
                    materials: n.materials,
                    renderer_enabled: n.renderer_enabled,
                    skinned: n.skinned,
                    lod: n.lod,
                    sky_applier: n.sky_applier,
                    lights: n.lights,
                    day_night_light: n.day_night_light,
                })
                .collect(),
        })
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
                SKINNED_MESH_RENDERER => skinned = true,
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
                    _ => {}
                },
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
        let index = prefab.nodes.len();
        prefab.nodes.push(BuildingNode {
            key: game_object.key(),
            name: go.name,
            parent,
            local,
            in_prefab,
            active,
            layer: go.layer,
            mesh,
            materials,
            renderer_enabled,
            skinned,
            lod: None,
            sky_applier: None,
            lights,
            day_night_light,
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

/// A node while the hierarchy is being read (keeps the object key for LOD
/// matching).
struct BuildingNode {
    key: (std::path::PathBuf, String, i64),
    name: String,
    parent: Option<usize>,
    local: Transform,
    in_prefab: Transform,
    active: bool,
    layer: u32,
    mesh: Option<ObjectRef>,
    materials: Vec<Option<ObjectRef>>,
    renderer_enabled: bool,
    skinned: bool,
    lod: Option<usize>,
    sky_applier: Option<i32>,
    lights: Vec<Light>,
    day_night_light: Option<DayNightLight>,
}

struct Building {
    key: String,
    nodes: Vec<BuildingNode>,
    /// `SkyApplier`s found, with the file their references are relative to.
    sky_appliers: Vec<(FileRef, SkyApplier)>,
}

impl Building {
    fn new(key: &str) -> Building {
        Building {
            key: key.to_string(),
            nodes: Vec::new(),
            sky_appliers: Vec::new(),
        }
    }
}
