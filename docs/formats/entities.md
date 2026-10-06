# World entities: placement caches and prefab ids

Described build: game build `10`, world data `SNUnmanagedData/Build18`.
Status: first look while planning M7 (2026-10-06). No code reads these yet.

Each fact is marked **confirmed** (with how) or **hypothesis**.

## Files

| File | What | Count |
|---|---|---|
| `Build18/BatchObjectsCache/batch-objects-X-Y-Z.bin` | Objects stored per terrain batch | 2,975 files, 12 MB |
| `Build18/CellsCache/baked-batch-cells-X-Y-Z.bin` | Objects stored per cell of a batch | 1,606 files, 164 MB |
| `SNUnmanagedData/prefabs.db` | ClassId → prefab path | 344 KB |
| `StreamingAssets/aa/catalog.json` | Addressables catalog: prefab path → bundle | 21,100 internal ids |

## Object trees — confirmed for one batch file

Confirmed by a throwaway wire dumper on `batch-objects-12-18-12.bin`: 96
messages that end exactly at the end of the file.

The file is a sequence of protobuf messages, each preceded by its length as a
varint (no field tag):

1. Header: field 1 = 1369164567 (**hypothesis**: a magic number), field 2 = 4
   (**hypothesis**: a version).
2. Field 1 = number of GameObjects (23 here).
3. Per GameObject:
   - The GameObject: field 2 = 1 and field 3 = 0 or 21 (**hypothesis**:
     active flag and Unity layer), field 4 = tag (`Untagged`), field 6 = its
     id (GUID text), field 7 = **ClassId** (GUID text, the prefab), field 8 =
     parent's id (absent for the root).
   - Field 1 = number of components.
   - Per component: a message with field 1 = type name
     (`UnityEngine.Transform`, `LargeWorldBatchRoot`, …) and field 2 = 1,
     then the component's own message.
   - `UnityEngine.Transform`: field 1 = position (x, y, z as fields 1–3,
     float), field 2 = rotation (quaternion x, y, z, w), field 3 = scale.
     Unity coordinates; **hypothesis**: local to the parent (the root of
     batch 12-18-12 sits at (−128, −160, −128), its children at
     positive values like (79.5, 92.0, 74.9)).
   - `LargeWorldBatchRoot` holds colours and distances that look like the
     batch's water and fog settings (**hypothesis**; useful for M8).

`CellsCache` files start with a small header message (field 1 = 9, field 2 =
256), then per cell a header (field 1 = cell coordinates as a 3-int message,
field 3 = byte length of what follows, more fields not yet understood),
followed by an object tree in the same format as above (**hypothesis** from
the first 60 bytes of `baked-batch-cells-12-18-12.bin`).

## `prefabs.db` — confirmed by hex dump

`i32` count (3,336), then pairs of strings, each a 7-bit-encoded length and
UTF-8 bytes (.NET `BinaryWriter` style): ClassId, prefab path, e.g.
`WorldEntities/coral_reef_grass_03_02.prefab`. Three ClassIds from the batch
file above were all found there.

Prefab paths are keys of the Addressables catalog (base64 key table in
`catalog.json`), which leads to the prefab's bundle in `aa/StandaloneWindows64/`.
