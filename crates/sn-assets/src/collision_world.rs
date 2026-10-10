//! What the player can collide with, loaded per batch (M9a, M9b): the
//! terrain's collision surface and the colliders of every object the
//! client draws there (placed objects, slot spawns, what placeholders
//! spawn), plus the objects of a scene (the lifepod and its spawned
//! modules). Shared by `sn-inspect` and the client.
//!
//! Creatures are left out (they move; a later milestone). The caller
//! filters by physics layer and trigger flag.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use sn_install::GameData;
use sn_terrain::{Neighbourhood, TerrainBatch, batch_voxels, collision_triangles};
use sn_unity::Catalog;
use sn_world::{BatchCoord, EntityInfo, SLOTS_COMPONENT, Transform, WorldIndex};

use crate::{
    Assets, ColliderCounts, ColliderMeshes, LootTable, Prefab, PrefabCollider, Result, Scene,
    WorldCollider,
};

/// Prefabs spawned by placeholders inside prefabs spawned by placeholders
/// … deeper than this are taken as a loop (as the client).
const MAX_PLACEHOLDER_DEPTH: usize = 8;

/// `PrefabCollider::node` of colliders that came from a placeholder spawn
/// (they belong to another prefab's hierarchy).
pub const SPAWNED: usize = usize::MAX;

/// A collider placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedCollider {
    pub collider: WorldCollider,
    pub layer: u32,
    pub trigger: bool,
}

/// A scene object's collider and where it came from: `(root, node)` in the
/// scene for its own objects, `None` for what its spawners spawned.
#[derive(Clone, Debug)]
pub struct SceneCollider {
    pub placed: PlacedCollider,
    pub node: Option<(usize, usize)>,
}

/// Counts over everything loaded so far.
#[derive(Clone, Debug, Default)]
pub struct LoaderStats {
    /// Over distinct prefabs (each counted once, with what its placeholders
    /// spawn).
    pub counts: ColliderCounts,
    pub objects_placed: usize,
    pub prefabs_read: usize,
    pub unreadable_prefabs: usize,
    pub creatures_skipped: usize,
    /// Placements whose placeholders spawn (in prefabs read with spawning).
    pub placeholders_spawned: usize,
    /// Placements whose saved objects already hold what their placeholders
    /// spawn, so nothing more is spawned.
    pub placeholders_already_saved: usize,
    pub terrain_ms: f64,
    pub objects_ms: f64,
}

/// The game's spawn-slot tables (`sn_world::fill_slots`).
pub struct SlotTables {
    pub seed: u64,
    pub loot: LootTable,
}

pub struct CollisionLoader<'a> {
    game: &'a GameData,
    assets: Assets<'a>,
    catalog: Catalog,
    index: WorldIndex,
    class_paths: HashMap<String, String>,
    infos: HashMap<String, EntityInfo>,
    slots: Option<SlotTables>,
    batches: HashMap<BatchCoord, Arc<Option<TerrainBatch>>>,
    /// (path, placeholders spawn) → colliders relative to the prefab root.
    prefabs: HashMap<(String, bool), Option<Arc<Vec<PrefabCollider>>>>,
    /// Prefab path → the class ids its placeholders spawn.
    placeholder_ids: HashMap<String, Vec<String>>,
    meshes: ColliderMeshes,
    stats: LoaderStats,
}

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

fn is_creature(path: &str) -> bool {
    path.starts_with("WorldEntities/Creatures/")
}

impl<'a> CollisionLoader<'a> {
    /// `slot_seed`: fill the spawn slots with this world seed (`None`: no
    /// slot spawns, like the client's `--no-slots`).
    pub fn new(game: &'a GameData, slot_seed: Option<u64>) -> Result<CollisionLoader<'a>> {
        let assets = Assets::index(game)?;
        let slots = match slot_seed {
            Some(seed) => Some(SlotTables {
                seed,
                loot: crate::loot_table(&assets)?,
            }),
            None => None,
        };
        Ok(CollisionLoader {
            game,
            catalog: assets.catalog()?,
            index: game.read_index().map_err(|e| e.to_string())?,
            class_paths: game.read_prefab_database().map_err(|e| e.to_string())?,
            infos: crate::entity_infos(&assets)?,
            assets,
            slots,
            batches: HashMap::new(),
            prefabs: HashMap::new(),
            placeholder_ids: HashMap::new(),
            meshes: ColliderMeshes::new(),
            stats: LoaderStats::default(),
        })
    }

    pub fn assets(&self) -> &Assets<'a> {
        &self.assets
    }

