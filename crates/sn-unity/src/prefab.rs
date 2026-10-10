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
        MeshRenderer::read(&mut r)
    }

    /// The fields every `Renderer` starts with, up to its materials.
    fn read(r: &mut Reader) -> Result<MeshRenderer> {
        let game_object = PPtr::read(r)?;
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
        let materials = pptr_vector(r)?;
        Ok(MeshRenderer {
            game_object,
            enabled,
            cast_shadows,
            materials,
        })
    }
}

/// `SkinnedMeshRenderer` (class 137): a renderer whose mesh is bent by
/// bones (Transforms) each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedMeshRenderer {
    pub renderer: MeshRenderer,
    pub mesh: PPtr,
    /// Transforms, one per bind pose of the mesh.
    pub bones: Vec<PPtr>,
    pub blend_shape_weights: Vec<f32>,
    pub root_bone: PPtr,
    /// Bounds (centre, half-size) relative to the root bone.
    pub aabb: ([f32; 3], [f32; 3]),
}

impl SkinnedMeshRenderer {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<SkinnedMeshRenderer> {
        let mut r = Reader::new(data, big_endian);
        let renderer = MeshRenderer::read(&mut r)?;
        r.u16()?;
        r.u16()?; // static batch info: first sub-mesh, count
        PPtr::read(&mut r)?; // static batch root
        PPtr::read(&mut r)?; // probe anchor
        PPtr::read(&mut r)?; // light probe volume override
        r.i32()?; // sorting layer id
        r.i16()?;
        r.i16()?; // sorting layer, sorting order
        r.align(4)?;
        r.i32()?; // quality
        r.u8()?;
        r.u8()?; // update when offscreen, skinned motion vectors
        r.align(4)?;
        let mesh = PPtr::read(&mut r)?;
        let bones = pptr_vector(&mut r)?;
        let n = r.count(4)?;
        let mut blend_shape_weights = Vec::with_capacity(n);
        for _ in 0..n {
            blend_shape_weights.push(r.f32()?);
        }
        r.align(4)?;
        let root_bone = PPtr::read(&mut r)?;
        let v = |r: &mut Reader| -> Result<[f32; 3]> { Ok([r.f32()?, r.f32()?, r.f32()?]) };
        let aabb = (v(&mut r)?, v(&mut r)?);
        r.u8()?; // dirty AABB
        Ok(SkinnedMeshRenderer {
            renderer,
            mesh,
            bones,
            blend_shape_weights,
            root_bone,
            aabb,
        })
    }
}

/// One level of a `LODGroup`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lod {
    /// Screen height fraction below which the next level takes over.
    pub screen_relative_height: f32,
    pub fade_transition_width: f32,
    pub renderers: Vec<PPtr>,
}

/// `LODGroup` (class 205).
#[derive(Clone, Debug, PartialEq)]
pub struct LodGroup {
    pub game_object: PPtr,
    /// The point distances are measured to, in the group's Transform space.
    pub local_reference_point: [f32; 3],
    /// Size in the group's Transform space (times its largest scale in
    /// the world).
    pub size: f32,
    /// `LODFadeMode`: 0 none, 1 cross-fade, 2 speed tree.
    pub fade_mode: i32,
    pub animate_cross_fading: bool,
    pub last_lod_is_billboard: bool,
    pub lods: Vec<Lod>,
    pub enabled: bool,
}

impl LodGroup {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LodGroup> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let local_reference_point = [r.f32()?, r.f32()?, r.f32()?];
        let size = r.f32()?;
        let fade_mode = r.i32()?;
        let animate_cross_fading = r.u8()? != 0;
        let last_lod_is_billboard = r.u8()? != 0;
        r.align(4)?;
        let n = r.count(12)?;
        let mut lods = Vec::with_capacity(n);
        for _ in 0..n {
            let screen_relative_height = r.f32()?;
            let fade_transition_width = r.f32()?;
            let renderers = pptr_vector(&mut r)?;
            lods.push(Lod {
                screen_relative_height,
                fade_transition_width,
                renderers,
            });
        }
        r.align(4)?;
        let enabled = r.u8()? != 0;
        Ok(LodGroup {
            game_object,
            local_reference_point,
            size,
            fade_mode,
            animate_cross_fading,
            last_lod_is_billboard,
            lods,
            enabled,
        })
    }
}

/// `ResourceManager` (class 147, in `globalgamemanagers`): the paths
/// `Resources.Load` accepts (lower case, e.g. `data/watercaustics00`) and
/// their objects. A path can appear more than once (sub-assets).
pub fn parse_resource_container(data: &[u8], big_endian: bool) -> Result<Vec<(String, PPtr)>> {
    let mut r = Reader::new(data, big_endian);
    let n = r.count(16)?;
    let mut container = Vec::with_capacity(n);
    for _ in 0..n {
        let path = r.aligned_string()?;
        container.push((path, PPtr::read(&mut r)?));
    }
    Ok(container)
}

/// `TextAsset` (class 49): its name and bytes.
pub fn parse_text_asset(data: &[u8], big_endian: bool) -> Result<(String, Vec<u8>)> {
    let mut r = Reader::new(data, big_endian);
    let name = r.aligned_string()?;
    let n = r.count(1)?;
    Ok((name, r.bytes(n)?.to_vec()))
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
