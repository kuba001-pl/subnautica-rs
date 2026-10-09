//! Readers for Unity's container formats, as used by Subnautica (Unity
//! 2019.4): UnityFS asset bundles and serialized files.
//!
//! Pure: takes bytes, returns data, never panics on malformed input. Object
//! *contents* (meshes, textures, …) are decoded elsewhere; this crate only
//! finds them. See `docs/formats/unity.md`.

mod addressables;
mod bundle;
mod camera;
mod classes;
pub mod json;
mod light;
mod marmo;
mod mesh;
mod objects;
mod placeholder;
mod prefab;
mod reader;
mod scene_scripts;
mod serialized;
mod shader;
mod sky;
mod texture;
mod voxeland;
mod water;
mod water_surface;
mod world_entity;

use std::fmt;

pub use addressables::{Catalog, Location};
pub use bundle::{BlockInfo, Bundle, BundleDirectory, BundleNode, Compression, write_bundle};
pub use camera::{Camera, TAG_MAIN_CAMERA};
pub use classes::class_name;
pub use light::{
    BAKE_BAKED, BAKE_MIXED, BAKE_REALTIME, DayNightLight, Light, LightKind, ShadowKind,
    VfxVolumetricLight,
};
pub use marmo::{MarmoSkiesPrefabs, MarmoSky, SH_CONSTANTS, SKIES_AUTO, SkyApplier};
pub use mesh::{Channel, Mesh, MeshFilter, MeshGeometry, SubMesh, channel};
pub use objects::{Material, MonoBehaviourHeader, MonoScript, PPtr, TexEnv};
pub use placeholder::{PrefabPlaceholder, PrefabPlaceholdersGroup};
pub use prefab::{
    AssetBundleManifest, GameObject, Lod, LodGroup, MeshRenderer, SkinnedMeshRenderer,
    TransformNode, parse_resource_container, parse_text_asset,
};
pub use scene_scripts::{
    AutoLoadScene, CrashedShipExploder, EscapePod, PrefabSpawner, SPAWN_INTERMITTENT, SPAWN_MANUAL,
    SPAWN_ON_AWAKE, SPAWN_ON_NEW_BORN, SPAWN_ON_START, SpawnPrefab, parse_additional_scenes,
    parse_autoload_scenes, parse_random_start,
};
pub use serialized::{
    External, ObjectInfo, SUPPORTED_VERSIONS, SerializedFile, SerializedType, TypeTreeNode,
};
pub use shader::{
    BlendState, PassState, Shader, ShaderPass, ShaderProperty, ShaderValue, SubShader,
};
pub use sky::{Gradient, SkyDome, SkyLight, SkyManager};
pub use texture::{StreamingInfo, Texture2D, TextureFormat};
pub use voxeland::{GrassSettings, Voxeland, VoxelandBlockType, VoxelandBlockTypePrefab};
pub use water::{BiomeWater, WaterBiomeManager, WaterSettings, WaterscapeVolume};
pub use water_surface::{AnimationCurve, FftWaves, Keyframe, WaterSurface};
pub use world_entity::{WorldEntityInfo, parse_world_entity_data};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Byte offset where the problem was found (in the decompressed data for
    /// errors inside a bundle's block table).
    pub offset: usize,
    pub kind: ErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEof { needed: usize },
    BadMagic(String),
    Unsupported(String),
    Decompress(String),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = self.offset;
        match &self.kind {
            ErrorKind::UnexpectedEof { needed } => {
                write!(
                    f,
                    "unexpected end of data at byte {at}: needed {needed} more"
                )
            }
            ErrorKind::BadMagic(found) => write!(f, "not a UnityFS bundle (signature {found:?})"),
            ErrorKind::Unsupported(what) => write!(f, "unsupported at byte {at}: {what}"),
            ErrorKind::Decompress(what) => write!(f, "decompression failed at byte {at}: {what}"),
            ErrorKind::Invalid(what) => write!(f, "invalid data at byte {at}: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
