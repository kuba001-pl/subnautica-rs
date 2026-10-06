# Terrain materials: octree type id → textures

Described build: game build `10`. Code: `crates/sn-unity` (readers),
`crates/sn-assets` (`terrain_materials`), shader `apps/sn-client/src/terrain.wgsl`.
Inspector: `sn-inspect terrain-materials`.

Each fact is marked **confirmed** (with how) or **hypothesis**.

## The chain — confirmed

```
octree node type id (u8, 1–255; 0 = empty)
  → block type (VoxelandBlockType: grass settings, layer, filled, material, …)
      from the main scene's `Voxeland` component (`types[id]`)
      and/or a `VoxelandBlockTypePrefab` whose `globalId` is id
  → Material (a terrain shader material)
  → Texture2D objects (colour and normal maps; DXT1/DXT5)
```

Confirmed by `sn-inspect terrain-materials`: **every one of the 209 non-empty
type ids that occur in the octrees has a material**, and 24 more defined
types never occur.

## Where block types are defined

1. **The main scene** (`main.unity_….bundle`, scene file `CAB-b6f2…`): one
   MonoBehaviour whose script is `Voxeland` (found via its `MonoScript`, class
   name `Voxeland`). Its `types` array has 255 entries; **56** are filled.
   Entry 0 is unfilled (empty space).
2. **Block-type prefabs** in the player's resources (`resources.assets`,
   listed by the ResourceManager under `blockprefabs/<biome>/<name>`, e.g.
   `blockprefabs/castle/gp_sandtorock`): MonoBehaviours with script
   `VoxelandBlockTypePrefab` holding one `VoxelandBlockType` plus
   `globalId` (u8): **233** with an id, **10** with `globalId` 0.

`meta.txt` next to the octrees says `BlockPrefabs`, and the scene's
`Voxeland.paletteResourceDir` is empty. **Hypothesis**: the game sets the
palette folder from `meta.txt` and loads the prefabs over the scene table.
We let prefabs win; ids 53, 54, 101 and 102 are the only ones where scene and
prefab name different materials. Prefabs with `globalId` 0 are skipped
(type 0 is empty space; **hypothesis**: they are unassigned).

## Field layouts — confirmed

The files have no type trees. The layouts below were generated *once, on
the dev machine,* from the field declarations in the game's own
`Managed/Assembly-CSharp-firstpass.dll` (read in place with UnityPy's
type-tree generator; nothing copied into the repo). Our readers are written by
hand from them, and every value decodes sensibly.

All MonoBehaviours start with: `PPtr m_GameObject, u8 m_Enabled (align 4),
PPtr m_Script, string m_Name`. A PPtr is `i32 file id, i64 path id`; strings
are `i32 length, bytes, align 4`; every `u8`/bool below is followed by
alignment to 4.

`VoxelandBlockType` (100 bytes): `f32 grassDensity, u8 grassZUp, f32
grassJitter, f32 grassMinScale, f32 grassMaxScale, i32 grassMinTilt, i32
grassMaxTilt, u8 grassRandomSpin, u8 perlinGrass, f32 perlinPeriod, i32 layer,
u8 filled, PPtr<Material> material, PPtr<VoxelandDecoType> decoOverride, u8
hasGrassAbove, PPtr<Mesh> grassMesh, PPtr<Material> grassMaterial`.

`Voxeland` (after the header): `PPtr<VoxelandData> data, u8 localAO, u8
castShadows, u8 scaleToolToEdit, PPtr<Material> opaqueMaterial, string
paletteResourceDir, VoxelandBlockType[] types, i32 selected, i32 chunkSize, i32
newChunkSize, i32 numChunksBuilt, 5 debug bools, f32 surfaceDensityValue, …`
(more debug fields follow; not read). `opaqueMaterial` is `TerrainOpaquePass`.
`surfaceDensityValue` is 0 in the scene: the density threshold is not stored
here (we use 125.5, see optoctrees.md).

`VoxelandBlockTypePrefab` (after the header): `VoxelandBlockType blockType,
u8 globalId`.

## Terrain material properties — confirmed

Two kinds of materials, both on the same terrain shader (shader references
are not followed):

| Kind | Count | Cap (upward faces) | Side (slopes) |
|---|---|---|---|
| Plain | 133 types | `_MainTex`, `_BumpMap`, `_TriplanarScale`, `_Color` | same as cap |
| Cap/side blend (e.g. `Sand01ToRock05`) | 100 types | `_CapTexture`, `_CapBumpMap`, `_CapScale`, `_CapColor` | `_SideTexture`, `_SideBumpMap`, `_SideScale`, `_Color` |

Scales are texture repeats per metre (0.05–1.5; most 0.1–0.15). Optional extras
we don't use yet: `_SIGMap` (specular/illumination/gloss), `_Mask`,
`_BorderBlend*` / `_CapBorderBlend*` (soft transitions between materials),
`_EmissionScale`, `_Gloss`. Shader keyword `UWE_SIG` marks SIG-map materials.

Textures: 183 distinct, DXT5 (format 12) or DXT1 (10), 128–1024 px, full
mip chains. Normal maps are treated as Unity "DXT5nm" (X in alpha, Y in
green) — **hypothesis** based on Unity's usual encoding for DXT5 normal maps;
the lit result looks plausible, but no other decoding was compared.

## How we render it

The client uploads the DXT data as is (BC1/BC3, all mip levels) and uses a
triplanar shader: cap textures on the Y projection, side textures on X and Z,
weights from the surface normal (power 4), normal maps combined with the
"whiteout" blend. See `apps/sn-client/src/terrain.wgsl`.

**Not done**: soft blending between neighbouring block types. Each triangle
gets exactly one material (the solid voxel on its edge), so borders follow the
voxel grid and look blocky, especially for patchy materials like red grass.
