# subnautica-rs — Design

Status: **M1–M8a done** (2026-10-07). Everything after M8a is a plan, not code.

## 1. Goal

A modern, lightweight, open-source Rust engine for **desktop** that:

1. Takes the path to a player's legally owned Subnautica install.
2. Reads the game's own data at runtime (terrain octrees, Unity asset bundles,
   world entity caches) — the repository never contains any of it.
3. Streams the ocean world around the player and meshes the voxel seabed.
4. Connects to a self-hosted server so friends can explore together, using a
   server-authoritative, simulation-ownership model inspired by Nitrox.

Non-goals (for now): a browser/WASM build, reproducing every gameplay system,
mod-loader compatibility, wire compatibility with Nitrox itself, audio (see §7).

The browser was dropped on 2026-10-06 to keep scope manageable. The layering
below still keeps core logic free of engine/OS dependencies (for headless
testing), which incidentally leaves the door open, but we do not build, test,
or design for WASM.

## 2. What we found in the install

Surveyed `D:\SteamLibrary\steamapps\common\Subnautica` (build `10`, built
2025-10-03). Relevant data, all under `Subnautica_Data/`:

| Path | What it is | Format | Used from |
|---|---|---|---|
| `StreamingAssets/SNUnmanagedData/Build18/CompiledOctreesCache/*.optoctrees` | Terrain SDF, 5,416 batch files, ~1.2 GB | Custom binary (documented below) | M1 |
| `.../Build18/index.txt` | World dimensions, octree size, batch size, one value per batch | Text | M1 |
| `.../Build18/meta.txt` | `39` / `BlockPrefabs` — meaning unknown | Text | M2/M6 |
| `.../Build18/biomeMap.bin`, `biomes.csv` | 2D biome map + biome names | Binary / CSV | M7 |
| `.../Build18/CellsCache/*.bin`, `BatchObjectsCache/*.bin` | Placed world entities per batch/cell | Likely protobuf-net serialized GameObjects | M8 |
| `StreamingAssets/SNUnmanagedData/clipmaps-*.json`, `streaming-*.json` | Original LOD / streaming parameters | JSON | M4 |
| `StreamingAssets/aa/` (`catalog.json`, `StandaloneWindows64/`) | Unity Addressables catalog + bundles (prefabs, meshes, textures) | UnityFS bundles | M5–M8 |
| `resources.assets`, `sharedassets0.assets`, `globalgamemanagers*` | Unity serialized files | Unity SerializedFile | M5 |
| `StreamingAssets/*.bank` | FMOD Studio sound banks | FMOD | deferred |
| `Managed/Assembly-CSharp.dll` | Game logic (.NET) | — | **reference only, never copied** |

### 2.1 Terrain: `.optoctrees`

Fully specified in [formats/optoctrees.md](formats/optoctrees.md); world
layout in [formats/world-index.md](formats/world-index.md). Summary of what M1
confirmed on all 5,416 files:

- World 4096 × 3200 × 4096 voxels = 128 × 100 × 128 octrees of 32³ = batches of
  5 × 5 × 5 octrees (edge batches 3 wide). +Y is up.
- File: `i32 version (4)`, then per octree `u16 count` + `count × {u8 type,
  u8 density, u16 first_child}`; `first_child == 0` marks a leaf, otherwise the
  8 children are consecutive.
- Child order and octree order are both **ZYX** (z fastest, x slowest),
  confirmed by seam continuity.
- `density` is a quantised signed distance with the surface between 125 and
  126 (confirmed world-wide in M2). Missing batches are uniform.
- Still open: `type` → material (M6); voxel → world offset is plausible but
  not confirmed (M3).

## 3. Architecture

### 3.1 Principles

1. **Read in place, at runtime, in Rust.** No Python extraction step in the
   player's path. UnityPy / AssetStudio are used only on the developer's machine
   as *oracles* to compare against, never as dependencies.
2. **Layered crates, enforced.** Format parsing, meshing, simulation rules and
   the network protocol are plain Rust libraries that run headless. The engine
   (Bevy), windowing, GPU, and sockets live at the edges.
3. **Bytes in, data out.** Parsers take `&[u8]`. They never open files, spawn
   threads, or panic on malformed input. File access lives in one crate
   (`sn-install`).
4. **Testable without the game.** Synthetic fixtures are generated in code
   (every parser crate ships a matching encoder for tests). Real-data tests are
   opt-in via `SUBNAUTICA_DIR`.
5. **Server-authoritative multiplayer from the start of networking.** Once
   Phase D lands, even solo play runs against a local in-process server.

### 3.2 Repository layout

