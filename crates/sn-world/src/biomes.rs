//! Biomes: the 2D biome map (`biomeMap.bin`) with its names (`biomes.csv`),
//! and the per-batch overrides stored in each batch's `LargeWorldBatchRoot`.
//! See `docs/formats/water.md`.

use crate::wire::{Value, Wire, WireError};

/// `biomeMap.bin`: one byte per cell (an index into `biomes.csv`'s rows),
/// row by row along z, then two bytes: width / 64 and height / 64.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiomeMap {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<u8>,
}

impl BiomeMap {
    pub fn parse(data: &[u8]) -> Result<BiomeMap, WireError> {
        let err = |offset: usize, message: &str| WireError {
            offset,
            message: message.into(),
        };
        let [.., w, h] = data else {
            return Err(err(0, "biome map shorter than its size bytes"));
        };
        let (width, height) = (usize::from(*w) * 64, usize::from(*h) * 64);
        if width == 0 || height == 0 || width * height > data.len() - 2 {
            return Err(err(
                data.len() - 2,
                &format!(
                    "{width}×{height} cells don't fit in {} bytes",
                    data.len() - 2
                ),
            ));
        }
        Ok(BiomeMap {
            width,
            height,
            cells: data[..width * height].to_vec(),
        })
    }

    /// The biome index at voxel column (`x`, `z`) of a world `land_size`
    /// voxels wide (the map is scaled down by `land_size / width`, rounded
    /// down, like the game). `None` outside the map.
    pub fn index_at(&self, x: i32, z: i32, land_size: usize) -> Option<u8> {
        let factor = (land_size / self.width).max(1) as i32;
        if x < 0 || z < 0 {
            return None;
        }
        let (cx, cz) = ((x / factor) as usize, (z / factor) as usize);
        if cx >= self.width {
            return None;
        }
        self.cells.get(cz * self.width + cx).copied()
    }
}

/// `biomes.csv`: a header row (`name`), then one biome name per row.
pub fn parse_biome_names(csv: &str) -> Vec<String> {
    csv.lines()
        .skip(1)
        .map(|l| l.split(',').next().unwrap_or("").trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The fields of a batch's `LargeWorldBatchRoot` component we use
/// (protobuf member numbers from the game's class).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchRootSettings {
    /// Member 2: biome for the whole batch instead of the map's.
    pub override_biome: Option<String>,
    /// Member 3: RGBA.
    pub fog_color: Option<[f32; 4]>,
    /// Members 4 and 5.
    pub fog_start: Option<f32>,
    pub fog_max: Option<f32>,
}

impl BatchRootSettings {
    pub fn parse(data: &[u8]) -> Result<BatchRootSettings, WireError> {
        let mut w = Wire::new(data);
        let mut out = BatchRootSettings::default();
        while let Some((number, value)) = w.field()? {
            match (number, value) {
                (2, Value::Bytes(b)) => {
                    let s = String::from_utf8_lossy(b.bytes()).into_owned();
                    out.override_biome = (!s.is_empty()).then_some(s);
                }
                (3, Value::Bytes(mut b)) => {
                    let mut c = [0.0f32; 4];
                    while let Some((n, v)) = b.field()? {
                        if let (1..=4, Value::Fixed32(bits)) = (n, v) {
                            c[n as usize - 1] = f32::from_bits(bits);
                        }
                    }
                    out.fog_color = Some(c);
                }
                (4, Value::Fixed32(bits)) => out.fog_start = Some(f32::from_bits(bits)),
                (5, Value::Fixed32(bits)) => out.fog_max = Some(f32::from_bits(bits)),
                _ => {}
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::encode::*;

    #[test]
    fn map_size_comes_from_the_last_two_bytes() {
        let mut data = vec![0u8; 64 * 128];
        data[64 + 3] = 7; // row 1, column 3
        data.extend([1, 2]);
        let map = BiomeMap::parse(&data).unwrap();
        assert_eq!((map.width, map.height), (64, 128));
        // A 256-voxel-wide world: 4 voxels per cell.
        assert_eq!(map.index_at(12, 4, 256), Some(7));
        assert_eq!(map.index_at(15, 7, 256), Some(7));
        assert_eq!(map.index_at(16, 4, 256), Some(0));
        assert_eq!(map.index_at(-1, 0, 256), None);
        assert_eq!(map.index_at(256, 0, 256), None);
        assert!(BiomeMap::parse(&[0, 0, 1, 1]).is_err());
        assert!(BiomeMap::parse(&[]).is_err());
    }

    #[test]
    fn names_skip_the_header() {
        assert_eq!(
            parse_biome_names("name\r\nsafeShallows\r\nkelpForest\r\n\r\n"),
            vec!["safeShallows", "kelpForest"]
        );
    }

    #[test]
    fn batch_root_settings() {
        let mut colour = Vec::new();
        float(&mut colour, 2, 0.5);
        float(&mut colour, 4, 1.0);
        let mut data = Vec::new();
        bytes(&mut data, 2, b"kelpForest");
        bytes(&mut data, 3, &colour);
        float(&mut data, 4, 35.0);
        float(&mut data, 5, 350.0);
        let s = BatchRootSettings::parse(&data).unwrap();
        assert_eq!(s.override_biome.as_deref(), Some("kelpForest"));
        assert_eq!(s.fog_color, Some([0.0, 0.5, 0.0, 1.0]));
        assert_eq!((s.fog_start, s.fog_max), (Some(35.0), Some(350.0)));
        assert_eq!(
            BatchRootSettings::parse(&[]).unwrap(),
            BatchRootSettings::default()
        );
    }
}
