# subnautica-rs — Design

Status (2026-10-10): **Phases A–B done up to M7f4g**, Phase C as first
passes (§ 4), **Phase E up to M9c and M9g** (the player's body and the
hatch cinematics; § 4.3). Next: **M10**. M7f4h moved after M11. Each
row's own mark says what is done; everything else is a plan, not code.

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
| **M7f4** (split into M7f4a–M7f4h, § 4.4, 2026-10-10) | Everything M7f1–M7f3 left different from the game (recorded 2026-10-09; nothing here is 1:1 yet). **Aurora and scenes:** levels of detail switch by distance as the game's `LODGroup`s (M7f4f; the rule is Unity's documented one, a hypothesis until compared); the explosion now runs on the game clock and the exterior cull is ported (M7f4e; the sun and sky do not follow the clock yet); the 316 `CullingOccludee`s not ported; fire, smoke, radiation and explosion effects (particle systems, `VFXController`), sounds. **Skinned meshes:** shown in the pose the hierarchy stores, not animated (`Animator`, e.g. the pod's `lifepod_damage` blend, creatures); blend shapes read past, not applied (e.g. `BrainCoral` LOD 0 shows its base shape); normals of bones with non-uniform scale are moved by the linear part, not its inverse transpose (Unity's GPU skinning: **hypothesis** it does the same). **Lifepod 5:** our seeded draw instead of Unity's unseeded `Random` (the point differs per run in the game anyway); fixed at y = 0 (the game floats it with `WorldForces`/`Stabilizer` around its anchor and the waves; needs physics, M9); camera at the `playerSpawn` transform, not the player's eye height or initial look direction; the pod's own sky and its lights by state (`MarmoLifepodSky`, `LightingController`) done in M7f4g; its `AtmosphereVolume` not used (M8c7b); intro state not shown (damage effects, fire, smoke, birds: the `Manual` spawners); spawned modules' own scripts not run (e.g. nested spawners, storage contents from `SpawnEscapePodSupplies`, the screen UI); `MoveAndRotateWithTransform` applied once, not every frame. | Each item ported or confirmed equal by a matched screenshot, number or unit test; this row emptied. |
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
| **M8c7b** | Specular cube reflections (load the skies' cube maps), atmosphere volumes in the biome lookup (cave and wreck skies), other `SkyApplier` anchors, `_UwePowerLoss` (bases' and the lifepod's `emissiveFromPower` appliers, from M7f4g). | Cube maps loaded and counted; skies per object in a cave logged; matched screenshot of a shiny object. |
| **M8c6b** | Sun shadows of terrain, rocks, coral and plants exactly as the game: read each renderer's `m_CastShadows`/`m_ReceiveShadows` (`MeshRenderer`) and honour them (`m_CastShadows` off: done in M7f4e; shadows-only and `m_ReceiveShadows` open); find whether and how the game's terrain (Voxeland chunks) casts and receives; alpha-clipped leaves cast cut-out shadows; the game's bias/normal bias (0.45/0.4) and Unity's soft-shadow filter; the 50 m shadow distance and cascade splits (done). | Counts of casting/non-casting renderers logged and equal to UnityPy on a sample of prefabs; terrain shadow settings documented; matched screenshots of a rock's shadow and a kelp shadow; GPU/CPU cost logged. |
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
| **M9g** ✅ (2026-10-10, M9g1–M9g5e; not compared with the game on screen) | **The player's body** (plan written 2026-10-10, below M9c's; split into M9g1–M9g5): the game's player model from the `main` scene with the suit the equipment rule picks, its animator driven by the game's parameter rules (`ArmsController`, `Player`), the view model's turn and bob (`MainCameraControl`), the head drawn only in shadows in first person; then the hatch cinematics and death animation. Pulls forward "After Phase E" item 3 (all but tools/IK). Before M10 so remote players are drawn as bodies. | Each step's own list in the plan. |
| **M10** | Multiplayer as planned (Phase D): `sn-protocol`, `sn-net`, `sn-server`, handshake with the build check, join, player sync (remote players drawn with M9g's body, animated by the same rules from synced state). From here solo play also runs against an in-process server (§ 3.1, principle 5), so items and crafting below are written server-authoritative once. | M10's own row. |
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

#### M9g plan: the player's body (written 2026-10-10)

**Why now:** the user wants the player's model and animations before
multiplayer. M10 would otherwise show remote players as capsules, and the
local player has no body or arms. The animation machinery is done (M7f4a–d:
the player's 77,373 bindings all match its hierarchy; `sn-anim` runs its
9-layer controller at about 50 µs per update), and the movement state that
drives it exists (M9b, M9c). M7f4h (the floating pod) is not needed for
this and moved after M11 (its row).

**What the game has** (looked at 2026-10-10 with `sn-inspect scene main
--tree 6` and the decompiled classes; to go to `docs/formats/gameplay.md`
§ The player's body, each fact marked):
- **The model is part of the `main` scene** (confirmed, scene tree):
  `Player/body/player_view` holds the animator (`player_view_controller`,
  avatar `player_viewAvatar`), the skeleton `export_skeleton` (with
  `head_rig/Cam`) and `male_geo` with one group per suit: `diveSuit`
  (body, hands, head), `radiationSuit`, `reinforcedSuit`, `stillSuit`,
  `scubaSuit` (inactive), three fin models (`UltraGlideFins`,
  `SwimChargeFins`, `generalSuit`). The camera is `camPivot/camRoot`
  (`MainCameraControl`); `camRoot/player_head` is inactive. 134 nodes, 12
  drawn, all skinned.
- **Several suits are active in the stored scene** (confirmed, scene
  tree). The game picks at run time (`Player.EquipmentChanged`, read in
  the code): for each `Player.equipmentModels` entry (an equipment slot,
  its models by tech type, a default model) the model whose tech type is
  in the slot is active, the others not; the default model is active when
  none matched. With nothing equipped (a new game) only the defaults show.
  Which models those are: to read (the field is not read yet).
- **The head in first person** (code): `Player.SetHeadVisible(false)` sets
  `Player.head` to `ShadowCastingMode.ShadowsOnly`; only the Cyclops
  cameras and the scanner room camera turn it visible. The head's stored
  mode: shadows only (confirmed M9g1). M7f4e drew "shadows only" as a
  normal renderer (fixed in M9g4).
  Bevy 0.19's sun shadow pass picks casters by the light's `RenderLayers`
  (`check_dir_light_mesh_visibility`, read), so a layer the sun sees and
  the camera does not draws shadows only (confirmed M9g4: the head is
  in the sun's shadow pass and off the camera's layer, counted).
- **The parameters** (code, `ArmsController.Update` and
  `SetPlayerSpeedParameters`): `move_speed` and `move_speed_x/y/z` from
  the velocity relative to the view (under water the aiming transform's
  frame; above water forward and right flattened), smoothed by
  `Vector3.Slerp` at `smoothSpeedUnderWater`/`smoothSpeedAboveWater` × dt;
  `view_pitch` (the camera's pitch); `view_turn` (yaw rate of the view
  model, through `SetFloat` with `turnAnimationDampTime`: Unity's damped
  set, which `sn-anim` does not have); `is_underwater` (and not in a
  vehicle); `on_surface`, `holding_welder`, `using_mechsuit` (rules in
  `InstallAnimationRules`); `diving`/`diving_land` (`UpdateDiving`:
  falling out of water, a ray for obstacles below every scan interval);
  `grab`, `bash` (0.4 s after being grabbed or bashed: false without
  creatures); `cinematics_enabled` (not VR: true). `Player` sets
  `vr_active` (false) and the death triggers `player_death`,
  `player_death_fire`, `player_death_explosion` by damage type. 15 more
  scripts set parameters (tools, PDA, vehicles, bed, bench, …): later,
  with their items.
- **IK** (code): the arms use FinalIK, a third-party plugin
  (`FullBodyBipedIK`, `AimIK`). `UpdateHandIKWeights` sets the solver's
  weight to 0 when neither hand has a target, and only tools, the PDA and
  world targets (`SetWorldIKTarget`) set one, so with empty hands the IK
  does nothing (**hypothesis** about the plugin at weight 0). IK comes
  with the tools.
- **The view model** (code, `MainCameraControl.OnUpdate`): `viewModel`
  turns by the camera's yaw only, and takes the camera's local position:
  the swim bob (`sin(6 t) − 1` × (0.02 + 0.15 × smoothed speed/5) ×
  `swimCameraAnimation`), the landing bob (`impactForce`, from `OnLand`),
  the step amount; the camera also tilts with strafing. Looking down
  turns `camRoot`, looking up turns `cameraUPTransform` (the pivot M9c
  left unmeasured).

**Steps** (each ends with numbers, a MODLOG entry and, where it shows, a
screenshot for the user; nothing is committed or pushed without the
user's OK):

| Step | Work | Done when |
|---|---|---|
| **M9g1** ✅ (2026-10-10; see "as built") | **Data, headless.** `sn-unity`: the rest of `Player` (`equipmentModels`, `head`, `playerAnimator` and the fields between), `ArmsController` (smoothing speeds, `turnAnimationDampTime`, `ikToggleTime`, dive scan interval, …), `MainCameraControl`'s `viewModel` and bob/tilt fields if M9c did not keep them. `sn-assets`: the player's body (its prefab nodes from the `main` scene, the active models for an equipment set, the head node, the body renderers' shadow modes and shaders). `sn-inspect player --body`. In `gameplay.md`, each of the controller's 201 parameters marked: set by a rule ported here, left at its default with empty hands, or later (the script named). | Unit tests of the readers on synthetic bytes; real-data test of the reads; the models active in a new game logged; no parameter left unmarked; shaders of the body listed (unported ones logged, as § 4.2). |
| **M9g2** ✅ (2026-10-10; see "as built") | **Rules, pure.** `sn-sim::body`: `ArmsController`'s empty-hand rules (relative velocity, smoothing, the parameters above, `UpdateDiving` with `collide`'s ray) and the view model's transform (yaw, swim bob, landing bob, step amount, strafe tilt, the look-up pivot); output a list of parameter values and the view model's local transform. `sn-anim`: `set_float_damped` (Unity's damped `SetFloat`: its exact formula is a **hypothesis** until compared). | Unit tests: speeds and smoothing on known inputs, the dive flags, the bobs' ranges, the damped set's step response. |
| **M9g3** ✅ (2026-10-10; see "as built") | **Scripted check, headless.** `sn-inspect walk` and `dive` run the player's animator with these rules: each layer's state changes per phase (in the pod, walking, leaving, swimming, diving, at the surface, death, respawn). | Each phase reaches its states (names logged, the expected ones written in the plan before the run); no NaN, unit quaternions; cost per step logged. |
| **M9g4** ✅ (2026-10-10; see "as built") | **Client.** The `Player` hierarchy spawned from the `main` scene (equipment rule, camera culling mask), its root at the simulated player, the view model's transform each frame, the animator each frame with the rules (a rig as M7f4c, GPU skinning). "Shadows only" drawn as the game does (render layer seen by the sun, not the camera), for the head and for the M7f4e renderer. The camera's near plane from `MainCamera` (read in M7g1). `--third-person`: a debug orbit camera that shows the head (also what remote players will look like). | Body nodes, bones and active models counted in the log; head drawn only in the shadow pass (counted); CPU time of the body logged; screenshots in first person (looking down, swimming) and third person for the user. |
| **M9g5** ✅ (2026-10-10; see "M9g5 as built") | **Cinematics.** The hatches as the game plays them (`PlayerCinematicController`: the player's animator state, the pod's hatch layer, the end at the animation's last frame), replacing M9b's end points (and the VR-only stand-ins); the death animation by damage type. Started 2026-10-10 (the user's choice, before M10); split into M9g5a–e, plan "M9g5 plan" below. | `walk` boards and leaves with the cinematic, end poses logged next to M9b's end points; screenshots. |

**As built (2026-10-10), where it differs from the plan:**
- **M9g1.** Built as planned. `PlayerFields` now reads every field
  (equipment models, `head`, `camRoot`, `armsController`,
  `playerAnimator`, `leftHandBone`) and fails on leftover bytes;
  `ArmsController` too; `MainCameraControl` keeps `viewModel`.
  `Assets::player_body`: the `Player` hierarchy with the equipment rule
  (`equipment_changes`, unit-tested) applied for a new game, the head,
  view model, camera and look-up nodes, the animator and its controller.
  Numbers (`sn-inspect player --body`, real-data test `player_body`): 134
  nodes, 3 slots (4 / 2 / 3 models), 3 renderers drawn in a new game (dive
  suit body, hands, head; all MarmosetUBER, so nothing new to port for
  the look), the head stored as "shadows only" (confirmed); `ArmsController`
  smoothing 10 / 15 (scene) where the code says 4 / 8, damp time 0, IK
  toggle 0. Parameters: 17 by ported rules, 12 fixed with empty hands, 172
  at their defaults until their item (§ The player's body in
  `gameplay.md`). New for M9g2: `cameraUPTransform` (`camOffset`) sits
  0.063 m up and 0.15 m back from `camRoot`, so looking up moves the eye
  around that point; how the drawn camera (`PlayerCameras`) follows the
  player was not found in the code yet (found in M9g2).
- **M9g2.** `sn-sim::body` (12 unit tests): `Body::update` gives the 29
  parameters (17 by rule, 12 fixed; a test in `sn-assets` checks the set
  equals `RULE_PARAMETERS` + `FIXED_PARAMETERS`, without the fire and
  explosion death triggers), and the camera rig (`CameraPose`: `camRoot`'s
  bob, look-down pitch, yaw and roll; `cameraUPTransform`'s look-up
  pitch; the eye's place and rotation). `fixed_step` runs the falling
  clock, `jumped`, `landed` and `died` take the events. Differences from
  the plan: (1) no damped `SetFloat` in `sn-anim`: the game's damp time
  is 0, so `view_turn` is set as computed; (2) **the camera is not at the
  player's transform**: the scene's `AutoParent` hangs `PlayerCameras`
  on `cameraOffsetTransform`, 0.063 m up and 0.15 m back from `camRoot`
  (`sn-assets` reads it; real-data test), which corrects M9c's "as
  built". The client and the scripts still put the camera and the
  hand's ray at the player's transform: M9g3 (scripts) and M9g4 (client)
  switch to `CameraPose::eye`. `PlayerBody::body_params` gives the
  numbers. Hypotheses: `Vector3.Slerp` (Unity's native code), the order
  of `ArmsController` before `MainCameraControl` in a frame.
- **M9g3.** `sn-inspect walk` and `dive` run the body and the player's
  animator every physics step (`body_run.rs`), collect the base and Death
  layers' states per phase and check them against the expected states
  below (written before the run). Seeds 1–5, both scripts: RUN OK, every
  state check ok, 0 NaNs, worst |q| − 1 1.2·10⁻⁷, all 29 parameters
  known, 54–60 µs per step (mean; p99 83–115 µs). Transitions on seed 1:
  `Walking → Swim` 0.02 s after leaving the pod, `Swim → surface swim`
  and back at the surface, Death `New State → player_death` at the death
  and back 1.02 s later, `Swim → Walking` after boarding. Two fixes the
  runs needed: (1) `sn-anim`: a sub-machine's exit node with no
  transition that holds goes on to the layer's exit (**hypothesis**,
  `animation.md`; without it the player stayed in `Swim` after
  boarding); (2) `sn-sim::Player::walk_grounded`, the walking motor's
  own flag (`gameplay.md`; without it the respawned player played the
  falling animation). The hand's ray now starts at the game's eye, so
  the scripts use the hatches from a little closer (seed 1: 1.68 m
  instead of 1.95 m, 0.08 s later); the dive's time to empty moved by
  one breath period (42.26 → 45.18 s), within its check. Runs are
  stepped at the physics rate (50 Hz); the game updates the animator
  every frame.
- **M9g4.** The objects worker loads `Assets::player_body` (new game
  equipment) and sends it as one prefab; the client spawns it once and
  tags its rig's base (`PlayerBodyRig`). `crate::player` runs
  `sn_sim::body` every frame (`fixed_step` per physics step; `jumped`,
  `landed`, `died` from the events) and hands `crate::body` the
  parameters and the view model's world placement (player transform,
  then `camRoot`'s position and yaw only, `MainCameraControl.OnUpdate`);
  the animator runs in the existing `animate` system with the game's
  culling mode (1, cull update transforms). The camera sits at
  `CameraPose::eye` with the pose's rotation; the hand's ray starts
  there. "Shadows only" (`m_CastShadows` 3) renderers go on render layer
  1, which the sun sees and the camera (layer 0) does not; this covers
  the head and M7f4e's one renderer. The near plane is the main
  camera's 0.03 m (was Bevy's 0.1), in every mode. `--third-person`
  puts the camera 2.5 m behind the eye and draws the head
  (`SetHeadVisible(true)`); no collision for that camera (debug only).
  Debug flags for checks: `--look-down DEG`, `--shot SECONDS NAME`.
  Numbers: 134 nodes, 3 renderers drawn (4 sub-meshes, all skinned on
  the rig, 1 shadows only); rig 113 nodes, 213 slots, skins of 43 / 36 /
  5 bones; the head off the camera's layer and in the sun's shadow pass
  (counted every 10 s); standing in the pod the animated nodes reach
  1.53 m below the view model (the capsule's bottom is 1.50 m below);
  base layer `Walking` in the pod and `Swim` in the water, 55–64 of 71
  animated rotations away from the stored pose; skin 0's joint matrices
  differ by up to 1.45 m in the swim pose (not drawn rigidly); rules and
  drive 11–13 µs per frame (worst 78). Screenshots
  `out/m9g4-pod-down.png` (legs, boots and hands looking straight
  down), `out/m9g4-swim-down.png` (hands treading water),
  `out/m9g4-swim-third.png` and `-third-b.png` (third person, 1.3 s
  apart: the pose changes). Not 1:1: the body is lit with the global
  sky (the game's `SkyApplier` on the player not read); the view model
  and camera use the physics position (no interpolation, as before).
  Not tested: motion against the game side by side (for the user);
  looking down 60° in the pod shows no body (the body is below and
  0.15 m ahead of the eye's pivot), not checked against the game.

**M9g3 expected states (written 2026-10-10 before the run, from the
controller's transitions in `sn-inspect anim scene:main --states`):**
base layer ("Base Modes") and "Death" layer, per phase of the scripts:
- in the pod standing, walking to the hatch, after boarding, after the
  respawn: base `Walking` (not under water, grounded); Death `New State`.
- swimming under water (away from the pod, down to the seabed, held
  there): base `Swim` (reached from `Walking` through its exit on
  `is_underwater` and the entry selector).
- at the surface (the dive's 2 s on top): base `surface swim` (`Swim`
  exits on `on_surface`).
- the death: Death `player_death` (the `player_death` trigger), then back
  to `New State` after the clip (exit time 1, 0.25 s).
- never: `Dive`, `Dive_loops`, `player_view_jump_loop` (the scripts don't
  fall out of the water for 0.45 s), the vehicle, PDA and tool states.

#### M9g5 plan: the hatch cinematics and the death camera (written 2026-10-10)

**What the game has** (read 2026-10-10 with the new `sn-inspect scene
escapepod --cinematics` and the decompiled classes; to go to
`gameplay.md` § The player's body, each fact marked):
- **9 `PlayerCinematicController`s in the pod scene** (confirmed, real
  data): the 8 hatch triggers' and the intro's. All move the player along
  `models/Life_Pod_damaged_03/root/player_cineLoc/cin_target`, a node the
  pod's own animator (`escape_pod_controller`, on `Life_Pod_damaged_03`)
  moves; all interpolate 0.25 s in and 0.25 s out (the intro 0 s in);
  none enforces the end by time. Per trigger: the pod parameter
  (`escapepod_topout`, `_topin`, `_botin`, `_botout`, `_botout_first`,
  `_topout_first`, `_left_side`, `_right_side`), its "prepare" parameter
  (`prepare_…`), the player's parameter of the same name (the two side
  hatches: `escapepod_side`), the end point and whether it is VR-only.
- **The flow** (confirmed, code: `PlayerCinematicController`,
  `CinematicModeTriggerBase`): use → no other cinematic running →
  `StartCinematicMode`: pod animator always animates, the pod's
  `prepare_…` true, player controls off, the camera's look folded into
  the player's rotation (`MainCameraControl.cinematicMode` = true);
  state **In** (each `LateUpdate`): the player's transform lerps (position)
  and slerps (rotation) from where it was to `cin_target` over 0.25 s,
  the camera (`camRoot`) from its offset to `Player.camAnchor`; then
  **Update**: the pod's and the player's parameter true, `prepare_…`
  false; the player's transform sits on `cin_target` every frame. The
  pod clip's last-frame event `OnPlayerCinematicModeEnd`
  (`top_out` 2.000 s, `top_in` 1.167 s, `bot_in` 1.667 s, `bot_out`
  0.667 s, `side`/`side2` 2.667 s, `escapepod_first_topout` 8.167 s,
  `escapepod_first_botout` 6.333 s) goes through
  `OnPlayerCinematicModeEndForward` to the controllers; the active one
  puts the player on `cin_target`, sets both parameters false and then
  either lerps 0.25 s to the end point (**Out**, only when the end point
  is not VR-only) or ends at once (the player stays where the animation
  left it); then controls on, the look taken back from the player's
  rotation (pitch clamped, the rest left as a tilt that
  `Player.UpdateRotation` eases out at 10/s), the trigger's
  `onCinematicEnd` calls (`EnterExitHelper.CinematicEnter`/`Exit`, the
  first-use swap).
- **Death** (confirmed, code): `Player.OnKill` sets `player_death`
  (`player_death_fire` / `_explosion` for fire and explosions, which
  don't exist yet) and `EnableHeadCameraController`: `MainCameraControl`
  stops and `camRoot` copies `CameraToPlayerManager.headCameraBone`
  every frame until respawn (`DisableHeadCameraController`). The death
  clips carry `DeathFade`/`DeathCut` events (the screen fade: ours is
  the M9c overlay, kept).
- **Not used here:** the other 4 events of the first-use clips
  (creatures leaving: no creatures yet), the intro (its own item).

**Steps:**

| Step | Work | Done when |
|---|---|---|
| **M9g5a** ✅ (2026-10-10; see "as built") | **Data, headless.** `sn-unity`: `OnPlayerCinematicModeEndForward`, `CameraToPlayerManager`; `Player.camAnchor` (already read: check). `sn-assets`: per cinematic trigger its controller (parameters, interpolation times, VR flag, `animated_transform` as a node of the pod's animator hierarchy, the end point), which of its calls run at the start and which at the end, the forwarder's list; the player body's head camera bone and camera anchor. `sn-inspect scene escapepod --cinematics` (exists) and `player --body` print them. | Unit tests of the readers on synthetic bytes; real-data test: 8 triggers, one animated node, the clip lengths above; facts in `gameplay.md`. |
| **M9g5b** ✅ (2026-10-10; see "as built") | **Events and node poses, pure.** `sn-anim`: animation events fired by an update (each playing clip with weight > 0, times crossed in (previous, now], loops counted; **hypothesis** for Unity's exact rule, written in `animation.md`); a node's placement from a pose (`sn-assets`, the rig's local chain with the animated slots). | Unit tests: events at the end of a non-looping clip, across a loop, during a transition; a node's placement against hand-computed chains. |
| **M9g5c** ✅ (2026-10-10; see "as built") | **Rules, pure.** `sn-sim::cinematic`: the controller's states (In / Update / Out, lerp and slerp, the parameters set when the game sets them), started and ended as above; the player gets a rotation (yaw and tilt) and `Player.UpdateRotation`; `MainCameraControl.cinematicMode` on and off (look ↔ rotation); `Hatches::use_trigger` starts a cinematic instead of teleporting, the start/end calls at their time; the death head camera. | Unit tests with a synthetic animated node: in/out timing, end at the animation's last frame vs the end point, look after the end, no second cinematic while one runs. |
| **M9g5d** ✅ (2026-10-10; see "as built") | **Scripted check, headless.** `walk` and `dive` run the pod's animator and the cinematics; the player's states (`escapepod_*`) per phase. Expected values written here before the run. | Each hatch use takes its clip length + 0.25 s (+ 0.25 s out); end poses logged next to M9b's end points; 0 penetrations after; the expected states reached; seeds 1–5. |
| **M9g5e** ✅ (2026-10-10; see "as built"; not compared with the game on screen) | **Client.** The drawn pod's animators get the same parameters (the hatch opens), the player and camera follow `cin_target`, the body plays its hatch animation, the head camera at death. | Cinematic duration and end pose logged; screenshots mid-hatch (first and third person) and during death for the user. |

**M9g5 as built:**
- **M9g5a.** Built as planned. `sn-unity`: `CinematicEndForward`
  (`OnPlayerCinematicModeEndForward`), `CameraToPlayerManager`,
  `PlayerFields::cam_anchor`. `sn-assets`: `CinematicTrigger` gains its
  `cinematic` controller and key, `animated_node`, `animator_node`;
  `Scene::cinematic_forwards`; `PlayerBody::cam_anchor_node` (`Cam`) and
  `head_camera_node` (`cam_deathpos`). `sn-inspect scene NAME
  --cinematics` prints the controllers, triggers (calls at start and
  end, forwarders) and every clip with events; `player --body` the two
  nodes. The intro clip `escapepod_full` also has an
  `OnPlayerCinematicModeEnd` event, mid-clip (the intro is not in M9g5).
- **M9g5b.** `sn-anim`: `Animator::events` (`FiredEvent`: layer, state,
  clip, event, function, weight) and `event_crossed`; `sn-assets`:
  `PosedNode` / `PosedLink`, `Prefab::posed_node`. On real data the 8
  hatch cinematics end one frame after their clip's length (`bot_in`
  1.70, `top_in` 1.18, sides 2.68, `top_out` 2.03, `bot_out` 0.70, first
  `botout` 6.35, first `topout` 8.20 s at 60 Hz) and `cin_target`
  travels 1.9–7.1 m; the first-use ones fire twice in that frame (two
  layers play a clip with the event; `animation.md`).
- **M9g5c.** `sn-sim`: `Q` (Unity's quaternion: `euler`, `to_euler`,
  `slerp`, `rotate`, `*`), `Pose`, `lerp_angle`, `V3::lerp`;
  `cinematic`: `Cinematic` (`start`, `late_update`, `end_event`;
  phases In / Update / Out; `Signal`s `Prepare`, `Play`, `TriggerEnd`,
  `Ended`), `HatchRun`, `Hatches::begin` / `finish`, `apply`,
  `look_into_rotation`, `rotation_into_look`, `ease_tilt`. `Player` gains
  `rotation`, `cinematic` (its `step` does nothing then) and
  `force_controller_size`; `HatchTrigger` gains `cinematic`
  (`CinematicTrigger::cinematic_params` in `sn-assets`). `Body` gains
  `head_camera` (`died` sets it, `respawned` clears it; the rig freezes
  meanwhile), `view_model` and `camera` (the rig in the world for any
  player rotation, with `camRoot` placed elsewhere by a cinematic or
  the death camera). Vitals: `Situation::cinematic` (no breaths, no
  refill). Differences from the plan: none in scope; `Hatches::use_trigger`
  (M9b's teleport) stays until M9g5d/e switch the callers. Hypotheses:
  `Quaternion.Slerp` the short way round; the controller's size forced
  when the controller comes back (the game does it at the end event).
- **M9g5d.** `walk` and `dive` use the hatches through the cinematics
  (`Hatches::begin` / `finish`, the pod's animator run with its
  parameters, `cin_target` posed each step) instead of M9b's teleport.
  Seeds 1–5, both scripts: RUN OK. Every hatch use took the corrected
  expected time (first-use exit 6.62 s, exit 0.96 s, entry 2.22 s, every
  seed); the "Cinematics" layer reached `escapepod_first_botout_cine`,
  `escapepod_botout` and `escapepod_botin` and was back in `New State`
  after each; oxygen unchanged during each cinematic; 0 penetrations, 0
  surfaces passed through. End places: first-use exit 0.48 m from M9b's
  (VR-only) end point, exit 0.00 m, entry on `botin_end`. Differences
  from the plan: (1) the expected times were corrected for the 50 Hz
  step before checking (above); (2) the first-use exit leaves the
  swimming capsule 0.27 m inside the pod, so the swimming motor now
  pushes out of overlaps as PhysX would (**hypothesis**,
  `gameplay.md`; unit test `swimming_pushes_out_of_an_overlap`); (3) the
  state checks match phase names exactly (a prefix match let "hatch
  bot_out_trigger" take the first-use hatch's states too). The client
  still teleports (M9g5e).
- **M9g5e.** The client's player system runs the hatches as the scripts
  do: its own copies of the pod's animator (`PodCinematics`) and the
  player's (`Assets::player_animation`, now shared with `sn-inspect`'s
  `BodyRun`), `Hatches::begin` / `finish`, the end events and the late
  update every physics step, the player's rotation (`apply`,
  `ease_tilt`), the camera at the cinematic's `camRoot` or, after a
  death, on the head camera bone (`Body::camera`, `view_model` for any
  player rotation). The drawn rigs get the same parameters: the player's
  through `BodyDrive::player_values`, every drawn rig of the pod's
  controller through `pod_values`. Debug flags `--use-hatch SECONDS`
  (aim at the nearest usable hatch and use it through the hand's ray)
  and `--kill SECONDS` (full health as damage). Numbers from the client
  (lifepod seed 1): the nearest hatch from the spawn is the top
  first-use exit ("ClimbLadder"): used from 1.37–1.48 m, ended after
  8.72 s on the pod's roof (y 5.73, out of the pod) = 0.26 + 0.02 +
  8.18 + 0.26 s (its end point is not VR-only, so it moves out to it);
  the first-use swap ran twice in the end frame, because the clip fires
  its end event on two layers and the game's controller handles both
  (`onCinematicModeEndCall` only guards re-entry; read in the code). The
  death camera: the head camera bone 0.10–0.12 m above and 0.13–0.15 m
  beside the player's transform during the 5 s before the respawn (the
  death clip is under a second; the Death layer then returns to its
  empty state); `MoveToRespawn`, `Restored`, `ControlsBack` as in M9c.
  Screenshots for the user: `out/m9g5e-hatch-mid.png` (hands on the
  hatch rim, the sky through it), `out/m9g5e-hatch-third.png` (the body
  on the ladder in the hatch), `out/m9g5e-hatch-end.png` (on the roof,
  the Aurora ahead), `out/m9g5e-death.png`. The drawn rigs run per
  frame and our copies per frame (player) and per physics step (pod), so
  the drawn hatch can be a frame apart from the one that moves the
  player (not measured). Keyboard use of the hatches: not tested by the
  agent.
  **Two bugs the user found playing it (fixed 2026-10-10):** (1) after
  the bottom hatch the controls "bugged out": the first-use clip's second
  end event in the same step put the player back on the animated node,
  leaving its yaw (270°) on the player's transform on top of the look
  (`Cinematic::end_event` now returns `None` when no cinematic runs, as
  the game's early return; unit test tightened; the scripts check the
  yaw left after each hatch); (2) after the top hatch's second (normal)
  exit the player was stuck: the animation ends 0.30 m inside the roof
  and our walking motor did not separate overlaps (it now pushes out as
  the swimming motor does, Unity's `CharacterController` overlap
  recovery, **hypothesis** in `gameplay.md`; unit test
  `walking_pushes_out_of_an_overlap`, which fails without it). Debug
  flags for reproducing such cases: `--use-hatch S1,S2,…` (walks to the
  nearest usable hatch, side-stepping when blocked, and uses it through
  the hand's ray, or directly after 2 s if the ray cannot reach it, which
  happens on the roof above `top_in`), `--hatch-name TEXT`,
  `--hold-forward S`; with them, the player is traced every physics step
  for 3 s after each cinematic (position, motor, gap, rotation, look).

**M9g5d expected values (written 2026-10-10 before the run, from the
M9g5a/b data and the player's controller, `sn-inspect anim scene:main
--states`):**
- Each hatch use lasts 0.25 s (move in) + the pod clip's length + at most
  one physics step (the end event lands on the next step) + 0.25 s
  where the end point is used. `walk` and `dive` leave through the
  first-use bottom hatch (`escapepod_first_botout`, VR-only end point):
  6.58–6.62 s, ending where the animation leaves `cin_target`; `dive`
  leaves a second time after the respawn through the normal bottom
  hatch (`bot_out`, VR-only end point): 0.92–0.96 s; both board through
  the bottom entry (`bot_in`, end point used): 2.17–2.19 s, ending on
  `botin_end` (within 1 mm).
- The player's "Cinematics" layer reaches `escapepod_first_botout_cine`
  while leaving the first time, `escapepod_botout` the second time and
  `escapepod_botin` while boarding, and is back in `New State` in the
  phase after each; the base layer stays in `Walking`, `Swim` or
  `surface swim`.
- Oxygen does not change while a cinematic plays; 0 penetrations and 0
  surfaces passed through after each hatch (the moves inside a
  cinematic are not checked: the game does not collide them either).
- The end places are logged next to M9b's end points (no limit: the
  game's VR-only points are not where the animation ends).
- **Corrected after the first run (2026-10-10):** the durations above
  forgot the scripts' 50 Hz step. Each 0.25 s move takes 13 steps
  (0.26 s), and the pod's animator takes a parameter on its next update
  (one step) before its clip starts, whose end event lands on the step
  that passes the clip's length. So: first-use bottom exit 0.26 + 0.02 +
  6.34 = 6.62 s, bottom exit 0.26 + 0.02 + 0.68 = 0.96 s, bottom entry
  0.26 + 0.02 + 1.68 + 0.26 = 2.22 s; checked within ±0.02 s. The first
  run gave 6.62 s and 2.22 s. At the game's frame rate the rounding is
  smaller (not measured).

**Not 1:1 after M9g (planned elsewhere):** tools, the PDA and IK (with
the tools, "After Phase E" item 1–2); the parameters the other 15 scripts
set (with their items); the intro cinematic (item 3); sounds and effects
(item 8, M7i); the player's saved field of view and VR mode (not read).

**After Phase E** (order to be agreed then; each gets its own plan with
steps before it starts):
1. **Survival:** food and water (`Survival`, eating, the fabricator's food
   tab), knife and harvesting, first aid, tanks and fins.
2. **Scanner and PDA:** fragments, blueprints unlocked by scanning
   (`analysisTech`, `TechFragment`), the databank text from the language
   files, the PDA screen.
3. **The player's animations** (the body, its parameters, view model,
   hatch cinematics and death pulled forward as M9g, above; what stays
   here: the intro, tools and IK): the hatch cinematics (`PlayerCinematicController`
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

### 4.4 M7f4 plan (written 2026-10-10)

**Why now:** the player's body, arms and the hatch cinematics ("After Phase
E" item 3) need the game's `Animator`, so M7f4 is pulled forward (the rule
in § 4.3: a deferred item comes back when it blocks play). M7f4 is the
list of what M7f1–M7f3 left different from the game. It is split into the
steps below so that each one is small and can be checked on its own. The
animation steps come first because they unblock the player.

**What the game's animation data is** (census 2026-10-10 over every
bundle with UnityPy as the oracle; each fact goes to
`docs/formats/animation.md`, marked):
- 600 `Animator`s, 289 `AnimatorController`s, 334 `Avatar`s, 2,294
  `AnimationClip`s (137 MB), 40 legacy `Animation` components (all in the
  UI prefabs), 0 `AnimatorOverrideController`s.
- **No humanoid rig anywhere**: every avatar's human skeleton is empty.
  So there are no muscle clips, IK or retargeting. Every clip is
  "generic": curves bound to a property of an object under the animator.
- A clip keeps its curves in three blocks: streamed (keyframed cubic
  segments), dense (sampled at a fixed rate) and constant. Bindings
  (`m_ClipBindingConstant`) list one property per curve, or 3–4 curves for
  a position, rotation, scale or Euler angle. Checked: in all 2,294 clips
  the bindings' curve counts add up to the clip's curve count, and the
  streamed data reads to its last byte. Each streamed key holds the cubic
  `((a·dt + b)·dt + c)·dt + d` up to that curve's next key. Checked: the
  value at the next key is equal (median gap 2·10⁻¹⁰), except at stepped
  keys, which jump on purpose.
- Bound properties: Transform position, rotation, scale and Euler angles
  (319,290 + 89), blend-shape weights (1,674), light values (151), script
  fields (185), renderer material values (32), animator parameters (78),
  `GameObject` active (17), `RectTransform` (14).
- Controllers use: parameters float 540, bool 987, trigger 113, int 2;
  conditions If, IfNot, Greater, Less, Equals; layers override (369) and
  additive (57), skeleton masks on 43; blend trees 1D (139), 2D simple
  directional (104), 2D freeform directional (5); any-state transitions
  (29); exit times, fixed and normalised durations, offsets (111),
  interruption by the source (26) or the destination (49); write defaults
  off in 2 states; 1 state machine behaviour.

**Steps** (each ends with numbers, a MODLOG entry and, where it shows, a
screenshot for the user; nothing is committed or pushed without the
user's OK):

| Step | Work | Done when |
|---|---|---|
| **M7f4a** ✅ | **Animation data, headless.** `sn-unity::anim`: `AnimationClip` (settings, streamed/dense/constant blocks, bindings, events), `AnimatorController` (parameters and defaults, layers, state machines, states, transitions, conditions, blend trees, selector states, the name table `m_TOS`), `Avatar` (its name table), `Animator`. Readers written from Unity 2019.4.36f1's layouts (UnityPy's type database, as for the other classes). `sn-assets`: a prefab node gets its `Animator`; the controller and its clips load on demand. `sn-inspect anim <prefab or scene>`: the animators, the controller's layers, states and parameters by name, each clip's length, loop flag and curves by kind. | Unit tests on synthetic bytes; real-data test: every clip, controller, avatar and animator in the game parses to its last byte; bindings' curve counts equal each clip's; streamed curves continuous at their keys; binding paths of the player's and the lifepod's clips found in their hierarchies (misses counted). |
| **M7f4b** ✅ | **Animation runtime, pure:** new crate `sn-anim` (layer 2; it uses `sn-unity`'s data types and nothing else). Clip sampling (streamed cubic, dense linear, constant; loop or clamp; quaternions normalised after blending). The `Animator` update: parameters, triggers consumed by the transition that uses them, state machines with entry/exit selectors and sub-machines, any-state transitions, conditions, exit time, fixed or normalised duration, offset, interruption sources, state speed and its parameter, cycle offset; blend trees (1D, 2D simple directional, 2D freeform directional); layers in order with weight, override or additive, skeleton mask; write defaults (the values the bound properties had when the animator started). The output is one value per bound property. | Unit tests on synthetic controllers for each rule above; `sn-inspect anim <prefab> --play <seconds>` logs each layer's state changes and the pose's bounds; real-data test: the lifepod's and the player's controllers run 60 s from their defaults with no NaN and with unit quaternions. |
| **M7f4c** ✅ (motion not compared with the game on screen) | **Client: animated objects.** Prefabs with an enabled `Animator` keep the transforms their controller binds as entities under the instance (instead of being flattened). Rigid parts follow their bone. Skinned parts use Bevy's GPU skinning, with the bones as joints. The animator runs every frame from its defaults, and the game's culling modes are honoured (always, cull transforms when invisible, cull completely). Light, material, script and active bindings are counted and logged, not applied yet. | Animators, bones and skinned parts counted in the log and stable across runs; CPU time of the animation system logged; the lifepod and a few placed animated prefabs (plants) move (screenshot or short clip for the user); frame time with and without (`--no-animation`). |
| **M7f4d** ✅ (motion not compared with the game on screen) | **Blend shapes.** `Mesh` blend shapes read (vertices, shapes, channels, full weights) instead of skipped; Bevy morph targets; weights from the renderer's `m_BlendShapeWeights`, then from animation (`blendShape.<name>` bindings). | Unit test of the reader on synthetic bytes; real-data: every mesh with shapes parses; `BrainCoral` LOD 0's bounds with its stored weights logged next to the base shape's; animated shapes counted. |
| **M7f4e** ✅ (2026-10-10; see "as built") | **Aurora on the game clock, its exterior cull, shadow flags.** `CrashedShipExploder`: `timeToStartCountdown` = start + `Random.Range(2.3, 4)` × 1,200 s (our seeded draw, `--aurora-countdown <s>` to choose), the model swap 27 s after it, on the client's game clock (`--aurora` stays as an override). `ShipExteriorCullManager`: the exploded exterior hidden while the camera is in one of its `ShipExteriorCull` volumes (every 10th frame, as the game). Renderers' `m_CastShadows` honoured (off → no sun shadow; shadows-only → shadow without colour), which is M8c6b's open item. | Countdown, swap time and cull volume count logged; real-data test of the cull volumes' read; swap seen at the logged time with `--time-scale` (screenshot pair); shadow casters counted by mode. |
| **M7f4f** ✅ (2026-10-10; see "as built") | **LOD by distance.** `LODGroup` (each level's screen-relative height, size, reference point, fade mode) and `QualitySettings` (the current level's `lodBias`, `maximumLODLevel`) read; per instance per frame the level the game would show: the group's size in world space against the screen height at the camera's distance and field of view, divided by the bias (Unity's documented rule, **hypothesis** for the exact form until checked against the game). Replaces "always LOD 0" for every prefab and the scenes. | Real-data test of the reads; levels shown per distance band logged; the Aurora at 500 m and 1,500 m (screenshots); triangle count and frame time before and after. |
| **M7f4g** ✅ (2026-10-10; see "as built"; not compared with the game on screen) | **The lifepod's own light.** `MarmoLifepodSky`: inside the pod the global sky is the pod's anchor sky, outside the Safe Shallows one. `LightingController`: its states, multi-state skies and lights, `LerpToState`; the pod's start state and its lights' animator (`Life_Pod_lights_controller`, from M7f4b). The pod's `AtmosphereVolume` waits for M8c7b (atmosphere volumes) and is listed there. | Sky switch logged on entering and leaving; light intensities per state logged against the read values; screenshot inside the pod. |
| **M7f4h** (moved after M11, 2026-10-10, the user's OK: the body does not need it; the floating pod is an object every player sees, so it is built once with M11's simulation owner, as its first owned rigid body, instead of single-player physics redone for co-op; it would also move the floor under the scripted `walk`/`dive` runs M9g relies on) | **The lifepod floats.** `WorldForces` (buoyancy above and below the water, its drag), `Stabilizer` (upright torque), `EscapePod.FixedUpdate` (pull back to the anchor), a rigid body for this one object in `sn-sim` (mass, drag and angular drag read from its `Rigidbody`; no contacts). The pod's colliders and triggers move with it. The player stands on it as on a moving platform (`GroundMotor` moving-platform rules, read in M9b). `MoveAndRotateWithTransform` every frame. | Unit tests of the forces; `sn-inspect walk` still boards and leaves the pod with the pod moving; pod height and tilt over 60 s logged; the client shows it bobbing. |

**As built (2026-10-10), where it differs from the plan:**
- **M7f4a.** Every reader fails on leftover bytes, so "parses to its last
  byte" is part of each parse. Two exit selectors in the game lead
  nowhere (`0xFFFFFFFF`); that is valid data. `sn-inspect anim` also
  prints the avatar and lists missing binding paths. Binding paths are
  matched against the actual hierarchy (CRC-32 of the path); the
  avatar's name table is not needed for that.
- **M7f4b.** A state without a motion writes nothing (its layer lets the
  layers below through); the plan's write-defaults rule applies only to
  states with a motion. The game's own controller needs this: its arm
  layers wait in empty states at weight 1 while the base layer moves the
  arms. Our first version wrote defaults there and froze the player's
  arms. `Animator::play` (`Animator.Play`) and layer weights are there
  for the scripts that come later.
- **M7f4c.** A rig is built only for animators that move a Transform:
  those that change only blend shapes (most of the waving corals: 3,299
  small deco corals, the jewelled disks) stay still until M7f4d. Skinned
  meshes bend on the GPU only when every bone is in one rig; the others
  keep M7f2's stored pose. Effect parts (glows, holograms) and lights
  below an animator stay where the hierarchy stores them. A rig's parts take
  their batch's shadow setting when spawned, but later switches (as the
  batch's detail changes) do not reach them. Light, material and script bindings are counted by `sn-inspect
  anim --placed`/`--play`, not by the client. Nested animators: the
  inner rig does not follow the outer one (how often that happens is
  not counted). New: `sn-inspect anim --placed` (82 placed prefabs,
  9,252 placements with a running animator; 6.7 ms per frame if all
  updated at once, one thread).
- **M7f4d.** Blend shape bindings name the channel by the CRC-32 of its
  name alone, not of `blendShape.<name>` as M7f4a–c assumed. Every
  stored weight in the game is 0, so the stored-weight path (applied
  before skinning, for meshes drawn in their stored pose) changes
  nothing today; blend shapes show only where an animator drives them.
  The project clamps weights to 0–100
  (`PlayerSettings.legacyClampBlendShapeWeights`, read and honoured);
  unclamped, the animated weights run from −91 to 103. In the client,
  rigs are also built for animators that drive only blend shapes, and
  skinned renderers without bones now hang on their rig node (they were
  drawn where the hierarchy stores them). Driven renderers get one Bevy
  morph target per blend shape frame; a bone-less one gets fixed bounds
  that hold every shape, a GPU-skinned one keeps its bone-driven bounds
  (blend shapes beyond them may be culled early: **not checked**).
  Driven shapes on a skinned mesh not bent by that rig stay still
  (logged). New: `sn-inspect prefab --shapes`.
- **M7f4e.** The game's code decided two things the plan did not expect.
  (1) `CullExplodedExterior` acts only when the exploder is both
  initialised (a new game) and deserialised (a loaded save), so in a new
  game the exterior is never hidden. The client runs the manager and the
  volumes as the game does (every 10th frame, the 7 boxes of the two
  rooms that have them, registered while the room is spawned) and so
  never hides it either; for saves this is a **hypothesis** to check with
  M9f. (2) The "exterior" is the whole wreck model (`starship_expoded`).
  The game clock is new: `GameClock` in the client, from `--time` (9.6 h =
  480 s), sped up by `--time-scale`; the sun and sky do **not** follow it
  yet (they stay at the start time; not 1:1 once the clock has run).
  `--aurora intact | exploded` now holds the ship in that state, as a
  save before or after the explosion; `--aurora-countdown <s>` sets the
  countdown; the draw uses `--lifepod-seed` (one seed for a new game's
  random draws). The countdown starts when the Aurora scene has loaded in
  the client (the game: the streamer's first ready frame). The countdown,
  sound and explosion moments are logged; their sounds, effects, camera
  shake, force and damage are not ported (M7i, audio, creatures). The
  Aurora scene is split into parts by the states they show in
  (`Scene::aurora_groups`); their drawn nodes keep the level of detail
  their state picks. Shadows: renderers with `m_CastShadows` off cast no
  sun shadow (5,380 drawn renderers in the placed prefabs, 195 in the
  Aurora); "shadows only" (1 renderer) is drawn as a normal one;
  two-sided does not occur. `DisableBeforeExplosion` is on one prefab
  that is never placed (nothing to do). New: `sn-inspect aurora`.

  **Not 1:1 after M7f4e** (each with where it is planned):
  - The sun, sky and day/night light stay at the start time while the
    game clock runs (a new step: the sky on the clock; with § 4.1).
  - The countdown is set when the Aurora scene has loaded in the client,
    not on the streamer's first ready frame; the clock may differ by the
    loading time (seconds).
  - Countdown and explosion: sounds (`shipCountdownSound`,
    `shipExplodeSound`, the rumble), effects (`fxControl`, the player's
    explosion effect, `unexplodedFX`/`explodedFX` contents), camera
    shakes (at the explosion, and every 15–25 s in the `crashedShip`
    biome after it), `WorldForces.AddExplosion`, `RadiusDamage(2000, 500
    m)`, the `OnShipExplode` broadcast: logged only (audio: "After Phase
    E" item 8; effects: M7i; force and damage: with creatures and
    physics).
  - `AuroraWarnings` (story goals at 20/50/80 %, the PDA's warnings) and
    the other scripts that ask `IsExploded` (`LeakingRadiation`, `Bed`):
    with the PDA, radiation and items.
  - Saves: `--aurora intact | exploded` stands in for a loaded save; the
    saved countdown, legacy saves (version < 2) and the exterior cull in
    a loaded save (**hypothesis**: it never acts there either) wait for
    M9f.
  - The Aurora's parts are always drawn at their most detailed level
    (M7f4f).
  - Shadows: the one "shadows only" renderer is drawn as a normal one
    (Bevy has no shadow-only draw); `m_ReceiveShadows` is not honoured
    (M8c6b); parts of animated rigs keep the shadow setting they were
    spawned with when their batch switches (from M7f4c).
  - Animators and lights below the Aurora's switched objects would be
    split by state like meshes; the scene has none (0 animators, 0
    lights), so this is not exercised.

- **M7f4f.** Built as planned below. Numbers: 3,692 groups in the
  placed prefabs, none cross-fading; High `lodBias` 10, so most groups
  stay at LOD 0 to 50 m and some are culled from ~200 m. From the lifepod:
  8,208 groups, by level 5,861 / 2,039 / 66 / 176 / 2, 64 culled; their
  parts' triangles 31.7 M against 36.4 M at LOD 0; the switch costs about
  0.3 ms per frame (worst 1.6 ms while loading). Entities 26,117 → 47,018
  (every level is spawned; the others hidden), which costs about 1.6 ms
  per frame; the 60° view (was 45°) about 2 ms more, since more is in
  view. Mean frame time from the lifepod 21.0 ms (M7f4e, 45°, no LOD) →
  25.0 ms (`--no-lod`: 24.8 and 24.5 ms in two runs; run-to-run noise
  about ±1 ms, so no gain there). Looking at the Aurora from 500 m:
  16.9 ms against 19.7 ms with `--no-lod`; from 1,500 m: 15.1 against
  16.6 ms (60-frame runs). The Aurora itself looks the same with and
  without LOD (its parts are large enough to stay at LOD 0 at bias 10). New flags: `--no-lod`,
  `--fov <deg>` (the game's option); new command `sn-inspect lods`.
  Scene instances now hang under one root entity each (the Aurora's swap
  hides the root).

  **Not 1:1 after M7f4f:**
  - The selection rule is Unity's documented one, not checked against the
    game (a screenshot pair at the same spot in the game would settle it).
  - The quality level is taken as High; the player's saved "Detail"
    choice and field-of-view option are not read (60° and High are the
    defaults we assume; `--fov` stands in for the option).
  - Hidden levels are spawned up front (a cost the game does not have;
    its renderers are culled in the engine). A later speed-up may spawn
    levels on demand.
  - The switch runs per frame for every group; Unity also does it per
    camera per frame, so equal in effect, but our shadow pass uses the
    same choice (Unity: the camera's choice too, **hypothesis**).
  - Placeholder-spawned and spawn-slot objects use the same rule; their
    cell-level streaming ranges (our own, M4b) still decide which exist.

- **M7f4g.** Built as planned below. Numbers: the controller's values
  logged equal to the read ones in each state (`--lifepod-state`);
  Damaged (default) sky master 2.5, diffuse 0.8, specular 1, lamps off;
  Danger 0.8 / 0.5 / 3, the two red spots at 1.25 and the red point at
  0.22, on; Operational 10 / 2 / 1.5, lamps off. At the start the global
  sky becomes `SkyEscapePod` (161 materials relit, 0.05 ms CPU); flying
  out past 15 m switches it back to `SkySafeShallows` (842 materials
  relit, 0.36 ms). 7 materials use the pod sky through the modules'
  appliers. Skipping the intro disables `EscapePodLights`' animator and
  `HatchLight` (`Scene::stop_pod_intro`). Found in the game's code: a
  `LerpToState(s, 5)` from another script becomes a 1 s fade on the next
  frame (`docs/formats/gameplay.md`). Frame time
  from the lifepod, alternating runs of the committed M7f4f build and
  this one while the machine was busy (other programs at 27–40 % CPU):
  M7f4f 46.7 and 45.0 ms, M7f4g 43.0 and 29.6 ms; no slowdown measurable
  at that noise (the M7f4f build measured 25.0 ms on a quiet machine).

  **Not 1:1 after M7f4g:**
  - `_UwePowerLoss` not drawn (our object shader lacks it): equal in the
    default Damaged state (emissive 1 → loss 0); in Operational the game
    sets loss 1 on the renderers of `emissiveFromPower` appliers below
    the pod (whether the modules have any: not counted). Added to M8c7b.
  - With the fly camera there is no player: "in the pod" is the start at
    its spawn until 15 m away; re-entering needs the player (`--free-cam`
    off, through a hatch).
  - Appliers with anchor BaseInterior/BaseGlass below the pod would take
    its sky in the game; ours take the global sky (none found in the pod's
    modules; not counted across all prefabs).
  - The intro itself (sky curve, lights animator, Danger state while the
    fire burns) is not played: "After Phase E" item 3.
  - Not compared with the game on screen (a screenshot pair inside the
    pod in the Damaged state would settle the look).

**M7f4f plan (written 2026-10-10, before the code):**
- Data: `LODGroup` read fully (local reference point, size, fade mode,
  cross-fading, each level's height and fade width). `QualitySettings`
  read from `globalgamemanagers` (found with the UnityPy oracle: Low
  `lodBias` 0.66, Medium 1, High **10**; `maximumLODLevel` 0 at every
  level). We keep using level "High", as for the shadows (how the game's
  "Detail" option maps to a level is in `GraphicsUtil`, another DLL:
  **hypothesis** High = index 2).
- The camera: the game's field of view is `MiscSettings.fieldOfView`, 60°
  vertical (a static initialiser, read from the DLL; the player can change
  it in the options, not read). Our client used Bevy's default 45°: fixed
  here, since the LOD choice depends on it.
- The rule (Unity's documented behaviour; **hypothesis** for the exact
  form until compared with the game): per group and frame, `h = size ·
  max|lossy scale| / 2 / (d · tan(fov / 2)) · lodBias`, `d` the distance
  from the camera to the group's reference point in the world; the level
  is the first whose height `h` reaches (`h ≥ height`); none: the group is
  culled; never more detailed than `maximumLODLevel`. Pure, in
  `sn-world::lod`, unit-tested.
- `sn-assets`: each prefab keeps its groups and, per node, its group and
  the set of levels listing it (a renderer may be in several levels).
- Client: every level's parts are built and spawned; a per-instance
  component holds its groups (world reference point and size) and switches
  the parts' visibility when the level changes. Scene instances get a root
  entity, so the Aurora's swap and the LOD switch don't overwrite each
  other. Cross-fading (if any group uses it) is not drawn: the switch is
  instant (counted).
- Done when: as the step's row; also entity count and frame time before
  and after, and the levels shown per distance band logged.

**M7f4g plan (written 2026-10-10, before the code):**
- What the game does (from its classes, `out/decompiled`, read only):
  - `MarmoLifepodSky` on the pod: when `Player.escapePod` changes, the
    global Marmoset sky becomes the pod's `anchorSky` (inside) or
    `MarmoSkies.GetSky(SafeShallow)` (outside). The escapepod scene has
    **no** `SkyApplier`, so the hull and interior use the global sky; so
    does every renderer in the world without a `SkyApplier` while the
    player is inside.
  - `SkyApplier.GetEnvironment`: an applier with a `MarmoLifepodSky` among
    its parents takes that `anchorSky` (anchors Auto, BaseInterior,
    BaseGlass). The pod's spawned modules are its children
    (`PrefabSpawnBase` spawns under its own transform), and
    `EscapePod.ForceSkyApplier` re-sends the environment after 0.5 s: their
    appliers take the pod's sky, wherever the player is.
  - `Player.escapePod`: true at the start of a new game
    (`EscapePod.Awake`), set and cleared by the hatches
    (`EnterExitHelper`), cleared beyond 15 m from the pod
    (`Player.ValidateEscapePod`). Ours: `sn-sim`'s `in_pod` (M9b) has
    these rules already.
  - `LightingController` (state, `fadeDuration`, `skies[]`: a sky with
    master/diffuse/specular intensity per state, `lights[]`: a light with
    an intensity per state, an emissive intensity per state →
    `_UwePowerLoss = 1 − i` on registered appliers' renderers). `Update`:
    a state change starts `LerpToState(state, fadeDuration)` from the
    current values; `Timer` linear over the time, then a snap.
    `MultiStatesLight` switches the light's GameObject on when its
    intensity is above 0, off at 0.
  - The pod's stored values (read with our reader, dumped with UnityPy
    for the bytes): state 0, fade 1 s; one sky (= `anchorSky` = the
    cinematic's `interiorSky`) master 10 / 0.8 / 2.5, diffuse 2 / 0.5 /
    0.8, specular 1.5 / 3 / 1 for Operational / Danger / Damaged; three
    lights (two red spots, one red point) 0 / 1.25 / 0, 0 / 1.25 / 0,
    0 / 0.22 / 0; emissive 0 / 1 / 1.
  - The state in a new game: the intro snaps 0, then 1 (Danger, red
    alert) when the pod is damaged, then lerps to 2 (Damaged) in 5 s when
    the player puts out the fire; **skipping** the intro
    (`StopIntroCinematic(interrupted)`) snaps to 2. Both end in 2 until
    the pod is repaired (`LerpToState(0, 5)`). The cinematic's `StopAll`
    disables the lights animator (`Life_Pod_lights_controller`; its idle
    state has no motion, so nothing it wrote stays).
- Data: `sn-unity` readers for `LightingController` and
  `MarmoLifepodSky`; `sn-assets::Scene::lifepod_lighting`: the anchor
  sky (`MarmoSky` and its world rotation), the controller with its lights
  (colour, range, type, world placement); real-data test of the numbers
  above.
- Logic: `sn-sim::lighting`, the controller as a pure state machine
  (`SnapToState`, `LerpToState`, `Timer`, `Update`; the sky's and lights'
  current values out). Unit tests.
- Client: the pod's sky joins the sky set. Materials are made per sky as
  before, plus two dynamic keys: "global" (parts without an applier) and
  "pod" (applier parts of the pod's spawned modules); when the player's
  `in_pod` changes or the controller moves the pod sky's intensities, the
  materials under those keys are relit in place (`apply_sky`). The
  controller's lights are spawned as local lights, shown while their
  intensity is above 0. The lights animator is not run. New flag
  `--lifepod-state operational | danger | damaged` (default damaged, the
  state after the intro, played or skipped). With `--free-cam` there is
  no player: in the pod at the start, out beyond 15 m from the pod (the
  game's rule for leaving without the hatch); entering then is not
  possible.
- Not in this step: `_UwePowerLoss` (our shader has no such input; with
  the default state's emissive 1 it is 0, so equal; other states differ,
  listed); the pod's `AtmosphereVolume` (M8c7b); the intro's sky curve
  (`EscapePodCinematicControl.skyIntensityCurve`, with the intro).
- Done when: as the step's row; the sky switch logged when leaving
  (`--flythrough` out of the pod) and at the start; light intensities per
  state logged equal to the read values; screenshots inside the pod for
  each state; the cost of a relight logged.

**Moved out of M7f4 (not dropped; each one goes where its subsystem
lives):**
- Particle effects (fire, smoke, radiation, the explosion,
  `VFXController`): a new milestone **M7i, particle systems**. The
  `ParticleSystem` class has about 20 modules to read and simulate, too
  big to be one step of M7f4.
- Sounds: "After Phase E" item 8 (audio).
- The intro (the damage effects, fire, the birds from the `Manual`
  spawners, the pod's `lifepod_damage` blend driven by
  `EscapePod.UpdateDamagedEffects`, the red-alert lighting state at the
  start): it plays with the hatch cinematics, "After Phase E" item 3. The
  machinery is M7f4b, M7f4g and M7i.
- The spawned modules' own scripts (nested spawners, storage contents
  from `SpawnEscapePodSupplies`, the screen UI): with the inventory
  (M9d) and the PDA (item 2).
- `CullingOccludee` (316): a speed-up that does not change the picture.
  It joins the "later" occlusion-culling row of § 4.2.

**Closed without code, with the reason written in the docs:**
- Our seeded start point instead of Unity's unseeded `Random`. The game
  draws a different point on every run, so any valid point is the game's
  behaviour (M7f3, `docs/formats/entities.md`).
- The camera at `playerSpawn` rather than at eye height: M9b put the
  player there, and M9c confirmed the camera's place (see M9c "as built").
- Skinned normals with non-uniform bone scale: after M7f4c, Bevy's GPU
  skinning does what Unity's does or the difference is written down
  (Bevy uses the joint matrix's inverse transpose; Unity's GPU skinning
  is a **hypothesis** until checked).

**Not 1:1 after M7f4 (planned elsewhere):** root motion (14 clips have a
generic root transform; not applied until a moving object needs it, then
with creatures); animation events (555; their receivers are game scripts
and come with the scripts that receive them); legacy `Animation` (UI
only, with the UI).

### Phase D — Multiplayer

| # | Goal | Done when |
|---|---|---|
| **M10** | `sn-protocol` + `sn-net` + `sn-server`: handshake (protocol + game build check), join, player transform sync over UDP (position, yaw, pitch, velocity and the body's flags; each client runs M9g's rules for the remote bodies). | Two clients on one machine see each other's animated bodies (M9g; capsules if M9g is not done); packet loss/latency injected in tests. |
| **M11** | Nitrox-style entity sync: simulation ownership + handoff, server persistence, pick up / drop an item. Then M7f4h, the floating lifepod, as the first owned rigid body. | Item state survives server restart; ownership handoff covered by headless tests. |
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
