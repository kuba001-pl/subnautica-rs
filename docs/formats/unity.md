# Unity containers: UnityFS bundles and serialized files

Described build: game build `10` (Unity **2019.4.36f1**). Reader:
`crates/sn-unity`. Inspector: `sn-inspect unity <FILE>…`, `unity --all`.
Cross-checked against UnityPy 1.25.4 (dev-time oracle, not a dependency).

Each fact is marked **confirmed** (with how) or **hypothesis**. Layout details
follow publicly documented Unity formats; this page records what *this game*
uses.

## Where the files are — confirmed

| Location | What | Count |
|---|---|---|
| `Subnautica_Data/*.assets`, `level0`, `globalgamemanagers` | Serialized files shipped with the player (`resources.assets` holds UI textures, shaders, materials…; `globalgamemanagers.assets` 4,046 MonoScripts) | 5 |
| `Subnautica_Data/StreamingAssets/aa/StandaloneWindows64/*.bundle` | Addressables asset bundles (prefabs, meshes, textures, the `main.unity` scene) | 5,467 (4.4 GB) |
| `…/resources.assets.resS`, `CAB-….resS` inside bundles | Raw resource blobs (texture pixels, mesh streams) referenced by objects | — |

`sn-inspect unity --all`: 5,472 files, 5,485 serialized files, 2,331
resource blobs, **423,677 objects**, parsed in about 2–3 s, 0 errors.

## UnityFS bundle — confirmed

All big-endian.

```
cstr  signature            "UnityFS"
u32   format               7 in every bundle
cstr  player version       "5.x.x"
cstr  engine version       "2019.4.36f1"
i64   total size
u32   blocks info size (compressed), u32 (uncompressed)
u32   flags                bits 0-5 compression of the blocks info (0 none, 1 LZMA,
                           2 LZ4, 3 LZ4HC); 0x40 blocks+directory combined;
                           0x80 blocks info at end of file; 0x200 pad before data
(format ≥ 7: align to 16)
blocks info (decompressed):
  16 bytes hash, i32 block count, per block {u32 uncompressed, u32 compressed, u16 flags},
  i32 node count, per node {i64 offset, i64 size, u32 flags, cstr path}
data: the blocks, each compressed per its own flags; decompressed they form
one stream that nodes are slices of. Node flag 4 = serialized file.
```

Observed in this game: header flags `0x43` (LZ4HC blocks info right after
the header). Block data is LZ4/LZ4HC; **no LZMA anywhere** (our reader
reports LZMA as unsupported, and `unity --all` found none).

## Serialized file — confirmed for version 21

Header is big-endian: `u32 metadata size, u32 file size, u32 version (21),
u32 data offset, u8 endianness (0 = little), 3 reserved`. Metadata follows
in the file's endianness (little in all files here):

```
cstr unity version, i32 platform (19 = StandaloneWindows64), u8 type trees enabled
i32 type count; per type: i32 class id, u8 stripped, i16 script type index,
    [16 bytes script id if class 114], 16 bytes type hash, [type tree if enabled]
i32 object count; per object: align 4 (absolute), i64 path id, u32 byte start
    (relative to data offset), u32 byte size, i32 type index
i32 script type count; per entry: i32 file index, align 4, i64 local id
i32 external count; per entry: cstr (empty), 16 bytes GUID, i32 type, cstr path
i32 ref type count (version ≥ 20); cstr user info
```

## Type trees are stripped — confirmed

**None of the 5,485 serialized files has type trees** (`typetree=0`, both in
the player's `.assets` files and in every bundle). Unity can't describe its
own object layouts here, so decoding object contents needs readers written
for Unity 2019.4's built-in class layouts (Texture2D, Mesh, Material,
GameObject, Transform, …). MonoBehaviour layouts depend on the game's
scripts, so they would have to come from `Managed/Assembly-CSharp.dll`
metadata (read at runtime, never copied). This affects M6/M7.

## Object census (whole game)

Most common classes: GameObject 98,852; Transform 96,017; MonoBehaviour 45,137;
BoxCollider 36,599; MeshRenderer and MeshFilter 29,778 each; MonoScript 24,457;
Mesh 16,019; Texture2D 5,631; AssetBundle 5,467; LODGroup 4,983; Material 4,960.

