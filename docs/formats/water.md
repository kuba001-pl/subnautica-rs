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

## Volume textures — confirmed from the class

`WaterBiomeManager` keeps a volume around the camera:
`settingsTextureSize`³ cells over ±`regionBounds` metres (8³ over ±64 m:
16 m cells). Each cell gets the biome at its position (plus local
`AtmosphereVolume` shapes, e.g. caves, rasterised on top — not read yet),
then the volume is blurred and upsampled to 32³ (stored as a 2D atlas of
slices) into three textures:

| Texture | xyz | w |
|---|---|---|
| `_UweExtinctionTexture` | (absorption + scattering) × murkiness / 100 | start distance |
| `_UweScatteringTexture` | linear(scattering colour) × scattering × murkiness / 100 | sunlight scale × water transmission |
| `_UweEmissiveTexture` | linear(emissive) × emissive scale / 100 | ambient scale |

## The scene's `WaterscapeVolume` — confirmed

Read from the main scene (layout from the class; the fields end exactly at
the object's end): water transmission **0.7**, emission ambient scale 1,
above-water start distance 5, scattering phase −0.3, sun attenuation
**0.25**, sun light amount **100**, colour cast distance factor 0.1, depth
factor 0.0002, caustics scale 2, amount 0.1, above-water density scale 10
between heights −0.5 and 2. (Several differ from the class defaults.)

## The underwater fog — confirmed from the compiled shader

A full-screen pass over the lit image (`WaterscapeVolume.RenderImage`, a
blit with the scene's fog shader), disassembled once on the dev machine
(Direct3D 11 bytecode; constant offsets from the subprogram's binding
table). Per pixel, in view space:

1. Distance `D` to the surface from the depth buffer; ray direction `v`.
   Sky pixels (depth 1) are recognised.
2. Water settings are read **once per pixel**, at `settingsSampleDistance`
   along the ray, from the volume textures: extinction σₜ, start distance
   `s`, in-scattering σₛ (colour), light scale `k`, emissive `e`.
3. Water plane `n·p + d = 0` (view space). Camera **above** water (`d > 0`):
   rays not going down, or hitting geometry before the water, get only "sky
   fog": `lerp(skyFogColour, C, exp(−D × skyFogDensity))`; otherwise water
   fog starts at the surface (distance along the ray + above-water start
   distance). Camera **below**: rays reaching the surface see the colour
   behind it sky-fogged, and travel `min(D, distance to surface)` in water.
4. With `t = D_water − s` (only if positive) and all coefficients times the
   global extinction scale:
   - transmittance `T = exp(−σₜ t)`;
   - Henyey–Greenstein phase `P = c₀ (c₁ − c₂ cos θ)^(−1.5)` with
     `c = ((1 − g²) / 4π, 1 + g², 2g)`, g = −0.3, θ between `v` and the sun;
   - sunlight below the surface is attenuated along its path to the surface:
     the exponent changes along the ray at rate
     `k_sun = −σₜ + (v·n) σₜ a / max(L·n, 10⁻⁴)` (a = sun attenuation 0.25),
     starting from `h σₜ a / max(L·n, 10⁻⁴)` at the fog start point, `h` its
     signed height above the water plane;
   - in-scattered light `∫₀ᵗ exp(k_sun x + start) dx · P σₛ · (sun × amount +
     top ambient) × k` (closed form; `t·exp(…)` when `k_sun` = 0);
   - emissive `e (1 − T) / σₜ`;
   - result `C T + in-scattering + emissive`.
5. Pixels whose G-buffer normal alpha marks them (fractional part of
   1.5 × alpha < 0.25), or with `_CameraInside` set, are blended back
   towards the unfogged colour; `_SpaceTransition` too.

`settingsSampleDistance` is not a shader property and nothing in the
game's code sets it, so it stays 0 (**hypothesis**, strong): the settings
are read at the camera, the same for the whole frame.

Where the light values come from (from the classes): the sun colour is
`sun light colour (linear) × intensity`, both set by `uSkyLight` from a
time-of-day gradient, the exposure and day/night factors; the top ambient
colour is `sky colour (linear) × indirect light fraction × ambientLight`
(`uSkyLight`); sky fog density and colour come from `uSkyManager`
(`skyFogDensity`, a gradient over the day). The sun direction is a rotation
from the time of day (`Timeline`), `SunDirection` and `NorthPoleOffset`.

## Sky — confirmed

Read from the main scene's `uSkyManager` and `uSkyLight` MonoBehaviours.
Their layouts were generated once with UnityPy's type-tree generator from
the game's own assembly (dev machine, `out/`), our readers
(`crates/sn-unity/src/sky.rs`) are written by hand from them, and the values
match UnityPy's reading exactly (real-data test `sky_system_values`).

- `uSkyLight` (716 bytes = header + 3 floats + 4 Unity `Gradient`s of 168
  bytes): sun intensity 1.37, sun colour gradient (7 keys: grey-blue night,
  orange at sunrise/sunset, near-white at noon), moon intensity 0.5, sky /
  equator / ground colour gradients, ambient light 0.35.
- `uSkyManager` (fields up to the sky fog): timeline 8.2 h (editor value),
  sun direction −141°, max sun angle 65°, north pole offset 0, exposure
  0.66, Rayleigh/Mie 1, wavelengths (680, 550, 440) nm, sky tint 0.5, sky
  fog density 0.0002, sky fog colour gradient (4 keys).
- A Unity `Gradient` (2019): 8 RGBA keys, 8 colour-key times and 8 alpha-key
  times as u16 (/ 65535), mode (0 blend, 1 fixed), key counts, align 4.

How the game turns them into light (classes `uSkyManager`, `uSkyLight`,
`DayNightCycle`):
- A new game starts at clock 09:36. The sky's timeline maps the clock's day
  (sunrise 0.125 → sunset 0.875 of the day) onto 6 h → 18 h: 09:36 → 10.4 h.
- Sun rotation: `Euler(0, sunDirection, northPoleOffset) · Euler(a + 90°, 0, 0)`
  with `a` going from −max angle at 6 h to +max angle at 18 h (and towards 0
  at night): the sun is at the zenith at 12 h and 25° up at 6 h and 18 h.
- Day/night factors from the sun's height (`uMuS`); sun light =
  `exposure × (sunIntensity × day + moonIntensity × night)` × the sun-colour
  gradient (linearised). Top ambient = sky-colour gradient through a small
  Rayleigh colour offset (from the wavelengths; our port reproduces the
  class's reference values 5.81, 13.57, 33.13) × exposure, linearised, ×
  ambient light. Sky fog colour = its gradient, linearised.
- Ignored: eclipses (the planet in front of the sun; the fog density is
  multiplied by 1 − eclipse).

## How we render it (M8b)

`apps/sn-client/src/water.rs` and `water_fog.wgsl`: an HDR camera and a
full-screen pass before tone mapping implementing steps 1–4 above in world
space (water plane y = `waterOffset` = 0). The CPU blends the coefficients of
the 8 surrounding 16 m cell centres at the camera every frame (the game
blurs and upsamples its volume; our blend is simpler). `apps/sn-client/src/sky.rs`
computes the sky state for `--time` (default 09:36) and drives both the fog
and Bevy's sun (direction, colour, illuminance = 8000 lux per game light
unit).

Not done yet: `AtmosphereVolume` shapes, step 5, the game's ambient lighting
of surfaces, and the units: one game light unit is taken as our radiance of
a white surface under 8000 lux (`--fog-unit` scales the fog's light). These
need screenshots of the game at the same places to check.
