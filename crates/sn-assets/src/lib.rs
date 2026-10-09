//! Finds and loads Unity objects across the game's files.
//!
//! - [`Assets::index`] reads only the directory of each bundle (a few KB from
//!   the start of each file) to learn which bundle holds which internal file
//!   (`CAB-…`).
//! - Bundles and the player's standalone files (`resources.assets`, …) are
//!   loaded on demand and cached.
//! - [`Assets::resolve`] follows object references (`PPtr`) across files.
//!
//! Read-only: everything comes from the player's install via `sn-install`.

mod marmo;
mod prefab;
mod scene;
mod skin;
mod slots;
mod terrain;
mod water;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sn_install::GameData;
use sn_unity::{Bundle, BundleDirectory, ObjectInfo, PPtr, SerializedFile};

pub use marmo::{BiomeSky, MarmoSkies, marmo_skies};
pub use prefab::{Prefab, PrefabNode};
pub use scene::{LIGHTMAPPED_PREFAB, Scene, SceneSpawn};
pub use skin::{Mat4, skin};
pub use slots::{LootTable, entity_infos, loot_table, parse_loot_table};
pub use terrain::{
    BlendSettings, BlockSource, GrassLook, GrassShader, SurfaceLayer, TerrainGrass,
    TerrainMaterial, TerrainMaterials, TerrainTexture, terrain_materials,
};
pub use water::{
    SkyTextures, WaterSurfaceData, builtin_texture, resource_bytes, resource_texture, sky,
    sky_textures, water_biomes, water_caustics, water_surface, water_volume,
};

pub type Result<T> = std::result::Result<T, String>;

/// Bytes read from the start of each bundle to find its directory. Bundles
/// with a bigger directory fall back to a full read.
const DIRECTORY_PREFIX: usize = 64 * 1024;

enum Storage {
    /// A decompressed bundle: serialized files and resources are its nodes.
    Bundle(Bundle),
    /// A serialized file shipped next to the player (`resources.assets`, …).
    /// Its resources (`*.resS`) are separate files, read by range on demand.
    Standalone { name: String, bytes: Vec<u8> },
}

/// A bundle (or standalone serialized file) in memory, with its serialized
/// files parsed.
pub struct LoadedBundle {
    pub path: PathBuf,
    storage: Storage,
    files: HashMap<String, SerializedFile>,
}

impl LoadedBundle {
    fn load(game: &GameData, path: &Path) -> Result<LoadedBundle> {
        let bytes = game.read_file(path)?;
        let mut files = HashMap::new();
        let storage = if bytes.starts_with(b"UnityFS\0") {
            let bundle = Bundle::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            for node in bundle.nodes.iter().filter(|n| n.is_serialized_file()) {
                let file = SerializedFile::parse(bundle.node_data(node))
                    .map_err(|e| format!("{}/{}: {e}", path.display(), node.path))?;
                files.insert(node.path.clone(), file);
            }
            Storage::Bundle(bundle)
        } else {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let file =
                SerializedFile::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            files.insert(name.clone(), file);
            Storage::Standalone { name, bytes }
        };
        Ok(LoadedBundle {
            path: path.to_path_buf(),
            storage,
            files,
        })
    }

    /// The bytes of a serialized file (or, in bundles, any node) by name.
    pub fn node(&self, name: &str) -> Option<&[u8]> {
        match &self.storage {
            Storage::Bundle(bundle) => {
                let node = bundle.nodes.iter().find(|n| n.path == name)?;
                Some(bundle.node_data(node))
            }
            Storage::Standalone { name: own, bytes } => (own == name).then_some(bytes.as_slice()),
        }
    }

    /// `len` bytes at `offset` of a resource file (`*.resS`) belonging to
    /// this bundle or standalone file.
    pub fn resource_range(
        &self,
        game: &GameData,
        name: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        match &self.storage {
            Storage::Bundle(bundle) => {
                let node = bundle
                    .nodes
                    .iter()
                    .find(|n| n.path == name)
                    .ok_or_else(|| format!("{}: no resource {name}", self.path.display()))?;
                let start =
                    usize::try_from(offset).map_err(|_| format!("{name}: offset too big"))?;
                let end = start
                    .checked_add(len)
                    .ok_or_else(|| format!("{name}: range overflows"))?;
                bundle
                    .node_data(node)
                    .get(start..end)
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| format!("{name}: range out of bounds"))
            }
            Storage::Standalone { .. } => {
                let dir = self.path.parent().unwrap_or(Path::new(""));
                game.read_file_range(&dir.join(name), offset, len)
                    .map_err(String::from)
            }
        }
    }

    /// Names of the serialized files in this bundle.
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

pub struct Assets<'g> {
    game: &'g GameData,
    /// `CAB-…` name → bundle path.
    by_cab: HashMap<String, PathBuf>,
    bundles: Vec<PathBuf>,
    cache: Mutex<HashMap<PathBuf, Arc<LoadedBundle>>>,
}