Notable bundles:
- `main-discrete_assets_worldmeshes_….bundle`: 6,016 Mesh objects (201 MB), with
  no other content. Probably the meshes of static world geometry — **hypothesis**.
- `main.unity_….bundle`: the main scene (`CAB-….sharedAssets` with 4,352 objects
  plus the scene file with 534).
- 1,718 `duplicateassetssorted_assets_bundle<N>` bundles: shared dependencies that
  Addressables split out.

Two class ids in `globalgamemanagers`/`main.unity` are not built-in classes
(937362698, 1403656975, 1542919678). They look like hashes, probably
script-defined or editor-only types — **hypothesis**.

## Object layouts we read — confirmed

Without type trees, these are hand-written for Unity 2019.4 and checked as
noted.

- **Texture2D** (class 28): `string m_Name, i32 forced fallback format, u8
  downscale fallback (align 4), i32 width, i32 height, i32 complete image size,
  i32 format, i32 mip count, u8 readable, u8 ignore master texture limit, u8
  streaming mipmaps (align 4), i32 streaming priority, i32 image count, i32
  dimension, texture settings {i32 filter, i32 aniso, f32 mip bias, i32 wrap
  U, V, W}, i32 lightmap format, i32 colour space, byte[] image data (align
  4), streaming info {u32 offset, u32 size, string path}`.
  Colour space: 1 = colour data stored sRGB, 0 = linear data (confirmed:
  every terrain albedo is 1, every terrain normal map 0; the water's
  displacement frames are 0).
  Checked: metadata of 1,062 textures in 43 files identical to UnityPy; decoded
  pixels (CRC-32 of RGBA) identical for 506 textures, including DXT1, DXT5,
  RGBA32, RGB24, Alpha8 and BC7. Alpha8 expands to (0, 0, 0, a), like the GPU
  and UnityPy.
- **Material** (class 21): `string name, PPtr shader, string keywords, u32
  lightmap flags, u8 + u8 (align 4), i32 custom render queue, map<string,string>
  tags, string[] disabled passes, saved properties {TexEnv[] (string name, PPtr
  texture, vec2 scale, vec2 offset), (string, f32)[] floats, (string, rgba)[]
  colours}`. Checked against UnityPy's reading of the terrain materials.
- **MonoScript** (class 115): `string name, i32 execution order, 16-byte hash,
  string class name, string namespace, string assembly`.
- **MonoBehaviour** (class 114) header: `PPtr game object, u8 enabled (align
  4), PPtr script, string name`, then the script's own fields.

Texture census (whole game): 5,631 textures; DXT5 3,651 (2.8 GB), DXT1 1,498
(705 MB), RGBA32 333, Alpha8 138, RGB24 10, BC7 1. **No crunch compression.**
5,611 keep their pixels in `.resS` files.

### Prefab and mesh classes (M7b) — confirmed

Layouts generated once from UnityPy's built-in Unity 2019.4.36f1 type
database (the files have no type trees), read by hand in
`crates/sn-unity/src/{prefab,mesh}.rs`. Every `vector` and `string` is
followed by alignment to 4; `map`s are not. Checked by decoding every mesh
used by a placed prefab (3,263) and comparing with UnityPy (below).

- **GameObject** (1): `PPtr[] components, u32 layer, string name, u16 tag,
  bool active`.
- **Transform** (4): `PPtr game object, quat local rotation (x, y, z, w),
  vec3 local position, vec3 local scale, PPtr[] children, PPtr father`.
  **RectTransform** (224) starts the same way and appears as a child in a
  few prefabs.
- **MeshFilter** (33): `PPtr game object, PPtr mesh`.
- **MeshRenderer** (23): `PPtr game object, bool enabled, u8 cast shadows,
  6 more u8 flags (align 4), u32 rendering layer mask, i32 priority, u16 + u16
  lightmap indices, 2 × vec4 lightmap tiling offsets, PPtr[] materials, …`
  (one material per sub-mesh; the rest is not read).
- **LODGroup** (205): `PPtr game object, vec3 reference point, f32 size, i32
  fade mode, bool + bool (align 4), {f32 screen-relative height, f32 fade
  width, PPtr[] renderers}[] LODs, bool enabled`.
