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
  with no scrolling, second texture, deformation or refraction. Not
  decoded yet: `FX_NEARCLIP`, `FX_TRIPLANAR`, the lighting modes,
  `FX_LIGHT_NORMALMAP`, and every other combination (the Lost River lakes,
  sand and lava falls, the Gun's elevator and deactivation column, the
  water force field, the tech light cones). Differences: the fog uses the
  camera's water settings, not the fog volume at the mesh (as for the
  glows); the refraction offset's sign in y is **not checked** (Unity's
  render-texture uv may run the other way; the force fields' maps are
  noise, so it does not show).

### Triplanar rocks (`UWE/SIG Triplanar with Capping`) — confirmed

- Textures `_CapTexture`, `_CapBumpMap`, `_CapSIGMap`, `_SideTexture`,
  `_SideBumpMap`, `_SideSIGMap`; no `_MainTex`. 976 placements, mostly in
  the Safe Shallows and the Crash Zone.
- Us: no `_MainTex`, so flat white. The terrain's own shader is also
  triplanar (`docs/formats/terrain-materials.md`); whether the two share
  their formula is **not checked**.

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
