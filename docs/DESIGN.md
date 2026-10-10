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
│  ├─ sn-dotnet/              .NET assemblies (PE, metadata, IL): data the game keeps
│  │                          only in code (TechType names, craft menus, defaults)
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
| **M7f3** ✅ (not compared with the game) | Lifepod 5: the `escapepod` scene placed as `EscapePod.ChooseRandomStart` does: `RandomStart.GetRandomStartPoint` draws x, z in ±2,048 m with y = 0 until the `validStartPointTexture` pixel there has green > 0.5 (the game's draw uses Unity's unseeded `Random`, so any valid point is the game's behaviour; ours is seeded, `--lifepod <X> <Z>` to choose); the camera starts at its player spawn point; the pod's spawned modules (`AddressablesPrefabSpawn`: fabricator, radio, medical cabinet, …) in place, on the mount points `MoveAndRotateWithTransform` puts them (stored pose). Not yet: the camera at the player's eye height (it stands at `playerSpawn`), the pod's own sky (`MarmoLifepodSky`) and lights (`LightingController`), the pod's animation, floating on the waves (`WorldForces`, `Stabilizer`; comes with physics, M9), the intro's damage effects. | Start point and the share of valid texture pixels logged; the pod and its modules visible (screenshot); the same seed gives the same start. |
| **M7f4** | Everything M7f1–M7f3 left different from the game (recorded 2026-10-09; nothing here is 1:1 yet). **Aurora and scenes:** always the most detailed LOD (the game's `LODGroup`s switch to LOD 1/2 with distance; same for the pod); the explosion is a flag (`--aurora`), not timed from the game clock (`timeToStartCountdown` = start + 2.3–4 days × 1,200 s, swap 27 s later); `ShipExteriorCullManager`/`CullExplodedExterior` (exterior hidden from inside) and the 316 `CullingOccludee`s not ported; fire, smoke, radiation and explosion effects (particle systems, `VFXController`), sounds; scene renderers all cast sun shadows (their `m_CastShadows` not read; M8c6b). **Skinned meshes:** shown in the pose the hierarchy stores, not animated (`Animator`, e.g. the pod's `lifepod_damage` blend, creatures); blend shapes read past, not applied (e.g. `BrainCoral` LOD 0 shows its base shape); normals of bones with non-uniform scale are moved by the linear part, not its inverse transpose (Unity's GPU skinning: **hypothesis** it does the same). **Lifepod 5:** our seeded draw instead of Unity's unseeded `Random` (the point differs per run in the game anyway); fixed at y = 0 (the game floats it with `WorldForces`/`Stabilizer` around its anchor and the waves; needs physics, M9); camera at the `playerSpawn` transform, not the player's eye height or initial look direction; interior lit by the outside: the pod's own sky (`MarmoLifepodSky`, `SkyEscapePod`), its 5 lights driven by `LightingController` (red alert in the intro) and the `AtmosphereVolume` not used; intro state not shown (damage effects, fire, smoke, birds: the `Manual` spawners); spawned modules' own scripts not run (e.g. nested spawners, storage contents from `SpawnEscapePodSupplies`, the screen UI); `MoveAndRotateWithTransform` applied once, not every frame. | Each item ported or confirmed equal by a matched screenshot, number or unit test; this row emptied. |
| **M7g** | Objects whose materials are not MarmosetUBER, drawn as the game does: the occluder shells hidden, fake volumetric lights, mesh effects, triplanar rocks: see § 4.2. | See § 4.2. |

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
| **M9** | Player: swim controller, terrain collision, surfacing/air, first-person camera. Split into M9a–M9f in § 4.3 (Phase E). | Can swim from the Lifepod to the Kelp Forest without clipping through terrain (logged collision checks). |

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

### 4.2 Materials the object shader doesn't cover (plan written 2026-10-09)

**Why:** the user sees invisible walls in the alien bases, solid white
spheres and cones, and untextured objects. `sn-inspect prefab --materials`
(`docs/formats/materials.md`) shows the cause: of the 1,975 materials we
draw, 72 use other shaders than MarmosetUBER. Three of them are its
IonCrystal and Mesmer variants, which we take as UBER; `material_desc`
draws the other 69 as plain `_MainTex` × `_Color`. Those with no `_MainTex`
come out flat white.
1. **Occluder shells:** 39 `Occluder_*_shell` nodes (`Unlit/DepthOnly`,
   layer 27 `Occluder`) in 38 Precursor rooms. The game never draws them in
   the picture: its main camera's culling mask (`0x65ffff17`) leaves layer
   27 out, and only `CullingCamera` draws them, into its occlusion depth
   texture. We draw them as opaque white shells.
2. **Fake volumetric lights:** 68 nodes, 239 placements (`x_AtmoLight_Sphere`
   on ion crystal pedestals, `x_AtmoLight_Cone` under Precursor lights),
   shader `UWE/Particles/WBOIT-FakeVolumetricLight`. We draw them as solid
   white blended meshes (alpha 1).
3. **Mesh effects** (`UWE/Particles/UBER`): 31 materials, 169 nodes, 1,058
   placements: Lost River brine lakes and waterfalls, Precursor terminal
   screens and halos, lava and sand falls, tech light cones. We draw them as
   plain blended textures.
4. **Triplanar rocks** (`UWE/SIG Triplanar with Capping`): 976 placements
   of Safe Shallows rocks and coral clumps, drawn white (no `_MainTex`).
   `UWE/SIG` (11,095 placements, coral deco, lava rocks) has a `_MainTex`
   but its own maps are not used.
5. Small: `Legacy Shaders/Diffuse` (8 nodes), `Blinn Phong` (the Sea
   Emperor babies, 4), `FX/WBOIT-WaterBase` (the Gun's moon pool, 1), SIG
   grass/waving/sand drift (3), geyser smoke (2), IonCrystal and Mesmer
   (UBER variants: their extra properties not read).

**What the earlier research got wrong** (checked 2026-10-09): the sphere
and cone lights are drawn alpha-blended, not opaque (it looks the same, as
their alpha is 1). The door force field `precursor_doorway_portal` is not
drawn at all, because its prefab is not placed in the world data (it is
spawned at run time). It missed the `UWE/Particles/UBER` meshes and the
triplanar rocks, which are larger groups than the spheres.

**Approach:** tell the shader by its **name** (read from the material's
`Shader`), not by property fingerprints, and give `MaterialDesc` a shader
kind. Each kind is drawn by its own port. Until a kind is ported it stays
drawn as now (the user's decision, 2026-10-09: hidden effects get
forgotten), and the client logs it at start as "shader not ported: N nodes
per kind", so the gap is visible in every run. Shaders are decoded
from their compiled programs as for MarmosetUBER (`lighting.md` § Objects),
and the findings written up in `docs/formats/materials.md`.

**Order of work** (each step ends with numbers, a screenshot for the user
and a MODLOG entry; nothing is committed or pushed without the user's OK):

| Step | Work | Done when |
|---|---|---|
| **M7g1** ✅ (screenshot too dark to compare: the Lava Castle base at −1,192 m has no sunlight and its own lights are not drawn as the game does) | Camera culling mask: read `Camera` (class 20: culling mask, near/far, field of view) in `sn-unity`, with a test on synthetic bytes; nodes keep their layer (they do: `PrefabNode::layer`). `sn-assets` gives the main camera's mask from the `main` scene; the client does not draw nodes whose layer the mask leaves out. No name matching: the rule is the game's. | Unit test of the layout; real-data test: `MainCamera` mask `0x65ffff17`, UI `0x20`; the client logs "skipped by layer: 39 nodes (40 placements)", all `Occluder_*_shell`; screenshot inside the Gun's large hallway. |
| **M7g2** ✅ | Shader names: parse enough of `Shader` (class 48, `m_ParsedForm.m_Name`, layout from UnityPy's 2019.4 type tree) to name every material's shader; `ShaderKind` in `MaterialDesc` (UBER, UBER variant, FakeVolumetricLight, Particles UBER, SIG, SIG triplanar, DepthOnly, other); kinds not ported yet stay drawn as now and are counted. Replaces the string scan in `sn-inspect prefab --materials`. Done: `sn_unity::Shader` (parsed form with passes and render state; programs walked over); the client keeps the property fingerprint for UBER (it picks exactly the UBER shaders, checked on all materials) and classifies by name for the log. | Every one of the 1,975 drawn materials gets a name (none unknown); all 363 shaders of the game parse; 17 used shaders equal to UnityPy; the client logs each unported shader when first drawn and the totals when loading settles. |
| **M7g3** built 2026-10-09, all but the side-by-side with the game (decoded in `docs/formats/materials.md` § Fake volumetric lights; the pass below; formulas tested; GPU 0.21 ms at a pedestal; the screenshots `out/m7g3/pedestal-final.png` and `out/m7g3/cone.png` are **not compared with the game yet**) | `UWE/Particles/WBOIT-FakeVolumetricLight`: decode the compiled shader (its keywords `FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_SOFTEDGES FX_SCROLL`, blend mode, properties), document it, port it as a transparent pass (soft edges need the depth texture). How the game's WBOIT (weighted blended order-independent transparency) composites is part of the decode; if we can't match it, write down the difference. | Formula unit-tested on known inputs; every constant explained in `materials.md`; screenshot of a pedestal and a Precursor spotlight next to the game's; GPU time logged. |
| **M7g4** (parts built 2026-10-09: the three variants of the Precursor consoles' holograms, `FX_ADDFOG FX_SCROLL WBOIT` ± `FX_MULMAP` ± `FX_FRESNELCLIP`, and the two of the door force fields, `FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT` ± `FX_REFRACTMAP`; decoded in `docs/formats/materials.md`; the client checks each material's keywords, blend, depth test and cull, logs the rest with the reason; extra materials draw the last sub-mesh again, as in Unity) | `UWE/Particles/UBER` meshes: decode the shader's variants used by the 31 materials (two scrolling textures, deform, normal and refraction maps, blend modes from the material), port them; the Lost River lakes are the biggest user. Remaining: `FX_NEARCLIP`, `FX_TRIPLANAR`, lighting modes, `FX_LIGHT_NORMALMAP` and the other combinations (lakes, sand and lava falls, the Gun's elevator and deactivation column, the water force field, tech light cones). | Variants used counted and each one decoded; matched screenshots of a Lost River brine lake and a Precursor terminal; GPU time logged. |
| **M7g5** | `UWE/SIG Triplanar with Capping` and `UWE/SIG`: decode both (cap/side textures, SIG maps), compare with our terrain shader's triplanar formula and share code where the formulas agree. | Rocks textured: the client counts no drawn material without a texture except UBER ones; matched screenshot of a Safe Shallows rock and a coral deco. |
| **M7g6** | The small rest (Legacy Diffuse, Blinn Phong, WaterBase moon pool, SIG grass/waving/sand drift, geyser smoke, IonCrystal and Mesmer properties): each ported or listed in M7f4 as not 1:1, with its count. | `materials.md` lists each with "ported" or "not ported"; no kind left unexplained. |
| later | `CullingCamera`'s occlusion culling with the occluder shells (a speed-up, not a visual change). | — |
| **M7h** (built 2026-10-09; a run at the Blood Kelp cache spawns 2,091 placeholders from 189 prefabs in the loaded area (mean frame 8.66 ms with, 8.24 ms without in one pair of runs; another pair, under load, 16.92 vs 16.95 ms); `--no-placeholders` turns it off; not compared with the game on screen) | Objects the game spawns from `PrefabPlaceholder`s when a prefab first starts (the cache doors with their force fields, the key terminals, the ion crystals on the pedestals; user report 2026-10-09 at the Blood Kelp cache). The game's rule (`PrefabPlaceholdersGroup.Start`, `PrefabPlaceholder.Spawn`): when the group's `isInitialized` is false (not saved in the world's cells: only the `Transform` is stored there, checked), each listed placeholder whose GameObject is active (`activeSelf`), with a `prefabClassId` that has a world entity info, spawns that prefab under the placeholder's parent with the placeholder's local transform; spawned prefabs start too, so their own placeholders spawn (nested). No spawn restrictions in this install. Read `PrefabPlaceholder` (`prefabClassId`, `highPriority`) and `PrefabPlaceholdersGroup` (`prefabPlaceholders`) in `sn-unity`; the client adds the spawned prefabs' parts and lights to the prefab that holds them (creatures left out, as for placed objects). | Unit tests of both readers; real-data test: the pedestal's crystal and the cache root's door and key terminal found with their class ids; the client logs placeholders spawned/skipped per prefab; screenshot at the Blood Kelp cache with the door and crystals. |
| **M7g open** (user review 2026-10-09 at the Blood Kelp cache; the values used are the game's, so they stay) | Concerns left after M7g3/M7g4/M7h, to come back to: (1) the spotlight cones are dimmer than in the user's game screenshot (green +11 vs +28 over the wall; geometry, direction and every term of the decoded formula checked; part of the gap is our brighter, bluer cave, the rest unexplained); (2) the Precursor pillars draw nearly black where the game shows them lit teal, probably the cache's atmosphere volume ambient (M8c7b), **not checked**; (3) the other `UWE/Particles/UBER` variants (Lost River lakes, sand and lava falls, the water force field, the Gun's elevator and deactivation column, tech light cones) keep the stand-in look (logged, M7g4); (4) the refraction offset's y sign is not checked against the game. Also: effect fog uses the camera's water settings, not the fog volume at the mesh. | Each item compared with a game screenshot at the same spot, fixed or explained in `docs/formats/`. |

**Decided 2026-10-09 (the user): our own pass after the fog (A), for 1:1.**
The game draws the glows and effect meshes after its fog image effect, each
fogging itself at its own distance, into the WBOIT targets, and its `WBOIT`
image effect composites them after the sun shafts (`MainCamera`'s
components in order: fog, water surface, sun shafts, `WBOIT`, screen
effects). Ours, `effects.rs`:
1. The worker marks parts whose shader we draw this way and sends their
   meshes with vertex colours (the glow's falloff uses the vertex alpha);
   the main world spawns them as `EffectPart` entities (no Bevy material).
2. Extracted every frame (world and inverse matrices, the material's
   values); meshes uploaded once into our own buffers.
3. Accumulation into two half-float targets A, B cleared to (0, 0, 0, 1)
   (run just before the composite: the glows write nothing to the image
   itself, the game's camera target gets 0, so when they are accumulated
   doesn't matter as long as the scene's depth is complete), the game's blend (colour `One One`, alpha `Zero
   OneMinusSrcAlpha`), no depth write, both sides; the scene's depth is
   tested in the shader and gives the soft edges; fog from our water fog
   model at the effect's own distance.
4. The composite (`Hidden/WBOIT Composite`, no keywords) after the sun
   shafts, before tonemapping; skipped when nothing was drawn.
Not 1:1 yet, recorded: the game's fog volume textures (we use the
camera's water settings, as our fog pass does); temperature refraction
(`FX_TEMPERATURE_REFRACT`, above 40 °C), sonar and PDA composite variants.

### 4.3 Road to a playable game (plan written 2026-10-09, approved in outline by the user the same day)

**Why:** Phases B and C are roughly 30 look milestones. Most are first
passes that wait on matched screenshots only the user can take, and each one
changes less than the last. Nothing can be *played* yet. The user's goal is
Subnautica 1:1 in Rust, and that means the game, not only the picture. From
now on gameplay comes first, and the look work resumes once the game can be
played.

**Deferred, not dropped:** M7e3, M7f4, the rest of M7g4, M7g5, M7g6, "M7g
open", M4b, the remaining M8b/M8c items (cave atmosphere volumes, SSR,
M8c6b, M8c7b), M8d, M8d1, the M8e2 falloff curve, M8e4, M8f, M8g. Their rows
stay as they are. Also deferred from M9a (accepted differences until they
block play): the game's terrain collision thinning (`SimplifyMeshPlugin`,
native code our .NET reader cannot read; ours is the unthinned level 0
surface), checking Unity's collider scaling rules in the game (a
hypothesis from Unity's documentation), and PhysX rigid-body response
instead of our kinematic slide (waits for § 3.3's physics decision).
A deferred item is pulled forward only when it blocks play
(e.g. water fog drawn inside the lifepod once the player stands in it).

**Where the game keeps its gameplay data** (found 2026-10-09; details and
how each was checked in `docs/formats/gameplay.md`). Most of it is data we
can read at runtime like everything else:
- Recipes, craft times, craft amounts, item sizes, equipment slots, energy
  costs, harvest outputs: the JSON text asset `Balance/TechData` in
  `resources.assets`, read like `Balance/EntityDistributions` (M7d).
- Starting blueprints and unlock rules: `PDAData` (`defaultTech`,
  `compoundTech`, `analysisTech`), a serialized asset (where it is stored:
  not found yet).
- Prefab ↔ tech type: the `EntTechData` resource (serialized).
- Player numbers (oxygen capacity, suffocation times, …): serialized fields
  of the player's components (`Oxygen.oxygenCapacity`, `Player.*`).
- Item and UI text: `StreamingAssets/SNUnmanagedData/LanguageFiles/*.json`.

Only a few things exist **only in the game's code** (`Assembly-CSharp.dll`):
the fabricator menus (`CraftTree`: nested `CraftNode` constructors), the
names of the `TechType` enum values (the JSON stores numbers), and a handful
of defaults (`TechData.defaults`). We read them from the player's own DLL at
runtime with our own reader (user decision 2026-10-09): the .NET metadata
tables for the enum names, and a small IL reader that follows the
constructor calls in `CraftTree`. Nothing from the DLL is copied into the
repository. Behaviour (how oxygen drains, how the fabricator works) is
ported the way the shaders and the grass were: read the game's code to
understand it, then write our own.

**Rules for this road:**
- Gameplay rules go in `sn-sim` (layer 2, pure, tested headless); the client
  only reads input and draws. The server (M10+) runs the same code.
- Every number comes from the install at runtime. If a value cannot be
  found, our stand-in is labelled "not the game's" in the code and in the
  step's row.
- Each step lists what is not 1:1 yet, as M7f4 does. Committing and
  pushing still need the user's explicit OK each time.

**Phase E — First playable (single player, then co-op):**

| Step | Work | Done when |
|---|---|---|
| **P0** ✅ | Gameplay data, headless. **Done 2026-10-09** (MODLOG; facts in `docs/formats/gameplay.md`): TechData 463 entries, 0 errors; `PDAData` reached through the main scene's `Player`; player numbers from the main scene; collider census on placed prefabs (`sn-inspect prefab --colliders`). Not 1:1 yet: TechData defaults and tech type names wait for P1. Plan as written: Read `Balance/TechData` (JSON) and `EntTechData` in `sn-assets`; find `PDAData` and the player's prefab and read their fields; census of collider components on placed prefabs (`BoxCollider` 65, `SphereCollider` 135, `CapsuleCollider` 136, `MeshCollider` 64) and of `Pickupable` / `BreakableResource`. `sn-inspect techdata`, `sn-inspect player`. | `docs/formats/gameplay.md` lists every fact with its source, *confirmed* or *hypothesis*; real-data test: TechData parses with 0 errors and the entry count is logged; every ingredient's tech type is also an entry or logged as a miss; collider counts per kind logged. |
| **P1** ✅ | **Done 2026-10-10** (MODLOG; facts in `docs/formats/dotnet.md`): all 21,383 method bodies of the game's DLL decode; 793 `TechType` names, every TechData entry named; 7 menus, 159 nodes (fabricator 100), every craft node has TechData; TechData's 17 defaults. The scheme reader also needed `newarr`/`dup`/`stelem.ref` (C# `params` arrays) and `ret`: still straight-line, no branches or locals. Plan as written: `sn-dotnet` (new crate, layer 1, pure): our own reader of .NET PE files: metadata tables (`TypeDef`, `Field`, `MethodDef`, `Constant`), string heaps, method bodies; `TechType` names from the enum's constants; the `CraftTree` menus by walking the IL of its tree methods (only the patterns used there: `ldstr`, `ldc.i4`, `newobj CraftNode`, `call AddNode`). Unit tests on synthetic bytes we encode ourselves (no game files as fixtures). **Stop and ask** if the IL needs more than a simple pattern reader. | Synthetic round-trip tests; real-data test: number of `TechType` names logged and every TechData entry has a name; the fabricator tree's node count logged; every craft node's tech type has a TechData entry. |
| **M9a** ✅ | **Done 2026-10-10** (MODLOG; facts in `docs/formats/gameplay.md` § Collision; plan below). `sn-sim::collide` passes 12 unit tests. `sn-inspect swim` reaches the Kelp Forest from the lifepod for seeds 1–5: 0 penetrations, smallest gap ≥ 5.1 mm, 1,860–3,693 contacts, about 21–31 µs mean and 60–120 µs p99 per step. The client doesn't use it yet (M9b). Plan as written: Collision: a kinematic capsule swept against triangles. Terrain from the LOD 0 meshes already built around the camera; objects from their prefabs' colliders (box, sphere, capsule, mesh; read in `sn-unity`). Our own sweep in `sn-sim`, no physics engine yet (§ 3.3's physics decision waits for rigid bodies: floating lifepod, dropped items). | Unit tests of the sweep on synthetic meshes (slide, corner, thin wall); a scripted swim lifepod → Kelp Forest logs 0 penetrations and the contacts; cost per frame logged. |
| **M9b** ✅ | **Done 2026-10-10** (MODLOG; facts in `docs/formats/gameplay.md` § Player movement; plan and "as built" below). `sn-inspect walk`: pod → hatch → 10 s swim (71 m) → back in, 0 penetrations, 0 surfaces passed through; speeds 3.5 walking and 7.22 swimming against 3.5 and 7.6 read (7.22 is 7.6 after one step of drag 2.5). Layer 19 hits all layers but 9; every loaded collider is on layer 0 (none dropped yet). 46 pod and module colliders; placeholder spawns 10–216 per run. Colliders are one-sided, as PhysX's (source read; winding measured). The client plays as the player by default. Keyboard and mouse play are not tested by the agent. Plan as written: Player: first-person camera at eye height, swimming, walking with gravity in the lifepod and above water, the lifepod hatch. Speeds from the player's serialized fields (P0). The fly camera stays as `--free-cam`. Collision gaps left by M9a that matter here: (1) read the physics layer collision matrix (`PhysicsManager`) and collide only with the layers the player's capsule hits; (2) colliders of the lifepod's spawned modules (fabricator, radio, …) and of what placeholders spawn (M7h), so walking inside the pod hits them; (3) find out whether the game's terrain and mesh colliders block from one side or both, and match it. | Movement rules unit-tested; speeds logged next to the values read; scripted run lifepod → water → lifepod (positions logged); the layer matrix logged and the colliders kept/dropped by layer counted; lifepod module and placeholder colliders counted in the run; one- vs two-sided recorded in `docs/formats/gameplay.md` with how it was checked. |
| **M9c** ✅ | **Done 2026-10-10** (MODLOG; facts in `docs/formats/gameplay.md` § Oxygen, health and death and § Mouse look; plan and "as built" below). `sn-sim::vitals` and `look` pass 14 unit tests; `sn-inspect dive` seeds 1–5: oxygen 0 at 42.26 s under water, death 8.02 s later, respawn in the pod 5.02 s, restore 1.02 s, controls 1.02 s; oxygen within one breath of the read rate; refill 0.56–0.74 s from surfacing; 0 penetrations. Look: 0.1125° per mouse count (sensitivity 0.15 from the DLL), pitch ±87° (scene; the code says ±80). The HUD and the look in play are not tested by the agent. Plan as written: Oxygen, health, depth: drain under water, refill at the surface and in the lifepod, suffocation → respawn in the lifepod, with the game's numbers. A minimal HUD of our own (bars and numbers; the game's UI sprites later). Also the mouse look as the game's (`MainCameraControl`: sensitivity, pitch limits, smoothing, read from its serialized fields and settings defaults), replacing our own values from M9b. | Rules unit-tested; a scripted dive logs oxygen over time against the values read; the look's values logged next to the ones read, its limits unit-tested. |
| **M10** | Multiplayer as planned (Phase D): `sn-protocol`, `sn-net`, `sn-server`, handshake with the build check, join, player sync. From here solo play also runs against an in-process server (§ 3.1, principle 5), so items and crafting below are written server-authoritative once. | M10's own row. |
| **M9d** | Pick up and inventory: `Pickupable` objects within reach, outcrops break into their resource (`BreakableResource`), inventory of the game's size, item sizes from TechData; picked objects gone for every player (server state keyed by entity id and slot seed). | Inventory rules unit-tested; a scripted pick-up logs item counts; the object's drawn count −1 on both clients. |
| **M9e** | Crafting at the lifepod's fabricator: the menu from P1's tree, recipes and times from TechData, starting blueprints from `PDAData`; item names from the language files. | Crafting rules unit-tested on synthetic recipes; one real recipe crafted in a scripted run (counts before/after logged); menu node count equal to P1's. |
| **M9f** | Save and load (player, inventory, removed objects, crafted items) on the server, in our own format, in the user's save folder (not `out/`, not the game folder). Becomes M11's persistence. | Round-trip unit test; server restart restores the logged state. |

#### M9a plan (written 2026-10-10)

**What the game does** (read in the decompiled code, to be recorded in
`docs/formats/gameplay.md` § Collision with each fact marked):
- Terrain collision exists only at the finest clipmap level:
  `clipmaps-high.json` level 0 has `colliders: true` and 7×7×7 chunks of
  `chunkMeshRes` 16 voxels, so about ±56 m around the player; levels 1–4
  have none. Each chunk's collision mesh (`VoxelandCollisionMeshSimplifier`)
  is made from the chunk's visible faces: corners 0, 2, 4, 6 of each face
  give two triangles; above 100 triangles and 100 vertices a native plugin
  (`SimplifyMeshPlugin`, ratio 0.8, chunk-border vertices fixed) thins it.
- Under water the player is a `Rigidbody` with a `CapsuleCollider`
  (`UnderwaterMotor`): radius `controllerRadius` 0.3, height
  `swimheight − cameraOffset` = 0.5 + 0.25 = 0.75, centre
  `−height/2 − cameraOffset` below the player's origin (the camera). On
  ground a `CharacterController` with `standheight − cameraOffset` = 1.75
  (M9b).

**Steps:**
1. **`sn-sim` (new crate, layer 2, pure, no dependencies): `collide`.**
   Our own small vector maths. Shapes: triangles (a mesh with a uniform
   grid for the broad phase), spheres, capsules, oriented boxes. A capsule
   (segment + radius) is swept by conservative advancement on exact
   distances (segment–triangle, segment–segment, segment–point,
   segment–box), so a thin wall cannot be tunnelled through.
   `move_and_slide` (up to 4 slides, 1 cm skin) returns the new position and
   the contacts; `clearance` measures the distance to the nearest shape (a
   penetration is clearance < −1 mm). Unit tests: slide along a floor and a
   wall, an inside corner, a thin one-sided wall at 100 m/s, each primitive,
   and a seeded random walk in a closed synthetic room that never
   penetrates.
2. **Game shapes in world space (`sn-assets::collision`).** Terrain: the
   LOD 0 batch mesh without skirts (`sn-terrain`), in Unity coordinates.
   Objects: every enabled, non-trigger collider on an active node of a
   placed prefab (the world's cells and batch objects, and the lifepod
   scene), with Unity's scaling rules (*hypothesis*, from Unity's
   documentation: box × per-axis scale; sphere radius × largest |scale|;
   capsule radius × the larger of the two other axes, height × its own
   axis) and mesh colliders as their mesh's triangles.
3. **`sn-inspect swim`: the scripted swim, headless.** Lifepod start for a
   seed (as the client), the nearest Kelp Forest point on the biome map,
   a path that dives from the lifepod towards it; terrain and objects
   loaded for the batches within 56 m of the player as it moves; 60 steps
   per second at the swim speed read in P0 (7.6 m/s). Logs: steps,
   contacts, penetrations, minimum clearance, distance left, cost per step
   (mean, p99) and per batch load.
4. The client uses this from M9b (the player); M9a changes nothing on
   screen.

**Not 1:1 yet** (where each is planned: the layer matrix and one- or two-sided colliders go to M9b; the thinning, scaling and rigid-body items are deferred, see "Deferred, not dropped" above): no `SimplifyMeshPlugin` (native code, not
ported; our terrain collision is the unsimplified LOD 0 surface, a little
finer than the game's); kinematic slide instead of PhysX's rigid body
response; the physics layer collision matrix (which layers the player
hits) not read yet: every non-trigger collider counts; convex mesh
colliders (1 in the census) use their triangles, not a hull.

**As built (2026-10-10), where it differs from the plan:** the swim aims
below the seabed (−150 m) so the capsule slides along terrain the whole
way. A fixed depth of −20 m gave 0 contacts and tested nothing. The target
is the nearest Kelp Forest map cell with Kelp Forest 20 m around it; the
nearest cell alone lies on the forest's edge. When the capsule is wedged
(seed 1: a 0.5 m slot between a terrain wall and an overhanging object),
the script detours sideways for 1.5 s, as a player would. Also not
loaded in the swim: what placeholders spawn (M7h), the lifepod's spawned
modules, creatures. Triangles collided on both sides; M9b made them
one-sided, as the game's.

#### M9b plan (written 2026-10-10)

**What the game does** (read in the decompiled code; each fact goes to
`docs/formats/gameplay.md` § Player movement, marked):
- **Which motor.** `PlayerController.HandleControllerState`: under water
  → `UnderwaterMotor` (a `Rigidbody` + `CapsuleCollider`, `FixedUpdate`),
  otherwise `GroundMotor` (Unity's `CharacterMotor` on a
  `CharacterController`). "Under water for swimming"
  (`Player.UpdateIsUnderwater`): inside the lifepod (`escapePod` flag)
  never; else against the ocean level (`Ocean.GetOceanLevel`, the `Ocean`
  object's y): swimming starts below level − 0.1 and lasts while below
  level + 0.1 when grounded (a ray down) or + 0.8 otherwise. Capsule
  height `swimheight` or `standheight` − `cameraOffset`, eased at 2 m/s
  when it grows, if there is room above.
- **Swimming** (`UnderwaterMotor.UpdateMove`): input direction in the
  camera's frame plus the up/down axis; max speed by direction (forward,
  backward, strafe, vertical: the largest that applies) and `AlterMaxSpeed`
  (tanks, fins, a held tool: none yet → unchanged; × 1.3 above water);
  the velocity gains `acceleration × dt` along the input and is capped at
  max(that speed, current speed); near the surface (from level − 0.5 to
  level + 0.52) the upward speed is scaled by
  clamp01((level + 0.52 − y) / 1.02)^0.3 unless diving; no gravity under
  water (`SetMotorMode` sets 0); the body's drag is `swimDrag`. At the
  surface, when blocked, it steps up onto land (1.85 m up, 0.3 m ahead,
  down to a walkable floor).
- **Walking** (`GroundMotor`): accelerate towards the input velocity (max
  speeds by direction; ground/air acceleration), gravity, jumping, sliding
  on slopes steeper than `slopeLimit`, step offset (pushed down by
  max(step offset, horizontal move) while grounded so it follows the
  floor), all through `CharacterController.Move`.
- **The hatch** (`UseableDiveHatch.OnHandClick`): from inside, the player
  goes to `outsideExit` and the `escapePod` flag clears; from outside, to
  `insideSpawn` and the flag is set. The game plays a cinematic between;
  its no-cinematic branch just sets the position, and that is what we do.
- **Collision mask:** the player's own capsule casts use `−524289`, all
  layers but 19; the rigid body collides by the layer matrix.

**Steps:**
1. **Data (headless).** Read `GroundMotor`'s own fields
   (`CharacterMotorMovement`, `…Jumping`, `…Sliding`, `…Controller`:
   step offset, slope limit); the hatch (`UseableDiveHatch`:
   `outsideExit`, `insideSpawn`, `isForEscapePod`) in the escapepod scene;
   the `PhysicsManager` (layer collision matrix, gravity, `queriesHitBackfaces`)
   and the `TimeManager` (fixed time step) in `globalgamemanagers`; the
   `Ocean` level. Move the swim's batch loader into `sn-assets` (terrain
   + object colliders per batch, filtered by the player's layer row) and
   add the lifepod's spawned modules and placeholder spawns. `sn-inspect
   player` prints the new numbers.
2. **`sn-sim::player` (pure).** State (position, velocity, motor, in the
   pod, grounded, capsule height), parameters (all from step 1), input
   (move axes, up/down, look yaw/pitch, jump, use). One fixed step: the
   motor choice, the swim rules, the walk rules (a `CharacterController`
   of our own: `move_and_slide` + step offset + slope limit + ground
   check), the hatch. Velocity loses its component into what it hit (a
   rigid body without bounce). Unit tests on synthetic worlds: swim speed
   reaches 7.6 and no more; drag stops; surface damping; enter/leave
   water switches motor at the game's thresholds; walking on a floor,
   up a 0.3 m step but not a 0.5 m one; sliding on a 60° slope; the
   hatch moves between its two points.
3. **`sn-inspect walk`: scripted run, headless.** Spawn at the pod's
   player spawn, walk to the hatch, leave, swim 10 s away and back,
   enter. Logs positions, motor changes, speeds next to the values read.
4. **Client.** The player is the default (`--free-cam` keeps the fly
   camera): first-person camera at the player's position, WASD, Space up
   / jump, C down, mouse look, E uses the hatch when it is within reach
   in front of the camera. Collision bodies stream on a worker thread
   (the same loader). Logs motor changes and the hatch.

**Not 1:1 yet (planned here):** no cinematic for the hatch (position
only); no animation of the body or the camera (bob, step smoothing);
PhysX's solver replaced by our slide + velocity clipping; tanks, fins and
tools don't change speeds yet (no inventory until M9d); the lifepod
does not float or move (M7f4); mouse sensitivity and look limits our
own until `MainCameraControl` is read (planned in M9c; done, see M9c's
"as built"). Where the rest
is planned: the cinematic, body and camera animation in "After Phase E"
item 3; tanks and fins in item 1; the physics solver in "Deferred, not
dropped".

**As built (2026-10-10), where it differs from the plan:**
- **The hatch.** The pod's `UseableDiveHatch` is on an inactive node and
  not used. The player goes in and out through the pod's 8 cinematic hand
  triggers (`CinematicModeTrigger`): E or a click within the hand's reach
  (`Targeting.GetTarget`: a 2 m ray, then 0.15 m and 0.3 m sphere casts)
  moves the player to the trigger's end point and sets the in-pod flag
  from its enter/exit call. The first use swaps the first-use triggers
  for their normal twins, as `EscapePodFirstUse`. The end points marked
  VR-only are used too: without the animation they are the only target
  we have.
- **One-sided triangles.** PhysX 4.1 culls back faces in capsule–mesh
  contacts, in mesh sweeps and (with `queriesHitBackfaces` false) in
  rays. `sn-sim` now does the same. Mirroring scales swap the winding
  back, as PhysX's `flipsNormal`. Because the clearance check no longer
  sees a surface from behind, both scripts also count the surfaces the
  capsule's centre passes through in a step (`World::crossings`); the
  runs fail if it is not 0. The swims got quicker (seed 1: 39.9 s, no
  detour, against 51.8 s): part of the 0.5 m slot that wedged it in M9a
  was back faces.
- **Shared pieces.** `sn-assets::collision_world`: the batch loader
  (terrain, objects and their placeholder spawns, layer rules), the
  lifepod's colliders and triggers, and the player's parameters, used by
  both scripts and the client. `sn-sim::collide` bodies are in groups
  (blocks movement, seen by the hand, or both) and gain `cast` for the
  hand. The client streams collision bodies on a worker thread (4 batches
  in about 2 s) and steps the player at the game's 50 Hz.

#### M9c plan (written 2026-10-10)

**What the game does** (read in the decompiled code; each fact goes to
`docs/formats/gameplay.md` § Oxygen, health and death and § Mouse look,
marked):
- **Under water** (`Player.UpdateIsUnderwater`, not the swimming flag of
  M9b): never in the lifepod, else the player's transform below the ocean
  level. **Can breathe** (`CanBreathe`, no sub or vehicle yet): not under
  water.
- **Depth class** (`GetDepthClass`, no `CrushDamage` on the player):
  depth = max(0, level − y); > 200 m crush, > 100 m unsafe, > 0.1 m safe,
  else surface. **Breaths** (`Player.Update`): while it cannot breathe and
  stats are not frozen, each time game time crosses a multiple of the
  breath period (`ScalarMonitor.DidChangeInterval`) it removes period ×
  cost: periods 3 / 2.25 / 1.5 s (safe / unsafe / crush; 99999 at the
  surface), cost × 1 / 1.5 / 2, so 1, 1.5 and 2 units per second.
- **Refill** (`OxygenManager.Update`, every frame): + 30 units/s
  (`oxygenUnitsPerSecondSurface`, a private field initialised in the
  constructor) when a source is above level − 1 m or the player can
  breathe, and no cinematic plays. Capacity: the player's `Oxygen` (45).
- **Suffocation** (`SuffocationUpdate`, a `Sequence`): when oxygen is
  exactly 0 (`Utils.NearlyEqual(x, 0)` is true only for 0) it runs from 1
  to 0 over `suffocationTime` (8 s) and then kills; when oxygen comes back
  it runs back to 1 over `suffocationRecoveryTime` (4 s). The screen
  overlay is 1 − t.
- **Health** (`LiveMixin`): `maxHealth` from `LiveMixinData` (100);
  `TakeDamage` × `DamageSystem.damageMultiplier` (1); death at 0
  (`Kill`). The only damage without creatures: landing on the walking
  motor out of water (`Player.OnLand`): (−min(0, impact y + 10)) × 2.5,
  impact = the velocity before landing. `CrushDamageUpdate` is never
  called for the player.
- **Death and respawn** (`OnKill`, `ResetPlayerOnDeath`): input, the
  controller and mouse look off, stats frozen; after 5 s the player goes
  to the respawn point (the lifepod's `playerSpawn`, in the pod); after
  1 s and the world settled, health = max × `startHealthPercent`
  (`ResetHealth`, then the `OnRespawn` handler), oxygen full, suffocation
  reset; 1 s later stats unfrozen, input back.
- **Mouse look** (`MainCameraControl.OnUpdate`, `GameInputSystem`): the
  mouse delta × `MouseSensitivity` × 1.5 × 0.5 degrees, added to yaw and
  pitch (pitch up positive in the game, unbounded yaw), pitch clamped to
  `minimumY`…`maximumY` (serialized on the main scene's
  `MainCameraControl`). No smoothing of the look itself (the game's
  smoothing is camera bob, tilt and impact bob, item 3 of "After Phase
  E"). Default sensitivity: the `const` `defaultMouseSensitivity`;
  invert off.

**Steps:**
1. **Data (headless).** `sn-unity`: `MainCameraControl` (every field, to
  the last byte), `LiveMixin` gains `startHealthPercent`. `sn-dotnet`:
  read a `const` float (`Constant` table) and a field initialiser from a
  constructor (only `ldarg.0; ldc.r4; stfld`). `sn-assets`: player data
  gains the look limits, the oxygen source's height above the player and
  the two code values; `sn-inspect player` prints them.
2. **`sn-sim::vitals` (pure).** State (oxygen, health, suffocation,
  breath clock, death phase), parameters (all from step 1; the
  constants inside method bodies, such as the breath periods, are the
  game's code ported, named after their method), one step per physics
  step. Unit tests: no drain in the pod or at the surface; 1 / 1.5 / 2
  units per second by depth; refill at 30/s; suffocation after exactly
  the read time, recovery when oxygen comes back; fall damage at the
  formula; death → respawn timeline. `sn-sim::look`: the mouse rule,
  with its limit unit-tested.
3. **`sn-inspect dive`:** out of the pod, a dive to the seabed held
  until death, respawn in the pod, then a dive cut short by surfacing.
  Logs oxygen each second next to the value the read numbers predict,
  the time to empty, to death, to respawn, the refill time.
4. **Client.** Vitals run with the player; the game's look replaces
  ours; a HUD of our own (Bevy UI): oxygen and health bars with numbers,
  depth, the suffocation overlay (black, alpha 1 − t). Logs deaths and
  respawns.

**Not 1:1 yet (planned here):** stats step at the physics step
(50 Hz), the game's at every frame (a breath or the death can land up to
20 ms apart); the breath clock starts at 0 when the game starts (the
game's `Time.time` includes loading, so the phase of the first breath
differs); script order of `Player` and `OxygenManager` in one frame is
assumed (Player first); no rebreather, tanks, subs, vehicles, water
parks or game modes other than Survival (each comes with its item); no
damage sounds or screen effects, no death animation or "you died"
message (text waits for the language files, M9e); the player's own
settings (sensitivity, invert) are not read, the defaults are used.

**As built (2026-10-10), where it differs from the plan:**
- **The camera's place.** `MainCameraControl` sits on `camRoot`, at the
  player's origin, and its `skin` is 0, so M9b's camera at the player's
  transform is the game's at rest. Whether the pivot of looking up
  (`cameraUPTransform`) moves the eye is not measured.
- **The look.** The game has no look smoothing (the plan's row said
  "smoothing"); its pitch limits are ±87° in the scene against ±80° in
  the code's initialisers.
- **In the pod after death.** The body is moved into the pod at 5 s;
  there `OxygenManager` refills while the stats are still frozen, so the
  suffocation recovery starts before the restore. The game does the
  same (the refill is not frozen).
- **Shared script pieces.** `sn-inspect walk`'s setup, pod exit, boarding
  and report are shared with `dive`; both scripts run the vitals the
  same way the client does (no movement, use or look while dead). The
  dive swims 8 m clear of the pod before going up: the exits end under
  it, and on seed 5 the seabed led back under it.

**After Phase E** (order to be agreed then; each gets its own plan with
steps before it starts):
1. **Survival:** food and water (`Survival`, eating, the fabricator's food
   tab), knife and harvesting, first aid, tanks and fins.
2. **Scanner and PDA:** fragments, blueprints unlocked by scanning
   (`analysisTech`, `TechFragment`), the databank text from the language
   files, the PDA screen.
3. **The player's animations:** the hatch cinematics (`PlayerCinematicController`
   plays an animation, then the end point; also the VR-only end points then
   come from the animation's last frame), the body and arms, the camera bob
   and step smoothing. Needs the `Animator` work deferred in M7f4; it is
   the same work creatures need, so the two go one after the other.
4. **Creatures:** spawning (the 102,777 slot creatures of M7d plus placed
   ones), swimming AI, skinned animation (needs the `Animator` work deferred
   in M7f4), attacks and damage. The largest single block.
5. **Vehicles:** Seaglide, Seamoth, Prawn suit, Cyclops; the Mobile Vehicle
   Bay (constructor tree, P1).
6. **Base building:** Habitat Builder, base pieces and their placement
   rules, power.
7. **Story:** the Aurora's countdown and explosion on the game clock
   (M7f4), radio messages, Precursor bases and keys, the ending.
8. **Audio:** FMOD banks (§ 7: decoding and licensing to check first).
9. **The deferred look items** above, reviewed with the user: which matter
   most once the game can be played.

**Honest size:** Phase E is about 9 milestones. "Subnautica 1:1" is the
whole list above, many times that. The plan keeps each step small and
verifiable so the game is playable early and grows from there.

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
