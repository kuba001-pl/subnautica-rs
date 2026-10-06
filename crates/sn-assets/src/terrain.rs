//! The terrain's materials: octree type id → block type → Material →
//! textures. See `docs/formats/terrain-materials.md`.
//!
//! Block types come from two places: the `types` table of the main scene's
//! `Voxeland` component, and `VoxelandBlockTypePrefab` components on prefabs
//! in the player's resources (`BlockPrefabs/…`), each declaring its type id
//! (`globalId`). Prefabs fill in or replace table entries.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use sn_unity::{
    Material, MonoBehaviourHeader, MonoScript, Texture2D, Voxeland, VoxelandBlockType,
    VoxelandBlockTypePrefab,
};

use crate::{Assets, FileRef, ObjectRef, Result};

const MONO_BEHAVIOUR: i32 = 114;
const MATERIAL: i32 = 21;
const TEXTURE_2D: i32 = 28;
const MONO_SCRIPT: i32 = 115;

/// A texture with its pixel data (all stored mip levels, in the stored format).
pub struct TerrainTexture {
    pub texture: Texture2D,
    pub data: Vec<u8>,
}

/// Where a block type's definition came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSource {
    Scene,
    Prefab,
}

/// One set of surface textures.
#[derive(Clone)]
pub struct SurfaceLayer {
    pub albedo: Option<Arc<TerrainTexture>>,
    pub normal: Option<Arc<TerrainTexture>>,
    /// Texture repeats per metre.
    pub scale: f32,
    pub tint: [f32; 4],
}

/// What a terrain block type looks like. Terrain materials come in two
/// kinds: plain (`_MainTex`, `_BumpMap`, `_TriplanarScale`, `_Color`), where
/// `cap` and `side` are the same, and cap/side blends (`_CapTexture`,
/// `_CapBumpMap`, `_CapScale`, `_CapColor` for upward-facing surfaces;
/// `_SideTexture`, `_SideBumpMap`, `_SideScale`, `_Color` for slopes).
pub struct TerrainMaterial {
    pub type_id: usize,
    pub source: BlockSource,
    pub name: String,
    pub shader_keywords: String,
    pub cap: SurfaceLayer,
    pub side: SurfaceLayer,
    pub layer: i32,
}

pub struct TerrainMaterials {
    /// Indexed by octree type id (256 entries); `None` for empty/unused types.
    pub types: Vec<Option<TerrainMaterial>>,
    /// `Voxeland.surfaceDensityValue`.
    pub surface_density_value: f32,
    pub scene_types: usize,
    pub prefab_types: usize,
    /// Prefabs with `globalId` 0, which is empty space and never drawn
    /// (presumably prefabs not assigned to a type); skipped.
    pub prefabs_without_id: usize,
    /// Type ids defined by both the scene and a prefab, with different materials.
    pub conflicts: Vec<usize>,
    /// Distinct textures loaded.
    pub texture_count: usize,
    /// Problems with individual types (the rest still load).
    pub warnings: Vec<String>,
}

fn expect_class(object: &ObjectRef, class: i32) -> Result<&[u8]> {
    let (info, data) = object.data()?;
    if info.class_id != class {
        return Err(format!(
            "{} object {}: class {} where {class} was expected",
            object.file.name, object.path_id, info.class_id
        ));
    }
    Ok(data)
}

/// All MonoBehaviours in `file` whose script class is `class_name`, as raw
/// object bytes.
fn behaviours<'f>(assets: &Assets, file: &'f FileRef, class_name: &str) -> Result<Vec<&'f [u8]>> {
    let big_endian = file.file().big_endian;
    let mut script_names: HashMap<(PathBuf, String, i64), bool> = HashMap::new();
    let mut out = Vec::new();
    for info in file
        .objects()
        .iter()
        .filter(|o| o.class_id == MONO_BEHAVIOUR)
    {
        let Some(data) = file.file().object_data(file.bytes(), info) else {
            continue;
        };
        let Ok(header) = MonoBehaviourHeader::parse(data, big_endian) else {
            continue;
        };
        // Scripts we can't resolve can't be the one we look for; skip them.
        let Ok(Some(script)) = assets.resolve(file, header.script) else {
            continue;
        };
        let matches = match script_names.get(&script.key()) {
            Some(m) => *m,
            None => {
                let m = expect_class(&script, MONO_SCRIPT)
                    .ok()
                    .and_then(|d| MonoScript::parse(d, script.file.file().big_endian).ok())
                    .is_some_and(|s| s.class_name == class_name);
                script_names.insert(script.key(), m);
                m
            }
        };
        if matches {
            out.push(data);
        }
    }
    Ok(out)
}

fn load_texture(assets: &Assets, object: &ObjectRef) -> Result<TerrainTexture> {
    let data = expect_class(object, TEXTURE_2D)?;
    let texture =
        Texture2D::parse(data, object.file.file().big_endian).map_err(|e| e.to_string())?;
    let pixels = match &texture.stream {
        None => texture.image_data.clone(),
        Some(stream) => object
            .file
            .resource_range(
                assets.game(),
                stream.file_name(),
                stream.offset,
                stream.size as usize,
            )
            .map_err(|e| format!("{}: {e}", texture.name))?,
    };
    Ok(TerrainTexture {
        texture,
        data: pixels,
    })
}

type TextureCache = HashMap<(PathBuf, String, i64), Arc<TerrainTexture>>;

