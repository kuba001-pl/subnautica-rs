# Materials and the shaders they use

What the world's materials ask for, and which of them our object shader
(`apps/sn-client/src/object.wgsl`, a port of MarmosetUBER:
`docs/formats/lighting.md` § Objects) cannot draw as the game does.
Everything here comes from `sn-inspect prefab --materials` (2026-10-09, game
build 10): every material on a node we draw at full detail
(`Prefab::visible`) in the 1,369 placed prefabs and the two startup scenes
we spawn (`aurora` intact, `escapepod` at the origin), counted per drawn
node and per placement in the world.

## Shader names — confirmed by a string scan, hypothesis for the rest

A material points to its `Shader` object (class 48). We don't parse that
class; the census finds the name as a length-prefixed string that looks like
`Group/Name` inside the shader object. For the shaders below the scan gives
one clear name. For MarmosetUBER (the 35 MB shader object) and three smaller
shaders it finds none (their names have no `/`, e.g. `MarmosetUBER`):
**hypothesis** from the properties (`_Shininess` + `_GlowStrengthNight`).
Reading `m_ParsedForm.m_Name` properly would settle this (plan: DESIGN.md
§ 4.2, M7g2).

## Census of drawn materials — confirmed from the data

1,973 materials on drawn nodes. Grouped by shader (drawn nodes, placements,
materials):

| Shader | Nodes | Placements | Materials | Examples |
|---|---:|---:|---:|---|
| (MarmosetUBER, by properties) | 34,907 | 258,914 | 1,903 | almost everything |
| `UWE/SIG` | 54 | 11,095 | 13 | `Coral_reef_small_deco_06/09/21`, stalactites, lava rocks |
| `UWE/Particles/UBER` | 169 | 1,058 | 31 | Lost River brine lakes and waterfalls, Precursor terminal screens and halos, lava fall, sand fall, smoke column, tech light cones |
| `UWE/SIG Triplanar with Capping` | 15 | 976 | 5 | `SS_Prop_SandtoRock` (Safe Shallows rocks), `SS_Prop_Coral02/03/08` (coral clumps) |
| `UWE/Particles/WBOIT-FakeVolumetricLight` | 68 | 239 | 3 | `x_AtmoLight_Sphere`, `x_AtmoLight_Cone` |
| `Unlit/DepthOnly` | 39 | 40 | 1 | `DepthOnly` on `Occluder_*_shell` nodes |
| `Legacy Shaders/Diffuse` | 8 | 8 | 2 | sand in two abandoned bases |
| `UWE/Experimental/Blinn Phong - All Maps` | 4 | 4 | 2 | the Sea Emperor baby models in the prison |
| `UWE/Marmoset/IonCrystal` | 4 | 4 | 2 | ion crystals |
| `UWE/Marmoset/Mesmer` | 1 | 8 | 1 | Mesmer |
| `UWE/SIG Terrain Grass`, `UWE/SIG Transparent Waving`, `UWE/SIG-SandDrift`, `FX/WBOIT-WaterBase` | 1 each | 1–2 | 1 each | coral grass, aquarium sand drift, the Gun's moon pool water |
| (no name found, 3 shaders) | 7 | 79 | 6 | `Fire Smoke` (geysers), coral grass, two default materials |

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
  How the shader shapes the glow is **not decoded yet**. The keyword names
  suggest a Fresnel fade at the silhouette, soft edges against the depth
  buffer, a fade near the camera and fog on top: **hypothesis**.
- Us: alpha-blended (queue ≥ 2500) with alpha 1 and our diffuse lighting,
  so in practice solid white shapes.

### `UWE/Particles/UBER` meshes — confirmed

- 31 materials, all in render queue 3101 (3000 for `x_flashlightCone`),
  textures `_MainTex` + `_MainTex2`, some with `_DeformMap`, `_NormalMap`,
  `_RefractMap` (the Lost River lakes). Their colours carry the glow (e.g.
  the terminal halos are green, the lakes' alpha is 0.471).
- Us: alpha-blended `_MainTex` × `_Color`, with no scrolling, second
  texture, deformation, refraction or additive blend.

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

33 UBER materials (332 placements) have no `_MainTex`, e.g. `submarine_engine_01_01` (109
placements), glass (`exosuit_glass_crashed_03`, `vending_machine_glass`, …),
some with only `_Lightmap` or `_Illum`. Without a texture MarmosetUBER
should sample the property's default, which we take to be white, as we
do (**hypothesis**: the shader's defaults are not read). They then show
`_Color`. Glass
(`_EnableSimpleGlass`) belongs with the object shader (M8c7).
