# MODLOG

One entry per change: what, why, how it was verified. Record dead ends too.

## 2026-10-06 — M5: Unity bundles and serialized files

**What:**
- New crate `sn-unity` (layer 1). `Bundle::parse` (UnityFS format 6–8; LZ4/LZ4HC/stored blocks; LZMA is
  reported as unsupported because the game has none) and `SerializedFile::parse` (versions 14–22: types, optional
  type trees, object table, externals) with `object_data`. Also `class_name` (built-in class ids) and
  `write_bundle` (synthetic bundles for tests). Bounds-checked reader: errors carry byte offsets.
- New dependency: `lz4_flex` 0.14 (MIT, pure Rust): LZ4 block decompression for bundles.
- `sn-install`: `data_dir`, `bundle_dir`, `read_file`, `serialized_files`, `bundles`.
- `sn-inspect unity <FILE>…` (canonical listing), `unity --types` (with class names), `unity --all`.
- `docs/formats/unity.md`.

**Findings:** Unity 2019.4.36f1, serialized format 21 and UnityFS 7 everywhere; **no type trees in any file**
(the risk from DESIGN.md is real); no LZMA. 5,472 files, 5,485 serialized files, 423,677 objects.

**Verified (2026-10-06):**
1. Oracle: UnityPy 1.25.4 in a temp venv (outside the repo), with a summary script in the same temp folder.
   Ran on 35 files: the 5 player files, the 5 largest bundles and 25 random bundles (seed 5).
   Our `sn-inspect unity` output is **identical** (395 lines: versions, type and object counts per class,
   object byte totals, externals, resource sizes).
2. `sn-inspect unity --all`: every file parses, 0 errors, 2.9 s.
3. `cargo test -p sn-unity -- --include-ignored`: synthetic round trips (with/without LZ4), every truncation
   and every single-byte corruption parses without panicking, and the real-data test (5,472 / 5,485 / 423,677,
   0 type trees) passes. Workspace: 42 tests, clippy clean.

**Dead end:** the first oracle diff failed only on Windows line endings (`
` from Python), which had also
crept into the random file names. Fixed in the harness, not the code.
**Scope change:** Addressables catalog moved from M5 to M7 (first user; avoids JSON/base64 dependencies now).

## 2026-10-06 — M4: terrain streaming and levels of detail

**What:**
- `sn-octree`: `Octree::sample` / `Batch::sample` look up one voxel by walking down the tree, so coarse
  levels never expand whole batches.
- `sn-mesh`: `Field::with_step` (sample spacing 1/2/4/8) and `add_skirts` (strips hanging from open edges
  into the solid, to hide cracks between levels of detail).
- `sn-terrain`: works on parsed `TerrainBatch`es (sampling) instead of fully expanded grids; `batch_field` /
  `batch_mesh` take a level of detail (0..=3). `sn-install::load_batch` returns a validated `TerrainBatch`.
- `sn-client`: `TerrainStreamer`. Worker threads with a queue rebuilt on every 8 m of camera movement
  (missing batches first, then nearest); level of detail by distance to the batch box
  (100 / 260 / 600 / 1200 m, scaled by `--view`) with 20 m hysteresis; old mesh kept until the new
  one is ready (no holes); unload outside the view; parsed-batch cache with a 1,200-batch limit
  (least-recently-used eviction); at most 300k triangles uploaded per frame.
  New options `--start`, `--look`, `--view`, `--flythrough X Y Z [--speed]`; `--radius` removed.