fn load_material(
    assets: &Assets,
    textures: &mut TextureCache,
    from: &FileRef,
    type_id: usize,
    source: BlockSource,
    block: &VoxelandBlockType,
) -> Result<TerrainMaterial> {
    let object = assets
        .resolve(from, block.material)?
        .ok_or("null material")?;
    let data = expect_class(&object, MATERIAL)?;
    let material =
        Material::parse(data, object.file.file().big_endian).map_err(|e| e.to_string())?;
    let mut texture = |slot: &str| -> Result<Option<Arc<TerrainTexture>>> {
        let Some(env) = material.texture(slot) else {
            return Ok(None);
        };
        let Some(target) = assets.resolve(&object.file, env.texture)? else {
            return Ok(None);
        };
        if let Some(t) = textures.get(&target.key()) {
            return Ok(Some(t.clone()));
        }
        let loaded = Arc::new(load_texture(assets, &target)?);
        textures.insert(target.key(), loaded.clone());
        Ok(Some(loaded))
    };
    let tint = material.color("_Color").unwrap_or([1.0; 4]);
    let (cap, side) = if material.texture("_CapTexture").is_some() {
        let cap = SurfaceLayer {
            albedo: texture("_CapTexture")?,
            normal: texture("_CapBumpMap")?,
            scale: material.float("_CapScale").unwrap_or(0.1),
            tint: material.color("_CapColor").unwrap_or([1.0; 4]),
        };
        let side = SurfaceLayer {
            albedo: texture("_SideTexture")?,
            normal: texture("_SideBumpMap")?,
            scale: material.float("_SideScale").unwrap_or(0.1),
            tint,
        };
        (cap, side)
    } else {
        let plain = SurfaceLayer {
            albedo: texture("_MainTex")?,
            normal: texture("_BumpMap")?,
            scale: material.float("_TriplanarScale").unwrap_or(0.1),
            tint,
        };
        (plain.clone(), plain)
    };
    Ok(TerrainMaterial {
        type_id,
        source,
        cap,
        side,
        shader_keywords: material.keywords.clone(),
        name: material.name,
        layer: block.layer,
    })
}

/// Reads every terrain block type with its material and textures.
pub fn terrain_materials(assets: &Assets) -> Result<TerrainMaterials> {
    // 1. The scene's Voxeland table.
    let bundle = assets
        .bundle_named("main.unity_")
        .ok_or("main scene bundle (main.unity_*.bundle) not found")?
        .to_path_buf();
    let loaded = assets.bundle(&bundle)?;
    let scene_name = loaded
        .file_names()
        .find(|n| !n.ends_with(".sharedAssets"))
        .ok_or("main scene bundle has no scene file")?
        .to_string();
    let scene = assets.file(&bundle, &scene_name)?;
    let voxeland_data = behaviours(assets, &scene, "Voxeland")?;
    let voxeland_bytes = voxeland_data
        .first()
        .ok_or("no Voxeland component in the main scene")?;
    let voxeland = Voxeland::parse(voxeland_bytes, scene.file().big_endian)
        .map_err(|e| format!("Voxeland: {e}"))?;

    // (definition, the file its references are relative to, source)
    let mut defs: Vec<Option<(VoxelandBlockType, FileRef, BlockSource)>> = vec![None; 256];
    let mut scene_types = 0;
    for (id, block) in voxeland.types.iter().enumerate().take(256) {
        if block.filled && !block.material.is_null() {
            defs[id] = Some((block.clone(), scene.clone(), BlockSource::Scene));
            scene_types += 1;
        }
    }

    // 2. Block-type prefabs in the player's resources.
    let resources = assets.standalone("resources.assets")?;
    let mut prefab_types = 0;
    let mut prefabs_without_id = 0;
    let mut conflicts = Vec::new();
    for data in behaviours(assets, &resources, "VoxelandBlockTypePrefab")? {
        let prefab = VoxelandBlockTypePrefab::parse(data, resources.file().big_endian)
            .map_err(|e| format!("VoxelandBlockTypePrefab: {e}"))?;
        let id = usize::from(prefab.global_id);
        if id == 0 {
            prefabs_without_id += 1;
            continue;
        }
        prefab_types += 1;
        if let Some((existing, from, BlockSource::Scene)) = &defs[id] {
            // The scene keeps its own copies of materials, so compare names.
            let name = |from: &FileRef, pptr| -> Result<Option<String>> {
                let Some(object) = assets.resolve(from, pptr)? else {
                    return Ok(None);
                };
                let data = expect_class(&object, MATERIAL)?;
                Ok(Some(
                    Material::parse(data, object.file.file().big_endian)
                        .map_err(|e| e.to_string())?
                        .name,
                ))
            };
            if name(from, existing.material)? != name(&resources, prefab.block_type.material)? {
                conflicts.push(id);
            }
        }
        if prefab.block_type.filled && !prefab.block_type.material.is_null() {
            defs[id] = Some((prefab.block_type, resources.clone(), BlockSource::Prefab));
        }
    }

    // 3. Materials and textures.
    let mut textures = TextureCache::new();
    let mut warnings = Vec::new();
    let mut types = Vec::with_capacity(256);
    for (id, def) in defs.iter().enumerate() {
        let Some((block, from, source)) = def else {
            types.push(None);
            continue;
        };
        match load_material(assets, &mut textures, from, id, *source, block) {
            Ok(material) => types.push(Some(material)),
            Err(e) => {
                warnings.push(format!("type {id}: {e}"));
                types.push(None);
            }
        }
    }
    conflicts.sort_unstable();
    conflicts.dedup();
    Ok(TerrainMaterials {
        types,
        surface_density_value: voxeland.surface_density_value,
        scene_types,
        prefab_types,
        prefabs_without_id,
        conflicts,
        texture_count: textures.len(),
        warnings,
    })
}
