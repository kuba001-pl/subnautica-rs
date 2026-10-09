//! The game's MonoBehaviours that decide which scenes are loaded and in
//! what state (fields in declaration order, Unity 2019.4 serialization):
//! `MainGameController`, `LightmappedPrefabs`, `CrashedShipExploder`.
//! See `docs/formats/unity.md` § Scenes.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

fn fields<'a>(data: &'a [u8], big_endian: bool) -> Result<Reader<'a>> {
    let header = MonoBehaviourHeader::parse(data, big_endian)?;
    let mut r = Reader::new(data, big_endian);
    r.seek(header.fields_offset)?;
    Ok(r)
}

fn pptrs(r: &mut Reader) -> Result<Vec<PPtr>> {
    let n = r.count(12)?;
    (0..n).map(|_| PPtr::read(r)).collect()
}

/// `MainGameController.additionalScenes`: scenes loaded additively after
/// the main scene.
pub fn parse_additional_scenes(data: &[u8], big_endian: bool) -> Result<Vec<String>> {
    let mut r = fields(data, big_endian)?;
    let n = r.count(4)?;
    (0..n).map(|_| r.aligned_string()).collect()
}

/// One of `LightmappedPrefabs.autoloadScenes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoLoadScene {
    pub scene_name: String,
    /// The scene's `__LIGHTMAPPED_PREFAB__` object is put at the origin and
    /// activated once loaded; otherwise it stays inactive as a template.
    pub spawn_on_start: bool,
}

/// `LightmappedPrefabs.autoloadScenes`.
pub fn parse_autoload_scenes(data: &[u8], big_endian: bool) -> Result<Vec<AutoLoadScene>> {
    let mut r = fields(data, big_endian)?;
    let n = r.count(8)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let scene_name = r.aligned_string()?;
        let spawn_on_start = r.u8()? != 0;
        r.align(4)?;
        out.push(AutoLoadScene {
            scene_name,
            spawn_on_start,
        });
    }
    Ok(out)
}

/// `CrashedShipExploder`: which GameObjects of the Aurora are swapped when
/// the ship explodes (`SwapModels`).
#[derive(Clone, Debug, PartialEq)]
pub struct CrashedShipExploder {
    pub crashed_ship_prefab: PPtr,
    /// Active until the explosion.
    pub disable_on_explosion: Vec<PPtr>,
    /// Active after it.
    pub enable_on_explosion: Vec<PPtr>,
    pub exploded_exterior: PPtr,
}

impl CrashedShipExploder {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<CrashedShipExploder> {
        let mut r = fields(data, big_endian)?;
        Ok(CrashedShipExploder {
            crashed_ship_prefab: PPtr::read(&mut r)?,
            disable_on_explosion: pptrs(&mut r)?,
            enable_on_explosion: pptrs(&mut r)?,
            exploded_exterior: PPtr::read(&mut r)?,
        })
    }
}

/// `RandomStart.validStartPointTexture`: the map of valid lifepod starts.
pub fn parse_random_start(data: &[u8], big_endian: bool) -> Result<PPtr> {
    let mut r = fields(data, big_endian)?;
    PPtr::read(&mut r)
}

/// `EscapePod`: the fields we use (its first two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EscapePod {
    pub bottom_hatch_entrance: PPtr,
    /// Transform where the player is put (`RespawnPlayer`).
    pub player_spawn: PPtr,
}

impl EscapePod {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<EscapePod> {
        let mut r = fields(data, big_endian)?;
        Ok(EscapePod {
            bottom_hatch_entrance: PPtr::read(&mut r)?,
            player_spawn: PPtr::read(&mut r)?,
        })
    }
}

/// `SpawnType`.
pub const SPAWN_ON_START: i32 = 0;
pub const SPAWN_INTERMITTENT: i32 = 1;
pub const SPAWN_ON_AWAKE: i32 = 2;
pub const SPAWN_ON_NEW_BORN: i32 = 3;
pub const SPAWN_MANUAL: i32 = 4;

/// What a spawner instantiates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnPrefab {
    /// `PrefabSpawn.prefab`: a GameObject.
    Object(PPtr),
    /// `AddressablesPrefabSpawn.prefab`: an asset GUID (a catalog key).
    Address(String),
}

/// `PrefabSpawn` or `AddressablesPrefabSpawn` (`PrefabSpawnBase` fields,
/// then the prefab).
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabSpawner {
    pub spawn_type: i32,
    pub use_prefab_transform_as_local: bool,
    pub use_current_transform_as_local: bool,
    pub keep_scale: bool,
    /// Parent of the spawned object; null: the spawner's own Transform.
    pub attach_to_parent: PPtr,
    pub deactivate_on_spawn: bool,
    pub prefab: SpawnPrefab,
}

impl PrefabSpawner {
    /// `addressable`: an `AddressablesPrefabSpawn` (else a `PrefabSpawn`).
    pub fn parse(data: &[u8], big_endian: bool, addressable: bool) -> Result<PrefabSpawner> {
        let mut r = fields(data, big_endian)?;
        let bool4 = |r: &mut Reader| -> Result<bool> {
            let b = r.u8()? != 0;
            r.align(4)?;
            Ok(b)
        };
        let spawn_type = r.i32()?;
        r.f32()?; // intermittent spawn time
        bool4(&mut r)?; // inherit layer
        let use_prefab_transform_as_local = bool4(&mut r)?;
        let use_current_transform_as_local = bool4(&mut r)?;
        let keep_scale = bool4(&mut r)?;
        let attach_to_parent = PPtr::read(&mut r)?;
        r.f32()?; // spawn at health percent
        bool4(&mut r)?; // use spawn at health
        PPtr::read(&mut r)?; // spawned object
        r.aligned_string()?; // message sent on spawn
        let deactivate_on_spawn = bool4(&mut r)?;
        let prefab = if addressable {
            SpawnPrefab::Address(r.aligned_string()?)
        } else {
            SpawnPrefab::Object(PPtr::read(&mut r)?)
        };
        Ok(PrefabSpawner {
            spawn_type,
            use_prefab_transform_as_local,
            use_current_transform_as_local,
            keep_scale,
            attach_to_parent,
            deactivate_on_spawn,
            prefab,
        })
    }

    /// Whether it spawns by itself when a new game starts (not `Manual`,
    /// which the game's code triggers, nor `Intermittent`).
    pub fn spawns_in_new_game(&self) -> bool {
        matches!(
            self.spawn_type,
            SPAWN_ON_START | SPAWN_ON_AWAKE | SPAWN_ON_NEW_BORN
        )
    }
}
