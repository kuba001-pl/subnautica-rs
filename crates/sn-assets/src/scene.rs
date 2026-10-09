//! Scenes: the game's `*.unity` bundles (e.g. `aurora.unity_….bundle`).
//! A built scene is a serialized file of GameObjects whose Transforms are
//! world placements at the top level; its meshes and materials sit in the
//! bundle's `.sharedAssets` file and in other bundles. Prefab instances are
//! already expanded at build time, so every object is in the file itself.

use std::collections::BTreeMap;

use sn_unity::{
    AutoLoadScene, CrashedShipExploder, EscapePod, MonoBehaviourHeader, PrefabSpawner, SpawnPrefab,
    TransformNode, parse_additional_scenes, parse_autoload_scenes, parse_random_start,
};
use sn_world::{StartMap, Transform};

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

/// What a scene's spawner will put in the world in a new game.
pub struct SceneSpawn {
    /// The spawner's GameObject name.
    pub name: String,
    pub spawner: PrefabSpawner,
    /// For `SpawnPrefab::Object`: the prefab's root GameObject.
    pub object: Option<ObjectRef>,
    /// World placement of the spawned object's parent.
    pub parent: Transform,
    /// The spawner's own local placement.
    pub spawner_local: Transform,
}

impl SceneSpawn {
    /// The spawned root's world placement, given the prefab root's own
    /// local placement (`PrefabSpawnBase.SpawnObj`: instantiated under the
    /// parent keeping the prefab's local values, then reset as the flags
    /// say).
    pub fn placement(&self, prefab_root: &Transform) -> Transform {
        let s = &self.spawner;
        let local = if s.use_prefab_transform_as_local {
            *prefab_root
        } else if s.use_current_transform_as_local {
            Transform {
                position: self.spawner_local.position,
                rotation: self.spawner_local.rotation,
                scale: prefab_root.scale,
            }
        } else if s.keep_scale {
            Transform {
                scale: prefab_root.scale,
                ..Transform::default()
            }
        } else {
            Transform::default()
        };
        self.parent.then(&local)
    }
}

/// Where a point lands in the inverse of `t` (`t` without shear).
fn inverse_point(t: &Transform, p: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = t.rotation;
    let back = Transform {
        rotation: [-x, -y, -z, w],
        ..Transform::default()
    };
    let d = [0, 1, 2].map(|a| p[a] - t.position[a]);
    let r = back
        .then(&Transform {
            position: d,
            ..Transform::default()
        })
        .position;
    [0, 1, 2].map(|a| {
        if t.scale[a] != 0.0 {
            r[a] / t.scale[a]
        } else {
            0.0
        }
    })
}

impl Scene {
    /// (root, node) of a GameObject.
    fn locate(&self, object: &ObjectRef) -> Option<(usize, usize)> {
        self.roots
            .iter()
            .enumerate()
            .find_map(|(r, root)| Some((r, root.node_of(object)?)))
    }

    /// (root, node) of the GameObject a Transform reference belongs to.
    fn locate_transform(
        &self,
        assets: &Assets,
        pptr: sn_unity::PPtr,
    ) -> Result<Option<(usize, usize)>> {
        let Some(t) = assets.resolve(&self.file, pptr)? else {
            return Ok(None);
        };
        let (_, data) = t.data()?;
        let tn = TransformNode::parse(data, t.file.file().big_endian)
            .map_err(|e| format!("Transform {}: {e}", t.path_id))?;
        Ok(assets
            .resolve(&t.file, tn.game_object)?
            .and_then(|go| self.locate(&go)))
    }

    /// `EscapePod.StartAtPosition`: moves the GameObject with the
    /// `EscapePod` script so it sits at `point` in the world. Returns the
    /// world placement of its `playerSpawn`, where the player starts.
    pub fn place_escape_pod(&mut self, assets: &Assets, point: [f32; 3]) -> Result<Transform> {
        let behaviour = self
            .behaviours(assets, "EscapePod")
            .into_iter()
            .next()
            .ok_or_else(|| format!("{}: no EscapePod script", self.name))?;
        let (_, data) = behaviour.data()?;
        let big_endian = self.file.file().big_endian;
        let header = MonoBehaviourHeader::parse(data, big_endian).map_err(|e| e.to_string())?;
        let pod = EscapePod::parse(data, big_endian).map_err(|e| format!("EscapePod: {e}"))?;
        let go = assets
            .resolve(&self.file, header.game_object)?
            .ok_or("EscapePod: no GameObject")?;
        let (r, node) = self
            .locate(&go)
            .ok_or("EscapePod: object not in the scene")?;
        let root = &mut self.roots[r];
        if node == 0 {
            root.nodes[0].local.position = point;
        } else {
            let parent = root.nodes[node].parent.unwrap_or(0);
            let parent_world = root.world(parent);
            let mut local = root.nodes[node].local;
            local.position = inverse_point(&parent_world, point);
            root.set_local(node, local);
        }
        let (sr, sn) = self
            .locate_transform(assets, pod.player_spawn)?
            .ok_or("EscapePod: playerSpawn not in the scene")?;
        Ok(self.roots[sr].world(sn))
    }