```
subnautica-rs/
├─ Cargo.toml                 workspace
├─ AGENTS.md  README.md  MODLOG.md  .gitignore (whitelist)
├─ docs/
│  ├─ DESIGN.md               this file
│  └─ formats/                prose specs of every format we decode
├─ crates/
│  │  ── layer 0: foundation ──
│  ├─ sn-core/                coordinate types (voxel/octree/batch/cell/world), ids, errors
│  │  ── layer 1: data access & formats (pure, headless) ──
│  ├─ sn-octree/              .optoctrees decode/encode, voxel sampling, LOD by depth
│  ├─ sn-world/               meta/index, batch/cell addressing, biome map, entity caches
│  ├─ sn-unity/               UnityFS bundles, SerializedFile, type trees, Addressables,
│  │                          Texture2D (BCn) / Mesh decoding → engine-neutral structs
│  │  ── layer 2: algorithms & rules (pure, headless) ──
│  ├─ sn-terrain/             batch + neighbours → meshing field → mesh (chunk seams)
│  ├─ sn-mesh/                SDF → triangles (surface nets → dual contouring), chunking,
│  │                          seam stitching, per-LOD simplification
│  ├─ sn-sim/                 shared gameplay rules: swim movement, inventory, ownership
│  ├─ sn-protocol/            message types + versioned binary codec, no sockets
│  │  ── layer 3: OS, engine & IO edges ──
│  ├─ sn-install/             find the install (--game-dir, SUBNAUTICA_DIR, Steam
│  │                          library scan), check build number, read/mmap files
│  ├─ sn-net/                 UDP transport (reliable + unreliable channels)
│  └─ sn-render/              Bevy plugins: terrain material, water/fog, camera, debug UI
└─ apps/
   ├─ sn-inspect/             headless CLI: inspect, validate, export to out/ (OBJ/PNG)
   ├─ sn-client/              Bevy desktop game client
   └─ sn-server/              headless dedicated server (reads the host's own install)
```

Dependency direction is strictly downward:
`apps → layer 3 → layer 2 → layer 1 → layer 0`. Layers 0–2 forbid Bevy, wgpu,
winit, tokio, `std::fs`, `std::net`.

Crates are created only when a milestone needs them. M1 created
`sn-octree`, `sn-world` and `apps/sn-inspect` (which reads files itself until
`sn-install` is needed). M2 added `sn-mesh`. M3 added `sn-terrain` (layer 2: batch + 26
neighbours → field → mesh, pure), `sn-install` (layer 3: finding the install
and reading files) and `apps/sn-client` (Bevy). M5 added `sn-unity`; M6 added
`sn-assets` (layer 3: bundle index, cross-file references, terrain materials). Rendering code lives in
`sn-client` until a second app needs it; then it moves to `sn-render`.

### 3.3 Key decisions

| Decision | Choice | Why | Revisit if |
|---|---|---|---|
| Platform | **Desktop only** (Windows first; Linux/macOS best-effort, unverified) | Browser roughly doubles the hard problems (file access, threads, networking, memory) for little gain now | Desktop game is playable and someone wants a web port |
| Engine | **Bevy** (latest stable at M3) for rendering, input, ECS, asset tasks | All-code engine, strong ecosystem; precedent in benilla/gang-beasts-rust | Perf or upgrade churn becomes a blocker → raw wgpu + winit |
| Terrain meshing | **Surface nets first** (simple, robust), **dual contouring** later for sharp features | Get pixels quickly; DC needs Hermite normals we must derive from the SDF | — |
| LOD | Clipmap rings around the camera; coarse LOD = octree truncated at shallower depth | The octree *is* a free mip chain; mirrors the game's own `clipmaps-*.json` | — |
| Game data access | `sn-install` reads files; parsers only see bytes | One place for path logic, mmap, and the "install is read-only" rule | — |
| Networking transport | **UDP** with reliable/unreliable channels (evaluate `renet` vs `quinn` vs own at M10) | Standard for real-time desktop games; no browser constraints | NAT traversal pain → relay/QUIC |
| Replication | Own small protocol in `sn-protocol`; evaluate `bevy_replicon`/`lightyear` at M10 | Keep the protocol Bevy-free so server and tests stay simple | Own protocol becomes a time sink |
| MP model | Nitrox-style: server owns persistent world state; each entity has one *simulation owner* (usually the nearest player) who simulates and broadcasts it; interest management by batch/cell | Proven for this exact game; scales to a handful of players | — |
| Physics | Defer choice to M9 (`avian` vs `rapier`); terrain colliders from the same meshes | — | — |
| Licence for our code | Proposed **MIT OR Apache-2.0** (Rust convention) | — | **Needs user decision** |

### 3.4 Data flow (target state)

```
sn-install (game dir) ─▶ sn-world (which batches exist, where)
                      ─▶ sn-octree (batch bytes → octrees → voxel SDF at LOD n)
                            ─▶ sn-mesh (chunk SDF → mesh + material ids)   [worker threads]
                                  ─▶ sn-render (GPU upload, terrain material from sn-unity textures)
                      ─▶ sn-unity (Addressables → prefab → mesh/texture) ─▶ sn-render
sn-client ⇄ sn-net (UDP) ⇄ sn-server   (sn-protocol messages; sn-sim rules on both sides)
```

## 4. Roadmap