- **AssetBundle** (142): `string name, PPtr[] preload table, map<string,
  {i32 preload index, i32 preload size, PPtr asset}> container, main asset,
  u32 runtime compatibility, string bundle name, string[] dependencies, …`.
  Container paths are the build-time paths, e.g.
  `Assets/AddressableResources/WorldEntities/…/X.prefab`.
- **Mesh** (43): `string name, SubMesh[] (u32 first byte, u32 index count,
  i32 topology, u32 base vertex, u32 first vertex, u32 vertex count, AABB),
  blend shapes {vertex[40 B], shape[12 B], channel {string, u32, i32, i32}[],
  f32[]}, mat4[] bind poses, u32[] bone hashes, u32 root bone hash, AABB[] bone
  bounds, u32[] variable bone weights, u8 compression, 3 × bool (align 4), i32
  index format, u8[] index buffer, vertex data {u32 vertex count, channel {u8
  stream, u8 offset, u8 format, u8 dimension}[], u8[] data}, compressed mesh
  {10 packed bit vectors, u32 UV info}, AABB, i32 usage flags, u8[] baked
  convex collision, u8[] baked triangle collision, 2 × f32 metrics (align 4),
  streaming info {u32 offset, u32 size, string path}`.

What the game's meshes use (all 3,263 placed ones; an earlier UnityPy census
of 2,115 meshes in 400 random bundles found the same formats, and no
compressed mesh):

- 16-bit indices, triangle lists only.
- Vertex data mostly in the bundle's `.resS` resource (streaming info), a few
  inline. Channels (index: position, normal, tangent, colour, UV0–7, blend
  weights, blend indices) are interleaved per stream; streams follow one
  another, each starting on a 16-byte boundary. Typical: float32 positions,
  float16 normals/tangents (4 components; the dimension byte's high bits
  are flags, low 4 bits the count), unorm8 colours, float16 or float32 UVs.
- **52** meshes (Aurora wreck rooms) use Unity's mesh compression: bit-packed
  vectors (values of `bit size` bits, least significant bit first; floats map
  linearly onto `start`..`start + range`), normals and tangents as x, y plus
  a sign bit for z (tangents also for w), UV sets described by `UV info`
  (4 bits per set: present, dimension − 1), indices as packed integers.

**Oracle check:** `sn-inspect prefab --placed --oracle` writes per mesh its
name, vertex count, triangles per sub-mesh and the sum of positions; a
throwaway UnityPy script wrote the same for the same objects. All 3,263
agree on names, vertex and triangle counts; position sums differ by at most
1.29 on sums up to 2.6 × 10⁷ (float summation order).

### Prefabs — confirmed

A placed object's prefab path (from `prefabs.db`) is a key of the
Addressables catalog. Its `UnityEngine.GameObject` entry has the build-time
path as internal id; the entry's dependencies are bundles, and the **first
is the prefab's own bundle** (all 1,369 placed prefabs resolve this way).
The bundle's AssetBundle container maps the build-time path (compared without
case) to the root GameObject; children follow the Transforms. Materials
usually live in shared `duplicateassetssorted_…` bundles, reached through the
serialized file's externals. Unity's built-in meshes (`Cube`, `Sphere`, …) are
in `Subnautica_Data/Resources/unity default resources` (externals name it
`Library/unity default resources`).

**The placement replaces the prefab root's transform** — confirmed on
`Spiral_blue_thing_cluster_07`: its root has a −90° X rotation and scale
0.0001 (the model is authored in tenths of a millimetre), and its saved
placements in `CellsCache` all have scale 0.0001 and their own rotations.
So: world = placement × (nodes below the root).

Of the 1,369 placed prefabs, 324 have LOD groups and 457 draw nothing
(cell/batch roots, atmosphere volumes, spawn slots, lights).

### Addressables catalog — confirmed

`StreamingAssets/aa/catalog.json` (12 MB, parsed in about 50 ms by our own
JSON reader in `crates/sn-unity/src/addressables.rs`): string lists
`m_InternalIds` (21,100), `m_ProviderIds`, `m_resourceTypes` (`m_ClassName`),
and three base64 tables:

- keys: `i32` count, then per key a type byte and its value (0: `i32` length +
  ASCII, 1: `i32` byte length + UTF-16LE; other types not needed);
