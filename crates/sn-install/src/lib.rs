//! Finds the player's Subnautica install and reads files from it.
//!
//! The only crate (besides apps) that touches the filesystem. Strictly
//! read-only: nothing here ever writes to the game folder.

use std::fmt;
use std::path::{Path, PathBuf};

use sn_octree::Batch;
use sn_terrain::TerrainBatch;
use sn_world::{BatchCoord, WorldIndex};

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

/// The terrain data folder of an install (`.../SNUnmanagedData/Build<N>`).
pub struct GameData {
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
        let unmanaged = root.join("Subnautica_Data/StreamingAssets/SNUnmanagedData");
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
        Ok(GameData { build_dir })
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

    /// All batches that have an octree file, sorted. Also returns the names
    /// of files in the folder that don't match the expected pattern.
    pub fn octree_batches(&self) -> Result<(Vec<BatchCoord>, Vec<String>)> {
        let dir = self.octree_dir();
        let entries = std::fs::read_dir(&dir).map_err(|e| io_error(&dir, e))?;
        let mut batches = Vec::new();
        let mut unknown = Vec::new();
        for entry in entries {
            let name = entry
                .map_err(|e| io_error(&dir, e))?
                .file_name()
                .to_string_lossy()
                .into_owned();
            match BatchCoord::from_octree_file_name(&name) {
                Some(coord) => batches.push(coord),
                None => unknown.push(name),
            }
        }
        batches.sort();
        Ok((batches, unknown))
    }
}