    pub fn index(&self) -> &WorldIndex {
        &self.index
    }

    pub fn stats(&self) -> &LoaderStats {
        &self.stats
    }

    /// The batch containing a Unity world position.
    pub fn batch_of(&self, p: [f32; 3]) -> BatchCoord {
        let size = batch_voxels(&self.index);
        let v = sn_world::world_to_voxel(p);
        BatchCoord::new(
            (v[0].floor() as i32).div_euclid(size[0]),
            (v[1].floor() as i32).div_euclid(size[1]),
            (v[2].floor() as i32).div_euclid(size[2]),
        )
    }

    /// Batches touched by the cube of half side `reach` around `p`.
    pub fn batches_around(&self, p: [f32; 3], reach: f32) -> Vec<BatchCoord> {
        let lo = self.batch_of(p.map(|v| v - reach));
        let hi = self.batch_of(p.map(|v| v + reach));
        let mut out = Vec::new();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    out.push(BatchCoord::new(x, y, z));
                }
            }
        }
        out
    }

    fn octrees(&mut self, c: BatchCoord) -> Result<Arc<Option<TerrainBatch>>> {
        if let Some(b) = self.batches.get(&c) {
            return Ok(b.clone());
        }
        let b = Arc::new(
            self.game
                .load_batch(&self.index, c)
                .map_err(|e| e.to_string())?,
        );
        self.batches.insert(c, b.clone());
        Ok(b)
    }

    /// Drops parsed octrees of batches not in `keep` (and not next to one).
    pub fn trim(&mut self, keep: &[BatchCoord]) {
        self.batches.retain(|c, _| {
            keep.iter()
                .any(|k| (k.x - c.x).abs() <= 1 && (k.y - c.y).abs() <= 1 && (k.z - c.z).abs() <= 1)
        });
        self.assets.trim_cache(256 << 20);
    }

    /// The batch's terrain collision triangles (`None`: no terrain file).
    pub fn terrain(&mut self, c: BatchCoord) -> Result<Option<Vec<[[f32; 3]; 3]>>> {
        let start = Instant::now();
        let mut around = Vec::with_capacity(27);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = c.offset(dx, dy, dz);
                    around.push((n, self.octrees(n)?));
                }
            }
        }
        let lookup = |n: BatchCoord| {
            around
                .iter()
                .find(|(coord, _)| *coord == n)
                .and_then(|(_, b)| b.as_ref().as_ref())
        };
        let hood = Neighbourhood::new(c, batch_voxels(&self.index), lookup);
        let t = collision_triangles(&hood);
        self.stats.terrain_ms += start.elapsed().as_secs_f64() * 1000.0;
        Ok(t)
    }

    /// The colliders of a prefab with what its placeholders spawn (when
    /// `spawn`), relative to its root; `None` for creatures and unreadable
    /// prefabs.
    fn prefab(&mut self, path: &str, spawn: bool) -> Option<Arc<Vec<PrefabCollider>>> {
        self.prefab_at_depth(path, spawn, 0)
    }

    fn prefab_at_depth(
        &mut self,
        path: &str,
        spawn: bool,
        depth: usize,
    ) -> Option<Arc<Vec<PrefabCollider>>> {
        if is_creature(path) {
            self.stats.creatures_skipped += 1;
            return None;
        }
        let key = (path.to_string(), spawn);
        if let Some(p) = self.prefabs.get(&key) {
            return p.clone();
        }
        let read = match self.assets.prefab(&self.catalog, path) {
            Ok(prefab) => self.content(&prefab, spawn, depth).ok(),
            Err(_) => None,
        };
        if read.is_none() {
            self.stats.unreadable_prefabs += 1;
        }
        let p = read.map(Arc::new);
        self.prefabs.insert(key, p.clone());
        p
    }

    /// A hierarchy's colliders (relative to its root) and, when `spawn`,
    /// those of what `PrefabPlaceholdersGroup.Start` spawns in a new game:
    /// the same rule as the client (`docs/DESIGN.md` M7h).
    fn content(
        &mut self,
        prefab: &Prefab,
        spawn: bool,
        depth: usize,
    ) -> Result<Vec<PrefabCollider>> {
        let (mut list, counts) = self.assets.prefab_colliders(prefab, &mut self.meshes)?;
        if depth == 0 {
            self.stats.counts.add(&counts);
            self.stats.prefabs_read += 1;
        }
        let ids: Vec<String> = prefab
            .placeholder_groups
            .iter()
            .flat_map(|g| &g.placeholders)
            .filter_map(|&n| prefab.nodes[n].placeholder.clone())
            .collect();
        self.placeholder_ids.insert(prefab.key.clone(), ids);
        if !spawn {
            return Ok(list);
        }
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
                if !node.active_self || !self.infos.contains_key(class_id) {
                    continue;
                }
                let Some(path) = self.class_paths.get(class_id).cloned() else {
                    continue;
                };
                if node.parent.is_some_and(|p| !prefab.nodes[p].active) {
                    continue;
                }
                if depth >= MAX_PLACEHOLDER_DEPTH {
                    continue;
                }
                let Some(child) = self.prefab_at_depth(&path, true, depth + 1) else {
                    continue;
                };
                // The spawned root takes the placeholder's place.
                let at = node.in_prefab;
                list.extend(child.iter().map(|c| PrefabCollider {
                    node: SPAWNED,
                    in_prefab: at.then(&c.in_prefab),
                    ..c.clone()
                }));
                self.stats.placeholders_spawned += 1;
            }
        }
        Ok(list)
    }

    /// Whether the saved objects below a placement already hold what its
    /// prefab's placeholders spawn (the client's `placeholders_saved`).
    fn placeholders_saved(&mut self, path: &str, below: &[&str]) -> bool {
        if below.is_empty() || is_creature(path) {
            return false;
        }
        if !self.placeholder_ids.contains_key(path) {
            self.prefab(path, true);
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
        found > missing
    }

    fn place(&mut self, path: &str, spawn: bool, at: &Transform, out: &mut Vec<PlacedCollider>) {
        let Some(list) = self.prefab(path, spawn) else {
            return;
        };
        self.stats.objects_placed += 1;
        out.extend(list.iter().map(|c| PlacedCollider {
            collider: c.world(at),
            layer: c.layer,
            trigger: c.trigger,
        }));
    }

    /// The colliders of every object of a batch the client draws: its
    /// cells' objects with their slot spawns, and its batch objects.
    pub fn objects(&mut self, c: BatchCoord) -> Result<Vec<PlacedCollider>> {
        let start = Instant::now();
        let mut out = Vec::new();
        if let Some(file) = self.game.read_batch_cells(c).map_err(|e| e.to_string())? {
            for cell in &file.cells {
                let Some(tree) = &cell.objects else { continue };
                let (world, _) = tree.world_transforms();
                let children = saved_children(tree);
                for (object, transform) in tree.objects.iter().zip(world) {
                    for comp in &object.components {
                        if comp.type_name != SLOTS_COMPONENT {
                            continue;
                        }
                        let Some(tables) = &self.slots else { continue };
                        let Ok(slots) = sn_world::parse_slots(&comp.data) else {
                            continue;
                        };
                        let spawns = sn_world::fill_slots(
                            tables.seed,
                            &object.id,
                            &slots,
                            &tables.loot.distribution,
                            &self.infos,
                        );
                        let wanted: Vec<(String, Transform)> = spawns
                            .iter()
                            .filter_map(|s| {
                                let path = self.class_paths.get(s.class_id)?;
                                Some((path.clone(), transform.then(&s.transform)))
                            })
                            .collect();
                        for (path, at) in wanted {
                            self.place(&path, true, &at, &mut out);
                        }
                    }
                    let Some(path) = self.class_paths.get(&object.class_id).cloned() else {
                        continue;
                    };
                    let below = saved_below(tree, &children, &object.id);
                    let spawn = !self.placeholders_saved(&path, &below);
                    if !spawn {
                        self.stats.placeholders_already_saved += 1;
                    }
                    self.place(&path, spawn, &transform, &mut out);
                }
            }
        }
        if let Some(mut tree) = self.game.read_batch_objects(c).map_err(|e| e.to_string())? {
            // Roots sit at the batch's corner (docs/formats/entities.md).
            let corner: Vec<f32> = [c.x, c.y, c.z]
                .map(|v| v as f32 * 160.0)
                .iter()
                .zip(sn_world::VOXEL_WORLD_OFFSET)
                .map(|(v, o)| v - o)
                .collect();
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
                let spawn = !self.placeholders_saved(&path, &below);
                if !spawn {
                    self.stats.placeholders_already_saved += 1;
                }
                self.place(&path, spawn, &transform, &mut out);
            }
        }
        self.stats.objects_ms += start.elapsed().as_secs_f64() * 1000.0;
        Ok(out)
    }

    /// A placed scene's colliders: its own objects (with what their
    /// placeholders spawn) and what its spawners spawn in a new game (the
    /// lifepod's modules), as the client places them. Call after the
    /// scene's objects were placed (`Scene::place_escape_pod`,
    /// `Scene::follow_targets`).
    pub fn scene(&mut self, scene: &Scene) -> Result<Vec<SceneCollider>> {
        let mut out = Vec::new();
        for (r, root) in scene.roots.iter().enumerate() {
            let list = self.content(root, true, 0)?;
            let at = root.nodes[0].local;
            for c in &list {
                out.push(SceneCollider {
                    placed: PlacedCollider {
                        collider: c.world(&at),
                        layer: c.layer,
                        trigger: c.trigger,
                    },
                    node: (c.node != SPAWNED).then_some((r, c.node)),
                });
            }
        }
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
            let Ok(prefab) = prefab else {
                self.stats.unreadable_prefabs += 1;
                continue;
            };
            let at = s.placement(&prefab.nodes[0].local);
            for c in self.content(&prefab, true, 0)? {
                out.push(SceneCollider {
                    placed: PlacedCollider {
                        collider: c.world(&at),
                        layer: c.layer,
                        trigger: c.trigger,
                    },
                    node: None,
                });
            }
        }
        Ok(out)
    }
}

