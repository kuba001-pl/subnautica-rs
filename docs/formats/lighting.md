# Lighting of opaque surfaces

Described build: game build `10`. Code: `apps/sn-client/src/game_light.rs`,
`game_light.wgsl` (used by `terrain.wgsl` and `object.wgsl`),
`crates/sn-assets` (`water_caustics`, `resource_texture`),
`crates/sn-unity` (`parse_resource_container`).

The game renders deferred with a custom lighting shader
(`GraphicsSettings.m_Deferred` = custom, `Hidden/Internal-DeferredShadingCustom`
in `globalgamemanagers.assets`; disassembled once on the dev machine, not
copied). Its directional-light pass also applies the ambient light. Each fact
is **confirmed** (how) or **hypothesis**.

## The G-buffer — confirmed for terrain (`docs/formats/terrain-materials.md`)

Albedo, specular colour + gloss (specular power `max(gloss × 128, 0.1)`),
normal (alpha ≥ 0.5 marks surfaces the pass leaves unlit), emission
written directly to the light buffer.

## The directional light pass — confirmed from the compiled shader

Variant `DIRECTIONAL_COOKIE` + `UNITY_HDR_ON` (the sun carries the caustics
as its cookie). Per pixel, world position `p`:
1. Water settings at `p` from the volume textures (`_UweExtinctionTexture`,
   `_UweScatteringTexture`, `_UweEmissiveTexture`, see `water.md`): σt,
   light scale `k` (sunlight scale × transmission), emissive `e`, ambient
   scale `a`.
2. **Caustics**, only below the water plane (`p.y < 0`, fog on): the cookie
   at `(worldToLight · p).xy × _UweCausticsScale`, mip bias −8; value
   `max(c × amount.y + 1 − amount.x, 0)`. Scene: caustics scale 2 ÷ size 2.5
   = 0.8 per cookie unit, amount (0.1, 0.6): the light varies between 0.9 and
   1.5. The frames (`Data/WaterCaustics00…63` in Unity's `Resources`, 256²
   DXT1, linear) change at 25 per second. The cookie is projected by the
   light's local x/y over its cookie size (10 m; the scene's `Light`, read
   with UnityPy) — **hypothesis**: Unity adds 0.5 (only a phase).
3. **Sunlight under water**: `path = p.y / L.y` metres along the light from
   the surface; attenuation `exp(−path × (σ + f (min(σ) − σ)))` with `σ =
   σt × sun attenuation` and the colour cast `f = exp(−d × 0.1 − path ×
   0.0002)` (`d`: distance to the camera): grey near, coloured far.
4. **Ambient**: `lerp(bottom, top, n.y × 0.5 + 0.5) × a × k (× attenuation
   under water)` plus the water's glow `e × emission ambient scale / σt`.
   Top/bottom ambient: `uSkyLight`'s sky and ground gradients through its
   colour offset, × exposure, linear, × ambient light (0.35).
5. **Sun**: diffuse `cookie × k × attenuation × max(n·L, 0) × light colour`;
   specular `pow(max(n·h, 0), power) × saturate(cookie.r) × luminance-ish
   weight of the light colour × specular colour × diffuse`.
6. Result: `albedo × (ambient + diffuse) + specular`.

The light's direction here is the directional light's (always down), not the
hour-angle sun of the sky and fog (see `sky.md`).

## Unity's own ambient — confirmed

The terrain's G-buffer pass (`LIGHTPROBE_SH` variants) adds Unity's ambient
into the emission target: `albedo × SH(n)`. The scene's `RenderSettings`
use the flat ambient mode (3) and `uSkyLight` sets the colour every frame
to `CurrentSkyColor × (1 − eclipse)` (linear in a linear project): the same
colour everywhere, under water too, without the 0.35 ambient factor.
`AtmosphereDirector` picks per-volume ambient settings but only logs them.

## Units and the final image — confirmed from the classes

