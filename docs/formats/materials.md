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
  How the shader shapes the glow is **not decoded yet**. The keyword names
  suggest a Fresnel fade at the silhouette, soft edges against the depth
  buffer, a fade near the camera and fog on top: **hypothesis**.
- Us: alpha-blended (queue ≥ 2500) with alpha 1 and our diffuse lighting,
  so in practice solid white shapes. The game's pass adds (`Blend One
  One`), does not write depth and draws both sides: confirmed from the
  shader's state.

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

33 UBER materials (305 placements) have no `_MainTex`, e.g.
`submarine_engine_01_01` (82 MarmosetUBER placements), glass
(`exosuit_glass_crashed_03`, `vending_machine_glass`, …), some with only
`_Lightmap` or `_Illum`. Without a texture Unity binds the property's
declared default: MarmosetUBER declares `_MainTex`, `_SpecTex` and `_Illum`
`white` and `_BumpMap` `bump` (confirmed: its parsed properties, read with
UnityPy), as our defaults do. They then show `_Color`. Glass
(`_EnableSimpleGlass`) belongs with the object shader (M8c7).
