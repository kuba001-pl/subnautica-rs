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

## Not yet read

- `StreamingAssets/aa/catalog.json` (Addressables catalog: asset name → bundle).
  Moved to M7, where world objects need it.
- `StreamingAssets/AssetBundles/` (`logos`, `waterdisplacement`): legacy bundles,
  not yet looked at.
