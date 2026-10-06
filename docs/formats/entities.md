# World entities: placement caches and prefab ids

Described build: game build `10`, world data `SNUnmanagedData/Build18`.
Code: `crates/sn-world/src/entities.rs` (parsers), `crates/sn-world/src/wire.rs`
(protobuf wire reader), `crates/sn-install` (file access). Inspector:
`sn-inspect entities <X> <Y> <Z>` and `sn-inspect entities --all`.

Each fact is marked **confirmed** (with how) or **hypothesis**.

## Files

| File | What | Count |
|---|---|---|
| `Build18/BatchObjectsCache/batch-objects-X-Y-Z.bin` | Objects stored per terrain batch | 2,975 files, 12 MB |
| `Build18/CellsCache/baked-batch-cells-X-Y-Z.bin` | Objects stored per cell of a batch | 1,606 files, 164 MB |
| `SNUnmanagedData/prefabs.db` | ClassId → prefab path | 3,336 entries |
| `StreamingAssets/aa/catalog.json` | Addressables catalog: prefab path → bundle | 21,100 internal ids |

All numbers below are **confirmed** by `sn-inspect entities --all` and the
opt-in test `crates/sn-install/tests/entities.rs`, unless marked otherwise.

## Object trees — confirmed

Every file of both caches parses to its last byte with this grammar.

The data is a sequence of protobuf messages, each preceded by its length as
a varint (no field tag):

1. Header: field 1 = 1369164567 (a fixed value in every tree; we treat it
   as a magic number), field 2 = 4 (**hypothesis**: a version).
2. Field 1 = number of GameObjects.
3. Per GameObject:
   - The GameObject: field 4 = tag (`Untagged`), field 6 = its id (GUID
     text), field 7 = **ClassId** (GUID text, the prefab; empty for 4,681
     objects, all baked `UnityEngine.Light`s), field 8 = parent's id (absent
     for roots), field 3 = 0 or 21 (**hypothesis**: the Unity layer).
     Fields 1, 2, 9 and 10 are varints we don't understand yet.
   - Field 1 = number of components.
   - Per component: a message with field 1 = type name and field 2 = 1,
     then the component's own message.
   - `UnityEngine.Transform`: field 1 = position (x, y, z as fields 1–3,
     float), field 2 = rotation (quaternion x, y, z, w), field 3 = scale.
     All 419,846 transforms write every field. Unity coordinates, relative
     to the parent (confirmed: cell roots sit on the cell grid, children
     within a few metres of them).

Ids are unique within a tree; no object is its own parent; every parent id
is found in the same tree.

## `BatchObjectsCache` — confirmed

One object tree per file: 2,975 files, 5,779 objects. Each file's single
root is a `Misc/BatchRoot.prefab` with a `LargeWorldBatchRoot` component
(colours and distances that look like the batch's water and fog settings,
**hypothesis**; useful for M8). Its children are atmosphere volumes
(`WorldEntities/Atmosphere/…`) and lights, not visible models.

The root is normally at the batch's corner in world space
(`batch × 160 − (2048, 3040, 2048)`). **63** files store it elsewhere: by
0.1 m, by about 153 m in x, or 640 m lower in y. **Hypothesis**: the game
places batch roots itself when it loads a batch, so the stored position is
stale. With the roots moved back to their corners, every batch object lies
within 1 m of its batch.

## `CellsCache` — confirmed

1. Header: field 1 = version (9 in 1,558 files, 10 in 48), field 2 =
   number of cells in the file.
2. Per cell: a header message with field 1 = cell coordinates (a message
   with x, y, z as fields 1–3), field 2 = level, fields 3, 4, 5 = byte
   lengths of up to three blobs that follow, in that order. Field 3's blob is
   an object tree; fields 4 and 5 are always 0 in build 10.

Totals: 437,003 cells (level 0: 324,785; 1: 12,830; 2: 9,036; 3: 90,352), of
which 118,920 have objects; 414,067 objects, 1,195 distinct prefabs.

Every cell's root is a `Misc/CellRoot.prefab` at the cell's centre in world
space. Cells are **16 m** at level 0 and **32 m** at levels 1–3 (root
offsets from the batch corner: 8, 24, …, 152 at level 0; 16, 48, …, 144 at
the others).

Most-placed prefabs: `EntitySlotsPlaceholder` (90,289; spawn slots,
**hypothesis**: filled at run time), then coral-reef doodads
(`Coral_reef_tree_mushrooms_connector_01` 22,290, …).

Oddities, as found:
- **1,473** objects end up at exactly the world origin (their local position
  is the negative of their cell root's). **Hypothesis**: stray data; the
  game would show them at (0, 0, 0) near the lifepod.
- **32** others lie outside their batch: 30 within 13 m (large precursor and
  debris pieces near an edge), two far off (309 m and 1,179 m).

## `prefabs.db` — confirmed

`i32` count (3,336), then pairs of strings, each a 7-bit-encoded length and
UTF-8 bytes (.NET `BinaryWriter` style): ClassId, prefab path, e.g.
`WorldEntities/coral_reef_grass_03_02.prefab`. The file ends right after the
last pair. Every non-empty ClassId in both caches is in it.

Prefab paths are keys of the Addressables catalog (base64 key table in
`catalog.json`), which leads to the prefab's bundle in
`aa/StandaloneWindows64/` (M7b).