    /// `MoveAndRotateWithTransform.LateUpdate`, once: each such object on an
    /// active GameObject takes its `target`'s world position and rotation
    /// (keeping its scale). Twice over, so chains settle. Returns how many
    /// objects follow a target.
    pub fn follow_targets(&mut self, assets: &Assets) -> Result<usize> {
        let big_endian = self.file.file().big_endian;
        let mut pairs = Vec::new();
        for b in self.behaviours(assets, "MoveAndRotateWithTransform") {
            let (_, data) = b.data()?;
            let header = MonoBehaviourHeader::parse(data, big_endian).map_err(|e| e.to_string())?;
            let target = parse_random_start(data, big_endian) // first field: a PPtr
                .map_err(|e| format!("MoveAndRotateWithTransform: {e}"))?;
            let Some(go) = assets.resolve(&self.file, header.game_object)? else {
                continue;
            };
            let (Some(own), Some(target)) =
                (self.locate(&go), self.locate_transform(assets, target)?)
            else {
                continue;
            };
            if header.enabled && self.roots[own.0].nodes[own.1].active {
                pairs.push((own, target));
            }
        }
        for _ in 0..2 {
            for &((r, n), (tr, tn)) in &pairs {
                let target = self.roots[tr].world(tn);
                let root = &mut self.roots[r];
                if n == 0 {
                    root.nodes[0].local.position = target.position;
                    root.nodes[0].local.rotation = target.rotation;
                    continue;
                }
                let parent = root.world(root.nodes[n].parent.unwrap_or(0));
                let [x, y, z, w] = parent.rotation;
                let back = Transform {
                    rotation: [-x, -y, -z, w],
                    ..Transform::default()
                };
                let mut local = root.nodes[n].local;
                local.position = inverse_point(&parent, target.position);
                local.rotation = back
                    .then(&Transform {
                        rotation: target.rotation,
                        ..Transform::default()
                    })
                    .rotation;
                root.set_local(n, local);
            }
        }
        Ok(pairs.len())
    }

    /// The scene's `PrefabSpawn` and `AddressablesPrefabSpawn` components on
    /// active objects that spawn by themselves in a new game.
    pub fn spawns(&self, assets: &Assets) -> Result<Vec<SceneSpawn>> {
        let big_endian = self.file.file().big_endian;
        let mut out = Vec::new();
        for (class, addressable) in [("PrefabSpawn", false), ("AddressablesPrefabSpawn", true)] {
            for b in self.behaviours(assets, class) {
                let (_, data) = b.data()?;
                let header =
                    MonoBehaviourHeader::parse(data, big_endian).map_err(|e| e.to_string())?;
                let spawner = PrefabSpawner::parse(data, big_endian, addressable)
                    .map_err(|e| format!("{class} {}: {e}", b.path_id))?;
                let Some(go) = assets.resolve(&self.file, header.game_object)? else {
                    continue;
                };
                let Some((r, node)) = self.locate(&go) else {
                    continue;
                };
                let n = &self.roots[r].nodes[node];
                if !n.active || !header.enabled || !spawner.spawns_in_new_game() {
                    continue;
                }
                let parent = match self.locate_transform(assets, spawner.attach_to_parent)? {
                    Some((pr, pn)) => self.roots[pr].world(pn),
                    None => self.roots[r].world(node),
                };
                let object = match &spawner.prefab {
                    SpawnPrefab::Object(p) => assets.resolve(&self.file, *p)?,
                    SpawnPrefab::Address(_) => None,
                };
                out.push(SceneSpawn {
                    name: n.name.clone(),
                    spawner_local: n.local,
                    spawner,
                    object,
                    parent,
                });
            }
        }
        Ok(out)
    }
}