Each milestone is small, independently verifiable, and ends with a "Done when"
checklist that is run, not assumed. Later milestones will be refined when we
reach them — expect the far end of this list to change.

### Phase A — Terrain, headless → visible

| # | Goal | Done when |
|---|---|---|
| **M0** | Repo bootstrap: AGENTS.md, whitelist .gitignore, this design. | `git status` lists only our docs; guides folder and game data are ignored. |
| **M1** ✅ | Decode every terrain octree in the install (see §5). | Synthetic unit tests + opt-in full-world decode test pass; spatial-coherence check passes. |
| **M2** ✅ | Turn one batch into a mesh: settle density/type semantics, octree ordering, surface nets, export OBJ to `out/`. | Mesh is watertight across the 125 octrees of a batch **and across batch seams** (no seam gaps by edge count); opens in Blender showing recognisable seabed (human check — done, user: "looks ok"). |
| **M3** ✅ | `sn-client`: Bevy window, free-fly camera, loads a fixed 3×3×3 block of batches around the Lifepod start, flat-shaded by type id. | 60 fps on dev machine; frame-time and triangle counts logged. |
| **M4** ✅ | Streaming + LOD (done as: per-batch levels 0–3 by point-sampling the octrees, skirts for cracks, worker queue, LRU batch cache; see MODLOG): background meshing on worker threads, clipmap rings, load/unload as the camera moves, cross-LOD seams hidden (skirts or stitching). | Fly from Safe Shallows to the Crater Edge with bounded memory; logged load latency per batch. |

### Phase B — Unity content

