//! Scenes: the game's `*.unity` bundles (e.g. `aurora.unity_….bundle`).
//! A built scene is a serialized file of GameObjects whose Transforms are
//! world placements at the top level; its meshes and materials sit in the
//! bundle's `.sharedAssets` file and in other bundles. Prefab instances are
//! already expanded at build time, so every object is in the file itself.

use std::collections::BTreeMap;

use sn_unity::{
    AutoLoadScene, CrashedShipExploder, TransformNode, parse_additional_scenes,
    parse_autoload_scenes,
};

use crate::prefab::Prefab;
use crate::{Assets, FileRef, ObjectRef, Result};

const GAME_OBJECT: i32 = 1;
const TRANSFORM: i32 = 4;
const RECT_TRANSFORM: i32 = 224;
/// Only a scene's own file has these (one each).
const RENDER_SETTINGS: i32 = 104;
const MONO_BEHAVIOUR: i32 = 114;

/// The top-level object `LightmappedPrefabs` takes from a loaded scene
/// (`LightmappedPrefabs.StandardMainObjectName`).
pub const LIGHTMAPPED_PREFAB: &str = "__LIGHTMAPPED_PREFAB__";

pub struct Scene {
    /// The bundle's file name up to `.unity`, e.g. `aurora`.
    pub name: String,
    /// The scene's serialized file (not the `.sharedAssets`).
    pub file: FileRef,
    /// One hierarchy per top-level GameObject, in path id order. Node 0's
    /// `local` is the object's world placement.
    pub roots: Vec<Prefab>,
    /// Objects in the scene file by class id.
    pub class_counts: BTreeMap<i32, usize>,
}

impl Scene {
    /// Every node of every root with its world placement.
    pub fn world_nodes(&self) -> impl Iterator<Item = (&Prefab, usize, sn_world::Transform)> {
        self.roots.iter().flat_map(|root| {
            let base = root.nodes[0].local;
            root.nodes
                .iter()
                .enumerate()
                .map(move |(i, n)| (root, i, base.then(&n.in_prefab)))
        })
    }
}

impl Scene {
    /// The scene file's MonoBehaviours of script class `class`.
    pub fn behaviours(&self, assets: &Assets, class: &str) -> Vec<ObjectRef> {
        self.file
            .objects()
            .iter()
            .filter(|o| o.class_id == MONO_BEHAVIOUR)
            .map(|o| ObjectRef {
                file: self.file.clone(),
                path_id: o.path_id,
            })
            .filter(|o| assets.script_class(o).as_deref() == Some(class))
            .collect()
    }

    /// `GameObject.SetActive` on a scene object; false if it is not in the
    /// scene.
    pub fn set_active(&mut self, object: &ObjectRef, active: bool) -> bool {
        for root in &mut self.roots {
            if let Some(i) = root.node_of(object) {
                root.set_active(i, active);
                return true;
            }
        }
        false
    }

    /// What `LightmappedPrefabs.ActivateLoadedPrefab` does to a scene
    /// spawned at start: its `__LIGHTMAPPED_PREFAB__` object goes to the
    /// origin (keeping its rotation and scale) and is activated. Returns
    /// whether the scene has one.
    pub fn spawn_lightmapped_prefab(&mut self) -> bool {
        let mut found = false;
        for root in &mut self.roots {
            if root.nodes[0].name == LIGHTMAPPED_PREFAB {
                root.nodes[0].local.position = [0.0; 3];
                root.set_active(0, true);
                found = true;
            }
        }
        found
    }