/// Which colliders block the player and which the hand's ray sees, from
/// the game's layer settings (`docs/formats/gameplay.md` § Player
/// movement).
#[derive(Clone, Debug)]
pub struct LayerRules {
    physics: sn_unity::PhysicsManager,
    player_layer: u32,
    useable: Option<u32>,
    not_useable: Option<u32>,
    trigger: Option<u32>,
    only_vehicle: Option<u32>,
}

impl LayerRules {
    pub fn new(settings: &crate::PhysicsSettings, player_layer: u32) -> LayerRules {
        let layer = |n: &str| settings.tags.layer(n);
        LayerRules {
            physics: settings.physics.clone(),
            player_layer,
            useable: layer("Useable"),
            not_useable: layer("NotUseable"),
            trigger: layer("Trigger"),
            only_vehicle: layer("OnlyVehicle"),
        }
    }

    /// A solid collider on a layer the player's layer collides with.
    pub fn blocks_player(&self, c: &PlacedCollider) -> bool {
        !c.trigger && self.physics.collides(self.player_layer, c.layer)
    }

    /// What `Targeting.GetTarget` keeps: not on the Trigger or OnlyVehicle
    /// layers (the ray's mask), triggers only on the Useable layer, nothing
    /// on the NotUseable layer.
    pub fn hand_sees(&self, c: &PlacedCollider) -> bool {
        let on = |l: Option<u32>| l == Some(c.layer);
        if on(self.trigger) || on(self.only_vehicle) || on(self.not_useable) {
            return false;
        }
        !c.trigger || on(self.useable)
    }
}

