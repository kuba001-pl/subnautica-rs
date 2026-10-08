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

**Ours (M8c5):** `sun_shafts.rs`/`.wgsl`, after the water surface, 0.37 ms.
**Not yet:** the shadow map (no sun shadows yet: every sample is lit).

## How we render it (M8c3, first pass)

Forward, in our terrain and object shaders, instead of Bevy's PBR lighting.
The per-frame values (sun, ambient, the water at the **camera**, caustics
frame and projection, Unity's flat ambient) are an 11-texel float texture every material binds;
its contents are rewritten on the GPU each frame. Shadows use Bevy's shadow
map when the sun casts shadows (off for now). **Not done:** per-pixel water
settings (the game's volume), shadows, object specular/gloss/emission maps
(objects get no specular yet), point lights.
