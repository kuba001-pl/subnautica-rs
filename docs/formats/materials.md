# Materials and the shaders they use

What the world's materials ask for, and which of them our object shader
(`apps/sn-client/src/object.wgsl`, a port of MarmosetUBER:
`docs/formats/lighting.md` § Objects) cannot draw as the game does.
Everything here comes from `sn-inspect prefab --materials` (2026-10-09, game
build 10): every material on a node we draw at full detail
(`Prefab::visible`) in the 1,369 placed prefabs and the two startup scenes
we spawn (`aurora` intact, `escapepod` at the origin), counted per drawn
node and per placement in the world.

## Shader names — confirmed (M7g2)

A material points to its `Shader` object (class 48). `sn_unity::Shader`
reads its parsed form (`m_ParsedForm`) up to the name: properties,
sub-shaders, passes and their render state; the compiled programs'
bindings are walked over (layout: `docs/formats/unity.md` § Shaders).
Checked: all 363 shader objects of the game parse (`sn-inspect unity
--all`); for the 17 shaders the drawn materials use, name, sub-shader and
pass counts and the first pass's blend, colour mask, depth write, depth
test and cull are identical to UnityPy's.

## Census of drawn materials — confirmed from the data

### Surface sidedness and reflected placements (M7v1, 2026-10-12)

**Confirmed from our loader:** object materials previously stored only a
two-sided boolean from a saved `_MyCullVariable == 0`. Front culling became
back culling; fixed shader states and declared defaults were ignored. The
loader now resolves the first pass's Cull state: a fixed value wins, or its
named material property overrides the declared shader default. Values 0/1/2
retain Off/Front/Back. SIG's confirmed fixed back culling stays unchanged.
Marmoset's declared default is 2, confirmed with installed shader metadata.

**Confirmed from an installed-data audit:** 1,770 local transform records
have negative determinant; 1,636 have a Precursor-named ancestor, including
prison aquarium wall and wall-pattern pieces. These are serialized records
across bundles, not a count of unique placed world objects. Material-name/
shader-ID census: 302 saved Off, 4 Front, 2,855 Back, 351 missing the named
property; Precursor-named subset: 34 Off, 648 Back, 3 missing. Dumps and
the read-only UnityPy research scripts stay in ignored `out/m7v1/`.

**Confirmed renderer mismatch:** Z reflection is already compensated when
mesh bytes are converted, but an additional negative placement/parent scale
reverses triangle order again. The ordinary object path did not compensate
for it. Bevy 0.19.1's mesh/material source was checked from its official
crate archive ([official package](https://crates.io/crates/bevy_pbr/0.19.1),
reference only, ignored under `out/`): the determinant flag
does not switch the standard material's raster culling state. A synthetic
triangle test verifies that all eight scale-sign combinations keep triangle
winding aligned with transformed normals after correction.

M7v1 caches a reversed-index mesh variant per original object mesh, choosing
it from the global determinant after transform propagation. This covers
nested parents and changing animated scale signs; original attributes,
normals, UVs, skinning and morph data are retained by cloning the mesh and
changing only indices. Returning to positive determinant restores the
original handle. The custom brine buffer retains its indices and compensates
its front-facing test instead. Materials that request two sides still draw
two sides; collision and camera layer masks are unchanged.

**Hypothesis pending user visual confirmation:** these reflected wall pieces
explain the reported inside/outside reversal across large alien structures.
This does not establish that every missing surface has this cause. Any
remaining missing exterior geometry, unsupported material/state or activation
rule needs a specific wall/location. Original-game matched images are
**not tested**.

**Confirmed lifetime correction (2026-10-12):** the user's 73-second
streaming run exposed `ExtractedToRenderWorld` during lazy index reversal.
Ordinary object meshes were created with `RENDER_WORLD` only, so their CPU
vertex/index data was gone when a later reflected placement first needed
a variant. Short settled camera checks did not exercise that ordering.
Original object meshes now retain `MAIN_WORLD | RENDER_WORLD`; skinned
meshes already did so for their bounds. Index reversal uses fallible access
and rejects unavailable data without a panic. A synthetic regression uses
the production mesh builder, applies the GPU extraction lifetime decision,
then spawns a reflected instance later and verifies reversed indices and
the intact original. This retains more CPU mesh memory; long streaming
results and memory measurements are recorded in MODLOG.

1,975 materials on drawn nodes, told apart by name **and** shader (names
repeat across bundles, e.g. `submarine_engine_01_01` is MarmosetUBER in
some prefabs and `Standard` in one wreck). By shader (drawn nodes,
placements, materials), with the first pass's state (blend source and
destination factors, `[_Prop]` = from the material; z write; cull):