/// The colliders as `sn-sim` shapes: (triangles, primitives).
fn shapes<'c>(
    colliders: impl Iterator<Item = &'c PlacedCollider>,
) -> (Vec<[[f32; 3]; 3]>, Vec<sn_sim::collide::Shape>) {
    use sn_sim::V3;
    use sn_sim::collide::Shape;
    let (mut tris, mut out) = (Vec::new(), Vec::new());
    for c in colliders {
        let v = V3::from_f32;
        match &c.collider {
            WorldCollider::Box { center, axes, half } => out.push(Shape::Box {
                center: v(*center),
                axes: axes.map(v),
                half: half.map(f64::from),
            }),
            WorldCollider::Sphere { center, radius } => out.push(Shape::Sphere {
                center: v(*center),
                radius: f64::from(*radius),
            }),
            WorldCollider::Capsule { a, b, radius } => out.push(Shape::Capsule {
                a: v(*a),
                b: v(*b),
                radius: f64::from(*radius),
            }),
            WorldCollider::Triangles(t) => tris.extend_from_slice(t),
        }
    }
    (tris, out)
}

/// The colliders as `sn-sim` bodies by group: what blocks the player and
/// the hand (`MOVE | HAND`), only the player (`MOVE`), only the hand
/// (`HAND`). Empty groups are left out.
pub fn bodies(colliders: &[PlacedCollider], rules: &LayerRules) -> Vec<sn_sim::collide::Body> {
    use sn_sim::collide::{Body, HAND, MOVE};
    let mut out = Vec::new();
    for groups in [MOVE | HAND, MOVE, HAND] {
        let picked = colliders.iter().filter(|c| {
            let g = u32::from(rules.blocks_player(c)) * MOVE + u32::from(rules.hand_sees(c)) * HAND;
            g == groups
        });
        let (tris, prims) = shapes(picked);
        if !tris.is_empty() || !prims.is_empty() {
            out.push(Body::new(tris, prims).with_groups(groups));
        }
    }
    out
}

