# Terrain materials: octree type id → textures

Described build: game build `10`. Code: `crates/sn-unity` (readers),
`crates/sn-assets` (`terrain_materials`), layers `crates/sn-mesh/src/layers.rs`,
shader `apps/sn-client/src/terrain.wgsl`.
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

Three shaders (confirmed by following each material's shader reference):

| Kind | Types | Cap (upward faces) | Side (slopes) |
|---|---|---|---|
| Plain | 114 | `_MainTex`, `_BumpMap`, `_SIGMap`, `_TriplanarScale`, `_Color`, `_SpecColor`, `_EmissionScale` | same as cap |
| Cap/side (e.g. `Sand01ToRock05`) | 119 | `_CapTexture`, `_CapBumpMap`, `_CapSIGMap`, `_CapScale`, `_CapColor`, `_CapSpecColor`, `_CapEmissionScale` | `_SideTexture`, `_SideBumpMap`, `_SideSIGMap`, `_SideScale`, `_Color`, `_SpecColor`, `_SideEmissionScale` |

One plain-type material (`Lava09`, type 31) uses a third, animated lava
shader (`_FlowTex`, `_FlowMap`, `_NoiseMap`, …); we draw it as a plain
material (**not** its flow animation).

Shared by both main shaders (231 of 233 materials; `sn-inspect
terrain-materials --props` lists them all):

- `_TriplanarBlendRange` (217 × 2.0; up to 40), `_Gloss` (0.07–0.93).
- `_BorderBlendRange`, `_BorderBlendOffset` (0–1, never negative),
  `_InnerBorderBlendRange`, `_InnerBorderBlendOffset` (210 × 1.0), `_BorderTint`.
- Cap/side only: `_CapBorderBlendRange`, `_CapBorderBlendOffset` (≤ 0),
  `_CapBorderBlendAngle` (1–3.55).
- Render state, identical on all 231: `_BlendSrcFactor` 5 (SrcAlpha),
  `_BlendDstFactor` 10 (OneMinusSrcAlpha), `_ZWrite` 0, `_ColorMask` 14 (RGB),
  `_IsOpaque` 0, `_AlphaTestValue` 0. So every terrain material is drawn
  **alpha-blended** without writing depth.

The shaders' property labels (stored in the shader object) name the albedo
alpha "Splotch" (`Base (RGB) Splotch(A)`) and the SIG map channels
`Spec(R) Illum(G) map *Gloss/B ignored*`. Shader keyword `UWE_SIG` (50
materials) turns the SIG maps on; 28 SIG textures exist.

Scales are texture repeats per metre (0.05–1.5; most 0.1–0.15).
Textures: 211 distinct (183 colour/normal + 28 SIG), DXT5 (format 12) or
DXT1 (10), 128–1024 px, full mip chains.

The project uses Unity's **linear** colour space (`PlayerSettings.
m_ActiveColorSpace` = 1, read once with UnityPy on the dev machine). Colour
textures are therefore sampled as sRGB, and material colours (stored as sRGB)
are converted to linear before the shader sees them.

## How the game draws terrain — confirmed

Sources: the game's compiled terrain shaders (Direct3D 11 bytecode inside
the `Shader` objects, LZ4-compressed; disassembled once on the dev machine
with Windows' `d3dcompiler_47.dll`, constant-buffer offsets from the
subprogram's binding table), and the chunk-building code in the game's
`Assembly-CSharp.dll` (read, not copied). Nothing from either is in the
repository; below is our own description.

### Layers per chunk

- The world is meshed in chunks of `Voxeland.chunkSize` = **16** voxels per
  side (value read from the main scene); at coarser levels a chunk covers 16
  coarser cells.
- Each chunk lists the block types its faces use and sorts them by
  `VoxelandBlockType.layer`, then by type id (**not** by how much of the
  chunk they cover).
- The chunk is drawn once per type in that order. The first draw covers
  every face of the chunk with the first type's material **plus** the
  `TerrainOpaquePass` material, which writes depth and the gloss but no
  colour (its pixel shader outputs zeros and `uv.y` into the specular
  target's alpha). Every later draw covers only faces that have at least one
  vertex with a non-zero weight for its type, and is blended on top.
- A vertex's weight for a type is the share of its adjacent faces of that
  type. Its gloss is the average of its faces' block-type gloss
  (**hypothesis**: the block type's cached gloss is the material's `_Gloss`).
  Both go to the mesh as `uv0 = (weight, gloss)`. The first draw uses weight 1.
- Each level of the terrain clipmap caps the types per chunk
  (`maxBlockTypes` in `StreamingAssets/SNUnmanagedData/clipmaps-*.json`;
  "high" preset: levels 0–4 allow 32, 8, 2, 1, 1). Over the cap, the chunk
  keeps the types with the most faces, then sorts those by layer; faces of
  dropped types only get the first draw. The same files confirm
  `chunkMeshRes` 16 at run time, and that only level 0 of "high" uses the
  9-vertex mesh (`useLowMesh` false).
- Near the camera ("hi-res" chunks) every face has 9 vertices — corners,
  edge midpoints, centre — and 8 triangles; a centre vertex touches only its
  own face, so a lone face of one type still reaches weight 1 in its middle.
  Far chunks use only the 4 corners.

### The pixel shader (per draw)

All in Unity world space, `p` = position, `n` = unit normal.

1. **Projection weights**: `w = (1.96·n²)^_TriplanarBlendRange`, normalised to
   sum 1. Projections use texture coordinates X: `(p.y, p.z)`, Y: `(p.x, p.z)`,
   Z: `(p.y, p.x)`, times the material's scale.
2. **Plain**: albedo (with alpha) = triplanar mix of `_MainTex`; colour =
   albedo × `_Color`.
3. **Cap/side**: `cap` = `_CapTexture` on the Y projection only. Slope factor
   `s = saturate((clamp(1 − (n.y + _CapBorderBlendOffset)·_CapBorderBlendAngle
   − cap.a, −1, 1) + _CapBorderBlendRange) / (2·_CapBorderBlendRange))` (0 = cap,
   1 = side; downward faces get side). Colour (including alpha) =
   `lerp(cap × _CapColor, triplanar(_SideTexture) × _Color, s)`; normal =
   `normalize(lerp(capNormal, sideNormal, s))`.
4. **Normal maps**: tangent-space x = `a·r`, y = `g` (×2 − 1), z from length.
   Each projection has its own frame, with `σ` the sign of the normal's
   component on that axis: X: tangent `σ(n.y, n.x, −n.z)`, bitangent
   `σ(−n.z, n.y, n.x)`; Y: `σ(n.y, −n.x, n.z)`, `σ(n.x, −n.z, n.y)`;
   Z: `σ(n.x, n.z, n.y)`, `σ(n.z, n.y, n.x)`; the z component goes along `n`.
5. **Borders**: with `t(R, O) = (R + 1)/(1.01 − O) · (1 − weight − O) − R`,
   `alpha = saturate((colour.a − t(_BorderBlendRange, _BorderBlendOffset)) /
   _BorderBlendRange)`, and the colour is pulled towards `_BorderTint` by
   `1 − saturate((colour.a − t(_InnerBorderBlendRange, _InnerBorderBlendOffset))
   / _InnerBorderBlendRange)`. At weight 1 the alpha is always 1 (offsets are
   never negative), so the first draw is opaque. The splotch alpha makes the
   borders ragged.
6. **SIG** (`UWE_SIG`): the SIG map is projected like the albedo (cap/side:
   blended by `s`, with each side's emission scale). Specular colour =
   `SIG.r × specular colour` (without SIG: `albedo.r × specular colour`);
   emission = colour × `SIG.g` × emission scale.

## How we render it

`sn_mesh::build_layers` rebuilds the layers from our surface-nets mesh: a
surface-nets quad is one face, it belongs to the chunk holding the solid cell
behind it, and faces are split into 9 vertices at level of detail 0 only.
Types per chunk are capped at 32, 2, 1, 1 for our levels 0–3 (the "high"
preset's caps for levels with the same sample spacing; mapping our levels to
the game's clipmap levels is our choice, not the game's).
Layers with the same type and rank in a batch become one mesh.

The client (`apps/sn-client/src/terrain.wgsl`) ports steps 1–6 into Bevy's PBR
lighting: rank 0 is opaque, later ranks are alpha-blended in rank order
(Bevy sorts blended meshes by distance plus depth bias; every mesh of a batch
gets the same bounding box, and the bias is the rank). Gloss becomes
`perceptual_roughness = 1 − gloss`; emission is scaled so that 1.0 equals a
white surface under our sun.

**Not ported**: the specular colour (the game's deferred lighting is custom,
and we have not checked how it reads the specular target; Bevy's default
reflectance is used), the lava flow shader, and the game's lighting, fog and
caustics (M8). DXT1 textures have alpha 1, which reads as full splotch.
