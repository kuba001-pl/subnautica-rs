//! The game's `Voxeland` component (a MonoBehaviour in the main scene): the
//! terrain engine's settings, including the block-type table that maps the
//! octrees' type ids to terrain materials.
//!
//! The layout was derived from the field declarations in the game's own
//! assemblies (read in place with a dev-time tool, see
//! `docs/formats/terrain-materials.md`); this is our own reader of it.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// How a block type scatters grass over its faces (`VoxelandTypeBase`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrassSettings {
    /// Share of candidate spots that get grass (or the Perlin threshold).
    pub density: f32,
    /// The grass mesh is modelled Z-up (turned −90° about x).
    pub z_up: bool,
    /// Random offset within a spot, as a share of its size (at most 0.5).
    pub jitter: f32,
    pub min_scale: f32,
    pub max_scale: f32,
    /// Allowed slope of the face, in degrees from up.
    pub min_tilt: i32,
    pub max_tilt: i32,
    pub random_spin: bool,
    /// Placement by Perlin noise over world x/z instead of random draws.
    pub perlin: bool,
    pub perlin_period: f32,
}

/// One entry of `Voxeland.types`, indexed by the octree node type id.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelandBlockType {
    pub grass: GrassSettings,
    pub layer: i32,
    /// False for type 0 (empty space) and unused slots.
    pub filled: bool,
    pub material: PPtr,
    pub deco_override: PPtr,
    pub has_grass_above: bool,
    pub grass_mesh: PPtr,
    pub grass_material: PPtr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Voxeland {
    pub header: MonoBehaviourHeader,
    pub data: PPtr,
    pub opaque_material: PPtr,
    pub palette_resource_dir: String,
    pub types: Vec<VoxelandBlockType>,
    pub chunk_size: i32,
    pub surface_density_value: f32,
}

impl Reader<'_> {
    pub(crate) fn bool_aligned(&mut self) -> Result<bool> {
        let v = self.u8()? != 0;
        self.align(4)?;
        Ok(v)
    }
}

impl VoxelandBlockType {
    fn read(r: &mut Reader) -> Result<VoxelandBlockType> {
        let grass = GrassSettings {
            density: r.f32()?,
            z_up: r.bool_aligned()?,
            jitter: r.f32()?,
            min_scale: r.f32()?,
            max_scale: r.f32()?,
            min_tilt: r.i32()?,
            max_tilt: r.i32()?,
            random_spin: r.bool_aligned()?,
            perlin: r.bool_aligned()?,
            perlin_period: r.f32()?,
        };
        let layer = r.i32()?;
        let filled = r.bool_aligned()?;
        let material = PPtr::read(r)?;
        let deco_override = PPtr::read(r)?;
        let has_grass_above = r.bool_aligned()?;
        let grass_mesh = PPtr::read(r)?;
        let grass_material = PPtr::read(r)?;
        Ok(VoxelandBlockType {
            grass,
            layer,
            filled,
            material,
            deco_override,
            has_grass_above,
            grass_mesh,
            grass_material,
        })
    }
}

/// The game's `VoxelandBlockTypePrefab` component: one terrain block type,
/// kept as a prefab in the `BlockPrefabs` resources folder. `global_id` is
/// the octree type id it defines.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelandBlockTypePrefab {
    pub header: MonoBehaviourHeader,
    pub block_type: VoxelandBlockType,
    pub global_id: u8,
}

impl VoxelandBlockTypePrefab {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<VoxelandBlockTypePrefab> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let block_type = VoxelandBlockType::read(&mut r)?;
        let global_id = r.u8()?;
        Ok(VoxelandBlockTypePrefab {
            header,
            block_type,
            global_id,
        })
    }
}

impl Voxeland {
    /// Parses the object bytes of the `Voxeland` MonoBehaviour.
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Voxeland> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let data_ref = PPtr::read(&mut r)?;
        let _local_ao = r.bool_aligned()?;
        let _cast_shadows = r.bool_aligned()?;
        let _scale_tool_to_edit = r.bool_aligned()?;
        let opaque_material = PPtr::read(&mut r)?;
        let palette_resource_dir = r.aligned_string()?;
        let count = r.count(100)?;
        let mut types = Vec::with_capacity(count);
        for _ in 0..count {
            types.push(VoxelandBlockType::read(&mut r)?);
        }
        let _selected = r.i32()?;
        let chunk_size = r.i32()?;
        let _new_chunk_size = r.i32()?;
        let _num_chunks_built = r.i32()?;
        for _ in 0..5 {
            r.bool_aligned()?; // debug flags
        }
        let surface_density_value = r.f32()?;
        Ok(Voxeland {
            header,
            data: data_ref,
            opaque_material,
            palette_resource_dir,
            types,
            chunk_size,
            surface_density_value,
        })
    }
}
