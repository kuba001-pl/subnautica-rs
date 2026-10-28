# subnautica-rs — Design

Status (2026-10-11): **Phases A–B done up to M7f4g**, Phase C as first
passes (§ 4), **Phase E up to M9c and M9g** (the player's body and the
hatch cinematics; § 4.3), **M10a–M10h** (the multiplayer messages,
transport, server and client logic, the headless server, bots with the
player's rules, solo play through the in-process server, hosting and
joining from the client with remote bodies and cinematics). Next:
**M10i** (the user's test over the internet). M7f4h moved after M11. Each
row's own mark says what is done; everything else is a plan, not code.

Visual fidelity backlog and the missing comparison/terrain plans are collected
in § 4.5 (2026-10-11). Implemented first passes are not claims of a 1:1 match.
M8c7b1's static object atmosphere selection was implemented and checked on
2026-10-14; native tie ordering and original-game comparison remain open.

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
│  ├─ sn-session/             multiplayer logic as state machines: server (join, clock,
│  │                          event order) and client (clock sync, interpolation)
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
| Networking transport | **UDP** with reliable/unreliable channels: our own thin layer on `std::net::UdpSocket`, no async runtime (user decision 2026-10-11, § 9; M10 plan) | Standard for real-time desktop games; no browser constraints | NAT traversal pain → relay/QUIC |
| Replication | Own small protocol in `sn-protocol`, logic in `sn-session`; `bevy_replicon`/`lightyear` not used (they tie replication to Bevy's ECS; M10 plan) | Keep the protocol Bevy-free so server and tests stay simple | Own protocol becomes a time sink |
| MP model | Client–server, not P2P (analysis in the M10 plan); Nitrox-style: server owns persistent world state; each entity has one *simulation owner* (usually the nearest player) who simulates and broadcasts it; interest management by batch/cell | Proven for this exact game; scales to a handful of players | — |
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

### Perf P1 — Client frame time (not the gameplay P1 below; commits P1a–P1e) (plan, 2026-10-27)

| # | Goal | Done when |
|---|---|---|
| **Perf P1** ✅ | Measure and remove client stalls without changing drawn content or simulation rules. First record release frame distributions and CPU/GPU costs; rank costs, fix each independently, repeat the same runs three times. Keep UI diffs small for concurrent inventory work. | Raw frame samples include p50/p95/p99/max and histogram counts; CPU system/stage and GPU scopes are explicit; three before/after runs per fix record spread and unchanged scene counts; workspace tests and clippy with warnings denied pass. Performance targets are reductions in the measured dominant work, not an unmeasured universal FPS claim. |

Plan: use the existing M8g0 baseline harness (fixed camera, seeds, resolution,
warm-up, uncapped presentation), plus a scripted player/PDA run. Add opt-in CPU
timers before optimizing; keep loading and settled samples distinguishable.
Inspect streaming uploads, animation, UI worker/build/render preparation,
hot-loop allocation/logging and presentation. Existing dev settings already
use opt-level 1 for workspace code and 3 for dependencies; README commands use
dev, so measure release rather than assume completely unoptimized development
builds. No new dependencies or architectural changes are planned.

P1 evidence and exact commands: [P1-PERFORMANCE.md](P1-PERFORMANCE.md).
Completion checks:
- [x] Three initial release runs per fixed-camera/player case, raw fixed
  samples and p50/p95/p99/max/histograms retained in ignored `out/`.
- [x] Main object-stream system worst below 2 ms in three matched player
  runs after worker image preparation (0.727–0.779 ms; before 39.214–49.650).
- [x] P1c tested in three alternating eager/pruned rounds and rejected:
  worker savings did not establish a net gameplay tail win. Eager UI retained.
- [x] Static scene snapshots and installed UI parity checked for candidate;
  candidate was reverted. See the evidence note for exact scope.
- [x] P1d: attribute gameplay tails separately from capture/exit across fixed,
  player, and 35 s swim routes; budget object GPU texture/mesh uploads per frame
  (player max `66.462–70.822 ms -> 27.872–36.735 ms`, `render::assets` max
  `48.557–56.673 ms -> 5.719–6.519 ms`, swim object burst `74.958–77.408 ms ->
  24.290–31.679 ms`).
- [x] P1e: chunk oversized terrain/grass meshes (`<= 12,000` triangles), stage
  in-place LOD transitions across frames (`48,000` vertices/frame), and enforce
  `SPAWN_BUDGET = 600` within each `(batch, level)` object slot (35 s swim max
  `168.571–171.400 ms -> 23.447–26.508 ms`, `render::mesh_alloc` max
  `122.900–143.738 ms -> 7.901–8.014 ms`, `objects::stream_objects` max
  `13.904–20.892 ms -> 1.703–2.015 ms`; player max `22.645–28.515 ms`).
- [x] Workspace tests (`724 passed, 0 failed, 63 ignored`), clippy with warnings
  denied, and fmt rerun before commits.
- [x] Default view-distance and dev/release profile checks recorded, with
  noise and remaining untested costs stated; no universal FPS claim.

### Perf P2 — Default-view steady frames and PDA activation (plan 2026-10-27, closed 2026-10-28, targets not met)

| # | Goal | Done when |
|---|---|---|
| **Perf P2** | Reduce steady frame time at view 1200 and remove the PDA-open stall while preserving drawn content and game rules. Rank measured costs before fixing them; one concern per local P2 commit. | Three matched release runs per fix, identical scene/draw counts, default-view median below 16.667 ms and at least 10% below the matched baseline; PDA activation window maximum below 20 ms in three runs, excluding capture/exit; tests, clippy and fmt pass after merging local main. Targets are machine-specific and remain unchecked until measured. |

Measure fixed camera (600 frames, 120 settled warm-up, 1600 × 900, view 1200)
and scripted player/PDA, retaining raw logs and profiler CSVs under ignored
`out/p2/`. Record concurrent processes and timing spread. Compare CPU stages
with recent GPU pass timings; inspect draw/instance counts, object and terrain
culling, repeated-mesh batching, material/bind-group churn, shadows, fog/post,
Bevy systems, allocations, and PDA activation. Do not reduce view distance,
disable effects, or change visibility rules to meet targets.

Implementation order after attribution: (1) measure Bevy direct instanced
submission against the indirect path for these CPU-culled, non-occlusion
views, preserving effective mesh draws and instances; (2) repair winding only
when a mesh/transform/material component changes, retrying unavailable meshes;
(3) update only newly written font-atlas regions rather than replacing whole
images on PDA activation. Keep these changes in separate commits. The inherited
atmosphere-volume candidate is deferred: it addresses streaming updates rather
than the measured steady render bottleneck. No dependency, whitelist, game-rule
or architecture change is planned. Machine contention is recorded per run;
contended frame distributions do not establish an uncontended FPS target.

- [x] Repeated baseline and ranked CPU/GPU/PDA attribution recorded (P2a).
- [x] Each retained fix has repeated before/after evidence and scene/draw parity
  (P2b, P2c, P2d; `docs/P1-PERFORMANCE.md` § Perf P2).
- [ ] Default-view median and PDA activation targets above passed: **not met**.
- [ ] Local main merged with crafting/UI changes preserved; workspace tests,
  clippy with warnings denied, and fmt pass; no game data tracked. The merge
  is left to the orchestrator. The branch alone passes tests, clippy and fmt.

**P2 close-out (2026-10-28).** All runs were contended by other agents'
builds; spreads are in `docs/P1-PERFORMANCE.md`.
- *P2a*: attribution probes only. The default view is CPU/driver-bound in
  render preparation, not GPU-bound. The PDA stall coincides with a 25 MB UI
  upload plus UI worker build and pipeline specialization.
- *P2b*: direct instanced draws (`NoIndirectDrawing`; the old path is behind
  `SN_CLIENT_INDIRECT_DRAWING=1`). In the matched re-measurement, fixed-view
  frame p50 went from 28.6 / 26.1 / 33.0 to 19.0 / 19.1 / 16.0 ms, with
  identical scene and effective-draw counts.
- *P2c*: winding repair only for changed parts. `repair_winding` p50 went
  from 2.2–4.4 to 0.12–0.18 ms and the main schedule from 6.0–15.1 to
  3.3–5.0 ms. The whole-frame medians overlap, so the frame-level gain is not established.
- *P2d*: PDA and item-atlas textures are sent ahead of time. The PDA
  window's UI upload went from 28.3 to 3.2 MB and the items-shown frame max from
  27.5–33.5 to 23.2–23.6 ms. The PDA-window max did not improve.
- Planned step (3), partial font-atlas region updates, was not started. A
  3,145,728-byte UI upload remains at PDA open (play ≈ 3.1 s, `render::images`
  2.7–4.7 ms). It is not identified; it may be a font atlas.

Open (not started): the default-view median is below 16.667 ms in only one
contended run (P2b round 3, 15.988 ms) and is not established. Uncontended
runs are needed. The PDA activation max is 30–39 ms against a 20 ms target. The
open frame is dominated by `game_ui::worker_build`/`scene_draw` (15–18 ms)
and `render::specialize` (≈ 8 ms for new pipeline variants); candidates are
warming those pipelines and building the PDA's draw lists ahead of time. After
P2d the take-all window max is higher (31.8–35.8 vs 24.3–28.8 ms) with no UI
texture work in those frames; the cause is not attributed. Font-atlas partial
updates and the atmosphere-volume candidate remain unmeasured.

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
| **M8c7** (first pass) | The object shader, MarmosetUBER (`docs/formats/lighting.md` § Objects): specular maps, gloss, fresnel, glow with day/night strengths, each biome's Marmoset sky (exposures, SH ambient, unlit flag), `SkyApplier`. Remaining: the sky's specular cube reflections, atmosphere rasterization/fades and native ties (static object point selection built in M8c7b1), anchors other than Auto. | Real-data test of the 37 skies; materials with specular/glow maps counted in the log; matched screenshots. |
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
| **M8c7b** | Specular cube reflections (load the skies' cube maps), atmosphere selection/fades (static object point queries built in M8c7b1), other `SkyApplier` anchors, `_UwePowerLoss` (bases' and the lifepod's `emissiveFromPower` appliers, from M7f4g). | Cube maps loaded and counted; skies per object in a cave logged; matched screenshot of a shiny object. |
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
| **M7g4** (parts built 2026-10-09: the three variants of the Precursor consoles' holograms, `FX_ADDFOG FX_SCROLL WBOIT` ± `FX_MULMAP` ± `FX_FRESNELCLIP`, and the two of the door force fields, `FX_ADDFOG FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES WBOIT` ± `FX_REFRACTMAP`; decoded in `docs/formats/materials.md`; the client checks each material's keywords, blend, depth test and cull, logs the rest with the reason; extra materials draw the last sub-mesh again, as in Unity) | `UWE/Particles/UBER` meshes: decode the shader's variants used by the 31 materials (two scrolling textures, deform, normal and refraction maps, blend modes from the material), port them; the Lost River lakes are the biggest user. M7g4a (2026-10-12) additionally implements unlit Tree Cove brine; see its checklist below. Remaining: other near-clip/triplanar combinations, lighting modes, `FX_LIGHT_NORMALMAP` and the other combinations (lakes, sand and lava falls, the Gun's elevator and deactivation column, the water force field, tech light cones). | Variants used counted and each one decoded; matched screenshots of a Lost River brine lake and a Precursor terminal; GPU time logged. |
| **M7g5** | `UWE/SIG Triplanar with Capping` and `UWE/SIG`: decode both (cap/side textures, SIG maps), compare with our terrain shader's triplanar formula and share code where the formulas agree. | Rocks textured: the client counts no drawn material without a texture except UBER ones; matched screenshot of a Safe Shallows rock and a coral deco. |
| **M7g6** | The small rest (Legacy Diffuse, Blinn Phong, WaterBase moon pool, SIG grass/waving/sand drift, geyser smoke, IonCrystal and Mesmer properties): each ported or listed in M7f4 as not 1:1, with its count. | `materials.md` lists each with "ported" or "not ported"; no kind left unexplained. |
| later | `CullingCamera`'s occlusion culling with the occluder shells (a speed-up, not a visual change). | — |
| **M7h** (built 2026-10-09; a run at the Blood Kelp cache spawns 2,091 placeholders from 189 prefabs in the loaded area (mean frame 8.66 ms with, 8.24 ms without in one pair of runs; another pair, under load, 16.92 vs 16.95 ms); `--no-placeholders` turns it off; not compared with the game on screen) | Objects the game spawns from `PrefabPlaceholder`s when a prefab first starts (the cache doors with their force fields, the key terminals, the ion crystals on the pedestals; user report 2026-10-09 at the Blood Kelp cache). The game's rule (`PrefabPlaceholdersGroup.Start`, `PrefabPlaceholder.Spawn`): when the group's `isInitialized` is false (not saved in the world's cells: only the `Transform` is stored there, checked), each listed placeholder whose GameObject is active (`activeSelf`), with a `prefabClassId` that has a world entity info, spawns that prefab under the placeholder's parent with the placeholder's local transform; spawned prefabs start too, so their own placeholders spawn (nested). No spawn restrictions in this install. Read `PrefabPlaceholder` (`prefabClassId`, `highPriority`) and `PrefabPlaceholdersGroup` (`prefabPlaceholders`) in `sn-unity`; the client adds the spawned prefabs' parts and lights to the prefab that holds them (creatures left out, as for placed objects). | Unit tests of both readers; real-data test: the pedestal's crystal and the cache root's door and key terminal found with their class ids; the client logs placeholders spawned/skipped per prefab; screenshot at the Blood Kelp cache with the door and crystals. |
| **M7g open** (user review 2026-10-09 at the Blood Kelp cache; the values used are the game's, so they stay) | Concerns left after M7g3/M7g4/M7h, to come back to: (1) the spotlight cones are dimmer than in the user's game screenshot (green +11 vs +28 over the wall; geometry, direction and every term of the decoded formula checked; part of the gap is our brighter, bluer cave, the rest unexplained); (2) the Precursor pillars draw nearly black where the game shows them lit teal, probably the cache's atmosphere volume ambient (M8c7b), **not checked**; (3) the other `UWE/Particles/UBER` variants (lit Lost River lakes, sand and lava falls, the water force field, the Gun's elevator and deactivation column, tech light cones) keep the stand-in look (logged, M7g4); (4) the refraction offset's y sign is not checked against the game. Also: effect fog uses the camera's water settings, not the fog volume at the mesh. | Each item compared with a game screenshot at the same spot, fixed or explained in `docs/formats/`. |

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
| **M10** | Multiplayer as planned (Phase D, "M10 plan", steps M10a–M10i): `sn-protocol`, `sn-net`, `sn-session`, `sn-server`, handshake with the build and rules check, join, player sync (remote players drawn with M9g's body, animated by the values the owner's rules computed). From here solo play also runs against an in-process server (§ 3.1, principle 5), so items and crafting below are written server-authoritative once. | M10's own row. |
| **M9d** ✅ | **Done 2026-10-12** (steps M9d1–M9d6 below, each in MODLOG; what is not 1:1 yet is listed under "Not 1:1 after M9d"). Pick up and inventory (plan below, steps M9d1–M9d6): `Pickupable` objects within reach, outcrops break into their resource (`BreakableResource`), inventory of the game's size, item sizes from TechData, equipment slots, blueprints unlocked by picking up, dropping, the lifepod's locker with its starting supplies, the `grab` animation; picked objects gone for every player (server state keyed by entity id and slot seed). What stays not 1:1 is listed in the plan with where each item goes. | Inventory rules unit-tested; a scripted pick-up logs item counts; the object's drawn count −1 on both clients. |
| **M9u** ✅ (2026-10-27, M9u0–M9u9; not compared with the game on screen) | The game's UI (uGUI, sprites, TextMesh Pro): the HUD, hand reticle, messages, the PDA with its inventory tab and equipment; replaces our own HUD and panel. Plan written 2026-10-12 ("M9u plan", below M9d's; steps M9u0–M9u9), approved by the user 2026-10-12. | Each step's own list in the plan. |
| **M9e** (next; plan written 2026-10-27, "M9e plan" below M9u's, steps M9e1–M9e5) | Crafting at the lifepod's fabricator: the menu from P1's tree, recipes and times from TechData, starting blueprints from `PDAData`; item names from the language files. | Crafting rules unit-tested on synthetic recipes; one real recipe crafted in a scripted run (counts before/after logged); menu node count equal to P1's. |
| **M9f** ✅ (2026-10-27, M9f1–M9f4) | Save and load (player, inventory, removed objects, crafted items) on the server, in our own format, in the user's save folder (not `out/`, not the game folder). Becomes M11's persistence. | Round-trip unit test; server restart restores the logged state. |
| **M9s** (planned 2026-10-27; M9s1–M9s15 below) | **Survival:** food/water, eating and first aid; per-item decay, tank oxygen and battery charge; tanks/fins, quick slots, held tools and knife harvesting; medical cabinets and supply crates. Pulls forward After Phase E item 1 and the survival gaps in M9d/M9u. Depends on M9e crafting (in progress) and M9f persistence (done 2026-10-27). | Each step’s own Done when below; headless rules first, then runtime data, server, persistence, client and UI; two-player scripted survival loop with 0 state/hash mismatches. |

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

#### M9d plan: pick up and inventory (written 2026-10-11; approved by the user the same day with equipment slots, unlocks, dropping and the grab animation pulled in, and every gap left written down below)

**Goal:** the player picks up loose items (titanium, quartz, …), breaks
limestone and sandstone outcrops into their resource, finds the lifepod's
starting supplies in its locker, and keeps items in a 6 × 8 inventory and
the equipment slots, packed and placed as the game does; items can be
dropped again. Every player sees the same world. Written server-first on
M10: the server decides who got what; solo play goes through the
in-process server like everything else.

**What the game does** (read in the decompiled code; each fact goes to
`docs/formats/gameplay.md` § Pick-ups and outcrops and a new § Inventory,
marked *confirmed* only once M9d1/M9d2 check it on data):
- **Pick up** (`Pickupable.OnHandClick` → `Inventory.Pickup`): allowed
  when `isPickupable`, more than 1 s since the object was dropped
  (`timeDropped`; **corrected in M9d2:** `Drop` sets it to 0, not to the
  time, so this never holds an item back), and the inventory has room
  (`HasRoomFor` with the
  item's size); else the hand shows "inventory full". The object is
  deactivated and unregistered from its cell. Unless it is `Eatable`, the
  item goes into the equipment first if a free slot takes its
  `EquipmentType`, else into the 6 × 8 container (`new
  ItemsContainer(6, 8, …)` in `Inventory.Awake`, a constant in the code).
  `Player.PlayGrab` sets `timeGrabbed`; the body's animator bool `grab`
  is true for 0.4 s after it (`ArmsController.Update`; our body rules
  send it always false today).
- **Unlocks on pick-up** (`Inventory.OnAddItem` → `KnownTech.Analyze`):
  adding an item analyses its tech type, which unlocks the blueprints
  `PDAData.analysisTech` lists for it (P0 reads `PDAData`).
- **Tech type of an object** (`CraftData.GetTechType`): the nearest
  `TechTag` component up the hierarchy, else the nearest
  `PrefabIdentifier`'s class id looked up in the entity tech table
  (`EntTechData`, read in P0). This settles P0's hypothesis in code; M9d1
  checks it on data.
- **The container** (`ItemsContainer`): items are grouped by tech type;
  the size is TechData's `itemSize` (default from `TechData.defaults`,
  P1). Every change re-packs the whole grid (`TrySort`): groups ordered
  by larger side (bigger first), then area (bigger first), then, at equal
  area, height (taller first), then tech type number; each item takes the
  first free cell scanning rows top to bottom, columns left to right
  (`GetRoomFor` / `NextFreeCell`). `HasRoomFor` packs a trial copy with
  a ghost item of the new size. Positions are a pure function of the
  contents.
- **Equipment** (`Equipment`): named slots, each taking one
  `EquipmentType` (`Equipment.slotMapping`, a static dictionary built in
  code: "Head", "Body", "Gloves", "Foots", "Chip1", "Chip2", "Tank", …);
  the player's equipment uses a subset of them. An item's type is
  TechData's `equipmentType`.
- **Drop** (`Inventory.InternalDropItem`): not allowed in a sub, in the
  lifepod, while piloting, or out of water in a Precursor base
  (`CanDropItemHere`). The point: a ray from the camera within 10° of
  its forward, at a random 0.75–1.5 m, stopped 0.2 m before a hit
  (`Inventory.RayCast`); then a 0.3 m sphere cast up to 5 m from the
  camera to that point stops it 0.15 m before the first hit
  (`Pickupable.FindDropPosition`). Random rotation if
  `randomizeRotationWhenDropped`. The item becomes a pickup again with
  `timeDropped` reset, a rigid body, and a `Smell` for creatures.
- **Outcrops** (`BreakableResource`): each hand click is a hit
  (`hitsToBreak`, 3 on all four prefabs) and plays the grab (or, with a
  bash tool held, the bash); at 0 the outcrop is destroyed and
  `numChances` times a resource is drawn from `prefabList` (first entry
  whose chance passes); if none passed, `defaultPrefab`. Each draw is the
  player's `PlayerEntropy` → `FairRandomizer.CheckChance`:
  `entropy += chance; if entropy > Random.value { entropy −= 1; true }`
  (luck evens out per tech type). The resource spawns at
  `position + up × verticalSpawnOffset` as a rigid body with a torque of
  3–5 about the world's x axis (**corrected in M9d2:** `Random.Range(3, 6)` is the int overload) and a push of 0.1 along `up`.
- **The lifepod's locker** (`SpawnEscapePodSupplies.OnNewBorn`): a new
  game fills the pod's `StorageContainer` with `LootSpawner`'s
  `escapePodTechTypes` (serialized data), each added if it has room. The
  locker is an `ItemsContainer` of its own size; moving items between
  it and the inventory is `Inventory.AddOrSwap`.
- **Death** (`Inventory.LoseItems`): every item with `destroyOnDeath` in
  the container and the equipment is dropped (as above, so nothing
  inside the lifepod), except that the loop stops one short
  (`i < Count − 1`): one item is always kept. A game quirk; ported as it
  is.

**Who decides what (on M10's table):**

| State | Decided by | Reaches others as |
|---|---|---|
| Which world objects are gone, and dropped or spawned ones | Server | Reliable world events; the full set in the join snapshot |
| Every player's inventory, equipment and known blueprints | Server (it reads TechData and `PDAData` from its own install) | Reliable events to the owner; others see only the world change |
| The lifepod locker's contents | Server | Reliable world events (shared: one pod for everyone, M10) |
| Outcrop hits, the resource draw, each player's `FairRandomizer`, drop rays' random angle and distance | Server for the draw and hits; the owner computes the drop point with its collision and sends it | World events |
| The `grab` animator bool | The owner (M9g's rules, sent in the player state like the other animator values) | Player states |

The game's `Random` is unseeded, so any draw is the game's behaviour (as
M7f3's start point); the server's random is its own seeded one. The game
has one player, so two players reaching for one item has no rule to
read: the server takes the first request in its order, the second is
refused (our rule, written in `gameplay.md` as ours).

**Object identity:** objects placed in the world's cells have the game's
own id (`SavedObject.id`, a GUID string); slot-filled objects have none,
so their key is (batch, cell, slot index) under the world's slot seed:
stable because M10 already checks that every client's slot fill is equal
(the per-batch fill hash); objects the server spawns or that are dropped
get `Spawned(n)`. One `EntityKey` type in `sn-protocol` for all three.

**Steps:**

| Step | Work | Done when |
|---|---|---|
| **M9d1** ✅ | **Done 2026-10-11** (MODLOG; facts in `gameplay.md` § Pick-ups and outcrops, § Inventory, § Text; `dotnet.md` § Item rules): `sn-inspect pickups` over 1,543 prefabs (1,369 placed, 174 slot-only), 0 read errors: `Pickupable` on 156 (50 placed, 23,832 placements), tech type by the game's rule from a `TechTag` (72) or the class id (84), 1 `None` (a prop inside an abandoned base, by the rule); 101 tech types, all named in `English.json` (3,818 keys), 15 with a 2×2 size, 86 the default 1×1; all 9 outcrop resources resolve to a prefab of the listed tech type; inventory 6 × 8, 38 slots, the player's 7; 8 luck randomizers (entropy 0); the pod locker 4 × 8 with 6 starting items; 126 analysis entries, 19 for pick-ups. `PrefabIdentifier` holds only the class id (layout corrected on data). Plan as written: **Data (headless).** Per placed and slot prefab: `Pickupable` (`isPickupable`, `destroyOnDeath`, `randomizeRotationWhenDropped`), `TechTag`, its tech type by the game's rule, `Eatable` present; `BreakableResource` with its `AssetReference`s resolved to prefabs and their tech types; the player's `PlayerEntropy` randomizers and equipment slots (main scene); `LootSpawner.escapePodTechTypes` and the pod locker's size; `PDAData.analysisTech`; from the DLL's IL (P1 reader): the inventory size in `Inventory.Awake` and `Equipment.slotMapping` (**stop and ask** if either needs more than a straight-line pattern); item names from the language files (key format checked and recorded). `sn-inspect pickups`. | Real-data test: every pickupable prefab has a tech type (misses listed), every tech type has an item size or the default; all 4 outcrops' resource lists resolve; slot names and types, the pod's supplies and the analysis list logged; the facts in `gameplay.md`. |
| **M9d2** ✅ | **Done 2026-10-11** (MODLOG; facts in `gameplay.md` § Inventory): `sn-sim::inventory`, 33 unit tests on synthetic data, every item of the list below passes; also: a .NET dictionary's enumeration order (what `LoseItems` walks), the battery rule of `RemoveItem(TechType)`, `KnownTech`'s chained unlocks, `PlayerEntropy` false without a randomizer, 20,000 random drags between grid, locker and slots lose or copy no item. Two plan corrections from the code: the 1 s after a drop never applies (`timeDropped` is set to 0), so it is not ported and M9d3/M9d6 changed; the resource torque is 3–5. Our casts stop up to 1 cm (`SKIN`) short of a surface, so a drop point can be up to 1 cm short of the game's. Plan as written: **`sn-sim::inventory` (pure).** The container (groups, the sort order, the packing, `HasRoomFor`, add, remove), the equipment (slots by type, equipment first, not for food), the pick-up rule (1 s after a drop, room), `AddOrSwap` between two containers, the analysis unlocks, `FairRandomizer`, the outcrop rule (hits, chances, default), the drop rules (where allowed; the point from the two casts, given `sn-sim::collide`), `LoseItems` with its quirk, the `grab` bool's 0.4 s. | Unit tests on synthetic data: hand-worked packings (mixed 1×1, 2×2, 3×3 …) give the expected cells; a full grid refuses; removal re-packs; a tank goes to an empty tank slot, food never does; swap between grid and locker; a seeded `FairRandomizer` run's hit rate converges on the chance; the outcrop falls back to the default; no drop in the pod; the drop point stops before a synthetic wall; death keeps one item. |
| **M9d3** | **Split into M9d3a–d (plan below, 2026-10-11).** Plan as first written: **The protocol and the server.** `sn-protocol` 4: `EntityKey`; requests `PickUp {key}`, `Hit {key}`, `Drop {item, point, rotation}`, `Move {item, to}` (grid, slot, locker); world events `Removed {key, by}`, `Broken {key, spawned: [(id, prefab, position)]}`, `Dropped {id, prefab, point, rotation}`, locker changes; owner events `InventoryChanged`, `Unlocked {tech}`, `Refused {reason}`; the removed, dropped and spawned objects and the locker in the join snapshot (chunked if it outgrows a message: the `Welcome` is under 1,200 B today, the set can hold thousands). `sn-session::server` applies them in order; the world-state hash covers all of it. | Unit tests with simulated clients: two players pick the same item in the same tick → one gets it, the other `Refused`; a full inventory refuses; a drop then an immediate pick-up by another player allowed (the game's rule, M9d2); a late joiner's snapshot equals the server's; hash 0 mismatches over 10,000 random ticks with picks, breaks, drops and locker moves; 1,000,000 bad inputs, 0 panics. |
| **M9d4a** ✅ | **Done 2026-10-11, headless** (MODLOG; the locker fact in `gameplay.md` § Inventory). M9d4 is split: a is the bodies, b the drawing. Real-data test `hand_targets_follow_the_items` around Lifepod 5 (seed 1, 4 batches): 1,718 pickups and 81 outcrops load as targets, each with its own bodies tagged with its key; 1,778 of them are hit by a cast from above. A removed key's bodies go and the cast misses it; a dropped item (server-made object) is hit, then not after it goes; a fresh load leaves a gone key out. The locker has 1 collider (the `GenericHandTarget` node). The ~1,800 small bodies made a player sweep 12× slower (7.35 µs against 0.59 µs). `sn-sim`'s `World` now keeps small bodies in an 8 m grid (results in the same id order, checked against a brute force on 3,000 random casts and sweeps): 0.89 µs with the targets, 0.80 µs without. Objects that a placed prefab's own placeholders spawn have no key and stay static. Plan as written for M9d4: |
| **M9d4b** ✅ | **Done 2026-10-11** (MODLOG). Client with `--start -197.3 -35.2 -10.2 --pick-up 3 --drop 6 --shot 10`: 3 pickups within 3 m requested and taken; 3 keys hidden (9 drawn entities), 3 bodies removed, so equal per key; the drop made 1 server object, given 1 body and drawn once (3 entities) after its prefab loaded; the screenshot `out/m9d4b-pick-drop.png` shows the dropped item in front of the camera. `sn-inspect coop` 4 bots on lan against `sn-server`: 18,264 of 18,264 ticks exact, 128 hash checks 0 differ. The debug drop puts the item 1 m ahead (our stand-in, labelled in the code; the game's drop ray is M9d5). Not checked by eye in motion: an item vanishing as it is picked. Plan as written: **Drawing follows the items** (client). Each drawn instance of a target keeps its key; a gone key hides its instances (at load and when the event comes); server-made objects are drawn at their point; the client's collision worker gets the gone keys and the spawned objects (`BatchBodies::update_with`, `sync_spawned`, `remove_key`). | Headless counters in the client's log: instances hidden per removed key equal to the bodies removed; a spawned object drawn once; the `coop` run's hash checks 0 differ. On screen (for the human): a picked-up item disappears. |
| **M9d4** (plan as first written) | **Targets in the world.** `collision_world` tags each hand body with its `EntityKey` and kind (pickup, outcrop, locker); a removed key drops its bodies and its drawn instances on every machine; spawned and dropped items are drawn and get hand bodies. | Headless: after a `Removed`, the hand cast no longer hits the key; after a `Dropped`, it does; counts of targets per kind per loaded batch logged. |
| **M9d5a** ✅ | **Done 2026-10-11** (MODLOG; facts in `gameplay.md` § The hand on items). M9d5 is split: a is the hand, b the inventory panel. The hand's ray each frame: over a pickup "Pick up *name*" (`PickUpFormat` and the English name), without room the name and "Inventory full", over an outcrop its break text, over the lifepod's locker its open text (our HUD, centred); E or a click sends the request and sets `timeGrabbed`, so the body's `grab` is true for 0.4 s; the locker's click is counted for the panel; the own inventory (cells per item, slots, blueprints known) and unlocks are logged after each change. `--use-item <SECONDS,…>` walks to the nearest target (stepping aside when blocked) and uses the hand when its ray reaches it. Runs: 3 acid mushrooms picked by the hand, the grid filling (0,0), (1,0), (2,0); a limestone outcrop broken in 3 hits into copper ore, picked up; `grab` on 0.39–0.40 s each time; the locker reached from 1.98 m, "Open storage container" on screen (`out/m9d5a-locker.png`). 1 unit test of the hand's text and requests (room, full, not pickupable, server-made, outcrop spawn point, locker). `sn-inspect coop` 4 bots: 18,252 of 18,252 ticks exact, 128 hash checks 0 differ. Keyboard and mouse play **not tested by the agent**. Not 1:1 yet: the hand's icon and the game's reticle look (M9u); the locker's flap animation (`storage_open`, M9d5b); sounds. |
| **M9d5b** ✅ | **Done 2026-10-12** (MODLOG; facts in `gameplay.md` § The inventory panel and the locker). The hand on the locker sets the pod's `storage_open` and the panel opens 0.25 s later with the locker beside the 6 × 8 grid and the 7 slots; closing clears `storage_open`; dying closes it. A left click is `GetItemAction(item, 0)` (`sn_sim::inventory::item_action`), a right click the drop. Runs: `--use-item 3 --panel 6=take-all,…`: the 6 supplies taken out, 6 `Move` requests answered by their world events (req 0–5), locker then empty, grid (0,0)–(5,0); a right click in the pod: no drop action (`CanDropItemHere`); out through the top hatch, a right click dropped a flare: camera, draws, ray point and landing point logged (1.37 m, no hit), accepted (`req 6`), drawn; screenshots `out/m9d5b-locker.png` (the panel), `out/m9d5b-drop.png` (the flare and "Pick up Flare"). 5 new unit tests (the item actions in each store and the pod, the opposite store, the random rotation, the draws, the script steps). `sn-inspect coop` 4 bots: 18,252 of 18,252 ticks exact, 124 hash checks 0 differ. **Not tested:** the distance rule in a real run (holding W walks into the locker); keyboard and mouse play by the agent. Not 1:1 yet: the game's PDA look (M9u), its 0.5 s opening, the arms' PDA animation, sounds; use, eat and quick slots; the drop's rotation without `randomizeRotationWhenDropped` (identity); other players do not see the locker's flap. Plan as written: **The inventory panel** (client; plan below, "M9d5b plan"). Tab opens our own panel (Bevy UI) as the game's PDA inventory tab: the 6 × 8 grid and the 7 slots, each item a box of its size with its name, and the lifepod's locker beside it when the hand opened it (`EscapePod.StorageOpen`: `storage_open` set, the panel 0.25 s later, ignored while that sequence runs; closing clears `storage_open`). While it is open the look and the hand are off and the cursor is free; it closes when the player strays (`PDA.activeSqrDistance`). Left click: the item's action by `Inventory.GetItemAction(item, 0)` (equip, unequip, swap, switch: a `Move` to the opposite store; use and eat logged as not ported); right click: drop, where the game would (`drop_ray` then `find_drop_position` from the camera, local random draws, `Random.rotation` if `randomizeRotationWhenDropped`; no drop action in the pod). Tab or Escape closes it. `--panel <SECONDS=ACTION,…>` opens, closes, takes everything out of the locker, equips and drops from the logs; `--drop` uses the real drop point. | Logs: each move and drop request and its answer, the inventory and locker after each change; a scripted run takes the pod's supplies out of the locker and drops one item outside; the drop point logged next to the camera and the ray; unit tests of the item actions (each action, each store, the pod) and of the random rotation. Keyboard and mouse play **not tested by the agent**. |
| **M9d5** (plan as first written) | **Client.** Hand text "Pick up *name*" / "Break *name*" / "Inventory full" / the locker (our HUD, the language file's text); click or E sends the request; the `grab` bool drives the body; Tab opens our own inventory panel (Bevy UI: the 6 × 8 grid, the equipment slots, the locker beside it when open; each item a box of its size with its name; click to move, a key to drop). | Logs: requests, answers, the inventory after each change (cells per item), unlocks. Keyboard and mouse play **not tested by the agent**. |
| **M9d6a** ✅ | **Done 2026-10-12** (MODLOG; facts in `gameplay.md` § Death drops). `sn_assets::hand::aim` (moved, with its test); `sn_session::drops` (`Draws`, `drop_at`, `death_drops`; 2 tests); the client's panel and `--drop` use `drop_at`; at `MoveToRespawn` the client sends `Lose` from its camera. Runs: supplies taken, out by the top hatch, `--kill 20`: `LoseItems` dropped 5 of 6 (offsets 1.09–1.45 m), the `Lost` event (req 6), 5 server-made objects drawn, Nutrient block #3 kept; killed in the pod: "nothing to drop (in pod true)". 470 tests pass. Not 1:1 yet: the camera of the drops is our eye, not the head camera's last place; dropped items do not fall. Plan as written: **Death drops and shared hand code** (plan below, "M9d6 plan"). The hand's aim moves to `sn_assets::hand` (both apps use it); the drop point and the death drops become pure functions in `sn_session::items` (`drop_at`, `death_drops`); `sn-client` sends `Lose` when the death's respawn starts (as `ResetPlayerOnDeath` calls `LoseItems` just before `MovePlayerToRespawnPoint`). | Unit tests of the death drops (one per lost item, the last kept, none in the pod, offsets within range); a client run: supplies taken, out of the pod, `--kill`: the `Lose` request, its `Lost` event, all but one item drawn in the world, one kept; the same in the pod: nothing dropped. |
| **M9d6b** ✅ | **Done 2026-10-12** (MODLOG). `sn-inspect pickup [--seed N]`: `walk`'s run (the player's rules, collision, hatch cinematics, vitals) with the item targets loaded, an in-process `sn-session` server and client on a simulated clock (the run's steps sent as states, one tick each), every request built by the hand's aim and the panel's rule. Seeds 1, 2, 3: 14 of 14 checks each: the locker reached by the hand (1.95 m), 6 `Move`s, out by the bottom hatch, an acid mushroom (+1, its key gone), a limestone outcrop broken in 3 hits into 1 resource (copper ore, titanium, copper ore) picked up (+1), a drop (−1, +1 object) picked up again at once, the death dropping 7 of 8 and keeping 1 (+7 objects); client and server hashes equal; 0 refused; 0 penetrations; 4–6 s each. On seeds 2 and 3 the nearest outcrop was not reached in 30 s (the script swims straight, without paths) and the next was taken. Plan as written: **`sn-inspect pickup`**: headless, the real rules and collision, an in-process server and client: from the pod, take the supplies from the locker (the hand on it), leave, swim to the nearest pickup and outcrop of the seed, pick up, hit three times, pick up the resource, drop one item and pick it up again at once, die and count what is kept; logs the inventory, the equipment and the cells after each step. | The run logs the expected counts at each step (6 supplies taken; +1; 3 hits, a resource object, +1; −1 and +1; after death all but one lost, objects in the world +n); 0 requests refused that the script expects to pass; seeds 1–3. |
| **M9d6c** ✅ | **Done 2026-10-12** (MODLOG). Two clients on this machine (`--host 27431`, `--join`, `--view 400`): the host took the supplies, left by the top hatch, dropped a flare (1.03 m out), picked it up again with the hand, died outside (5 drops). Both clients logged, line for line: object 4 drawn, asked at world event 18 (1 drawn); removed at event 19 (0 drawn); objects 0, 1, 2, 4, 5 drawn, asked at event 23 (5 drawn). Host's server: 100 hash checks, 0 differ (the session now prints a last report on exit). `sn-inspect coop` 4 bots against `sn-server`: lan 18,252/18,252 exact, bad 18,025/18,060 (99.806 %, target 99 %); 252 hash checks, 0 differ. Solo `--hatch-name top --use-hatch 3,15,27 --hold-forward 31 --kill 40`: 453/453 traced states equal to M10g's, no step shift, 15/15 events. **Not tested:** two machines; both players at the locker at once. Plan as written: **Two players.** Two clients (`--host`, `--join`, `--view 400`): A takes the supplies and drops one outside, picks it up again; B logs the item's drawn count. `sn-inspect coop` against `sn-server` as before; the solo hatch trace (M10g). | On both clients the object's drawn count +1 after the drop and −1 after the pick-up at the same world-event index; 0 hash mismatches; `coop` all exact; the solo trace equal to M10g's (453/453). |
| **M9d6** (plan as first written) | **Scripted proof.** `sn-inspect pickup`: from the pod, take the supplies from the locker, swim to the nearest pickup and outcrop (seeded), pick up, hit three times, pick up the resource, drop one item and pick it up again at once, die and count what is kept; logs inventory, equipment and cells after each step. `sn-inspect coop` and two clients (`--host`, `--join`): bot A picks and drops, bot B logs the object's drawn count. | The scripted run logs the expected counts; on both clients the object's drawn count −1 (+1 after a drop) at the same world-event index; hashes 0 mismatches; solo trace unchanged (M10g's 453/453). |


**M9d6 plan (written 2026-10-12, after M9d5b).** Writing the proof showed
that no client sends a death's drops yet (`Request::Lose` exists in the
protocol and the server since M9d3), and that `sn-inspect` would need the
hand's aim and the drop point the client has. So M9d6 is split in three:
a moves that code where both apps can use it and adds the death drops,
b is the headless proof, c the two-player check. Facts from the game's
code (`sn-inspect code --il 'Player/<ResetPlayerOnDeath>d__302'
MoveNext`): `Player.OnKill` turns the controls off, ignores the PDA's
input and starts `ResetPlayerOnDeath(5 s)`; after the wait it sends
`DisableHeadCameraController`, `ToNormalMode(false)`, then
`Inventory.LoseItems()`, then `MovePlayerToRespawnPoint`. So the client
sends `Lose` at our `MoveToRespawn`, before the teleport, from its camera
(stand-in: our eye, not the head camera's last place), each item's point
drawn as `InternalDropItem` draws it, relative to the camera
(`DeathDrop`'s millimetres). The server takes them as given (the M9d3
plan). Our panel closes on death (the game's `SetIgnorePDAInput(true)`
stops new opens; whether it closes an open PDA is not read).

**M9d5b plan (written 2026-10-12, after M9d5a; read from the game's
code with `sn-inspect code --il` / `--enum`).**
- **What a click does** (`uGUI_ItemIcon.OnPointerClick` → button 0 left,
  1 right, 2 middle → `uGUI_InventoryTab.OnPointerClick` →
  `Inventory.GetItemAction` / `ExecuteItemAction`). `GetAllItemActions`:
  for an item in the player's grid, `Eat` if it has an `Eatable` (in
  survival), `Use` for `FirstAidKit`, `Drop` if `CanDropItemHere`,
  `Assign` if bindable; then by the opposite store
  (`GetOppositeContainer`: from the grid, the first used storage with
  room, else the first, else the equipment; from anywhere else, the
  grid): from the slots `Unequip` if the grid has room; into the slots
  `Equip` with a free compatible slot, else `Swap` with any compatible
  one; into a storage `Switch` if `AllowedToAdd` (its tech filter only,
  not room). Left click takes the first of Use, Eat, Equip, Unequip,
  Swap, Switch; right click Drop (the mouse branch; the controller's
  buttons 2 and 3 are Drop and Assign). Equip, Unequip, Swap and Switch
  all run `AddOrSwap(item, opposite)`: our `Move {to, Store}`. Ported as
  a pure function in `sn-sim::inventory` (`item_actions`,
  `item_action`). Use, eat and assign are not ported (logged).
- **The locker** (`EscapePod.StorageOpen`, `StorageOnFlapOpened`,
  `StorageOnClosePDA`): ignored while the storage sequence runs; sets
  `storage_open` on the pod's animator (ours and the drawn rigs'), and
  0.25 s later opens the PDA with the locker as used storage (`PDA.Open`
  refuses in a cinematic, which closes it again). Closing the PDA runs
  the sequence 0.25 s and clears `storage_open`.
- **The PDA** (`PDA.Open`, `ManagedUpdate`): open stores the target and
  `activeSqrDistance` = (target − PDA)² + 1, and closes once the
  distance² reaches it. Our stand-ins: the target is the locker's hand
  collider's centre (`storagePivot`'s node is not read), the PDA's place
  is the player's position. The look and the hand are off; movement keeps
  working (**hypothesis**: `PDA.Open` does not stop it, and the distance
  check needs it). Tab toggles, Escape closes (our keys; the game's
  button 1 is not read). Not ported: the PDA's 0.5 s opening sequence,
  the arms' PDA animation (`ArmsController.SetUsingPda`), `pdaPause`, the
  sounds, quick slots.
- **A drop** (`InternalDropItem`): from `MainCameraControl`'s position and
  forward (our camera: the eye and the look), `drop_ray` with
  `Random.Range(0.75, 1.5)`'s value and `insideUnitCircle`, then
  `find_drop_position`, in the solid group (**hypothesis**: the ray's
  trigger handling is not read). Rotation: `Random.rotation` if
  `randomizeRotationWhenDropped` (a new `PickupInfo` field), else the
  item's current rotation, which we do not keep: identity (stand-in).
  The draws are the client's own random, as the server takes drop points
  as given (M9d3 plan).

**M9d3 plan (written 2026-10-11, after M9d2).** M9d3 is split into four
steps, each revertable on its own. Four choices made here, each the
recommended option (§ 9, decision 10):
- **Inventories are shared state.** Every accepted request becomes one
  world event, and every machine replays it through `sn-sim::inventory`
  with the same rules (same build and fingerprint), so all machines hold
  every player's grid, slots, blueprints and luck, plus the locker and the
  world's objects. The world hash covers all of it. The table above said
  "owner events"; this replaces them. The reasons: one log and one hash
  find any divergence, the events are tiny, co-op has nothing to hide, and
  other players' equipment will be drawn later. Only a refusal goes to the
  requester alone (`Refused {req, reason}`).
- **Random draws travel as results.** The server draws (its own seeded
  random) and the event carries what was drawn: the outcrop's spawned
  resources, and the luck values each draw used, so every machine's
  `FairRandomizer` moves the same way. The owner computes drop points
  with its collision and sends them; the server takes them as given (as
  movement, decision 6).
- **The client names the object.** A request about a world object
  carries its key and its prefab class id. The server does not load the
  world: it checks the class id against the item rules (a pickupable or
  an outcrop) and that the key is not already gone. Objects the server
  made (`Spawned`) it knows itself. Revisit with movement for public
  servers.
- **Keys:** `Placed(id)` for an object saved in a cell (`SavedObject.id`).
  `Slot {placeholder, slot, copy}` for a slot filler: the placeholder's
  id, the slot's index and the copy, which are exactly what
  `sn_world::fill_slots` is seeded from, and so stable. This replaces
  "(batch, cell, slot index)" above. `Spawned(n)` for what the server
  made. One server counter numbers spawned objects and items: an item
  keeps its number when dropped (`Spawned(n)`) and when picked up again.
  Objects inside a placed prefab (spawned by its own placeholders) have
  no key yet: not targetable in M9d. Logged by M9d4 with a count.

| Step | Work | Done when |
|---|---|---|
| **M9d3a** ✅ | **Done 2026-10-11** (MODLOG; format in `protocol.md` § Items): every new message and variant round-trips canonically; 1,000,000 bad inputs, 0 panics; malformed fields name their offsets; the largest message is 1,033 B (limit 1,161). Two refinements: the drop rotation is 4 × i16 like the states' and a death's drops are millimetre offsets from one origin (14 B each), so a full inventory's 54 drops fit one message; `sn-protocol::codec` is public (the snapshot reuses it). Plan as written: **Protocol 4** (`sn-protocol`). `EntityKey`, `ItemId`; client requests with a client-numbered `req`: `PickUp {key, class}`, `Hit {key, class, kind}`, `Drop {item, point, rotation}`, `Move {item, to, target}` (grid, slots, the pod's locker), `Lose {drops}` (death, the points of what `LoseItems` drops); world events `PickedUp`, `Hit`, `Broken {spawned, luck}`, `Dropped`, `Moved`, `Lost`, each with the player and `req`; server messages `Refused {req, reason}` and `WorldChunk {index, total, bytes}`. `docs/formats/protocol.md` updated. | Round-trip tests for every new message and every variant; canonical (re-encode equal); 1,000,000 random and mutated inputs decode without a panic; every malformed field gives its offset. |
| **M9d3b** ✅ | **Done 2026-10-11, together with M9d3c** (MODLOG; format in `protocol.md` § Items state). b alone broke two M10 tests: a late joiner could not know a left player's kept items without c's snapshot, so b and c land as one change. `sn-session::items` (`ItemRules`, `ItemsState`); 8 tests in `tests/items.rs` and 1 unit test: each event's effect, refusals change nothing, the snapshot round-trips (dictionary order included), every part moves the hash, 5,000 random events by 4 players leave a server, a client and a halfway joiner equal; 200,000 corrupted snapshots are errors, not panics. Plan as written: **The shared state** (`sn-session::world`, using `sn-sim`). `ItemRules` (pure: tech sizes and equipment types, slot table and the player's slots, analysis list, default tech, pickupable and outcrop prefabs by class id, the pod's locker size and supplies, the luck's starting values, tech type → class id for what the server spawns). `WorldState` gains the gone keys, the world objects the server made, each player's inventory and luck (kept after a leave: a rejoin is the same player), the locker, outcrop hits; a new world fills the locker. Each new event applies through the M9d2 rules; the hash covers all; the state encodes to bytes canonically for the snapshot. | Unit tests: each event's effect; an event that does not fit is refused and changes nothing; the snapshot decodes to an equal state with the same hash; every new field moves the hash; two states fed the same random log stay equal. |
| **M9d3c** ✅ | **Done 2026-10-11** (MODLOG; rules in `protocol.md` § Server rules, § Client rules): 7 new tests in `tests/server.rs`, all from the list below. Same-tick reach: one gets it, the other `Gone`. Full: `NoRoom`. Drop then an immediate pick-up by another player is allowed. The pod rules hold. A late joiner equals the server (2 chunks, 666 B). 10,000 ticks with 4 players: 5,902 requests, 1,688 events, 93 rejoins, 1,026 hash reports, 0 mismatches. 1,000,000 bad requests: 0 panics. The apps pass empty rules until M9d3d. The world hash keeps running sums of the large maps: hashing the whole encoding after every event took 28 s for the bad-request test (37,352 events, the gone set growing), 0.65 s after. Plan as written: **Server and client** (`sn-session`). The server checks each request (known object, not gone, room, in the pod for the locker, can drop, item owned), draws, emits the event or `Refused`; a joiner gets `Welcome`, then the state in chunks, then events. The client applies them and reports hashes as now. | Unit tests with simulated clients: two players pick the same item in the same tick → one gets it, the other `Refused`; a full inventory refuses; a drop then an immediate pick-up by another player allowed; a late joiner's state equals the server's; 0 hash mismatches over 10,000 random ticks of picks, hits, drops, moves and deaths by 4 players; 1,000,000 bad inputs to the server, 0 panics. |
| **M9d3d** ✅ | **Done 2026-10-11** (MODLOG; facts in `gameplay.md` § Inventory, fingerprint in `protocol.md`). `sn_assets::item_rules` gives 158 pickup class ids, 102 tech types, 4 outcrops, 7 slots, 6 supplies, 126 analysis entries and 8 luck values; equal across two reads. It takes 2.1 s warm (batches and prefabs read in parallel; 4.7 s before). `ItemRules` moved to `sn-sim::inventory`, so `sn-assets` builds it; the session re-exports it. `Rules.items` feeds the server and clients, and the fingerprint has an `items` part. `sn-inspect coop` against `sn-server` (4 bots, lan and bad): all ticks drawn as before, 361 hash checks, 0 differ. Each bot loaded the item state (280–718 B, the locker's supplies in it). Plan as written: **Rules from the install** (`sn-assets`). `item_rules(assets)` builds `ItemRules` from M9d1's readers; the rules fingerprint gets an `items` part; server and client build it at start. | Real-data test: counts equal M9d1's (101 tech types, 7 slots, 6 supplies, 126 analysis entries, 4 outcrop prefabs); every supply and outcrop resource has a class id; fingerprint stable across two reads. |

**Not 1:1 after M9d** (everything known; each goes where its subsystem
is built; none is dropped):
- **Physics** (§ 3.3's physics decision, first used in M11 with the
  floating lifepod): dropped items, outcrop resources and death drops
  stay where they appear instead of falling and tumbling (rigid body,
  the break's torque and push, a drop's push velocity).
- **Sounds** ("After Phase E" item 8): pick-up and drop sounds (TechData
  `soundPickup` / `soundDrop`), the outcrop's hit and break sounds.
- **Effects** (M7i, particle systems): the outcrop's hit and break
  particles, `VFXBurstModel`'s break.
- **The game's UI** (M9u, below): the PDA's inventory tab and equipment
  screen with icons, dragging, the hand reticle's icons and text style,
  the pick-up icon notifier, inventory notifications, "inventory full"
  and "can't drop here" messages, the unlock message. M9d has our own
  panels and text.
- **Tools, quick slots and item use** ("After Phase E" item 1): quick
  slots (tools are bound to 1–5 when picked up; scrap metal never), held
  tools, the bash animation with a tool, eating, using and equipping
  effects (tanks' oxygen through `Inventory.ChangeOxygen`, fins' speed,
  suits).
- **Per-item state** (with each item in item 1, saved in M9f): battery
  charge, a tank's oxygen, food decay; M9d's items are tech types only.
- **Supply crates** in the world (`SupplyCrate`, `LootSpawner`'s
  `supplyTechTypes`): item 1.
- **PDA and goals** (item 2 and item 7): the databank entry and PDA
  message an unlock or pick-up starts (`GoalManager` custom goals,
  encyclopedia).
- **Creatures** (item 4): a dropped item's `Smell`; creatures attracted
  to it.
- **Vehicles and bases** (items 5 and 6): the Prawn suit's claw and the
  propulsion cannon picking up and breaking; storage lockers in bases;
  `SecureItems` and `destroyOnDeath` cleared inside a sub; water parks;
  dropping inside a sub (`Place`).
- **Look** (item 9): `SkyEnvironmentChanged` on items moved between
  inside and outside lighting.

#### M9u plan: the game's UI (written 2026-10-12, after M9d; approved by the user the same day)

The game's screens are uGUI (146 `uGUI_*` classes): canvases of
`RectTransform`s, `Image`s with sprites from `SpriteAtlas`es,
TextMesh Pro text with SDF fonts, masks and animated panels. M9u reads
them from the install and draws them as the game does, so the HUD, the
hand reticle, the messages and the PDA's inventory tab look and behave
like the game's. It replaces M9c's HUD (`hud.rs`) and M9d5b's panel
(`inventory.rs`). It comes before M9e: the fabricator menu
(`uGUI_CraftingMenu`) is built on M9u's pieces. The agent checks it by
numbers (rectangles, vertex counts, timings against values read). The
screenshots are for the user.

**Found while writing this plan** (2026-10-12, `sn-inspect unity
--types`, a throwaway probe in `out/probe` that reads only names and
counts, `sn-inspect code --il`). These are *confirmed* by those reads
unless marked otherwise:
- **Where it is.** Three Addressables bundles and two scene files:
  - `allprefabs_asset_ugui.prefab_…` (`Assets/uGUI/uGUI.prefab`):
    13 `Canvas`, 385 `RectTransform`, 292 `CanvasRenderer`, 29
    `CanvasGroup`, 98 `Sprite`, 75 `Texture2D`, 12 materials, 7
    shaders, 6 clips with 5 legacy `Animation`s, and 504
    MonoBehaviours. Those include 148 `Image`, 110 `TextMeshProUGUI`,
    15 + 9 horizontal and vertical layout groups, 19
    `ContentSizeFitter`s, `HandReticle` with 10 `uGUI_HandReticleIcon`,
    the four bars (`uGUI_OxygenBar`, `HealthBar`, `FoodBar`,
    `WaterBar`, 5 `uGUI_CircularBar`), `uGUI_DepthCompass`,
    `ErrorMessage`, `uGUI_IconNotifier`, `uGUI_PopupNotification`,
    `uGUI_QuickSlots`, `uGUI_PlayerDeath`, `uGUI_SceneRespawning`,
    `uGUI_Tooltip` and 4 `uGUI_CanvasScaler`.
  - `main.unity`'s `sharedAssets` file holds the PDA: `uGUI_PDA`, its
    tabs (`uGUI_InventoryTab`, `LogTab`, `EncyclopediaTab`, `PingTab`,
    `GalleryTab`, `TimeCapsuleTab`), `uGUI_ItemsContainer` and
    `uGUI_Equipment`. Its prefab is referenced from the scene's `PDA`
    script (*hypothesis*: through `PDA`'s screen prefab field).
  - `essentials.unity`'s and `xmenu.unity`'s files hold the options
    and the main menu (out of scope).
  - `atlases_…`: 8 `SpriteAtlas` (class 687078895). `Items` packs 470
    sprites, `Categories` 29, `Tabs` 12, `Pings` 9, `Log` 7,
    `Backgrounds` 4, `Default` and `ItemActions` 2 each. Seven of the
    atlas textures are uncompressed RGBA32 pages in this bundle. The
    `Items` atlas's page and its sprites are in another bundle (the
    atlas file's one external, `CAB-ef9560b0…`). `SpriteManager.Group`'s
    values (None, Item, Background, Category, Log, Tab, Pings,
    ItemActions) match the atlases' names.
  - `fonts_assets_all_…`: 9 `Font`s with their TrueType/OpenType bytes
    (Aller_Rg, Aller_W_Bd, PTSans, PTSans-Bold, OpenSans, Noto Sans
    CJK SC and Thai, and a debug font), 8 `TMP_FontAsset`s, 1
    `TMP_SpriteAsset` (`InputSymbols`, a 1024×512 page). Shaders:
    `TextMeshPro/Distance Field` and `…/Mobile/Distance Field`.
- **Text atlases are mostly built while the game runs.** Only
  `Aller_Rg Atlas` is stored (1024×1024, Alpha8). The other font
  atlases are 0×0 in the file, so TMP fills them from the font's
  TrueType bytes when the game runs.
- **Canvases are in world space.** 12 of the 13 canvases have render
  mode 2 (world space), one has 0 (screen-space overlay).
  `uGUI_CanvasScaler.UpdateTransform` places a canvas at the UI camera's
  position + forward × `distance`, with the camera's rotation, or at an
  `_anchor`'s pose. The UI camera is the main scene's `MainCamera (UI)`
  (field of view 60, near 0.03, mask `0x20` = layer 5 UI).
- **UI shaders and materials:** `UI/Simple` (`HUDBackground`,
  `OxygenBarPulse`, keyword `ALPHA_PREMULTIPLY`), `UI/Bar`
  (`_FILL_HORIZONTAL`, `SHEAR`), `uGUI/WorldCursor`, `uGUI/Compass`,
  `UI/ResourcesBlip`, `uGUI/IntroLogo`, `uGUI/DepthClear`; plus Unity's
  built-in `UI/Default` (external `unity default resources`). The fonts
  have Outline and Shadow material presets (`UNDERLAY_ON`).
- **Text** comes from the language files we already read (M9d1,
  `items.rs`). Static labels are translated by `TranslateOnStart` /
  `TranslationLiveUpdate`.

**Design.**
- **Layers.** `sn-dotnet` gives script field layouts. `sn-unity` reads
  the objects. A new crate `sn-ui` (layer 2, pure: no Bevy or files)
  does layout, mesh building, text, hit testing and the UI's rules.
  It outputs draw lists (vertices, a material, a clip rectangle, a
  sort key) that can be checked headlessly. The client draws those
  lists and sends it the input. The same split as `sn-sim` / the client.
- **Script fields from the DLLs, not by hand.** The UI needs dozens of
  MonoBehaviour layouts (Image, TMP's 80-odd fields, Selectable's colour
  blocks, layout groups, every `uGUI_*`). We would write those by hand
  the way `gameplay.rs` does. Instead, M9u reads the layout from the field metadata of
  `Assembly-CSharp`, `UnityEngine.UI.dll` and `Unity.TextMeshPro.dll`,
  using Unity's serialization rules. Those rules: public or
  `[SerializeField]`; not static, const, readonly or `[NonSerialized]`;
  `[Serializable]` classes and structs inline; `UnityEngine.Object`
  references as PPtrs; arrays and `List<T>`; base classes first; and the
  built-in structs (Vector2/3/4, Color, Color32, Rect, Quaternion,
  Matrix4x4, AnimationCurve, Gradient, LayerMask, RectOffset) from a
  table. Every object read must end at its last byte (`at_end`, as
  now). That check proves the layout object by object. The hand-written
  readers stay, and they must agree with the generic reader on the same
  objects.
- **Behaviour is ported, not run.** Each `uGUI_*` rule that M9u needs
  (bar smoothing and pulse, reticle states, message timings, PDA
  open/close, the items container) is read with `sn-inspect code --il`
  and written in `sn-ui` with the numbers from its fields, as M9c and
  M9d5b did. Its facts go in a new `docs/formats/ui.md`, each marked
  *confirmed* or *hypothesis*.
- **Drawing.** uGUI draws in hierarchy order: depth first, with nested
  canvases and their sorting orders, `CanvasGroup` alpha and
  `RectMask2D` clip rectangles. Our draw list keeps that order. The
  client draws each canvas as a world-space quad mesh, through our own
  UI camera with the game's numbers, after the 3D scene and its effects.
  Bevy's UI is not used for the game's screens. It stays only for
  debug text.
- **Fonts.** The stored `Aller_Rg` atlas is used as it is. Glyphs for
  the other atlases are made on demand, as TMP does: the glyph outline
  from the font bytes, rasterized at the asset's sampling size, then a
  distance field with its padding. **New dependency: `ttf-parser`.**
  It is pure Rust with no dependencies and parses TrueType and CFF
  outlines (Noto CJK is `.otf`/CFF). Writing a CFF reader ourselves
  isn't worth it. The rasterizer and the distance field are our own
  code. Check: we regenerate `Aller_Rg`'s glyphs our way and compare
  them with the shipped atlas.
- **Input.** A `GraphicRaycaster` equivalent: hits in reverse draw order
  on `raycastTarget` graphics, blocked by `CanvasGroup.blocksRaycasts`.
  Hover, click and drag go to the ported handlers. Keyboard and mouse
  only. Gamepad navigation (`uGUI_NavigableControlGrid`) is listed as
  not 1:1.
- **No network change.** The UI is local. The client's requests are the
  same M9d messages, so the protocol, server and rules fingerprint stay
  as they are.

**Steps.** Each one is one change, revertable alone, and has its own
MODLOG entry.

| Step | Work | Done when |
|---|---|---|
| **M9u0** ✅ | **Done 2026-10-12** (MODLOG; facts in `docs/formats/ui.md`; see "M9u as built"). `sn-inspect ui`: 0 of 23 checks differ in 0.73 s. Plan as written: **Census, headless.** `sn-inspect ui`: the files above, each canvas (render mode, sorting, scaler mode and `distance`), and the script hosts for the in-scope screens. Also who spawns `uGUI.prefab` and the PDA screen, and how (from the IL). `docs/formats/ui.md` started. | Counts equal the ones above (13 canvases, 385 rect transforms, 504 behaviours, 8 atlases, 470 item sprites, 9 fonts); every in-scope `uGUI_*` named with its object path; spawn path written as *confirmed* or *hypothesis*. |
| **M9u1** ✅ | **Done 2026-10-12** (MODLOG; facts in `docs/formats/ui.md` § Script fields; see "M9u as built"). `sn-inspect ui --scripts --all`: **45,137 of 45,137** MonoBehaviours of the whole game (1,205 script classes) end at their last byte; 9 objects agree with the hand-written readers. Plan as written: **Script layouts from the DLLs** (`sn-dotnet`: field signatures, `CustomAttribute`, type refs across the four assemblies; `sn-unity`: a generic field reader giving a value tree). | Synthetic tests (our own encoded assemblies and objects); real data: every MonoBehaviour in the uGUI prefab, the main scene's PDA, the fonts bundle and the main scene ends exactly at its last byte, N/N logged with each miss named; the generic reader equals the hand-written readers (`Player`, `PDAData`, `BreakableResource`, …) on every object they read. **Stop and ask** if more than 2% miss after two attempts. |
| **M9u2** ✅ | **Done 2026-10-12** (MODLOG; facts in `docs/formats/ui.md` § Sprite atlases; see "M9u as built"). Every UI built-in object of the game parses to its last byte (1,190 sprites, 8 atlases, 2,835 rect transforms, …); 470 of 470 `Items` sprites resolve; 20 sprites and both `Items` pages equal UnityPy's; 515 icons written. Plan as written: **Built-in UI classes and sprites** (`sn-unity`): `RectTransform`, `Canvas`, `CanvasGroup`, `CanvasRenderer`, `Sprite` (rect, border, pivot, its render data), `SpriteAtlas` (render data map); `SpriteManager`'s lookup rule by group and name from the IL; tech type → icon. | Every one of the game's 1,190 sprites and 8 atlases parses; every item tech type in `ItemRules` gets an icon or the game's fallback, counts logged; 20 sprites' rects and borders equal UnityPy's (dev-time oracle, not a dependency); `sn-inspect ui --icons` writes the item icons to `out/ui/` for the user. |
| **M9u3** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Fonts; see "M9u as built"). `sn-inspect ui --fonts`: 0 checks differ. Face metrics 136 of 136 fields equal (the hinted cap and mean lines reported apart: 12 of 24 differ by under a pixel); both stored glyphs equal in metrics, rectangles, used and free lists; their texels' mean error **1.04/255** (target ≤ 3); English 118 characters, 0 missing; 2.2 ms per glyph. Plan as written: **TMP fonts** (`sn-unity` + `sn-ui`; revised after M9u1, see "M9u as built"): `TMP_FontAsset` through M9u1 (face info, creation settings: point size, padding 9, render mode SDFAA); `Font` bytes; every glyph made on demand, as the game does (every asset is in dynamic mode). | Every font asset parses; our face metrics from each font file, at the asset's point size, equal its stored `m_FaceInfo` (8 assets, within 0.01); `Aller_Rg`'s 2 stored glyphs: our metrics equal theirs and our distance texels' mean error is logged (target written before the run: mean ≤ 3/255); English strings used by the in-scope screens have 0 missing glyphs; glyph cost logged. |
| **M9u4** | **Layout and meshes** (`sn-ui`, pure): RectTransform (anchors, pivot, size), layout groups, `FlexibleGridLayout`, `ContentSizeFitter`, `LayoutElement`, `AspectRatioFitter`; Image meshes (simple, sliced, tiled, filled with all fill methods); `RawImage`; TMP layout (size, alignment, wrap, overflow, line and character spacing, kerning, the rich-text tags the screens use, auto-size); draw order, `RectMask2D`, `CanvasGroup` alpha. | Unit tests on synthetic trees. Real data, *hypothesis to check first*: rects driven by layout groups and fitters are saved with their last editor values, so we can use them as an oracle; ours equal them within 0.01 px for N/N (each miss named), and text-fitted sizes equal within 0.5 px. Vertex counts per graphic logged. The draw-list hash is the same across two runs. |
| **M9u4a** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Layout). **The oracle hypothesis is false:** Unity saves driven values as 0, so 370 of 370 values our layout drives are stored as 0. This confirms *which* values are driven, not the values. As the risks paragraph foresaw, the values are checked by hand-computed unit tests (12) and the M9u5 screenshots; the "equal within 0.01" check below is replaced by "stored as 0, N/N". 168 of 370 wait for text sizes or unported components (M9u4c); a second run moves nothing. Plan as written: *Split from M9u4 on 2026-10-13 (one concern per change).* **Rects and the layout system** (`sn-ui::layout`, pure; rules read from the IL of `UnityEngine.UI.dll` and the game's DLL): RectTransform rects; `LayoutRebuilder`'s roots (`MarkLayoutForRebuild`) and four passes (calculate and control, horizontal then vertical); `LayoutUtility`'s priority rule; Horizontal/Vertical layout groups (padding, spacing, alignment, child control, force expand, child scale); `ContentSizeFitter`; `LayoutElement` (ignore, sizes, priority); `AspectRatioFitter`; `uGUI_CircularBar`'s self-sizing; `Image` as a layout element (sprite size, sliced/tiled minimum). Text sizes are unknown here: values that depend on them are counted apart, compared in M9u4c. `FlexibleGridLayout` and the game's other layout elements are logged where met. | Unit tests on synthetic trees, one per rule. Real data: first the hypothesis (driven values are saved as the editor last computed them); then every value our layout drives in the in-scope trees equals the stored one within 0.01 px, N/N, each miss named; text-dependent values and unported components counted apart. |
| **M9u4b** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Graphic meshes). 8 unit tests on hand-computed meshes; `ui --meshes`: 243 meshes, 2,028 vertices, logged per kind; 0 fail the checks (vertices inside rects, UVs inside sprites, indices). `useSpriteMesh` is drawn simple (not met enabled in scope: logged as its own kind); the game's own graphics are counted for M9u6/M9u8b. Plan as written: **Graphic meshes**: `Image` (simple, sliced, tiled, filled with every fill method, preserve aspect, a tight-packed sprite's trimmed margin, `pixelsPerUnitMultiplier`); `RawImage`. | Unit tests against hand-computed meshes; vertex counts per graphic logged for the in-scope trees. |
| **M9u4c** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Text layout). Ported from the decompiled TMP (read only, in `out/`). **Check revised:** text-fitted rects are driven, so they are saved as 0 (M9u4a), and no stored value can check them; instead **auto-size: 20 of 20 texts reach the size the editor stored** (`m_fontSize`). Unit tests: 12 hand-computed. M9u4a's unknown driven values drop from 168 to 19. Runtime kerning (3 texts) and the text quads (M9u5) are not built here. Plan as written: **TMP layout**: size, alignment, wrap, overflow, line and character spacing, kerning, the rich-text tags the screens use, auto-size; preferred sizes as a layout element. | Unit tests; text-fitted sizes equal the stored within 0.5 px; M9u4a's text-dependent values compared. |
| **M9u4d** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Draw order). 6 unit tests; `ui --draw`: 12 of 12 draw-list hashes equal across two runs from scratch (uGUI 174 draws, PDA screen 192, 2 of them clipped). Plain Transforms are now zero-size rects. Plan as written: **Draw order**: hierarchy order with nested canvases and sorting, `RectMask2D` clip rectangles, `CanvasGroup` alpha; the draw list. | Unit tests; the draw-list hash is the same across two runs. |
| **M9u5** ✅ | **Done 2026-10-13** in four steps (M9u5a–d, below): the HUD drawn with the game's shaders, 171 of 171 draws, 0.23 ms CPU per frame, screenshots at 1920×1080 and 2560×1440, 0 warnings or errors. Plan as written: **Drawing in the client**: the UI camera; world-space canvases placed by the `uGUI_CanvasScaler` rule; the UI shaders ported to WGSL (`UI/Default`, `UI/Simple`, `UI/Bar`, `TextMeshPro/Distance Field` with outline and underlay, `uGUI/WorldCursor`, `uGUI/Compass`) from their compiled passes, as in M7g; colour space as the game's. The HUD drawn static (its stored state). | Draw calls, vertices and textures per canvas logged; CPU cost per frame logged (target < 0.5 ms for the HUD); screenshots at 1920×1080 and 2560×1440 for the user; 0 GPU validation errors. |
| **M9u5a** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Materials and text vertices; see "M9u as built"). `ui --draw`: 372 of 372 draw items have a material; 817 text quads, 0 UVs off their glyph; 110 glyphs in 0.325 s (target ≤ 1 s); 12 of 12 hashes equal; 12 new unit tests and 1 opt-in real-data test. The census found `UI/Circle` (43 PDA draws) and `uGUI/DepthClear` (1), not in the research: rows M9u5c and M9u8b updated. Plan as written: *Split from M9u5 on 2026-10-13 (one concern per change; research in `docs/formats/ui.md` § UI shaders and materials).* **Materials and text vertices** (headless; `sn-assets`, `sn-ui`): each graphic's material (`m_Material`; none → `UI/Default` from `unity_builtin_extra`; a text → its `m_sharedMaterial`, else its font asset's material): shader name, keywords, the properties its programs read, its textures; `UNITY_UI_CLIP_RECT` and `_TextureSampleAdd` by uGUI's rules (IL). Text quads get their glyph's atlas UVs and TMP's `uv2` (scale and packed face UV, from the decompiled `TMP_Text`, read only) and normal; the glyphs every stored text needs are made at load into the font's `DynamicAtlas`. `ui --draw` logs draws per (shader, keyword set) and names each one that M9u5c–d won't port. | Every draw item has a material, N/N; the census per shader and keywords logged; every text vertex's UV inside its glyph's atlas rect, N/N; glyph count and load time logged (target written before the run: ≤ 1 s for the HUD and the PDA screen, else the distance field is sped up first); unit tests (material selection, `uv2` packing, clip keyword); two runs from scratch give equal hashes. |
| **M9u5b** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Canvas placement; see "M9u as built"). `ui --canvas`: 16 of 16 Static placements (4 scalers × 4 screens) put the corners on the screen's within 0.0001 px; trees laid out at the placed sizes, 0 non-finite; 7 unit tests and 1 opt-in real-data test. Plan as written: **The UI camera and canvas placement** (`sn-ui::scaler`, pure): `uGUI_CanvasScaler` from its IL (`UpdateFrustum`, `UpdateTransform`, `SetScaleFactor`; Static mode, `scaleMode` and `_uiScale`), the UI camera's numbers from the main scene (`MainCamera (UI)`: field of view, near, far, clear flags, mask, its pose relative to the main camera). | Unit tests on hand-computed cases; `ui --canvas` at 1920×1080 and 2560×1440 logs each top-level canvas's size, scale factor and pose against the camera, and where its rect's corners project on the screen (expected before the run: the screen's corners, within 0.5 px). |
| **M9u5c** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Drawing in the client; see "M9u as built"). `--game-ui`: 96 of 96 draws at 1920×1080 and 2560×1440, CPU 0.21 ms per frame, 0 warnings or errors; screenshots `out/m9u5c-1080.png` and `-1440.png`. **Found:** the stored HUD is black (4 full-screen faders and overlays), so the scripts' start-up state was ported (`ui_start_state`). The stored state of the other screens (feedback smileys, the confirmation dialog, pop-ups) still shows until M9u6 and M9u7. Plan as written: **The client draws images**: `--game-ui` (off by default until M9u6 removes `hud.rs`, so the two HUDs never draw together); the worker builds `uGUI`'s draw list at load; textures uploaded in their stored colour space; one render pass of our own after tonemapping (as `effects.rs`), in the draw list's order (Bevy's transparent sort would reorder it), with the UI camera's projection, each item's clip rect, softness and blend state; `UI/Default` and `UI/Simple` in WGSL from their programs. Texts skipped and counted. `uGUI/DepthClear` (the `DepthClear` canvas: depth only, colour mask 0) draws nothing in our pass, which does not use depth; it is skipped and logged (M9u5a census). | Draw calls, vertices and textures per canvas logged; CPU cost per frame logged; 0 GPU validation errors; a screenshot at 1920×1080 for the user. |
| **M9u5d** ✅ | **Done 2026-10-13** (MODLOG; facts in `docs/formats/ui.md` § Text). 75 text draws (both faces, outline and shadow material) at 1920×1080 and 2560×1440; pass CPU 0.23 ms per frame (target < 0.5); 101 glyphs in 0.7 s on the worker; 0 warnings or errors. `UI/Bar`, `uGUI/WorldCursor` and `uGUI/Compass` are not ported: `UI/Bar` (2 draws, the exosuit and loading bars) is drawn as `UI/Default` until its screens are ported; `WorldCursor` is not in the HUD tree. Plan as written: **The client draws text**: `TextMeshPro/Distance Field` in WGSL (face, outline, underlay, soft clip) and the font atlases uploaded (updated when glyphs are added); the other shaders M9u5a's census finds drawn in scope (`UI/Bar`, `uGUI/WorldCursor`, `uGUI/Compass` otherwise wait for M9u6 and M9u8). M9u5a's census: `UI/Bar` 2 draws (`ExosuitThrust`, `LoadingBar`), `uGUI/WorldCursor` 1; `UI/Circle` is only on the PDA screen and moves to M9u8b. Closes M9u5. | M9u5's own list: draw calls, vertices and textures per canvas; CPU < 0.5 ms per frame for the HUD; screenshots at 1920×1080 and 2560×1440; 0 GPU validation errors. |
| **M9u6** ✅ | **Done 2026-10-17** (as M9u6a–e below; bars: `ui --bars`/`--pulse` and the client logs; depth and compass: `ui --depth`/`--compass`; reticle: `ui --reticle`; death: `ui --death`, the client drowning; `hud.rs` gone in M9u6e2). Plan as written: **The HUD's rules**: the four bars (`uGUI_CircularBar` and each bar's smoothing, pulse and warning), the depth, the compass while one is equipped, the hand reticle (icon per hand state, text and subscript, progress), the death and respawn screens and the fader. `hud.rs` removed. *Split on 2026-10-14 into M9u6a–e below (one concern per change; scripts read from the decompiled `Assembly-CSharp`, read only, in `out/`).* | `--dive` and `pickup`-style scripted runs in the client log each bar's shown value against the rule's each second, the reticle's icon per target kind and the fade timings against the values read; our HUD's code gone. |
| **M9u6a** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The live UI; see "M9u as built"). `ui --live`: hashes unchanged (stored `83d22425b9e33ca7`, placed `f1365274ab2fbbf7`; `ui --draw`'s 12 and `ui --canvas`'s 4 equal); 7 of 7 change kinds act and the start hash comes back after each; 120 frames with every HUD text changed: mean 2.66–2.81 ms (target ≤ 4); client 171 of 171 draws, 0 warnings. Plan as written: **The live UI** (`sn-assets`, `sn-ui`; the base every screen's rules write to). The install is read once into a `UiScene` (the tree, its layout tree, draw parts, graphics, texts, materials); `UiScene::draw(placements, state)` rebuilds the draw list from a `UiState`: the changes the game's scripts make through Unity's API, each as uGUI applies it: `SetActive`, `CanvasGroup.alpha`, `CanvasRenderer.SetAlpha`, `Graphic.enabled` and `color`, `Image.sprite` and `fillAmount`, TMP `text` and `color`, a node's local position, rotation and scale. `UiStartState` becomes the first `UiState`. Helpers the rules share, from the decompiled code: `Mathf.SmoothDamp` (`UnityEngine.CoreModule`), UWE's `Sequence`. The client's worker keeps the scene and rebuilds when the main world sends a new state or size. | Unit tests (each override kind on a synthetic tree; `SmoothDamp` and `Sequence` against hand-computed values). `ui --live`: hashes equal the ones before the change (stored state: `ui --draw` uGUI `83d22425b9e33ca7`, PDA screen `52a23807bd350bbb`; placed: `ui --canvas` 1920×1080 `f1365274ab2fbbf7`); a state applied then removed gives the start hash again; rebuild time per changed state logged (target written before the run: mean ≤ 4 ms for uGUI at 1920×1080 with every HUD text changed; above it, text meshes are cached per text before going on). Client: `--game-ui` still 171 of 171 draws, 0 warnings or errors. |
| **M9u6b** ✅ | **Done 2026-10-14** (as M9u6b1 and M9u6b2 below). **The four bars** *(split on 2026-10-14 into M9u6b1–b2 below: the shader, values and texts first, then the animations; research in `docs/formats/ui.md` § The HUD's bars)*: `uGUI_SceneHUD` (bars and backgrounds by game mode, `IsActive`), `uGUI_CircularBar` (self-sizing, material values, keywords) and its shader in WGSL from its compiled passes, and the oxygen, health, food and water bars' rules: `SmoothDamp` value, the number text, the pulse (curves, its legacy `Animation` clip), the oxygen overlays' scroll and alpha, the health icon's coin turn and punch. Food and water show their starting values (out of scope above). | A `--dive` run logs each bar's shown value and text against the rule's, each second (equal), and the pulse period against the curves at three oxygen levels; the bars' draws counted under the new shader; 0 GPU errors; screenshot for the user. |
| **M9u6b1** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The HUD's bars; see "M9u as built"). `ui --bars`: 300 of 300 bar checks equal the replay (value drawn, text, its quads, oxygen overlay ST and alpha); draw mean 2.6–2.9 ms. Client dive: the per-second log follows the simulation (oxygen 45 → 0), 176 of 176 draws, worker 3.3–4.4 ms per frame, 0 warnings. The screenshot is no visual check yet: stored-state screens cover the bars (now M9u6s). Plan as written: **The bars drawn and driven**: `UiState` moved to `sn-ui` (sprites as ids, so the rules stay pure) with per-node material values (the scripts' `material.SetFloat`/`SetVector`); `uGUI_CircularBar` as a graphic (`Graphic`'s quad, its own material instance with the keywords and values its setters write); `Hidden/UI/CircularBar` in WGSL from its 8 + 8 programs (a third pipeline kind, with `_OverlayTex`); `sn-ui::hud` (pure): `uGUI_SceneHUD` (game mode Survival: the four bars and the quad background; `IsActive`), the oxygen, health, food and water bars' `SetValue` (`SmoothDamp` with `PDA.deltaTime`, the text's rounding and caching) and the oxygen overlays' scroll and alpha; `sn-assets` reads their fields and nodes; the per-frame path in the client (the main world sends the rules' inputs, the worker runs the rules and draws). Food and water show `Survival`'s stored start values. | `ui --bars` (headless): a scripted oxygen and health timeline drives the rules; each second, each bar's `_Value` and text in the draw list equal an independent `SmoothDamp` replay (N/N), and the overlay ST and alpha equal the formula's; unit tests for the rules. Client `--game-ui` dive: each bar's shown value and text logged each second against the simulation's; 0 GPU errors; the per-frame draw time on the worker logged; screenshot for the user. |
| **M9u6b2** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The bars' animations and `docs/formats/animation.md` § Editor curves). `ui --pulse`: 13,440 of 13,440 halo alphas equal a replay from the decompiled scripts; the pulse period at oxygen 6, 3, 1 of 45 and health 12 within one frame of the curves' delay + time (0.64, 0.52, 0.44, 1.26 s); 300 of 300 icon widths during punches equal the clip's scale + the punch; the coin turn logged. Corrections to the plan: the health punch is on healing (`LiveMixin.AddHealth` fires `onHealDamage`), not on damage; `pulseReferenceCapacity` is stored as 100 on all four bars (oxygen's code default is 45). `CoinRotation` steps whole seconds (`FloorToInt`), ported as written. `UnityEngine.Random` is not reproduced (our generator, same ranges). Our player is never healed and never eats or drinks yet: the punches are checked headless only. Plan as written: **The bars' animations**: the legacy `Animation` clips (`OxygenBarPulse`, `IconPulse`: editor curves for scale and `CanvasGroup` alpha, read in `sn-unity`), `CoroutineTween`'s loop, each bar's pulse delay and time from its curves, the health, food and water icons' coin turn (`MathExtensions.CoinRotation`, while the PDA is open) and punch (`Oscillation`, on damage, eating, drinking). | Unit tests (curve sampling, tween, coin turn and oscillation against hand-computed values). A `--dive` run logs the oxygen pulse's period at three oxygen levels against the curves' delay + time, and the halo alpha against the clip's curve. |
| **M9u6s** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § Start-up state). `ui --start`: 37 start rules applied; 151 visible draws in 8 screens before, 23 after, 23 of 23 named with the rule that shows them; the only draw over 5 % of its canvas is `DepthClear` (not drawn by our pass). The bars' pulse halos and wave stay in their stored state until M9u6b2. Found and fixed: the client dropped the textures of a UI build superseded in the same frame (every image a flat square). Client: 62 of 62 draws, 0 warnings; screenshot `out/m9u6s-dive.png`. Plan as written: *Added 2026-10-14 (moved out of M9u6e, so that every later screenshot can show the HUD).* **The other screens' start rules**: each screen of the HUD tree still drawn in its stored state at start (`FeedbackCollector`, `Confirmation`, the radiation, notification and pop-up panels, `CameraCyclops`, `CameraScannerRoom`, the loading screen, the intro's skip text and others the census finds) hidden or shown as its script's `Awake`/`Start` leaves it (decompiled), extending `ui_start`. Each one is named with its rule; a screen whose start depends on game state not ported is logged. | Census before and after: every node `ui --live` draws at start is named with the rule that shows it, N/N; full-canvas and large draws at start: only the ones the game shows; screenshot at 1920×1080 for the user. |
| **M9u6c** ✅ | **Done 2026-10-14** (as M9u6c1 and M9u6c2 below). **Depth and compass**: `uGUI_DepthCompass` (player mode: `floor` depth, `MeterSuffix` from the language file, the depth class's sprites, colours and blending as the decompiled code sets them; hidden while the PDA is open or the player pilots), `uGUI_Compass` and `uGUI/Compass` in WGSL, shown only with a compass equipped (our equipment state), direction labels from the language file. *Split on 2026-10-14 into M9u6c1–c2 below (one script each; research in `docs/formats/ui.md` § The depth and compass).* | `--dive` logs the shown depth against `floor(depth)` each second; the compass's direction against the camera's yaw; compass shown iff one is equipped (a scripted equip and unequip). |
| **M9u6c1** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The depth and compass). `ui --depth`: 91,230 of 91,230 checks over 4,200 frames equal the replay; client dive: the shown depth equals `floor(depth)` in 29 of 29 shown seconds (3 → 19 m), hidden while the PDA is open, 0 warnings. Plan as written: **The depth (`uGUI_DepthCompass`)**: `sn_ui::depth` (pure): `Initialize` (the meter suffix from the language file, `OnDepthClassChanged` with the class the IL forces to `Safe`: `shadowNormal`, `SetBlending(Multiplicative, premultiplied)` on the shadow's own material, `textColorNormal` on both texts), `UpdateDepth` (mode `Player` or `None`; on a change the player and submersible graphics enabled or not and the half moon's sprite; the text when `floor(depth)` changes, `IntStringCache` = the integer's invariant string), `UpdateCompass`'s visibility part (`compass.SetVisible` → the compass graphic and its 8 labels enabled; the half moon's compass sprites). New `HudInputs`: `depth` (`Ocean.GetDepthOf(player)`), `compass` (false until M9u6c2). `sn-assets::depth_config`; `sn-inspect ui --depth` (a scripted timeline checked against a replay); the client sends the depth and logs it each second. | `ui --depth`: every frame's text equals `floor(max(0, −y))`'s string, the suffix equals the language file's `MeterSuffix`, the player graphics are hidden exactly while the PDA is open or the HUD is inactive, the half moon's sprite follows the compass flag, N/N; client dive: the per-second log's shown depth equals `floor(depth)` every second; 0 warnings. |
| **M9u6c2** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The depth and compass). `ui --compass`: 644 of 644 checks (the 8 stored label places equal ours at the stored `debugDirection` within 0.00005 px; 36 headings against an `f64` replay; shown iff equipped); client: direction = yaw ÷ 360 in 10 of 10 shown seconds across an equip, the PDA and an unequip; 0 warnings; `out/m9u6c2-compass.png`. Settled: `--test-supply` (host or solo; the item's pickup joins the rules). Plan as written: **The compass (`uGUI_Compass`)**: `uGUI/Compass` in WGSL (from its 2 + 2 compiled programs, M9u5 research); `UiGraphicKind::Compass` with its fields; `OnPopulateMesh`'s projected quad (`Matrix4x4.Perspective`, `TRS`, the 1/z UVs) in `sn_ui::compass`; `direction` from the camera's yaw (`eulerAngles.y / 360`, set only on a change): `_Angle`, `_AlphaFrom`, `_AlphaTo` and the labels' positions and alphas (`UpdateLabels`); label texts `CompassDirection{name}` from the language file; `compass` input = the local player's equipment holds a `Compass` (`TechType` read from the DLL). The scripted equip needs a compass in the inventory: *to settle before code* how a test run gets one without a cheat in normal play (a host-only start item, logged). | `ui --compass`: the mesh's 4 vertices and every label's position and alpha equal a replay of the decompiled code at 36 headings, N/N; client: the per-second log's direction equals the camera's yaw ÷ 360; compass drawn iff equipped across a scripted equip and unequip; 0 warnings; a screenshot for the user. |
| **M9u6d** ✅ | **Done 2026-10-14** (as M9u6d1 and M9u6d2 below). **The hand reticle**: `HandReticle` (the icon per state with `uGUI_HandReticleIcon`'s 0.1 s fade, the icon scale's lerp, the four texts and their show rule, the progress image's radial fill and `HandReticleProgressPercentFormat`), driven by our hand logic (M9d5); our hand text removed. *Split on 2026-10-14 into M9u6d1–d2 below: the hand texts end in the key's icon, a TMP `<sprite>` (research in `docs/formats/ui.md` § TMP inline sprites).* | A `pickup`-style run logs the icon per target kind, each fade's length against 0.1 s, and the texts equal M9d5's strings. |
| **M9u6d1** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § TMP inline sprites). `ui --tmp-sprites` (the planned `ui --sprites` was taken): 371 of 371 checks (164 of 164 names resolve; `MouseButtonLeft` at 24 px: scale, 44.0294 px quad, UVs at the glyph rect, `#ADF8FFFF`); 9 new unit tests; client 0 warnings. The sprite's material is a Subnautica change (`FindMatchingMaterial`: Outline / Shadow by name), all `TextMeshPro/Distance Field`, so no `TextMeshPro/Sprite` WGSL. Plan as written: **TMP inline sprites (`<sprite>`)**: the sprite asset read (`TMP_SpriteAsset`: face info, sprite glyph table (rect, metrics, scale), character table (name, unicode, scale), its `spriteSheet` texture and material, fallbacks); the runtime default sprite asset as `LanguageSDF` sets it (the first sprite asset of its entry points: `InputSymbols`, with the rest as fallbacks); the tag (`name=`, index, `tint=`, `color=`) as `ValidateHtmlTag` parses it; the sprite element's scale, ascent/descent, advance and line metrics in `GenerateTextMesh`; its quad (`SaveSpriteVertexInfo`: colour rule, UVs from the glyph rect) in its own sub-mesh with the sprite asset's material; `TextMeshPro/Sprite` in WGSL from its compiled programs (if that is the material's shader). | Unit tests on hand-computed sprite layouts; `ui --sprites`: a text with `<sprite name="MouseButtonLeft" color=#ADF8FFFF>` laid out and drawn: its quad's size against the formula, UVs inside the glyph rect, its colour; every sprite name in `InputSymbols` resolves; 0 warnings in the client. |
| **M9u6d2** ✅ | **Done 2026-10-14** (MODLOG; facts in `docs/formats/ui.md` § The hand reticle). `ui --reticle`: 22 of 22 checks, 6,480 frame checks equal to the replay, fades 0.1000 s; client: outcrop, dropped ore, ladder hatch and locker each show `Hand` and their text with the mouse sprite, fades within 0.1 s + one frame, `None` while the panel is open, 0 warnings; screenshots. *Not reached in the client*: a full inventory (no way to fill it without a new test flag; covered by `ui --reticle` and the `hand::aim` test). Also fixed: the start state's hidden icons (inactive, not alpha 0). Plan as written: **The reticle's rules**: `sn_ui::reticle` (`HandReticle.LateUpdate`: the texts and their show rule, `SetIconInternal` with `uGUI_HandReticleIcon`'s `Sequence` fade of 0.1 s (0 to `None`), the icon canvas's scale lerp, `UpdateProgress` with `HandReticleProgressPercentFormat` (`{0:P0}`), then the reset to `Default`); `GetText`'s button format (`HandReticleAddButtonFormat`, `GameInput.FormatButton(LeftHand)` → the keyboard's default binding `MouseButtonLeft` → a sprite tag when the default sprite asset has that sprite, else the language text, coloured `#ADF8FFFF`); M9d5's `hand::aim` gives the icon (`Hand`, `PackUp` by `usePackUpIcon`, or none: `Default`) and the button; hidden (`None`) while the PDA is open or the player has no control. Our hand text (`hud.rs`) removed under `--game-ui`. | A `pickup`-style client run logs the icon per target kind (pickup, full inventory, outcrop, locker, nothing), each fade's length against 0.1 s, and the texts equal the game's strings; `ui --reticle` checks a scripted timeline against a replay. |
| **M9u6e** ✅ | **Done 2026-10-17** (as M9u6e1 and M9u6e2 below). Plan as written: **Death, respawn and overlays**: `uGUI_PlayerDeath` (fade to black, its 6 s, fade out), `uGUI_SceneRespawning` with `uGUI_Fader` (sequence, cosine curve, fade times read), `uGUI_Overlays`' out-of-oxygen curve from our suffocation value (the other screens' start rules moved to M9u6s); `--game-ui` on by default and `hud.rs` removed (closes M9u6). *Split on 2026-10-17 into M9u6e1–e2 below: the screens' rules first, then the switch-over (research in `docs/formats/ui.md` § Death, respawn and overlays).* | A scripted drowning and respawn logs the black overlay's alpha and the fader's against the rules' times; the overlay's alpha against its curve; our HUD's code gone; screenshot for the user. |
| **M9u6e1** ✅ | **Done 2026-10-17** (MODLOG; facts in `docs/formats/ui.md` § Death, respawn and overlays; see "M9u as built"). `ui --death`: 24 of 24 checks, 9,840 frame checks over a scripted drowning equal to the replay; client drowning: the overlay equals its curve in 85 of 85 logged seconds, `DeathFade` 11 ms after the death, `BeginFadeOut` 6.006 s after it, the respawn fader clear 1.014 s after `Hide`, 0 warnings; screenshots. Plan as written: **Death, respawn and overlays: the rules.** `sn_ui::death` (pure): `uGUI_PlayerDeath` (`Start`: `blackOverlay` colour (0, 0, 0, 0); `TriggerDeathVignette(type)` once while inactive: `FadeToBlack` → `Invoke(BeginDeathFade, 0)`, `CutToBlack` → `CutToBlack()` at once, both `Invoke(BeginFadeOut, 6)`; `Update`'s `Color.Lerp` by `deltaTime × 5` in (to 1 at ≥ 0.98) and `× 2` out (reset below 0.02); Unity's `Invoke` run after the frame's `Update`s, *hypothesis*), `uGUI_Fader` (`Sequence`, `0.5 (1 − cos π t)`, `FadeOut` uses `fadeInTime` and `FadeIn` `fadeOutTime` as the code does; unscaled time), `uGUI_SceneRespawning` (`Show`: `Loading` from the language file, `SetState(true)`; `Hide`: `FadeOut()`), `uGUI_Overlays` (`Set(0, v)`: the overlay's curve, clamped; disabled at ≤ 0). `sn-assets::death_config` (nodes, fader fields, the overlay's curve; the vignette's delays, rates and thresholds from the IL: `Invoke` calls, `ldc.r4; mul`, the `Color.a` compares). Inputs: the suffocation value (`sn_sim::vitals::overlay`, 1 on the death's frame as `OnKill` sets it), the respawn's `Show`/`Hide` (vitals' `MoveToRespawn`/`Restored`), and the player animator's `DeathFade`/`DeathCut` events. `sn-inspect ui --death` (a scripted drowning timeline against a replay); our M9c fade off under `--game-ui`. | `ui --death`: every frame's black overlay alpha and enabled flag, fader alpha, loading text and overlay alpha equal a replay of the decompiled code, N/N; the vignette's times (in, hold until 6 s, out) and the fader's 1 s logged against the values read. Client drowning (`--start 0 -3 0 --look-down 60 --hold-forward 3`): the log's death event, `DeathFade`/`DeathCut` time, overlay alpha each second against its curve, the vignette's and the fader's alphas against the rules' times; 0 warnings; screenshots (dying, black, respawned). |
| **M9u6e2** ✅ | **Done 2026-10-17** (MODLOG). `hud.rs` gone (no `hud::` in the client); default run: 98 game UI lines, `--no-game-ui`: none, `--free-cam` benchmark: no panic, 0 warnings each; 669 tests pass without the game. Not tested: `--baseline`, a joiner. Plan as written: **The game's UI by default; our HUD gone** (closes M9u6). `--game-ui` on by default (`--no-game-ui` to turn it off, logged; a `--baseline` report records which); `hud.rs` removed (its `Hud` resource becomes the player's feed to the game UI in `player.rs`/`game_ui.rs`); the help text, `README.md` and M9u6's row updated. | `cargo test --workspace` passes; `grep` finds no `hud::` in `sn-client`; a default client run draws the game's HUD (its log) and a `--no-game-ui` run draws none; 0 warnings; the M9u6 row's checks (bars, depth, reticle, death) each pointed at their step's evidence. |
| **M9u7** | **Messages**: `ErrorMessage` (inventory full, can't drop here), `uGUI_IconNotifier` (picked-up icons), `uGUI_PopupNotification` (blueprint unlocked), the inventory tab's notification count; text from the language file. *Split on 2026-10-17 into M9u7a–d below: the messages make UI objects at run time (pools of a template, `new GameObject`, `SetAsLastSibling`), which the scene cannot do yet; research in `docs/formats/ui.md` § Messages.* | A scripted pick-up run logs each message's language key, its show and hide times against the IL's numbers, and the queue length at its limit. |
| **M9u7a** ✅ | **Done 2026-10-17** (MODLOG; facts in `docs/formats/ui.md` § Objects made at run time). `ui --instantiate`: 10 of 10 (the copy equals the original moved by the offset, 0.000000 error; message copies in sibling order; `SetAsLastSibling`; 55 stored draws unchanged); every older `ui` check unchanged. Not done here: nodes with no stored template (moved to M9u7c with `uGUI_ItemIcon`). Plan as written: **Objects made at run time.** `UiScene::instantiate(template, parent)`: a root of `uGUI.prefab`'s file other than `uGUI` (`MessageInstance`, `NotificationLabel`, …; `sn-inspect ui` lists 10) read as its own tree (layout, draw parts, graphics, texts) and grafted under a scene node: its nodes appended (no stored index moves), its root added as the parent's last child (`SetParent(parent, false)`), its stored local values kept; `UiState` gains a node's sibling index (`SetAsLastSibling`) and the scene can hold nodes with no stored object (`new GameObject` + components made by code, for `uGUI_ItemIcon` in M9u7c). Synthetic tests on hand-made trees. | `ui --instantiate`: a copy of a stored subtree of `uGUI` grafted beside it draws the same items (vertices, colours, textures, order) moved by the expected offset, N/N; four `MessageInstance` copies under `ErrorMessageCanvas` draw in sibling order; `SetAsLastSibling` moves a draw last among its siblings; every older `ui` check unchanged (hashes). |
| **M9u7b** ✅ | **Done 2026-10-17** (MODLOG; facts in `docs/formats/ui.md` § Text messages). `ui --messages`: 7 of 7, 900 frames and 90 draws equal to the replay; client drowning: "You died." at controls back, gone after 6.0106 s (rule 6 s), 0 warnings, a screenshot. `InventoryCantDropHere` is not reached (only held tools call it); "Inventory full" not tested in the client. Plan as written: **`ErrorMessage`.** `sn_ui::messages` (pure): `_AddMessage` (merging an equal text into `"{text} (x{n})"` and renewing its time), the pool of `prefabMessage` copies in chunks of 4 (`GetEntry`/`ReleaseEntry`), `OnLateUpdate` (expiry by `PDA.time`, `offsetY`'s `× 7` decay, the fly-in from the centre with `EaseOutSine` and the fade, `GetYPos` from the entries' preferred heights, `GetInverseScale`); the times (0.3, 5, 0.6, 0.1 s) read from the constructor (IL), `offset`/`ySpacing` from the stored fields. Inputs: the language keys the game's calls use that our rules reach (`YouDied`/`YouDiedLostStuff` after the respawn, `InventoryFull`, `InventoryCantDropHere`). | `ui --messages`: a scripted timeline (one message, a repeat merged to x2, three at once, expiry while others stay) against a replay every frame: each entry's position, alpha, text and the pool size, N/N; client: a drowning logs `YouDiedLostStuff` (or `YouDied`) at controls back, its show and hide times against 6 s; 0 warnings; a screenshot. |
| **M9u7c** ✅ | **Done 2026-10-17** (as M9u7c1 and M9u7c2 below). *Split on 2026-10-17 into M9u7c1–c2 below: the icon is a node made by code with the game's `uGUI_Icon` (a sprite's packed mesh) and `UI/Icon` shader, which the scene cannot draw yet; research in `docs/formats/ui.md` § Picked-up icons.* Plan as written: **Picked-up icons.** `uGUI_ItemIcon` as the notifier uses it (`Init`'s children made by code, `SetForegroundSprite` from the item's sprite (M9u2's 470), `SetSize`, `SetPosition`, `SetScale`, `SetAlpha`; the rest of the class (bars, labels, backgrounds by size) logged as not ported) and `uGUI_IconNotifier` (queue, `interval`, the two `IconAnimator`s' curves read, `Oscillation` with `Random.value` seeds, release at t = 1). Input: `Inventory.Pickup`'s `Play(techType, From)` from our pick-ups. | `ui --icons`: a scripted queue (3 pick-ups in one frame, one later) against a replay each frame: dequeue times 0.3 s apart, each icon's position, scale and alpha, N/N; client: a `--use-item` pick-up logs the icon's start, its end after the animator's duration, and draws the item's sprite; 0 warnings; a screenshot. |
| **M9u7c1** ✅ | **Done 2026-10-17** (MODLOG; facts in `docs/formats/ui.md` § Picked-up icons). `ui --icon-mesh`: 12 of 12, 21 icons drawn from their sprites' meshes with `UI/Icon`, UVs within 0.15 px of the packed rects; every older `ui` check unchanged. Plan as written: **Nodes made by code; `uGUI_Icon`; `UI/Icon`.** `UiScene::from_code`: a template whose nodes have no stored object (`new GameObject` + `AddComponent`: a new `RectTransform`'s defaults, `RectTransformExtensions.SetParams`), grafted as M9u7a's; `UiNode.game_object` becomes optional. `UiGraphicKind::Icon` (`uGUI_Icon`, its own material instance): `GeneratePackedMesh` (the sprite's `vertices` × (rect ÷ sprite size × ppu), `uv`, `triangles`; `GenerateQuadMesh` without a sprite); the sprite's mesh read from `m_RD` (positions; `uv` from the render data's `uvTransform` and page size, *hypothesis*: the stored UV stream is zeros). `UiState` gains `sizeDelta` (`SetSizeWithCurrentAnchors`) and `uGUI_Icon.sprite` (as `Image.sprite`). `UI/Icon` (in `preloadedshaders`): its base variant is `UI/Simple`'s program without premultiplying (`_SrcFactor`/`_DstFactor` for colour and alpha), drawn by that path; `FILL_*`, `GRAYSCALE`, `NOTIFICATION`, `SLICE_9_GRID`, `ALPHA_PREMULTIPLY` logged as not ported. The `uGUI_ItemIcon` the notifier uses: `Init`, `SetForegroundSprite` (`CreateForeground`, `Fit` keeping aspect), `SetSize`, `SetPosition`, `SetScale`, `SetAlpha` (`CanvasRenderer.SetColor` of white with alpha a); its own `OnPopulateMesh` is empty. | `ui --icon-mesh`: a `NotifierIcon` made by code under `IconNotifications` with the sprites of 20 tech types and `Unknown`: each draws one item, vertices = the sprite's, every UV inside its packed `textureRect`, the mesh's bounds inside the 64 × 64 fit, the material `UI/Icon` with no keyword, N/N; the stored nodes' draws unchanged; every older `ui` check unchanged. |
| **M9u7c2** ✅ | **Done 2026-10-17** (MODLOG). `ui --icons`: 10 of 10, 400 frames and 80 draws equal to the replay; at most three icons out, so the pool never grows past four (the plan's "repeat while the pool is out" cannot happen). Client: three pick-ups, shown 0.30–0.31 s apart, released after 0.7 s + under a step, 0 warnings, a screenshot. Plan as written: **`uGUI_IconNotifier`.** `sn_ui::notifier` (pure): `Play` (queue), `Update` (dequeue when `PDA.time − time ≥ interval`, `time` starting at −interval; `GetInstance`'s pool of four made at a time; `SetAsLastSibling`; `Random.value` from our generator, as M9u6b2), `Animate` (x, y × the canvas rect, scale + `Oscillation` × `oscScale` from `oscOffset`, alpha; released on the frame t = 1; t += `PDA.deltaTime` ÷ duration) with the stored `interval`, `iconSize` and the two `IconAnimator`s' curves. Frame order: after `PDA.UpdateTime` (*hypothesis*: `ManagedUpdate` and the notifier both have execution order 0). Input: `Inventory.Pickup`'s `Play(techType, From)` (our pick-ups; `noMessage` pick-ups not reached); `To` (batteries, crafting) not reached. | `ui --icons`: a scripted queue (3 pick-ups in one frame, one later, a repeat while the pool is out) against a replay each frame: dequeue times ≥ 0.3 s apart, each icon's position, scale and alpha, the pool size, N/N, and draws (one quad set per shown icon at its alpha); client: a pick-up logs the icon's start, its end after 0.7 s (+ a frame) and draws the item's sprite; 0 warnings; a screenshot. |
| **M9u7d** ✅ | **Done 2026-10-17** (as M9u7d1 and M9u7d2 below). *Rewritten and split on 2026-10-17 (research in `docs/formats/ui.md` § The unlock pop-up): the plan's input does not exist. `Inventory.Pickup` and `OnAddItem` call `KnownTech.Analyze(techType, verbose: false)`, so a pick-up never shows the pop-up nor adds a `Blueprints` notification; `OnAnalyze` only acts on a verbose `KnownTech.Add` (crafting through `OnCraft…` story goals, e.g. a battery unlocks the power cell; scanning; databoxes; the specimen analyzer; the rocket), none of which is ported. And `NotificationManager` shows nothing on the HUD: its inventory notifications are drawn by the PDA's item icons and the inventory tab's count, and they count down only while such a target is visible; it moves to M9u8b. M9u7d is now M9u7d1–d2 below.* Plan as written: **Notifications and the unlock pop-up.** `NotificationManager` (the inventory group: `Add(Inventory, id, 4)` on a pick-up, the count per group; the label drawn when its tab is drawn, M9u8a, logged until then) and `uGUI_PopupNotification` for `KnownTech`'s analysis unlocks (`OnAnalyze`: the `Unlock` skin, title and text from the language file, the tech's sprite, `defaultDuration`, the queue; sounds not ported, logged). Input: our `sn-sim` known-tech unlocks on a pick-up. | `ui --popup`: a scripted unlock pair against a replay (show, queue, hide times, texts); client: a pick-up that unlocks a blueprint logs the pop-up's title, text and times and the inventory count; 0 warnings; a screenshot. |
| **M9u7d1** ✅ | **Done 2026-10-17** (MODLOG; facts in `docs/formats/ui.md` § The unlock pop-up). `ui --popup`: 13 of 13, 1200 frames and 240 draws equal to the replay; every older `ui` check unchanged. Plan as written: **`uGUI_PopupNotification` and its `Unlock` skin** (headless; `sn_ui::popup`, `sn-assets`): `Enqueue`, `Show`, `Hide`, `ShowNext`, `OnUpdate`'s phases (`Trapezoid`/`Step` with `timeIn` 0.25, the entry's duration, `timeOut` 0.25; the group's alpha on each phase change; the next entry at `None`/`Done`), `Mathf.Approximately` before `OnLayoutComplete`'s `SetTransition` (`GetRectPositions` at `MiddleLeft` with `ox`/`oy`, `EaseOutSine`), `SetSkin`'s skin alphas, `uGUI_PopupNotificationDefault.OnShow` (title, text, controls, the image's sprite or `defaultSprite`); `OnAnalyze`'s entry (`defaultDuration`, title `Language.Get(unlockMessage)`, text `Language.Get(techType.AsString())`, `unlockPopup`). Frame order: `OnUpdate` in `UpdateAfterInput`, before `PDA.UpdateTime` (`UpdatePDA`), `SetTransition` in `UILayoutComplete`. Sounds not ported (named in the log). | `ui --popup`: a scripted queue (three `OnAnalyze` entries in one frame, one `Hide`, one `ShowNext`) every frame against a replay in doubles (phase, value, the skin's anchored position, the group alpha, the texts and sprite); draws: the skin's background, image and texts at the replay's place while shown, nothing while hidden; the stored nodes' draws unchanged; unit tests (phases, `Trapezoid`, `Approximately`). |
| **M9u7d2** ✅ | **Done 2026-10-17** (MODLOG). Client `--analyze PowerCell,CyclopsDecoyModule`: both shown in turn, done after 5.5137 and 5.5010 s (rule 5.5 s + under a step), 0 warnings, a screenshot. Plan as written: **The client shows the pop-up.** The UI worker runs it each frame; its only input is a test flag, `--analyze <TechType>[,…]`, which calls `OnAnalyze(analysis, verbose: true)` for each tech's `PDAData.analysisTech` entry once the controls are back, as a verbose `KnownTech.Add` would (the game's own sources come with crafting, M9e, and the scanner). | Client `--analyze PowerCell,DoubleTank`: the log names each pop-up's title, text and sprite, shown 0.25 + 5 + 0.25 s, the second right after the first; 0 warnings; a screenshot while shown. |
| **M9u8a** | *Planned and split on 2026-10-18 into M9u8a1–a4 below (research: the decompiled `PDA`, `uGUI_PDA`, `uGUI_CanvasScaler`, `uGUI_Toolbar`, `uGUI_PDATab`, `ArmsController`, `MainCameraControl`, `Sequence`; a throwaway probe of the main scene; `sn-inspect anim scene:main --states`). Found: the PDA model is `PlayerPDA` under the left hand (`…/hand_L/attachL/PlayerPDA`, stored inactive, layer 8; it holds the `PDA` script; its `Mesh` child has 3 MarmosetUBER materials `PDA`, `PDA_border` (WBOIT, premultiplied) and `PDA_screen` (`MARMO_SIMPLE_GLASS`, alpha); its `ScreenAnchor` child sits at (−0.205, 0, 0), turned 180° about y, scale 0.1). `PDA.Open` sets it active and `Deactivated` (the close sequence's end) inactive. The arms' PDA states are on the `upperbody` layer (4): `New State` → `first_check` (`player_pda_start`) → `open` (`player_pda_equip`, 1.667 s at speed 2) → `idle` (`player_pda_idle`) → `close` (`player_pda_away`, 0.833 s at speed 2, exit 0.9) → `New State`; the base layer has a `pda` state from `Walking` and `surface swim` only. With the PDA in use `ArmsController` sets no IK target, so the pose is the animation's alone. The screen canvas (`uGUI_CanvasScaler` mode Inversed, `SetAnchor(screenAnchor)`): each frame its pose is the anchor's world pose in the main camera's space, drawn by the UI camera at the origin; `UpdateFrustum` is skipped, so the canvas keeps its stored size and scale. `MainCameraControl` while the PDA is in use: locked mode (the look-around offset clamped to ±60° and pulled back at 10 × `PDA.deltaTime`; the pitch eased to 0 at 15 × `PDA.deltaTime`; `cameraOffsetTransform` z moved at 0.25 m/s up to `camPDAZOffset` = 0.18 m; no bob). M9d5b's "PDA place = the player's position" is the game's own rule (`main.transform.position`), not a stand-in. `MiscSettings.pdaPause` defaults to false.* Plan as written: **The PDA itself**: the PDA model in the hand, `ArmsController.SetUsingPda` (the arms' animation), `PDA.Open`'s sequence and close distance, the screen canvas on the PDA's anchor, the tab bar (the other tabs drawn but not opened: logged). Replaces M9d5b's stand-ins. | Open and close times equal the values read; the screen canvas's pose relative to the camera logged at rest; the arms' animator reaches the PDA states (names written before the run). |
| **M9u8a1** ✅ | **Done 2026-10-18** (MODLOG; facts in `docs/formats/gameplay.md` § The PDA). 5 unit tests of `sn-sim::pda`, one of the camera's locked mode; real-data `the_pda_in_the_hand` and `EscapePod` against the generic reader; `storagePivot` is the locker's hand-target node. Plan as written: **Data and the PDA's rules** (headless; `sn-assets`, `sn-sim`). `sn-assets`: `PlayerPda` (the `PDA` script's node, its mesh node and materials with their shaders, `screenAnchor`'s node and local placement, `prefabScreen`) from the player's body; `EscapePod.storagePivot`'s node (M9d5b used the hand collider's centre). `sn-sim::pda` (pure): `Sequence` as the game's (`Set`, `Update`: the callback on the update after `t` reaches its end), `PDA.state` (Opened, Closed, Opening, Closing), `Open` (refused while in use, with input ignored, or in a cinematic; the 0.5 s sequence; `targetWasSet`, `activeSqrDistance` = d² + 1), `Close` (0.3 s), `ManagedUpdate` (the PDA button when focused, the distance rule), `Activated` (focused: the screen's content interactable) and `Deactivated` (the model hidden, `isInUse` false), `PDA.time`/`deltaTime` without `pdaPause`. `sn-sim::body`: `MainCameraControl`'s locked mode for the PDA (above) and `using_pda` among the body's parameters. | Unit tests: the sequence's times (Opening for 0.5 s + under a step, Closing 0.3 s + under a step), refusals, the distance close, the camera's locked-mode easing (pitch to 0, offset z to 0.18 m in 0.72 s, back on close); real-data test: `PlayerPDA`'s path, 3 materials, the anchor's local placement, `storagePivot`'s node. |
| **M9u8a2** ✅ | **Done 2026-10-18** (MODLOG; facts in `docs/formats/gameplay.md` § The PDA). `walk --pda`: RUN OK, 12 cycle checks and 10 state checks ok; opening 0.520 s, closing 0.340 s; `idle` 0.980 s and `New State` 0.400 s after; the screen anchor 0.187 m ahead of the eye, 0.9° off its axis; `walk` and `dive` unchanged. Plan as written: **The arms with the PDA** (headless: `sn-inspect walk --pda`): the player's animator with the body's rules and the PDA opened at a set time and closed later, in the pod (walking) and under water (swimming); the PDA model's and the anchor's poses from the animated bones. Expected before the run: on open, `upperbody` `New State` → `first_check` → `open` → `idle` (about 0.1 + 0.83 + 0.1 s after the open); on close `idle` → `close` → `New State` (about 0.38 + 0.1 s after); the base layer `Walking` → `pda` → `Walking` in the pod, `Swim` unchanged under water; layers 1 and 3 stay in their defaults. | Each expected state reached in that order (names and times logged); the PDA model active from the open to the close + 0.3 s + under a step; the screen canvas's pose relative to the eye logged once the arms are in `idle` (expected: in front of the eye and facing it); no NaN; the older `walk`/`dive` checks unchanged. |
| **M9u8a3** ✅ | **Done 2026-10-18** (MODLOG). Client: `Activated` 0.511 s and `Deactivated` 0.300 s + a frame after the open and close; `upperbody` `idle` 0.946 s and `New State` 0.382 s after them (M9u8a2: 0.980 / 0.400 at 50 Hz); walking back from the locker closed it at 2.79 m of `storagePivot` (limit 2.75 m); `--panel` gives M9d5b's item results; death closes it once and it works after the respawn; 0 warnings in 6 runs; screenshots mid-way and open. Plan as written: **The client: the PDA in the hand.** Tab (the game's `Button.PDA`) and the locker open and close through `sn-sim::pda` instead of `inventory.rs`'s own flags; `PlayerPDA` drawn while active (its 3 materials through the object shader), the arms by the animator, the camera's locked mode; `storagePivot` as the locker's target. The M9d5b panel stays as the inventory tab's content until M9u8b, opened when the PDA is `Opened` (logged as a stand-in). Sounds (`uGUI_PDA.soundOpen`/`soundClose`), `UwePostProcessingManager.OpenPDA` and the quick slots: logged as not ported. | Client: open and close logged with the sequence's times and the arms' states, equal to M9u8a2's; the locker opens and closes at the `storagePivot` distance (logged); `--panel` gives the same item results as M9d5b's run; 0 warnings; screenshots open and mid-way. |
| **M9u8a4** | *Split on 2026-10-18 into M9u8a4a–a4b below, after the user saw the PDA's screen see-through outside the pod. (A first note here said the `PDA_screen` glass is transparent in the game; that came from a probe that sampled the texture upside down. Corrected the same day: its `_MainTex` alpha over the screen is 0.832, area-weighted, so the glass is mostly opaque dark blue; the canvas's `PDABackground` is the see-through one, a frame with alpha only near its edges. Our glass looked see-through because the sky, water-surface and fog passes run after the transparent pass and the terrain's blended layers sort after it; fixed in M9u8a4a.)* The canvas is 1195 × 970 at scale 0.000212 (0.253 × 0.205 m). Plan as written: **The screen on the PDA.** The UI worker loads `uGUI_PDAScreen` (`Initialize`: the tabs closed, `SetTabs` with the 6 regular tabs) and each frame places its canvas at the anchor's pose in the camera's space (Inversed); `OnOpenPDA` (the canvas visible, the content's alpha 1, `tabPrev` = Inventory opened), `OnPDAOpened`, `OnClosePDA`, `OnPDAClosed`; `uGUI_Toolbar.Initialize` (icons made by code: `uGUI_ItemIcon` with `backgroundSpriteNormal`, the tab sprites from the `Tabs` atlas, x = ½·bg.x + i·(bg.x + step), the bar's width n·bg.x + (n − 1)·step), `Select` (`backgroundSpriteSelected`, the selection marker), notification counts 0 (hidden); a click on a tab icon opens that tab (`OpenTab`: the old tab's `Close`, the new one's `Open`; tabs other than Inventory show their stored content and are logged as not ported). Headless check `ui --pda`. | `ui --pda`: the 6 toolbar icons' rects equal the rule, the selected one has the selected sprite, the marker at its place, N/N; open/close/tab-switch alphas against a replay; every older `ui` check unchanged. Client: the screen drawn on the PDA (its canvas pose logged against M9u8a2's), a tab click logged; 0 warnings; screenshots. |
| **M9u8a4a** ✅ | **Done 2026-10-18** (MODLOG; facts in `docs/formats/ui.md` § The PDA's screen on the PDA, `materials.md` § The view model's glass). Client: the canvas's pose at rest (0.0217, −0.0012, 0.1874) m, 0.89° off the axis, equal to M9u8a2's anchor; `OnOpenPDA` shows canvas, content, toolbar and the inventory tab (1 of 8), `OnClosePDA` hides the content, `OnPDAClosed` the canvas; the PDA's blended materials write depth (the view model), so the sky, water and terrain no longer paint over its glass; the toolbar's gamepad labels hidden (`UpdateCycleBtnText`); 0 warnings; screenshots in the pod, under water and at the surface. Draw counts of the screen not logged (its hidden groups stay in the list at alpha 0). Plan as written: **The screen canvas on the PDA** (client; `sn_ui::pda`, `sn-assets`). The UI worker reads `uGUI_PDAScreen` from the main scene's `PDA.prefabScreen` and grafts it into the UI scene as `uGUI_PDAScreen(Clone)` (`PDA.ui`'s `Instantiate`); its canvas laid out at its stored size and scale (`UpdateFrustum` skipped). `uGUI_PDA`'s states (pure, `sn_ui::pda`): `Awake` (content visible, canvas invisible), `Initialize` (every tab's `Close`: the regular tabs' `content` CanvasGroup hidden, the intro and time capsule tabs' GameObjects inactive; the back button inactive), `OnOpenPDA` (canvas visible, content alpha 1, `tabPrev` opened: its content shown, the toolbar group visible), `OnClosePDA` (the open tab closed, content hidden), `OnPDAClosed` (canvas invisible); driven by the PDA's flags each frame (`using_pda` up: `OnOpenPDA`; focused: `OnPDAOpened`; `using_pda` down: `OnClosePDA`; the model off: `OnPDAClosed`). The canvas's pose: each frame after the transforms propagate, `ScreenAnchor`'s world pose in the main camera's space (Inversed), given to the render pass directly (no worker latency). The toolbar's icons are not made yet (M9u8a4b). | Client: the canvas's pose relative to the camera logged at rest and equal to M9u8a2's anchor (within 1 mm, 0.5°); the root group alpha 0 → 1 at the open and back to 0 at `Deactivated`, the inventory tab's content shown, the other tabs' hidden (counts logged); draws of the PDA screen counted; 0 warnings; screenshots in the pod, under water and at the surface with the PDA open. |
| **M9u8a4b** ✅ | **Done 2026-10-20** (MODLOG; facts in `docs/formats/ui.md` § The PDA's toolbar). `ui --pda`: 173 of 173 checks ok (bar 778 wide, 6 icons at the rule, sizes, sprites, the marker at icon + offset, 7 clicks from screen pixels through the ray back to their icons, the tabs' alphas against the expected table, close and reopen on `tabPrev`; 12 icon draws, nothing drawn over them); client: `--panel …,tabN` clicks logged with pixel, canvas and bar points, each opening its tab; 0 warnings; screenshots. Older `ui --icons`, `--icon-mesh`, `--draw`, `--popup` unchanged. Plan as written: **The tab bar and tab clicks.** `uGUI_Toolbar.Initialize` (icons made by code: `uGUI_ItemIcon` with `backgroundSpriteNormal`, the tab sprites from the `Tabs` atlas, x = ½·bg.x + i·(bg.x + step), the bar's width n·bg.x + (n − 1)·step), `Select` (`backgroundSpriteSelected`, the selection marker), notification counts 0 (hidden); a click on a tab icon opens that tab (`OpenTab`); tabs other than Inventory show their stored content and are logged as not ported. Headless check `ui --pda`. | `ui --pda`: the 6 toolbar icons' rects equal the rule, the selected one has the selected sprite, the marker at its place, N/N; open/close/tab-switch alphas against a replay; every older `ui` check unchanged. Client: a tab click logged; 0 warnings; screenshots. |
| **M9u8b** | *Planned and split on 2026-10-20 into M9u8b1–b5 below (research: the decompiled `uGUI_InventoryTab`, `uGUI_ItemsContainer`, `uGUI_Equipment`, `uGUI_EquipmentSlot`, `uGUI_ItemIcon`; `NotificationManager`, `uGUI_Tooltip`, `ItemDragManager` read for b3–b5). Found: `uGUI_ItemsContainer.Awake` and `uGUI_Equipment.Awake` make their objects inactive, and the latter every one of its 38 `uGUI_EquipmentSlot`s (the player's, the vehicles'); M9u8a4a does not run them, so the doll and every slot visual show on every tab (the round gauges the user saw). `OnOpenPDA` shows `storage` with one used storage (the locker), else `equipment` (`Init`: only the slots the player's `Equipment` has, each item's icon); `inventory.Init(container)` at `Start`. Cells are 71 units, icons `66·w + 5·(w − 1)` at `(71·x + 1, −(71·y + 1))` from the top-left anchor; an item icon's background is a Resources sprite (`Sprites/Backgrounds/<type>`, so `SLICE_9_GRID` on, radius 33), its foreground the item sprite, a `UI/IconBar` bar when `TooltipFactory.GetBarValue` ≥ 0, `GRAYSCALE` when disabled. A slot's background `Image` is tinted by its state (default white at alpha 127, highlighted white, allowed green, denied red, selected cyan).* Plan as written: **The inventory tab**: `NotificationManager` (moved from M9u7d on 2026-10-17: `Add(Inventory, id, 4)` on a pick-up, `Remove` on a drop, `LateUpdate`'s countdown while a target is visible, `GetCount` for the tab's label; the item icons' `Progress`); `UI/Circle` in WGSL (the equipment slots and the PDA's intro circles; `docs/formats/ui.md`); `uGUI_ItemsContainer` (cells, icons by size, the locker's container beside), `uGUI_Equipment` (slot backgrounds and icons), the tooltip (name and description), hover, click by M9d5b's `item_action`, drag and drop (`uGUI_ItemsContainer` / `uGUI_Equipment` rules), the cursor. `inventory.rs`'s panel removed; `--panel` clicks at icon centres from the layout. | `--panel` gives the same item results as M9d5b's run; every drawn cell's rect equals the container's rule; a drag between grid, slots and locker sends the same `Move` as a click. |
| **M9u8b1** ✅ | **Done 2026-10-20** (MODLOG; facts in `docs/formats/ui.md` § The inventory tab). `ui --inventory`: 22 of 22 (38 named slots, all in `slotMapping`; the player's seven as written; the inventory 426 × 568; the doll with exactly those seven, 0 draws of the 31 others; the locker 4 × 8 → 284 × 568; labels INVENTORY / EQUIPPED / STORAGE; close hides storage and doll); client: logs and screenshots with and without the locker, 0 warnings; older `ui --pda`, `--draw`, `--icons` unchanged. Plan as written: **The tab's containers, no icons yet** (headless first, then the client). `uGUI_ItemsContainer.Awake`, `uGUI_Equipment.Awake` (all slots inactive), `uGUI_InventoryTab`'s `Start` (`inventory.Init`: `OnResize` = rect 71·w × 71·h, `grid.uvRect` (0, 0, w, h)), `OnOpenPDA` (the locker as `storage` when open, else `equipment.Init`: the player's slots active, the others not; the labels from the language file: `InventoryLabel`, the storage's label or `StorageLabel`), `OnClosePDA` (`Uninit`s). Pure in `sn_ui::inventory_tab`, data in `sn-assets`. Headless `ui --inventory`. | `ui --inventory`: each container's rect equals 71 × its size, N/N; active slots = our `Equipment`'s slot names (names written before the run: Head, Body, Gloves, Foots, Chip1, Chip2, Tank), every other slot inactive; with and without the locker; the labels' texts; older `ui` checks unchanged. Client: the doll shows only the player's slots; screenshots with and without the locker. |
| **M9u8b2** | *Split on 2026-10-20 into M9u8b2a–b2b (one concern each: the icons' rules, then the shaders). Cut from b2, recorded here: `UI/IconBar` and `UI/Icon`'s `GRAYSCALE`. A bar shows only for an item with an `EnergyMixin` or `IBattery` (charge), or food that `decomposes` (`TooltipFactory.GetBarValue`), and `GRAYSCALE` only for a disabled `InventoryItem`. Our sim keeps a battery's charge (`Item::charge`) but not a tool's `EnergyMixin`, food decay or disabled items, so most bars and the grey cannot show a true value; both wait for those rules (logged, not drawn).* Plan as written: **Item and slot icons.** `OnAddItem` / `OnRemoveItem` / `OnChangeItemPosition` icons made by code from our container's layout (background `SpriteManager.GetBackground(techType)`, radius 33, foreground, size and position by the rule, chroma 0 when disabled), `uGUI_EquipmentSlot.SetItem` / `ClearIcon` (the hint). WGSL: `UI/Icon`'s `SLICE_9_GRID` and `GRAYSCALE` variants, `UI/IconBar`, `UI/Circle`. | `ui --inventory`: every icon's rect equals the rule for its item's size and place, N/N; one draw per icon graphic; the three shaders' variants drawn (none as `UI/Default`); client screenshots with items in the grid, the slots and the locker. |
| **M9u8b2a** ✅ | **Done 2026-10-20** (MODLOG; facts in `docs/formats/ui.md` § The inventory tab's icons). `ui --inventory`: 42 of 42 (the 7 Resources backgrounds found, 2-pixel strips; 4 grid items 1 × 1 and 2 × 2, 3 equipped (2 × 2, 2 × 3), 3 locker items, each icon at the rule; slot icons 124 × 124 on their `iconRect`, hints hidden; 14 draws for 7 icons, every background `SLICE_9_GRID`; a move, a removal and an unequip; the close clears the locker's and the slots' icons); client: the locker's 6 starting items and, after take-all, the same 6 in the grid; 0 warnings; screenshots (backgrounds drawn as `UI/Default` until b2b). Plan as written: **The icons by the game's rules** (headless, then the client). `uGUI_ItemsContainer.OnAddItem` / `OnRemoveItem` / `OnChangeItemPosition` from our containers (the inventory always, the locker while it is the used storage): `Item Icon` made by code under the container (anchor and pivot (0, 1)), background `Resources.Load("Sprites/Backgrounds/<type>")` by `TechData`'s background type (`SLICE_9_GRID` on: a `GetResource` sprite), radius 33, the item's sprite as foreground, `SetSize(66·w + 5·(w − 1), …)`, `SetPosition(71·x + 1, −(71·y + 1))`. `uGUI_EquipmentSlot.SetItem` / `ClearIcon` for the player's equipped items (`Equipment Slot Icon` under `iconRect`, sized to it, radius min(size) / 2, the hint hidden). `Uninit` destroys (ours: hides) the storage's icons and clears the slots. The backgrounds draw as `UI/Default` until b2b (logged). | `ui --inventory`: a scripted set of items (sizes 1 × 1 and 2 × 2, grid, locker, slots) → every icon's rect equals the rule, N/N; sprites and material values as the rule says; one draw per icon graphic; remove and move update them. Client: icons logged and shown for the starting locker items; screenshots. |
| **M9u8b2b** ✅ | **Done 2026-10-20** (MODLOG; facts in `docs/formats/ui.md` § The inventory tab's icons). Client census once the PDA is open: `UI/Icon SLICE_9_GRID` 6 and `UI/Circle` 7 drawn by their own programs, none as `UI/Default` (the loading bar's `UI/Bar`, out of scope, stays); 2 unit tests of the Rust references `slice9_uv` and `circle_uv` (values worked by hand), which the WGSL mirrors (the WGSL itself is not run by a test); 0 warnings or GPU validation errors; screenshots. Plan as written: **The shaders:** `UI/Icon`'s `SLICE_9_GRID` variant (from its disassembly, M9u7c's dump: the border mapped by `_Radius` and `_Size`, the strip's v rescaled by the texture's height) and `UI/Circle` (M9u5's dump) in WGSL. | Census: no `UI/Icon` `SLICE_9_GRID` or `UI/Circle` draw falls back to `UI/Default`; unit tests of the slice-9 UV mapping against values worked by hand from the disassembly; 0 GPU validation errors; screenshots. |
| **M9u8b3** | *Split on 2026-10-20 into M9u8b3a–b3b: the tooltip (`uGUI_Tooltip`, its prefab and layout, `TooltipFactory.InventoryItem`'s texts and button hints) is its own system.* Plan as written: **Pointer: hover, click and the tooltip; the panel removed.** Hit tests over the tab's icons and slots (generalised from M9u8a4b); `OnPointerEnter` (`hoveredItem`, `HighlightSlots`, slot states and tints), `OnPointerClick` by `sn_sim::inventory::item_action`; `uGUI_Tooltip` with `TooltipFactory.InventoryItem` (name, description, the action hints). `inventory.rs`'s Bevy panel removed; `--panel` clicks at icon centres from the layout. | `--panel` gives the same item results as M9d5b's run; hover tints against the rule; the tooltip's texts for a set of items logged; 0 warnings; screenshots. |
| **M9u8b3a** ✅ | **Done 2026-10-20** (MODLOG; facts in `docs/formats/ui.md` § The inventory tab's icons). `ui --inventory`: 53 of 53 (hits at each icon's and slot's centre find their item, an empty cell nothing; the Tank lights only the Tank slot, Titanium none, as written; nothing drawn over the icons); client: `--use-item 3 --panel 6=take-all,7=drop,9.5=close` through the tab's hit tests gives M9d5b's results (6 left clicks "switch", the right click "no drop in the lifepod"); the Bevy panel is gone; 0 warnings; screenshot. Hover in the client is not tested with a real cursor (headless only). Plan as written: **Hover and click on the tab; our panel removed.** The UI worker hit-tests the cursor (each frame while the PDA is focused) and each click on the canvas at the frame's pose (M9u8a4b's ray): the toolbar, the item icons (their `uGUI_ItemIcon` rects), the slots (`uGUI_EquipmentSlot` rects). `uGUI_InventoryTab.OnPointerEnter` (`hoveredItem`, `HighlightSlots(equipmentType)`: the compatible slots tinted white) / `OnPointerExit` (`ExtinguishSlots`: default tint); `OnPointerClick` → the item's action by `sn_sim::inventory::item_action` (the main thread's `click`), then `ExtinguishSlots`. `inventory.rs`'s Bevy panel removed; `--panel`'s steps click at the items' icon centres through the same path. | `ui --inventory`: a hit at each icon's and slot's centre finds its item, an empty cell nothing, N/N; hovering an item tints exactly its compatible slots (the rule written before the run: a `Tank` lights the Tank slot only); nothing drawn over the icons. Client: `--panel` gives M9d5b's item results (6 left clicks "switch", the right click "no drop in the lifepod"); 0 warnings; screenshots. |
| **M9u8b3b** | *Split on 2026-10-27 into M9u8b3b1–b3b3 (research: the decompiled `uGUI_Tooltip`, `TooltipFactory`, `FPSInputModule`, `CursorManager.GetPointerInfo`, `FlexibleGridLayout`, TMP's `GetPreferredValues`; facts in `docs/formats/ui.md` § The tooltip). Found: `uGUI/Tooltip` is itself the canvas, the script's node and the rect `OnLayout` sizes (pivot top left); `UpdatePosition` puts it in the plane of the hovered graphic's canvas (the PDA screen's root canvas), scaled to keep its size on screen (distance × `scaleFactor` 0.00116 × `uiScale`), beside the cursor by `margin` (10, 20), flipped to the cursor's other side at the canvas's edge. With no ingredient icons (an inventory item has none) the icon row is 0 × 0. The button texts are `FormatButton(UISubmit / UICancel)`: the mouse's left and right button sprites.* Plan as written: **The tooltip.** `uGUI_Tooltip` (its prefab in `uGUI`, its layout and placement by the cursor), `TooltipFactory.InventoryItem` (the item's name and description from the language file, the action hints with their buttons), shown for `hoveredItem`. | The tooltip's texts for a set of items logged against the language file; its rect beside the cursor by the rule; screenshots. |
| **M9u8b3b1** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § The tooltip). `ui --tooltip`: 23 of 23 (every prefix and postfix line of the seven items equal to one built from the language file; the actions as written before the run; the buttons the mouse's sprites); the Tank's `Oxygen` line is left out and logged; a new battery's charge is its capacity (`_charge` is not stored). Plan as written: **The texts** (headless; `sn_ui::tooltip`, `sn-assets`). `TooltipFactory.InventoryItem`: `ItemCommons` (the title `Language.Get(techType)`; an `Eatable`'s food and water lines in survival (`FoodFormat`, `WaterFormat` with C#'s `+#;-#;+0`), its `DecomposingFormat` prefix; `FirstAidKit`'s `HealthFormat`; an `IBattery`'s `BatteryCharge` instead of the description; the description `Tooltip_<TechType>`), `ItemActions` for the keyboard and mouse (the quick-slot hint `KeyRange15` – `BindQuickSlot` for a hovered `Hand` item in the player's grid; then button 0's action, button 1's (drop); `FormatButton(UISubmit)`, `(UICancel)`), `WriteTitle`, `WriteDescription`, `WriteAction`'s tags. The item's values from its prefab (the `Pickupable`'s node: `Eatable`'s `foodValue`, `waterValue`, `decomposes`; `Battery`'s `_capacity`, a new battery's charge: `OnAfterDeserialize` sets `_charge` to it); food decay (`GetFoodValue` over time), a tool's `EnergyMixin`, `IOxygenSource`, `Signal`, `FireExtinguisher` and a dead `Creature` are not ported (logged when met). Headless `ui --tooltip`. | `ui --tooltip`: the texts of a set of items written before the run (the lifepod's Flare, FilteredWater, NutrientBlock; FirstAidKit, Titanium, Battery, a Tank in a slot) logged, each title, description and action word equal to its language entry, N/N. The rule written before the run (the items in the player's grid, no storage open, outside the lifepod, survival, each one hovered): Flare: the bind hint and drop; FilteredWater: its water line, eat, drop; NutrientBlock: its non-zero food and water lines, eat, drop; FirstAidKit: health +50, use, drop; Titanium: drop; Battery: its charge line and no description, drop; the Tank in its slot: unequip only (its oxygen line not ported, logged); the button texts the mouse sprites; unit tests of the writers and the number format. |
| **M9u8b3b2** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § The tooltip). `ui --tooltip`: 65 of 65 (the seven items' `OnLayout` sizes by the rule from TMP's preferred sizes, four of them held at 429; their texts laid out at their places inside the root; on the PDA screen's canvas at the client's 0.1874 m the pointer at the centre, near the right edge, near the bottom and in the bottom right corner: the rect on the side the rule says, inside the canvas, clear of the pointer); 2 unit tests (`OnLayout`; `UpdatePosition`'s cases, both sides overflowing included). The visibility (`Set` / `Clear`, alpha) goes with the client (b3b3). Plan as written: **Layout and placement** (headless; `sn_ui::tooltip`). `OnLayout` with no icons (the widths: the texts' unbounded preferred widths, at most 429; the heights at that width (TMP `GetPreferredValues(text, a, 0)`); the texts at (20, −20) and below, 10 between when the postfix is not empty; the rect (a + 40) × the sum + 20); `UpdatePosition` (scale = the cursor's depth × `scaleFactor` × `uiScale`, the rect at the cursor + (10, −20) in the canvas's space, flipped left or up when it would leave the canvas's rect, pinned to an edge when both sides overflow), `canvasGroup.alpha` = the hovered graphic's nested alpha; visible only while an item is under the pointer (`FPSInputModule`: `Set` / `Clear`). | `ui --tooltip`: each item's rect against the rule worked here, its texts laid out inside it; the cursor at the canvas's centre, near its right edge and near its bottom: the rect beside the cursor on the side the rule says, N/N; unit tests of the placement's four cases. |
| **M9u8b3b3** ✅ | **Done 2026-10-27**: client hover/clear runs, 0 warnings, two screenshots; headless tooltip 65/65; workspace 727 tests and clippy pass. **The client.** Implementation: preserve the unfinished scripted-pointer and placement edits; send current actions/class/charge from the sim, cache prefab facts per class, use the tested text/layout/placement rules, inherit nested alpha and canvas sorting +2, clear off icons/unfocused/other tabs. Verify the hover and clear separately with screenshots and logs. The worker builds the tooltip for the item under the pointer (each entry's actions and class from the main thread; the item's prefab values read once per class), lays it out and draws `uGUI/Tooltip` after the PDA's screen; its pose each frame from the PDA's pose (`UpdatePosition`). `--panel T=hoverN`: the pointer rests on item N's icon centre (scripted; the agent cannot move a mouse). | Client: a scripted hover logs the tooltip's texts and rect, beside the cursor by the rule; nothing shown off an icon; 0 warnings; screenshots. |
| **M9u8b4** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § Drag and drop). `ui --inventory` 74 of 74 (new: the Tank's and the Fins' slot tints over each of the seven slots, the empty slots' snaps on their icons with radius 62, a locked icon drawn at 0.3, `ItemActions/Swap` 64 × 64); client runs: every pair below sent the click's `Move` and the server applied it, the dragged icon laid out on its rule's place (0.000 off over ~180 frames a drag); 0 warnings; three screenshots. Plan as written (2026-10-27): (1) pure pointer lifecycle, slot feedback and snap interpolation in sn-ui with synthetic tests; (2) app worker routes press/motion/release and scripted drag through the same hit tests, hides the source foreground, draws the dragged icon and swap hint, suppresses tooltip/click while dragging; (3) evaluate existing AddOrSwap against cloned local stores for feedback and target selection, send the existing Move request (Store / Item / Slot), cancel on close or disappearance; (4) headless and real-data replay plus scripted client moves grid↔locker and grid↔slots, invalid slot/red, compatible slot/green, cleanup. No protocol or server change. Research changed (2): the left button's click now comes on the release (`FPSInputModule`), a release on the dragged item itself does nothing, and the source is locked at 0.3 (not hidden). `ItemDragManager` (the dragged icon, snap, action hint), `uGUI_ItemsContainer.OnDrop` / `uGUI_Equipment.OnDrop` / `CanSwitchOrSwap`, `OnDragHoverStay` tints. `--panel` steps `dragN`, `drag-lockerN`, `drag-slotS`, `over-grid`, `over-locker`, `over-slotS`, `press`, `release`. | A scripted drag between grid, slots and locker sends the same `Move` as a click (each pair); the slot tints while dragging against the rule. |
| **M9u8b5** ✅ | **Done 2026-10-27** as M9u8b5a and M9u8b5b (below). **Notifications.** `NotificationManager` (`Add(Inventory, id, 4)` on a pick-up, `Remove` on a drop, `LateUpdate`'s countdown while a target is visible, `GetCount`), the toolbar's count label (`uGUI_NotificationLabel`), the icons' `Progress` (`UI/Icon` `NOTIFICATION` variant). Split (plan 2026-10-27, after reading `NotificationManager`, `Inventory`, `uGUI_ItemIcon`, `uGUI_Toolbar`, `uGUI_PDA`, `uGUI_ItemsContainer`, `uGUI_Equipment`, `uGUI_EquipmentSlot` and `UI/Icon`'s blobs 144 and 176) into b5a and b5b. | A pick-up's count on the inventory tab's icon and its icon's progress against a replay; the count clears after the target is visible for its time; screenshots. |
| **M9u8b5a** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § Notifications). `ui --notifications` 14 of 14 (labels at their anchors + offsets to 1e-3, alpha 180 = round(255 · sin(π/4)), the replay cleared after 241 frames = 4.017 s visible); 5 unit tests; client: 3 pick-ups → count 3, '+' at 1 until the PDA opened, 0.75 / 0.5 / 0.25 a second apart, count 0 after 3.995 s visible; a drop removed one (3 → 2); 0 warnings; screenshot. Plan as written: **The manager, the counts and the labels.** `sn_ui::notifications`: `NotificationManager` (keys (group, item id); `Add` with `max(0.1, d)`; `Remove`; `GetCount`; `RegisterTarget` (an old visible target's `Progress(0)`, the new one's `timeLeft/duration` if visible); `UnregisterTarget` (removed if more than 1 s of its duration has passed, else reset; `Progress(0)`); `LateUpdate` (visible: count down by the PDA's delta, `Progress(timeLeft/duration)`, removed at ≤ 0; hidden: removed after > 1 s seen, else `Progress(1)` and reset)). Client: `Add(Inventory, id, 4)` for each item our pick-ups put in the grid or the slots (`Inventory.Pickup`), `Remove` when an item leaves the grid (`OnRemoveItem`) or its slot (`OnUnequip`); the icons are targets (`uGUI_ItemsContainer.OnAddItem` / `OnRemoveItem` / `Uninit`, `uGUI_Equipment.OnEquip` / `OnUnequip` / `Uninit`), visible when active with a foreground whose inherited alpha is above 0. `Progress(p)` on an icon (`SetNotificationProgress`): the `NotificationLabel` prefab (`uGUI.prefabNotificationLabel`) under the icon at alpha sin(p·π/2) with "+", anchor upper right, offset (−10, −10) (a slot's: `iconRect`'s size × −0.1464466), white; gone at 0. `uGUI_PDA.OnLateUpdate`: the toolbar's inventory icon's label with `GetCount(Inventory)` at `notificationAnchor` and `notificationOffset` while above 0 (the other tabs' counts are not ported: 0). `sn-inspect ui --notifications`. | `ui --notifications`: the label prefab's nodes; grafted under a toolbar icon and an item icon, laid out at the anchor and offset written before the run, its texts "1" and "+", drawn, alpha sin(p·π/2) at p = 0.5; a replay (pick-up, the PDA closed 3 s, opened) gives count 1 until 4 s of visible time, then 0. Unit tests of the manager's cases. Client: `--pick-up` then `--panel open`: the count and the "+" logged against the replay's times, cleared after 4 s; 0 warnings; screenshot. |
| **M9u8b5b** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § The glow). Client census while notified: `UI/Icon NOTIFICATION` 3 and `UI/Icon NOTIFICATION SLICE_9_GRID` 3, none drawn as `UI/Default`; 0 warnings or GPU validation errors; 1 unit test of the factor and u by hand (the WGSL mirrors it; not run by a test); `ui --notifications` 15 of 15 (the overlay texture found); screenshots. Plan as written: **The glow.** `UI/Icon`'s `NOTIFICATION` variant (blob 144; with `SLICE_9_GRID` blob 176) in WGSL: rgb × (1 + 0.75 · overlay · `_NotificationStrength`), overlay = `_NotificationOverlayTex` (`Resources` `Sprites/NotificationOverlay`) at u = \|UV − ½\| · `_Size` ÷ 66 − 3.18 · `_PDATime`, v = ½; `_PDATime` the PDA's time (`Shader.SetGlobalFloat`). `SetGodRaysIntensity(sin(p·π/2))` on the icon's two materials (`NOTIFICATION` on while above 0). | Census: no `NOTIFICATION` draw falls back to `UI/Default`; a unit test of the factor against values worked by hand from the disassembly; 0 GPU validation errors; screenshot of a glowing icon. |
| **M9u9** ✅ | **Done 2026-10-27** (MODLOG; facts in `docs/formats/ui.md` § Two players). Three two-player runs (host + `--join` on 127.0.0.1, release, logs `out/m9u9/`, 0 WARN/ERROR in all six logs): (A) locker: the host takes the 8 supplies out, puts the Tank back by drag and the Fins by click, hovers tooltips; the joiner drags the Tank into its grid: both log the same locker after world events 11, 13, 14, 15, and the host's inventory is unchanged by the joiner's move; (B) the host picks up 3 mushrooms (icons, `Add` × 3, count 3 only on the host) and drops one; the joiner sees it drawn at event 8, picks it up with 2 more (icons and count 3 only on the joiner), both remove it at event 11; (C) the host's death: "You died and lost some belongings." only on the host (6.006 s), both draw the 2 drops. Merge smoke check the same day: tooltip, drag/swap and notification runs as in MODLOG M9u8b3b3/b4/b5a; P1 swim max 23.1 / 26.8 ms. `cargo test --workspace` 743 passed without the game. Plan as written: **Close-out**: two clients (host and join) each with the UI; `README.md`, `docs/formats/ui.md` and the "Not 1:1 after M9u" list below updated. | Two-player run: each client's messages and inventory views match its own state; `cargo test --workspace` passes without the game; `git status --short` shows only our code and docs. |

**Not 1:1 after M9u** (collected 2026-10-27 from the M9u8b3–b5 entries in
MODLOG and the rows above; each goes where its subsystem is built; the
out-of-scope list below still holds):
- **The tooltip** (M9u8b3b): food decay, a tank's oxygen, a tool's
  `EnergyMixin` charge and the other `TooltipFactory` components are not
  ported (survival, "After Phase E" item 1); `{0:P0}`'s spacing is our
  guess for Mono's en-US (*hypothesis*); the tooltip canvas is
  deactivated on `Clear` instead of only set to alpha 0 (same picture).
- **Drag and drop** (M9u8b4): right and middle clicks act on the press
  (the game's on the release; the game also drags with them); the slot
  tints are worked out each frame (the game only on events; same result
  in the cases met); a swap target `CanSwap` allowed but the move refused
  goes back to alpha 1 at the drop (the game leaves it dimmed); `CanSwap`
  is approximated by `AddOrSwap` on a copy; the drag threshold (10,
  Unity's default) is a *hypothesis*; a real mouse drag is not tested.
- **Bars and grey on icons** (cut from M9u8b2): `UI/IconBar` and
  `UI/Icon`'s `GRAYSCALE` are not ported, so no battery/oxygen/decay bar
  shows on any icon, the dragged one included (item 1).
- **Notifications** (M9u8b5): only the `Inventory` group (the tab's
  count, the items' "+"); `Blueprints`, Log and the other tabs' counts
  are 0 (M9e for blueprints, item 2 for the rest); a pick-up is
  recognised by a new id coming into the grid or a slot from outside our
  stores, not by `Inventory.Pickup` (so a `noMessage` pick-up would
  count; none happens yet); saved notifications wait for M9f;
  `_PDATime`'s phase is our own sum, not the PDA's time since the start.
- **Toolbar tooltips** (`TooltipFactory.Label("Tab<name>")`), hover
  sounds and the gamepad's tab cycling (M9u8a4b): not ported.
- **Two players** (M9u9): the original game has no multiplayer, so who
  sees what (shared world, each player's own UI) is our rule, not the
  game's.
- **Everywhere:** nothing above is compared against the original game
  on screen; Linux/macOS not tested; only English tested.

**Out of scope (each kept in its own milestone):** the fabricator menu
(M9e, built on M9u4–M9u5); the main menu and in-game menu with options
(`xmenu`, `essentials`; a later step of its own); the other PDA tabs'
content (Log, Encyclopedia, Pings, Gallery, Time Capsule: PDA and goals,
"After Phase E" item 2); quick slots and held tools (item 1; the
bar is drawn as the game draws it with nothing bound, *hypothesis* to
check in M9u6); vehicle and Cyclops HUDs, the builder menu (items 5 and
6); pings and signals; subtitles and voice (audio, item 8); gamepad
navigation; the food and water drain (the bars show the starting values
until eating lands with item 1, because a drain without food would starve the
player); languages other than English (the code path is the same; not
tested).

**M9u as built:**
- **M9u0.** Built as planned, with `sn-dotnet` field types
  (`SigType`, `Assembly::field_type`, `base_type`) and `sn-inspect code
  --fields`, the `Canvas` and `uGUI_CanvasScaler` readers and
  `SpriteAtlasHead` (`sn-unity::ui`), and `Assets::ui_tree` /
  `ugui_root` / `pda_screen_refs` (`sn-assets::ui`). Corrections to the
  numbers above, which were bundle totals: the bundle is `uGUI` (361
  nodes, 12 canvases, 143 `Image`, 101 text) plus 10 small prefabs its
  scripts instantiate (26 nodes); together they give the 387 / 385 / 13
  / 148 / 110 above. The 504 behaviours include assets that are not
  components (`FMODAsset`), so that count was not checked. Spawn path
  confirmed: `uGUI.InitializeAsync` loads the key `uGUI.prefab`;
  `PDA.prefabScreen` is `uGUI_PDAScreen` (402 nodes). **New risk for
  M9u3:** every `TMP_FontAsset` is 784–1,024 bytes, so its glyph tables
  are probably empty (*hypothesis*, settled in M9u1). If so, the stored
  `Aller_Rg` atlas has no glyph rectangles, and M9u3's check against it
  must first find each glyph in the atlas or use another oracle. M9u3
  stops and asks if neither works.
- **M9u1.** Built as planned. `sn-dotnet::meta` adds type flags,
  `extends` (with generic bases), nested types, TypeRef scopes,
  assembly names and refs, and custom attributes. The test encoder
  `sn-dotnet::encode` is opt-in through the `encode` feature, for other
  crates' tests (only `sn-assets`'s dev-dependency turns it on).
  `sn-unity::script` adds `Kind` / `StructLayout`, `read_script` (a
  value tree; the data must end at the last field) and `trace_script`.
  `sn-assets::script_layout` adds `ScriptAssemblies::load` (the four
  roots and what they reference: 51 assemblies, the .NET libraries
  left out) and `Layouts` (Unity's rules, built-in structs, depth limit
  7, a cache by type and depth). Tools: `sn-inspect ui --scripts
  [--all]`, `ui --layout <CLASS> [<NAME>]`. Found on data, in three
  runs: (1) `GUIStyle` is stored natively; it was added to the
  built-in table with its 2019.4 layout (states without scaled
  backgrounds); `UWE.LargeWorldStreamerGUI` and `Player` hold one and
  end exactly after it. (2) A string is padded to 4 bytes also as an
  array's element (`Story.CompoundGoal.preconditions`); the padding is
  to the object's absolute 4-byte boundary. (3) Nothing else: the UI
  and startup files 2,197 of 2,197, the whole game 45,137 of 45,137.
  The depth limit 7 is not tested by any object: no game object has a
  chain that deep. **M9u3 settled:** every font asset is in dynamic
  mode (`m_AtlasPopulationMode` 1, render mode 4165 = SDFAA, padding
  9). Only `Aller_Rg SDF` stores glyphs, and just 2 (one of them
  U+2026). So the stored atlas is no oracle for most glyphs, and M9u3's
  check is revised (its row): face metrics against every asset's
  stored `m_FaceInfo`, and the 2 stored glyphs.
- **M9u2.** Built as planned. `sn-unity::ui`: `RectTransform`,
  `CanvasGroup`, `CanvasRenderer`, `Sprite` (with its render data and
  mesh) and the full `SpriteAtlas` (replacing M9u0's head). The field
  order comes from UnityPy's type-tree database for 2019.4.36f1 (a
  dev-time oracle in `out/`, not a dependency): 2019.4's
  `SpriteAtlasData` has no secondary textures. `sn-assets::sprites`:
  `Sprites` (`get`, `get_no_default`, `default_sprite`, `item_icon`) by
  `SpriteManager`'s rule (read from its IL), and `sprite_pixels` cut from
  the atlas page. `sn-inspect ui --sprites [--icons]`, a small PNG writer
  of our own (stored deflate, no dependency), `code --fields` also lists
  methods. Found: tight-packed atlases trim each sprite's transparent
  margin (for M9u4's Image mesh); `ItemRules.tech_class` holds 515 tech
  types, not only inventory items (103 fall back to `Unknown`, as in
  the game). The "20 sprites against UnityPy" check is done; the icons
  are not compared with the game on screen.

- **M9u3.** Built as planned. New crate `sn-ui` (layer 2, pure; its one
  dependency `ttf-parser`, as planned) with `font`: `FontFile`
  (`face_info`, `glyph`, `render`), FreeType's fixed-point scaling
  (`scale`), our own coverage rasterizer (`raster`) and distance field
  (`sdf`), composite glyph placement (`composite`), and `DynamicAtlas` /
  `Packer` (MaxRects, best short side fit). `sn-unity::Font` (class 128);
  `sn-assets::fonts` (`font_assets`: font assets typed with `stored()`,
  sprite assets, `TMP Settings`); `sn-inspect ui --fonts [--atlas]`;
  `code --il` reads other assemblies as `ASSEMBLY:TYPE`. Found on data:
  (1) the cap and mean lines are hinted (TMP uses the stored values, so
  do we); (2) composite glyphs need FreeType's offset rule (scale, then
  round to the pixel), not `ttf-parser`'s placement; (3) the CJK asset
  differs: point size 45, padding 5, a 2048 atlas and multi-atlas on
  (the plan's "padding 9" is true for the other 7); (4) 6 stored texts
  hold private-use characters no font has, all replaced by the game
  before shown (IL), so the check lists them apart. Changes to the
  design: every glyph is made by us, the 2 stored ones too (they equal
  the stored ones), instead of using the stored atlas as it is; the
  distance is exact to the outline, not the game's anti-aliased-bitmap
  method (mean error 1.04/255). **For M9u5:** a glyph costs 2.2 ms
  (brute force over every segment); a screen showing ~100 new glyphs at
  once would stall about 0.2 s. Make the needed glyphs at load time, or
  speed up the field, before the client draws text. Multi-atlas (CJK)
  is not ported; a full atlas is logged.
- **M9u5a.** Built as planned. `sn-ui::material` (`UiMaterial`, TMP's
  `UpdateShaderRatios`, `GetPadding`, `GetFallbackMaterial`, `PackUV`),
  `sn-ui::text::mesh` (`text_meshes`: UVs, `uv2`, sub-meshes per font),
  `Pads` / `layout_text_with` (each font's material padding and style
  padding; layout still runs with none), `DynamicAtlas::add_glyph`,
  `draw::scale_y_in_root_canvas`, a second UV (`uv1`) on every vertex
  and a material index on every draw item. The draw-list builder moved
  from `sn-inspect` to `sn-assets::ui_draw` (`Assets::ui_draw`,
  `GlyphAtlases`, `texture_key`, `font_texture_key`), so the client
  can reuse it in M9u5c. Found on data: every TMP material saves every
  property its programs read; the padding (1.25 texels) is below the
  atlas padding, so no quad reaches into a neighbour; bold text draws
  `Aller_W_Bd` glyphs in sub-meshes; two shaders the research missed
  (`UI/Circle`, `uGUI/DepthClear`). The glyph cost (2.96 ms each, 110 at
  load) stays under the target, so the distance field was not sped up.
  The clip keyword is set natively (not in the IL), so it remains a
  hypothesis until M9u5c draws it.
- **M9u5b.** Built as planned. `sn-ui::scaler` (`UiCamera`, `place`,
  `Placement`, `apply_to_tree`), `Assets::ui_scalers` and
  `Assets::ui_draw_placed` (the canvases sized before the layout),
  `sn-unity::Camera::clear_flags`, `sn-inspect ui --canvas` (also 16:10
  and ultrawide screens). Found: the canvas's frustum follows the
  **main** camera's field of view, and the UI camera copies it
  (`SNCameraRoot`), so the two always match; the UI camera clears
  nothing and draws after the main camera, so the UI goes over the
  finished world image, as M9u5c's pass after tonemapping does. Only
  Static mode is ported; Inversed (the PDA screen) waits for M9u8a.
- **M9u5c.** Built as planned, plus a correction found on the first
  run. Built: `sn-client::game_ui` (a worker thread that owns the
  install and rebuilds on a window-size change; textures sent once; a
  render pass of our own after tonemapping with one pipeline per blend
  state; per-item values in a storage buffer), `game_ui.wgsl`
  (`UI/Default`, `UI/Simple`), `--game-ui` (hides our bars and depth;
  our hand text and fade stay), `DrawItem::root`, `UiDraw::textures`,
  and `UiNode::game_object`. The first run was a black screen: the
  stored HUD keeps four full-screen black panels that the game's
  scripts hide at start. The fix, `sn-assets::ui_start`
  (`UiStartState`), ports their first-frame state (`uGUI_Fader`,
  `uGUI_SceneIntro`, `uGUI_Overlays`, `uGUI_HardcoreGameOver`), which
  `ui_draw_placed` now takes. **For M9u6/M9u7:** which screens show at
  start is each screen's rule. The feedback collector, confirmation
  dialog and pop-ups still draw in their stored state.
- **M9u5d.** Built as planned: `game_ui_text.wgsl` (TMP's Distance Field,
  every term of the underlay-and-clip variant, others by flag), a second
  item buffer and pipeline kind in `game_ui.rs`, the font atlases sent
  as R8 textures and replaced in place when glyphs are added, and TMP's
  material values per draw. Not ported: `_FaceTex`/`_OutlineTex` (white;
  none set in scope), the glow and bevel terms (not in this shader's
  keywords), `UNITY_UI_ALPHACLIP` (no material). The texts show their
  stored strings until each screen's rules set them (M9u6, M9u7).
- **M9u6a.** Built as planned: `sn-assets::ui_scene` (`UiScene`,
  `UiState`, `LocalTransform`, `SpriteRef`, `color32`), `sn-ui::tween`
  (`smooth_damp`, `Sequence`), `Graphic::alpha` in `sn-ui::draw` (the
  renderer's own alpha), `Assets::script_fields` made public (the
  rules read their scripts' fields), and `sn-inspect ui --live`.
  `ui_draw_placed` is now a scene drawn once, so every older check runs
  through the new path. Its hashes are unchanged. Found: the first run
  missed the target (mean 4.15 ms). Half of the text time was copying,
  padding and interning each text's material on every draw, so the
  derived text and fallback materials are now kept per (material,
  font). A frame that needs new glyphs still pays M9u3's ~5 ms each
  (frame 0: 8 digits, 42 ms). **For M9u6b:** the client's worker keeps
  the scene and draws the start state on a size change. The per-frame
  path (the main world sends the rules' inputs, the worker runs the
  rules and draws) comes with the first rules, as there is nothing to
  send before them. The digits the bars show can be made at load to
  avoid frame 0's cost.
- **M9u6b1.** Built as planned. `sn_ui::state` (moved from `sn-assets`:
  `SpriteId`, `MaterialValues`), `sn_ui::bar` (`CircularBar`,
  `mesh::graphic_quad`), `sn_ui::hud` (`Hud`, `HudConfig`, `HudInputs`,
  `hud_mode`), `sn-assets::hud` (`hud_config`), `survival_start` and
  `sn_dotnet::method_store_f32` (a store in any method), `Assets::
  object_fields`, `UiGraphicKind::CircularBar`, `sn-inspect ui --bars`,
  and in the client `game_ui_bar.wgsl` (a third pipeline kind with the
  overlay texture in group 2) and the per-frame path. The main world
  sends `HudInputs` every frame. The worker runs every frame's rules in
  order but draws only the latest state, and logs in full only on a
  size change, with a summary every 600 frames. Found:
  - the main scene stores `Survival.food`/`water` as 0; the start values
    are `ResetStats`' constants (read from the IL);
  - the shader is `Hidden/UI/CircularBar` in the `preloadedshaders`
    bundle, and only the oxygen bar has an overlay;
  - `mesh::color32` rounded half away from zero, unlike `Mathf.Round`
    (fixed; no hash moved);
  - WGSL needs the texture samples before the program's branches.

  The worker's per-frame draw (3.3–4.4 ms while the world streams) runs
  off the main thread. The pulse, coin turn and punch are M9u6b2.
- **M9u6c1.** Built as planned: `sn_ui::depth` (`DepthCompass`,
  `DepthConfig`, `DepthMode`, `floor_depth`), `HudInputs::depth` and
  `compass`, `Assets::depth_config` (the language read with
  `sn_assets::language`), `sn-inspect ui --depth`, and the client's
  depth input and per-second log. Found: the game's
  `OnDepthClassChanged` throws away its argument and uses `Safe` (IL), so
  the danger sprites and colour are never shown and are not read; the
  label texts are set in `Initialize`, so `SetVisible(true)` would show
  them at their stored places until M9u6c2 moves them (headless only:
  the client sends no compass yet). Mode `Player` is "the HUD active and
  the PDA closed": vehicles, the camera drone, the intro and the rocket
  are not ported.
- **M9u6c2.** Built as planned: `sn_ui::compass` (`Compass`, `Label`,
  Unity's `Quaternion.Euler`, `TRS`, `Perspective`, `MultiplyPoint3x4`),
  `DepthCompass::direction` and `HudInputs::yaw`,
  `UiGraphicKind::Compass` drawn by the scene, `uGUI/Compass` as flag 16
  of `game_ui.wgsl` (its values in the `_MainTex_ST` slot, no new
  pipeline), the client's yaw (`euler_y`) and compass inputs,
  `sn-inspect ui --compass`. Test aids: `--test-supply TECH` (a new
  world's locker, before anyone joins: `Server::add_test_supplies`,
  `ItemsState::add_supply`; the pickup prefab read into the rules by
  `item_rules_with` / `rules_with`, so a joiner passes the same flag),
  `--panel …=unequip`, `--turn DEG_PER_S`. Found: (1) the labels' stored
  places are the editor's values at `debugDirection` 0.646, an oracle;
  (2) the rule must always state the compass's visibility, since the
  start state already applied a first `SetVisible(false)`; equipping
  from the first frame otherwise never showed it; (3) the compass is
  not a pickup in our rules (none is placed in the world), so the first
  test supply was refused; (4) `uGUI_TextGradient` on the labels does
  nothing: TMP never calls mesh modifiers.
- **M9u6e1.** Built as planned: `sn_ui::death` (`DeathScreens`,
  `PlayerDeath`, `Fader`, `DeathInputs`, `DeathType`), `sn-assets::
  death_config` and `vignette_times` (the IL's `Invoke` delays, rates and
  alpha limits through the new `sn_dotnet::invokes`, `multipliers_f32`,
  `field_compares_f32`), `sn-inspect ui --death`, and in the client
  `body::death_events` (the local player animator's `DeathFade` /
  `DeathCut`) with the player's frame events in `hud::Hud`; the UI
  worker's inputs now go out in `PostUpdate`. Found: `player_death` plays
  `quick_death`, whose `DeathFade` is at 0 s, so the vignette starts on
  the death's next frame; `uGUI_Fader.FadeOut()` uses `fadeInTime`. Our
  M9c fade stays only without `--game-ui` until M9u6e2 removes it.
  The respawn screen draws under the vignette and the out-of-oxygen
  overlay, so after a drowning "Loading..." is covered (kept as read).
- **M9u6e2.** Built as planned: `--game-ui` the default, `--no-game-ui`,
  `hud.rs` removed (`Hud`/`HandHover` in `player.rs`), the feed created
  by `GameUiPlugin` too (fly camera), `game_ui=` in baseline reports.

**Risks.** TMP's layout has many options. M9u4 ports only what the
in-scope screens set, and logs any option it meets but does not port.
Our distance field will not match FreeType bit for bit. M9u3 measures
the gap against the shipped atlas instead of assuming it is small. The
step-M9u4 oracle (saved driven rects) is a hypothesis. If it does not
hold, layout is checked against hand-computed cases and the
screenshots only, and the plan says so. Size: about M9g's, ten steps.

#### M9e plan: crafting at the lifepod's fabricator (written 2026-10-27, after M9u; M9e1–M9e3 done)

**Why next:** the road (§ 4.3, Phase E) puts M9e after M9u, and both of
its bases are done: the menus are read from the DLL (P1: 7 menus, the
fabricator's 100 nodes (87 craft nodes)), the recipes from TechData (P0), and the
client can draw, hit-test and animate the game's own canvases (M9u).
Crafting is the first thing in the game that turns picked-up items into
new ones, so the inventory, the locker and the notifications get their
first real use. M9f (saves) comes after it, so a crafted item can be
saved.

**What the game has** (class and field names checked 2026-10-27 with
`sn-inspect code --fields`; the behaviour is read in the decompiled code
in M9e1; confirmed rules and corrections are recorded in gameplay.md
§ Crafting and ui.md § Crafting menu research):
- `Fabricator` extends `GhostCrafter` extends `Crafter` extends
  `HandTarget`: the hand target (`handOverText`, `closeDistance`), the
  open/close animator, beams, sparks, light and sounds; `craftTree` picks
  the menu (`Fabricator`).
- `CrafterLogic`: `timeCraftingBegin`, `timeCraftingEnd`,
  `craftingTechType`, `linkedIndex`, `numCrafted`; the crafted item waits
  in the fabricator until the player takes it (`onItemPickup`).
- `CrafterGhostModel`: the item's ghost model with the fabricating
  shader (`_EmissiveTex`, `_NoiseTex`) and `VFXFabricating`.
- `uGUI_CraftingMenu` (an `uGUI_InputGroup`; `uGUI/CraftingCanvas`
  exists, M9u0): the tree of icons, expand/craft sprites, punch
  animations on expand and click; grey icons for unknown or unaffordable
  recipes; the recipe tooltip with its ingredients.
- Power: the fabricator spends energy per craft from the pod's power
  (`PowerRelay`); without power it shows `unpoweredSecondaryText`.
- Craft time: TechData's `craftTime`; an entry without one uses the
  crafting code's default (`dotnet.md` hypothesis, settled in M9e1).

**Steps** (each one change, revertable alone, with its own MODLOG entry):

| Step | Work | Done when |
|---|---|---|
| **M9e1** ✅ (2026-10-27; see as built) | **Rules, headless.** Read `Crafter`, `GhostCrafter`, `CrafterLogic`, `Fabricator`, `uGUI_CraftingMenu`, `KnownTech` and the recipe checks in the decompiled code; record them in `docs/formats/gameplay.md` § Crafting (each fact marked). `sn-sim::craft` (pure): can-craft (known, ingredients in the inventory, power), room checked per output at pickup (confirmed in M9e1), consume, the result and `craftAmount`, linked items, the craft time and its default, the energy cost. `sn-inspect craft`: every fabricator node resolved. | Unit tests on synthetic recipes (missing ingredient, unknown blueprint, no power, full inventory, `craftAmount` > 1, linked items); `sn-inspect craft` on the install: 100 fabricator nodes, including 87 craft nodes (= P1; corrected count in M9e1), each with ingredients, time and energy logged, 0 unresolved; the starting blueprints from `PDAData` counted; the default craft time settled with its source. |
| **M9e2** ✅ (2026-10-27; scripted path; limitations below) | **Server-authoritative crafting.** `sn-protocol`: `Request::Craft { crafter, tech }`, `Request::TakeCrafted { crafter }`; world events for a craft started (with its duration), done and taken; the fabricator's state and the pod's power in the world state and its hash. `sn-session` checks with `sn-sim::craft`; one craft at a time per fabricator, shared by all players. A `--craft SECONDS=TECH` test hook. | Protocol round-trip tests; a scripted solo run crafts one real recipe (e.g. Titanium from Scrap metal picked up nearby): inventory counts before/after, the duration equal to the rule, the energy drop logged; a refused craft (no ingredients) logged with its reason; 0 warnings. |
| **M9e3** ✅ (2026-10-27; plain effects, see as built) | **The fabricator in the world.** The hand on it (`handOverText` from the language file, `closeDistance`), its animator's open and close, the result shown in the fabricator until taken (the item's model at `itemSpawnPoint`); beams, sparks and `CrafterGhostModel`'s shader drawn if they fit, otherwise kept drawn plainly and logged (never hidden). | Client run: hand text logged; open at the hand, close beyond `closeDistance` (distances logged against the stored value); the crafted item drawn at the spawn point during the craft and gone when taken; 0 warnings; a screenshot for the user. |
| **M9e4** ✅ (split 2026-10-27 into M9e4a–b below) | **The crafting menu.** `uGUI_CraftingMenu` on `CraftingCanvas`: the fabricator tree's nodes as icons (made as the game makes them), expand and craft sprites, grey for unknown/unaffordable, the punch animations, the recipe tooltip; hover and clicks through the existing hit-test; a click sends `Request::Craft`. The unlock pop-up and `Blueprints` notifications if crafting unlocks anything. `sn-inspect ui --crafting`. | `ui --crafting`: node count and layout of the top level and one sub-menu against a replay of the decompiled code, N/N; client: open the menu, hover a recipe (tooltip text logged), click it: the same craft as M9e2, the item taken into the inventory with its notification; 0 warnings; a screenshot. |
| **M9e4a** ✅ (2026-10-27) | *Split from M9e4 (its risk note): the rules first.* `sn-ui::crafting` (`UpdateTree`'s filter, `GetIconMetrics`, expand/collapse, punches, chroma, lock/fade/progress, hover sprites, clicks, the hit-test), `TooltipFactory.CraftRecipe`/`CraftNode` texts, `sn-assets` menu nodes/sprites/tree/icons, `sn-inspect ui --crafting`. | Unit tests on a synthetic tree; `ui --crafting`: node counts (100 = P1 with all known; with the starting blueprints = the replay), top level and two sub-menus N/N against a replay, chroma N/N, tooltips, a click crafting and locking, the draw. |
| **M9e4b** ✅ | *The client half of M9e4.* The menu opened by the fabricator's hand (cursor freed, look stopped, movement kept), its icons drawn on `CraftingCanvas` with `GRAYSCALE` ported, hover and clicks through the hit-test, the tooltip's texts, a click sends `Request::Craft`; Lock/Close at the craft's start, closed beyond `closeDistance`; `--menu` test hook. | M9e4's client Done-when. |
| **M9e5** | **Close-out.** Two players: one crafts, the other sees the fabricator busy (its menu refuses with the game's message) and the result taken; README, `docs/formats/gameplay.md`, `docs/formats/ui.md`, "Not 1:1 after M9e". | Two-player run logs as listed; `cargo test --workspace` passes without the game; `git status --short` shows only our code and docs. |

**M9e4b harness follow-up (2026-10-28):** preserve script order with one
step per frame, authoritative transfer confirmation, PDA fully closed,
and UI-worker sequence acknowledgements plus hit-test readiness for each
menu target. Absolute timestamps are earliest times, never a reason to
drain steps together. No game-rule changes.

**M9e done when:** every step's list above has been run and passed, and
the original row's checks hold (rules unit-tested on synthetic recipes,
one real recipe crafted in a scripted run with counts before/after
logged, the menu's node count equal to P1's).

**M9e3 as built (2026-10-27):** hand, range, preview and automatic
collection verified in scripted runs; scoped optimized-avatar restoration
resolves 120/120 bindings. The checkpoint's open/close pose residual
(0.031879187) was the test's reference sampled mid-way through the
0.033 s non-looping `closed` clip; referenced after that clip ends, the
cycle returns to the resting pose with residual 0 (animation.md). The
fabrication shader, particles and light are drawn plainly and logged,
not ported.

**M9e1 as built (2026-10-27):** pure recipe validation/consumption and
incremental output collection, runtime TechData conversion, `sn-inspect
craft` and an opt-in install test. Corrections to the hypotheses: P1's
100 is all nodes (87 craft, 12 category, 1 root); room is checked during
pickup, which can leave partial output; the default animation is 2.7 s
and the logic adds 0.1 s; energy is 5 per craft. The original code
automatically tries pickup in range, and a busy hand click has no text
refusal (progress reticle only). M9e2–5 must preserve these rules.

**Out of scope:** the other crafting machines (Constructor and the
Mobile Vehicle Bay, Workbench, upgrade consoles, the Cyclops fabricator:
vehicles, item 5); eating and using what is crafted (item 1); the
crafting sounds (item 8).

**Risks:** the menu's layout code (`uGUI_CraftingMenu`'s node placement
and expand animation) may be larger than the tooltip's; M9e4 is split if
so. The lifepod's power may depend on pod state we do not model yet
(damage, power cells); M9e1 finds out, and a missing piece gets a
labelled stand-in ("not the game's") rather than a guess. Size: about
M9d's, five steps.

#### M9f plan

Written 2026-10-27. Our own server save format, independent of the game's
Protobuf saves; this becomes M11's persistence. No game assets are saved.
Research is recorded in `docs/formats/saves.md`; no decompiled code is copied.

Use the existing checked binary codec in pure `sn-session` (layer 2), with
magic, save version, protocol version, build/rules fingerprint, payload length
and FNV checksum. Reuse the item snapshot for inventory (crafted items are
ordinary items), equipment, unlocks, luck, locker, removed/picked keys,
outcrop hit counts, spawned/dropped items and id counters. Save stable
name/id mappings, latest player pose and vitals, world settings, elapsed
ticks and the server random generator. Connections and credentials are never
saved. Names retain M10's trust model; they are not authenticated identities.
No dependency on the concurrent M9e implementation.

File I/O lives in the apps, shared through `sn-server`: default user data
folder `%LOCALAPPDATA%/subnautica-rs/saves/default` on Windows,
`~/Library/Application Support/subnautica-rs/saves/default` on macOS,
`$XDG_DATA_HOME/subnautica-rs/saves/default` (fallback `~/.local/share`) on
Linux. An explicit save-directory option supports slots and tests; reject
paths inside the game install or `out/`. Tests and agent runs use system temp.
Save every 30 s (configurable) and on orderly shutdown, including Ctrl-C;
forced termination can only retain the last autosave. Flush a temporary file,
retain the previous validated save as a backup, then install the new file.
Corruption, incompatible versions/build/rules and semantic errors return an
offset-bearing error and refuse startup without overwriting either file;
recovery from backup is explicit. No silent reset or automatic migration.
New dependency planned: `ctrlc` in the app handles orderly console shutdown;
all serialization uses existing dependencies.

| Step | Work | Done when |
|---|---|---|
| **M9f1** ✅ | Plan and read-only format/state research. | Plan written before code; confirmed findings documented, no game code/data copied. |
| **M9f2** ✅ | Pure versioned serialization and session restore, stable offline players, pose/vitals restore messages (app application in M9f3). | Synthetic round-trip preserves all saved categories; truncated, corrupt, incompatible and invalid saves fail with offsets; reconnect sends inventory and own pose/vitals. |
| **M9f3** ✅ | Shared app save I/O, platform folder, autosave and shutdown in dedicated and embedded servers. | Temp-directory tests cover backup, corruption refusal, path rejection and restart; both server modes compile; no filesystem dependencies in layer 2. |
| **M9f4** ✅ | Real server/scripted-client restart proof and documentation close-out. | Real install run logs pickup/move/drop and matching saved/restored numbers; workspace tests, clippy and fmt pass; merge local main, resolve keeping both docs, rerun tests/clippy. |

M9f is done only when every Done-when above has passed. Linux/macOS runtime,
power loss durability, original-game save compatibility and M11 ownership
are not tested here; original-game save compatibility is not implemented.
#### M9s plan: survival (written 2026-10-27)

**Scope and dependencies.** Pull After Phase E item 1 into Phase E after
M9f. M9d supplies inventory ids, equipment, world keys, outcrops and the
locker; M9c supplies health, breathing and death; M9g supplies the body;
M9u supplies the actual HUD/PDA. M9e (crafting, in progress on main by
another agent) must deliver its server output/collection path and food
menu before the end-to-end loop. M9f (save/load, also in progress) owns the
save schema and atomic restart boundary. Coordinate narrow extensions to
those interfaces; do not rewrite either milestone or mark either complete.
Rules below can be built headlessly while those integrations are unfinished.
No new crate, architecture change or dependency is proposed.

**Evidence and boundary.** `docs/formats/gameplay.md` sections “Survival”
through “M9s runtime provenance” record direct source-confirmed rules and
unmeasured hypotheses. This is a plan, **not tested implementation**.
Every gameplay number must be loaded from the player's install at runtime:
TechData/defaults; prefab/scene fields; DLL metadata constants and bounded
constructor/method IL patterns for code-only values. In particular neither
a familiar tank capacity nor a C# field initializer proves the stored prefab
value. Use `sn-install` for I/O, existing `sn-assets` orchestration and pure
`sn-unity`/`sn-dotnet` readers. Layers 0–2 take bytes/data only. Missing or
ambiguous required numbers stop the affected feature with an offset/source
error; never silently compile these research numbers into balance defaults.
Stay within the existing limited IL-reader approach; if extracting a rule
would require a general interpreter, stop and ask before expanding scope.

**Rules to preserve.** Hunger's 10 s cadence and phase, deficit-based Starve
damage, regeneration, reset/freeze and the 99 food gate; successful
consumption even when clamped; food age and source-specific oxygen survive
moves. Binding/held state uses item identities. Knife damage occurs at an
authorized use event, not both input and animation; DamageAlive and
DamageDead use before/after liveness. Cabinets, crate contents and harvesting
are shared; bars, inventory actions and bindings are the owner's. Co-op
ordering, replay protection and loot seeds are explicitly our server rules.

**Steps** (each one independently revertable change with its own MODLOG;
synthetic unit tests never embed game fixtures; numbered verification hooks
below are proposed interfaces, **not available commands yet**):

| Step | Work | Done when |
|---|---|---|
| **M9s1** ✅ (2026-10-27; MODLOG) | **Hunger rules in sn-sim, pure.** *As built:* `sn_sim::survival::{HungerRules, Survival}` (ordered warning/damage/heal effects, `start`/`advance`/`respawn`/`heal_temp_damage`; 7 tests) and `sn_assets::hunger_rules`, which reads every number from `Survival`'s IL and checks it against `SurvivalConstants` (3 tests; opt-in `the_hunger_rules_are_read`). *Planned:* Typed runtime rule input, survival clock accumulator/phase, food/water, drain, starvation, regeneration, warning crossings, freeze and respawn. Do not enable live drain yet. | Synthetic hand-calculated tests: 60 s starting at food/water 100 loses 2.380952/3.333333 (within float tolerance); both empty for 10 s requests 23.809524 damage; exact/over threshold warnings, combined-stat healing gate after drain, freeze and respawn all pass. Partitioned tick input preserves the same interval count and phase. |
| **M9s2** ✅ (2026-10-27; MODLOG) | **Per-item state and consumption in sn-sim.** *As built:* `sn_sim::inventory::{ItemState, EnergySlot}` on `Item` (`Item::ids`), `sn_sim::survival::{ConsumeRules, EatableInfo, Eaten, Used, UsedItem, eat, eat_item, use_tech, use_item, add_health, initial_state}` (8 tests) and `sn_assets::consume_rules` + `sn_dotnet::int_literals` (2 tests; opt-in `the_consume_rules_are_read`). *Planned:* Extend existing item ids with optional Eatable age, oxygen and battery state plus battery-slot ownership, rather than introducing another inventory. Pure eat/drink/first-aid and item initialization rules; move/drop/death preserve state. | Synthetic tests cover food 99 versus >99, 200/100 caps, negative and zero food/water, successful clamped consumption, first-aid actual-heal threshold, and Bladderfish oxygen. Two identical tech types with different ages/charge stay distinct through 1,000 grid/locker/equipment/drop transfers; no item or charge duplicated. |
| **M9s3** ✅ (2026-10-27; MODLOG) | **Decay and energy rules in sn-sim.** *As built:* `sn_sim::survival::{DecayRules, DespawnState, Freshness, freshness, is_rotten, iterate_despawn, set_decomposes}` (3 tests), `sn_sim::survival::{EnergyMixinInfo, EnergyRules, ReloadOutcome, ReloadRefused, add_energy, consume_energy, energy_capacity, energy_charge, energy_scalar, make_battery, modify_charge, restore_battery, select_battery, set_battery_charge, sort_reload_candidates, spawn_default_battery}` + `sn_sim::inventory::{force_pickup, flush_pending}` (4 tests), and `sn_assets::{decay_rules, energy_rules}` (1 test; opt-in `the_decay_and_energy_rules_are_read`). *Planned:* Game-clock freshness/negative floor, active-world rotten despawn, battery add/partial consume, default battery only if absent, compatible swap/unload through existing pending inventory handling. Keep tool-specific costs separate. | Synthetic tests check exact half/nine-tenths freshness boundaries, −25 floor, inactive rotten retention versus active deadline, and disabling decomposition. Empty/full/partial energy changes conserve charge; reload at full grid retains the outgoing battery exactly once; restoring a used battery never fills it. Missing required rule input refuses explicitly. |
| **M9s4** ✅ (2026-10-27; headless; MODLOG) | **Equipment effects in sn-sim.** *As built:* borrowed `OxygenSources`, restore/swap/fill/drain/totals; `EquipmentRules`, `swim_max_speed`, `swim_charge`; runtime DLL coefficients and stored oxygen reader, four tank prefab checks. Serialized charging-component placement/rate stopped after two checks and remains explicitly required input, never a default: the main scene and the SwimChargeFins prefab hierarchy each have zero `UpdateSwimCharge` behaviours, and the DLL rule and field initializer prove neither a placed component nor a stored rate (hypothesis: another spawned hierarchy owns it, or it is unused legacy code; M9s7's census settles it). Live wiring remains M9s11. Registered oxygen sources, tank swap, surface fill/drain order, totals for M9c suffocation; ordered tank/fins/held-tool swim modifiers; charging-fin gate using actual velocity. | Synthetic player reserve plus two differently filled tanks: equipped tank drains first, spare contributes 0, unequip/re-equip retains its amount, fill adds no more than supplied. Scripted pure movement compares max-speed calculation before/after modifiers in the game's order; charging at exactly 2 m/s adds 0 and above it adds rate × elapsed time up to capacity; holstered/above-water adds 0. |
| **M9s5** ✅ (2026-10-27; workspace check passed with M9s6; MODLOG) | **Quick-slot and tool state in sn-sim.** *As built:* `QuickSlots`, typed grid identities and tool timing, event trace and first-use set; six synthetic tests, bounded DLL count/timing rules and stored Knife timings. No live rendering/input integration (M9s11). Five-slot count supplied by rules, Hand eligibility, auto/manual binding, duplicate-type restrictions, same-type refill, desired versus held slot, draw/hold/holster/drop Sequence, first-use flags and interaction/PDA/death suspension. | Synthetic tests exercise same-slot unbind, moving a binding, first copy/full toolbar, scroll end deselection, removal/refill, changing slot mid-draw, bash/use delaying holster and PDA/death interruption. Event traces assert one equip/unequip per transition, no dangling item id, transition timing within two supplied simulation steps of the Sequence rule. |
| **M9s6** ✅ (2026-10-27; headless; MODLOG) | **Harvest and supply rules in sn-sim.** *As built:* `sn_sim::survival::{ToolUseGate, ToolAction, knife_strike, grant_harvest, outcrop_click, bash_outcrop, Cabinet, SupplyCrate, draw_stored_loot, StoredLoot}` (6 tests) and `sn_assets::{world_survival_rules, verify_loot_sampler, knife_info, cabinet_info, harvest_info, stored_loot_info}` (2 reader tests; opt-in `the_world_survival_rules_are_read`). Crate items proved to be placeholder children (8/8 prefabs); SpawnStoredLoot has 0 prefab users (open, M9s7). Seed RNG and rejection limit are ours. *Planned:* Headless knife damage requests call a supplied authoritative DamageSystem resolver exactly once and use its before/after liveness; full target-specific damage modifiers/lifecycle stay with M9s9. Stored-loot curve evaluation is supplied from the runtime AnimationCurve reader, with bounded rejection sampling (our termination limit). Knife target validity/damage/liveness and TechData output; connect existing outcrop bash once. Cabinet ready/door/take timers; crate sealed/open/child state; separate SpawnStoredLoot initialization with seeded sampling and once flag. | Synthetic tests: DamageAlive live/surviving/killed/dead, DamageDead killing/dead hit, final bonus, None, invalid target and full inventory. One input/event produces one hit. Cabinet equality at deadline gives no kit, later gives one; full grid retains it. Crate unopened/sealed/full/empty and absent Animation cases pass. Seeded storage initialization repeats identically and runs once. |
| **M9s7** | **Install data and rule provenance, headless.** Minimal serialized readers and bounded IL extraction; assemble the prior rule inputs and fingerprint. Census edible/tank/battery/tool/cabinet/crate prefabs, harvest targets’ LiveMixinData and tool animation-event times, plus charging-component placement and serialized rate, resolve TechData harvest outputs/defaults and food crafting nodes. Trace crate child activation/loot and tank initial oxygen; settle hypotheses before claiming support. Proposed `sn-inspect survival --check-data`. | Synthetic malformed bytes and altered/missing/ambiguous IL patterns return errors with offsets, 0 panics. Opt-in install run logs each required number with asset field or DLL member/pattern, counts by component, every supported output's resolved prefab, tank capacities and initial-state origin; 0 unresolved required fields. Two reads have identical fingerprints. Properties/NonSerialized fields are excluded. No exported content table enters git. |
| **M9s8** | **Owner actions on server.** sn-protocol/sn-session validate consume, slot bind/select, battery reload and equip against authoritative inventory/state. Same sn-sim rules in solo and UDP sessions; owner snapshots/deltas and hashes cover survival and per-item state. Implement but gate live hunger until acquisition/use paths work. | Round-trip tests; simultaneous use/drop/craft of the same item succeeds only once, wrong-owner and stale requests refuse without mutation. 10,000 synthetic ticks including swaps/uses match authoritative hashes with 0 differences; malformed requests never panic. Rules fingerprint mismatch refuses join; no game assets sent. |
| **M9s9** | **World actions on server.** Authorize tool-use timing, reach/occlusion, target eligibility, one attack event per accepted action; shared harvest health/removal/output, cabinet timers, crates and once-only loot keyed by existing EntityKey. Do not trust client-reported damage, health, output or elapsed time. | Two simulated players race a final cut, cabinet kit and crate pickup: each shared result occurs exactly once. Out-of-range/behind-wall, replayed and too-fast attacks grant 0 damage/output. Late join snapshot and 10,000 interleaved-action ticks yield 0 world-hash differences; unloading/reloading a batch never rerolls loot or resets health. |
| **M9s10** | **Persistence extension with M9f.** Add versioned survival/interval phase, game time, item age/oxygen/charge, nested battery ids, slot identities/used-tool flags, harvest target state, cabinets and crates/loot flags. Preserve M9e in-flight and waiting output state. Default migration only for fields legitimately absent in old project saves. | With M9f, synthetic round-trip retains every id and state. Proposed scripted save/restart mid-decay, mid-cabinet wait and with partially empty tank/battery has 0 state differences after restore; no recharge, freshening or loot reroll. Old-format migration and unsupported-version refusal tested. Offline wall time does not advance survival in our save policy; test that explicitly. |
| **M9s11** | **Client input and acquisition/use loop.** Route inventory Use/Eat and quick-slot hotkeys/scroll/PDA-hover binding to server; equip tanks/fins; cabinet/crate hand targets and reticle/progress, child loot and harvest collision updates. Integrate M9e food tab/results and initialize every crafted item's state once. Enable live hunger only after this loop passes. Proposed `sn-inspect survival --scenario loop` plus equivalent client hooks. | Install-backed scripted loop obtains supplies, crafts available food/water and equipment using M9e, consumes them, equips/swaps a tank, harvests with a crafted knife and takes a kit/crate item. Log item ids/counts, food/water/health, oxygen and every refusal before/after; acquired amount equals consumed + retained, 0 duplicated objects. After enabling drain run 1,800 game seconds with scripted replenishment; measured interval losses equal runtime rules. |
| **M9s12** | **Held models and animation events.** Plug tools into existing body sockets with prefab offsets/layers/collider policy; animator holding/first-use/use/bash flags and event timing; apply held-tool speed changes. Replicate remote held identity/action so both clients draw their own installed model. Support the knife path and generic energy/reload hooks; other tools keep their own later behaviour. | Headless client logs active socket/layer and exactly one held model per player, draw/holster timestamps versus prefab times; one knife animation use produces one server hit. PDA, swap, drop, death and respawn leave 0 orphan held models/colliders. Two clients agree on held ids and action ticks. Human visual comparison is a bonus, not a pass criterion. |
| **M9s13** | **Live HUD, toolbar and per-item UI.** Feed M9u food/water/oxygen bars and onEat/onDrink, bind/select toolbar icons, localized use/full-health/reload messages and battery selector. Add EnergyMixin/Battery/Oxygen/Eatable tooltips, UI/IconBar and GRAYSCALE for grid/equipment/toolbar/dragged icons using TooltipFactory precedence. | Proposed `sn-inspect ui --survival` replay: synthetic empty/half/full/overfilled food, differently aged food, empty/partial tanks/batteries, disabled icons and dragged items give N/N expected values/visibility. HUD ceil/smoothing and event punch timings match rule replay; client click/hotkey/scroll/reload scripts log accepted actions and matching UI ids/values, 0 new unsupported-material warnings. |
| **M9s14** | **Two-player close-out and regressions.** Proposed `sn-inspect survival --scenario coop` and `--scenario restart` exercise food crafting/consumption, tool swaps/harvest, tanks/fins, cabinets/crates, late join and M9f restart under existing impaired-network harness. Reuse current CLI/session patterns; document final exact syntax in README/MODLOG. | Both scenarios log 0 duplicate grants, 0 hash mismatches and expected rule-sourced times/amounts, with owner UI isolated and shared objects consistent. `cargo test --workspace` passes without game files; opt-in tests skip cleanly without SUBNAUTICA_DIR. With install, `cargo test --workspace -- --ignored` passes applicable M9s checks; record any unrelated failure honestly. |
| **M9s15** | **Document and audit completion.** Update tested README capabilities, gameplay/UI format findings, Phase E row and the residual list below; retain all older unrelated gaps. | Every prior Done when has recorded command/output evidence; no unchecked hypothesis claimed implemented. `git diff --check` passes; `git status --short` lists only our source/docs. No game content, new binary whitelist, dependency or game-install mutation. Commit/push only when separately authorized for implementation work. |

**Verification commands planned:** implement and document
`cargo run -p sn-inspect -- survival --check-data --game-dir <install>`,
`cargo run -p sn-inspect -- survival --scenario loop --game-dir <install>`,
`cargo run -p sn-inspect -- survival --scenario coop --game-dir <install>`,
`cargo run -p sn-inspect -- survival --scenario restart --game-dir <install>`
and `cargo run -p sn-inspect -- ui --survival --game-dir <install>` with the
checks in their rows. These new flags are **not tested/not implemented**.
Use synthetic rule values in headless tests and source values only through
runtime loaders in real runs. Test counts are evidence to record when built,
not a target to satisfy by mirroring implementation.

**M9s done when:** all fifteen lists pass, M9e's actual food/menu/output
integration and M9f's restart integration are complete, and survival drain
is enabled only with working food/water acquisition and consumption. Each
step is additive at its boundary: pure rules do not change live play, runtime
loading does not enable drain, server state precedes UI, and each can be
reverted without refactoring another milestone. Incompatible protocol/save
changes must use existing version gates rather than pretend compatibility.

**Open questions to settle in M9s7/integration:** actual serialized tank
capacities and spawn-state initialization, Eatable rates and active-state
lifecycle, pod cabinet wiring/interval, child-loot versus random storage
crate population, harvest target destruction/collision changes, and bounded
IL coverage for inlined constants. Clock phase and animation callback order
against Unity are hypotheses until measured; equal-priority oxygen-source
ordering must not be claimed stable. No weakness movement penalty unless
its caller is proved. If a supported object lacks required data, log the
miss and keep the corresponding step incomplete rather than substitute a
remembered number.

**Not 1:1 after M9s** (planned residuals, not claims of work completed):
- **Audio** (After Phase E item 8) and **particles** (M7i): food/tool/knife,
  cabinet and crate sounds, hit/break effects and charging-fin loop audio.
- **Tools and IK** (remaining item 3): generic held flow and knife are in
  M9s; exact tool-specific IK solvers/aim blending and behaviour of scanner,
  repair tool, propulsion cannon, builder and vehicle tools stay with their
  own milestones. Prefab targets are retained; unsupported IK is logged.
- **Creatures** (item 4): live fish capture/spawning/AI, creature combat,
  smell and attraction. Do not gate basic food access on unimplemented AI;
  the verified loop uses installed recipes and reachable existing supplies.
- **Vehicles/bases/suits** (items 5/6): non-player oxygen sources, water
  parks, rebreather/suit effects beyond existing rules, base medical cabinets'
  power/construction lifecycle, powered tools' individual drain/abilities and
  electronics-disable effects. No new universal tool drain in M9s.
- **Story/PDA** (items 2/7): custom-goal side effects, infection/cure and
  full databank/notification integration. Existing M9u formatting, drag,
  toolbar-tooltip, gamepad and language limitations remain unless a step
  explicitly verifies them; food/tank/energy tooltips and icon bars move here.
- **Physics/look** (M11/§ 3.3 and deferred look): rigid-body drops/tumble,
  floating objects and sky-environment changes remain M9d gaps. Exact Unity
  frame scheduling and original-game visual comparison are **not tested**.
- **Co-op/persistence policy:** deterministic server ordering/loot, owner
  isolation and offline paused time are our rules; original Subnautica is
  single player. Linux/macOS, VR and gamepad use remain **not tested**.

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
1. **Survival** (pulled forward as M9s, planned above; not done): food and water (`Survival`, eating, the fabricator's food
   tab), knife and harvesting, first aid, tanks and fins; quick slots,
   held tools and item use; per-item state (battery charge, tank oxygen,
   food decay); supply crates (the gaps M9d leaves, listed in its plan).
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

### 4.5 Visual fidelity: remaining work and implementation order (written 2026-10-11)

**Scope:** the user asked to record the missing pieces of the visual review.
This section adds plans, not completed features or new format findings. It
collects existing gaps without replacing their detailed plans. The order below
is the proposed order when visual work resumes; § 4.3's gameplay deferral and
the decision to build M7f4h after M11 remain in effect. On 2026-10-11 the user
approved starting this visual plan; M8g0a below is the first implementation
slice, pulled forward from the gameplay deferral. A large refactor or an
architecture change still needs the user's approval before implementation.

**What already exists:** terrain materials and blending, placed objects, grass
shading, baked and FFT water waves, sky and stars, caustics, sun shafts, local
lights, object animation and blend shapes, the player's body and hatch
cinematics, and remote player bodies. Many are first passes whose original-game
comparison is still open. Earlier roadmap rows and "as built" notes describe
their state at the time: stars were added in M8c5; object specular/emission in
M8c7; animation and blend shapes in M7f4c/d; player eye placement in M9b/c;
shadow-only rendering in M9g. Do not count those features as wholly missing.
Their remaining fidelity checks still apply. M8g0a's implementation results are
recorded below; original-game visual comparisons remain **not tested**.

#### M8g0 — Reproducible visual and performance baselines

Before changing the look, define a comparison case by game build, client
revision, camera position/direction, resolution/aspect ratio, vertical FOV,
detail and water quality, post-processing options, world/slot seeds, lifepod
placement and lighting state, Aurora state, game day/time, and elapsed animation
and wave time. Record which original-game states can actually be reproduced;
our seed is not proof that the original game drew the same objects or waves.
Use existing flags where possible. Any missing capture/time-control tooling
gets its own small implementation step before relying on it.

Cases: Safe Shallows seabed; glowing coral at night; Kelp Forest; a cave; a
wreck; a Precursor interior; inside Lifepod 5; open sea above and below the
surface. Include biome/volume boundaries and shadow edges. Original-game
reference captures are supplied by the user; the agent's primary checks are
numbers. Store captures, game-derived geometry and measurement dumps only in
ignored `out/`, never as repository fixtures. Share findings as prose and
aggregate counts/errors, not game content.

For each case, log object/material/light counts, selected LODs and skies,
camera water coefficients, and CPU/GPU timings. Establish tolerances and
measurement methods before evaluating a change: region colour/luminance,
depth or silhouette differences where comparable, bounds and positions, and
event ticks. Separate deterministic errors from random wave/creature variation.
A screenshot alone cannot prove equal geometry or animation. Keep an explicit
list of differences and unavailable reference measurements.

Performance: release builds with matching flags, fixed routes/cases, three
before/after runs, and the same loading/warm-up policy. Record median/tail frame
times, individual GPU passes, memory and streaming stalls. The target from
§ 4.1 is 16.7 ms in the dense start area on the dev machine; historical README
or MODLOG timings are not a fresh baseline or proof that the target is met.

**Done when:**

- Every case has a reproducible configuration and numerical logs, or an
  explicit "not tested" entry stating the missing reference/tooling.
- At least one outdoor, one interior and one water-surface case has the user's
  matched reference and a recorded difference list; missing cases stay open.
- Three baseline performance runs are recorded with their build/settings.
- Measurement tolerances are written before judging fixes. M8g0 is not called
  complete while required cases or reference measurements remain unavailable.

##### M8g0a — Named performance baselines (built 2026-10-11)

Add `--baseline NAME --baseline-revision LABEL` to a solo `--benchmark` run.
Save `report.txt`, per-frame timings in `frames.csv`, and `capture.png` under
`out/baselines/run-NAME/`; refuse an existing name rather than overwrite evidence.
Do not record usernames, network addresses, environment variables or raw
command lines. Record the explicit revision label, game build/rules fingerprint,
effective camera/settings/seeds/scene state, physical resolution, loading and
warm-up policy, scene counts, LOD/sky counts, fog coefficients, clock values,
frame-time statistics and available GPU diagnostics. GPU diagnostic averages
must be labelled with their actual scope, not assumed to cover every measured
frame. Use real elapsed frame time so Bevy's virtual-time clamp cannot hide
stalls. `--resolution WIDTH HEIGHT` and `--warmup FRAMES` provide fixed settings;
named baselines default to 120 settled warm-up frames. If streaming becomes
unsettled during warm-up, restart that warm-up count; during measurement count
unsettled frames in the report. Existing unnamed benchmark output remains
available.

This slice records, but does not freeze/reset, animation, waves or foam history.
It does not complete M8g0's matched visual references. Deterministic visual
time and original-game reference cases require later slices. Before assessing
the three repeated runs: camera/settings must match exactly; terrain/object/
material counts must match after settling, with differences explained rather
than silently tolerated. Frame times have no equality requirement: report the
spread and compare against the 16.7 ms target without calling it a regression
test until there is a before/after implementation pair.

**Done when:** argument/path validation and frame-statistics checks pass;
`cargo test --workspace`, clippy and fmt pass; three named runs of one fixed
outdoor case produce separate reports/CSVs/captures with the requested frame
count and physical size, unchanged camera/settings and checked scene counts;
loading and warm-up frames are excluded from the samples; existing-name
rejection preserves the first run. Record results and the remaining reference/
time-control gaps. Do not claim full M8g0 completion.

Initial outdoor case (configured before the three runs): Unity camera
`(0, -10, 0)`, look-at `(60, -40, 60)`, vertical FOV 60°, 1600 × 900 physical
pixels, view 1,200 m, High water, grading off, lifepod seed 1, slot seed 1,
damaged pod, Aurora held intact, start 9.6 hours with game time scale 0;
all ordinary object/grass/animation/light passes enabled. Warm up 120 settled
frames and measure 300. Expected deterministic fields: identical camera,
settings, terrain/grass counts, loaded object/material counts and light counts;
zero unsettled measurement frames. Animated bounds/LOD selection and GPU
times are recorded separately; unexpected changes require explanation.
Original-game reference for this case: **not tested**. An original save's
random placements/wave phase are not assumed to match these seeds.

**As built and verified:** `baseline.rs` and the client's existing measurement
path implement the controls and reports above. The snapshot runs after the
frame's updates. Five new tests check argument restrictions, output labels,
consecutive settled warm-up and frame statistics (including a 1,000 ms stall).
Physical resolution overrides apply only to named baselines or an explicit
`--resolution`; ordinary play retains its default DPI behaviour. Frame samples
use unclamped real time for both named and existing measurements. GPU values
are labelled as recent diagnostic history; live animation/water times and the
possibility of capture completing after the measured frame are explicit.

Three release runs (`out/baselines/run-m8g0a-shallows-1/` through `-3/`, build
label `cae48d6-m8g0a`, RTX 3080): each produced a 1600 × 900 PNG and 300 samples
after 120 warm-up frames, with 0 unsettled measurement frames. Sample sums
equal the measured real-time interval within 20 µs. All 58 checked static
configuration/scene fields match across runs and at each run's start/end:
1,393 shown batches, 7,759,704 terrain triangles, 34,469 grass tufts,
39,342 cell/batch objects, 1,968 materials, 508 point lights and 51 spots;
camera, sky, fog and LOD counts agree. The parsed-batch cache is a separate
memory metric: 1,150 / 1,200 / 1,166 entries. Its worker request order varies
and the LRU evicts 120 entries at a time; this does not change the shown scene.
Peak sampled process memory: 1.893 / 1.894 / 1.851 GiB.

Mean frame times: 17.94 / 17.95 / 18.02 ms; medians 17.69 / 17.83 / 17.86 ms;
p99 20.31 / 19.27 / 20.52 ms. The 16.7 ms target is **not met** in this case.
These are baselines, not a before/after performance claim. Existing-name
rejection returns exit 1 with all three original file hashes unchanged.
An ordinary player run (`--view 400 --kill 3 --shot 10 m8g0a-player-smoke`)
logged death, respawn, restoration and controls returning, then saved its
capture. Keyboard play and original-game image comparisons: **not tested**.
Workspace tests, clippy and fmt are recorded in MODLOG. M8g0a is complete;
M8g0 remains open for the other cases, original-game references, and controlled
visual time. Commands to create a new named run are in README.

#### M8h — Sky and lighting on the shared game clock

Make the sun, sky timeline, day-dependent planet/moon state, ambient light,
stars and day/night material/light strengths follow the session's clock,
instead of the start-up `--time`. Use the original data and rules; distinguish
game-time effects from effects whose original rule uses elapsed real time.
Update dependent fog, water reflections and lighting in a consistent frame
order. This replaces the unnamed "sky on the clock" gap in M7f4e; it does not
change the already-fixed Aurora countdown origin (M10g).

Implementation sequence (2026-10-11):

1. Retain the decoded sky manager/light and evaluate the existing sky rules
   from session game seconds each frame. Wrap hours in double precision;
   retain continuous days for the planet orbit. Match the manager's exact
   midnight wrap (24 to 0 before gradient evaluation).
2. Update only the marked game sun, including its shadow direction. Order
   sky evaluation after the clock and before fog, sky/stars, water reflection
   inputs and shafts; light parameters already follow fog. Keep cloud,
   twinkle, caustic and wave elapsed-time rules unchanged.
3. Test wrap, frozen time, frame-history independence and same-frame ECS
   propagation with synthetic data. Run workspace checks and real-data
   frozen/advancing-clock measurements. Record remaining comparison gaps
   explicitly; do not call the milestone complete without its checklist.

**Done when:** synthetic checks cover day wrap and multiple clock times;
logged sky/light values at fixed times equal the existing evaluation rules;
host and joiner evaluate the same values at the same session tick; cases at
day, sunset and night have recorded comparisons; affected GPU/CPU costs are
logged. Eclipse and ending overrides remain with story presentation below.

**Implementation status (2026-10-11):** shared-clock updates implemented;
full milestone acceptance remains open for original-game matched visual
comparisons and isolated CPU profiling. No eclipse/ending override added.

- [x] Synthetic day wrap, negative/multiple days, fixed-time evaluation,
  frozen state, different peer frame histories and same-frame consumer/sun
  propagation. Workspace: 405 passed, 37 opt-in tests ignored; fmt and Clippy
  with warnings denied pass.
- [x] Frozen 09:36 reports have identical sky, fog, object and terrain values
  to M8g0a (terrain cache occupancy excluded as before). Three 300-frame
  1600×900 runs, 120 settled warm-up frames, 1200 m view: means
  18.136/17.640/17.651 ms versus 17.936/17.946/18.021 ms before; p99
  21.246/19.461/22.168 ms versus 20.314/19.273/20.524 ms. These measurements
  do not establish a speedup or a 60 fps result. First new run enabled tick
  logging; the other two did not.
- [x] Recent GPU sky costs 0.047–0.053 ms (previously 0.046), fog
  0.032–0.034 ms (0.032), water surface 0.129–0.130 ms (0.127–0.128),
  shafts 0.845–0.868 ms (0.837–0.848). Full reports retain all pass samples;
  these are recent diagnostic averages, not whole-run GPU distributions.
- [x] Fixed noon/sunset/midnight at 400 m view: sky timelines 12/18/0,
  means 8.513/9.139/8.371 ms. Accelerated 240× run crossed midnight during
  measurement: game seconds 1008.989→1647.763, sky timeline
  17.453186→9.970176, mean 8.872 ms. All four reports: 300 frames, zero
  unsettled frames, 1600×900 captures. This compares numerical rules and
  records captures; original-game visual comparison is **not tested**.
- [x] Local host/join at 120×: all 82 common sampled ticks (150–4200,
  stride 50) had identical complete sky states and continuous planet days,
  including eight midnight crossings. Samples evaluate fixed ticks, not
  simultaneous render frames. Clients were manually stopped after logging:
  `--shot` is player-only and does not terminate free-camera runs.
- [ ] Matched original-game day/sunset/night visual comparisons; **not tested**.
- [ ] Isolated CPU cost of sky evaluation; **not tested**. Above CPU-facing
  measurements are whole-frame wall intervals, not isolated system timings.

Reproduce the frozen performance run with the M8g0a command in §4.5 using
new baseline names `m8h-shallows-N` and revision label `203f27a-m8h`.
For the recorded clock cases (choose a fresh NAME; existing names refuse
replacement):

```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
.\target\release\sn-client.exe --benchmark 300 --baseline NAME --baseline-revision 203f27a-m8h --warmup 120 --resolution 1600 900 --start 0 5 0 --look 60 5 60 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 12 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings --log-sky
```

Use `--time 21` for sunset, `--time 0` for midnight, or `--time 0
--time-scale 240` for a crossing. Expected: saved report/300 CSV rows/capture,
zero unsettled frames, fixed timelines 12/18/0 or a changing wrapped timeline.
Records: ignored `out/baselines/run-m8h-{shallows-1,shallows-2,shallows-3,
day,sunset,night-final,cycle-final}/`. Earlier `night`/`cycle` runs predate the
exact-midnight correction; use the `-final` results for that edge case.

Peer verification uses these two commands in separate terminals (stop both
after collecting overlapping samples):

```powershell
.\target\release\sn-client.exe --host 27420 --name m8h-host --view 400 --resolution 960 540 --free-cam --time 0 --time-scale 120 --lifepod-seed 1 --slot-seed 1 --aurora intact --log-sky --log-ticks 50
.\target\release\sn-client.exe --join 127.0.0.1:27420 --name m8h-join --view 400 --resolution 960 540 --free-cam --log-sky --log-ticks 50
```

Expected: identical text after `sky clock probe: tick=N` for each shared N;
at ticks divisible by 500, timeline/day scalar are zero and continuous days
increase. `cargo test --workspace`, `cargo clippy --workspace --all-targets
-- -D warnings`, and `cargo fmt --all -- --check` must pass without game data.

#### M4c — Terrain geometry fidelity (new explicit milestone)

M2 proved seam continuity, not equality with the original terrain. Our
surface-nets faces, normals and skirted batch LODs differ from the original
Voxeland pipeline; a switch to dual contouring alone would not prove a match.
First document the original surface construction, interpolation, smoothing,
normal generation, chunk ownership and cross-LOD boundaries, marking each
finding *confirmed* with its method or *hypothesis*. Establish a lawful,
read-only reference measurement method; no game patching or access-control
circumvention. If reference geometry cannot be measured, record that blocker
rather than asserting exactness from a recognisable screenshot.

Split implementation into surface positions/topology, normals/material face
assignment, then LOD transitions, each with its own plan and checklist. Keep
the existing mesher until evidence justifies replacing it; seek approval for
a large refactor. M4b owns original clipmap/streaming rules, while M4c owns
surface fidelity. Coordinate their chunk conventions before implementation.
M7e3's exact tuft positions follow only after the original face order/positions
and random/noise rules can be checked. The game's collision simplification
remains a separate deferred M9a gap; rerun movement regressions if terrain
changes alter collision meshes.

**Done when:**

- Representative flat, sloped, sharp, cave and boundary cases have numerical
  surface-position and normal errors against a reference, within tolerances
  defined in advance; unavailable reference measurements keep the step open.
- Synthetic topology/seam tests pass; same-LOD and cross-LOD boundaries have
  no unexplained holes, overlaps or visible skirt substitutions.
- Material boundaries and grass-support faces are checked separately from
  lighting; original chunk/face enumeration is documented for M7e3.
- Mesh cost, triangle counts, streaming latency and memory are logged; affected
  collision/movement regressions pass. Geometry parity is not inferred from
  texture decoding or screenshots alone.

#### Remaining gaps and their homes

- **Atmosphere, reflections and water:** M8b/M8c7b own cave/wreck atmosphere
  shapes, sky selection/anchors, cube reflections and `_UwePowerLoss`; M8f
  owns spatial water settings for fog, surface lighting and effect fog, plus
  the water clip map/interior exclusion and shore foam. Split volume selection,
  blending, GPU sampling and clip-map work into independently checked steps.
  M8c1/c4 retain reflection/refraction and wave comparisons; SSR follows the
  original scene/options where enabled, not an unconditional added effect.
- **Materials and compositing:** M7g5 owns triplanar rocks and world-object SIG;
  M7g4 owns remaining UBER mesh variants (brine, sand/lava falls and other
  effects); M7g6 owns the smaller shader families and IonCrystal/Mesmer extras.
  "M7g open" retains dim cones, dark pillars and the unchecked refraction y
  sign. Temperature refraction, sonar and PDA composite variants from § 4.2
  get sub-steps when their gameplay state is available. Finish by counting
  unsupported variants and explaining/fixing each, not by hiding them.
- **Post-processing:** M8d1 owns bloom/lens dirt and AO; M8d owns the exact
  grading curves, remaining enabled effects (SSR, DOF, motion blur, FXAA,
  dithering), original settings and render-pass order. The current
  Reinhard/ACES-fitted substitutions are not exact original-game curves.
- **Streaming, grass and shadows:** M4b owns original clipmap extents, cell
  activation, material budgets and shadow levels; M7e3 owns exact grass
  placement and, with M4b, its farther LOD. M8e2 owns verified light falloff;
  M8e4 owns local shadows and the custom spotlight cookie; M8c6b owns sun
  bias/filtering, receiving flags, cut-out plant shadows and animated-rig
  shadow updates as detail changes. Shadow-only drawing already exists;
  matching its result remains part of the shadow validation.
- **Animation follow-up:** add sub-steps to M7f4 for animated light/material
  values, active-state bindings, effect/light attachments to moving nodes,
  nested rigs and morph/skinning culling bounds. Script-field bindings require
  the relevant script behaviour, not a blanket attempt to execute game code.
  Each step logs applied/unsupported bindings and validates animated values,
  transforms or bounds. Root motion and remaining animation-event receivers
  belong with the moving objects/scripts that need them; legacy UI animation
  belongs with M9u. Already-working hatch events are not missing.
- **Particles and moving scenery:** M7i needs a staged reader/simulation/render
  plan for particles and `VFXController` before code (fire, smoke, explosion
  and other emitters). Check counts, lifetimes, bounds and seeded trajectories
  where applicable. M7f4h remains after M11 for the floating pod, moving
  colliders/platform and continuously attached modules. Creatures ("After
  Phase E" item 4) require spawning, motion, animation and co-op ownership;
  they are currently skipped, including slot creatures. Split by behaviour
  family rather than treating all creatures as one rendering change.
- **Player and state-dependent presentation:** M9u owns the original HUD,
  reticle, inventory/equipment panels, notifications and UI rendering; PDA,
  tools/IK, intro/damage effects, vehicles and habitats retain their "After
  Phase E" homes. Story owns Aurora explosion presentation/camera shake,
  warnings, eclipse/ending sky overrides and other state-driven scenes.
  Physics/damage and audio stay with their own subsystems. Static-world visual
  parity does not mean those parts of the full game's picture are complete.

Each existing milestone retains its own "Done when" requirements. The follow-up
groups above must receive smaller numbered steps and measurable checklists
before implementation. No untested item becomes complete merely because it
has been assigned a home.

#### M7g4a Tree Cove brine plan (2026-10-12)

Continue from `95c1883`, preserving the inventory/death-drop work. Implement
only the unlit Tree Cove lake variant: world-space triplanar scrolling and
deformation, vertex near fade, back-face rejection, ordinary opacity and
clamped colour before WBOIT. The compiled program still applies water fog
without `FX_ADDFOG`; do not infer behaviour from keyword names alone.
Keep the five existing hologram/door variants and their render-state checks.
Lit Lost River brine, normal maps and other UBER combinations remain open.

**Done when:**
- [x] Record confirmed program/property findings in `docs/formats/materials.md`.
- [x] Synthetic tests cover the new exact keyword/state selection and retained
  three-component projection/scroll vectors; unsupported lighting stays rejected.
- [x] Workspace tests, fmt, clippy and release build pass.
- [x] Run matching Tree Cove before/after benchmarks and a Precursor regression;
  record effect selection, shader validation, counts and timings in MODLOG.
- [ ] Matched original-game visual comparison: **not tested**, remains open.

**Implemented and numerically checked:** 471 workspace tests passed (39
ignored); GPU validation passes at Tree Cove and a Precursor door. The new
material replaces the Tree Cove fallback; entity/mesh/texture counts stay
equal. Final paired mean frame times: 5.281 to 6.503 ms at Tree Cove,
5.950 to 6.192 ms at the door. Tree Cove adds a 0.057 ms effects GPU pass;
the overall timing regression is recorded, not attributed solely to that
pass. Commands, counts and limitations are in MODLOG (2026-10-12 M7g4a).
Visual acceptance remains open. Lost River's apparent lit brine is addressed
by M7g4b below; its saved lighting keywords proved inactive.

#### M7g4b Lost River brine plan (2026-10-12)

Next after the implemented M7g5 and M7g4a slices: inspect the apparent lit
brine keyword sets and compiled vertex/fragment programs, then implement
only confirmed rules in the existing effect pass. Preserve Tree Cove
brine and the five hologram/door variants.
Keep extracted programs and property dumps in ignored `out/`.

**Done when:**
- [x] Document confirmed lighting selection and render state: the saved
  `FX_LIGHT_2SIDED`/`FX_LIGHT_NORMALMAP` keywords are absent from the installed
  shader's compiled keyword table. Both lake families have light mode 0 and
  select M7g4a's program, which has no normal-map sampler or lighting terms.
  The presumed need to add normal-map/two-sided lighting was incorrect.
- [x] Implement exact variant selection and required inputs; reject undecoded sets.
- [x] Synthetic tests, workspace tests, fmt, clippy and release build pass.
- [x] Matching Lost River before/after benchmarks and Tree Cove/Precursor regressions
  pass GPU validation; record counts, timings and reproduction commands.
- [ ] Matched original-game comparison: **not tested** until available.

**Implemented and numerically checked:** 472 workspace tests pass, 39 ignored.
Both Lost River lake families replace their fallback through the existing
brine shader. Six GPU runs complete with matching scene counts, zero loader
warnings and zero unsettled measurement frames. Ghost Tree mean frame time
5.939 → 6.152 ms; effects GPU pass 0.026 ms. Tree Cove and Precursor
regressions pass. Full visual acceptance remains open. Next implementation
slice: remaining M7g4 mesh-effect sets (sand/lava falls, elevator and column,
water force field and tech cones), followed by M7g6's smaller shader census.

#### M7g4c Precursor control-room effects plan (2026-10-12)

Decode the two exact UV-space deformation sets without `FX_SOFTEDGES`,
used by the Gun deactivation column's force field and its terminal screens.
Compare their installed programs with the existing door effect. Reuse the
current shader only if colour, opacity, scrolling, deformation and optional
refraction rules agree; soft fading must stay off. Preserve all existing sets.
Rigid effect meshes must retain their animated-node parent, as ordinary
object meshes do; the first GPU run exposed a missing attachment.
Keep program/property dumps under ignored `out/`.

**Done when:**
- [x] Document confirmed formulas, render state and defaults.
- [x] Add exact selection with synthetic tests for deformation independently
  of soft fading, optional refraction and rejected sets/states.
- [x] Workspace tests, fmt, clippy and release build pass.
- [x] Matched control-room before/after GPU benchmarks and a door regression
  complete; log counts, shader validation, timings and exact commands.
- [ ] Original-game visual comparison: **not tested** until available.

**Implemented and numerically checked:** 473 workspace tests pass, 39 ignored.
The column force field and two terminal material instances replace their
fallbacks. Rigid effect parts keep their rig reference through loading and
spawn under their animated node; five attachments logged in the control room.
Final GPU pairs have zero errors, warnings or unsettled frames. Mesh/material/
texture counts stay equal; tracked top-level handles decrease by four because
existing effects become rig-owned children. Mean control-room frame time
17.223 → 16.841 ms; door 8.049 → 8.072 ms. Another client was running on the
same GPU, so no isolated performance or speedup claim. Release built in
`out/m7g4c/target` because the user's running client locked the default output.
Commands and limitations are in MODLOG. The column's back-culled inner layer,
elevator, tech cones and other M7g4 sets remain open; terminal scripted colour
activation and original-game imagery are **not tested**.

#### M7v1 alien surface visibility investigation (2026-10-12)

User reports walls visible from only one side across large alien structures;
this is visual disappearance, not blocked movement. Inspect effective shader
culling state (fixed or material property/default), mirrored transforms and
camera-layer filtering. Preserve the game's intended sidedness; do not
globally make every surface double-sided or alter collision.
The installed-data audit found reflected Precursor wall transforms. Reuse
cached mesh variants with triangle order reversed for negative world
determinants, after transform propagation, retaining attributes and restoring
the original when the sign changes. The custom brine pass compensates its
front-facing test separately. Keep the existing mesh/material pipelines.

**Done when:**
- [x] Record confirmed cause(s) and remaining hypotheses in format docs.
- [x] Preserve Off/Front/Back culling and shader defaults, with synthetic tests.
- [x] Audit negative-scale placement/rig handling; fix only confirmed mismatches.
- [x] Workspace tests, fmt, clippy and release build pass.
- [x] GPU runs in multiple alien structures compile the affected paths;
  log counts, effective culling and exact reproduction commands.
- [ ] User visual check of the reported walls: **not tested** until confirmed.

**Implemented and numerically checked:** 477 workspace tests pass, 39 ignored.
Gun and prison aquarium GPU runs compensate 4,387 and 1,518 reflected
object mesh instances with zero unsupported meshes. Scene/terrain/material/
texture counts match before/after; Tree Cove brine regression passes. Shader
Off/Front/Back selection and restored original mesh handles have synthetic
coverage. Single paired mean frame times: Gun 18.975 → 18.203 ms, aquarium
14.390 → 12.967 ms, Cove 8.603 → 8.858 ms; no speedup claim. Mesh variants
cost memory (Gun peak sampled process memory 1.854 → 2.001 GiB). Exact
commands and limits are in MODLOG. User confirmation that these fixes cover
all reported disappearing surfaces remains open.

**Crash follow-up (2026-10-12):** the user hit `ExtractedToRenderWorld`
after about 73 s while streaming from Tree Cove. The short settled GPU
runs above missed this delayed lifetime case. Keep original object mesh
CPU data in `MAIN_WORLD` for later reflected instances, use fallible index
access for unavailable data, and cover GPU extraction before a delayed
reflection in a synthetic test. Verify with workspace checks and a long
Tree Cove flythrough that loads additional batches. Record the CPU-memory
cost and exact results in MODLOG before claiming the crash fixed.

**Crash correction verified:** 479 workspace tests pass, 39 ignored; fmt,
clippy and release build pass. A Tree Cove flythrough streamed for 167 s
(170 s session), 15,051 measured frames, settled at its destination and
exited 0, with no panic/GPU errors and zero unsupported meshes throughout.
The variant cache grew from the initial area's 244 to 251. Peak sampled
process memory was 1.73 GiB along this route; no isolated memory delta
is established. User interactive replay and visual acceptance remain open.

#### M7g4e lit UBER plan (2026-10-13)

Port the remaining placed sand falls, Lava Castle smoke, peeper-pipe
bubbles and Precursor entrance water fields. Start with their input
contract and numerical reference math, then connect the effect pass. Do
not select a lit set merely because its keywords have been recognized.

1. Decode exact installed vertex/fragment pairs (119/437 bubbles,
   142/460 smoke, 148/466 water field, 154/472 sand). Separate inactive
   saved keywords from compiled terms. Record material coefficients,
   stage boundaries, four-light inputs, sky SH/exposure, underwater
   light attenuation and alpha/fog order in `docs/formats/materials.md`.
2. Add exact-set input diagnostics and synthetic numerical checks before
   enabling rendering. Preserve stand-ins until all required inputs have
   a source; do not silently reuse opaque clustered lighting or ordinary
   Lambert diffuse. This first checkpoint is M7g4e1.
3. Feed per-renderer four-light data, the selected sky and frame water/day
   inputs to the existing effect pass. Port the shared lighting formula
   and preserve vertex versus pixel evaluation and vertex attenuation.
   Finish the smoke/triplanar terms and neutral refraction defaults.
4. Validate all four families at their placed locations, plus unlit
   brine, door and key-terminal regressions. Record timings/counts and
   exact commands. Original-game comparisons remain a human acceptance
   item. No new dependencies or unrelated animation/terrain changes.

**Done when:**
- [x] Exact compiled mappings and input/arithmetic findings documented.
- [x] Synthetic tests cover keyword mapping, scalar defaults, wrap,
  desaturation, self illumination, conditional SH and preserved alpha.
- [ ] Required renderer/sky/light inputs reach both shader stages.
- [ ] All four materials select decoded effects with no unsupported terms.
- [ ] Workspace tests, fmt, clippy, release build and GPU regressions pass.
- [ ] Original-game visual comparison (**not tested** until available).

**M7g4e1 checked:** exact-set diagnostics report the program/stage and five
lighting coefficients while retaining stand-ins. Four synthetic tests
exercise contracts and pixel-lighting reference terms. Workspace checks
passed (592 tests, 50 ignored on the current repository); four placed-scene
GPU runs confirm the expected diagnostics without errors. This is the input
checkpoint, not completed GPU lighting. Next: per-renderer four-light/sky
feeding, neutral refraction and smoke vertex noise, then exact-set rendering.

**M7g4e2 plan:** connect frame lights and selected sky to effects, then
enable only entrance water-field program 148/466. Keep the other three
families gated. Store original game-unit light colours, evaluate direction/
range/spot attenuation per vertex, preserve pixel-stage normal bias and
lighting/SH order. Use the existing camera water approximation. Four-light
ranking will use a deterministic brightness-at-renderer estimate; Unity's
exact engine ranking remains unconfirmed and must be stated as a limitation.
Done when synthetic selection/math tests and field GPU validation pass,
unlit/terminal regressions pass, workspace/fmt/clippy/release pass, and
commands/counts are recorded. Human comparison remains open.

**M7g4e2 implemented and automatically checked; manual visual testing not tested:** the entrance-field pixel variant receives
four lights, selected sky and frame water/ambient inputs. Lit/unlit pipelines
are specialized. Workspace checks passed (594 tests, 50 ignored), and five
matched GPU pairs passed, including three field sites and brine/terminal
regressions. Original image parity, exact native ranking/lookup and original
SH orientation remain open; full details/commands in MODLOG. The other three
families remain gated. Next: sand's projected pixel program, then smoke noise
and bubble vertex lighting/neutral refraction.

#### M7g4d unlit UBER remainder plan (2026-10-13)

Resumes the visual track after M9u4d. A whole-game census (`sn-inspect
prefab --materials`) lists 32 `UWE/Particles/UBER` materials on placed
prefabs. This slice takes the **unlit** ones still on the stand-in look.
Each program below was read from the installed shader's D3D11 dump (ignored
`out/m7g4-brine/fresh-programs`). Each is a combination of terms we have
already decoded, plus one new term:

| Material (placements) | Keyword set | Program (vertex/fragment) | What it adds |
|---|---|---|---|
| `x_TechLight_Cone` (7) | `FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT` | 67/385 | halo chain without `_MainTex2`, door soft edges, brine near-clip vertex fade |
| `x_Gun_Elevator_TubeSmk` (1) | `FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_NEARCLIP FX_SCROLL FX_SOFTEDGES WBOIT` | 68/386 | UV deformation; no `FX_ADDFOG`: ordinary alpha, colour saturated after fog (as brine) |
| `x_Precursor_ThermalReactor_TowerElec` (8) | `FX_ADDFOG FX_DEFORM FX_FRESNELCLIP FX_MULMAP FX_SCROLL WBOIT` | 57/375 | M7g4c's control-room program plus the Fresnel term |
| `x_LavaFall` (11) | `FX_ADDFOG FX_ALPHAPOW FX_DEFORM FX_MULMAP FX_SCROLL FX_SOFTEDGES FX_TRIPLANAR WBOIT` | 34/352 | brine's projection, no near clip, brightness opacity; **new:** `FX_ALPHAPOW` |
| `x_Gun_Elevator_TubeDots`, `…InnerBlock`, `…InnerBlock2` (3) | already decoded sets | as M7g4/M7g4c | `_MyCullVariable` 2 (back culling) |
| `Precursor_Symbol_Orange` | `FX_ADDFOG WBOIT` | 5/323 | one texture without scrolling; stored speeds are unread |

`FX_ALPHAPOW`: the starting alpha (soft edge × vertex alpha × 2 ×
`_Color.a`) is raised to `_AlphaPow` with `log`/`exp`, before any texture.
All of these request the blend/depth state the pass already draws; culling is
pass state, so back culling applies to any confirmed set, not only brine.

**Design:** replace the `brine` switch by the keywords' own terms
(`FX_TRIPLANAR`, `FX_NEARCLIP`, `FX_ADDFOG`, `FX_ALPHAPOW`) plus a back-cull
flag, so every confirmed program is the same shader with terms on or off.
Selection stays an exact list of confirmed keyword sets; anything else is
rejected and logged as now. No new dependencies, no architecture change.

**Not in this slice:** the lit sets (`FX_LIGHTMODE_PIXEL`/`_VERTEX`: sand
falls 120, Lava Castle smoke 21, peeper bubbles 14, water force field 8),
which need the game's light inputs decoded first (M7g4e), and the
flashlight cone (no `WBOIT`, layer 8, the player's tool).

**Done when:**
- [x] Confirmed program terms and render states in `docs/formats/materials.md`.
- [x] Synthetic tests: exact selection of the new sets, flags per keyword,
  back culling accepted for any confirmed set, front culling and
  undecoded sets rejected, alpha power in the reference formula.
- [x] Workspace tests, fmt, clippy and release build pass.
- [x] GPU runs at a tech light wreck, the Gun elevator/control room, the
  thermal reactor and a lava fall: the new materials log as effect
  materials, no shader validation errors, counts and timings in MODLOG,
  with a Tree Cove brine and Precursor door regression.
- [ ] Original-game visual comparison: **not tested** until available.

**Implemented and numerically checked (2026-10-13):** five added exact
sets, including the non-scrolling orange symbol, and general back-cull
pass-state handling. Workspace checks passed (560 tests, 47 ignored).
Seven matched before/after GPU pairs exited 0, each with 300 settled
measurement frames and zero warnings, shader errors or panics. Scene
geometry counts match; commands, counts and timings are in MODLOG.
**Human test failed (2026-10-13):** the Blood Kelp cache screenshot shows
the symbol detached from the terminal body. The shader checks above did
not verify terminal startup material selection or optimized-avatar skinning.
The purple terminal's animator has `has_transform_hierarchy = false`,
12 avatar skeleton nodes and 0/240 bound clip properties; its renderer has
no serialized bones/root bone. `PrecursorKeyTerminal.Start` selects
`keyMats[acceptKeyType]` for `keyFace`, replacing the saved orange material.
Do not call terminal symbols visually ready until these are handled.

**Correction approved (2026-10-13; checkpoint `ff4ef2d` pushed):**
1. Decode the terminal script's stored material references and key type;
   apply its startup material selection through the existing material loader.
2. Extend the existing avatar/rig loader to reconstruct optimized skeleton
   transforms, map mesh bone identities to the avatar, bind clip properties
   and skin the terminal in its default closed pose. Keep ordinary rigs on
   their existing path; no hard-coded symbol offsets or game geometry.
3. Add synthetic coverage for optimized binding/skinning and material
   selection. Numerically verify terminal body and symbol transforms at
   the cache, then request a fresh human comparison at the same location.

**Done when:** the required material is selected, all required optimized
bindings resolve, body vertices use the reconstructed skin palette, default
closed pose is applied, workspace/GPU regressions pass, and the user confirms
the symbol/body placement.

**Implemented; human confirmation pending:** the loader reads the terminal
startup fields, selects its required material, restores its optimized avatar
paths/default locals and maps the mesh palette from bone-name hashes. Scope
is key terminals; ordinary rigs keep their current path. All three terminal
templates resolve 240/240 clip bindings and seven skin bones and use distinct
materials. Synthetic malformed-data and reference-remapping tests passed.
Numerical closed-pose and GPU validation are recorded in MODLOG. Original
game comparison and a fresh user screenshot remain open; insertion gameplay
is outside this correction.

After this correction, next visual slice: M7g4e,
decode actual light inputs for the remaining lit particle variants before
enabling their programs. Shadow rendering remains a separate planned
lighting task; this unlit slice does not implement it.

#### M7g5 implementation plan (2026-10-11)

1. Decode the installed `UWE/SIG` and `UWE/SIG Triplanar with Capping`
   deferred variants and declared defaults. Record their surface rules in
   `docs/formats/materials.md`; keep all bytecode/disassembly in `out/`.
2. Add explicit material selection by shader name, retain the required
   textures and defaults, and extend the existing object surface shader.
   Share only confirmed projection/normal/border math with terrain; object
   capping has different specular rules and no terrain-layer alpha blend.
3. Cover keyword/default/texture selection with synthetic tests. Log loaded
   SIG/capping counts and missing albedo slots. Compile both real GPU paths,
   run workspace checks, and compare three matching performance runs.

**Done when:** both families use their confirmed albedo/normal/specular/gloss
and emission rules; no loaded capping material silently falls back to white
because `_MainTex` is absent; synthetic tests and GPU validation pass; counts,
timings and exact reproduction commands are recorded. Matched original-game
rock/coral captures remain a separate open acceptance item until supplied.
No architecture change, unrelated material variants or terrain mesher work.

**Implemented and checked (2026-10-11):** the object material loader selects
the two exact shader names and retains their texture inputs, keyword and
declared scalar/colour defaults. Plain SIG now supplies specular, gloss and
emission; capped objects use cap/side textures, projected normals, inner tint
and the compiled specular branch. The capping shader's unused SIG slots do
not generate glow. Shared terrain math moved unchanged into `triplanar.wgsl`.
Sky changes cannot reclassify these materials as Marmoset.

- [x] Four synthetic tests cover exact shader selection, keyword gating,
  texture slots/colour spaces, defaults/overrides, a missing SIG map and
  Marmoset sky-switch isolation. `cargo test --workspace`: 464 passed,
  39 ignored. Clippy with warnings denied, fmt and release build pass.
- [x] Three alternating before/after runs against current committed control
  `86ee42a`, at 1600×900, 1200 m view, 120 settled warm-up frames and 300
  measured frames: all completed with no shader errors, zero unsettled
  frames and matching terrain/object/light/sky/fog counts. Loaded materials:
  12 plain SIG, 5 capped, 7 SIG-enabled; zero missing albedo slots. Texture
  uploads increase from 2242 to 2265, as expected for the added maps.
- [x] Before mean times: 18.912/19.638/19.878 ms; after:
  20.422/19.183/18.819 ms. Before p99: 25.346/25.955/26.153 ms;
  after: 30.321/26.685/22.458 ms. Recent opaque GPU pass averages:
  1.610–1.684 ms before, 1.689–1.824 ms after. The distributions overlap;
  neither a speedup nor meeting the 16.7 ms target is established.
- [ ] User-supplied matched original-game rock/coral comparisons:
  **not tested**. Full M7g5 visual acceptance remains open.
- [x] Normal player smoke with a running 120× clock: death, restoration and
  controls-return logs, then timed capture and clean exit. Command:
  `sn-client --view 400 --resolution 960 540 --time-scale 120 --lifepod-seed 1
  --slot-seed 1 --kill 3 --shot 10 m7g5-player`; expected `Restored`,
  `ControlsBack` and `out/m7g5-player.png`. Keyboard play: **not tested**.

Reproduce after `cargo build -p sn-client --release` with a fresh baseline
name per run (existing names are deliberately rejected):

```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
.\target\release\sn-client.exe --benchmark 300 --baseline m7g5-check-1 --baseline-revision 86ee42a-m7g5 --warmup 120 --resolution 1600 900 --start 0 -10 0 --look 60 -40 60 --fov 60 --view 1200 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expected: `SIG materials: 12 plain, 5 capped, 7 with SIG keyword; 0 missing
albedo slots`, no shader errors, saved report/300 CSV samples/1600×900 capture.
Evidence is under ignored `out/baselines/run-m7g5-{before,after}-{1,2,3}/`;
`out/m7g5/build-provenance.txt` records the archived control commit and binary
hashes. Mistyped caller revision labels were corrected to the archived commit;
the original labels remain in the reports and timing samples are unchanged.
The initial `m7g5-shallows-1` smoke capture predates the sky-switch guard and
had elevated unrelated GPU costs; it is not the final comparison set.

Verification commands: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, and `git diff --check`; expect success.

#### M8c7b1 — Streamed atmosphere volumes for object sky selection (2026-10-14)

Read the installed AtmosphereVolume fields using the existing checked script
layout reader and its attached collider. Retain active, enabled colliders
that affect visuals; support boxes, spheres and capsules. Preserve the full
prefab transform chain, reject unsupported/sheared box shapes explicitly,
and keep volume lifetimes tied to the existing object streaming instances,
including objects with no drawable mesh. Query enclosing volumes before the
batch/map biome fallback at the owning Auto SkyApplier's position. Reassign ordinary and effect
material sky bindings when volumes stream in/out; do not modify shared sky
materials in ways that relight objects outside the volume.

The installed point query chooses the greatest priority, breaking ties by
runtime instance ID. Use an explicit local lifetime tie key and log
overlapping equal-priority different-biome selections; native instance-ID
parity remains unverified. This slice does not implement fog-volume
rasterization, custom/fixed anchors, reflections or AtmosphereDirector fades.
Moving applier/volume transforms and script-driven activation remain animation
follow-ups; this slice records their static placement at spawn.

Done when: synthetic primitive/boundary/priority/tie/fallback and streaming
rebinding tests pass; an opt-in installed-prefab audit checks all placed
volume families and records supported/unsupported shapes; outdoor/interior
GPU runs record selected skies, volume counts and timings with no new shader
errors; workspace tests, clippy, fmt and diff checks pass. Matched original-game
visual comparison remains explicitly not tested until a reference is available.

**Implemented and verified (2026-10-14):** all checklist implementation checks
passed. Workspace: 637 passed, 0 failed, 57 ignored; clippy, fmt and diff checks
pass. The installed-data audit checked 1,369 placed prefab families, finding
151 visual-volume families and 2,409 supported placements (1,425 boxes,
984 spheres, no capsules), with no rejected transforms. The terminal regression
also checks the reader after hidden avatar bones have been reconstructed.

Three final GPU runs per case (960×540, 400 m view, 60 settled warm-up frames,
120 measured frames) exit successfully with zero loader warnings and zero
unsettled measurement frames. The Shallows case has 132 active volumes,
461 enclosed query positions and no different-biome equal-priority ambiguities;
the thermal-room case has 67 active volumes, 410 enclosed positions and 18
ambiguities. Cave and Precursor-interior sky materials are selected. Terrain,
loaded mesh/texture counts and light counts match the corresponding before
cases; tracked entity counts increase for volume metadata. Final mean frame
times are 10.22–11.48 ms outdoors and 6.91–7.38 ms indoors. Before runs and
the full provenance/performance limitations are in MODLOG; no speedup or
full-resolution 60 fps claim is made. Captures/reports stay in ignored out/.

M8c7b1's implementation slice is complete. Full M8c7b and 1:1 visual acceptance
remain open: native overlap tie order, moving appliers/volumes, script activation,
fog rasterization/fades, other anchors, cube reflections and power state.
Matched original-game visual comparison: **not tested**.

#### Proposed order for visual work

1. M8g0 baselines, then M8h shared-clock sky/light updates.
2. M7g5's obvious material failures, then remaining M7g4/M7g6 variants.
3. Atmosphere/sky selection, spatial water settings, cube reflections and
   water exclusion, in separate M8b/M8c7b/M8f slices.
4. M8d1 bloom/lens dirt and AO, then exact grading and the rest of M8d.
5. M4b streaming and M4c terrain fidelity, then M7e3 grass; M8e2/M8e4/M8c6b
   light/shadow fidelity once activation and shadow ranges are correct.
6. Animation follow-ups and M7i particles; creatures with ownership; M7f4h
   after M11 as already agreed.
7. Original UI, tools, intro and other state-dependent presentation as their
   gameplay rules become available.
8. M8g final calibration over the full case set. Compare at every earlier
   step too: final calibration is not a reason to defer discovering errors.

For each implementation slice, run appropriate synthetic tests and opt-in
real-data checks, record exact commands/results in MODLOG, compare the affected
baseline cases and measure performance. Keep parser/algorithm work headless
and rendering at the existing engine edge. A 1:1 claim requires every relevant
difference resolved and every checklist run; neither ported formulas nor
similar-looking captures alone establish pixel-for-pixel equality.

### Phase D — Multiplayer

| # | Goal | Done when |
|---|---|---|
| **M10** | Co-op players (plan below, steps M10a–M10i): `sn-protocol`, `sn-net`, `sn-session`, `sn-server`; client–server with "host a game" in the client; handshake (protocol, game build, rules fingerprint), join, one shared clock and world settings, player state sync per physics tick over UDP (position, velocity, look, flags and the animator values M9g's rules computed on the owner's machine), remote bodies, cinematics and deaths; solo play through an in-process server. | Each step's own list in the plan; in the end two clients on one machine see each other's animated bodies with the logged error targets met under injected loss and latency, and the user plays with a friend over the internet. |
| **M11** | Nitrox-style entity sync: simulation ownership + handoff, server persistence, pick up / drop an item. Then M7f4h, the floating lifepod, as the first owned rigid body. | Item state survives server restart; ownership handoff covered by headless tests. |
| **M12** | **Playable multiplayer desktop demo:** two+ players spawn in Safe Shallows, swim the textured, populated seabed, see each other, pick up items, chat. | A friend on another machine joins over the internet using their own install. |

Beyond M12 (unordered): creatures + AI, inventory/crafting, PDA, vehicles,
habitat building, day/night, save import, audio, mod support, maybe a web port.

#### M10 plan: co-op multiplayer (written 2026-10-11, approved by the user the same day with the recommended choices: § 9, items 4–8)

**Goal of M10:** two to eight friends play in one world, each with their
own install. M10 is about *players*: joining, seeing each other's bodies
swim, walk, animate, use the lifepod's hatches and die, under one shared
clock and one shared world. Items, inventory and saves (M9d–M9f) and
creatures and other owned bodies (M11) are built on top of it and are not
part of M10.

##### Peer-to-peer or client–server? (analysis for the user)

**First, what "perfectly synced" can mean.** It is three different
promises; no architecture can give a fourth ("everyone sees the present
of everyone"):
1. **The same rules on every machine.** Every number in our rules comes
   from the player's own install, so two different game builds would be
   two different games. Checked once, at join: game build number and a
   *rules fingerprint* (a hash of every value `sn-sim` reads from the
   install: player, vitals, look and body parameters, TechData, the
   cinematic clips, …).
2. **The same shared state, in the same order.** Who joined, which hatch
   was used, when the Aurora exploded, later who picked up which item:
   identical on every machine, applied in the same order, at the same
   tick. This part has to be *perfect*, and it is only perfect if exactly
   one machine decides the order.
3. **The same history of every moving thing.** Player B sees player A at
   exactly the positions and animations A had, tick by tick, but *later*:
   one network trip plus a small buffer (typically 50–150 ms). Seeing the
   present of another machine is physically impossible (the signal needs
   time to arrive); every architecture, P2P included, shows the past.
   P2P can only save one hop in some cases (below).

**The options:**

| | **Lockstep P2P** (RTS-style: every peer simulates everything from everyone's inputs) | **Rollback P2P** (GGPO-style: predict, re-simulate on late input) | **P2P with split ownership** (each peer decides for its own things, no server) | **Client–server, one authority + owners** (Nitrox-style; the server can run inside the host's game) |
|---|---|---|---|---|
| Shared state (promise 2) | Identical only if every peer's simulation is bit-identical forever | As lockstep, plus visible corrections | **No single order**: two players grabbing the same item at once need an arbiter, which is a server again | **Identical by construction**: one machine orders every event |
| Needs a bit-identical simulation on every PC | **Yes**, and ours is not: collision is loaded asynchronously on a worker thread around *your* player only; `sin`/`cos`/`acos`/`atan2` (look, slerp, body) are not guaranteed bit-equal across CPUs, OSes and compilers; one difference = a permanent, silent desync | Yes, the same | No | No (only for checks, by hash) |
| Your own movement | Delayed by the slowest peer's ping (the game feels laggy) | Instant, but remote players jump back on corrections | Instant | **Instant** (your client simulates you, as in single player) |
| Big open world | Each peer loads and simulates the area around **every** player | The same, re-simulated several ticks per frame | Fine | Fine: each client simulates only what is near it |
| Joining late | Needs a full state transfer anyway | The same | From whom? | The server sends the state |
| Saves | Every peer has a copy; which one is right? | The same | The same | One save, on the server (M9f, M11) |
| Hosting, NAT | Every peer must reach every peer (n·(n−1) links); NAT hole punching needs a rendezvous (STUN/relay) service | The same | The same | One open UDP port on the host; everyone else connects out |
| AGENTS.md rule 5 ("only through our own self-hosted server") | Needs a rule change, and a third-party rendezvous service would break it | The same | The same | **Fits as written** |
| Work | Medium, then endless desync hunting | Large | Medium, plus an arbiter | Medium; Nitrox shows the model works for this exact game |

**P2P's one real advantage** is latency between two players who are both
not the host: in client–server A → host → B is two hops, in P2P A → B is
one. For co-op without player-versus-player combat this is a few tens of
milliseconds more delay in *seeing* the other player; it never touches
your own controls. The host's own player has no network delay at all.

**Recommendation: client–server, with "host a game" in the client.**
From the players' side this *feels* like P2P: one friend hosts (our
server runs inside their game, on its own thread), the others join by
address, and no separate server machine is needed. A dedicated headless
`sn-server` exists too, for an always-on world. Solo play runs the same
in-process server without a network (§ 3.1, principle 5), so there is one
code path to test. It is the only option that gives promise 2 by
construction, and promise 3 is as good as physics allows.

##### Who decides what

| State | Decided by | Reaches others as | Checked by |
|---|---|---|---|
| World settings: slot seed, Lifepod 5's point, the Aurora's explosion draw, the clock's start, the time scale | Server (its flags, its own install) | `Welcome` at join: **values, not seeds**, wherever the result is small (the pod's point is 3 numbers, so no client re-derives it) | — |
| Big seeded results (the slot fill: thousands of objects) | Each client, from the server's seed (the fill is per slot, `SlotRng(seed, placeholder, slot)`, independent of load order) | The seed in `Welcome` | Each client reports a hash of its fill per loaded batch; the server compares (0 mismatches expected) |
| Game clock (tick; `DayNightCycle` time) | Server | The tick in every packet; clients sync to it | Clock offset and drift logged per client |
| Clock-derived events: sky, the Aurora's countdown, explosion and swap, the pod's lighting states | Each client, from the shared clock and the `Welcome` values | — | Every client logs each event's tick; the ticks must be equal |
| A player's movement, look, body, vitals, cinematics | That player's client (the **owner**) | Player states (unreliable, every tick) and player events (reliable) | The server logs implausible moves (speed above the read maxima); no rejection in M10 |
| Shared objects (in M10 only the lifepod's hatch animator; later doors, items) | Server orders requests | Reliable world events, tick-stamped | World-state hash |
| Join, leave, player ids, names | Server | Reliable | — |

**Why the owner decides its own movement:** swimming then feels exactly
like single player (no input delay, no corrections), and the server needs
no collision world. Cheating is not a concern among friends on a
self-hosted server; revisit if anyone wants public servers. (Nitrox does
the same: *confirmed* M10c, read in its source: its server's
`PlayerMovementProcessor` stores the position the client sent and relays
it to the others without a check.)

##### The timeline: ticks

- **One tick = one physics step of the game** (`Time.fixedDeltaTime`,
  0.02 s, read from the install as `PlayerParams::fixed_dt`). The server
  counts ticks from the world's start; every state and every event
  carries the tick it happened in. Ticks, not wall-clock times, are the
  shared time base.
- **Clock sync:** each client estimates the server's tick from ping/pong
  pairs (offset and round trip, filtered over the best recent samples).
  Its physics loop then steps *on the server's tick numbers*. Drift is
  corrected by slewing the step accumulator by at most 1 %; a jump only
  when the error is above 0.25 s (logged).
- **Remote players are drawn at `estimated tick − delay`.** The delay
  starts at 100 ms (5 ticks) and adapts to the measured jitter. Between
  two received ticks: interpolation. Missing ticks: cubic interpolation
  with the received velocities. Past the newest received tick:
  extrapolation for at most 250 ms, then hold. Every interpolated,
  extrapolated and held frame is counted.
- With no packet loss, B's copy of A at tick T is **exactly** A's state
  at T after quantization. That is a test, not a hope ("How we prove it",
  below).

##### What a player sends

**A state every tick** (46 bytes, measured in M10a; the exact layout is
in `docs/formats/protocol.md`, our own format): the tick; position
(3 × `f32`, better than 0.25 mm at 2 km from the origin); velocity
(3 × `i16`, cm/s); look yaw and pitch (2 × `u16`); the player transform's
rotation (only while a cinematic or its ease-out turns it; smallest-three
quaternion); flags (motor, swimming, grounded, walk-grounded, in pod,
cinematic, controls on, alive); and **the animator values M9g's rules
computed on the owner's machine** (`move_speed`, `move_speed_x/y/z`,
`view_pitch`, `view_turn`, `is_underwater`, `on_surface`, `diving`, …;
floats as `i16`, bools as bits).

Sending the computed animator values, not only the inputs to M9g's rules,
makes the remote body move with the owner's own values: no second
evaluation that could differ, and no collision needed around remote
players for the `diving` ray. One-shot animator triggers (death, …) are
events.

**Packets:** one every 2 ticks (25 per second) carrying the last 6
states, so every state travels three times (changed in M10d from 4
states and two copies: with two, the sync oracle missed 0.1 % of ticks at
1 % loss), on an **unordered** channel (also M10d: states carry their
ticks, so a message overtaken by a newer one still has ticks to give; on
the latest-only channel the "bad" preset lost 10 % of ticks that way).
Measured upload about 8.0–8.1 KB/s per player including UDP/IP headers,
pings and acks (M10d); with 8 players about 57 KB/s down per client. Every packet stays under 1,200
bytes (no IP fragmentation).

**Events, reliable and ordered, each with its tick:** `Joined`, `Ready`
(the client has loaded and spawned), `Left`; `CinematicStarted {hatch,
clip, tick}`, `CinematicEnded`; `Died {kind}`, `Respawned`;
`AnimatorTrigger {id}`; and world events, of which M10 has one: a trigger
on the lifepod's animator (its hatch doors), applied by every client in
the server's order at its tick.

**Remote bodies:** M9g's body (the same model, suit and animator) for
each remote player, posed from the received state and animated by the
received values; the head is drawn normally (shadows-only is only for
your own first-person view). A cinematic is played from its start tick
(the clip is the same on every machine), so the remote body goes through
the hatch as the owner's did.

**Two players, one hatch:** no lock in M10. Each player's cinematic is
their own; the pod's door animator gets both triggers in the server's
order. If the data shows the pod's animator cannot play two at once, a
lock is added and this plan updated.

**Lifepod 5 is shared:** every player starts and respawns in the one pod
at the server's point (as Nitrox: *confirmed* M10c, its `EscapePodManager`
puts up to 50 players in one pod and makes a new one only when it is
full).

##### Crates (created in the step that needs them)

- **`sn-protocol`** (layer 2, pure): ids (`PlayerId`, `Tick`), every
  message, our own small versioned binary codec (varints, quantization),
  `PROTOCOL_VERSION`. Bytes in, data out; malformed input is an error
  with its byte offset, never a panic.
- **`sn-net`** (layer 3): a `Transport` trait with two implementations,
  UDP (`std::net`) and an in-memory link with seeded impairments (delay,
  jitter, loss, duplication, reordering) for tests. On top: the
  connection (handshake, keep-alive, 5 s timeout), sequence numbers and
  acks, a reliable-ordered channel (events), a latest-only channel
  (pings) and an unordered one (states; added in M10d), the round-trip
  estimate, byte and packet counters.
- **`sn-session`** (layer 2, pure): the multiplayer logic as state
  machines without sockets, files or threads, so it runs headless, in
  tests, with dozens of simulated clients:
  - `server`: join checks, ids, the clock, settings, relaying states,
    ordering events, the world-state hash, timeouts;
  - `client`: clock sync, a tick buffer per remote player, interpolation
    and extrapolation, applying events in order.
- **`apps/sn-server`**: `sn-session::server` + `sn-net`'s UDP +
  `sn-install` (reads the host's own install: build number, rules
  fingerprint, the pod's point from the seed). It never sends game data
  (rule 5): only ids, numbers, seeds and hashes.
- **`sn-client`**: embeds `sn-session::server` for solo play and
  `--host`; `--join ADDRESS` connects to another one.
- **`sn-inspect coop`**: headless bot clients (the real `sn-sim` player
  with real collision, scripted like `walk` and `dive`) against a real
  server. This is the main evidence, since the agent can't see the
  screen.

**Not used:** `bevy_replicon` and `lightyear` (§ 3.3 said "evaluate at
M10"): they tie replication to Bevy's ECS, and the server and the tests
must stay Bevy-free (layering rule). Async runtimes (`tokio`): not needed
for one UDP socket and up to eight peers; a plain thread does it.

##### Steps

Each step ends with numbers and a MODLOG entry; nothing is committed or
pushed without the user's OK. Every step can be reverted on its own.
The decisions it needs are taken (§ 9, 4–8); **stop and ask** if a step's evidence contradicts one.

| Step | Work | Done when |
|---|---|---|
| **M10a** ✅ | **Done 2026-10-11** (MODLOG; format in `docs/formats/protocol.md`): 15 tests; a state is 46 B (54 B with a rotation), 4 states 188 B, the largest `Welcome` 862 B; 1,000,000 bad inputs, 0 panics, and every one that decodes re-encodes to the same bytes; position error ≤ 2⁻¹³ m below 4,096 m. Also in the crate: the animator tables (a value's index is its wire slot) and FNV-1a 64 for the hashes. Plan as written: `sn-protocol`: ids, `Tick`, the messages above, our codec, quantization helpers; `docs/formats/protocol.md` (our own format, every field). | Round-trip unit tests of every message; quantization error bounds tested (position ≤ 0.25 mm at 2 km); 1,000,000 seeded random, truncated and bit-flipped inputs decode to `Ok` or `Err`, no panic; the encoded size of a player state logged. |
| **M10b** ✅ | **Done 2026-10-11** (MODLOG; packets in `docs/formats/protocol.md` § Packets): 19 tests. On every preset every reliable message arrived exactly once in order and no latest-only message went back; round trip within 10 % (lan 0 %, internet −1.8 / −3.1 %, bad −3.2 … +5.9 % over 4 seeds); a cut link times out 5.000–5.001 s after the last packet; UDP on 127.0.0.1: 500 messages in 8 ms, 0 resent. Added on the way: at most 32 packets with reliable frames in flight (the ack window), which took resends without loss from 80 to 0 (lan) and 275 to 0 (UDP). Plan as written: `sn-net`: `Transport`, the in-memory impaired link, the connection layer and both channels; the UDP transport. | Headless tests on the impaired link with the presets **lan** (1 ms), **internet** (40 ± 10 ms, 1 % loss) and **bad** (150 ± 50 ms, 5 % loss, 1 % duplicates, reordering): the reliable channel delivers every message exactly once, in order; the latest-only channel never delivers an older state after a newer one; timeouts fire at 5 s; the round-trip estimate is within 10 % of the configured delay; a UDP test on 127.0.0.1 passes. Counters logged per preset. |
| **M10c** ✅ | **Done 2026-10-11** (MODLOG; the world state and its hash in `docs/formats/protocol.md` § World state): 10 tests. Every reject reason with its message; 8 join with ids 0–7, the 9th refused; a goodbye, a timeout, a rejoin by name gets its old id; 10,000 ticks of random play (70 joins, 65 leaves, up to 8 at once, 734 events): every client's log equals the server's from its join on, its hash equal to the server's at every tick (65,525 comparisons), 560 hash reports, 0 mismatches; a server that skips one client once is caught. Protocol 2: `WorldHash` names an event count. **From Nitrox** (read, not copied): movement is the client's and relayed unchecked, players share one pod (both plan hypotheses confirmed); its join is a reservation (name, password, capacity, each with a reason) then a queue that syncs one joiner at a time and tells the others only when the joiner is done. We keep the reasons and "others see you after Ready"; we need no queue, because our `Welcome` is the state after event N and every later event follows in order; a name keeps its id, as Nitrox keeps players by name. Plan as written: `sn-session::server`: `Hello` (protocol version, build number, rules fingerprint, name) → `Welcome` or a readable reject; `Ready`; leave and timeout; the clock; relaying states; ordered events; the world-state hash. First read Nitrox's join and movement flow (architecture only, rule 6) and note here what we take. | Unit tests with simulated clients: a wrong build, protocol or fingerprint is rejected with its reason; 8 clients join, leave, time out and rejoin with consistent ids; every client receives every event in the same order with the same ticks; the hash is equal on all clients over 10,000 ticks with random joins and leaves. |
| **M10d** ✅ | **Done 2026-10-11** (MODLOG; client rules in `docs/formats/protocol.md` § Client rules): 7 tests (4 client, 3 oracle). Sync oracle, two clients and a server over the simulated network, 70 s each, both directions: **lan** 100 % of drawn ticks exact (3,498 of 3,498), error 0; **internet** 99.914 % exact (3 of 3,479 drawn before they arrived, at start-up and in a loss burst), max error 0.0000 m; **bad** (3 seeds) 99.68–99.91 % exact, max error 0.0000 m (the misses fell on straight segments). Clock error after 10 s: 0 µs (lan), ≤ 0.5 ms (internet), ≤ 10.5 ms (bad), all under a tick; world hashes 0 mismatches. Changed on the way, with the numbers in the MODLOG: states on a new unordered `sn-net` channel; 6 states per message; per-tick lateness for the delay (10 s window, cap 50 ticks, 1 s warm-up). Plan as written: `sn-session::client`: clock sync, tick buffers, interpolation and extrapolation, applying events. | **The sync oracle** (headless, impaired link, scripted trajectories with sharp turns, stops and teleports): on **lan** the drawn state equals the sent state at every tick (bit-exact after quantization); on **internet** ≥ 99.9 % of drawn ticks exact, the max error logged; on **bad** ≥ 99 % exact and max error < 0.5 m. These are targets: if the data says otherwise, we record why and agree new ones with the user. Clock offset error after 10 s logged (target < 1 tick). |
| **M10e** ✅ (one target open, see below) | **Done 2026-10-11** (MODLOG; fingerprint in `docs/formats/protocol.md` § Rules fingerprint): `sn-server` starts headless in 0.7 s (build "10", fingerprint `d269297a0830e68b`, tick 20,000 µs, Lifepod 5 for seed 1 at (−127.7, 0, −49.7)); over UDP on 127.0.0.1 a client of build "9" is refused with the build reason and 200 junk datagrams are survived (test); **10 minutes with 8 bots** (`sn-inspect bots`, scripted paths): 0 panics, 0 problems, 1,679,888 drawn ticks over 56 pairs all exact, 4,700+ hash checks 0 mismatches, 8.16 KB/s up per bot. Server work per loop pass: mean 37–54 µs (target < 1 ms met); worst pass per minute 0.6–3.0 ms (6.0 ms while a build ran), so the target is **not met for the worst case**; wall time on a machine also running the 8 bots, so OS scheduling is the suspect, not proven: M10f measures it with CPU time. Plan as written: `apps/sn-server`: flags (port, seeds, time scale, max players, an optional password typed at start and never stored), reads the host's install, binds UDP; logs joins, leaves, bandwidth per player and time per tick. | Runs headless without a window; rejects a client of another build (a fake `Hello` in a test); 0 panics in a 10-minute run with bots; time per tick logged (target < 1 ms with 8 players). |
| **M10f** ✅ (one target open, see below) | **Done 2026-10-11** (MODLOG): `sn-inspect coop`: bots run their scripts with the player's own rules and collision (recorded headless with `walk`'s pieces, about 3 s each, then replayed in the server's ticks: they cannot touch each other, § 9 decision 7, and the collision is static), over real UDP to `sn-server` with each bot's link impaired (`sn_net::ImpairedTransport`). Scripts: the pod's hatch with its first-use cinematic (6.6 s), a swim out in the bot's own direction; even bots board again (2.2 s cinematic), odd ones are killed and respawn in the pod. Results, drawn ticks exact: **lan** 2 bots 6,394/6,394, 4 bots 38,316/38,316, 8 bots 347,032/347,032; **internet** 100 % (2), 99.971 % (4), max error 0.058 m; **bad** 99.811 % (2), 99.846 % (4), max error 0.237 m; every bot saw every other bot's hatch, cinematic-end, death, respawn and animator-trigger events at the owner's own ticks (0 differences); world-state hash and slot-fill checks 0 differences; upload 8.0–8.3 KB/s per bot. Server work per pass with 8 players: median 31–32 µs, p99.9 0.45–0.47 ms, worst 0.58–1.12 ms (1 pass over 1 ms in about 59,000), so the worst-case target is **met to p99.9, not strictly** (the target became p99.9 by decision 9); the larger stalls (up to 25.6 ms) came in windows with 2–4 players, not 8, so they do not follow the load (OS scheduling suspected). Still wall time: CPU time needs OS calls (`unsafe`, which our lints forbid) or a dependency: § 9, item 9. Plan as written: `sn-inspect coop`: N bot clients (the real player rules and collision; scripted swims and walks from the lifepod, a hatch use, a `--kill`) against `sn-server` over UDP on this machine, impairment presets in between; each bot reports its slot-fill hashes. | For 2 and 4 bots on each preset: every bot's log of every other bot matches that bot's own log (M10d's targets, now with real movement); cinematic, death and respawn ticks equal everywhere; slot-fill and world-state hash mismatches 0; bandwidth per player logged (target < 10 KB/s up; it was 8 KB/s, but M10d measured 8.0–8.1 KB/s with the redundancy sync needs); the server's work per pass measured as CPU time, worst case < 1 ms with 8 players (left open by M10e, which measured wall time). |
| **M10g** ✅ | **Done 2026-10-11** (MODLOG): `sn-client`'s new `session.rs` runs `sn-session`'s server and client over a direct in-memory link (every message encoded and decoded); the server's tick 0 is the first frame. Start-up reads the rules (build "10", fingerprint `d269297a0830e68b`, 0.57–0.78 s), makes the new world's settings from the flags, joins, and takes Lifepod 5's point, the slot seed, the Aurora and the clock back from the `Welcome`. The player steps on the server's ticks (`Client::steps`, at most 10 per frame; the rest come later instead of being dropped); the game clock is `clock_start` + the server's time × `time_scale`. **Changed:** the Aurora's countdown now counts from the world's start (`Exploder::new_game_at`), not from the frame the ship's scene first updated, so it no longer moves with load time (before: 4983.0 and 4982.7 s in two runs; now 4979.6 s, server tick 224,979, every run). Regression (`--hatch-name top --use-hatch 3,15,27 --hold-forward 31 --kill 40`, two runs each): **the scripts' timing was already not exact before the change**: they are checked once per frame, so two baseline runs placed a scripted moment one step apart (85 of 452 trace lines a step later; 1 line differs at a window's edge). Measured instead: the 453 traced states (position, velocity, motor, flags, look, input) are equal in sequence before and after, the 15 event lines (hatch uses, push-outs, death, respawn) are equal, and scripted moments stay within one step of the baseline's (as between baseline runs); the two runs after the change are identical line for line. `--kill 3`: the 11 player lines are equal. `--benchmark 300` runs (no player). Plan as written: **Solo through the session.** The client always runs `sn-session::server` in-process (a direct in-memory link, no sockets, no impairment) and takes its clock and world settings from it; the player rules stay where they are. Nothing new on screen. | Regression: the scripted runs (`--use-hatch 3,15,27 --hold-forward 31`, `--kill`) print the same per-step trace as before the change (equal positions at every step); the clock, Aurora and lighting event ticks are equal before and after. |
| **M10h** ✅ (by its sub-steps; keyboard play together not tested) | **Play together.** `--host [PORT]` opens the in-process server to the network; `--join ADDRESS[:PORT]`, `--name`. Remote bodies (M9g) posed from the tick buffer and animated by the received values; remote cinematics and deaths; the pod's door animator from world events; players appear only after `Ready` and disappear on leave. `--net-sim lan\|internet\|bad` impairs the client's own link for testing. | Two clients on this machine (`--host`, `--join 127.0.0.1`), both scripted: each logs the other's drawn position every second next to the other's own log (M10d's targets); both log the same tick for each cinematic, death and respawn; screenshots for the user (a bonus). Keyboard play together **not tested by the agent**. |
| ↳ **M10h1** ✅ | **Done 2026-10-11** (MODLOG): `--host [PORT]`, `--join ADDRESS[:PORT]`, `--name`, `--net-sim`, `--log-ticks N` (testing). Two scripted clients on this machine (`--view 400`: two full clients on one GPU fell to 3 fps), every tick logged, about 2,500 ticks each way per run: no impairment **100 % exact** both ways; joiner on `internet` 2,502/2,503 and 2,622/2,622; on `bad` 2,498/2,503 (99.8 %) and 2,611/2,611; every miss is among the joiner's last ticks before it quit (its final states never left). Events: every hatch use, cinematic end, death, animator trigger and respawn seen by the other client at the owner's tick (events from before a join come in the `Welcome`, as designed). World-state hash checks 115–116 per minute, 0 differ; own steps skipped 0. Solo unchanged (M10g's trace, 453/453 states, same ticks). **Changed:** `Client::steps` skips a backlog over 25 ticks (the first run without it: the host at 2–4 fps fell 15.7 s behind). Plan as written: **The link.** `--host [PORT]` (default 27420), `--join ADDRESS[:PORT]`, `--name NAME`, `--net-sim lan\|internet\|bad`. The session's server, client and socket move to a network thread (every 1 ms, so a slow frame or the load before the first frame cannot time a connection out or blur the clock samples); the main thread takes steps, pushes states and events and samples remote players under a lock. `--join` waits up to 10 s for the `Welcome` before loading (the host's settings decide the world) and prints a reject's reason. Every physics step pushes the own state (position, velocity, look, rotation in a cinematic, flags, life, the body's animator values) and the owner's events (hatch used, cinematic ended, died, respawned, animator triggers). Logs: the own state at every 50th tick, each remote player's drawn state at the same ticks (with how it was made), events with their ticks. | Two scripted clients on this machine (`--host`, `--join 127.0.0.1`): every logged tick of the other matches the other's own log (no `--net-sim`: all exact; `--net-sim internet` / `bad` on the joiner: M10d's targets); both log the same tick for each hatch use, cinematic end, death and respawn; world-state hashes 0 differences. |
| ↳ **M10h2** ✅ | **Done 2026-10-11** (MODLOG): `sn-client` `remote.rs`; the body prefab spawned again per remote player once `Ready` (`objects::RemoteBodyRig`), removed on leave; rig placed from the sample at the drawn tick (rotation from the state, `camRoot` height less the skin and `verticalOffset`, the look's yaw); the state's animator floats and bools and the owner's triggers. Logs (host and joiner): body spawned at the join, the head drawn (1 shadows-only part), placed vs drawn rig base 0.000000 m over 566–940 checks per 10 s, removed when the joiner left. Screenshot `out/m10h2-join-view.png`: the host's body on Lifepod 5's roof after its hatch exit, seen by the joiner from the water. Not checked: the swim and walk animations by eye in motion. Plan as written: **Remote bodies.** For each remote player after `Ready`: the body prefab (head drawn, as the plan says), its rig at the drawn state's transform (rotation from the state, view model turned by the look's yaw), its animator given the received values; removed on leave. | Logs: bodies spawned and removed with the joins and leaves; each body's rig base equals its drawn sample (0 difference); screenshots of the other player swimming (a bonus). |
| ↳ **M10h3** ✅ | **Done 2026-10-11** (MODLOG): protocol 3 adds `CinematicPrepare` / `CinematicPlay` player events: the owner sends its hatch cinematic's signals at their steps (after `HatchUsed`, before `CinematicEnded`); receivers apply them when the drawn tick reaches them to the drawn pod's animators and the remote body's animator. The pod (colliders, hatches, cinematics) now loads whenever the world has one, also when the own player starts elsewhere (`--start`). Host floating outside, joiner using the top hatch, swimming, killed: the host saw all 10 of the joiner's events at the owner's ticks (hatch 644, prepare on 664, play on 676, prepare off 676, play off ×2 1086, end 1079, death 1743, trigger 1744, respawn 1993 in the owner's log; the host's lines match) and applied all 5 signals; drawn ticks 2,002/2,003 and 2,117/2,117 exact (the miss is the joiner's last tick). Solo unchanged (453/453). **Not done:** the own simulated pod animator does not get other players' signals (two players in one hatch at once: not tested); the mid-hatch screenshot missed the moment (timing), so the climb is not seen by eye. Plan as written: **Remote cinematics and the pod.** A remote `HatchUsed` at its tick: the pod's animator (every client's own copy) gets the hatch's signals, and the remote body's animator the cinematic's bools, so the remote body climbs as the owner's did; `CinematicEnded`, deaths (the death trigger) and respawns likewise. | Both clients log the same tick for each remote cinematic start and end, death and respawn as its owner; screenshots mid-hatch (a bonus). |
| **M10i** | **Over the real internet.** README "Play together": hosting, joining, the UDP port to forward (or a LAN), what is synced and what isn't yet. | The user and a friend on another machine, with their own install, see each other swim and use the hatch (run by the user; their log lines go into the MODLOG). |

##### How we prove it's in sync

1. **Same rules:** join rejects a different build, protocol or rules
   fingerprint (M10c).
2. **Same shared state:** clients report their world-state hash every
   50 ticks, named by the number of world events applied (the state is a
   function of the ordered log, so a count names it exactly; a tick would
   not, since player events carry their owner's tick: changed in M10c);
   the server compares with its own after that many events; mismatches
   counted and logged (target 0: any mismatch is a bug, not noise).
3. **Same history:** every bot and client logs each remote player's drawn
   state next to that player's own log of the same tick; error
   statistics per impairment preset (M10d, M10f, M10h).
4. **Same seeded world:** the slot-fill hash per batch, compared by the
   server.
5. **Same clock:** every client logs the tick of the Aurora's events and
   the pod's lighting changes; equal everywhere.

##### Not in M10 (and where it goes)

Items, inventory, crafting, saves (M9d–M9f, written server-first on top
of M10); creatures, the floating pod, simulation ownership and its
handoff (M11); chat and a join menu (M12); players colliding with each
other (the game has one player, so there is no rule to read: § 9);
voice; encryption (M10 is for friends on trust: no encryption, an
optional password, not fit for public servers, and the README says so);
NAT hole punching and relays (they need a third-party rendezvous service,
rule 5); interest management by batch (M11, when entities make it matter;
with eight players everything goes to everyone).

**Risks:** the clock slew and the interpolation targets may need tuning
with real numbers (M10d); the in-process server changes how solo play
starts (M10g's regression check guards it); remote cinematics rely on
the clip being the same everywhere, so the clips belong in the rules
fingerprint. The client's debug scripts (`--use-hatch`, `--hold-forward`)
act once per frame, so their moments vary by a step between runs (found
in M10g); M10h compares two clients tick by tick, which does not need
them exact.

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
Decided 2026-10-11 (the user took the M10 plan's recommendations):

4. M10 is client–server with "host a game" in the client, not P2P
   (analysis in the M10 plan).
5. M10b transport: our own thin layer on `std::net::UdpSocket`, no
   `renet`, `quinn` or `tokio` (small needs, two channels, fully testable
   on the in-memory link, no dependency).
6. Each player's client decides its own movement; the server only logs
   implausible moves (revisit for public servers).
7. Players pass through each other (the game has one player, so there is
   no rule to read).
8. Up to 8 players; tests with 2 and 4.

9. The server's work per pass is measured in wall time (M10e, M10f):
   median 31 µs and p99.9 0.47 ms with 8 players, single passes up to
   1.1 ms and stalls up to 25 ms that do not follow the load. Decided
   2026-10-11 (the user left it to the recommendation): the target is
   p99.9 < 1 ms, met; the worst case keeps being logged; no CPU-time
   dependency for now (revisit if the server ever falls behind).
10. M9d3's four choices (the M9d3 plan, 2026-10-11; the user leaves open
   choices to the recommendation): inventories are shared world state
   replayed on every machine; random draws travel as results; the client
   names the object it acts on (key and class id); slot fillers are keyed
   by placeholder id, slot and copy.

#### Lost River waterfall cap correction (2026-10-14)

The user reports green and blue waterfall tops extending too high. Research
found an unported `VFXWaterFall` startup scaling rule: the top and bottom
local Y scales divide their configured heights by the waterfall root's
local Y scale. Keep the middle and cap positions unchanged. Decode the
stored fields/references and apply the rule per placement, not to a shared
mesh or template. No architecture changes or dependencies.

Done when:
- [x] Checked parser and synthetic scale tests pass, including malformed input.
- [x] Both waterfall families resolve their cap references from the install;
      cap heights remain constant across different placement Y scales.
- [x] Workspace tests, fmt, clippy and client GPU regression pass.
- [ ] Human comparison confirms green and blue starting heights (not tested).

Implemented and numerically checked: three installed families/six caps,
630 workspace tests passed. Both matched GPU scenes retain identical
terrain/object counts and no loader warnings; 50 cap instances retain
configured thickness. Details and commands in MODLOG. Human visual
confirmation remains open.

#### Lost River lake coverage investigation (2026-10-14)

The user reports extra and missing green surfaces in multiple client locations.
Audit the placed lake meshes, especially rotated border-fading children under
nonuniform placement scales: our flattened TRS composition may change their
footprints. Compare flattened vertices with sequential parent/child application
from the installed hierarchy before changing rendering. Also check depth/culling
contracts. Keep this concern separate from committed waterfall cap correction.

Done when:
- [x] Quantify any transform discrepancy on installed lake placements.
- [x] If confirmed, make a scoped rendering correction without an architecture change.
- [x] Synthetic regression and installed-data checks, workspace tests and GPU checks pass.
- [x] Human confirmation of lake coverage: user reports "looks good to me" (2026-10-14).

Implemented as a scoped rigid-effect spawn correction: a placement parent
and its local mesh children replace flattened world TRS. Animated effects
retain their existing rig attachments; core coordinate APIs are unchanged.
328 installed lake surfaces audited, 16 previously distorted by up to
17.380856 m; affine composition agrees with sequential application within
0.000123 m. 631 workspace tests pass; two GPU regressions pass. Exact user
screenshot location and original-game coverage remain unverified.