/// The player's movement numbers for `sn-sim` from the game's data: the
/// motors' speeds as `PlayerController.SetMotorMode` sets them (swimming:
/// `swim*`; walking: `walkRun*`), the walking motor's own settings, the
/// ocean level and the physics step.
pub fn player_params(
    data: &crate::PlayerData,
    settings: &crate::PhysicsSettings,
) -> sn_sim::player::PlayerParams {
    let c = &data.controller;
    let g = &data.ground_motor;
    let f = f64::from;
    sn_sim::player::PlayerParams {
        radius: f(c.controller_radius),
        stand_height: f(c.stand_height),
        swim_height: f(c.swim_height),
        camera_offset: f(c.camera_offset),
        swim_forward: f(c.swim_forward_max_speed),
        swim_backward: f(c.swim_backward_max_speed),
        swim_strafe: f(c.swim_strafe_max_speed),
        swim_vertical: f(c.swim_vertical_max_speed),
        water_acceleration: f(c.swim_water_acceleration),
        swim_drag: f(c.default_swim_drag),
        walk_speed: f(c.walk_run_forward_max_speed),
        ground_acceleration: f(g.motor.ground_acceleration),
        air_acceleration: f(g.motor.air_acceleration),
        gravity: f(g.motor.gravity),
        max_fall_speed: f(g.max_fall_speed),
        jump_enabled: g.jump_enabled && g.motor.can_jump,
        jump_base_height: f(g.jump_base_height),
        jump_perp_amount: f(g.jump_perp_amount),
        jump_steep_perp_amount: f(g.jump_steep_perp_amount),
        sliding_enabled: g.sliding_enabled,
        sliding_speed: f(g.sliding_speed),
        sliding_sideways_control: f(g.sliding_sideways_control),
        sliding_speed_control: f(g.sliding_speed_control),
        step_offset: f(g.step_offset),
        slope_limit_degrees: f(g.slope_limit),
        ocean_level: f(data.ocean_level),
        fixed_dt: f(settings.time.fixed_timestep),
    }
}

