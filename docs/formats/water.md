# Water: biomes and per-biome water settings

Described build: game build `10`, world data `SNUnmanagedData/Build18`.
Code: `crates/sn-world/src/biomes.rs` (map, names, batch overrides),
`crates/sn-unity/src/water.rs` (`WaterBiomeManager`), `crates/sn-assets`
(`water_biomes`). Inspector: `sn-inspect biomes`, `sn-inspect biomes --at X Y Z`.

Behaviour notes come from reading the game's classes (`LargeWorld`,
`BiomeMapData`, `WaterBiomeManager`, `WaterscapeVolume`,
`LargeWorldBatchRoot` in `Assembly-CSharp.dll`, decompiled once on the dev
machine into the gitignored `out/` folder; nothing copied). Each fact is
marked **confirmed** (with how) or **hypothesis**.

## Biome map — confirmed

`Build18/biomeMap.bin`: one byte per cell, an index into the rows of
`Build18/biomes.csv` (after its `name` header), row by row; the file's
**last two bytes** are width / 64 and height / 64. Build 10: 1024 × 1024
cells (+ 2 bytes = 1,048,578), 19 names. The game divides voxel x and z by
`land width / map width` (integer division; 4160 / 1024 = 4), so one cell
is 4 × 4 voxels: cell = (z / 4) × width + x / 4.

16 indices occur; the most common is 11, `void` (34 %, the open ocean
around the world). Confirmed by `sn-inspect biomes`; the lifepod (0, −10, 0)
is in `safeShallows`, the crater edge (1700, −80, 0) in `crashZone`.

## Batch override biome — confirmed

The biome at a voxel is its batch's `LargeWorldBatchRoot.overrideBiome`
(protobuf member 2) if set, else the map's. 467 of 2,975 batches set it
(Lost River, lava zones, Jellyshroom caves, …), which is how the 2D map
gets depth.

Other `LargeWorldBatchRoot` members: 3 fog colour (RGBA), 4 fog start
distance, 5 fog max distance, 6 fade default lights, 7 fade rate, 8 fog
settings, 9 sunlight settings, 10 ambient light settings, 11 version, 12
atmosphere prefab ClassId. Only 2–5 are read so far.

## Water settings — confirmed

The main scene has one `WaterBiomeManager` MonoBehaviour. Serialized fields
after the header: 6 shader references, a `LargeWorld` reference,
`biomeSettings` (list), `i32 settingsTextureSize` (8), `i32
settingsTextureUpsampledSize` (32), `f32 regionBounds` (**64**, not the
class default 50), `bool enableBlur`, then debug flags.

Each `biomeSettings` entry: `string name`, `WaterscapeVolume.Settings`, a sky
prefab reference. Settings: `vec3 absorption` (tooltip: attenuation of
light, 1/cm), `f32 scattering`, `Color scatteringColor`, `f32 murkiness`
(0–20), `Color emissive`, `f32 emissiveScale`, `f32 startDistance`, `f32
sunlightScale`, `f32 ambientScale`, `f32 temperature` (°C).

**145** entries in build 10; every biome in the map and in the batch
overrides has one (names compared without case), except `void` and
`EmperorFacility`. The game then uses `WaterscapeVolume`'s default settings
(absorption (100, 18.29, 3.53), scattering 1, murkiness 1, start 25 m, …).

Examples: `safeShallows` absorption (125, 20, 4), scattering 1.2, murkiness
0.18, ambient ×1.5, 28 °C; `kelpForest` (80, 25, 30), murkiness 0.16;
`grandReef` (16, 12, 6), murkiness 0.25, ambient ×20; lava zones 60–80 °C.

## How the game uses them — confirmed from the class, shader not yet read

- Extinction and scattering coefficients (`GetExtinctionAndScatteringCoefficients`):
  `(absorption + scattering, scattering) × murkiness / 100`; emissive:
  `linear(emissive) × emissiveScale / 100`.
- `WaterBiomeManager` keeps a volume texture around the camera:
  `settingsTextureSize`³ cells over ±`regionBounds` metres (8³ over ±64 m:
  16 m cells), each holding the settings of the biome at its position,
  blurred and upsampled to 32³, exposed to shaders as
  `_UweExtinctionTexture`, `_UweScatteringTexture`, `_UweEmissiveTexture`.
- How shaders turn these into colour (in-scattering, sun and ambient
  scales, start distance) is in the compiled water shaders; to be read for
  M8b.
