//! Objects that make up a prefab's hierarchy, Unity 2019.4 layouts:
//! `GameObject` (1), `Transform` (4), `MeshRenderer` (23), `LODGroup` (205),
//! and `AssetBundle` (142), whose container maps asset paths to objects.

use crate::Result;
use crate::objects::PPtr;
use crate::reader::Reader;

fn pptr_vector(r: &mut Reader) -> Result<Vec<PPtr>> {
    let n = r.count(12)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(PPtr::read(r)?);
    }
    r.align(4)?;
    Ok(out)
}

/// `GameObject` (class 1).
#[derive(Clone, Debug, PartialEq)]
pub struct GameObject {
    pub components: Vec<PPtr>,
    pub layer: u32,
    pub name: String,
    pub tag: u16,
    pub active: bool,
}

impl GameObject {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<GameObject> {
        let mut r = Reader::new(data, big_endian);
        let components = pptr_vector(&mut r)?;
        Ok(GameObject {
            components,
            layer: r.u32()?,
            name: r.aligned_string()?,
            tag: r.u16()?,
            active: r.u8()? != 0,
        })
    }
}

/// `Transform` (class 4): local placement relative to `father`.
#[derive(Clone, Debug, PartialEq)]
pub struct TransformNode {
    pub game_object: PPtr,
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub children: Vec<PPtr>,
    pub father: PPtr,
}

impl TransformNode {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<TransformNode> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let rotation = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let position = [r.f32()?, r.f32()?, r.f32()?];
        let scale = [r.f32()?, r.f32()?, r.f32()?];
        let children = pptr_vector(&mut r)?;
        let father = PPtr::read(&mut r)?;
        Ok(TransformNode {
            game_object,
            rotation,
            position,
            scale,
            children,
            father,
        })
    }
}

/// `MeshRenderer` (class 23); the fields we use.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshRenderer {
    pub game_object: PPtr,
    pub enabled: bool,
    pub cast_shadows: u8,
    pub materials: Vec<PPtr>,
}

impl MeshRenderer {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MeshRenderer> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let enabled = r.u8()? != 0;
        let cast_shadows = r.u8()?;
        // receive shadows, dynamic occludee, motion vectors, light probe and
        // reflection probe usage, ray tracing mode
        r.bytes(6)?;
        r.align(4)?;
        r.u32()?; // rendering layer mask
        r.i32()?; // renderer priority
        r.u16()?;
        r.u16()?; // lightmap indices
        r.bytes(32)?; // lightmap tiling offsets
        let materials = pptr_vector(&mut r)?;
        Ok(MeshRenderer {
            game_object,
            enabled,
            cast_shadows,
            materials,
        })
    }
}

/// One level of a `LODGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lod {
    /// Screen height fraction below which the next level takes over.
    pub screen_relative_height: f32,
    pub renderers: Vec<PPtr>,
}

/// `LODGroup` (class 205).
#[derive(Clone, Debug, PartialEq)]
pub struct LodGroup {
    pub game_object: PPtr,
    pub size: f32,
    pub lods: Vec<Lod>,
    pub enabled: bool,
}

impl LodGroup {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LodGroup> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        r.bytes(12)?; // local reference point
        let size = r.f32()?;
        r.i32()?; // fade mode
        r.u8()?;
        r.u8()?; // animate cross fading, last LOD is billboard
        r.align(4)?;
        let n = r.count(12)?;
        let mut lods = Vec::with_capacity(n);
        for _ in 0..n {
            let screen_relative_height = r.f32()?;
            r.f32()?; // fade transition width
            let renderers = pptr_vector(&mut r)?;
            lods.push(Lod {
                screen_relative_height,
                renderers,
            });
        }
        r.align(4)?;
        let enabled = r.u8()? != 0;
        Ok(LodGroup {
            game_object,
            size,
            lods,
            enabled,
        })
    }
}

/// `AssetBundle` (class 142): which objects each asset path names.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetBundleManifest {
    pub name: String,
    /// Asset path (as given at build time, e.g. `Assets/…/X.prefab`) → its
    /// main object. A path can appear more than once (sub-assets).
    pub container: Vec<(String, PPtr)>,
    /// Internal file names (`cab-…`) of bundles this one needs.
    pub dependencies: Vec<String>,
}

impl AssetBundleManifest {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<AssetBundleManifest> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        pptr_vector(&mut r)?; // preload table
        let n = r.count(24)?;
        let mut container = Vec::with_capacity(n);
        for _ in 0..n {
            let path = r.aligned_string()?;
            r.i32()?;
            r.i32()?; // preload index, size
            container.push((path, PPtr::read(&mut r)?));
        }
        r.i32()?;
        r.i32()?;
        PPtr::read(&mut r)?; // main asset
        r.u32()?; // runtime compatibility
        r.aligned_string()?; // asset bundle name
        let n = r.count(4)?;
        let mut dependencies = Vec::with_capacity(n);
        for _ in 0..n {
            dependencies.push(r.aligned_string()?);
        }
        r.align(4)?;
        Ok(AssetBundleManifest {
            name,
            container,
            dependencies,
        })
    }
}