/// Half the side of the region with collision: the game's finest clipmap
/// level, 7 chunks of 16 m (`clipmaps-high.json`).
pub const COLLISION_REACH: f32 = 56.0;

/// `sn-sim` body ids: a terrain batch is its coordinates; the rest are
/// tagged with one of these bits.
pub const OBJECTS_BODY: u64 = 1 << 62;
pub const SCENE_BODY: u64 = 1 << 61;
/// Hatch triggers: `HATCH_BODY | trigger index << 8 | body index`.
pub const HATCH_BODY: u64 = 1 << 60;

fn coord_bits(c: BatchCoord) -> u64 {
    ((c.x as u64 & 0xffff) << 32) | ((c.y as u64 & 0xffff) << 16) | (c.z as u64 & 0xffff)
}

/// What kind of thing a body id is (for logs).
pub fn body_kind(id: u64) -> &'static str {
    if id & HATCH_BODY != 0 {
        "hatch"
    } else if id & SCENE_BODY != 0 {
        "lifepod"
    } else if id & OBJECTS_BODY != 0 {
        "objects"
    } else {
        "terrain"
    }
}

/// Bodies to add to and remove from an `sn-sim` world after the player
/// moved ([`BatchBodies::update`]).
#[derive(Default)]
pub struct BodyChanges {
    pub add: Vec<(u64, sn_sim::collide::Body)>,
    pub remove: Vec<u64>,
    /// Time to load each new batch (terrain and objects), ms.
    pub load_ms: Vec<f64>,
    pub terrain_triangles: usize,
    pub object_triangles: usize,
    pub object_shapes: usize,
    pub skipped: usize,
}

/// Which batches have bodies in the world, kept to those within
/// [`COLLISION_REACH`] of the player.
#[derive(Default)]
pub struct BatchBodies {
    loaded: HashMap<BatchCoord, Vec<u64>>,
}

impl BatchBodies {
    pub fn new() -> BatchBodies {
        BatchBodies::default()
    }

    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }

    /// Loads the batches within reach of `pos` not loaded yet and lets go
    /// of the others.
    pub fn update(
        &mut self,
        loader: &mut CollisionLoader,
        rules: &LayerRules,
        pos: [f32; 3],
    ) -> Result<BodyChanges> {
        let want = loader.batches_around(pos, COLLISION_REACH);
        let mut out = BodyChanges::default();
        for &c in &want {
            if self.loaded.contains_key(&c) {
                continue;
            }
            let start = Instant::now();
            let mut ids = Vec::new();
            if let Some(tris) = loader.terrain(c)? {
                let body = sn_sim::collide::Body::new(tris, Vec::new());
                out.terrain_triangles += body.triangle_count();
                out.skipped += body.skipped();
                ids.push(coord_bits(c));
                out.add.push((coord_bits(c), body));
            }
            let objects = loader.objects(c)?;
            for (i, body) in bodies(&objects, rules).into_iter().enumerate() {
                out.object_triangles += body.triangle_count();
                out.object_shapes += body.shape_count();
                out.skipped += body.skipped();
                let id = OBJECTS_BODY | ((i as u64) << 48) | coord_bits(c);
                ids.push(id);
                out.add.push((id, body));
            }
            self.loaded.insert(c, ids);
            out.load_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        self.loaded.retain(|c, ids| {
            let keep = want.contains(c);
            if !keep {
                out.remove.append(ids);
            }
            keep
        });
        loader.trim(&want);
        Ok(out)
    }
}