| Shader | Nodes | Placements | Materials | First pass | Examples |
|---|---:|---:|---:|---|---|
| `MarmosetUBER` | 34,879 | 258,879 | 1,903 | `1 0`, z write `[_ZWrite]`, cull `[_MyCullVariable]` | almost everything |
| `UWE/SIG` | 54 | 11,095 | 13 | `1 0`, z write on, cull back | `Coral_reef_small_deco_06/09/21`, stalactites, lava rocks |
| `UWE/Particles/UBER` | 169 | 1,058 | 31 | `[_SrcBlend] [_DstBlend]`, z write off, z test `[_Ztest]` | Lost River brine lakes and waterfalls, Precursor terminal screens and halos, lava fall, sand fall, smoke column, tech light cones |
| `UWE/SIG Triplanar with Capping` | 15 | 976 | 5 | `1 0`, z write on, cull back | `SS_Prop_SandtoRock` (Safe Shallows rocks), `SS_Prop_Coral02/03/08` (coral clumps) |
| `UWE/Particles/WBOIT-FakeVolumetricLight` | 68 | 239 | 3 | `1 1` (additive), z write off, cull off | `x_AtmoLight_Sphere`, `x_AtmoLight_Cone` |
| `UWE/SIG AlphaCutout + Noisey Wave` | 3 | 55 | 3 | `1 0`, colour mask RGB, cull off | `Coral_reef_grass_01_red`, kelp grass |
| `Unlit/DepthOnly` | 39 | 40 | 1 | `1 0`, z write on | `DepthOnly` on `Occluder_*_shell` nodes |
| `Standard` (Unity's) | 30 | 37 | 4 | `[_SrcBlend] [_DstBlend]`, z write `[_ZWrite]` | 27 wires in `ExplorableWreck_Grassy_1`, a prison vent, two default materials |
| `Legacy Shaders/Particles/~Additive-Multiply` | 2 | 22 | 1 | `1 10`, z write off | `Fire Smoke` (geysers) |
| `Legacy Shaders/Diffuse` | 8 | 8 | 2 | `1 0` | sand in two abandoned bases |
| `UWE/Marmoset/Mesmer` | 1 | 8 | 1 | as UBER | Mesmer |
| `UWE/Marmoset/IonCrystal` | 4 | 4 | 2 | as UBER | ion crystals |
| `UWE/Experimental/Blinn Phong - All Maps` | 4 | 4 | 2 | `1 0` | the Sea Emperor baby models in the prison |
| `UWE/SIG Terrain Grass`, `UWE/SIG Transparent Waving` (`5 10`, alpha blend), `UWE/SIG-SandDrift`, `FX/WBOIT-WaterBase` (`1 1`) | 1 each | 1–2 | 1 each | | coral grass, aquarium sand drift, the Gun's moon pool water |

The property fingerprint our code uses for UBER (`_Shininess` +
`_GlowStrengthNight`) picks exactly the `MarmosetUBER`, `IonCrystal` and
`Mesmer` materials, and no others (checked on all 1,975).
`IonCrystal` and `Mesmer` have MarmosetUBER's two properties, so our code
already takes them as UBER. Every other non-UBER material is drawn with
`_MainTex` × `_Color` and our default lighting. Materials with no `_MainTex`
come out flat `_Color`, which is white for all of the ones above.

### Occluder shells (`Unlit/DepthOnly`) — confirmed

- 39 nodes named `Occluder_<room>_shell` in 38 Precursor prefabs (Gun,
  prison, Lava Castle base, Lost River base rooms), layer **27**
  (`Occluder`), renderer enabled, material `DepthOnly`, no textures.
- The game: `CullingOccluder` registers the renderer with `CullingCamera`,
  which draws these meshes with `DrawMeshNow` into its own 512 × 256 depth
  texture (and a mip chain) to cull other objects. The main camera's
  culling mask is `0x65ffff17` (`docs/formats/unity.md` § Cameras), which
  leaves layer 27 out, so the shells never reach the picture.
- Us, before M7g1: drawn as opaque white meshes (render queue −1, back
  faces culled). Since M7g1 the client reads the main camera's mask
  (`Assets::main_camera`) and draws no node on a layer it leaves out. That
  is the game's rule, so no names are matched, and it removes exactly these
  nodes.

### Fake volumetric lights — confirmed

- `x_AtmoLight_Sphere` (mesh `Sphere_24`, 11 nodes, 107 placements: ion
  crystal pedestals, …), `x_AtmoLight_Cone` (9 cone meshes, 53 nodes, 128
  placements: Precursor spotlights, Gun hallway lights, …),
  `x_AtmoLight_Cone_Antechamber` (4 nodes). Most nodes are named
  `x_FakeVolumletricLight` (sic).
- Shader `UWE/Particles/WBOIT-FakeVolumetricLight`, render queue 3101,
  keywords `FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_REFRACT_OFF FX_SCROLL
  FX_SOFTEDGES`, no textures, `_Color` white (the antechamber's light blue).
- **Decoded (M7g3, confirmed from the compiled D3D11 programs**, one
  vertex and one fragment program, the keywords compiled in; constant names
  from the programs' binding data; disassembled once on the dev machine
  with Windows' `d3dcompiler_47.dll`). Per pixel, with `d` the eye depth,
  `n` the object-space normal, `v` the object-space direction to the
  camera (normalised per vertex), `a_v` the vertex colour's alpha:
  - soft edges: `soft = saturate((sceneEyeDepth − d) × _InvFade)` from
    `_CameraDepthTexture`;
  - "Fresnel": `fres = |v · normalize(n)|^_FresnelPow × _FresnelFade`, so the
    glow is strongest facing the camera and gone at the silhouette;
  - near fade: `clip = min((d − _ClipOffset − near) / (_ClipFade +
    _ClipOffset), 1)` (`near` = `_ProjectionParams.y`);
  - falloff along the mesh: `t = saturate((1 − a_v − _Offset + 0.01) /
    (_Fallof + 0.01))`, `fall = smoothstep(t) × a_v`;
  - `α = saturate(fall × soft × fres × clip) × _Color.a × _Intensity`,
    colour `c = _Color.rgb × _Intensity`;
  - fog (`_UweFogEnabled`): the water fog of the glow's own distance
    (extinction, in-scattering, emission from the fog volume textures, the
    water plane and sun, as the fog image effect does), and
    `α ×= saturate(10 − 0.08 × t)` with `t` the glow's path through fogged
    water (past the fog's start distance, up to the surface), so gone when
    `t` is between 112.5 and 125 m; only where the water fog applies
    (re-read from the program 2026-10-09: `t` = water distance − start, not
    the whole distance);
  - out: WBOIT targets (below).
- **WBOIT, confirmed from `WBOIT.cs` and the shaders.** The `WBOIT`
  component on the main camera (`useDepthWeighting` 1,
  `depthWeightingSharpness` 0.1, read from the main scene) clears two
  half-float targets A and B to (0, 0, 0, 1), draws the overlays after the
  forward transparent pass into (camera, A, B), and composites with
  `Hidden/WBOIT Composite`. The glow writes camera 0, A = (W α c, α),
  B = (α W, 0, 0, 0) with `W = α × clamp(e^(−0.1 d), 0.01, 1)`; one blend
  for all targets (no separate blend): colour `One One`, alpha `Zero
  OneMinusSrcAlpha`. The composite (no keywords) samples B, moves the uv by
  `B.yz` (refraction offsets), and writes `(1 − A.a) × A.rgb / clamp(B.x,
  1e−4, 5e4) + A.a × scene` (alpha `(1 − A.a)² + A.a × scene.a`), clamped
  at 0. For one glow over a pixel this is plain alpha
  blending, `α c + (1 − α) scene`; where several overlap, their colours are
  averaged with the weights `W`, and the scene is dimmed by the product of
  their `(1 − α)`.
- Material values (logged by the client): `x_AtmoLight_Sphere` `_Intensity`
  1, `_FresnelFade` 0.75, `_FresnelPow` 4; `x_AtmoLight_Cone` `_Intensity`
  0.5, `_FresnelFade` 2, `_FresnelPow` 2; both `_ClipOffset` 0, `_ClipFade`
  1, `_Offset` 0, `_Fallof` 0, `_InvFade` 2, `_Color` white. With `_Offset`
  and `_Fallof` 0 the falloff is just the vertex alpha.
- **These material values are not what the game draws with — confirmed
  (2026-10-09) from the decompiled `VFXVolumetricLight` script** (44 on the
  placed prefabs, `docs/formats/lighting.md`). It sits next to a `Light`
  and, on `Awake`, gives its glow's renderer (`volumGO`) a property block:
  `_Intensity` = `intensity`, `_Offset` = `startOffset`, `_Fallof` =
  `startFallof`, `_InvFade` = `softEdges`, `_ClipFade` = `nearClip`,
  `_Color` = the light's colour with alpha × light intensity / 8 (only if
  `lightSource` — else the `Light` on its own object — and both `coneMat`
  and `sphereMat` are set). Every `LateUpdate` the glow's renderer is
  enabled as the light is, and `_Color` follows the light's colour and
  intensity. Field layout confirmed on a scene copy (176 bytes after the
  32-byte header): `syncMeshWithLight` (bool, aligned), `angle` i32,
  `range`, `intensity`, `startOffset`, `startFallof`, `nearClip`,
  `softEdges` f32, `segments` i32, `lightSource` PPtr, `lightType` i32,
  `color` 4 × f32, `lightIntensity` f32, then PPtrs `volumGO`,
  `volumRenderer`, `coneMat`, `sphereMat`, `volumMeshFilter`, `volumMesh`.
  Example (one in the `aurora` startup scene's bundle): intensity 0.3, start offset 0.25,
  start falloff 0.4, soft edges 3, light (1, 0.93, 0.66) at intensity 3, so
  `_Color.a` 0.375 and the glow's alpha at most ≈ 0.11, warm, fading along
  the cone. `UpdateScale` (sizing the mesh to the light's range) only runs
  from the editor; the stored scale is used.
- Mesh data (confirmed, logged): the cones (25 vertices) have alpha 1 at
  the apex and 0 on the rim; the apex normal points along the axis and the
  side normals belong to a narrower cone than the geometry (about 24° from
  the side, the shape opens about 40°). So near the apex the interpolated
  normal is mostly the axis, and seen from beside the apex the Fresnel term
  is high all across (measured ≈ 0.9): the top of a cone looks evenly lit
  with hard sides. That is the data under the decoded formula; whether the
  game looks the same there is **not checked**. Placed cones are big
  (uniform scale 30, 40, 68.9).
- Us (M7g3): ported in `effects.rs`, `effects.wgsl` and
  `effects_composite.wgsl` as above. Differences recorded: the fog uses the
  camera's water settings, not the game's fog volume textures at the
  glow's position (as our fog pass does); the composite's refraction,
  sonar and PDA variants are not ported (the glows don't refract). Before
  M7g3: alpha-blended with alpha 1 and our diffuse lighting, so solid white
  shapes. The first M7g3 build used the material values (white, alpha 1)
  and drew the cones as flat grey solids; the `VFXVolumetricLight` values
  are applied since (`sn-assets` `VolumetricGlow`, the client's
  `glow_material`), as the game has them on its first frame. Not followed:
  later changes of the light (a `DayNightLight` or another script changing
  its colour, intensity or enabled state while playing).

### `UWE/Particles/UBER` meshes — confirmed

- 31 materials, all in render queue 3101 (3000 for `x_flashlightCone`),
  textures `_MainTex` + `_MainTex2`, some with `_DeformMap`, `_NormalMap`,
  `_RefractMap` (the Lost River lakes). Their colours carry the glow (e.g.
  the terminal halos are green, the lakes' alpha is 0.471).
- The shader (`UWE/Particles/UBER`, one pass, tags `LIGHTMODE Vertex`,
  queue `Transparent+101`) compiles to 636 program blobs (1,272 programs in
  D3D11); its pass state takes blend, depth test and cull from the material:
  `Blend [_SrcBlend] [_DstBlend], [_SrcBlend2] [_DstBlend2]`, `ZTest
  [_Ztest]`, `Cull [_MyCullVariable]` (`Enum(TwoSided 0, OneSidedReverse 1,
  OneSided 2)`), no ZWrite. No game code sets these, so each material's
  stored values decide. `_ColorStrength` and `_ColorStrengthAtNight` are
  **Vector** properties (used as stored; `_Color` is a Color, made linear).
- **Decoded (M7g4, confirmed from the compiled D3D11 programs**, as for the
  glows): the variants `FX_ADDFOG FX_SCROLL WBOIT` with `FX_MULMAP` and
  `FX_FRESNELCLIP` (console halo, screen, screen background), with
  `FX_MULMAP` only (small symbol, squares), and with neither (symbol). All
  three are one program with those terms on or off. Per pixel, with `t` =
  `_Time.y` + `_UWE_EditorTime`, `v` the object-space direction to the
  camera (normalised again per pixel), `n` the object-space normal as
  interpolated (not normalised):
  - `c = 2 × _Color` (rgb and alpha);
  - `FX_FRESNELCLIP`: `rim = 1` if `_FresnelFade < 0`, else 0;
    `c.a ×= min(|rim − |v · n||^_FresnelPow × |_FresnelFade|, 1)` (so a
    positive fade keeps what faces the camera, a negative one the rim);
  - `c ×= vertex colour × _MainTex(uv × _MainTex_ST.xy + _MainTex_ST.zw +
    frac(t × _MainTex_Speed.xy))`, and with `FX_MULMAP` the same for
    `_MainTex2`;
  - `c ×= lerp(_ColorStrength, _ColorStrengthAtNight, 1 −
    _UweLocalLightScalar)`;
  - discarded where `c.a − _Cutoff < 0`;
  - `α = saturate((c.r + c.g + c.b) / 3) × c.a`: dark texels are
    transparent;
  - `FX_ADDFOG`: fog and the fade over the fogged path exactly as the glows
    (`saturate(10 − 0.08 × path)`), then `α = saturate(α)`;
  - out: the WBOIT targets as the glows (A = (W α c, α), B = (α W, 0, 0,
    0)).
- The console materials' state: `_SrcBlend` 1, `_DstBlend` 1, `_SrcBlend2`
  0, `_DstBlend2` 10 (the glows' blend), `_Ztest` 2 (Less), `_MyCullVariable`
  0. Example, `x_Precursor_ComputerTerminal_Halo`: `_Color` (0.228, 0.912,
  0.261, 1), `_ColorStrength` (1, 1, 1, 0.25) day and night, `_FresnelFade`
  3, `_FresnelPow` 1, `_MainTex_ST` (20, 2), speeds (0.1, −0.2), (0.1, 0.1).
- **The door force fields (decoded 2026-10-09, confirmed the same way):**
  `FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT`, with
  `FX_REFRACTMAP` (`precursor_doorway_portal`) or without
  (`precursor_doorway_portal2`); the two differ only in the refraction
  output (register allocation aside). The same program as above (no
  Fresnel term) with:
  - `FX_SOFTEDGES`: `c.a` starts at `saturate((sceneEyeDepth − d) ×
    _InvFade)` × vertex alpha (`_CameraDepthTexture`, as the glows);
  - `FX_DEFORM`: `uv' = uv + (_DeformMap(uv × _DeformMap_ST.xy +
    _DeformMap_ST.zw + frac(t × _DeformMap_Speed.xy)).xy − 1) ×
    _DeformStrength` (map value 1 = no shift, not centred on 0.5); every
    other texture is sampled at `uv'` instead of `uv`;
  - `FX_REFRACTMAP`: sampled at `uv' × _RefractMap_ST + frac(t ×
    _RefractMap_Speed.xy)` and unpacked like a normal map, `n = (a × r, g) ×
    2 − 1`; `B.yz = n × _RefractStrength × c.a × e^(−σt.b × path)` (`c.a`
    the colour chain's alpha after the strengths, before the brightness
    mean; `σt.b` the fog volume's blue extinction × its scale; `path` the
    fogged path as above, the factor 1 without fog). The composite moves
    its lookup by `B.yz` (summed over the effects, `One One`). Without
    `FX_REFRACTMAP`, `B.yz` = 0.
  - Both materials hold a float **and** a vector named `_RefractStrength`
    (0.02 and (0.01, 0.01, 0.01, 0.005)); the program reads the float
    (constant of dim 1). Values of `precursor_doorway_portal`: `_Color`
    (0.404, 1, 0.546, 1), `_ColorStrength` (4, 4, 4, 0.1), `_InvFade` 1.6,
    `_DeformStrength` 0.005, `_Cutoff` 0.01, `_MainTex_ST` (40, 40),
    `_MainTex2_ST` (0.1, 50, 0.5, 0), `_DeformMap_ST` (0.2, 30),
    `_RefractMap_ST` (0.1, 60), speeds main (0.01, −0.05), main2 (−0.01,
    −2), deform (0, 0.5), refract (0, 0.5); same render state as the
    consoles.
- **Extra materials draw the last sub-mesh again** (Unity's documented rule
  for a renderer with more materials than its mesh has sub-meshes; not
  checked against the game's picture). The force
  field node `x_Forcefield` has one sub-mesh and both portal materials, so
  the second layer is such an extra pass; others seen: a coral plant's
  `_opaquepass` + normal material, Precursor columns, abandoned base
  corridors, the thermal reactor halo, the Gun's elevator tube and
  terminal screen. Before this (M7g4) we drew only one material per
  sub-mesh.
- Us: since M7g4 these five variants are drawn by our effect pass
  (`effects_uber.wgsl`) when a material also has that render state; the
  client logs every other `UWE/Particles/UBER` material with the reason
  (keywords not decoded, or a blend/depth/cull value the pass does not
  draw) and keeps it on the old look: alpha-blended `_MainTex` × `_Color`,
  with no scrolling, second texture, deformation or refraction. M7g4a/b also
  support Tree Cove and Lost River brine described below. M7g4c/d add the
  control-room fields, tech light cones, Gun elevator, reactor electricity,
  lava falls and the orange key symbol. Not decoded yet: the actual
  lighting modes (sand falls, Lava Castle smoke, peeper bubbles and the
  water force field), kelp billboard variants and the tool flashlight.
  Differences: the fog uses the
  camera's water settings, not the fog volume at the mesh (as for the
  glows); the refraction offset's sign in y is **not checked** (Unity's
  render-texture uv may run the other way; the force fields' maps are
  noise, so it does not show).

### Remaining unlit mesh combinations (M7g4d) — confirmed

Confirmed by reading the installed D3D11 program bindings and arithmetic,
cross-checking saved material properties with `sn-inspect prefab --props`.
The research dumps remain in ignored `out/m7g4-brine/fresh-programs` and
`out/m7g4d`. These combine previously decoded terms:

- Tech light cones: vertex/fragment blobs 67/385, Fresnel, near-camera
  vertex fade and soft depth intersections, one scrolling texture,
  brightness-based opacity (`FX_ADDFOG`).
- Gun elevator smoke: 68/386, two scrolling textures and UV deformation,
  Fresnel, near-camera fade and soft intersections. No `FX_ADDFOG`:
  opacity uses alpha directly and colour is saturated after fog, as brine.
- Thermal reactor electricity: 57/375, the control-room two-texture UV
  deformation program with Fresnel, no near or soft fade.
- Lava falls: 34/352, world-space triplanar projection/deformation and
  soft intersections, brightness-based opacity, no near-camera fade.
  The additional `FX_ALPHAPOW` term raises the starting alpha (soft fade
  × vertex alpha × 2 × `_Color.a`) to `_AlphaPow`, using log/exp, before
  texture multiplication and day/night strength. The saved lava material
  has `_AlphaPow = 6`.
- Orange key symbol: 5/323, the one-texture brightness-opacity program
  without scrolling. `_MainTex_Speed` is stored but absent from the
  program's bindings: without `FX_SCROLL`, saved speeds are not read.

All request the blend and depth state already used by our WBOIT pass.
`_MyCullVariable` is independent pass state: 0 draws both faces, 2 culls
back faces. The elevator's dots/smoke, control-room inner blocks and orange
symbol use 2. Our custom effect buffers retain their original indices;
fragment-facing rejection accounts for reflected model transforms, as in
brine. Front culling (1) is still rejected. The loader accepts only the
14 individually confirmed exact keyword sets; it does not assume arbitrary
combinations work. The remaining lit sets require their actual light inputs
to be decoded before selection (M7g4e).

Original-game visual comparison: **not tested**. Existing camera-local fog
and refraction-sign limitations above still apply.

**Terminal startup selection — confirmed (2026-10-13):** the installed
`PrecursorKeyTerminal.Start` IL assigns `keyMats[acceptKeyType]` to
`keyFace.material`. A purple-key terminal can therefore store the material
named `Precursor_Symbol_Orange` in its prefab and replace it on startup.
Rendering the stored material alone does not reproduce its runtime appearance.
Our loader does not implement this selection yet. A human screenshot at
the Blood Kelp cache also shows the symbol detached from the terminal body;
optimized-avatar handling is an additional missing requirement (animation.md).

### Control-room UV deformation without soft fading (M7g4c) — confirmed

Confirmed from the installed shader's D3D11 vertex/fragment metadata and
programs, dumped freshly during M7g4b: vertex/fragment blobs 17/335 for
`FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL WBOIT`, and 82/400 when
`FX_REFRACTMAP` is also present. Rust `sn-inspect prefab --props` checks
the Gun control-room column and disable-gun terminal. Dumps remain ignored.

Both sets use the door's UV deformation and colour chain: twice material
colour times vertex colour, both scrolling main textures and local day/night
strength. Deformation samples its independent ST/scroll coordinates, then
adds `(map.xy − 1) × _DeformStrength` to the base UV before the main and
refraction maps use their own ST/scroll. Cutoff tests the resulting colour
alpha. Opacity is the saturated mean of RGB times that alpha, followed by
fog-path fading and saturation; WBOIT weighting is unchanged.

There is no scene-depth sampler or `_InvFade` constant in these fragment
programs: no soft-intersection fade. Their vertices have no near-clip alpha
fade. Saved `_InvFade`, `_ClipOffset` and `_ClipFade` therefore do not enable
either fade. Depth occlusion is still the material's Less test. The refracting
set uses the same packed-map unpacking, scalar `_RefractStrength`, colour
alpha and blue fog extinction as the door's refraction output. It does not
multiply the offset by brightness opacity or WBOIT weight.

The sampled materials request the existing effect blend/depth state and
cull off. Declared defaults confirmed with UnityPy: main, second main and
deformation textures white; deformation strength 0.2; refraction strength
0.1; refraction texture bump. Our shader already implements these formulas,
so M7g4c only selects the two exact sets with deformation on and soft fading
off. Missing/failed active refraction maps are rejected rather than using
the current white fallback in place of the declared neutral bump. Missing
main/second/deformation maps continue using white.

Rigid animated effect meshes retain their animated-node parent and identity
local transform, matching ordinary object meshes. The first M7g4c run
exposed that the old effect path placed these meshes independently in world
space. This was corrected before the final checks; scripted activation is
still **not tested**.

The terminal's stored colour is zero, so its selected effect is invisible
until something changes that colour. Original-game scripted activation and
matched visual comparison are **not tested**. Camera-selected fog settings
and unchecked refraction Y sign retain the earlier limitations.

### Tree Cove unlit brine (M7g4a) — confirmed from compiled programs

The exact keyword set `FX_DEFORM FX_MULMAP FX_NEARCLIP FX_SCROLL
FX_SOFTEDGES FX_TRIPLANAR WBOIT` uses a different colour/opacity path from
the additive holograms. Confirmed by the installed shader's first D3D11
vertex/fragment pair for this set and `sn-inspect prefab ... --props` on
the Tree Cove water prefab; research outputs remain ignored under `out/`.

- Projection uses Unity world position, with yz, xz and **yx** texture
  coordinates. Weights are proportional to the fourth power of the world
  normal components. The inverse-transpose normal is normalized per vertex;
  interpolation is not normalized again before weighting. Constant factors
  cancel when the three weights are normalized.
- Each map has an independent `_TriplanarUV.xyz` scale and `_Speed.xyz`
  scroll, with the fractional time-times-speed added after scaling. Texture
  ST offsets do not participate. The blended deformation map's RGB minus
  one, times `_DeformStrength`, displaces world position before both main
  maps are projected. This is not SIG's projection formula.
- Vertex alpha is multiplied by the saturated ratio of eye depth minus
  `_ClipOffset` minus camera near distance to `_ClipFade + _ClipOffset`.
  Soft scene intersections still use `_InvFade`. Colour and alpha multiply
  both main maps, vertex colour, twice the material colour, and the local
  day/night strength. Alpha cutoff precedes fog.
- Water fog is still evaluated although `FX_ADDFOG` is absent. Opacity is
  ordinary alpha, without the hologram brightness multiplier. Colour and
  alpha are saturated after fog, before weighted transparency accumulation.
  Colour saturation is in the game's light units.
- This material requests back culling (`_MyCullVariable = 2`); the two
  prefab surfaces supply their own orientations. Blend/depth settings match
  the earlier five variants. Our pass rejects back-facing fragments for this
  exact variant and continues rejecting unsupported render states.

Our implementation reuses the existing mapping slots for triplanar scales,
retains all three scroll components, and restores Unity's world Z coordinate
before projection. Explicit texture gradients permit conditional triplanar
sampling. The Bones Field/Ghost Tree saved-keyword combination is supported
by M7g4b as the same unlit program (see below).
Fog-volume selection still uses camera settings; original-game visual parity
is **not tested**.

### Lost River saved lighting keywords (M7g4b) — confirmed

Confirmed on 2026-10-12 by a fresh UnityPy metadata and D3D11 program dump
of the installed `UWE/Particles/UBER`, plus Rust `sn-inspect prefab --props`
on Ghost Tree lake and Bones Field cave pool prefabs. Research dumps are
ignored under `out/m7g4-brine/`.

The two lake material families save the M7g4a keyword set plus
`FX_LIGHT_2SIDED FX_LIGHT_NORMALMAP`. Neither keyword exists in any of the
shader's 636 vertex or 636 fragment metadata entries. Both materials have
`FX_LightMode = 0`, back culling, and the same blend/depth settings as Tree
Cove. The compiled lighting selectors are `FX_LIGHTMODE_VERTEX`,
`FX_LIGHTMODE_PIXEL` and `FX_FORCELIGHT_VERTEX`; none is saved on these lakes.

Removing the two inactive saved keywords selects vertex blob 39 and
fragment blob 357, the exact Tree Cove pair. The fresh fragment disassembly
matches the previous dump by SHA-256. Its samplers include main, second main,
deformation, scene depth and fog-volume textures; no normal map. The surface
colour chain has no lighting term. A saved normal texture and its properties
therefore do not justify adding normal-mapped lighting to this program.
Runtime keyword changes in the original game are **not tested**; no claim
of matched game imagery follows from this static program finding.

Our loader accepts only this exact additional saved set and uses the
existing brine pass. It does not globally discard unknown keywords. Actual
lighting selectors, partial saved sets and unsupported render states remain
rejected. The earlier description of these lakes as requiring lit shader
variants was an incorrect hypothesis. Camera-based fog-volume selection
and original-game visual acceptance remain open.

### Triplanar rocks (`UWE/SIG Triplanar with Capping`) — confirmed

- Textures `_CapTexture`, `_CapBumpMap`, `_CapSIGMap`, `_SideTexture`,
  `_SideBumpMap`, `_SideSIGMap`; no `_MainTex`. 976 placements, mostly in
  the Safe Shallows and the Crash Zone.
- Before M7g5: no `_MainTex`, so flat white. M7g5 explicitly loads the cap
  and side albedos and normal maps and draws their world-space projections.

### World-object SIG surfaces (M7g5) — confirmed from compiled programs

Read-only D3D11 deferred/HDR program inspection, with texture and constant
bindings from the installed shader objects, confirms the following. Shader
property defaults were checked separately from their serialized declarations.
Disassembly and property inspection stay in ignored `out/m7g5/`.

`UWE/SIG` uses UV coordinates, each map's own scale/offset, and DXT5nm normal
packing. Albedo is `_MainTex.rgb` times the linear `_Color`. With `UWE_SIG`,
the SIG map's red multiplies linear `_SpecColor`, green multiplies the tinted
albedo and `_EmissionScale` for emission, and blue supplies gloss (minimum
0.01). Without that keyword, raw albedo red and blue supply specular strength
and gloss; emission is zero. `_EnableSIG` alone does not select the compiled
variant. The declared SIG map default is **black**, not Marmoset's white.
Both object families are opaque, back-face culled, and output alpha one.

For capping, cap UV is world XZ times `_CapScale`; side UVs are YZ/XZ/YX
times `_SideScale`. The normalized projection weights, slope/alpha cap blend
and normal-map projection frames agree with terrain's documented formulas.
The geometric normal is the interpolated world normal; the surface shader
also converts the blended mapped normal to tangent space and back. The
object's inner border tint uses the terrain border formula at weight one,
but there is no terrain-layer alpha blend. Albedo includes the cap/side
tints and then the inner border tint. Gloss is `_Gloss`.

Its specular rule differs from terrain: multiply the final tinted albedo's
**red** by the interpolated cap/side specular colour. When the side weight is
exactly zero, the compiled branch uses `_SpecColor` rather than
`_CapSpecColor`; the client preserves this edge case. No inspected capping
program samples `_CapSIGMap`/`_SideSIGMap`, and the HDR emission target is
zero. Declared SIG/emission properties therefore must not enable invented
glow on these rocks.

The client shares its own normal unpacking, projection frames and border
function through `triplanar.wgsl`; terrain sampling and its layer blending
are unchanged. The object path uses existing game lighting, fog and shadows.
Global Marmoset sky changes must not reclassify SIG surfaces as Marmoset.
Missing capping albedo textures are counted explicitly; the white error
fallback is not claimed to reproduce the shader's named green/yellow defaults.
Matched original-game rock/coral images remain **not tested**.

### Precursor door force fields — confirmed

`Precursor_Prison_MainDoor_LockedForceField` has an active node
`x_ForceField` (mesh `FXPlane`, material `precursor_doorway_portal`) with
its renderer enabled, yet the material is not in the census. So the prefab
is **not placed** in the world's batch or cell data, and we don't draw it.
The game must spawn it at run time (how is not traced yet). The force field
we do draw is `x_PrecursorWaterForceField` (8 placements, `UWE/Particles/UBER`).

### UBER materials without textures — not a fault on its own

33 UBER materials (305 placements) have no `_MainTex`, e.g.
`submarine_engine_01_01` (82 MarmosetUBER placements), glass
(`exosuit_glass_crashed_03`, `vending_machine_glass`, …), some with only
`_Lightmap` or `_Illum`. Without a texture Unity binds the property's
declared default: MarmosetUBER declares `_MainTex`, `_SpecTex` and `_Illum`
`white` and `_BumpMap` `bump` (confirmed: its parsed properties, read with
UnityPy), as our defaults do. They then show `_Color`. Glass
(`_EnableSimpleGlass`) belongs with the object shader (M8c7).