- buckets, one per key: `i32` key offset into the key table, `i32` entry count,
  that many `i32` entry indices;
- entries: `i32` count, 7 `i32` each — internal id, provider, dependency key
  (−1: none), dependency hash, extra data offset, primary key, resource type.

38,483 string keys, 22,406 entries.

### Scenes (M7f1) — confirmed

A scene is a bundle `<name>.unity_<hash>.bundle` (13 in this build: `main`,
`essentials`, `aurora`, `escapepod`, `cyclops`, `rocketspace`, menus,
credits, cleaners). It holds two serialized files: the scene itself (the
only one with a `RenderSettings`, class 104, and `LightmapSettings`) and
`….sharedAssets` (meshes, materials, textures, prefabs the scene's scripts
reference). Other assets come from other bundles through the externals, as
for prefabs. Built scenes have no prefab instances left: every object is a
GameObject of the scene file. A **top-level object** is one whose Transform
has a null `father`; its Transform is its world placement (confirmed: the
Aurora lines up with the terrain and the water line at y = 0, screenshot
`out/m7f1-aurora-far.png`).

**Which scenes the game loads** (read with `sn-inspect scene <name>
--script <Class>` and the game's code):
- `main` has one `MainGameController`; its `additionalScenes`
  (`string[]`, the first field) is `["Essentials"]`, loaded additively.
- `essentials` has one `LightmappedPrefabs`; its `autoloadScenes` (first
  field, array of `{string sceneName; bool spawnOnStart}`, the bool padded
  to 4 bytes) is `Cyclops` (false), `EscapePod` (true), `Aurora` (true).
- When such a scene has loaded, `LightmappedPrefabs` takes its object named
  `__LIGHTMAPPED_PREFAB__`, deactivates it and keeps it as a template; with
  `spawnOnStart` it unparents it, puts it at the origin and activates it
  (`ActivateLoadedPrefab`). The scene's other top-level objects stay where
  they are, active as stored.

**Aurora scene** (`aurora`): 3,189 GameObjects, 437 mesh renderers, 291 LOD
groups, 361 MonoBehaviours (316 `CullingOccludee`), no lights. Six top-level
objects: `__LIGHTMAPPED_PREFAB__` (the ship, 1,838 nodes),
`StopwatchProfiler`, and four non-streaming parts of the world:
`Precursor_Prison_Interior_Aquarium_NonStreaming`,
`Precursor_Prison_exterior_NonStreaming`,
`Precursor_LostRiverBase_NonStreaming`, `LostRiver_LargeTrees_NonStreaming`.
`CrashedShipExploder` fields in order: `crashedShipPrefab` (PPtr),
`disableOnExplosion` (PPtr[]), `enableOnExplosion` (PPtr[]),
`explodedExterior` (PPtr), then non-PPtr fields we don't read. Its lists
have 2 objects each; `SwapModels(exploded)` sets the first list active =
!exploded and the second = exploded. Before the explosion (a new game) 330
nodes are drawn at full detail (the hull `starship_crashed` and 10
`CrashedShip_Damaged_Exterior_Quad`s among them); after it 337
(`starship_exploded_02`, 18 meshes). The explosion time is
`timeToStartCountdown = start + Random.Range(2.3, 4) × 1200 s`, swap 27 s
later (game code, not from data).

**Escape pod scene** (`escapepod`): 248 GameObjects, one top-level object
(`__LIGHTMAPPED_PREFAB__` → `EscapePod`), 35 `SkinnedMeshRenderer`s (the
hull `Life_Pod_damaged_03` is skinned), 5 lights, 7
`AddressablesPrefabSpawn`.

**Lifepod placement (M7f3)** — confirmed from data and code:
- `RandomStart` sits on the `essentials` scene's spawner prefab (its
  `.sharedAssets`); its first field `validStartPointTexture` is the
  Texture2D `RandomStart` (512 × 512, DXT1, embedded). 0.59 % of its
  pixels have green > 0.5. `IsStartPointValid(p)`: pixel
  (`(int)(clamp01((x + 2048) / 4096) × width)`, same with z for the row),
  rows counted from the bottom (`GetPixel`). `GetRandomStartPoint`: up to
  1,000 draws of x, z with `Random.Range(−2048, 2048)`, y = 0; else the
  origin.
