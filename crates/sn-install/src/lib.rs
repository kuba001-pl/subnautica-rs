//! Finds the player's Subnautica install and reads files from it.
//!
//! The only crate (besides apps) that touches the filesystem. Strictly
//! read-only: nothing here ever writes to the game folder.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sn_octree::Batch;
use sn_terrain::TerrainBatch;
use sn_world::{
    BatchCells, BatchCoord, BiomeMap, ObjectTree, WorldIndex, parse_biome_names,
    parse_prefab_database,
};

/// Environment variable naming the folder that contains `Subnautica.exe`.
pub const GAME_DIR_ENV: &str = "SUBNAUTICA_DIR";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.0
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn io_error(path: &Path, e: std::io::Error) -> Error {
    Error(format!("{}: {e}", path.display()))
}

/// Folders of an install that we read from.
pub struct GameData {
    /// `Subnautica_Data`: Unity serialized files (`*.assets`, `level0`, …).
    pub data_dir: PathBuf,
    /// The terrain data folder (`.../SNUnmanagedData/Build<N>`).
    pub build_dir: PathBuf,
}

impl GameData {
    /// Uses `game_dir` if given, else the [`GAME_DIR_ENV`] environment variable.
    pub fn locate(game_dir: Option<PathBuf>) -> Result<GameData> {
        let root = match game_dir {
            Some(dir) => dir,
            None => std::env::var_os(GAME_DIR_ENV)
                .map(PathBuf::from)
                .ok_or_else(|| {
                    Error(format!(
                        "no game folder: pass --game-dir <PATH> or set {GAME_DIR_ENV}"
                    ))
                })?,
        };
        let data_dir = root.join("Subnautica_Data");
        let unmanaged = data_dir.join("StreamingAssets/SNUnmanagedData");
        let entries = std::fs::read_dir(&unmanaged).map_err(|e| {
            Error(format!(
                "{} does not look like a Subnautica install (can't read {}: {e})",
                root.display(),
                unmanaged.display()
            ))
        })?;
        // Pick the highest Build<N> folder.
        let build_dir = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let n: u32 = name.strip_prefix("Build")?.parse().ok()?;
                Some((n, e.path()))
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, path)| path)
            .ok_or_else(|| Error(format!("no Build<N> folder in {}", unmanaged.display())))?;
        Ok(GameData {
            data_dir,
            build_dir,
        })
    }

    /// Addressables bundles (`StreamingAssets/aa/StandaloneWindows64`).
    pub fn bundle_dir(&self) -> PathBuf {
        self.data_dir.join("StreamingAssets/aa/StandaloneWindows64")
    }

    /// Reads a whole file. Relative paths are taken from `Subnautica_Data`.
    pub fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.data_dir.join(path)
        };
        std::fs::read(&full).map_err(|e| io_error(&full, e))
    }

    /// Reads at most `max` bytes from the start of a file.
    pub fn read_file_prefix(&self, path: &Path, max: usize) -> Result<Vec<u8>> {
        use std::io::Read;
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.data_dir.join(path)
        };
        let file = std::fs::File::open(&full).map_err(|e| io_error(&full, e))?;
        let mut out = Vec::with_capacity(max.min(1 << 20));
        file.take(max as u64)
            .read_to_end(&mut out)
            .map_err(|e| io_error(&full, e))?;
        Ok(out)
    }

    /// Reads `len` bytes at `offset` of a file (relative paths are taken
    /// from `Subnautica_Data`).
    pub fn read_file_range(&self, path: &Path, offset: u64, len: usize) -> Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.data_dir.join(path)
        };
        let mut file = std::fs::File::open(&full).map_err(|e| io_error(&full, e))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| io_error(&full, e))?;
        let mut out = vec![0; len];
        file.read_exact(&mut out).map_err(|e| io_error(&full, e))?;
        Ok(out)
    }

    /// Unity files outside the bundles: `*.assets`, `level<N>` and
    /// `globalgamemanagers` in `Subnautica_Data` (sorted).
    pub fn serialized_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let dir = &self.data_dir;
        for entry in std::fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
            let path = entry.map_err(|e| io_error(dir, e))?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let is_level = name
                .strip_prefix("level")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if path.is_file()
                && (name.ends_with(".assets") || is_level || name == "globalgamemanagers")
            {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    /// All `*.bundle` files in [`GameData::bundle_dir`] (sorted).
    pub fn bundles(&self) -> Result<Vec<PathBuf>> {
        let dir = self.bundle_dir();
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| io_error(&dir, e))? {
            let path = entry.map_err(|e| io_error(&dir, e))?.path();
            if path.extension().is_some_and(|e| e == "bundle") {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn octree_dir(&self) -> PathBuf {
        self.build_dir.join("CompiledOctreesCache")
    }

    pub fn read_index(&self) -> Result<WorldIndex> {
        let path = self.build_dir.join("index.txt");
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        WorldIndex::parse(&text).map_err(|e| Error(format!("{}: {e}", path.display())))
    }

    /// Raw bytes of a batch file, or `None` if the batch has no file.
    pub fn read_batch(&self, coord: BatchCoord) -> Result<Option<Vec<u8>>> {
        let path = self.octree_dir().join(coord.octree_file_name());
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(&path, e)),
        }
    }

    /// Reads and parses a batch and validates every octree in it. `None` if
    /// it has no file or lies outside the world.
    pub fn load_batch(
        &self,
        index: &WorldIndex,
        coord: BatchCoord,
    ) -> Result<Option<TerrainBatch>> {
        let Some(octree_dims) = index.batch_octree_dims(coord) else {
            return Ok(None);
        };
        let Some(bytes) = self.read_batch(coord)? else {
            return Ok(None);
        };
        let batch = Batch::parse(&bytes).map_err(|e| Error(format!("batch {coord}: {e}")))?;
        if batch.octrees.len() != octree_dims.iter().product::<usize>() {
            return Err(Error(format!(
                "batch {coord}: {} octrees, expected {octree_dims:?}",
                batch.octrees.len()
            )));
        }
        for (i, octree) in batch.octrees.iter().enumerate() {
            octree
                .validate()
                .map_err(|e| Error(format!("batch {coord} octree {i}: {e}")))?;
        }
        Ok(Some(TerrainBatch { batch, octree_dims }))
    }

    fn read_optional(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(path, e)),
        }
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.build_dir.join("BatchObjectsCache")
    }

    pub fn cells_dir(&self) -> PathBuf {
        self.build_dir.join("CellsCache")
    }

    /// A batch's saved objects (`BatchObjectsCache`); `None` without a file.
    pub fn read_batch_objects(&self, coord: BatchCoord) -> Result<Option<ObjectTree>> {
        let path = self.objects_dir().join(coord.objects_file_name());
        let Some(bytes) = self.read_optional(&path)? else {
            return Ok(None);
        };
        ObjectTree::parse(&bytes)
            .map(Some)
            .map_err(|e| Error(format!("{}: {e}", path.display())))
    }

    /// A batch's baked cells (`CellsCache`); `None` without a file.
    pub fn read_batch_cells(&self, coord: BatchCoord) -> Result<Option<BatchCells>> {
        let path = self.cells_dir().join(coord.cells_file_name());
        let Some(bytes) = self.read_optional(&path)? else {
            return Ok(None);
        };
        BatchCells::parse(&bytes)
            .map(Some)
            .map_err(|e| Error(format!("{}: {e}", path.display())))
    }

    /// `biomeMap.bin` and the names from `biomes.csv`.
    pub fn read_biome_map(&self) -> Result<(BiomeMap, Vec<String>)> {
        let path = self.build_dir.join("biomeMap.bin");
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        let map = BiomeMap::parse(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let path = self.build_dir.join("biomes.csv");
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        Ok((map, parse_biome_names(&text)))
    }

    /// `SNUnmanagedData/prefabs.db`: ClassId → prefab path.
    pub fn read_prefab_database(&self) -> Result<HashMap<String, String>> {
        let path = self
            .build_dir
            .parent()
            .unwrap_or(&self.build_dir)
            .join("prefabs.db");
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        let entries =
            parse_prefab_database(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Ok(entries.into_iter().collect())
    }

    /// Batches with a file in `dir` named by `parse`, sorted, plus the names
    /// of other files there.
    fn batches_in(
        &self,
        dir: &Path,
        parse: impl Fn(&str) -> Option<BatchCoord>,
    ) -> Result<(Vec<BatchCoord>, Vec<String>)> {
        let entries = std::fs::read_dir(dir).map_err(|e| io_error(dir, e))?;
        let mut batches = Vec::new();
        let mut unknown = Vec::new();
        for entry in entries {
            let name = entry
                .map_err(|e| io_error(dir, e))?
                .file_name()
                .to_string_lossy()
                .into_owned();
            match parse(&name) {
                Some(coord) => batches.push(coord),
                None => unknown.push(name),
            }
        }
        batches.sort();
        Ok((batches, unknown))
    }

    /// Batches with a `BatchObjectsCache` file.
    pub fn object_batches(&self) -> Result<(Vec<BatchCoord>, Vec<String>)> {
        self.batches_in(&self.objects_dir(), |n| {
            BatchCoord::from_file_name(n, "batch-objects-", ".bin")
        })
    }

    /// Batches with a `CellsCache` file.
    pub fn cell_batches(&self) -> Result<(Vec<BatchCoord>, Vec<String>)> {
        self.batches_in(&self.cells_dir(), |n| {
            BatchCoord::from_file_name(n, "baked-batch-cells-", ".bin")
        })
    }

    /// All batches that have an octree file, sorted. Also returns the names
    /// of files in the folder that don't match the expected pattern.
    pub fn octree_batches(&self) -> Result<(Vec<BatchCoord>, Vec<String>)> {
        self.batches_in(&self.octree_dir(), BatchCoord::from_octree_file_name)
    }
}
