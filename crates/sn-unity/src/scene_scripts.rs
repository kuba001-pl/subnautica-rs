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
