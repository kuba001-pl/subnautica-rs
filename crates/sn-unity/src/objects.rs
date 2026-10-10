//! Readers for built-in object classes, Unity 2019.4 layouts (the files have
//! no type trees). Each layout is checked against UnityPy; see
//! `docs/formats/unity.md`.

use crate::Result;
use crate::reader::Reader;

/// A reference to an object: `file_id` 0 is the same serialized file, `n > 0`
/// is `externals[n - 1]`; `path_id` 0 is a null reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PPtr {
    pub file_id: i32,
    pub path_id: i64,
}

impl PPtr {
    pub fn is_null(&self) -> bool {
        self.path_id == 0
    }

    pub(crate) fn read(r: &mut Reader) -> Result<PPtr> {
        Ok(PPtr {
            file_id: r.i32()?,
            path_id: r.i64()?,
        })
    }

    /// A component's GameObject: every `Component` (Transform, renderers,
    /// scripts, …) starts with its `m_GameObject` reference.
    pub fn component_game_object(data: &[u8], big_endian: bool) -> Result<PPtr> {
        PPtr::read(&mut Reader::new(data, big_endian))
    }
}

impl Reader<'_> {
    pub(crate) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
}

/// A texture slot of a material.
#[derive(Clone, Debug, PartialEq)]
pub struct TexEnv {
    pub name: String,
    pub texture: PPtr,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

/// `Material` (class 21).
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub shader: PPtr,
    pub keywords: String,
    pub custom_render_queue: i32,
    pub tags: Vec<(String, String)>,
    pub textures: Vec<TexEnv>,
    pub floats: Vec<(String, f32)>,
    pub colors: Vec<(String, [f32; 4])>,
}

impl Material {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Material> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let shader = PPtr::read(&mut r)?;
        let keywords = r.aligned_string()?;
        let _lightmap_flags = r.u32()?;
        let _enable_instancing_variants = r.u8()?;
        let _double_sided_gi = r.u8()?;
        r.align(4)?;
        let custom_render_queue = r.i32()?;
        let tag_count = r.count(8)?;
        let mut tags = Vec::with_capacity(tag_count);
        for _ in 0..tag_count {
            tags.push((r.aligned_string()?, r.aligned_string()?));
        }
        let disabled_passes = r.count(4)?;
        for _ in 0..disabled_passes {
            r.aligned_string()?;
        }
        let tex_count = r.count(32)?;
        let mut textures = Vec::with_capacity(tex_count);
        for _ in 0..tex_count {
            textures.push(TexEnv {
                name: r.aligned_string()?,
                texture: PPtr::read(&mut r)?,
                scale: [r.f32()?, r.f32()?],
                offset: [r.f32()?, r.f32()?],
            });
        }
        let float_count = r.count(8)?;
        let mut floats = Vec::with_capacity(float_count);
        for _ in 0..float_count {
            floats.push((r.aligned_string()?, r.f32()?));
        }
        let color_count = r.count(20)?;
        let mut colors = Vec::with_capacity(color_count);
        for _ in 0..color_count {
            colors.push((
                r.aligned_string()?,
                [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            ));
        }
        Ok(Material {
            name,
            shader,
            keywords,
            custom_render_queue,
            tags,
            textures,
            floats,
            colors,
        })
    }

    pub fn texture(&self, name: &str) -> Option<&TexEnv> {
        self.textures
            .iter()
            .find(|t| t.name == name && !t.texture.is_null())
    }

    pub fn float(&self, name: &str) -> Option<f32> {
        self.floats.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    pub fn color(&self, name: &str) -> Option<[f32; 4]> {
        self.colors.iter().find(|(n, _)| n == name).map(|(_, c)| *c)
    }
}

/// `MonoScript` (class 115): identifies the C# class of a MonoBehaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonoScript {
    pub name: String,
    pub class_name: String,
    pub namespace: String,
    pub assembly: String,
}

impl MonoScript {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MonoScript> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let _execution_order = r.i32()?;
        let _properties_hash: [u8; 16] = r.array()?;
        Ok(MonoScript {
            name,
            class_name: r.aligned_string()?,
            namespace: r.aligned_string()?,
            assembly: r.aligned_string()?,
        })
    }
}

/// The fields every `MonoBehaviour` (class 114) starts with. The script's
/// own fields follow at `fields_offset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonoBehaviourHeader {
    pub game_object: PPtr,
    pub enabled: bool,
    pub script: PPtr,
    pub name: String,
    pub fields_offset: usize,
}

impl MonoBehaviourHeader {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MonoBehaviourHeader> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let enabled = r.u8()? != 0;
        r.align(4)?;
        let script = PPtr::read(&mut r)?;
        let name = r.aligned_string()?;
        Ok(MonoBehaviourHeader {
            game_object,
            enabled,
            script,
            name,
            fields_offset: r.pos(),
        })
    }
}
