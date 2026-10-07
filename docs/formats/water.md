# Water: biomes, per-biome water settings, fog and surface

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
(`skyFogDensity`, a gradient over the day). The sun direction is
`uSkyManager.GetLightDirection()`: a plain hour angle (`Timeline × 15° −
90°`) with `SunDirection` and `NorthPoleOffset`, not the directional
light's own direction (see `docs/formats/sky.md` § Two sun directions).

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

## Water surface — confirmed from the classes and compiled shaders

Read from the main scene's `WaterSurface` MonoBehaviour (reader
`crates/sn-unity/src/water_surface.rs`, layout generated once with UnityPy's
type-tree generator; our reading ends exactly at the object's last byte and
matches UnityPy's values, real-data test `water_surface_values`). Behaviour
from the class (`WaterSurface`, decompiled on the dev machine) and its four
shaders (`Custom/WaterSurface`, `Hidden/Waterscape/InterpolateDisplacement`,
`…/UpdateNormals`, `…/UpdateFoam`; Direct3D 11 bytecode disassembled on the
dev machine, constant names from the binding tables, render states from the
shaders' parsed form).

**Scene values (build 10):** patch length 2000 cm (one wave tile is 20 m),
sequence length **5 s** (class default 10), linear interpolation (cubic
off), screen-space reflection off, refraction index 1.33, under-water
refraction index 1.1 (+ 0.01 per metre of camera depth), reflection colour
(0.707, 0.956, 1.0), refraction colour white, back-light tint (0.114, 1,
0.059), sun glint gloss 400 and amount 1, foam: smoothing 5, rate 3, scale
6, decay 5, distance 5, displacement-texture multiplier 5; caustics: 64
frames at 25 fps. Under-water sky brightness by camera depth: an
`AnimationCurve` (1.51 at the surface, 3.98 at 47 m, 1.0 from 112 m;
clamped).

**Water quality:** "Medium" (the default) plays baked waves; "High" runs an
FFT (`WaterDisplacementGenerator`, compute shader) — not ported.

**Wave frames:** the 64 textures labelled `WaterDisplacement` in the
Addressables catalog (`Assets/Waterscape/Data/WaterFrame00…63.png`, one
bundle), 256² RGBA32, stored linear. (`StreamingAssets/AssetBundles/
waterdisplacement` holds a copy the game does not load this way.) A texel
holds a displacement in cm: x = R, y = G + A / 255 (16 bits), z = B, each
`v × 2 max − max` with max = (100, 300, 100). Over all frames x spans
−65…72 cm, y −71…75 cm, z −67…68 cm.

**Per frame** (`WaterSurface.DoUpdate`), with `time += Δt × timeScale`:
1. Frames `i = floor(time × 64 / 5) mod 64` and `i + 1` blended by the
   fraction into a 512² float displacement map (repeat).
2. Normal map, 512², with mips and anisotropic filtering: per texel
   `((h(u−1) − h(u+1)) / 2, (h(v−1) − h(v+1)) / 2)` of the height (y).
3. Foam amount, 512² half float, blended `new + old × decay` ("Blend One
   SrcAlpha") with decay `max(1 − Δt × 5, 0)` and rate `3 Δt / 0.008333`:
   `new = (1 − smoothstep(min(11 L, 1)))¹⁰ × rate`, where `L` is the length
   of (Δx, Δh) between neighbouring texels (metres, minus their 3.9 cm
   spacing). The game's shader compares the **heights** along v, not the z
   displacement (kept as is).

**Surface vertex:** texture coordinates `world xz × 100 / patch length`;
displacement × 0.01 × `fade`, with `fade = saturate((200 − d) / 192)` and
`d` the horizontal distance to the camera (no waves beyond 200 m). Drawn
in the transparent queue after the fog image effect, both sides (Cull
Off), depth test LEqual.

**Surface pixel, from above** (front face), with `n` = normalize(normal
map × fade, 2 × texel length, …) and `v` towards the camera:
- sky reflection: the sky map along `reflect(−v, n)` (a paraboloid-like
  lookup, `xz × 2 / (1 + y) × 0.227 + 0.5`), blended to the mean sky colour
  (`uSkyManager.meanSkyColor` over the day) by `max(1 − 1 / (0.03 d), 0)`;
- refraction: the (already fogged) scene, offset on screen by the
  view-space x/y of `refract(−v, n, 1 / 1.33)` × (−0.1, 0.2) / depth (less
  near the top of the screen, mirrored at the edges); where the offset
  lands on something in front of the water, the unshifted pixel is used;
- Fresnel: `R0 + (1 − R0)(1 − n·v)⁵`, `R0 = ((1 − 1.33) / (1 + 1.33))²`;
  colour = lerp(refraction × refraction colour, sky × reflection colour);
- back light: `(saturate(slope^1.2 × c³) × 15 + max(height × 10⁻⁴, 0) × 15)
  × tint × transmission × sun`, slope from a blurred normal (mip bias = foam
  smoothing), `c = v·toSun` (× −0.55 when negative);
- glint: `saturate(reflect · toSun)^400 × sun × 1`;
- foam (both faces): amount `e^(−0.02 d) × foam map × 5`, thresholded by
  the foam mask (`FoamBubbles`, × 12 tiling) against the foam texture
  (`WaterFoam`, × 6), lit by sun + ambient, water-fogged, mixed at half its
  alpha; then sky fog over the camera distance.

**From below** (back face): `k = 1 − η²(1 − (n·v)²)` with η the under-water
refraction index. Total internal reflection (`k < 0`): the deep water's own
colour along the reflected ray (in-scattering and emission over 1000 m from
the surface), water-fogged up to the surface. Otherwise the refracted ray:
the scene above where it is behind the surface, else the sky map (fogged);
× the under-water sky brightness.

**Clip map:** an orthographic camera 30 m up renders only "base clip proxy"
objects (bases, the Aurora, the lifepod) into a 1024² RG half map cleared
to (0, 1): value `R + 10 + 1000 G`. The surface is cut out where it is
negative; shore and sub-surface foam grow where it is below the foam
distance. With no proxies it is 1010 everywhere: no cut-outs, no shore
foam. **Not read yet** (our surface uses the cleared value).

## How we render the surface (M8c1)

`apps/sn-client/src/water_surface.rs`, `water_sim.wgsl`,
`water_surface.wgsl`; the fog model is shared with the fog pass
(`water_common.wgsl`). The per-frame maps are made on the GPU as above
(half floats instead of floats). The mesh is ours: 0.25 m quads within
±32 m, 1 m quads to ±208 m (snapped to whole metres; the fine grid's edge
follows the coarse one, no cracks) and a flat ring to 40 km, generated in
the vertex shader. The surface is drawn after the fog pass on a copy of the
fogged image, with its own depth buffer; the scene's depth is tested in the
shader. Without a sky dome yet (M8c2) the sky map is the mean sky colour;
the clip map is not read (see above).