- `sn-inspect`: `mesh --lod L`, and `voxel X Y Z` (what's at a world position + surfaces in that column).

**Verified (2026-10-06, RTX 3080, debug build):**
1. `sn-inspect mesh 12 18 12 --radius 1 --lod 0..3`: 2,031,606 / 493,862 / 118,880 / 27,316 triangles,
   0 open edges on batch seams at every level. Level 0 matches M2/M3 exactly (sampling = expanding).
2. `sn-client --flythrough 1700 -80 0` (lifepod → crater edge, 40 m/s, 42.5 s): process memory
   0.61–0.89 GiB throughout, batch cache capped at 1,200, start area loaded in 2.0–2.3 s, destination
   loaded 0.1–0.25 s after arrival. Request→screen latency per level: lod0 ~400 ms (meshing-bound),
   lod1 ~60 ms, lod2 ~15 ms, lod3 ~10 ms.
3. Frame times vary a lot between identical runs on this machine (mean 104 / 288 / 300 / 405 fps over 4 runs),
   so something else was loading the PC. Best run with the upload cap: p95 3.0 ms, worst 26 ms (before the
   cap: p95 5.3 ms, worst 64 ms).
4. `cargo test --workspace`: 37 passed (sampling = expanding; step scaling; skirts; seams closed at
   lod 0–2; coarser levels have fewer triangles; mixed levels crack and get skirts). clippy clean.

**Investigated, not bugs:** the flythrough's final screenshot is pure fog: the camera looks outward over the
crater-edge drop (seafloor 390 m below, per `sn-inspect voxel 1700 -80 0`). A mid-way screenshot showed a grid
of strips: the straight fly path was 22 m inside rock there (`voxel 800 -40 0` → solid, surface at −18), and the
strips are skirts seen from inside the rock. Players can't be there.
**Not tested:** interactive flying by a human; release build; whether the upload cap helps reliably (noisy machine).

## 2026-10-06 — M3: Bevy client with free-fly camera

**What:**
- `sn-install` (layer 3): `GameData` moved out of sn-inspect; adds `load_batch` (read + parse + rasterize).
- `sn-terrain` (layer 2, pure): `Neighbourhood` (batch + 26 neighbours), `batch_field`, `batch_mesh`,
  `debug_colour`. Moved out of sn-inspect's `terrain.rs`.
- `sn-world`: `VOXEL_WORLD_OFFSET`, `voxel_to_world`, `world_to_voxel` (offset still a hypothesis).
- `apps/sn-client`: Bevy 0.19.1 (+ `free_camera` feature for the built-in fly camera). Loads and meshes
  batches on background threads (all cores but one), splits meshes per material, converts Unity
  left-handed → Bevy right-handed (flip z, reverse winding), distance fog, a stats log every 2 s, and
  `--benchmark N` (N frames without vsync after loading → frame-time stats + screenshot → exit).
- New dependency: `bevy` 0.19.1, the engine chosen in DESIGN.md; latest stable (0.20 is still RC).

**Verified (2026-10-06, RTX 3080, debug build with opt-level 1):**
1. `sn-client --benchmark 600` on 3×3×3 batches around 12-18-12: 2,031,606 triangles,
   loaded in 7.25 s; mean 3.50 ms/frame (286 fps), 95th percentile 4.30 ms, worst 159.75 ms
   (the frame where all meshes upload at once).
2. Screenshot `out/client-benchmark.png` (gitignored) shows recognisable Safe Shallows seabed
   (pillars, arches, rolling sand), lit, false-coloured per type id.
3. `cargo test --workspace`: 30 passed (3 new sn-terrain tests: sphere across 8 batches closed,
   clamped apron adds no surface, absent centre). clippy `-D warnings` clean.
4. `sn-inspect mesh 7 17 10` after the refactor: same numbers as before (220,258 triangles, 0 unexpected open edges).

**Notes:** Vulkan prints a harmless "Loader Message" ERROR at start-up (driver loader noise).
**Not tested:** interactive controls (needs a human); release build; other GPUs; whether the map is mirrored.

## 2026-10-06 — M2: terrain to mesh (OBJ export)

**What:**
- New crate `sn-mesh` (no deps): `Field`, `surface_nets`, `edge_report` (topology checks).
  Chunking rule: the outer sample layer of a field is an apron; neighbouring fields
  overlap by 2 samples; vertices are computed in global coordinates, so they weld
  bit-exactly.
- `sn-octree`: `Voxel::signed_density`, `SURFACE_DENSITY`, `Batch::rasterize` → `BatchGrid`.
- `sn-inspect`: `density X Y Z` (evidence for density semantics), `mesh X Y Z [--radius R]`
  (OBJ + MTL with false colours per type into `out/`, plus hole/orientation checks);
  `octree --all` now also checks density against type world-wide.

**Findings:**
- Density ≤ 125 → empty, ≥ 126 → solid, with no exceptions across 263 M leaves. 0 = far from the surface.
  Gradient is about 15 units/voxel near the surface, compressed further out.
- Missing batch files are uniform (faces touching them are 0% or 100% solid), so
  the apron copies edge voxels outward ("clamp") when a neighbour is missing.
- World offset hypothesis is plausible: seabed at world (0,0) is y = −17; highest terrain y = +156 at (345, 907).

**Dead ends / bugs found by tests:**
- Sharing 1 sample layer between chunks leaves a strip of holes (120 open edges in the
  test). Fixed with the apron + 2-sample-overlap rule.
- Overflow in the neighbour lookup (`(-1) as usize + 1`). Caught at the first real run.

**Verified (2026-10-06):**
1. `cargo test --workspace`: 27 passed (sphere/torus closed with correct Euler characteristic,
   split-field weld identical to whole, materials, origins, batch placement).
2. `sn-inspect mesh 7 17 10`: 220,258 triangles, 0 flipped, 0 unexpected open edges, 99.8% facing water.
3. `sn-inspect mesh 12 18 12 --radius 1`: 27 batches, 2,031,606 triangles, 0 open edges along
   batch seams, 0 flipped, 99.6% facing water, 13.8 s (debug build), 145 MB OBJ.
4. clippy `-D warnings` clean; real-data test still passes.

**Not tested:** how it looks in Blender (human check pending). 1,395 edges are shared by 3+ triangles
(about 0.05%; a known surface-nets artefact at thin features). Mirroring vs the in-game map.

## 2026-10-06 — M1: decode every terrain octree

**What:** Cargo workspace (edition 2024, licence MIT OR Apache-2.0, dev profile
opt-level 1 / deps 3). New crates, all without dependencies:
- `sn-octree`: parse, validate, rasterize, encode (synthetic fixtures); `AxisOrder`;
  confirmed `GAME_CHILD_ORDER` / `GAME_OCTREE_ORDER` = ZYX.
- `sn-world`: `index.txt` parser, `BatchCoord`, partial edge-batch dimensions.
- `sn-inspect`: `index`, `octree X Y Z`, `octree --all`, `orient X Y Z`.
Docs: `docs/formats/optoctrees.md`, `docs/formats/world-index.md`, README, licence files.

**Findings:**
- `index.txt` (not `meta.txt`, as M0 said) holds the dimensions, followed by
  exactly 13,520 per-batch floats (655 non-zero, meaning unknown). `meta.txt` is `39` / `BlockPrefabs`.
- All 669,150 octrees validate; 300,431,310 nodes; largest 37,449 (complete tree).
- Child order and octree order are both ZYX (z fastest). `orient` on 7 batches:
  seam/interior disagreement ratio 0.76–1.11 for ZYX/ZYX, runner-up 1.4–10× worse.
- +Y is up: non-empty share per batch row goes from 100% (Y=7) to 0.18% (Y=19).

**Dead end:** The M1 criterion "≥ 0.95 agreement for each of 8 orderings" was
flawed. Inside one octree all axis orders score the same (0.9942 on 12-18-12),
and even a scrambled order scores 0.985. Seams are what distinguish orders; DESIGN.md is updated.
Also: random batches like 8-15-14 are uniform and carry no orientation signal,
so `orient` now reports them as INCONCLUSIVE.

**Environment issue:** linking failed because the Windows SDK was missing
(rustc doesn't find MSVC `link.exe` without it). The user installed SDK 10.0.26100.
Note: in Git Bash, `/usr/bin/link` shadows MSVC's linker, so run cargo from PowerShell/cmd.

**Verified (all run 2026-10-06):**
1. `cargo test --workspace`: 19 passed, 1 ignored (real data), without the game.
2. `SUBNAUTICA_DIR=… cargo test -p sn-octree -- --ignored`: passed (5,416 files, 5,259×125 + 157×75, max 37,449 nodes).
3. `sn-inspect octree 12 18 12`: node type histogram identical to the M0 Python probe (24 ids, 172:47194, …).
4. `sn-inspect orient` on 12-18-12, 7-17-10, 9-16-5, 4-16-11, 6-17-9, 5-16-19, 22-17-11: ZYX/ZYX best on all.
5. `cargo clippy --workspace --all-targets -- -D warnings`: clean.
6. `git status`: only source and docs.

**Not tested:** Linux/macOS; density/type semantics (M2).

## 2026-10-06 — Scope: desktop only

**What:** Dropped the browser/WASM target from DESIGN.md, AGENTS.md and .gitignore.
Removed `sn-vfs` (browser file access) in favour of a desktop `sn-install` crate;
networking switched from WebSocket to UDP; web milestones removed and the roadmap
renumbered (now M0–M12, ending in a playable multiplayer desktop demo).
**Why:** User decision — the browser made the project much harder for little gain now.
**Verified:** `git status --short` still lists only whitelisted files.

## 2026-10-06 — M0: repository bootstrap

**What:** Added `AGENTS.md`, whitelist `.gitignore`, `docs/DESIGN.md`, this log.
`git init` (no commits).

**Why:** Safety rules and architecture before any engine code.

**Probe of the install (read-only, throwaway Python script outside the repo):**
- Build: `__buildnumber.txt` = `10`, `__buildtime.txt` = `10/03/2025 17:26:27`, data in `SNUnmanagedData/Build18`.
- `meta.txt`: `4096 3200 4096` / `128 100 128` / `32` / `5 5 5`.
- `CompiledOctreesCache`: 5,416 files, all start with i32 version `4`.
- Parsing as `u16 count` + `count × 4-byte node` repeated: every file consumed exactly;
  5,259 files contain 125 octrees, 157 contain 75. Max nodes in one octree: 37,449.
- `compiled-batch-12-18-12.optoctrees` node type histogram (all nodes, incl. internal):
  `0:99021 1:46916 2:2502 3:3357 5:1559 7:8 8:4537 10:297 13:286 14:1697 19:865 20:209
  21:14 22:1503 35:2084 50:393 51:39 52:14 100:2224 101:2538 102:13880 104:805 105:991
  172:47194` (24 distinct ids). Reference values for M1 "Done when" item 4.

**Verified:** `git status --short` lists only the whitelisted files (see below).
**Not tested:** anything about node semantics (child order, density meaning) — that is M1/M2.