impl CollisionLoader<'_> {
    /// A placed scene's colliders split in two: those of the objects in
    /// `groups` and their children, per group (worked out with each group's
    /// object active, as hatch triggers switch on and off), and the rest.
    pub fn scene_split(
        &mut self,
        scene: &mut Scene,
        groups: &[(usize, usize)],
    ) -> Result<(Vec<PlacedCollider>, Vec<Vec<PlacedCollider>>)> {
        let was: Vec<bool> = groups
            .iter()
            .map(|&(r, n)| scene.roots[r].nodes[n].active_self)
            .collect();
        for &(r, n) in groups {
            scene.roots[r].set_active(n, true);
        }
        let all = self.scene(scene);
        for (&(r, n), &active) in groups.iter().zip(&was) {
            scene.roots[r].set_active(n, active);
        }
        let group_of = |c: &SceneCollider| -> Option<usize> {
            let (r, mut n) = c.node?;
            loop {
                if let Some(g) = groups.iter().position(|&g| g == (r, n)) {
                    return Some(g);
                }
                n = scene.roots[r].nodes[n].parent?;
            }
        };
        let mut per_group: Vec<Vec<PlacedCollider>> = vec![Vec::new(); groups.len()];
        let mut rest = Vec::new();
        for c in all? {
            match group_of(&c) {
                Some(g) => per_group[g].push(c.placed),
                None => rest.push(c.placed),
            }
        }
        Ok((rest, per_group))
    }
}

/// A hand trigger of the lifepod (`CinematicModeTrigger`) with its
/// collision bodies.
pub struct LifepodTrigger {
    pub trigger: crate::CinematicTrigger,
    /// Its object's world position (where to look to use it).
    pub at: [f32; 3],
    /// Active in a new game.
    pub active: bool,
    pub bodies: Vec<sn_sim::collide::Body>,
}

/// Lifepod 5 as in a new game, for collision: its bodies (the pod, its
/// modules), its hand triggers kept apart, and the player's spawn.
pub struct Lifepod {
    /// Where the pod is.
    pub point: [f32; 3],
    /// `EscapePod.playerSpawn`.
    pub spawn: Transform,
    pub bodies: Vec<sn_sim::collide::Body>,
    /// Colliders in `bodies`.
    pub colliders: usize,
    pub triggers: Vec<LifepodTrigger>,
    /// (normal, first use) indices into `triggers` of the bottom and top
    /// hatches (`EscapePodFirstUseCinematicsController`).
    pub first_use: Vec<(usize, usize)>,
}

impl CollisionLoader<'_> {
    /// The lifepod placed at `point` with no hatch used yet.
    pub fn lifepod(&mut self, rules: &LayerRules, point: [f32; 3]) -> Result<Lifepod> {
        let assets = &self.assets;
        let mut scene = assets.scene("escapepod")?;
        scene.spawn_lightmapped_prefab();
        let spawn = scene.place_escape_pod(assets, point)?;
        scene.follow_targets(assets)?;
        let pairs = scene.init_lifepod_hatches(assets, false, false)?;
        let found: Vec<crate::CinematicTrigger> = scene
            .cinematic_triggers(assets)?
            .into_iter()
            .filter(|t| t.hand)
            .collect();
        let nodes: Vec<(usize, usize)> = found.iter().map(|t| t.node).collect();
        let (rest, per_trigger) = self.scene_split(&mut scene, &nodes)?;
        let triggers: Vec<LifepodTrigger> = found
            .into_iter()
            .zip(per_trigger)
            .map(|(t, colliders)| {
                let (r, n) = t.node;
                LifepodTrigger {
                    at: scene.roots[r].world(n).position,
                    active: scene.roots[r].nodes[n].active,
                    bodies: bodies(&colliders, rules),
                    trigger: t,
                }
            })
            .collect();
        let index_of = |node: (usize, usize)| triggers.iter().position(|t| t.trigger.node == node);
        let first_use = pairs
            .into_iter()
            .flatten()
            .filter_map(|(normal, first)| Some((index_of(normal)?, index_of(first)?)))
            .collect();
        Ok(Lifepod {
            point,
            spawn,
            bodies: bodies(&rest, rules),
            colliders: rest.len(),
            triggers,
            first_use,
        })
    }
}
