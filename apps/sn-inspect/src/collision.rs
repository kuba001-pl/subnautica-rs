//! The collision world of the headless scripts (`swim`, `walk`): the
//! shared loader (`sn_assets::CollisionLoader`, `BatchBodies`) keeping the
//! batches within the game's collision range of the player in an
//! `sn-sim` world, and the lifepod.

use sn_assets::{BatchBodies, CollisionLoader, LayerRules, SCENE_BODY, Scene, bodies};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::collide::{Body, Capsule, World};

pub use sn_assets::{HATCH_BODY as HATCH, body_kind as kind};

use crate::Result;

pub struct Streamed<'a> {
    pub loader: CollisionLoader<'a>,
    pub rules: LayerRules,
    pub world: World,
    batches: BatchBodies,
    pub load_ms: Vec<f64>,
    pub terrain_triangles: usize,
    pub object_triangles: usize,
    pub object_shapes: usize,
    pub skipped: usize,
}

impl<'a> Streamed<'a> {
    /// `slot_seed`: the world seed the spawn slots are filled with.
    pub fn new(game: &'a GameData, slot_seed: u64) -> Result<Streamed<'a>> {
        let loader = CollisionLoader::new(game, Some(slot_seed))?;
        let player = sn_assets::player_data(loader.assets())?;
        let settings = sn_assets::physics_settings(loader.assets())?;
        let rules = LayerRules::new(&settings, player.player_layer);
        Ok(Streamed {
            loader,
            rules,
            world: World::new(),
            batches: BatchBodies::new(),
            load_ms: Vec::new(),
            terrain_triangles: 0,
            object_triangles: 0,
            object_shapes: 0,
            skipped: 0,
        })
    }

    /// Loads the batches within reach of `pos` and drops the others.
    pub fn stream(&mut self, pos: V3) -> Result<()> {
        let c = self
            .batches
            .update(&mut self.loader, &self.rules, pos.to_f32())?;
        for id in c.remove {
            self.world.remove(id);
        }
        for (id, body) in c.add {
            self.world.insert(id, body);
        }
        self.load_ms.extend(c.load_ms);
        self.terrain_triangles += c.terrain_triangles;
        self.object_triangles += c.object_triangles;
        self.object_shapes += c.object_shapes;
        self.skipped += c.skipped;
        Ok(())
    }

    /// Adds a placed scene's colliders (the lifepod and its modules),
    /// except those of the objects in `groups` and their children: those
    /// are returned as bodies per group for the caller to insert and remove
    /// (the hatch triggers). Returns how many colliders went in.
    pub fn add_scene(
        &mut self,
        scene: &mut Scene,
        groups: &[(usize, usize)],
    ) -> Result<(usize, Vec<Vec<Body>>)> {
        let (rest, per_group) = self.loader.scene_split(scene, groups)?;
        for (i, body) in bodies(&rest, &self.rules).into_iter().enumerate() {
            self.world.insert(SCENE_BODY | i as u64, body);
        }
        let per_group = per_group.iter().map(|g| bodies(g, &self.rules)).collect();
        Ok((rest.len(), per_group))
    }

    /// Pushes the capsule out of anything it overlaps; the gap found first.
    pub fn settle(&self, capsule: &Capsule, pos: V3) -> (V3, Option<f64>) {
        self.world.push_out(capsule, pos)
    }
}