- `EscapePod` (on the `EscapePod` child of `__LIGHTMAPPED_PREFAB__`):
  first fields `bottomHatchEntrance` (GameObject), `playerSpawn`
  (Transform). `StartAtPosition` sets the pod's world position to the
  point (its anchor for floating) and puts the player at `playerSpawn`:
  about 1 m from the pod's axis, 2.1 m above the water line.
- 6 `MoveAndRotateWithTransform` (first field `target`, a Transform): each
  frame the object takes the target's world position and rotation. The
  module roots (`ModulesRoot/…Root`) follow mount points this way; with the
  stored pose they sit 1.5–2.4 m up, within 3 m of the axis.
- `PrefabSpawnBase` fields in order: `spawnType` (`SpawnType`: 0 OnStart, 1
  Intermittent, 2 OnAwake, 3 OnNewBorn, 4 Manual), `intermittentSpawnTime`,
  `inheritLayer`, `usePrefabTransformAsLocal`, `useCurrentTransformAsLocal`,
  `keepScale` (bools padded to 4 bytes), `attachToParent` (Transform),
  `spawnAtHealthPercent`, `useSpawnAtHealth`, `spawnedObj`,
  `sendObjectMessageOnSpawn` (string), `deactivateOnSpawn`; then
  `PrefabSpawn.prefab` (GameObject PPtr) or
  `AddressablesPrefabSpawn.prefab` (`AssetReference`: GUID string, sub-object
  name, sub-object type; the GUID is a catalog key). The spawned object is
  instantiated under `attachToParent` (or the spawner) and reset to local
  position 0 and no rotation, keeping the prefab's scale (`keepScale`), unless
  one of the two "as local" flags is set.
- In the escape pod: 6 `AddressablesPrefabSpawn` with OnNewBorn (medical
  cabinet, fabricator, radio, 3 power cells), 1 `PrefabSpawn` OnAwake
  (`UISpawn`, the pod's screen UI, nothing drawn), 2 `PrefabSpawn` Manual
  (triggered by the intro: damage effects, birds; not drawn).

### Skinned meshes (M7f2) — confirmed

`SkinnedMeshRenderer` (class 137): the `Renderer` fields as in
`MeshRenderer` up to `m_Materials`, then static batch info (2 × `u16`),
three PPtrs (static batch root, probe anchor, light probe volume override),
sorting layer id (`i32`), sorting layer and order (2 × `i16`), quality
(`i32`), update-when-offscreen and skinned-motion-vectors (2 bytes, padded
to 4), `m_Mesh` (PPtr), `m_Bones` (PPtr[] to Transforms),
`m_BlendShapeWeights` (`f32[]`), `m_RootBone` (PPtr), `m_AABB` (centre,
extent), `m_DirtyAABB` (byte). Confirmed on the game's data: every mesh
and bone resolves (0 bones outside their hierarchy over 73 placed prefabs
and the escape pod), and bones match bind poses in number except once.

`Mesh.m_BindPose`: one 4×4 matrix per bone, **stored row by row**
(`e00 e01 e02 e03 e10 …`) — confirmed: read that way, all 269 checked
matrices have the bottom row 0 0 0 1; read column by column, none do.
Bone weights and indices are vertex channels 12 (`BlendWeight`, float32,
1–4 components) and 13 (`BlendIndices`, uint32), in the vertex streams
like the other channels. With a single bone per vertex the weight channel
is absent (weight 1).

Skinned position = Σ wᵢ · boneᵢ.localToWorld · bindPoseᵢ · v (the
renderer's own Transform is not used). Confirmed on the three prefabs that
have a skinned LOD 0 and a static LOD 1 (`AbandonedBaseFloatingIsland1–3`):
with the bones where the hierarchy stores them, the skinned LOD 0's bounds
equal the static LOD 1's within 0.1 % (centres within 1 cm;
`sn-inspect prefab --skinned`).

Renderers with **no bones** (113 of 304 in placed prefabs, e.g. all of
`BrainCoral`'s LOD 0, which use blend shapes) are drawn by Unity as the
plain mesh at the renderer's Transform; we do the same.

## Not yet read

- Blend shapes (read past, not applied), creature animation.
- `StreamingAssets/AssetBundles/` (`logos`, `waterdisplacement`): legacy bundles,
  not yet looked at.
