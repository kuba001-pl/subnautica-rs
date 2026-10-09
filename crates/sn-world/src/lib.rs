//! Subnautica world layout: how big the world is, how it is cut into octrees
//! and batches, and how batch files are named.
//!
//! Also reads the world's saved objects (`entities`) and fills their spawn
//! slots (`slots`).
//!
//! Pure: parses bytes and strings and does arithmetic, never touches the
//! filesystem.

mod biomes;
mod entities;
mod slots;
mod wire;

use std::fmt;

pub use biomes::{BatchRootSettings, BiomeMap, parse_biome_names};
pub use entities::{
    BakedCell, BatchCells, EntityError, ObjectTree, SavedComponent, SavedObject, TREE_MAGIC,
    Transform, parse_prefab_database,
};
pub use slots::{
    EXTRA_COPY_RADIUS, EntityInfo, EntitySlot, FILLER_CLASS_ID, Filler, LootDistribution,
    LootEntry, LootRows, SLOTS_COMPONENT, SlotKind, SlotRng, SlotSpawn, choose, fill_slots,
    parse_slots,
};

/// Offset between voxel indices and Unity world coordinates. **Plausible
/// hypothesis**, not confirmed (see `docs/formats/optoctrees.md`): it puts the
/// seabed at the lifepod start 17 m deep and the highest peak at +156 m.
pub const VOXEL_WORLD_OFFSET: [f32; 3] = [2048.0, 3040.0, 2048.0];

/// Converts a position in voxel-index space (voxel `v` spans `v..v+1`; mesh
/// positions from `sn-terrain` use voxel centres at integers) to Unity world
/// coordinates (left-handed, y up). Callers convert handedness themselves.
pub fn voxel_to_world(p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|a| p[a] + 0.5 - VOXEL_WORLD_OFFSET[a])
}

/// Inverse of [`voxel_to_world`].
pub fn world_to_voxel(p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|a| p[a] - 0.5 + VOXEL_WORLD_OFFSET[a])
}