    /// Puts the Aurora in its state before (`exploded` false, a new game)
    /// or after the explosion, as `CrashedShipExploder.SwapModels` does.
    /// Returns how many objects were switched off and on; an error if the
    /// scene has no exploder or a listed object is missing.
    pub fn swap_aurora_models(
        &mut self,
        assets: &Assets,
        exploded: bool,
    ) -> Result<(usize, usize)> {
        let behaviour = self
            .behaviours(assets, "CrashedShipExploder")
            .into_iter()
            .next()
            .ok_or_else(|| format!("{}: no CrashedShipExploder", self.name))?;
        let (_, data) = behaviour.data()?;
        let exploder = CrashedShipExploder::parse(data, self.file.file().big_endian)
            .map_err(|e| format!("CrashedShipExploder: {e}"))?;
        let mut counts = (0, 0);
        for (list, active) in [
            (&exploder.disable_on_explosion, !exploded),
            (&exploder.enable_on_explosion, exploded),
        ] {
            for pptr in list {
                let Some(object) = assets.resolve(&self.file, *pptr)? else {
                    continue;
                };
                if !self.set_active(&object, active) {
                    return Err(format!(
                        "{}: exploder object {} not in the scene",
                        self.name, pptr.path_id
                    ));
                }
                if active {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
        Ok(counts)
    }
}

impl Assets<'_> {
    /// The scenes the game loads with the main scene and spawns at start,
    /// in order: `MainGameController.additionalScenes` of `main`, then each
    /// of those scenes' `LightmappedPrefabs.autoloadScenes`.
    /// Returns (additional scenes, autoload scenes).
    pub fn startup_scenes(&self) -> Result<(Vec<String>, Vec<AutoLoadScene>)> {
        let main = self.scene("main")?;
        let mut additional = Vec::new();
        for b in main.behaviours(self, "MainGameController") {
            let (_, data) = b.data()?;
            additional.extend(
                parse_additional_scenes(data, main.file.file().big_endian)
                    .map_err(|e| format!("MainGameController: {e}"))?,
            );
        }
        let mut autoload = Vec::new();
        for name in std::iter::once("main".to_string()).chain(additional.iter().cloned()) {
            let scene = self.scene(&name)?;
            for b in scene.behaviours(self, "LightmappedPrefabs") {
                let (_, data) = b.data()?;
                autoload.extend(
                    parse_autoload_scenes(data, scene.file.file().big_endian)
                        .map_err(|e| format!("LightmappedPrefabs: {e}"))?,
                );
            }
        }
        Ok((additional, autoload))
    }

    /// The class name of a MonoBehaviour's script; `None` if `behaviour` is
    /// not one or its script can't be read.
    pub fn script_class(&self, behaviour: &ObjectRef) -> Option<String> {
        crate::terrain::script_class(self, behaviour)
    }

    /// Loads the scene in the bundle whose name starts with `name.unity_`
    /// (lower case, e.g. `aurora`, `escapepod`).
    pub fn scene(&self, name: &str) -> Result<Scene> {
        let prefix = format!("{}.unity_", name.to_ascii_lowercase());
        let path = self
            .bundle_named(&prefix)
            .ok_or_else(|| format!("no scene bundle {prefix}*"))?
            .to_path_buf();
        let loaded = self.bundle(&path)?;
        let names: Vec<String> = loaded.file_names().map(String::from).collect();
        let mut file = None;
        for n in &names {
            let f = self.file(&path, n)?;
            if f.objects().iter().any(|o| o.class_id == RENDER_SETTINGS) {
                file = Some(f);
                break;
            }
        }
        let file = file.ok_or_else(|| format!("{}: no scene file", path.display()))?;
        let big_endian = file.file().big_endian;

        let mut class_counts = BTreeMap::new();
        let mut root_objects = Vec::new();
        for info in file.objects() {
            *class_counts.entry(info.class_id).or_insert(0) += 1;
            if info.class_id != TRANSFORM && info.class_id != RECT_TRANSFORM {
                continue;
            }
            let Some((_, data)) = file.object(info.path_id) else {
                continue;
            };
            let t = TransformNode::parse(data, big_endian)
                .map_err(|e| format!("{name}: Transform {}: {e}", info.path_id))?;
            if t.father.is_null()
                && let Some(go) = self.resolve(&file, t.game_object)?
            {
                root_objects.push(go);
            }
        }
        root_objects.sort_by_key(|o: &ObjectRef| o.path_id);
        let mut roots = Vec::with_capacity(root_objects.len());
        for go in &root_objects {
            if go.data()?.0.class_id != GAME_OBJECT {
                return Err(format!("{name}: root {} is not a GameObject", go.path_id));
            }
            roots.push(self.hierarchy(&format!("{name}:{}", go.path_id), go)?);
        }
        Ok(Scene {
            name: name.to_ascii_lowercase(),
            file,
            roots,
            class_counts,
        })
    }
}