Unity's HDR buffer holds the game's values: a white surface lit by a light
of intensity 1 is 1.0. The camera's post-processing (Unity Post Processing
Stack v1, `UwePostProcessingManager`, profile `default_Post-FXProfile`):
colour grading per the "Color grading" option — 0 off (the default: values
clamped to 0…1), 1 Neutral tonemapper, 2 ACES; the profile's grading is
otherwise neutral (no exposure, contrast or colour changes); bloom on by
default (intensity 0.2, threshold 0.9, soft knee 0.55, radius 5.5, lens dirt
5); eye adaptation off. We now write the game's values unchanged (one game
light unit = 1.0) and tonemap per `--color-grading` (off by default;
neutral/ACES use Bevy's nearest curves — **not exact**). Bloom: not yet.

## Sun shadows — confirmed from the game's quality settings

`QualitySettings` (globalgamemanagers, read with UnityPy), level "High" (the
user's "Detail" option): soft shadows, resolution High, stable fit, 4
cascades over 50 m split at 6.7 %, 20 % and 46.7 % (3.3, 10, 23.3, 50 m),
near plane offset 2. Medium: 2 cascades over 35 m; Low: none. The sun
`Light`: soft shadows, strength 1, bias 0.45, normal bias 0.4.

**Ours (M8c6):** Bevy's cascaded shadow map with exactly these cascade
bounds, 2048² per cascade, Gaussian filtering for the soft shadows (Bevy's
bias defaults: **not** the game's values yet). Only the nearest level of
detail (terrain and objects within ~100 m) casts, since the shadows end at
50 m: this kept the frame time at 11 ms instead of 21 ms. The light shafts
sample the same shadow map, one hardware comparison per step, as the game.

## Light shafts — confirmed from the class and compiled shader

`WaterSunShaftsOnCamera` on the main camera (an image effect after the
transparent geometry, so it also lights the water surface seen from below;
off when shadows are off). Scene values: start 5 m, max 15 m, shafts scale
−0.16, intensity 3.93, trace step 0.05 m, half resolution (class defaults
differ: 0.05, 0.003). Per pixel, along the view ray through water only
(from max(start, the surface) to min(scene, 15 m, the surface)):
- every 0.05 m: the caustics texture (current frame) at the point's light-
  space x/y × −0.16, × the sun's shadow map, × `exp(−σt t)`;
- × light colour × step × σs × light scale × 6 (caustics stored ÷ 6) ×
  `smoothstep(saturate(0.8 (1 − z)))` (`z`: the view direction along the
  light; strongest looking towards the sun) × the sunlight's attenuation
  from the surface to the start point (with the colour cast) × intensity;
- added to the image (bilinear from half resolution).

**Ours (M8c5, M8c6):** `sun_shafts.rs`/`.wgsl`, after the water surface,
with Bevy's view bindings for the shadow map (as Bevy's volumetric fog):
1.9 ms (0.37 ms without shadows; 4.8 ms with the soft filter).

## How we render it (M8c3, first pass)

Forward, in our terrain and object shaders, instead of Bevy's PBR lighting.
The per-frame values (sun, ambient, the water at the **camera**, caustics
frame and projection, Unity's flat ambient) are a 12-texel float texture every material binds;
its contents are rewritten on the GPU each frame. Shadows use Bevy's shadow
map when the sun casts shadows (off for now). **Not done:** per-pixel water
settings (the game's volume), point lights. Objects: see § Objects.

## Objects (MarmosetUBER) — confirmed from the compiled shader and classes

Almost every world-object material uses the game's `MarmosetUBER` shader
(1,537 of the 1,588 materials loaded around the lifepod; of the 2,746 UBER
materials in the game, 2,715 have the `MARMO_SPECMAP` keyword, 1,092
`MARMO_EMISSION`). Its deferred pass (D3D11 bytecode, disassembled once on
the dev machine with Windows' `d3dcompiler_47.dll`; variant
`MARMO_EMISSION MARMO_SPECMAP UNITY_HDR_ON`; constant-buffer offsets from
the program's binding data) writes, per pixel (uv: mesh uv × each map's
`_ST`):
- **Albedo** = `_MainTex` × `_Color` × camera exposure (`_ExposureIBL.w`) ×
  `g`, where `g = 1 + _EnableSimpleGlass (alpha − 1)`; then mixed towards
  its grey by `_Gray` and offset by `_Brightness` (both only set by the
  coral-bleaching script; 0 otherwise).
- **Specular** = `saturate(1.25 − |n·v| _Fresnel)⁵ × _SpecInt × _SpecTex.rgb
  × _SpecColor × exposure.w`. Gloss `a = _SpecTex.a`, `r = (1 − a)²`, mip
  level `m = 8 − r − _Shininess (1 − r)`, power `p = 2^(8 − m)`; stored as
  colour `× (p · 0.159155 + 0.31831) / 8` and gloss `p / 64`, which the
  light pass turns back into the power `gloss × 128` (§ The G-buffer).
- **Normal alpha** = `((1 − _Outdoors) + 2 (1 − _AffectedByDayNightCycle)) / 3`:
  for skies not affected by the day/night cycle (caves, interiors) it is
  ≥ 0.5, and the directional light pass then adds **neither sun nor
  ambient** (confirmed in its bytecode: everything × `alpha < 0.5`).
- **Emission** (light buffer): night factor `k = (1 −
  _UweLocalLightScalar) × _AffectedByDayNightCycle`; glow `G =
  lerp(_GlowStrength, _GlowStrengthNight, k)`, `E = lerp(_EmissionLM,
  _EmissionLMNight, k)`; `_Illum.rgb × g × _GlowColor × G × exposure.w +
  albedo × _Illum.a × g × E`, × (1 − `_UwePowerLoss`, 0 outdoors).
  Plus the sky's specular cube along the reflected view direction (mip
  `m`) × specular × `_ExposureIBL.y` × (1 − `min(k, _IBLreductionAtNight)`),
  and, only when `_AffectedByDayNightCycle` is 0, the sky's 9-coefficient SH
  at the normal (in the sky's frame, absolute value) × `_ExposureIBL.x` ×
  (1 − that reduction) × albedo. **No** Unity ambient (unlike terrain).
- `UWE_LIGHTMAP` (55 % of the materials) samples `_Lightmap` with the
  second uv but only blends it with `_UniformOcclusion`, which `mset.Sky`
  sets to 1: no visible effect. Alpha clip: discard below `_Cutoff`.

`_UweLocalLightScalar` (`DayNightCycle`): `GammaToLinear(saturate(I ×
mean(colour) × 1.2 − 0.15))` of the sun light (`uSkyLight`: intensity =
exposure × (sun × day + moon × night) × direct fraction, 1 in the scene;
colour as stored).

**Which sky (confirmed from the classes):** the sky values (`_ExposureIBL
= (master × diff, master × spec, master × sky × cam, cam)`, SH = stored
coefficients × `SHEncoding.sEquationConstants`, `_AffectedByDayNightCycle`,
`_Outdoors`, specular cube) come from an `mset.Sky`. Renderers listed by a
`SkyApplier` (on most drawable prefabs: 1,658 of 3,201 world prefabs,
e.g. 859 of 1,020 doodads) take, with `anchorSky` Auto, the sky of the
biome at the object's position (`WaterBiomeManager.biomeSettings[].skyPrefab`;
unknown biomes take the first entry's, safe shallows); others take the
global sky, which outside the lifepod is `MarmoSkies.skySafeShallowsPrefab`.
The biome comes from atmosphere volumes first, then the batch override and
the biome map. 37 distinct skies for 145 biomes (read with our reader and
UnityPy, equal): e.g. safe shallows exposure (1, 0.65, 0.17, 1), affected;
grand reef master 3, diffuse 0, specular 0.38; safe-shallows caves master
0.25, not affected. Only the explorable-wreck sky is rotated (−90° about y).

**Ours (M8c7):** `object.wgsl` computes these G-buffer values and feeds
them to our port of the light pass (skipped where the alpha says unlit),
then adds the emission and SH terms. Each material is made once per sky
it's used with; the sky is picked per placed object from our biome lookup
(batch override, then map). **Not done:** the specular cube reflection;
atmosphere volumes in the biome lookup (so cave skies are rarely picked);
anchors other than Auto (taken as the global sky); `_UwePowerLoss` (bases);
the non-`MARMO_SPECMAP` variant is assumed to use a white map
(**hypothesis**); the SH rotation by the sky's frame assumes `_SkyMatrix ×
n` (**hypothesis**, only matters for the wreck sky).