| # | Goal | Done when |
|---|---|---|
| **M5** ✅ | `sn-unity`: read UnityFS bundles + SerializedFiles, list objects (`sn-inspect unity`). The Addressables catalog moved to M7b, its first user. | Object counts/types for `resources.assets` and N bundles match UnityPy on the dev machine. |
| **M6** ✅ | Textures + terrain look: decode Texture2D (BC1/3/5/7; detect crunch), map octree type ids → terrain materials, triplanar shader. | Terrain textured; decoded texture hashes match UnityPy output for a sample set. |
| **M6b** ✅ | Soft blending between terrain materials the way the game does it: per-chunk material layers (ordered by `VoxelandBlockType.layer`, then type id), per-vertex weights from adjacent faces, alpha from the weight and the texture's splotch (`_BorderBlend*`), cap/side transition, border tint. | Side-by-side screenshots; no visible voxel-grid edges at material borders. |
| **M7a** ✅ | Entity placements, headless: read `prefabs.db` (ClassId → prefab path), `BatchObjectsCache` and `CellsCache` (length-prefixed protobuf object trees, see `docs/formats/entities.md`) with our own wire reader in `sn-world`; `sn-inspect entities` per batch and `--all`. | Every cache file parses to its last byte with 0 errors; every ClassId resolves through `prefabs.db` (count of misses logged); entity counts per batch stable across runs; world positions inside (or logged next to) their batch. |
| **M7b** ✅ | Prefab → mesh: Addressables catalog (path → bundle), prefab hierarchy (GameObject, Transform, MeshFilter, MeshRenderer, LODGroup) and Unity `Mesh` decoding in `sn-unity`; `sn-inspect prefab <path>` exports OBJ to `out/`. | Vertex/index counts of a sample of meshes match UnityPy; exported coral/rock prefabs open in Blender (human check). |
| **M7c** ✅ | Spawn static entities (rocks, coral, flora) with the terrain batches in `sn-client`, albedo + normal maps only (the game's object shader is ported later). Plan: a worker thread reads a batch's baked cells once, resolves prefabs (cached) and sends instances plus new meshes/materials; the main thread shows cell level *n* while the batch's terrain level of detail is ≤ *n* (level 0 within ~100 m, level 3 out to 1.2 km; our choice, the game's distances are not in its data files). Look: `_MainTex` × `_Color`, `_BumpMap` (DXT5nm, own shader extension), alpha clip (`MARMO_ALPHA_CLIP`, `_Cutoff`) and blending (render queue ≥ 3000); spec/emission later. | Entity counts per batch logged and stable; Safe Shallows shows coral/rocks in place; frame time logged. |
| **M7d** ✅ | Spawn slots: the 90,289 `EntitySlotsPlaceholder` objects in the cells are filled by the game when a cell first loads, from its loot/flora distribution. Plan (`docs/formats/entities.md` § Spawn slots): parse each placeholder's slots (biome, allowed types, density, local position/rotation) in `sn-world`; read the distribution (`Balance/EntityDistributions`, a JSON text asset) and `WorldEntities/WorldEntityData` (ClassId → slot type, Z-up, cell level, scale) in `sn-unity`/`sn-assets`; fill with the game's rule (`probability / density`, only allowed slot types, the rest of the probability left empty, `count` copies within 4 m) using our own RNG keyed by (seed, placeholder id, slot index), so the result does not depend on load order; `sn-inspect slots`; the client spawns the fillers with the cell objects at their cell level (creatures skipped, as for placed objects), `--slot-seed`, `--no-slots`. Not ported: the PDA's known-fragment filter (nothing is known in a new game) and spawn restrictions (no `spawnrestrictions-*.csv` in this install). **Finding:** the slots hold no plants; they fill with creatures (102,777 at seed 1, not drawn), resource outcrops (35,808), eggs, fragments and a few tools; the missing small vegetation is not here (see M7e). | Slot counts per biome logged; ~~filled vegetation visible~~ filled outcrops visible in Safe Shallows (there is no slot vegetation); the same seed gives the same world. |
| **M7e1** ✅ (first-pass look) | Terrain grass, placement and meshes: the 60 block types with `hasGrassAbove` and a grass mesh (`VoxelandBlockType`: density, tilt range, scale range, jitter, spin, Z-up, Perlin placement) scatter their grass mesh over their faces as the game's `VoxelandGrassBuilder` does: per face 4 sub-quads; the face normal within the tilt range; a random draw (or Perlin noise over x/z) against the density; the tuft at the sub-quad centre plus jitter, turned from up to the face normal, spun, Z-up turned, scaled; per 16-voxel chunk at most 10,000 vertices and 10,000 triangles over all types (the game raises the reduction when a type would exceed its share, ×0.8); vertex colour random RGB, alpha = height along the face normal ÷ 5. One merged mesh per (batch, type), no shadow casting. Pure code in `sn-terrain::grass` (unit tests); our faces are our surface-nets quads, not Voxeland's (same unit of 1 voxel, positions differ in detail); our own seeded RNG; Unity's `Mathf.PerlinNoise` replaced by our Perlin noise (**hypothesis**: same range and period). Shown on our LOD 0 batches (≈ the game's level 0, reduction 0; the game's level 1 with reduction 0.5 waits for M4b). First-pass look: our object material (`_MainTex` × `_Color`, alpha cut at `_Cutoff`, `_BumpMap`, double-sided). `sn-inspect grass <X> <Y> <Z>`, `--no-grass`. | Tuft and vertex counts per batch and type logged and stable across runs; chunk budget never exceeded; screenshots of the grassy plateaus and the Safe Shallows; frame time A/B with `--no-grass`. |
| **M7e2** ✅ (matched screenshots open: user) | The grass shader `UWE/SIG Terrain Grass` (decoded like the others), and the object shader for the coral-deco grass types whose materials are MarmosetUBER (types 52, 76, 83, 251: specular and glow maps): the two tints by vertex colour, the bottom colour gradient by height (`_BotColor`, `_GradientParams`), the world-space mask (`_Mask`, `_MaskScale`, `_MaskStr`), the SIG map, waves (`_WaveAmount`, `_WaveSpeed`, `_WaveUpMin`). | Every material property explained in `docs/formats/`; matched screenshot with the game. |
| **M7e3** | Grass placed tuft for tuft as in the game (deferred 2026-10-09 by the user's review of M7e). Today the amount and spread follow the game's rules but each tuft lands elsewhere. Read from the game's code (`VoxelandGrassBuilder`, `VoxelandChunk.EnumerateGrass`/`GrassPos.ComputeTransform`): its random numbers are `Unity.Mathematics.Random` (xorshift) from `VoxelandMisc.CreateRandom(seed)` with seeds `offsetX·9999 + offsetY·999 + offsetZ·99 (+ randSeed·9)` per chunk (`randSeed` = the block type id), a separate `reductionRng` from `randSeed`, and the draw order `NextDouble` for spin, two for jitter, `NextFloat` for scale, `NextByte` ×3 per vertex for the colour. Port that generator and seeding; port `Mathf.PerlinNoise` (Unity's native code: needs its exact definition, **hypothesis** that it is classic Perlin; verify against values taken from the game); give faces to chunks as the game does (by block, `offset = cellId × 16`) and enumerate them in the game's face order (`VoxelandChunkWorkspace.faces`), which needs Voxeland's own faces rather than our surface-nets quads (positions differ in detail). Also: grass beyond our LOD 0 (the game's clipmap level 1 with reduction 0.5) comes with M4b. | Tuft positions in one chunk equal to positions read from the running game (method to find; e.g. a screenshot pair at a fixed spot) — **not possible today**; counts per chunk equal to the game's formula; the same world on every run. |
| **M7f** | Scenes and special objects, in three steps below (plan written 2026-10-09). How the game loads them (read from its code and scene data, `docs/formats/unity.md` § Scenes): the main scene loads `Essentials` (`MainGameController.additionalScenes`), whose `LightmappedPrefabs.autoloadScenes` are `Cyclops` (kept as a template, not spawned), `EscapePod` and `Aurora` (both spawned: the scene's `__LIGHTMAPPED_PREFAB__` root is put at the origin and activated); the other top-level objects of those scenes stay where the scene has them. | Each step's own list. |
| **M7f1** ✅ (not compared with the game) | Scene reader and the Aurora scene: `sn-assets::Scene` (the scene's serialized file, one hierarchy per top-level object, world placements), `sn-inspect scene <name>` (class counts, top-level objects, drawn nodes, script census). The `aurora` scene drawn always (not streamed with the batches), most detailed LOD: the Aurora in the state of a new game, intact (`CrashedShipExploder`: its `disableOnExplosion`/`enableOnExplosion` objects swap 2.3–4 game days × 1,200 s + 27 s after the start; `--aurora exploded` shows the other state), and the scene's other top-level objects (Precursor prison exterior and aquarium, Lost River base, Lost River large trees: the game never streams these). Not yet: LOD levels by distance, the exterior cull manager, effects (fire, smoke, radiation). | Object counts per scene logged; real-data test of the counts and the explode lists; the Aurora visible at its place (screenshot for the user); frame time with and without it. |
| **M7f2** ✅ | Skinned meshes (`SkinnedMeshRenderer`: 73 placed prefabs, e.g. `BrainCoral` LOD 0, and the lifepod's hull): read the renderer (materials, mesh, bones, root bone) and the mesh's bind poses and bone weights; skin on the CPU at load time in the pose the hierarchy stores (the game's `Animator` poses them; animation comes later). | Unit test of the skinning on synthetic bones; skinned prefabs counted in the log; a skinned LOD 0 has the bounds of its static LOD 1 (within a few %) on a sample. Done: the three prefabs with both match within 0.1 %; bone-less skinned renderers (blend shapes) are drawn as plain meshes; blend shapes not applied. |
| **M7f3** | Lifepod 5: the `escapepod` scene placed as `EscapePod.ChooseRandomStart` does: `RandomStart.GetRandomStartPoint` draws x, z in ±2,048 m with y = 0 until the `validStartPointTexture` pixel there has green > 0.5 (the game's draw uses Unity's unseeded `Random`, so any valid point is the game's behaviour; ours is seeded, `--lifepod <X> <Z>` to choose); the camera starts at its player spawn point; the pod's spawned modules (`AddressablesPrefabSpawn`: fabricator, radio, medical cabinet, …) in place. Not yet: floating on the waves (`WorldForces`, `Stabilizer`; comes with physics, M9), the intro's damage effects. | Start point and the share of valid texture pixels logged; the pod and its modules visible (screenshot); the same seed gives the same start. |

### Phase C — Being underwater

| # | Goal | Done when |
|---|---|---|
| **M8a** ✅ | Water data, headless: the biome map (`biomeMap.bin`, `biomes.csv`), each batch's override biome (`LargeWorldBatchRoot`), and the per-biome water settings (`WaterBiomeManager.biomeSettings` in the main scene: absorption, scattering, murkiness, emissive, sunlight/ambient scale, …); `sn-inspect biomes`. | Every biome name in the map and the overrides has settings (misses listed); the biome at a few known places (lifepod: safe shallows, …) is right; values logged. |
| **M8b** (fog + sky values in code) | Underwater look: the game's water fog (extinction/scattering per biome at the camera, Henyey–Greenstein phase, depth-attenuated sunlight, emissive; decoded from the compiled fog shader, `docs/formats/water.md`) as a full-screen HDR pass. Done: the sky system's values (`uSkyManager`/`uSkyLight`: time of day → sun direction and colour, top ambient, sky fog). Next: `AtmosphereVolume` shapes (caves, wreck interiors); calibrate light units against screenshots of the game at the same places. | Side-by-side screenshots with the game (lifepod, coral spot, Kelp Forest, a deep biome) taken by the user; GPU time of the pass logged (`--gpu-timings`). |
| **M8c1** (first pass) | Water surface, ported from the game (`WaterSurface` in the main scene, its four shaders decoded, `docs/formats/water.md` § Water surface), "Medium" water quality (the default: baked waves, no FFT): the 64 baked frames `WaterFrame00…63` (bundle `waterdisplacement`, 256² RGBA8) played over the scene's `sequenceLength` (5 s), linearly interpolated on the GPU into a 512² displacement map each frame, its normal map (with mips) and the accumulated foam amount, as the game does; a surface mesh around the camera at y = 0 (dense within the 200 m where waves fade out, flat beyond); the surface shader drawn after the fog pass, reading the fogged image (refraction) and depth: from above sky reflection (the game's mean sky colour until M8c2 brings the sky map), refraction, Fresnel, sub-surface back light, sun glint, foam, sky fog; from below total internal reflection (deep-water fog colour) and refraction out to the sky, with the water fog up to the surface. Not in M8c1: the clip map (water cut out of bases/the Aurora, shore foam), screen-space reflections (off in the scene). | Real-data test: frames and `WaterSurface` values read; numbers logged (displacement range, foam coverage); GPU time of the water passes logged; screenshots from above and below for the user. |
| **M8c2** (first pass) | Sky: port of the game's uSky skybox and sky map (scattering, sun disc, planet and corona, night sky, moon, clouds; `docs/formats/sky.md`), drawn behind the scene before the fog; the water reflects the sky map. Not yet: stars. | Screenshots with the game above water at two times of day. |
| **M8c3** (first pass) | Light on surfaces: the game's deferred lighting ported into our terrain/object shaders (`docs/formats/lighting.md`): caustics (64 frames, 25 fps), sunlight attenuated under water with the colour cast, top/bottom ambient, the water's glow, Blinn specular (terrain specular colours). Next: per-pixel water settings, sun shadows, object specular/gloss/emission maps, light shafts, stars. | Side-by-side screenshots with the game at the lifepod, coral spot, Kelp Forest; perf budget logged. |
| **M8c4** (first pass) | Water "High" quality (the user's setting): port of `WaterDisplacementGenerator` (Phillips spectrum + FFT compute shaders, the scene's wind/amplitude/choppiness) replacing the baked frames; `--water-quality medium|high`. Then the water's screen-space reflections (`ENABLE_SCREEN_SPACE_REFLECTION`, if the scene enables it). | Side-by-side screenshot of the open sea with the game; GPU time logged. |
| **M8c5** ✅ (first pass) | Light shafts under water (`WaterSunShaftsOnCamera` and its shaders), stars (`StarField`). | Screenshots with the game at the lifepod by day and at night. |
| **M8c6** (first pass) | Sun shadows: the game's cascades (4 over 50 m, `QualitySettings` High), soft; only the nearest level of detail casts; the light shafts read the shadow map. Remaining: the game's bias (0.45) and normal bias (0.4), Unity's soft-shadow filter instead of Bevy's Gaussian. | Matched screenshot of a shadow edge; frame time logged. |
| **M8c7** (first pass) | The object shader, MarmosetUBER (`docs/formats/lighting.md` § Objects): specular maps, gloss, fresnel, glow with day/night strengths, each biome's Marmoset sky (exposures, SH ambient, unlit flag), `SkyApplier`. Remaining: the sky's specular cube reflections, atmosphere volumes in the biome lookup (cave skies), anchors other than Auto. | Real-data test of the 37 skies; materials with specular/glow maps counted in the log; matched screenshots. |
| **M8e** | Local lights (point and spot), the game's light sources: see § 4.1. | See § 4.1. |
| **M8d** | The game's camera post-processing (Unity Post Processing Stack v1 with `default_Post-FXProfile`, per the user's options): bloom + lens dirt, ambient occlusion, screen-space reflections, depth of field, motion blur, FXAA, dithering, colour grading (off/neutral/ACES exactly as the stack does it). | Matched screenshots (same place and time) with the game; GPU time per effect logged. |
| **M9** | Player: swim controller, terrain collision, surfacing/air, first-person camera. | Can swim from the Lifepod to the Kelp Forest without clipping through terrain (logged collision checks). |

### 4.1 Lighting plan (written 2026-10-08)

**Why:** our scenes look flat next to the game's. The biggest missing
pieces, from the game's data:
1. **Local lights.** 263 world prefabs carry Unity `Light` components,
   about 2,290 lights: 1,793 point, 486 spot, 11 directional (counted with
   a throwaway reader on the dev machine; layout to be confirmed by a
   test). Typical range 6 m (up to 150 m), intensity 2. A `Lights` folder
   of placed lights per biome (e.g. Lost River 28, Precursor 22), 16 of 17
   glowing coral prefabs (`Coral_reef_Light`), floating stones, creepvine
   seed clusters, wrecks, alien bases, tools (flashlight, flare). This is
   the glow the user sees on the sand around glowing plants. Only 24 of
   them cast shadows.
2. **Bloom** (the halo around bright things), **ambient occlusion**.
3. **Reflections** of the sky's cube map on shiny objects.
4. Smaller: shadow bias and filter, cave skies, per-pixel water settings.

**Approach for local lights:** we light surfaces forward in our own
shaders (`game_light.wgsl`), not with Bevy's PBR. Lights become Bevy
`PointLight`/`SpotLight` entities only so that Bevy culls and clusters
them; our shaders walk Bevy's cluster light lists and apply the game's
formula. The game's deferred point-light program (`POINT`, disassembled):
falloff = `_LightTextureB0` sampled at `distance² / range²`, × max(n·l, 0)
× light colour; Blinn specular with the G-buffer's power, as the sun. To
check in the HDR, `SPOT`, `POINT_COOKIE` and shadow variants: the unlit
flag, spot cone/cookie, water attenuation (none in the `POINT` program).

**Order of work** (each step ends with numbers, screenshots for the user,
and a MODLOG entry; nothing is pushed without the user's OK):

| Step | Work | Done when |
|---|---|---|
| **M8e1** ✅ | Read `Light` (type, colour, intensity, range, spot angle, cookie, shadows, render mode, culling mask, enabled) in `sn-unity` with tests on synthetic bytes; collect lights per prefab node in `sn-assets`. Census in `sn-inspect`. | Real-data test: light counts per type and shadow mode equal to UnityPy's on a sample of prefabs; census logged. |
| **M8e2** (programs decoded; falloff curve open) | Decode the HDR `POINT`, `SPOT`, `POINT_COOKIE` light programs and the falloff texture `_LightTextureB0` (from the game's built-in resources, or the Unity version's formula if it is generated, then checked against the data); document in `lighting.md`. | Falloff curve values logged; every constant of the programs explained in the doc. |
| **M8e3** ✅ (falloff curve still a hypothesis; the one placed spot with its own cookie moved to M8e4) | Spawn lights with their objects (same streaming, same levels); port the point and spot formula into `game_light.wgsl` for terrain and objects; spots with their cookies. | Lights on screen at the lifepod and in a glowing-coral area counted; light at known distances checked against the formula in a unit test; GPU time logged; screenshot at night. |
| **M4b** | The game's own streaming distances: the terrain clipmap of `clipmaps-high.json` (the user's "Detail" High; 5 levels, `chunksPerSide` 7/7/7/8/1 of 16-voxel chunks, level scale and ring placement to read from the Voxeland clipmap code), entity cell level *n* awake with clipmap level *n* (`entities` on for levels 0–3), terrain casting sun shadows at levels 0–1 only (`castShadows`), `maxBlockTypes` 32/8/2/1/1. Replaces our own LOD ranges (100/260/600/1200 m). This decides which objects and lights exist around the camera, so it comes before the shadowed lights. | Ring extents per level logged and equal to the game's formula; entity and light counts at the lifepod logged before/after; frame time A/B. |
| **M8e4** | The 24 shadowed lights (cube/spot shadow maps, the game's resolution and bias), and the one placed spot light with its own cookie (it is also shadowed; drawn with the default cookie for now). | Screenshot of a shadowed local light; GPU time logged. |
| **M8d1** | Bloom and lens dirt as the game's post stack does them (`default_Post-FXProfile`: intensity 0.2, threshold 0.9, soft knee 0.55, radius 5.5, dirt 5); ambient occlusion with the profile's settings. | Matched screenshots by night (glowing coral) and day; GPU time per effect. |
| **M8c7b** | Specular cube reflections (load the skies' cube maps), atmosphere volumes in the biome lookup (cave and wreck skies), other `SkyApplier` anchors. | Cube maps loaded and counted; skies per object in a cave logged; matched screenshot of a shiny object. |
| **M8c6b** | Sun shadows of terrain, rocks, coral and plants exactly as the game: read each renderer's `m_CastShadows`/`m_ReceiveShadows` (`MeshRenderer`) and honour them; find whether and how the game's terrain (Voxeland chunks) casts and receives; alpha-clipped leaves cast cut-out shadows; the game's bias/normal bias (0.45/0.4) and Unity's soft-shadow filter; the 50 m shadow distance and cascade splits (done). | Counts of casting/non-casting renderers logged and equal to UnityPy on a sample of prefabs; terrain shadow settings documented; matched screenshots of a rock's shadow and a kelp shadow; GPU/CPU cost logged. |
| **M8f** | Per-pixel water settings (the game's volume around the camera) for fog and lighting; the water clip map. | Screenshot across a biome border; numbers logged. |
| **M8g** | Calibration: matched screenshots (same place, time, settings) with the game at the lifepod, a glowing-coral spot at night, the Kelp Forest, a cave, a wreck; differences measured (mean colour per region) and listed. | Every listed difference explained or fixed. |

**Performance budget:** the dense start area must stay under 16.7 ms
(60 fps) on the dev machine (RTX 3080) with all of the above. Measure each
step with `--benchmark` A/B against the step before (same build flags,
three runs each).

### Phase D — Multiplayer

| # | Goal | Done when |
|---|---|---|
| **M10** | `sn-protocol` + `sn-net` + `sn-server`: handshake (protocol + game build check), join, player transform sync over UDP. | Two clients on one machine see each other as capsules; packet loss/latency injected in tests. |
| **M11** | Nitrox-style entity sync: simulation ownership + handoff, server persistence, pick up / drop an item. | Item state survives server restart; ownership handoff covered by headless tests. |
| **M12** | **Playable multiplayer desktop demo:** two+ players spawn in Safe Shallows, swim the textured, populated seabed, see each other, pick up items, chat. | A friend on another machine joins over the internet using their own install. |

Beyond M12 (unordered): creatures + AI, inventory/crafting, PDA, vehicles,
habitat building, day/night, save import, audio, mod support, maybe a web port.

## 5. Milestone 1

> **M1: `sn-inspect` decodes every `.optoctrees` batch in the player's
> Subnautica install into 32³ voxel grids of (type, density) and reports
> per-batch and whole-world statistics, backed by synthetic unit tests and an
> opt-in test that decodes all 5,416 batches without a single error.**

### Scope

- Workspace `Cargo.toml`; crates `crates/sn-octree` and `crates/sn-world`
  (libs, no deps) and `apps/sn-inspect` (bin, no deps).
- `sn-octree`:
  - `Batch::parse(&[u8]) -> Result<Batch, ParseError>` (version, octrees, nodes);
    errors carry byte offsets.
  - `Octree::validate()` — child indices in bounds, no cycles, each non-root
    node referenced exactly once, leaves tile the 32³ cube exactly.
  - `Octree::rasterize() -> VoxelGrid32` — expands leaves to 32,768 `(type, density)` voxels.
  - `encode()` — writes our own synthetic octrees, used by unit tests (round-trip).
- `sn-inspect`:
  - `--game-dir <path>` or `SUBNAUTICA_DIR`; locates `SNUnmanagedData/Build*`.
  - `sn-inspect index` — prints world/octree/batch dimensions from `index.txt`.
  - `sn-inspect octree <x> <y> <z>` — node counts, depth histogram, type histogram,
    solid/empty voxel counts, coherence check against a scrambled control.
  - `sn-inspect orient <x> <y> <z>` — finds child order and octree order from
    seam continuity (added during M1; see below).
  - `sn-inspect octree --all` — decodes every batch, prints totals and timing.
- `docs/formats/optoctrees.md` with confirmed facts vs hypotheses.

Out of scope for M1: density semantics, meshing, rendering, Bevy.

### Done when — all met 2026-10-06 (evidence in MODLOG.md)

1. `cargo test --workspace` passes on a machine **without** Subnautica.
2. `SUBNAUTICA_DIR=… cargo test -p sn-octree -- --ignored` decodes all 5,416
   files: all version 4, 5,259 × 125 + 157 × 75 octrees, every octree validates
   and rasterizes to exactly 32,768 voxels.
3. `sn-inspect octree 12 18 12` node and type histograms match the independent
   probe recorded in the MODLOG (e.g. 24 distinct type ids, 47,194 nodes of type 172).
4. **Orientation check** (validates the node layout without looking at
   anything): `sn-inspect orient` finds one child order × octree order
   combination whose octree and batch seams are as continuous as the octree
   interiors, consistently across several detailed batches.
   *Revised during M1:* the original criterion ("≥ 0.95 of adjacent voxel pairs
   agree, for each of 8 orderings") was flawed. Within one octree every axis
   order scores the same (it only transposes the octree), and even a scrambled
   order scores 0.985 because most of the volume is uniform. Seams are what
   tell orders apart.
5. `cargo clippy --workspace -- -D warnings` is clean.
6. `git status --short` shows only source and docs.

## 6. Testing strategy

| Level | Where | Needs game? |
|---|---|---|
| Unit + round-trip (encode → parse) | each layer 0–2 crate | No |
| Property / fuzz tests on parsers (`proptest`, later `cargo-fuzz`) | layer 1 | No |
| Real-data invariants (`#[ignore]`, `SUBNAUTICA_DIR`) | layer 1–2 | Yes |
| Oracle comparisons vs UnityPy output (dev-machine only, results not committed) | `sn-unity` | Yes |
| Headless sim/protocol tests incl. simulated latency & loss | `sn-sim`, `sn-protocol`, `sn-server` | No |

## 7. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Asset bundles have type trees stripped — **confirmed in M5: no file has them** | Can't decode any object generically | Built-in classes (Texture2D, Mesh, Material, …): hand-written readers for the Unity 2019.4 layouts, checked against UnityPy. MonoBehaviours: derive layouts from `Assembly-CSharp.dll` metadata at runtime (our own reader); never commit the DLL or generated dumps |
| Crunch-compressed textures — **not used by this game (M6 census)** | — | `texture2ddecoder` could decode it anyway |
| Entity caches are protobuf-net with game-specific schemas | M7 slips | Build schemas incrementally; count-only validation first |
| 1.2 GB of terrain | Memory/IO pressure | Stream and evict by batch; mmap files; keep only meshes + nearby octrees resident |
| Game updates change formats | Breakage | Handshake checks build number; format docs record the build they describe |
| Internet play behind NAT | Friends can't connect | Document port forwarding first; relay/QUIC later |
| FMOD audio | No sound | Deferred; FMOD banks are non-trivial and runtime licensing needs checking |
| Scope | Never finishes | Strict milestones; README lists what works and what doesn't |

## 8. References (read, don't copy)

- Nitrox (multiplayer architecture: server authority, simulation ownership,
  batch-cell entity spawning) — copyleft; architecture reference only.
- Subnautica-TerrainPatcher (terrain batch format notes).
- UnityPy, AssetStudio (Unity bundle/SerializedFile structure; dev-time oracle).
- OpenMW, hl2-rs, benilla, gang-beasts-rust (project structure and hygiene).
- `fast-surface-nets`, `isosurface` crates (meshing; evaluate at M2).

## 9. Open decisions for the user

1. Licence for our code (proposed: MIT OR Apache-2.0).
2. Confirm Bevy as the client engine.
3. Confirm: our own protocol (not Nitrox wire-compatible).