impl<'g> Assets<'g> {
    /// Indexes every bundle's directory.
    pub fn index(game: &'g GameData) -> Result<Assets<'g>> {
        let bundles = game.bundles()?;
        let mut by_cab = HashMap::new();
        for path in &bundles {
            let prefix = game.read_file_prefix(path, DIRECTORY_PREFIX)?;
            let dir = match BundleDirectory::parse(&prefix) {
                Ok(dir) => dir,
                Err(_) => BundleDirectory::parse(&game.read_file(path)?)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            };
            for node in dir.nodes {
                let cab = node
                    .path
                    .split('.')
                    .next()
                    .unwrap_or(&node.path)
                    .to_string();
                by_cab.insert(cab, path.clone());
            }
        }
        Ok(Assets {
            game,
            by_cab,
            bundles,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn game(&self) -> &GameData {
        self.game
    }

    pub fn bundle_paths(&self) -> &[PathBuf] {
        &self.bundles
    }

    pub fn indexed_files(&self) -> usize {
        self.by_cab.len()
    }

    /// Loads a bundle or standalone file (cached).
    pub fn bundle(&self, path: &Path) -> Result<Arc<LoadedBundle>> {
        if let Some(b) = self.cache.lock().unwrap().get(path) {
            return Ok(b.clone());
        }
        let loaded = Arc::new(LoadedBundle::load(self.game, path)?);
        self.cache
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), loaded.clone());
        Ok(loaded)
    }

    /// Bytes held by loaded bundles and files.
    pub fn cached_bytes(&self) -> usize {
        self.cache
            .lock()
            .unwrap()
            .values()
            .map(|b| match &b.storage {
                Storage::Bundle(bundle) => bundle.data_len(),
                Storage::Standalone { bytes, .. } => bytes.len(),
            })
            .sum()
    }

    /// Forgets every loaded bundle once they hold more than `max_bytes`
    /// (objects already handed out keep theirs alive).
    pub fn trim_cache(&self, max_bytes: usize) {
        if self.cached_bytes() > max_bytes {
            self.cache.lock().unwrap().clear();
        }
    }

    /// The first bundle whose file name starts with `prefix`.
    pub fn bundle_named(&self, prefix: &str) -> Option<&Path> {
        self.bundles
            .iter()
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix))
            })
            .map(PathBuf::as_path)
    }

    /// A serialized file inside a bundle.
    pub fn file(&self, bundle: &Path, name: &str) -> Result<FileRef> {
        let loaded = self.bundle(bundle)?;
        if !loaded.files.contains_key(name) {
            return Err(format!("{}: no serialized file {name}", bundle.display()));
        }
        Ok(FileRef {
            bundle: loaded,
            name: name.to_string(),
        })
    }

    /// A serialized file shipped next to the player, e.g. `resources.assets`.
    pub fn standalone(&self, name: &str) -> Result<FileRef> {
        let path = self.game.data_dir.join(name);
        // Loaded standalone files are keyed by their file name alone.
        let key = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name)
            .to_string();
        Ok(FileRef {
            bundle: self.bundle(&path)?,
            name: key,
        })
    }

    /// Follows a reference made from `from`. `None` for null references.
    pub fn resolve(&self, from: &FileRef, pptr: PPtr) -> Result<Option<ObjectRef>> {
        if pptr.is_null() {
            return Ok(None);
        }
        let file = if pptr.file_id == 0 {
            from.clone()
        } else {
            let external = usize::try_from(pptr.file_id - 1)
                .ok()
                .and_then(|i| from.file().externals.get(i))
                .ok_or_else(|| format!("{}: bad file id {}", from.name, pptr.file_id))?;
            if let Some(archive) = external.path.strip_prefix("archive:/") {
                // "archive:/CAB-x/CAB-x.sharedAssets" → node "CAB-x.sharedAssets"
                // in the bundle that holds CAB-x.
                let node = archive.rsplit('/').next().unwrap_or(archive);
                let cab = node.split('.').next().unwrap_or(node);
                let bundle = self
                    .by_cab
                    .get(cab)
                    .ok_or_else(|| format!("no bundle contains {}", external.path))?
                    .clone();
                self.file(&bundle, node)?
            } else if let Some(builtin) = external
                .path
                .get(..8)
                .filter(|p| p.eq_ignore_ascii_case("library/"))
                .map(|_| &external.path[8..])
            {
                // Unity's built-in resources ship in Subnautica_Data/Resources.
                self.standalone(&format!("Resources/{builtin}"))?
            } else {
                // A file next to the player, e.g. "globalgamemanagers.assets".
                self.standalone(&external.path)?
            }
        };
        Ok(Some(ObjectRef {
            file,
            path_id: pptr.path_id,
        }))
    }
}

/// A serialized file inside a loaded bundle (or a standalone file).
#[derive(Clone)]
pub struct FileRef {
    pub bundle: Arc<LoadedBundle>,
    pub name: String,
}

impl FileRef {
    pub fn file(&self) -> &SerializedFile {
        &self.bundle.files[&self.name]
    }

    pub fn bytes(&self) -> &[u8] {
        self.bundle.node(&self.name).unwrap_or_default()
    }

    pub fn objects(&self) -> &[ObjectInfo] {
        &self.file().objects
    }

    pub fn object(&self, path_id: i64) -> Option<(&ObjectInfo, &[u8])> {
        let info = self.file().objects.iter().find(|o| o.path_id == path_id)?;
        Some((info, self.file().object_data(self.bytes(), info)?))
    }

    /// `len` bytes at `offset` of a resource file (`*.resS`) of this file.
    pub fn resource_range(
        &self,
        game: &GameData,
        name: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        self.bundle.resource_range(game, name, offset, len)
    }
}

/// An object in a specific file.
#[derive(Clone)]
pub struct ObjectRef {
    pub file: FileRef,
    pub path_id: i64,
}

impl ObjectRef {
    pub fn data(&self) -> Result<(&ObjectInfo, &[u8])> {
        self.file
            .object(self.path_id)
            .ok_or_else(|| format!("{}: no object {}", self.file.name, self.path_id))
    }

    /// Stable key for caching (bundle path, file, path id).
    pub fn key(&self) -> (PathBuf, String, i64) {
        (
            self.file.bundle.path.clone(),
            self.file.name.clone(),
            self.path_id,
        )
    }
}