impl Assets<'_> {
    /// `RandomStart.validStartPointTexture` (on the `essentials` scene's
    /// spawner prefab) as a start map.
    pub fn start_map(&self) -> Result<StartMap> {
        let scene = self.scene("essentials")?;
        let path = scene.file.bundle.path.clone();
        let names: Vec<String> = scene.file.bundle.file_names().map(String::from).collect();
        for name in &names {
            let file = self.file(&path, name)?;
            for info in file
                .objects()
                .iter()
                .filter(|o| o.class_id == MONO_BEHAVIOUR)
            {
                let object = ObjectRef {
                    file: file.clone(),
                    path_id: info.path_id,
                };
                if self.script_class(&object).as_deref() != Some("RandomStart") {
                    continue;
                }
                let (_, data) = object.data()?;
                let pptr = parse_random_start(data, file.file().big_endian)
                    .map_err(|e| format!("RandomStart: {e}"))?;
                let texture = self
                    .resolve(&file, pptr)?
                    .ok_or("RandomStart: no texture")?;
                let t = self.texture(&texture)?;
                let rgba = t
                    .texture
                    .decode_rgba(&t.data)
                    .map_err(|e| format!("RandomStart texture: {e}"))?;
                let (w, h) = (t.texture.width as usize, t.texture.height as usize);
                // Decoded top row first; GetPixel counts rows from the bottom.
                let mut green = Vec::with_capacity(w * h);
                for y in 0..h {
                    let row = h - 1 - y;
                    green.extend(
                        (0..w).map(|x| rgba.get((row * w + x) * 4 + 1).copied().unwrap_or(0)),
                    );
                }
                // TextureWrapMode: 0 repeat, 1 clamp.
                let repeat = t.texture.wrap[0] == 0;
                return StartMap::new(w, h, green, repeat)
                    .ok_or_else(|| "RandomStart texture: bad size".to_string());
            }
        }
        Err("essentials: no RandomStart".into())
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use sn_unity::{PPtr, SPAWN_ON_NEW_BORN};

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn inverse_point_undoes_a_transform() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let t = Transform {
            position: [3.0, -1.0, 2.0],
            rotation: [0.0, s, 0.0, s],
            scale: [2.0, 1.0, 0.5],
        };
        let p = [0.4, 1.5, -2.0];
        let world = t
            .then(&Transform {
                position: p,
                ..Transform::default()
            })
            .position;
        assert!(close(inverse_point(&t, world), p));
    }

    #[test]
    fn spawned_objects_are_placed_as_the_flags_say() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let spawn = |prefab_local: bool, current: bool, keep_scale: bool| SceneSpawn {
            name: "x".into(),
            spawner: PrefabSpawner {
                spawn_type: SPAWN_ON_NEW_BORN,
                use_prefab_transform_as_local: prefab_local,
                use_current_transform_as_local: current,
                keep_scale,
                attach_to_parent: PPtr::default(),
                deactivate_on_spawn: false,
                prefab: SpawnPrefab::Object(PPtr::default()),
            },
            object: None,
            parent: Transform {
                position: [10.0, 0.0, 0.0],
                ..Transform::default()
            },
            spawner_local: Transform {
                position: [0.0, 1.0, 0.0],
                ..Transform::default()
            },
        };
        let root = Transform {
            position: [5.0, 5.0, 5.0],
            rotation: [0.0, s, 0.0, s],
            scale: [2.0; 3],
        };
        // Default (keep scale): at the parent, no rotation, the prefab's scale.
        let t = spawn(false, false, true).placement(&root);
        assert!(close(t.position, [10.0, 0.0, 0.0]) && t.scale == [2.0; 3]);
        assert_eq!(t.rotation, [0.0, 0.0, 0.0, 1.0]);
        // Zeroed: scale 1.
        assert_eq!(spawn(false, false, false).placement(&root).scale, [1.0; 3]);
        // The prefab's own local placement.
        let t = spawn(true, false, true).placement(&root);
        assert!(close(t.position, [15.0, 5.0, 5.0]) && t.rotation == root.rotation);
        // The spawner's local position.
        assert!(close(
            spawn(false, true, true).placement(&root).position,
            [10.0, 1.0, 0.0]
        ));
    }
}
