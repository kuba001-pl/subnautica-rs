# Sky: uSky's sky dome and sky map

Described build: game build `10`. Code: `crates/sn-unity/src/sky.rs`
(`SkyManager` incl. `SkyDome`, `SkyLight`, `Gradient`), `crates/sn-assets`
(`sky`, `sky_textures`), `apps/sn-client/src/sky.rs`, `sky_dome.rs`,
`sky_common.wgsl`, `sky_dome.wgsl`. The sun, ambient and fog values are in
`docs/formats/water.md` § Sky.

Behaviour from the game's classes (`uSkyManager`, `uSkyLight`,
`uSkymapRenderer`, `DayNightCycle`; decompiled once on the dev machine, not
copied) and the compiled shaders of the materials `uSkybox` and `uSkymap`
(Direct3D 11 bytecode disassembled on the dev machine; constant names from
the binding tables). Each fact is **confirmed** (how) or **hypothesis**.

## Two sun directions — confirmed from the classes

- `uSkyManager.SunDir` (inside the manager, its own `GetLightDirection`):
  `sunEuler · forward` with `sunEuler = Euler(0, SunDirection,
  NorthPoleOffset) · Euler(Timeline × 15° − 90°, 0, 0)` — a plain hour angle
  that sets at night. Used by the sky shader, the day/night/sunset factors
  (`uMuS`), the water fog and the water surface.
- The directional light (`uSkyLight.GetLightDirection`, the light's
  transform): `Euler(0, SunDirection, NorthPoleOffset) · Euler(a + 90°, 0,
  0)` with `a` from −65° to +65° over the day and back at night: it always
  points down (the night light is moon-like). Lights the scene only.

Until 2026-10-07 we used the light's direction for the fog and the day
factor; at 09:36 the two differ by 7° (66° vs 73° up), at night by far more.

## Scene values — confirmed

`uSkyManager` read to the object's last byte (layout from UnityPy's type-tree
generator; values equal UnityPy's): planet radius 3500, zenith 59°, distance
10000, orbit 1000° per game day, light wrap 0.353; clouds rotate 1°/s,
attenuation 1.05, alpha saturation 2.5, scattering ×5.35 ^3.9, sun colour
×3.72, sky colour ×1.87, night brightness 0.25; night sky mode 2 (static),
moon size 0.2, star intensity 1; linear colour space, LDR sky (`SkyboxHDR`
off → colour correction (1, 2)). The manager's transform is the identity;
there is no moon light (the moon uses a fixed rotation, quaternion
(−0.924, 0, 0, 0.383)).

Textures: planet `moon_01` (2048², DXT1, repeat, sRGB), sun burst
`SunGlow` (1024², DXT5, clamp), moon `Full_Moon` (256², clamp), clouds
`Sample_Rectangular_4096` (2048 × 512, one mip, repeat, linear).

The planet's position: `(sin z cos o, cos z, sin z sin o) × distance` with
`z` the zenith angle and `o = orbit speed × game day` (`DayNightCycle`: one
day is 1200 s; a new game starts at day 0.4, 09:36).

## The sky function — confirmed from the compiled shaders

Per direction `d` (Unity coordinates), with `μ = d · SunDir`:
1. Optical depth `h = max(d.y + 0.06, 0.06) + max(−d.y, 0) × ground` (ground =
   βR / (ground colour × 0.01)); `sR = 8 / h`, `sM = 1.2 / h`, both × (1 −
   eclipse).
2. A zenith term `Z sR (2 − Z sR)` (Z = night zenith colour × 0.01),
   extinction `E = exp(−(βR sR + βM sM))`, in-scattering `I = term × E +
   sunset × ((1 − E) − E × term)`, Mie colour `sM I / I.r × mieConst`,
   Henyey–Greenstein Mie phase (g = 0.76), sky = `(0.75 I + Mie × phase) ×
   (1 + μ²) × day factor`.
3. The planet (ray–sphere test): a textured, wrapped-lit sphere with a rim,
   or around it two corona terms; added × `E`.
4. The sun disc (`min(Mie colour, d.y) × min((1 − μ) × 32/size)^−1.5, 1000)
   × E`) where the sun is above −0.1 and the planet is not in front; a burst
   texture only during an eclipse.
5. At night (sun below 0.25): the night horizon colour × the zenith term,
   the moon texture (projected along the moon's axis) and its coronas.
6. Tone curve: `((1 − e^−c) × 1)²`.
7. Clouds: a panorama (azimuth / 2π, elevation / (π/2)) rotating about the
   vertical axis; density from 6 taps upwards, transmission `2^(−sum × 1.05)`,
   lit between the sky and sun shade colours (sun colour gradient and sky
   colour × max(0.25^1.5, day) × √exposure, × their multipliers), alpha
   `cloud^2.5`, faded out below the horizon (skybox only).

The skybox draws this behind everything (before the fog image effect). The
sky map (`uSkymapRenderer`, 256² ARGB32 with mips, redrawn every frame) is
the same function on the upper hemisphere, `p = (uv − 0.5) × 2.2`,
`d = (2p.x, 1 − |p|², 2p.y) / (1 + |p|²)`; the water reflects it.

## How we render it (M8c2)

`sky_dome.rs` builds the uniform from the scene values every frame (cloud
rotation from the time since start-up, the planet from `--time` on day 0).
One pass draws the sky map and its mips, another the dome where the depth
buffer is empty, before the fog pass. Our sky value 1 = one game light
unit (see `docs/formats/water.md`). **Not done:** stars (`StarField`, a
mesh drawn at night), the eclipse/end-sequence and rocket effects; the
planet's texture detail is chosen from its size on screen.
