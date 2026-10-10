//! The game's MonoBehaviours that decide which scenes are loaded and in
//! what state (fields in declaration order, Unity 2019.4 serialization):
//! `MainGameController`, `LightmappedPrefabs`, `CrashedShipExploder`,
//! the lifepod's `LightingController` and `MarmoLifepodSky`.
//! See `docs/formats/unity.md` § Scenes.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;
use crate::water_surface::AnimationCurve;

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

/// `ShipExteriorCullManager`: every `updateEveryXFrames` frames it tells
/// the exploder whether the camera is inside one of the registered
/// `ShipExteriorCull` volumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShipExteriorCullManager {
    pub crashed_ship_exploder: PPtr,
    pub update_every_x_frames: i32,
}

impl ShipExteriorCullManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<ShipExteriorCullManager> {
        let mut r = fields(data, big_endian)?;
        Ok(ShipExteriorCullManager {
            crashed_ship_exploder: PPtr::read(&mut r)?,
            update_every_x_frames: r.i32()?,
        })
    }
}

/// `ShipExteriorCull.colliders`: the `BoxCollider`s whose boxes (in their
/// Transform's space) are the volume.
pub fn parse_ship_exterior_cull(data: &[u8], big_endian: bool) -> Result<Vec<PPtr>> {
    let mut r = fields(data, big_endian)?;
    pptrs(&mut r)
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

/// `MarmoLifepodSky.anchorSky`: the `mset.Sky` that is the global sky
/// while the player is in the pod, and the sky of the appliers below it.
pub fn parse_marmo_lifepod_sky(data: &[u8], big_endian: bool) -> Result<PPtr> {
    let mut r = fields(data, big_endian)?;
    PPtr::read(&mut r)
}

fn floats(r: &mut Reader) -> Result<Vec<f32>> {
    let n = r.count(4)?;
    (0..n).map(|_| r.f32()).collect()
}

/// `LightingController.MultiStatesSky`: a sky's intensities per state.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiStatesSky {
    pub sky: PPtr,
    pub master: Vec<f32>,
    pub diffuse: Vec<f32>,
    pub specular: Vec<f32>,
}

/// `MultiStatesLight`: a light's intensity per state.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiStatesLight {
    pub light: PPtr,
    pub intensities: Vec<f32>,
}

/// `LightingController`: lighting states (0 Operational, 1 Danger, 2
/// Damaged) of the skies, lights and emissive renderers it controls.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingController {
    /// The state it starts in (`LightingState`).
    pub state: i32,
    /// Seconds of a `LerpToState` without a time.
    pub fade_duration: f32,
    pub skies: Vec<MultiStatesSky>,
    pub lights: Vec<MultiStatesLight>,
    /// `MultiStatesEmissive.intensities`: `_UwePowerLoss = 1 − i` on the
    /// renderers of appliers with `emissiveFromPower`.
    pub emissive: Vec<f32>,
}

impl LightingController {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LightingController> {
        let mut r = fields(data, big_endian)?;
        let state = r.i32()?;
        let fade_duration = r.f32()?;
        let n = r.count(24)?;
        let mut skies = Vec::with_capacity(n);
        for _ in 0..n {
            skies.push(MultiStatesSky {
                sky: PPtr::read(&mut r)?,
                master: floats(&mut r)?,
                diffuse: floats(&mut r)?,
                specular: floats(&mut r)?,
            });
        }
        let n = r.count(16)?;
        let mut lights = Vec::with_capacity(n);
        for _ in 0..n {
            lights.push(MultiStatesLight {
                light: PPtr::read(&mut r)?,
                intensities: floats(&mut r)?,
            });
        }
        let emissive = floats(&mut r)?;
        Ok(LightingController {
            state,
            fade_duration,
            skies,
            lights,
            emissive,
        })
    }
}

/// `EscapePodCinematicControl`: the intro's control of the pod's lights.
#[derive(Clone, Debug, PartialEq)]
pub struct EscapePodCinematicControl {
    pub lighting_control: PPtr,
    pub interior_sky: PPtr,
    /// The interior sky's master intensity over the explosion (intro only).
    pub sky_intensity_curve: AnimationCurve,
    /// Disabled by `StopAll` (the intro ends or is skipped).
    pub lights_animator: PPtr,
    /// Deactivated by `StopAll`.
    pub hatch_light: PPtr,
}

impl EscapePodCinematicControl {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<EscapePodCinematicControl> {
        let mut r = fields(data, big_endian)?;
        PPtr::read(&mut r)?; // escape pod
        PPtr::read(&mut r)?; // intro effects
        let lighting_control = PPtr::read(&mut r)?;
        let interior_sky = PPtr::read(&mut r)?;
        let sky_intensity_curve = AnimationCurve::read(&mut r)?;
        let lights_animator = PPtr::read(&mut r)?;
        let hatch_light = PPtr::read(&mut r)?;
        Ok(EscapePodCinematicControl {
            lighting_control,
            interior_sky,
            sky_intensity_curve,
            lights_animator,
            hatch_light,
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