/// Contents of `SNUnmanagedData/Build*/index.txt`.
///
/// Layout (confirmed for build 18):
/// ```text
/// 0                  <- header value, meaning unknown (always 0 so far)
/// 4096 3200 4096     <- world size in voxels (x y z)
/// 128 100 128        <- world size in octrees
/// 32                 <- octree edge length in voxels
/// 5 5 5              <- batch size in octrees
/// <one float per batch, 26*20*26 lines>  <- meaning unknown, mostly 0
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct WorldIndex {
    pub header: i64,
    pub world_voxels: [u32; 3],
    pub world_octrees: [u32; 3],
    pub octree_size: u32,
    pub batch_octrees: [u32; 3],
    /// One value per batch, in file order. Order and meaning not yet known.
    pub batch_values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexError {
    /// 1-based line number.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "index.txt line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for IndexError {}

impl WorldIndex {
    pub fn parse(text: &str) -> Result<WorldIndex, IndexError> {
        let mut lines = text.lines().map(str::trim).enumerate();
        let mut next = |what: &str| {
            lines
                .next()
                .map(|(i, l)| (i + 1, l))
                .ok_or_else(|| IndexError {
                    line: 0,
                    message: format!("file ends before {what}"),
                })
        };
        let header = parse_values::<i64, 1>(next("header")?)?[0];
        let world_voxels = parse_values(next("world size")?)?;
        let world_octrees = parse_values(next("octree counts")?)?;
        let octree_size = parse_values::<u32, 1>(next("octree size")?)?[0];
        let batch_octrees = parse_values(next("batch size")?)?;

        let index = WorldIndex {
            header,
            world_voxels,
            world_octrees,
            octree_size,
            batch_octrees,
            batch_values: Vec::new(),
        };
        if (0..3).any(|a| world_octrees[a] * octree_size != world_voxels[a])
            || batch_octrees.contains(&0)
        {
            return Err(IndexError {
                line: 5,
                message: "world, octree and batch sizes are inconsistent".into(),
            });
        }

        let mut batch_values = Vec::new();
        for (i, line) in lines.filter(|(_, l)| !l.is_empty()) {
            batch_values.push(parse_values::<f32, 1>((i + 1, line))?[0]);
        }
        let expected = index.batch_count();
        if batch_values.len() != expected {
            return Err(IndexError {
                line: 6,
                message: format!(
                    "expected {expected} per-batch values, found {}",
                    batch_values.len()
                ),
            });
        }
        Ok(WorldIndex {
            batch_values,
            ..index
        })
    }

    /// Number of batches along x, y, z (the last one may be partial).
    pub fn batches(&self) -> [u32; 3] {
        [0, 1, 2].map(|a| self.world_octrees[a].div_ceil(self.batch_octrees[a]))
    }

    pub fn batch_count(&self) -> usize {
        self.batches().iter().map(|&n| n as usize).product()
    }

    /// How many octrees the batch spans along x, y, z, or `None` if the batch
    /// lies outside the world. Batches on the far edges are partial.
    pub fn batch_octree_dims(&self, batch: BatchCoord) -> Option<[usize; 3]> {
        let coord = [batch.x, batch.y, batch.z];
        let mut dims = [0; 3];
        for a in 0..3 {
            let start = i64::from(coord[a]) * i64::from(self.batch_octrees[a]);
            let end = i64::from(self.world_octrees[a]);
            if coord[a] < 0 || start >= end {
                return None;
            }
            dims[a] = (end - start).min(i64::from(self.batch_octrees[a])) as usize;
        }
        Some(dims)
    }
}

fn parse_values<T: std::str::FromStr, const N: usize>(
    (line, text): (usize, &str),
) -> Result<[T; N], IndexError> {
    let err = || IndexError {
        line,
        message: format!("expected {N} number(s), found {text:?}"),
    };
    let mut parts = text.split_whitespace();
    let mut out = Vec::with_capacity(N);
    for _ in 0..N {
        out.push(parts.next().ok_or_else(err)?.parse().map_err(|_| err())?);
    }
    if parts.next().is_some() {
        return Err(err());
    }
    out.try_into().map_err(|_| err())
}

/// Integer coordinates of a batch, as used in file names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BatchCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BatchCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    /// `batch-objects-X-Y-Z.bin` (in `BatchObjectsCache`)
    pub fn objects_file_name(self) -> String {
        format!("batch-objects-{}-{}-{}.bin", self.x, self.y, self.z)
    }

    /// `baked-batch-cells-X-Y-Z.bin` (in `CellsCache`)
    pub fn cells_file_name(self) -> String {
        format!("baked-batch-cells-{}-{}-{}.bin", self.x, self.y, self.z)
    }

    /// `compiled-batch-X-Y-Z.optoctrees`
    pub fn octree_file_name(self) -> String {
        format!("compiled-batch-{}-{}-{}.optoctrees", self.x, self.y, self.z)
    }

    /// Inverse of [`BatchCoord::octree_file_name`]. Negative coordinates are
    /// not supported (the game has none; the name format would be ambiguous).
    pub fn from_octree_file_name(name: &str) -> Option<Self> {
        Self::from_file_name(name, "compiled-batch-", ".optoctrees")
    }

    /// `PREFIX` + `X-Y-Z` + `SUFFIX` → coordinates (non-negative only).
    pub fn from_file_name(name: &str, prefix: &str, suffix: &str) -> Option<Self> {
        let rest = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
        let mut parts = rest.split('-').map(str::parse::<i32>);
        let coord = Self::new(
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        parts.next().is_none().then_some(coord)
    }
}

impl fmt::Display for BatchCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}-{}", self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index(values: usize) -> String {
        let mut text = String::from("0\r\n4096 3200 4096\r\n128 100 128\r\n32\r\n5 5 5\r\n");
        for i in 0..values {
            text += if i == 3 { "0.5\r\n" } else { "0\r\n" };
        }
        text
    }

    #[test]
    fn parses_index() {
        let index = WorldIndex::parse(&sample_index(26 * 20 * 26)).unwrap();
        assert_eq!(index.world_octrees, [128, 100, 128]);
        assert_eq!(index.batches(), [26, 20, 26]);
        assert_eq!(index.batch_values[3], 0.5);
    }

    #[test]
    fn rejects_wrong_value_count() {
        let err = WorldIndex::parse(&sample_index(10)).unwrap_err();
        assert!(err.message.contains("13520"), "{err}");
    }

    #[test]
    fn rejects_inconsistent_sizes() {
        let text = sample_index(0).replace("4096 3200", "4000 3200");
        assert!(WorldIndex::parse(&text).is_err());
        assert!(WorldIndex::parse("0\n1 2").is_err());
    }

    #[test]
    fn edge_batches_are_partial() {
        let index = WorldIndex::parse(&sample_index(26 * 20 * 26)).unwrap();
        let dims = |x, y, z| index.batch_octree_dims(BatchCoord::new(x, y, z));
        assert_eq!(dims(12, 18, 12), Some([5, 5, 5]));
        assert_eq!(dims(25, 12, 14), Some([3, 5, 5]));
        assert_eq!(dims(10, 12, 25), Some([5, 5, 3]));
        assert_eq!(dims(26, 0, 0), None);
        assert_eq!(dims(-1, 0, 0), None);
    }

    #[test]
    fn file_names_round_trip() {
        let coord = BatchCoord::new(12, 18, 3);
        let name = coord.octree_file_name();
        assert_eq!(name, "compiled-batch-12-18-3.optoctrees");
        assert_eq!(BatchCoord::from_octree_file_name(&name), Some(coord));
        assert_eq!(
            BatchCoord::from_octree_file_name("compiled-batch-1-2.optoctrees"),
            None
        );
        assert_eq!(
            BatchCoord::from_octree_file_name("batch-objects-0-1-2.bin"),
            None
        );
    }
}
