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
| **M7d** | Spawn slots: the 90,289 `EntitySlotsPlaceholder` objects in the cells are filled by the game at world creation from its loot/flora distribution (much of the missing small vegetation and loot, **hypothesis**). Read the slot data and the distribution tables, fill slots deterministically (fixed seed; the real choice is per save game). | Slot counts per biome logged; filled vegetation visible in Safe Shallows; the same seed gives the same world. |
| **M7e** | Terrain grass: block types with `grassMesh`/`grassDensity` (`VoxelandBlockType`) scatter grass meshes over their faces (e.g. red grass on the grassy plateaus); game levels 0–1 only (`clipmaps-*.json`). | Grass instance counts per batch logged; screenshots of grassy plateaus. |
| **M7f** | Scenes and special objects: the Aurora (`aurora.unity` bundle: 3,189 GameObjects, ~620 mesh renderers, 291 LOD groups), Lifepod 5 (`escapepod.unity`; placed at run time by `EscapePod.ChooseRandomStart`, floats), skinned meshes (`SkinnedMeshRenderer`; 73 placed prefabs, e.g. `BrainCoral` LOD 0). Needs a scene reader (scene GameObjects without prefab instances). | Aurora and lifepod visible at their places; object counts per scene logged. |

### Phase C — Being underwater

| # | Goal | Done when |
|---|---|---|
| **M8a** ✅ | Water data, headless: the biome map (`biomeMap.bin`, `biomes.csv`), each batch's override biome (`LargeWorldBatchRoot`), and the per-biome water settings (`WaterBiomeManager.biomeSettings` in the main scene: absorption, scattering, murkiness, emissive, sunlight/ambient scale, …); `sn-inspect biomes`. | Every biome name in the map and the overrides has settings (misses listed); the biome at a few known places (lifepod: safe shallows, …) is right; values logged. |
| **M8b** (fog + sky values in code) | Underwater look: the game's water fog (extinction/scattering per biome at the camera, Henyey–Greenstein phase, depth-attenuated sunlight, emissive; decoded from the compiled fog shader, `docs/formats/water.md`) as a full-screen HDR pass. Done: the sky system's values (`uSkyManager`/`uSkyLight`: time of day → sun direction and colour, top ambient, sky fog). Next: `AtmosphereVolume` shapes (caves, wreck interiors); calibrate light units against screenshots of the game at the same places. | Side-by-side screenshots with the game (lifepod, coral spot, Kelp Forest, a deep biome) taken by the user; GPU time of the pass logged (`--gpu-timings`). |
| **M8c1** (first pass) | Water surface, ported from the game (`WaterSurface` in the main scene, its four shaders decoded, `docs/formats/water.md` § Water surface), "Medium" water quality (the default: baked waves, no FFT): the 64 baked frames `WaterFrame00…63` (bundle `waterdisplacement`, 256² RGBA8) played over the scene's `sequenceLength` (5 s), linearly interpolated on the GPU into a 512² displacement map each frame, its normal map (with mips) and the accumulated foam amount, as the game does; a surface mesh around the camera at y = 0 (dense within the 200 m where waves fade out, flat beyond); the surface shader drawn after the fog pass, reading the fogged image (refraction) and depth: from above sky reflection (the game's mean sky colour until M8c2 brings the sky map), refraction, Fresnel, sub-surface back light, sun glint, foam, sky fog; from below total internal reflection (deep-water fog colour) and refraction out to the sky, with the water fog up to the surface. Not in M8c1: the clip map (water cut out of bases/the Aurora, shore foam), screen-space reflections (off in the scene). | Real-data test: frames and `WaterSurface` values read; numbers logged (displacement range, foam coverage); GPU time of the water passes logged; screenshots from above and below for the user. |
| **M8c2** (first pass) | Sky: port of the game's uSky skybox and sky map (scattering, sun disc, planet and corona, night sky, moon, clouds; `docs/formats/sky.md`), drawn behind the scene before the fog; the water reflects the sky map. Not yet: stars. | Screenshots with the game above water at two times of day. |
| **M8c3** | Light on surfaces underwater: caustics (64 baked frames `WaterCaustics00…63` at 25 fps, the scene's values), colour cast by depth and distance (`WaterscapeVolume` factors), the game's ambient (sky/equator/ground) and the object shader's spec/gloss/emission maps. | Side-by-side screenshots with the game at the lifepod, coral spot, Kelp Forest; perf budget logged. |
| **M9** | Player: swim controller, terrain collision, surfacing/air, first-person camera. | Can swim from the Lifepod to the Kelp Forest without clipping through terrain (logged collision checks). |

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
