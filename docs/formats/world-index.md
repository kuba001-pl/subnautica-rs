# World index: `index.txt` and `meta.txt`

Location: `Subnautica_Data/StreamingAssets/SNUnmanagedData/Build18/`
Described build: game build `10`, data folder `Build18`. Reader: `crates/sn-world`.

## `index.txt` — confirmed

Text, CRLF line endings, 13,525 lines.

| Line | Content (build 18) | Meaning |
|---|---|---|
| 1 | `0` | unknown (header/version?) |
| 2 | `4096 3200 4096` | world size in voxels (x y z) |
| 3 | `128 100 128` | world size in octrees |
| 4 | `32` | octree edge length in voxels |
| 5 | `5 5 5` | batch size in octrees |
| 6… | one decimal number per line | one value per batch |

Lines 2–4 are consistent (128 × 32 = 4096, 100 × 32 = 3200). There are exactly
26 × 20 × 26 = 13,520 per-batch values (batches per axis = ceil(octrees / 5)).
655 are non-zero, ranging from about 0.00095 to 5.06. Their meaning and their
ordering (which value belongs to which batch) are **unknown**.

## `meta.txt` — not yet understood

Two lines: `39` and `BlockPrefabs`. Possibly the number of block (terrain
material) prefabs and where to find them — **hypothesis**, relevant to the
type → material mapping (M2/M6).
