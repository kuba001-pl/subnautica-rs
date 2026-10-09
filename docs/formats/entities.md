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
filled when a cell first loads, see § Spawn slots), then coral-reef doodads
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

Prefab paths are keys of the Addressables catalog, which leads to the
prefab's bundle in `aa/StandaloneWindows64/`; see `unity.md` § Prefabs. A
placed object's saved Transform **replaces** its prefab root's transform
(confirmed there).

## Spawn slots

Code: `crates/sn-world/src/slots.rs` (slots, the fill rule),
`crates/sn-unity/src/world_entity.rs`, `crates/sn-assets/src/slots.rs`
(tables). Inspector: `sn-inspect slots [<X> <Y> <Z>] [--seed <N>]`. Opt-in
test: `spawn_slot_tables_and_fill` in `crates/sn-assets/tests/real_data.rs`.

### Slot data — confirmed

An `EntitySlotsPlaceholder` component's saved data (in the cell object
trees) is a protobuf message: field 1 = version (1), field 2 repeated =
one slot each. A slot: field 1 = version (1), 2 = biome (the game's
`BiomeType` number, e.g. 102), 3 = allowed slot types as flags (1 small,
2 medium, 4 large, 8 tall, 16 creature), 4 = density (float), 5 = position
and 6 = rotation relative to the placeholder (vector/quaternion messages
as in transforms, zero fields left out). An absent density is 1 (the
class's initial value; protobuf-net runs the constructor). Confirmed by
parsing all 90,289 placeholders with 0 errors, and by the slots landing in
place: all but 7 of 1,288,139 slots lie inside their batch.

Census (`sn-inspect slots`): 1,288,139 slots; allowed types 0x03 (small or
medium) 528,027, 0x10 (creature) 759,730, a few hundred others; density
0.94–139.7. Slot offsets from their placeholder reach 1,638 m (not
understood; the world positions are fine). 72,670 slots are in 59 biomes
without a loot table (e.g. 122, 0); those stay empty.

### Tables — confirmed

- **Loot distribution**: the `TextAsset` that `Resources.Load` finds under
  `Balance/EntityDistributions` (in `resources.assets`). JSON with `//`
  comments: an object of ClassId → `{prefabPath, distribution: [{biome,
  count, probability}, …]}`; the key `None` is a filler entry the game
  skips. The game's editor writes the biome's enum name as a comment above
  each row; we read those names for logs only. Build 10: 190 entries,
  1,295 rows, 352 biomes.
- **World entity infos**: the MonoBehaviour `WorldEntities/WorldEntityData`:
  after the MonoBehaviour header, an `i32` count (3,336) and per info: the
  ClassId (aligned string), `TechType` (`i32`), slot type (`i32` index: 0
  small, 1 medium, 2 large, 3 tall, 4 creature), Z-up (bool padded to 4),
  cell level (`i32`: 0–3, 10 batch, 100 global), local scale (3 floats).
  68 bytes per info; 226,900 bytes in all, read to the last byte. Every
  prefab the distribution can pick has an info and a `prefabs.db` path.
- **Spawn restrictions** (`SNUnmanagedData/spawnrestrictions-<quality>.csv`,
  used by the game to drop a share of some prefabs): no such file in this
  install, so nothing is dropped.

### How the game fills a slot

From the game's behaviour (our reading of its scripts, re-implemented):
per slot, take the biome's rows in table order, skip `None`, prefabs whose
slot type the slot does not allow, and weights ≤ 0, where weight =
`probability / density`. A uniform draw *r* in [0, 1] is scaled by the
total if the total exceeds 1; the first row whose running sum reaches *r*
wins, otherwise the slot stays empty (so with a total below 1 the rest is
"nothing"). Density is the number of same-biome slots around (weighted,
32 m radius), computed when the world was built, so each neighbourhood
gets about `probability` objects. The winner spawns `count` times: the
first at the slot, the others at a random point within 4 m; rotation is
the slot's, turned −90° about x for Z-up prefabs; scale is the info's.
The game also skips fragments the player has already scanned (none in a
new game) — not ported.

The game draws from Unity's global random generator when a cell first
loads, so every save differs. We draw from SplitMix64 seeded by FNV-1a of
(placeholder id, seed, slot index): the same seed gives the same world in
any loading order (unit test and real-data test).

### What the slots hold — confirmed (seed 1)

141,639 objects: creatures 102,777 (`WorldEntities/Creatures/…`, e.g.
Boomerang, SpadeFish, Peeper), resource outcrops 35,808
(`WorldEntities/Natural/…`: limestone, sandstone, quartz, salt, metal),
eggs 1,186, fragments 1,029, doodads 654, others ~185. **No flora.** By
cell level: 0: 121,899, 1: 19,292, 2: 448. Seed 2 gives 142,446.
Lifepod batch 12-18-12: 30 placeholders, 1,719 slots, 325 objects (225
creatures).
